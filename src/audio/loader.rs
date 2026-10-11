//! Audio decode worker — fetches soundbank/track SWFs through the same
//! disk-cache/CDN path as the sprite workers (`AssetManager::file_source`,
//! `fetch_file`) and extracts `DefineSound` payloads on a background
//! thread. The render thread polls [`AudioLoader::done`].

use std::sync::mpsc::{channel, Receiver, Sender};
use std::thread::JoinHandle;

use crate::assets::FileSource;
use crate::swf_extract::sound::{extract_sound, DecodedAudio};
use crate::swf_extract::SwfLibrary;

/// A queued decode job.
#[derive(Clone, Debug)]
pub(crate) enum AudioRequest {
    /// Exported `class` inside the soundbank SWF cached as `file_id`.
    Sfx { file_id: String, class: String },
    /// Music track SWF — the embedded class is always `"track"`
    /// (`AudioManager.playMusic`).
    Music { file_id: String },
}

/// One finished decode.
pub(crate) struct AudioDone {
    pub file_id: String,
    pub class: String,
    /// `Ok` = decoded WAV/MP3 bytes; `Err` = fetch/unsupported/parse
    /// failure (logged once, not retried until the next play request).
    pub result: Result<DecodedAudio, String>,
}

/// Handle to the worker (render-thread end).
pub(crate) struct AudioLoader {
    tx: Option<Sender<AudioRequest>>,
    rx: Receiver<AudioDone>,
    _thread: Option<JoinHandle<()>>,
}

impl AudioLoader {
    /// Spawn the worker. `source` performs cache-hit reads and CDN
    /// fetches exactly like the sprite workers.
    pub(crate) fn spawn(source: FileSource) -> Self {
        let (tx, rx) = channel::<AudioRequest>();
        let (dtx, drx) = channel::<AudioDone>();
        let thread = std::thread::Builder::new()
            .name("audio-decode".into())
            .spawn(move || {
                let rt = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(rt) => Some(rt),
                    Err(e) => {
                        log::warn!("audio loader: no runtime ({e})");
                        None
                    }
                };
                for req in rx {
                    let done = decode(&source, rt.as_ref(), req);
                    if dtx.send(done).is_err() {
                        return;
                    }
                }
            });
        let thread = match thread {
            Ok(t) => Some(t),
            Err(e) => {
                log::warn!("audio loader: spawn failed ({e})");
                None
            }
        };
        Self {
            tx: Some(tx),
            rx: drx,
            _thread: thread,
        }
    }

    /// Queue a decode (worker applies them in order).
    pub(crate) fn request(&self, req: AudioRequest) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(req);
        }
    }

    /// Non-blocking drain of finished decodes.
    pub(crate) fn done(&self) -> impl Iterator<Item = AudioDone> + '_ {
        self.rx.try_iter()
    }
}

fn decode(source: &FileSource, rt: Option<&tokio::runtime::Runtime>, req: AudioRequest) -> AudioDone {
    let (file_id, class) = match &req {
        AudioRequest::Sfx { file_id, class } => (file_id.clone(), class.clone()),
        AudioRequest::Music { file_id } => (file_id.clone(), "track".to_owned()),
    };
    let result = (|| -> anyhow::Result<DecodedAudio> {
        let bytes = source.fetch(rt.ok_or_else(|| anyhow::anyhow!("no tokio runtime"))?, &file_id)?;
        let lib = SwfLibrary::from_bytes(&bytes)?;
        extract_sound(&lib, &class)
    })()
    .map_err(|e| format!("{e:#}"));
    AudioDone {
        file_id,
        class,
        result,
    }
}
