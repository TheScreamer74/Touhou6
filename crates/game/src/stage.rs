//! Stage gameplay driven by the original ECL scripts.
//!
//! The timeline spawns enemies exactly as the 2002 engine does; enemy
//! behavior runs in ecl_vm. The player, bomb and HUD live here.

use std::collections::HashMap;
use std::sync::Arc;

use th06_engine::{DrawCmd, Input, Key};
use th06_formats::anm0::{Entry, Instr as AnmInstr, Sprite};
use th06_formats::ecl::Ecl;
use th06_formats::msg::Msg;
use th06_formats::replay::{ReplayInput, ReplayStage};
use th06_formats::score::SpellcardRecord;

use crate::anm_vm::AnmRunner;
use crate::background::Background;
use crate::ecl_vm::{
    BULLET_BASE_SPRITE, Bullet, Enemy, Rng, SpawnReq, World, WorldEvent, normalize_angle,
};
use crate::hud::Hud;
use crate::player_shots::{MARISA_A_RANKS, MARISA_B_RANKS, REIMU_A_RANKS, REIMU_B_RANKS, ShotDef};
use th06_engine::BgScene;

pub const FIELD_W: f32 = 384.0;
pub const FIELD_H: f32 = 448.0;
const FIELD_X: f32 = 32.0;
const FIELD_Y: f32 = 16.0;

/// Original GameManager.hpp difficulty indices, including the Extra script bit.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u8)]
pub enum Difficulty {
    Easy = 0,
    #[default]
    Normal = 1,
    Hard = 2,
    Lunatic = 3,
    Extra = 4,
}

impl Difficulty {
    pub const ALL: [Self; 5] = [
        Self::Easy,
        Self::Normal,
        Self::Hard,
        Self::Lunatic,
        Self::Extra,
    ];

    /// GameManager_AddedCallback's g_DifficultyInfo (EoSD 1.02h).
    pub const fn rank_limits(self) -> (i32, i32, i32) {
        match self {
            Self::Easy => (16, 12, 20),
            Self::Extra => (16, 14, 18),
            _ => (16, 10, 32),
        }
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::Easy => "Easy",
            Self::Normal => "Normal",
            Self::Hard => "Hard",
            Self::Lunatic => "Lunatic",
            Self::Extra => "Extra",
        }
    }

    /// ItemManager::CALCULATE_POINT_SCORE truncates Y before comparing to 128.
    fn point_score(self, y: f32) -> i64 {
        let (top, bottom, multiplier) = match self {
            Self::Easy | Self::Normal => (100_000, 60_000, 100),
            Self::Hard => (150_000, 100_000, 180),
            Self::Lunatic => (200_000, 150_000, 270),
            Self::Extra => (300_000, 200_000, 400),
        };
        let y = y as i64;
        if y < 128 {
            top
        } else {
            bottom - (y - 128) * multiplier
        }
    }
}

const EXTEND_SCORES: [i64; 5] = [
    10_000_000,
    20_000_000,
    40_000_000,
    60_000_000,
    1_900_000_000,
];

/// Frames you can still bomb after a lethal hit (Player.cpp respawnTimer = 6).
const DEATHBOMB_FRAMES: u32 = 6;

/// g_PowerItemScore: score for collecting a power item at max power, indexed by
/// the running power-item count (ItemManager.cpp).
const POWER_ITEM_SCORE: [i64; 31] = [
    10, 20, 30, 40, 50, 60, 70, 80, 90, 100, 200, 300, 400, 500, 600, 700, 800, 900, 1000, 2000,
    3000, 4000, 5000, 6000, 7000, 8000, 9000, 10000, 11000, 12000, 51200,
];

// Texture slots, fixed by main.rs.
pub const TEX_PLAYER: usize = 2;
pub const TEX_BULLET: usize = 3;
pub const TEX_FAIRY: usize = 4;
pub const TEX_RUMIA: usize = 5;
pub const TEX_FRONT: usize = 6;
pub const TEX_WHITE: usize = 7;
pub const TEX_ASCII: usize = 8;
pub const TEX_FACE_REIMU: usize = 9; // face00a (Reimu player portrait)
pub const TEX_FACE_MARISA: usize = 10; // face01a (Marisa player portrait)
// 11 stg1bg, 12 player01.
pub const TEX_POWER_GRAD: usize = 13; // 2x1 power-bar gradient

/// SoundIdx -> g_SoundBufferIdxVol.bufferIdx -> g_SFXList. The original
/// has 32 sound variants over 26 WAV buffers; ECL uses variant indices.
pub const SFX_BY_IDX: [&str; 32] = [
    "plst00", "plst00", "enep00", "enep00", "pldead00", "power0", "power1", "tan00", "tan01",
    "tan02", "ok00", "cancel00", "select00", "gun00", "cat00", "tan00", "lazer00", "lazer01",
    "enep01", "nep00", "damage00", "item00", "tan00", "tan01", "tan02", "kira00", "kira01",
    "kira02", "extend", "timeout", "graze", "powerup",
];

pub enum Event {
    Sfx(&'static str),
    SfxIdx(u8),
    Bgm(&'static str),
    BgmFade(f32),
    BackToTitle,
    /// Stage cleared — advance to the next stage (Game decides the target).
    NextStage,
    Quit,
    SaveScore(i64),
    /// All lives lost — carries the final score so Game can offer the
    /// high-score name entry / leaderboard (Game supplies the stage number).
    GameOver(i64),
}

/// Per-stage wiring that varies by stage/character: dialogue portrait texture
/// slots and the stage/boss music tracks.
#[derive(Clone, Copy)]
pub struct StageConfig {
    pub face_player_tex: usize,
    pub face_boss_tex: usize,
    pub stage_bgm: &'static str,
    pub boss_bgm: &'static str,
    /// 1-based stage number (Gui currentStage), for the clear bonus + banner.
    pub stage_num: i32,
    pub difficulty: Difficulty,
    pub practice: bool,
    /// Total starting lives; EoSD's config stores this value minus one.
    pub starting_lives: i32,
    pub starting_bombs: i32,
    /// RNG state at stage entry, before EnemyManager's two item-table draws.
    pub rng_seed: u16,
}

/// One original face ANM file, in Gui's A/B/C load order. Sprites keep their
/// original local indices; Stage applies the distinct player/boss offsets.
#[derive(Clone)]
pub struct PortraitAsset {
    pub tex: usize,
    pub entry: Arc<Entry>,
    pub tex_size: (u32, u32),
}

impl PortraitAsset {
    pub fn new(tex: usize, entry: &Entry) -> Self {
        Self {
            tex,
            tex_size: (entry.width, entry.height),
            entry: Arc::new(Entry {
                width: entry.width,
                height: entry.height,
                format: entry.format,
                name: entry.name.clone(),
                alpha_name: entry.alpha_name.clone(),
                sprites: entry.sprites.clone(),
                scripts: entry.scripts.clone(),
            }),
        }
    }
}

/// Player progress carried from one stage to the next.
#[derive(Clone, Copy)]
pub struct Carry {
    pub lives: i32,
    pub bombs: i32,
    pub power: i32,
    pub score: i64,
    pub graze: i64,
    pub power_item_count: usize,
    pub rng_seed: u16,
    pub rank: i32,
    pub continues_used: u8,
    pub next_extend: usize,
    pub point_items_total: i64,
    pub deaths: u32,
    pub bombs_used: u32,
    pub spellcards_captured: u32,
    pub starting_spare_lives: i32,
    pub starting_bombs: i32,
}

struct Shot {
    /// Original fixed Player::bullets[80] index: reused from the first free slot.
    pool_index: usize,
    pos: [f32; 2],
    size: [f32; 2],
    vel: [f32; 2],
    damage: i32,
    /// BULLET_TYPE_*: 0 straight, 1 homing orb (Reimu A), 2 gravity orb-missile
    /// (Marisa A), 3 laser (handled separately).
    bt: u8,
    /// Player-anm script id (BulletData anmScriptIdx). Resolved against the
    /// current character's sheet at draw, so each character shows their own
    /// shot sprite from the same id.
    anm_script: i32,
    runner: AnmRunner,
    collided: bool,
    angle: f32,
    motion: [f32; 2],
    spawn: u8,
    laser_slot: usize,
    age: u32,
    /// Current speed magnitude (Player.cpp `unk_134.y`), grown while homing.
    spd: f32,
}

#[derive(Clone, Copy, PartialEq)]
enum OrbState {
    Hidden,
    Unfocused,
    Focusing,
    Focused,
    Unfocusing,
}

/// Player character + shot type.
#[derive(Clone, Copy, PartialEq)]
pub enum Character {
    ReimuA,
    ReimuB,
    MarisaA,
    MarisaB,
}

impl Character {
    pub fn label(self) -> &'static str {
        match self {
            Character::ReimuA => "Reimu A",
            Character::ReimuB => "Reimu B",
            Character::MarisaA => "Marisa A",
            Character::MarisaB => "Marisa B",
        }
    }
    /// True for Marisa (uses player01.anm instead of player00).
    pub fn is_marisa(self) -> bool {
        matches!(self, Character::MarisaA | Character::MarisaB)
    }
    fn ranks(self) -> &'static [&'static [ShotDef]; 9] {
        match self {
            Character::ReimuA => &REIMU_A_RANKS,
            Character::ReimuB => &REIMU_B_RANKS,
            Character::MarisaA => &MARISA_A_RANKS,
            Character::MarisaB => &MARISA_B_RANKS,
        }
    }
}

/// Power thresholds shared by all shot types (CharacterPowerData.power).
const POWER_THRESH: [i32; 9] = [8, 16, 32, 48, 64, 80, 96, 127, 999];

fn power_rank(power: i32) -> usize {
    let mut i = 0;
    while power >= POWER_THRESH[i] {
        i += 1;
    }
    i
}

/// One axis-aligned bomb rectangle (Player::PlayerRect), rebuilt every frame.
/// `bomb_projectiles` cancel bullets (Player::CalcKillBoxCollision / CheckGraze
/// return 2 → the bullet despawns and drops a point item); `bomb_regions` damage
/// enemies (Player::CalcDamageToEnemy via bombRegionSizes/Positions/Damages). An
/// entry with `size[0] == 0` is inactive, matching the decomp's per-frame clear
/// (Player::OnUpdate zeroes every `sizeX` before the bomb calc repopulates them).
#[derive(Clone, Copy, Default)]
struct BombRect {
    pos: [f32; 2],
    size: [f32; 2],
}

/// Independent ScreenEffect chain; equal-priority chains keep registration order.
struct ScreenShake {
    timer: i32,
    length: i32,
    start: f32,
    end: f32,
}

/// Preserve CalcDamageToEnemy's corner arithmetic and inclusive edge tests.
fn player_shot_in_bounds(pos: [f32; 2], size: [f32; 2], region: [f32; 2]) -> bool {
    // GameManager::IsInBounds uses ordered rejection tests, so NaN positions
    // remain alive until their ANM ends, as in the original homing code.
    !(size[0] / 2.0 + pos[0] < 0.0
        || pos[0] - size[0] / 2.0 > region[0]
        || size[1] / 2.0 + pos[1] < 0.0
        || pos[1] - size[1] / 2.0 > region[1])
}

fn attack_rect_overlap(a: [f32; 2], a_size: [f32; 2], b: [f32; 2], b_size: [f32; 2]) -> bool {
    !(a[1] - a_size[1] / 2.0 > b[1] + b_size[1] / 2.0
        || a[0] - a_size[0] / 2.0 > b[0] + b_size[0] / 2.0
        || a[1] + a_size[1] / 2.0 < b[1] - b_size[1] / 2.0
        || a[0] + a_size[0] / 2.0 < b[0] - b_size[0] / 2.0)
}

/// Falling collectible (ItemManager port). `kind` matches ItemType:
/// 0 power small, 1 point, 2 power big, 3 bomb, 4 full power, 5 life,
/// 6 point-bullet (from cancelled bullets). `state` is the ItemManager state:
/// 0 = normal fall, 1 = homing to player (latched, never reverts), 2 = the
/// 60-frame scatter arc used by death/boss-kill drops, then it falls.
struct Item {
    pool_index: usize,
    pos: [f32; 2],
    vy: f32,
    kind: i32,
    state: u8,
    timer: i32,
    start: [f32; 2],
    target: [f32; 2],
}

impl Item {
    /// Normal drop: pops up (vy -2.2) then falls under gravity.
    fn fall(pos: [f32; 2], kind: i32) -> Self {
        Item {
            pool_index: 0,
            pos,
            vy: -2.2,
            kind,
            state: 0,
            timer: 0,
            start: [0.0; 2],
            target: [0.0; 2],
        }
    }
    /// Cancelled-bullet point item: homes to the player immediately (state 1).
    fn homing(pos: [f32; 2], kind: i32) -> Self {
        Item {
            pool_index: 0,
            pos,
            vy: 0.0,
            kind,
            state: 1,
            timer: 0,
            start: [0.0; 2],
            target: [0.0; 2],
        }
    }
    /// Death / boss-kill scatter (state 2): arcs to a random target over 60
    /// frames (SpawnItem state==2: x in 48..336, y in -64..128) then falls.
    fn scatter(pos: [f32; 2], kind: i32, rng: &mut Rng) -> Self {
        let target = [
            rng.f32_in_range(288.0) + 48.0,
            rng.f32_in_range(192.0) - 64.0,
        ];
        Item {
            pool_index: 0,
            pos,
            vy: 0.0,
            kind,
            state: 2,
            timer: 0,
            start: pos,
            target,
        }
    }
}

/// Spell card names in English, indexed by the ECL spell id (op93). Only
/// stage 1's are filled in; the in-game names are Shift-JIS and cannot be
/// drawn with the ASCII font.
/// Walk every msg entry and collect (instruction offset, raw Shift-JIS text
/// bytes) from each TEXTDIALOGUE (op3, text NUL-terminated from arg byte 4).
/// Keyed by absolute offset so run_dialogue can look up the pre-baked texture.
pub fn scan_dialogue_lines(msg: &Msg) -> Vec<(u32, Vec<u8>)> {
    let mut out = Vec::new();
    for &start in &msg.offsets {
        let mut off = start;
        let mut budget = 10_000;
        while let Some(i) = msg.instr_at(off) {
            if matches!(i.opcode, 3 | 8) && i.args.len() > 4 {
                let text = &i.args[4..];
                let end = text.iter().position(|&b| b == 0).unwrap_or(text.len());
                if end > 0 {
                    out.push((off, text[..end].to_vec()));
                }
            }
            if i.opcode == 0 || i.opcode == 10 || i.size == 0 {
                break; // MSGDELETE / MSGHALT ends this entry
            }
            off += i.size;
            budget -= 1;
            if budget == 0 {
                break;
            }
        }
    }
    out
}

/// Walk every sub of an ecl and collect (spellcard id, raw Shift-JIS name
/// bytes) from each SPELLCARDSTART (op93: id at arg byte 2, NUL-terminated name
/// from byte 4). Used to pre-bake the original Japanese name textures at load.
pub fn scan_spell_names(ecl: &Ecl) -> Vec<(i32, Vec<u8>)> {
    let mut out = Vec::new();
    for &start in &ecl.sub_offsets {
        let mut off = start;
        let mut budget = 100_000; // guard against a malformed offset loop
        while let Some(ins) = ecl.instr_at(off) {
            if ins.opcode == 93 && ins.args.len() >= 4 {
                let id = i16::from_le_bytes([ins.args[2], ins.args[3]]) as i32;
                let name = &ins.args[4..];
                let end = name.iter().position(|&c| c == 0).unwrap_or(name.len());
                out.push((id, name[..end].to_vec()));
            }
            if ins.offset_to_next <= 0 {
                break;
            }
            off += ins.offset_to_next as u32;
            budget -= 1;
            if budget == 0 {
                break;
            }
        }
    }
    out
}

/// English spellcard names indexed by the ECL SPELLCARDSTART id (op93 byte 2).
/// Kept as a fallback for cards whose JP-name texture failed to bake.
#[allow(dead_code)]
fn spellcard_name(id: i32) -> &'static str {
    match id {
        // Rumia (stage 1)
        0 => "Moon Sign \"Moonlight Ray\"",
        1 => "Night Sign \"Night Bird\"",
        2 => "Darkness Sign \"Demarcation\"",
        // Cirno (stage 2)
        3 => "Ice Sign \"Icicle Fall\"",
        4 => "Hail Sign \"Hailstorm\"",
        5 => "Freeze Sign \"Perfect Freeze\"",
        6 => "Snow Sign \"Diamond Blizzard\"",
        // Hong Meiling (stage 3)
        7 => "Flower Sign \"Gorgeous Sweet Flower\"",
        8 => "Flower Sign \"Selaginella 9\"",
        9 => "Rainbow Sign \"Wind Chime of the Colorful Rainbow\"",
        10 => "Illusion Sign \"Flower-Imagining Dreamweed\"",
        11 => "Colorful Sign \"Colorful Rain\"",
        12 => "Colorful Sign \"Wild Dance of Colorful Light\"",
        13 => "Colorful Sign \"Intense Colorful Typhoon\"",
        // Patchouli Knowledge (stage 4)
        14 => "Fire Sign \"Agni Shine\"",
        15 => "Water Sign \"Princess Undine\"",
        16 => "Wood Sign \"Sylphae Horn\"",
        17 => "Earth Sign \"Rage Trilithon\"",
        18 => "Metal Sign \"Metal Fatigue\"",
        19 => "Fire Sign \"Agni Shine (Advanced)\"",
        20 => "Wood Sign \"Sylphae Horn (Advanced)\"",
        21 => "Earth Sign \"Rage Trilithon (Advanced)\"",
        22 => "Fire Sign \"Agni Radiance\"",
        23 => "Water Sign \"Bury in Lake\"",
        24 => "Wood Sign \"Green Storm\"",
        25 => "Earth Sign \"Trilithon Shake\"",
        26 => "Metal Sign \"Silver Dragon\"",
        27 => "Fire & Earth Sign \"Lava Cromlech\"",
        28 => "Wood & Fire Sign \"Forest Blaze\"",
        29 => "Water & Wood Sign \"Water Elf\"",
        30 => "Metal & Water Sign \"Mercury Poison\"",
        31 => "Earth & Metal Sign \"Emerald Megalith\"",
        // Sakuya Izayoi (stage 5)
        32 => "Conjuring \"Misdirection\"",
        33 => "Conjuring \"Illusionary Misdirection\"",
        34 => "Illusion Existence \"Clock Corpse\"",
        35 => "Illusion Image \"Luna Clock\"",
        36 => "Maid Secret Skill \"Marionette\"",
        37 => "Illusion Ghost \"Jack the Ludo Bile\"",
        38 => "Illusion World \"The World\"",
        39 => "Maid Secret Skill \"Killing Doll\"",
        // Remilia Scarlet (stage 6)
        40 => "Conjuring \"Eternal Meek\"",
        41 => "Divine Punishment \"Star of David\"",
        42 => "Nether Sign \"Scarlet Netherworld\"",
        43 => "Curse \"Vlad Tepes' Curse\"",
        44 => "Scarlet Sign \"Scarlet Shoot\"",
        45 => "\"Red Magic\"",
        46 => "Divine Punishment \"Young Demon Lord\"",
        47 => "Hell Sign \"Mountain of a Thousand Needles\"",
        48 => "God Sign \"Vampire Illusion\"",
        49 => "Scarlet Sign \"Scarlet Meister\"",
        50 => "\"Scarlet Gensokyo\"",
        _ => "Spell Card",
    }
}

/// Per-spellcard capture score, indexed by spellcard id (g_SpellcardScore,
/// EclManager.cpp:18). The capture bonus is `score * (1 + secondsLeft/10)`.
const SPELLCARD_SCORE: [i64; 64] = [
    200000, 200000, 200000, 200000, 200000, 200000, 200000, 250000, 250000, 250000, 250000, 250000,
    250000, 250000, 300000, 300000, 300000, 300000, 300000, 300000, 300000, 300000, 300000, 300000,
    300000, 300000, 300000, 300000, 300000, 300000, 300000, 300000, 400000, 400000, 400000, 400000,
    400000, 400000, 400000, 400000, 500000, 500000, 500000, 500000, 500000, 500000, 600000, 600000,
    600000, 600000, 600000, 700000, 700000, 700000, 700000, 700000, 700000, 700000, 700000, 700000,
    700000, 700000, 700000, 700000,
];

/// Item drop pattern for ITEM_RANDOM_ITEM enemies (g_RandomItems).
const RANDOM_ITEMS: [i32; 32] = [
    0, 0, 1, 0, 1, 0, 0, 1, 1, 1, 0, 0, 0, 1, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 0, 1, 1, 1, 0, 2,
];

/// Running MSG dialogue (GuiImpl::RunMsg port, text-only for now).
struct Dialogue {
    active: bool,
    msg_idx: usize,
    off: u32,
    timer: u16,
    frames_pause: u16,
    ecl_resumed: bool,
    skippable: bool,
    lines: [String; 2],
    line_colors: [usize; 2],
    /// Pre-baked JP line textures (slot, w, h) for the two visible lines.
    line_tex: [Option<(usize, f32, f32)>; 2],
    line_runners: [Option<AnmRunner>; 2],
    portraits: [Option<AnmRunner>; 2],
    intro_lines: [String; 2],
    intro_colors: [usize; 2],
    intro_tex: [Option<(usize, f32, f32)>; 2],
    intro_runners: [Option<AnmRunner>; 2],
}

impl Default for Dialogue {
    fn default() -> Self {
        Self {
            active: false,
            msg_idx: 0,
            off: 0,
            timer: 0,
            frames_pause: 0,
            ecl_resumed: false,
            skippable: true,
            lines: [String::new(), String::new()],
            line_colors: [0, 0],
            line_tex: [None, None],
            line_runners: [None, None],
            portraits: [None, None],
            intro_lines: [String::new(), String::new()],
            intro_colors: [0, 0],
            intro_tex: [None, None],
            intro_runners: [None, None],
        }
    }
}

#[derive(Hash)]
enum PlayerState {
    Alive,
    /// Elapsed Player::invulnerabilityTimer while dead or spawning.
    Dead(u32),
    Spawning(u32),
    GameOver,
    Cleared(u32),
    /// Stage-results display and transition are timed by the original MSG.
    Results,
}

/// One enemy ANM script with its sprite table and texture slot.
pub struct ScriptRef {
    pub tex: usize,
    pub instrs: Vec<AnmInstr>,
    pub sprites: HashMap<u32, Sprite>,
    pub tex_size: [f32; 2],
}

/// Build a sprite DrawCmd from an ANM runner + its script, centred at (cx, cy).
/// Shared by the enemy primary vm and the op99 sub-vms.
fn sprite_cmd_from(
    anim: &AnmRunner,
    script: &ScriptRef,
    cx: f32,
    cy: f32,
    rot: f32,
    additive: bool,
) -> Option<DrawCmd> {
    let idx = anim.sprite?;
    let sp = script.sprites.get(&idx)?;
    let [tw, th] = script.tex_size;
    let w = sp.width * anim.scale[0].abs();
    let h = sp.height * anim.scale[1].abs();
    // Sprite coordinates can exceed the sheet edge (Rumia); wrap.
    let sx = sp.x % tw;
    let sy = sp.y % th;
    let (mut u0, mut u1) = (sx / tw, (sx + sp.width) / tw);
    if anim.flip_x {
        std::mem::swap(&mut u0, &mut u1);
    }
    let (mut v0, mut v1) = (sy / th, (sy + sp.height) / th);
    if anim.flip_y {
        std::mem::swap(&mut v0, &mut v1);
    }
    Some(DrawCmd {
        tex: script.tex,
        dst: [cx - w / 2.0, cy - h / 2.0, w, h],
        src: [u0, v0, u1, v1],
        tint: [anim.color[0], anim.color[1], anim.color[2], anim.alpha],
        rot,
        additive,
        color_add: anim.color_add,
    })
}

/// Build the enemy script table keyed by ANM script id (stg1enm uses ids
/// 0..7, stg1enm2 the boss bank at 128+), matching ECL ANMSETMAIN indices.
pub fn build_enemy_scripts(entries: &[(&Entry, usize)]) -> HashMap<i32, ScriptRef> {
    let mut out = HashMap::new();
    for (entry, tex) in entries {
        for (id, instrs) in &entry.scripts {
            out.insert(
                *id as i32,
                ScriptRef {
                    tex: *tex,
                    instrs: instrs.clone(),
                    sprites: entry.sprites.iter().map(|s| (s.index, s.clone())).collect(),
                    tex_size: [entry.width as f32, entry.height as f32],
                },
            );
        }
    }
    out
}

struct BombBanner {
    name_script: Vec<AnmInstr>,
    portrait_script: Vec<AnmInstr>,
    textures: [(usize, usize); 4],
    name: Option<AnmRunner>,
    portrait: Option<AnmRunner>,
}

pub struct Stage {
    tick: u32,
    anim: u32,
    ecl: Ecl,
    world: World,
    enemies: Vec<Enemy>,
    anims: Vec<Option<AnmRunner>>,
    /// Sub-vm anm runners per enemy (ECL op99 ANMSETSLOT, enemy->vms[8]). Parallel
    /// to `enemies`/`anims`. Slots 0-3 draw behind the primary sprite, 4-7 in front.
    sub_anims: Vec<[Option<AnmRunner>; 8]>,
    timeline_off: u32,
    timeline_time: i32,
    enemy_scripts: HashMap<i32, ScriptRef>,
    bullet_sprites: HashMap<u32, Sprite>,
    bullet_tex_size: [f32; 2],
    /// etama4 (effect) sprite table + texture for rendering particles.
    eff_sprites: HashMap<u32, Sprite>,
    eff_tex: usize,
    eff_tex_size: [f32; 2],
    /// Per bullet-type (etama3.anm scripts 0..9) "auto-rotate" flag (anm op 26):
    /// directional bullets (rice/kunai/shard/dagger) face their travel angle,
    /// round ones don't. Read from etama3.anm, not hard-coded.
    bullet_autorotate: [bool; 10],
    // player
    character: Character,
    /// Texture slot for the player body sprite (player00 Reimu / player01 Marisa).
    player_tex: usize,
    /// Player::laserTimer (only slots 0 and 1 are used by the data tables).
    laser_timers: [i32; 2],
    /// Shared laser/bomb hit-particle cadence, Player::particleTimer.
    player_particle_timer: u8,
    /// Active bomb kind: 0 Fantasy Seal, 1 Dream cross, 2 Stardust, 3 Master Spark.
    bomb_kind: u8,
    /// player00.anm: sprites + scripts, for the banking/idle animation.
    player_sprites: HashMap<u32, Sprite>,
    player_scripts: HashMap<i32, Vec<AnmInstr>>,
    player_tex_size: [f32; 2],
    player_runner: AnmRunner,
    player_script_id: i32,
    /// Previous horizontal speed sign, to detect banking transitions.
    prev_hspeed: f32,
    orb_state: OrbState,
    orb_timer: u32,
    orb_offsets: [f32; 2],
    orbs_pos: [[f32; 2]; 2],
    orb_runners: [AnmRunner; 2],
    pos: [f32; 2],
    lives: i32,
    bombs: i32,
    invuln: u32,
    bullet_grace_period: u32,
    /// `> 0` while a bomb is running (mirrors PlayerBombInfo.isInUse for the many
    /// `bombing > 0` gates).
    bombing: u32,
    /// Bomb timer (PlayerBombInfo.timer): counts up 0..bomb_duration.
    bomb_timer: i32,
    /// Bomb duration (PlayerBombInfo.duration), set by the active bomb's calc.
    bomb_duration: i32,
    /// PlayerBombInfo.isInUse.
    bomb_in_use: bool,
    /// Movement speed multiplier during a bomb (MarisaB slows to 0.3, others 1.0).
    bomb_move_mult: f32,
    /// ReimuA Fantasy Seal per-seal state machine (PlayerBombInfo): 0 dormant,
    /// 1 homing, 2.. exploding/cooldown. `seal_speed` is reimuABombProjectiles-
    /// Related; `seal_pos`/`seal_vel` are bombInfo.bombRegion{Positions,Velocities}.
    seal_state: [i32; 8],
    seal_speed: [f32; 8],
    seal_pos: [[f32; 2]; 8],
    seal_vel: [[f32; 2]; 8],
    /// Per-region accumulated raw bomb damage (Player::unk_838): ReimuA explodes a
    /// seal once it has dealt >= 100. Reset at bomb start, not each frame.
    bomb_dmg_accum: [i32; 32],
    /// Transient bomb rects, zeroed each frame then rebuilt by the bomb calc.
    /// `bomb_projectiles` cancel bullets; `bomb_regions`/`bomb_region_dmg` damage
    /// enemies (see [`BombRect`]).
    bomb_projectiles: [BombRect; 16],
    bomb_regions: [BombRect; 32],
    bomb_region_dmg: [i32; 32],
    /// ScreenEffect SHAKE (ScreenEffect.cpp). `arcade` is the current
    /// arcadeRegion [topLeftX, topLeftY, sizeW, sizeH]; baseline 32/16/384/448.
    /// Each registered shake consumes two u32 RNG draws (x, y) per active tick.
    /// Overlapping Fantasy Seal explosions retain separate ScreenEffect chains.
    arcade: [f32; 4],
    shakes: Vec<ScreenShake>,
    /// spellcardInfo.usedBomb: a bomb was fired while a spellcard was active.
    spell_used_bomb: bool,
    /// Bomb sprite anm runners (PlayerBombInfo.sprites[8][4], flattened to 32),
    /// started by each bomb's calc (ExecuteAnmIdx) and ticked per frame
    /// (ExecuteScript). ReimuA: seal i slot k → index i*4+k (scripts 133+k).
    /// ReimuB: beam i (scripts 137+i). MarisaA: star i, 0..8 (scripts 5+i%3).
    /// MarisaB: i, 0..4 (scripts 8+i). ReimuB also reads each runner's `pos` as
    /// the posOffset added to its beam's region/projectile.
    bomb_sprites: Vec<Option<AnmRunner>>,
    /// Player.cpp fireBulletTimer: -1 idle, else counts 0..=30 while shooting.
    fire_timer: i32,
    /// Deathbomb grace: frames left to bomb after a lethal hit (0 = not dying).
    dying: u32,
    graze: i64,
    /// Point items collected this stage (Gui shows it as "Point"). Not carried.
    point_items: i64,
    point_items_total: i64,
    practice: bool,
    starting_lives: i32,
    starting_bombs: i32,
    continues_used: u8,
    next_extend: usize,
    deaths: u32,
    bombs_used: u32,
    spellcards_captured: u32,
    spellcard_records: Vec<SpellcardRecord>,
    is_replay: bool,
    stage_rng_seed: u16,
    /// Focus held last update, for drawing the orbs in their focused position.
    last_input_focus: bool,
    /// positionOfLastEnemyHit: the lowest on-screen enemy, for the orb-amulet and
    /// ReimuA-seal homing (set in `collide`).
    last_enemy_hit: Option<[f32; 2]>,
    state: PlayerState,
    shots: Vec<Shot>,
    items: Vec<Item>,
    next_item_index: usize,
    score_popups: crate::score_popups::ScorePopups,
    score: i64,
    /// Displayed score, rolled toward `score` each frame (Gui guiScore).
    gui_score: i64,
    next_score_inc: i64,
    /// "Full Power Mode!!" popup: frames remaining + edge-tracking of max power.
    full_power_timer: u32,
    was_full: bool,
    /// Eased boss health bar (Gui bossHealthBar2): rises 0.01/frame toward the
    /// boss life fraction, falls 0.02/frame.
    boss_bar: f32,
    hiscore: i64,
    clear_bonus: i64,
    /// Running count for scoring power items collected at max power.
    power_item_count: usize,
    paused: bool,
    pause_state: u8,
    pause_timer: u16,
    pause_asset: Option<PortraitAsset>,
    pause_runners: [Option<AnmRunner>; 6],
    rand_item_table: usize,
    rand_item_spawn: usize,
    spell_active: bool,
    spell_name: String,
    spell_capturing: bool,
    spell_result: u32,
    spell_captured: bool,
    /// Current spellcard id (g_SpellcardScore index) and last-seen seconds left,
    /// for the capture bonus; the "Spell Card Bonus!" popup (frames, amount).
    spell_id: i32,
    spell_secs: i32,
    /// text.anm script 7 (TEXT_ENEMY_SPELLCARD_NAME) + the live runner that
    /// drives the spell-name announce animation (fly-in, shrink, fly-up/off).
    spell_name_script: Vec<AnmInstr>,
    spell_name_runner: Option<AnmRunner>,
    /// Pre-baked JP spellcard-name textures by id (slot, w, h), and the current
    /// card's entry while its announce runs.
    spell_name_tex: HashMap<i32, (usize, f32, f32)>,
    spell_name_cur: Option<(usize, f32, f32)>,
    spell_name_byte_len: usize,
    bomb_banner: Option<BombBanner>,
    /// Pre-baked JP dialogue-line textures by TEXTDIALOGUE offset.
    dialogue_tex: HashMap<u32, (usize, f32, f32)>,
    /// Stage-name intro (text.anm script 0): pre-baked JP texture + live runner.
    stage_name_tex: Option<(usize, f32, f32)>,
    stage_name_runner: Option<AnmRunner>,
    /// Boss spellcard portrait (face*.anm script 3) + which boss-face sprite.
    portrait_script: Vec<AnmInstr>,
    portrait_runner: Option<AnmRunner>,
    spell_bonus_timer: u32,
    spell_bonus_amount: i64,
    /// "BONUS" popup (bullet-cancel total) — frames remaining, amount.
    bonus_score_timer: u32,
    bonus_score_amount: i64,
    boss_bgm_started: bool,
    /// Original face files, including full-width boss expressions in B/C.
    player_portraits: Vec<PortraitAsset>,
    boss_portraits: Vec<PortraitAsset>,
    extra_boss_portrait: Option<PortraitAsset>,
    dialogue_scripts: HashMap<u32, Vec<AnmInstr>>,
    /// Music tracks for this stage (field theme / boss theme).
    stage_bgm: &'static str,
    boss_bgm: &'static str,
    /// 1-based stage number + the cumulative graze at stage start, for the
    /// clear bonus (Gui currentStage / grazeInStage).
    stage_num: i32,
    graze_in_stage: i64,
    msg: Msg,
    dialogue: Dialogue,
    background: Option<Background>,
    hud: Hud,
    pub events: Vec<Event>,
}

impl Stage {
    pub fn set_pause_art(&mut self, asset: PortraitAsset) {
        self.pause_asset = Some(asset);
    }

    pub fn set_dialogue_scripts(&mut self, text: &Entry) {
        self.dialogue_scripts = text.scripts.iter().cloned().collect();
    }

    pub fn set_portrait_assets(&mut self, player: Vec<PortraitAsset>, boss: Vec<PortraitAsset>) {
        if let Some(script) = player
            .first()
            .and_then(|asset| asset.entry.scripts.iter().find(|(id, _)| *id == 3))
        {
            self.portrait_script = script.1.clone();
        }
        self.player_portraits = player;
        self.boss_portraits = boss;
    }

    pub fn set_extra_boss_portrait(&mut self, asset: PortraitAsset) {
        self.extra_boss_portrait = Some(asset);
    }

    pub fn configure_bomb_banner(&mut self, text: &Entry, textures: [(usize, usize); 4]) {
        self.bomb_banner = Some(BombBanner {
            name_script: text
                .scripts
                .iter()
                .find(|(id, _)| *id == 6)
                .map(|(_, script)| script.clone())
                .unwrap_or_default(),
            portrait_script: self
                .player_portraits
                .first()
                .and_then(|asset| asset.entry.scripts.iter().find(|(id, _)| *id == 1))
                .map(|(_, script)| script.clone())
                .unwrap_or_default(),
            textures,
            name: None,
            portrait: None,
        });
    }

    fn show_bomb_banner(&mut self) {
        if let Some(banner) = &mut self.bomb_banner {
            banner.name = Some(AnmRunner::new(banner.name_script.clone()));
            let mut portrait = AnmRunner::new(banner.portrait_script.clone());
            portrait.set_sprite(if matches!(self.bomb_kind, 0 | 3) {
                1
            } else {
                3
            });
            banner.portrait = Some(portrait);
        }
        self.events.push(Event::Sfx("cat00"));
    }

    fn end_bomb_banner(&mut self) {
        if let Some(banner) = &mut self.bomb_banner
            && let Some(runner) = &mut banner.name
        {
            runner.interrupt(1);
        }
    }

    /// ReplayManager records dialogue frames as well as combat. Entering or
    /// leaving the pause menu consumes no simulation/input-recording frame.
    pub fn replay_frame_active(&self, input: &Input) -> bool {
        !(self.paused
            || input.pressed(Key::Pause)
                && matches!(
                    self.state,
                    PlayerState::Alive | PlayerState::Dead(_) | PlayerState::Spawning(_)
                ))
    }

    pub fn replay_accelerates_dialogue(&self) -> bool {
        !self.paused && self.dialogue.active && self.dialogue.skippable
    }

    fn portrait_sprite(&self, index: u32) -> Option<(&PortraitAsset, &Sprite)> {
        const PLAYER_OFFSETS: [u32; 3] = [0, 2, 4];
        const BOSS_OFFSETS: [u32; 3] = [8, 10, 11];
        for (assets, offsets) in [
            (&self.player_portraits, PLAYER_OFFSETS),
            (&self.boss_portraits, BOSS_OFFSETS),
        ] {
            for (asset, offset) in assets.iter().zip(offsets) {
                if let Some(sprite) = index
                    .checked_sub(offset)
                    .and_then(|id| asset.entry.sprites.iter().find(|sprite| sprite.index == id))
                {
                    return Some((asset, sprite));
                }
            }
        }
        None
    }

    /// Gui::DrawGameScene places the unscaled bar independently of the text
    /// sprite's entrance scale. The names occupy fixed 256×16 ANM sprites.
    fn draw_spell_banner(
        &self,
        runner: &AnmRunner,
        tex: usize,
        byte_len: usize,
        bomb: bool,
        cmds: &mut Vec<DrawCmd>,
    ) {
        if !runner.visible() {
            return;
        }
        let x = runner.pos[0] + runner.pos_offset[0];
        let y = runner.pos[1] + runner.pos_offset[1];
        let len = byte_len as f32 * 15.0 / 2.0 + 16.0;
        if let Some(([sx, sy, sw, sh], _, scale, alpha)) =
            self.hud.script_state(if bomb { 23 } else { 24 })
        {
            let bar_x = x + if bomb {
                len * 16.0 / 15.0 / 2.0 - 144.0
            } else {
                128.0 - len * 16.0 / 15.0 / 2.0
            };
            let width = sw * len / 14.0;
            let height = sh * scale[1];
            let ts = self.hud.tex_size();
            cmds.push(DrawCmd {
                tex: self.hud.tex(),
                dst: [bar_x - width / 2.0, y - height / 2.0, width, height],
                src: [sx / ts, sy / ts, (sx + sw) / ts, (sy + sh) / ts],
                tint: [1.0, 1.0, 1.0, alpha],
                rot: 0.0,
                additive: false,
                color_add: false,
            });
        }
        let w = 256.0 * runner.scale[0];
        let h = 16.0 * runner.scale[1];
        cmds.push(DrawCmd {
            tex,
            dst: [x - w / 2.0, y - h / 2.0, w, h],
            src: [0.0, 0.0, 1.0, 1.0],
            tint: [
                runner.color[0],
                runner.color[1],
                runner.color[2],
                runner.alpha,
            ],
            rot: 0.0,
            additive: runner.additive,
            color_add: false,
        });
    }

    fn draw_portrait(&self, runner: &AnmRunner, cmds: &mut Vec<DrawCmd>) {
        if !runner.visible() {
            return;
        }
        let Some((asset, sprite)) = runner
            .sprite
            .and_then(|sprite| self.portrait_sprite(sprite))
        else {
            return;
        };
        let w = sprite.width * runner.scale[0].abs();
        let h = sprite.height * runner.scale[1].abs();
        let x = runner.pos[0] + runner.pos_offset[0];
        let y = runner.pos[1] + runner.pos_offset[1];
        let (tw, th) = (asset.tex_size.0 as f32, asset.tex_size.1 as f32);
        let mut src = [
            sprite.x / tw,
            sprite.y / th,
            (sprite.x + sprite.width) / tw,
            (sprite.y + sprite.height) / th,
        ];
        if runner.flip_x ^ (runner.scale[0] < 0.0) {
            src.swap(0, 2);
        }
        if runner.flip_y ^ (runner.scale[1] < 0.0) {
            src.swap(1, 3);
        }
        cmds.push(DrawCmd {
            tex: asset.tex,
            dst: if runner.corner {
                [x, y, w, h]
            } else {
                [x - w / 2.0, y - h / 2.0, w, h]
            },
            src,
            tint: [
                runner.color[0],
                runner.color[1],
                runner.color[2],
                runner.alpha,
            ],
            rot: 0.0,
            additive: runner.additive,
            color_add: false,
        });
    }

    #[allow(clippy::too_many_arguments)] // Original data-table/function argument layout.
    fn draw_dialogue_line(
        &self,
        cmds: &mut Vec<DrawCmd>,
        runner: Option<&AnmRunner>,
        texture: Option<(usize, f32, f32)>,
        text: &str,
        color: usize,
        fallback_pos: [f32; 2],
    ) {
        if text.is_empty() || runner.is_some_and(|runner| !runner.visible()) {
            return;
        }
        let mut pos = fallback_pos;
        let mut scale = [1.0, 1.0];
        let mut alpha = 1.0;
        if let Some(runner) = runner {
            pos = [
                runner.pos[0] + runner.pos_offset[0],
                runner.pos[1] + runner.pos_offset[1],
            ];
            scale = runner.scale;
            alpha = runner.alpha;
            // Dynamic text occupies a 256x16 ANM sprite; the glyph texture
            // contains only the used width, so its origin follows that sprite.
            if !runner.corner {
                pos[0] -= 128.0 * scale[0];
                pos[1] -= 8.0 * scale[1];
            }
        }
        let tint = if color == 0 {
            [232.0 / 255.0, 240.0 / 255.0, 1.0, alpha]
        } else {
            [1.0, 232.0 / 255.0, 240.0 / 255.0, alpha]
        };
        if let Some((tex, width, height)) = texture {
            cmds.push(DrawCmd {
                tex,
                dst: [pos[0], pos[1], width * scale[0], height * scale[1]],
                src: [0.0, 0.0, 1.0, 1.0],
                tint,
                rot: 0.0,
                additive: false,
                color_add: false,
            });
        } else {
            draw_text(cmds, pos, 16.0, tint, text);
        }
    }

    #[allow(clippy::too_many_arguments)] // Original data-table/function argument layout.
    pub fn new(
        ecl: Ecl,
        enemy_scripts: HashMap<i32, ScriptRef>,
        etama: &Entry,
        eff: &Entry,
        eff_tex: usize,
        player: &Entry,
        player_tex: usize,
        character: Character,
        msg: Msg,
        background: Option<Background>,
        hud: Hud,
        cfg: StageConfig,
        spell_name_script: Vec<AnmInstr>,
        portrait_script: Vec<AnmInstr>,
        spell_name_tex: HashMap<i32, (usize, f32, f32)>,
        dialogue_tex: HashMap<u32, (usize, f32, f32)>,
        stage_name_script: Vec<AnmInstr>,
        stage_name_tex: Option<(usize, f32, f32)>,
    ) -> Self {
        let timeline_off = ecl.timeline_offset;
        // etama4 (effect) ANM: per-script instructions for the runners, and the
        // shared sprite table + texture for rendering particles.
        let eff_scripts: HashMap<u32, Vec<AnmInstr>> = eff
            .scripts
            .iter()
            .map(|(id, instrs)| (*id, instrs.clone()))
            .collect();
        let eff_sprites: HashMap<u32, Sprite> =
            eff.sprites.iter().map(|s| (s.index, s.clone())).collect();
        let eff_tex_size = [eff.width as f32, eff.height as f32];
        let player_scripts: HashMap<i32, Vec<AnmInstr>> = player
            .scripts
            .iter()
            .map(|(id, instrs)| (*id as i32, instrs.clone()))
            .collect();
        let idle = player_scripts.get(&0).cloned().unwrap_or_default();
        // Bullet-type auto-rotate flags from etama3.anm (scripts 0..9): a type
        // rotates with its travel angle iff its script runs SetAutoRotate (op 26)
        // with a non-zero arg.
        let mut bullet_autorotate = [false; 10];
        for (id, instrs) in &etama.scripts {
            if (*id as usize) < bullet_autorotate.len() {
                bullet_autorotate[*id as usize] =
                    instrs.iter().any(|i| i.opcode == 26 && i.arg_u32(0) != 0);
            }
        }
        // Sprite height (px) per bullet type, from each type's base sprite — used
        // by ex-instructions that only touch big bullets (e.g. Sakuya's redirect).
        let mut bullet_heights = [0.0f32; 10];
        let mut bullet_widths = [0.0f32; 10];
        for (t, &base) in BULLET_BASE_SPRITE.iter().enumerate() {
            let sprites = if t == 9 { &eff.sprites } else { &etama.sprites };
            if let Some(sp) = sprites.iter().find(|s| s.index == base) {
                bullet_heights[t] = sp.height;
                bullet_widths[t] = sp.width;
            }
        }
        let (rank, min_rank, max_rank) = cfg.difficulty.rank_limits();
        let starting_lives = if cfg.practice || cfg.difficulty == Difficulty::Extra {
            2
        } else {
            cfg.starting_lives.clamp(1, 5) - 1
        };
        let power = if cfg.practice {
            match cfg.stage_num {
                1 => 0,
                2 => 64,
                _ => 128,
            }
        } else {
            0
        };
        let starting_bombs = if cfg.practice || cfg.difficulty == Difficulty::Extra {
            3
        } else {
            cfg.starting_bombs.clamp(0, 3)
        };
        // EnemyManager_AddedCallback: item-drop phase and initial table offset.
        let mut rng = Rng::new(cfg.rng_seed);
        let item_spawn_index = rng.u16_in_range(3) as usize;
        let item_table_index = rng.u16_in_range(8) as usize;
        Self {
            tick: 0,
            anim: 0,
            ecl,
            world: World {
                rng,
                difficulty: cfg.difficulty as u8,
                rank,
                sub_rank: 0,
                min_rank,
                max_rank,
                player_pos: [FIELD_W / 2.0, FIELD_H - 64.0],
                bullets: Vec::new(),
                next_bullet_index: 0,
                lasers: Vec::new(),
                events: Vec::new(),
                enemy_slots: [false; 256],
                pending_spawns: Vec::new(),
                kill_trash: false,
                boss_present: false,
                power,
                character: character.is_marisa() as u8,
                shot_type: matches!(character, Character::ReimuB | Character::MarisaB) as u8,
                time_stopped: false,
                bomb_active: false,
                spellcard_active: 0,
                bullet_heights,
                bullet_widths,
                bullet_templates: crate::projectiles::BulletVisuals::templates(etama, eff),
                effects: Vec::new(),
                next_effect_index: 0,
                eff_scripts,
                random_item_spawn_index: item_spawn_index as i32,
                random_item_table_index: item_table_index as i32,
            },
            enemies: Vec::new(),
            anims: Vec::new(),
            sub_anims: Vec::new(),
            timeline_off,
            timeline_time: 0,
            enemy_scripts,
            bullet_sprites: etama.sprites.iter().map(|s| (s.index, s.clone())).collect(),
            bullet_tex_size: [etama.width as f32, etama.height as f32],
            eff_sprites,
            eff_tex,
            eff_tex_size,
            bullet_autorotate,
            character,
            player_tex,
            laser_timers: [0; 2],
            player_particle_timer: 0,
            bomb_kind: 0,
            player_sprites: player
                .sprites
                .iter()
                .map(|s| (s.index, s.clone()))
                .collect(),
            player_tex_size: [player.width as f32, player.height as f32],
            player_runner: AnmRunner::new(idle),
            orb_runners: [
                AnmRunner::new(player_scripts.get(&128).cloned().unwrap_or_default()),
                AnmRunner::new(player_scripts.get(&129).cloned().unwrap_or_default()),
            ],
            player_scripts,
            player_script_id: 0,
            prev_hspeed: 0.0,
            orb_state: OrbState::Hidden,
            orb_timer: 0,
            orb_offsets: [0.0; 2],
            orbs_pos: [[0.0; 2]; 2],
            pos: [FIELD_W / 2.0, FIELD_H - 64.0],
            lives: starting_lives,
            bombs: starting_bombs,
            invuln: 0,
            bullet_grace_period: 0,
            bombing: 0,
            bomb_timer: 0,
            bomb_duration: 0,
            bomb_in_use: false,
            bomb_move_mult: 1.0,
            seal_state: [0; 8],
            seal_speed: [0.0; 8],
            seal_pos: [[0.0; 2]; 8],
            seal_vel: [[0.0; 2]; 8],
            bomb_dmg_accum: [0; 32],
            bomb_projectiles: [BombRect::default(); 16],
            bomb_regions: [BombRect::default(); 32],
            bomb_region_dmg: [0; 32],
            arcade: [FIELD_X, FIELD_Y, FIELD_W, FIELD_H],
            shakes: Vec::new(),
            spell_used_bomb: false,
            bomb_sprites: (0..32).map(|_| None).collect(),
            fire_timer: -1,
            dying: 0,
            graze: 0,
            point_items: 0,
            point_items_total: 0,
            practice: cfg.practice,
            starting_lives,
            starting_bombs,
            continues_used: 0,
            next_extend: if cfg.difficulty == Difficulty::Extra {
                4
            } else {
                0
            },
            deaths: 0,
            bombs_used: 0,
            spellcards_captured: 0,
            spellcard_records: vec![SpellcardRecord::default(); 64],
            is_replay: false,
            stage_rng_seed: cfg.rng_seed,
            last_input_focus: false,
            last_enemy_hit: None,
            state: PlayerState::Spawning(120),
            shots: Vec::new(),
            items: Vec::new(),
            next_item_index: 0,
            score_popups: crate::score_popups::ScorePopups::default(),
            score: 0,
            gui_score: 0,
            next_score_inc: 0,
            full_power_timer: 0,
            was_full: false,
            boss_bar: 0.0,
            hiscore: 0,
            clear_bonus: 0,
            power_item_count: 0,
            paused: false,
            pause_state: 0,
            pause_timer: 0,
            pause_asset: None,
            pause_runners: std::array::from_fn(|_| None),
            rand_item_table: item_table_index,
            rand_item_spawn: item_spawn_index,
            spell_active: false,
            spell_name: String::new(),
            spell_name_tex,
            spell_name_cur: None,
            spell_name_byte_len: 0,
            bomb_banner: None,
            dialogue_tex,
            stage_name_tex,
            // Start the stage-name intro immediately (Gui shows it at stage init).
            stage_name_runner: (!stage_name_script.is_empty() && stage_name_tex.is_some())
                .then(|| AnmRunner::new(stage_name_script)),
            spell_id: 0,
            spell_secs: 0,
            spell_name_script,
            spell_name_runner: None,
            portrait_script,
            portrait_runner: None,
            spell_bonus_timer: 0,
            spell_bonus_amount: 0,
            bonus_score_timer: 0,
            bonus_score_amount: 0,
            boss_bgm_started: false,
            spell_capturing: false,
            spell_result: 0,
            spell_captured: false,
            player_portraits: Vec::new(),
            boss_portraits: Vec::new(),
            extra_boss_portrait: None,
            dialogue_scripts: HashMap::new(),
            stage_bgm: cfg.stage_bgm,
            boss_bgm: cfg.boss_bgm,
            stage_num: cfg.stage_num,
            graze_in_stage: 0,
            msg,
            dialogue: Dialogue::default(),
            background,
            hud,
            events: vec![Event::Bgm(cfg.stage_bgm)],
        }
    }

    /// Snapshot the player progress to carry into the next stage.
    pub fn carry(&self) -> Carry {
        Carry {
            lives: self.lives,
            bombs: self.bombs,
            power: self.world.power,
            score: self.score,
            graze: self.graze,
            power_item_count: self.power_item_count,
            rng_seed: self.world.rng.seed(),
            rank: self.world.rank,
            continues_used: self.continues_used,
            next_extend: self.next_extend,
            point_items_total: self.point_items_total,
            deaths: self.deaths,
            bombs_used: self.bombs_used,
            spellcards_captured: self.spellcards_captured,
            starting_spare_lives: self.starting_lives,
            starting_bombs: self.starting_bombs,
        }
    }

    /// Restore progress carried from the previous stage.
    pub fn apply_carry(&mut self, c: Carry) {
        self.lives = c.lives;
        self.bombs = c.bombs;
        self.world.power = c.power;
        self.score = c.score;
        self.gui_score = c.score;
        self.next_score_inc = 0;
        self.graze = c.graze;
        self.power_item_count = c.power_item_count;
        // grazeInStage is an independent counter with its own 9999 cap.
        self.graze_in_stage = 0;
        self.world.rank = c.rank;
        // GameManager_AddedCallback preserves rank/RNG, resets subRank per stage.
        self.world.sub_rank = 0;
        self.continues_used = c.continues_used;
        self.next_extend = c.next_extend;
        self.point_items_total = c.point_items_total;
        self.deaths = c.deaths;
        self.bombs_used = c.bombs_used;
        self.spellcards_captured = c.spellcards_captured;
        self.starting_lives = c.starting_spare_lives;
        self.starting_bombs = c.starting_bombs;
        self.set_rng_seed(c.rng_seed);
    }

    /// Seed at stage entry; the two EnemyManager item-table draws happen next.
    pub fn set_rng_seed(&mut self, seed: u16) {
        self.stage_rng_seed = seed;
        self.world.rng = Rng::new(seed);
        self.rand_item_spawn = self.world.rng.u16_in_range(3) as usize;
        self.rand_item_table = self.world.rng.u16_in_range(8) as usize;
        self.world.random_item_spawn_index = self.rand_item_spawn as i32;
        self.world.random_item_table_index = self.rand_item_table as i32;
    }

    pub fn continues_used(&self) -> u8 {
        self.continues_used
    }

    pub fn spellcard_records(&self) -> &[SpellcardRecord] {
        &self.spellcard_records
    }

    pub fn set_spellcard_records(&mut self, records: &[SpellcardRecord]) {
        for (dst, src) in self.spellcard_records.iter_mut().zip(records) {
            *dst = src.clone();
        }
    }

    /// ReplayManager_AddedCallback records the stage-entry seed, not the RNG
    /// state after EnemyManager has selected its random item phase/table.
    pub fn replay_state(&self) -> ReplayStage {
        ReplayStage {
            score: self.score as u32,
            random_seed: self.stage_rng_seed,
            point_items: self.point_items_total as u16,
            power: self.world.power as u8,
            lives: self.lives as i8,
            bombs: self.bombs as i8,
            rank: self.world.rank as u8,
            power_item_count: self.power_item_count as i8,
            inputs: vec![ReplayInput {
                frame: 0,
                buttons: 0,
            }],
        }
    }

    /// Playback initialization runs before the first gameplay update. `score`
    /// comes from the previous recorded stage, as in ReplayManager.cpp.
    pub fn restore_replay_state(&mut self, replay: &ReplayStage, score: u32) {
        self.is_replay = true;
        self.set_rng_seed(replay.random_seed);
        self.world.rank = replay.rank as i32;
        self.world.sub_rank = 0;
        self.world.power = replay.power as i32;
        self.lives = replay.lives as i32;
        self.bombs = replay.bombs as i32;
        self.point_items_total = replay.point_items as i64;
        self.point_items = 0;
        self.power_item_count = replay.power_item_count.max(0) as usize;
        self.score = score as i64;
        self.gui_score = self.score;
        self.next_score_inc = 0;
        self.next_extend = if self.world.difficulty == Difficulty::Extra as u8 {
            4
        } else {
            0
        };
        while EXTEND_SCORES
            .get(self.next_extend)
            .is_some_and(|&value| value <= self.gui_score)
        {
            self.next_extend += 1;
        }
    }

    pub fn can_continue(&self) -> bool {
        !self.practice
            && !self.is_replay
            && self.world.difficulty < Difficulty::Extra as u8
            && self.continues_used < 3
    }

    /// StageMenu::OnUpdateRetryMenu, EoSD 1.02h. Continues resume this stage's
    /// existing enemies and timeline; the RNG and dynamic rank keep their state.
    pub fn continue_run(&mut self) -> bool {
        if !self.can_continue() {
            return false;
        }
        self.continues_used += 1;
        self.score = self.continues_used as i64;
        self.gui_score = self.score;
        self.next_score_inc = 0;
        self.lives = self.starting_lives;
        self.bombs = self.starting_bombs;
        self.graze_in_stage = 0;
        self.point_items = 0;
        self.world.power = 0;
        self.next_extend = 0;
        self.pos = [FIELD_W / 2.0, FIELD_H - 64.0];
        self.invuln = 0;
        self.dying = 0;
        self.state = PlayerState::Spawning(1);
        self.paused = false;
        true
    }

    pub fn set_lives(&mut self, lives: i32) {
        self.lives = lives;
    }

    /// Debug: seed starting power / score.
    pub fn set_power(&mut self, power: i32) {
        self.world.power = power.clamp(0, 128);
    }
    pub fn set_score(&mut self, score: i64) {
        self.score = score;
    }

    /// Debug warp: true once a boss/midboss enemy is actually on screen. (The
    /// global boss_present flag can read stale when bosses overlap, so the warp
    /// checks the live entities instead.)
    pub fn debug_boss_onscreen(&self) -> bool {
        self.enemies.iter().any(|e| e.occupied && e.is_boss)
    }
    /// The pre-boss dialogue starts the boss music — true only for the real
    /// boss, not the dialogue-less midboss.
    pub fn debug_boss_music_started(&self) -> bool {
        self.boss_bgm_started
    }

    /// Current player x (headless auto-play harness).
    pub fn player_x(&self) -> f32 {
        self.pos[0]
    }

    /// Player centre [x, y] in field coords — for the headless dodging auto-player.
    pub fn player_pos(&self) -> [f32; 2] {
        self.pos
    }

    /// Scalar state for comparison with the unmodified 1.02h executable.
    pub fn reference_state(&self) -> String {
        format!(
            "{},{:.6},{:.6},{},{},{},{},{},{},{},{},{},{},{},{},{}",
            self.stage_num,
            self.pos[0],
            self.pos[1],
            self.lives,
            self.bombs,
            self.world.power,
            self.score,
            self.graze_in_stage,
            self.graze,
            self.world.rank,
            self.world.sub_rank,
            self.world.rng.seed(),
            self.world.bullets.len(),
            self.world.lasers.len(),
            self.items.len(),
            self.player_particle_timer
        )
    }

    pub fn reference_bullets(&self, frame: i32) -> String {
        use std::fmt::Write;
        let mut out = String::new();
        for b in &self.world.bullets {
            let state = if b.despawn_timer > 0 {
                5
            } else if b.spawn_delay > 0 {
                if b.ex_flags & 2 != 0 {
                    2
                } else if b.ex_flags & 4 != 0 {
                    3
                } else {
                    4
                }
            } else {
                1
            };
            writeln!(
                out,
                "{frame},{},{:.9},{:.9},{:.9},{:.9},{:.9},{:.9},{state},{}",
                b.pool_index,
                b.pos[0],
                b.pos[1],
                b.angle,
                b.speed,
                b.velocity[0],
                b.velocity[1],
                b.timer
            )
            .unwrap();
        }
        out
    }

    pub fn reference_effects(&self, frame: i32) -> String {
        self.world.reference_effects(frame)
    }

    /// Remaining bombs — the auto-player bombs to escape when cornered.
    pub fn player_bombs(&self) -> i32 {
        self.bombs
    }

    /// Live item positions [x, y] — the auto-player drifts toward them when safe.
    pub fn items_dump(&self) -> Vec<[f32; 2]> {
        self.items.iter().map(|it| it.pos).collect()
    }

    /// Every live bullet as [pos.x, pos.y, angle, speed] — for the full-VM
    /// oracle diff against the decomp.
    pub fn bullets_dump(&self) -> Vec<[f32; 4]> {
        self.world
            .bullets
            .iter()
            .map(|b| [b.pos[0], b.pos[1], b.angle, b.speed])
            .collect()
    }

    /// Occupied enemy positions [x, y] — oracle diff (isolates movement).
    pub fn enemies_dump(&self) -> Vec<[f32; 2]> {
        self.enemies
            .iter()
            .filter(|e| e.occupied)
            .map(|e| [e.pos[0], e.pos[1]])
            .collect()
    }

    /// X of the boss if present, else the lowest (closest) live enemy — the
    /// headless harness steers the player under this to actually fight.
    pub fn target_x(&self) -> Option<f32> {
        if let Some(b) = self.enemies.iter().find(|e| e.occupied && e.is_boss) {
            return Some(b.pos[0]);
        }
        self.enemies
            .iter()
            .filter(|e| e.occupied && e.interactable)
            .max_by(|a, b| a.pos[1].total_cmp(&b.pos[1]))
            .map(|e| e.pos[0])
    }

    pub fn set_hiscore(&mut self, hiscore: i64) {
        self.hiscore = hiscore;
    }

    pub fn background_scene(&self) -> Option<BgScene> {
        self.background.as_ref().map(|b| {
            let mut sc = b.scene();
            // The 3D bg fills the arcade region, so the shake shifts its origin
            // and shrinks its viewport (arcadeRegionTopLeftPos + arcadeRegionSize).
            sc.arcade = self.arcade;
            sc
        })
    }

    pub fn update(&mut self, input: &Input) -> Vec<DrawCmd> {
        // The pause menu freezes the whole game (no tick, no sim).
        if self.paused {
            self.run_pause_menu(input);
            return self.draw();
        }
        if input.pressed(Key::Pause)
            && matches!(
                self.state,
                PlayerState::Alive | PlayerState::Dead(_) | PlayerState::Spawning(_)
            )
        {
            self.paused = true;
            self.pause_state = 0;
            self.arcade = [FIELD_X, FIELD_Y, FIELD_W, FIELD_H];
            self.run_pause_menu(&Input::default());
            return self.draw();
        }

        self.score_popups.tick();
        self.tick += 1;
        self.anim += 1;
        if !self.world.time_stopped {
            if let Some(bg) = &mut self.background {
                bg.tick();
            }
            self.hud.tick();
        }
        self.roll_gui_score();
        // "Full Power Mode!!" popup when power first reaches max.
        let full = self.world.power >= 128;
        if full && !self.was_full {
            self.full_power_timer = 180;
        }
        self.was_full = full;
        self.full_power_timer = self.full_power_timer.saturating_sub(1);
        self.roll_boss_bar();
        // Track the boss spell timer (for the capture bonus) and tick popups.
        if let Some(secs) = self
            .enemies
            .iter()
            .find(|e| e.is_boss && e.occupied)
            .and_then(|e| e.spell_seconds_left())
        {
            self.spell_secs = secs;
        }
        self.spell_bonus_timer = self.spell_bonus_timer.saturating_sub(1);
        self.bonus_score_timer = self.bonus_score_timer.saturating_sub(1);
        if let Some(banner) = &mut self.bomb_banner {
            for runner in [&mut banner.name, &mut banner.portrait] {
                if let Some(vm) = runner {
                    vm.tick();
                    if !vm.visible() {
                        *runner = None;
                    }
                }
            }
        }
        if let Some(r) = &mut self.spell_name_runner {
            r.tick();
            if !r.visible() {
                self.spell_name_runner = None;
            }
        }
        if let Some(r) = &mut self.stage_name_runner {
            r.tick();
            if !r.visible() {
                self.stage_name_runner = None;
            }
        }
        if let Some(r) = &mut self.portrait_runner {
            r.tick();
            if !r.visible() {
                self.portrait_runner = None;
            }
        }

        if let PlayerState::Cleared(t) = &mut self.state {
            *t -= 1;
            if *t == 0 {
                self.events.push(Event::NextStage);
                return self.draw();
            }
        }
        // Player's chain callback precedes EnemyManager. Time stop freezes the
        // player, its shots and its respawn timers, while ECL keeps running.
        if !self.world.time_stopped && self.update_player_lifecycle(input) {
            return self.draw();
        }
        self.spell_result = self.spell_result.saturating_sub(1);
        self.world.player_pos = self.pos;
        self.world.bomb_active = self.bombing > 0;

        // Only the timeline freezes during dialogue (EnemyManager gates it on
        // !HasCurrentMsgIdx), which keeps the boss's scripted attack from
        // triggering. Bullets, enemies and collision keep running so the
        // player must still dodge whatever is already on screen.
        self.run_timeline();
        self.update_enemies();
        // EffectManager::OnUpdate (chain prio 10, between enemy 9 and bullet 11):
        // draw the RNG for effects spawned this frame (enemy deaths, etc.). (#26)
        self.world.update_effects();
        // ECL cancels patterns and spawns items before BulletManager and
        // ItemManager run. Drain here so cancellation cannot hit the player
        // or erase a replacement pattern fired later in the same ECL frame.
        self.drain_world_events();
        if !self.world.time_stopped {
            self.update_items();
        }
        self.update_bullets();
        self.drain_world_events();
        // Gui's MSG interpreter runs after enemy/bullet/item update. An
        // ECLRESUME therefore releases the next frame's timeline MSGWAIT.
        if self.dialogue.active && !self.world.time_stopped {
            self.run_dialogue(input);
        }

        // Stage clear: timeline exhausted and field empty.
        if matches!(self.state, PlayerState::Alive)
            && (self.msg.offsets.is_empty() || std::env::var_os("TH06_NO_DIALOGUE").is_some())
            && self.timeline_done()
            && !self.enemies.iter().any(|e| e.occupied)
            && !self.world.boss_present
        {
            let bonus = stage_clear_bonus(
                self.stage_num,
                self.graze_in_stage,
                self.world.power as i64,
                self.point_items,
                self.lives.max(0) as i64,
                self.bombs.max(0) as i64,
                self.world.difficulty,
                self.starting_lives,
            );
            self.score += bonus;
            self.clear_bonus = bonus;
            if self.score > self.hiscore {
                self.hiscore = self.score;
            }
            self.events.push(Event::SaveScore(self.hiscore));
            self.state = PlayerState::Cleared(420);
        }

        // ScreenEffect calc chain (screen shake) runs after the gameplay calc.
        self.update_shake();
        self.draw()
    }

    fn run_pause_menu(&mut self, input: &Input) {
        if input.pressed(Key::Pause) {
            self.pause_state = 3;
            self.pause_timer = 0;
            for runner in self.pause_runners.iter_mut().flatten() {
                if runner.visible() {
                    runner.interrupt(2);
                }
            }
        }
        if self.pause_state == 0 {
            self.pause_runners = std::array::from_fn(|idx| {
                self.pause_asset
                    .as_ref()
                    .and_then(|asset| {
                        asset
                            .entry
                            .scripts
                            .iter()
                            .find(|(id, _)| *id == idx as u32 + 2)
                    })
                    .map(|(_, script)| AnmRunner::new(script.clone()))
            });
            self.pause_interrupt(0..3, 1);
            self.pause_state = 1;
            self.pause_timer = 0;
        }
        let active = match self.pause_state {
            1 => Some(1),
            2 => Some(2),
            4 => Some(4),
            5 => Some(5),
            _ => None,
        };
        if let Some(active) = active {
            let options = if active < 3 { 1..3 } else { 4..6 };
            for idx in options {
                if let Some(runner) = &mut self.pause_runners[idx] {
                    let selected = idx == active;
                    runner.color = if selected {
                        [1.0, 128.0 / 255.0, 128.0 / 255.0]
                    } else {
                        [128.0 / 255.0; 3]
                    };
                    runner.alpha = if selected { 1.0 } else { 128.0 / 255.0 };
                    runner.scale = if selected { [1.7; 2] } else { [1.5; 2] };
                    runner.pos_offset = if selected { [-4.0; 2] } else { [0.0; 2] };
                }
            }
            if self.pause_timer >= 4 {
                if input.pressed(Key::Up) || input.pressed(Key::Down) {
                    self.pause_state = match self.pause_state {
                        1 => 2,
                        2 => 1,
                        4 => 5,
                        _ => 4,
                    };
                }
                if input.pressed(Key::Shoot) || input.pressed(Key::Enter) {
                    // The switch executes the originally selected state's
                    // branch even if a direction was pressed simultaneously.
                    match active {
                        1 => {
                            self.pause_interrupt(0..3, 2);
                            self.pause_state = 3;
                        }
                        2 => {
                            self.pause_interrupt(0..3, 2);
                            self.pause_interrupt(3..6, 1);
                            self.pause_state = 5;
                        }
                        4 => {
                            self.pause_interrupt(3..6, 2);
                            self.pause_state = 6;
                        }
                        _ => {
                            self.pause_interrupt(0..3, 1);
                            self.pause_interrupt(3..6, 2);
                            self.pause_state = 2;
                        }
                    }
                    self.pause_timer = 0;
                }
            }
        } else if matches!(self.pause_state, 3 | 6) && self.pause_timer >= 20 {
            if self.pause_state == 6 {
                self.events.push(Event::BackToTitle);
            }
            self.paused = false;
            self.pause_state = 0;
            self.pause_runners = std::array::from_fn(|_| None);
        }
        for runner in self.pause_runners.iter_mut().flatten() {
            runner.tick();
        }
        self.pause_timer = self.pause_timer.saturating_add(1);
    }

    fn pause_interrupt(&mut self, range: std::ops::Range<usize>, label: u32) {
        for runner in self.pause_runners[range].iter_mut().flatten() {
            runner.interrupt(label);
        }
    }

    fn timeline_done(&self) -> bool {
        self.ecl
            .timeline_at(self.timeline_off)
            .map(|t| t.time < 0)
            .unwrap_or(true)
    }

    /// Port of EnemyManager::RunEclTimeline.
    fn run_timeline(&mut self) {
        // Survival subrank tick (EnemyManager.cpp:161-172): every
        // (2400 - lives*240) frames of timeline time, +100 subrank (= +1 rank).
        // The zeroed original timer has not ticked at stage entry. Subsequent
        // checks observe the current timeline time before this callback's Tick.
        let interval = 2400 - self.lives * 240;
        if !self.dialogue.active
            && interval > 0
            && self.timeline_time > 0
            && self.timeline_time % interval == 0
        {
            self.world.increase_subrank(100);
        }
        loop {
            // Copy the instruction out so the borrow of the ECL data ends
            // before any spawning mutates `self`.
            struct T {
                time: i32,
                arg0: i32,
                opcode: i16,
                size: u32,
                pos: [f32; 3],
                life: i16,
                item: i16,
                score: i32,
                a0: i32,
                a1: i32,
            }
            let t = {
                let Some(raw) = self.ecl.timeline_at(self.timeline_off) else {
                    self.timeline_time += 1;
                    return;
                };
                let has_full = raw.args.len() >= 20;
                T {
                    time: raw.time as i32,
                    arg0: raw.arg0 as i32,
                    opcode: raw.opcode,
                    size: raw.size as u32,
                    pos: if raw.args.len() >= 12 {
                        [raw.arg_f32(0), raw.arg_f32(4), raw.arg_f32(8)]
                    } else {
                        [0.0; 3]
                    },
                    life: if has_full { raw.arg_u16(12) as i16 } else { -1 },
                    item: if has_full { raw.arg_u16(14) as i16 } else { -2 },
                    score: if has_full { raw.arg_i32(16) } else { -1 },
                    a0: if raw.args.len() >= 4 {
                        raw.arg_i32(0)
                    } else {
                        0
                    },
                    a1: if raw.args.len() >= 8 {
                        raw.arg_i32(4)
                    } else {
                        0
                    },
                }
            };
            if t.time < 0 {
                self.timeline_time += 1;
                return;
            }
            if self.timeline_time < t.time {
                break;
            }
            if self.timeline_time == t.time {
                match t.opcode {
                    0..=7 => {
                        if !self.world.boss_present {
                            let mut pos = t.pos;
                            if t.opcode >= 4 {
                                if pos[0] <= -990.0 {
                                    pos[0] = self.world.rng.f32_in_range(368.0);
                                }
                                if pos[1] <= -990.0 {
                                    pos[1] = self.world.rng.f32_in_range(416.0);
                                }
                                if pos[2] <= -990.0 {
                                    pos[2] = self.world.rng.f32_in_range(800.0);
                                }
                            }
                            let with_args = t.opcode % 2 == 0;
                            let (life, item, score) = if with_args {
                                (t.life, t.item, t.score)
                            } else {
                                (-1, -1, -1) // ITEM_RANDOM_ITEM
                            };
                            let mirror = matches!(t.opcode, 2 | 3 | 6 | 7);
                            self.spawn(SpawnReq { sub: t.arg0, pos, life, item, score, mirror });
                        }
                    }
                    8 => {
                        // Easy stage 5 uses the alternate ending dialogue;
                        // Marisa's MSG scripts start at offset index 10.
                        let msg = if self.world.difficulty == Difficulty::Easy as u8
                            && self.stage_num == 5 && t.arg0 == 1 {
                            3
                        } else {
                            t.arg0
                        };
                        self.start_dialogue((msg + self.world.character as i32 * 10) as usize);
                    }
                    9 => {
                        // MsgWait: hold the timeline while dialogue runs.
                        if self.dialogue.active && !self.dialogue.ecl_resumed {
                            return; // do not advance time
                        }
                    }
                    10 => {
                        let interrupt = t.a1;
                        let _ = t.a0;
                        for e in &mut self.enemies {
                            if e.is_boss {
                                e.fire_interrupt(interrupt);
                            }
                        }
                    }
                    11 => self.world.power = t.arg0, // set power
                    12
                        // Wait for the boss slot to clear.
                        if self.enemies.iter().any(|e| e.is_boss && e.occupied) => {
                            return; // do not advance time
                        }
                    _ => {}
                }
            }
            self.timeline_off += t.size;
        }
        self.timeline_time += 1;
    }

    fn spawn(&mut self, req: SpawnReq) {
        self.sync_enemy_slots();
        if self.enemies.len() >= 256 && self.enemies.iter().all(|enemy| enemy.occupied) {
            return;
        }
        if std::env::var_os("TH06_TRACE").is_some() {
            eprintln!(
                "[{}] spawn sub={} pos=({:.0},{:.0}) life={}",
                self.timeline_time, req.sub, req.pos[0], req.pos[1], req.life
            );
        }
        if let Some(e) = Enemy::spawn(&self.ecl, &mut self.world, &req) {
            self.insert_enemy(e);
        }
        self.flush_spawns();
    }

    fn sync_enemy_slots(&mut self) {
        self.world.enemy_slots.fill(false);
        for (index, enemy) in self.enemies.iter().enumerate() {
            self.world.enemy_slots[index] = enemy.occupied;
        }
    }

    /// Children reserve their original fixed slot before their initialization
    /// ECL runs, including nested ENEMYCREATE. Publish each initialized instance
    /// at that reserved index rather than allocating again after RunEcl.
    fn insert_enemy(&mut self, enemy: Enemy) {
        let index = enemy.pool_index;
        while self.enemies.len() <= index {
            self.enemies.push(Enemy::vacant());
            self.anims.push(None);
            self.sub_anims.push(Default::default());
        }
        self.enemies[index] = enemy;
        self.anims[index] = None;
        self.sub_anims[index] = Default::default();
    }

    fn flush_spawns(&mut self) {
        for enemy in std::mem::take(&mut self.world.pending_spawns) {
            self.insert_enemy(enemy);
        }
    }

    /// ENEMYKILLALL (op 96) / boss-timer / life callbacks set kill_trash. The
    /// decomp applies it INLINE the instant it's set — HandleTimerCallback
    /// (EnemyManager.cpp:418) before the killer's RunEcl, the ECL opcode during
    /// it — setting life=0 on every non-boss enemy using its CURRENT isBoss flag.
    /// Enemies later in the slot order that are still bosses this frame are
    /// skipped, then their own ECL runs (e.g. BOSSSET(-1) + ENEMYLIFESET) and can
    /// revive — which is how Sakuya's dolls survive the midboss timeout. Applying
    /// it post-loop would kill an already-un-bossed doll.
    fn apply_kill_trash(&mut self) {
        if !self.world.kill_trash {
            return;
        }
        self.world.kill_trash = false;
        for j in 0..self.enemies.len() {
            if self.enemies[j].occupied && !self.enemies[j].is_boss {
                self.enemies[j].kill_as_trash(&self.ecl);
            }
        }
    }

    fn update_enemies(&mut self) {
        // Dynamic bound (the decomp loops over a fixed enemies[256]): a child
        // created mid-loop by ENEMYCREATE lands in a free slot and, if that slot
        // is ahead of the parent, must be processed THIS frame too — so we re-read
        // the length each iteration rather than freezing it at loop start.
        let mut i = 0;
        while i < self.enemies.len() {
            if !self.enemies[i].occupied {
                i += 1;
                continue;
            }
            self.enemies[i].frame_move();
            // Current primaryVm sprite size (px) for the off-screen despawn test
            // (EnemyManager.cpp:549). (0,0) until the first ANMSETMAIN runs.
            let (sw, sh) = {
                let anm_script = self.enemies[i].anm_script;
                self.anims
                    .get(i)
                    .and_then(|a| a.as_ref())
                    .and_then(|r| r.sprite)
                    .and_then(|idx| {
                        self.enemy_scripts
                            .get(&anm_script)
                            .and_then(|s| s.sprites.get(&idx))
                    })
                    .map(|sp| (sp.width, sp.height))
                    .unwrap_or((0.0, 0.0))
            };
            if !self.enemies[i].update_bounds(sw, sh, [self.arcade[2], self.arcade[3]]) {
                self.enemies[i].despawn(&mut self.world);
                i += 1;
                continue;
            }
            self.sync_enemy_slots();
            self.enemies[i].handle_callbacks(&self.ecl, &mut self.world);
            self.apply_kill_trash(); // timeout/life-callback kill, before this enemy's RunEcl
            self.enemies[i].run_ecl(&self.ecl, &mut self.world);
            self.enemies[i].clamp(); // ClampPos again after RunEcl (EnemyManager.cpp:571)
            self.apply_kill_trash(); // ENEMYKILLALL during RunEcl
            // Place ENEMYCREATE children into free slots immediately, as the
            // decomp's SpawnEnemy does mid-loop (EnemyManager.cpp:856/110): a
            // child in a slot ahead of its parent then gets its own RunEcl this
            // same frame via the loop above, keeping its ECL clock in step.
            self.flush_spawns();
            if !self.enemies[i].occupied {
                i += 1;
                continue;
            }

            // Refresh the ANM runner when the script changed.
            if self.enemies[i].anm_dirty {
                self.enemies[i].anm_dirty = false;
                let anm_script = self.enemies[i].anm_script;
                self.anims[i] = self
                    .enemy_scripts
                    .get(&anm_script)
                    .map(|s| AnmRunner::new(s.instrs.clone()));
            }
            // ECL op128 ANMINTERRUPTMAIN: jump the primary runner to its queued
            // interrupt label before the tick (primaryVm.pendingInterrupt).
            if let Some(n) = self.enemies[i].pending_interrupt.take()
                && n >= 0
                && let Some(anim) = &mut self.anims[i]
            {
                anim.interrupt(n as u32);
            }
            if let Some(anim) = &mut self.anims[i] {
                anim.tick();
            }
            // SetRandomSprite (op16): resolve the deferred RNG draw here, right
            // after ExecuteScript, so it lands at the decomp's point in the
            // shared RNG stream (EnemyManager.cpp:573).
            let pend = self.anims[i].as_mut().and_then(|a| a.take_pending_random());
            if let Some((base, range)) = pend {
                let v = self.world.rng.u16_in_range(range) as u32;
                if let Some(a) = &mut self.anims[i] {
                    a.sprite = Some(base + v);
                }
            }
            // Sub-vm runners (op99 ANMSETSLOT): (re)build dirtied slots, then tick.
            for s in 0..8 {
                if self.enemies[i].sub_dirty[s] {
                    self.enemies[i].sub_dirty[s] = false;
                    let script = self.enemies[i].sub_scripts[s];
                    self.sub_anims[i][s] = self
                        .enemy_scripts
                        .get(&script)
                        .map(|sc| AnmRunner::new(sc.instrs.clone()));
                }
                if let Some(sv) = &mut self.sub_anims[i][s] {
                    if let Some(interrupt) = self.enemies[i].sub_interrupts[s].take()
                        && interrupt != 0
                    {
                        sv.interrupt(interrupt as u32);
                    }
                    sv.tick();
                }
                let pend = self.sub_anims[i][s]
                    .as_mut()
                    .and_then(|a| a.take_pending_random());
                if let Some((base, range)) = pend {
                    let v = self.world.rng.u16_in_range(range) as u32;
                    if let Some(a) = &mut self.sub_anims[i][s] {
                        a.sprite = Some(base + v);
                    }
                }
            }
            self.collide_enemy(i);
            // Enemy::UpdateEffects and phaseTimer follow this enemy's damage
            // and death handling, before the next fixed slot runs its ECL.
            self.enemies[i].update_effects(&mut self.world);
            if !self.world.time_stopped {
                self.enemies[i].tick_boss_timer();
            }
            i += 1;
        }
        self.flush_spawns();
    }

    fn update_player(&mut self, input: &Input) {
        let focus = input.held(Key::Focus);
        self.last_input_focus = focus;
        // Per-character movement speed (g_CharData, Player.cpp:27): Reimu 4.0/2.0,
        // Marisa 5.0/2.5 (ortho / focus). Diagonal is ortho/sqrt(2), applied below.
        let (ortho, focus_speed) = if self.character.is_marisa() {
            (5.0, 2.5)
        } else {
            (4.0, 2.0)
        };
        // MarisaB's Master Spark slows the player to 0.3 while bombing
        // (verticalMovementSpeedMultiplierDuringBomb); other bombs leave it 1.0.
        let mut speed = if focus { focus_speed } else { ortho };
        // HandlePlayerInputs resolves opposing directions with Up and Right
        // taking precedence, including the diagonal cases.
        let d = [
            if input.held(Key::Right) {
                1.0
            } else if input.held(Key::Left) {
                -1.0
            } else {
                0.0
            },
            if input.held(Key::Up) {
                -1.0
            } else if input.held(Key::Down) {
                1.0
            } else {
                0.0
            },
        ];
        if d[0] != 0.0 && d[1] != 0.0 {
            speed /= 2.0f32.sqrt();
        }
        self.pos[0] = (self.pos[0] + d[0] * speed * self.bomb_move_mult).clamp(8.0, FIELD_W - 8.0);
        self.pos[1] =
            (self.pos[1] + d[1] * speed * self.bomb_move_mult).clamp(16.0, FIELD_H - 16.0);

        // Banking animation (HandlePlayerInputs): switch player ANM script on
        // the horizontal-speed sign transitions. Stopping scripts loop back
        // into the idle frames on their own.
        let hspeed = d[0] * speed;
        let prev = self.prev_hspeed;
        if hspeed < 0.0 && prev >= 0.0 {
            self.set_player_script(1); // MOVING_LEFT
        } else if hspeed == 0.0 && prev < 0.0 {
            self.set_player_script(2); // STOPPING_LEFT
        }
        if hspeed > 0.0 && prev <= 0.0 {
            self.set_player_script(3); // MOVING_RIGHT
        } else if hspeed == 0.0 && prev > 0.0 {
            self.set_player_script(4); // STOPPING_RIGHT
        }
        self.prev_hspeed = hspeed;

        self.update_orbs(focus);
    }

    /// HandlePlayerInputs' five-state orb transition. Horizontal movement uses
    /// squared easing; reversing focus also executes the opposite branch in
    /// this callback, including its extra timer increment.
    fn update_orbs(&mut self, focus: bool) {
        if self.world.power < 8 {
            self.orb_state = OrbState::Hidden;
        } else if self.orb_state == OrbState::Hidden {
            self.orb_state = OrbState::Unfocused;
        }
        self.orb_offsets = [0.0; 2];
        match self.orb_state {
            OrbState::Hidden => self.orb_timer = 0,
            OrbState::Unfocused => {
                self.orb_timer = 0;
                self.orb_offsets = [24.0, 0.0];
                if focus {
                    self.orb_state = OrbState::Focusing;
                    self.step_orb_transition(focus);
                }
            }
            OrbState::Focused => {
                self.orb_timer = 0;
                self.orb_offsets = [8.0, -32.0];
                if !focus {
                    self.orb_state = OrbState::Unfocusing;
                    self.step_orb_transition(focus);
                }
            }
            OrbState::Focusing | OrbState::Unfocusing => self.step_orb_transition(focus),
        }
        let [horizontal, vertical] = self.orb_offsets;
        self.orbs_pos = [
            [self.pos[0] - horizontal, self.pos[1] + vertical],
            [self.pos[0] + horizontal, self.pos[1] + vertical],
        ];
    }

    fn step_orb_transition(&mut self, focus: bool) {
        loop {
            self.orb_timer += 1;
            let mut fraction = self.orb_timer as f32 / 8.0;
            let focusing = self.orb_state == OrbState::Focusing;
            let y = if focusing {
                (1.0 - fraction) * 32.0 - 32.0
            } else {
                32.0 * fraction - 32.0
            };
            fraction *= fraction;
            if !focusing {
                fraction = 1.0 - fraction;
            }
            self.orb_offsets = [-16.0 * fraction + 24.0, y];
            if self.orb_timer >= 8 {
                self.orb_state = if focusing {
                    OrbState::Focused
                } else {
                    OrbState::Unfocused
                };
            }
            if focusing == focus {
                break;
            }
            self.orb_state = if focus {
                OrbState::Focusing
            } else {
                OrbState::Unfocusing
            };
            self.orb_timer = 8 - self.orb_timer;
        }
    }

    fn update_player_fire(&mut self, input: &Input) {
        // Shooting and bombing are disabled during dialogue (Player.cpp gates
        // both on !HasCurrentMsgIdx), but the player can still move and dodge.
        let can_act = !self.dialogue.active && matches!(self.state, PlayerState::Alive);

        // Player.cpp StartFireBulletTimer / UpdateFireBulletsTimer: holding
        // Shoot (re)starts the idle fire timer, which then counts 0..=29 and
        // spawns the matching table entries each frame before resetting.
        if can_act && input.held(Key::Shoot) && self.fire_timer < 0 {
            self.fire_timer = 0;
        }
        if self.fire_timer >= 0 {
            if !self.bomb_in_use || self.character != Character::MarisaB {
                self.spawn_player_bullets(self.fire_timer);
            }
            self.fire_timer += 1;
            if self.fire_timer >= 30 || !matches!(self.state, PlayerState::Alive) {
                self.fire_timer = -1;
            }
        }
    }

    /// Player_OnUpdate: bomb handling happens before the death/spawn state
    /// machine, so a bomb in the six-frame respawnTimer window saves the stock
    /// while Player::Die has already counted the miss and failed the capture.
    fn update_player_lifecycle(&mut self, input: &Input) -> bool {
        for rect in &mut self.bomb_projectiles {
            rect.size[0] = 0.0;
        }
        for rect in &mut self.bomb_regions {
            rect.size[0] = 0.0;
        }
        if self.bomb_in_use {
            self.update_bomb();
        } else if !self.dialogue.active
            && (matches!(self.state, PlayerState::Alive) || self.dying > 0)
            && self.bombs > 0
            && input.pressed(Key::Bomb)
        {
            self.fire_bomb();
        }
        if let PlayerState::Dead(elapsed) = self.state {
            if self.dying > 0 {
                self.dying -= 1;
                if self.dying == 0 {
                    self.commit_death();
                }
                self.state = PlayerState::Dead(elapsed + 1);
            } else if elapsed >= 30 {
                self.pos = [self.arcade[2] / 2.0, self.arcade[3] - 64.0];
                self.invuln = 0;
                self.set_player_script(0);
                if self.lives <= 0 {
                    self.state = PlayerState::GameOver;
                    self.events.push(Event::GameOver(self.score));
                    return true;
                }
                self.lives -= 1;
                self.bombs = self.starting_bombs;
                self.state = PlayerState::Spawning(0);
            } else {
                self.state = PlayerState::Dead(elapsed + 1);
            }
        }
        if let PlayerState::Spawning(elapsed) = self.state {
            self.bullet_grace_period = 90;
            self.bomb_move_mult = 1.0;
            if elapsed >= 30 {
                self.state = PlayerState::Alive;
                self.invuln = 240;
            } else {
                self.state = PlayerState::Spawning(elapsed + 1);
            }
        }
        if self.bullet_grace_period > 0 {
            self.bullet_grace_period -= 1;
            self.world.despawn_all_bullets();
        }
        if matches!(self.state, PlayerState::Alive) {
            self.invuln = self.invuln.saturating_sub(1);
            self.update_player(input);
        }
        self.player_runner.tick();
        self.update_shots();
        if self.orb_state != OrbState::Hidden {
            for orb in &mut self.orb_runners {
                orb.tick();
            }
        }
        self.last_enemy_hit = None;
        self.update_player_fire(input);
        false
    }

    /// Fire a bomb (Player::OnUpdate bomb branch): mark it in use, reset the
    /// timer, magnet uncollected items (ItemManager::RemoveAllItems), then run the
    /// calc once for frame 0. Bullets are no longer cleared wholesale — the
    /// per-bomb `bombProjectiles` cancel them region-by-region in `collide`.
    fn fire_bomb(&mut self) {
        self.bombs_used += 1;
        if self.dying > 0 {
            self.state = PlayerState::Alive;
        }
        self.dying = 0; // a bomb in the deathbomb window cancels the stock loss
        self.bombs -= 1;
        self.world.decrease_subrank(200); // Player.cpp:185: bombing drops rank ~2
        self.spell_capturing = false; // bombing forfeits the capture
        self.spell_used_bomb = self.spell_active; // spellcardInfo.usedBomb
        for item in &mut self.items {
            item.state = 1; // ItemManager::MagnetAllItems
        }
        self.bomb_kind = match self.character {
            Character::ReimuA => 0,
            Character::ReimuB => 1,
            Character::MarisaA => 2,
            Character::MarisaB => 3,
        };
        self.bomb_in_use = true;
        self.bomb_timer = 0;
        self.bomb_duration = 999; // each calc overwrites this on its frame 0
        self.bomb_dmg_accum = [0; 32]; // Player::unk_838, reset per bomb
        for s in &mut self.bomb_sprites {
            *s = None;
        }
        self.show_bomb_banner();
        self.update_bomb(); // run the calc for frame 0
    }

    /// End the active bomb (Gui::EndPlayerSpellcard + isInUse = 0); also restores
    /// the movement multiplier MarisaB's Master Spark lowered.
    fn end_bomb(&mut self) {
        self.bomb_in_use = false;
        self.bomb_move_mult = 1.0;
        self.end_bomb_banner();
    }

    /// Per-frame bomb update: the per-frame rect clear already ran in `update`,
    /// so each calc only repopulates its active rects. Dispatches to the
    /// character's BombData calc, then refreshes the `bombing` gate.
    fn update_bomb(&mut self) {
        match self.bomb_kind {
            0 => self.bomb_reimu_a(),
            1 => self.bomb_reimu_b(),
            2 => self.bomb_marisa_a(),
            _ => self.bomb_marisa_b(),
        }
        // Active-state gate: tracks isInUse exactly (no off-by-one at the last
        // processed frame, where the timer has already reached the duration).
        self.bombing = self.bomb_in_use as u32;
    }

    /// ScreenEffect::RegisterChain allocates a new equal-priority calc chain.
    /// Global.cpp inserts it after older chains, so the last processed chain
    /// determines the viewport while every live chain still advances the RNG.
    fn register_shake(&mut self, len: i32, p1: f32, p2: f32) {
        self.shakes.push(ScreenShake {
            timer: 0,
            length: len,
            start: p1,
            end: p2,
        });
    }

    /// ScreenEffect::ShakeScreen. Each chain writes a complete region, including
    /// a baseline reset when it expires, even if an older chain is still active.
    fn update_shake(&mut self) {
        self.arcade = [FIELD_X, FIELD_Y, FIELD_W, FIELD_H];
        if self.world.time_stopped {
            return;
        }
        for shake in &mut self.shakes {
            self.arcade = [FIELD_X, FIELD_Y, FIELD_W, FIELD_H];
            shake.timer += 1;
            if shake.timer >= shake.length {
                continue;
            }
            let off = (shake.timer as f32 * (shake.end - shake.start)) / shake.length as f32
                + shake.start;
            match self.world.rng.u32_in_range(3) {
                1 => {
                    self.arcade[0] = FIELD_X + off;
                    self.arcade[2] = FIELD_W - off;
                }
                2 => self.arcade[2] = FIELD_W - off,
                _ => {}
            }
            match self.world.rng.u32_in_range(3) {
                1 => {
                    self.arcade[1] = FIELD_Y + off;
                    self.arcade[3] = FIELD_H - off;
                }
                2 => self.arcade[3] = FIELD_H - off,
                _ => {}
            }
        }
        self.shakes.retain(|shake| shake.timer < shake.length);
    }

    /// BombData::BombReimuACalc — Fantasy Seal. A one-frame 256² central blast
    /// (`bombProjectiles[8]`) at the cast, then seals 1..=7 spawn over frames
    /// 80..=176 (seal 0 is never spawned — the decomp's `&& (i = …)` short-
    /// circuits on i==0, a ZUN quirk), each homing toward the lowest on-screen
    /// enemy and exploding into a 256²/200-damage burst once it has dealt 100
    /// damage or the bomb is in its last 30 frames.
    #[allow(clippy::manual_clamp)] // Original ordered comparisons preserve NaN behavior.
    fn bomb_reimu_a(&mut self) {
        use std::f32::consts::{PI, TAU};
        if self.bomb_timer >= self.bomb_duration {
            self.end_bomb();
            return;
        }
        if self.bomb_timer == 0 {
            self.bomb_duration = 300;
            self.invuln = 360;
            self.seal_state = [0; 8];
            self.world.spawn_particles(
                12,
                [self.pos[0], self.pos[1], 0.0],
                1,
                [64.0 / 255.0, 64.0 / 255.0, 1.0, 1.0],
            );
            // 256² central blast; cleared next frame, so it cancels bullets once.
            self.bomb_projectiles[8] = BombRect {
                pos: self.pos,
                size: [256.0, 256.0],
            };
        }
        if (60..180).contains(&self.bomb_timer) && self.bomb_timer % 16 == 0 {
            let i = ((self.bomb_timer - 60) / 16) as usize;
            if i != 0 {
                self.seal_state[i] = 1;
                self.seal_speed[i] = 4.0;
                self.seal_pos[i] = self.pos;
                let angle = self.world.rng.f32_zero_to_one() * TAU - PI;
                self.seal_vel[i] = [
                    angle.cos() * self.seal_speed[i],
                    angle.sin() * self.seal_speed[i],
                ];
                self.bomb_dmg_accum[i] = 0;
                // ExecuteAnmIdx the seal's 4 sprites (REIMU_A_BOMB_ARRAY = 133+k).
                for k in 0..4 {
                    self.bomb_sprites[i * 4 + k] = Some(AnmRunner::new(
                        self.player_scripts[&(133 + k as i32)].clone(),
                    ));
                }
                self.events.push(Event::SfxIdx(13)); // SOUND_BOMB_REIMU_A
            }
        }
        // positionOfLastEnemyHit: the lowest on-screen enemy, else the player.
        let pivot = self
            .last_enemy_hit
            .filter(|pos| pos[0] > -100.0)
            .unwrap_or(self.pos);
        for i in 0..8 {
            if self.seal_state[i] == 0 {
                continue;
            }
            if self.seal_state[i] == 1 {
                let mut ax = pivot[0] - self.seal_pos[i][0];
                let mut ay = pivot[1] - self.seal_pos[i][1];
                let mut len = (ax * ax + ay * ay).sqrt() / (self.seal_speed[i] / 8.0);
                if len < 1.0 {
                    len = 1.0;
                }
                ax = ax / len + self.seal_vel[i][0];
                ay = ay / len + self.seal_vel[i][1];
                len = (ax * ax + ay * ay).sqrt();
                self.seal_speed[i] = if len > 10.0 { 10.0 } else { len };
                if self.seal_speed[i] < 1.0 {
                    self.seal_speed[i] = 1.0;
                }
                self.seal_vel[i] = [ax * self.seal_speed[i] / len, ay * self.seal_speed[i] / len];

                self.bomb_regions[i] = BombRect {
                    pos: self.seal_pos[i],
                    size: [48.0, 48.0],
                };
                self.bomb_region_dmg[i] = 8;
                self.bomb_projectiles[i] = BombRect {
                    pos: self.seal_pos[i],
                    size: [48.0, 48.0],
                };

                if self.bomb_dmg_accum[i] >= 100 || self.bomb_timer >= self.bomb_duration - 30 {
                    let burst_pos = [self.seal_pos[i][0], self.seal_pos[i][1], 0.0];
                    self.world.spawn_particles(6, burst_pos, 8, [1.0; 4]);
                    self.world.spawn_particles(
                        12,
                        burst_pos,
                        1,
                        [64.0 / 255.0, 64.0 / 255.0, 1.0, 1.0],
                    );
                    self.seal_state[i] = 2;
                    self.bomb_regions[i].size = [256.0, 256.0];
                    self.bomb_region_dmg[i] = 200;
                    self.bomb_projectiles[i].size = [256.0, 256.0];
                    // pendingInterrupt = 1: jump the 4 seal sprites to the burst.
                    for k in 0..4 {
                        if let Some(r) = &mut self.bomb_sprites[i * 4 + k] {
                            r.interrupt(1);
                        }
                    }
                    self.events.push(Event::SfxIdx(15)); // SOUND_F
                    self.register_shake(16, 8.0, 0.0); // ScreenEffect SHAKE (16,8,0)
                }
            } else {
                self.seal_state[i] += 1;
                if self.seal_state[i] >= 30 {
                    self.seal_state[i] = 0;
                }
            }
            self.seal_pos[i][0] += self.seal_vel[i][0];
            self.seal_pos[i][1] += self.seal_vel[i][1];
            for k in 0..4 {
                if let Some(r) = &mut self.bomb_sprites[i * 4 + k] {
                    r.tick(); // ExecuteScript
                }
            }
        }
        self.bomb_timer += 1;
    }

    /// BombData::BombReimuBCalc — Dream Sign "Evil Sealing Circle". Two vertical
    /// (62×448) and two horizontal (384×62) beams cross at the player; their
    /// bullet-cancel rects stay live every frame, but they only damage enemies
    /// on odd frames (`timer % 2`).
    fn bomb_reimu_b(&mut self) {
        if self.bomb_timer >= self.bomb_duration {
            self.end_bomb();
            return;
        }
        if self.bomb_timer == 0 {
            self.bomb_duration = 140;
            self.invuln = 200;
            self.events.push(Event::SfxIdx(6)); // SOUND_BOMB_REIMARI
            // ExecuteAnmIdx the 4 beam sprites (REIMU_B_BOMB_ARRAY = scripts 137+i);
            // their animated pos becomes each beam's posOffset.
            for i in 0..4 {
                self.bomb_sprites[i] = Some(AnmRunner::new(
                    self.player_scripts[&(137 + i as i32)].clone(),
                ));
            }
            // bombRegionPositions seeds (z layering is draw-only, omitted).
            self.seal_pos[0] = [self.pos[0], 224.0];
            self.seal_pos[1] = [192.0, self.pos[1]];
            self.seal_pos[2] = [self.pos[0], 224.0];
            self.seal_pos[3] = [192.0, self.pos[1]];
            self.register_shake(60, 2.0, 6.0); // ScreenEffect SHAKE (60,2,6)
        } else {
            if self.bomb_timer == 60 {
                self.register_shake(80, 20.0, 0.0); // ScreenEffect SHAKE (80,20,0)
            }
            self.bomb_projectiles[0].size = [62.0, 448.0];
            self.bomb_projectiles[1].size = [384.0, 62.0];
            self.bomb_projectiles[2].size = [62.0, 448.0];
            self.bomb_projectiles[3].size = [384.0, 62.0];
            for i in 0..4 {
                let off = if let Some(r) = self.bomb_sprites[i].as_mut() {
                    r.tick(); // ExecuteScript
                    r.pos_offset
                } else {
                    [0.0, 0.0]
                };
                if self.bomb_timer % 2 != 0 {
                    let pos = [self.seal_pos[i][0] + off[0], self.seal_pos[i][1] + off[1]];
                    self.bomb_projectiles[i].pos = pos;
                    self.bomb_regions[i] = BombRect {
                        pos,
                        size: self.bomb_projectiles[i].size,
                    };
                    self.bomb_region_dmg[i] = 8;
                }
            }
        }
        self.bomb_timer += 1;
    }

    /// BombData::BombMarisaACalc — Stardust Reverie. Eight 128² stars fan out
    /// from the player at speed 2, cancelling bullets and dealing 8 on two of
    /// every three frames (`timer % 3`).
    fn bomb_marisa_a(&mut self) {
        use std::f32::consts::TAU;
        if self.bomb_timer >= self.bomb_duration {
            self.end_bomb();
            return;
        }
        if self.bomb_timer == 0 {
            self.bomb_duration = 250;
            self.invuln = 300;
            for i in 0..8 {
                // ExecuteAnmIdx the star sprite (BLUE/GREEN/RED = 5 + i%3).
                self.bomb_sprites[i] = Some(AnmRunner::new(
                    self.player_scripts[&(5 + (i % 3) as i32)].clone(),
                ));
                self.seal_pos[i] = self.pos;
                let a = i as f32 * TAU / 8.0;
                self.seal_vel[i] = [a.cos() * 2.0, a.sin() * 2.0];
            }
            self.events.push(Event::SfxIdx(6)); // SOUND_BOMB_REIMARI
            self.register_shake(120, 4.0, 1.0); // ScreenEffect SHAKE (120,4,1)
        } else {
            for i in 0..8 {
                self.seal_pos[i][0] += self.seal_vel[i][0];
                self.seal_pos[i][1] += self.seal_vel[i][1];
                if self.bomb_timer % 3 != 0 {
                    self.bomb_projectiles[i] = BombRect {
                        pos: self.seal_pos[i],
                        size: [128.0, 128.0],
                    };
                    self.bomb_regions[i] = BombRect {
                        pos: self.seal_pos[i],
                        size: [128.0, 128.0],
                    };
                    self.bomb_region_dmg[i] = 8;
                }
                if let Some(r) = self.bomb_sprites[i].as_mut() {
                    r.tick(); // ExecuteScript
                }
            }
        }
        self.bomb_timer += 1;
    }

    /// BombData::BombMarisaBCalc — Master Spark. A 384-wide column from the top
    /// edge down to the player (height = player y), 12 damage on three of every
    /// four frames (`timer % 4`); the player crawls at 0.3 speed while firing.
    fn bomb_marisa_b(&mut self) {
        if self.bomb_timer >= self.bomb_duration {
            self.end_bomb();
            return;
        }
        if self.bomb_timer == 0 {
            self.bomb_duration = 300;
            self.invuln = 360;
            for i in 0..4 {
                // ExecuteAnmIdx the spark sprites (MASTER_SPARK = 8 + i).
                self.bomb_sprites[i] =
                    Some(AnmRunner::new(self.player_scripts[&(8 + i as i32)].clone()));
                self.seal_pos[i] = self.pos;
            }
            self.events.push(Event::SfxIdx(19)); // SOUND_BOMB_MARISA_B
            self.bomb_move_mult = 0.3;
        } else {
            if self.bomb_timer == 60 {
                self.register_shake(60, 1.0, 7.0); // ScreenEffect SHAKE (60,1,7)
            } else if self.bomb_timer == 120 {
                self.register_shake(200, 24.0, 0.0); // ScreenEffect SHAKE (200,24,0)
            }
            if self.bomb_timer % 4 != 0 {
                let spark = BombRect {
                    pos: [192.0, self.pos[1] / 2.0],
                    size: [384.0, self.pos[1]],
                };
                self.bomb_projectiles[0] = spark;
                self.bomb_regions[0] = spark;
                self.bomb_region_dmg[0] = 12;
            }
            for i in 0..4 {
                if let Some(r) = self.bomb_sprites[i].as_mut() {
                    r.tick(); // ExecuteScript
                }
            }
        }
        self.bomb_timer += 1;
    }

    /// Player::Die runs at collision time, before the deathbomb window.
    fn register_hit(&mut self) {
        self.deaths += 1;
        self.spell_capturing = false;
        self.invuln = 0;
        self.dying = DEATHBOMB_FRAMES;
        self.state = PlayerState::Dead(0);
        let pos = [self.pos[0], self.pos[1], 0.0];
        self.world
            .spawn_particles(12, pos, 1, [64.0 / 255.0, 64.0 / 255.0, 1.0, 1.0]);
        self.world.spawn_particles(6, pos, 16, [1.0, 1.0, 1.0, 1.0]);
        self.events.push(Event::Sfx("pldead00"));
    }

    /// Expiring Player::respawnTimer scatters items and drops power. The stock
    /// and bombs change later, when the thirty-frame death animation ends.
    fn commit_death(&mut self) {
        self.world.decrease_subrank(1600);
        let pos = self.pos;
        if self.lives > 0 {
            self.spawn_scatter_item(pos, 2);
            for _ in 0..5 {
                self.spawn_scatter_item(pos, 0);
            }
            self.world.power = (self.world.power - 16).max(0);
        } else {
            for _ in 0..5 {
                self.spawn_scatter_item(pos, 4);
            }
            self.world.power = 0;
            self.next_extend = EXTEND_SCORES.len();
        }
        self.power_item_count = 0;
    }

    /// Switch the player ANM script (idle/banking) if it changed.
    fn set_player_script(&mut self, id: i32) {
        if self.player_script_id == id {
            return;
        }
        if let Some(instrs) = self.player_scripts.get(&id) {
            self.player_runner = AnmRunner::new(instrs.clone());
            self.player_script_id = id;
        }
    }

    fn orb_positions(&self) -> [[f32; 2]; 2] {
        self.orbs_pos
    }

    /// FireSingleBullet/SpawnBullets, including the independent laser timers
    /// and first-free allocation in the original 80-slot player bullet pool.
    fn spawn_player_bullets(&mut self, timer: i32) {
        let rank = self.character.ranks()[power_rank(self.world.power)];
        let orbs = self.orb_positions();
        for def in rank {
            let free = (0..80).find(|slot| !self.shots.iter().any(|shot| shot.pool_index == *slot));
            let Some(pool_index) = free else { break };
            if def.bt == 3 {
                if self.laser_timers[def.frame as usize] != 0 {
                    continue;
                }
                self.laser_timers[def.frame as usize] = def.wait;
            } else if timer % def.wait != def.frame {
                continue;
            }
            let base = match def.spawn {
                1 => orbs[0],
                2 => orbs[1],
                _ => self.pos,
            };
            let mut runner = AnmRunner::new(
                self.player_scripts
                    .get(&def.script)
                    .cloned()
                    .unwrap_or_default(),
            );
            if let Some((sprite, range)) = runner.take_pending_random() {
                runner.set_sprite(sprite + self.world.rng.u16_in_range(range) as u32);
            }
            let shot = Shot {
                pool_index,
                pos: [base[0] + def.motion[0], base[1] + def.motion[1]],
                size: def.size,
                vel: [def.angle.cos() * def.speed, def.angle.sin() * def.speed],
                damage: def.damage,
                bt: def.bt,
                anm_script: def.script,
                runner,
                collided: false,
                angle: def.angle,
                motion: def.motion,
                spawn: def.spawn,
                laser_slot: def.frame as usize,
                age: 0,
                spd: def.speed,
            };
            let insert_at = self
                .shots
                .binary_search_by_key(&pool_index, |shot| shot.pool_index)
                .unwrap_err();
            self.shots.insert(insert_at, shot);
            if def.sound {
                self.events.push(Event::Sfx("plst00"));
            }
        }
    }

    /// UpdatePlayerBullets BULLET_TYPE_1: orb amulets steer toward the last
    /// enemy hit for their first 40 frames, then accelerate up to speed 10.
    #[allow(clippy::manual_clamp)] // Preserve the original ordered min/max behavior, including NaN.
    fn home_shot(s: &mut Shot, target: Option<[f32; 2]>) {
        match target {
            Some(t) if s.age < 40 => {
                let mut vx = t[0] - s.pos[0];
                let mut vy = t[1] - s.pos[1];
                let mut len = (vx * vx + vy * vy).sqrt() / (s.spd / 4.0);
                if len < 1.0 {
                    len = 1.0;
                }
                vx = vx / len + s.vel[0];
                vy = vy / len + s.vel[1];
                len = (vx * vx + vy * vy).sqrt();
                s.spd = if len > 10.0 { 10.0 } else { len };
                if s.spd < 1.0 {
                    s.spd = 1.0;
                }
                // Decomp (Player.cpp:522) divides by vecLength unconditionally.
                s.vel = [vx * s.spd / len, vy * s.spd / len];
            }
            _ => {
                if s.spd < 10.0 {
                    s.spd += 1.0 / 3.0;
                    let len = (s.vel[0] * s.vel[0] + s.vel[1] * s.vel[1]).sqrt();
                    // Decomp (Player.cpp:534) divides unconditionally.
                    s.vel = [s.vel[0] * s.spd / len, s.vel[1] * s.spd / len];
                }
            }
        }
    }

    fn update_shots(&mut self) {
        for timer in &mut self.laser_timers {
            if *timer != 0 {
                *timer -= 1;
            }
        }
        let target = self.last_enemy_hit.filter(|pos| pos[0] > -100.0);
        let orbs = self.orb_positions();
        for s in &mut self.shots {
            match s.bt {
                1 if !s.collided => Self::home_shot(s, target),
                2 if !s.collided => s.vel[1] -= 0.3,
                3 => {
                    if matches!(self.laser_timers[s.laser_slot], 70 | 1) {
                        s.runner.interrupt(1);
                    }
                    let orb = orbs[s.spawn as usize - 1];
                    s.pos = [orb[0] + s.motion[0], orb[1] / 2.0];
                    s.runner.scale[1] = s.pos[1] * 2.0 / 14.0;
                    s.size[1] = s.pos[1] * 2.0;
                }
                _ => {}
            }
            s.pos[0] += s.vel[0];
            s.pos[1] += s.vel[1];
            s.runner.tick();
            if let Some((sprite, range)) = s.runner.take_pending_random() {
                s.runner
                    .set_sprite(sprite + self.world.rng.u16_in_range(range) as u32);
            }
            s.age += 1;
        }
        let [_, _, region_width, region_height] = self.arcade;
        self.shots.retain(|s| {
            if s.runner.ended() {
                return false;
            }
            if s.bt == 3 {
                return true;
            }
            let sprite = s
                .runner
                .sprite
                .and_then(|sprite| self.player_sprites.get(&sprite));
            let [width, height] = sprite.map_or(s.size, |sprite| [sprite.width, sprite.height]);
            player_shot_in_bounds(s.pos, [width, height], [region_width, region_height])
        });
    }

    /// Player::CalcDamageToEnemy: inclusive rectangle overlap, retained missile
    /// explosions, persistent lasers, bomb weakening and original hit effects.
    fn calc_player_damage(&mut self, pos: [f32; 3], hitbox: [f32; 2]) -> (i32, bool) {
        let mut damage = 0;
        for shot in &mut self.shots {
            if shot.collided && shot.bt != 2 {
                continue;
            }
            if !attack_rect_overlap(shot.pos, shot.size, [pos[0], pos[1]], hitbox) {
                continue;
            }
            damage += if self.bomb_in_use {
                (shot.damage / 3).max(1)
            } else {
                shot.damage
            };
            if shot.bt == 2 {
                shot.damage = (shot.damage / 4).max(1);
                let explosion_size = match shot.anm_script {
                    65 => Some(32.0),
                    66 => Some(42.0),
                    67 | 68 => Some(48.0),
                    _ => None,
                };
                if let Some(size) = explosion_size {
                    shot.size = [size; 2];
                }
                if shot.age % 6 == 0 {
                    self.world
                        .spawn_particles(5, [shot.pos[0], shot.pos[1], 0.0], 1, [1.0; 4]);
                }
            }
            if shot.bt != 3 {
                if !shot.collided {
                    shot.anm_script += 32;
                    shot.runner = AnmRunner::new(
                        self.player_scripts
                            .get(&shot.anm_script)
                            .cloned()
                            .unwrap_or_default(),
                    );
                    if let Some((sprite, range)) = shot.runner.take_pending_random() {
                        shot.runner
                            .set_sprite(sprite + self.world.rng.u16_in_range(range) as u32);
                    }
                    self.world
                        .spawn_particles(5, [shot.pos[0], shot.pos[1], 0.0], 1, [1.0; 4]);
                }
                shot.collided = true;
                shot.vel[0] /= 8.0;
                shot.vel[1] /= 8.0;
            } else {
                self.player_particle_timer = self.player_particle_timer.wrapping_add(1);
                if self.player_particle_timer.is_multiple_of(8) {
                    self.world
                        .spawn_particles(5, [shot.pos[0], pos[1], pos[2]], 1, [1.0; 4]);
                }
            }
        }
        let mut hit_by_bomb = false;
        for region in 0..self.bomb_regions.len() {
            let rect = self.bomb_regions[region];
            if rect.size[0] <= 0.0
                || !attack_rect_overlap(rect.pos, rect.size, [pos[0], pos[1]], hitbox)
            {
                continue;
            }
            damage += self.bomb_region_dmg[region];
            self.bomb_dmg_accum[region] += self.bomb_region_dmg[region];
            self.player_particle_timer = self.player_particle_timer.wrapping_add(1);
            if self.player_particle_timer.is_multiple_of(4) {
                self.world.spawn_particles(3, pos, 1, [1.0; 4]);
            }
            hit_by_bomb |= self.bomb_in_use;
        }
        (damage, hit_by_bomb)
    }

    fn update_bullets(&mut self) {
        if self.world.time_stopped {
            return;
        }
        // BulletManager runs each slot's movement, graze, kill and ANM before
        // the next slot. In particular a lethal hit changes the player state
        // before a second nearby bullet reaches CheckGraze.
        let mut bullets = std::mem::take(&mut self.world.bullets);
        for bullet in &mut bullets {
            if bullet.step(self.pos, [self.arcade[2], self.arcade[3]]) {
                self.collide_bullet(bullet);
                bullet.finish_fired_step();
            }
        }
        bullets.retain(|bullet| bullet.oob_count < 256);
        self.world.bullets = bullets;
        let mut lasers = std::mem::take(&mut self.world.lasers);
        for laser in &mut lasers {
            for collision in laser.step() {
                self.collide_laser(collision);
            }
        }
        self.world.lasers = lasers;
    }

    /// Half of the bullet's collision box (grazeSize/2), used for both the kill
    /// box and the +20 graze box against the player's 1.25 hitbox. The decomp
    /// (BulletManager.cpp:1382) derives grazeSize from the type's sprite height
    /// and its anm script: <=8px -> 4; <=16px -> rice/shard 4, kunai 5, else 6;
    /// <=32px -> fireball 11, dagger 9, else 16; bigger -> 32.
    fn bullet_radius(&self, b: &Bullet) -> f32 {
        let t = b.type_index;
        let graze_size = if b.height <= 8.0 {
            4.0
        } else if b.height <= 16.0 {
            match t {
                2 | 5 => 4.0, // rice, shard
                4 => 5.0,     // kunai
                _ => 6.0,
            }
        } else if b.height <= 32.0 {
            match t {
                7 => 11.0, // fireball
                8 => 9.0,  // dagger
                _ => 16.0,
            }
        } else {
            32.0
        };
        graze_size / 2.0
    }

    /// EnemyManager's per-slot damage/death pass runs immediately after that
    /// enemy's ECL/ANM, before the following slot can fire or create children.
    fn collide_enemy(&mut self, index: usize) {
        if !self.enemies[index].ready_for_damage() {
            return;
        }
        let life_before = self.enemies[index].life;
        let (pos, hitbox, interactable, collidable, is_boss) = {
            let enemy = &self.enemies[index];
            (
                enemy.pos,
                enemy.hitbox,
                enemy.interactable,
                enemy.collidable,
                enemy.is_boss,
            )
        };
        if interactable && collidable {
            let half = [hitbox[0] / 3.0, hitbox[1] / 3.0];
            let bomb_intercepts = self.bomb_projectiles.iter().any(|rect| {
                rect.size[0] > 0.0
                    && (rect.pos[0] - pos[0]).abs() <= rect.size[0] / 2.0 + half[0]
                    && (rect.pos[1] - pos[1]).abs() <= rect.size[1] / 2.0 + half[1]
            });
            if !bomb_intercepts
                && (self.pos[0] - pos[0]).abs() <= half[0] + 1.25
                && (self.pos[1] - pos[1]).abs() <= half[1] + 1.25
            {
                if matches!(self.state, PlayerState::Alive)
                    && self.invuln == 0
                    && std::env::var_os("TH06_GOD").is_none()
                {
                    self.register_hit();
                }
                if !is_boss {
                    self.enemies[index].life -= 10;
                }
            }
        }
        if !interactable {
            self.update_enemy_damage_feedback(index, life_before);
            return;
        }
        let (mut damage, hit_by_bomb) = self.calc_player_damage(pos, hitbox);
        damage = damage.min(70);
        self.score += (damage / 5 * 10) as i64;
        if self.world.spellcard_active != 0 {
            damage = if !hit_by_bomb {
                if damage > 7 {
                    damage / 7
                } else {
                    i32::from(damage != 0)
                }
            } else if self.spell_used_bomb {
                if damage > 3 {
                    damage / 3
                } else {
                    i32::from(damage != 0)
                }
            } else {
                0
            };
        }
        if self.enemies[index].damageable {
            self.enemies[index].life -= damage;
        }
        if self
            .last_enemy_hit
            .is_none_or(|previous| previous[1] < pos[1])
        {
            self.last_enemy_hit = Some([pos[0], pos[1]]);
        }
        if self.enemies[index].life <= 0 {
            let mode = self.enemies[index].death_mode;
            let drop = self.enemies[index].item_drop;
            if mode == 0 || mode == 1 {
                self.score += self.enemies[index].score as i64;
            }
            self.enemies[index].spawn_death_effects(&mut self.world);
            if mode != 3 {
                self.spawn_drop([pos[0], pos[1]], drop, hit_by_bomb);
            }
            if is_boss && mode != 3 && self.world.spellcard_active == 0 {
                self.despawn_bullets(false);
            }
            self.enemies[index].on_death(&self.ecl, &mut self.world);
            self.events.push(Event::SfxIdx(2 + (index % 2) as u8));
        }
        self.update_enemy_damage_feedback(index, life_before);
    }

    fn update_enemy_damage_feedback(&mut self, index: usize, life_before: i32) {
        let flash = self.enemies[index].update_damage_feedback(life_before);
        if let Some(anim) = self.anims.get_mut(index).and_then(Option::as_mut) {
            anim.color_add = flash;
        }
        if flash {
            self.events.push(Event::SfxIdx(20)); // SOUND_TOTAL_BOSS_DEATH
        }
    }

    fn collide_bullet(&mut self, bullet: &mut Bullet) {
        let radius = self.bullet_radius(bullet);
        let margin = radius + if bullet.grazed { 0.0 } else { 20.0 };
        for projectile in self.bomb_projectiles {
            if projectile.size[0] == 0.0 {
                continue;
            }
            if (projectile.pos[0] - bullet.pos[0]).abs() <= projectile.size[0] / 2.0 + margin
                && (projectile.pos[1] - bullet.pos[1]).abs() <= projectile.size[1] / 2.0 + margin
            {
                bullet.begin_despawn();
                self.spawn_item(Item::homing(bullet.pos, 6));
                return;
            }
        }
        let dx = (bullet.pos[0] - self.pos[0]).abs();
        let dy = (bullet.pos[1] - self.pos[1]).abs();
        const PLAYER_HALF: f32 = 1.25;
        if !bullet.grazed {
            // CheckGraze excludes DEAD/SPAWNING, but ScoreGraze still emits
            // score, subrank and particles during a bomb; only its counts stop.
            if !matches!(self.state, PlayerState::Alive) {
                return;
            }
            if dx > radius + PLAYER_HALF + 20.0 || dy > radius + PLAYER_HALF + 20.0 {
                return;
            }
            self.score_graze(bullet.pos);
            bullet.grazed = true;
        }
        // CalcKillBoxCollision always consumes a touching bullet, even while
        // the player is invulnerable, dead, or spawning.
        if dx <= radius + PLAYER_HALF && dy <= radius + PLAYER_HALF {
            bullet.begin_despawn();
            if matches!(self.state, PlayerState::Alive)
                && self.invuln == 0
                && self.bombing == 0
                && std::env::var_os("TH06_GOD").is_none()
            {
                self.register_hit();
            }
        }
    }

    fn collide_laser(&mut self, laser: crate::projectiles::LaserCollision) {
        let dx = self.pos[0] - laser.origin[0];
        let dy = self.pos[1] - laser.origin[1];
        let (sin, cos) = laser.angle.sin_cos();
        let along = dx * cos + dy * sin;
        let across = -dx * sin + dy * cos;
        let half_x = laser.length / 2.0 + 1.25;
        let half_y = laser.width / 2.0 + 1.25;
        if (along - laser.center).abs() <= half_x && across.abs() <= half_y {
            if matches!(self.state, PlayerState::Alive)
                && self.invuln == 0
                && self.bombing == 0
                && std::env::var_os("TH06_GOD").is_none()
            {
                self.register_hit();
            }
            return;
        }
        if laser.can_graze
            && matches!(self.state, PlayerState::Alive)
            && (along - laser.center).abs() <= half_x + 48.0
            && across.abs() <= half_y + 48.0
        {
            self.score_graze(self.pos);
        }
    }

    /// Player::ScoreGraze. Total and stage counts have independent caps; all
    /// side effects still run when either cap is reached or a bomb is active.
    fn score_graze(&mut self, center: [f32; 2]) {
        if self.bombing == 0 {
            self.graze = (self.graze + 1).min(999999);
            self.graze_in_stage = (self.graze_in_stage + 1).min(9999);
        }
        let pos = [
            (self.pos[0] + center[0]) / 2.0,
            (self.pos[1] + center[1]) / 2.0,
            0.0,
        ];
        self.world.spawn_particles(8, pos, 1, [1.0; 4]);
        self.score += 500;
        self.world.increase_subrank(6);
        self.events.push(Event::SfxIdx(30));
    }

    fn start_dialogue(&mut self, idx: usize) {
        // Oracle-comparison mode: skip dialogue so the timeline doesn't freeze
        // (matches the full-VM oracle's stubbed Gui::MsgWait). The MsgWait
        // timeline op then never holds, so the boss/midboss attack interrupts
        // fire on schedule, exactly as in the stubbed decomp run.
        if std::env::var_os("TH06_NO_DIALOGUE").is_some() {
            return;
        }
        let Some(&off) = self.msg.offsets.get(idx) else {
            return;
        };
        self.dialogue = Dialogue {
            active: true,
            msg_idx: idx,
            off,
            ..Default::default()
        };
        if self.stage_num == 7
            && matches!(idx, 0 | 10)
            && let Some(asset) = &self.extra_boss_portrait
            && let Some(first) = self.boss_portraits.first_mut()
        {
            *first = asset.clone();
        }
    }

    /// Gui executes portrait animation even while a WAIT or MSGHALT holds.
    fn run_dialogue(&mut self, input: &Input) {
        self.run_dialogue_instructions(input);
        if self.dialogue.active {
            for runner in self
                .dialogue
                .portraits
                .iter_mut()
                .chain(self.dialogue.line_runners.iter_mut())
                .chain(self.dialogue.intro_runners.iter_mut())
                .flatten()
            {
                runner.tick();
            }
        }
    }

    fn run_dialogue_instructions(&mut self, input: &Input) {
        let d = &mut self.dialogue;
        d.ecl_resumed = false;
        if d.skippable
            && input.held(Key::Skip)
            && let Some(i) = self.msg.instr_at(d.off)
        {
            d.timer = i.time;
        }
        loop {
            let Some(i) = self.msg.instr_at(d.off) else {
                d.active = false;
                return;
            };
            if d.timer < i.time {
                break;
            }
            match i.opcode {
                0 => {
                    // MSGDELETE
                    d.active = false;
                    return;
                }
                3 => {
                    // TEXTDIALOGUE — original Japanese (Shift-JIS) via a pre-baked
                    // texture keyed by this instruction's offset.
                    let color = i.arg_i16(0).clamp(0, 1) as usize;
                    let line = i.arg_i16(2).clamp(0, 1) as usize;
                    let off = d.off;
                    if line == 0 {
                        d.lines[1].clear();
                        d.line_tex[1] = None;
                    }
                    let raw = {
                        let t = &i.args[4.min(i.args.len())..];
                        let end = t.iter().position(|&b| b == 0).unwrap_or(t.len());
                        t[..end].to_vec()
                    };
                    d.lines[line] = th06_engine::text::decode_shiftjis(&raw);
                    d.line_colors[line] = color;
                    d.line_tex[line] = self.dialogue_tex.get(&off).copied();
                    d.line_runners[line] = self
                        .dialogue_scripts
                        .get(&(2 + line as u32))
                        .map(|script| AnmRunner::new(script.clone()));
                    d.frames_pause = 0;
                }
                4 => {
                    // WAIT n frames; Z after 8 frames advances.
                    let wait = i.arg_i32(0) as u16;
                    let advance = (d.skippable && input.held(Key::Skip))
                        || (input.pressed(Key::Shoot) && d.frames_pause >= 8);
                    if d.frames_pause < wait && !advance {
                        d.frames_pause += 1;
                        return; // time frozen during the pause
                    }
                }
                1 => {
                    let idx = i.arg_i16(0).clamp(0, 1) as usize;
                    let script = i.arg_i16(2) as u32 + if idx == 0 { 0 } else { 2 };
                    d.portraits[idx] = self
                        .player_portraits
                        .first()
                        .and_then(|asset| asset.entry.scripts.iter().find(|(id, _)| *id == script))
                        .map(|(_, instrs)| AnmRunner::new(instrs.clone()));
                }
                2 => {
                    let idx = i.arg_i16(0).clamp(0, 1) as usize;
                    if let Some(portrait) = &mut d.portraits[idx] {
                        portrait.set_sprite(i.arg_i16(2) as u32 + if idx == 0 { 0 } else { 8 });
                    }
                }
                5 => {
                    let idx = i.arg_i16(0) as usize;
                    let runner = if idx < 2 {
                        d.portraits.get_mut(idx)
                    } else {
                        d.line_runners.get_mut(idx - 2)
                    };
                    if let Some(Some(runner)) = runner {
                        runner.interrupt(i.args.get(2).copied().unwrap_or(0) as u32);
                    }
                }
                6 => d.ecl_resumed = true, // ECLRESUME
                7 => {
                    // MUSIC: stage 1 track 1 is the boss theme.
                    let track = i.arg_i32(0);
                    self.boss_bgm_started = track == 1;
                    self.events.push(Event::Bgm(if track == 1 {
                        self.boss_bgm
                    } else {
                        self.stage_bgm
                    }));
                }
                8 => {
                    let color = i.arg_i16(0).clamp(0, 1) as usize;
                    let line = i.arg_i16(2).clamp(0, 1) as usize;
                    let raw = &i.args[4.min(i.args.len())..];
                    let end = raw.iter().position(|&byte| byte == 0).unwrap_or(raw.len());
                    d.intro_lines[line] = th06_engine::text::decode_shiftjis(&raw[..end]);
                    d.intro_colors[line] = color;
                    d.intro_tex[line] = self.dialogue_tex.get(&d.off).copied();
                    d.intro_runners[line] = self
                        .dialogue_scripts
                        .get(&(4 + line as u32))
                        .map(|script| AnmRunner::new(script.clone()));
                    d.frames_pause = 0;
                }
                9 => {
                    let bonus = stage_clear_bonus(
                        self.stage_num,
                        self.graze_in_stage,
                        self.world.power as i64,
                        self.point_items,
                        self.lives as i64,
                        self.bombs as i64,
                        self.world.difficulty,
                        self.starting_lives,
                    );
                    self.score += bonus;
                    self.clear_bonus = bonus;
                    self.hiscore = self.hiscore.max(self.score);
                    self.events.push(Event::SaveScore(self.hiscore));
                    self.state = PlayerState::Results;
                    if self.stage_num >= 6 {
                        self.next_extend = EXTEND_SCORES.len();
                    }
                }
                10 => {
                    // MSGHALT keeps the current message/timer in place.
                    return;
                }
                11 => {
                    self.gui_score = self.score;
                    self.next_score_inc = 0;
                    self.events.push(Event::NextStage);
                    d.active = false;
                    return;
                }
                12 => self.events.push(Event::BgmFade(4.0)),
                13 => d.skippable = i.arg_i32(0) != 0,
                _ => {}
            }
            d.off += i.size;
        }
        d.timer += 1;
        if d.skippable && input.held(Key::Skip) && d.timer < 60 {
            d.timer = 60;
        }
    }

    fn spawn_drop(&mut self, pos: [f32; 2], drop: i16, hit_by_bomb: bool) {
        let kind = match drop {
            -2 => return, // ITEM_NO_ITEM
            -1 => {
                // ITEM_RANDOM_ITEM: every third enemy drops, following the
                // fixed pattern table.
                let drop = self.rand_item_spawn.is_multiple_of(3);
                self.rand_item_spawn += 1;
                if !drop {
                    return;
                }
                let k = RANDOM_ITEMS[self.rand_item_table];
                self.rand_item_table = (self.rand_item_table + 1) % RANDOM_ITEMS.len();
                k
            }
            k => k as i32,
        };
        self.spawn_item(if hit_by_bomb {
            Item::homing(pos, kind)
        } else {
            Item::fall(pos, kind)
        });
    }

    fn reserve_item_slot(&mut self) -> Option<(usize, usize)> {
        for _ in 0..512 {
            let candidate = self.next_item_index;
            self.next_item_index = (candidate + 1) % 512;
            if let Err(insert_at) = self
                .items
                .binary_search_by_key(&candidate, |item| item.pool_index)
            {
                return Some((candidate, insert_at));
            }
        }
        None
    }

    fn spawn_item(&mut self, mut item: Item) {
        if let Some((pool_index, insert_at)) = self.reserve_item_slot() {
            item.pool_index = pool_index;
            self.items.insert(insert_at, item);
        }
    }

    fn spawn_scatter_item(&mut self, pos: [f32; 2], kind: i32) {
        // SpawnItem reserves its fixed slot before drawing a scatter target.
        if let Some((pool_index, insert_at)) = self.reserve_item_slot() {
            let mut item = Item::scatter(pos, kind, &mut self.world.rng);
            item.pool_index = pool_index;
            self.items.insert(insert_at, item);
        }
    }

    fn update_items(&mut self) {
        let player = self.pos;
        let alive = matches!(self.state, PlayerState::Alive);
        // Visit original fixed slots in order. Items spawned by a full-power
        // pickup can themselves update later in this same ItemManager frame.
        for pool_index in 0..512 {
            let Ok(index) = self
                .items
                .binary_search_by_key(&pool_index, |item| item.pool_index)
            else {
                continue;
            };
            let item = &mut self.items[index];
            let interpolating = item.state == 2 && item.timer < 60;
            if interpolating {
                let fraction = item.timer as f32 / 60.0;
                item.pos[0] = fraction * item.target[0] + item.start[0] * (1.0 - fraction);
                item.pos[1] = fraction * item.target[1] + item.start[1] * (1.0 - fraction);
            } else {
                let mut horizontal_velocity = 0.0;
                if item.state == 2 {
                    if item.timer == 60 {
                        item.vy = 0.0;
                    }
                } else if item.state == 1 || self.world.power >= 128 && player[1] < 128.0 {
                    let angle = (player[1] - item.pos[1]).atan2(player[0] - item.pos[0]);
                    let (sin, cos) = angle.sin_cos();
                    horizontal_velocity = cos * 8.0;
                    item.vy = sin * 8.0;
                    item.state = 1;
                } else if item.vy < -2.2 {
                    item.vy = -2.2;
                }
                item.pos[0] += horizontal_velocity;
                item.pos[1] += item.vy;
                if item.pos[1] >= self.arcade[3] + 16.0 {
                    self.items.remove(index);
                    self.world.decrease_subrank(3);
                    continue;
                }
                // Original OnUpdate applies gravity after movement; the step
                // can overshoot 3.0 once before the next callback clamps it.
                if item.vy < 3.0 {
                    item.vy += 0.03;
                } else {
                    item.vy = 3.0;
                }
            }
            let dx = (player[0] - item.pos[0]).abs();
            let dy = (player[1] - item.pos[1]).abs();
            // CalcItemBoxCollision: player's 12px half-box plus the 16px item.
            if alive && dx <= 20.0 && dy <= 20.0 {
                let item = self.items.remove(index);
                self.collect_item(item.kind, item.pos);
            } else {
                item.timer += 1;
            }
        }
    }

    fn collect_item(&mut self, kind: i32, pos: [f32; 2]) {
        let before_score = self.score;
        let before_power = self.world.power;
        // Per-item subrank (ItemManager.cpp collect switch): small power +1,
        // point +30 (high) / +3, bomb +5, life +200; big/full power and
        // point-bullet give none.
        match kind {
            0 => self.collect_power(1),
            2 => self.collect_power(8),
            4 => {
                // ITEM_FULL_POWER: reaching max cancels bullets into points.
                if self.world.power < 128 {
                    self.bullets_to_points();
                }
                self.world.power = 128;
                self.score += 1000;
            }
            1 => {
                let difficulty = Difficulty::ALL[self.world.difficulty as usize];
                self.score += difficulty.point_score(pos[1]);
                self.point_items += 1;
                self.point_items_total += 1;
                self.world
                    .increase_subrank(if pos[1] < 128.0 { 30 } else { 3 });
            }
            6 => {
                // ITEM_POINT_BULLET: (grazeInStage/3)*10 + 500, or 100 while
                // a bomb is active.
                let graze_in_stage = self.graze_in_stage;
                self.score += if self.bombing > 0 {
                    100
                } else {
                    (graze_in_stage / 3) * 10 + 500
                };
            }
            3 => self.bombs = (self.bombs + 1).min(8),
            5 => self.lives = (self.lives + 1).min(8),
            _ => {}
        }
        match kind {
            0 => self.world.increase_subrank(1),
            3 => self.world.increase_subrank(5),
            5 => self.world.increase_subrank(200),
            _ => {}
        }
        let white = [1.0; 4];
        let yellow = [1.0, 1.0, 0.0, 1.0];
        let power_up = [8, 16, 32, 48, 64, 80, 96, 128]
            .iter()
            .any(|threshold| before_power < *threshold && self.world.power >= *threshold);
        match kind {
            0 | 2 => {
                if power_up {
                    self.score_popups.spawn(
                        false,
                        pos,
                        -1,
                        [128.0 / 255.0, 192.0 / 255.0, 1.0, 1.0],
                    );
                    self.events.push(Event::SfxIdx(31));
                } else {
                    let value = self.score - before_score;
                    self.score_popups.spawn(
                        false,
                        pos,
                        value,
                        if value >= 12800 { yellow } else { white },
                    );
                }
            }
            1 => {
                let maximum = match self.world.difficulty {
                    2 => 150000,
                    3 => 200000,
                    4 => 300000,
                    _ => 100000,
                };
                let value = self.score - before_score;
                self.score_popups.spawn(
                    false,
                    pos,
                    value,
                    if value >= maximum { yellow } else { white },
                );
            }
            4 => {
                if before_power < 128 {
                    self.score_popups.spawn(
                        false,
                        pos,
                        -1,
                        [128.0 / 255.0, 192.0 / 255.0, 1.0, 1.0],
                    );
                    self.events.push(Event::SfxIdx(31));
                }
                self.score_popups.spawn(false, pos, 1000, white);
            }
            5 => self.events.push(Event::SfxIdx(28)),
            6 => self
                .score_popups
                .spawn(true, pos, self.score - before_score, white),
            _ => {}
        }
        self.events.push(Event::SfxIdx(21));
    }

    /// ItemManager power-item collection: below max, +`amount` power and +10
    /// score; at max power, items convert to score via the g_PowerItemScore
    /// table indexed by a running count.
    fn collect_power(&mut self, amount: i32) {
        if self.world.power >= 128 {
            self.power_item_count = (self.power_item_count + amount as usize).min(30);
            self.score += POWER_ITEM_SCORE[self.power_item_count];
        } else {
            self.power_item_count = 0;
            self.world.power = (self.world.power + amount).min(128);
            self.score += 10;
            // Reaching max power cancels every bullet into point items.
            if self.world.power >= 128 {
                self.bullets_to_points();
            }
        }
    }

    /// BulletManager::TurnAllBulletsIntoPoints: each live bullet becomes a
    /// homing point-bullet item; lasers are cancelled too. Used on full power,
    /// spellcard start, and spellcard capture.
    fn bullets_to_points(&mut self) {
        let points = self.world.turn_bullets_into_points();
        for pos in points {
            self.spawn_item(Item::homing(pos, 6));
        }
    }

    /// BulletManager::DespawnBullets: cancel every bullet/laser, awarding an
    /// escalating bonus (2000, +10 each, capped at 12800), optionally as point
    /// items, then add it to the score and show the "BONUS" popup.
    fn despawn_bullets(&mut self, award_points: bool) {
        let (points, total) = self.world.despawn_bullets_for_bonus(award_points);
        self.award_bullet_cancel(points, total);
    }

    fn award_bullet_cancel(&mut self, points: Vec<[f32; 2]>, total: i64) {
        for pos in points {
            self.spawn_item(Item::homing(pos, 6));
        }
        self.score += total;
        if total != 0 {
            self.bonus_score_amount = total;
            self.bonus_score_timer = 250;
        }
    }

    fn drain_world_events(&mut self) {
        let events: Vec<WorldEvent> = self.world.events.drain(..).collect();
        for ev in events {
            match ev {
                WorldEvent::ScorePopup(pos, value, color) => {
                    self.score_popups.spawn(false, pos, value, color)
                }
                WorldEvent::Sfx(idx) => {
                    if let Ok(index) = u8::try_from(idx) {
                        self.events.push(Event::SfxIdx(index));
                    }
                }
                WorldEvent::SpellcardStart(id, sprite, raw) => {
                    self.spell_active = true;
                    // Original Japanese name: pre-baked texture + decoded string.
                    self.spell_name = th06_engine::text::decode_shiftjis(&raw);
                    self.spell_name_cur = self.spell_name_tex.get(&id).copied();
                    self.spell_name_byte_len = raw.len();
                    self.spell_id = id;
                    if !self.is_replay
                        && let Some(record) = self.spellcard_records.get_mut(id as usize)
                    {
                        let checksum = raw.iter().fold(0u8, |sum, &byte| sum.wrapping_add(byte));
                        if checksum != record.name_checksum {
                            record.attempts = 0;
                            record.successes = 0;
                            record.name_checksum = checksum;
                        }
                        record.name = raw.clone();
                        record.capture_score =
                            SPELLCARD_SCORE.get(id as usize).copied().unwrap_or(0) as u32;
                        if record.attempts < 9999 {
                            record.attempts += 1;
                        }
                    }
                    self.spell_capturing = true;
                    self.spell_used_bomb = false; // fresh spellcardInfo
                    // Start the name announce + portrait slide-in (ShowSpellcard).
                    self.spell_name_runner = Some(AnmRunner::new(self.spell_name_script.clone()));
                    self.portrait_runner = Some(AnmRunner::new(self.portrait_script.clone()));
                    if let Some(portrait) = &mut self.portrait_runner {
                        portrait.set_sprite(8 + sprite as u32);
                    }
                    self.events.push(Event::Sfx("cat00"));
                }
                WorldEvent::SpellcardEnd { points, bonus } => {
                    // SPELLCARDEND (EclManager.cpp): a no-op unless a spellcard is
                    // active (isActive != 0). Only then does it run the bullet-
                    // cancel DespawnBullets (+ capture bonus). Non-spell attacks —
                    // e.g. the midboss opener — never set spell_active, so they
                    // don't clear here; their timeout fades the field instead. (#14)
                    let captured = self.spell_active && self.spell_capturing;
                    if self.spell_active {
                        self.spell_result = 120;
                        self.spell_captured = self.spell_capturing;
                        // Fly the name banner off (interrupt label 1, text.anm:130).
                        if let Some(r) = &mut self.spell_name_runner {
                            r.interrupt(1);
                        }
                        self.award_bullet_cancel(points, bonus);
                        if captured {
                            self.spellcards_captured += 1;
                            if !self.is_replay
                                && let Some(record) =
                                    self.spellcard_records.get_mut(self.spell_id as usize)
                            {
                                record.successes = record.successes.wrapping_add(1);
                                record.character_shot_type.copy_within(0..4, 1);
                                record.character_shot_type[0] =
                                    self.world.character * 2 + self.world.shot_type;
                            }
                            let base = SPELLCARD_SCORE
                                .get(self.spell_id.max(0) as usize)
                                .copied()
                                .unwrap_or(0);
                            let bonus = base + base * self.spell_secs.max(0) as i64 / 10;
                            self.score += bonus;
                            self.spell_bonus_amount = bonus;
                            self.spell_bonus_timer = 280;
                        }
                    }
                    self.spell_active = false;
                    self.spell_used_bomb = false;
                }
                WorldEvent::SpellTimeout => {
                    // Ran the timer out on a non-timeout spell: no capture bonus,
                    // and the field fades via RemoveAllBullets(0) — the bullets
                    // drift out over the donut anm rather than vanishing
                    // (EnemyManager.cpp:421). (#14)
                    self.spell_capturing = false;
                }
                WorldEvent::BulletCancel(points) => {
                    for pos in points {
                        self.spawn_item(Item::homing(pos, 6));
                    }
                }
                WorldEvent::BossSet(_present) => {
                    // Boss music is driven by the pre-boss dialogue, not here:
                    // BossSet also fires for the midboss, which keeps the stage
                    // theme.
                }
                WorldEvent::DropItem(pos, kind) => {
                    if kind >= 0 {
                        self.spawn_item(Item::fall(pos, kind));
                    }
                }
                WorldEvent::Unpause => {
                    if let Some(bg) = self.background.as_mut() {
                        bg.unpause();
                    }
                }
            }
        }
    }

    fn player_sprite_cmd(
        &self,
        runner: &AnmRunner,
        pos: [f32; 2],
        rotation: f32,
    ) -> Option<DrawCmd> {
        if !runner.visible() {
            return None;
        }
        let sprite = self.player_sprites.get(&runner.sprite?)?;
        let [tw, th] = self.player_tex_size;
        let width = sprite.width * runner.scale[0].abs();
        let height = sprite.height * runner.scale[1].abs();
        let (mut u0, mut u1) = (sprite.x / tw, (sprite.x + sprite.width) / tw);
        let (mut v0, mut v1) = (sprite.y / th, (sprite.y + sprite.height) / th);
        if runner.flip_x ^ (runner.scale[0] < 0.0) {
            std::mem::swap(&mut u0, &mut u1);
        }
        if runner.flip_y ^ (runner.scale[1] < 0.0) {
            std::mem::swap(&mut v0, &mut v1);
        }
        Some(DrawCmd {
            tex: self.player_tex,
            dst: [
                FIELD_X + pos[0] - if runner.corner { 0.0 } else { width / 2.0 },
                FIELD_Y + pos[1] - if runner.corner { 0.0 } else { height / 2.0 },
                width,
                height,
            ],
            src: [u0, v0, u1, v1],
            tint: [
                runner.color[0],
                runner.color[1],
                runner.color[2],
                runner.alpha,
            ],
            // Draw2/Draw3 work in D3D's Y-up world before its screen projection.
            rot: -rotation,
            additive: runner.additive,
            color_add: false,
        })
    }

    fn draw_player_shots(&self, collided: bool, cmds: &mut Vec<DrawCmd>) {
        for shot in self.shots.iter().filter(|shot| shot.collided == collided) {
            let rotation = if shot.runner.auto_rotate {
                std::f32::consts::FRAC_PI_2 - normalize_angle(shot.angle + std::f32::consts::PI)
            } else {
                shot.runner.rotation
            };
            if let Some(cmd) = self.player_sprite_cmd(&shot.runner, shot.pos, rotation) {
                cmds.push(cmd);
            }
        }
    }

    /// Draw one bomb sprite runner (player sheet) centred at screen (cx,cy).
    /// `scale` overrides the runner's anm scale (MarisaA's trailing stars draw at
    /// 3.2 / 2.2 / 1.0); `None` uses the runner's own scale.
    fn bomb_sprite_cmd(
        &self,
        slot: usize,
        cx: f32,
        cy: f32,
        rot: f32,
        scale: Option<[f32; 2]>,
    ) -> Option<DrawCmd> {
        let r = self.bomb_sprites[slot].as_ref()?;
        if !r.visible() {
            return None;
        }
        let sp = self.player_sprites.get(&r.sprite?)?;
        let [tw, th] = self.player_tex_size;
        let s = scale.unwrap_or(r.scale);
        let w = sp.width * s[0].abs();
        let h = sp.height * s[1].abs();
        let (sx, sy) = (sp.x % tw, sp.y % th);
        let (mut u0, mut u1) = (sx / tw, (sx + sp.width) / tw);
        if r.flip_x {
            std::mem::swap(&mut u0, &mut u1);
        }
        Some(DrawCmd {
            tex: self.player_tex,
            dst: [cx - w / 2.0, cy - h / 2.0, w, h],
            src: [u0, sy / th, u1, (sy + sp.height) / th],
            tint: [r.color[0], r.color[1], r.color[2], r.alpha],
            rot: -rot,
            additive: r.additive,
            color_add: false,
        })
    }

    fn draw(&self) -> Vec<DrawCmd> {
        let mut cmds = Vec::with_capacity(96 + self.world.bullets.len());
        if self.background.is_some() {
            // The 3D background fills the field; only dim it during spells.
            if self.spell_active {
                cmds.push(rect(
                    [FIELD_X, FIELD_Y, FIELD_W, FIELD_H],
                    [0.0, 0.0, 0.05, 0.45],
                ));
            }
        } else {
            let base = if self.spell_active { 0.02 } else { 0.07 };
            cmds.push(rect(
                [FIELD_X, FIELD_Y, FIELD_W, FIELD_H],
                [base, base * 0.6, base * 0.9, 1.0],
            ));
        }

        self.draw_player_shots(false, &mut cmds);

        // BombData::DarkenViewport: a black square over the playfield whose alpha
        // ramps 0..176 over the first 60 frames, holds at 176, then ramps back to
        // 0 over the last 60.
        if self.bombing > 0 {
            let dur = self.bomb_duration as f32;
            let t = self.bomb_timer as f32;
            let level = if self.bomb_timer < 60 {
                (t * 176.0 / 60.0).min(176.0)
            } else if self.bomb_timer >= self.bomb_duration - 60 {
                ((dur - t) * 176.0 / 60.0).max(0.0)
            } else {
                176.0
            };
            cmds.push(rect(
                [FIELD_X, FIELD_Y, FIELD_W, FIELD_H],
                [0.0, 0.0, 0.0, level as i32 as f32 / 255.0],
            ));
        }

        // Bomb sprites, drawn from the player sheet via each bomb's anm runner —
        // faithful ports of BombData::Bomb*Draw.
        if self.bombing > 0 {
            use std::f32::consts::PI;
            match self.bomb_kind {
                // BombReimuADraw: each active seal's 4 sprites at the seal centre
                // plus the sprite's own posOffset, no rotation.
                0 => {
                    for idx in 0..8 {
                        if self.seal_state[idx] == 0 {
                            continue;
                        }
                        for k in 0..4 {
                            let slot = idx * 4 + k;
                            let Some(r) = self.bomb_sprites[slot].as_ref() else {
                                continue;
                            };
                            let cx = FIELD_X + self.seal_pos[idx][0] + r.pos_offset[0];
                            let cy = FIELD_Y + self.seal_pos[idx][1] + r.pos_offset[1];
                            if let Some(c) = self.bomb_sprite_cmd(slot, cx, cy, 0.0, None) {
                                cmds.push(c);
                            }
                        }
                    }
                }
                // BombReimuBDraw: each beam at its seed + posOffset, with rotation.
                1 => {
                    for i in 0..4 {
                        let Some(r) = self.bomb_sprites[i].as_ref() else {
                            continue;
                        };
                        let cx = FIELD_X + self.seal_pos[i][0] + r.pos_offset[0];
                        let cy = FIELD_Y + self.seal_pos[i][1] + r.pos_offset[1];
                        let rot = r.rotation;
                        if let Some(c) = self.bomb_sprite_cmd(i, cx, cy, rot, None) {
                            cmds.push(c);
                        }
                    }
                }
                // BombMarisaADraw: each star drawn three times along its trail at
                // scale 3.2 / 2.2 / 1.0 (offsets built from its velocity).
                2 => {
                    for idx in 0..8 {
                        let Some(r) = self.bomb_sprites[idx].as_ref() else {
                            continue;
                        };
                        let rot = r.rotation;
                        let a = self.seal_pos[idx];
                        let v = self.seal_vel[idx];
                        let trail = [
                            (a, 3.2),
                            ([a[0] - v[0] * 6.0 - 32.0, a[1] - v[1] * 6.0 - 32.0], 2.2),
                            ([a[0] - v[0] * 10.0, a[1] - v[1] * 10.0], 1.0),
                        ];
                        for (p, sc) in trail {
                            if let Some(c) = self.bomb_sprite_cmd(
                                idx,
                                FIELD_X + p[0],
                                FIELD_Y + p[1],
                                rot,
                                Some([sc, sc]),
                            ) {
                                cmds.push(c);
                            }
                        }
                    }
                }
                // BombMarisaBDraw: the four spark sprites fanned around the player,
                // offset along their angle by half the (scaled) sprite height.
                _ => {
                    for i in 0..4 {
                        let Some(r) = self.bomb_sprites[i].as_ref() else {
                            continue;
                        };
                        let Some(sp) = r.sprite.and_then(|s| self.player_sprites.get(&s)) else {
                            continue;
                        };
                        let mut ang = ((PI / 5.0) * i as f32) / 3.0 - PI + (2.0 * PI / 5.0);
                        let reach = sp.height * r.scale[1] / 2.0;
                        let cx = FIELD_X + self.pos[0] + ang.cos() * reach;
                        let cy = FIELD_Y + self.pos[1] + ang.sin() * reach;
                        ang = PI / 2.0 - ang;
                        let rot = normalize_angle(ang + PI);
                        if let Some(c) = self.bomb_sprite_cmd(i, cx, cy, rot, None) {
                            cmds.push(c);
                        }
                    }
                }
            }
        }

        // Player: the player anm runner drives the idle/banking sprite.
        if !matches!(self.state, PlayerState::GameOver) {
            let (scale, alpha, additive) = match self.state {
                PlayerState::Dead(elapsed) if self.dying == 0 => {
                    let factor = elapsed.saturating_sub(1) as f32 / 30.0;
                    (
                        [1.0 - factor, 3.0 * factor + 1.0],
                        ((255.0 - elapsed.saturating_sub(1) as f32 * 255.0 / 30.0) as u32 & 255)
                            as f32
                            / 255.0,
                        true,
                    )
                }
                PlayerState::Spawning(elapsed) => {
                    let timer = elapsed.saturating_sub(1);
                    let factor = 1.0 - timer as f32 / 30.0;
                    (
                        [1.0 - factor, 2.0 * factor + 1.0],
                        ((timer * 255 / 30) & 255) as f32 / 255.0,
                        true,
                    )
                }
                _ => ([1.0, 1.0], 1.0, false),
            };
            {
                let [ptw, pth] = self.player_tex_size;
                let sp = self
                    .player_runner
                    .sprite
                    .and_then(|i| self.player_sprites.get(&i))
                    .unwrap_or_else(|| &self.player_sprites[&0]);
                let (mut u0, mut u1) = (sp.x / ptw, (sp.x + sp.width) / ptw);
                if self.player_runner.flip_x {
                    std::mem::swap(&mut u0, &mut u1);
                }
                let flash = matches!(self.state, PlayerState::Alive)
                    && self.invuln > 0
                    && self.invuln % 8 < 2;
                cmds.push(DrawCmd {
                    tex: self.player_tex,
                    dst: [
                        FIELD_X + self.pos[0] - sp.width * scale[0] / 2.0,
                        FIELD_Y + self.pos[1] - sp.height * scale[1] / 2.0,
                        sp.width * scale[0],
                        sp.height * scale[1],
                    ],
                    src: [u0, sp.y / pth, u1, (sp.y + sp.height) / pth],
                    tint: if flash {
                        [64.0 / 255.0, 64.0 / 255.0, 64.0 / 255.0, alpha]
                    } else {
                        [1.0, 1.0, 1.0, alpha]
                    },
                    rot: 0.0,
                    additive,
                    color_add: flash,
                });
            }
            if self.orb_state != OrbState::Hidden
                && matches!(self.state, PlayerState::Alive | PlayerState::Cleared(_))
            {
                for (orb, runner) in self.orb_positions().into_iter().zip(&self.orb_runners) {
                    if let Some(cmd) = self.player_sprite_cmd(runner, orb, runner.rotation) {
                        cmds.push(cmd);
                    }
                }
            }
        }

        // Enemies via their ANM state. The decomp (EnemyManager::OnDraw) draws
        // sub-vms 0-3, then the primary vm, then sub-vms 4-7, each at the enemy's
        // position plus the runner's own position offset.
        for (i, e) in self.enemies.iter().enumerate() {
            if !e.occupied || e.invisible {
                continue;
            }
            let (ex, ey) = (FIELD_X + e.pos[0], FIELD_Y + e.pos[1]);
            let draw_sub = |s: usize, cmds: &mut Vec<DrawCmd>| {
                let Some(sv) = &self.sub_anims[i][s] else {
                    return;
                };
                if !sv.visible() {
                    return; // an ended/spriteless sub-vm is inactive (anmFileIndex < 0)
                }
                let Some(script) = self.enemy_scripts.get(&e.sub_scripts[s]) else {
                    return;
                };
                // op26 SetAutoRotate: the sub-vm faces the enemy's angle
                // (EnemyManager.cpp:794), like the primary vm's rotateAnm flag.
                let rot = if sv.auto_rotate { e.angle } else { sv.rotation };
                if let Some(cmd) = sprite_cmd_from(
                    sv,
                    script,
                    ex + sv.pos_offset[0],
                    ey + sv.pos_offset[1],
                    rot,
                    sv.additive,
                ) {
                    cmds.push(cmd);
                }
            };
            for s in 0..4 {
                draw_sub(s, &mut cmds);
            }
            if let Some(anim) = &self.anims[i]
                && let Some(script) = self.enemy_scripts.get(&e.anm_script)
            {
                // op120 ANMFLAGROTATION: face the movement angle when flagged.
                let rot = if e.rotate_anm { e.angle } else { anim.rotation };
                if let Some(cmd) = sprite_cmd_from(
                    anim,
                    script,
                    ex + anim.pos_offset[0],
                    ey + anim.pos_offset[1],
                    rot,
                    anim.additive,
                ) {
                    cmds.push(cmd);
                }
            }
            for s in 4..8 {
                draw_sub(s, &mut cmds);
            }
        }

        // (Boss health bar / timer / spell name are drawn in draw_hud, over the
        // field, from front.anm sprites — Gui::DrawGameScene.)

        self.draw_player_shots(true, &mut cmds);

        let [tw, th] = self.bullet_tex_size;
        // Laser ANM dimensions and state are computed before advancing the
        // clock, just as BulletManager::OnUpdate/OnDraw. The base glow remains
        // visible during despawn when its original offset condition permits.
        for laser in &self.world.lasers {
            if !laser.in_use {
                continue;
            }
            let length = (laser.end_offset - laser.start_offset).max(0.0);
            let width = laser.draw_width.max(0.0);
            let midpoint = laser.start_offset + length / 2.0;
            let cx = FIELD_X + laser.pos[0] + laser.angle.cos() * midpoint;
            let cy = FIELD_Y + laser.pos[1] + laser.angle.sin() * midpoint;
            let color = laser.color.max(0) as u32;
            let base = if laser.script == 9 { 146 } else { 62 };
            if let Some(sprite) = self.bullet_sprites.get(&(base + color)) {
                cmds.push(DrawCmd {
                    tex: TEX_BULLET,
                    dst: [cx - width / 2.0, cy - length / 2.0, width, length],
                    src: [
                        sprite.x / tw,
                        sprite.y / th,
                        (sprite.x + sprite.width) / tw,
                        (sprite.y + sprite.height) / th,
                    ],
                    tint: [1.0, 1.0, 1.0, laser.draw_alpha],
                    rot: laser.angle - std::f32::consts::FRAC_PI_2,
                    additive: true,
                    color_add: false,
                });
            }
            if (laser.start_offset < 16.0 || laser.speed == 0.0)
                && let Some(sprite) = self.bullet_sprites.get(
                    &(140
                        + crate::projectiles::SPAWN_COLORS_16
                            .get(color as usize)
                            .copied()
                            .unwrap_or(0)),
                )
            {
                let ox = FIELD_X + laser.pos[0] + laser.angle.cos() * laser.start_offset;
                let oy = FIELD_Y + laser.pos[1] + laser.angle.sin() * laser.start_offset;
                let mut scale = laser.width / 10.0 * ((16.0 - laser.start_offset) / 16.0);
                if scale < 0.0 {
                    scale = laser.width / 10.0;
                }
                let (width, height) = (sprite.width * scale, sprite.height * scale);
                cmds.push(DrawCmd {
                    tex: TEX_BULLET,
                    dst: [ox - width / 2.0, oy - height / 2.0, width, height],
                    src: [
                        sprite.x / tw,
                        sprite.y / th,
                        (sprite.x + sprite.width) / tw,
                        (sprite.y + sprite.height) / th,
                    ],
                    tint: [1.0; 4],
                    rot: 0.0,
                    additive: true,
                    color_add: false,
                });
            }
        }

        // Items (etama3 sprites 0..6, index = item kind). Point-bullet items
        // (kind 6) fall back to the point-item sprite if the sheet lacks one.
        let [btw, bth] = self.bullet_tex_size;
        for it in &self.items {
            let sp = match self.bullet_sprites.get(&(it.kind as u32)) {
                Some(sp) => sp,
                None if it.kind == 6 => match self.bullet_sprites.get(&1) {
                    Some(sp) => sp,
                    None => continue,
                },
                None => continue,
            };
            cmds.push(DrawCmd {
                tex: TEX_BULLET,
                dst: [
                    FIELD_X + it.pos[0] - sp.width / 2.0,
                    FIELD_Y + it.pos[1] - sp.height / 2.0,
                    sp.width,
                    sp.height,
                ],
                src: [
                    sp.x / btw,
                    sp.y / bth,
                    (sp.x + sp.width) / btw,
                    (sp.y + sp.height) / bth,
                ],
                tint: [1.0, 1.0, 1.0, 1.0],
                rot: 0.0,
                additive: false,
                color_add: false,
            });
        }

        // Particle effects (etama4) via each effect's ANM runner — death
        // splashes, item-attract glows, etc., with original additive blending.
        let [etw, eth] = self.eff_tex_size;
        for e in self.world.effects.iter().flatten() {
            let Some(idx) = e.runner.sprite else { continue };
            let Some(sp) = self.eff_sprites.get(&idx) else {
                continue;
            };
            let src = [
                sp.x / etw,
                sp.y / eth,
                (sp.x + sp.width) / etw,
                (sp.y + sp.height) / eth,
            ];
            let tint = [
                e.color[0],
                e.color[1],
                e.color[2],
                e.runner.alpha * e.color[3],
            ];
            if e.spell_bubble {
                // Draw3: the bubble lives at a 3D world point. Project its centre
                // and a half-width/height offset through the stage camera, so the
                // on-screen size carries the camera's perspective scale.
                let hw_w = sp.width * e.runner.scale[0].abs() / 2.0;
                let hh_w = sp.height * e.runner.scale[1].abs() / 2.0;
                let (Some(c), Some(rx), Some(by)) = (
                    Background::project_point(e.pos),
                    Background::project_point([e.pos[0] + hw_w, e.pos[1], e.pos[2]]),
                    Background::project_point([e.pos[0], e.pos[1] + hh_w, e.pos[2]]),
                ) else {
                    continue;
                };
                let hw = ((rx[0] - c[0]).powi(2) + (rx[1] - c[1]).powi(2)).sqrt();
                let hh = ((by[0] - c[0]).powi(2) + (by[1] - c[1]).powi(2)).sqrt();
                cmds.push(DrawCmd {
                    tex: self.eff_tex,
                    dst: [FIELD_X + c[0] - hw, FIELD_Y + c[1] - hh, hw * 2.0, hh * 2.0],
                    src,
                    tint,
                    rot: e.runner.rotation,
                    additive: e.runner.additive,
                    color_add: false,
                });
                continue;
            }
            let w = sp.width * e.runner.scale[0].abs();
            let h = sp.height * e.runner.scale[1].abs();
            cmds.push(DrawCmd {
                tex: self.eff_tex,
                dst: [
                    FIELD_X + e.pos[0] - w / 2.0,
                    FIELD_Y + e.pos[1] - h / 2.0,
                    w,
                    h,
                ],
                src,
                tint,
                rot: e.runner.rotation,
                additive: e.runner.additive,
                color_add: false,
            });
        }

        // BulletManager draws the original size/type groups in four passes.
        // Spawn and cancellation states select their own ANM, not the bullet's
        // normal sprite; huge bubbles use etama4 rather than etama3.
        let [tw, th] = self.bullet_tex_size;
        for group in 0..4 {
            for bullet in &self.world.bullets {
                let kind = bullet.type_index;
                let draw_group = if bullet.height > 16.0 {
                    0
                } else if bullet.height == 16.0 && matches!(kind, 1 | 3) {
                    1
                } else if bullet.height == 16.0 {
                    2
                } else {
                    3
                };
                if draw_group != group {
                    continue;
                }
                let visual = bullet.visual();
                if visual.is_some_and(|runner| !runner.visible()) {
                    continue;
                }
                let index = visual
                    .and_then(|runner| runner.sprite)
                    .unwrap_or(bullet.sprite);
                let effect_texture = bullet.visuals.as_ref().is_some_and(|v| v.effect_texture);
                let (sprites, tex, [sheet_w, sheet_h]) = if effect_texture {
                    (&self.eff_sprites, self.eff_tex, self.eff_tex_size)
                } else {
                    (&self.bullet_sprites, TEX_BULLET, [tw, th])
                };
                let Some(sprite) = sprites.get(&index) else {
                    continue;
                };
                let scale = visual.map_or([1.0; 2], |runner| runner.scale);
                let width = sprite.width * scale[0].abs();
                let height = sprite.height * scale[1].abs();
                let rotate =
                    visual.map_or(self.bullet_autorotate[kind], |runner| runner.auto_rotate);
                let rotation = if rotate {
                    bullet.angle - std::f32::consts::FRAC_PI_2
                } else {
                    visual.map_or(0.0, |runner| -runner.rotation)
                };
                let mut src = [
                    sprite.x / sheet_w,
                    sprite.y / sheet_h,
                    (sprite.x + sprite.width) / sheet_w,
                    (sprite.y + sprite.height) / sheet_h,
                ];
                if visual.is_some_and(|runner| runner.flip_x) {
                    src.swap(0, 2);
                }
                if visual.is_some_and(|runner| runner.flip_y) {
                    src.swap(1, 3);
                }
                cmds.push(DrawCmd {
                    tex,
                    dst: [
                        FIELD_X + bullet.pos[0] - width / 2.0,
                        FIELD_Y + bullet.pos[1] - height / 2.0,
                        width,
                        height,
                    ],
                    src,
                    tint: [1.0, 1.0, 1.0, visual.map_or(1.0, |runner| runner.alpha)],
                    rot: rotation,
                    additive: visual.is_some_and(|runner| runner.additive),
                    color_add: false,
                });
            }
        }

        if let Some(asset) = &self.pause_asset {
            self.score_popups
                .draw(&mut cmds, &asset.entry, asset.tex, [FIELD_X, FIELD_Y]);
        }

        // Boss attack timer at the field top-right, "%.2d" coloured by seconds
        // left (Gui.cpp:1007-1037, COLOR1-4).
        if let Some(secs) = self
            .enemies
            .iter()
            .find(|e| e.is_boss && e.occupied)
            .and_then(|e| e.spell_seconds_left())
        {
            let secs = secs.min(99);
            let tint = if secs >= 20 {
                [0.627, 0.816, 1.0, 1.0] // COLOR1 0xa0d0ff
            } else if secs >= 10 {
                [0.627, 0.502, 1.0, 1.0] // COLOR2 0xa080ff
            } else if secs >= 5 {
                [0.878, 0.502, 0.753, 1.0] // COLOR3 0xe080c0
            } else {
                [1.0, 0.251, 0.251, 1.0] // COLOR4 0xff4040
            };
            draw_text(
                &mut cmds,
                [FIELD_X + FIELD_W - 28.0, FIELD_Y + 8.0],
                16.0,
                tint,
                &format!("{secs:02}"),
            );
        }

        if let Some(banner) = &self.bomb_banner
            && let Some(runner) = &banner.portrait
        {
            self.draw_portrait(runner, &mut cmds);
        }

        // Boss spellcard portrait (enemySpellcardPortrait): the boss face sprite
        // selected by spellcardSprite, slid in from (480,240) to (320,240) and
        // faded out by face*.anm script 3.
        if let Some(r) = &self.portrait_runner {
            self.draw_portrait(r, &mut cmds);
        }

        if let Some(banner) = &self.bomb_banner
            && let Some(runner) = &banner.name
        {
            let (tex, len) = banner.textures[self.bomb_kind as usize];
            self.draw_spell_banner(runner, tex, len, true, &mut cmds);
        }
        if let Some(runner) = &self.spell_name_runner
            && let Some((tex, _, _)) = self.spell_name_cur
        {
            self.draw_spell_banner(runner, tex, self.spell_name_byte_len, false, &mut cmds);
        }

        // Stage-name intro (text.anm script 0): the STD stage title, lower-left,
        // fading with the VM. The VM pos is playfield-relative; y follows it.
        if let Some(r) = &self.stage_name_runner
            && r.visible()
            && let Some((slot, tw, th)) = self.stage_name_tex
        {
            let x = FIELD_X + 24.0;
            let y = FIELD_Y + r.pos[1].max(0.0);
            cmds.push(DrawCmd {
                tex: slot,
                dst: [x, y, tw, th],
                src: [0.0, 0.0, 1.0, 1.0],
                tint: [1.0, 1.0, 1.0, r.alpha],
                rot: 0.0,
                additive: false,
                color_add: false,
            });
        }

        // Gui draws the original portrait VMs and the dialogue box even before
        // the first text line. Stage 6's post-boss message only drives results.
        if self.dialogue.active && !(self.stage_num == 6 && matches!(self.dialogue.msg_idx, 1 | 11))
        {
            for portrait in self.dialogue.portraits.iter().flatten() {
                self.draw_portrait(portrait, &mut cmds);
            }
            let height = (self.dialogue.timer.min(60) as f32) * 48.0 / 60.0;
            // Gui.cpp uses a vertical alpha gradient (0xd0 -> 0x90).
            for y in 0..height.ceil() as u32 {
                let alpha = (208.0 - 64.0 * y as f32 / height.max(1.0)) / 255.0;
                cmds.push(rect(
                    [80.0, 384.0 + y as f32, 288.0, 1.0f32.min(height - y as f32)],
                    [0.0, 0.0, 0.0, alpha],
                ));
            }
            for line in 0..2 {
                self.draw_dialogue_line(
                    &mut cmds,
                    self.dialogue.line_runners[line].as_ref(),
                    self.dialogue.line_tex[line],
                    &self.dialogue.lines[line],
                    self.dialogue.line_colors[line],
                    [96.0, 388.0 + line as f32 * 24.0],
                );
                self.draw_dialogue_line(
                    &mut cmds,
                    self.dialogue.intro_runners[line].as_ref(),
                    self.dialogue.intro_tex[line],
                    &self.dialogue.intro_lines[line],
                    self.dialogue.intro_colors[line],
                    [112.0, 336.0 + line as f32 * 16.0],
                );
            }
        }

        // ScreenEffect shake: the 2D playfield rides the arcade region's
        // top-left (a translate). Everything pushed so far is field-relative;
        // the HUD (draw_hud, absolute 640x480) is appended after and stays put.
        let dx = self.arcade[0] - FIELD_X;
        let dy = self.arcade[1] - FIELD_Y;
        if dx != 0.0 || dy != 0.0 {
            for c in &mut cmds {
                c.dst[0] += dx;
                c.dst[1] += dy;
            }
        }

        self.draw_hud(&mut cmds);
        cmds
    }

    /// Roll the displayed score toward the real score (GameManager guiScore).
    fn roll_gui_score(&mut self) {
        // GameManager_OnUpdate clamps score before rolling the display.
        if self.score > 999_999_999 {
            self.score = 999_999_990;
        }
        if self.gui_score == self.score {
            return;
        }
        if self.score < self.gui_score {
            self.score = self.gui_score;
        }
        let mut inc = (self.score - self.gui_score) >> 5;
        inc = inc.clamp(10, 78910);
        inc -= inc % 10;
        if self.next_score_inc < inc {
            self.next_score_inc = inc;
        }
        if self.gui_score + self.next_score_inc > self.score {
            self.next_score_inc = self.score - self.gui_score;
        }
        self.gui_score += self.next_score_inc;
        if self.gui_score >= self.score {
            self.next_score_inc = 0;
            self.gui_score = self.score;
        }
        // One threshold per frame, checked against the rolling score, even if
        // the life stock is already full. Extra starts at the unreachable fifth
        // threshold (GameManager_AddedCallback / GameManager_OnUpdate).
        if let Some(&threshold) = EXTEND_SCORES.get(self.next_extend)
            && self.gui_score >= threshold
        {
            if self.lives < 8 {
                self.lives += 1;
                self.events.push(Event::SfxIdx(28));
            }
            self.next_extend += 1;
            self.world.increase_subrank(200);
        }
        self.hiscore = self.hiscore.max(self.gui_score);
    }

    /// Ease the boss health bar toward the boss's life fraction (Gui
    /// bossHealthBar2 toward bossHealthBar1): up 0.01/frame, down 0.02/frame.
    fn roll_boss_bar(&mut self) {
        let target = self
            .enemies
            .iter()
            .find(|e| e.is_boss && e.occupied)
            .map(|b| (b.life.max(0) as f32 / b.max_life.max(1) as f32).clamp(0.0, 1.0));
        match target {
            Some(t) => {
                if self.boss_bar < t {
                    self.boss_bar = (self.boss_bar + 0.01).min(t);
                } else if self.boss_bar > t {
                    self.boss_bar = (self.boss_bar - 0.02).max(t);
                }
            }
            None => self.boss_bar = 0.0,
        }
    }

    /// Boss UI over the field (Gui::DrawGameScene): the front.anm health bar,
    /// the remaining-attack count, the spellcard timer, and the spell name.
    /// Positions are the decomp's arcade-region coords plus the field origin.
    fn draw_boss_ui(&self, cmds: &mut Vec<DrawCmd>) {
        let Some(boss) = self.enemies.iter().find(|e| e.is_boss && e.occupied) else {
            return;
        };

        let ts = self.hud.tex_size();

        // "Enemy" label (script 19), self-placed by its own script.
        if let Some(([sx, sy, sw, sh], pos, scale, alpha)) = self.hud.script_state(19) {
            cmds.push(DrawCmd {
                tex: self.hud.tex(),
                dst: [
                    FIELD_X + pos[0] - sw * scale[0] / 2.0,
                    FIELD_Y + pos[1] - sh * scale[1] / 2.0,
                    sw * scale[0],
                    sh * scale[1],
                ],
                src: [sx / ts, sy / ts, (sx + sw) / ts, (sy + sh) / ts],
                tint: [1.0, 1.0, 1.0, alpha],
                rot: 0.0,
                additive: false,
                color_add: false,
            });
        }

        // Health bar (script 21): top-left anchored at field (96, 24), width
        // `bossHealthBar2 * 288` (the decomp's scaleX*14), keeping the script's
        // own scaleY (0.3 -> ~4px tall) and fade-in alpha.
        if let Some(([sx, sy, sw, sh], _, scale, alpha)) = self.hud.script_state(21) {
            cmds.push(DrawCmd {
                tex: self.hud.tex(),
                dst: [
                    FIELD_X + 96.0,
                    FIELD_Y + 24.0,
                    self.boss_bar * 288.0,
                    sh * scale[1],
                ],
                src: [sx / ts, sy / ts, (sx + sw) / ts, (sy + sh) / ts],
                tint: [1.0, 1.0, 1.0, alpha],
                rot: 0.0,
                additive: false,
                color_add: false,
            });
        }

        // Remaining-attack count (eclSetLives) at field (80, 16), yellow.
        // (The spell timer and name banner are drawn by the field pass.)
        if boss.spell_count > 0 {
            draw_text(
                cmds,
                [FIELD_X + 76.0, FIELD_Y + 12.0],
                14.0,
                [1.0, 1.0, 0.5, 1.0],
                &boss.spell_count.to_string(),
            );
        }
    }

    fn draw_hud(&self, cmds: &mut Vec<DrawCmd>) {
        let val = [1.0, 1.0, 1.0, 1.0];
        let vx = 496.0; // value column (Gui.cpp)

        // Border frame from front.anm tiles (Gui.cpp:1046-1074): vms[6] left
        // column + right block, vms[7] top row, vms[8] bottom row.
        let mut y = 0.0;
        while y < 464.0 {
            self.hud.draw_sprite(cmds, 6, 0.0, y, 1.0);
            y += 32.0;
        }
        let mut x = 416.0;
        while x < 624.0 {
            let mut y = 0.0;
            while y < 464.0 {
                self.hud.draw_sprite(cmds, 6, x, y, 1.0);
                y += 32.0;
            }
            x += 32.0;
        }
        let mut x = 32.0;
        while x < 416.0 {
            self.hud.draw_sprite(cmds, 7, x, 0.0, 1.0);
            self.hud.draw_sprite(cmds, 8, x, 464.0, 1.0);
            x += 32.0;
        }

        // Self-placed front.anm labels (vms[9-15]) + rotating emblems (vms[0-5]).
        self.hud.draw(cmds);

        // Value-row plates (vms[22]) behind each value (Gui.cpp:1096-1130).
        for &py in &[58.0, 82.0, 122.0, 146.0, 186.0, 206.0, 226.0] {
            self.hud.draw_sprite(cmds, 22, vx, py, 1.0);
        }
        self.hud.draw_sprite(cmds, 22, 488.0, 464.0, 1.0);
        self.hud.draw_sprite(cmds, 22, 0.0, 464.0, 1.0);

        // HiScore (y58) and rolling Score (y82), "%.9d" (Gui.cpp:1205-1208).
        draw_num(cmds, [vx, 58.0], val, &format!("{:09}", self.hiscore));
        draw_num(cmds, [vx, 82.0], val, &format!("{:09}", self.gui_score));

        // Lives (vms[16]) / bombs (vms[17]) stars, x=496 + idx*16.
        for i in 0..self.lives.max(0) {
            self.hud
                .draw_sprite(cmds, 16, vx + i as f32 * 16.0, 122.0, 1.0);
        }
        for i in 0..self.bombs.max(0) {
            self.hud
                .draw_sprite(cmds, 17, vx + i as f32 * 16.0, 146.0, 1.0);
        }

        // Power bar: the decomp's gradient quad, width = currentPower px
        // (Gui.cpp:1152-1198), 0xe0e0e0 -> 0x80e0e0. Drawn as the 2x1 gradient
        // texture sampled at its texel centres (0.25..0.75) so the endpoints are
        // exact and the GPU linearly interpolates between them. Then the MAX
        // sprite (vms[18]) at full, else the numeric value, both at (496,186).
        let power = self.world.power.max(0);
        if power > 0 {
            cmds.push(DrawCmd {
                tex: TEX_POWER_GRAD,
                dst: [vx, 186.0, power as f32, 16.0],
                src: [0.25, 0.5, 0.75, 0.5],
                tint: [1.0, 1.0, 1.0, 1.0],
                rot: 0.0,
                additive: false,
                color_add: false,
            });
        }
        if power >= 128 {
            self.hud.draw_sprite(cmds, 18, vx, 186.0, 1.0);
        } else {
            draw_num(cmds, [vx, 186.0], val, &format!("{power}"));
        }

        // Graze (y206) and point items (y226), "%d".
        draw_num(cmds, [vx, 206.0], val, &format!("{}", self.graze));
        draw_num(cmds, [vx, 226.0], val, &format!("{}", self.point_items));

        // "Full Power Mode!!" popup (Gui.cpp:185-190,908-922): pale blue, slides
        // in from the right edge to x=104 over 30 frames, shown 180 frames.
        if self.full_power_timer > 0 {
            let elapsed = 180 - self.full_power_timer.min(180);
            let px = if elapsed < 30 {
                640.0 - elapsed as f32 * 312.0 / 30.0
            } else {
                104.0
            };
            draw_num(
                cmds,
                [px, 232.0],
                [0.753, 0.690, 1.0, 1.0],
                "Full Power Mode!!",
            );
        }

        // "BONUS %8d" popup (Gui.cpp:179-183,891-906): light yellow, slides in
        // from the right edge to x=104, y=32.
        if self.bonus_score_timer > 0 {
            let elapsed = 250 - self.bonus_score_timer.min(250);
            let px = if elapsed < 30 {
                640.0 - elapsed as f32 * 312.0 / 30.0
            } else {
                104.0
            };
            draw_num(
                cmds,
                [px, 32.0],
                [1.0, 1.0, 0.502, 1.0],
                &format!("BONUS {:8}", self.bonus_score_amount),
            );
        }

        // "Spell Card Bonus!" + "+N" popup (Gui.cpp:192-210), centred near top.
        if self.spell_bonus_timer > 0 {
            let title = "Spell Card Bonus!";
            let tx = (FIELD_W - title.len() as f32 * 16.0) / 2.0 + FIELD_X;
            draw_num(cmds, [tx, FIELD_Y + 64.0], [1.0, 0.0, 0.0, 1.0], title);
            let amt = format!("+{}", self.spell_bonus_amount);
            let ax = (FIELD_W - amt.len() as f32 * 32.0) / 2.0 + FIELD_X;
            draw_num_scaled(
                cmds,
                [ax, FIELD_Y + 80.0],
                2.0,
                [1.0, 0.502, 0.502, 1.0],
                &amt,
            );
        }

        self.draw_boss_ui(cmds);

        match self.state {
            PlayerState::GameOver => {
                cmds.push(rect(
                    [FIELD_X, FIELD_Y, FIELD_W, FIELD_H],
                    [0.0, 0.0, 0.0, 0.6],
                ));
            }
            PlayerState::Cleared(_) | PlayerState::Results => {
                // Stage-clear bonus breakdown (Gui.cpp:88-172), drawn over the
                // field at (GAME_REGION_LEFT+42, TOP+112).
                let x = FIELD_X + 42.0;
                let graze_in = self.graze_in_stage;
                let (diff_text, _) = match self.world.difficulty {
                    0 => ("Easy Rank      * 0.5", 0),
                    2 => ("Hard Rank      * 1.2", 0),
                    3 => ("Lunatic Rank   * 1.5", 0),
                    4 => ("Extra Rank     * 2.0", 0),
                    _ => ("Normal Rank    * 1.0", 0),
                };
                draw_num(
                    cmds,
                    [x, FIELD_Y + 112.0],
                    [1.0, 1.0, 0.251, 1.0],
                    "Stage Clear",
                );
                draw_num(
                    cmds,
                    [x, FIELD_Y + 144.0],
                    [1.0, 1.0, 1.0, 1.0],
                    &format!("Stage * 1000 = {:5}", self.stage_num as i64 * 1000),
                );
                draw_num(
                    cmds,
                    [x, FIELD_Y + 160.0],
                    [0.878, 0.878, 1.0, 1.0],
                    &format!("Power *  100 = {:5}", self.world.power as i64 * 100),
                );
                draw_num(
                    cmds,
                    [x, FIELD_Y + 176.0],
                    [0.816, 0.816, 1.0, 1.0],
                    &format!("Graze *   10 = {:5}", graze_in * 10),
                );
                draw_num(
                    cmds,
                    [x, FIELD_Y + 192.0],
                    [1.0, 0.502, 0.502, 1.0],
                    &format!("    * Point Item {:3}", self.point_items),
                );
                draw_num(
                    cmds,
                    [x, FIELD_Y + 224.0],
                    [1.0, 0.502, 0.502, 1.0],
                    diff_text,
                );
                draw_num(
                    cmds,
                    [x, FIELD_Y + 240.0],
                    [1.0, 1.0, 1.0, 1.0],
                    &format!("Total     = {:8}", self.clear_bonus),
                );
            }
            _ => {}
        }

        if self.paused
            && let Some(asset) = &self.pause_asset
        {
            for runner in self.pause_runners.iter().flatten() {
                if !runner.visible() {
                    continue;
                }
                let Some(sprite) = runner.sprite.and_then(|idx| {
                    asset
                        .entry
                        .sprites
                        .iter()
                        .find(|sprite| sprite.index == idx)
                }) else {
                    continue;
                };
                let w = sprite.width * runner.scale[0];
                let h = sprite.height * runner.scale[1];
                let x = runner.pos[0] + runner.pos_offset[0];
                let y = runner.pos[1] + runner.pos_offset[1];
                let (tw, th) = (asset.tex_size.0 as f32, asset.tex_size.1 as f32);
                cmds.push(DrawCmd {
                    tex: asset.tex,
                    dst: [x - w / 2.0, y - h / 2.0, w, h],
                    src: [
                        sprite.x / tw,
                        sprite.y / th,
                        (sprite.x + sprite.width) / tw,
                        (sprite.y + sprite.height) / th,
                    ],
                    tint: [
                        runner.color[0],
                        runner.color[1],
                        runner.color[2],
                        runner.alpha,
                    ],
                    rot: 0.0,
                    additive: false,
                    color_add: false,
                });
            }
        }
    }
}

/// Stage-clear bonus, an exact port of `Gui.cpp:935-963`:
/// `(stage*1000 + grazeInStage*10 + power*100) * pointItems`, then (final stage
/// only) `+lives*3M + bombs*1M`, then the difficulty multiplier with a `-= %10`
/// round-down. `difficulty`: 0 Easy, 1 Normal, 2 Hard, 3 Lunatic, 4 Extra.
#[allow(clippy::too_many_arguments)] // Original data-table/function argument layout.
fn stage_clear_bonus(
    stage: i32,
    graze_in_stage: i64,
    power: i64,
    point_items: i64,
    lives: i64,
    bombs: i64,
    difficulty: u8,
    starting_spare_lives: i32,
) -> i64 {
    let mut s = stage as i64 * 1000;
    s += graze_in_stage * 10;
    s += power * 100;
    s *= point_items;
    if stage >= 6 {
        s += lives * 3_000_000;
        s += bombs * 1_000_000;
    }
    s = match difficulty {
        0 => s / 2,       // Easy
        2 => s * 12 / 10, // Hard
        3 => s * 15 / 10, // Lunatic
        4 => s * 2,       // Extra
        _ => s,           // Normal: no multiplier
    };
    if difficulty != Difficulty::Normal as u8 {
        s -= s % 10;
    }
    // Gui::OnUpdate uses defaultConfig.lifeCount (1.02h), captured at the
    // beginning of the run, rather than the player's remaining life stock.
    match starting_spare_lives {
        3 => s = s * 5 / 10,
        4 => s = s * 2 / 10,
        _ => return s,
    }
    s - s % 10
}

/// Draw HUD numbers/text with the original AsciiManager metrics: a 15px glyph
/// advancing 14px per character (`charWidth = 14 * scale.x`, AsciiManager.cpp).
fn draw_num(cmds: &mut Vec<DrawCmd>, pos: [f32; 2], tint: [f32; 4], text: &str) {
    draw_num_scaled(cmds, pos, 1.0, tint, text);
}

/// As [`draw_num`] but with an AsciiManager scale (e.g. 2.0 for the spellcard
/// bonus "+N").
fn draw_num_scaled(cmds: &mut Vec<DrawCmd>, pos: [f32; 2], scale: f32, tint: [f32; 4], text: &str) {
    let mut x = pos[0];
    for ch in text.chars() {
        let c = ch as u32;
        if (0x21..=0x7e).contains(&c) {
            let idx = c - 0x20;
            let (col, row) = ((idx % 16) as f32, (idx / 16 + 2) as f32);
            let e = 0.5 / 256.0;
            cmds.push(DrawCmd {
                tex: TEX_ASCII,
                dst: [x.round(), pos[1].round(), 15.0 * scale, 15.0 * scale],
                src: [
                    col * 16.0 / 256.0 + e,
                    row * 16.0 / 256.0 + e,
                    (col + 1.0) * 16.0 / 256.0 - e,
                    (row + 1.0) * 16.0 / 256.0 - e,
                ],
                tint,
                rot: 0.0,
                additive: false,
                color_add: false,
            });
        }
        x += 14.0 * scale;
    }
}

/// Draw ASCII text using the 16x16 glyph grid in ascii.png.
pub fn draw_text(cmds: &mut Vec<DrawCmd>, pos: [f32; 2], size: f32, tint: [f32; 4], text: &str) {
    let mut x = pos[0];
    for ch in text.chars() {
        let c = ch as u32;
        if (0x21..=0x7e).contains(&c) {
            let idx = c - 0x20;
            // Glyph grid: rows 0-1 hold kanji counters/labels, ASCII printable
            // starts at row 2 (space at 0x20).
            let (col, row) = ((idx % 16) as f32, (idx / 16 + 2) as f32);
            // Inset by half a texel so linear filtering does not bleed the
            // neighbouring glyph cell at the exact boundary.
            let e = 0.5 / 256.0;
            cmds.push(DrawCmd {
                tex: TEX_ASCII,
                dst: [x.round(), pos[1].round(), size, size],
                src: [
                    col * 16.0 / 256.0 + e,
                    row * 16.0 / 256.0 + e,
                    (col + 1.0) * 16.0 / 256.0 - e,
                    (row + 1.0) * 16.0 / 256.0 - e,
                ],
                tint,
                rot: 0.0,
                additive: false,
                color_add: false,
            });
        }
        x += size * 0.75;
    }
}

fn rect(dst: [f32; 4], tint: [f32; 4]) -> DrawCmd {
    DrawCmd {
        tex: TEX_WHITE,
        dst,
        src: [0.25, 0.25, 0.75, 0.75],
        tint,
        rot: 0.0,
        additive: false,
        color_add: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stage(difficulty: Difficulty, practice: bool, number: i32, lives: i32) -> Stage {
        let entry = Entry {
            width: 256,
            height: 256,
            format: 0,
            name: String::new(),
            alpha_name: None,
            sprites: Vec::new(),
            scripts: Vec::new(),
        };
        Stage::new(
            Ecl {
                data: Vec::new(),
                sub_offsets: Vec::new(),
                timeline_offset: 0,
            },
            HashMap::new(),
            &entry,
            &entry,
            0,
            &entry,
            0,
            Character::ReimuA,
            Msg {
                data: Vec::new(),
                offsets: Vec::new(),
            },
            None,
            Hud::new(&entry, 0),
            StageConfig {
                face_player_tex: 0,
                face_boss_tex: 0,
                stage_bgm: "",
                boss_bgm: "",
                stage_num: number,
                difficulty,
                practice,
                starting_lives: lives,
                starting_bombs: 2,
                rng_seed: 0x1234,
            },
            Vec::new(),
            Vec::new(),
            HashMap::new(),
            HashMap::new(),
            Vec::new(),
            None,
        )
    }

    fn attack_shot() -> Shot {
        Shot {
            pool_index: 0,
            pos: [100.0; 2],
            size: [12.0; 2],
            vel: [2.0, -12.0],
            damage: 48,
            bt: 0,
            anm_script: 64,
            runner: AnmRunner::new(Vec::new()),
            collided: false,
            angle: 0.0,
            motion: [0.0; 2],
            spawn: 0,
            laser_slot: 0,
            age: 0,
            spd: 0.0,
        }
    }

    fn player_reference(mode: &str) -> String {
        let binary = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../oracle/player/build/player_reference");
        let output = std::process::Command::new(binary)
            .arg(mode)
            .output()
            .expect("build oracle/player/build.py first");
        assert!(output.status.success());
        String::from_utf8(output.stdout).unwrap()
    }

    #[test]
    fn player_opposing_keys_and_master_spark_diagonal_match_original_order() {
        let mut s = stage(Difficulty::Normal, false, 1, 3);
        s.character = Character::MarisaB;
        s.pos = [100.0; 2];
        s.bomb_move_mult = 0.3;
        s.update_player(&Input::from_buttons(240, 0));
        let speed = 5.0 / 2.0f32.sqrt();
        assert_eq!(s.pos, [100.0 + speed * 0.3, 100.0 - speed * 0.3]);
        assert_eq!(s.prev_hspeed, speed);
    }

    #[test]
    #[ignore = "requires original decomp; build with TH06_DECOMP_SRC=... python3 oracle/player/build.py"]
    fn original_cpp_player_movement_matches_all_input_focus_and_bomb_combinations() {
        use std::fmt::Write;
        let mut actual = String::new();
        for scenario in 0..8 {
            for directions in 0..16 {
                let mut s = stage(Difficulty::Normal, false, 1, 3);
                s.character = if scenario & 4 != 0 {
                    Character::MarisaB
                } else {
                    Character::ReimuA
                };
                s.pos = [100.0; 2];
                s.bomb_move_mult = if scenario & 2 != 0 { 0.3 } else { 1.0 };
                s.player_scripts = (0..5).map(|id| (id, Vec::new())).collect();
                s.update_player(&Input::from_buttons(
                    directions * 16 + if scenario & 1 != 0 { 4 } else { 0 },
                    0,
                ));
                writeln!(
                    actual,
                    "{scenario} {directions} {} {} {} {}",
                    s.pos[0].to_bits(),
                    s.pos[1].to_bits(),
                    s.prev_hspeed.to_bits(),
                    s.player_script_id
                )
                .unwrap();
            }
        }
        assert_eq!(actual, player_reference("movement"));
    }

    #[test]
    fn player_respawn_and_shot_bounds_use_the_shaken_game_region() {
        let mut s = stage(Difficulty::Normal, false, 1, 3);
        s.arcade = [FIELD_X + 4.0, FIELD_Y + 4.0, FIELD_W - 4.0, FIELD_H - 4.0];
        s.state = PlayerState::Dead(30);
        s.dying = 0;
        s.update_player_lifecycle(&Input::default());
        assert_eq!(s.pos, [190.0, 380.0]);
        let mut shot = attack_shot();
        shot.pos = [390.0, 100.0];
        shot.vel = [0.0; 2];
        shot.runner.set_sprite(0);
        s.shots.push(shot);
        s.update_shots();
        assert!(s.shots.is_empty());
        assert!(player_shot_in_bounds(
            [390.0, 100.0],
            [12.0; 2],
            [384.0, 448.0]
        ));
        assert!(player_shot_in_bounds(
            [f32::NAN, 100.0],
            [12.0; 2],
            [384.0, 448.0]
        ));
    }

    #[test]
    #[ignore = "requires original decomp; build with TH06_DECOMP_SRC=... python3 oracle/player/build.py"]
    fn original_cpp_player_bounds_match_edges_shake_and_nan_positions() {
        use std::fmt::Write;
        let cases = [
            [-6.0, 100.0, 12.0, 12.0, 384.0, 448.0],
            [-6.001, 100.0, 12.0, 12.0, 384.0, 448.0],
            [390.0, 100.0, 12.0, 12.0, 384.0, 448.0],
            [390.001, 100.0, 12.0, 12.0, 384.0, 448.0],
            [100.0, -6.0, 12.0, 12.0, 384.0, 448.0],
            [100.0, -6.001, 12.0, 12.0, 384.0, 448.0],
            [100.0, 454.0, 12.0, 12.0, 384.0, 448.0],
            [100.0, 454.001, 12.0, 12.0, 384.0, 448.0],
            [390.0, 100.0, 12.0, 12.0, 380.0, 444.0],
            [100.0, 454.0, 12.0, 12.0, 380.0, 444.0],
            [f32::NAN, 100.0, 12.0, 12.0, 384.0, 448.0],
            [100.0, f32::NAN, 12.0, 12.0, 384.0, 448.0],
        ];
        let mut actual = String::new();
        for (idx, [x, y, w, h, rw, rh]) in cases.into_iter().enumerate() {
            let expected = player_shot_in_bounds([x, y], [w, h], [rw, rh]);
            let mut enemy = Enemy::default();
            enemy.pos = [100.0, 100.0, 0.0];
            assert!(enemy.update_bounds(w, h, [rw, rh]));
            enemy.pos = [x, y, 0.0];
            assert_eq!(
                enemy.update_bounds(w, h, [rw, rh]),
                expected,
                "enemy bounds {idx}"
            );
            writeln!(actual, "{idx} {}", i32::from(expected)).unwrap();
        }
        assert_eq!(actual, player_reference("bounds"));
    }

    #[test]
    fn player_attack_boxes_are_inclusive_and_missile_explosions_keep_damaging() {
        let mut s = stage(Difficulty::Normal, false, 1, 3);
        s.shots.push(attack_shot());
        assert_eq!(s.calc_player_damage([114.0, 114.0, 0.0], [16.0; 2]).0, 48);
        assert!(s.shots[0].collided);
        assert_eq!(s.calc_player_damage([114.0, 114.0, 0.0], [16.0; 2]).0, 0);
        s.shots.clear();
        let mut missile = attack_shot();
        missile.bt = 2;
        missile.anm_script = 66;
        missile.damage = 18;
        s.shots.push(missile);
        let hits = (0..4)
            .map(|_| s.calc_player_damage([100.0, 100.0, 0.0], [16.0; 2]).0)
            .collect::<Vec<_>>();
        assert_eq!(hits, [18, 4, 1, 1]);
        assert_eq!(s.shots[0].size, [42.0; 2]);
        assert_eq!(s.shots[0].anm_script, 98);
        assert_eq!(s.shots[0].vel, [2.0 / 4096.0, -12.0 / 4096.0]);
    }

    #[test]
    fn player_shot_pool_reuses_first_slot_and_beams_outlive_released_fire() {
        let mut s = stage(Difficulty::Normal, false, 1, 3);
        s.character = Character::MarisaB;
        s.world.power = 8;
        s.update_orbs(false);
        s.spawn_player_bullets(0);
        assert_eq!(s.shots.len(), 3);
        assert_eq!(s.laser_timers, [120; 2]);
        assert_eq!(s.shots.iter().filter(|shot| shot.bt == 3).count(), 2);
        s.update_shots();
        s.update_player_fire(&Input::default());
        assert_eq!(s.shots.iter().filter(|shot| shot.bt == 3).count(), 2);
        assert_eq!(s.laser_timers, [119; 2]);
        s.shots.remove(0);
        s.spawn_player_bullets(5);
        assert_eq!(s.shots[0].pool_index, 0);
        s.shots = (0..80)
            .map(|pool_index| {
                let mut shot = attack_shot();
                shot.pool_index = pool_index;
                shot
            })
            .collect();
        s.spawn_player_bullets(0);
        assert_eq!(s.shots.len(), 80);
        s.shots.clear();
        s.bomb_in_use = true;
        s.fire_timer = 0;
        s.state = PlayerState::Alive;
        s.update_player_fire(&Input::from_buttons(1, 0));
        assert!(s.shots.is_empty());
        assert_eq!(s.fire_timer, 1);
    }

    #[test]
    fn orb_focus_uses_squared_easing_and_original_reverse_transition() {
        let mut s = stage(Difficulty::Normal, false, 1, 3);
        s.world.power = 8;
        s.update_orbs(true);
        assert_eq!(s.orb_offsets, [23.75, -4.0]);
        for _ in 0..3 {
            s.update_orbs(true);
        }
        assert_eq!(s.orb_offsets, [20.0, -16.0]);
        s.update_orbs(false);
        assert_eq!(s.orb_offsets, [12.0, -16.0]);
        for _ in 0..4 {
            s.update_orbs(false);
        }
        assert_eq!(s.orb_offsets, [24.0, 0.0]);
    }

    #[test]
    #[ignore = "requires original decomp; build with TH06_DECOMP_SRC=... python3 oracle/player/build.py"]
    fn original_cpp_player_damage_matches_rectangles_explosions_beams_and_bombs() {
        use std::fmt::Write;
        let mut actual = String::new();
        for scenario in 0..18 {
            let mut s = stage(Difficulty::Normal, false, 1, 3);
            for (script, effect) in [(6, 3u32), (10, 5u32)] {
                s.world.eff_scripts.insert(
                    script,
                    vec![AnmInstr {
                        time: 0,
                        opcode: 1,
                        args: effect.to_le_bytes().to_vec(),
                        offset: 0,
                    }],
                );
            }
            let mut b = attack_shot();
            let mut pos = [100.0, 100.0, 0.0];
            match scenario {
                1 => pos[0] = 114.0,
                2 => pos[0] = 114.001,
                3 => pos = [114.0, 114.0, 0.0],
                4 => {
                    b.size[1] = 48.0;
                    pos[1] = 132.0;
                    b.anm_script = 66;
                }
                5 => b.collided = true,
                6 => {
                    b.collided = true;
                    b.bt = 1;
                }
                7..=10 => {
                    b.bt = 2;
                    b.anm_script = scenario - 7 + 65;
                    b.damage = 18 - (scenario - 7);
                    b.age = match scenario {
                        7 => 0,
                        8 => 1,
                        9 => 6,
                        _ => 5,
                    };
                }
                11 => {
                    b.bt = 2;
                    b.collided = true;
                    b.anm_script = 97;
                    b.damage = 4;
                    b.age = 6;
                    b.size = [32.0; 2];
                }
                12 => {
                    b.bt = 3;
                    b.damage = 3;
                    s.player_particle_timer = 7;
                }
                13 => s.bomb_in_use = true,
                14 => {
                    s.bomb_in_use = true;
                    b.bt = 2;
                    b.damage = 2;
                    b.anm_script = 65;
                }
                15..=17 => {
                    b.pos[0] = 500.0;
                    s.bomb_regions[0] = BombRect {
                        size: [32.0; 2],
                        pos: [100.0; 2],
                    };
                    s.bomb_region_dmg[0] = 8;
                    s.player_particle_timer = 3;
                    s.bomb_in_use = scenario != 16;
                    if scenario == 17 {
                        pos[0] = 125.0;
                    }
                }
                _ => {}
            }
            s.shots.push(b);
            for hit in 0..4 {
                let (damage, bomb) = s.calc_player_damage(pos, [16.0; 2]);
                let b = &s.shots[0];
                let effects = s
                    .world
                    .effects
                    .iter()
                    .flatten()
                    .map(|effect| effect.runner.sprite.unwrap())
                    .collect::<Vec<_>>();
                write!(
                    actual,
                    "{scenario} {hit} {damage} {} {} {} {} {} {} {} {} {} {} {}",
                    u8::from(bomb),
                    s.player_particle_timer,
                    if b.collided { 2 } else { 1 },
                    b.damage,
                    b.size[0].to_bits(),
                    b.size[1].to_bits(),
                    b.vel[0].to_bits(),
                    b.vel[1].to_bits(),
                    b.anm_script,
                    s.bomb_dmg_accum[0],
                    effects.len()
                )
                .unwrap();
                for effect in effects {
                    write!(actual, " {effect}").unwrap();
                }
                actual.push('\n');
            }
        }
        assert_eq!(actual, player_reference("damage"));
    }

    #[test]
    #[ignore = "requires original decomp; build with TH06_DECOMP_SRC=... python3 oracle/player/build.py"]
    fn original_cpp_player_motion_matches_homing_gravity_and_anchored_beams() {
        use std::fmt::Write;
        let mut actual = String::new();
        for scenario in 0..10 {
            let mut s = stage(Difficulty::Normal, false, 1, 3);
            let mut b = attack_shot();
            b.bt = 1;
            b.spd = 3.0;
            b.vel = [0.0, -3.0];
            match scenario {
                0 => s.last_enemy_hit = Some([110.0, 80.0]),
                1 => {
                    s.last_enemy_hit = Some([110.0, 80.0]);
                    b.age = 39;
                }
                2 => {
                    s.last_enemy_hit = Some([110.0, 80.0]);
                    b.age = 40;
                }
                3 => b.collided = true,
                4 => b.bt = 2,
                5 => {
                    b.bt = 2;
                    b.collided = true;
                }
                6..=9 => {
                    b.bt = 3;
                    b.spawn = 1;
                    b.motion = [2.0, 0.0];
                    s.orbs_pos[0] = [120.0, 320.0];
                    s.laser_timers[0] = match scenario {
                        6 => 71,
                        7 => 2,
                        8 => 1,
                        _ => 0,
                    };
                }
                _ => {}
            }
            s.shots.push(b);
            for frame in 0..4 {
                s.update_shots();
                let b = &s.shots[0];
                writeln!(
                    actual,
                    "{scenario} {frame} {} {} {} {} {} {} {} {} {}",
                    b.pos[0].to_bits(),
                    b.pos[1].to_bits(),
                    b.vel[0].to_bits(),
                    b.vel[1].to_bits(),
                    b.spd.to_bits(),
                    b.size[1].to_bits(),
                    b.runner.scale[1].to_bits(),
                    b.age,
                    s.laser_timers[0]
                )
                .unwrap();
            }
        }
        let reference = player_reference("update")
            .lines()
            .map(|line| line.rsplit_once(' ').unwrap().0.to_owned() + "\n")
            .collect::<String>();
        assert_eq!(actual, reference);
    }

    #[test]
    #[ignore = "requires original decomp; build with TH06_DECOMP_SRC=... python3 oracle/player/build.py"]
    fn original_cpp_orb_focus_matches_reversals_and_power_loss() {
        use std::fmt::Write;
        let mut s = stage(Difficulty::Normal, false, 1, 3);
        let mut actual = String::new();
        for frame in 0..40 {
            let focus = frame < 4 || (frame >= 8 && frame % 5 < 2);
            s.world.power = if (22..26).contains(&frame) { 0 } else { 8 };
            s.update_orbs(focus);
            let state = match s.orb_state {
                OrbState::Hidden => 0,
                OrbState::Unfocused => 1,
                OrbState::Focusing => 2,
                OrbState::Focused => 3,
                OrbState::Unfocusing => 4,
            };
            writeln!(
                actual,
                "{frame} {state} {} {} {} {} {}",
                s.orb_timer,
                s.orbs_pos[0][0].to_bits(),
                s.orbs_pos[0][1].to_bits(),
                s.orbs_pos[1][0].to_bits(),
                s.orbs_pos[1][1].to_bits()
            )
            .unwrap();
        }
        assert_eq!(actual, player_reference("orbs"));
    }

    #[test]
    fn item_motion_applies_velocity_before_gravity_and_interpolates_at_timer_zero() {
        let mut s = stage(Difficulty::Normal, false, 1, 3);
        s.spawn_item(Item::fall([100.0, 100.0], 1));
        s.update_items();
        assert_eq!(s.items[0].pos, [100.0, 100.0 - 2.2]);
        assert_eq!((s.items[0].vy, s.items[0].timer), (-2.2 + 0.03, 1));
        s.items.clear();
        s.spawn_scatter_item([100.0, 100.0], 0);
        let target = s.items[0].target;
        s.update_items();
        assert_eq!(s.items[0].pos, [100.0, 100.0]);
        s.update_items();
        let fraction = 1.0 / 60.0;
        assert_eq!(
            s.items[0].pos,
            [
                fraction * target[0] + 100.0 * (1.0 - fraction),
                fraction * target[1] + 100.0 * (1.0 - fraction)
            ]
        );
        s.items.clear();
        s.state = PlayerState::Alive;
        s.spawn_item(Item::fall([s.pos[0] + 20.0, s.pos[1] + 20.0], 1));
        s.update_items();
        assert!(s.items.is_empty());
    }

    #[test]
    fn cropped_shake_region_removes_missed_item_at_sixteen_pixel_margin() {
        let mut s = stage(Difficulty::Normal, false, 1, 3);
        s.arcade[3] = 442.0;
        let mut item = Item::fall([100.0, 455.0], 1);
        item.vy = 3.0;
        s.spawn_item(item);
        s.update_items();
        assert!(s.items.is_empty());

        s.arcade[3] = FIELD_H;
        let mut item = Item::fall([100.0, 455.0], 1);
        item.vy = 3.0;
        s.spawn_item(item);
        s.update_items();
        assert_eq!(s.items[0].pos[1], 458.0);
    }

    #[test]
    fn overlapping_screen_shakes_consume_independent_rng_and_expire_in_order() {
        let mut s = stage(Difficulty::Normal, false, 1, 3);
        s.world.rng = Rng::new(16878);
        s.register_shake(40, 2.0, 6.0);
        s.register_shake(2, 8.0, 0.0);
        s.update_shake();
        assert_eq!(s.world.rng.seed(), 31427); // two chains, eight u16 draws
        assert_eq!(s.shakes.len(), 2);
        s.world.time_stopped = true;
        s.update_shake();
        assert_eq!(s.world.rng.seed(), 31427);
        assert!(s.shakes.iter().all(|shake| shake.timer == 1));
        s.world.time_stopped = false;
        s.update_shake();
        assert_eq!(s.shakes.len(), 1);
        // The younger chain expires after the older chain writes its region.
        assert_eq!(s.arcade, [FIELD_X, FIELD_Y, FIELD_W, FIELD_H]);
    }

    #[test]
    #[ignore = "requires original ScreenEffect/Rng source output in TH06_SHAKE_REFERENCE"]
    fn original_screen_shake_chains_match_source_with_overlap_and_time_stop() {
        use std::fmt::Write;
        let expected =
            std::fs::read_to_string(std::env::var("TH06_SHAKE_REFERENCE").unwrap()).unwrap();
        let mut actual = String::new();
        for scenario in 0..8 {
            let mut s = stage(Difficulty::Normal, false, 1, 3);
            s.world.rng = Rng::new(16878 + scenario * 71);
            for frame in 0..60 {
                if frame == 0 {
                    s.register_shake(40, 2.0, 6.0);
                }
                if frame == 2 {
                    s.register_shake(16, 8.0, 0.0);
                }
                if frame == 4 {
                    s.register_shake(
                        if scenario & 1 != 0 { 7 } else { 16 },
                        if scenario & 2 != 0 { 20.0 } else { 8.0 },
                        0.0,
                    );
                }
                s.world.time_stopped = scenario & 4 != 0 && (8..13).contains(&frame);
                s.update_shake();
                writeln!(
                    actual,
                    "{scenario} {frame} {} {} {} {} {} {}",
                    s.world.rng.seed(),
                    s.shakes.len(),
                    s.arcade[0].to_bits(),
                    s.arcade[1].to_bits(),
                    s.arcade[2].to_bits(),
                    s.arcade[3].to_bits()
                )
                .unwrap();
            }
        }
        assert_eq!(actual, expected);
    }

    #[test]
    fn new_player_shots_stay_at_spawn_position_until_next_player_callback() {
        let mut s = stage(Difficulty::Normal, false, 1, 3);
        s.update_player_lifecycle(&Input::from_buttons(1, 0));
        assert!(!s.shots.is_empty());
        assert!(s.shots.iter().all(|shot| shot.age == 0));
        let position = s.shots[0].pos;
        let velocity = s.shots[0].vel;
        s.update_player_lifecycle(&Input::from_buttons(1, 1));
        assert_eq!(s.shots[0].age, 1);
        assert_eq!(
            s.shots[0].pos,
            [position[0] + velocity[0], position[1] + velocity[1]]
        );
    }

    #[test]
    fn enemy_damage_flash_and_sound_use_actual_hp_loss_and_four_tick_cooldown() {
        let mut s = stage(Difficulty::Normal, false, 1, 3);
        let mut enemy = Enemy::default();
        enemy.pos = [100.0, 100.0, 0.0];
        enemy.life = 100;
        enemy.collidable = false;
        enemy.update_bounds(12.0, 12.0, [FIELD_W, FIELD_H]);
        s.enemies.push(enemy);
        s.anims.push(Some(AnmRunner::new(Vec::new())));
        let mut shot = attack_shot();
        shot.bt = 2;
        shot.collided = true;
        shot.anm_script = 97;
        shot.damage = 1;
        shot.age = 1;
        s.shots.push(shot);
        for frame in 0..11 {
            s.events.clear();
            s.collide_enemy(0);
            let flash = frame % 5 == 0;
            assert_eq!(
                s.anims[0].as_ref().unwrap().color_add,
                flash,
                "frame {frame}"
            );
            assert_eq!(
                s.events
                    .iter()
                    .filter(|event| matches!(event, Event::SfxIdx(20)))
                    .count(),
                usize::from(flash)
            );
        }
        assert_eq!(s.enemies[0].life, 89);
        s.enemies[0].damageable = false;
        for _ in 0..5 {
            s.events.clear();
            s.collide_enemy(0);
            assert!(!s.anims[0].as_ref().unwrap().color_add);
            assert!(
                !s.events
                    .iter()
                    .any(|event| matches!(event, Event::SfxIdx(20)))
            );
        }
        s.enemies[0].damageable = true;
        s.events.clear();
        s.collide_enemy(0);
        assert!(s.anims[0].as_ref().unwrap().color_add);
        assert!(
            s.events
                .iter()
                .any(|event| matches!(event, Event::SfxIdx(20)))
        );
    }

    #[test]
    fn enemy_death_drops_before_bullet_callback_and_spell_disables_boss_cancel() {
        use crate::ecl_vm::{BulletProps, spawn_bullet_pattern};
        let mut s = stage(Difficulty::Normal, false, 1, 3);
        let mut enemy = Enemy::default();
        enemy.pool_index = 0;
        enemy.pos = [100.0, 100.0, 0.0];
        enemy.life = 1;
        enemy.item_drop = 1;
        enemy.is_boss = true;
        enemy.update_bounds(16.0, 16.0, [FIELD_W, FIELD_H]);
        s.enemies.push(enemy);
        s.shots.push(Shot {
            pool_index: 0,
            pos: [100.0, 100.0],
            size: [12.0; 2],
            vel: [0.0; 2],
            damage: 70,
            bt: 0,
            anm_script: 0,
            runner: AnmRunner::new(Vec::new()),
            collided: false,
            angle: 0.0,
            motion: [0.0; 2],
            spawn: 0,
            laser_slot: 0,
            age: 0,
            spd: 0.0,
        });
        spawn_bullet_pattern(
            &mut s.world,
            &BulletProps {
                count1: 1,
                count2: 1,
                ..Default::default()
            },
        );
        s.world.spellcard_active = 1;
        s.collide_enemy(0);
        assert!(!s.enemies[0].occupied);
        assert_eq!(s.items.len(), 1);
        assert_eq!(s.world.bullets[0].despawn_timer, 0);
        assert_eq!(s.score, 240); // capped70 damage /5*10 + enemy100
    }

    #[test]
    fn player_hit_deathbomb_and_respawn_follow_original_callback_frames() {
        let mut s = stage(Difficulty::Normal, false, 1, 3);
        s.update_player_lifecycle(&Input::default());
        assert!(matches!(s.state, PlayerState::Alive));
        assert_eq!(
            (s.pos, s.invuln, s.bullet_grace_period),
            ([192.0, 384.0], 239, 89)
        );
        s.world.power = 128;
        s.spell_capturing = true;
        s.register_hit();
        assert_eq!((s.deaths, s.dying, s.lives), (1, 6, 2));
        assert!(!s.spell_capturing);
        for frame in 1..=30 {
            let old_position = s.pos;
            assert!(!s.update_player_lifecycle(&Input::from_buttons(0x10, 0)));
            assert_eq!(s.pos, old_position, "movement while dead, frame {frame}");
            assert_eq!(s.lives, 2);
            if frame == 6 {
                assert_eq!((s.world.power, s.items.len()), (112, 6));
            }
        }
        s.update_player_lifecycle(&Input::default());
        assert!(matches!(s.state, PlayerState::Spawning(1)));
        assert_eq!((s.lives, s.bombs, s.bullet_grace_period), (1, 2, 89));
        for _ in 0..29 {
            s.update_player_lifecycle(&Input::default());
        }
        assert!(matches!(s.state, PlayerState::Spawning(30)));
        s.update_player_lifecycle(&Input::default());
        assert!(matches!(s.state, PlayerState::Alive));
        assert_eq!((s.invuln, s.bullet_grace_period), (239, 89));

        s.spell_capturing = true;
        s.register_hit();
        s.update_player_lifecycle(&Input::from_buttons(2, 0));
        assert!(matches!(s.state, PlayerState::Alive));
        assert_eq!((s.deaths, s.lives, s.bombs, s.dying), (2, 1, 1, 0));
        assert!(!s.spell_capturing);
    }

    #[test]
    fn fatal_hit_reports_game_over_after_death_animation_without_clearing_field() {
        let mut s = stage(Difficulty::Normal, false, 1, 1);
        s.update_player_lifecycle(&Input::default());
        s.register_hit();
        for _ in 0..30 {
            assert!(!s.update_player_lifecycle(&Input::default()));
        }
        assert_eq!((s.items.len(), s.next_extend), (5, EXTEND_SCORES.len()));
        assert!(s.update_player_lifecycle(&Input::default()));
        assert!(matches!(s.state, PlayerState::GameOver));
        assert!(
            s.events
                .iter()
                .any(|event| matches!(event, Event::GameOver(_)))
        );
    }

    #[test]
    fn replay_restores_pre_item_draw_seed_and_initial_progress() {
        let mut live = stage(Difficulty::Hard, false, 3, 3);
        live.set_rng_seed(0xbeef);
        live.world.rank = 25;
        live.world.power = 128;
        live.lives = 5;
        live.bombs = 7;
        live.point_items_total = 312;
        live.power_item_count = 24;
        let state = live.replay_state();
        assert_eq!(state.random_seed, 0xbeef);
        let mut playback = stage(Difficulty::Hard, false, 3, 3);
        playback.restore_replay_state(&state, 12_000_000);
        assert_eq!(playback.world.rng, live.world.rng);
        assert_eq!(
            (playback.rand_item_spawn, playback.rand_item_table),
            (live.rand_item_spawn, live.rand_item_table)
        );
        assert_eq!(
            (
                playback.lives,
                playback.bombs,
                playback.world.power,
                playback.world.rank
            ),
            (5, 7, 128, 25)
        );
        assert_eq!(
            (
                playback.point_items_total,
                playback.power_item_count,
                playback.next_extend
            ),
            (312, 24, 1)
        );
        assert!(!playback.can_continue());
    }

    #[test]
    fn fixed_bullet_pool_stops_before_rng_and_reuses_circular_slot() {
        use crate::ecl_vm::{BulletProps, spawn_bullet_pattern};
        let mut s = stage(Difficulty::Extra, false, 7, 3);
        let mut props = BulletProps {
            count1: 640,
            count2: 1,
            aim_mode: 8,
            angle1: 1.0,
            speed1: 2.0,
            ..Default::default()
        };
        spawn_bullet_pattern(&mut s.world, &props);
        assert_eq!(s.world.bullets.len(), 640);
        let rng = s.world.rng;
        props.count1 = 10;
        spawn_bullet_pattern(&mut s.world, &props);
        assert_eq!(s.world.rng, rng);
        assert_eq!(s.world.next_bullet_index, 0);
        s.world.bullets.remove(100);
        props.count1 = 1;
        spawn_bullet_pattern(&mut s.world, &props);
        assert_eq!(s.world.next_bullet_index, 101);
        assert!(
            s.world
                .bullets
                .iter()
                .enumerate()
                .all(|(index, bullet)| bullet.pool_index == index)
        );
    }

    #[test]
    fn grazes_cap_independently_and_keep_score_rank_particles_during_bombs() {
        let mut s = stage(Difficulty::Normal, false, 1, 3);
        s.graze = 999998;
        s.graze_in_stage = 9998;
        s.world.sub_rank = 96;
        for _ in 0..3 {
            s.score_graze([100.0, 100.0]);
        }
        assert_eq!((s.graze, s.graze_in_stage, s.score), (999999, 9999, 1500));
        assert_eq!((s.world.rank, s.world.sub_rank), (17, 14));
        s.graze_in_stage = 0;
        s.score_graze([100.0, 100.0]);
        assert_eq!((s.graze, s.graze_in_stage), (999999, 1));
        s.bombing = 1;
        s.score_graze(s.pos);
        assert_eq!((s.graze, s.graze_in_stage, s.score), (999999, 1, 2500));
        assert_eq!(s.world.sub_rank, 26);
    }

    #[test]
    fn touching_bullet_grazes_before_death_and_bomb_cancellation_keeps_fade() {
        use crate::ecl_vm::{BulletProps, spawn_bullet_pattern};
        let mut s = stage(Difficulty::Normal, false, 1, 3);
        s.state = PlayerState::Alive;
        let pos = s.pos;
        spawn_bullet_pattern(
            &mut s.world,
            &BulletProps {
                pos,
                ..Default::default()
            },
        );
        let mut bullet = s.world.bullets.remove(0);
        s.collide_bullet(&mut bullet);
        assert_eq!((s.graze, s.score), (1, 500));
        assert!(bullet.grazed);
        assert_eq!(bullet.despawn_timer, 12);
        assert!(matches!(s.state, PlayerState::Dead(0)));
        s.bomb_projectiles[0] = BombRect {
            pos,
            size: [8.0, 8.0],
        };
        spawn_bullet_pattern(
            &mut s.world,
            &BulletProps {
                pos,
                ..Default::default()
            },
        );
        let mut bomb_bullet = s.world.bullets.remove(0);
        s.collide_bullet(&mut bomb_bullet);
        assert_eq!(bomb_bullet.despawn_timer, 12);
        assert_eq!(s.items.len(), 1);
        assert_eq!((s.graze, s.score), (1, 500));
    }

    #[test]
    #[ignore = "requires laser-reference.txt from oracle/projectiles/build.sh"]
    fn original_laser_states_and_collisions_match_decompiled_source() {
        use crate::ecl_vm::Laser;
        let file = std::env::var("TH06_LASER_REFERENCE").expect("set TH06_LASER_REFERENCE");
        let reference = std::fs::read_to_string(file).expect("read C++ observations");
        let mut lines = reference.lines();
        let mut id = 0;
        for flags in [0, 1] {
            for start in [0, 20, 60] {
                for duration in [0, 25] {
                    for despawn in [0, 16] {
                        for end in [0, 16] {
                            for angle in [0.0f32, 0.7, std::f32::consts::FRAC_PI_2] {
                                for location in [0, 1, 2] {
                                    for bomb in [0, 1] {
                                        let mut s = stage(Difficulty::Normal, false, 1, 3);
                                        let (along, across) = (
                                            if location == 2 { 215.0 } else { 110.0 },
                                            if location == 0 { 0.0 } else { 30.0 },
                                        );
                                        s.pos = [
                                            100.0 + angle.cos() * along - angle.sin() * across,
                                            100.0 + angle.sin() * along + angle.cos() * across,
                                        ];
                                        s.state = PlayerState::Alive;
                                        s.bombing = bomb;
                                        s.invuln = if bomb == 1 { 120 } else { 0 };
                                        let mut laser = Laser {
                                            in_use: true,
                                            pos: [100.0, 100.0],
                                            angle,
                                            speed: 0.5,
                                            start_offset: 16.0,
                                            end_offset: 200.0,
                                            start_length: 200.0,
                                            width: 24.0,
                                            start_time: start,
                                            duration,
                                            despawn_duration: despawn,
                                            hitbox_start: 10,
                                            hitbox_end: end,
                                            flags,
                                            script: 9,
                                            draw_width: 16.0,
                                            draw_alpha: 1.0,
                                            state: if start == 0 { 1 } else { 0 },
                                            timer: 0,
                                            color: 0,
                                        };
                                        for frame in 0..85 {
                                            for collision in laser.step() {
                                                s.collide_laser(collision);
                                            }
                                            let state = if matches!(s.state, PlayerState::Dead(_)) {
                                                2
                                            } else if bomb == 1 {
                                                3
                                            } else {
                                                0
                                            };
                                            let line = format!(
                                                "{} {} {} {} {} {} {} {} {} {} {:.6} {} {:.6} {:.6}",
                                                id,
                                                frame,
                                                laser.state,
                                                laser.timer,
                                                u8::from(laser.in_use),
                                                state,
                                                s.graze,
                                                s.score,
                                                s.world.rank,
                                                s.world.sub_rank,
                                                laser.draw_width,
                                                (laser.draw_alpha * 255.0).round() as u32,
                                                laser.start_offset,
                                                laser.end_offset
                                            );
                                            assert_eq!(
                                                Some(line.as_str()),
                                                lines.next(),
                                                "original laser case {id}, frame {frame}"
                                            );
                                        }
                                        id += 1;
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        assert_eq!(id, 864);
        assert!(lines.next().is_none(), "extra original observations");
    }

    #[test]
    fn bullet_cancels_sample_lasers_and_bonus_counts_fading_bullets_only() {
        use crate::ecl_vm::{BulletProps, Laser, spawn_bullet_pattern};
        let mut s = stage(Difficulty::Normal, false, 1, 3);
        spawn_bullet_pattern(
            &mut s.world,
            &BulletProps {
                count1: 3,
                count2: 1,
                ..Default::default()
            },
        );
        s.world.bullets[1].despawn_timer = 5;
        s.world.lasers.push(Laser {
            in_use: true,
            pos: [10.0, 20.0],
            angle: 0.0,
            speed: 0.0,
            start_offset: 16.0,
            end_offset: 90.0,
            start_length: 90.0,
            width: 8.0,
            start_time: 0,
            duration: 20,
            despawn_duration: 16,
            hitbox_start: 0,
            hitbox_end: 0,
            flags: 0,
            script: 9,
            draw_width: 8.0,
            draw_alpha: 1.0,
            state: 1,
            timer: 10,
            color: 0,
        });
        let points = s.world.turn_bullets_into_points();
        assert_eq!(points.len(), 5);
        assert_eq!(&points[2..], &[[26.0, 20.0], [58.0, 20.0], [90.0, 20.0]]);
        assert_eq!(s.world.bullets.len(), 1);
        assert_eq!((s.world.lasers[0].state, s.world.lasers[0].timer), (2, 0));
        s.world.lasers[0].state = 1;
        let (points, bonus) = s.world.despawn_bullets_for_bonus(true);
        assert_eq!(points.len(), 5);
        assert_eq!(bonus, 2000);
        assert_eq!(s.world.bullets[0].despawn_timer, 5);
    }

    #[test]
    fn item_pool_full_scatter_does_not_draw_rng() {
        let mut s = stage(Difficulty::Normal, false, 1, 3);
        for _ in 0..512 {
            s.spawn_item(Item::fall([0.0, 0.0], 0));
        }
        let rng = s.world.rng;
        s.spawn_scatter_item([0.0, 0.0], 0);
        assert_eq!(s.world.rng, rng);
        assert_eq!(s.items.len(), 512);
        s.items.remove(100);
        s.spawn_scatter_item([0.0, 0.0], 0);
        assert_eq!(s.next_item_index, 101);
        assert_ne!(s.world.rng, rng);
    }

    #[test]
    fn spellcard_attempts_and_capture_history_match_original_catk() {
        let mut s = stage(Difficulty::Hard, false, 3, 3);
        let name = b"spell".to_vec();
        let checksum = name.iter().fold(0u8, |sum, &byte| sum.wrapping_add(byte));
        s.spellcard_records[8] = SpellcardRecord {
            name_checksum: checksum,
            attempts: 9999,
            successes: 5,
            character_shot_type: [0, 1, 2, 3, 0],
            ..Default::default()
        };
        s.world.character = 1;
        s.world.shot_type = 1;
        s.world.events.push(WorldEvent::SpellcardStart(8, 0, name));
        s.drain_world_events();
        s.spell_secs = 10;
        s.world.events.push(WorldEvent::SpellcardEnd {
            points: Vec::new(),
            bonus: 2000,
        });
        s.drain_world_events();
        let record = &s.spellcard_records[8];
        assert_eq!(
            (record.attempts, record.successes, record.capture_score),
            (9999, 6, SPELLCARD_SCORE[8] as u32)
        );
        assert_eq!(record.character_shot_type, [3, 0, 1, 2, 3]);
        assert_eq!(
            (s.spellcards_captured, s.score),
            (1, 2000 + SPELLCARD_SCORE[8] * 2)
        );
    }

    #[test]
    fn pause_uses_original_confirmation_default_and_twenty_frame_close() {
        let mut s = stage(Difficulty::Normal, false, 1, 3);
        s.paused = true;
        s.run_pause_menu(&Input::default());
        for _ in 0..4 {
            s.run_pause_menu(&Input::default());
        }
        s.run_pause_menu(&Input::synthetic(&[], &[Key::Down]));
        assert_eq!(s.pause_state, 2);
        s.run_pause_menu(&Input::synthetic(&[], &[Key::Shoot]));
        assert_eq!(s.pause_state, 5);
        for _ in 0..4 {
            s.run_pause_menu(&Input::default());
        }
        s.run_pause_menu(&Input::synthetic(&[], &[Key::Shoot]));
        assert_eq!(s.pause_state, 2);
        s.run_pause_menu(&Input::synthetic(&[], &[Key::Pause]));
        assert_eq!(s.pause_state, 3);
        for _ in 0..19 {
            s.run_pause_menu(&Input::default());
        }
        assert!(s.paused);
        s.run_pause_menu(&Input::default());
        assert!(!s.paused);
    }

    fn original_stage(
        base: &str,
        difficulty: Difficulty,
        number: i32,
        character: Character,
    ) -> Stage {
        use th06_formats::{anm0::Anm0, std::Std};
        let directory = |kind: &str| {
            let direct = format!("{base}/{kind}");
            if std::path::Path::new(&direct).is_dir() {
                direct
            } else {
                format!("{base}/{kind}-assets")
            }
        };
        let cm = directory("cm");
        let st = directory("st");
        let inn = directory("inn");
        let read = |dir: &str, name: &str| std::fs::read(format!("{dir}/{name}")).unwrap();
        let anm = |dir: &str, name: &str| Anm0::parse(&read(dir, name)).unwrap();
        let etama = anm(&cm, "etama3.anm");
        let eff = anm(&cm, "etama4.anm");
        let player = anm(
            &cm,
            if character.is_marisa() {
                "player01.anm"
            } else {
                "player00.anm"
            },
        );
        let front = anm(&cm, "front.anm");
        let text = anm(&inn, "text.anm");
        let ascii = anm(&inn, "ascii.anm");
        let enemy = anm(&st, &format!("stg{number}enm.anm"));
        let enemy2_name = format!("stg{number}enm2.anm");
        let enemy2 = std::path::Path::new(&format!("{st}/{enemy2_name}"))
            .is_file()
            .then(|| anm(&st, &enemy2_name));
        let mut pairs = vec![(&enemy.entries[0], 4)];
        if let Some(enemy2) = &enemy2 {
            pairs.push((&enemy2.entries[0], 5));
        }
        let bg = anm(&st, &format!("stg{number}bg.anm"));
        let background = Background::new(
            Std::parse(&read(&st, &format!("stage{number}.std"))).unwrap(),
            &bg.entries[0],
            11,
        );
        let player_portraits = ["a", "b", "c"]
            .iter()
            .enumerate()
            .map(|(slot, suffix)| {
                let name = format!("face{:02}{suffix}.anm", u8::from(character.is_marisa()));
                PortraitAsset::new(20 + slot, &anm(&cm, &name).entries[0])
            })
            .collect::<Vec<_>>();
        const FACES: [&[&str]; 7] = [
            &["face03a", "face03b"],
            &["face05a"],
            &["face06a", "face06b"],
            &["face08a", "face08b"],
            &["face09a", "face09b"],
            &["face09b", "face10a", "face10b"],
            &["face08a", "face12a", "face12b"],
        ];
        let bosses = FACES[(number - 1) as usize]
            .iter()
            .enumerate()
            .map(|(slot, name)| {
                PortraitAsset::new(30 + slot, &anm(&st, &format!("{name}.anm")).entries[0])
            })
            .collect();
        let script = |id| {
            text.entries[0]
                .scripts
                .iter()
                .find(|(index, _)| *index == id)
                .unwrap()
                .1
                .clone()
        };
        let mut result = Stage::new(
            Ecl::parse(read(&st, &format!("ecldata{number}.ecl"))).unwrap(),
            build_enemy_scripts(&pairs),
            &etama.entries[0],
            &eff.entries[0],
            14,
            &player.entries[0],
            2,
            character,
            Msg::parse(read(&st, &format!("msg{number}.dat"))).unwrap(),
            Some(background),
            Hud::new(&front.entries[0], 6),
            StageConfig {
                face_player_tex: 20,
                face_boss_tex: 30,
                stage_bgm: "stage",
                boss_bgm: "boss",
                stage_num: number,
                difficulty,
                practice: false,
                starting_lives: 3,
                starting_bombs: 3,
                rng_seed: 0x1234,
            },
            script(7),
            Vec::new(),
            HashMap::new(),
            HashMap::new(),
            script(0),
            None,
        );
        result.set_portrait_assets(player_portraits, bosses);
        result.set_dialogue_scripts(&text.entries[0]);
        result.set_pause_art(PortraitAsset::new(8, &ascii.entries[0]));
        if number == 7 {
            result.set_extra_boss_portrait(PortraitAsset::new(
                33,
                &anm(&st, "face12c.anm").entries[0],
            ));
        }
        result
    }

    fn frame_hash(stage: &Stage, cmds: &[DrawCmd]) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        (
            stage.tick,
            stage.timeline_off,
            stage.timeline_time,
            stage.score,
            stage.gui_score,
            stage.graze,
            stage.point_items,
            stage.lives,
            stage.bombs,
            stage.world.rank,
            stage.world.sub_rank,
            stage.world.rng.seed(),
        )
            .hash(&mut h);
        (
            stage.world.power,
            stage.world.next_bullet_index,
            stage.world.next_effect_index,
            stage.world.spellcard_active,
            stage.dialogue.off,
            stage.dialogue.timer,
            stage.bombing,
            stage.dying,
        )
            .hash(&mut h);
        stage.state.hash(&mut h);
        (
            stage.invuln,
            stage.bullet_grace_period,
            stage.player_particle_timer,
            stage.fire_timer,
        )
            .hash(&mut h);
        for value in stage.pos {
            value.to_bits().hash(&mut h);
        }
        for enemy in &stage.enemies {
            (
                enemy.occupied,
                enemy.is_boss,
                enemy.life,
                enemy.spell_seconds_left(),
            )
                .hash(&mut h);
            for value in enemy.pos {
                value.to_bits().hash(&mut h);
            }
        }
        for bullet in &stage.world.bullets {
            (
                bullet.pool_index,
                bullet.sprite,
                bullet.timer,
                bullet.spawn_delay,
                bullet.despawn_timer,
                bullet.ex_flags,
            )
                .hash(&mut h);
            for value in [bullet.pos[0], bullet.pos[1], bullet.angle, bullet.speed] {
                value.to_bits().hash(&mut h);
            }
        }
        for item in &stage.items {
            (item.kind, item.state, item.timer).hash(&mut h);
            for value in item.pos {
                value.to_bits().hash(&mut h);
            }
        }
        for cmd in cmds {
            cmd.tex.hash(&mut h);
            cmd.additive.hash(&mut h);
            cmd.color_add.hash(&mut h);
            for value in cmd
                .dst
                .into_iter()
                .chain(cmd.src)
                .chain(cmd.tint)
                .chain([cmd.rot])
            {
                value.to_bits().hash(&mut h);
            }
        }
        h.finish()
    }

    #[test]
    #[ignore = "requires original ST/CM/IN assets in TH06_REFERENCE_DIR; full gameplay and T6RP regression"]
    fn original_assets_all_routes_and_replays_match_every_frame() {
        use th06_formats::replay::Replay;
        let base =
            std::env::var("TH06_REFERENCE_DIR").unwrap_or_else(|_| "/tmp/th06-reference".into());
        let stage_filter = std::env::var("TH06_GAMEPLAY_STAGE")
            .ok()
            .and_then(|v| v.parse::<i32>().ok());
        // Invulnerability allows the automated coverage bot to exercise every
        // original phase. Both recording and playback use the same debug flag.
        assert!(
            std::env::var_os("TH06_GOD").is_some(),
            "run this isolated regression with TH06_GOD=1"
        );
        let mut cases = 0;
        for number in 1..=7 {
            if stage_filter.is_some_and(|filter| filter != number) {
                continue;
            }
            for difficulty in Difficulty::ALL {
                if (number == 7) != (difficulty == Difficulty::Extra)
                    || (number == 6 && difficulty == Difficulty::Easy)
                {
                    continue;
                }
                for (shot, character) in [
                    Character::ReimuA,
                    Character::ReimuB,
                    Character::MarisaA,
                    Character::MarisaB,
                ]
                .into_iter()
                .enumerate()
                {
                    let mut live = original_stage(&base, difficulty, number, character);
                    if number > 1 {
                        live.set_power(128);
                    }
                    let mut initial = live.replay_state();
                    let mut traces = Vec::new();
                    let mut previous = 0;
                    let mut finished = false;
                    for frame in 0..100_000 {
                        let mut buttons = 1 | 4 | 256;
                        if let Some(x) = live.target_x() {
                            if x < live.pos[0] - 3.0 {
                                buttons |= 64;
                            } else if x > live.pos[0] + 3.0 {
                                buttons |= 128;
                            }
                        }
                        if frame % 1800 == 900 {
                            buttons |= 2;
                        }
                        initial.record(frame, buttons);
                        let input = Input::from_buttons(buttons, previous);
                        previous = buttons;
                        let cmds = live.update(&input);
                        traces.push(frame_hash(&live, &cmds));
                        finished = live
                            .events
                            .iter()
                            .any(|event| matches!(event, Event::NextStage));
                        live.events.clear();
                        assert!(
                            live.world.bullets.len() <= 640
                                && live.world.lasers.len() <= 64
                                && live.enemies.len() <= 256
                        );
                        if finished {
                            break;
                        }
                    }
                    assert!(
                        finished,
                        "stage {number} {difficulty:?} shot {shot} failed to reach its original MSG stage end: timeline={} dialog={} off={} boss={}",
                        live.timeline_time,
                        live.dialogue.active,
                        live.dialogue.off,
                        live.world.boss_present
                    );
                    initial.finish(traces.len() as i32, live.score as u32);
                    let mut replay = Replay {
                        character: shot as u8,
                        difficulty: difficulty as u8,
                        date: "10/12/26".into(),
                        name: "TEST".into(),
                        score: live.score as u32,
                        slowdown: 0.0,
                        stages: std::array::from_fn(|_| None),
                    };
                    replay.stages[(number - 1) as usize] = Some(initial);
                    let roundtrip = Replay::parse(&replay.encode()).unwrap();
                    let state = roundtrip.stages[(number - 1) as usize].as_ref().unwrap();
                    let mut playback = original_stage(&base, difficulty, number, character);
                    playback.restore_replay_state(state, 0);
                    previous = 0;
                    for (frame, expected) in traces.iter().enumerate() {
                        let buttons = state.buttons_at(frame as i32);
                        let input = Input::from_buttons(buttons, previous);
                        previous = buttons;
                        let cmds = playback.update(&input);
                        assert_eq!(
                            frame_hash(&playback, &cmds),
                            *expected,
                            "stage {number} {difficulty:?} shot {shot} replay diverges at frame {frame}"
                        );
                        playback.events.clear();
                    }
                    cases += 1;
                    eprintln!(
                        "stage {number} {difficulty:?} shot {shot}: {} frames, score {}, replay identical",
                        traces.len(),
                        live.score
                    );
                }
            }
        }
        assert_eq!(
            cases,
            if stage_filter == Some(7) {
                4
            } else if stage_filter == Some(6) {
                12
            } else if stage_filter.is_some() {
                16
            } else {
                96
            }
        );
    }

    #[test]
    #[ignore = "requires original archives in TH06_REFERENCE_DIR; real player ANM lifecycle"]
    fn original_assets_player_impacts_and_laser_pulses_end_on_original_anm_frames() {
        let base =
            std::env::var("TH06_REFERENCE_DIR").unwrap_or_else(|_| "/tmp/th06-reference".into());
        for (character, kind, frames) in [
            (Character::ReimuA, 0, 30),
            (Character::ReimuB, 0, 30),
            (Character::MarisaA, 2, 20),
        ] {
            let mut s = original_stage(&base, Difficulty::Normal, 1, character);
            s.world.power = 128;
            s.update_orbs(false);
            s.spawn_player_bullets(0);
            let keep = s.shots.iter().position(|shot| shot.bt == kind).unwrap();
            let mut shot = s.shots.remove(keep);
            shot.pos = [100.0; 2];
            shot.vel = [0.0; 2];
            s.shots.clear();
            s.shots.push(shot);
            assert!(s.calc_player_damage([100.0, 100.0, 0.0], [16.0; 2]).0 > 0);
            assert!(s.shots[0].collided);
            for frame in 1..frames {
                s.update_shots();
                assert_eq!(
                    s.shots.len(),
                    1,
                    "{} impact freed at {frame}",
                    character.label()
                );
            }
            s.update_shots();
            assert!(
                s.shots.is_empty(),
                "{} impact must end at {frames}",
                character.label()
            );
        }
        for (power, period) in [
            (8, 120),
            (16, 170),
            (32, 200),
            (48, 210),
            (64, 230),
            (80, 250),
            (96, 270),
            (128, 330),
        ] {
            let mut s = original_stage(&base, Difficulty::Normal, 1, Character::MarisaB);
            s.world.power = power;
            s.update_orbs(false);
            s.spawn_player_bullets(0);
            s.shots.retain(|shot| shot.bt == 3);
            assert_eq!(s.shots.len(), 2);
            assert_eq!(s.laser_timers, [period; 2]);
            for frame in 1..period - 40 {
                s.update_shots();
                assert_eq!(s.shots.len(), 2, "power{power} lasers freed at {frame}");
            }
            s.update_shots();
            assert!(
                s.shots.is_empty(),
                "power{power} lasers must end at {}",
                period - 40
            );
            assert_eq!(s.laser_timers, [40; 2]);
            for _ in 0..40 {
                s.update_shots();
            }
            s.spawn_player_bullets(0);
            assert_eq!(s.shots.iter().filter(|shot| shot.bt == 3).count(), 2);
        }
    }

    #[test]
    #[ignore = "requires original archives in TH06_REFERENCE_DIR; all four original bomb attack animations"]
    fn original_assets_bombs_magnet_items_and_preserve_original_attack_windows() {
        let base =
            std::env::var("TH06_REFERENCE_DIR").unwrap_or_else(|_| "/tmp/th06-reference".into());
        for (character, duration) in [
            (Character::ReimuA, 300),
            (Character::ReimuB, 140),
            (Character::MarisaA, 250),
            (Character::MarisaB, 300),
        ] {
            let mut s = original_stage(&base, Difficulty::Normal, 1, character);
            s.update_player_lifecycle(&Input::default());
            s.world.power = 128;
            s.update_orbs(false);
            s.spawn_item(Item::fall([100.0, 200.0], 0));
            s.fire_bomb();
            assert_eq!(s.items.len(), 1);
            assert_eq!(
                s.items[0].state,
                1,
                "{} must magnet existing items",
                character.label()
            );
            assert_eq!(s.bomb_duration, duration);
            assert!(s.bomb_in_use);
            assert_eq!(
                s.bomb_regions
                    .iter()
                    .filter(|rect| rect.size[0] > 0.0)
                    .count(),
                0
            );
            for frame in 1..duration {
                s.update_player_lifecycle(&Input::default());
                let regions = s
                    .bomb_regions
                    .iter()
                    .filter(|rect| rect.size[0] > 0.0)
                    .count();
                match character {
                    Character::ReimuA => {
                        if frame < 80 {
                            assert_eq!(regions, 0);
                        }
                        if frame == 80 {
                            assert_eq!(s.seal_state[1], 1);
                            assert_eq!(s.bomb_regions[1].size, [48.0; 2]);
                            assert_eq!(s.bomb_region_dmg[1], 8);
                            s.bomb_dmg_accum[1] = 100;
                        }
                        if frame == 81 {
                            assert_eq!(s.seal_state[1], 2);
                            assert_eq!(s.bomb_regions[1].size, [256.0; 2]);
                            assert_eq!(s.bomb_region_dmg[1], 200);
                            assert_eq!(s.bomb_projectiles[1].size, [256.0; 2]);
                        }
                    }
                    Character::ReimuB => {
                        assert_eq!(regions, if frame % 2 != 0 { 4 } else { 0 });
                        assert_eq!(
                            s.bomb_projectiles
                                .iter()
                                .filter(|rect| rect.size[0] > 0.0)
                                .count(),
                            4
                        );
                    }
                    Character::MarisaA => assert_eq!(regions, if frame % 3 != 0 { 8 } else { 0 }),
                    Character::MarisaB => {
                        assert_eq!(regions, usize::from(frame % 4 != 0));
                        if frame % 4 != 0 {
                            assert_eq!(s.bomb_regions[0].pos, [192.0, 192.0]);
                            assert_eq!(s.bomb_regions[0].size, [384.0, 384.0]);
                        }
                        assert_eq!(s.bomb_move_mult, 0.3);
                    }
                }
                assert!(
                    s.bomb_in_use,
                    "{} ended early at frame {frame}",
                    character.label()
                );
            }
            s.update_player_lifecycle(&Input::default());
            assert!(!s.bomb_in_use);
            assert_eq!(s.bomb_move_mult, 1.0);
        }
    }

    #[test]
    fn difficulty_rank_and_practice_initialization_match_game_manager() {
        for difficulty in Difficulty::ALL {
            let s = stage(difficulty, false, 1, 5);
            assert_eq!(
                (s.world.rank, s.world.min_rank, s.world.max_rank),
                difficulty.rank_limits()
            );
            assert_eq!(
                s.lives,
                if difficulty == Difficulty::Extra {
                    2
                } else {
                    4
                }
            );
            assert_eq!(
                s.bombs,
                if difficulty == Difficulty::Extra {
                    3
                } else {
                    2
                }
            );
        }
        for (number, power) in [(1, 0), (2, 64), (3, 128), (6, 128)] {
            let s = stage(Difficulty::Hard, true, number, 5);
            assert_eq!(s.world.power, power);
            assert_eq!((s.lives, s.bombs), (2, 3));
            assert!(!s.can_continue());
        }
    }

    #[test]
    fn stage_transition_preserves_rank_rng_and_run_counters_but_resets_subrank() {
        let mut first = stage(Difficulty::Hard, false, 1, 3);
        first.world.rank = 23;
        first.world.sub_rank = 74;
        first.world.rng.u32();
        first.gui_score = 123;
        first.score = 456;
        first.next_extend = 2;
        first.continues_used = 1;
        first.graze = 96;
        first.point_items_total = 21;
        first.deaths = 3;
        let carry = first.carry();
        let mut expected_rng = first.world.rng;
        let item_phase = expected_rng.u16_in_range(3);
        let item_table = expected_rng.u16_in_range(8);
        let mut next = stage(Difficulty::Hard, false, 2, 3);
        next.apply_carry(carry);
        assert_eq!(next.world.rng, expected_rng);
        assert_eq!(
            (next.rand_item_spawn, next.rand_item_table),
            (item_phase as usize, item_table as usize)
        );
        assert_eq!((next.world.rank, next.world.sub_rank), (23, 0));
        assert_eq!((next.score, next.gui_score), (456, 456));
        assert_eq!(
            (next.next_extend, next.continues_used, next.deaths),
            (2, 1, 3)
        );
        assert_eq!(
            (
                next.graze_in_stage,
                next.point_items,
                next.point_items_total
            ),
            (0, 0, 21)
        );
    }

    #[test]
    fn continue_resets_only_original_retry_fields_and_stops_after_three() {
        let mut s = stage(Difficulty::Lunatic, false, 4, 4);
        s.world.rank = 13;
        s.world.sub_rank = 19;
        s.graze = 84;
        s.point_items = 20;
        s.world.power = 128;
        let rng = s.world.rng;
        for retry in 1..=3 {
            assert!(s.continue_run());
            assert_eq!((s.score, s.gui_score), (retry, retry));
            assert_eq!((s.lives, s.bombs, s.world.power), (3, 2, 0));
            assert_eq!((s.graze_in_stage, s.point_items), (0, 0));
            assert_eq!((s.world.rank, s.world.sub_rank), (13, 19));
            assert_eq!(s.world.rng, rng);
        }
        assert!(!s.continue_run());
        assert!(!stage(Difficulty::Extra, false, 7, 3).can_continue());
    }

    #[test]
    fn original_point_values_include_difficulty_and_integer_y_boundary() {
        for (difficulty, top, line, bottom) in [
            (Difficulty::Normal, 100_000, 60_000, 30_000),
            (Difficulty::Hard, 150_000, 100_000, 46_000),
            (Difficulty::Lunatic, 200_000, 150_000, 69_000),
            (Difficulty::Extra, 300_000, 200_000, 80_000),
        ] {
            assert_eq!(difficulty.point_score(127.99), top);
            assert_eq!(difficulty.point_score(128.99), line);
            assert_eq!(difficulty.point_score(428.0), bottom);
        }
    }

    #[test]
    fn clear_bonus_applies_difficulty_before_configured_life_penalty() {
        let normal = stage_clear_bonus(1, 5, 16, 3, 2, 3, Difficulty::Normal as u8, 2);
        assert_eq!(normal, 7950);
        assert_eq!(
            stage_clear_bonus(1, 5, 16, 3, 2, 3, Difficulty::Hard as u8, 3),
            4770
        );
        assert_eq!(
            stage_clear_bonus(1, 5, 16, 3, 2, 3, Difficulty::Easy as u8, 4),
            790
        );
    }

    #[test]
    fn score_extends_follow_rolling_score_once_and_are_disabled_in_extra() {
        let mut normal = stage(Difficulty::Normal, false, 1, 3);
        normal.score = 10_000_000;
        normal.gui_score = 9_999_990;
        normal.roll_gui_score();
        assert_eq!(
            (normal.lives, normal.next_extend, normal.world.rank),
            (3, 1, 18)
        );
        normal.roll_gui_score();
        assert_eq!(normal.lives, 3);
        let mut extra = stage(Difficulty::Extra, false, 7, 3);
        extra.score = 10_000_000;
        extra.gui_score = 9_999_990;
        extra.roll_gui_score();
        assert_eq!(extra.lives, 2);
    }
}
