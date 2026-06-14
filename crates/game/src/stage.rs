//! Stage gameplay driven by the original ECL scripts.
//!
//! The timeline spawns enemies exactly as the 2002 engine does; enemy
//! behavior runs in ecl_vm. The player, bomb and HUD live here.

use std::collections::HashMap;

use th06_engine::{DrawCmd, Input, Key};
use th06_formats::anm0::{Entry, Instr as AnmInstr, Sprite};
use th06_formats::ecl::Ecl;
use th06_formats::msg::Msg;

use crate::anm_vm::AnmRunner;
use crate::background::Background;
use th06_engine::BgScene;
use crate::ecl_vm::{Bullet, Enemy, Rng, SpawnReq, World, WorldEvent};

pub const FIELD_W: f32 = 384.0;
pub const FIELD_H: f32 = 448.0;
const FIELD_X: f32 = 32.0;
const FIELD_Y: f32 = 16.0;

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
pub const TEX_FACE_PLAYER: usize = 9; // face00a (Reimu)
pub const TEX_FACE_BOSS: usize = 10; // face01a (Rumia)

pub enum Event {
    Sfx(&'static str),
    Bgm(&'static str),
    BackToTitle,
    Quit,
    SaveScore(i64),
}

#[derive(Clone, Copy)]
struct SpriteRef {
    tex: usize,
    rect: [f32; 4],
}

const fn spr(tex: usize, x: f32, y: f32, w: f32, h: f32) -> SpriteRef {
    SpriteRef { tex, rect: [x, y, w, h] }
}

const AMULET: SpriteRef = spr(TEX_PLAYER, 129.0, 1.0, 14.0, 14.0);
const BOMB_GLOW: SpriteRef = spr(TEX_PLAYER, 1.0, 97.0, 62.0, 62.0);
/// player00 sprite 66: the focus hitbox marker.
const HITBOX_MARKER: SpriteRef = spr(TEX_PLAYER, 160.0, 0.0, 16.0, 16.0);

const HUD_LOGO: SpriteRef = spr(TEX_FRONT, 128.0, 128.0, 128.0, 128.0);
const HUD_PLAYER_LABEL: SpriteRef = spr(TEX_FRONT, 0.0, 208.0, 32.0, 16.0);
const HUD_BOMB_LABEL: SpriteRef = spr(TEX_FRONT, 0.0, 240.0, 32.0, 16.0);
const HUD_STAR_RED: SpriteRef = spr(TEX_FRONT, 32.0, 240.0, 16.0, 16.0);
const HUD_STAR_GREEN: SpriteRef = spr(TEX_FRONT, 48.0, 240.0, 16.0, 16.0);

struct Shot {
    pos: [f32; 2],
    vel: [f32; 2],
    damage: i32,
    /// BULLET_TYPE_*: 0 straight, 1 homing orb (Reimu A), 2 gravity orb-missile
    /// (Marisa A), 3 laser (handled separately).
    bt: u8,
    age: u32,
    /// Current speed magnitude (Player.cpp `unk_134.y`), grown while homing.
    spd: f32,
}

/// One entry of a power-rank fire table (CharacterPowerBulletData).
struct ShotDef {
    /// Fires when `fireTimer % wait == frame` (frames 0..=30).
    wait: i32,
    frame: i32,
    /// Spawn offset from the firing point.
    motion: [f32; 2],
    /// Direction in degrees, 0 = right, clockwise (so -90 = straight up).
    dir_deg: f32,
    vel: f32,
    damage: i32,
    /// 0 = player center, 1 = left orb, 2 = right orb.
    spawn: u8,
    /// Bullet type (see Shot::bt).
    bt: u8,
}

/// Reimu A constructor (motion.y = 0; bullet type from the homing flag).
const fn sd(wait: i32, frame: i32, mx: f32, dir_deg: f32, vel: f32, damage: i32, spawn: u8, homing: bool) -> ShotDef {
    ShotDef { wait, frame, motion: [mx, 0.0], dir_deg, vel, damage, spawn, bt: homing as u8 }
}

/// General constructor with explicit motion.y and bullet type.
const fn sb(wait: i32, frame: i32, mx: f32, my: f32, dir_deg: f32, vel: f32, damage: i32, spawn: u8, bt: u8) -> ShotDef {
    ShotDef { wait, frame, motion: [mx, my], dir_deg, vel, damage, spawn, bt }
}

/// ReimuA shot tiers (g_CharacterPowerDataReimuA in BulletData.cpp). Indexed
/// by power rank (see `power_rank`).
const REIMU_A_RANKS: [&[ShotDef]; 9] = [
    // Rank 1
    &[sd(5, 0, 0.0, -90.0, 12.0, 48, 0, false)],
    // Rank 2
    &[
        sd(5, 0, 0.0, -90.0, 12.0, 48, 0, false),
        sd(30, 0, 0.0, -120.0, 10.0, 14, 1, true),
        sd(30, 0, 0.0, -60.0, 10.0, 14, 2, true),
    ],
    // Rank 3
    &[
        sd(5, 0, -4.0, -91.0, 12.0, 30, 0, false),
        sd(5, 0, 4.0, -89.0, 12.0, 30, 0, false),
        sd(30, 0, 0.0, -120.0, 10.0, 14, 1, true),
        sd(30, 0, 0.0, -60.0, 10.0, 14, 2, true),
    ],
    // Rank 4
    &[
        sd(5, 0, 0.0, -96.0, 12.0, 24, 0, false),
        sd(5, 0, 0.0, -90.0, 12.0, 30, 0, false),
        sd(5, 0, 0.0, -84.0, 12.0, 24, 0, false),
        sd(30, 0, 0.0, -120.0, 10.0, 14, 1, true),
        sd(30, 0, 0.0, -60.0, 10.0, 14, 2, true),
    ],
    // Rank 5
    &[
        sd(5, 0, 0.0, -97.0, 12.0, 24, 0, false),
        sd(5, 0, 0.0, -90.0, 12.0, 30, 0, false),
        sd(5, 0, 0.0, -83.0, 12.0, 24, 0, false),
        sd(15, 0, 0.0, -120.0, 10.0, 12, 1, true),
        sd(15, 0, 0.0, -60.0, 10.0, 12, 2, true),
    ],
    // Rank 6
    &[
        sd(5, 0, 0.0, -97.0, 12.0, 24, 0, false),
        sd(5, 0, 0.0, -90.0, 12.0, 29, 0, false),
        sd(5, 0, 0.0, -83.0, 12.0, 24, 0, false),
        sd(15, 0, 0.0, -120.0, 10.0, 9, 1, true),
        sd(15, 0, 0.0, -60.0, 10.0, 9, 2, true),
        sd(30, 0, 0.0, -150.0, 10.0, 12, 1, true),
        sd(30, 0, 0.0, -30.0, 10.0, 12, 2, true),
    ],
    // Rank 7
    &[
        sd(5, 0, 0.0, -97.0, 12.0, 24, 0, false),
        sd(5, 0, 0.0, -90.0, 12.0, 28, 0, false),
        sd(5, 0, 0.0, -83.0, 12.0, 24, 0, false),
        sd(30, 0, 0.0, -110.0, 10.0, 10, 1, true),
        sd(30, 0, 0.0, -70.0, 10.0, 10, 2, true),
        sd(30, 10, 0.0, -130.0, 10.0, 9, 1, true),
        sd(30, 10, 0.0, -50.0, 10.0, 9, 2, true),
        sd(30, 20, 0.0, -150.0, 10.0, 11, 1, true),
        sd(30, 20, 0.0, -30.0, 10.0, 11, 2, true),
    ],
    // Rank 8
    &[
        sd(5, 0, 0.0, -97.0, 12.0, 24, 0, false),
        sd(5, 0, 0.0, -90.0, 12.0, 28, 0, false),
        sd(5, 0, 0.0, -83.0, 12.0, 24, 0, false),
        sd(15, 0, 0.0, -110.0, 10.0, 8, 1, true),
        sd(15, 0, 0.0, -70.0, 10.0, 8, 2, true),
        sd(15, 5, 0.0, -130.0, 10.0, 8, 1, true),
        sd(15, 5, 0.0, -50.0, 10.0, 8, 2, true),
        sd(15, 10, 0.0, -150.0, 10.0, 8, 1, true),
        sd(15, 10, 0.0, -30.0, 10.0, 8, 2, true),
    ],
    // Rank 9
    &[
        sd(5, 0, -8.0, -97.0, 12.0, 23, 0, false),
        sd(5, 0, -8.0, -90.0, 12.0, 24, 0, false),
        sd(5, 0, 8.0, -90.0, 12.0, 24, 0, false),
        sd(5, 0, 8.0, -83.0, 12.0, 23, 0, false),
        sd(16, 0, 0.0, -110.0, 10.0, 10, 1, true),
        sd(16, 0, 0.0, -70.0, 10.0, 10, 2, true),
        sd(16, 4, 0.0, -130.0, 10.0, 8, 1, true),
        sd(16, 4, 0.0, -50.0, 10.0, 8, 2, true),
        sd(16, 8, 0.0, -150.0, 10.0, 7, 1, true),
        sd(16, 8, 0.0, -30.0, 10.0, 7, 2, true),
        sd(16, 12, 0.0, -170.0, 10.0, 10, 1, true),
        sd(16, 12, 0.0, -10.0, 10.0, 10, 2, true),
    ],
];

/// ReimuB shot tiers (g_CharacterPowerDataReimuB). All straight bullets:
/// a front spread (centre) plus fast needles from the orbs (motion.y -16).
const REIMU_B_RANKS: [&[ShotDef]; 9] = [
    // Rank 1
    &[sd(5, 0, 0.0, -90.0, 12.0, 48, 0, false)],
    // Rank 2
    &[
        sd(5, 0, 0.0, -90.0, 12.0, 48, 0, false),
        sb(15, 0, 0.0, -16.0, -90.0, 22.0, 12, 1, 0),
        sb(15, 0, 0.0, -16.0, -90.0, 22.0, 12, 2, 0),
    ],
    // Rank 3
    &[
        sd(5, 0, -4.0, -91.0, 12.0, 32, 0, false),
        sd(5, 0, 4.0, -89.0, 12.0, 32, 0, false),
        sb(10, 0, 0.0, -16.0, -90.0, 22.0, 12, 1, 0),
        sb(10, 0, 0.0, -16.0, -90.0, 22.0, 12, 2, 0),
    ],
    // Rank 4
    &[
        sd(5, 0, -4.0, -91.0, 12.0, 30, 0, false),
        sd(5, 0, 4.0, -89.0, 12.0, 30, 0, false),
        sb(8, 0, 0.0, -16.0, -90.0, 22.0, 12, 1, 0),
        sb(8, 0, 0.0, -16.0, -90.0, 22.0, 12, 2, 0),
    ],
    // Rank 5
    &[
        sd(5, 0, 0.0, -97.0, 12.0, 20, 0, false),
        sd(5, 0, 0.0, -90.0, 12.0, 28, 0, false),
        sd(5, 0, 0.0, -83.0, 12.0, 20, 0, false),
        sb(8, 0, 0.0, -16.0, -90.0, 22.0, 12, 1, 0),
        sb(8, 0, 0.0, -16.0, -90.0, 22.0, 12, 2, 0),
    ],
    // Rank 6
    &[
        sd(5, 0, 0.0, -97.0, 12.0, 16, 0, false),
        sd(5, 0, 0.0, -90.0, 12.0, 27, 0, false),
        sd(5, 0, 0.0, -83.0, 12.0, 16, 0, false),
        sb(5, 0, 8.0, -16.0, -90.0, 22.0, 12, 1, 0),
        sb(5, 0, 8.0, -16.0, -90.0, 22.0, 12, 2, 0),
        sb(8, 0, -8.0, -16.0, -90.0, 22.0, 10, 1, 0),
        sb(8, 0, -8.0, -16.0, -90.0, 22.0, 10, 2, 0),
    ],
    // Rank 7
    &[
        sd(5, 0, 0.0, -98.0, 12.0, 16, 0, false),
        sd(5, 0, 0.0, -90.0, 12.0, 22, 0, false),
        sd(5, 0, 0.0, -82.0, 12.0, 16, 0, false),
        sb(3, 0, 8.0, -16.0, -90.0, 22.0, 10, 1, 0),
        sb(3, 0, 8.0, -16.0, -90.0, 22.0, 10, 2, 0),
        sb(5, 0, -8.0, -16.0, -90.0, 22.0, 10, 1, 0),
        sb(5, 0, -8.0, -16.0, -90.0, 22.0, 10, 2, 0),
    ],
    // Rank 8
    &[
        sd(5, 0, 0.0, -106.0, 12.0, 9, 0, false),
        sd(5, 0, 0.0, -98.0, 12.0, 17, 0, false),
        sd(5, 0, 0.0, -90.0, 12.0, 20, 0, false),
        sd(5, 0, 0.0, -82.0, 12.0, 17, 0, false),
        sd(5, 0, 0.0, -74.0, 12.0, 9, 0, false),
        sb(3, 0, 12.0, -16.0, -90.0, 22.0, 10, 1, 0),
        sb(3, 0, 12.0, -16.0, -90.0, 22.0, 10, 2, 0),
        sb(5, 0, -12.0, -16.0, -90.0, 22.0, 10, 1, 0),
        sb(5, 0, -12.0, -16.0, -90.0, 22.0, 10, 2, 0),
        sb(10, 0, 0.0, -16.0, -90.0, 22.0, 10, 1, 0),
        sb(10, 0, 0.0, -16.0, -90.0, 22.0, 10, 2, 0),
    ],
    // Rank 9
    &[
        sd(5, 0, 0.0, -106.0, 12.0, 9, 0, false),
        sd(5, 0, 0.0, -98.0, 12.0, 17, 0, false),
        sd(5, 0, 0.0, -90.0, 12.0, 20, 0, false),
        sd(5, 0, 0.0, -82.0, 12.0, 17, 0, false),
        sd(5, 0, 0.0, -74.0, 12.0, 9, 0, false),
        sb(3, 0, 12.0, -16.0, -90.0, 22.0, 10, 1, 0),
        sb(3, 0, 12.0, -16.0, -90.0, 22.0, 10, 2, 0),
        sb(3, 0, -12.0, -16.0, -90.0, 22.0, 10, 1, 0),
        sb(3, 0, -12.0, -16.0, -90.0, 22.0, 10, 2, 0),
        sb(5, 0, 0.0, -16.0, -90.0, 22.0, 10, 1, 0),
        sb(5, 0, 0.0, -16.0, -90.0, 22.0, 10, 2, 0),
    ],
];

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
            // Marisa tables land in later phases; fall back to Reimu for now.
            Character::ReimuA | Character::MarisaA => &REIMU_A_RANKS,
            Character::ReimuB | Character::MarisaB => &REIMU_B_RANKS,
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

/// One Fantasy Seal bomb orb (ReimuA bomb): flies out, then homes onto the
/// nearest enemy as a moving damage zone.
struct BombOrb {
    pos: [f32; 2],
    vel: [f32; 2],
    spd: f32,
    hue: f32,
}

/// Falling collectible (ItemManager port, simplified physics).
struct Item {
    pos: [f32; 2],
    vy: f32,
    kind: i32, // 0 power small, 1 point, 2 power big, 3 bomb, 4 full, 5 life
}

/// A short-lived visual puff (enemy death, item pickup, bullet cancel).
struct Particle {
    pos: [f32; 2],
    vel: [f32; 2],
    life: f32,
    max_life: f32,
    size: f32,
    color: [f32; 3],
}

/// Spell card names in English, indexed by the ECL spell id (op93). Only
/// stage 1's are filled in; the in-game names are Shift-JIS and cannot be
/// drawn with the ASCII font.
fn spellcard_name(id: i32) -> &'static str {
    match id {
        0 => "Moon Sign \"Moonlight Ray\"",
        1 => "Night Sign \"Night Bird\"",
        2 => "Dark Sign \"Demarcation\"",
        _ => "Spell Card",
    }
}

/// Item drop pattern for ITEM_RANDOM_ITEM enemies (g_RandomItems).
const RANDOM_ITEMS: [i32; 32] = [
    0, 0, 1, 0, 1, 0, 0, 1, 1, 1, 0, 0, 0, 1, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 0, 1, 1, 1, 0, 2,
];

/// Running MSG dialogue (GuiImpl::RunMsg port, text-only for now).
struct Dialogue {
    active: bool,
    off: u32,
    timer: u16,
    frames_pause: u16,
    ecl_resumed: bool,
    lines: [String; 2],
    line_colors: [usize; 2],
    /// Which portrait (0 player / 1 boss) spoke last, -1 none.
    portrait_active: i32,
    /// Whether each portrait has appeared, and its chosen expression sprite.
    portrait_shown: [bool; 2],
    portrait_expr: [usize; 2],
}

impl Default for Dialogue {
    fn default() -> Self {
        Self {
            active: false,
            off: 0,
            timer: 0,
            frames_pause: 0,
            ecl_resumed: false,
            lines: [String::new(), String::new()],
            line_colors: [0, 0],
            portrait_active: -1,
            portrait_shown: [false, false],
            portrait_expr: [0, 0],
        }
    }
}

enum PlayerState {
    Alive,
    Dead(u32),
    GameOver(u32),
    Cleared(u32),
}

/// One enemy ANM script with its sprite table and texture slot.
pub struct ScriptRef {
    pub tex: usize,
    pub instrs: Vec<AnmInstr>,
    pub sprites: HashMap<u32, Sprite>,
    pub tex_size: [f32; 2],
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

pub struct Stage {
    tick: u32,
    anim: u32,
    ecl: Ecl,
    world: World,
    enemies: Vec<Enemy>,
    anims: Vec<Option<AnmRunner>>,
    timeline_off: u32,
    timeline_time: i32,
    enemy_scripts: HashMap<i32, ScriptRef>,
    bullet_sprites: HashMap<u32, Sprite>,
    bullet_tex_size: [f32; 2],
    // player
    character: Character,
    /// player00.anm: sprites + scripts, for the banking/idle animation.
    player_sprites: HashMap<u32, Sprite>,
    player_scripts: HashMap<i32, Vec<AnmInstr>>,
    player_tex_size: [f32; 2],
    player_runner: AnmRunner,
    player_script_id: i32,
    /// Previous horizontal speed sign, to detect banking transitions.
    prev_hspeed: f32,
    /// Focus blend 0..1 for the orb slide-in/out animation.
    focus_anim: f32,
    pos: [f32; 2],
    lives: i32,
    bombs: i32,
    invuln: u32,
    bombing: u32,
    /// Player.cpp fireBulletTimer: -1 idle, else counts 0..=30 while shooting.
    fire_timer: i32,
    /// Deathbomb grace: frames left to bomb after a lethal hit (0 = not dying).
    dying: u32,
    graze: i64,
    /// Focus held last update, for drawing the orbs in their focused position.
    last_input_focus: bool,
    /// Position of the last enemy a player shot hit, for orb-amulet homing.
    last_enemy_hit: Option<[f32; 2]>,
    state: PlayerState,
    shots: Vec<Shot>,
    bomb_orbs: Vec<BombOrb>,
    items: Vec<Item>,
    particles: Vec<Particle>,
    score: i64,
    hiscore: i64,
    clear_bonus: i64,
    /// Running count for scoring power items collected at max power.
    power_item_count: usize,
    paused: bool,
    pause_cursor: usize,
    rand_item_table: usize,
    rand_item_spawn: usize,
    spell_active: bool,
    spell_name: String,
    spell_capturing: bool,
    spell_result: u32,
    spell_captured: bool,
    boss_bgm_started: bool,
    msg: Msg,
    dialogue: Dialogue,
    background: Option<Background>,
    pub events: Vec<Event>,
}

impl Stage {
    pub fn new(ecl: Ecl, enemy_scripts: HashMap<i32, ScriptRef>, etama: &Entry, player: &Entry, character: Character, msg: Msg, background: Option<Background>) -> Self {
        let timeline_off = ecl.timeline_offset;
        let player_scripts: HashMap<i32, Vec<AnmInstr>> =
            player.scripts.iter().map(|(id, instrs)| (*id as i32, instrs.clone())).collect();
        let idle = player_scripts.get(&0).cloned().unwrap_or_default();
        Self {
            tick: 0,
            anim: 0,
            ecl,
            world: World {
                rng: Rng::new(0x1234),
                difficulty: 1, // Normal
                rank: 16,
                player_pos: [FIELD_W / 2.0, FIELD_H - 40.0],
                bullets: Vec::new(),
                lasers: Vec::new(),
                events: Vec::new(),
                pending_spawns: Vec::new(),
                kill_trash: false,
                boss_present: false,
                power: 0,
            },
            enemies: Vec::new(),
            anims: Vec::new(),
            timeline_off,
            timeline_time: 0,
            enemy_scripts,
            bullet_sprites: etama.sprites.iter().map(|s| (s.index, s.clone())).collect(),
            bullet_tex_size: [etama.width as f32, etama.height as f32],
            character,
            player_sprites: player.sprites.iter().map(|s| (s.index, s.clone())).collect(),
            player_tex_size: [player.width as f32, player.height as f32],
            player_runner: AnmRunner::new(idle),
            player_scripts,
            player_script_id: 0,
            prev_hspeed: 0.0,
            focus_anim: 0.0,
            pos: [FIELD_W / 2.0, FIELD_H - 40.0],
            lives: 2,
            bombs: 3,
            invuln: 0,
            bombing: 0,
            fire_timer: -1,
            dying: 0,
            graze: 0,
            last_input_focus: false,
            last_enemy_hit: None,
            state: PlayerState::Alive,
            shots: Vec::new(),
            bomb_orbs: Vec::new(),
            items: Vec::new(),
            particles: Vec::new(),
            score: 0,
            hiscore: 0,
            clear_bonus: 0,
            power_item_count: 0,
            paused: false,
            pause_cursor: 0,
            rand_item_table: 0,
            rand_item_spawn: 0,
            spell_active: false,
            spell_name: String::new(),
            spell_capturing: false,
            spell_result: 0,
            spell_captured: false,
            boss_bgm_started: false,
            msg,
            dialogue: Dialogue::default(),
            background,
            events: vec![Event::Bgm("th06_02.wav")],
        }
    }

    pub fn set_lives(&mut self, lives: i32) {
        self.lives = lives;
    }

    pub fn set_hiscore(&mut self, hiscore: i64) {
        self.hiscore = hiscore;
    }

    pub fn background_scene(&self) -> Option<BgScene> {
        self.background.as_ref().map(|b| b.scene())
    }

    pub fn update(&mut self, input: &Input) -> Vec<DrawCmd> {
        // The pause menu freezes the whole game (no tick, no sim).
        if self.paused {
            self.run_pause_menu(input);
            return self.draw();
        }
        if input.pressed(Key::Pause)
            && matches!(self.state, PlayerState::Alive | PlayerState::Dead(_))
        {
            self.paused = true;
            self.pause_cursor = 0;
            return self.draw();
        }

        self.tick += 1;
        self.anim += 1;
        if let Some(bg) = &mut self.background {
            bg.tick();
        }

        // Player state machine.
        let mut respawn = false;
        match &mut self.state {
            PlayerState::Alive => {}
            PlayerState::Dead(t) => {
                *t -= 1;
                if *t == 0 {
                    if self.lives < 0 {
                        self.state = PlayerState::GameOver(180);
                    } else {
                        respawn = true;
                    }
                }
            }
            PlayerState::GameOver(t) | PlayerState::Cleared(t) => {
                *t -= 1;
                if *t == 0 {
                    self.events.push(Event::BackToTitle);
                    return self.draw();
                }
            }
        }
        if respawn {
            self.pos = [FIELD_W / 2.0, FIELD_H - 40.0];
            self.invuln = 180;
            self.state = PlayerState::Alive;
        }
        // The player can still move (but not shoot/bomb) during dialogue.
        if matches!(self.state, PlayerState::Alive) {
            self.update_player(input);
        }
        if self.dialogue.active {
            self.run_dialogue(input);
        }
        self.invuln = self.invuln.saturating_sub(1);
        self.spell_result = self.spell_result.saturating_sub(1);
        self.world.player_pos = self.pos;
        self.player_runner.tick();

        // Only the timeline freezes during dialogue (EnemyManager gates it on
        // !HasCurrentMsgIdx), which keeps the boss's scripted attack from
        // triggering. Bullets, enemies and collision keep running so the
        // player must still dodge whatever is already on screen.
        if !self.dialogue.active {
            self.run_timeline();
        }
        self.update_enemies();
        self.update_shots();
        self.update_bullets();
        self.update_items();
        self.update_particles();
        self.collide();
        self.drain_world_events();

        // Stage clear: timeline exhausted and field empty.
        if matches!(self.state, PlayerState::Alive)
            && self.timeline_done()
            && self.enemies.is_empty()
            && !self.world.boss_present
        {
            // Clear bonus + persist the score (capped graze/lives bonus).
            let bonus = self.lives.max(0) as i64 * 1_000_000 + self.bombs.max(0) as i64 * 300_000;
            self.score += bonus;
            self.clear_bonus = bonus;
            if self.score > self.hiscore {
                self.hiscore = self.score;
            }
            self.events.push(Event::SaveScore(self.hiscore));
            self.state = PlayerState::Cleared(420);
        }

        self.draw()
    }

    const PAUSE_OPTIONS: [&'static str; 3] = ["Resume", "Return to Title", "Quit Game"];

    fn run_pause_menu(&mut self, input: &Input) {
        if input.pressed(Key::Pause) {
            self.paused = false;
            return;
        }
        let n = Self::PAUSE_OPTIONS.len();
        if input.pressed(Key::Up) {
            self.pause_cursor = (self.pause_cursor + n - 1) % n;
            self.events.push(Event::Sfx("tan00"));
        }
        if input.pressed(Key::Down) {
            self.pause_cursor = (self.pause_cursor + 1) % n;
            self.events.push(Event::Sfx("tan00"));
        }
        if input.pressed(Key::Shoot) || input.pressed(Key::Enter) {
            match self.pause_cursor {
                0 => self.paused = false,
                1 => self.events.push(Event::BackToTitle),
                _ => self.events.push(Event::Quit),
            }
        }
    }

    fn timeline_done(&self) -> bool {
        self.ecl
            .timeline_at(self.timeline_off)
            .map(|t| t.time < 0)
            .unwrap_or(true)
    }

    /// Port of EnemyManager::RunEclTimeline (dialogue ops are skipped — no
    /// MSG interpreter yet).
    fn run_timeline(&mut self) {
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
                let Some(raw) = self.ecl.timeline_at(self.timeline_off) else { return };
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
                    a0: if raw.args.len() >= 4 { raw.arg_i32(0) } else { 0 },
                    a1: if raw.args.len() >= 8 { raw.arg_i32(4) } else { 0 },
                }
            };
            if t.time < 0 {
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
                    8 => self.start_dialogue(t.arg0 as usize),
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
                    12 => {
                        // Wait for the boss slot to clear.
                        if self.enemies.iter().any(|e| e.is_boss && e.occupied) {
                            return; // do not advance time
                        }
                    }
                    _ => {}
                }
            }
            self.timeline_off += t.size;
        }
        self.timeline_time += 1;
    }

    fn spawn(&mut self, req: SpawnReq) {
        if std::env::var_os("TH06_TRACE").is_some() {
            eprintln!(
                "[{}] spawn sub={} pos=({:.0},{:.0}) life={}",
                self.timeline_time, req.sub, req.pos[0], req.pos[1], req.life
            );
        }
        if let Some(e) = Enemy::spawn(&self.ecl, &mut self.world, &req) {
            self.enemies.push(e);
            self.anims.push(None);
        }
        self.flush_spawns();
    }

    fn flush_spawns(&mut self) {
        while let Some(req) = self.world.pending_spawns.pop() {
            if let Some(e) = Enemy::spawn(&self.ecl, &mut self.world, &req) {
                self.enemies.push(e);
                self.anims.push(None);
            }
        }
    }

    fn update_enemies(&mut self) {
        for i in 0..self.enemies.len() {
            let e = &mut self.enemies[i];
            if !e.occupied {
                continue;
            }
            e.frame_move();
            if !e.update_bounds() {
                e.occupied = false;
                continue;
            }
            e.handle_callbacks(&self.ecl, &mut self.world);
            e.run_ecl(&self.ecl, &mut self.world);

            // Refresh the ANM runner when the script changed.
            if e.anm_dirty {
                e.anm_dirty = false;
                self.anims[i] = self
                    .enemy_scripts
                    .get(&e.anm_script)
                    .map(|s| AnmRunner::new(s.instrs.clone()));
            }
            if let Some(anim) = &mut self.anims[i] {
                anim.tick();
            }
        }
        self.flush_spawns();

        if self.world.kill_trash {
            self.world.kill_trash = false;
            for i in 0..self.enemies.len() {
                let e = &mut self.enemies[i];
                if e.occupied && !e.is_boss {
                    e.life = 0;
                    e.on_death(&self.ecl, &mut self.world);
                }
            }
        }

        // Compact dead slots.
        let mut kept_anims = Vec::with_capacity(self.anims.len());
        let mut idx = 0;
        self.enemies.retain(|e| {
            let keep = e.occupied;
            if keep {
                kept_anims.push(self.anims[idx].take());
            }
            idx += 1;
            keep
        });
        self.anims = kept_anims;
    }

    fn update_player(&mut self, input: &Input) {
        let focus = input.held(Key::Focus);
        self.last_input_focus = focus;
        let speed = if focus { 2.0 } else { 4.0 };
        let mut d = [0.0f32, 0.0f32];
        if input.held(Key::Left) {
            d[0] -= 1.0;
        }
        if input.held(Key::Right) {
            d[0] += 1.0;
        }
        if input.held(Key::Up) {
            d[1] -= 1.0;
        }
        if input.held(Key::Down) {
            d[1] += 1.0;
        }
        if d[0] != 0.0 && d[1] != 0.0 {
            let inv = 1.0 / 2.0f32.sqrt();
            d[0] *= inv;
            d[1] *= inv;
        }
        self.pos[0] = (self.pos[0] + d[0] * speed).clamp(12.0, FIELD_W - 12.0);
        self.pos[1] = (self.pos[1] + d[1] * speed).clamp(20.0, FIELD_H - 20.0);

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

        // Orb slide blend toward focused (1) / unfocused (0) over 8 frames.
        let target = if focus { 1.0 } else { 0.0 };
        self.focus_anim += (target - self.focus_anim).clamp(-0.125, 0.125);

        // Shooting and bombing are disabled during dialogue (Player.cpp gates
        // both on !HasCurrentMsgIdx), but the player can still move and dodge.
        let can_act = !self.dialogue.active;

        // Player.cpp StartFireBulletTimer / UpdateFireBulletsTimer: holding
        // Shoot (re)starts the idle fire timer, which then counts 0..=29 and
        // spawns the matching table entries each frame before resetting.
        if can_act {
            if input.held(Key::Shoot) && self.fire_timer < 0 {
                self.fire_timer = 0;
            }
            if self.fire_timer >= 0 {
                self.spawn_player_bullets(self.fire_timer);
                self.fire_timer += 1;
                if self.fire_timer >= 30 {
                    self.fire_timer = -1;
                }
            }
        } else {
            self.fire_timer = -1;
        }

        if self.bombing > 0 {
            self.update_bomb();
        } else if can_act && input.pressed(Key::Bomb) && self.bombs > 0 {
            self.fire_bomb();
        } else if self.dying > 0 {
            // No bomb this frame: tick the deathbomb window, commit on expiry.
            self.dying -= 1;
            if self.dying == 0 {
                self.commit_death();
            }
        }
    }

    /// Fantasy Seal (ReimuA bomb): 8 orbs fly out from the player, 360-frame
    /// invulnerability, ~300-frame duration.
    fn fire_bomb(&mut self) {
        self.dying = 0; // a bomb in the deathbomb window cancels the death
        self.bombs -= 1;
        self.bombing = 300;
        self.invuln = self.invuln.max(360);
        self.spell_capturing = false; // bombing forfeits the capture
        self.world.bullets.clear();
        self.cancel_lasers();
        self.bomb_orbs.clear();
        for i in 0..8 {
            let a = i as f32 / 8.0 * std::f32::consts::TAU + 0.39;
            self.bomb_orbs.push(BombOrb {
                pos: self.pos,
                vel: [a.cos() * 4.0, a.sin() * 4.0],
                spd: 4.0,
                hue: i as f32 / 8.0,
            });
        }
        self.spawn_burst(self.pos, 24, 6.0, [0.6, 0.8, 1.0], 16.0);
        self.events.push(Event::Sfx("power1"));
    }

    /// Per-frame bomb update: clears bullets, homes the orbs onto the nearest
    /// enemy, and damages enemies inside each orb's region (8/frame).
    fn update_bomb(&mut self) {
        self.bombing -= 1;
        self.world.bullets.clear();
        // Pick the nearest enemy as the shared homing pivot.
        let pivot = self
            .enemies
            .iter()
            .filter(|e| e.occupied && e.interactable)
            .min_by(|a, b| {
                let da = (a.pos[0] - self.pos[0]).hypot(a.pos[1] - self.pos[1]);
                let db = (b.pos[0] - self.pos[0]).hypot(b.pos[1] - self.pos[1]);
                da.total_cmp(&db)
            })
            .map(|e| [e.pos[0], e.pos[1]]);
        for o in &mut self.bomb_orbs {
            if let Some(t) = pivot {
                // BombData.cpp homing: steer toward the pivot, speed capped 10.
                let mut vx = t[0] - o.pos[0];
                let mut vy = t[1] - o.pos[1];
                let mut len = (vx * vx + vy * vy).sqrt() / (o.spd / 8.0).max(0.01);
                if len < 1.0 {
                    len = 1.0;
                }
                vx = vx / len + o.vel[0];
                vy = vy / len + o.vel[1];
                len = (vx * vx + vy * vy).sqrt().max(0.01);
                o.spd = len.min(10.0).max(1.0);
                o.vel = [vx * o.spd / len, vy * o.spd / len];
            }
            o.pos[0] += o.vel[0];
            o.pos[1] += o.vel[1];
        }
        // Each orb is a 48px damage region dealing 8/frame.
        for e in &mut self.enemies {
            if !e.occupied || !e.interactable || !e.damageable {
                continue;
            }
            for o in &self.bomb_orbs {
                if (e.pos[0] - o.pos[0]).abs() < 24.0 && (e.pos[1] - o.pos[1]).abs() < 24.0 {
                    e.life -= 8;
                }
            }
        }
        if self.bombing == 0 {
            self.bomb_orbs.clear();
        }
    }

    /// Player::Die plus the respawn bookkeeping: lose a life and 16 power.
    fn commit_death(&mut self) {
        self.lives -= 1;
        self.world.power = (self.world.power - 16).max(0);
        self.bombs = 3;
        self.spell_capturing = false; // dying forfeits the capture
        self.world.bullets.clear();
        self.cancel_lasers();
        self.spawn_burst(self.pos, 20, 4.0, [1.0, 0.5, 0.5], 12.0);
        self.state = PlayerState::Dead(60);
        self.events.push(Event::Sfx("pldead00"));
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

    /// HandlePlayerInputs orb offsets, blended over the focus animation:
    /// unfocused (24, 0) slides to focused (8, -32).
    fn orb_positions(&self) -> [[f32; 2]; 2] {
        let a = self.focus_anim;
        let h = 24.0 - 16.0 * a;
        let v = -32.0 * a;
        [[self.pos[0] - h, self.pos[1] + v], [self.pos[0] + h, self.pos[1] + v]]
    }

    /// SpawnBullets / FireSingleBullet: fire every table entry of the current
    /// character's power rank whose timing matches the fire timer.
    fn spawn_player_bullets(&mut self, timer: i32) {
        let rank = self.character.ranks()[power_rank(self.world.power)];
        let orbs = self.orb_positions();
        let orbs_shown = self.world.power >= 8;
        let mut fired_main = false;
        for def in rank {
            if timer % def.wait != def.frame {
                continue;
            }
            if def.spawn != 0 && !orbs_shown {
                continue; // orb shots only exist at power >= 8
            }
            let base = match def.spawn {
                1 => orbs[0],
                2 => orbs[1],
                _ => self.pos,
            };
            let a = def.dir_deg.to_radians();
            self.shots.push(Shot {
                pos: [base[0] + def.motion[0], base[1] + def.motion[1]],
                vel: [a.cos() * def.vel, a.sin() * def.vel],
                damage: def.damage,
                bt: def.bt,
                age: 0,
                spd: def.vel,
            });
            fired_main |= def.bt == 0;
        }
        if fired_main {
            self.events.push(Event::Sfx("plst00"));
        }
    }

    /// UpdatePlayerBullets BULLET_TYPE_1: orb amulets steer toward the last
    /// enemy hit for their first 40 frames, then accelerate up to speed 10.
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
                s.spd = len.min(10.0).max(1.0);
                if len > 0.0 {
                    s.vel = [vx * s.spd / len, vy * s.spd / len];
                }
            }
            _ => {
                if s.spd < 10.0 {
                    s.spd += 1.0 / 3.0;
                    let len = (s.vel[0] * s.vel[0] + s.vel[1] * s.vel[1]).sqrt();
                    if len > 0.0 {
                        s.vel = [s.vel[0] * s.spd / len, s.vel[1] * s.spd / len];
                    }
                }
            }
        }
    }

    fn update_shots(&mut self) {
        let target = self.last_enemy_hit;
        for s in &mut self.shots {
            match s.bt {
                1 => Self::home_shot(s, target), // Reimu A homing orb
                2 => s.vel[1] -= 0.3,            // Marisa A orb-missile gravity
                _ => {}
            }
            s.pos[0] += s.vel[0];
            s.pos[1] += s.vel[1];
            s.age += 1;
        }
        self.shots.retain(|s| {
            s.pos[1] > -50.0 && s.pos[1] < FIELD_H + 50.0 && s.pos[0] > -50.0 && s.pos[0] < FIELD_W + 50.0
        });
    }

    fn update_bullets(&mut self) {
        for b in &mut self.world.bullets {
            b.timer += 1;
            // Ex behaviors, ported from BulletManager::OnUpdate.
            if b.ex_flags & 1 != 0 {
                if b.timer <= 16 {
                    // Spawn boost: extra speed decaying 5 -> 0 over 16 frames.
                    let boost = 5.0 - b.timer as f32 * 5.0 / 16.0;
                    b.pos[0] += b.angle.cos() * boost;
                    b.pos[1] += b.angle.sin() * boost;
                } else {
                    b.ex_flags &= !1;
                }
            } else if b.ex_flags & 0x10 != 0 {
                if b.timer >= b.ex_int0 {
                    b.ex_flags &= !0x10;
                } else {
                    let vx = b.angle.cos() * b.speed + b.ex_accel[0];
                    let vy = b.angle.sin() * b.speed + b.ex_accel[1];
                    b.speed = (vx * vx + vy * vy).sqrt();
                    b.angle = vy.atan2(vx);
                }
            } else if b.ex_flags & 0x20 != 0 {
                if b.timer >= b.ex_int0 {
                    b.ex_flags &= !0x20;
                } else {
                    b.angle += b.ex_f[1];
                    b.speed += b.ex_f[0];
                }
            }
            if b.ex_flags & 0x40 != 0 && b.ex_int0 > 0 && b.timer >= b.ex_int0 * (b.ex_count + 1) {
                b.ex_count += 1;
                if b.ex_count >= b.ex_int1 {
                    b.ex_flags &= !0x40;
                }
                b.angle += b.ex_f[0];
                b.speed = b.ex_f[1];
            }
            let factor = if b.spawn_delay > 0 {
                b.spawn_delay -= 1;
                1.0 / 2.5
            } else {
                1.0
            };
            b.pos[0] += b.angle.cos() * b.speed * factor;
            b.pos[1] += b.angle.sin() * b.speed * factor;
        }
        self.world.bullets.retain(|b| {
            b.pos[0] > -20.0 && b.pos[0] < FIELD_W + 20.0 && b.pos[1] > -20.0 && b.pos[1] < FIELD_H + 20.0
        });

        // Lasers (state machine from BulletManager::OnUpdate).
        for l in &mut self.world.lasers {
            if !l.in_use {
                continue;
            }
            l.end_offset += l.speed;
            if l.start_length < l.end_offset - l.start_offset {
                l.start_offset = l.end_offset - l.start_length;
            }
            if l.start_offset < 0.0 {
                l.start_offset = 0.0;
            }
            l.timer += 1;
            match l.state {
                0 => {
                    if l.timer >= l.start_time {
                        l.state = 1;
                        l.timer = 0;
                    }
                }
                1 => {
                    if l.timer >= l.duration {
                        l.state = 2;
                        l.timer = 0;
                    }
                }
                _ => {
                    if l.timer >= l.despawn_duration {
                        l.in_use = false;
                    }
                }
            }
        }
    }

    fn bullet_radius(&self, b: &Bullet) -> f32 {
        match self.bullet_sprites.get(&b.sprite).map(|s| s.height as u32) {
            Some(h) if h <= 8 => 2.0,
            Some(h) if h <= 16 => 3.2,
            Some(_) => 8.0,
            None => 3.0,
        }
    }

    fn collide(&mut self) {
        // Player shots vs enemies. Damage is the table value; orb amulets
        // remember where they last connected so they can home (positionOf-
        // LastEnemyHit, reset each frame). Hitting an enemy scores
        // (min(dmg,70)/5)*10 (EnemyManager.cpp), regardless of any spellcard
        // damage cut; the cut only reduces the life lost.
        let spell_penalty = self.spell_active && self.bombing == 0;
        let mut last_hit = None;
        let mut dmg_score: i64 = 0;
        for s in &mut self.shots {
            for e in &mut self.enemies {
                if !e.occupied || !e.interactable {
                    continue;
                }
                let r = (e.hitbox[0].max(e.hitbox[1])) / 2.0 + 6.0;
                let dx = s.pos[0] - e.pos[0];
                let dy = s.pos[1] - e.pos[1];
                if dx * dx + dy * dy < r * r {
                    dmg_score += (s.damage.min(70) / 5 * 10) as i64;
                    let dmg = if spell_penalty {
                        if s.damage > 7 { s.damage / 7 } else { 1 }
                    } else {
                        s.damage
                    };
                    if e.damageable {
                        e.life -= dmg;
                    }
                    last_hit = Some([e.pos[0], e.pos[1]]);
                    s.pos[1] = -100.0;
                    break;
                }
            }
        }
        self.score += dmg_score;
        self.last_enemy_hit = last_hit;
        self.shots.retain(|s| s.pos[1] > -90.0);

        // Enemy deaths award the enemy's score value (default 100).
        let mut drops: Vec<([f32; 2], i16)> = Vec::new();
        let mut death_fx: Vec<[f32; 2]> = Vec::new();
        for i in 0..self.enemies.len() {
            let e = &mut self.enemies[i];
            if e.occupied && e.interactable && e.life <= 0 {
                drops.push(([e.pos[0], e.pos[1]], e.item_drop));
                if !e.is_boss {
                    death_fx.push([e.pos[0], e.pos[1]]);
                }
                self.score += e.score as i64;
                e.on_death(&self.ecl, &mut self.world);
                self.events.push(Event::Sfx("enep00"));
            }
        }
        for pos in death_fx {
            // Bright flash ring + a puff, like the original enemy pop.
            self.spawn_burst(pos, 12, 3.0, [1.0, 0.95, 0.7], 7.0);
            self.particles.push(Particle {
                pos,
                vel: [0.0, 0.0],
                life: 12.0,
                max_life: 12.0,
                size: 10.0,
                color: [1.0, 1.0, 0.9],
            });
        }
        for (pos, drop) in drops {
            self.spawn_drop(pos, drop);
        }
        self.flush_spawns();

        // Bullets / lasers / enemy bodies vs player. Player.cpp uses an AABB:
        // a tiny hitbox (half-extent 1.25) for kills, expanded by 20 (bullets)
        // or 48 (lasers) for grazes. Grazing is allowed during the post-respawn
        // invulnerability, but never while dying or bombing.
        const PH: f32 = 1.25;
        let can_graze = matches!(self.state, PlayerState::Alive) && self.dying == 0 && self.bombing == 0;
        if !can_graze {
            return;
        }
        let can_be_hit = self.invuln == 0;
        let p = self.pos;
        let radii: Vec<f32> = self.world.bullets.iter().map(|b| self.bullet_radius(b)).collect();

        let mut kill = false;
        let mut graze_count: i64 = 0;
        for (b, &r) in self.world.bullets.iter_mut().zip(&radii) {
            let dx = (b.pos[0] - p[0]).abs();
            let dy = (b.pos[1] - p[1]).abs();
            if can_be_hit && dx < r + PH && dy < r + PH {
                kill = true;
            } else if !b.grazed && dx < r + PH + 20.0 && dy < r + PH + 20.0 {
                b.grazed = true;
                graze_count += 1;
            }
        }
        for l in &self.world.lasers {
            if !l.in_use {
                continue;
            }
            let hitbox_live = match l.state {
                0 => l.timer >= l.hitbox_start,
                1 => true,
                _ => false,
            };
            if !hitbox_live {
                continue;
            }
            let (dx, dy) = (p[0] - l.pos[0], p[1] - l.pos[1]);
            let (c, s) = (l.angle.cos(), l.angle.sin());
            let along = dx * c + dy * s;
            let across = (-dx * s + dy * c).abs();
            let half = l.width / 4.0;
            if can_be_hit && along >= l.start_offset && along <= l.end_offset && across < half + PH {
                kill = true;
            } else if along >= l.start_offset - 48.0
                && along <= l.end_offset + 48.0
                && across < half + PH + 48.0
            {
                graze_count += 1; // lasers graze every frame, like the original
            }
        }
        if can_be_hit
            && self.enemies.iter().any(|e| {
                if !e.occupied || !e.collidable || !e.interactable {
                    return false;
                }
                let r = (e.hitbox[0].max(e.hitbox[1])) / 1.5 / 2.0 + 2.0;
                let dx = e.pos[0] - p[0];
                let dy = e.pos[1] - p[1];
                dx * dx + dy * dy < r * r
            })
        {
            kill = true;
        }

        if graze_count > 0 {
            self.graze += graze_count;
            self.score += 500 * graze_count;
            self.events.push(Event::Sfx("graze"));
        }
        if kill && std::env::var_os("TH06_GOD").is_none() {
            // Open the deathbomb window; commit_death fires if it expires unbombed.
            self.dying = DEATHBOMB_FRAMES;
        }
    }

    fn start_dialogue(&mut self, idx: usize) {
        let Some(&off) = self.msg.offsets.get(idx) else { return };
        self.dialogue = Dialogue { active: true, off, ..Default::default() };
        // Dialogue clears the field of bullets in practice.
        self.world.bullets.clear();
        self.cancel_lasers();
        // The boss theme starts at the pre-boss confrontation. Stage 1's MSG
        // has no MUSIC instruction, so the first dialogue is the cue (the
        // midboss has no dialogue, so it keeps the stage theme).
        if !self.boss_bgm_started {
            self.boss_bgm_started = true;
            self.events.push(Event::Bgm("th06_03.wav"));
        }
    }

    /// Port of GuiImpl::RunMsg (text/wait/music subset).
    fn run_dialogue(&mut self, input: &Input) {
        let d = &mut self.dialogue;
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
                    // TEXTDIALOGUE
                    let color = i.arg_i16(0).clamp(0, 1) as usize;
                    let line = i.arg_i16(2).clamp(0, 1) as usize;
                    if line == 0 {
                        d.lines[1].clear();
                    }
                    d.lines[line] = i.arg_str(4);
                    d.line_colors[line] = color;
                    d.frames_pause = 0;
                }
                4 => {
                    // WAIT n frames; Z after 8 frames advances.
                    let wait = i.arg_i32(0) as u16;
                    let advance = (input.pressed(Key::Shoot) || input.pressed(Key::Enter))
                        && d.frames_pause >= 8;
                    if d.frames_pause < wait && !advance {
                        d.frames_pause += 1;
                        return; // time frozen during the pause
                    }
                }
                1 | 2 => {
                    // PORTRAITANMSCRIPT / PORTRAITANMSPRITE: pick portrait
                    // side and expression. anmScriptIdx's parity selects one
                    // of the two expression sprites in the face sheet.
                    let idx = i.arg_i16(0).clamp(0, 1) as usize;
                    let expr = (i.arg_i16(2).max(0) as usize) & 1;
                    d.portrait_shown[idx] = true;
                    d.portrait_expr[idx] = expr;
                    d.portrait_active = idx as i32;
                }
                6 => d.ecl_resumed = true, // ECLRESUME
                7 => {
                    // MUSIC: stage 1 track 1 is the boss theme.
                    let track = i.arg_i32(0);
                    self.boss_bgm_started = true;
                    self.events.push(Event::Bgm(if track == 1 {
                        "th06_03.wav"
                    } else {
                        "th06_02.wav"
                    }));
                }
                10 => {
                    // MSGHALT: nothing left to show.
                    d.active = false;
                    return;
                }
                _ => {} // portraits / anm interrupts: later
            }
            d.off += i.size;
        }
        d.timer += 1;
    }

    fn spawn_drop(&mut self, pos: [f32; 2], drop: i16) {
        let kind = match drop {
            -2 => return,            // ITEM_NO_ITEM
            -1 => {
                // ITEM_RANDOM_ITEM: every third enemy drops, following the
                // fixed pattern table.
                self.rand_item_spawn += 1;
                if (self.rand_item_spawn - 1) % 3 != 0 {
                    return;
                }
                let k = RANDOM_ITEMS[self.rand_item_table];
                self.rand_item_table = (self.rand_item_table + 1) % RANDOM_ITEMS.len();
                k
            }
            k => k as i32,
        };
        self.items.push(Item { pos, vy: -2.2, kind });
    }

    /// Burst of fading puffs, used for enemy deaths and pickups.
    fn spawn_burst(&mut self, pos: [f32; 2], count: u32, speed: f32, color: [f32; 3], size: f32) {
        for i in 0..count {
            let a = i as f32 / count as f32 * std::f32::consts::TAU
                + self.world.rng.f32_in_range(0.5);
            let s = speed * (0.5 + self.world.rng.f32_zero_to_one());
            self.particles.push(Particle {
                pos,
                vel: [a.cos() * s, a.sin() * s],
                life: 18.0,
                max_life: 18.0,
                size,
                color,
            });
        }
    }

    fn update_particles(&mut self) {
        for p in &mut self.particles {
            p.pos[0] += p.vel[0];
            p.pos[1] += p.vel[1];
            p.vel[0] *= 0.86;
            p.vel[1] *= 0.86;
            p.life -= 1.0;
        }
        self.particles.retain(|p| p.life > 0.0);
    }

    fn update_items(&mut self) {
        let player = self.pos;
        let alive = matches!(self.state, PlayerState::Alive);
        let magnet = alive && self.world.power >= 128 && player[1] < 128.0;
        let mut collected: Vec<(i32, [f32; 2])> = Vec::new();
        for it in &mut self.items {
            if magnet {
                let dx = player[0] - it.pos[0];
                let dy = player[1] - it.pos[1];
                let len = (dx * dx + dy * dy).sqrt().max(0.001);
                it.pos[0] += dx / len * 8.0;
                it.pos[1] += dy / len * 8.0;
            } else {
                it.vy = (it.vy + 0.04).min(2.5);
                it.pos[1] += it.vy;
            }
            if alive {
                let dx = player[0] - it.pos[0];
                let dy = player[1] - it.pos[1];
                if dx * dx + dy * dy < 18.0 * 18.0 {
                    collected.push((it.kind, it.pos));
                    it.kind = -100; // mark
                }
            }
        }
        self.items.retain(|it| it.kind != -100 && it.pos[1] < FIELD_H + 16.0);
        for (kind, pos) in collected {
            let color = match kind {
                1 => [0.5, 0.7, 1.0],       // point = blue
                3 | 5 => [1.0, 0.6, 0.6],   // bomb/life = red
                _ => [1.0, 0.4, 0.4],       // power = red
            };
            match kind {
                0 => self.collect_power(1),
                2 => self.collect_power(8),
                4 => self.world.power = 128,
                1 => {
                    // ITEM_POINT (Normal, calculatePointScore): 100000 at the
                    // collection line (y < 128), else 60000 - (y-128)*100.
                    let y = pos[1] as i64;
                    self.score += if y < 128 { 100_000 } else { (60_000 - (y - 128) * 100).max(0) };
                }
                3 => self.bombs = (self.bombs + 1).min(8),
                5 => self.lives = (self.lives + 1).min(8),
                _ => {}
            }
            self.spawn_burst(pos, 4, 1.5, color, 6.0);
            self.events.push(Event::Sfx("item00"));
        }
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
        }
    }

    fn cancel_lasers(&mut self) {
        for l in &mut self.world.lasers {
            if l.in_use && l.state < 2 {
                l.state = 2;
                l.timer = 0;
            }
        }
    }

    fn drain_world_events(&mut self) {
        let events: Vec<WorldEvent> = self.world.events.drain(..).collect();
        for ev in events {
            match ev {
                WorldEvent::Sfx(idx) => {
                    // SoundIdx -> our named sfx; a few common ones mapped.
                    let name = match idx {
                        0 => "plst00",
                        1 => "enep00",
                        5 => "power1",
                        16 => "tan00",
                        17 => "tan01",
                        18 => "tan02",
                        22 => "cat00",
                        _ => "tan00",
                    };
                    self.events.push(Event::Sfx(name));
                }
                WorldEvent::SpellcardStart(id, _raw) => {
                    self.spell_active = true;
                    self.spell_name = spellcard_name(id).to_string();
                    self.spell_capturing = true;
                    self.world.bullets.clear();
                    self.events.push(Event::Sfx("cat00"));
                }
                WorldEvent::SpellcardEnd => {
                    if self.spell_active {
                        self.spell_result = 120;
                        self.spell_captured = self.spell_capturing;
                    }
                    self.spell_active = false;
                    self.world.bullets.clear();
                }
                WorldEvent::BulletCancel => {
                    self.world.bullets.clear();
                    self.cancel_lasers();
                }
                WorldEvent::BossSet(_present) => {
                    // Boss music is driven by the pre-boss dialogue, not here:
                    // BossSet also fires for the midboss, which keeps the stage
                    // theme.
                }
                WorldEvent::EnemyDeath(pos) => {
                    self.spawn_burst(pos, 10, 3.0, [1.0, 0.95, 0.7], 10.0);
                    self.events.push(Event::Sfx("enep00"));
                }
                WorldEvent::DropItem(pos, kind) => {
                    if kind >= 0 {
                        self.items.push(Item { pos, vy: -2.2, kind });
                    }
                }
            }
        }
    }

    fn draw(&self) -> Vec<DrawCmd> {
        let mut cmds = Vec::with_capacity(96 + self.world.bullets.len());
        if self.background.is_some() {
            // The 3D background fills the field; only dim it during spells.
            if self.spell_active {
                cmds.push(rect([FIELD_X, FIELD_Y, FIELD_W, FIELD_H], [0.0, 0.0, 0.05, 0.45]));
            }
        } else {
            let base = if self.spell_active { 0.02 } else { 0.07 };
            cmds.push(rect(
                [FIELD_X, FIELD_Y, FIELD_W, FIELD_H],
                [base, base * 0.6, base * 0.9, 1.0],
            ));
        }

        // Spellcard aura: soft pulsing glows behind the boss (BOMB_GLOW is a
        // round radial sprite, so this reads as an aura rather than squares).
        if self.spell_active {
            if let Some(boss) = self.enemies.iter().find(|e| e.is_boss && e.occupied) {
                let [gx, gy, gw, gh] = BOMB_GLOW.rect;
                let t = self.anim as f32;
                for i in 0..4 {
                    let s = 120.0 + i as f32 * 55.0 + (t * 0.06 + i as f32).sin() * 16.0;
                    let a = (0.22 - i as f32 * 0.045) * (0.7 + 0.3 * (t * 0.05).sin());
                    cmds.push(DrawCmd {
                        tex: TEX_PLAYER,
                        dst: [
                            FIELD_X + boss.pos[0] - s / 2.0,
                            FIELD_Y + boss.pos[1] - s / 2.0,
                            s,
                            s,
                        ],
                        src: [gx / 256.0, gy / 256.0, (gx + gw) / 256.0, (gy + gh) / 256.0],
                        tint: [0.95, 0.35, 0.6, a.max(0.0)],
                        rot: t * 0.015 * (i as f32 + 1.0),
                    });
                }
            }
        }

        // Enemies via their ANM state.
        for (e, anim) in self.enemies.iter().zip(&self.anims) {
            if !e.occupied || e.invisible {
                continue;
            }
            let Some(anim) = anim else { continue };
            let Some(script) = self.enemy_scripts.get(&e.anm_script) else { continue };
            let Some(idx) = anim.sprite else { continue };
            let Some(sp) = script.sprites.get(&idx) else { continue };
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
            cmds.push(DrawCmd {
                tex: script.tex,
                dst: [
                    FIELD_X + e.pos[0] - w / 2.0,
                    FIELD_Y + e.pos[1] - h / 2.0,
                    w,
                    h,
                ],
                src: [u0, sy / th, u1, (sy + sp.height) / th],
                tint: [1.0, 1.0, 1.0, anim.alpha],
                rot: 0.0,
            });
        }

        // Boss HP bar.
        if let Some(boss) = self.enemies.iter().find(|e| e.is_boss && e.occupied) {
            let frac = (boss.life.max(0) as f32 / boss.max_life.max(1) as f32).clamp(0.0, 1.0);
            cmds.push(rect(
                [FIELD_X + 8.0, FIELD_Y + 4.0, (FIELD_W - 16.0) * frac, 4.0],
                [0.9, 0.15, 0.15, 0.9],
            ));
        }

        // Items (etama3 sprites 0..6, index = item kind).
        let [btw, bth] = self.bullet_tex_size;
        for it in &self.items {
            let Some(sp) = self.bullet_sprites.get(&(it.kind as u32)) else { continue };
            cmds.push(DrawCmd {
                tex: TEX_BULLET,
                dst: [
                    FIELD_X + it.pos[0] - sp.width / 2.0,
                    FIELD_Y + it.pos[1] - sp.height / 2.0,
                    sp.width,
                    sp.height,
                ],
                src: [sp.x / btw, sp.y / bth, (sp.x + sp.width) / btw, (sp.y + sp.height) / bth],
                tint: [1.0, 1.0, 1.0, 1.0],
                rot: 0.0,
            });
        }

        // Player shots (ReimuA fires amulets, both main and orb).
        for s in &self.shots {
            cmds.push(sprite_at(AMULET, s.pos, 0.85));
        }

        // Player: the player00.anm runner drives the idle/banking sprite.
        if matches!(self.state, PlayerState::Alive | PlayerState::Cleared(_)) {
            let blink = (self.invuln > 0 || self.dying > 0) && (self.anim / 4) % 2 == 0;
            if !blink {
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
                cmds.push(DrawCmd {
                    tex: TEX_PLAYER,
                    dst: [
                        FIELD_X + self.pos[0] - sp.width / 2.0,
                        FIELD_Y + self.pos[1] - sp.height / 2.0,
                        sp.width,
                        sp.height,
                    ],
                    src: [u0, sp.y / pth, u1, (sp.y + sp.height) / pth],
                    tint: [1.0, 1.0, 1.0, 1.0],
                    rot: 0.0,
                });
            }
            // Floating option orbs (appear at power >= 8).
            if self.world.power >= 8 {
                for o in self.orb_positions() {
                    let mut orb = sprite_at(AMULET, o, 0.8);
                    orb.tint = [1.0, 0.6, 0.7, 0.95];
                    cmds.push(orb);
                }
            }
            // Focus hitbox marker: fades/spins in while focusing.
            if self.focus_anim > 0.01 {
                let mut m = sprite_at(HITBOX_MARKER, self.pos, 0.95);
                m.tint = [1.0, 1.0, 1.0, self.focus_anim];
                m.rot = self.anim as f32 * 0.1;
                cmds.push(m);
            }
        }

        // Fantasy Seal bomb orbs: spinning rainbow glows tracking enemies.
        for o in &self.bomb_orbs {
            let h = o.hue * std::f32::consts::TAU;
            let col = [
                0.55 + 0.45 * h.cos(),
                0.55 + 0.45 * (h + 2.094).cos(),
                0.55 + 0.45 * (h + 4.188).cos(),
            ];
            let mut glow = sprite_at(BOMB_GLOW, o.pos, 0.85);
            glow.tint = [col[0], col[1], col[2], 0.85];
            glow.rot = self.anim as f32 * 0.2 + o.hue * 6.28;
            cmds.push(glow);
        }

        // Death / pickup puffs (under bullets, additive-looking glow).
        for p in &self.particles {
            let a = (p.life / p.max_life).clamp(0.0, 1.0);
            let s = p.size * (1.5 - a); // expand as it fades
            cmds.push(DrawCmd {
                tex: TEX_WHITE,
                dst: [FIELD_X + p.pos[0] - s / 2.0, FIELD_Y + p.pos[1] - s / 2.0, s, s],
                src: [0.25, 0.25, 0.75, 0.75],
                tint: [p.color[0], p.color[1], p.color[2], a * 0.8],
                rot: 0.0,
            });
        }

        // Bullets from the original scripts.
        let [tw, th] = self.bullet_tex_size;
        for b in &self.world.bullets {
            let Some(sp) = self.bullet_sprites.get(&b.sprite) else { continue };
            cmds.push(DrawCmd {
                tex: TEX_BULLET,
                dst: [
                    FIELD_X + b.pos[0] - sp.width / 2.0,
                    FIELD_Y + b.pos[1] - sp.height / 2.0,
                    sp.width,
                    sp.height,
                ],
                src: [sp.x / tw, sp.y / th, (sp.x + sp.width) / tw, (sp.y + sp.height) / th],
                tint: [1.0, 1.0, 1.0, 1.0],
                rot: 0.0,
            });
        }

        // Lasers: a rotated quad over the lit segment, tinted by color.
        for l in &self.world.lasers {
            if !l.in_use {
                continue;
            }
            let len = (l.end_offset - l.start_offset).max(0.0);
            if len <= 0.0 {
                continue;
            }
            let width = match l.state {
                0 => 1.2 + (l.width - 1.2) * (l.timer as f32 / l.start_time.max(1) as f32),
                1 => l.width,
                _ => l.width * (1.0 - l.timer as f32 / l.despawn_duration.max(1) as f32),
            }
            .max(0.0);
            let mid = l.start_offset + len / 2.0;
            let cx = FIELD_X + l.pos[0] + l.angle.cos() * mid;
            let cy = FIELD_Y + l.pos[1] + l.angle.sin() * mid;
            let tint = LASER_COLORS[(l.color as usize) % LASER_COLORS.len()];
            cmds.push(DrawCmd {
                tex: TEX_WHITE,
                dst: [cx - len / 2.0, cy - width / 2.0, len, width],
                src: [0.25, 0.25, 0.75, 0.75],
                tint,
                rot: l.angle,
            });
            // Bright core.
            cmds.push(DrawCmd {
                tex: TEX_WHITE,
                dst: [cx - len / 2.0, cy - width / 6.0, len, width / 3.0],
                src: [0.25, 0.25, 0.75, 0.75],
                tint: [1.0, 1.0, 1.0, 0.9],
                rot: l.angle,
            });
        }

        // Boss attack timer (top center of the field).
        if let Some(secs) = self
            .enemies
            .iter()
            .find(|e| e.is_boss && e.occupied)
            .and_then(|e| e.spell_seconds_left())
        {
            draw_text(
                &mut cmds,
                [FIELD_X + FIELD_W / 2.0 - 16.0, FIELD_Y + 12.0],
                16.0,
                [1.0, 1.0, 1.0, 0.9],
                &format!("{secs:02}"),
            );
        }

        // Spellcard name banner (right-aligned near the top of the field).
        if self.spell_active && !self.spell_name.is_empty() {
            let w = self.spell_name.len() as f32 * 14.0 * 0.75;
            draw_text(
                &mut cmds,
                [FIELD_X + FIELD_W - w - 8.0, FIELD_Y + 30.0],
                14.0,
                [1.0, 0.85, 0.9, 0.95],
                &self.spell_name,
            );
            // "Spell Card" marker to the left.
            draw_text(
                &mut cmds,
                [FIELD_X + 8.0, FIELD_Y + 30.0],
                12.0,
                [0.8, 0.8, 1.0, 0.8],
                "Spell Card",
            );
        }

        // Capture / failure result flash.
        if self.spell_result > 0 {
            let (text, tint) = if self.spell_captured {
                ("Spell Card Captured!", [0.6, 1.0, 0.6, 1.0])
            } else {
                ("Spell Card Bonus Failed", [1.0, 0.6, 0.6, 1.0])
            };
            let w = text.len() as f32 * 14.0 * 0.75;
            draw_text(
                &mut cmds,
                [FIELD_X + FIELD_W / 2.0 - w / 2.0, FIELD_Y + FIELD_H / 2.0],
                14.0,
                tint,
                text,
            );
        }

        // Dialogue box.
        if self.dialogue.active && (!self.dialogue.lines[0].is_empty() || !self.dialogue.lines[1].is_empty()) {
            // Portraits: player (Reimu) lower-left, boss (Rumia) lower-right,
            // the active speaker at full brightness. Each face sheet holds
            // two 128x256 expression columns.
            let pw = 96.0;
            let ph = 192.0;
            let py = FIELD_Y + FIELD_H - 80.0 - ph;
            for (idx, tex) in [(0usize, TEX_FACE_PLAYER), (1usize, TEX_FACE_BOSS)] {
                if !self.dialogue.portrait_shown[idx] {
                    continue;
                }
                let lit = self.dialogue.portrait_active == idx as i32;
                let c = if lit { 1.0 } else { 0.45 };
                let expr = self.dialogue.portrait_expr[idx] as f32;
                let u0 = expr * 128.0 / 256.0;
                let (px, flip_u0, flip_u1) = if idx == 0 {
                    (FIELD_X + 4.0, u0, u0 + 128.0 / 256.0)
                } else {
                    // Mirror the boss portrait to face left.
                    (FIELD_X + FIELD_W - pw - 4.0, u0 + 128.0 / 256.0, u0)
                };
                cmds.push(DrawCmd {
                    tex,
                    dst: [px, py, pw, ph],
                    src: [flip_u0, 0.0, flip_u1, 1.0],
                    tint: [c, c, c, 1.0],
                    rot: 0.0,
                });
            }

            let box_y = FIELD_Y + FIELD_H - 80.0;
            cmds.push(rect([FIELD_X + 8.0, box_y, FIELD_W - 16.0, 64.0], [0.0, 0.0, 0.0, 0.65]));
            for (li, text) in self.dialogue.lines.iter().enumerate() {
                if text.is_empty() {
                    continue;
                }
                let tint = if self.dialogue.line_colors[li] == 0 {
                    [1.0, 1.0, 1.0, 1.0] // player
                } else {
                    [1.0, 0.55, 0.55, 1.0] // boss
                };
                draw_text(
                    &mut cmds,
                    [FIELD_X + 20.0, box_y + 10.0 + li as f32 * 24.0],
                    14.0,
                    tint,
                    text,
                );
            }
        }

        self.draw_hud(&mut cmds);
        cmds
    }

    fn draw_hud(&self, cmds: &mut Vec<DrawCmd>) {
        let border = [0.12, 0.05, 0.08, 1.0];
        cmds.push(rect([0.0, 0.0, 640.0, FIELD_Y], border));
        cmds.push(rect([0.0, FIELD_Y + FIELD_H, 640.0, 480.0 - FIELD_Y - FIELD_H], border));
        cmds.push(rect([0.0, 0.0, FIELD_X, 480.0], border));
        cmds.push(rect([FIELD_X + FIELD_W, 0.0, 640.0 - FIELD_X - FIELD_W, 480.0], border));

        let sx = FIELD_X + FIELD_W + 24.0;
        cmds.push(hud_sprite(HUD_PLAYER_LABEL, [sx, 120.0]));
        for i in 0..self.lives.max(0) {
            cmds.push(hud_sprite(HUD_STAR_RED, [sx + 40.0 + i as f32 * 18.0, 120.0]));
        }
        cmds.push(hud_sprite(HUD_BOMB_LABEL, [sx, 144.0]));
        for i in 0..self.bombs.max(0) {
            cmds.push(hud_sprite(HUD_STAR_GREEN, [sx + 40.0 + i as f32 * 18.0, 144.0]));
        }
        // Score and power readouts.
        draw_text(cmds, [sx, 68.0], 16.0, [1.0, 0.9, 0.5, 1.0], &format!("Hi {}", self.hiscore.max(self.score)));
        draw_text(cmds, [sx, 84.0], 16.0, [1.0, 1.0, 1.0, 1.0], &format!("Score {}", self.score));
        draw_text(cmds, [sx, 172.0], 16.0, [1.0, 0.8, 0.4, 1.0], &format!("Power {:3}", self.world.power));
        draw_text(cmds, [sx, 190.0], 16.0, [0.8, 0.9, 1.0, 1.0], &format!("Graze {}", self.graze));

        let mut logo = hud_sprite(HUD_LOGO, [sx - 4.0, 300.0]);
        logo.tint = [1.0, 1.0, 1.0, 0.85];
        cmds.push(logo);

        match self.state {
            PlayerState::GameOver(_) => {
                cmds.push(rect([FIELD_X, FIELD_Y, FIELD_W, FIELD_H], [0.0, 0.0, 0.0, 0.6]));
            }
            PlayerState::Cleared(_) => {
                cmds.push(rect([FIELD_X, FIELD_Y, FIELD_W, FIELD_H], [0.0, 0.0, 0.08, 0.72]));
                let cx = FIELD_X + 40.0;
                let mut y = FIELD_Y + 90.0;
                draw_text(cmds, [FIELD_X + FIELD_W / 2.0 - 80.0, FIELD_Y + 50.0], 22.0, [1.0, 1.0, 0.5, 1.0], "STAGE 1 CLEAR");
                let rows = [
                    ("Graze".to_string(), self.graze.to_string()),
                    ("Spell".to_string(), if self.spell_captured { "Captured".into() } else { "-".into() }),
                    ("Clear bonus".to_string(), self.clear_bonus.to_string()),
                    ("Score".to_string(), self.score.to_string()),
                    ("Hi-Score".to_string(), self.hiscore.to_string()),
                ];
                for (label, val) in rows {
                    draw_text(cmds, [cx, y], 16.0, [0.8, 0.85, 1.0, 1.0], &label);
                    draw_text(cmds, [cx + 130.0, y], 16.0, [1.0, 1.0, 1.0, 1.0], &val);
                    y += 30.0;
                }
            }
            _ => {}
        }

        if self.paused {
            cmds.push(rect([0.0, 0.0, 640.0, 480.0], [0.0, 0.0, 0.0, 0.62]));
            draw_text(cmds, [FIELD_X + FIELD_W / 2.0 - 36.0, 150.0], 24.0, [1.0, 1.0, 1.0, 1.0], "PAUSE");
            for (i, opt) in Self::PAUSE_OPTIONS.iter().enumerate() {
                let sel = i == self.pause_cursor;
                let tint = if sel { [1.0, 1.0, 0.4, 1.0] } else { [0.65, 0.65, 0.65, 1.0] };
                let label = if sel { format!("> {opt}") } else { opt.to_string() };
                draw_text(cmds, [FIELD_X + 40.0, 220.0 + i as f32 * 34.0], 18.0, tint, &label);
            }
        }
    }
}

/// th06 bullet color palette, approximated (index = color offset).
const LASER_COLORS: [[f32; 4]; 16] = [
    [0.4, 0.4, 0.4, 0.8],
    [0.9, 0.2, 0.2, 0.8],
    [1.0, 0.4, 0.6, 0.8],
    [0.7, 0.3, 0.9, 0.8],
    [0.5, 0.3, 1.0, 0.8],
    [0.3, 0.3, 1.0, 0.8],
    [0.3, 0.6, 1.0, 0.8],
    [0.3, 0.9, 1.0, 0.8],
    [0.3, 1.0, 0.8, 0.8],
    [0.3, 1.0, 0.4, 0.8],
    [0.6, 1.0, 0.3, 0.8],
    [0.8, 1.0, 0.3, 0.8],
    [1.0, 1.0, 0.3, 0.8],
    [1.0, 0.8, 0.3, 0.8],
    [1.0, 0.6, 0.3, 0.8],
    [1.0, 1.0, 1.0, 0.8],
];

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
            });
        }
        x += size * 0.75;
    }
}

fn rect(dst: [f32; 4], tint: [f32; 4]) -> DrawCmd {
    DrawCmd { tex: TEX_WHITE, dst, src: [0.25, 0.25, 0.75, 0.75], tint, rot: 0.0 }
}

fn sprite_at(s: SpriteRef, field_pos: [f32; 2], alpha: f32) -> DrawCmd {
    let [x, y, w, h] = s.rect;
    DrawCmd {
        tex: s.tex,
        dst: [FIELD_X + field_pos[0] - w / 2.0, FIELD_Y + field_pos[1] - h / 2.0, w, h],
        src: [x / 256.0, y / 256.0, (x + w) / 256.0, (y + h) / 256.0],
        tint: [1.0, 1.0, 1.0, alpha],
        rot: 0.0,
    }
}

fn hud_sprite(s: SpriteRef, screen_pos: [f32; 2]) -> DrawCmd {
    let [x, y, w, h] = s.rect;
    DrawCmd {
        tex: s.tex,
        dst: [screen_pos[0], screen_pos[1], w, h],
        src: [x / 256.0, y / 256.0, (x + w) / 256.0, (y + h) / 256.0],
        tint: [1.0, 1.0, 1.0, 1.0],
        rot: 0.0,
    }
}
