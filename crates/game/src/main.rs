//! Native entry point: reads the game archives from disk, builds the game
//! via the shared `th06` library, and runs it (windowed, screenshot, or
//! scripted demo).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use th06::{build_game, Character, GameFiles};
use th06_engine::{Engine, Frame, Input, Key};

/// All files from one PBG3 archive, keyed by entry name.
fn load_archive(path: &Path) -> HashMap<String, Vec<u8>> {
    use th06_formats::pbg3::Pbg3;
    let data = std::fs::read(path).expect("read archive");
    let archive = Pbg3::parse(&data).expect("parse PBG3");
    archive
        .entries
        .iter()
        .map(|e| (e.name.clone(), archive.extract(e).expect("extract")))
        .collect()
}

/// Read every `*.wav` in the game's `bgm/` directory, keyed by basename.
fn load_bgm(dir: &Path) -> HashMap<String, Vec<u8>> {
    let mut out = HashMap::new();
    let Ok(entries) = std::fs::read_dir(dir) else { return out };
    for e in entries.flatten() {
        let path = e.path();
        if path.extension().and_then(|x| x.to_str()) == Some("wav") {
            if let (Some(name), Ok(data)) = (path.file_name().and_then(|n| n.to_str()), std::fs::read(&path)) {
                out.insert(name.to_string(), data);
            }
        }
    }
    out
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

/// Read the persisted high score (0 if absent/unparseable).
fn load_hiscore(path: &Path) -> i64 {
    std::fs::read_to_string(path).ok().and_then(|s| s.trim().parse().ok()).unwrap_or(0)
}

fn main() {
    install_crash_logger();
    let mut args = std::env::args().skip(1);
    let mut screenshot: Option<String> = None;
    let mut frames = 120u32;
    let mut scene_arg = String::from("title");
    let mut debug_lives: Option<i32> = None;
    let mut debug_stage = 1usize;
    let mut debug_char = 0usize;
    let mut demo: Option<String> = None;
    let mut demo_interval = 300u32;
    let mut game_dir = String::from("../TH06 ~ The Embodiment of Scarlet Devil/kouma");
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--screenshot" => screenshot = Some(args.next().expect("--screenshot <out.png>")),
            "--frames" => frames = args.next().expect("--frames <n>").parse().expect("frame count"),
            "--scene" => scene_arg = args.next().expect("--scene <title|stage>"),
            "--lives" => debug_lives = Some(args.next().expect("--lives <n>").parse().expect("lives")),
            "--stage" => debug_stage = args.next().expect("--stage <1-6>").parse().expect("stage"),
            "--char" => debug_char = args.next().expect("--char <0-3>").parse().expect("char"),
            "--demo" => demo = Some(args.next().expect("--demo <out_dir>")),
            "--demo-interval" => demo_interval = args.next().expect("--demo-interval <n>").parse().expect("interval"),
            "--game-dir" => game_dir = args.next().expect("--game-dir <path>"),
            other => panic!("unknown argument: {other}"),
        }
    }

    let game_dir = PathBuf::from(game_dir);
    let files = GameFiles {
        tl: load_archive(&game_dir.join("TL.DAT")),
        cm: load_archive(&game_dir.join("CM.DAT")),
        st: load_archive(&game_dir.join("ST.DAT")),
        inn: load_archive(&game_dir.join("IN.DAT")),
        st_en: load_archive(&game_dir.join("th06e_ST.DAT")),
        bgm: load_bgm(&game_dir.join("bgm")),
    };

    let engine = Engine::new();
    let (textures, mut game) = build_game(&engine, &files, screenshot.is_none());

    let hiscore_path = game_dir.join("th06_hiscore.txt");
    game.set_hiscore(load_hiscore(&hiscore_path));
    game.set_hiscore_path(hiscore_path);

    if scene_arg == "stage" {
        let ch = [Character::ReimuA, Character::ReimuB, Character::MarisaA, Character::MarisaB]
            [debug_char.min(3)];
        game.debug_start_stage(ch, debug_lives, debug_stage.saturating_sub(1));
    }

    if let Some(dir) = demo.clone() {
        // Drive the full game from the title exactly as a player would:
        // tap Start, then hold Shoot, dumping a PNG every `demo_interval`
        // frames so the real title->stage->boss path is visible headlessly.
        std::fs::create_dir_all(&dir).expect("create demo dir");
        let textures_ref: Vec<&th06_engine::Texture> = textures.iter().collect();
        let mut frame = Frame { cmds: Vec::new(), bg: None, quit: false };
        for f in 0..frames {
            // Title/char-select: tap Start then confirm. In a stage: hold Shoot,
            // steer under the boss/nearest enemy, and pulse Shoot to advance
            // dialogue — so the run actually fights and progresses.
            let input = if f == 3 || f == 15 {
                Input::synthetic(&[], &[Key::Shoot])
            } else if game.stage_aim().is_some() {
                let mut held = vec![Key::Shoot];
                if let Some((px, Some(tx))) = game.stage_aim() {
                    if tx < px - 4.0 {
                        held.push(Key::Left);
                    } else if tx > px + 4.0 {
                        held.push(Key::Right);
                    }
                }
                let pressed: &[Key] = if f % 12 == 0 { &[Key::Shoot] } else { &[] };
                Input::synthetic(&held, pressed)
            } else if f > 3 {
                Input::synthetic(&[Key::Shoot], &[])
            } else {
                Input::default()
            };
            frame = game.update(&input);
            if f % demo_interval == 0 {
                let pixels = engine.render_to_image(&frame.cmds, &textures_ref, frame.bg.as_ref());
                let path = format!("{dir}/frame_{f:05}.png");
                image::save_buffer(&path, &pixels, th06_engine::SCREEN_W, th06_engine::SCREEN_H, image::ColorType::Rgba8)
                    .expect("save demo frame");
                println!("wrote {path}");
            }
        }
        let _ = frame;
    } else if let Some(out) = screenshot {
        // Auto-play: hold Shoot, steer under the boss/nearest enemy, and pulse
        // Shoot to advance dialogue — so headless runs actually fight.
        let mut frame = Frame { cmds: Vec::new(), bg: None, quit: false };
        // TH06_NOSHOOT: steer but never fire — lets a midboss reach its timeout
        // so the leave-without-kill path can be exercised headlessly.
        let no_shoot = std::env::var_os("TH06_NOSHOOT").is_some();
        for f in 0..frames {
            let mut held = if no_shoot { Vec::new() } else { vec![Key::Shoot] };
            if let Some((px, Some(tx))) = game.stage_aim() {
                if tx < px - 4.0 {
                    held.push(Key::Left);
                } else if tx > px + 4.0 {
                    held.push(Key::Right);
                }
            }
            let pressed: &[Key] = if no_shoot {
                &[]
            } else if std::env::var_os("TH06_BOMB").is_some() && f == 40 {
                &[Key::Bomb]
            } else if f % 12 == 0 {
                &[Key::Shoot]
            } else {
                &[]
            };
            frame = game.update(&Input::synthetic(&held, pressed));
        }
        let textures_ref: Vec<&th06_engine::Texture> = textures.iter().collect();
        let pixels = engine.render_to_image(&frame.cmds, &textures_ref, frame.bg.as_ref());
        image::save_buffer(&out, &pixels, th06_engine::SCREEN_W, th06_engine::SCREEN_H, image::ColorType::Rgba8)
            .expect("save screenshot");
        println!("wrote {out}");
    } else {
        game.start_title_bgm();
        engine.run_game("Touhou 6 ~ EoSD (Mac port)", textures, move |input| game.update(input));
    }
}
