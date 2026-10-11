//! Game audio — the public queue + the render-thread device.
//!
//! Gameplay/net threads enqueue [`SoundRequest`]s via [`play_sfx`] /
//! [`play_sfx_at`] / [`play_sound`]; the [`AudioDevice`] on the render
//! thread drains them once per frame, resolves the sound pattern from
//! `game.xml` (`GameXml::audio`) and plays the matching `DefineSound`
//! out of the soundbank SWF.
//!
//! - `play_sound(id)` for a specific `game.xml` sound id (`soundID`
//!   attributes on laser/pyro patterns, `InGameCatalog.SOUND_ID_*`).
//! - `play_sfx(sfx)` keeps the coarse enum, mapped to catalog ids.
//! - `at` carries the world position for the stock positional mix
//!   (800-unit radius, linear falloff, pan `(x-hero.x)/500`).
//!
//! Looping/ambient channels (`playLoop`, engine and NPC ambient
//! `soundIDs`) are intentionally not driven yet — TODO: stock starts
//! those from `Engines2D`/`updateLoopChannels`.
//!
//! Not thread-affine: everything here is a plain `Mutex` queue.

use std::sync::Mutex;
use std::sync::OnceLock;

mod device;
mod loader;
pub mod music;

pub use device::AudioDevice;

/// Coarse effect id used by the pump thread and gameplay code where no
/// specific `soundID` pattern applies.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Sfx {
    Explosion,
    Laser,
    Rocket,
    Impact,
    Collect,
    Spawn,
    Smartbomb,
    Emp,
    Mine,
    PortalJump,
    PortalDone,
}

impl Sfx {
    /// `InGameCatalog.SOUND_ID_*` fallback (pattern `soundID`s win where
    /// the caller has one — the game.xml sound table is what actually
    /// resolves a bank + class).
    pub fn sound_id(self) -> i32 {
        match self {
            // damageSounds explosion1_*
            Sfx::Explosion => 12,
            // combatSounds laser0_*
            Sfx::Laser => 0,
            // combatSounds rocket1_0
            Sfx::Rocket => 2,
            // damageSounds rocketDamage0_*
            Sfx::Impact => 7,
            // uiSounds select0 — closest stock feedback for a pickup
            Sfx::Collect => 24,
            // pirateSounds "spawn" (only catalogued spawn id)
            Sfx::Spawn => 79,
            // damageSounds smartbomb0 — SOUND_ID_SMARTBOMB_0
            Sfx::Smartbomb => 30,
            // damageSounds emp — SOUND_ID_EMP
            Sfx::Emp => 43,
            // mines explode with the pyro pattern's soundID; generic
            // fallback is the same ship-explosion bank slot
            Sfx::Mine => 12,
            Sfx::PortalJump => 5,
            Sfx::PortalDone => 6,
        }
    }
}

/// One queued sound request — a `game.xml` `sounds` id + optional world
/// position (`at` drives the stock positional mix).
#[derive(Clone, Copy, Debug)]
pub struct SoundRequest {
    pub id: i32,
    pub at: Option<(f32, f32)>,
}

/// `Settings` audio panel state (persisted client-side, defaults match
/// `Settings.as`: playMusic/playSFX/playVoice/playCombatMusic on, all
/// volumes 50). Read by the device every frame.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct AudioSettings {
    /// `Settings.playSFX`
    pub play_sfx: bool,
    /// `Settings.playMusic`
    pub play_music: bool,
    /// `Settings.playVoice`
    pub play_voice: bool,
    /// `Settings.playCombatMusic`
    pub play_combat_music: bool,
    /// `Settings.soundVolume` (0..100)
    pub sound_volume: i32,
    /// `Settings.musicVolume` (0..100)
    pub music_volume: i32,
    /// `Settings.voiceVolume` (0..100)
    pub voice_volume: i32,
}

impl Default for AudioSettings {
    fn default() -> Self {
        Self {
            play_sfx: true,
            play_music: true,
            play_combat_music: true,
            play_voice: true,
            sound_volume: 50,
            music_volume: 50,
            voice_volume: 50,
        }
    }
}

struct Queue {
    requests: Vec<SoundRequest>,
    muted: bool,
}

fn state() -> &'static Mutex<Queue> {
    static STATE: OnceLock<Mutex<Queue>> = OnceLock::new();
    STATE.get_or_init(|| {
        Mutex::new(Queue {
            requests: Vec::new(),
            muted: false,
        })
    })
}

/// Play `game.xml` sound `id` (positionless — full volume, centered).
/// Unknown/missing patterns are a silent no-op; positional requests
/// outside the 800-unit radius are dropped.
pub fn play_sound(id: i32, at: Option<(f32, f32)>) {
    enqueue(SoundRequest { id, at });
}

/// `play_sound(id, None)` shorthand.
pub fn play_sfx(sfx: Sfx) {
    play_sound(sfx.sound_id(), None);
}

/// Positional variant used by world-space effects.
pub fn play_sfx_at(sfx: Sfx, x: f32, y: f32) {
    play_sound(sfx.sound_id(), Some((x, y)));
}

/// Global mute (`Settings`-level kill switch, applied as master volume).
pub fn set_muted(muted: bool) {
    if let Ok(mut s) = state().lock() {
        s.muted = muted;
    }
}

fn enqueue(req: SoundRequest) {
    if let Ok(mut s) = state().lock() {
        // Drop requests while muted — they must not burst through on
        // unmute.
        if s.muted {
            return;
        }
        // Cap the backlog: nobody drains during sign-in/load screens.
        if s.requests.len() < 64 {
            s.requests.push(req);
        }
    }
}

/// Render-thread drain (called by `AudioDevice::tick`).
pub(crate) fn drain() -> (Vec<SoundRequest>, bool) {
    match state().lock() {
        Ok(mut s) => (std::mem::take(&mut s.requests), s.muted),
        Err(_) => (Vec::new(), false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Single test — the queue and mute flag are global, so separate
    /// tests would race each other.
    #[test]
    fn queue_drains_in_order_and_mute_drops() {
        play_sound(5, None);
        play_sfx(Sfx::Laser);
        play_sfx_at(Sfx::Explosion, 100.0, -50.0);
        let (reqs, _) = drain();
        let tail = &reqs[reqs.len().saturating_sub(3)..];
        assert_eq!(tail[0].id, 5);
        assert_eq!(tail[1].id, 0);
        assert_eq!(tail[2].id, 12);
        assert_eq!(tail[2].at, Some((100.0, -50.0)));

        set_muted(true);
        play_sfx(Sfx::Emp);
        set_muted(false);
        let (reqs, _) = drain();
        assert!(!reqs.iter().any(|r| r.id == Sfx::Emp.sound_id()));
    }
}
