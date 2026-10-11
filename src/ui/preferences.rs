//! Small client-local UI preferences that are independent of game settings.

use crate::assets::uberlib::TextureQuality;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Per-component rendering mode preference. `true` selects the native 3D
/// asset pipeline for that component when the game data supplies a 3D recipe.
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(default)]
pub struct RenderPreferences {
    pub camera_3d: bool,
    pub collectables_3d: bool,
    pub npcs_3d: bool,
    pub ships_3d: bool,
    pub drones_3d: bool,
    pub backgrounds_3d: bool,
    pub portals_3d: bool,
    pub effects_3d: bool,
    pub explosions_3d: bool,
    /// Client-local 3D texture quality tier (Low/Medium/High).
    pub texture_quality: TextureQuality,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WindowSkinChoice {
    /// The stock `SimpleWindow` chrome — `window` SWF `pattern` +
    /// scale9 `window` sprite + real `closeBtn`/`minimizeBtn` art.
    #[default]
    Stock,
    /// Flat `questWin` scale9 export (newer Spark-era window look).
    ModernBlue,
    /// `main.swf` `DOWindowSkin` export.
    Legacy,
}

impl WindowSkinChoice {
    pub fn label(self) -> &'static str {
        match self {
            Self::Stock => "Window skin: Classic",
            Self::ModernBlue => "Window skin: Modern blue",
            Self::Legacy => "Window skin: Legacy",
        }
    }

    pub fn toggle(self) -> Self {
        match self {
            Self::Stock => Self::ModernBlue,
            Self::ModernBlue => Self::Legacy,
            Self::Legacy => Self::Stock,
        }
    }
}

#[derive(Deserialize, Serialize)]
struct PreferencesFile {
    #[serde(default = "version")]
    version: u32,
    #[serde(default)]
    window_skin: WindowSkinChoice,
    #[serde(default = "default_true")]
    minimap_map_image: bool,
    #[serde(default = "default_true")]
    toolbar_background: bool,
    #[serde(default = "default_mobile_hud")]
    mobile_hud: bool,
    #[serde(default)]
    render: RenderPreferences,
    /// `Settings.as` audio toggles/volumes (`playSFX`/`playMusic`/
    /// `playVoice`/`playCombatMusic` + 0-100 volume triplet).
    #[serde(default)]
    audio: crate::audio::AudioSettings,
}

fn version() -> u32 {
    1
}

fn default_true() -> bool {
    true
}

fn default_mobile_hud() -> bool {
    cfg!(target_os = "android")
}

pub struct UiPreferences {
    pub window_skin: WindowSkinChoice,
    /// Use the current map's game-provided minimap image where available.
    pub minimap_map_image: bool,
    /// Use game-authored menu button artwork in the top toolbar.
    pub toolbar_background: bool,
    /// Use a compact touch-first HUD layout.
    pub mobile_hud: bool,
    /// Independent 2D/3D rendering choices for map components.
    pub render: RenderPreferences,
    /// Audio toggles + volumes (stock `Settings` defaults).
    pub audio: crate::audio::AudioSettings,
    path: Option<PathBuf>,
    dirty: bool,
}

impl Default for UiPreferences {
    fn default() -> Self {
        Self {
            window_skin: WindowSkinChoice::default(),
            minimap_map_image: true,
            toolbar_background: true,
            mobile_hud: cfg!(target_os = "android"),
            render: RenderPreferences::default(),
            audio: crate::audio::AudioSettings::default(),
            path: None,
            dirty: false,
        }
    }
}

impl UiPreferences {
    pub fn set_path(&mut self, path: PathBuf) {
        self.path = Some(path.clone());
        if let Some(prefs) = std::fs::read_to_string(&path)
            .ok()
            .and_then(|text| serde_json::from_str::<PreferencesFile>(&text).ok())
        {
            self.window_skin = prefs.window_skin;
            self.minimap_map_image = prefs.minimap_map_image;
            self.toolbar_background = prefs.toolbar_background;
            self.mobile_hud = prefs.mobile_hud;
            self.render = prefs.render;
            self.audio = prefs.audio;
        }
    }

    pub fn set_audio(&mut self, audio: crate::audio::AudioSettings) {
        if self.audio != audio {
            self.audio = audio;
            self.dirty = true;
        }
    }

    pub fn select_window_skin(&mut self, choice: WindowSkinChoice) {
        if self.window_skin != choice {
            self.window_skin = choice;
            self.dirty = true;
        }
    }

    pub fn toggle_minimap_map_image(&mut self) {
        self.minimap_map_image = !self.minimap_map_image;
        self.dirty = true;
    }

    pub fn toggle_toolbar_background(&mut self) {
        self.toolbar_background = !self.toolbar_background;
        self.dirty = true;
    }

    pub fn set_render_preferences(&mut self, render: RenderPreferences) {
        if self.render != render {
            self.render = render;
            self.dirty = true;
        }
    }

    pub fn set_mobile_hud(&mut self, enabled: bool) {
        if self.mobile_hud != enabled {
            self.mobile_hud = enabled;
            self.dirty = true;
        }
    }

    pub fn toggle_mobile_hud(&mut self) {
        self.set_mobile_hud(!self.mobile_hud);
    }

    pub fn flush(&mut self) {
        if !self.dirty {
            return;
        }
        let Some(path) = self.path.as_ref() else {
            return;
        };
        let prefs = PreferencesFile {
            version: 1,
            window_skin: self.window_skin,
            minimap_map_image: self.minimap_map_image,
            toolbar_background: self.toolbar_background,
            mobile_hud: self.mobile_hud,
            render: self.render,
            audio: self.audio,
        };
        match write_atomic(&prefs, path) {
            Ok(()) => self.dirty = false,
            Err(e) => log::warn!("could not save UI preferences {}: {e}", path.display()),
        }
    }
}

fn write_atomic(prefs: &PreferencesFile, path: &Path) -> std::io::Result<()> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    let text = serde_json::to_vec_pretty(prefs)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    let mut temp = path.as_os_str().to_owned();
    temp.push(".tmp");
    let temp = PathBuf::from(temp);
    std::fs::write(&temp, text)?;
    if let Err(e) = std::fs::rename(&temp, path) {
        let _ = std::fs::remove_file(&temp);
        return Err(e);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_skin_choice_persists() {
        let root = std::env::temp_dir().join(format!("xenomit-ui-prefs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let path = root.join("ui_settings.json");
        let mut prefs = UiPreferences::default();
        prefs.set_path(path.clone());
        prefs.select_window_skin(WindowSkinChoice::Legacy);
        let mut render = RenderPreferences::default();
        render.ships_3d = true;
        render.backgrounds_3d = true;
        render.texture_quality = TextureQuality::Low;
        prefs.set_render_preferences(render);
        prefs.set_mobile_hud(true);
        prefs.flush();

        let mut loaded = UiPreferences::default();
        loaded.set_path(path);
        assert_eq!(loaded.window_skin, WindowSkinChoice::Legacy);
        assert_eq!(loaded.render, render);
        assert_eq!(loaded.render.texture_quality, TextureQuality::Low);
        assert!(loaded.mobile_hud);
        loaded.select_window_skin(WindowSkinChoice::ModernBlue);
        loaded.flush();
        let mut overwritten = UiPreferences::default();
        overwritten.set_path(root.join("ui_settings.json"));
        assert_eq!(overwritten.window_skin, WindowSkinChoice::ModernBlue);
        let _ = std::fs::remove_dir_all(root);
    }
}
