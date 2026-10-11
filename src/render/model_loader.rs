//! Lazy CPU-side preparation for authored 3D resources.
//!
//! AWD parsing/OBJ cache writes and ATF/JXR image decoding can be expensive,
//! so they run on one background thread. This module has no raylib dependency;
//! model and texture GPU creation remains on the render thread.

use anyhow::{Context, Result};
use image::RgbaImage;
use std::collections::{hash_map::DefaultHasher, HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::sync::{mpsc, Arc};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Kind {
    Mesh,
    Texture,
}

/// Identity includes the raw-content digest so a new CDN version does not
/// reuse a stale prepared value or a previous version's negative-cache entry.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct AssetKey {
    kind: Kind,
    id: String,
    digest: u64,
}

impl AssetKey {
    pub(crate) fn id(&self) -> &str {
        &self.id
    }

    pub(crate) fn digest(&self) -> u64 {
        self.digest
    }
}

pub(crate) enum PreparedAsset {
    MeshObj(PathBuf),
    Texture(RgbaImage),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Status {
    Unknown,
    Pending,
    Ready,
    Failed,
}

enum Job {
    Mesh {
        key: AssetKey,
        bytes: Arc<[u8]>,
        cache_path: PathBuf,
    },
    Texture {
        key: AssetKey,
        bytes: Arc<[u8]>,
    },
}

struct Completion {
    key: AssetKey,
    result: Result<PreparedAsset>,
}

/// One worker per `TextureCache`, started only after a 3D asset is requested.
/// Requested, prepared, and failed maps are all digest-keyed and deduplicated.
#[derive(Default)]
pub(crate) struct ModelLoader {
    sender: Option<mpsc::Sender<Job>>,
    receiver: Option<mpsc::Receiver<Completion>>,
    requested: HashSet<AssetKey>,
    pending: HashSet<AssetKey>,
    prepared: HashMap<AssetKey, PreparedAsset>,
    failed: HashMap<AssetKey, String>,
    worker_failed: bool,
}

impl ModelLoader {
    pub(crate) fn mesh_key(id: &str, bytes: &[u8]) -> AssetKey {
        AssetKey {
            kind: Kind::Mesh,
            id: id.to_owned(),
            digest: digest(bytes),
        }
    }

    pub(crate) fn texture_key(id: &str, bytes: &[u8]) -> AssetKey {
        AssetKey {
            kind: Kind::Texture,
            id: id.to_owned(),
            digest: digest(bytes),
        }
    }

    pub(crate) fn request_mesh(&mut self, key: AssetKey, bytes: Arc<[u8]>, cache_path: PathBuf) {
        if !self.begin_request(&key) {
            return;
        }
        self.enqueue(Job::Mesh {
            key,
            bytes,
            cache_path,
        });
    }

    pub(crate) fn request_texture(&mut self, key: AssetKey, bytes: Arc<[u8]>) {
        if !self.begin_request(&key) {
            return;
        }
        self.enqueue(Job::Texture { key, bytes });
    }

    fn begin_request(&mut self, key: &AssetKey) -> bool {
        if self.requested.contains(key) {
            return false;
        }
        self.requested.insert(key.clone());
        self.pending.insert(key.clone());
        true
    }

    fn enqueue(&mut self, job: Job) {
        let key = match &job {
            Job::Mesh { key, .. } | Job::Texture { key, .. } => key.clone(),
        };
        if !self.ensure_worker() {
            self.pending.remove(&key);
            self.failed
                .insert(key, "3D preparation worker unavailable".into());
            return;
        }
        if self
            .sender
            .as_ref()
            .is_none_or(|sender| sender.send(job).is_err())
        {
            self.pending.remove(&key);
            self.failed
                .insert(key, "3D preparation worker stopped".into());
            self.worker_failed = true;
        }
    }

    fn ensure_worker(&mut self) -> bool {
        if self.worker_failed {
            return false;
        }
        if self.sender.is_some() {
            return true;
        }
        let (jobs, rx) = mpsc::channel::<Job>();
        let (done, completions) = mpsc::channel::<Completion>();
        let spawned = std::thread::Builder::new()
            .name("models3d-cpu".into())
            .spawn(move || {
                while let Ok(job) = rx.recv() {
                    let completion = match job {
                        Job::Mesh {
                            key,
                            bytes,
                            cache_path,
                        } => Completion {
                            key,
                            result: prepare_mesh(&bytes, &cache_path).map(PreparedAsset::MeshObj),
                        },
                        Job::Texture { key, bytes } => Completion {
                            key,
                            result: decode_texture(&bytes).map(PreparedAsset::Texture),
                        },
                    };
                    if done.send(completion).is_err() {
                        break;
                    }
                }
            });
        if let Err(error) = spawned {
            log::warn!("cannot start 3D preparation worker: {error}");
            self.worker_failed = true;
            return false;
        }
        self.sender = Some(jobs);
        self.receiver = Some(completions);
        true
    }

    /// Drain ready CPU results without waiting for outstanding work.
    pub(crate) fn poll(&mut self) {
        let Some(receiver) = &self.receiver else {
            return;
        };
        let mut disconnected = false;
        loop {
            match receiver.try_recv() {
                Ok(completion) => {
                    self.pending.remove(&completion.key);
                    match completion.result {
                        Ok(prepared) => {
                            self.prepared.insert(completion.key, prepared);
                        }
                        Err(error) => {
                            let message = format!("{error:#}");
                            log::warn!(
                                "3D asset preparation {} failed: {message}",
                                completion.key.id()
                            );
                            self.failed.insert(completion.key, message);
                        }
                    }
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    disconnected = true;
                    break;
                }
            }
        }
        if disconnected {
            self.mark_pending_failed("3D preparation worker stopped".into());
        }
    }

    fn mark_pending_failed(&mut self, message: String) {
        self.worker_failed = true;
        for key in self.pending.drain() {
            self.failed.insert(key, message.clone());
        }
    }

    pub(crate) fn status(&self, key: &AssetKey) -> Status {
        if self.prepared.contains_key(key) {
            Status::Ready
        } else if self.failed.contains_key(key) {
            Status::Failed
        } else if self.pending.contains(key) {
            Status::Pending
        } else {
            Status::Unknown
        }
    }

    pub(crate) fn take(&mut self, key: &AssetKey) -> Option<PreparedAsset> {
        self.prepared.remove(key)
    }

    pub(crate) fn failure(&self, key: &AssetKey) -> Option<&str> {
        self.failed.get(key).map(String::as_str)
    }
}

fn digest(bytes: &[u8]) -> u64 {
    let mut hasher = DefaultHasher::new();
    bytes.hash(&mut hasher);
    hasher.finish()
}

fn prepare_mesh(bytes: &[u8], cache_path: &std::path::Path) -> Result<PathBuf> {
    if cache_path
        .metadata()
        .is_ok_and(|metadata| metadata.len() > 0)
    {
        return Ok(cache_path.to_owned());
    }
    let mesh = crate::assets::awd::parse_aw2(bytes).context("parse AWD geometry")?;
    let obj = mesh.to_obj();
    let parent = cache_path
        .parent()
        .context("OBJ cache path has no parent")?;
    std::fs::create_dir_all(parent).context("create OBJ cache directory")?;
    let mut temp = cache_path.as_os_str().to_owned();
    temp.push(".tmp");
    let temp = PathBuf::from(temp);
    std::fs::write(&temp, obj).context("write prepared OBJ")?;
    if let Err(error) = std::fs::rename(&temp, cache_path) {
        if !cache_path
            .metadata()
            .is_ok_and(|metadata| metadata.len() > 0)
        {
            let _ = std::fs::remove_file(&temp);
            return Err(error).context("publish prepared OBJ");
        }
        let _ = std::fs::remove_file(&temp);
    }
    Ok(cache_path.to_owned())
}

fn decode_texture(bytes: &[u8]) -> Result<RgbaImage> {
    if bytes.starts_with(b"ATF") {
        return crate::assets::atf::decode(bytes).context("decode ATF texture");
    }
    if bytes.starts_with(b"II\xBC\x01") || bytes.starts_with(b"MM\x01\xBC") {
        return crate::assets::tilemap::raster_image("jxr", bytes).context("decode JXR texture");
    }
    image::load_from_memory(bytes)
        .context("decode 3D texture")
        .map(|image| image.to_rgba8())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn wait_for(loader: &mut ModelLoader, key: &AssetKey) -> Status {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            loader.poll();
            let status = loader.status(key);
            if status != Status::Pending || Instant::now() >= deadline {
                return status;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    #[test]
    fn lazily_prepares_real_awd_to_obj_off_thread() {
        let bytes: Arc<[u8]> =
            Arc::from(include_bytes!("../../tests/fixtures/3d/aegis.awd").as_slice());
        let key = ModelLoader::mesh_key("aegis-test", &bytes);
        let cache =
            std::env::temp_dir().join(format!("xenomit-model-loader-{}", std::process::id()));
        let path = cache.join(format!("aegis-{:016x}.obj", key.digest()));
        let mut loader = ModelLoader::default();
        assert!(
            loader.sender.is_none(),
            "worker must remain lazy until a request"
        );
        loader.request_mesh(key.clone(), bytes, path.clone());
        assert_eq!(loader.status(&key), Status::Pending);
        assert_eq!(wait_for(&mut loader, &key), Status::Ready);
        assert!(loader.sender.is_some());
        let Some(PreparedAsset::MeshObj(prepared)) = loader.take(&key) else {
            panic!("worker did not produce the cached OBJ path")
        };
        let obj = std::fs::read_to_string(prepared).unwrap();
        assert!(obj.lines().any(|line| line.starts_with("v ")));
        assert!(obj.lines().any(|line| line.starts_with("f ")));
        let _ = std::fs::remove_dir_all(cache);
    }

    #[test]
    fn failures_are_deduplicated_per_content_version_and_valid_texture_decodes() {
        let mut loader = ModelLoader::default();
        let bad: Arc<[u8]> = Arc::from(&b"not an image"[..]);
        let bad_key = ModelLoader::texture_key("versioned-texture", &bad);
        loader.request_texture(bad_key.clone(), bad.clone());
        assert_eq!(wait_for(&mut loader, &bad_key), Status::Failed);
        let requested = loader.requested.len();
        loader.request_texture(bad_key.clone(), bad);
        assert_eq!(
            loader.requested.len(),
            requested,
            "same failed version must not enqueue twice"
        );

        let mut png = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(RgbaImage::from_pixel(
            2,
            3,
            image::Rgba([12, 34, 56, 255]),
        ))
        .write_to(&mut png, image::ImageFormat::Png)
        .unwrap();
        let valid: Arc<[u8]> = Arc::from(png.into_inner());
        let valid_key = ModelLoader::texture_key("versioned-texture", &valid);
        assert_ne!(
            bad_key, valid_key,
            "new bytes must not inherit the prior negative cache"
        );
        loader.request_texture(valid_key.clone(), valid);
        assert_eq!(wait_for(&mut loader, &valid_key), Status::Ready);
        let Some(PreparedAsset::Texture(image)) = loader.take(&valid_key) else {
            panic!("worker did not produce decoded pixels")
        };
        assert_eq!(image.dimensions(), (2, 3));
    }
}
