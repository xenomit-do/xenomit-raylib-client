//! Map music sequencing — a pure port of `MapAudioManager` +
//! `AudioManager.updateMusic`/`switchMusicTrackTo` (no raylib, unit
//! tested). The render-thread device applies the returned commands.
//!
//! Stock constants: `FADE_OUT`/`FADE_IN` = 3 s; a combat switch uses a
//! 0.5 s fade-out + `crossFade(1500)` = 1.5 s fade-in; a returning map
//! track fades in at `musicFadeIn` (fade-in time of the sound in
//! ms / 1000 — every music pattern we ship uses default fadeIn 1 → 1 s).
//!
//! `combatMusicMode` logic in the AS3 `update()` is self-cancelling when
//! combat music is enabled (check 3 makes the flag false while checks
//! 1+2 keep request it on again); the *effective* behaviour is: combat
//! track plays while `combatMusicActive && playCombatMusic &&
//! playMusic && !disableCombatMusic`, map list resumes afterwards.

use std::time::{Duration, Instant};

/// `AudioManager.MUSIC_ID_COMBAT`.
pub const MUSIC_ID_COMBAT: i32 = 1000001;
/// `MapAudioManager.FADE_OUT` / `FADE_IN` (seconds).
pub const FADE_SECS: f32 = 3.0;
/// `playCombatMusic` switch: `musicTween…:0.5` + `crossFade(1500)`.
pub const COMBAT_FADE_OUT_SECS: f32 = 0.5;
pub const COMBAT_FADE_IN_SECS: f32 = 1.5;
/// `musicFadeIn` on return to the map track (pattern fadeIn default 1 s).
pub const RESUME_FADE_IN_SECS: f32 = 1.0;

/// What the state machine asks the device to do.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MusicCmd {
    /// Start `track`; `loop_track` mirrors the stock channel-loop flag;
    /// fades: out of the current, into the new.
    Play {
        track: i32,
        loop_track: bool,
        fade_out: f32,
        fade_in: f32,
    },
    /// Fade out and stop (`fade_out` seconds).
    Stop { fade_out: f32 },
}

#[derive(Default)]
pub struct MapMusic {
    /// maps-config `music` attr, exploded: alternating
    /// `track[,delay,track,…]` (bare ints become `track,delay` pairs).
    tracks: Vec<i32>,
    delays: Vec<f32>,
    idx: usize,
    /// `loopLastTrack` (`String(attr) !== "false"` → default true).
    loop_last: bool,
    /// `disableCombatMusic` — same stock quirk: absent attr disables.
    disable_combat: bool,
    /// AS3 `playMusic` setting copy.
    enabled: bool,
    /// `Settings.playCombatMusic`.
    play_combat: bool,
    /// `Settings.combatMusicActive` (CombatMusicCommand).
    combat_active: bool,
    /// AS3 `_combatMusicMode` — we're on the combat track.
    combat_mode: bool,
    /// AS3 `_musicTrackPlaying` — index into `tracks`, -1 = none.
    playing: i32,
    /// AS3 `_trackProgressive`/`_currentTrack` for `tick` bookkeeping.
    next_at: Option<Instant>,
    /// What the map list wants to hear right now (combat excluded).
    map_want: Option<i32>,
    /// Track the output was last asked to play.
    current: Option<i32>,
    current_loops: bool,
}

impl MapMusic {
    /// `MapAudioManager.update()` — call on map change.
    /// `music` is the raw maps-config attr; `loop_last` /
    /// `disable_combat` are the map attributes (AS3 quirks preserved).
    pub fn set_map(
        &mut self,
        music: &str,
        loop_last: bool,
        disable_combat: bool,
        now: Instant,
    ) -> Option<MusicCmd> {
        self.loop_last = loop_last;
        self.disable_combat = disable_combat;

        // AS3 builds [track0, delay0, track1, delay1, …]; a trailing
        // delay drops out. `loopLastTrack` from XML is stored then
        // overwritten below anyway (dead in stock too — kept for
        // completeness).
        self.tracks.clear();
        self.delays.clear();
        let mut iter = music.split(',').map(str::trim).filter(|s| !s.is_empty());
        while let Some(t) = iter.next() {
            if let Ok(track) = t.parse::<i32>() {
                self.tracks.push(track);
                self.delays
                    .push(iter.next().and_then(|d| d.parse::<f32>().ok()).unwrap_or(0.0));
            }
        }
        self.idx = 0;
        self.playing = -1;
        self.combat_mode = false;

        // The map list always restarts first; if combat is still
        // wanted, the next `tick` switches to the combat track (stock
        // does it on the same 1 s timer tick). An empty list (or music
        // disabled) stops playback; a deduped `Play` returns None with
        // the current track untouched.
        if self.tracks.is_empty() || !self.enabled {
            self.stop_if_needed(FADE_SECS)
        } else {
            self.play_next_track(now, true)
        }
    }

    /// `Settings.combatMusicActive` — CombatMusicCommand event.
    pub fn set_combat_active(&mut self, active: bool, now: Instant) -> Option<MusicCmd> {
        if self.combat_active == active {
            return None;
        }
        self.combat_active = active;
        if self.combat_wanted() {
            self.play_combat_track()
        } else if self.combat_mode {
            self.leave_combat_track(now)
                .or_else(|| self.stop_if_needed(0.0))
        } else {
            // Stock only stops when the update wanted combat mode —
            // a blocked/off combat event leaves the map track playing.
            None
        }
    }

    /// `Settings.playMusic` — on→off stops, off→on restarts the map list.
    pub fn set_enabled(&mut self, enabled: bool, now: Instant) -> Option<MusicCmd> {
        if self.enabled == enabled {
            return None;
        }
        self.enabled = enabled;
        if !enabled {
            return self.stop_if_needed(FADE_SECS);
        }
        if self.combat_wanted() {
            self.play_combat_track()
        } else {
            self.play_next_track(now, true)
        }
    }

    /// `Settings.playCombatMusic` — flips whether combat takes over.
    pub fn set_play_combat(&mut self, play: bool, now: Instant) -> Option<MusicCmd> {
        if self.play_combat == play {
            return None;
        }
        self.play_combat = play;
        if self.combat_wanted() {
            self.play_combat_track()
        } else if self.combat_mode {
            self.leave_combat_track(now)
                .or_else(|| self.stop_if_needed(0.0))
        } else {
            None
        }
    }

    /// Per-frame update — delay timers, looping the last track, and the
    /// combat ↔ map transitions the AS3 timer drives.
    pub fn tick(&mut self, now: Instant) -> Option<MusicCmd> {
        if self.combat_wanted() && !self.combat_mode {
            return self.play_combat_track();
        }
        if !self.combat_wanted() && self.combat_mode {
            return self
                .leave_combat_track(now)
                .or_else(|| self.stop_if_needed(0.0));
        }
        if self.combat_mode || self.tracks.is_empty() {
            return None;
        }
        if self.playing < 0 {
            return self.play_next_track(now, false);
        }
        let next = self.next_at?;
        if now >= next {
            self.play_next_track(now, false)
        } else {
            None
        }
    }

    /// Currently requested track (for dedupe/volume updates).
    pub fn current(&self) -> Option<i32> {
        self.current
    }

    // --- internals ------------------------------------------------------

    fn combat_wanted(&self) -> bool {
        // MapAudioManager.updateMusic: playMusic && playCombatMusic &&
        // combatMusicActive && !disableCombatMusic
        self.enabled && self.play_combat && self.combat_active && !self.disable_combat
    }

    /// `playCombatMusicTrack` — switchMusicTrackTo(COMBAT, 0.5) +
    /// crossFade(1500).
    fn play_combat_track(&mut self) -> Option<MusicCmd> {
        if self.combat_mode {
            return None;
        }
        self.combat_mode = true;
        self.next_at = None;
        self.playing = -1;
        self.emit_play(MUSIC_ID_COMBAT, false, COMBAT_FADE_OUT_SECS, COMBAT_FADE_IN_SECS)
    }

    /// `playNextTrack(true)` after combat (or when combat is
    /// unavailable): no fade-out (`noFadeOut` in tick start-up too).
    fn leave_combat_track(&mut self, now: Instant) -> Option<MusicCmd> {
        self.combat_mode = false;
        self.playing = -1;
        self.next_at = None;
        self.play_next_track(now, false).map(|cmd| match cmd {
            MusicCmd::Play {
                track,
                loop_track,
                fade_in,
                ..
            } => MusicCmd::Play {
                track,
                loop_track,
                fade_out: 0.0,
                fade_in,
            },
            c => c,
        })
    }

    /// `playNextTrack(fadeOut)`: single-track maps loop; multi-track
    /// lists advance with the per-track delay, wrapping to 0.
    fn play_next_track(&mut self, now: Instant, fade_out: bool) -> Option<MusicCmd> {
        if self.tracks.is_empty() || !self.enabled {
            return None;
        }
        // `!multiTrack || !loopLastTrack` → channel loops (9999).
        let loop_track = self.tracks.len() < 2 || !self.loop_last;
        let track = self.tracks[self.idx % self.tracks.len()];
        self.playing = self.idx as i32;
        self.idx = (self.idx + 1) % self.tracks.len();
        self.next_at = (!loop_track).then(|| {
            now + Duration::from_secs_f32(self.delays[self.playing as usize].max(0.0))
        });
        self.emit_play(
            track,
            loop_track,
            if fade_out { FADE_SECS } else { 0.0 },
            RESUME_FADE_IN_SECS,
        )
    }

    fn emit_play(
        &mut self,
        track: i32,
        loop_track: bool,
        fade_out: f32,
        fade_in: f32,
    ) -> Option<MusicCmd> {
        // Dedupe like `switchMusicTrackTo`'s resKey early-exit (same
        // track id => same resKey => keep the running channel).
        if self.current == Some(track) {
            return None;
        }
        self.current = Some(track);
        self.current_loops = loop_track;
        self.map_want = Some(track);
        Some(MusicCmd::Play {
            track,
            loop_track,
            fade_out,
            fade_in,
        })
    }

    fn stop_if_needed(&mut self, fade_out: f32) -> Option<MusicCmd> {
        self.map_want = None;
        self.next_at = None;
        self.playing = -1;
        self.combat_mode = false;
        if self.current.take().is_some() {
            Some(MusicCmd::Stop { fade_out })
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> Instant {
        Instant::now()
    }

    #[test]
    fn single_track_loops() {
        let mut m = MapMusic {
            enabled: true,
            ..Default::default()
        };
        // X-1 style: `music="6"` → looping track 6.
        assert_eq!(
            m.set_map("6", true, true, now()),
            Some(MusicCmd::Play {
                track: 6,
                loop_track: true,
                fade_out: FADE_SECS,
                fade_in: RESUME_FADE_IN_SECS,
            })
        );
        // No timer — looping; tick is quiet.
        assert_eq!(m.tick(now()), None);
    }

    #[test]
    fn multi_track_sequences_with_delays() {
        let mut m = MapMusic {
            enabled: true,
            ..Default::default()
        };
        let t0 = Instant::now();
        // "2,20,3,15,5" → tracks [2,3,5] delays [20,15,0]
        let cmd = m.set_map("2,20,3,15,5", true, true, t0);
        assert_eq!(cmd, Some(MusicCmd::Play { track: 2, loop_track: false, fade_out: FADE_SECS, fade_in: 1.0 }));
        assert_eq!(m.tick(t0 + Duration::from_secs(10)), None);
        // After 20 s → track 3, delay 15.
        assert!(matches!(
            m.tick(t0 + Duration::from_secs(20)),
            Some(MusicCmd::Play { track: 3, .. })
        ));
        assert!(matches!(
            m.tick(t0 + Duration::from_secs(36)),
            Some(MusicCmd::Play { track: 5, .. })
        ));
        // Loop back to 2 after the last delay (0 s → immediately on
        // next tick since delay 0).
        assert!(matches!(
            m.tick(t0 + Duration::from_secs(36)),
            Some(MusicCmd::Play { track: 2, .. })
        ));
    }

    #[test]
    fn no_loop_last_repeats_single_track() {
        // `loopLastTrack="false"` + 2 tracks → channels loop anyway
        // (stock `!multiTrack || !loopLastTrack`), no sequencing.
        let mut m = MapMusic {
            enabled: true,
            ..Default::default()
        };
        let t0 = Instant::now();
        let cmd = m.set_map("4,10,7,10", false, true, t0);
        assert!(matches!(cmd, Some(MusicCmd::Play { track: 4, loop_track: true, .. })));
        assert_eq!(m.tick(t0 + Duration::from_secs(600)), None);
    }

    #[test]
    fn combat_track_gating() {
        let mut m = MapMusic {
            enabled: true,
            play_combat: true,
            ..Default::default()
        };
        let t0 = Instant::now();
        m.set_map("6", true, true, t0); // disableCombatMusic default → combat off
        m.set_combat_active(true, t0);
        assert_eq!(m.tick(t0), None); // disableCombatMusic blocks it

        // Map that allows combat music (`disableCombatMusic="false"`).
        let mut m = MapMusic {
            enabled: true,
            play_combat: true,
            ..Default::default()
        };
        m.set_map("6", true, false, t0);
        assert_eq!(
            m.set_combat_active(true, t0),
            Some(MusicCmd::Play {
                track: MUSIC_ID_COMBAT,
                loop_track: false,
                fade_out: COMBAT_FADE_OUT_SECS,
                fade_in: COMBAT_FADE_IN_SECS,
            })
        );
        // Stay on combat while active.
        assert_eq!(m.tick(t0 + Duration::from_secs(60)), None);
        // Combat off → map track resumes without fade-out.
        assert!(matches!(
            m.set_combat_active(false, t0),
            Some(MusicCmd::Play { track: 6, fade_out: 0.0, .. })
        ));
    }

    #[test]
    fn play_music_off_stops() {
        let mut m = MapMusic {
            enabled: true,
            ..Default::default()
        };
        let t0 = Instant::now();
        m.set_map("6", true, true, t0);
        assert_eq!(
            m.set_enabled(false, t0),
            Some(MusicCmd::Stop { fade_out: FADE_SECS })
        );
        // Back on → first track restarts.
        assert!(matches!(
            m.set_enabled(true, t0),
            Some(MusicCmd::Play { track: 6, .. })
        ));
    }

    #[test]
    fn empty_music_list_stays_silent() {
        let mut m = MapMusic {
            enabled: true,
            ..Default::default()
        };
        assert_eq!(m.set_map("", true, true, now()), None);
        assert_eq!(m.tick(now()), None);
    }

    #[test]
    fn dedupe_same_track() {
        let mut m = MapMusic {
            enabled: true,
            ..Default::default()
        };
        let t0 = Instant::now();
        m.set_map("6", true, true, t0);
        // Re-entering the same map (same track+loop) emits nothing.
        assert_eq!(m.set_map("6", true, true, t0), None);
    }
}
