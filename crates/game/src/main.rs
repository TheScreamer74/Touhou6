mod anm_vm;
mod ecl_vm;
mod stage;
mod title;

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use th06_engine::audio::Audio;
use th06_engine::{compose_rgba, Engine, Frame, Input, Key};
use th06_formats::anm0::Anm0;
use th06_formats::pbg3::Pbg3;

use th06_formats::ecl::Ecl;
use th06_formats::msg::Msg;

use stage::{Event, Stage};
use title::{Title, TitleAction};

/// Everything needed to build a fresh stage 1 run.
struct StageAssets {
    ecl_data: Vec<u8>,
    msg_data: Vec<u8>,
    stg1enm: Anm0,
    stg1enm2: Anm0,
    etama: Anm0,
}

impl StageAssets {
    fn new_stage(&self) -> Stage {
        let ecl = Ecl::parse(self.ecl_data.clone()).expect("parse ecl");
        let scripts = stage::build_enemy_scripts(&[
            (&self.stg1enm.entries[0], stage::TEX_FAIRY),
            (&self.stg1enm2.entries[0], stage::TEX_RUMIA),
        ]);
        let msg = Msg::parse(self.msg_data.clone()).expect("parse msg");
        Stage::new(ecl, scripts, &self.etama.entries[0], msg)
    }
}

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

enum Scene {
    Title,
    Stage(Box<Stage>),
}

struct Game {
    scene: Scene,
    title: Title,
    audio: Option<Audio>,
    bgm_dir: PathBuf,
    assets: StageAssets,
}

const SFX_NAMES: [&str; 12] = [
    "plst00", "enep00", "enep01", "pldead00", "tan00", "tan01", "tan02", "damage00", "power1",
    "cat00", "item00", "powerup",
];

impl Game {
    fn play_bgm(&mut self, file: &str) {
        if let Some(audio) = &mut self.audio {
            audio.play_bgm(&self.bgm_dir.join(file));
        }
    }

    fn update(&mut self, input: &Input) -> Frame {
        if std::env::var_os("TH06_TICKRATE").is_some() {
            use std::sync::atomic::{AtomicU32, Ordering};
            use std::sync::Mutex;
            use std::time::Instant;
            static COUNT: AtomicU32 = AtomicU32::new(0);
            static LAST: Mutex<Option<Instant>> = Mutex::new(None);
            let n = COUNT.fetch_add(1, Ordering::Relaxed) + 1;
            let now = Instant::now();
            let mut last = LAST.lock().unwrap();
            let prev = last.get_or_insert(now);
            if now.duration_since(*prev).as_secs_f32() >= 1.0 {
                eprintln!("ticks/sec: {n}");
                COUNT.store(0, Ordering::Relaxed);
                *last = Some(now);
            }
        }
        match &mut self.scene {
            Scene::Title => {
                let (cmds, action) = self.title.update(input);
                match action {
                    TitleAction::StartGame => {
                        self.scene = Scene::Stage(Box::new(self.assets.new_stage()));
                        if let Some(a) = &self.audio {
                            a.play_sfx("plst00");
                        }
                    }
                    TitleAction::Quit => return Frame { cmds, quit: true },
                    TitleAction::None => {}
                }
                Frame { cmds, quit: false }
            }
            Scene::Stage(stage) => {
                let cmds = stage.update(input);
                let events: Vec<Event> = stage.events.drain(..).collect();
                let mut back = false;
                for ev in events {
                    match ev {
                        Event::Sfx(name) => {
                            if let Some(a) = &self.audio {
                                a.play_sfx(name);
                            }
                        }
                        Event::Bgm(file) => {
                            let file = file.to_string();
                            self.play_bgm(&file);
                        }
                        Event::BackToTitle => back = true,
                    }
                }
                if back {
                    self.scene = Scene::Title;
                    self.title.reset();
                    self.play_bgm("th06_01.wav");
                }
                Frame { cmds, quit: false }
            }
        }
    }
}

fn main() {
    install_crash_logger();
    let mut args = std::env::args().skip(1);
    let mut screenshot: Option<String> = None;
    let mut frames = 120u32;
    let mut scene_arg = String::from("title");
    let mut debug_lives: Option<i32> = None;
    let mut game_dir = String::from("../TH06 ~ The Embodiment of Scarlet Devil/kouma");
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--screenshot" => screenshot = Some(args.next().expect("--screenshot <out.png>")),
            "--frames" => frames = args.next().expect("--frames <n>").parse().expect("frame count"),
            "--scene" => scene_arg = args.next().expect("--scene <title|stage>"),
            "--lives" => debug_lives = Some(args.next().expect("--lives <n>").parse().expect("lives")),
            "--game-dir" => game_dir = args.next().expect("--game-dir <path>"),
            other => panic!("unknown argument: {other}"),
        }
    }

    let game_dir = PathBuf::from(game_dir);
    let tl = load_archive(&game_dir.join("TL.DAT"));
    let cm = load_archive(&game_dir.join("CM.DAT"));
    let st = load_archive(&game_dir.join("ST.DAT"));
    let inn = load_archive(&game_dir.join("IN.DAT"));

    let anm = Anm0::parse(&tl["title01.anm"]).expect("parse title01.anm");
    let entry = &anm.entries[0];

    let engine = Engine::new();

    // Texture slots (see stage.rs constants):
    // 0 title bg, 1 title menu, 2 player00, 3 etama3, 4 stg1enm,
    // 5 stg1enm2, 6 front, 7 white.
    let mut textures = Vec::new();
    let (bg_rgba, bg_w, bg_h) = compose_rgba(&tl["title00.jpg"], None);
    textures.push(engine.create_texture(&bg_rgba, bg_w, bg_h));
    let alpha = entry.alpha_name.as_deref().map(|n| tl[basename(n)].as_slice());
    let (rgba, w, h) = compose_rgba(&tl[basename(&entry.name)], alpha);
    textures.push(engine.create_texture(&rgba, w, h));
    for (archive, color, mask) in [
        (&cm, "player00.png", Some("player00_a.png")),
        (&cm, "etama3.png", Some("etama3_a.png")),
        (&st, "stg1enm.png", Some("stg1enm_a.png")),
        (&st, "stg1enm2.png", Some("stg1enm2_a.png")),
        (&cm, "front.png", Some("front_a.png")),
    ] {
        let alpha = mask.map(|m| archive[m].as_slice());
        let (rgba, w, h) = compose_rgba(&archive[color], alpha);
        textures.push(engine.create_texture(&rgba, w, h));
    }
    textures.push(engine.create_texture(&[255u8; 2 * 2 * 4], 2, 2));
    // Slot 8: ascii font. The alpha mask alone is the cleanest glyph
    // source (white shapes); using it for both color and alpha gives
    // tintable text.
    let (rgba, w, h) = compose_rgba(&inn["ascii_a.png"], Some(inn["ascii_a.png"].as_slice()));
    textures.push(engine.create_texture(&rgba, w, h));

    let title = Title::new(entry, 0, 1);

    let mut audio = Audio::new();
    if let Some(a) = &mut audio {
        for name in SFX_NAMES {
            let file = format!("{name}.wav");
            if let Some(wav) = inn.get(&file) {
                a.register_sfx(name, wav.clone());
            }
        }
    }

    // English-patch ST archive provides ASCII dialogue text.
    let st_en = load_archive(&game_dir.join("th06e_ST.DAT"));
    let assets = StageAssets {
        ecl_data: st["ecldata1.ecl"].clone(),
        msg_data: st_en["msg1.dat"].clone(),
        stg1enm: Anm0::parse(&st["stg1enm.anm"]).expect("parse stg1enm"),
        stg1enm2: Anm0::parse(&st["stg1enm2.anm"]).expect("parse stg1enm2"),
        etama: Anm0::parse(&cm["etama3.anm"]).expect("parse etama3"),
    };

    let mut game = Game {
        scene: match scene_arg.as_str() {
            "stage" => {
                let mut s = assets.new_stage();
                if let Some(l) = debug_lives {
                    s.set_lives(l);
                }
                Scene::Stage(Box::new(s))
            }
            _ => Scene::Title,
        },
        title,
        audio: if screenshot.is_some() { None } else { audio },
        bgm_dir: game_dir.join("bgm"),
        assets,
    };

    if let Some(out) = screenshot {
        // Headless: hold Shoot so stage scenes show combat.
        let input = Input::synthetic(&[Key::Shoot], &[]);
        let mut frame = game.update(&input);
        for _ in 1..frames {
            frame = game.update(&input);
        }
        let textures_ref: Vec<&th06_engine::Texture> = textures.iter().collect();
        let pixels = engine.render_to_image(&frame.cmds, &textures_ref);
        image::save_buffer(&out, &pixels, th06_engine::SCREEN_W, th06_engine::SCREEN_H, image::ColorType::Rgba8)
            .expect("save screenshot");
        println!("wrote {out}");
    } else {
        game.play_bgm("th06_01.wav");
        engine.run_game("Touhou 6 ~ EoSD (Mac port)", textures, move |input| game.update(input));
    }
}
