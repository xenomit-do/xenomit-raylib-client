//! `LensFlaresLayer` + `LensFlareView` — parallax sun flares over the
//! world (above the effects layer, under the HUD).
//!
//! Two providers like stock: the maps-config `<lensflares>` rows
//! (`Background2D`, parallax anchor + `pFactor`) and the authored
//! `display3D>lensFlare` scene rows (`Background3D`, projected through
//! the 3D camera). The 3D list wins in the perspective view when the
//! scene provides flares, matching
//! `provider = background3D.isAvailableForCurrentMap ? 3D : 2D`.
//!
//! Each `LensFlareView` is the `lensflare{typeID}` file's `lens0..N`
//! children: `lens0` at the flare's screen position, the rest fanning
//! out at `i * -(screen - center) * 3 / N` per `updateLensPositions`.
//! A `star` clip rotates −0.15°/frame; the `lensFlash` bitmap flashes
//! once per becoming-visible (0→0.75 over 0.2 s, then out over 3 s).

use crate::assets::{AssetManager, SpriteId};
use std::time::Instant;

/// Upper bound of `lens{i}` children requested per flare file.
const MAX_LENSES: usize = 8;

/// `TweenMax.to(layer,0.1,{alpha:1})` fade-in seconds.
const FADE_IN: f32 = 0.1;
/// `lensFlash` ramp to 0.75, then the 3 s fade-out (`flashLens`).
const FLASH_RAMP: f32 = 0.2;
const FLASH_OUT: f32 = 3.0;
const FLASH_PEAK: f32 = 0.75;

/// One map flare — the `LensFlareVO` + view bookkeeping.
pub struct LensFlare {
    /// VO `id`.
    pub id: i32,
    /// VO `typeID` → `lensflare{typeID}` file.
    pub type_id: i32,
    /// 2D provider: parallax anchor (`@(x,y)` + `@pFactor`).
    pub anchor: (f32, f32),
    pub p_factor: f32,
    /// 3D provider: authored scene point in Away3D space
    /// (`LensFlareVO(x, -z, -y)`); `Some` only on `flares3d` rows.
    pub scene_pos: Option<(f32, f32, f32)>,
    /// VO `isStar` — the rotating `star` clip at the flare position.
    pub star: bool,
    /// VO `blendMode == "add"` → additive children.
    pub additive: bool,
    /// `lens{i}` sprites in child order (lazy — requested while live).
    pub lenses: Vec<SpriteId>,
    pub star_sprite: Option<SpriteId>,
    /// false→true transition stamp (the 0.1 s fade-in restarts).
    pub fade_at: Instant,
    /// Currently drawn — the flash only re-fires on a fresh show.
    pub visible: bool,
    /// `flashLens` start (once per becoming-visible).
    pub flash_at: Option<Instant>,
    /// `starMC` rotation in degrees (−0.15/frame).
    pub star_rot: f32,
}

/// Per-map flare state — rebuilt on map change.
#[derive(Default)]
pub struct LensFlares {
    map_id: i32,
    /// maps-config provider list (`Background2D.lensFlares`).
    pub flares2d: Vec<LensFlare>,
    /// `display3D` provider list (`Background3D._lensFlares`) — used in
    /// the perspective view when non-empty.
    pub flares3d: Vec<LensFlare>,
    /// A `hideLensflare` tilemap occludes the whole scene's flares
    /// (no per-pixel mask — documented approximation).
    pub hide3d: bool,
    /// Shared `lensFlash` bitmap sprite.
    pub flash: Option<SpriteId>,
}

impl LensFlares {
    /// Rebuild on map change; request the active provider's sprites.
    /// `perspective` selects the provider like `MapView3D`'s
    /// `isAvailableForCurrentMap` check does.
    pub fn update(&mut self, map_id: i32, perspective: bool, dt: f32, assets: &mut AssetManager) {
        if self.map_id != map_id {
            *self = LensFlares {
                map_id,
                flares2d: assets
                    .maps
                    .get(map_id)
                    .map(|d| {
                        d.lensflares
                            .iter()
                            .map(LensFlare::new2d)
                            .collect()
                    })
                    .unwrap_or_default(),
                flares3d: assets
                    .map_scene3d_for_map(map_id)
                    .map(|scene| {
                        scene
                            .lensflares
                            .iter()
                            .map(LensFlare::new3d)
                            .collect()
                    })
                    .unwrap_or_default(),
                hide3d: assets
                    .map_scene3d_for_map(map_id)
                    .is_some_and(|s| s.hides_lensflares()),
                flash: None,
            };
        }
        for flare in self.active_mut(perspective) {
            flare.star_rot -= 0.15 * dt * 60.0;
            flare.request(assets);
        }
        if !self.active(perspective).is_empty() && self.flash.is_none() {
            self.flash = Some(assets.request("lensFlash", "lensFlash"));
        }
    }

    /// The provider list the current view draws (`MapView3D` prefers
    /// the 3D background's flares when the scene provides them).
    pub fn active(&self, perspective: bool) -> &[LensFlare] {
        if perspective && !self.flares3d.is_empty() {
            &self.flares3d
        } else {
            &self.flares2d
        }
    }

    pub fn active_mut(&mut self, perspective: bool) -> &mut Vec<LensFlare> {
        if perspective && !self.flares3d.is_empty() {
            &mut self.flares3d
        } else {
            &mut self.flares2d
        }
    }

    /// Mark a flare shown/hidden (`LensFlareView.setVisible`): 0.1 s
    /// fade-in on a fresh show, instant hide; the flash re-fires once
    /// per becoming-visible.
    pub fn set_visible(flare: &mut LensFlare, on: bool, now: Instant) {
        if on && !flare.visible {
            flare.fade_at = now;
            flare.flash_at = Some(now);
        }
        flare.visible = on;
    }
}

impl LensFlare {
    fn new2d(f: &crate::assets::mapxml::MapLensflare) -> Self {
        Self {
            id: f.id,
            type_id: f.type_id,
            anchor: f.pos,
            p_factor: f.p_factor,
            scene_pos: None,
            star: f.star,
            additive: f.blend == "add",
            lenses: Vec::new(),
            star_sprite: None,
            fade_at: Instant::now(),
            visible: false,
            flash_at: None,
            star_rot: 0.0,
        }
    }

    fn new3d(f: &crate::assets::SceneLensFlare3D) -> Self {
        Self {
            id: f.id,
            type_id: f.type_id,
            anchor: (0.0, 0.0),
            p_factor: 0.0,
            scene_pos: Some(f.position),
            star: f.star,
            additive: f.blend == "add",
            lenses: Vec::new(),
            star_sprite: None,
            fade_at: Instant::now(),
            visible: false,
            flash_at: None,
            star_rot: 0.0,
        }
    }

    /// `AssetLibrary.loadFinisher("lensflare"+typeID)` — request the
    /// `lens{i}` children + `star`. Requested ids that don't resolve
    /// stay empty clips and are skipped at draw time.
    fn request(&mut self, assets: &mut AssetManager) {
        if self.lenses.is_empty() {
            let file = format!("lensflare{}", self.type_id);
            self.lenses = (0..MAX_LENSES)
                .map(|i| assets.request(&file, &format!("lens{i}")))
                .collect();
            if self.star {
                self.star_sprite = Some(assets.request(&file, "star"));
            }
        }
    }
}

/// `LensFlareView.updateLensPositions` — child `i` sits at
/// `pos + i * (-(pos - center) * 3 / N)`: the chain fans out opposite
/// the screen center, three times the center distance across `N` lenses.
pub fn lens_step(screen: (f32, f32), viewport: (f32, f32), n: usize) -> (f32, f32) {
    let n = n.max(1) as f32;
    (
        -(screen.0 - viewport.0 * 0.5) * 3.0 / n,
        -(screen.1 - viewport.1 * 0.5) * 3.0 / n,
    )
}

/// `TweenMax.to(layer,0.1,{alpha:1})` — linear 0.1 s fade-in.
pub fn fade_alpha(age: f32) -> f32 {
    (age / FADE_IN).clamp(0.0, 1.0)
}

/// `flashLens` — alpha 0→`FLASH_PEAK` over 0.2 s, then out over 3 s.
pub fn flash_alpha(age: f32) -> f32 {
    if age < 0.0 {
        return 0.0;
    }
    if age < FLASH_RAMP {
        FLASH_PEAK * age / FLASH_RAMP
    } else {
        FLASH_PEAK * (1.0 - (age - FLASH_RAMP) / FLASH_OUT).max(0.0)
    }
}

/// 2D provider screen position — `Background2D.calculateScreenPosition`:
/// the planet parallax math (`(pos - cam) / pFactor` around the
/// viewport centre; `pFactor == 0` pins the point to the screen).
pub fn screen_pos_2d(
    anchor: (f32, f32),
    p_factor: f32,
    cam_center: (f32, f32),
    viewport: (f32, f32),
    zoom: f32,
) -> (f32, f32) {
    let inv = if p_factor == 0.0 {
        0.0
    } else {
        1.0 / p_factor
    };
    (
        viewport.0 * 0.5 + (anchor.0 - cam_center.0) * inv * zoom,
        viewport.1 * 0.5 + (anchor.1 - cam_center.1) * inv * zoom,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lens_step_fans_away_from_center() {
        // Flare right of center → children step back left (toward the
        // center), 3× the offset split across the chain.
        let (sx, sy) = lens_step((700.0, 300.0), (800.0, 600.0), 6);
        assert!((sx + 150.0).abs() < 0.001 && sy.abs() < 0.001);
        let (sx, sy) = lens_step((100.0, 100.0), (800.0, 600.0), 3);
        assert!((sx - 300.0).abs() < 0.001 && (sy - 200.0).abs() < 0.001);
    }

    #[test]
    fn fade_is_linear_over_point_one_seconds() {
        assert_eq!(fade_alpha(0.0), 0.0);
        assert!((fade_alpha(0.05) - 0.5).abs() < 1e-6);
        assert_eq!(fade_alpha(1.0), 1.0);
    }

    #[test]
    fn flash_ramps_then_decays() {
        assert_eq!(flash_alpha(0.0), 0.0);
        assert!((flash_alpha(0.2) - 0.75).abs() < 1e-6);
        assert!((flash_alpha(0.2 + 1.5) - 0.375).abs() < 1e-3);
        assert_eq!(flash_alpha(3.3), 0.0);
    }

    #[test]
    fn screen_pos_2d_parallax() {
        // pFactor 10: 1000 world units of camera travel shift the flare
        // by 100 px. pFactor 0 pins it.
        let p = screen_pos_2d((1000.0, 500.0), 10.0, (2000.0, 500.0), (800.0, 600.0), 1.0);
        assert!((p.0 - 300.0).abs() < 0.001 && (p.1 - 300.0).abs() < 0.001);
        let p = screen_pos_2d((100.0, 100.0), 0.0, (9999.0, 9999.0), (800.0, 600.0), 1.0);
        assert!((p.0 - 400.0).abs() < 0.001 && (p.1 - 300.0).abs() < 0.001);
    }
}
