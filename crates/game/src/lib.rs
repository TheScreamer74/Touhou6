//! Shared game assembly: turns a set of in-memory archive byte maps into
//! GPU textures plus a runnable `Game`. Both the native binary (`main.rs`,
//! files read from disk) and the web entry (`web.rs`, files uploaded in the
//! browser) call [`build_game`] — no filesystem access lives here.

pub mod anm_vm;
pub mod background;
pub mod community;
pub mod ecl_vm;
pub mod ending;
pub mod hud;
pub mod key_config;
pub mod localization;
pub mod menu_art;
pub mod music_room;
pub mod options;
mod player_shots;
pub mod projectiles;
pub mod replay;
pub mod replay_menu;
pub mod result;
pub mod retry_menu;
pub mod save;
mod score_popups;
pub mod selection;
mod sprite_geometry;
pub mod stage;
pub mod title;

#[cfg(target_arch = "wasm32")]
pub mod web;

use std::collections::HashMap;

use th06_engine::audio::Audio;
use th06_engine::{DrawCmd, Engine, Frame, Input, Key, Texture, compose_rgba};
use th06_formats::anm0::Anm0;
use th06_formats::ecl::Ecl;
use th06_formats::msg::Msg;
use th06_formats::std::Std;

use background::Background;
use selection::{RunMode, Selection, SelectionAction};
pub use stage::{Character, Difficulty};
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
    pub ed: HashMap<String, Vec<u8>>,
    pub music_comments: Vec<u8>,
    /// BGM wavs keyed by basename ("th06_01.wav", ...).
    pub bgm: HashMap<String, Vec<u8>>,
    /// Per-track loop points (loop_start, loop_end) in sample-frames, from the
    /// track's `.pos` file (decomp SoundPlayer::LoadPos). Absent for tracks
    /// shipped without a `.pos`, in which case the whole track loops.
    pub bgm_pos: HashMap<String, (u32, u32)>,
    /// Original MIDI files from MD.DAT (the game used a system MIDI device).
    pub midi: HashMap<String, Vec<u8>>,
    /// User-supplied or freely licensed General MIDI SoundFont, never game data.
    pub soundfont: Option<Vec<u8>>,
    /// Optional user-supplied MS Gothic TTC/TTF. No proprietary font is bundled.
    pub text_font: Option<Vec<u8>>,
}

/// Reject incomplete uploads before texture creation can index missing entries.
pub fn validate_game_files(files: &GameFiles) -> Result<(), String> {
    let required =
        |archive: &HashMap<String, Vec<u8>>, label: &str, names: &[&str]| -> Result<(), String> {
            for name in names {
                if !archive.contains_key(*name) {
                    return Err(format!(
                        "{label}: missing {name}. Use the full Touhou 6 v1.02h game folder."
                    ));
                }
            }
            Ok(())
        };
    required(
        &files.tl,
        "TL.DAT",
        &[
            "title00.jpg",
            "title01.anm",
            "title02.anm",
            "title03.anm",
            "title04.anm",
            "select00.jpg",
            "select01.anm",
            "select02.anm",
            "select03.anm",
            "select04.anm",
            "select05.anm",
            "slpl00a.anm",
            "slpl00b.anm",
            "slpl01a.anm",
            "slpl01b.anm",
            "replay00.anm",
        ],
    )?;
    required(
        &files.cm,
        "CM.DAT",
        &[
            "player00.anm",
            "player01.anm",
            "front.anm",
            "etama3.anm",
            "etama4.anm",
            "face00a.anm",
            "face00b.anm",
            "face00c.anm",
            "face01a.anm",
            "face01b.anm",
            "face01c.anm",
        ],
    )?;
    required(
        &files.inn,
        "IN.DAT",
        &["ascii.anm", "ascii_a.png", "text.anm", "th06logo.jpg"],
    )?;
    for stage in 1..=TOTAL_STAGES {
        for name in [
            format!("ecldata{stage}.ecl"),
            format!("stage{stage}.std"),
            format!("stg{stage}bg.anm"),
            format!("stg{stage}enm.anm"),
            format!("msg{stage}.dat"),
        ] {
            required(&files.st, "ST.DAT", &[&name])?;
        }
    }
    // Every ANM names its image files; check both color and optional mask now.
    for (label, archive) in [
        ("TL.DAT", &files.tl),
        ("CM.DAT", &files.cm),
        ("ST.DAT", &files.st),
        ("IN.DAT", &files.inn),
    ] {
        for (name, bytes) in archive.iter().filter(|(name, _)| name.ends_with(".anm")) {
            let anm = Anm0::parse(bytes).map_err(|_| format!("{label}: invalid {name}"))?;
            for entry in anm.entries {
                if entry.name != "@" {
                    required(archive, label, &[basename(&entry.name)])?;
                    if let Some(mask) = entry.alpha_name {
                        required(archive, label, &[basename(&mask)])?;
                    }
                }
            }
        }
    }
    Ok(())
}

/// Every sound effect (SoundPlayer.cpp g_SFXList order; index == SoundIdx).
/// Loaded by basename from the archive; absent ones are simply skipped.
use th06_engine::audio::SFX_FILES as SFX_NAMES;

/// All BGM tracks used in the main game (title + 6 stages × field/boss).
const BGM_NAMES: [&str; 17] = [
    "th06_01.wav",
    "th06_02.wav",
    "th06_03.wav",
    "th06_04.wav",
    "th06_05.wav",
    "th06_06.wav",
    "th06_07.wav",
    "th06_08.wav",
    "th06_09.wav",
    "th06_10.wav",
    "th06_11.wav",
    "th06_12.wav",
    "th06_13.wav",
    "th06_14.wav",
    "th06_15.wav",
    "th06_16.wav",
    "th06_17.wav",
];

/// Number of normal-mode stages.
pub const N_STAGES: usize = 6;
pub const TOTAL_STAGES: usize = 7;
/// Per-stage field theme.
const STAGE_BGM: [&str; TOTAL_STAGES] = [
    "th06_02.wav",
    "th06_04.wav",
    "th06_06.wav",
    "th06_08.wav",
    "th06_10.wav",
    "th06_12.wav",
    "th06_14.wav",
];
/// Per-stage boss theme.
const BOSS_BGM: [&str; TOTAL_STAGES] = [
    "th06_03.wav",
    "th06_05.wav",
    "th06_07.wav",
    "th06_09.wav",
    "th06_11.wav",
    "th06_13.wav",
    "th06_15.wav",
];
/// Per-stage boss dialogue portrait (Gui.cpp FACE_STAGE_A), all in ST.DAT.
const BOSS_FACE: [&str; TOTAL_STAGES] = [
    "face03a", "face05a", "face06a", "face08a", "face09a", "face09b", "face12a",
];
/// Whether stage N ships a separate boss sprite sheet (stgNenm2); stages 3 & 4
/// keep their boss in the main enemy sheet.
const HAS_ENM2: [bool; TOTAL_STAGES] = [true, true, false, false, true, true, true];

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
    boss_portraits: Vec<stage::PortraitAsset>,
    extra_portrait: Option<stage::PortraitAsset>,
    /// SPELLCARDSTART id -> (texture slot, width, height) for the pre-baked
    /// original Japanese spellcard-name textures.
    spell_name_tex: HashMap<i32, (usize, f32, f32)>,
    /// TEXTDIALOGUE instruction offset -> pre-baked JP line texture.
    dialogue_tex: HashMap<u32, (usize, f32, f32)>,
    /// Pre-baked JP stage-name texture (STD header name[128]), shown at start.
    stage_name_tex: Option<(usize, f32, f32)>,
    song_name_tex: Vec<(usize, f32, f32)>,
    spell_background: stage::PortraitAsset,
    late_spell_background: Option<stage::PortraitAsset>,
}

/// Shared (cross-stage) assets plus the per-stage table.
struct Assets {
    player: Anm0,
    player_tex: usize,
    player_marisa: Anm0,
    player_marisa_tex: usize,
    bomb_name_tex: [(usize, usize); 4],
    etama: Anm0,
    etama4: Anm0,
    etama4_tex: usize,
    front: Anm0,
    text: Anm0,
    ascii: Anm0,
    face_chara: Anm0,
    player_portraits: [Vec<stage::PortraitAsset>; 2],
    loading: stage::PortraitAsset,
    stages: Vec<StageData>,
}

impl Assets {
    fn new_stage(
        &self,
        idx: usize,
        character: Character,
        difficulty: Difficulty,
        practice: bool,
        starting_lives: i32,
        starting_bombs: i32,
    ) -> Stage {
        let sd = &self.stages[idx];
        let ecl = Ecl::parse(sd.ecl_data.clone()).expect("parse ecl");
        let mut pairs = vec![(&sd.enm.entries[0], sd.enm_tex)];
        if let Some(enm2) = &sd.enm2 {
            pairs.push((&enm2.entries[0], sd.enm2_tex));
        }
        let scripts = stage::build_enemy_scripts(&pairs);
        let msg = Msg::parse(sd.msg_data.clone()).expect("parse msg");
        let background =
            Std::parse(&sd.std_data).map(|std| Background::new(std, &sd.bg.entries[0], sd.bg_tex));
        let (player_anm, player_tex, face_player_tex) = if character.is_marisa() {
            (
                &self.player_marisa.entries[0],
                self.player_marisa_tex,
                stage::TEX_FACE_MARISA,
            )
        } else {
            (
                &self.player.entries[0],
                self.player_tex,
                stage::TEX_FACE_REIMU,
            )
        };
        let cfg = stage::StageConfig {
            face_player_tex,
            face_boss_tex: sd.boss_face_tex,
            stage_bgm: STAGE_BGM[idx],
            boss_bgm: BOSS_BGM[idx],
            stage_num: idx as i32 + 1,
            difficulty,
            practice,
            starting_lives,
            starting_bombs,
            rng_seed: 0,
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
        // text.anm script 0 = TEXT_STAGE_NAME (the stage-name fly-in at start).
        let stage_name_script = self.text.entries[0]
            .scripts
            .iter()
            .find(|(id, _)| *id == 0)
            .map(|(_, i)| i.clone())
            .unwrap_or_default();
        let mut result = Stage::new(
            ecl,
            scripts,
            &self.etama.entries[0],
            &self.etama4.entries[0],
            self.etama4_tex,
            player_anm,
            player_tex,
            character,
            msg,
            background,
            hud,
            cfg,
            spell_name_script,
            portrait_script,
            sd.spell_name_tex.clone(),
            sd.dialogue_tex.clone(),
            stage_name_script,
            sd.stage_name_tex,
        );
        result.configure_loading_screen(self.loading.clone());
        result.configure_spell_background(
            sd.spell_background.clone(),
            sd.late_spell_background.clone(),
        );
        result.configure_song_names(
            self.text.entries[0]
                .scripts
                .iter()
                .find(|(id, _)| *id == 1)
                .map(|(_, instructions)| instructions.clone())
                .unwrap_or_default(),
            sd.song_name_tex.clone(),
        );
        result.set_portrait_assets(
            self.player_portraits[usize::from(character.is_marisa())].clone(),
            sd.boss_portraits.clone(),
        );
        result.configure_bomb_banner(&self.text.entries[0], self.bomb_name_tex);
        result.set_dialogue_scripts(&self.text.entries[0]);
        result.set_pause_art(stage::PortraitAsset::new(
            stage::TEX_ASCII,
            &self.ascii.entries[0],
        ));
        if let Some(portrait) = &sd.extra_portrait {
            result.set_extra_boss_portrait(portrait.clone());
        }
        result
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
    Selection,
    MusicRoom,
    Options,
    Replay,
    Ending {
        player: Box<ending::EndingPlayer>,
        score: i64,
    },
    Continue {
        stage: Box<Stage>,
        frozen: Vec<DrawCmd>,
        score: i64,
    },
    Stage(Box<Stage>),
    Result,
}

/// One high-score table row.
#[derive(Clone)]
pub struct ScoreEntry {
    pub name: String,
    pub score: i64,
    pub stage: u8,
}

const MAX_SCORES: usize = 10;

pub const CHARACTERS: [Character; 4] = [
    Character::ReimuA,
    Character::ReimuB,
    Character::MarisaA,
    Character::MarisaB,
];

pub struct ReplayReferenceTerminal {
    pub event: &'static str,
    pub frame: i32,
    pub stage: usize,
    pub state: String,
    pub pools: String,
    pub lifecycle: String,
}

impl ReplayReferenceTerminal {
    pub fn json_line(&self) -> String {
        format!(
            "{{\"event\":\"{}\",\"frame\":{},\"stage\":{},\"state\":\"{}\",\"pools\":{},\"lifecycle\":\"{}\"}}\n",
            self.event,
            self.frame,
            self.stage,
            self.state,
            self.pools.trim_end(),
            self.lifecycle
        )
    }
}

pub struct Game {
    scene: Scene,
    title: Title,
    selection: Selection,
    music_room: music_room::MusicRoom,
    options_menu: options::OptionsMenu,
    retry_menu: retry_menu::RetryMenu,
    pub options: options::Options,
    difficulty: Difficulty,
    mode: RunMode,
    pub progress: th06_formats::score::ScoreData,
    progress_path: String,
    options_path: String,
    controller_path: String,
    replay_directory: String,
    replay_session: Option<replay::Session>,
    replay_reference_terminal: Option<ReplayReferenceTerminal>,
    pending_stage_transition: bool,
    pending_stage_back: bool,
    pending_replay_start: Option<(th06_formats::replay::Replay, usize)>,
    pending_live_start: Option<Box<Stage>>,
    pending_practice_exit: u8,
    pending_result: Option<i64>,
    pending_result_back: bool,
    replay_menu: replay_menu::ReplayMenu,
    replay_slots: Vec<Option<th06_formats::replay::Replay>>,
    ending_assets: Result<ending::EndingAssets, ending::EndingError>,
    result: result::ResultScreen,
    pending_stats: Option<result::RunStats>,
    result_stats: Option<result::RunStats>,
    active_frames: u32,
    community_run_open: bool,
    community_results: Vec<serde_json::Value>,
    fps_counter: hud::FpsCounter,
    draw_serial: u64,
    diagnostic_draw_millis: Option<u32>,
    diagnostic_color_mode: Option<bool>,
    diagnostic_stage_seed: Option<u16>,
    diagnostic_score_fixture: Option<(u64, i64)>,
    title_idle: u16,
    demo_frames: Option<u32>,
    demo_replay: Option<th06_formats::replay::Replay>,
    pub persistence_error: Option<String>,
    starting_lives: i32,
    starting_bombs: i32,
    audio: Option<Audio>,
    audio_stage: usize,
    pending_music_room_back: bool,
    assets: Assets,
    /// Stage currently being played (0-based), and the chosen character — used
    /// to build the next stage on clear.
    current_stage: usize,
    /// GameManager's camera direction survives stage registration and menus.
    stage_camera_facing: [f32; 3],
    character: Character,
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
    localization::begin_textures();
    if let Err(error) = th06_engine::text::set_font(files.text_font.clone()) {
        eprintln!("{error}; using bundled font");
    }
    let anm = Anm0::parse(&files.tl["title01.anm"]).expect("parse title01.anm");
    let entry = &anm.entries[0];
    let mut texture_formats = HashMap::new();
    for (name, bytes) in files
        .tl
        .iter()
        .chain(&files.cm)
        .chain(&files.st)
        .chain(&files.inn)
    {
        if name.ends_with(".anm")
            && let Ok(anm) = Anm0::parse(bytes)
        {
            for entry in anm.entries {
                texture_formats.insert(basename(&entry.name).to_string(), entry.format);
            }
        }
    }

    // Upload a composed texture and return its slot; the optional alpha mask
    // is skipped when absent (some sheets ship without a `_a` mask).
    let load = |map: &HashMap<String, Vec<u8>>,
                color: &str,
                mask: Option<&str>,
                textures: &mut Vec<Texture>|
     -> usize {
        let alpha = mask.and_then(|m| map.get(m)).map(|v| v.as_slice());
        let (rgba, w, h) = compose_rgba(&map[color], alpha);
        let slot = textures.len();
        textures.push(engine.create_texture_with_format(
            &rgba,
            w,
            h,
            texture_formats.get(color).copied().unwrap_or(1),
        ));
        slot
    };
    // Fixed slots (referenced by stage.rs constants): 0 title bg, 1 title menu,
    // 2 player00, 3 etama3, 4 stg1enm, 5 stg1enm2, 6 front, 7 white, 8 ascii,
    // 9 face00a (Reimu), 10 face01a (Marisa), 11 stg1bg, 12 player01.
    let mut textures = Vec::new();
    let (bg_rgba, bg_w, bg_h) = compose_rgba(&files.tl["title00.jpg"], None);
    textures.push(engine.create_texture_with_format(&bg_rgba, bg_w, bg_h, 4)); // 0
    let alpha = entry
        .alpha_name
        .as_deref()
        .map(|n| files.tl[basename(n)].as_slice());
    let (rgba, w, h) = compose_rgba(&files.tl[basename(&entry.name)], alpha);
    textures.push(engine.create_texture_with_format(&rgba, w, h, entry.format)); // 1
    for (archive, color, mask) in [
        (&files.cm, "player00.png", Some("player00_a.png")),
        (&files.cm, "etama3.png", Some("etama3_a.png")),
        (&files.st, "stg1enm.png", Some("stg1enm_a.png")),
        (&files.st, "stg1enm2.png", Some("stg1enm2_a.png")),
        (&files.cm, "front.png", Some("front_a.png")),
    ] {
        let alpha = mask.map(|m| archive[m].as_slice());
        let (rgba, w, h) = compose_rgba(&archive[color], alpha);
        textures.push(engine.create_texture_with_format(
            &rgba,
            w,
            h,
            texture_formats.get(color).copied().unwrap_or(1),
        )); // 2,3,4,5,6
    }
    textures.push(engine.create_texture(&[255u8; 2 * 2 * 4], 2, 2)); // 7 white
    // 8: original ASCII color atlas plus its separate alpha mask. Using the
    // mask as color would replace the original outlined digits with solid ink.
    let (rgba, w, h) = compose_rgba(
        &files.inn["ascii.png"],
        Some(files.inn["ascii_a.png"].as_slice()),
    );
    textures.push(engine.create_texture_with_format(
        &rgba,
        w,
        h,
        texture_formats.get("ascii.png").copied().unwrap_or(1),
    ));
    // 9-10: player dialogue portraits (Reimu, Marisa).
    load(
        &files.cm,
        "face00a.png",
        Some("face00a_a.png"),
        &mut textures,
    ); // 9
    load(
        &files.cm,
        "face01a.png",
        Some("face01a_a.png"),
        &mut textures,
    ); // 10
    // 11: stage 1 background texture.
    load(&files.st, "stg1bg.png", Some("stg1bg_a.png"), &mut textures);
    // 12: Marisa player body sprite (player01).
    let player_marisa_tex = load(
        &files.cm,
        "player01.png",
        Some("player01_a.png"),
        &mut textures,
    );
    // 13: untextured diffuse gradient: original D3DCOLOR is ARGB, so
    // 0xe0e0e0ff -> 0x80e0e0ff changes alpha, retaining RGB (224,224,255).
    // Diffuse vertex colors retain 8-bit precision even in 16-bit mode.
    debug_assert_eq!(textures.len(), stage::TEX_POWER_GRAD);
    textures.push(engine.create_diffuse_texture(&[224, 224, 255, 224, 224, 224, 255, 128], 2, 1));
    // 14: etama4 (particle/effect sprite sheet). Fixed slot before the dynamic
    // per-stage textures.
    let etama4_tex = load(&files.cm, "etama4.png", Some("etama4_a.png"), &mut textures);

    let portrait = |archive: &HashMap<String, Vec<u8>>, name: &str, textures: &mut Vec<Texture>| {
        let tex = load(
            archive,
            &format!("{name}.png"),
            Some(&format!("{name}_a.png")),
            textures,
        );
        let anm = Anm0::parse(&archive[&format!("{name}.anm")]).expect("portrait ANM");
        stage::PortraitAsset::new(tex, &anm.entries[0])
    };
    let player_portraits = std::array::from_fn(|c| {
        ["a", "b", "c"]
            .iter()
            .map(|suffix| portrait(&files.cm, &format!("face{c:02}{suffix}"), &mut textures))
            .collect()
    });
    let bomb_name_tex = [
        "霊符「夢想封印」",
        "夢符「封魔陣」",
        "魔符「スターダストレヴァリエ」",
        "恋符「マスタースパーク」",
    ]
    .map(|name| {
        let (bytes, _, _) = encoding_rs::SHIFT_JIS.encode(name);
        let (slot, _, _) = localization::bake_sprite_shiftjis_aligned(
            engine,
            &mut textures,
            &bytes,
            256,
            16,
            15,
            th06_engine::text::SpriteTextAlign::Left,
            [240, 240, 255],
            1,
        );
        (slot, bytes.len())
    });
    const BOSS_PORTRAITS: [&[&str]; TOTAL_STAGES] = [
        &["face03a", "face03b"],
        &["face05a"],
        &["face06a", "face06b"],
        &["face08a", "face08b"],
        &["face09a", "face09b"],
        &["face09b", "face10a", "face10b"],
        &["face08a", "face12a", "face12b"],
    ];
    // Per-stage sprite sheets + boss faces, appended after the fixed slots.
    let mut stages: Vec<StageData> = Vec::new();
    for n in 1..=TOTAL_STAGES {
        let (enm_tex, enm2, enm2_tex, bg_tex) = if n == 1 {
            // Stage 1 reuses the fixed slots loaded above.
            (
                4,
                Some(Anm0::parse(&files.st["stg1enm2.anm"]).expect("stg1enm2")),
                5,
                11,
            )
        } else {
            let enm_tex = load(
                &files.st,
                &format!("stg{n}enm.png"),
                Some(&format!("stg{n}enm_a.png")),
                &mut textures,
            );
            let (enm2, enm2_tex) = if HAS_ENM2[n - 1] {
                let t = load(
                    &files.st,
                    &format!("stg{n}enm2.png"),
                    Some(&format!("stg{n}enm2_a.png")),
                    &mut textures,
                );
                (
                    Some(Anm0::parse(&files.st[&format!("stg{n}enm2.anm")]).expect("enm2")),
                    t,
                )
            } else {
                (None, 0)
            };
            let bg_tex = load(
                &files.st,
                &format!("stg{n}bg.png"),
                Some(&format!("stg{n}bg_a.png")),
                &mut textures,
            );
            (enm_tex, enm2, enm2_tex, bg_tex)
        };
        let face = BOSS_FACE[n - 1];
        let boss_face_tex = load(
            &files.st,
            &format!("{face}.png"),
            Some(&format!("{face}_a.png")),
            &mut textures,
        );
        // Pre-bake the original Japanese spellcard names (Shift-JIS in the ecl)
        // into textures — the announce banner draws these instead of the English
        // stopgap. ShowSpellcard colour: 0xfff0f0 text, black shadow (Gui.cpp:235).
        let ecl_bytes = files.st[&format!("ecldata{n}.ecl")].clone();
        let mut spell_name_tex = HashMap::new();
        if let Ok(ecl) = th06_formats::ecl::Ecl::parse(ecl_bytes.clone()) {
            for (id, raw) in stage::scan_spell_names(&ecl) {
                if raw.is_empty() || spell_name_tex.contains_key(&id) {
                    continue;
                }
                let texture = localization::bake_sprite_shiftjis_aligned(
                    engine,
                    &mut textures,
                    &raw,
                    256,
                    16,
                    15,
                    th06_engine::text::SpriteTextAlign::Right,
                    [255, 240, 240],
                    1,
                );
                spell_name_tex.insert(id, texture);
            }
        }
        // Original Japanese dialogue lives in ST.DAT (the port previously used the
        // English patch th06e_ST.DAT). Pre-bake each TEXTDIALOGUE line texture.
        let msg_data = files
            .st
            .get(&format!("msg{n}.dat"))
            .cloned()
            .unwrap_or_else(|| files.st_en[&format!("msg{n}.dat")].clone());
        let mut dialogue_tex = HashMap::new();
        if let Some(msg) = th06_formats::msg::Msg::parse(msg_data.clone()) {
            for (off, raw) in stage::scan_dialogue_lines(&msg) {
                if dialogue_tex.contains_key(&off) {
                    continue;
                }
                // White text + black shadow; the line colour (player/boss) is a
                // draw-time tint (Gui line_colors), so keep the bake neutral.
                let texture = localization::bake_shiftjis(
                    engine,
                    &mut textures,
                    &raw,
                    16.0,
                    [255; 3],
                    Some([0; 3]),
                    [256, 18],
                );
                dialogue_tex.insert(off, texture);
            }
        }
        // Stage name: STD header name[128] at offset 0x10 (Gui.cpp:438 uses
        // stdData->stageName), drawn light-cyan with a black shadow (Gui.cpp:437).
        let std_bytes = files.st[&format!("stage{n}.std")].clone();
        let stage_name_tex = std_bytes.get(0x10..0x10 + 128).and_then(|nm| {
            let end = nm.iter().position(|&c| c == 0).unwrap_or(nm.len());
            if end == 0 {
                return None;
            }
            Some(localization::bake_sprite_shiftjis_aligned(
                engine,
                &mut textures,
                &nm[..end],
                256,
                16,
                15,
                th06_engine::text::SpriteTextAlign::Center,
                [224, 255, 255],
                5,
            ))
        });
        let song_name_tex = (0..4)
            .map(|song| {
                let raw = &std_bytes[0x90 + song * 128..0x90 + (song + 1) * 128];
                let end = raw.iter().position(|&c| c == 0).unwrap_or(raw.len());
                let mut name = vec![0x81, 0xf4]; // Shift-JIS ♪, TH_SONG_NAME
                name.extend_from_slice(&raw[..end]);
                localization::bake_sprite_shiftjis_aligned(
                    engine,
                    &mut textures,
                    &name,
                    256,
                    16,
                    16,
                    th06_engine::text::SpriteTextAlign::Right,
                    [224, 255, 255],
                    5,
                )
            })
            .collect();
        stages.push(StageData {
            ecl_data: ecl_bytes,
            msg_data,
            std_data: std_bytes,
            enm: Anm0::parse(&files.st[&format!("stg{n}enm.anm")]).expect("enm"),
            enm_tex,
            enm2,
            enm2_tex,
            bg: Anm0::parse(&files.st[&format!("stg{n}bg.anm")]).expect("bg"),
            bg_tex,
            boss_face_tex,
            boss_portraits: BOSS_PORTRAITS[n - 1]
                .iter()
                .map(|name| portrait(&files.st, name, &mut textures))
                .collect(),
            extra_portrait: (n == 7).then(|| portrait(&files.st, "face12c", &mut textures)),
            spell_name_tex,
            dialogue_tex,
            stage_name_tex,
            song_name_tex,
            spell_background: portrait(
                &files.st,
                &format!(
                    "eff0{}",
                    match n {
                        6 => 5,
                        7 => 4,
                        _ => n,
                    }
                ),
                &mut textures,
            ),
            late_spell_background: (n >= 6)
                .then(|| portrait(&files.st, &format!("eff0{n}"), &mut textures)),
        });
    }

    let title_sheets = [
        ("title01.anm", 0),
        ("title02.anm", 27),
        ("title03.anm", 31),
        ("title04.anm", 34),
    ];
    let title = Title::new(menu_art::MenuArt::load(
        engine,
        &files.tl,
        &mut textures,
        "title00.jpg",
        &title_sheets,
    ));
    let mut options_menu = options::OptionsMenu::new(menu_art::MenuArt::load(
        engine,
        &files.tl,
        &mut textures,
        "title00.jpg",
        &title_sheets,
    ));
    // MainMenu::LoadDiffCharSelect, ANM offsets relative to SELECT01 (0x150).
    let selection_font = menu_art::MenuFont::load(engine, &files.inn, &mut textures);
    let selection = Selection::new(
        menu_art::MenuArt::load(
            engine,
            &files.tl,
            &mut textures,
            "select00.jpg",
            &[
                ("select01.anm", 0),
                ("select02.anm", 3),
                ("select05.anm", 5),
                ("slpl00a.anm", 6),
                ("slpl00b.anm", 7),
                ("slpl01a.anm", 8),
                ("slpl01b.anm", 9),
                ("select03.anm", 10),
                ("select04.anm", 12),
            ],
        ),
        selection_font,
    );

    let music_room = music_room::MusicRoom::load(engine, files, &mut textures);
    let replay_menu = replay_menu::ReplayMenu::new(engine, &files.tl, &files.inn, &mut textures);
    let mut result = result::ResultScreen::new(engine, &files.tl, &files.inn, &mut textures);
    result.load_spell_names(
        engine,
        &mut textures,
        stages.iter().flat_map(|stage| {
            Ecl::parse(stage.ecl_data.clone())
                .map(|ecl| stage::scan_spell_names(&ecl))
                .unwrap_or_default()
        }),
    );
    let ending_assets =
        ending::EndingAssets::load(engine, &files.ed, &files.inn["text.anm"], &mut textures);
    let retry_menu = retry_menu::RetryMenu::new(menu_art::MenuArt::load(
        engine,
        &files.inn,
        &mut textures,
        "th06logo.jpg",
        &[("ascii.anm", 0)],
    ));

    let mut audio = if with_audio { Audio::new() } else { None };
    if let Some(a) = &mut audio {
        if let Some(bank) = &files.soundfont
            && let Err(error) = a.register_soundfont(bank)
        {
            eprintln!("{error}");
        }
        for (name, bytes) in &files.midi {
            if let Err(error) = a.register_midi(name, bytes.clone()) {
                eprintln!("{name}: {error}");
            }
        }
        for name in SFX_NAMES {
            if let Some(wav) = files.inn.get(&format!("{name}.wav")) {
                a.register_sfx(name, wav.clone());
            }
        }
        for name in BGM_NAMES {
            if let Some(wav) = files.bgm.get(name) {
                a.register_bgm(name, wav.clone(), files.bgm_pos.get(name).copied());
            }
        }
    }

    options_menu.midi_available = if with_audio {
        audio.as_ref().is_some_and(Audio::midi_available)
    } else {
        // Headless diagnostics still use the supplied assets' availability;
        // absence of a hardware output must not change the original menu.
        files
            .soundfont
            .as_deref()
            .is_some_and(|bank| th06_engine::midi::soundfont(bank).is_ok())
            && (1..=17).all(|index| {
                files
                    .midi
                    .get(&format!("th06_{index:02}.mid"))
                    .is_some_and(|bytes| th06_engine::midi::Sequence::parse(bytes).is_ok())
            })
    };
    let assets = Assets {
        player: Anm0::parse(&files.cm["player00.anm"]).expect("parse player00"),
        player_tex: stage::TEX_PLAYER,
        player_marisa: Anm0::parse(&files.cm["player01.anm"]).expect("parse player01"),
        player_marisa_tex,
        bomb_name_tex,
        etama: Anm0::parse(&files.cm["etama3.anm"]).expect("parse etama3"),
        etama4: Anm0::parse(&files.cm["etama4.anm"]).expect("parse etama4"),
        etama4_tex,
        front: Anm0::parse(&files.cm["front.anm"]).expect("parse front"),
        ascii: Anm0::parse(&files.inn["ascii.anm"]).expect("parse ascii"),
        text: Anm0::parse(&files.inn["text.anm"]).expect("parse text"),
        player_portraits,
        loading: portrait(&files.cm, "loading", &mut textures),
        face_chara: Anm0::parse(&files.cm["face00a.anm"]).expect("parse face00a"),
        stages,
    };

    let game = Game {
        scene: Scene::Title,
        title,
        selection,
        music_room,
        options_menu,
        retry_menu,
        options: options::Options::default(),
        difficulty: Difficulty::Normal,
        mode: RunMode::Story,
        progress: th06_formats::score::ScoreData::default(),
        progress_path: "score.dat".into(),
        options_path: "th06-options.bin".into(),
        controller_path: "th06-controller.bin".into(),
        replay_directory: "replay".into(),
        replay_session: None,
        replay_reference_terminal: None,
        pending_stage_transition: false,
        pending_stage_back: false,
        pending_replay_start: None,
        pending_live_start: None,
        pending_practice_exit: 0,
        pending_result: None,
        pending_result_back: false,
        persistence_error: None,
        starting_lives: 3,
        starting_bombs: 3,
        replay_menu,
        replay_slots: Vec::new(),
        ending_assets,
        result,
        pending_stats: None,
        result_stats: None,
        active_frames: 0,
        community_run_open: false,
        community_results: Vec::new(),
        fps_counter: hud::FpsCounter::default(),
        draw_serial: 0,
        diagnostic_draw_millis: None,
        diagnostic_color_mode: None,
        diagnostic_stage_seed: None,
        diagnostic_score_fixture: None,
        title_idle: 0,
        demo_frames: None,
        demo_replay: files
            .cm
            .get("demo00.rpy")
            .and_then(|b| th06_formats::replay::Replay::parse(b).ok()),
        audio,
        audio_stage: 0,
        pending_music_room_back: false,
        assets,
        current_stage: 0,
        stage_camera_facing: [0.0; 3],
        character: Character::ReimuA,
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
    /// Windowed D3D uses the desktop format, independent of colorMode16bit.
    /// The native capture CLI can explicitly override the framebuffer format.
    pub fn effective_color_16bit(&self) -> bool {
        self.diagnostic_color_mode
            .unwrap_or(self.options.color_16bit && !self.options.windowed)
    }

    pub fn set_diagnostic_color_mode(&mut self, value: bool) {
        self.diagnostic_color_mode = Some(value);
        th06_engine::set_color_16bit(value);
    }

    /// Match refcap's declared synthetic Present clock during image capture.
    /// This affects only the FPS label; simulation timers and inputs are separate.
    pub fn set_diagnostic_draw_millis(&mut self, milliseconds: u32) {
        self.diagnostic_draw_millis = Some(milliseconds);
    }

    pub fn configure_storage(&mut self, directory: &str) {
        let join = |file: &str| {
            if directory.is_empty() {
                file.to_string()
            } else {
                format!("{}/{file}", directory.trim_end_matches('/'))
            }
        };
        self.set_progress_path(join("score.dat"));
        self.options_path = join("th06-options.bin");
        self.controller_path = join("th06-controller.bin");
        self.replay_directory = join("replay");
        if let Some(options) =
            save::load_bytes(&self.options_path).and_then(|b| options::Options::parse(&b))
        {
            self.options = options;
        }
        self.starting_lives = self.options.life_count as i32 + 1;
        self.starting_bombs = self.options.bomb_count as i32;
        if !self.options_menu.midi_available {
            self.options.music_mode = self.options.music_mode.min(1);
        }
        th06_engine::set_color_16bit(self.effective_color_16bit());
        #[cfg(not(target_arch = "wasm32"))]
        th06_engine::set_windowed(self.options.windowed);
        if let Some(bytes) = save::load_bytes(&self.controller_path).filter(|b| b.len() == 18) {
            th06_engine::set_controller_mapping(std::array::from_fn(|i| {
                u16::from_le_bytes([bytes[i * 2], bytes[i * 2 + 1]])
            }));
        }
    }
    pub fn set_difficulty(&mut self, difficulty: Difficulty) {
        self.difficulty = difficulty;
    }
    pub fn import_progress(&mut self, bytes: &[u8]) -> Result<(), String> {
        let progress = th06_formats::score::ScoreData::parse(bytes)?;
        self.set_progress(progress);
        self.save_progress();
        self.persistence_error.clone().map_or(Ok(()), Err)
    }
    pub fn import_replay(&mut self, bytes: &[u8], slot: usize) -> Result<(), String> {
        let data = th06_formats::replay::Replay::parse(bytes)?;
        if slot >= replay::SLOTS {
            return Err("Invalid replay slot".into());
        }
        save::store_bytes(&replay::path(&self.replay_directory, slot), &data.encode())?;
        self.replay_slots = replay::load_slots(&self.replay_directory);
        Ok(())
    }
    pub fn start_replay(
        &mut self,
        data: th06_formats::replay::Replay,
        index: usize,
    ) -> Result<(), String> {
        self.start_replay_inner(data, index, false)
    }

    fn start_replay_inner(
        &mut self,
        data: th06_formats::replay::Replay,
        index: usize,
        attract_demo: bool,
    ) -> Result<(), String> {
        self.community_run_open = false;
        if index >= TOTAL_STAGES || data.stages[index].is_none() {
            return Err("Replay has no data for this stage".into());
        }
        self.replay_reference_terminal = None;
        self.pending_stage_transition = false;
        self.character = CHARACTERS[data.character as usize];
        self.difficulty = *Difficulty::ALL
            .get(data.difficulty as usize)
            .ok_or("Invalid replay difficulty")?;
        self.mode = if data.difficulty == 4 {
            RunMode::Extra
        } else {
            RunMode::Story
        };
        self.current_stage = index;
        self.refresh_score_table();
        let mut stage = self.assets.new_stage(
            index,
            self.character,
            self.difficulty,
            false,
            self.starting_lives,
            self.starting_bombs,
        );
        let mut session = replay::Session::playback(data);
        session.begin_stage(index, &mut stage)?;
        stage.set_hiscore(self.hiscore);
        stage.set_attract_demo(attract_demo);
        self.replay_session = Some(session);
        self.demo_frames = attract_demo.then_some(0);
        self.enter_stage(stage);
        Ok(())
    }

    fn enter_stage(&mut self, mut stage: Stage) {
        stage.inherit_camera_facing(self.stage_camera_facing);
        if let Some(seed) = self.diagnostic_stage_seed {
            stage.set_rng_seed(seed);
        }
        // GameManager::AddedCallback requests stage music before the first
        // calculation tick; the title's attract demo keeps the title music.
        self.audio_stage = self.current_stage + 1;
        th06_engine::audio::trace::context(None, self.audio_stage);
        stage.events.retain(|event| {
            if let Event::Bgm(file) = event {
                if self.demo_frames.is_none() {
                    self.play_bgm(file);
                }
                false
            } else {
                true
            }
        });
        self.scene = Scene::Stage(Box::new(stage));
    }

    fn stop_stage_audio(&mut self) {
        // GameManager_DeletedCallback skips this only for the attract demo.
        if self.demo_frames.is_none() {
            th06_engine::audio::trace::context(None, self.audio_stage);
            th06_engine::audio::trace::stop();
            if let Some(audio) = &mut self.audio {
                audio.stop_bgm();
            }
        }
    }

    pub fn replay_reference_state(&self) -> Option<String> {
        let Scene::Stage(stage) = &self.scene else {
            return None;
        };
        let session = self.replay_session.as_ref()?;
        Some(format!("{},{}", session.frame(), stage.reference_state()))
    }

    /// Read-only live and paused-stage observations for the native capture CLI.
    pub fn diagnostic_stage_state(&self) -> Option<String> {
        let stage = match &self.scene {
            Scene::Stage(stage) | Scene::Continue { stage, .. } => stage,
            _ => return None,
        };
        Some(format!(
            "{},{}",
            self.active_frames,
            stage.reference_state()
        ))
    }

    pub fn diagnostic_transition(&self) -> String {
        let frame = self
            .replay_session
            .as_ref()
            .map_or(self.active_frames as i32, replay::Session::frame);
        if self.pending_practice_exit >= 2
            || self.pending_result.is_some()
            || self.pending_result_back
        {
            let scene = if self.pending_practice_exit == 3 || self.pending_result_back {
                1
            } else {
                7
            };
            return format!("{scene},-1,{},0,0,0,0,0,0", self.audio_stage);
        }
        match &self.scene {
            Scene::Stage(stage) | Scene::Continue { stage, .. } => {
                let (paused, pause_state, pause_timer) = stage.diagnostic_pause();
                let retry =
                    matches!(self.scene, Scene::Continue { .. }) || self.pending_practice_exit == 1;
                let (retry_state, retry_timer) = self.retry_menu.diagnostic_state();
                let game_menu = if paused {
                    if pause_state == 0 { 1 } else { 2 }
                } else {
                    0
                };
                let frame = if paused || (retry && retry_state != 0) {
                    -1
                } else {
                    frame
                };
                let scene = if self.pending_stage_back { 1 } else { 2 };
                let frame = if self.pending_stage_back || frame == 0 {
                    -1
                } else {
                    frame
                };
                format!(
                    "{scene},{frame},{},{pause_state},{pause_timer},{retry_state},{retry_timer},{game_menu},{}",
                    self.current_stage + 1,
                    u8::from(retry)
                )
            }
            _ => format!(
                "{},-1,{},0,0,0,0,0,0",
                if self.pending_replay_start.is_some() || self.pending_live_start.is_some() {
                    2
                } else if matches!(self.scene, Scene::Result) {
                    if self.result_stats.is_some() { 7 } else { 6 }
                } else {
                    1
                },
                self.audio_stage
            ),
        }
    }

    pub fn diagnostic_navigation(&self) -> Option<String> {
        let (state, cursor, timer) = match &self.scene {
            Scene::Title => self.title.reference_navigation(),
            Scene::Options => self.options_menu.reference_navigation(),
            Scene::Selection => self.selection.reference_navigation(),
            Scene::Replay => self.replay_menu.reference_navigation(),
            _ => return None,
        };
        Some(format!("{state},{cursor},{timer}"))
    }

    pub fn diagnostic_boss_ui(&self) -> Option<String> {
        let stage = match &self.scene {
            Scene::Stage(stage) | Scene::Continue { stage, .. } => stage,
            _ => return None,
        };
        let frame = self
            .replay_session
            .as_ref()
            .map_or(self.active_frames as i32, |s| s.frame());
        Some(format!("{frame},{}", stage.reference_boss_ui()))
    }

    pub fn diagnostic_results(&self) -> Option<String> {
        // Leaving requests Title and removes Result's callback immediately;
        // retain its scene only for the already drawn final presentation.
        if !matches!(self.scene, Scene::Result) || self.pending_result_back {
            return None;
        }
        let (state, timer, cursor, cheat) = self.result.reference_state();
        let mut progress = String::new();
        for character in 0..4 {
            for values in [
                &self.progress.stages_cleared_without_continues[character],
                &self.progress.stages_cleared[character],
            ] {
                for value in values {
                    use std::fmt::Write;
                    write!(progress, "{value:02x}").unwrap();
                }
            }
        }
        Some(format!("{state},{timer},{cursor},{cheat},{progress}"))
    }

    pub fn diagnostic_lifecycle(&self) -> Option<String> {
        let stage = match &self.scene {
            Scene::Stage(stage) | Scene::Continue { stage, .. } => stage,
            _ => return None,
        };
        let frame = self
            .replay_session
            .as_ref()
            .map_or(self.active_frames as i32, |s| s.frame());
        Some(format!(
            "{frame},{},{}",
            self.current_stage + 1,
            stage.reference_lifecycle()
        ))
    }

    pub fn replay_reference_bullets(&self) -> Option<String> {
        let Scene::Stage(stage) = &self.scene else {
            return None;
        };
        Some(stage.reference_bullets(self.replay_session.as_ref()?.frame()))
    }
    pub fn replay_reference_effects(&self) -> Option<String> {
        let Scene::Stage(stage) = &self.scene else {
            return None;
        };
        Some(stage.reference_effects(self.replay_session.as_ref()?.frame()))
    }

    pub fn replay_reference_pools(&self) -> Option<String> {
        let Scene::Stage(stage) = &self.scene else {
            return None;
        };
        Some(stage.reference_pools(self.replay_session.as_ref()?.frame()))
    }

    /// A stage-ending update replaces Scene::Stage before a CLI can inspect
    /// it. Retain that exact final observation and its semantic event once.
    pub fn take_replay_reference_terminal(&mut self) -> Option<ReplayReferenceTerminal> {
        self.replay_reference_terminal.take()
    }
    pub fn extra_unlocked(&self) -> [bool; 4] {
        std::array::from_fn(|c| {
            (1..=3).any(|d| self.progress.stages_cleared_without_continues[c][d] == 99)
        })
    }
    fn practice_stages(&self, character: usize, difficulty: usize) -> usize {
        (self.progress.stages_cleared[character][difficulty] as usize)
            .min(if difficulty == 0 { 5 } else { 6 })
            .max(1)
    }
    pub fn set_progress(&mut self, progress: th06_formats::score::ScoreData) {
        self.progress = progress;
        self.refresh_score_table();
    }
    pub fn set_progress_path(&mut self, path: String) {
        self.progress_path = path;
        self.set_progress(save::load_progress(&self.progress_path));
    }
    pub fn save_progress(&mut self) {
        self.persistence_error = save::store_progress(&self.progress, &self.progress_path).err();
    }
    fn refresh_score_table(&mut self) {
        let character = CHARACTERS
            .iter()
            .position(|&v| v == self.character)
            .unwrap_or(0) as u8;
        self.scores = self
            .progress
            .high_scores
            .iter()
            .filter(|s| s.character == character && s.difficulty == self.difficulty as u8)
            .map(|s| ScoreEntry {
                name: s.name.clone(),
                score: s.score as i64,
                stage: s.stage,
            })
            .collect();
        self.scores.sort_by_key(|s| std::cmp::Reverse(s.score));
        self.scores.truncate(MAX_SCORES);
        self.hiscore = if self.mode == RunMode::Practice {
            self.progress.practice_scores[character as usize][self.current_stage]
                [self.difficulty as usize] as i64
        } else {
            self.scores
                .first()
                .map(|s| s.score)
                .unwrap_or(0)
                .max(1_000_000)
        };
    }
    fn run_stats(&self, carry: &stage::Carry, completed: bool) -> result::RunStats {
        result::RunStats {
            character: CHARACTERS
                .iter()
                .position(|&c| c == self.character)
                .unwrap_or(0) as u8,
            difficulty: self.difficulty as u8,
            score: carry.score.clamp(0, u32::MAX as i64) as u32,
            stage: self.current_stage as u8 + 1,
            completed,
            continues: carry.continues_used,
            deaths: carry.deaths,
            bombs_used: carry.bombs_used,
            spellcards_captured: carry.spellcards_captured,
            point_items: carry.point_items_total.max(0) as u32,
            graze: carry.graze.max(0) as u32,
            frames: self.active_frames,
            slowdown: 0.0,
            date: replay::date(),
        }
    }
    fn prepare_result(&mut self, carry: &stage::Carry, completed: bool) {
        let stats = self.run_stats(carry, completed);
        if completed {
            self.queue_community_result(&stats);
        }
        self.pending_stats = Some(stats);
    }
    fn queue_community_result(&mut self, stats: &result::RunStats) {
        if self.community_run_open {
            self.community_run_open = false;
            if self.diagnostic_stage_seed.is_none()
                && self.diagnostic_draw_millis.is_none()
                && std::env::var_os("TH06_GOD").is_none()
                && let Some(payload) = community::result_payload(stats, self.mode)
            {
                self.community_results.push(payload);
            }
        }
    }
    pub fn take_community_results(&mut self) -> Vec<serde_json::Value> {
        std::mem::take(&mut self.community_results)
    }
    fn finish_run(&mut self, score: i64) {
        let stats = self.pending_stats.take().unwrap_or(result::RunStats {
            score: score.clamp(0, u32::MAX as i64) as u32,
            stage: self.current_stage as u8 + 1,
            difficulty: self.difficulty as u8,
            ..Default::default()
        });
        let available = self
            .replay_session
            .as_ref()
            .is_some_and(|session| !session.is_playback());
        self.replay_slots = replay::load_slots(&self.replay_directory);
        self.result_stats = Some(stats.clone());
        self.queue_community_result(&stats);
        self.result.begin_run(stats, &self.progress, available);
        self.scene = Scene::Result;
    }
    pub fn set_hiscore(&mut self, v: i64) {
        self.hiscore = v;
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn set_hiscore_path(&mut self, path: std::path::PathBuf) {
        self.hiscore_path = path;
    }

    /// Seed the high-score table (kept sorted descending, capped).
    pub fn set_scores(&mut self, mut scores: Vec<ScoreEntry>) {
        scores.sort_by_key(|s| std::cmp::Reverse(s.score));
        scores.truncate(MAX_SCORES);
        self.scores = scores;
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn set_scores_path(&mut self, path: std::path::PathBuf) {
        self.scores_path = path;
    }

    /// Insert a finished run and return its row index in the sorted table.
    fn insert_score(&mut self, name: String, score: i64, stage: u8) -> usize {
        let character = CHARACTERS
            .iter()
            .position(|&v| v == self.character)
            .unwrap_or(0) as u8;
        self.progress
            .high_scores
            .push(th06_formats::score::HighScore {
                name: name.clone(),
                score: score.clamp(0, u32::MAX as i64) as u32,
                stage,
                character,
                difficulty: self.difficulty as u8,
            });
        self.save_progress();
        self.scores.push(ScoreEntry { name, score, stage });
        self.scores.sort_by_key(|s| std::cmp::Reverse(s.score));
        self.scores.truncate(MAX_SCORES);
        self.scores
            .iter()
            .position(|e| e.score == score)
            .unwrap_or(0)
    }

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
        self.current_stage = stage.min(TOTAL_STAGES - 1);
        self.difficulty = if self.current_stage == 6 {
            Difficulty::Extra
        } else {
            self.difficulty
        };
        self.mode = if self.current_stage == 6 {
            RunMode::Extra
        } else {
            RunMode::Story
        };
        let mut s = self.assets.new_stage(
            self.current_stage,
            character,
            self.difficulty,
            false,
            self.starting_lives,
            self.starting_bombs,
        );
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
        self.enter_stage(s);
    }

    /// Shared seed protocol for the independent C++/Rust ECL oracle.
    pub fn debug_set_stage_seed(&mut self, seed: u16) {
        self.diagnostic_stage_seed = Some(seed);
        if let Scene::Stage(stage) = &mut self.scene {
            stage.set_rng_seed(seed);
        }
    }

    /// Explicit capture fixture, applied after drawing and before calculation.
    pub fn debug_set_score_fixture(&mut self, present: u64, score: i64) {
        self.diagnostic_score_fixture = Some((present, score));
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
            let pressed: &[Key] = if _i % 12 == 0 { &[Key::Shoot] } else { &[] };
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
            Scene::Title | Scene::Selection | Scene::MusicRoom | Scene::Options | Scene::Replay => {
                RecordPhase::Menu
            }
            Scene::Stage(_) | Scene::Continue { .. } => RecordPhase::Stage(self.current_stage),
            Scene::Result | Scene::Ending { .. } => RecordPhase::Ended,
        }
    }

    /// Read-only name for deterministic screenshot receipts.
    pub fn diagnostic_scene(&self) -> &'static str {
        match self.scene {
            Scene::Title => "title",
            Scene::Selection => "selection",
            Scene::Stage(_) => "stage",
            Scene::Continue { .. } => "continue",
            Scene::Ending { .. } => "ending",
            Scene::Result => "result",
            Scene::MusicRoom => "music-room",
            Scene::Replay => "replay",
            Scene::Options => "options",
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

    /// Player centre + remaining bombs while in a stage — the dodging auto-player.
    pub fn stage_player(&self) -> Option<([f32; 2], i32)> {
        if let Scene::Stage(s) = &self.scene {
            Some((s.player_pos(), s.player_bombs()))
        } else {
            None
        }
    }

    /// Live item positions [x,y] of the current stage.
    pub fn stage_items(&self) -> Vec<[f32; 2]> {
        if let Scene::Stage(s) = &self.scene {
            s.items_dump()
        } else {
            Vec::new()
        }
    }

    /// Prepare isolated browser audio before the initial playback request.
    #[cfg(target_arch = "wasm32")]
    pub async fn prepare_audio(&self) {
        if let Some(audio) = &self.audio {
            audio.prepare().await;
        }
    }

    /// Start the title BGM (call once the audio context is unlocked).
    pub fn start_title_bgm(&mut self) {
        self.play_bgm("th06_01.wav");
    }

    fn run_title_cursor(&self) -> usize {
        if self.mode == RunMode::Practice {
            2
        } else {
            usize::from(self.difficulty == Difficulty::Extra)
        }
    }

    fn play_bgm(&mut self, file: &str) {
        th06_engine::audio::trace::bgm(file, self.options.music_mode != 0);
        if self.options.music_mode == 0 {
            return;
        }
        if let Some(audio) = &mut self.audio {
            if self.options.music_mode == 2 {
                audio.play_bgm(&file.replace(".wav", ".mid"));
            } else {
                audio.play_bgm(file);
            }
        }
    }

    pub fn update(&mut self, input: &Input) -> Frame {
        localization::set_language(self.options.language);
        self.draw_serial = self.draw_serial.wrapping_add(1);
        if let Some((milliseconds, presentations)) = self
            .diagnostic_draw_millis
            .map(|milliseconds| (milliseconds, self.draw_serial))
            .or_else(th06_engine::presentation_clock)
        {
            self.fps_counter.observe(milliseconds, presentations);
        }
        // GameWindow::Render draws first, runs the calculation chain, then
        // presents that earlier image. Replay dialogue may restart only the
        // calculation chain, so draw exactly once before all three ticks.
        if let Scene::Stage(stage) | Scene::Continue { stage, .. } = &mut self.scene
            && let Some(sound) = stage.take_draw_sound()
        {
            th06_engine::audio::trace::sfx(sound);
            if let Some(audio) = &self.audio {
                audio.play_sfx_idx(sound);
            }
        }
        let mut output = self.draw_scene_before_calc();
        if let Some((present, score)) = self.diagnostic_score_fixture
            && self.draw_serial == present + 1
        {
            if let Scene::Stage(stage) = &mut self.scene {
                stage.set_score(score);
            }
            self.diagnostic_score_fixture = None;
        }
        output.quit |= self.update_render_frame(input).quit;
        for _ in 0..2 {
            let accelerate = matches!(&self.scene, Scene::Stage(stage) if stage.replay_accelerates_dialogue())
                && self
                    .replay_session
                    .as_ref()
                    .is_some_and(|session| session.is_playback() && session.frame() % 3 != 2);
            if !accelerate {
                break;
            }
            output.quit |= self.update_render_frame(input).quit;
        }
        th06_engine::audio::trace::context(
            self.replay_session.as_ref().map(replay::Session::frame),
            self.audio_stage,
        );
        th06_engine::audio::trace::flush(self.options.play_sounds);
        if self.options.play_sounds
            && let Some(audio) = &self.audio
        {
            audio.flush_sfx();
        }
        localization::refresh(&output.cmds);
        output
    }

    fn draw_scene_before_calc(&mut self) -> Frame {
        if self.pending_practice_exit == 3 || self.pending_result_back {
            // Result's deleted callback has removed the old draw chain;
            // MainMenu is registered in the following calculation.
            return Frame {
                cmds: Vec::new(),
                bg: None,
                quit: false,
            };
        }
        let (cmds, bg) = if matches!(self.scene, Scene::Selection) {
            let character = self.selection.character * 2 + self.selection.shot;
            let difficulty = self.selection.difficulty.min(3);
            let count = self.practice_stages(character, difficulty);
            let scores: Vec<i64> = self.progress.practice_scores[character]
                .iter()
                .map(|score| score[difficulty] as i64)
                .collect();
            (self.selection.draw(&scores, count), None)
        } else {
            match &mut self.scene {
                Scene::Title => {
                    self.title
                        .set_extra_unlocked(self.extra_unlocked().iter().any(|&value| value));
                    (self.title.draw(), None)
                }
                Scene::Stage(stage) => {
                    stage.set_hitbox_mode(self.options.hitbox);
                    (stage.draw(), stage.background_scene())
                }
                Scene::Continue { frozen, stage, .. } => {
                    let mut cmds = frozen.clone();
                    cmds.extend(self.retry_menu.draw());
                    (cmds, stage.background_scene())
                }
                Scene::Ending { player, .. } => (player.draw(), None),
                Scene::Result => (self.result.draw(), None),
                Scene::MusicRoom => (self.music_room.draw(), None),
                Scene::Replay => (self.replay_menu.draw(&self.replay_slots), None),
                Scene::Options => (self.options_menu.draw(&self.options), None),
                Scene::Selection => unreachable!(),
            }
        };
        let mut frame = Frame {
            cmds,
            bg,
            quit: false,
        };
        if !matches!(self.scene, Scene::Ending { .. }) {
            self.fps_counter.draw(&mut frame.cmds);
        }
        if matches!(self.scene, Scene::Title | Scene::Options)
            && let Some(background) = frame.cmds.first_mut()
        {
            // MainMenu_OnDraw copies title00.jpg, then draws the active
            // menu's black alpha64 square before any of its ANM sprites.
            background.tint[0] *= 191.0 / 255.0;
            background.tint[1] *= 191.0 / 255.0;
            background.tint[2] *= 191.0 / 255.0;
        }
        if let Some(elapsed) = self.demo_frames.filter(|&elapsed| elapsed >= 3600) {
            frame.cmds.push(DrawCmd {
                clip: None,
                tex: stage::TEX_WHITE,
                dst: [0.0, 0.0, 640.0, 480.0],
                src: [0.0, 0.0, 1.0, 1.0],
                tint: [0.0, 0.0, 0.0, (elapsed - 3600) as f32 / 120.0],
                rot: 0.0,
                additive: false,
                color_add: false,
                depth: None,
            });
        }
        // Original SW texture blending leaves D3DTOP_MODULATE active even
        // when a VM requests ADD (AnmManager::Draw, line 462). Alpha blending
        // and additive destination blending remain independent and unchanged.
        if !self.options.hardware_texture_blending {
            for command in &mut frame.cmds {
                command.color_add = false;
            }
        }
        frame
    }

    fn update_render_frame(&mut self, input: &Input) -> Frame {
        th06_engine::audio::trace::context(
            self.replay_session.as_ref().map(replay::Session::frame),
            self.audio_stage,
        );
        if let Some(frame) = &mut self.demo_frames {
            *frame += 1;
            if input.buttons() != 0 || *frame >= 3720 {
                self.demo_frames = None;
                self.title_idle = 0;
                self.replay_session = None;
                self.scene = Scene::Title;
                self.title.reset(0);
            }
        }
        let applying_stage_transition = self.pending_stage_transition;
        let neutral = Input::default();
        let mut output = self.update_inner(if applying_stage_transition {
            &neutral
        } else {
            input
        });
        if applying_stage_transition {
            // Supervisor creates the new chain, clears the input globals, then
            // the newly registered callbacks calculate within this same tick.
            output.quit |= self.update_inner(&neutral).quit;
        }
        let sounds = [
            self.title.take_sounds(),
            self.selection.take_sounds(),
            self.options_menu.take_sounds(),
            self.music_room.take_sounds(),
            self.retry_menu.take_sounds(),
            self.result.take_sounds(),
            self.replay_menu.take_sounds(),
        ];
        for sound in sounds.into_iter().flatten() {
            th06_engine::audio::trace::sfx_named(sound);
            if let Some(audio) = &self.audio {
                audio.play_sfx(sound);
            }
        }
        output
    }
    fn update_inner(&mut self, input: &Input) -> Frame {
        if std::mem::take(&mut self.pending_result_back) {
            self.replay_session = None;
            self.scene = Scene::Title;
            self.title.reset(if self.result_stats.is_some() {
                self.run_title_cursor()
            } else {
                4
            });
            self.play_bgm("th06_01.wav");
            return self.update_inner(&Input::default());
        }
        if let Some(stage) = self.pending_live_start.take() {
            self.enter_stage(*stage);
            return self.update_inner(&Input::default());
        }
        if let Some((data, stage)) = self.pending_replay_start.take() {
            match self.start_replay(data, stage) {
                Ok(()) => return self.update_inner(&Input::default()),
                Err(error) => self.persistence_error = Some(error),
            }
        }
        if std::mem::take(&mut self.pending_stage_back) {
            self.stop_stage_audio();
            self.replay_session = None;
            self.scene = Scene::Title;
            self.title.reset(self.run_title_cursor());
            self.play_bgm("th06_01.wav");
            return self.update_inner(&Input::default());
        }
        if self.pending_practice_exit != 0 {
            match self.pending_practice_exit {
                1 => {
                    self.pending_practice_exit = 0;
                    if let Scene::Stage(stage) = &mut self.scene {
                        stage.resume_after_game_over();
                    }
                    // AsciiManager releases Retry and requests Result, then
                    // the current stage chain finishes this calculation.
                    let frame = self.update_inner(input);
                    self.pending_practice_exit = 2;
                    return frame;
                }
                2 => {
                    self.stop_stage_audio();
                    self.replay_session = None;
                    self.pending_practice_exit = 3;
                }
                _ => {
                    self.pending_practice_exit = 0;
                    self.scene = Scene::Title;
                    self.title.reset(2);
                    self.play_bgm("th06_01.wav");
                    return self.update_inner(&Input::default());
                }
            }
            return Frame {
                cmds: Vec::new(),
                bg: None,
                quit: false,
            };
        }
        if let Some(score) = self.pending_result.take() {
            self.stop_stage_audio();
            self.finish_run(score);
            return self.update_inner(&Input::default());
        }
        if !self.pending_stage_transition
            && let Some(audio) = &mut self.audio
        {
            audio.tick();
            if input.buttons() != 0 {
                audio.resume();
            }
        }
        if std::mem::take(&mut self.pending_music_room_back) {
            // Supervisor observes MusicRoom's state change on the next calc.
            self.scene = Scene::Title;
            self.title.reset(5);
            self.play_bgm("th06_01.wav");
        }
        if matches!(self.scene, Scene::Replay) {
            match self.replay_menu.update(input, &self.replay_slots) {
                replay_menu::ReplayAction::Back => {
                    self.scene = Scene::Title;
                    self.title.begin_startup(0);
                    self.title.input = self.replay_menu.input;
                    let (cmds, _) = self.title.update(input);
                    return Frame {
                        cmds,
                        bg: None,
                        quit: false,
                    };
                }
                replay_menu::ReplayAction::Play { slot, stage } => {
                    if let Some(data) = self.replay_slots[slot].clone() {
                        // MainMenu requests Supervisor's gameplay state.
                        // Registration follows the next pre-calc menu draw.
                        self.pending_replay_start = Some((data, stage - 1));
                    }
                }
                replay_menu::ReplayAction::None => {}
            }
            return Frame {
                cmds: self.replay_menu.draw(&self.replay_slots),
                bg: None,
                quit: false,
            };
        }
        if let Scene::Ending { player, score } = &mut self.scene {
            let score = *score;
            let events = player.update(input);
            // Ending drawing (including its fade timer) already ran before
            // this calculation, once per presentation, in draw_scene_before_calc.
            let cmds = Vec::new();
            let done = player.finished();
            match events {
                Ok(events) => {
                    for event in events {
                        match event {
                            ending::EndingEvent::Music(name) => self.play_bgm(&format!(
                                "{}.wav",
                                basename(&name)
                                    .trim_end_matches(".mid")
                                    .trim_end_matches(".wav")
                            )),
                            ending::EndingEvent::FadeMusic(seconds) => {
                                th06_engine::audio::trace::fade(seconds);
                                if let Some(audio) = &mut self.audio {
                                    audio.fade_bgm(seconds);
                                }
                            }
                        }
                    }
                }
                Err(error) => {
                    self.persistence_error = Some(error.to_string());
                    self.finish_run(score);
                }
            }
            if done {
                self.finish_run(score);
            }
            return Frame {
                cmds,
                bg: None,
                quit: false,
            };
        }
        if matches!(self.scene, Scene::Continue { .. }) {
            if let Scene::Continue { stage, .. } = &mut self.scene {
                stage.prepare_draw_camera();
            }
            let action = self.retry_menu.update(input);
            if !matches!(action, retry_menu::RetryAction::None) {
                let Scene::Continue {
                    mut stage, score, ..
                } = std::mem::replace(&mut self.scene, Scene::Title)
                else {
                    unreachable!()
                };
                match action {
                    retry_menu::RetryAction::Continue => {
                        stage.continue_run();
                        self.scene = Scene::Stage(stage);
                        // AsciiManager clears Retry before GameManager and
                        // Player callbacks, which resume in this same calc.
                        return self.update_inner(input);
                    }
                    retry_menu::RetryAction::Finish => {
                        stage.resume_after_game_over();
                        self.scene = Scene::Stage(stage);
                        let frame = self.update_inner(input);
                        if let Some(session) = &mut self.replay_session {
                            session.end_stage(self.current_stage, score);
                        }
                        self.pending_result = Some(score);
                        return frame;
                    }
                    retry_menu::RetryAction::None => unreachable!(),
                }
            }
            let Scene::Continue { frozen, stage, .. } = &self.scene else {
                unreachable!()
            };
            let mut cmds = frozen.clone();
            cmds.extend(self.retry_menu.draw());
            return Frame {
                cmds,
                bg: stage.background_scene(),
                quit: false,
            };
        }
        if matches!(self.scene, Scene::Options) {
            let previous_options = self.options.clone();
            let action = self.options_menu.update(input, &mut self.options);
            let final_music_mode = self.options.music_mode;
            for event in self.options_menu.take_audio_events() {
                match event {
                    options::OptionsAudio::Sound(sound) => {
                        th06_engine::audio::trace::sfx_named(sound);
                        if let Some(audio) = &self.audio {
                            audio.play_sfx(sound);
                        }
                    }
                    options::OptionsAudio::Music(mode) => {
                        th06_engine::audio::trace::stop();
                        if let Some(audio) = &mut self.audio {
                            audio.stop_bgm();
                        }
                        self.options.music_mode = mode;
                        self.play_bgm("th06_01.wav");
                    }
                }
            }
            self.options.music_mode = final_music_mode;
            if previous_options != self.options {
                th06_engine::set_color_16bit(self.effective_color_16bit());
                if previous_options.windowed != self.options.windowed {
                    th06_engine::set_windowed(self.options.windowed);
                }
                self.starting_lives = self.options.life_count as i32 + 1;
                self.starting_bombs = self.options.bomb_count as i32;
                if let Err(error) = save::store_bytes(&self.options_path, &self.options.encode()) {
                    self.persistence_error = Some(error);
                }
            }
            match action {
                options::OptionsAction::Back => {
                    self.scene = Scene::Title;
                    self.title.resume_menu(6);
                    self.title.input = self.options_menu.input;
                }
                options::OptionsAction::Changed | options::OptionsAction::DefaultsRestored => {}
                options::OptionsAction::ControllerChanged => {
                    let bytes: Vec<u8> = th06_engine::controller_mapping()
                        .iter()
                        .flat_map(|v| v.to_le_bytes())
                        .collect();
                    if let Err(error) = save::store_bytes(&self.controller_path, &bytes) {
                        self.persistence_error = Some(error);
                    }
                }
                options::OptionsAction::None => {}
            }
            return Frame {
                cmds: self.options_menu.draw(&self.options),
                bg: None,
                quit: false,
            };
        }
        if matches!(self.scene, Scene::MusicRoom) {
            match self.music_room.update(input) {
                music_room::MusicAction::None => {}
                music_room::MusicAction::Back => {
                    self.pending_music_room_back = true;
                }
                music_room::MusicAction::Play(path) => {
                    let path = basename(&path).replace(".mid", ".wav");
                    self.play_bgm(&path);
                }
                music_room::MusicAction::PlayAndBack(path) => {
                    let path = basename(&path).replace(".mid", ".wav");
                    self.play_bgm(&path);
                    self.pending_music_room_back = true;
                }
            }
            return Frame {
                cmds: self.music_room.draw(),
                bg: None,
                quit: false,
            };
        }
        if matches!(self.scene, Scene::Selection) {
            let extra = self.extra_unlocked();
            let c = self.selection.character * 2 + self.selection.shot;
            let d = self.selection.difficulty.min(3);
            let count = self.practice_stages(c, d);
            let scores: Vec<i64> = self.progress.practice_scores[c]
                .iter()
                .map(|s| s[d] as i64)
                .collect();
            let action = self.selection.update(input, extra, count);
            if self.selection.mode != RunMode::Extra
                && matches!(self.selection.step, selection::SelectionStep::TitleUnload)
            {
                self.options.default_difficulty = self.selection.difficulty as u8;
            }
            match action {
                SelectionAction::Back { cursor } => {
                    if self.selection.mode != RunMode::Extra {
                        self.options.default_difficulty = self.selection.difficulty as u8;
                    }
                    self.scene = Scene::Title;
                    self.title.begin_startup(cursor);
                    // Selection and Title are states of the same original
                    // MainMenu chain: Supervisor's held-input counter survives.
                    self.title.input = self.selection.input;
                    let _ = self.title.update(input);
                }
                SelectionAction::Start {
                    character,
                    difficulty,
                    stage,
                    mode,
                } => {
                    self.community_run_open = true;
                    self.active_frames = 0;
                    self.character = character;
                    self.difficulty = difficulty;
                    if difficulty != Difficulty::Extra {
                        self.options.default_difficulty = difficulty as u8;
                    }
                    self.mode = mode;
                    self.current_stage = stage;
                    self.refresh_score_table();
                    let mut s = self.assets.new_stage(
                        stage,
                        character,
                        difficulty,
                        mode == RunMode::Practice,
                        self.starting_lives,
                        self.starting_bombs,
                    );
                    s.set_hiscore(self.hiscore);
                    s.set_spellcard_records(&self.progress.spellcards);
                    s.set_rng_seed(self.diagnostic_stage_seed.unwrap_or_else(replay::seed));
                    self.replay_session = if mode == RunMode::Practice {
                        None
                    } else {
                        Some(replay::Session::recording(
                            c as u8,
                            difficulty as u8,
                            replay::date(),
                        ))
                    };
                    if let Some(session) = &mut self.replay_session {
                        let _ = session.begin_stage(stage, &mut s);
                    }
                    if let Err(error) =
                        save::store_bytes(&self.options_path, &self.options.encode())
                    {
                        self.persistence_error = Some(error);
                    }
                    self.pending_live_start = Some(Box::new(s));
                    return Frame {
                        cmds: Vec::new(),
                        bg: None,
                        quit: false,
                    };
                }
                SelectionAction::None => {}
            }
            return Frame {
                cmds: self.selection.draw(&scores, count),
                bg: None,
                quit: false,
            };
        }
        if matches!(self.scene, Scene::Result) {
            match self.result.update(input, &self.replay_slots) {
                result::ResultAction::None => {}
                result::ResultAction::UnlockAll => {
                    self.progress.stages_cleared = [[99; 5]; 4];
                    self.progress.stages_cleared_without_continues = [[99; 5]; 4];
                    self.save_progress();
                }
                result::ResultAction::Back => {
                    self.pending_result_back = true;
                }
                result::ResultAction::SaveScore { name } => {
                    let stats = self
                        .result_stats
                        .as_ref()
                        .expect("result run stats")
                        .clone();
                    self.insert_score(
                        name,
                        stats.score as i64,
                        if stats.completed { 99 } else { stats.stage },
                    );
                }
                result::ResultAction::SaveReplay { slot, name } => {
                    if let Some(session) = &mut self.replay_session
                        && let Err(error) = session.save(&self.replay_directory, slot, &name)
                    {
                        self.persistence_error = Some(error);
                    }
                    self.replay_slots = replay::load_slots(&self.replay_directory);
                }
            }
            return Frame {
                cmds: self.result.draw(),
                bg: None,
                quit: false,
            };
        }

        match &mut self.scene {
            Scene::Selection
            | Scene::MusicRoom
            | Scene::Options
            | Scene::Replay
            | Scene::Ending { .. }
            | Scene::Continue { .. } => unreachable!("handled above"),
            Scene::Result => unreachable!("handled above"),
            Scene::Title => {
                if input.buttons() == 0 {
                    self.title_idle = self.title_idle.saturating_add(1);
                } else {
                    self.title_idle = 0;
                }
                if self.title_idle >= 720 {
                    self.title_idle = 0;
                    if let Some(replay) = self.demo_replay.clone()
                        && self.start_replay_inner(replay, 3, true).is_ok()
                    {
                        return Frame {
                            cmds: Vec::new(),
                            bg: None,
                            quit: false,
                        };
                    }
                }
                self.title
                    .set_extra_unlocked(self.extra_unlocked().iter().any(|&v| v));
                let (cmds, action) = self.title.update(input);
                match action {
                    TitleAction::StartGame => {
                        self.selection
                            .begin(RunMode::Story, self.options.default_difficulty as usize);
                        self.selection.input = self.title.input;
                        self.scene = Scene::Selection;
                    }
                    TitleAction::Extra | TitleAction::Practice => {
                        let mode = if action == TitleAction::Extra {
                            RunMode::Extra
                        } else {
                            RunMode::Practice
                        };
                        self.selection
                            .begin(mode, self.options.default_difficulty as usize);
                        self.selection.input = self.title.input;
                        self.scene = Scene::Selection;
                    }
                    TitleAction::Score => {
                        self.result_stats = None;
                        self.result.begin_browse(&self.progress);
                        self.scene = Scene::Result;
                    }
                    TitleAction::Music => {
                        self.music_room.begin();
                        self.scene = Scene::MusicRoom;
                    }
                    TitleAction::Options => {
                        self.options_menu.begin();
                        self.options_menu.input = self.title.input;
                        self.scene = Scene::Options;
                    }
                    TitleAction::Replay => {
                        self.replay_slots = replay::load_slots(&self.replay_directory);
                        self.replay_menu.begin();
                        self.replay_menu.input = self.title.input;
                        self.scene = Scene::Replay;
                    }
                    TitleAction::Quit => {
                        return Frame {
                            cmds,
                            bg: None,
                            quit: true,
                        };
                    }
                    TitleAction::None => {}
                }
                Frame {
                    cmds,
                    bg: None,
                    quit: false,
                }
            }
            Scene::Stage(stage) => {
                let applying_transition = std::mem::take(&mut self.pending_stage_transition);
                let playback = self
                    .replay_session
                    .as_ref()
                    .is_some_and(replay::Session::is_playback);
                let (cmds, events) = if applying_transition {
                    (stage.draw(), vec![Event::NextStage])
                } else {
                    if stage.replay_frame_active(input) {
                        self.active_frames = self.active_frames.saturating_add(1);
                    }
                    let replay_input = self
                        .replay_session
                        .as_mut()
                        .map(|session| session.input(self.current_stage, stage, input));
                    th06_engine::audio::trace::context(
                        self.replay_session.as_ref().map(replay::Session::frame),
                        self.audio_stage,
                    );
                    let cmds = stage.update(replay_input.as_ref().unwrap_or(input));
                    (cmds, stage.events.drain(..).collect::<Vec<Event>>())
                };
                let bg = stage.background_scene();
                if let Some(facing) = stage.camera_facing() {
                    self.stage_camera_facing = facing;
                }
                // Snapshot now, before the event loop reborrows self.
                let carry = stage.carry();
                let can_continue = stage.can_continue();
                let spellcards = stage.spellcard_records().to_vec();
                if playback
                    && !applying_transition
                    && let Some(event) = events.iter().find_map(|event| match event {
                        Event::NextStage => Some("msg-stage-end"),
                        Event::GameOver(_) => Some("game-over"),
                        _ => None,
                    })
                {
                    let frame = self.replay_session.as_ref().unwrap().frame();
                    self.replay_reference_terminal = Some(ReplayReferenceTerminal {
                        event,
                        frame,
                        stage: self.current_stage + 1,
                        state: format!("{frame},{}", stage.reference_state()),
                        pools: stage.reference_pools(frame),
                        lifecycle: stage.reference_lifecycle(),
                    });
                }
                let mut back = false;
                let mut next_stage = false;
                let mut game_over: Option<i64> = None;
                for ev in events {
                    match ev {
                        Event::ScoreExtend => {
                            th06_engine::audio::trace::score_extend();
                            if let Some(audio) = &self.audio {
                                audio.play_sfx_idx(28);
                            }
                        }
                        Event::SfxIdx(index) => {
                            th06_engine::audio::trace::sfx(index);
                            if let Some(audio) = &self.audio {
                                audio.play_sfx_idx(index);
                            }
                        }
                        Event::Sfx(name) => {
                            th06_engine::audio::trace::sfx_named(name);
                            if let Some(a) = &self.audio {
                                a.play_sfx(name);
                            }
                        }
                        Event::Bgm(file) => {
                            if self.demo_frames.is_some() {
                                continue;
                            }
                            let file = file.to_string();
                            self.play_bgm(&file);
                        }
                        Event::BgmFade(seconds) => {
                            if self.demo_frames.is_some() {
                                continue;
                            }
                            th06_engine::audio::trace::fade(seconds);
                            if let Some(a) = &mut self.audio {
                                a.fade_bgm(seconds);
                            }
                        }
                        Event::BackToTitle => back = true,
                        Event::NextStage => next_stage = true,
                        Event::GameOver(score) => game_over = Some(score),
                        Event::Quit => {
                            return Frame {
                                cmds,
                                bg,
                                quit: true,
                            };
                        }
                        Event::SaveScore(score) => {
                            if !playback && score > self.hiscore {
                                self.hiscore = score;
                                #[cfg(not(target_arch = "wasm32"))]
                                {
                                    let _ = std::fs::write(&self.hiscore_path, score.to_string());
                                }
                            }
                        }
                    }
                }
                if !playback {
                    self.progress.spellcards = spellcards;
                }
                if (next_stage && applying_transition)
                    || (game_over.is_some()
                        && (playback || (self.mode != RunMode::Practice && !can_continue)))
                {
                    self.stop_stage_audio();
                }
                if let Some(score) = game_over {
                    if self.mode == RunMode::Practice {
                        self.queue_community_result(&self.run_stats(&carry, false));
                        let character = CHARACTERS
                            .iter()
                            .position(|&v| v == self.character)
                            .unwrap_or(0);
                        let record = &mut self.progress.practice_scores[character]
                            [self.current_stage][self.difficulty as usize];
                        *record = (*record).max(score.clamp(0, u32::MAX as i64) as u32);
                        self.save_progress();
                        self.pending_practice_exit = 1;
                        return Frame {
                            cmds,
                            bg: None,
                            quit: false,
                        };
                    }
                    if (playback || !can_continue)
                        && let Some(session) = &mut self.replay_session
                    {
                        session.end_stage(self.current_stage, score);
                    }
                    if playback {
                        self.demo_frames = None;
                        self.title_idle = 0;
                        self.replay_session = None;
                        self.scene = Scene::Title;
                        self.title.reset(self.run_title_cursor());
                        self.play_bgm("th06_01.wav");
                        return Frame {
                            cmds,
                            bg: None,
                            quit: false,
                        };
                    }
                    self.save_progress();
                    self.prepare_result(&carry, false);
                    if can_continue {
                        let Scene::Stage(stage) = std::mem::replace(&mut self.scene, Scene::Title)
                        else {
                            unreachable!()
                        };
                        self.retry_menu.begin(stage.continues_used());
                        self.scene = Scene::Continue {
                            stage,
                            frozen: cmds.clone(),
                            score,
                        };
                        return Frame {
                            cmds,
                            bg,
                            quit: false,
                        };
                    }
                    self.prepare_result(&carry, false);
                    self.finish_run(score);
                    return Frame {
                        cmds,
                        bg: None,
                        quit: false,
                    };
                }
                if next_stage && !applying_transition {
                    if self.mode == RunMode::Practice {
                        self.queue_community_result(&self.run_stats(&carry, true));
                        // Gui::MSG_OPCODE_STAGEEND requests ResultFromGame
                        // directly; practice has no retry prompt on a clear.
                        let character = CHARACTERS
                            .iter()
                            .position(|&v| v == self.character)
                            .unwrap_or(0);
                        let record = &mut self.progress.practice_scores[character]
                            [self.current_stage][self.difficulty as usize];
                        *record = (*record).max(carry.score.clamp(0, u32::MAX as i64) as u32);
                        self.save_progress();
                        self.pending_practice_exit = 2;
                        return Frame {
                            cmds,
                            bg,
                            quit: false,
                        };
                    }
                    // Gui only changes Supervisor.curState here. The old
                    // scene remains alive for the next pre-calc draw.
                    self.pending_stage_transition = true;
                    return Frame {
                        cmds,
                        bg,
                        quit: false,
                    };
                }
                if next_stage {
                    if let Some(session) = &mut self.replay_session {
                        session.end_stage(self.current_stage, carry.score);
                    }
                    if playback {
                        let next = self.current_stage + 1;
                        if next < TOTAL_STAGES
                            && self.replay_session.as_ref().unwrap().data().stages[next].is_some()
                        {
                            let mut s = self.assets.new_stage(
                                next,
                                self.character,
                                self.difficulty,
                                false,
                                self.starting_lives,
                                self.starting_bombs,
                            );
                            // NEXT_STAGE preserves GameManager's run totals;
                            // ReplayManager then overwrites only its recorded
                            // per-stage fields (stock, seed, rank, power...).
                            s.apply_carry(carry);
                            s.reveal_after_loading();
                            self.replay_session
                                .as_mut()
                                .unwrap()
                                .begin_stage(next, &mut s)
                                .expect("validated replay stage");
                            self.current_stage = next;
                            self.enter_stage(s);
                        } else {
                            self.replay_session = None;
                            self.demo_frames = None;
                            self.title_idle = 0;
                            self.scene = Scene::Title;
                            self.title.reset(self.run_title_cursor());
                            self.play_bgm("th06_01.wav");
                        }
                        return Frame {
                            cmds,
                            bg: None,
                            quit: false,
                        };
                    }
                    let c = CHARACTERS
                        .iter()
                        .position(|&v| v == self.character)
                        .unwrap_or(0);
                    let d = self.difficulty as usize;
                    if self.mode == RunMode::Practice {
                        let record = &mut self.progress.practice_scores[c][self.current_stage][d];
                        *record = (*record).max(carry.score.clamp(0, u32::MAX as i64) as u32);
                    } else {
                        let cleared = (self.current_stage + 2) as u8;
                        self.progress.stages_cleared[c][d] =
                            self.progress.stages_cleared[c][d].max(cleared);
                        if carry.continues_used == 0 {
                            self.progress.stages_cleared_without_continues[c][d] =
                                self.progress.stages_cleared_without_continues[c][d].max(cleared);
                        }
                    }
                    if has_next_stage(self.mode, self.difficulty, self.current_stage) {
                        self.save_progress();
                        // Carry progress into the next stage.
                        self.current_stage += 1;
                        let mut s = self.assets.new_stage(
                            self.current_stage,
                            self.character,
                            self.difficulty,
                            self.mode == RunMode::Practice,
                            self.starting_lives,
                            self.starting_bombs,
                        );
                        s.apply_carry(carry);
                        s.reveal_after_loading();
                        s.set_spellcard_records(&self.progress.spellcards);
                        if let Some(session) = &mut self.replay_session {
                            let _ = session.begin_stage(self.current_stage, &mut s);
                        }
                        s.set_hiscore(self.hiscore);
                        self.enter_stage(s);
                        return Frame {
                            cmds,
                            bg: None,
                            quit: false,
                        };
                    }
                    // Ending::AddedCallback remembers this shot/difficulty
                    // and distinguishes endings reached with a Continue.
                    let has_seen = if carry.continues_used == 0 {
                        self.progress.stages_cleared_without_continues[c][d] == 99
                    } else {
                        self.progress.stages_cleared[c][d] == 99
                    };
                    if self.mode != RunMode::Practice {
                        self.progress.stages_cleared[c][d] = 99;
                        if carry.continues_used == 0 {
                            self.progress.stages_cleared_without_continues[c][d] = 99;
                        }
                    }
                    self.save_progress();
                    if self.mode == RunMode::Practice {
                        self.pending_practice_exit = 1;
                        return Frame {
                            cmds,
                            bg: None,
                            quit: false,
                        };
                    }
                    let score = carry.score;
                    self.prepare_result(&carry, true);
                    if self.mode == RunMode::Story {
                        match self
                            .ending_assets
                            .as_ref()
                            .map_err(ToString::to_string)
                            .and_then(|assets| {
                                assets
                                    .start(
                                        self.character,
                                        self.difficulty,
                                        carry.continues_used,
                                        has_seen,
                                    )
                                    .map_err(|e| e.to_string())
                            }) {
                            Ok(player) => {
                                self.scene = Scene::Ending {
                                    player: Box::new(player),
                                    score,
                                }
                            }
                            Err(error) => {
                                self.persistence_error = Some(error);
                                self.finish_run(score);
                            }
                        }
                    } else {
                        self.finish_run(score);
                    }
                    return Frame {
                        cmds,
                        bg: None,
                        quit: false,
                    };
                }
                if back {
                    if !playback {
                        self.save_progress();
                    }
                    // StageMenu requests MainMenu after Supervisor has run.
                    // Preserve this stage for one final pre-calc draw.
                    self.pending_stage_back = true;
                    return Frame {
                        cmds,
                        bg,
                        quit: false,
                    };
                }
                Frame {
                    cmds,
                    bg,
                    quit: false,
                }
            }
        }
    }
}

/// Gui::MSG_OPCODE_STAGE_END: Easy ends at stage 5; Extra and practice never advance.
fn has_next_stage(mode: RunMode, difficulty: Difficulty, current_stage: usize) -> bool {
    mode == RunMode::Story && current_stage + 1 < if difficulty == Difficulty::Easy { 5 } else { 6 }
}

#[cfg(test)]
mod route_tests {
    use super::*;
    #[test]
    fn story_easy_and_practice_have_original_stage_limits() {
        assert!(has_next_stage(RunMode::Story, Difficulty::Normal, 4));
        assert!(!has_next_stage(RunMode::Story, Difficulty::Normal, 5));
        assert!(!has_next_stage(RunMode::Story, Difficulty::Easy, 4));
        assert!(!has_next_stage(RunMode::Practice, Difficulty::Normal, 0));
        assert!(!has_next_stage(RunMode::Extra, Difficulty::Extra, 6));
    }
}
