//! Shared game assembly: turns a set of in-memory archive byte maps into
//! GPU textures plus a runnable `Game`. Both the native binary (`main.rs`,
//! files read from disk) and the web entry (`web.rs`, files uploaded in the
//! browser) call [`build_game`] — no filesystem access lives here.

pub mod anm_vm;
pub mod background;
pub mod ecl_vm;
pub mod hud;
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

/// Every sound effect (SoundPlayer.cpp g_SFXList order; index == SoundIdx).
/// Loaded by basename from the archive; absent ones are simply skipped.
use stage::SFX_BY_IDX as SFX_NAMES;

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
    front: Anm0,
    text: Anm0,
    face_chara: Anm0,
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
            stage_num: idx as i32 + 1,
        };
        let hud = crate::hud::Hud::new(&self.front.entries[0], stage::TEX_FRONT);
        // text.anm script 7 = TEXT_ENEMY_SPELLCARD_NAME (the announce animation).
        let spell_name_script = self.text.entries[0]
            .scripts
            .iter()
            .find(|(id, _)| *id == 7)
            .map(|(_, i)| i.clone())
            .unwrap_or_default();
        // face*.anm script 3 = FACE_ENEMY_SPELLCARD_PORTRAIT (the slide-in).
        let portrait_script = self.face_chara.entries[0]
            .scripts
            .iter()
            .find(|(id, _)| *id == 3)
            .map(|(_, i)| i.clone())
            .unwrap_or_default();
        Stage::new(
            ecl, scripts, &self.etama.entries[0], player_anm, player_tex, character, msg,
            background, hud, cfg, spell_name_script, portrait_script,
        )
    }
}

/// Coarse run phase reported by [`Game::record_phase`] for the split recorder.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RecordPhase {
    Menu,
    Stage(usize),
    Ended,
}

pub enum Scene {
    Title,
    CharSelect { cursor: usize },
    Stage(Box<Stage>),
    /// High-score name entry after a game over whose score made the table.
    NameEntry { score: i64, stage: u8, name: String, sel: usize },
    /// The high-score table; `highlight` marks a freshly entered row.
    Leaderboard { highlight: Option<usize> },
}

/// One high-score table row.
#[derive(Clone)]
pub struct ScoreEntry {
    pub name: String,
    pub score: i64,
    pub stage: u8,
}

/// Selectable characters for the name-entry ribbon (ResultScreen.cpp char set,
/// trimmed to the glyphs our ascii font draws).
const NAME_RIBBON: &str =
    "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789.,!?+-/ ";
const RIBBON_COLS: usize = 14;
const MAX_NAME_LEN: usize = 8;
const MAX_SCORES: usize = 10;

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
    /// Base texture slot for the character-select art (see build_game).
    select_tex: usize,
    hiscore: i64,
    /// High-score table (descending), capped at MAX_SCORES.
    scores: Vec<ScoreEntry>,
    /// Native persists the high score to disk; web keeps it in memory only.
    #[cfg(not(target_arch = "wasm32"))]
    hiscore_path: std::path::PathBuf,
    #[cfg(not(target_arch = "wasm32"))]
    scores_path: std::path::PathBuf,
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
    // Background textures tile and scroll, so they use a wrapping sampler.
    let load_bg = |map: &HashMap<String, Vec<u8>>, color: &str, mask: Option<&str>, textures: &mut Vec<Texture>| -> usize {
        let alpha = mask.and_then(|m| map.get(m)).map(|v| v.as_slice());
        let (rgba, w, h) = compose_rgba(&map[color], alpha);
        let slot = textures.len();
        textures.push(engine.create_texture_wrapped(&rgba, w, h));
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
    load_bg(&files.st, "stg1bg.png", Some("stg1bg_a.png"), &mut textures);
    // 12: Marisa player body sprite (player01).
    let player_marisa_tex = load(&files.cm, "player01.png", Some("player01_a.png"), &mut textures);
    // 13: power-bar gradient, 2x1 (0xe0e0e0 -> 0x80e0e0) sampled linearly so the
    // HUD power bar is the decomp's exact per-vertex gradient (Gui.cpp:1162-1163).
    debug_assert_eq!(textures.len(), stage::TEX_POWER_GRAD);
    textures.push(engine.create_texture(&[224, 224, 224, 255, 128, 224, 224, 255], 2, 1));

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
            let bg_tex = load_bg(&files.st, &format!("stg{n}bg.png"), Some(&format!("stg{n}bg_a.png")), &mut textures);
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

    // Character-select art (TL.DAT): bg + the four slpl character illustrations
    // (Reimu A/B, Marisa A/B) + select04 shot-name banners + select03 prompts.
    let select_tex = textures.len();
    let (rgba, w, h) = compose_rgba(&files.tl["select00.jpg"], None);
    textures.push(engine.create_texture(&rgba, w, h)); // +0 bg
    for art in ["slpl00a", "slpl00b", "slpl01a", "slpl01b", "select04", "select03"] {
        load(&files.tl, &format!("{art}.png"), Some(&format!("{art}_a.png")), &mut textures);
    } // +1..+6

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
        front: Anm0::parse(&files.cm["front.anm"]).expect("parse front"),
        text: Anm0::parse(&files.inn["text.anm"]).expect("parse text"),
        face_chara: Anm0::parse(&files.cm["face00a.anm"]).expect("parse face00a"),
        stages,
    };

    let game = Game {
        scene: Scene::Title,
        title,
        audio,
        assets,
        current_stage: 0,
        character: Character::ReimuA,
        select_tex,
        hiscore: 0,
        scores: Vec::new(),
        #[cfg(not(target_arch = "wasm32"))]
        hiscore_path: std::path::PathBuf::new(),
        #[cfg(not(target_arch = "wasm32"))]
        scores_path: std::path::PathBuf::new(),
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

    /// Seed the high-score table (kept sorted descending, capped).
    pub fn set_scores(&mut self, mut scores: Vec<ScoreEntry>) {
        scores.sort_by(|a, b| b.score.cmp(&a.score));
        scores.truncate(MAX_SCORES);
        self.scores = scores;
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn set_scores_path(&mut self, path: std::path::PathBuf) {
        self.scores_path = path;
    }

    /// A score makes the table if there is a free slot or it beats the lowest.
    fn score_qualifies(&self, score: i64) -> bool {
        self.scores.len() < MAX_SCORES || self.scores.iter().any(|e| score > e.score)
    }

    /// Insert a finished run and return its row index in the sorted table.
    fn insert_score(&mut self, name: String, score: i64, stage: u8) -> usize {
        self.scores.push(ScoreEntry { name, score, stage });
        self.scores.sort_by(|a, b| b.score.cmp(&a.score));
        self.scores.truncate(MAX_SCORES);
        self.scores.iter().position(|e| e.score == score).unwrap_or(0)
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn save_scores(&self) {
        if self.scores_path.as_os_str().is_empty() {
            return;
        }
        let body: String = self
            .scores
            .iter()
            .map(|e| format!("{}\t{}\t{}\n", e.score, e.stage, e.name))
            .collect();
        let _ = std::fs::write(&self.scores_path, body);
    }
    #[cfg(target_arch = "wasm32")]
    fn save_scores(&self) {}

    /// Jump straight into a stage (native `--scene stage` debugging). `stage`
    /// is 0-based and clamped to the available stages. Optional lives / power /
    /// score seed the starting state.
    pub fn debug_start_stage(
        &mut self,
        character: Character,
        lives: Option<i32>,
        stage: usize,
        power: Option<i32>,
        score: Option<i64>,
    ) {
        self.character = character;
        self.current_stage = stage.min(N_STAGES - 1);
        let mut s = self.assets.new_stage(self.current_stage, character);
        s.set_hiscore(self.hiscore);
        if let Some(l) = lives {
            s.set_lives(l);
        }
        if let Some(p) = power {
            s.set_power(p);
        }
        if let Some(sc) = score {
            s.set_score(sc);
        }
        self.scene = Scene::Stage(Box::new(s));
    }

    /// Debug: fast-forward the current stage's simulation until the midboss
    /// (`to_boss=false`) or the real boss (`to_boss=true`) is on screen, so the
    /// fight can be debugged/recorded without playing through the trash. Runs
    /// invulnerable, never shoots (keeps the boss alive) and taps Enter to clear
    /// the pre-boss dialogue. The caller must enable god mode first.
    pub fn debug_warp(&mut self, to_boss: bool) -> bool {
        // Count boss-present episodes: the 1st is the midboss, the 2nd is the
        // real boss. Never shoot, so the midboss times out and leaves (it would
        // stall the boss timeline if killed); Enter is pulsed to step the
        // pre-boss dialogue. The caller forces god, so the player can't die.
        for _i in 0..40_000 {
            // Never shoot: a killed midboss/trash stalls the boss timeline, and
            // shooting toward an overlapping spawn corrupts the boss state. The
            // midboss times out and leaves; Enter (pulsed) steps the pre-boss
            // dialogue; god (forced by the caller) keeps the player alive.
            let mut held = Vec::new();
            if let Some((px, Some(tx))) = self.stage_aim() {
                if tx < px - 4.0 {
                    held.push(Key::Left);
                } else if tx > px + 4.0 {
                    held.push(Key::Right);
                }
            }
            let pressed: &[Key] = if _i % 12 == 0 { &[Key::Enter] } else { &[] };
            self.update(&Input::synthetic(&held, pressed));
            if let Scene::Stage(s) = &self.scene {
                // Detect the on-screen boss entity directly (the global
                // boss_present flag can be stale when bosses overlap). The
                // pre-boss dialogue starting the boss music distinguishes the
                // real boss from the dialogue-less midboss.
                if s.debug_boss_onscreen() && (s.debug_boss_music_started() == to_boss) {
                    return true;
                }
            } else {
                break;
            }
        }
        false
    }

    /// Coarse phase for the split recorder: menu (title/char-select), an
    /// in-progress stage (0-based index, covers its clear breakdown), or the
    /// post-run screens. Used to cut a continuous run into per-stage segments.
    pub fn record_phase(&self) -> RecordPhase {
        match &self.scene {
            Scene::Title | Scene::CharSelect { .. } => RecordPhase::Menu,
            Scene::Stage(_) => RecordPhase::Stage(self.current_stage),
            Scene::NameEntry { .. } | Scene::Leaderboard { .. } => RecordPhase::Ended,
        }
    }

    /// Headless auto-play aim: (player_x, target_x) while in a stage.
    /// Live bullets [x,y,angle,speed] of the current stage (full-VM oracle diff).
    pub fn stage_bullets(&self) -> Vec<[f32; 4]> {
        if let Scene::Stage(s) = &self.scene {
            s.bullets_dump()
        } else {
            Vec::new()
        }
    }

    /// Occupied enemy positions [x,y] of the current stage (oracle diff).
    pub fn stage_enemies(&self) -> Vec<[f32; 2]> {
        if let Scene::Stage(s) = &self.scene {
            s.enemies_dump()
        } else {
            Vec::new()
        }
    }

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

    /// Render the character-select with the real EoSD art: the select00
    /// emblem background, the highlighted character's slpl illustration, and
    /// the four select04 shot-name banners (current one lit).
    fn charselect_cmds(&self, cursor: usize) -> Vec<DrawCmd> {
        let sw = th06_engine::SCREEN_W as f32;
        let sh = th06_engine::SCREEN_H as f32;
        let bg = self.select_tex; // +0
        let slpl = self.select_tex + 1 + cursor; // +1..+4, CHARACTERS order
        let banners = self.select_tex + 5; // select04 (4 sprites of 256x48)
        let prompt = self.select_tex + 6; // select03 (sprite 0 = choose player)

        let full = |tex: usize, c: f32| DrawCmd {
            tex,
            dst: [0.0, 0.0, sw, sh],
            src: [0.0, 0.0, 1.0, 1.0],
            tint: [c, c, c, 1.0],
            rot: 0.0,
        };
        let mut cmds = vec![full(bg, 1.0)];

        // Character illustration (slpl is one 256x256 sprite), left side.
        cmds.push(DrawCmd {
            tex: slpl,
            dst: [24.0, 96.0, 256.0, 256.0],
            src: [0.0, 0.0, 1.0, 1.0],
            tint: [1.0, 1.0, 1.0, 1.0],
            rot: 0.0,
        });

        // "Select your player" prompt banner (select03 sprite 0), top.
        cmds.push(DrawCmd {
            tex: prompt,
            dst: [180.0, 24.0, 256.0, 64.0],
            src: [0.0, 0.0, 1.0, 64.0 / 256.0],
            tint: [1.0, 1.0, 1.0, 1.0],
            rot: 0.0,
        });

        // The four shot-name banners (select04, 256x48 each), stacked right;
        // the selected one full brightness, the rest dimmed.
        for i in 0..4 {
            let v0 = i as f32 * 48.0 / 256.0;
            let v1 = (i as f32 * 48.0 + 48.0) / 256.0;
            let c = if i == cursor { 1.0 } else { 0.4 };
            cmds.push(DrawCmd {
                tex: banners,
                dst: [310.0, 150.0 + i as f32 * 56.0, 256.0, 48.0],
                src: [0.0, v0, 1.0, v1],
                tint: [c, c, c, 1.0],
                rot: 0.0,
            });
        }

        stage::draw_text(&mut cmds, [320.0, 410.0], 13.0, [0.85, 0.85, 0.9, 1.0], "Z: start   X: back");
        cmds
    }

    /// Full-screen black backdrop used by the name-entry / leaderboard screens.
    fn dark_bg() -> DrawCmd {
        DrawCmd {
            tex: stage::TEX_WHITE,
            dst: [0.0, 0.0, th06_engine::SCREEN_W as f32, th06_engine::SCREEN_H as f32],
            src: [0.25, 0.25, 0.75, 0.75],
            tint: [0.02, 0.0, 0.05, 1.0],
            rot: 0.0,
        }
    }

    fn name_entry_cmds(&self, score: i64, stage: u8, name: &str, sel: usize) -> Vec<DrawCmd> {
        let mut cmds = vec![Self::dark_bg()];
        stage::draw_text(&mut cmds, [180.0, 40.0], 22.0, [1.0, 0.5, 0.5, 1.0], "GAME OVER");
        stage::draw_text(&mut cmds, [120.0, 84.0], 14.0, [1.0, 0.9, 0.5, 1.0], &format!("Score {:09}", score));
        stage::draw_text(&mut cmds, [120.0, 104.0], 14.0, [0.8, 0.85, 1.0, 1.0], &format!("Reached Stage {}", stage));
        stage::draw_text(&mut cmds, [120.0, 150.0], 14.0, [1.0, 1.0, 1.0, 1.0], "Enter your name:");

        // Current name with a blinking caret.
        let shown = if name.chars().count() < MAX_NAME_LEN { format!("{name}_") } else { name.to_string() };
        stage::draw_text(&mut cmds, [120.0, 176.0], 20.0, [0.6, 1.0, 0.6, 1.0], &shown);

        // Character ribbon grid; the selected glyph is highlighted.
        let ribbon: Vec<char> = NAME_RIBBON.chars().collect();
        let (ox, oy, cell) = (120.0f32, 220.0f32, 26.0f32);
        for (i, ch) in ribbon.iter().enumerate() {
            let col = (i % RIBBON_COLS) as f32;
            let row = (i / RIBBON_COLS) as f32;
            let x = ox + col * cell;
            let y = oy + row * cell;
            let tint = if i == sel { [1.0, 1.0, 0.3, 1.0] } else { [0.7, 0.7, 0.8, 1.0] };
            if i == sel {
                cmds.push(DrawCmd {
                    tex: stage::TEX_WHITE,
                    dst: [x - 3.0, y - 2.0, cell - 4.0, cell - 4.0],
                    src: [0.25, 0.25, 0.75, 0.75],
                    tint: [0.4, 0.4, 0.1, 0.8],
                    rot: 0.0,
                });
            }
            stage::draw_text(&mut cmds, [x, y], 18.0, tint, &ch.to_string());
        }
        stage::draw_text(&mut cmds, [120.0, 410.0], 12.0, [0.8, 0.8, 0.9, 1.0],
            "Arrows: pick   Z: add   X: erase   Enter: confirm");
        cmds
    }

    fn leaderboard_cmds(&self, highlight: Option<usize>) -> Vec<DrawCmd> {
        let mut cmds = vec![Self::dark_bg()];
        stage::draw_text(&mut cmds, [210.0, 36.0], 22.0, [1.0, 0.85, 0.4, 1.0], "HIGH SCORES");
        let (rx, nx, sx, tx, oy, dy) = (70.0f32, 120.0f32, 280.0f32, 470.0f32, 90.0f32, 30.0f32);
        stage::draw_text(&mut cmds, [rx, oy], 13.0, [0.6, 0.7, 1.0, 1.0], "#");
        stage::draw_text(&mut cmds, [nx, oy], 13.0, [0.6, 0.7, 1.0, 1.0], "Name");
        stage::draw_text(&mut cmds, [sx, oy], 13.0, [0.6, 0.7, 1.0, 1.0], "Score");
        stage::draw_text(&mut cmds, [tx, oy], 13.0, [0.6, 0.7, 1.0, 1.0], "Stage");
        for i in 0..MAX_SCORES {
            let y = oy + 24.0 + i as f32 * dy;
            let hot = highlight == Some(i);
            let tint = if hot { [1.0, 1.0, 0.4, 1.0] } else { [0.9, 0.9, 0.95, 1.0] };
            stage::draw_text(&mut cmds, [rx, y], 16.0, tint, &format!("{}", i + 1));
            if let Some(e) = self.scores.get(i) {
                stage::draw_text(&mut cmds, [nx, y], 16.0, tint, &e.name);
                stage::draw_text(&mut cmds, [sx, y], 16.0, tint, &format!("{:09}", e.score));
                stage::draw_text(&mut cmds, [tx, y], 16.0, tint, &format!("{}", e.stage));
            } else {
                stage::draw_text(&mut cmds, [nx, y], 16.0, [0.4, 0.4, 0.5, 1.0], "--------");
            }
        }
        stage::draw_text(&mut cmds, [200.0, 440.0], 13.0, [0.85, 0.85, 0.9, 1.0], "Z: return to title");
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

        // Name entry (after a qualifying game over): a char ribbon driven by the
        // arrows, Z appends, X backspaces, Enter confirms.
        if let Scene::NameEntry { score, stage, name, sel } = &self.scene {
            let (score, stage) = (*score, *stage);
            let mut name = name.clone();
            let mut sel = *sel;
            let ribbon: Vec<char> = NAME_RIBBON.chars().collect();
            let n = ribbon.len();
            let sfx = |g: &Game| if let Some(a) = &g.audio { a.play_sfx("tan00"); };
            if input.pressed(Key::Left) { sel = (sel + n - 1) % n; sfx(self); }
            if input.pressed(Key::Right) { sel = (sel + 1) % n; sfx(self); }
            if input.pressed(Key::Up) { sel = (sel + n - RIBBON_COLS) % n; sfx(self); }
            if input.pressed(Key::Down) { sel = (sel + RIBBON_COLS) % n; sfx(self); }
            if input.pressed(Key::Shoot) && name.chars().count() < MAX_NAME_LEN {
                name.push(ribbon[sel]);
                if let Some(a) = &self.audio { a.play_sfx("plst00"); }
            }
            if input.pressed(Key::Bomb) {
                name.pop();
                sfx(self);
            }
            if input.pressed(Key::Enter) || input.pressed(Key::Pause) {
                let final_name = if name.trim().is_empty() { "PLAYER".to_string() } else { name.trim().to_string() };
                let rank = self.insert_score(final_name, score, stage);
                self.save_scores();
                self.scene = Scene::Leaderboard { highlight: Some(rank) };
                return Frame { cmds: self.leaderboard_cmds(Some(rank)), bg: None, quit: false };
            }
            let cmds = self.name_entry_cmds(score, stage, &name, sel);
            self.scene = Scene::NameEntry { score, stage, name, sel };
            return Frame { cmds, bg: None, quit: false };
        }

        // Leaderboard: any confirm returns to the title.
        if let Scene::Leaderboard { highlight } = &self.scene {
            let hl = *highlight;
            if input.pressed(Key::Shoot) || input.pressed(Key::Enter) || input.pressed(Key::Bomb) {
                self.scene = Scene::Title;
                self.title.reset();
                self.play_bgm("th06_01.wav");
                return Frame { cmds: Vec::new(), bg: None, quit: false };
            }
            return Frame { cmds: self.leaderboard_cmds(hl), bg: None, quit: false };
        }

        match &mut self.scene {
            Scene::CharSelect { .. } => unreachable!("handled above"),
            Scene::NameEntry { .. } | Scene::Leaderboard { .. } => unreachable!("handled above"),
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
                let mut game_over: Option<i64> = None;
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
                        Event::GameOver(score) => game_over = Some(score),
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
                if let Some(score) = game_over {
                    // A qualifying score opens name entry; otherwise straight to
                    // the leaderboard. Either way the run is over.
                    let stage = self.current_stage as u8 + 1;
                    self.play_bgm("th06_01.wav");
                    self.scene = if self.score_qualifies(score) {
                        Scene::NameEntry { score, stage, name: String::new(), sel: 0 }
                    } else {
                        Scene::Leaderboard { highlight: None }
                    };
                    return Frame { cmds, bg: None, quit: false };
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
