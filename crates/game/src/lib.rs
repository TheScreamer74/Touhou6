//! Shared game assembly: turns a set of in-memory archive byte maps into
//! GPU textures plus a runnable `Game`. Both the native binary (`main.rs`,
//! files read from disk) and the web entry (`web.rs`, files uploaded in the
//! browser) call [`build_game`] — no filesystem access lives here.

pub mod anm_vm;
pub mod background;
pub mod ecl_vm;
pub mod stage;
pub mod title;

#[cfg(target_arch = "wasm32")]
pub mod web;

use std::collections::HashMap;

use th06_engine::audio::Audio;
use th06_engine::{compose_rgba, DrawCmd, Engine, Frame, Input, Key, Texture};
use th06_formats::anm0::Anm0;
use th06_formats::ecl::Ecl;
use th06_formats::msg::Msg;
use th06_formats::std::Std;

use background::Background;
pub use stage::Character;
use stage::{Event, Stage};
use title::{Title, TitleAction};

/// Raw bytes of the game archives, keyed by flat entry name. Whoever builds
/// this (disk loader or browser upload) is responsible for supplying every
/// entry the builder reads below.
#[derive(Default)]
pub struct GameFiles {
    pub tl: HashMap<String, Vec<u8>>,
    pub cm: HashMap<String, Vec<u8>>,
    pub st: HashMap<String, Vec<u8>>,
    pub inn: HashMap<String, Vec<u8>>,
    pub st_en: HashMap<String, Vec<u8>>,
    /// BGM wavs keyed by basename ("th06_01.wav", ...).
    pub bgm: HashMap<String, Vec<u8>>,
}

const SFX_NAMES: [&str; 13] = [
    "plst00", "enep00", "enep01", "pldead00", "tan00", "tan01", "tan02", "damage00", "power1",
    "cat00", "item00", "powerup", "graze",
];

const BGM_NAMES: [&str; 8] = [
    "th06_01.wav", "th06_02.wav", "th06_03.wav", "th06_04.wav", "th06_05.wav", "th06_06.wav",
    "th06_07.wav", "th06_08.wav",
];

/// ANM texture names look like "data/title/title01.png"; archive entries
/// are flat basenames.
fn basename(path: &str) -> &str {
    path.rsplit('/').next().unwrap()
}

/// Everything needed to build a fresh stage 1 run.
struct StageAssets {
    ecl_data: Vec<u8>,
    msg_data: Vec<u8>,
    player: Anm0,
    player_tex: usize,
    player_marisa: Anm0,
    player_marisa_tex: usize,
    stg1enm: Anm0,
    stg1enm2: Anm0,
    etama: Anm0,
    stg1bg: Anm0,
    std_data: Vec<u8>,
    bg_tex_slot: usize,
}

impl StageAssets {
    fn new_stage(&self, character: Character) -> Stage {
        let ecl = Ecl::parse(self.ecl_data.clone()).expect("parse ecl");
        let scripts = stage::build_enemy_scripts(&[
            (&self.stg1enm.entries[0], stage::TEX_FAIRY),
            (&self.stg1enm2.entries[0], stage::TEX_RUMIA),
        ]);
        let msg = Msg::parse(self.msg_data.clone()).expect("parse msg");
        let background = Std::parse(&self.std_data)
            .map(|std| Background::new(std, &self.stg1bg.entries[0], self.bg_tex_slot));
        let (player_anm, player_tex) = if character.is_marisa() {
            (&self.player_marisa.entries[0], self.player_marisa_tex)
        } else {
            (&self.player.entries[0], self.player_tex)
        };
        Stage::new(ecl, scripts, &self.etama.entries[0], player_anm, player_tex, character, msg, background)
    }
}

pub enum Scene {
    Title,
    CharSelect { cursor: usize },
    Stage(Box<Stage>),
}

pub const CHARACTERS: [Character; 4] =
    [Character::ReimuA, Character::ReimuB, Character::MarisaA, Character::MarisaB];

pub struct Game {
    scene: Scene,
    title: Title,
    audio: Option<Audio>,
    assets: StageAssets,
    hiscore: i64,
    /// Native persists the high score to disk; web keeps it in memory only.
    #[cfg(not(target_arch = "wasm32"))]
    hiscore_path: std::path::PathBuf,
}

/// Build the full set of GPU textures and a `Game` at the title screen.
/// `with_audio` lets headless/screenshot callers skip audio device setup.
pub fn build_game(engine: &Engine, files: &GameFiles, with_audio: bool) -> (Vec<Texture>, Game) {
    let anm = Anm0::parse(&files.tl["title01.anm"]).expect("parse title01.anm");
    let entry = &anm.entries[0];

    // Texture slots (see stage.rs constants):
    // 0 title bg, 1 title menu, 2 player00, 3 etama3, 4 stg1enm,
    // 5 stg1enm2, 6 front, 7 white, 8 ascii, 9-10 faces, 11 stg1bg, 12 player01.
    let mut textures = Vec::new();
    let (bg_rgba, bg_w, bg_h) = compose_rgba(&files.tl["title00.jpg"], None);
    textures.push(engine.create_texture(&bg_rgba, bg_w, bg_h));
    let alpha = entry.alpha_name.as_deref().map(|n| files.tl[basename(n)].as_slice());
    let (rgba, w, h) = compose_rgba(&files.tl[basename(&entry.name)], alpha);
    textures.push(engine.create_texture(&rgba, w, h));
    for (archive, color, mask) in [
        (&files.cm, "player00.png", Some("player00_a.png")),
        (&files.cm, "etama3.png", Some("etama3_a.png")),
        (&files.st, "stg1enm.png", Some("stg1enm_a.png")),
        (&files.st, "stg1enm2.png", Some("stg1enm2_a.png")),
        (&files.cm, "front.png", Some("front_a.png")),
    ] {
        let alpha = mask.map(|m| archive[m].as_slice());
        let (rgba, w, h) = compose_rgba(&archive[color], alpha);
        textures.push(engine.create_texture(&rgba, w, h));
    }
    textures.push(engine.create_texture(&[255u8; 2 * 2 * 4], 2, 2));
    // Slot 8: ascii font (alpha mask doubles as tintable glyph color).
    let (rgba, w, h) = compose_rgba(&files.inn["ascii_a.png"], Some(files.inn["ascii_a.png"].as_slice()));
    textures.push(engine.create_texture(&rgba, w, h));
    // Slots 9-10: dialogue portraits (Reimu, Rumia).
    for face in ["face00a", "face01a"] {
        let (rgba, w, h) = compose_rgba(
            &files.cm[&format!("{face}.png")],
            Some(files.cm[&format!("{face}_a.png")].as_slice()),
        );
        textures.push(engine.create_texture(&rgba, w, h));
    }
    // Slot 11: stage 1 background texture.
    let bg_tex_slot = textures.len();
    let (rgba, w, h) = compose_rgba(&files.st["stg1bg.png"], Some(files.st["stg1bg_a.png"].as_slice()));
    textures.push(engine.create_texture(&rgba, w, h));
    // Slot 12: Marisa player sprite (player01).
    let player_marisa_tex = textures.len();
    let (rgba, w, h) = compose_rgba(&files.cm["player01.png"], Some(files.cm["player01_a.png"].as_slice()));
    textures.push(engine.create_texture(&rgba, w, h));

    let title = Title::new(entry, 0, 1);

    let mut audio = if with_audio { Audio::new() } else { None };
    if let Some(a) = &mut audio {
        for name in SFX_NAMES {
            if let Some(wav) = files.inn.get(&format!("{name}.wav")) {
                a.register_sfx(name, wav.clone());
            }
        }
        for name in BGM_NAMES {
            if let Some(wav) = files.bgm.get(name) {
                a.register_bgm(name, wav.clone());
            }
        }
    }

    let assets = StageAssets {
        ecl_data: files.st["ecldata1.ecl"].clone(),
        msg_data: files.st_en["msg1.dat"].clone(),
        player: Anm0::parse(&files.cm["player00.anm"]).expect("parse player00"),
        player_tex: stage::TEX_PLAYER,
        player_marisa: Anm0::parse(&files.cm["player01.anm"]).expect("parse player01"),
        player_marisa_tex,
        stg1enm: Anm0::parse(&files.st["stg1enm.anm"]).expect("parse stg1enm"),
        stg1enm2: Anm0::parse(&files.st["stg1enm2.anm"]).expect("parse stg1enm2"),
        etama: Anm0::parse(&files.cm["etama3.anm"]).expect("parse etama3"),
        stg1bg: Anm0::parse(&files.st["stg1bg.anm"]).expect("parse stg1bg"),
        std_data: files.st["stage1.std"].clone(),
        bg_tex_slot,
    };

    let game = Game {
        scene: Scene::Title,
        title,
        audio,
        assets,
        hiscore: 0,
        #[cfg(not(target_arch = "wasm32"))]
        hiscore_path: std::path::PathBuf::new(),
    };
    (textures, game)
}

impl Game {
    pub fn set_hiscore(&mut self, v: i64) {
        self.hiscore = v;
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn set_hiscore_path(&mut self, path: std::path::PathBuf) {
        self.hiscore_path = path;
    }

    /// Jump straight into stage 1 (native `--scene stage` debugging).
    pub fn debug_start_stage(&mut self, character: Character, lives: Option<i32>) {
        let mut s = self.assets.new_stage(character);
        s.set_hiscore(self.hiscore);
        if let Some(l) = lives {
            s.set_lives(l);
        }
        self.scene = Scene::Stage(Box::new(s));
    }

    /// Start the title BGM (call once the audio context is unlocked).
    pub fn start_title_bgm(&mut self) {
        self.play_bgm("th06_01.wav");
    }

    fn play_bgm(&mut self, file: &str) {
        if let Some(audio) = &mut self.audio {
            audio.play_bgm(file);
        }
    }

    /// Render the character-select screen (title art + dark overlay + list).
    fn charselect_cmds(&self, cursor: usize) -> Vec<DrawCmd> {
        let mut cmds = vec![
            DrawCmd {
                tex: 0, // title background
                dst: [0.0, 0.0, th06_engine::SCREEN_W as f32, th06_engine::SCREEN_H as f32],
                src: [0.0, 0.0, 1.0, 1.0],
                tint: [1.0, 1.0, 1.0, 1.0],
                rot: 0.0,
            },
            DrawCmd {
                tex: 7, // white pixel, dimmed
                dst: [0.0, 0.0, th06_engine::SCREEN_W as f32, th06_engine::SCREEN_H as f32],
                src: [0.25, 0.25, 0.75, 0.75],
                tint: [0.0, 0.0, 0.05, 0.72],
                rot: 0.0,
            },
        ];
        stage::draw_text(&mut cmds, [180.0, 90.0], 26.0, [1.0, 1.0, 0.5, 1.0], "SELECT CHARACTER");
        for (i, ch) in CHARACTERS.iter().enumerate() {
            let sel = i == cursor;
            let tint = if sel { [1.0, 1.0, 0.4, 1.0] } else { [0.6, 0.6, 0.7, 1.0] };
            let label = if sel { format!("> {}", ch.label()) } else { ch.label().to_string() };
            stage::draw_text(&mut cmds, [220.0, 180.0 + i as f32 * 40.0], 22.0, tint, &label);
        }
        let note = match CHARACTERS[cursor] {
            Character::ReimuA => "Homing amulets - forgiving",
            Character::ReimuB => "Piercing needles - focused",
            Character::MarisaA => "Power missiles (WIP)",
            Character::MarisaB => "Illusion lasers (WIP)",
        };
        stage::draw_text(&mut cmds, [150.0, 360.0], 14.0, [0.8, 0.85, 1.0, 1.0], note);
        stage::draw_text(&mut cmds, [150.0, 400.0], 14.0, [0.7, 0.7, 0.7, 1.0], "Z: start   X: back");
        cmds
    }

    pub fn update(&mut self, input: &Input) -> Frame {
        // Character select is handled before the borrow of self.scene so it can
        // freely touch audio / start a stage.
        if let Scene::CharSelect { cursor } = &self.scene {
            let n = CHARACTERS.len();
            let mut cur = *cursor;
            if input.pressed(Key::Up) {
                cur = (cur + n - 1) % n;
                if let Some(a) = &self.audio { a.play_sfx("tan00"); }
            }
            if input.pressed(Key::Down) {
                cur = (cur + 1) % n;
                if let Some(a) = &self.audio { a.play_sfx("tan00"); }
            }
            if input.pressed(Key::Bomb) || input.pressed(Key::Pause) {
                self.scene = Scene::Title;
                return Frame { cmds: self.charselect_cmds(0), bg: None, quit: false };
            }
            if input.pressed(Key::Shoot) || input.pressed(Key::Enter) {
                let mut stage = self.assets.new_stage(CHARACTERS[cur]);
                stage.set_hiscore(self.hiscore);
                self.scene = Scene::Stage(Box::new(stage));
                if let Some(a) = &self.audio {
                    a.play_sfx("plst00");
                }
                return Frame { cmds: Vec::new(), bg: None, quit: false };
            }
            self.scene = Scene::CharSelect { cursor: cur };
            return Frame { cmds: self.charselect_cmds(cur), bg: None, quit: false };
        }
        match &mut self.scene {
            Scene::CharSelect { .. } => unreachable!("handled above"),
            Scene::Title => {
                let (cmds, action) = self.title.update(input);
                match action {
                    TitleAction::StartGame => {
                        self.scene = Scene::CharSelect { cursor: 0 };
                        if let Some(a) = &self.audio {
                            a.play_sfx("tan00");
                        }
                    }
                    TitleAction::Quit => return Frame { cmds, bg: None, quit: true },
                    TitleAction::None => {}
                }
                Frame { cmds, bg: None, quit: false }
            }
            Scene::Stage(stage) => {
                let cmds = stage.update(input);
                let bg = stage.background_scene();
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
                        Event::Quit => return Frame { cmds, bg, quit: true },
                        Event::SaveScore(score) => {
                            if score > self.hiscore {
                                self.hiscore = score;
                                #[cfg(not(target_arch = "wasm32"))]
                                {
                                    let _ = std::fs::write(&self.hiscore_path, score.to_string());
                                }
                            }
                        }
                    }
                }
                if back {
                    self.scene = Scene::Title;
                    self.title.reset();
                    self.play_bgm("th06_01.wav");
                    return Frame { cmds, bg: None, quit: false };
                }
                Frame { cmds, bg, quit: false }
            }
        }
    }
}
