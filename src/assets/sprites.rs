//! CPU-side sprite clips + the lazy `(srcKey,resKey) -> frames` store.

use std::collections::HashMap;

/// One CPU-side frame.
#[derive(Clone, Debug)]
pub struct SpriteFrame {
    pub image: image::RgbaImage,
    /// Draw offset relative to the clip origin (pixels).
    pub offset: (f32, f32),
    /// Frame duration in ms (animated clips; rotation strips use 0).
    pub duration_ms: u32,
}

/// A rasterized clip — what `SpriteId` resolves to.
#[derive(Clone, Debug, Default)]
pub struct SpriteClip {
    pub frames: Vec<SpriteFrame>,
    /// True for rotation strips (32 frames, indexed by heading).
    pub rotation_strip: bool,
    /// True when `frames` are the atlas tiles of a `StarlingTiledMap`
    /// tile map (one frame per `SubTexture`, atlas-name order).
    pub tiles: bool,
    /// Authored `DefineScalingGrid` splitter rect `(x, y, w, h)` in sprite
    /// px — scale9 skins (the `window` chrome) keep their edges fixed.
    pub scale9: Option<(f32, f32, f32, f32)>,
}

#[derive(Clone, Copy, Debug, Hash, Eq, PartialEq)]
pub struct SpriteId(pub u64);

/// Lazy sprite store: `ensure(src,res)` dedupes, `insert` fills after
/// extraction, `get` reads.
#[derive(Default)]
pub struct SpriteStore {
    by_key: HashMap<(String, String), SpriteId>,
    clips: HashMap<SpriteId, SpriteClip>,
    next: u64,
}

impl SpriteStore {
    pub fn ensure(&mut self, src_key: &str, res_key: &str) -> SpriteId {
        if let Some(id) = self.by_key.get(&(src_key.to_string(), res_key.to_string())) {
            return *id;
        }
        self.next += 1;
        let id = SpriteId(self.next);
        self.by_key
            .insert((src_key.to_string(), res_key.to_string()), id);
        id
    }

    pub fn insert(&mut self, id: SpriteId, clip: SpriteClip) {
        self.clips.insert(id, clip);
    }

    pub fn get(&self, id: SpriteId) -> Option<&SpriteClip> {
        self.clips.get(&id)
    }

    /// Enumerate every stored clip (extracted or empty). The GPU cache
    /// iterates this to upload new frames.
    pub fn iter(&self) -> impl Iterator<Item = (SpriteId, &SpriteClip)> + '_ {
        self.clips.iter().map(|(id, clip)| (*id, clip))
    }

    /// True if `id` has been resolved (extraction finished, ok or failed).
    pub fn resolved(&self, id: SpriteId) -> bool {
        self.clips.contains_key(&id)
    }
}
