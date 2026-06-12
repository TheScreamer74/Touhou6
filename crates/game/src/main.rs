mod anm_vm;
mod title;

use std::collections::HashMap;
use std::path::Path;

use th06_engine::{compose_rgba, Engine, Input};
use th06_formats::anm0::Anm0;
use th06_formats::pbg3::Pbg3;

use title::Title;

/// All files from one PBG3 archive, keyed by entry name.
fn load_archive(path: &Path) -> HashMap<String, Vec<u8>> {
    let data = std::fs::read(path).expect("read archive");
    let archive = Pbg3::parse(&data).expect("parse PBG3");
    archive
        .entries
        .iter()
        .map(|e| (e.name.clone(), archive.extract(e).expect("extract")))
        .collect()
}

/// ANM texture names look like "data/title/title01.png"; archive entries
/// are flat basenames.
fn basename(path: &str) -> &str {
    path.rsplit('/').next().unwrap()
}

/// Write panics (message, location, backtrace) to logs/crash-<timestamp>.log
/// in addition to stderr, so window-mode crashes leave a trace.
fn install_crash_logger() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let backtrace = std::backtrace::Backtrace::force_capture();
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let report = format!(
            "th06 crash report\ntime: {timestamp} (unix)\nversion: {}\n\n{info}\n\nbacktrace:\n{backtrace}\n",
            env!("CARGO_PKG_VERSION"),
        );
        let _ = std::fs::create_dir_all("logs");
        let path = format!("logs/crash-{timestamp}.log");
        if std::fs::write(&path, &report).is_ok() {
            eprintln!("crash report written to {path}");
        }
        default_hook(info);
    }));
}

fn main() {
    install_crash_logger();
    let mut args = std::env::args().skip(1);
    let mut screenshot: Option<String> = None;
    let mut frames = 120u32;
    let mut game_dir = String::from("../TH06 ~ The Embodiment of Scarlet Devil/kouma");
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--screenshot" => screenshot = Some(args.next().expect("--screenshot <out.png>")),
            "--frames" => frames = args.next().expect("--frames <n>").parse().expect("frame count"),
            "--game-dir" => game_dir = args.next().expect("--game-dir <path>"),
            other => panic!("unknown argument: {other}"),
        }
    }

    let tl = load_archive(&Path::new(&game_dir).join("TL.DAT"));

    let anm = Anm0::parse(&tl["title01.anm"]).expect("parse title01.anm");
    let entry = &anm.entries[0];

    let engine = Engine::new();

    // Texture 0: menu background (no alpha mask). Texture 1: menu sprites.
    let (bg_rgba, bg_w, bg_h) = compose_rgba(&tl["title00.jpg"], None);
    let bg_tex = engine.create_texture(&bg_rgba, bg_w, bg_h);
    let alpha = entry.alpha_name.as_deref().map(|n| tl[basename(n)].as_slice());
    let (rgba, w, h) = compose_rgba(&tl[basename(&entry.name)], alpha);
    let menu_tex = engine.create_texture(&rgba, w, h);

    let mut title = Title::new(entry, 0, 1);

    if let Some(out) = screenshot {
        // Settle the entrance animation, then capture one frame.
        let input = Input::default();
        let mut frame = title.update(&input);
        for _ in 1..frames {
            frame = title.update(&input);
        }
        let textures = [&bg_tex, &menu_tex];
        let pixels = engine.render_to_image(&frame.cmds, &textures);
        image::save_buffer(&out, &pixels, th06_engine::SCREEN_W, th06_engine::SCREEN_H, image::ColorType::Rgba8)
            .expect("save screenshot");
        println!("wrote {out}");
    } else {
        engine.run_game(
            "Touhou 6 ~ EoSD (Mac port)",
            vec![bg_tex, menu_tex],
            move |input| title.update(input),
        );
    }
}
