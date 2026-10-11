//! `DefineSound` -> playable audio bytes (`SWFFinisher.getEmbeddedSound`
//! returns a `flash.media.Sound`; here we hand raylib a container it can
//! decode).
//!
//! - `AudioCompression::Mp3` (2): `data` starts with an SI16
//!   `SeekSamples` latency count — strip it and return bare MP3 frames
//!   (`.mp3`).
//! - `Uncompressed` (3) / `UncompressedUnknownEndian` (0): PCM, wrapped
//!   in a RIFF/WAVE header so raylib can `LoadWaveFromMemory` (`.wav`).
//! - `Adpcm` (1): Flash's IMA-ADPCM variant decoded to PCM16 → `.wav`
//!   (port of ruffle's `decoders::adpcm`).
//! - Nellymoser/Speex/etc: `Err` — none of the resources.xml audio files
//!   use them (see the `real_sounds` test, which skips gracefully when
//!   the CDN cache is absent).

use anyhow::{anyhow, bail, Result};
use swf::{AudioCompression, Sound};

/// Decoded buffer for raylib (`LoadWaveFromMemory`/`LoadMusicStreamFromMemory`).
pub struct DecodedAudio {
    /// `"wav"` | `"mp3"` — raylib's `fileType` argument.
    pub file_type: &'static str,
    pub data: Vec<u8>,
    /// Compression id of the source `DefineSound` (diagnostics).
    pub compression: u8,
}

/// One `DefineSound` entry for `--list` style dumps.
pub struct SoundEntry {
    pub name: String,
    pub compression: AudioCompression,
    /// Source format fields (pre-decode).
    pub sample_rate: u16,
    pub is_stereo: bool,
    pub is_16_bit: bool,
    pub num_samples: u32,
}

/// Every exported class that resolves to a `DefineSound` character,
/// sorted by name.
pub fn sound_inventory(lib: &super::SwfLibrary) -> Vec<SoundEntry> {
    let mut out = Vec::new();
    let tags = super::collect_tags(&lib.buf);
    for (name, _) in sorted_symbols(lib) {
        if let Some(s) = tags.iter().find_map(|t| match t {
            swf::Tag::DefineSound(s) if s.id == lib.symbols[name] => Some(s),
            _ => None,
        }) {
            out.push(SoundEntry {
                name: name.clone(),
                compression: s.format.compression,
                sample_rate: s.format.sample_rate,
                is_stereo: s.format.is_stereo,
                is_16_bit: s.format.is_16_bit,
                num_samples: s.num_samples,
            });
        }
    }
    out
}

fn sorted_symbols(lib: &super::SwfLibrary) -> Vec<(&String, u16)> {
    let mut v: Vec<_> = lib.symbols.iter().map(|(n, i)| (n, *i)).collect();
    v.sort_by(|a, b| a.0.cmp(b.0));
    v
}

/// Extract `class_name` (exported symbol) from an audio SWF as
/// `DecodedAudio`.
pub fn extract_sound(lib: &super::SwfLibrary, class_name: &str) -> Result<DecodedAudio> {
    let sound = lib
        .define_sound(class_name)
        .ok_or_else(|| anyhow!("no DefineSound for {class_name:?}"))?;
    decode_sound(&sound)
}

/// Decode a `DefineSound` tag's payload into a playable container.
pub fn decode_sound(sound: &Sound<'_>) -> Result<DecodedAudio> {
    match sound.format.compression {
        AudioCompression::Mp3 => {
            // SWF19: SoundData for MP3 starts with SeekSamples (SI16).
            let data = sound
                .data
                .get(2..)
                .ok_or_else(|| anyhow!("mp3 DefineSound shorter than SeekSamples"))?;
            Ok(DecodedAudio {
                file_type: "mp3",
                data: data.to_vec(),
                compression: 2,
            })
        }
        AudioCompression::Uncompressed | AudioCompression::UncompressedUnknownEndian => {
            let wav = wrap_wav(
                sound.data,
                sound.format.is_16_bit,
                sound.format.is_stereo,
                u32::from(sound.format.sample_rate),
            );
            Ok(DecodedAudio {
                file_type: "wav",
                data: wav,
                compression: sound.format.compression as u8,
            })
        }
        AudioCompression::Adpcm => {
            let pcm = adpcm_decode(sound.data, sound.format.is_stereo)?;
            let wav = wrap_pcm16_wav(&pcm, sound.format.is_stereo, u32::from(sound.format.sample_rate));
            Ok(DecodedAudio {
                file_type: "wav",
                data: wav,
                compression: 1,
            })
        }
        other => bail!("unsupported DefineSound compression {other:?}"),
    }
}

/// Wrap raw PCM (interleaved; `is_16_bit` selects i16-LE vs u8) into a
/// canonical 44-byte RIFF/WAVE header.
fn wrap_wav(pcm: &[u8], is_16_bit: bool, stereo: bool, rate: u32) -> Vec<u8> {
    let bits: u16 = if is_16_bit { 16 } else { 8 };
    let channels: u16 = if stereo { 2 } else { 1 };
    wav_header(pcm.len() as u32, channels, rate, bits, pcm.to_vec())
}

/// Wrap decoded i16 PCM samples (interleaved L/R when stereo).
fn wrap_pcm16_wav(samples: &[i16], stereo: bool, rate: u32) -> Vec<u8> {
    let mut pcm = Vec::with_capacity(samples.len() * 2);
    for s in samples {
        pcm.extend_from_slice(&s.to_le_bytes());
    }
    let channels: u16 = if stereo { 2 } else { 1 };
    wav_header(pcm.len() as u32, channels, rate, 16, pcm)
}

fn wav_header(data_len: u32, channels: u16, rate: u32, bits: u16, pcm: Vec<u8>) -> Vec<u8> {
    let byte_rate = rate * channels as u32 * (bits as u32 / 8);
    let block_align = channels * (bits / 8);
    let mut out = Vec::with_capacity(44 + pcm.len());
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36u32 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes()); // fmt chunk size
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&channels.to_le_bytes());
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&byte_rate.to_le_bytes());
    out.extend_from_slice(&block_align.to_le_bytes());
    out.extend_from_slice(&bits.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    out.extend_from_slice(&pcm);
    out
}

// ---------------------------------------------------------------------------
// Flash ADPCM decode (ruffle core/backend/audio/decoders/adpcm.rs).

/// The Flash ADPCM table contains the four "magic" values at its start,
/// followed by the standard IMA step table.
const ADPCM_STEP_TABLE: [u16; 89] = [
    7, 8, 9, 10, 11, 12, 13, 14, 16, 17, 19, 21, 23, 25, 28, 31, 34, 37, 41, 45, 50, 55, 60, 66, 73,
    80, 88, 97, 107, 118, 130, 143, 157, 173, 190, 209, 230, 253, 279, 307, 337, 371, 408, 449, 494,
    544, 598, 658, 724, 796, 876, 963, 1060, 1166, 1282, 1411, 1552, 1707, 1878, 2066, 2272, 2499,
    2749, 3024, 3327, 3660, 4026, 4428, 4871, 5358, 5894, 6484, 7132, 7845, 8630, 9493, 10442, 11487,
    12635, 13899, 15289, 16818, 18500, 20350, 22385, 24623, 27086, 29794, 32767,
];

/// The index table implicitly defined in SWF19.
const ADPCM_INDEX_TABLE: [i32; 16] = [
    -1, -1, -1, -1, 2, 4, 6, 8, -1, -1, -1, -1, 2, 4, 6, 8,
];

const ADPCM_BLOCK_SIZE: u32 = 4095;

/// MSB-first bit reader (bitstream_io's `BigEndian`).
struct Bits<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Bits<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    fn read(&mut self, n: u32) -> Option<u32> {
        let mut v = 0u32;
        for _ in 0..n {
            let byte = *self.data.get(self.pos / 8)?;
            v = (v << 1) | ((byte >> (7 - (self.pos % 8))) & 1) as u32;
            self.pos += 1;
        }
        Some(v)
    }

    fn read_signed(&mut self, n: u32) -> Option<i32> {
        let v = self.read(n)? as i32;
        // sign-extend
        Some(if v & (1 << (n - 1)) != 0 {
            v - (1 << n)
        } else {
            v
        })
    }
}

struct AdpcmChannel {
    sample: i16,
    step_index: i32,
}

impl AdpcmChannel {
    fn decode_frame(&mut self, bits: &mut Bits, bits_per_sample: u32) -> Option<i16> {
        let sign = bits.read(1)?;
        let magnitude = bits.read(bits_per_sample - 1)?;

        // Flash ADPCM sign-magnitude form.
        self.step_index += ADPCM_INDEX_TABLE[magnitude as usize];
        self.step_index = self.step_index.clamp(0, (ADPCM_STEP_TABLE.len() - 5) as i32);

        let step = i32::from(ADPCM_STEP_TABLE[self.step_index as usize]);
        let mut delta = step >> 2;
        for i in (0..bits_per_sample - 1).rev() {
            if magnitude & (1 << i) != 0 {
                delta += step >> 1;
            }
        }

        let sample = i32::from(self.sample)
            + if sign != 0 { -delta } else { delta };
        self.sample = sample.clamp(-32768, 32767) as i16;
        Some(self.sample)
    }
}

/// Decode a Flash ADPCM bitstream to interleaved i16 PCM samples.
pub fn adpcm_decode(data: &[u8], stereo: bool) -> Result<Vec<i16>> {
    let num_channels = if stereo { 2usize } else { 1 };
    let mut reader = Bits::new(data);
    let mut left = AdpcmChannel { sample: 0, step_index: 0 };
    let mut right = AdpcmChannel { sample: 0, step_index: 0 };

    let bits_per_sample = reader
        .read(2)
        .ok_or_else(|| anyhow!("adpcm stream truncated at bit width"))?
        + 2;

    let mut pcm = Vec::new();
    let mut sample_num = 0u32;
    loop {
        if sample_num.is_multiple_of(ADPCM_BLOCK_SIZE) {
            // Block header: 16-bit initial sample + 6-bit step index per channel.
            left.sample = reader
                .read_signed(16)
                .and_then(|v| i16::try_from(v).ok())
                .ok_or_else(|| anyhow!("adpcm block truncated"))?;
            left.step_index = reader
                .read(6)
                .ok_or_else(|| anyhow!("adpcm block truncated"))? as i32;
            pcm.push(left.sample);
            if stereo {
                right.sample = reader
                    .read_signed(16)
                    .and_then(|v| i16::try_from(v).ok())
                    .ok_or_else(|| anyhow!("adpcm block truncated"))?;
                right.step_index = reader
                    .read(6)
                    .ok_or_else(|| anyhow!("adpcm block truncated"))? as i32;
                pcm.push(right.sample);
            }
        } else {
            match left.decode_frame(&mut reader, bits_per_sample) {
                Some(s) => pcm.push(s),
                None => break,
            }
            if num_channels == 2 {
                match right.decode_frame(&mut reader, bits_per_sample) {
                    Some(s) => pcm.push(s),
                    None => break,
                }
            }
        }
        sample_num += 1;
    }
    Ok(pcm)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::swf_extract::SwfLibrary;
    use std::path::{Path, PathBuf};

    /// Locate a cached file (`<id>__h<hash>`) under the asset dir.
    fn cached_file(file_id: &str) -> Option<PathBuf> {
        let dir = Path::new(".xenomit-assets/files");
        for entry in std::fs::read_dir(dir).ok()? {
            let p = entry.ok()?.path();
            if p.file_name()?.to_str()?.starts_with(&format!("{file_id}__h")) {
                return Some(p);
            }
        }
        None
    }

    /// Synthesized 2-bit ADPCM mono stream: bit width (00 → 2 bits),
    /// block header (16-bit sample 0, 6-bit index 0), then frames.
    /// Codes 0/2 = step/4 down/up; 1/3 = ±(step+step/4)... just assert
    /// the decoder consumes it and tracks deltas.
    #[test]
    fn adpcm_decoder_roundtrip_shapes() {
        // Build: 2-bit codes; initial sample 0, index 0 (step=7).
        // Frames: code 2 (sign 0, mag 2 = "11" in low bits → delta += step/2+step/2+step/4)
        // For 2-bit samples: 1 sign bit + 1 magnitude bit. Code 1 → delta = step>>2 + step>>1.
        let mut bits = Vec::new();
        let mut acc: u64 = 0;
        let mut nbits = 0u32;
        let push = |v: u64, n: u32, acc: &mut u64, nbits: &mut u32, bits: &mut Vec<u8>| {
            *acc = (*acc << n) | (v & ((1 << n) - 1));
            *nbits += n;
            while *nbits >= 8 {
                bits.push((*acc >> (*nbits - 8)) as u8);
                *acc &= (1 << (*nbits - 8)) - 1;
                *nbits -= 8;
            }
        };
        push(0, 2, &mut acc, &mut nbits, &mut bits); // bits-2 = 0 → 2-bit samples
        push(0, 16, &mut acc, &mut nbits, &mut bits); // initial sample 0
        push(0, 6, &mut acc, &mut nbits, &mut bits); // index 0
        // 8 frames of code 1 (mag bit set, sign 0) → climb; then code 3 (sign+mag) → descend.
        for _ in 0..8 {
            push(0b01, 2, &mut acc, &mut nbits, &mut bits);
        }
        for _ in 0..8 {
            push(0b11, 2, &mut acc, &mut nbits, &mut bits);
        }
        if nbits > 0 {
            bits.push((acc << (8 - nbits)) as u8);
        }

        let pcm = adpcm_decode(&bits, false).expect("decode");
        assert_eq!(pcm[0], 0); // initial sample
        assert_eq!(pcm.len(), 17);
        // Climbed for 8 frames, then descended.
        assert!(pcm[8] > pcm[1]);
        assert!(pcm[16] < pcm[8]);
    }

    #[test]
    fn wav_header_is_wellformed() {
        let wav = wrap_pcm16_wav(&[0, 1000, -1000, 32767], false, 44100);
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
        assert_eq!(u32::from_le_bytes(wav[40..44].try_into().unwrap()), 8);
        assert_eq!(wav.len(), 44 + 8);
    }

    /// Every cached audio SWF: report + try to decode each exported
    /// sound. Skips when the cache dir is absent (CI).
    #[test]
    fn real_sounds_decode() {
        let dir = Path::new(".xenomit-assets/files");
        if !dir.is_dir() {
            return; // graceful skip — no asset cache
        }
        let audio_ids = [
            "combatSounds",
            "damageSounds",
            "miscSounds",
            "portalSounds",
            "uiSounds",
            "voiceSounds",
            "track0",
            "trackMMO",
            "trackEIC",
            "trackVRU",
            "trackBattlemap",
            "trackPirate",
            "trackRZone",
            "trackCombat",
            "track_tunnelOfTerror",
            "trackBL",
        ];
        let mut found = false;
        for id in audio_ids {
            let Some(path) = cached_file(id) else { continue };
            found = true;
            let lib = SwfLibrary::open(&path).expect("swf parses");
            let inventory = sound_inventory(&lib);
            eprintln!("{id}: {} sound(s)", inventory.len());
            for entry in &inventory {
                eprintln!(
                    "  {}: {:?} rate={} stereo={} 16bit={} samples={}",
                    entry.name,
                    entry.compression,
                    entry.sample_rate,
                    entry.is_stereo,
                    entry.is_16_bit,
                    entry.num_samples
                );
            }
            for entry in &inventory {
                extract_sound(&lib, &entry.name)
                    .unwrap_or_else(|e| panic!("{id}/{} decode failed: {e}", entry.name));
            }
        }
        if !found {
            eprintln!("no cached audio files — skipped");
        }
    }

    /// `combatSounds.laser0_0` + `track0.track` specifically (the
    /// sound_dump example targets).
    #[test]
    fn real_laser_and_track_decode() {
        for (file_id, class) in [("combatSounds", "laser0_0"), ("track0", "track")] {
            let Some(path) = cached_file(file_id) else {
                eprintln!("{file_id} not cached — skipped");
                continue;
            };
            let lib = SwfLibrary::open(&path).expect("swf parses");
            let audio = extract_sound(&lib, class).expect("decode");
            assert!(!audio.data.is_empty());
            eprintln!(
                "{file_id}.{class}: {} bytes ({})",
                audio.data.len(),
                audio.file_type
            );
        }
    }
}
