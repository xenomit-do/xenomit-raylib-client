//! World3D.as:73 separate resource collection. Raw geometry AND textures
//! share this index; CPU decoding and GPU ownership belong to the renderer.
//! All HTTP/disk reads run on a lazy worker, never on the render thread.

use super::{bootstrap::BootConfig, collection::Collection, resources::FileIndex};
use anyhow::{Context, Result};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

type Completion = (String, Result<Arc<[u8]>, String>);

/// Waits after transient failure #1/#2/#3 — a resource is sent at most
/// `1 + RETRY_DELAYS.len()` times before landing in `failed` permanently.
const RETRY_DELAYS: [Duration; 3] = [
    Duration::from_secs(2),
    Duration::from_secs(5),
    Duration::from_secs(15),
];

/// A (re)sent fetch may be in flight for the whole HTTP timeout; a
/// re-request inside this window must not double-enqueue. Ends early when
/// the completion lands (success clears the entry, failure reschedules).
const IN_FLIGHT_WINDOW: Duration = Duration::from_secs(70);

/// When the manifest index itself failed, resource retries re-attempt it —
/// throttled so a burst of ids doesn't serialize repeated index fetches.
const INDEX_RETRY_DELAY: Duration = Duration::from_secs(15);

/// Per-request fetch timeout (the shared client defaults to 20s, too tight
/// for multi-MB ATF/AWD files on a slow CDN stream).
const FETCH_TIMEOUT: Duration = Duration::from_secs(60);

/// A fetch that errored but may succeed later — re-sent after `next_at`.
struct Retry {
    /// Retry sends performed so far; indexes `RETRY_DELAYS` for the wait
    /// scheduled after the next failure.
    sent: u32,
    next_at: Instant,
}

pub struct GeometryCollection {
    base: String,
    hash: String,
    manifest: &'static str,
    cache_dir: PathBuf,
    http: reqwest::Client,
    requests: Option<mpsc::Sender<String>>,
    completions: Option<mpsc::Receiver<Completion>>,
    requested: HashSet<String>,
    ready: HashMap<String, Arc<[u8]>>,
    failed: HashMap<String, String>,
    /// Transient failures awaiting their backoff — invisible to `failure`
    /// so callers keep treating the id as pending until retries run out.
    retries: HashMap<String, Retry>,
    /// The manifest index once `load_index` succeeded — published by the
    /// worker so the render thread can presence-check file ids without
    /// queueing a doomed download.
    index: Arc<Mutex<Option<Arc<FileIndex>>>>,
}

impl GeometryCollection {
    pub fn new(
        static_host: String,
        base_path: String,
        hash: String,
        cache_dir: PathBuf,
        http: reqwest::Client,
    ) -> Self {
        let base = BootConfig {
            cdn_host: static_host,
            base_path,
            ..Default::default()
        }
        .resource_base();
        Self {
            base,
            hash,
            cache_dir,
            http,
            manifest: "resources_3d",
            requests: None,
            completions: None,
            requested: HashSet::new(),
            ready: HashMap::new(),
            failed: HashMap::new(),
            retries: HashMap::new(),
            index: Arc::new(Mutex::new(None)),
        }
    }

    pub fn particles(
        static_host: String,
        base_path: String,
        hash: String,
        cache_dir: PathBuf,
        http: reqwest::Client,
    ) -> Self {
        let mut result = Self::new(static_host, base_path, hash, cache_dir, http);
        result.manifest = "resources_3d_particles";
        result
    }

    /// Enqueue a fetch. In-flight and finished ids dedupe; a retryable
    /// failure is re-sent once its backoff elapses — callers request every
    /// frame, so `retries` doubles as the retry timer. A new client
    /// connection recreates the store.
    pub fn request(&mut self, id: &str) {
        if id.is_empty() {
            return;
        }
        if self.requested.contains(id) {
            let Some(retry) = self.retries.get_mut(id) else {
                return;
            };
            if Instant::now() < retry.next_at {
                return;
            }
            retry.sent += 1;
            retry.next_at = Instant::now() + IN_FLIGHT_WINDOW;
        } else {
            self.requested.insert(id.to_owned());
        }
        if self.requests.is_none() {
            self.start_worker();
        }
        if self
            .requests
            .as_ref()
            .is_none_or(|tx| tx.send(id.to_owned()).is_err())
        {
            self.failed
                .insert(id.to_owned(), "3D resource worker unavailable".into());
        }
    }

    pub fn get(&self, id: &str) -> Option<Arc<[u8]>> {
        self.ready.get(id).cloned()
    }
    pub fn failure(&self, id: &str) -> Option<&str> {
        self.failed.get(id).map(String::as_str)
    }

    /// The manifest index finished loading — `has_file` answers are real.
    pub fn index_ready(&self) -> bool {
        self.index
            .lock()
            .map(|slot| slot.is_some())
            .unwrap_or(false)
    }

    /// `id` has a manifest entry. False until the index is loaded (and
    /// permanently false if the manifest fetch failed).
    pub fn has_file(&self, id: &str) -> bool {
        let Ok(slot) = self.index.lock() else {
            return false;
        };
        slot.as_ref().is_some_and(|index| index.entry(id).is_some())
    }

    pub fn pump(&mut self) {
        let Some(rx) = &self.completions else {
            return;
        };
        while let Ok((id, result)) = rx.try_recv() {
            match result {
                Ok(bytes) => {
                    self.retries.remove(&id);
                    self.ready.insert(id, bytes);
                }
                Err(error) => {
                    let sent = self.retries.get(&id).map(|r| r.sent).unwrap_or(0);
                    if is_permanent(&error) || sent as usize >= RETRY_DELAYS.len() {
                        log::warn!("3D resource {id}: {error}");
                        self.retries.remove(&id);
                        self.failed.insert(id, error);
                    } else {
                        let delay = RETRY_DELAYS[sent as usize];
                        log::warn!(
                            "3D resource {id}: {error} (retrying in {}s)",
                            delay.as_secs()
                        );
                        self.retries.insert(
                            id,
                            Retry {
                                sent,
                                next_at: Instant::now() + delay,
                            },
                        );
                    }
                }
            }
        }
    }

    fn start_worker(&mut self) {
        let (tx, rx) = mpsc::channel::<String>();
        let (done, completions) = mpsc::channel::<Completion>();
        let (base, hash, manifest, cache, http, index_slot) = (
            self.base.clone(),
            self.hash.clone(),
            self.manifest,
            self.cache_dir.clone(),
            self.http.clone(),
            self.index.clone(),
        );
        let spawned = std::thread::Builder::new()
            .name("assets-3d".into())
            .spawn(move || {
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build();
                let collection = Collection::new(cache.join(manifest));
                // Fetch only once on this worker. A bad manifest is reported for
                // each requested asset and cannot interrupt the playable 2D view.
                let mut files = match &rt {
                    Ok(rt) => rt
                        .block_on(load_index(&http, &base, manifest, &hash, &collection))
                        .map_err(|e| format!("{e:#}")),
                    Err(e) => Err(format!("3D runtime: {e}")),
                };
                let publish_index = |index: &FileIndex| {
                    let mut slot = index_slot.lock().unwrap_or_else(|p| p.into_inner());
                    *slot = Some(Arc::new(index.clone()));
                };
                // Publish the index so the render thread can presence-check ids.
                if let Ok(index) = &files {
                    publish_index(index);
                }
                let mut index_retry_at = Instant::now();
                while let Ok(id) = rx.recv() {
                    // A failed index retries (throttled) when a request still
                    // comes in — resource retries would otherwise re-fail on the
                    // cached manifest error.
                    if files.is_err() && Instant::now() >= index_retry_at {
                        index_retry_at = Instant::now() + INDEX_RETRY_DELAY;
                        if let Ok(rt) = &rt {
                            files = rt
                                .block_on(load_index(&http, &base, manifest, &hash, &collection))
                                .map_err(|e| format!("{e:#}"));
                            if let Ok(index) = &files {
                                publish_index(index);
                            }
                        }
                    }
                    let result = match (&rt, &files) {
                        (Ok(rt), Ok(index)) => rt
                            .block_on(load_resource(&http, &base, index, &collection, &id))
                            .map(Arc::from)
                            .map_err(|e| format!("{e:#}")),
                        (_, Err(error)) => Err(error.clone()),
                        (Err(error), _) => Err(error.to_string()),
                    };
                    if done.send((id, result)).is_err() {
                        break;
                    }
                }
            });
        if let Err(error) = spawned {
            log::warn!("cannot spawn 3D asset worker: {error}");
            return;
        }
        self.requests = Some(tx);
        self.completions = Some(completions);
    }
}

/// Errors no retry can fix — the manifest has no such id, the response
/// violated a hard cap, or the fetch pipeline itself is down. Anything else
/// (timeouts, resets, HTTP 5xx, disk hiccups) stays retryable.
fn is_permanent(error: &str) -> bool {
    error.contains("no resource entry")
        || error.contains("exceeds size limit")
        || error.contains("URL missing")
        || error.contains("worker unavailable")
        || error.starts_with("3D runtime")
}

async fn bounded_fetch(http: &reqwest::Client, url: &str, limit: usize) -> Result<Vec<u8>> {
    let mut response = http
        .get(url)
        .timeout(FETCH_TIMEOUT)
        .send()
        .await?
        .error_for_status()?;
    anyhow::ensure!(
        response.content_length().is_none_or(|n| n <= limit as u64),
        "3D resource exceeds size limit"
    );
    let mut result = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        anyhow::ensure!(
            result.len().saturating_add(chunk.len()) <= limit,
            "3D resource exceeds size limit"
        );
        result.extend_from_slice(&chunk);
    }
    Ok(result)
}

async fn load_index(
    http: &reqwest::Client,
    base: &str,
    manifest: &str,
    hash: &str,
    cache: &Collection,
) -> Result<FileIndex> {
    // A content hash permits an immutable disk hit. Unversioned manifests
    // are refreshed, with cached data retained only as an offline fallback.
    let version = if hash.is_empty() { "unversioned" } else { hash };
    let cached = cache
        .cached_file_version(manifest, version)
        .and_then(|p| std::fs::read(p).ok());
    if !hash.is_empty() {
        if let Some(bytes) = &cached {
            if let Some(index) = std::str::from_utf8(bytes)
                .ok()
                .and_then(|xml| FileIndex::parse(xml).ok())
            {
                return Ok(index);
            }
        }
    }
    let url = format!("{base}xml/{manifest}.xml?__cv={hash}");
    let bytes = match bounded_fetch(http, &url, 16 * 1024 * 1024).await {
        Ok(bytes) => bytes,
        Err(error) => cached.context(format!("{manifest} download failed: {error}"))?,
    };
    let index = FileIndex::parse(std::str::from_utf8(&bytes)?)?;
    if let Err(error) = cache.store_file_version(manifest, version, &bytes) {
        log::warn!("3D manifest cache: {error}");
    }
    Ok(index)
}

async fn load_resource(
    http: &reqwest::Client,
    base: &str,
    index: &FileIndex,
    cache: &Collection,
    id: &str,
) -> Result<Vec<u8>> {
    let file = index
        .entry(id)
        .with_context(|| format!("no resource entry for {id}"))?;
    let version = if file.hash.is_empty() {
        &file.version
    } else {
        &file.hash
    };
    if let Some(path) = cache.cached_file_version(id, version) {
        let bytes = std::fs::read(path)?;
        if !bytes.is_empty() {
            return Ok(bytes);
        }
    }
    let url = index.url(base, id).context("3D resource URL missing")?;
    let bytes = bounded_fetch(http, &url, 64 * 1024 * 1024).await?;
    anyhow::ensure!(!bytes.is_empty(), "empty 3D resource {id}");
    if let Err(error) = cache.store_file_version(id, version, &bytes) {
        log::warn!("3D resource cache {id}: {error}");
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn collections_keep_namespaces_and_versioned_cache_separate() {
        let main = GeometryCollection::new(
            "//cdn.example/".into(),
            "/spacemap/".into(),
            "abc".into(),
            PathBuf::from("cache"),
            reqwest::Client::new(),
        );
        let particles = GeometryCollection::particles(
            "//cdn.example/".into(),
            "/spacemap/".into(),
            "def".into(),
            PathBuf::from("cache"),
            reqwest::Client::new(),
        );
        assert_eq!(main.base, "https://cdn.example/spacemap/");
        assert_ne!(main.manifest, particles.manifest);
        assert!(
            main.requests.is_none(),
            "2D startup must not start a downloader"
        );
    }
    /// A store wired for direct request/completion injection — no worker
    /// thread, no network.
    fn harness() -> (
        GeometryCollection,
        mpsc::Receiver<String>,
        mpsc::Sender<Completion>,
    ) {
        let mut store = GeometryCollection::new(
            "cdn.example".into(),
            "spacemap".into(),
            "".into(),
            PathBuf::new(),
            reqwest::Client::new(),
        );
        let (req_tx, req_rx) = mpsc::channel();
        store.requests = Some(req_tx);
        let (done_tx, done_rx) = mpsc::channel();
        store.completions = Some(done_rx);
        (store, req_rx, done_tx)
    }

    fn fail(
        store: &mut GeometryCollection,
        done: &mpsc::Sender<Completion>,
        id: &str,
        error: &str,
    ) {
        done.send((id.to_owned(), Err(error.to_owned()))).unwrap();
        store.pump();
    }

    fn elapse_backoff(store: &mut GeometryCollection, id: &str) {
        store.retries.get_mut(id).unwrap().next_at = Instant::now() - Duration::from_secs(1);
    }

    #[test]
    fn pump_reports_success_and_failure_without_blocking() {
        let (mut store, _req, done) = harness();
        done.send(("mesh".into(), Ok(Arc::from(&b"AWD"[..]))))
            .unwrap();
        done.send((
            "missing".into(),
            Err("no resource entry for missing".into()),
        ))
        .unwrap();
        store.pump();
        assert_eq!(store.get("mesh").unwrap().as_ref(), b"AWD");
        assert_eq!(
            store.failure("missing"),
            Some("no resource entry for missing")
        );
        assert!(store.get("missing").is_none());
    }

    #[test]
    fn transient_failures_retry_with_backoff_then_fail_permanently() {
        let (mut store, req, done) = harness();
        store.request("sibelon_diffuse_512");
        assert_eq!(req.recv().unwrap(), "sibelon_diffuse_512");

        // Each transient failure schedules one retry; the request-side
        // dedup holds until the backoff elapses.
        for delay in RETRY_DELAYS {
            fail(
                &mut store,
                &done,
                "sibelon_diffuse_512",
                "operation timed out",
            );
            assert!(
                store.failure("sibelon_diffuse_512").is_none(),
                "still retriable"
            );
            store.request("sibelon_diffuse_512");
            assert!(
                req.try_recv().is_err(),
                "backoff {delay:?} must hold re-requests"
            );
            elapse_backoff(&mut store, "sibelon_diffuse_512");
            store.request("sibelon_diffuse_512");
            assert_eq!(req.recv().unwrap(), "sibelon_diffuse_512");
        }
        // After `RETRY_DELAYS` retries the failure is permanent.
        fail(
            &mut store,
            &done,
            "sibelon_diffuse_512",
            "operation timed out",
        );
        assert_eq!(
            store.failure("sibelon_diffuse_512"),
            Some("operation timed out")
        );
        store.request("sibelon_diffuse_512");
        assert!(req.try_recv().is_err(), "permanent failures never re-send");
    }

    #[test]
    fn succeeded_retry_clears_the_retry_entry() {
        let (mut store, req, done) = harness();
        store.request("tex");
        req.recv().unwrap();
        fail(&mut store, &done, "tex", "connection reset");
        elapse_backoff(&mut store, "tex");
        store.request("tex");
        req.recv().unwrap();
        done.send(("tex".into(), Ok(Arc::from(&b"ATF"[..]))))
            .unwrap();
        store.pump();
        assert_eq!(store.get("tex").unwrap().as_ref(), b"ATF");
        assert!(store.retries.is_empty());
        store.request("tex");
        assert!(req.try_recv().is_err(), "ready ids dedupe");
    }

    #[test]
    fn manifest_absent_ids_fail_permanently_without_retry() {
        let (mut store, req, _done) = harness();
        store.request("bogus_diffuse_256");
        req.recv().unwrap();
        fail(
            &mut store,
            &_done,
            "bogus_diffuse_256",
            "no resource entry for bogus_diffuse_256",
        );
        assert!(store.failure("bogus_diffuse_256").is_some());
        assert!(store.retries.is_empty());
        store.request("bogus_diffuse_256");
        assert!(req.try_recv().is_err());
    }

    #[test]
    fn versioned_assets_load_offline_without_network() {
        let root = std::env::temp_dir().join(format!("xenomit-3d-cache-{}", std::process::id()));
        let cache = Collection::new(root.clone());
        cache
            .store_file_version("mesh", "new", b"AWD-current")
            .unwrap();
        cache
            .store_file_version("mesh", "old", b"AWD-stale")
            .unwrap();
        let index = FileIndex::parse("<resources><location id='mesh' path='3d/'/><file id='mesh' location='mesh' name='ship' type='awd' hash='new'/></resources>").unwrap();
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let bytes = rt
            .block_on(load_resource(
                &reqwest::Client::new(),
                "http://127.0.0.1:1/",
                &index,
                &cache,
                "mesh",
            ))
            .unwrap();
        assert_eq!(bytes, b"AWD-current");
        let missing = rt
            .block_on(load_resource(
                &reqwest::Client::new(),
                "http://127.0.0.1:1/",
                &index,
                &cache,
                "absent",
            ))
            .unwrap_err();
        assert!(missing.to_string().contains("no resource entry"));
        let _ = std::fs::remove_dir_all(root);
    }

    /// `AssetManager` over an injected particle collection — no worker,
    /// no network; completions are fed by hand.
    fn particle_manager(store: GeometryCollection) -> crate::assets::AssetManager {
        let mut assets = crate::assets::AssetManager::new(
            PathBuf::new(),
            FileIndex::default(),
            crate::assets::GameXml::default(),
            crate::assets::UberAssets::default(),
            crate::assets::mapxml::MapDb::default(),
        );
        assets.particles_3d = Some(store);
        assets
    }

    /// `update_particle_group` fetches `id.type` keys: the archive is
    /// `{id}.zip`; anim dependencies remap `.png -> .atf` and
    /// `.obj -> .awd` (`ParticlesAssetLoaderContext.getRemappedUrl`).
    #[test]
    fn particle_group_requests_typed_ids() {
        let (store, req, done) = harness();
        let mut assets = particle_manager(store);

        assets.update_particle_group("box_2");
        assert_eq!(req.recv().unwrap(), "box_2.zip");

        // One anim with a texture and an external mesh (bare JSON — the
        // zip wrapper is optional for `ParticleGroup::from_bytes`).
        let group = br#"{"animationDatas":[{"property":{"data":{}},"data":{
            "geometry":{"data":{"assembler":{"data":{
                "shape":{"id":"ExternalShapeSubParser","data":{"url":"fx_loot.obj"}}}}}},
            "material":{"data":{"url":"loot_glow.png"}}}}]}"#;
        done.send(("box_2.zip".into(), Ok(Arc::from(&group[..]))))
            .unwrap();
        assets.particles_3d.as_mut().unwrap().pump();
        assets.update_particle_group("box_2");
        assert_eq!(req.recv().unwrap(), "loot_glow.atf");
        assert_eq!(req.recv().unwrap(), "fx_loot.awd");
        assert!(assets.particle_group("box_2").is_some());
    }

    /// A permanently-failed `{id}.zip` resolves the group to `None` and
    /// is never re-requested.
    #[test]
    fn particle_group_zip_failure_resolves_empty() {
        let (store, req, done) = harness();
        let mut assets = particle_manager(store);

        assets.update_particle_group("bad");
        assert_eq!(req.recv().unwrap(), "bad.zip");
        done.send((
            "bad.zip".into(),
            Err("no resource entry for bad.zip".into()),
        ))
        .unwrap();
        assets.particles_3d.as_mut().unwrap().pump();
        assets.update_particle_group("bad");
        assets.update_particle_group("bad");
        assert!(assets.particle_group("bad").is_none());
        assert!(req.try_recv().is_err(), "resolved groups never re-send");
    }
}
