//! `<patterns><music>` / `<patterns><sounds>` — the audio half of
//! `PatternManager` (`parseMusicPatterns` / `parseSoundPatterns`).
//!
//! ```xml
//! <music><track id="0" resKey="track0" volume="0.3"/></music>
//! <sounds><sound id="0" volume="0.4" soundbank="combatSounds">
//!     <reskeys><reskey resKey="laser0_0"/>...</reskeys>
//! </sound></sounds>
//! ```
//!
//! Music `resKey` is the resources.xml file id of a track SWF whose
//! embedded sound class is always `"track"` (`AudioManager.playMusic`
//! calls `finisher.getEmbeddedSound("track")`). Sound `soundbank` is the
//! file id of the soundbank SWF and each `reskey@resKey` an exported
//! sound class inside it — `AudioMultiPattern.resKey` picks one at
//! random per play.

use roxmltree::Node;
use std::collections::HashMap;

/// `<music><track>` — `AudioPattern(id, null)` with `resKey` = file id.
#[derive(Clone, Debug, Default)]
pub struct MusicPattern {
    /// `id` — `InGameCatalog.MUSIC_ID_*` / maps-config `music` list key.
    pub id: i32,
    /// `resKey` — resources.xml file id of the track SWF.
    pub res_key: String,
    /// `volume` attr (AudioPattern default 1).
    pub volume: f32,
}

/// `<sounds><sound>` — `AudioMultiPattern(id, soundbank)`.
#[derive(Clone, Debug, Default)]
pub struct SoundPattern {
    /// `id` — `InGameCatalog.SOUND_ID_*` / pattern `soundID` lookup key.
    pub id: i32,
    /// `soundbank` — resources.xml file id of the soundbank SWF.
    pub soundbank: String,
    /// `<reskeys><reskey resKey>` — exported sound classes; a random one
    /// plays each time (`AudioMultiPattern.resKey` getter).
    pub res_keys: Vec<String>,
    /// `volume` attr (AudioPattern default 1).
    pub volume: f32,
    /// `loop="true"` — channel plays with 9999 loops.
    pub loop_play: bool,
    /// `isVoice="true"` — mixed on `voiceVolume`/`playVoice` instead of
    /// `soundVolume`/`playSFX`.
    pub is_voice: bool,
    /// `interval` attr (seconds in the file; the AS3 setter stores ms).
    pub interval_ms: i32,
    /// `intervalFluctuation` (seconds -> ms like `interval`).
    pub interval_fluctuation_ms: i32,
}

#[derive(Default)]
pub struct AudioPatterns {
    /// `PatternManager.musicPatterns` — track id -> pattern.
    pub music: HashMap<i32, MusicPattern>,
    /// `PatternManager.soundPatterns` — sound id -> pattern.
    pub sounds: HashMap<i32, SoundPattern>,
}

fn attr<'a>(node: &Node<'a, 'a>, name: &str) -> &'a str {
    node.attribute(name).unwrap_or_default()
}

fn attr_i32(node: &Node, name: &str) -> i32 {
    node.attribute(name)
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or_default()
}

impl AudioPatterns {
    /// Parse the `<music>`/`<sounds>` sections under the `<patterns>`
    /// element (the same node `GameXml::parse` walks).
    pub fn parse(patterns: &Node) -> Self {
        let mut audio = AudioPatterns::default();
        for section in patterns.children().filter(|n| n.is_element()) {
            if section.has_tag_name("music") {
                for track in section
                    .children()
                    .filter(|n| n.is_element() && n.has_tag_name("track"))
                {
                    // `@volume` only applied when present (default 1).
                    let volume = track
                        .attribute("volume")
                        .and_then(|v| v.trim().parse().ok())
                        .unwrap_or(1.0);
                    let pat = MusicPattern {
                        id: attr_i32(&track, "id"),
                        res_key: attr(&track, "resKey").to_string(),
                        volume,
                    };
                    audio.music.insert(pat.id, pat);
                }
            } else if section.has_tag_name("sounds") {
                for sound in section
                    .children()
                    .filter(|n| n.is_element() && n.has_tag_name("sound"))
                {
                    let mut pat = SoundPattern {
                        id: attr_i32(&sound, "id"),
                        soundbank: attr(&sound, "soundbank").to_string(),
                        volume: sound
                            .attribute("volume")
                            .and_then(|v| v.trim().parse().ok())
                            .unwrap_or(1.0),
                        loop_play: sound.attribute("loop").is_some_and(|v| v == "true"),
                        is_voice: sound.attribute("isVoice").is_some_and(|v| v == "true"),
                        // `interval`/`intervalFluctuation` setters store ms.
                        interval_ms: (attr(&sound, "interval")
                            .trim()
                            .parse::<f32>()
                            .unwrap_or(0.0)
                            * 1000.0) as i32,
                        interval_fluctuation_ms: (attr(&sound, "intervalFluctuation")
                            .trim()
                            .parse::<f32>()
                            .unwrap_or(0.0)
                            * 1000.0) as i32,
                        ..Default::default()
                    };
                    for reskeys in sound
                        .children()
                        .filter(|n| n.is_element() && n.has_tag_name("reskeys"))
                    {
                        for reskey in reskeys
                            .children()
                            .filter(|n| n.is_element() && n.has_tag_name("reskey"))
                        {
                            let key = attr(&reskey, "resKey");
                            if !key.is_empty() {
                                pat.res_keys.push(key.to_string());
                            }
                        }
                    }
                    audio.sounds.insert(pat.id, pat);
                }
            }
        }
        audio
    }
}

#[cfg(test)]
mod tests {
    use crate::assets::GameXml;

    /// Real fixture: sound 0 = combatSounds laser0_0..2 @ vol 0.4,
    /// track 1000001 = trackCombat, voice rows carry `isVoice`.
    #[test]
    fn audio_patterns_parse_from_fixture() {
        let xml = std::fs::read_to_string("tests/fixtures/game.xml").expect("fixture missing");
        let game = GameXml::parse(&xml).expect("game.xml parses");

        let laser = &game.audio.sounds[&0];
        assert_eq!(laser.soundbank, "combatSounds");
        assert_eq!(laser.res_keys, ["laser0_0", "laser0_1", "laser0_2"]);
        assert!((laser.volume - 0.4).abs() < 1e-6);
        assert!(!laser.is_voice);

        // `isVoice="true"` rows resolve to voice patterns.
        let voice = &game.audio.sounds[&20];
        assert!(voice.is_voice);
        assert_eq!(voice.soundbank, "voiceSounds");

        let combat = &game.audio.music[&1000001];
        assert_eq!(combat.res_key, "trackCombat");
        assert!((combat.volume - 0.3).abs() < 1e-6);

        let ambient = &game.audio.music[&0];
        assert_eq!(ambient.res_key, "track0");
    }
}
