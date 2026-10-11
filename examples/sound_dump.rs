//! `sound_dump <swf> <class|--list> <out.wav|out.mp3>` — extract a
//! `DefineSound` from an audio SWF to a file raylib/any player can
//! open, for offline audio verification.
//!
//! Examples:
//!   cargo run --example sound_dump -- .xenomit-assets/files/combatSounds__h<hash> laser0_0 target/audio_check/laser0_0.wav
//!   cargo run --example sound_dump -- .xenomit-assets/files/track0__h<hash> track target/audio_check/track0.mp3
//!   cargo run --example sound_dump -- <swf> --list ignored  # list exports
//!
//! `<swf>` may also be a bare resources.xml file id — resolved against
//! `.xenomit-assets/files/<id>__h*`.

use std::path::{Path, PathBuf};
use xenomit_client::swf_extract::sound::{extract_sound, sound_inventory};
use xenomit_client::swf_extract::SwfLibrary;

fn resolve_swf(arg: &str) -> PathBuf {
    let p = Path::new(arg);
    if p.is_file() {
        return p.to_path_buf();
    }
    // Bare file id -> newest cached `<id>__h<hash>` blob.
    let dir = Path::new(".xenomit-assets/files");
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with(&format!("{arg}__h")) {
                return entry.path();
            }
        }
    }
    eprintln!("cannot resolve {arg:?} (not a file, no {arg}__h* in .xenomit-assets/files)");
    std::process::exit(2);
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() < 2 {
        eprintln!("usage: sound_dump <swf|file-id> <class|--list> [out.wav|out.mp3]");
        std::process::exit(2);
    }
    let path = resolve_swf(&args[0]);
    let lib = SwfLibrary::open(&path).unwrap_or_else(|e| {
        eprintln!("{}: {e:#}", path.display());
        std::process::exit(1);
    });

    if args[1] == "--list" {
        for s in sound_inventory(&lib) {
            println!(
                "{}\t{:?}\trate={} stereo={} 16bit={} samples={}",
                s.name, s.compression, s.sample_rate, s.is_stereo, s.is_16_bit,
                s.num_samples
            );
        }
        return;
    }

    let class = &args[1];
    let audio = extract_sound(&lib, class).unwrap_or_else(|e| {
        eprintln!("{class}: {e:#}");
        std::process::exit(1);
    });
    let out = args.get(2).cloned().unwrap_or_else(|| {
        format!("{}.{}", class, audio.file_type)
    });
    if let Some(parent) = Path::new(&out).parent().filter(|p| !p.as_os_str().is_empty()) {
        let _ = std::fs::create_dir_all(parent);
    }
    std::fs::write(&out, &audio.data).unwrap_or_else(|e| {
        eprintln!("write {out}: {e}");
        std::process::exit(1);
    });
    println!(
        "{} ({}, {} bytes) -> {out}",
        class,
        audio.file_type,
        audio.data.len()
    );
}
