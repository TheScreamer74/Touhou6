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

/// All BGM tracks used in the main game (title + 6 stages × field/boss).
const BGM_NAMES: [&str; 13] = [
    "th06_01.wav", "th06_02.wav", "th06_03.wav", "th06_04.wav", "th06_05.wav", "th06_06.wav",
    "th06_07.wav", "th06_08.wav", "th06_09.wav", "th06_10.wav", "th06_11.wav", "th06_12.wav",
    "th06_13.wav",
];

/// Number of normal-mode stages.
pub const N_STAGES: usize = 6;
/// Per-stage field theme.
const STAGE_BGM: [&str; N_STAGES] =
    ["th06_02.wav", "th06_04.wav", "th06_06.wav", "th06_08.wav", "th06_10.wav", "th06_12.wav"];
/// Per-stage boss theme.
const BOSS_BGM: [&str; N_STAGES] =
    ["th06_03.wav", "th06_05.wav", "th06_07.wav", "th06_09.wav", "th06_11.wav", "th06_13.wav"];
/// Per-stage boss dialogue portrait (Gui.cpp FACE_STAGE_A), all in ST.DAT.
const BOSS_FACE: [&str; N_STAGES] =
    ["face03a", "face05a", "face06a", "face08a", "face09a", "face09b"];
/// Whether stage N ships a separate boss sprite sheet (stgNenm2); stages 3 & 4
/// keep their boss in the main enemy sheet.
const HAS_ENM2: [bool; N_STAGES] = [true, true, false, false, true, true];

/// ANM texture names look like "data/title/title01.png"; archive entries
/// are flat basenames.
fn basename(path: &str) -> &str {
    path.rsplit('/').next().unwrap()
}

/// Per-stage assets: scripts/data bytes plus the parsed sprite sheets and the
/// texture slots they were uploaded to.
struct StageData {
    ecl_data: Vec<u8>,
    msg_data: Vec<u8>,
    std_data: Vec<u8>,
    enm: Anm0,
    enm_tex: usize,
    enm2: Option<Anm0>,
    enm2_tex: usize,
    bg: Anm0,
    bg_tex: usize,
    boss_face_tex: usize,
}

/// Shared (cross-stage) assets plus the per-stage table.
struct Assets {
    player: Anm0,
    player_tex: usize,
    player_marisa: Anm0,
    player_marisa_tex: usize,
    etama: Anm0,
    stages: Vec<StageData>,
}

impl Assets {
    fn new_stage(&self, idx: usize, character: Character) -> Stage {
        let sd = &self.stages[idx];
        let ecl = Ecl::parse(sd.ecl_data.clone()).expect("parse ecl");
        let mut pairs = vec![(&sd.enm.entries[0], sd.enm_tex)];
        if let Some(enm2) = &sd.enm2 {
            pairs.push((&enm2.entries[0], sd.enm2_tex));
        }
        let scripts = stage::build_enemy_scripts(&pairs);
        let msg = Msg::parse(sd.msg_data.clone()).expect("parse msg");
        let background = Std::parse(&sd.std_data)
            .map(|std| Background::new(std, &sd.bg.entries[0], sd.bg_tex));
        let (player_anm, player_tex, face_player_tex) = if character.is_marisa() {
            (&self.player_marisa.entries[0], self.player_marisa_tex, stage::TEX_FACE_MARISA)
        } else {
            (&self.player.entries[0], self.player_tex, stage::TEX_FACE_REIMU)
        };
        let cfg = stage::StageConfig {
            face_player_tex,
            face_boss_tex: sd.boss_face_tex,
            stage_bgm: STAGE_BGM[idx],
            boss_bgm: BOSS_BGM[idx],
        };
        Stage::new(
            ecl, scripts, &self.etama.entries[0], player_anm, player_tex, character, msg,
            background, cfg,
        )
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
    assets: Assets,
    /// Stage currently being played (0-based), and the chosen character — used
    /// to build the next stage on clear.
    current_stage: usize,
    character: Character,
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

    // Upload a composed texture and return its slot; the optional alpha mask
    // is skipped when absent (some sheets ship without a `_a` mask).
    let load = |map: &HashMap<String, Vec<u8>>, color: &str, mask: Option<&str>, textures: &mut Vec<Texture>| -> usize {
        let alpha = mask.and_then(|m| map.get(m)).map(|v| v.as_slice());
        let (rgba, w, h) = compose_rgba(&map[color], alpha);
        let slot = textures.len();
        textures.push(engine.create_texture(&rgba, w, h));
        slot
    };

    // Fixed slots (referenced by stage.rs constants): 0 title bg, 1 title menu,
    // 2 player00, 3 etama3, 4 stg1enm, 5 stg1enm2, 6 front, 7 white, 8 ascii,
    // 9 face00a (Reimu), 10 face01a (Marisa), 11 stg1bg, 12 player01.
    let mut textures = Vec::new();
    let (bg_rgba, bg_w, bg_h) = compose_rgba(&files.tl["title00.jpg"], None);
    textures.push(engine.create_texture(&bg_rgba, bg_w, bg_h)); // 0
    let alpha = entry.alpha_name.as_deref().map(|n| files.tl[basename(n)].as_slice());
    let (rgba, w, h) = compose_rgba(&files.tl[basename(&entry.name)], alpha);
    textures.push(engine.create_texture(&rgba, w, h)); // 1
    for (archive, color, mask) in [
        (&files.cm, "player00.png", Some("player00_a.png")),
        (&files.cm, "etama3.png", Some("etama3_a.png")),
        (&files.st, "stg1enm.png", Some("stg1enm_a.png")),
        (&files.st, "stg1enm2.png", Some("stg1enm2_a.png")),
        (&files.cm, "front.png", Some("front_a.png")),
    ] {
        let alpha = mask.map(|m| archive[m].as_slice());
        let (rgba, w, h) = compose_rgba(&archive[color], alpha);
        textures.push(engine.create_texture(&rgba, w, h)); // 2,3,4,5,6
    }
    textures.push(engine.create_texture(&[255u8; 2 * 2 * 4], 2, 2)); // 7 white
    // 8: ascii font (alpha mask doubles as tintable glyph color).
    let (rgba, w, h) = compose_rgba(&files.inn["ascii_a.png"], Some(files.inn["ascii_a.png"].as_slice()));
    textures.push(engine.create_texture(&rgba, w, h));
    // 9-10: player dialogue portraits (Reimu, Marisa).
    load(&files.cm, "face00a.png", Some("face00a_a.png"), &mut textures); // 9
    load(&files.cm, "face01a.png", Some("face01a_a.png"), &mut textures); // 10
    // 11: stage 1 background texture.
    load(&files.st, "stg1bg.png", Some("stg1bg_a.png"), &mut textures);
    // 12: Marisa player body sprite (player01).
    let player_marisa_tex = load(&files.cm, "player01.png", Some("player01_a.png"), &mut textures);

    // Per-stage sprite sheets + boss faces, appended after the fixed slots.
    let mut stages: Vec<StageData> = Vec::new();
    for n in 1..=N_STAGES {
        let (enm_tex, enm2, enm2_tex, bg_tex) = if n == 1 {
            // Stage 1 reuses the fixed slots loaded above.
            (4, Some(Anm0::parse(&files.st["stg1enm2.anm"]).expect("stg1enm2")), 5, 11)
        } else {
            let enm_tex = load(&files.st, &format!("stg{n}enm.png"), Some(&format!("stg{n}enm_a.png")), &mut textures);
            let (enm2, enm2_tex) = if HAS_ENM2[n - 1] {
                let t = load(&files.st, &format!("stg{n}enm2.png"), Some(&format!("stg{n}enm2_a.png")), &mut textures);
                (Some(Anm0::parse(&files.st[&format!("stg{n}enm2.anm")]).expect("enm2")), t)
            } else {
                (None, 0)
            };
            let bg_tex = load(&files.st, &format!("stg{n}bg.png"), Some(&format!("stg{n}bg_a.png")), &mut textures);
            (enm_tex, enm2, enm2_tex, bg_tex)
        };
        let face = BOSS_FACE[n - 1];
        let boss_face_tex =
            load(&files.st, &format!("{face}.png"), Some(&format!("{face}_a.png")), &mut textures);
        stages.push(StageData {
            ecl_data: files.st[&format!("ecldata{n}.ecl")].clone(),
            msg_data: files.st_en[&format!("msg{n}.dat")].clone(),
            std_data: files.st[&format!("stage{n}.std")].clone(),
            enm: Anm0::parse(&files.st[&format!("stg{n}enm.anm")]).expect("enm"),
            enm_tex,
            enm2,
            enm2_tex,
            bg: Anm0::parse(&files.st[&format!("stg{n}bg.anm")]).expect("bg"),
            bg_tex,
            boss_face_tex,
        });
    }

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

    let assets = Assets {
        player: Anm0::parse(&files.cm["player00.anm"]).expect("parse player00"),
        player_tex: stage::TEX_PLAYER,
        player_marisa: Anm0::parse(&files.cm["player01.anm"]).expect("parse player01"),
        player_marisa_tex,
        etama: Anm0::parse(&files.cm["etama3.anm"]).expect("parse etama3"),
        stages,
    };

    let game = Game {
        scene: Scene::Title,
        title,
        audio,
        assets,
        current_stage: 0,
        character: Character::ReimuA,
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

    /// Jump straight into a stage (native `--scene stage` debugging). `stage`
    /// is 0-based and clamped to the available stages.
    pub fn debug_start_stage(&mut self, character: Character, lives: Option<i32>, stage: usize) {
        self.character = character;
        self.current_stage = stage.min(N_STAGES - 1);
        let mut s = self.assets.new_stage(self.current_stage, character);
        s.set_hiscore(self.hiscore);
        if let Some(l) = lives {
            s.set_lives(l);
        }
        self.scene = Scene::Stage(Box::new(s));
    }

    /// Headless auto-play aim: (player_x, target_x) while in a stage.
    pub fn stage_aim(&self) -> Option<(f32, Option<f32>)> {
        if let Scene::Stage(s) = &self.scene {
            Some((s.player_x(), s.target_x()))
        } else {
            None
        }
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
                self.character = CHARACTERS[cur];
                self.current_stage = 0;
                let mut stage = self.assets.new_stage(0, self.character);
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
                // Snapshot now, before the event loop reborrows self.
                let carry = stage.carry();
                let mut back = false;
                let mut next_stage = false;
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
                        Event::NextStage => next_stage = true,
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
                if next_stage {
                    // Carry progress into the next stage; the last stage clear
                    // returns to the title (ending not yet implemented).
                    if self.current_stage + 1 < N_STAGES {
                        self.current_stage += 1;
                        let mut s = self.assets.new_stage(self.current_stage, self.character);
                        s.apply_carry(carry);
                        s.set_hiscore(self.hiscore);
                        self.scene = Scene::Stage(Box::new(s));
                        return Frame { cmds, bg: None, quit: false };
                    }
                    back = true;
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
