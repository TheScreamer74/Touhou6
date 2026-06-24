//! Native entry point: reads the game archives from disk, builds the game
//! via the shared `th06` library, and runs it (windowed, screenshot, or
//! scripted demo).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use th06::{build_game, Character, GameFiles, RecordPhase};
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

/// Read the high-score table (one `score<TAB>stage<TAB>name` row per line).
fn load_scores(path: &Path) -> Vec<th06::ScoreEntry> {
    let Ok(body) = std::fs::read_to_string(path) else { return Vec::new() };
    body.lines()
        .filter_map(|line| {
            let mut it = line.splitn(3, '\t');
            let score = it.next()?.trim().parse().ok()?;
            let stage = it.next()?.trim().parse().ok()?;
            let name = it.next()?.to_string();
            Some(th06::ScoreEntry { name, score, stage })
        })
        .collect()
}

/// Auto-player input for a full title-to-clear run: tap Shoot to walk the menus,
/// then in a stage hold Shoot and slide horizontally under the boss/nearest
/// enemy (no bullet dodging — relies on god mode to survive).
fn auto_play_input(game: &th06::Game, f: u32) -> Input {
    match game.stage_aim() {
        Some((px, target)) => {
            let mut held = vec![Key::Shoot];
            if let Some(tx) = target {
                if tx < px - 4.0 {
                    held.push(Key::Left);
                } else if tx > px + 4.0 {
                    held.push(Key::Right);
                }
            }
            let pressed: &[Key] = if f % 12 == 0 { &[Key::Shoot] } else { &[] };
            Input::synthetic(&held, pressed)
        }
        // Menus: pulse Shoot to advance title -> char select -> stage, paced so
        // the menu screens are actually visible in a recording.
        None => {
            if f % 48 == 0 {
                Input::synthetic(&[], &[Key::Shoot])
            } else {
                Input::default()
            }
        }
    }
}

/// Encode one segment's PNG sequence (`<frames_dir>/f_%06d.png`) to an mp4 with
/// ffmpeg, then delete the PNG dir to bound disk use. Returns false if ffmpeg
/// is missing or fails.
fn encode_segment(frames_dir: &Path, out_mp4: &Path) -> bool {
    let status = std::process::Command::new("ffmpeg")
        .args(["-y", "-framerate", "60", "-i"])
        .arg(frames_dir.join("f_%06d.png"))
        .args(["-c:v", "libx264", "-pix_fmt", "yuv420p", "-crf", "20"])
        .arg(out_mp4)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
    let ok = matches!(status, Ok(s) if s.success());
    if ok {
        let _ = std::fs::remove_dir_all(frames_dir);
    }
    ok
}

/// Segment directory/file label for a phase, e.g. `00_menu`, `01_stage1`.
fn phase_label(phase: RecordPhase, idx: usize, entered_stage: bool) -> String {
    match phase {
        RecordPhase::Menu if !entered_stage => format!("{idx:02}_menu"),
        RecordPhase::Menu | RecordPhase::Ended => format!("{idx:02}_end"),
        RecordPhase::Stage(s) => format!("{idx:02}_stage{}", s + 1),
    }
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
    let mut record: Option<String> = None;
    let mut record_split: Option<String> = None;
    let mut debug_power: Option<i32> = None;
    let mut debug_score: Option<i64> = None;
    let mut god = false;
    let mut probe_run = false;
    let mut ecl_dump = false;
    let mut warp: Option<bool> = None; // Some(false) = midboss, Some(true) = boss
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
            "--record" => record = Some(args.next().expect("--record <out_dir>")),
            "--record-split" => record_split = Some(args.next().expect("--record-split <out_dir>")),
            "--power" => debug_power = Some(args.next().expect("--power <0-128>").parse().expect("power")),
            "--score" => debug_score = Some(args.next().expect("--score <n>").parse().expect("score")),
            "--god" => god = true,
            "--probe-run" => probe_run = true,
            "--ecl-dump" => ecl_dump = true,
            "--midboss" => warp = Some(false),
            "--boss" => warp = Some(true),
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
    // Headless dumps (screenshot / record / demo) run silent.
    let with_audio = screenshot.is_none()
        && record.is_none()
        && record_split.is_none()
        && demo.is_none()
        && !probe_run
        && !ecl_dump;
    let (textures, mut game) = build_game(&engine, &files, with_audio);

    let hiscore_path = game_dir.join("th06_hiscore.txt");
    game.set_hiscore(load_hiscore(&hiscore_path));
    game.set_hiscore_path(hiscore_path);

    let scores_path = game_dir.join("th06_scores.txt");
    game.set_scores(load_scores(&scores_path));
    game.set_scores_path(scores_path);

    // `--god` makes the player invulnerable (the collide() check reads this).
    // Safe: still single-threaded here, before the game/run loop starts.
    if god {
        unsafe { std::env::set_var("TH06_GOD", "1") };
    }

    // Full-VM oracle: run a stage's ECL from frame 0 with NO input (player fixed
    // at spawn, no shots = no damage), dumping every live bullet per frame in the
    // same format as oracle/vm/oracle_main.cpp. Diff to find any execution-layer
    // divergence from the decomp.
    if ecl_dump {
        // The oracle's player is invincible (no collision); match that so the
        // fixed player doesn't die and clear the field.
        unsafe { std::env::set_var("TH06_GOD", "1") };
        // The full-VM oracle stubs the Gui dialogue (Gui::MsgWait returns false),
        // so its timeline never freezes for boss/midboss conversations. Match that
        // here so the bullet comparison continues through the dialogue points
        // instead of stalling on the real (faithful) text — dialogue timing is a
        // Gui concern the oracle can't model, not an ECL/bullet concern.
        unsafe { std::env::set_var("TH06_NO_DIALOGUE", "1") };
        let ch = [Character::ReimuA, Character::ReimuB, Character::MarisaA, Character::MarisaB][debug_char.min(3)];
        game.debug_start_stage(ch, debug_lives, debug_stage.saturating_sub(1), debug_power, debug_score);
        let dump_enemies = std::env::var_os("DUMP_ENEMIES").is_some();
        for fr in 0..frames {
            game.update(&Input::default());
            let mut lines: Vec<String> = if dump_enemies {
                game.stage_enemies().iter().map(|e| format!("{:.4} {:.4}", e[0], e[1])).collect()
            } else {
                game.stage_bullets()
                    .iter()
                    .map(|b| format!("{:.4} {:.4} {:.4} {:.4}", b[0], b[1], b[2], b[3]))
                    .collect()
            };
            lines.sort();
            println!("F{fr} {}", lines.len());
            for l in &lines {
                println!(" {l}");
            }
        }
        return;
    }

    // Drive a full god-mode run from the title and print the frame at every
    // phase change (no rendering), to measure per-stage clear times and confirm
    // the auto-player can actually reach each stage clear.
    if probe_run {
        unsafe { std::env::set_var("TH06_GOD", "1") };
        game.start_title_bgm();
        let mut last = game.record_phase();
        eprintln!("frame 0: {last:?}");
        for f in 0..frames {
            let input = auto_play_input(&game, f);
            game.update(&input);
            let phase = game.record_phase();
            if phase != last {
                eprintln!("frame {f}: {last:?} -> {phase:?}");
                last = phase;
                if phase == RecordPhase::Ended {
                    break;
                }
            }
        }
        eprintln!("probe done at <= {frames} frames, ended in {last:?}");
        return;
    }

    if scene_arg == "stage" {
        let ch = [Character::ReimuA, Character::ReimuB, Character::MarisaA, Character::MarisaB]
            [debug_char.min(3)];
        game.debug_start_stage(ch, debug_lives, debug_stage.saturating_sub(1), debug_power, debug_score);
        // `--midboss` / `--boss`: fast-forward straight to that fight. The warp
        // must be invulnerable; restore the chosen god state afterwards.
        if let Some(to_boss) = warp {
            let had_god = std::env::var_os("TH06_GOD").is_some();
            unsafe { std::env::set_var("TH06_GOD", "1") };
            if !game.debug_warp(to_boss) {
                eprintln!("warp: target boss never appeared");
            }
            if !had_god {
                unsafe { std::env::remove_var("TH06_GOD") };
            }
        }
    }

    // Full god-mode run from the title, recorded to one mp4 per phase
    // (menu nav, then each stage incl. its clear screen). Each segment is
    // encoded and its PNGs deleted as soon as the phase changes, so peak disk
    // stays around a single stage's frames.
    if let Some(out_root) = record_split.clone() {
        unsafe { std::env::set_var("TH06_GOD", "1") };
        let out_root = PathBuf::from(out_root);
        std::fs::create_dir_all(&out_root).expect("create record dir");
        game.start_title_bgm();
        let textures_ref: Vec<&th06_engine::Texture> = textures.iter().collect();

        let mut idx = 0usize;
        let mut entered_stage = false;
        let mut phase = game.record_phase();
        let mut label = phase_label(phase, idx, entered_stage);
        let mut frames_dir = out_root.join(format!("_f_{label}"));
        std::fs::create_dir_all(&frames_dir).unwrap();
        let mut seg_frame = 0u32;
        let mut tail = 0u32; // frames recorded after the run-ending phase began

        for f in 0..frames {
            // Once the run is over, stop sending input so the auto-player doesn't
            // immediately start a fresh game from the title during the end tail.
            let run_over = phase == RecordPhase::Ended || (phase == RecordPhase::Menu && entered_stage);
            let input = if run_over { Input::default() } else { auto_play_input(&game, f) };
            let frame = game.update(&input);
            let new_phase = game.record_phase();
            if matches!(new_phase, RecordPhase::Stage(_)) {
                entered_stage = true;
            }
            if new_phase != phase {
                println!("frame {f}: {label} done ({seg_frame} frames) -> encoding");
                encode_segment(&frames_dir, &out_root.join(format!("{label}.mp4")));
                idx += 1;
                phase = new_phase;
                seg_frame = 0;
                label = phase_label(phase, idx, entered_stage);
                frames_dir = out_root.join(format!("_f_{label}"));
                std::fs::create_dir_all(&frames_dir).unwrap();
            }
            let pixels = engine.render_to_image(&frame.cmds, &textures_ref, frame.bg.as_ref());
            image::save_buffer(
                frames_dir.join(format!("f_{seg_frame:06}.png")),
                &pixels,
                th06_engine::SCREEN_W,
                th06_engine::SCREEN_H,
                image::ColorType::Rgba8,
            )
            .expect("save frame");
            seg_frame += 1;

            // The run is over once we hit the post-game screens or return to the
            // title after having played a stage; grab a short tail then stop.
            let run_over = phase == RecordPhase::Ended || (phase == RecordPhase::Menu && entered_stage);
            if run_over {
                tail += 1;
                if tail >= 240 {
                    break;
                }
            }
        }
        println!("frame end: {label} done ({seg_frame} frames) -> encoding");
        encode_segment(&frames_dir, &out_root.join(format!("{label}.mp4")));
        println!("split recording written to {}", out_root.display());
        return;
    }

    if let Some(dir) = record.clone() {
        // Record EVERY frame of the auto-played stage to a PNG sequence (encode
        // to video afterwards). Same auto-play as --screenshot: hold Shoot and
        // steer under the boss (TH06_GOD to survive, TH06_NOSHOOT to let the
        // boss cycle every card via timeouts).
        std::fs::create_dir_all(&dir).expect("create record dir");
        let textures_ref: Vec<&th06_engine::Texture> = textures.iter().collect();
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
                if f % 12 == 0 { &[Key::Enter] } else { &[] }
            } else if f % 12 == 0 {
                &[Key::Shoot]
            } else {
                &[]
            };
            let frame = game.update(&Input::synthetic(&held, pressed));
            let pixels = engine.render_to_image(&frame.cmds, &textures_ref, frame.bg.as_ref());
            let path = format!("{dir}/frame_{f:06}.png");
            image::save_buffer(&path, &pixels, th06_engine::SCREEN_W, th06_engine::SCREEN_H, image::ColorType::Rgba8)
                .expect("save record frame");
        }
        println!("recorded {frames} frames to {dir}");
        return;
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
        // TH06_NOSHOOT: steer but never fire — lets a midboss/boss reach its
        // timeout so leave-without-kill and time-out-only spellcards (the ones
        // that use ex-instructions) are exercised. Dialogue is advanced with
        // Enter (which doesn't fire shots) so the fight actually starts.
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
                if f % 12 == 0 { &[Key::Enter] } else { &[] }
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
