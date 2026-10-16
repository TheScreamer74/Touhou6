//! Native entry point: reads the game archives from disk, builds the game
//! via the shared `th06` library, and runs it (windowed, screenshot, or
//! scripted demo).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use th06::{Character, Difficulty, GameFiles, RecordPhase, build_game};
use th06_engine::{Engine, Frame, Input, Key};
mod capture;

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
type BgmFiles = (HashMap<String, Vec<u8>>, HashMap<String, (u32, u32)>);
fn load_bgm(dir: &Path) -> BgmFiles {
    let mut out = HashMap::new();
    let mut pos = HashMap::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return (out, pos);
    };
    for e in entries.flatten() {
        let path = e.path();
        if path.extension().and_then(|x| x.to_str()) != Some("wav") {
            continue;
        }
        let Some(name) = path
            .file_name()
            .and_then(|n| n.to_str())
            .map(str::to_string)
        else {
            continue;
        };
        let Ok(data) = std::fs::read(&path) else {
            continue;
        };
        // Sibling `.pos`: 2 little-endian u32 (loop_start, loop_end) in frames
        // (decomp SoundPlayer::LoadPos). Present in a standard TH06 install.
        if let Ok(p) = std::fs::read(path.with_extension("pos"))
            && p.len() >= 8
        {
            let s = u32::from_le_bytes([p[0], p[1], p[2], p[3]]);
            let ed = u32::from_le_bytes([p[4], p[5], p[6], p[7]]);
            pos.insert(name.clone(), (s, ed));
        }
        out.insert(name, data);
    }
    (out, pos)
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
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0)
}

/// Read the high-score table (one `score<TAB>stage<TAB>name` row per line).
fn load_scores(path: &Path) -> Vec<th06::ScoreEntry> {
    let Ok(body) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
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

/// Auto-player input for a full title-to-clear run. In the menus it taps Shoot
/// to advance; in a stage it dodges: it scores nine candidate moves (stay + the
/// eight directions) by how close the nearest bullet comes over a short
/// look-ahead, holds Shoot, steps toward the safest, drifts toward items / under
/// the target when comfortable, and bombs when even the safest move is tight.
/// No god mode needed to survive most of a stage.
fn auto_play_input(game: &th06::Game, f: u32) -> Input {
    let Some((p, bombs)) = game.stage_player() else {
        // Menus: pulse Shoot to advance title -> char select -> stage.
        return if f.is_multiple_of(48) {
            Input::synthetic(&[], &[Key::Shoot])
        } else {
            Input::default()
        };
    };

    // Project each bullet linearly: (x, y, vx, vy).
    let bullets = game.stage_bullets();
    let vels: Vec<[f32; 4]> = bullets
        .iter()
        .map(|b| [b[0], b[1], b[2].cos() * b[3], b[2].sin() * b[3]])
        .collect();

    const SPD: f32 = 4.0; // unfocused move speed (conservative for both characters)
    const H: i32 = 22; // look-ahead frames
    // Player clamp bounds (Stage::clamp): x [12, 372], y [20, 428].
    let d = 1.0 / 2f32.sqrt();
    let moves: [[f32; 2]; 9] = [
        [0.0, 0.0],
        [-1.0, 0.0],
        [1.0, 0.0],
        [0.0, -1.0],
        [0.0, 1.0],
        [-d, -d],
        [d, -d],
        [-d, d],
        [d, d],
    ];
    // (min bullet distance over the horizon, the move, the end position).
    let mut scored: Vec<(f32, [f32; 2], [f32; 2])> = Vec::with_capacity(9);
    for m in moves {
        let mut mind = f32::MAX;
        let mut end = p;
        for t in 1..=H {
            let tx = (p[0] + m[0] * SPD * t as f32).clamp(12.0, 372.0);
            let ty = (p[1] + m[1] * SPD * t as f32).clamp(20.0, 428.0);
            if t == H {
                end = [tx, ty];
            }
            for v in &vels {
                let bx = v[0] + v[2] * t as f32;
                let by = v[1] + v[3] * t as f32;
                let dd = (bx - tx) * (bx - tx) + (by - ty) * (by - ty);
                if dd < mind {
                    mind = dd;
                }
            }
        }
        scored.push((mind.sqrt(), m, end));
    }

    let safest = scored
        .iter()
        .cloned()
        .fold(scored[0], |a, b| if b.0 > a.0 { b } else { a });

    const BOMB_DIST: f32 = 10.0; // even the best escape is this tight -> bomb
    const SAFE: f32 = 48.0; // comfortable enough to go for goals

    // Goal x: the nearest item, else the target (boss / lowest enemy).
    let goal_x = game
        .stage_items()
        .into_iter()
        .min_by(|a, b| (a[1] - p[1]).abs().total_cmp(&(b[1] - p[1]).abs()))
        .map(|it| it[0])
        .or_else(|| game.stage_aim().and_then(|(_, t)| t));

    let chosen = {
        let comfy: Vec<&(f32, [f32; 2], [f32; 2])> = scored.iter().filter(|s| s.0 > SAFE).collect();
        if comfy.is_empty() {
            safest.1 // under threat: take the safest escape
        } else if let Some(gx) = goal_x {
            // comfortable: pick the move whose end position sits nearest the goal x
            comfy
                .iter()
                .min_by(|a, b| (a.2[0] - gx).abs().total_cmp(&(b.2[0] - gx).abs()))
                .unwrap()
                .1
        } else {
            comfy
                .iter()
                .find(|s| s.1 == [0.0, 0.0])
                .map_or(safest.1, |s| s.1)
        }
    };

    let mut held = vec![Key::Shoot];
    if chosen[0] < -0.01 {
        held.push(Key::Left);
    } else if chosen[0] > 0.01 {
        held.push(Key::Right);
    }
    if chosen[1] < -0.01 {
        held.push(Key::Up);
    } else if chosen[1] > 0.01 {
        held.push(Key::Down);
    }
    let pressed: &[Key] = if safest.0 < BOMB_DIST && bombs > 0 {
        &[Key::Bomb]
    } else {
        &[]
    };
    Input::synthetic(&held, pressed)
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
    let mut difficulty = Difficulty::Normal;
    let mut replay_file: Option<String> = None;
    let mut replay_dump: Option<String> = None;
    let mut replay_details: Option<String> = None;
    let mut replay_effects: Option<String> = None;
    let mut replay_pools: Option<String> = None;
    let mut god = false;
    let mut autoplay = false;
    let mut probe_run = false;
    let mut ecl_dump = false;
    let mut color_mode: Option<bool> = None;
    let mut language = None;
    let mut hitbox = None;
    let mut hardware_texture_blending: Option<bool> = None;
    let mut capture_seed: Option<u16> = None;
    let mut score_fixture: Option<(u64, i64)> = None;
    let mut capture_dir: Option<String> = None;
    let mut capture_frames = std::collections::BTreeSet::new();
    let mut capture_pool_frames = std::collections::BTreeSet::new();
    let mut complete_reference = false;
    let mut capture_clock: Option<String> = None;
    let mut input_script: Option<String> = None;
    let mut warp: Option<bool> = None; // Some(false) = midboss, Some(true) = boss
    let mut game_dir = String::from("../TH06 ~ The Embodiment of Scarlet Devil/kouma");
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--screenshot" => screenshot = Some(args.next().expect("--screenshot <out.png>")),
            "--frames" => {
                frames = args
                    .next()
                    .expect("--frames <n>")
                    .parse()
                    .expect("frame count")
            }
            "--scene" => scene_arg = args.next().expect("--scene <title|stage>"),
            "--lives" => {
                debug_lives = Some(args.next().expect("--lives <n>").parse().expect("lives"))
            }
            "--stage" => debug_stage = args.next().expect("--stage <1-7>").parse().expect("stage"),
            "--char" => debug_char = args.next().expect("--char <0-3>").parse().expect("char"),
            "--demo" => demo = Some(args.next().expect("--demo <out_dir>")),
            "--demo-interval" => {
                demo_interval = args
                    .next()
                    .expect("--demo-interval <n>")
                    .parse()
                    .expect("interval")
            }
            "--record" => record = Some(args.next().expect("--record <out_dir>")),
            "--record-split" => record_split = Some(args.next().expect("--record-split <out_dir>")),
            "--power" => {
                debug_power = Some(
                    args.next()
                        .expect("--power <0-128>")
                        .parse()
                        .expect("power"),
                )
            }
            "--score" => {
                debug_score = Some(args.next().expect("--score <n>").parse().expect("score"))
            }
            "--score-fixture" => {
                let value = args.next().expect("--score-fixture <present>:<score>");
                let (present, score) = value.split_once(':').expect("score fixture present:score");
                score_fixture = Some((
                    present.parse().expect("fixture present"),
                    score.parse().expect("fixture score"),
                ));
            }
            "--difficulty" => {
                let value = args
                    .next()
                    .expect("--difficulty <easy|normal|hard|lunatic|extra>");
                difficulty = match value.to_lowercase().as_str() {
                    "easy" | "0" => Difficulty::Easy,
                    "normal" | "1" => Difficulty::Normal,
                    "hard" | "2" => Difficulty::Hard,
                    "lunatic" | "3" => Difficulty::Lunatic,
                    "extra" | "4" => Difficulty::Extra,
                    _ => panic!("invalid difficulty: {value}"),
                };
            }
            "--replay" => replay_file = Some(args.next().expect("--replay <path.rpy>")),
            "--replay-dump" => replay_dump = Some(args.next().expect("--replay-dump <out.csv>")),
            "--replay-details" => {
                replay_details = Some(args.next().expect("--replay-details <bullets.csv>"))
            }
            "--replay-effects" => {
                replay_effects = Some(args.next().expect("--replay-effects <effects.csv>"))
            }
            "--replay-pools" => {
                replay_pools = Some(args.next().expect("--replay-pools <out.jsonl>"))
            }
            "--god" => god = true,
            "--autoplay" => autoplay = true,
            "--probe-run" => probe_run = true,
            "--ecl-dump" => ecl_dump = true,
            "--language" => {
                let value = args.next().expect("--language <ja|fr|en|de|nl>");
                language = Some(match value.as_str() {
                    "ja" => th06::localization::Language::Original,
                    "fr" => th06::localization::Language::French,
                    "en" => th06::localization::Language::English,
                    "de" => th06::localization::Language::German,
                    "nl" => th06::localization::Language::Dutch,
                    _ => panic!("invalid language: {value}"),
                });
            }
            "--hitbox" => {
                let value = args.next().expect("--hitbox <off|focus|always>");
                hitbox = Some(match value.as_str() {
                    "off" => th06::options::Hitbox::Off,
                    "focus" => th06::options::Hitbox::Focus,
                    "always" => th06::options::Hitbox::Always,
                    _ => panic!("invalid hitbox: {value}"),
                });
            }
            "--color-16bit" => color_mode = Some(true),
            "--color-32bit" => color_mode = Some(false),
            "--hardware-texture-blending" => hardware_texture_blending = Some(true),
            "--software-texture-blending" => hardware_texture_blending = Some(false),
            "--seed" => {
                let value = args.next().expect("--seed <u16|0xhex>");
                capture_seed = Some(if let Some(hex) = value.strip_prefix("0x") {
                    u16::from_str_radix(hex, 16).expect("seed")
                } else {
                    value.parse().expect("seed")
                });
            }
            "--capture-dir" => capture_dir = Some(args.next().expect("--capture-dir <directory>")),
            "--capture-frames" => {
                capture_frames = capture::frames(&args.next().expect("--capture-frames <1,60,...>"))
                    .expect("capture frames")
            }
            "--capture-pool-frames" => {
                capture_pool_frames =
                    capture::frames(&args.next().expect("--capture-pool-frames <1,60,...>"))
                        .expect("pool frames")
            }
            "--complete-reference" => complete_reference = true,
            "--capture-clock" => {
                capture_clock = Some(args.next().expect("--capture-clock <replay|update>"))
            }
            "--input-script" => input_script = Some(args.next().expect("--input-script <path>")),
            "--midboss" => warp = Some(false),
            "--boss" => warp = Some(true),
            "--game-dir" => game_dir = args.next().expect("--game-dir <path>"),
            other => panic!("unknown argument: {other}"),
        }
    }

    let game_dir = PathBuf::from(game_dir);
    let (bgm, mut bgm_pos) = load_bgm(&game_dir.join("bgm"));
    let md = if game_dir.join("MD.DAT").exists() {
        load_archive(&game_dir.join("MD.DAT"))
    } else {
        HashMap::new()
    };
    let inn = load_archive(&game_dir.join("IN.DAT"));
    for (name, bytes) in md
        .iter()
        .chain(inn.iter())
        .filter(|(n, b)| n.ends_with(".pos") && b.len() >= 8)
    {
        bgm_pos
            .entry(name.replace(".pos", ".wav"))
            .or_insert_with(|| {
                (
                    u32::from_le_bytes(bytes[..4].try_into().unwrap()),
                    u32::from_le_bytes(bytes[4..8].try_into().unwrap()),
                )
            });
    }
    let files = GameFiles {
        tl: load_archive(&game_dir.join("TL.DAT")),
        cm: load_archive(&game_dir.join("CM.DAT")),
        st: load_archive(&game_dir.join("ST.DAT")),
        inn: inn.clone(),
        st_en: if game_dir.join("th06e_ST.DAT").exists() {
            load_archive(&game_dir.join("th06e_ST.DAT"))
        } else {
            Default::default()
        },
        ed: if game_dir.join("ED.DAT").exists() {
            load_archive(&game_dir.join("ED.DAT"))
        } else {
            Default::default()
        },
        music_comments: std::fs::read(game_dir.join("musiccmt.txt"))
            .ok()
            .or_else(|| {
                md.get("musiccmt.txt")
                    .or_else(|| inn.get("musiccmt.txt"))
                    .cloned()
            })
            .unwrap_or_default(),
        bgm,
        bgm_pos,
        midi: md
            .iter()
            .filter(|(name, _)| name.ends_with(".mid"))
            .map(|(name, bytes)| (name.clone(), bytes.clone()))
            .collect(),
        soundfont: std::env::var_os("TH06_SOUNDFONT")
            .map(PathBuf::from)
            .into_iter()
            .chain([game_dir.join("gm.sf2")])
            .find_map(|path| std::fs::read(path).ok()),
        text_font: std::env::var_os("TH06_FONT")
            .map(PathBuf::from)
            .into_iter()
            .chain([game_dir.join("msgothic.ttc"), game_dir.join("msgothic.ttf")])
            .find_map(|path| std::fs::read(path).ok()),
    };

    th06::validate_game_files(&files).expect("validate game data");
    let engine = Engine::new();
    // Headless dumps (screenshot / record / demo) run silent.
    let with_audio = screenshot.is_none()
        && record.is_none()
        && record_split.is_none()
        && demo.is_none()
        && !probe_run
        && replay_dump.is_none()
        && replay_pools.is_none()
        && capture_dir.is_none()
        && !ecl_dump;
    if let Some(value) = language {
        th06::localization::set_language(value);
    }
    let (mut textures, mut game) = build_game(&engine, &files, with_audio);
    if let Ok(path) = std::env::var("TH06_TEXT_FIT_AUDIT") {
        // Diagnostic: measure every baked translated text against its canvas.
        let dump = std::env::var("TH06_TEXT_FIT_DUMP").ok();
        let rows: Vec<_> = th06::localization::audit_text_fit()
            .into_iter()
            .enumerate()
            .map(|(index, row)| {
                if let (Some(dir), Some((w, h, rgba))) = (&dump, &row.baked) {
                    let name = format!("{dir}/{index:05}-{}.png", row.language);
                    image::save_buffer(&name, rgba, *w, *h, image::ColorType::Rgba8)
                        .expect("write text fit image");
                }
                serde_json::json!({
                    "language": row.language, "source": row.source, "text": row.text,
                    "canvas": row.canvas, "natural": row.natural,
                    "horizontal_scale": row.horizontal_scale,
                    "vertically_clipped": row.vertically_clipped,
                    "clipped_ink_pixels": row.clipped_ink_pixels,
                })
            })
            .collect();
        std::fs::write(
            &path,
            serde_json::to_vec_pretty(&rows).expect("serialize text fit"),
        )
        .expect("write text fit audit");
        return;
    }

    let hiscore_path = game_dir.join("th06_hiscore.txt");
    game.set_hiscore(load_hiscore(&hiscore_path));
    game.set_hiscore_path(hiscore_path);

    let scores_path = game_dir.join("th06_scores.txt");
    game.set_scores(load_scores(&scores_path));
    game.set_scores_path(scores_path);
    game.configure_storage(&game_dir.to_string_lossy());
    // Explicit capture settings override persisted preferences without writing
    // them back; initial recipes were already baked in the requested language.
    if let Some(value) = language {
        game.options.language = value;
    }
    if let Some(value) = hitbox {
        game.options.hitbox = value;
    }
    if let Some(value) = hardware_texture_blending {
        game.options.hardware_texture_blending = value;
    }
    if let Some(color_16bit) = color_mode {
        game.options.color_16bit = color_16bit;
        game.set_diagnostic_color_mode(color_16bit);
    }
    game.set_difficulty(difficulty);
    if let Some(seed) = capture_seed {
        game.debug_set_stage_seed(seed);
    }
    if let Some((present, score)) = score_fixture {
        game.debug_set_score_fixture(present, score);
    }
    // `--god` makes the player invulnerable (the collide() check reads this).
    // Safe: still single-threaded here, before the game/run loop starts.
    // `--autoplay` implies god (the windowed demo shouldn't die); the dodging
    // auto-player's no-god survival is exercised via `--probe-run`.
    if god || autoplay {
        unsafe { std::env::set_var("TH06_GOD", "1") };
    }

    if let Some(path) = replay_file {
        let bytes = std::fs::read(path).expect("read replay");
        let replay = th06_formats::replay::Replay::parse(&bytes).expect("parse replay");
        let index = replay
            .stages
            .iter()
            .position(Option::is_some)
            .expect("replay stage");
        game.start_replay(replay, index).expect("start replay");
    }

    if let Some(path) = replay_dump {
        use std::io::Write;
        let mut script = input_script
            .map(|path| {
                capture::Script::parse(&std::fs::read_to_string(path).expect("read input script"))
                    .expect("parse input script")
            })
            .unwrap_or_default();
        let mut terminal_out = std::io::BufWriter::new(
            std::fs::File::create(format!("{path}.terminal.jsonl")).expect("create terminal dump"),
        );
        let mut lifecycle = std::io::BufWriter::new(
            std::fs::File::create(format!("{path}.lifecycle.csv")).expect("create lifecycle dump"),
        );
        writeln!(
            lifecycle,
            "update,frame,stage,player_state,respawn,grace,invuln,bomb,bomb_timer,retry,game_menu"
        )
        .unwrap();
        let mut out =
            std::io::BufWriter::new(std::fs::File::create(path).expect("create replay dump"));
        let mut details = replay_details.map(|path| {
            let mut file =
                std::io::BufWriter::new(std::fs::File::create(path).expect("create bullet dump"));
            writeln!(file, "frame,slot,x,y,angle,speed,vx,vy,state,timer").unwrap();
            file
        });
        let mut effects = replay_effects.map(|path| {
            let mut file =
                std::io::BufWriter::new(std::fs::File::create(path).expect("create effect dump"));
            writeln!(file, "frame,slot,timer,callback,sprite,x,y,z").unwrap();
            file
        });
        let mut pools = replay_pools.map(|path| {
            std::io::BufWriter::new(std::fs::File::create(path).expect("create pools dump"))
        });
        writeln!(out, "frame,stage,x,y,lives,bombs,power,score,graze_stage,graze_total,rank,subrank,rng,bullets,lasers,items,particle_timer").unwrap();
        for update in 0..frames {
            game.update(&script.input(update));
            let terminal = game.take_replay_reference_terminal();
            if let Some(terminal) = &terminal {
                writeln!(terminal_out, "{}", terminal.json_line().trim_end()).unwrap();
            }
            let Some(state) = game
                .replay_reference_state()
                .or_else(|| terminal.as_ref().map(|end| end.state.clone()))
            else {
                break;
            };
            writeln!(out, "{state}").unwrap();
            if let Some(state) = game.diagnostic_lifecycle() {
                writeln!(lifecycle, "{update},{state}").unwrap();
            } else if let Some(end) = &terminal {
                writeln!(
                    lifecycle,
                    "{update},{},{},{}",
                    end.frame, end.stage, end.lifecycle
                )
                .unwrap();
            }
            let checkpoint: u32 = state.split(',').next().unwrap().parse().unwrap();
            if (terminal.is_some()
                || capture_frames.is_empty()
                || capture_frames.contains(&checkpoint))
                && let Some(file) = &mut pools
                && let Some(snapshot) = terminal
                    .as_ref()
                    .map(|end| end.pools.clone())
                    .or_else(|| game.replay_reference_pools())
            {
                writeln!(file, "{}", snapshot.trim_end()).unwrap();
            }
            if let Some(file) = &mut details
                && let Some(bullets) = game.replay_reference_bullets()
            {
                file.write_all(bullets.as_bytes()).unwrap();
            }
            if let Some(file) = &mut effects
                && let Some(effects) = game.replay_reference_effects()
            {
                file.write_all(effects.as_bytes()).unwrap();
            }
        }
        return;
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
        let ch = [
            Character::ReimuA,
            Character::ReimuB,
            Character::MarisaA,
            Character::MarisaB,
        ][debug_char.min(3)];
        game.debug_start_stage(
            ch,
            debug_lives,
            debug_stage.saturating_sub(1),
            debug_power,
            debug_score,
        );
        let dump_enemies = std::env::var_os("DUMP_ENEMIES").is_some();
        game.debug_set_stage_seed(0x1234);
        for fr in 0..frames {
            game.update(&Input::default());
            let mut lines: Vec<String> = if dump_enemies {
                game.stage_enemies()
                    .iter()
                    .map(|e| format!("{:.4} {:.4}", e[0], e[1]))
                    .collect()
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
        // Respect --god so `--probe-run` alone exercises the dodging auto-player
        // for real (no invulnerability), which is the point of a no-god probe.
        if god {
            unsafe { std::env::set_var("TH06_GOD", "1") };
        }
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
        let ch = [
            Character::ReimuA,
            Character::ReimuB,
            Character::MarisaA,
            Character::MarisaB,
        ][debug_char.min(3)];
        game.debug_start_stage(
            ch,
            debug_lives,
            debug_stage.saturating_sub(1),
            debug_power,
            debug_score,
        );
        if let Some(seed) = capture_seed {
            game.debug_set_stage_seed(seed);
        }
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

    if capture_dir.is_some() || replay_pools.is_some() {
        use std::io::Write;
        assert!(
            !complete_reference || capture_dir.is_some(),
            "--complete-reference requires --capture-dir"
        );
        assert!(
            capture_dir.is_none() || !capture_frames.is_empty(),
            "--capture-dir requires --capture-frames"
        );
        let replay_clock = match capture_clock.as_deref() {
            None => game.replay_reference_state().is_some(),
            Some("replay") => true,
            Some("update") => false,
            Some(other) => panic!("unknown capture clock: {other}"),
        };
        let mut script = input_script
            .map(|path| {
                capture::Script::parse(&std::fs::read_to_string(path).expect("read input script"))
                    .expect("parse input script")
            })
            .unwrap_or_default();
        let directory = capture_dir.map(PathBuf::from);
        let mut report = directory.as_ref().map(|directory| {
            std::fs::create_dir_all(directory).expect("create capture directory");
            let mut file = std::io::BufWriter::new(
                std::fs::File::create(directory.join("checkpoints.csv")).expect("capture report"),
            );
            writeln!(file, "update,clock,checkpoint,scene,color_bits,image,draw_frame,draw_stage,draw_scene,state").unwrap();
            file
        });
        let mut options = directory.as_ref().map(|directory| {
            let mut file = std::io::BufWriter::new(
                std::fs::File::create(directory.join("options.csv")).expect("options trace"),
            );
            writeln!(file, "update,scene,life_count,bomb_count,color_16bit,music_mode,play_sounds,default_difficulty,windowed").unwrap();
            file
        });
        let mut stages = directory.as_ref().map(|directory| {
            let mut file = std::io::BufWriter::new(
                std::fs::File::create(directory.join("stages.csv")).expect("stage trace"),
            );
            writeln!(file, "update,active_calc,stage,x,y,lives,bombs,power,score,graze_stage,graze_total,rank,subrank,rng,bullets,lasers,items,particle_timer").unwrap();
            file
        });
        let mut transitions = directory.as_ref().map(|directory| {
            let mut file = std::io::BufWriter::new(
                std::fs::File::create(directory.join("transitions.csv")).expect("transition trace"),
            );
            writeln!(file, "update,scene,frame,stage,pause_state,pause_timer,retry_state,retry_timer,game_menu,retry").unwrap();
            file
        });
        let mut navigation = directory.as_ref().map(|directory| {
            let mut file = std::io::BufWriter::new(
                std::fs::File::create(directory.join("navigation.csv")).expect("navigation trace"),
            );
            writeln!(file, "update,menu_state,cursor,timer").unwrap();
            file
        });
        let mut results = directory.as_ref().map(|directory| {
            let mut file = std::io::BufWriter::new(
                std::fs::File::create(directory.join("results.csv")).expect("results trace"),
            );
            writeln!(file, "update,state,timer,cursor,cheat_step,progress").unwrap();
            file
        });
        let mut boss_ui = directory.as_ref().map(|directory| {
            let mut file = std::io::BufWriter::new(
                std::fs::File::create(directory.join("boss-ui.csv")).expect("boss UI trace"),
            );
            writeln!(file, "update,frame,stage,boss_present,health_bar_state,bar_target,bar_display,opacity,lives,seconds,last_draw_seconds,dialogue").unwrap();
            file
        });
        let mut lifecycle = directory.as_ref().map(|directory| {
            let mut file = std::io::BufWriter::new(
                std::fs::File::create(directory.join("lifecycle.csv")).expect("lifecycle trace"),
            );
            writeln!(file, "update,frame,stage,player_state,respawn,grace,invuln,bomb,bomb_timer,retry,game_menu").unwrap();
            file
        });
        let mut pools = replay_pools.map(|path| {
            std::io::BufWriter::new(std::fs::File::create(path).expect("create pools dump"))
        });
        let terminal_path = directory
            .as_ref()
            .map(|directory| directory.join("terminal.jsonl"));
        let mut terminal_out = terminal_path.map(|path| {
            std::io::BufWriter::new(std::fs::File::create(path).expect("create terminal dump"))
        });
        // Record simulation and rendering from the same execution. Pool
        // selection is independent of PNG selection, so a complete comparison
        // does not require an image for every simulation checkpoint.
        let mut reference_out = directory.as_ref().filter(|_| complete_reference).map(|directory| {
            let mut file = std::io::BufWriter::new(
                std::fs::File::create(directory.join("replay-state.csv"))
                    .expect("create complete replay dump"),
            );
            writeln!(file, "frame,stage,x,y,lives,bombs,power,score,graze_stage,graze_total,rank,subrank,rng,bullets,lasers,items,particle_timer").unwrap();
            file
        });
        let mut reference_terminal =
            directory
                .as_ref()
                .filter(|_| complete_reference)
                .map(|directory| {
                    std::io::BufWriter::new(
                        std::fs::File::create(directory.join("replay-state.csv.terminal.jsonl"))
                            .expect("create complete terminal dump"),
                    )
                });
        let all_pool_frames = pools.is_some() && capture_frames.is_empty();
        let mut pending = capture_frames;
        let mut pending_pools = capture_pool_frames;
        for update in 1..=frames {
            // Like refcap's synthetic timeGetTime hook, this clock describes
            // presentation cadence only. Gameplay still consumes replay ticks.
            game.set_diagnostic_draw_millis(((u64::from(update) - 1) * 1000 / 60) as u32);
            // Original Present observes the post-calc replay frame, but its
            // backbuffer was drawn from the state preceding those 1–3 ticks.
            let draw_scene = game.diagnostic_scene();
            let draw_key = game.replay_reference_state().map(|state| {
                let mut fields = state.split(',');
                (
                    fields.next().unwrap().to_owned(),
                    fields.next().unwrap().to_owned(),
                )
            });
            let frame = game.update(&script.input(update - 1));
            if directory.is_some() {
                // The original retains color between Presents. Every draw
                // chain contributes, even when no PNG checkpoint is requested.
                engine.apply_texture_replacements(&mut textures);
                let textures_ref = textures.iter().collect::<Vec<_>>();
                engine.advance_frame(&frame.cmds, &textures_ref, frame.bg.as_ref());
            }
            if let (Some(file), Some(state)) = (&mut stages, game.diagnostic_stage_state()) {
                writeln!(file, "{update},{state}").unwrap();
            }
            if let Some(file) = &mut options {
                let options = &game.options;
                writeln!(
                    file,
                    "{update},{},{},{},{},{},{},{},{}",
                    game.diagnostic_scene(),
                    options.life_count,
                    options.bomb_count,
                    options.color_16bit as u8,
                    options.music_mode,
                    options.play_sounds as u8,
                    options.default_difficulty,
                    options.windowed as u8
                )
                .unwrap();
            }
            let terminal = game.take_replay_reference_terminal();
            if let Some(file) = &mut transitions {
                writeln!(file, "{},{}", update, game.diagnostic_transition()).unwrap();
            }
            if let (Some(file), Some(state)) = (&mut navigation, game.diagnostic_navigation()) {
                writeln!(file, "{update},{state}").unwrap();
            }
            if let (Some(file), Some(state)) = (&mut results, game.diagnostic_results()) {
                writeln!(file, "{update},{state}").unwrap();
            }
            if let (Some(file), Some(state)) = (&mut boss_ui, game.diagnostic_boss_ui()) {
                writeln!(file, "{update},{state}").unwrap();
            }
            if let Some(file) = &mut lifecycle {
                if let Some(state) = game.diagnostic_lifecycle() {
                    writeln!(file, "{update},{state}").unwrap();
                } else if let Some(end) = &terminal {
                    writeln!(
                        file,
                        "{update},{},{},{}",
                        end.frame, end.stage, end.lifecycle
                    )
                    .unwrap();
                }
            }
            if let (Some(terminal), Some(file)) = (&terminal, &mut terminal_out) {
                writeln!(file, "{}", terminal.json_line().trim_end()).unwrap();
            }
            if let (Some(terminal), Some(file)) = (&terminal, &mut reference_terminal) {
                writeln!(file, "{}", terminal.json_line().trim_end()).unwrap();
            }
            let state = game
                .replay_reference_state()
                .or_else(|| terminal.as_ref().map(|end| end.state.clone()));
            if let (Some(file), Some(state)) = (&mut reference_out, &state) {
                writeln!(file, "{state}").unwrap();
            }
            let replay_key = state.as_ref().map(|state| {
                let mut fields = state.split(',');
                (
                    fields.next().unwrap().parse::<u32>().expect("replay frame"),
                    fields.next().unwrap().parse::<u32>().expect("replay stage"),
                )
            });
            let Some((checkpoint, stage)) = (if replay_clock {
                replay_key
            } else {
                Some((update, 0))
            }) else {
                continue;
            };
            let selected = pending.remove(&checkpoint);
            let pool_selected = pending_pools.remove(&checkpoint);
            if (terminal.is_some() || all_pool_frames || selected || pool_selected)
                && let Some(file) = &mut pools
                && let Some(snapshot) = terminal
                    .as_ref()
                    .map(|end| end.pools.clone())
                    .or_else(|| game.replay_reference_pools())
            {
                writeln!(file, "{}", snapshot.trim_end()).unwrap();
            }
            if !selected {
                continue;
            }
            if let (Some(directory), Some(report)) = (&directory, &mut report) {
                let image = if replay_clock {
                    format!("replay_stage{stage:02}_frame{checkpoint:06}.png")
                } else {
                    format!("update_frame{checkpoint:06}.png")
                };
                let pixels = engine.read_frame();
                image::save_buffer(
                    directory.join(&image),
                    &pixels,
                    th06_engine::SCREEN_W,
                    th06_engine::SCREEN_H,
                    image::ColorType::Rgba8,
                )
                .expect("save checkpoint");
                writeln!(
                    report,
                    "{update},{},{checkpoint},{},{},{image},{},{},{draw_scene},\"{}\"",
                    if replay_clock { "replay" } else { "update" },
                    game.diagnostic_scene(),
                    if game.effective_color_16bit() { 16 } else { 32 },
                    draw_key.as_ref().map_or("", |key| key.0.as_str()),
                    draw_key.as_ref().map_or("", |key| key.1.as_str()),
                    state.unwrap_or_default()
                )
                .unwrap();
            }
            if pending.is_empty() && pending_pools.is_empty() && !all_pool_frames {
                break;
            }
        }
        assert!(
            pending.is_empty(),
            "capture checkpoints were not reached/rendered: {pending:?}"
        );
        assert!(
            pending_pools.is_empty(),
            "pool checkpoints were not reached: {pending_pools:?}"
        );
        return;
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
            let run_over =
                phase == RecordPhase::Ended || (phase == RecordPhase::Menu && entered_stage);
            let input = if run_over {
                Input::default()
            } else {
                auto_play_input(&game, f)
            };
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
            engine.apply_texture_replacements(&mut textures);
            let textures_ref = textures.iter().collect::<Vec<_>>();
            engine.advance_frame(&frame.cmds, &textures_ref, frame.bg.as_ref());
            let pixels = engine.read_frame();
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
            let run_over =
                phase == RecordPhase::Ended || (phase == RecordPhase::Menu && entered_stage);
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
        let no_shoot = std::env::var_os("TH06_NOSHOOT").is_some();
        for f in 0..frames {
            let mut held = if no_shoot {
                Vec::new()
            } else {
                vec![Key::Shoot]
            };
            if let Some((px, Some(tx))) = game.stage_aim() {
                if tx < px - 4.0 {
                    held.push(Key::Left);
                } else if tx > px + 4.0 {
                    held.push(Key::Right);
                }
            }
            let pressed: &[Key] = if no_shoot {
                if f % 12 == 0 { &[Key::Shoot] } else { &[] }
            } else if f % 12 == 0 {
                &[Key::Shoot]
            } else {
                &[]
            };
            let frame = game.update(&Input::synthetic(&held, pressed));
            engine.apply_texture_replacements(&mut textures);
            let textures_ref = textures.iter().collect::<Vec<_>>();
            engine.advance_frame(&frame.cmds, &textures_ref, frame.bg.as_ref());
            let pixels = engine.read_frame();
            let path = format!("{dir}/frame_{f:06}.png");
            image::save_buffer(
                &path,
                &pixels,
                th06_engine::SCREEN_W,
                th06_engine::SCREEN_H,
                image::ColorType::Rgba8,
            )
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
        let mut frame = Frame {
            cmds: Vec::new(),
            bg: None,
            quit: false,
        };
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
                engine.apply_texture_replacements(&mut textures);
                let textures_ref = textures.iter().collect::<Vec<_>>();
                engine.advance_frame(&frame.cmds, &textures_ref, frame.bg.as_ref());
                let pixels = engine.read_frame();
                let path = format!("{dir}/frame_{f:05}.png");
                image::save_buffer(
                    &path,
                    &pixels,
                    th06_engine::SCREEN_W,
                    th06_engine::SCREEN_H,
                    image::ColorType::Rgba8,
                )
                .expect("save demo frame");
                println!("wrote {path}");
            }
        }
        let _ = frame;
    } else if let Some(out) = screenshot {
        // Auto-play: hold Shoot, steer under the boss/nearest enemy, and pulse
        // Shoot to advance dialogue — so headless runs actually fight.
        // TH06_NOSHOOT: steer but never fire — lets a midboss/boss reach its
        // timeout so leave-without-kill and time-out-only spellcards (the ones
        // that use ex-instructions) are exercised. Dialogue is advanced with
        // Enter (which doesn't fire shots) so the fight actually starts.
        let no_shoot = std::env::var_os("TH06_NOSHOOT").is_some();
        for f in 0..frames {
            let mut held = if no_shoot {
                Vec::new()
            } else {
                vec![Key::Shoot]
            };
            if let Some((px, Some(tx))) = game.stage_aim() {
                if tx < px - 4.0 {
                    held.push(Key::Left);
                } else if tx > px + 4.0 {
                    held.push(Key::Right);
                }
            }
            let pressed: &[Key] = if no_shoot {
                if f % 12 == 0 { &[Key::Shoot] } else { &[] }
            } else if std::env::var_os("TH06_BOMB").is_some() && f == 40 {
                &[Key::Bomb]
            } else if f % 12 == 0 {
                &[Key::Shoot]
            } else {
                &[]
            };
            let frame = game.update(&Input::synthetic(&held, pressed));
            engine.apply_texture_replacements(&mut textures);
            let textures_ref = textures.iter().collect::<Vec<_>>();
            engine.advance_frame(&frame.cmds, &textures_ref, frame.bg.as_ref());
        }
        let pixels = engine.read_frame();
        image::save_buffer(
            &out,
            &pixels,
            th06_engine::SCREEN_W,
            th06_engine::SCREEN_H,
            image::ColorType::Rgba8,
        )
        .expect("save screenshot");
        println!("wrote {out}");
    } else if autoplay {
        // Drive the live window with the god-mode auto-player (steer under the
        // nearest enemy, hold Shoot, pulse to advance menus), ignoring keyboard.
        game.start_title_bgm();
        let mut f = 0u32;
        engine.run_game("Touhou 6 ~ EoSD (autoplay)", textures, move |_input| {
            let inp = auto_play_input(&game, f);
            f = f.wrapping_add(1);
            game.update(&inp)
        });
    } else {
        game.start_title_bgm();
        engine.run_game("Touhou 6 ~ EoSD (Mac port)", textures, move |input| {
            game.update(input)
        });
    }
}
