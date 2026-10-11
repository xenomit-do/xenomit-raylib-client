//! Render-thread audio device (`AudioManager` port).
//!
//! Raylib `Sound`/`Music` handles are not `Send`, so every handle is
//! created and owned here. [`AudioDevice::tick`] runs once per frame
//! from the `Playing` state: it drains the public request queue, pulls
//! decoded buffers off the [`AudioLoader`] worker, recycles
//! `SoundAlias` channels (stock caps a sound at ~10 concurrent plays),
//! applies the positional mix, and drives the [`MapMusic`] state
//! machine + fades for the music stream.
//!
//! The raylib token can only be obtained once per process
//! (`init_audio_device` fails while `IsAudioDeviceReady`), so the token
//! — plus everything expensive to rebuild (decoded buffers, `Sound`/
//! alias caches, track positions, the decode worker) — lives in the
//! render-thread [`SHARED`] singleton and survives `PlayState` re-creation
//! (Failed → SignIn → Loading → Playing). `AudioDevice` itself only
//! carries per-session state: the map-music machine, the active stream,
//! pending plays and the map id.
//!
//! Device init failure (`init_audio_device` error or `!IsAudioDeviceReady`)
//! leaves the token `None` — audio silently disabled, never a panic,
//! and headless tests never touch it.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::time::Instant;

use raylib::audio::{Music, RaylibAudio, Sound, SoundAlias};

use crate::assets::audioxml::AudioPatterns;
use crate::assets::AssetManager;
use crate::audio::loader::{AudioLoader, AudioRequest};
use crate::audio::music::{MapMusic, MusicCmd};
use crate::audio::AudioSettings;
use crate::net::SharedWorld;
use crate::swf_extract::sound::DecodedAudio;

/// `AudioManager.soundRadius` — positional sounds beyond this are
/// skipped.
const SOUND_RADIUS: f32 = 800.0;
/// `AudioManager.soundVolumeLevel` — global sfx gain.
const SOUND_LEVEL: f32 = 0.8;
/// `AudioManager`: `pan = -(hero.x - x) / 500` → raylib `SetSoundPan`
/// is 0..1 (0.5 center), so `rl_pan = (as3 + 1) / 2`.
const PAN_DIVISOR: f32 = 500.0;
/// Stock channel cap for one sound (`sndCnt > 10` check).
const MAX_STACKING: usize = 10;
/// `loadAndCall(..., 1000)` — pending plays expire after a second.
const PENDING_TTL_MS: u128 = 1000;

thread_local! {
    /// Process-lifetime render-thread audio state (see module docs).
    static SHARED: RefCell<Shared> = RefCell::new(Shared::new());
}

/// Per-session facade (`Playing` state owns one). [`AudioDevice::new`]
/// always succeeds — the real device check happens against `SHARED`
/// inside `tick`.
pub struct AudioDevice {
    session: Session,
}

/// State that resets with every `Playing` session (stock re-creates
/// `AudioManager`'s map/track machinery per map anyway).
struct Session {
    /// plays queued while their class decodes.
    pending: Vec<PendingPlay>,
    /// pattern id -> last play — `interval` cooldown (`lastSettle`).
    last_play: HashMap<i32, Instant>,
    /// `Play` waiting for its buffer to decode.
    wanted: Option<MusicCmd>,
    /// The stream playing right now — dropping `AudioDevice` with the
    /// `PlayState` unloads it, so sign-in screens stay silent.
    active: Option<ActiveMusic>,
    /// maps-config state machine.
    map_music: MapMusic,
    /// last-seen inputs (diffed per tick).
    map_id: i32,
    combat_active: bool,
    pref_play_music: bool,
    pref_play_combat: bool,
    music_volume: i32,
    rng: u64,
}

impl Session {
    fn new() -> Self {
        Self {
            pending: Vec::new(),
            last_play: HashMap::new(),
            wanted: None,
            active: None,
            map_music: MapMusic::default(),
            map_id: 0,
            combat_active: false,
            pref_play_music: false,
            pref_play_combat: false,
            music_volume: 50,
            rng: 0x2545F4914F6CDD1D,
        }
    }
}

/// Process-lifetime render-thread state: token, worker and every cache
/// that costs a fetch/decode or a device handle.
struct Shared {
    /// `Some` once init was attempted (success or logged failure) —
    /// `init_audio_device` must never run twice in a process.
    init_tried: bool,
    /// Leaked: `Sound<'static>`/`Music<'static>` need an audio token
    /// with a static lifetime so they can live in `Shared`.
    audio: Option<&'static RaylibAudio>,
    loader: Option<AudioLoader>,
    /// last master-volume state applied to the token.
    muted: bool,
    // --- sfx -------------------------------------------------------
    /// (file_id, class) -> decoded WAV/MP3 buffer.
    decoded: HashMap<(String, String), Rc<DecodedAudio>>,
    /// decode jobs in flight.
    requested: HashSet<(String, String)>,
    /// decode jobs that failed once (don't hot-loop the CDN on a
    /// missing class — retried on the next *new* play request anyway).
    failed: HashSet<(String, String)>,
    /// (pattern id, variant) -> raylib sound + alias pool.
    cache: HashMap<(i32, usize), SfxEntry>,
    // --- music ------------------------------------------------------
    /// track id -> decoded stream bytes (kept for raylib's decoder).
    music_bufs: HashMap<i32, Rc<DecodedAudio>>,
    /// resKey(file id) -> track id of in-flight decodes.
    music_pending: HashMap<String, i32>,
    /// resKey -> remembered playback seconds (`setSoundProgress`).
    positions: HashMap<String, f32>,
}

impl Shared {
    fn new() -> Self {
        Self {
            init_tried: false,
            audio: None,
            loader: None,
            muted: false,
            decoded: HashMap::new(),
            requested: HashSet::new(),
            failed: HashSet::new(),
            cache: HashMap::new(),
            music_bufs: HashMap::new(),
            music_pending: HashMap::new(),
            positions: HashMap::new(),
        }
    }

    /// The raylib token, initialised on first use and memoised forever —
    /// `init_audio_device` errors while the device is ready, so the
    /// second `PlayState` must reuse the first token (and its caches).
    fn token(&mut self) -> Option<&'static RaylibAudio> {
        if !self.init_tried {
            self.init_tried = true;
            self.audio = match RaylibAudio::init_audio_device() {
                Ok(a) if a.is_audio_device_ready() => Some(Box::leak(Box::new(a))),
                Ok(_) => {
                    log::warn!("audio: device opened but not ready — audio disabled");
                    None
                }
                Err(_) => {
                    log::warn!("audio: device init failed — audio disabled");
                    None
                }
            };
        }
        self.audio
    }
}

struct SfxEntry {
    /// The source sound, leaked so pooled `SoundAlias<'static,'static>`
    /// can borrow it (`None` when raylib refused the buffer).
    sound: Option<&'static Sound<'static>>,
    aliases: Vec<SoundAlias<'static, 'static>>,
}

struct PendingPlay {
    id: i32,
    variant: usize,
    vol: f32,
    pan: f32,
    at: Instant,
}

/// Post-fade-out action (`AudioManager`'s tween callbacks).
enum FadeEnd {
    Stop,
    /// switchMusicTrackTo's deferred start.
    Switch { track: i32, loop_track: bool, fade_in: f32 },
}

struct ActiveMusic {
    music: Music<'static>,
    /// Decoded bytes kept alive for the stream's decoder.
    _data: Rc<DecodedAudio>,
    /// resources.xml file id — key into `positions`.
    res_key: String,
    loop_track: bool,
    /// current applied volume.
    vol: f32,
    /// fade target.
    target_vol: f32,
    /// |Δvol|/sec while fading (0 = settled).
    fade: f32,
    /// runs when `vol` reaches 0 mid-fade.
    fade_end: Option<FadeEnd>,
}

impl Default for AudioDevice {
    fn default() -> Self {
        Self::new()
    }
}

impl AudioDevice {
    /// Per-session state; the device token and caches are shared.
    pub fn new() -> Self {
        Self {
            session: Session::new(),
        }
    }

    /// One frame: queues, decodes, sequencing, fades.
    pub fn tick(
        &mut self,
        assets: &mut AssetManager,
        world: &SharedWorld,
        prefs: &AudioSettings,
        map_id: i32,
        dt: f32,
    ) {
        let session = &mut self.session;
        SHARED.with(|cell| {
            let mut shared = cell.borrow_mut();
            shared.tick(session, assets, world, prefs, map_id, dt);
        });
    }
}

impl Shared {
    fn tick(
        &mut self,
        s: &mut Session,
        assets: &mut AssetManager,
        world: &SharedWorld,
        prefs: &AudioSettings,
        map_id: i32,
        dt: f32,
    ) {
        let now = Instant::now();
        let Some(audio) = self.token() else {
            // No device — still drain so the queue doesn't grow.
            super::drain();
            return;
        };

        // Worker lazily — shares the sprite pipeline's file source.
        if self.loader.is_none() {
            self.loader = Some(AudioLoader::spawn(assets.file_source()));
        }

        let (requests, muted) = super::drain();
        if muted != self.muted {
            self.muted = muted;
            audio.set_master_volume(if muted { 0.0 } else { 1.0 });
        }

        let hero = world.hero_pos();

        // --- music inputs -----------------------------------------
        if prefs.play_music != s.pref_play_music {
            s.pref_play_music = prefs.play_music;
            if let Some(cmd) = s.map_music.set_enabled(prefs.play_music, now) {
                self.apply_music(s, &assets.game_xml.audio, cmd);
            }
        }
        if prefs.play_combat_music != s.pref_play_combat {
            s.pref_play_combat = prefs.play_combat_music;
            if let Some(cmd) = s.map_music.set_play_combat(prefs.play_combat_music, now) {
                self.apply_music(s, &assets.game_xml.audio, cmd);
            }
        }
        if world.combat_music_active != s.combat_active {
            s.combat_active = world.combat_music_active;
            if let Some(cmd) = s.map_music.set_combat_active(world.combat_music_active, now) {
                self.apply_music(s, &assets.game_xml.audio, cmd);
            }
        }
        if map_id != s.map_id {
            // Wait for the maps-config row — retry until it is loaded so
            // a late index doesn't lock the map to silence.
            if let Some(desc) = assets.maps.get(map_id) {
                s.map_id = map_id;
                let (music, loop_last, disable_combat) = (
                    desc.music.clone(),
                    desc.loop_last_track,
                    desc.disable_combat_music,
                );
                if let Some(cmd) = s.map_music.set_map(&music, loop_last, disable_combat, now) {
                    self.apply_music(s, &assets.game_xml.audio, cmd);
                }
            }
        }
        if let Some(cmd) = s.map_music.tick(now) {
            self.apply_music(s, &assets.game_xml.audio, cmd);
        }
        // `updateMusicVolume`: live volume on the active stream.
        if prefs.music_volume != s.music_volume {
            s.music_volume = prefs.music_volume;
            if let (Some(a), Some(track)) = (&mut s.active, s.map_music.current()) {
                if let Some(pat) = assets.game_xml.audio.music.get(&track) {
                    a.target_vol = pat.volume * prefs.music_volume as f32 / 50.0;
                    if a.fade <= 0.0 {
                        a.vol = a.target_vol;
                        a.music.set_volume(a.vol);
                    }
                }
            }
        }

        // --- sfx queue --------------------------------------------
        for req in requests {
            self.play(s, &assets.game_xml.audio, req, prefs, hero, now);
        }

        // --- decode completions ------------------------------------
        if let Some(loader) = &self.loader {
            let done: Vec<_> = loader.done().collect();
            for item in done {
                match item.result {
                    Ok(audio) => {
                        let buf = Rc::new(audio);
                        if let Some(track) = self.music_pending.remove(&item.file_id) {
                            self.music_bufs.insert(track, buf);
                            // Only a wanted Play for *this* track consumes
                            // the decode.
                            if matches!(s.wanted, Some(MusicCmd::Play { track: t, .. }) if t == track)
                            {
                                let cmd = s.wanted.take().unwrap();
                                self.apply_music(s, &assets.game_xml.audio, cmd);
                            }
                        } else {
                            self.decoded
                                .insert((item.file_id.clone(), item.class.clone()), buf);
                            self.requested
                                .remove(&(item.file_id.clone(), item.class.clone()));
                            self.flush_pending(s, &assets.game_xml.audio);
                        }
                    }
                    Err(e) => {
                        log::debug!("audio: {}/{} decode failed: {e}", item.file_id, item.class);
                        if self.music_pending.remove(&item.file_id).is_none() {
                            self.requested
                                .remove(&(item.file_id.clone(), item.class.clone()));
                            self.failed
                                .insert((item.file_id.clone(), item.class.clone()));
                        }
                    }
                }
            }
        }

        // --- per-frame music stream + fades -------------------------
        self.tick_music(s, dt, &assets.game_xml.audio);
    }

    /// `AudioManager.playSoundEffect(id, false, false, x, y)`.
    fn play(
        &mut self,
        s: &mut Session,
        patterns: &AudioPatterns,
        req: super::SoundRequest,
        prefs: &AudioSettings,
        hero: Option<(f32, f32)>,
        now: Instant,
    ) {
        let Some(pat) = patterns.sounds.get(&req.id) else {
            return;
        };
        let (on, pref_vol) = if pat.is_voice {
            (prefs.play_voice, prefs.voice_volume)
        } else {
            (prefs.play_sfx, prefs.sound_volume)
        };
        if !on || pat.res_keys.is_empty() {
            return;
        }
        // `interval` cooldown (`lastSettle` table).
        if pat.interval_ms > 0 {
            let last = s
                .last_play
                .get(&req.id)
                .map(|t| t.elapsed().as_millis() as i32)
                .unwrap_or(-1);
            if last >= 0 && last < pat.interval_ms {
                return;
            }
        }
        // volume = pattern.volume * 0.8 * volume/100, positional falloff
        // + pan when `at` is given and the hero is known.
        let mut vol = pat.volume * SOUND_LEVEL * (pref_vol.clamp(0, 100) as f32 / 100.0);
        let mut pan = 0.5_f32;
        if let (Some((x, y)), Some((hx, hy))) = (req.at, hero) {
            let dist = ((hx - x).powi(2) + (hy - y).powi(2)).sqrt();
            if dist > SOUND_RADIUS {
                return;
            }
            vol -= vol / SOUND_RADIUS * dist;
            let p = (-(hx - x) / PAN_DIVISOR).clamp(-1.0, 1.0);
            pan = (p + 1.0) / 2.0;
        }
        let variant = (Self::rand(s) * pat.res_keys.len() as f32) as usize % pat.res_keys.len();
        let key = (pat.soundbank.clone(), pat.res_keys[variant].clone());
        if self.decoded.contains_key(&key) {
            s.last_play.insert(req.id, now);
            self.play_decoded(req.id, variant, &key, vol, pan);
        } else if !self.failed.contains(&key) {
            // `loadAndCall` — play lands when the decode finishes.
            s.pending.push(PendingPlay {
                id: req.id,
                variant,
                vol,
                pan,
                at: now,
            });
            if self.requested.insert(key.clone()) {
                if let Some(l) = &self.loader {
                    l.request(AudioRequest::Sfx {
                        file_id: key.0.clone(),
                        class: key.1.clone(),
                    });
                }
            }
        }
        // `failed` entries: skip (CDN miss / unsupported — logged at
        // decode time, permanent like the sprite pipeline's failures).
    }

    /// Pending plays whose class finished decoding (`loadAndCall`'s
    /// 1000 ms wait reproduced via `PENDING_TTL_MS`).
    fn flush_pending(&mut self, s: &mut Session, patterns: &AudioPatterns) {
        let now = Instant::now();
        let mut keep = Vec::new();
        for p in std::mem::take(&mut s.pending) {
            if now.duration_since(p.at).as_millis() > PENDING_TTL_MS {
                continue;
            }
            let Some(pat) = patterns.sounds.get(&p.id) else {
                continue;
            };
            let key = (
                pat.soundbank.clone(),
                pat.res_keys.get(p.variant).cloned().unwrap_or_default(),
            );
            if self.decoded.contains_key(&key) {
                s.last_play.insert(p.id, now);
                self.play_decoded(p.id, p.variant, &key, p.vol, p.pan);
            } else {
                keep.push(p);
            }
        }
        s.pending = keep;
    }

    /// Build the raylib `Sound` on first use and play through an alias.
    fn play_decoded(
        &mut self,
        id: i32,
        variant: usize,
        key: &(String, String),
        vol: f32,
        pan: f32,
    ) {
        let Some(audio) = self.audio else { return };
        let Some(buf) = self.decoded.get(key).cloned() else {
            return;
        };
        let entry = self.cache.entry((id, variant)).or_insert_with(|| {
            let file_type = format!(".{}", buf.file_type);
            let sound = audio
                .new_wave_from_memory(&file_type, &buf.data)
                .and_then(|wave| audio.new_sound_from_wave(&wave))
                .ok()
                .map(|s| &*Box::leak(Box::new(s)));
            if sound.is_none() {
                log::debug!("audio: raylib rejected {}/{}", key.0, key.1);
            }
            SfxEntry {
                sound,
                aliases: Vec::new(),
            }
        });
        let Some(sound) = entry.sound else { return };
        // `sndCnt > 10` — drop instead of stacking further.
        if entry.aliases.iter().filter(|a| a.is_playing()).count() >= MAX_STACKING {
            return;
        }
        let alias = if let Some(i) = entry.aliases.iter().position(|a| !a.is_playing()) {
            entry.aliases.swap_remove(i)
        } else {
            match sound.alias() {
                Ok(a) => a,
                Err(_) => {
                    // No alias capacity — drive the source sound itself.
                    sound.set_volume(vol);
                    sound.set_pan(pan);
                    sound.play();
                    return;
                }
            }
        };
        alias.set_volume(vol);
        alias.set_pan(pan);
        alias.play();
        entry.aliases.push(alias);
    }

    /// `AudioManager.updateMusic`/`switchMusicTrackTo`/`stopMusic`.
    fn apply_music(&mut self, s: &mut Session, patterns: &AudioPatterns, cmd: MusicCmd) {
        match cmd {
            MusicCmd::Stop { fade_out } => {
                s.wanted = None;
                let mut hard = false;
                if let Some(a) = &mut s.active {
                    if fade_out <= 0.0 {
                        hard = true;
                    } else {
                        a.fade = a.vol / fade_out;
                        a.target_vol = 0.0;
                        a.fade_end = Some(FadeEnd::Stop);
                    }
                }
                if hard {
                    if let Some(a) = s.active.take() {
                        a.music.stop_stream();
                    }
                }
            }
            MusicCmd::Play {
                track,
                loop_track,
                fade_out,
                fade_in,
            } => {
                // Fade the current stream out first when asked, like the
                // stock musicTween chain.
                if fade_out > 0.0 {
                    if let Some(a) = &mut s.active {
                        a.fade = a.vol / fade_out;
                        a.target_vol = 0.0;
                        a.fade_end = Some(FadeEnd::Switch {
                            track,
                            loop_track,
                            fade_in,
                        });
                        return;
                    }
                }
                self.start_track(s, patterns, track, loop_track, fade_in);
            }
        }
    }

    /// Start `track` now (position memory, looping, fade-in).
    fn start_track(
        &mut self,
        s: &mut Session,
        patterns: &AudioPatterns,
        track: i32,
        loop_track: bool,
        fade_in: f32,
    ) {
        // `musicSndChannelLoop` positions memory — keep where the
        // outgoing looping track stopped.
        if let Some(old) = s.active.take() {
            if old.loop_track && old.music.is_stream_playing() {
                self.positions
                    .insert(old.res_key.clone(), old.music.get_time_played());
            }
            old.music.stop_stream();
        }
        let Some(pat) = patterns.music.get(&track) else {
            return;
        };
        let Some(buf) = self.music_bufs.get(&track).cloned() else {
            // Decode in flight or not started — queue and wait.
            s.wanted = Some(MusicCmd::Play {
                track,
                loop_track,
                fade_out: 0.0,
                fade_in,
            });
            if !self.music_pending.contains_key(&pat.res_key) {
                self.music_pending.insert(pat.res_key.clone(), track);
                if let Some(l) = &self.loader {
                    l.request(AudioRequest::Music {
                        file_id: pat.res_key.clone(),
                    });
                }
            }
            return;
        };
        let Some(audio) = self.audio else { return };
        let file_type = format!(".{}", buf.file_type);
        let Ok(mut music) = audio.new_music_from_memory(&file_type, &buf.data) else {
            log::debug!("audio: raylib rejected music {}", pat.res_key);
            return;
        };
        // raylib streams loop by default; explicit either way.
        music.as_mut().looping = loop_track;
        if let Some(p) = self.positions.get(&pat.res_key) {
            if *p > 0.0 {
                music.seek_stream(*p);
            }
        }
        let target = pat.volume * s.music_volume as f32 / 50.0;
        music.set_volume(if fade_in > 0.0 { 0.0 } else { target });
        music.play_stream();
        s.active = Some(ActiveMusic {
            music,
            _data: buf,
            res_key: pat.res_key.clone(),
            loop_track,
            vol: if fade_in > 0.0 { 0.0 } else { target },
            target_vol: target,
            fade: if fade_in > 0.0 { target / fade_in } else { 0.0 },
            fade_end: None,
        });
    }

    /// Per-frame stream pump + fade/step driver.
    fn tick_music(&mut self, s: &mut Session, dt: f32, patterns: &AudioPatterns) {
        let mut stop = false;
        let mut switch = None;
        if let Some(a) = &mut s.active {
            a.music.update_stream();
            if a.fade > 0.0 {
                let up = a.target_vol > a.vol;
                a.vol += if up { a.fade } else { -a.fade } * dt;
                let done = if up {
                    a.vol >= a.target_vol
                } else {
                    a.vol <= a.target_vol
                };
                if done {
                    a.vol = a.target_vol;
                    a.fade = 0.0;
                }
                a.music.set_volume(a.vol.max(0.0));
                if done {
                    match a.fade_end.take() {
                        Some(FadeEnd::Stop) => stop = true,
                        Some(FadeEnd::Switch {
                            track,
                            loop_track,
                            fade_in,
                        }) => switch = Some((track, loop_track, fade_in)),
                        None => {}
                    }
                }
            }
            // Looping safety net (`music.looping` is set on start;
            // decoder errors can still stop a stream).
            if !stop
                && switch.is_none()
                && a.loop_track
                && a.fade_end.is_none()
                && !a.music.is_stream_playing()
            {
                a.music.seek_stream(0.0);
                a.music.play_stream();
            }
        }
        if stop {
            if let Some(a) = s.active.take() {
                a.music.stop_stream();
            }
        }
        if let Some((track, loop_track, fade_in)) = switch {
            self.start_track(s, patterns, track, loop_track, fade_in);
        }
    }

    /// splitmix64 -> [0,1) — variant pick (`AudioMultiPattern.resKey`
    /// getter's `Math.random`).
    fn rand(s: &mut Session) -> f32 {
        s.rng = s.rng.wrapping_add(0x9E3779B97F4A7C15);
        let mut z = s.rng;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
        z ^= z >> 31;
        ((z >> 40) & 0xFF_FFFF) as f32 / (1u64 << 24) as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decoded() -> DecodedAudio {
        DecodedAudio {
            file_type: "wav",
            data: vec![0u8; 4],
            compression: 3,
        }
    }

    /// The shared half keeps fetched/decoded state; a new `AudioDevice`
    /// only resets session state (lead: second `PlayState` must reuse
    /// the first token and its caches).
    #[test]
    fn shared_caches_survive_a_new_session() {
        let mut shared = Shared::new();
        shared
            .decoded
            .insert(("bank".into(), "cls".into()), Rc::new(decoded()));
        shared.positions.insert("track0".into(), 12.5);
        let _device_a = AudioDevice::new();
        let device_b = AudioDevice::new();
        assert!(device_b.session.pending.is_empty());
        assert!(device_b.session.wanted.is_none());
        assert!(device_b.session.map_id == 0);
        // …while the shared caches (and token slot) persist untouched.
        assert!(shared.decoded.contains_key(&("bank".into(), "cls".into())));
        assert_eq!(shared.positions["track0"], 12.5);
    }

    /// `init_audio_device` must run at most once per process even when
    /// it fails — `init_tried` memoises the attempt.
    #[test]
    fn token_init_is_memoised() {
        let mut shared = Shared::new();
        assert!(!shared.init_tried);
        let _ = shared.token(); // may succeed on machines with audio
        assert!(shared.init_tried);
        let again = shared.token();
        assert_eq!(again.is_some(), shared.audio.is_some());
    }

    /// Pending plays resolve against the shared decoded cache without a
    /// device (`play_decoded` no-ops when `audio` is `None`).
    #[test]
    fn flush_pending_consumes_decoded() {
        let mut shared = Shared::new();
        let mut s = Session::new();
        let mut patterns = AudioPatterns::default();
        patterns.sounds.insert(
            7,
            crate::assets::audioxml::SoundPattern {
                id: 7,
                volume: 1.0,
                soundbank: "bank".into(),
                res_keys: vec!["cls".into()],
                ..Default::default()
            },
        );
        shared
            .decoded
            .insert(("bank".into(), "cls".into()), Rc::new(decoded()));
        s.pending.push(PendingPlay {
            id: 7,
            variant: 0,
            vol: 1.0,
            pan: 0.5,
            at: Instant::now(),
        });
        shared.flush_pending(&mut s, &patterns);
        assert!(s.pending.is_empty());
        assert!(s.last_play.contains_key(&7));
    }
}
