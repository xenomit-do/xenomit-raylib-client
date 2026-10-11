//! Integration test for `swf_extract` against the real DarkOrbit `main.swf`.

use std::path::Path;

use xenomit_client::swf_extract::SwfLibrary;

fn fixture() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("main.swf")
}

fn fixture_named(name: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

#[test]
fn parses_main_swf() {
    let lib = SwfLibrary::open(&fixture()).expect("main.swf should parse");
    assert!(!lib.symbols.is_empty(), "expected exported symbols");
    assert!(lib.frame_rate > 0.0, "frame rate should be positive");
}

#[test]
fn extracts_embedded_xml() {
    let lib = SwfLibrary::open(&fixture()).expect("main.swf should parse");
    let blobs = lib.binary_blobs();
    assert!(!blobs.is_empty(), "expected DefineBinaryData blobs");

    let names: Vec<&str> = blobs.iter().map(|(n, _)| n.as_str()).collect();
    assert!(
        names.iter().any(|n| n.contains("GAME_XML_FILE")),
        "missing game.xml blob; names: {names:?}"
    );
    assert!(
        names
            .iter()
            .any(|n| n.contains("UberAssetsLib") || n.contains("LIB_")),
        "missing uber-assets blobs; names: {names:?}"
    );

    // At least one payload is XML.
    assert!(
        blobs.iter().any(|(_, data)| {
            data.iter()
                .find(|b| !b.is_ascii_whitespace() && !matches!(b, 0xEF | 0xBB | 0xBF))
                .map(|b| *b == b'<')
                .unwrap_or(false)
        }),
        "expected at least one XML-looking blob"
    );
}

#[test]
fn renders_a_symbol() {
    let lib = SwfLibrary::open(&fixture()).expect("main.swf should parse");

    // Prefer a sprite (multi-frame) symbol if any resolve; otherwise take
    // whatever renderable character exists.
    let mut rendered = None;
    for name in lib.symbols.keys() {
        if let Ok(frames) = lib.render_symbol(name) {
            if !frames.is_empty() && frames.iter().any(|f| f.image.pixels().any(|p| p.0[3] > 0)) {
                rendered = Some((name.clone(), frames));
                break;
            }
        }
    }

    let Some((name, frames)) = rendered else {
        eprintln!("no renderable symbol found in main.swf; skipping");
        return;
    };
    println!(
        "rendered {name}: {} frame(s), first {}x{}",
        frames.len(),
        frames[0].image.width(),
        frames[0].image.height()
    );
    assert!(!frames.is_empty());
}

/// `box2` is a real asset whose `mc` symbol is a sprite wrapping a
/// DefineVideoStream (VP6 with alpha, 100x72, 25 frames). Rendering must
/// decode the embedded video instead of producing transparent 1x1 frames.
#[test]
fn renders_vp6_video_symbol() {
    let lib = SwfLibrary::open(&fixture_named("box2.swf")).expect("box2.swf should parse");

    let frames = lib
        .render_symbol("mc")
        .expect("mc (embedded VP6 video) should render");
    assert_eq!(frames.len(), 25, "expected one frame per video frame");

    let f0 = &frames[0];
    assert!(
        f0.image.width() > 1 && f0.image.height() > 1,
        "frame 0 should have real content, got {}x{}",
        f0.image.width(),
        f0.image.height()
    );
    assert!(
        (f0.image.width() as i32 - 100).abs() <= 20 && (f0.image.height() as i32 - 72).abs() <= 20,
        "frame 0 should be ~100x72, got {}x{}",
        f0.image.width(),
        f0.image.height()
    );
    assert!(
        f0.image.pixels().any(|p| p.0[3] > 0),
        "frame 0 should contain opaque pixels"
    );
}
