//! CDN download + on-disk cache helpers.
//!
//! Mirrors `FileCollection.load` + `SWFFinisher`/`ImageFinisher`: a file id
//! resolves to a URL; completed bytes land under `{cache_dir}/files/` and
//! are served from disk on later runs.

use std::path::{Path, PathBuf};

/// Cache layout: `{cache_dir}/files/{file_id}__{content_version}` (raw
/// download), `{cache_dir}/sprites/{srcKey}/{resKey}/{cache_key}/` PNG
/// frames + metadata. Processed clips are versioned independently from
/// the running process so a changed resource or extractor invalidates only
/// the affected cache entry.
pub struct Collection {
    pub cache_dir: PathBuf,
}

/// File ids become filenames — forbid path separators/up-levels so an
/// id like `../x` can never escape `files/`.
fn safe_name(file_id: &str) -> String {
    file_id
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            _ => c,
        })
        .collect()
}

impl Collection {
    pub fn new(cache_dir: PathBuf) -> Self {
        Self { cache_dir }
    }

    /// `{cache_dir}/files/` — raw downloads, created on demand by callers.
    pub fn files_dir(&self) -> PathBuf {
        self.cache_dir.join("files")
    }

    /// `{cache_dir}/sprites/` — decoded frame cache.
    pub fn sprites_dir(&self) -> PathBuf {
        self.cache_dir.join("sprites")
    }

    /// Local path of a downloaded file, if cached (`files/{file_id}`).
    pub fn cached_file(&self, file_id: &str) -> Option<PathBuf> {
        if file_id.is_empty() {
            return None;
        }
        let path = self.files_dir().join(safe_name(file_id));
        path.is_file().then_some(path)
    }

    /// Target path for a download (parent dir is NOT created here —
    /// `store_file` does that).
    pub fn file_path(&self, file_id: &str) -> PathBuf {
        self.files_dir().join(safe_name(file_id))
    }

    pub fn versioned_file_path(&self, file_id: &str, version: &str) -> PathBuf {
        self.files_dir()
            .join(format!("{}__{}", safe_name(file_id), safe_name(version)))
    }

    pub fn cached_file_version(&self, file_id: &str, version: &str) -> Option<PathBuf> {
        let path = self.versioned_file_path(file_id, version);
        path.is_file().then_some(path)
    }

    /// Persist downloaded bytes for `file_id`.
    pub fn store_file(&self, file_id: &str, bytes: &[u8]) -> std::io::Result<PathBuf> {
        std::fs::create_dir_all(self.files_dir())?;
        let path = self.file_path(file_id);
        std::fs::write(&path, bytes)?;
        Ok(path)
    }

    pub fn store_file_version(
        &self,
        file_id: &str,
        version: &str,
        bytes: &[u8],
    ) -> std::io::Result<PathBuf> {
        std::fs::create_dir_all(self.files_dir())?;
        let path = self.versioned_file_path(file_id, version);
        std::fs::write(&path, bytes)?;
        Ok(path)
    }

    /// Decoded-frame cache dir for `(src,res)`:
    /// `{cache_dir}/sprites/{srcKey}/{resKey}/`. `res` is the effective
    /// symbol name (empty res already mapped to `mc`/file by the caller).
    pub fn sprite_dir(&self, src_key: &str, res_key: &str) -> PathBuf {
        self.sprites_dir()
            .join(safe_name(src_key))
            .join(safe_name(if res_key.is_empty() {
                "_file"
            } else {
                res_key
            }))
    }

    pub fn versioned_sprite_dir(&self, src_key: &str, res_key: &str, cache_key: &str) -> PathBuf {
        self.sprite_dir(src_key, res_key).join(safe_name(cache_key))
    }

    /// Read a sprite frame-cache dir if present (`f*.png` + meta.json).
    pub fn cached_sprite_dir(&self, src_key: &str, res_key: &str) -> Option<PathBuf> {
        let dir = self.sprite_dir(src_key, res_key);
        dir.is_dir().then_some(dir)
    }

    /// Ensure the sprite-cache dir exists and return it.
    pub fn ensure_sprite_dir(&self, src_key: &str, res_key: &str) -> std::io::Result<PathBuf> {
        let dir = self.sprite_dir(src_key, res_key);
        std::fs::create_dir_all(&dir)?;
        Ok(dir)
    }
}

impl AsRef<Path> for Collection {
    fn as_ref(&self) -> &Path {
        &self.cache_dir
    }
}
