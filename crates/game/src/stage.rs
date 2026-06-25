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
use crate::hud::Hud;
use crate::background::Background;
use th06_engine::BgScene;
use crate::ecl_vm::{Bullet, Enemy, Rng, SpawnReq, World, WorldEvent, BULLET_BASE_SPRITE};

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
pub const TEX_FACE_REIMU: usize = 9; // face00a (Reimu player portrait)
pub const TEX_FACE_MARISA: usize = 10; // face01a (Marisa player portrait)
// 11 stg1bg, 12 player01.
pub const TEX_POWER_GRAD: usize = 13; // 2x1 power-bar gradient

/// Sound effects by SoundIdx (SoundPlayer.cpp g_SFXList order). Indexing this
/// with the raw idx the ECL/bullet code passes gives the right cue — the
/// previous ad-hoc map had several wrong (e.g. idx 5 played power1, not tan00).
pub const SFX_BY_IDX: [&str; 26] = [
    "plst00", "enep00", "pldead00", "power0", "power1", "tan00", "tan01", "tan02", "ok00",
    "cancel00", "select00", "gun00", "cat00", "lazer00", "lazer01", "enep01", "nep00", "damage00",
    "item00", "kira00", "kira01", "kira02", "extend", "timeout", "graze", "powerup",
];

pub enum Event {
    Sfx(&'static str),
    Bgm(&'static str),
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
}

#[derive(Clone, Copy)]
struct SpriteRef {
    tex: usize,
    rect: [f32; 4],
}

const fn spr(tex: usize, x: f32, y: f32, w: f32, h: f32) -> SpriteRef {
    SpriteRef { tex, rect: [x, y, w, h] }
}

/// Fallback shot sprite (player00 amulet) when a script can't be resolved.
const AMULET: SpriteRef = spr(TEX_PLAYER, 129.0, 1.0, 14.0, 14.0);
const BOMB_GLOW: SpriteRef = spr(TEX_PLAYER, 1.0, 97.0, 62.0, 62.0);
/// player00 sprite 66: the focus hitbox marker.
const HITBOX_MARKER: SpriteRef = spr(TEX_PLAYER, 160.0, 0.0, 16.0, 16.0);

struct Shot {
    pos: [f32; 2],
    vel: [f32; 2],
    damage: i32,
    /// BULLET_TYPE_*: 0 straight, 1 homing orb (Reimu A), 2 gravity orb-missile
    /// (Marisa A), 3 laser (handled separately).
    bt: u8,
    /// Player-anm script id (BulletData anmScriptIdx). Resolved against the
    /// current character's sheet at draw, so each character shows their own
    /// shot sprite from the same id.
    anm_script: i32,
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

/// MarisaA shot tiers: a fast front shot (motion.y -8) plus type-2 orb
/// "missiles" (low speed, gravity-accelerated) fired from the orbs.
const MARISA_A_RANKS: [&[ShotDef]; 9] = [
    &[sb(5, 0, 0.0, -8.0, -90.0, 12.0, 48, 0, 0)],
    &[
        sb(5, 0, 0.0, -8.0, -90.0, 12.0, 36, 0, 0),
        sb(30, 0, 0.0, 0.0, -90.0, 3.0, 18, 1, 2),
        sb(30, 0, 0.0, 0.0, -90.0, 3.0, 18, 2, 2),
    ],
    &[
        sb(5, 0, 0.0, -8.0, -90.0, 12.0, 32, 0, 0),
        sb(30, 0, 0.0, 0.0, -95.0, 3.0, 16, 1, 2),
        sb(30, 0, 0.0, 0.0, -85.0, 3.0, 16, 2, 2),
        sb(30, 15, 0.0, 0.0, -85.0, 3.0, 10, 1, 2),
        sb(30, 15, 0.0, 0.0, -95.0, 3.0, 10, 2, 2),
    ],
    &[
        sb(5, 0, 0.0, -8.0, -90.0, 12.0, 32, 0, 0),
        sb(15, 0, 0.0, 0.0, -95.0, 3.0, 15, 1, 2),
        sb(15, 0, 0.0, 0.0, -85.0, 3.0, 15, 2, 2),
        sb(15, 15, 0.0, 0.0, -85.0, 3.0, 10, 1, 2),
        sb(15, 15, 0.0, 0.0, -95.0, 3.0, 10, 2, 2),
    ],
    &[
        sb(5, 0, 0.0, -8.0, -90.0, 12.0, 32, 0, 0),
        sb(15, 0, 0.0, 0.0, -95.0, 3.0, 16, 1, 2),
        sb(15, 0, 0.0, 0.0, -85.0, 3.0, 16, 2, 2),
        sb(15, 20, 0.0, 0.0, -85.0, 3.0, 11, 1, 2),
        sb(15, 20, 0.0, 0.0, -95.0, 3.0, 11, 2, 2),
    ],
    &[
        sb(5, 0, -8.0, -8.0, -90.0, 12.0, 16, 0, 0),
        sb(5, 0, 8.0, -8.0, -90.0, 12.0, 16, 0, 0),
        sb(10, 0, 0.0, 0.0, -95.0, 3.0, 16, 1, 2),
        sb(10, 0, 0.0, 0.0, -85.0, 3.0, 16, 2, 2),
        sb(15, 5, 0.0, 0.0, -85.0, 3.0, 10, 1, 2),
        sb(15, 5, 0.0, 0.0, -95.0, 3.0, 10, 2, 2),
    ],
    &[
        sb(5, 0, -8.0, -8.0, -90.0, 12.0, 13, 0, 0),
        sb(5, 0, 8.0, -8.0, -90.0, 12.0, 13, 0, 0),
        sb(10, 0, 0.0, 0.0, -98.0, 3.0, 16, 1, 2),
        sb(10, 0, 0.0, 0.0, -82.0, 3.0, 16, 2, 2),
        sb(10, 5, 0.0, 0.0, -82.0, 3.0, 10, 1, 2),
        sb(10, 5, 0.0, 0.0, -98.0, 3.0, 10, 2, 2),
    ],
    &[
        sb(5, 0, 0.0, 0.0, -94.0, 12.0, 8, 0, 0),
        sb(5, 0, 0.0, 0.0, -90.0, 12.0, 12, 0, 0),
        sb(5, 0, 0.0, 0.0, -86.0, 12.0, 8, 0, 0),
        sb(10, 0, 0.0, 0.0, -98.0, 3.0, 15, 1, 2),
        sb(10, 0, 0.0, 0.0, -82.0, 3.0, 15, 2, 2),
        sb(10, 5, 0.0, 0.0, -82.0, 3.0, 10, 1, 2),
        sb(10, 5, 0.0, 0.0, -98.0, 3.0, 10, 2, 2),
        sb(15, 0, 0.0, 0.0, -78.0, 3.0, 9, 1, 2),
        sb(15, 0, 0.0, 0.0, -102.0, 3.0, 9, 2, 2),
    ],
    &[
        sb(5, 0, 0.0, 0.0, -94.0, 12.0, 8, 0, 0),
        sb(5, 0, 0.0, 0.0, -90.0, 12.0, 12, 0, 0),
        sb(5, 0, 0.0, 0.0, -86.0, 12.0, 8, 0, 0),
        sb(10, 0, 0.0, 0.0, -98.0, 3.0, 14, 1, 2),
        sb(10, 0, 0.0, 0.0, -82.0, 3.0, 14, 2, 2),
        sb(10, 5, 0.0, 0.0, -82.0, 3.0, 10, 1, 2),
        sb(10, 5, 0.0, 0.0, -98.0, 3.0, 10, 2, 2),
        sb(10, 0, 0.0, 0.0, -75.0, 3.0, 10, 1, 2),
        sb(10, 0, 0.0, 0.0, -105.0, 3.0, 10, 2, 2),
    ],
];

/// MarisaB front shots (the orb LASERS are handled separately as vertical
/// beams; see `marisa_beam_dmg`).
const MARISA_B_RANKS: [&[ShotDef]; 9] = [
    &[sb(5, 0, 0.0, -8.0, -90.0, 12.0, 48, 0, 0)],
    &[sb(5, 0, 0.0, -8.0, -90.0, 12.0, 32, 0, 0)],
    &[sb(5, 0, 0.0, -8.0, -90.0, 12.0, 32, 0, 0)],
    &[
        sb(5, 0, -8.0, -8.0, -92.0, 12.0, 22, 0, 0),
        sb(5, 0, 8.0, -8.0, -88.0, 12.0, 22, 0, 0),
    ],
    &[
        sb(5, 0, -8.0, -8.0, -92.0, 12.0, 22, 0, 0),
        sb(5, 0, 8.0, -8.0, -88.0, 12.0, 22, 0, 0),
    ],
    &[
        sb(5, 0, -8.0, -8.0, -92.0, 12.0, 20, 0, 0),
        sb(5, 0, 8.0, -8.0, -88.0, 12.0, 20, 0, 0),
    ],
    &[
        sb(5, 0, 0.0, 0.0, -95.0, 12.0, 15, 0, 0),
        sb(5, 0, 0.0, 0.0, -90.0, 12.0, 20, 0, 0),
        sb(5, 0, 0.0, 0.0, -85.0, 12.0, 15, 0, 0),
    ],
    &[
        sb(5, 0, 0.0, 0.0, -95.0, 12.0, 15, 0, 0),
        sb(5, 0, 0.0, 0.0, -90.0, 12.0, 20, 0, 0),
        sb(5, 0, 0.0, 0.0, -85.0, 12.0, 15, 0, 0),
    ],
    &[
        sb(5, 0, 0.0, 0.0, -100.0, 12.0, 12, 0, 0),
        sb(5, 0, 0.0, 0.0, -95.0, 12.0, 15, 0, 0),
        sb(5, 0, 0.0, 0.0, -90.0, 12.0, 20, 0, 0),
        sb(5, 0, 0.0, 0.0, -85.0, 12.0, 15, 0, 0),
        sb(5, 0, 0.0, 0.0, -80.0, 12.0, 12, 0, 0),
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
            Character::ReimuA => &REIMU_A_RANKS,
            Character::ReimuB => &REIMU_B_RANKS,
            Character::MarisaA => &MARISA_A_RANKS,
            Character::MarisaB => &MARISA_B_RANKS,
        }
    }

    /// Sprite look for a shot of bullet type `bt`: 0 amulet, 1 needle, 2 missile.
    /// Player-anm script id for a shot (BulletData anmScriptIdx). Straight
    /// shots are ANM_SCRIPT_PLAYER_BULLET (64) for everyone; orb/special shots
    /// differ per character. Resolved against each character's own sheet, so
    /// the same id yields Reimu's amulet vs Marisa's star automatically.
    fn shot_anm_script(self, bt: u8) -> i32 {
        match (self, bt) {
            (_, 0) => 64,             // ANM_SCRIPT_PLAYER_BULLET (straight)
            (Character::ReimuA, _) => 65, // REIMU_A_ORB_BULLET
            (Character::ReimuB, _) => 66, // REIMU_B_ORB_BULLET
            (Character::MarisaA, _) => 65, // MARISA_A_ORB_BULLET_1
            (Character::MarisaB, _) => 69, // MARISA_B_ORB_LASER_1
        }
    }

    /// Per-frame damage of each MarisaB orb laser at the given power (0 = no
    /// beams). Beams appear from rank 2 (power >= 8).
    fn marisa_beam_dmg(self, power: i32) -> i32 {
        if self != Character::MarisaB || power < 8 {
            return 0;
        }
        match power_rank(power) {
            0..=4 => 3,
            5..=6 => 4,
            7 => 5,
            _ => 6,
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

/// Falling collectible (ItemManager port). `kind` matches ItemType:
/// 0 power small, 1 point, 2 power big, 3 bomb, 4 full power, 5 life,
/// 6 point-bullet (from cancelled bullets). `state` is the ItemManager state:
/// 0 = normal fall, 1 = homing to player (latched, never reverts), 2 = the
/// 60-frame scatter arc used by death/boss-kill drops, then it falls.
struct Item {
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
        Item { pos, vy: -2.2, kind, state: 0, timer: 0, start: [0.0; 2], target: [0.0; 2] }
    }
    /// Cancelled-bullet point item: homes to the player immediately (state 1).
    fn homing(pos: [f32; 2], kind: i32) -> Self {
        Item { pos, vy: 0.0, kind, state: 1, timer: 0, start: [0.0; 2], target: [0.0; 2] }
    }
    /// Death / boss-kill scatter (state 2): arcs to a random target over 60
    /// frames (SpawnItem state==2: x in 48..336, y in -64..128) then falls.
    fn scatter(pos: [f32; 2], kind: i32, rng: &mut Rng) -> Self {
        let target = [rng.f32_in_range(288.0) + 48.0, rng.f32_in_range(192.0) - 64.0];
        Item { pos, vy: 0.0, kind, state: 2, timer: 0, start: pos, target }
    }
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
        2 => "Darkness Sign \"Demarcation\"",
        _ => "Spell Card",
    }
}

/// Per-spellcard capture score, indexed by spellcard id (g_SpellcardScore,
/// EclManager.cpp:18). The capture bonus is `score * (1 + secondsLeft/10)`.
const SPELLCARD_SCORE: [i64; 64] = [
    200000, 200000, 200000, 200000, 200000, 200000, 200000, 250000, 250000, 250000, 250000, 250000, 250000,
    250000, 300000, 300000, 300000, 300000, 300000, 300000, 300000, 300000, 300000, 300000, 300000, 300000,
    300000, 300000, 300000, 300000, 300000, 300000, 400000, 400000, 400000, 400000, 400000, 400000, 400000,
    400000, 500000, 500000, 500000, 500000, 500000, 500000, 600000, 600000, 600000, 600000, 600000, 700000,
    700000, 700000, 700000, 700000, 700000, 700000, 700000, 700000, 700000, 700000, 700000, 700000,
];

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
    /// Per-frame damage of each MarisaB orb beam this frame (0 = no beams).
    beam_dmg: i32,
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
    /// Point items collected this stage (Gui shows it as "Point"). Not carried.
    point_items: i64,
    /// Focus held last update, for drawing the orbs in their focused position.
    last_input_focus: bool,
    /// Position of the last enemy a player shot hit, for orb-amulet homing.
    last_enemy_hit: Option<[f32; 2]>,
    state: PlayerState,
    shots: Vec<Shot>,
    bomb_orbs: Vec<BombOrb>,
    items: Vec<Item>,
    particles: Vec<Particle>,
    /// Cosmetic-only RNG for visual particle bursts (`spawn_burst`), kept
    /// separate from the gameplay `world.rng` so effects never desync it (#26).
    fx_rng: Rng,
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
    pause_cursor: usize,
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
    /// Boss spellcard portrait (face*.anm script 3) + which boss-face sprite.
    portrait_script: Vec<AnmInstr>,
    portrait_runner: Option<AnmRunner>,
    spell_portrait_sprite: i32,
    spell_bonus_timer: u32,
    spell_bonus_amount: i64,
    /// "BONUS" popup (bullet-cancel total) — frames remaining, amount.
    bonus_score_timer: u32,
    bonus_score_amount: i64,
    boss_bgm_started: bool,
    /// Dialogue portrait texture slots (player left / boss right).
    face_player_tex: usize,
    face_boss_tex: usize,
    /// Music tracks for this stage (field theme / boss theme).
    stage_bgm: &'static str,
    boss_bgm: &'static str,
    /// 1-based stage number + the cumulative graze at stage start, for the
    /// clear bonus (Gui currentStage / grazeInStage).
    stage_num: i32,
    graze_start: i64,
    msg: Msg,
    dialogue: Dialogue,
    background: Option<Background>,
    hud: Hud,
    pub events: Vec<Event>,
}

impl Stage {
    pub fn new(ecl: Ecl, enemy_scripts: HashMap<i32, ScriptRef>, etama: &Entry, eff: &Entry, eff_tex: usize, player: &Entry, player_tex: usize, character: Character, msg: Msg, background: Option<Background>, hud: Hud, cfg: StageConfig, spell_name_script: Vec<AnmInstr>, portrait_script: Vec<AnmInstr>) -> Self {
        let timeline_off = ecl.timeline_offset;
        // etama4 (effect) ANM: per-script instructions for the runners, and the
        // shared sprite table + texture for rendering particles.
        let eff_scripts: HashMap<u32, Vec<AnmInstr>> =
            eff.scripts.iter().map(|(id, instrs)| (*id, instrs.clone())).collect();
        let eff_sprites: HashMap<u32, Sprite> =
            eff.sprites.iter().map(|s| (s.index, s.clone())).collect();
        let eff_tex_size = [eff.width as f32, eff.height as f32];
        let player_scripts: HashMap<i32, Vec<AnmInstr>> =
            player.scripts.iter().map(|(id, instrs)| (*id as i32, instrs.clone())).collect();
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
            if let Some(sp) = etama.sprites.iter().find(|s| s.index == base) {
                bullet_heights[t] = sp.height;
                bullet_widths[t] = sp.width;
            }
        }
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
                character: character.is_marisa() as u8,
                shot_type: matches!(character, Character::ReimuB | Character::MarisaB) as u8,
                time_stopped: false,
                bullet_heights,
                bullet_widths,
                effects: Vec::new(),
                eff_scripts,
                random_item_spawn_index: 0,
                random_item_table_index: 0,
            },
            enemies: Vec::new(),
            anims: Vec::new(),
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
            beam_dmg: 0,
            bomb_kind: 0,
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
            point_items: 0,
            last_input_focus: false,
            last_enemy_hit: None,
            state: PlayerState::Alive,
            shots: Vec::new(),
            bomb_orbs: Vec::new(),
            items: Vec::new(),
            particles: Vec::new(),
            fx_rng: Rng::new(0x4321),
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
            pause_cursor: 0,
            rand_item_table: 0,
            rand_item_spawn: 0,
            spell_active: false,
            spell_name: String::new(),
            spell_id: 0,
            spell_secs: 0,
            spell_name_script,
            spell_name_runner: None,
            portrait_script,
            portrait_runner: None,
            spell_portrait_sprite: 0,
            spell_bonus_timer: 0,
            spell_bonus_amount: 0,
            bonus_score_timer: 0,
            bonus_score_amount: 0,
            spell_capturing: false,
            spell_result: 0,
            spell_captured: false,
            boss_bgm_started: false,
            face_player_tex: cfg.face_player_tex,
            face_boss_tex: cfg.face_boss_tex,
            stage_bgm: cfg.stage_bgm,
            boss_bgm: cfg.boss_bgm,
            stage_num: cfg.stage_num,
            graze_start: 0,
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
        }
    }

    /// Restore progress carried from the previous stage.
    pub fn apply_carry(&mut self, c: Carry) {
        self.lives = c.lives;
        self.bombs = c.bombs;
        self.world.power = c.power;
        self.score = c.score;
        self.graze = c.graze;
        self.power_item_count = c.power_item_count;
        // grazeInStage counts from the stage start (Gui clear bonus).
        self.graze_start = c.graze;
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

    /// Every live bullet as [pos.x, pos.y, angle, speed] — for the full-VM
    /// oracle diff against the decomp.
    pub fn bullets_dump(&self) -> Vec<[f32; 4]> {
        self.world.bullets.iter().map(|b| [b.pos[0], b.pos[1], b.angle, b.speed]).collect()
    }

    /// Occupied enemy positions [x, y] — oracle diff (isolates movement).
    pub fn enemies_dump(&self) -> Vec<[f32; 2]> {
        self.enemies.iter().filter(|e| e.occupied).map(|e| [e.pos[0], e.pos[1]]).collect()
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
        self.hud.tick();
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
        if let Some(r) = &mut self.spell_name_runner {
            r.tick();
            if !r.visible() {
                self.spell_name_runner = None;
            }
        }
        if let Some(r) = &mut self.portrait_runner {
            r.tick();
            if !r.visible() {
                self.portrait_runner = None;
            }
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
            PlayerState::GameOver(t) => {
                *t -= 1;
                if *t == 0 {
                    self.events.push(Event::GameOver(self.score));
                    return self.draw();
                }
            }
            PlayerState::Cleared(t) => {
                *t -= 1;
                if *t == 0 {
                    self.events.push(Event::NextStage);
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
        // EffectManager::OnUpdate (chain prio 10, between enemy 9 and bullet 11):
        // draw the RNG for effects spawned this frame (enemy deaths, etc.). (#26)
        self.world.update_effects();
        self.update_shots();
        self.update_bullets();
        self.update_items();
        self.update_particles();
        self.collide();
        self.drain_world_events();

        // Stage clear: timeline exhausted and field empty.
        if matches!(self.state, PlayerState::Alive)
            && self.timeline_done()
            && !self.enemies.iter().any(|e| e.occupied)
            && !self.world.boss_present
        {
            let bonus = stage_clear_bonus(
                self.stage_num,
                (self.graze - self.graze_start).max(0),
                self.world.power as i64,
                self.point_items,
                self.lives.max(0) as i64,
                self.bombs.max(0) as i64,
                self.world.difficulty,
            );
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
            self.insert_enemy(e);
        }
        self.flush_spawns();
    }

    /// EnemyManager::SpawnEnemy (EnemyManager.cpp:92): place the enemy in the
    /// first free slot (lowest index with `!occupied`), reusing slots freed by
    /// dead enemies, exactly like the decomp's fixed `enemies[]` pool. Slots are
    /// never compacted/reordered, so the processing (and bullet-firing) order
    /// stays decomp-faithful. (#30)
    fn insert_enemy(&mut self, e: Enemy) {
        if let Some(slot) = self.enemies.iter().position(|x| !x.occupied) {
            self.enemies[slot] = e;
            self.anims[slot] = None;
        } else {
            self.enemies.push(e);
            self.anims.push(None);
        }
    }

    fn flush_spawns(&mut self) {
        // FIFO: the decomp spawns sequentially in creation order, each taking the
        // next free slot. Draining front-to-back preserves that order.
        while !self.world.pending_spawns.is_empty() {
            let req = self.world.pending_spawns.remove(0);
            if let Some(e) = Enemy::spawn(&self.ecl, &mut self.world, &req) {
                self.insert_enemy(e);
            }
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
                    .and_then(|idx| self.enemy_scripts.get(&anm_script).and_then(|s| s.sprites.get(&idx)))
                    .map(|sp| (sp.width, sp.height))
                    .unwrap_or((0.0, 0.0))
            };
            if !self.enemies[i].update_bounds(sw, sh) {
                self.enemies[i].despawn(&mut self.world);
                i += 1;
                continue;
            }
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
            // Boss timer ticks once per frame per enemy, unless time is stopped
            // (EnemyManager.cpp:735).
            if !self.world.time_stopped {
                self.enemies[i].tick_boss_timer();
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
            if let Some(anim) = &mut self.anims[i] {
                anim.tick();
            }
            i += 1;
        }
        self.flush_spawns();

        // Death-effect particles (EnemyManager.cpp:641-706): a dead interactable
        // enemy queues its splash/bubble effects here, in EnemyManager::OnUpdate
        // (before the bullet update), so their RNG draws land at the faithful
        // point. The drop/score/removal stay in collide(); this only spawns the
        // effects (and fires once via death_fx_done). (#26)
        for e in &mut self.enemies {
            if e.occupied && e.interactable && e.life <= 0 && !e.death_fx_done {
                e.spawn_death_effects(&mut self.world);
            }
        }
        // No compaction: dead slots (occupied=false) stay in place and are reused
        // by insert_enemy, keeping the slot pool decomp-faithful. (#30)
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

        // MarisaB orb beams: two vertical lasers while shooting at power >= 8.
        self.beam_dmg = if can_act && input.held(Key::Shoot) {
            self.character.marisa_beam_dmg(self.world.power)
        } else {
            0
        };

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

    /// Bomb, one per shot type (BombData.cpp). 360-frame invulnerability.
    fn fire_bomb(&mut self) {
        self.dying = 0; // a bomb in the deathbomb window cancels the death
        self.bombs -= 1;
        self.spell_capturing = false; // bombing forfeits the capture
        self.world.bullets.clear();
        self.cancel_lasers();
        self.bomb_orbs.clear();
        self.bomb_kind = match self.character {
            Character::ReimuA => 0,
            Character::ReimuB => 1,
            Character::MarisaA => 2,
            Character::MarisaB => 3,
        };
        // Per-bomb invulnerability (BombData SetCurrent): ReimuB 200, MarisaA
        // 300, the others 360.
        let invuln = match self.bomb_kind {
            1 => 200,
            2 => 300,
            _ => 360,
        };
        self.invuln = self.invuln.max(invuln);
        match self.bomb_kind {
            0 => {
                // Fantasy Seal: 8 homing orbs.
                self.bombing = 300;
                for i in 0..8 {
                    let a = i as f32 / 8.0 * std::f32::consts::TAU + 0.39;
                    self.bomb_orbs.push(BombOrb { pos: self.pos, vel: [a.cos() * 4.0, a.sin() * 4.0], spd: 4.0, hue: i as f32 / 8.0 });
                }
            }
            1 => self.bombing = 140, // Dream cross: beams, no orbs
            2 => {
                // Stardust: a ring of stars drifting outward (no homing).
                self.bombing = 250;
                for i in 0..16 {
                    let a = i as f32 / 16.0 * std::f32::consts::TAU;
                    self.bomb_orbs.push(BombOrb { pos: self.pos, vel: [a.cos() * 3.0, a.sin() * 3.0], spd: 3.0, hue: i as f32 / 16.0 });
                }
            }
            _ => self.bombing = 300, // Master Spark
        }
        self.spawn_burst(self.pos, 24, 6.0, [0.6, 0.8, 1.0], 16.0);
        self.events.push(Event::Sfx("power1"));
    }

    /// Per-frame bomb update.
    fn update_bomb(&mut self) {
        self.bombing -= 1;
        self.world.bullets.clear();
        match self.bomb_kind {
            1 => {
                // Dream cross: a vertical beam at the player's x and a
                // horizontal beam at the player's y melt anything they touch.
                let (px, py) = (self.pos[0], self.pos[1]);
                for e in &mut self.enemies {
                    if e.occupied && e.interactable && e.damageable
                        && ((e.pos[0] - px).abs() < 40.0 || (e.pos[1] - py).abs() < 40.0)
                    {
                        e.life -= 12;
                    }
                }
                return;
            }
            3 => {
                // Master Spark (BombMarisaBCalc): full playfield width, from the
                // top down to the player, 12 damage per frame (skipping every
                // 4th, matching `timer % 4`).
                if self.bombing % 4 != 0 {
                    for e in &mut self.enemies {
                        if e.occupied && e.interactable && e.damageable && e.pos[1] <= self.pos[1] {
                            e.life -= 12;
                        }
                    }
                }
                return;
            }
            _ => {}
        }
        let stardust = self.bomb_kind == 2;
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
            // Fantasy Seal orbs home; Stardust stars drift straight outward.
            if let (false, Some(t)) = (stardust, pivot) {
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
        // Each orb is a damage region (Stardust's stars are larger, 64px).
        let r = if stardust { 32.0 } else { 24.0 };
        for e in &mut self.enemies {
            if !e.occupied || !e.interactable || !e.damageable {
                continue;
            }
            for o in &self.bomb_orbs {
                if (e.pos[0] - o.pos[0]).abs() < r && (e.pos[1] - o.pos[1]).abs() < r {
                    e.life -= 8;
                }
            }
        }
        if self.bombing == 0 {
            self.bomb_orbs.clear();
        }
    }

    /// Player::Die plus the respawn bookkeeping: lose a life, scatter items and
    /// drop power. With lives left: 1 big + 5 small power items scatter and power
    /// drops by 16 (Player.cpp). On the fatal death: 5 full-power items, power 0.
    fn commit_death(&mut self) {
        self.lives -= 1;
        let pos = self.pos;
        if self.lives >= 0 {
            self.items.push(Item::scatter(pos, 2, &mut self.world.rng)); // POWER_BIG
            for _ in 0..5 {
                self.items.push(Item::scatter(pos, 0, &mut self.world.rng)); // POWER_SMALL
            }
            self.world.power = (self.world.power - 16).max(0);
        } else {
            for _ in 0..5 {
                self.items.push(Item::scatter(pos, 4, &mut self.world.rng)); // FULL_POWER
            }
            self.world.power = 0;
        }
        self.power_item_count = 0;
        self.bombs = 3;
        self.spell_capturing = false; // dying forfeits the capture
        self.world.bullets.clear();
        self.world.time_stopped = false;
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
                anm_script: self.character.shot_anm_script(def.bt),
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
        // Sakuya's time-stop: bullets and lasers freeze in place (the boss keeps
        // moving and laying down new ones over the frozen field).
        if self.world.time_stopped {
            return;
        }
        let player = self.world.player_pos;
        for b in &mut self.world.bullets {
            // BULLET_STATE_DESPAWNING (BulletManager.cpp:947): a fading bullet
            // drifts at half velocity over the 12-frame donut anm, then is
            // removed (no ex-behaviour, no cull). (#14)
            if b.despawn_timer > 0 {
                b.pos[0] += b.angle.cos() * b.speed * 0.5;
                b.pos[1] += b.angle.sin() * b.speed * 0.5;
                b.despawn_timer -= 1;
                if b.despawn_timer == 0 {
                    b.oob_count = i32::MAX; // swept by the retain below
                }
                continue;
            }
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
            // Direction-change group (exFlags & 0x1c0): every `ex_int0` frames,
            // up to `ex_int1` times, re-point the bullet and reset its speed;
            // between changes the speed ramps down to a near-stop. The three
            // modes differ only in how the new angle is chosen:
            //   0x40  rotate by a fixed delta   (angle += rotation)
            //   0x100 snap to an absolute angle (angle  = rotation)
            //   0x80  re-aim at the player      (angle  = atan2(player) + rotation)
            // 0x80 is Cirno's Perfect Freeze: shards freeze, then home.
            let mut move_speed = b.speed;
            let mode = b.ex_flags & 0x1c0;
            if mode != 0 {
                let interval = b.ex_int0;
                let trigger = if interval > 0 {
                    b.timer >= interval * (b.ex_count + 1)
                } else {
                    true
                };
                if trigger {
                    b.ex_count += 1;
                    if b.ex_count >= b.ex_int1 {
                        b.ex_flags &= !mode;
                    }
                    b.angle = if mode & 0x40 != 0 {
                        b.angle + b.ex_f[0]
                    } else if mode & 0x100 != 0 {
                        b.ex_f[0]
                    } else {
                        (player[1] - b.pos[1]).atan2(player[0] - b.pos[0]) + b.ex_f[0]
                    };
                    b.speed = b.ex_f[1];
                    move_speed = b.speed;
                } else {
                    let t = b.timer as f32 - (interval * b.ex_count) as f32;
                    move_speed = b.speed - t * b.speed / interval as f32;
                }
            }
            let factor = if b.spawn_delay > 0 {
                b.spawn_delay -= 1;
                b.spawn_factor
            } else {
                1.0
            };
            b.pos[0] += b.angle.cos() * move_speed * factor;
            b.pos[1] += b.angle.sin() * move_speed * factor;
            // On the spawn->fired transition frame the decomp falls through into
            // the FIRED case and applies a second, full-velocity move
            // (BulletManager.cpp:698 then :895).
            if factor != 1.0 && b.spawn_delay == 0 {
                b.pos[0] += b.angle.cos() * move_speed;
                b.pos[1] += b.angle.sin() * move_speed;
            }
            // Off-screen cull (BulletManager.cpp:896, only for FIRED bullets):
            // GameManager::IsInBounds — the sprite (w x h, ~square) must overlap
            // [0,FIELD_W] x [0,FIELD_H]. Normal bullets die on the first OOB frame;
            // dir-change/bounce bullets (0x40/0x80/0x100/0x400/0x800) get 256.
            if b.spawn_delay == 0 {
                let (hw, hh) = (b.width / 2.0, b.height / 2.0);
                let in_bounds = b.pos[0] + hw >= 0.0
                    && b.pos[0] - hw <= FIELD_W
                    && b.pos[1] + hh >= 0.0
                    && b.pos[1] - hh <= FIELD_H;
                let roamer = b.ex_flags & 0xdc0 != 0;
                if in_bounds {
                    b.oob_count = 0;
                } else if !roamer && b.oob_count == 0 {
                    b.oob_count = i32::MAX;
                } else {
                    b.oob_count += 1;
                }
            }
        }
        self.world.bullets.retain(|b| b.oob_count < 256);

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

        // MarisaB orb beams: damage enemies in each orb's vertical column.
        if self.beam_dmg > 0 {
            let orbs = self.orb_positions();
            for e in &mut self.enemies {
                if !e.occupied || !e.interactable || !e.damageable {
                    continue;
                }
                for o in orbs {
                    if (e.pos[0] - o[0]).abs() < 10.0 && e.pos[1] <= o[1] {
                        e.life -= self.beam_dmg;
                        self.score += (self.beam_dmg.min(70) / 5 * 10) as i64;
                        break;
                    }
                }
            }
        }

        // Enemy deaths award the enemy's score value (default 100).
        let mut drops: Vec<([f32; 2], i16)> = Vec::new();
        let mut boss_died = false;
        for i in 0..self.enemies.len() {
            let e = &mut self.enemies[i];
            if e.occupied && e.interactable && e.life <= 0 {
                // Per EnemyManager.cpp:645 death switch: modes 0/1 award score,
                // modes 0/1/2 drop items, mode 3 (survived final spell) does
                // neither and leaves the boss alive.
                let dm = e.death_mode;
                if dm != 3 {
                    drops.push(([e.pos[0], e.pos[1]], e.item_drop));
                }
                if dm == 0 || dm == 1 {
                    self.score += e.score as i64;
                }
                // Faithful death particles (EnemyManager.cpp:641-706): the etama4
                // bubble + splash, drawn by the EffectManager. Idempotent via
                // death_fx_done. Player kills reach here (life set in collide), so
                // without this they only got the placeholder spawn_burst.
                e.spawn_death_effects(&mut self.world);
                if e.is_boss && dm != 3 {
                    boss_died = true;
                }
                e.on_death(&self.ecl, &mut self.world);
                self.events.push(Event::Sfx("enep00"));
            }
        }
        // A boss death cancels the field for the bonus (EnemyManager.cpp:693
        // DespawnBullets(12800, false)) — bonus only, no point items.
        if boss_died {
            self.despawn_bullets(false);
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
        // Oracle-comparison mode: skip dialogue so the timeline doesn't freeze
        // (matches the full-VM oracle's stubbed Gui::MsgWait). The MsgWait
        // timeline op then never holds, so the boss/midboss attack interrupts
        // fire on schedule, exactly as in the stubbed decomp run.
        if std::env::var_os("TH06_NO_DIALOGUE").is_some() {
            return;
        }
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
            self.events.push(Event::Bgm(self.boss_bgm));
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
                        self.boss_bgm
                    } else {
                        self.stage_bgm
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
        self.items.push(Item::fall(pos, kind));
    }

    /// Burst of fading puffs, used for enemy deaths and pickups. Purely cosmetic:
    /// it draws from a SEPARATE `fx_rng`, never the gameplay `world.rng`, so the
    /// visual particle jitter can't desync the decomp-faithful RNG stream (#26).
    /// The faithful gameplay effect RNG is handled by World::spawn_particles.
    fn spawn_burst(&mut self, pos: [f32; 2], count: u32, speed: f32, color: [f32; 3], size: f32) {
        for i in 0..count {
            let a = i as f32 / count as f32 * std::f32::consts::TAU
                + self.fx_rng.f32_in_range(0.5);
            let s = speed * (0.5 + self.fx_rng.f32_zero_to_one());
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
        // Point of collection: at max power, crossing y<128 latches every
        // falling item to homing (state 1). Once latched it stays homing even
        // if the player drops back below the line; only items spawned afterwards
        // (still state 0) keep falling. Matches ItemManager::OnUpdate.
        let poc = alive && self.world.power >= 128 && player[1] < 128.0;
        let mut collected: Vec<(i32, [f32; 2])> = Vec::new();
        for it in &mut self.items {
            if it.state == 2 {
                // Scatter arc: lerp to target over 60 frames, then fall.
                it.timer += 1;
                if it.timer < 60 {
                    let f = it.timer as f32 / 60.0;
                    it.pos[0] = f * it.target[0] + it.start[0] * (1.0 - f);
                    it.pos[1] = f * it.target[1] + it.start[1] * (1.0 - f);
                } else {
                    if it.timer == 60 {
                        it.vy = 0.0;
                    }
                    it.vy = (it.vy + 0.03).min(3.0);
                    it.pos[1] += it.vy;
                }
            } else {
                if poc {
                    it.state = 1;
                }
                if it.state == 1 {
                    let dx = player[0] - it.pos[0];
                    let dy = player[1] - it.pos[1];
                    let len = (dx * dx + dy * dy).sqrt().max(0.001);
                    it.pos[0] += dx / len * 8.0;
                    it.pos[1] += dy / len * 8.0;
                } else {
                    it.vy = (it.vy + 0.03).min(3.0);
                    it.pos[1] += it.vy;
                }
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
                1 | 6 => [0.5, 0.7, 1.0],   // point / point-bullet = blue
                3 | 5 => [1.0, 0.6, 0.6],   // bomb/life = red
                _ => [1.0, 0.4, 0.4],       // power = red
            };
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
                    // ITEM_POINT (Normal, calculatePointScore): 100000 at the
                    // collection line (y < 128), else 60000 - (y-128)*100.
                    let y = pos[1] as i64;
                    self.score += if y < 128 { 100_000 } else { (60_000 - (y - 128) * 100).max(0) };
                    self.point_items += 1;
                }
                6 => {
                    // ITEM_POINT_BULLET: (grazeInStage/3)*10 + 500, or 100 while
                    // a bomb is active.
                    self.score += if self.bombing > 0 { 100 } else { (self.graze / 3) * 10 + 500 };
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
            // Reaching max power cancels every bullet into point items.
            if self.world.power >= 128 {
                self.bullets_to_points();
            }
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

    /// BulletManager::TurnAllBulletsIntoPoints: each live bullet becomes a
    /// homing point-bullet item; lasers are cancelled too. Used on full power,
    /// spellcard start, and spellcard capture.
    fn bullets_to_points(&mut self) {
        for b in &self.world.bullets {
            self.items.push(Item::homing(b.pos, 6));
        }
        self.world.bullets.clear();
        self.cancel_lasers();
    }

    /// BulletManager::DespawnBullets: cancel every bullet/laser, awarding an
    /// escalating bonus (2000, +10 each, capped at 12800), optionally as point
    /// items, then add it to the score and show the "BONUS" popup.
    fn despawn_bullets(&mut self, award_points: bool) {
        const MAX_BONUS: i64 = 12800;
        let positions: Vec<[f32; 2]> = self.world.bullets.iter().map(|b| b.pos).collect();
        let laser_count = self.world.lasers.iter().filter(|l| l.in_use && l.state < 2).count();
        let mut bullet_score = 2000i64;
        let mut total = 0i64;
        for pos in &positions {
            if award_points {
                self.items.push(Item::homing(*pos, 6));
            }
            total += bullet_score;
            bullet_score = (bullet_score + 10).min(MAX_BONUS);
        }
        for _ in 0..laser_count {
            total += bullet_score;
            bullet_score = (bullet_score + 10).min(MAX_BONUS);
        }
        self.world.bullets.clear();
        self.cancel_lasers();
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
                WorldEvent::Sfx(idx) => {
                    // SoundIdx indexes g_SFXList directly (SFX_BY_IDX).
                    let name = SFX_BY_IDX.get(idx as usize).copied().unwrap_or("tan00");
                    self.events.push(Event::Sfx(name));
                }
                WorldEvent::SpellcardStart(id, sprite, _raw) => {
                    self.spell_active = true;
                    self.spell_name = spellcard_name(id).to_string();
                    self.spell_id = id;
                    self.spell_portrait_sprite = sprite;
                    self.spell_capturing = true;
                    // Start the name announce + portrait slide-in (ShowSpellcard).
                    self.spell_name_runner = Some(AnmRunner::new(self.spell_name_script.clone()));
                    self.portrait_runner = Some(AnmRunner::new(self.portrait_script.clone()));
                    // SPELLCARDSTART cancels the prior pattern into point items.
                    self.bullets_to_points();
                    self.events.push(Event::Sfx("cat00"));
                }
                WorldEvent::SpellcardEnd => {
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
                        self.despawn_bullets(true);
                        if captured {
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
                }
                WorldEvent::SpellTimeout => {
                    // Ran the timer out on a non-timeout spell: no capture bonus,
                    // and the field fades via RemoveAllBullets(0) — the bullets
                    // drift out over the donut anm rather than vanishing
                    // (EnemyManager.cpp:421). (#14)
                    self.spell_capturing = false;
                    self.world.despawn_all_bullets();
                    self.cancel_lasers();
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
                WorldEvent::DropItem(pos, kind) => {
                    if kind >= 0 {
                        self.items.push(Item::fall(pos, kind));
                    }
                }
            }
        }
    }

    /// Resolve a player-anm script id to its first sprite from the *current*
    /// character's sheet (player00 Reimu / player01 Marisa), so the same shot
    /// id renders that character's own bullet art.
    fn shot_sprite_ref(&self, script_id: i32) -> Option<SpriteRef> {
        let script = self.player_scripts.get(&script_id)?;
        let idx = script.iter().find(|i| i.opcode == 1).map(|i| i.arg_u32(0))?;
        let sp = self.player_sprites.get(&idx)?;
        Some(SpriteRef { tex: self.player_tex, rect: [sp.x, sp.y, sp.width, sp.height] })
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
                        additive: false,
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
                additive: false,
            });
        }

        // (Boss health bar / timer / spell name are drawn in draw_hud, over the
        // field, from front.anm sprites — Gui::DrawGameScene.)

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
                src: [sp.x / btw, sp.y / bth, (sp.x + sp.width) / btw, (sp.y + sp.height) / bth],
                tint: [1.0, 1.0, 1.0, 1.0],
                rot: 0.0,
                additive: false,
            });
        }

        // Player shots: each shot's anm script id is resolved against the
        // current character's own sheet, so Reimu shows amulets and Marisa her
        // stars/missiles from the same ids. Tall sprites (needles/lasers) are
        // oriented along their travel direction.
        for s in &self.shots {
            let sr = self.shot_sprite_ref(s.anm_script).unwrap_or(AMULET);
            let mut cmd = sprite_at(sr, s.pos, 0.95);
            if sr.rect[3] > sr.rect[2] * 1.4 {
                cmd.rot = s.vel[1].atan2(s.vel[0]) + std::f32::consts::FRAC_PI_2;
            }
            cmds.push(cmd);
        }

        // MarisaB orb beams behind the player.
        if self.beam_dmg > 0 {
            for o in self.orb_positions() {
                cmds.push(DrawCmd {
                    tex: TEX_WHITE,
                    dst: [FIELD_X + o[0] - 5.0, FIELD_Y, 10.0, o[1]],
                    src: [0.25, 0.25, 0.75, 0.75],
                    tint: [0.7, 0.5, 1.0, 0.55 + 0.2 * (self.anim as f32 * 0.5).sin()],
                    rot: 0.0,
                    additive: false,
                });
            }
        }

        // Player: the player anm runner drives the idle/banking sprite.
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
                    tex: self.player_tex,
                    dst: [
                        FIELD_X + self.pos[0] - sp.width / 2.0,
                        FIELD_Y + self.pos[1] - sp.height / 2.0,
                        sp.width,
                        sp.height,
                    ],
                    src: [u0, sp.y / pth, u1, (sp.y + sp.height) / pth],
                    tint: [1.0, 1.0, 1.0, 1.0],
                    rot: 0.0,
                    additive: false,
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

        // Master Spark: a huge bright beam fired straight up from the player.
        // The damage is full playfield width, but the *visible* spark is a wide
        // central column — bright white core wrapped in pink/cyan bloom, with a
        // flare at the muzzle — built from stacked alpha quads (the renderer is
        // alpha-blended, so overlapping layers bloom toward white).
        if self.bomb_kind == 3 && self.bombing > 0 {
            let t = self.anim as f32;
            let flick = 0.85 + 0.15 * (t * 0.9).sin();
            let pulse = 0.92 + 0.08 * (t * 0.5).sin();
            let h = self.pos[1]; // beam reaches from the top edge to the player
            let cx = self.pos[0];
            // Faint full-width wash: the spark lights the whole field.
            cmds.push(rect([FIELD_X, FIELD_Y, FIELD_W, h], [0.80, 0.70, 1.0, 0.20 * flick]));
            // Wide column: cyan/pink outer glow grading into a white-hot core.
            for (w, col) in [
                (210.0, [0.55, 0.85, 1.0, 0.32]), // cyan outer
                (150.0, [1.0, 0.65, 1.0, 0.42]),  // pink mid
                (96.0, [1.0, 0.95, 1.0, 0.65]),   // bright
                (52.0, [1.0, 1.0, 1.0, 0.92]),    // core
                (22.0, [1.0, 1.0, 1.0, 1.0]),     // hot center
            ] {
                let ww = w * pulse;
                cmds.push(rect(
                    [FIELD_X + cx - ww / 2.0, FIELD_Y, ww, h],
                    [col[0], col[1], col[2], col[3] * flick],
                ));
            }
            // Muzzle flare: layered glow bloom where the beam leaves the player.
            for s in [130.0, 80.0, 44.0] {
                let mut g = sprite_at(BOMB_GLOW, self.pos, 0.9 * flick);
                g.dst = [FIELD_X + cx - s / 2.0, FIELD_Y + h - s / 2.0, s, s];
                g.rot = t * 0.08;
                cmds.push(g);
            }
        }
        // Dream cross: a vertical + horizontal beam through the player.
        if self.bomb_kind == 1 && self.bombing > 0 {
            let a = 0.5 + 0.3 * (self.anim as f32 * 0.6).sin();
            cmds.push(DrawCmd {
                tex: TEX_WHITE,
                dst: [FIELD_X + self.pos[0] - 40.0, FIELD_Y, 80.0, FIELD_H],
                src: [0.25, 0.25, 0.75, 0.75],
                tint: [1.0, 0.6, 0.7, a],
                rot: 0.0,
                additive: false,
            });
            cmds.push(DrawCmd {
                tex: TEX_WHITE,
                dst: [FIELD_X, FIELD_Y + self.pos[1] - 40.0, FIELD_W, 80.0],
                src: [0.25, 0.25, 0.75, 0.75],
                tint: [1.0, 0.6, 0.7, a],
                rot: 0.0,
                additive: false,
            });
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
                additive: false,
            });
        }

        // Particle effects (etama4) via each effect's ANM runner — death
        // splashes, item-attract glows, etc. Drawn under the bullets. (NOTE: the
        // glow sprites are additively blended in the original; the engine only
        // does alpha tint, so glows read slightly dim — #17.)
        let [etw, eth] = self.eff_tex_size;
        for e in self.world.effects.iter().flatten() {
            let Some(idx) = e.runner.sprite else { continue };
            let Some(sp) = self.eff_sprites.get(&idx) else { continue };
            let w = sp.width * e.runner.scale[0].abs();
            let h = sp.height * e.runner.scale[1].abs();
            cmds.push(DrawCmd {
                tex: self.eff_tex,
                dst: [FIELD_X + e.pos[0] - w / 2.0, FIELD_Y + e.pos[1] - h / 2.0, w, h],
                src: [sp.x / etw, sp.y / eth, (sp.x + sp.width) / etw, (sp.y + sp.height) / eth],
                tint: [e.color[0], e.color[1], e.color[2], e.runner.alpha * e.color[3]],
                rot: e.runner.rotation,
                additive: e.runner.additive,
            });
        }

        // Bullets from the original scripts. Directional types (auto-rotate
        // flag in etama3.anm) face their travel angle; round types stay upright.
        let [tw, th] = self.bullet_tex_size;
        for b in &self.world.bullets {
            let Some(sp) = self.bullet_sprites.get(&b.sprite) else { continue };
            let rot = if self.bullet_autorotate[bullet_type_of(b.sprite)] {
                // Decomp: rotation.z = pi/2 - angle, applied with the matrix
                // [cos sin; -sin cos] (TranslateRotation). Our engine rotates with
                // the opposite-handed [cos -sin; sin cos], so the equivalent angle
                // is the negation: angle - pi/2.
                b.angle - std::f32::consts::FRAC_PI_2
            } else {
                0.0
            };
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
                rot,
                additive: false,
            });
        }

        // Lasers: the real etama3 laser sprite (ANM_SPRITE_BULLET3_LASER = 146,
        // + colour offset) stretched along the lit segment, plus the glowing
        // base ball (SPAWN_BIG_BALL = 140) at the origin — like SpawnLaserPattern.
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
            let color = l.color.max(0) as u32;
            if let Some(sp) = self.bullet_sprites.get(&(146 + color)) {
                // Beam: the laser sprite stretched to the lit length x width.
                cmds.push(DrawCmd {
                    tex: TEX_BULLET,
                    dst: [cx - len / 2.0, cy - width / 2.0, len, width],
                    src: [sp.x / tw, sp.y / th, (sp.x + sp.width) / tw, (sp.y + sp.height) / th],
                    tint: [1.0, 1.0, 1.0, 1.0],
                    rot: l.angle,
                    additive: true, // laser beam glows (etama3 laser op13)
                });
            } else {
                // Fallback (sprite missing): a tinted rect.
                let tint = LASER_COLORS[(l.color as usize) % LASER_COLORS.len()];
                cmds.push(DrawCmd {
                    tex: TEX_WHITE,
                    dst: [cx - len / 2.0, cy - width / 2.0, len, width],
                    src: [0.25, 0.25, 0.75, 0.75],
                    tint,
                    rot: l.angle,
                    additive: true,
                });
            }
            // Glowing ball at the laser base, while it is warming up or active.
            if l.state < 2 {
                if let Some(sp) = self.bullet_sprites.get(&(140 + color)) {
                    let ox = FIELD_X + l.pos[0] + l.angle.cos() * l.start_offset;
                    let oy = FIELD_Y + l.pos[1] + l.angle.sin() * l.start_offset;
                    let s = (width * 2.0).max(sp.width);
                    cmds.push(DrawCmd {
                        tex: TEX_BULLET,
                        dst: [ox - s / 2.0, oy - s / 2.0, s, s],
                        src: [sp.x / tw, sp.y / th, (sp.x + sp.width) / tw, (sp.y + sp.height) / th],
                        tint: [1.0, 1.0, 1.0, 1.0],
                        rot: 0.0,
                        additive: true, // SpawnLaserPattern sets vm1 blendMode = One
                    });
                }
            }
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
            draw_text(&mut cmds, [FIELD_X + FIELD_W - 28.0, FIELD_Y + 8.0], 16.0, tint, &format!("{secs:02}"));
        }

        // Boss spellcard portrait (enemySpellcardPortrait): the boss face sprite
        // selected by spellcardSprite, slid in from (480,240) to (320,240) and
        // faded out by face*.anm script 3.
        if let Some(r) = &self.portrait_runner {
            if r.visible() {
                let psx = if self.spell_portrait_sprite == 0 { 0.0 } else { 128.0 };
                let w = 128.0 * r.scale[0];
                let h = 256.0 * r.scale[1];
                cmds.push(DrawCmd {
                    tex: self.face_boss_tex,
                    dst: [r.pos[0] - w / 2.0, r.pos[1] - h / 2.0, w, h],
                    src: [psx / 256.0, 0.0, (psx + 128.0) / 256.0, 1.0],
                    tint: [1.0, 1.0, 1.0, r.alpha],
                    rot: 0.0,
                    additive: false,
                });
            }
        }

        // Spellcard name announce: driven by text.anm script 7's VM (fly-in at
        // (256,312) scale 3 -> shrink to 1 -> fly to (256,40) -> off). The name
        // sprite is centre-anchored at the VM pos; the blue bar (front.anm
        // script 24, enemySpellcardBackground) follows it, width strlen*15/2+16.
        if let Some(r) = &self.spell_name_runner {
            if r.visible() && !self.spell_name.is_empty() {
                let scale = r.scale[0].max(0.01);
                // The original draws the name in the clipped game-region viewport
                // with short Japanese text; the VM x is fixed at 256 until the
                // fly-off. Centre it in the playfield so the longer English name
                // stays readable, and follow the VM x once it flies off right.
                let cx = if r.pos[0] > 256.5 { r.pos[0] } else { FIELD_X + FIELD_W / 2.0 };
                let bar_len = self.spell_name.chars().count() as f32 * 15.0 / 2.0 + 16.0;
                if let Some(([sx, sy, sw, sh], _, bscale, _)) = self.hud.script_state(24) {
                    let ts = self.hud.tex_size();
                    let bh = sh * bscale[1];
                    cmds.push(DrawCmd {
                        tex: self.hud.tex(),
                        dst: [cx - bar_len / 2.0, r.pos[1] - bh / 2.0, bar_len, bh],
                        src: [sx / ts, sy / ts, (sx + sw) / ts, (sy + sh) / ts],
                        tint: [1.0, 1.0, 1.0, r.alpha],
                        rot: 0.0,
                        additive: false,
                    });
                }
                // Name text: 15px glyphs scaled by the VM, centred at `cx`.
                let w = self.spell_name.chars().count() as f32 * 14.0 * scale;
                draw_num_scaled(
                    &mut cmds,
                    [cx - w / 2.0, r.pos[1] - 7.5 * scale],
                    scale,
                    [1.0, 0.94, 0.94, r.alpha],
                    &self.spell_name,
                );
            }
        }

        // Dialogue box.
        if self.dialogue.active && (!self.dialogue.lines[0].is_empty() || !self.dialogue.lines[1].is_empty()) {
            // Portraits: player (Reimu) lower-left, boss (Rumia) lower-right,
            // the active speaker at full brightness. Each face sheet holds
            // two 128x256 expression columns.
            let pw = 96.0;
            let ph = 192.0;
            let py = FIELD_Y + FIELD_H - 80.0 - ph;
            for (idx, tex) in [(0usize, self.face_player_tex), (1usize, self.face_boss_tex)] {
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
                    additive: false,
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

    /// Roll the displayed score toward the real score (GameManager guiScore).
    fn roll_gui_score(&mut self) {
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
            });
        }

        // Health bar (script 21): top-left anchored at field (96, 24), width
        // `bossHealthBar2 * 288` (the decomp's scaleX*14), keeping the script's
        // own scaleY (0.3 -> ~4px tall) and fade-in alpha.
        if let Some(([sx, sy, sw, sh], _, scale, alpha)) = self.hud.script_state(21) {
            cmds.push(DrawCmd {
                tex: self.hud.tex(),
                dst: [FIELD_X + 96.0, FIELD_Y + 24.0, self.boss_bar * 288.0, sh * scale[1]],
                src: [sx / ts, sy / ts, (sx + sw) / ts, (sy + sh) / ts],
                tint: [1.0, 1.0, 1.0, alpha],
                rot: 0.0,
                additive: false,
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
            self.hud.draw_sprite(cmds, 16, vx + i as f32 * 16.0, 122.0, 1.0);
        }
        for i in 0..self.bombs.max(0) {
            self.hud.draw_sprite(cmds, 17, vx + i as f32 * 16.0, 146.0, 1.0);
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
            draw_num(cmds, [px, 232.0], [0.753, 0.690, 1.0, 1.0], "Full Power Mode!!");
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
            draw_num(cmds, [px, 32.0], [1.0, 1.0, 0.502, 1.0], &format!("BONUS {:8}", self.bonus_score_amount));
        }

        // "Spell Card Bonus!" + "+N" popup (Gui.cpp:192-210), centred near top.
        if self.spell_bonus_timer > 0 {
            let title = "Spell Card Bonus!";
            let tx = (FIELD_W - title.len() as f32 * 16.0) / 2.0 + FIELD_X;
            draw_num(cmds, [tx, FIELD_Y + 64.0], [1.0, 0.0, 0.0, 1.0], title);
            let amt = format!("+{}", self.spell_bonus_amount);
            let ax = (FIELD_W - amt.len() as f32 * 32.0) / 2.0 + FIELD_X;
            draw_num_scaled(cmds, [ax, FIELD_Y + 80.0], 2.0, [1.0, 0.502, 0.502, 1.0], &amt);
        }

        self.draw_boss_ui(cmds);

        match self.state {
            PlayerState::GameOver(_) => {
                cmds.push(rect([FIELD_X, FIELD_Y, FIELD_W, FIELD_H], [0.0, 0.0, 0.0, 0.6]));
            }
            PlayerState::Cleared(_) => {
                // Stage-clear bonus breakdown (Gui.cpp:88-172), drawn over the
                // field at (GAME_REGION_LEFT+42, TOP+112).
                let x = FIELD_X + 42.0;
                let graze_in = (self.graze - self.graze_start).max(0);
                let (diff_text, _) = match self.world.difficulty {
                    0 => ("Easy Rank      * 0.5", 0),
                    2 => ("Hard Rank      * 1.2", 0),
                    3 => ("Lunatic Rank   * 1.5", 0),
                    4 => ("Extra Rank     * 2.0", 0),
                    _ => ("Normal Rank    * 1.0", 0),
                };
                draw_num(cmds, [x, FIELD_Y + 112.0], [1.0, 1.0, 0.251, 1.0], "Stage Clear");
                draw_num(cmds, [x, FIELD_Y + 144.0], [1.0, 1.0, 1.0, 1.0], &format!("Stage * 1000 = {:5}", self.stage_num as i64 * 1000));
                draw_num(cmds, [x, FIELD_Y + 160.0], [0.878, 0.878, 1.0, 1.0], &format!("Power *  100 = {:5}", self.world.power as i64 * 100));
                draw_num(cmds, [x, FIELD_Y + 176.0], [0.816, 0.816, 1.0, 1.0], &format!("Graze *   10 = {:5}", graze_in * 10));
                draw_num(cmds, [x, FIELD_Y + 192.0], [1.0, 0.502, 0.502, 1.0], &format!("    * Point Item {:3}", self.point_items));
                draw_num(cmds, [x, FIELD_Y + 224.0], [1.0, 0.502, 0.502, 1.0], diff_text);
                draw_num(cmds, [x, FIELD_Y + 240.0], [1.0, 1.0, 1.0, 1.0], &format!("Total     = {:8}", self.clear_bonus));
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

/// etama3 bullet type (0..9) for a final sprite index, via the per-type base
/// sprite ranges (`BULLET_BASE_SPRITE` is ascending).
fn bullet_type_of(sprite: u32) -> usize {
    let mut t = 0;
    for (i, &base) in BULLET_BASE_SPRITE.iter().enumerate() {
        if sprite >= base {
            t = i;
        } else {
            break;
        }
    }
    t
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

/// Stage-clear bonus, an exact port of `Gui.cpp:935-963`:
/// `(stage*1000 + grazeInStage*10 + power*100) * pointItems`, then (final stage
/// only) `+lives*3M + bombs*1M`, then the difficulty multiplier with a `-= %10`
/// round-down. `difficulty`: 0 Easy, 1 Normal, 2 Hard, 3 Lunatic, 4 Extra.
/// (The lifeCount-config penalty isn't modelled — the port has no such option.)
fn stage_clear_bonus(
    stage: i32,
    graze_in_stage: i64,
    power: i64,
    point_items: i64,
    lives: i64,
    bombs: i64,
    difficulty: u8,
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
        0 => s / 2,          // Easy
        2 => s * 12 / 10,    // Hard
        3 => s * 15 / 10,    // Lunatic
        4 => s * 2,          // Extra
        _ => return s,       // Normal: no multiplier, no round-down
    };
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
            });
        }
        x += size * 0.75;
    }
}

fn rect(dst: [f32; 4], tint: [f32; 4]) -> DrawCmd {
    DrawCmd { tex: TEX_WHITE, dst, src: [0.25, 0.25, 0.75, 0.75], tint, rot: 0.0, additive: false }
}

fn sprite_at(s: SpriteRef, field_pos: [f32; 2], alpha: f32) -> DrawCmd {
    let [x, y, w, h] = s.rect;
    DrawCmd {
        tex: s.tex,
        dst: [FIELD_X + field_pos[0] - w / 2.0, FIELD_Y + field_pos[1] - h / 2.0, w, h],
        src: [x / 256.0, y / 256.0, (x + w) / 256.0, (y + h) / 256.0],
        tint: [1.0, 1.0, 1.0, alpha],
        rot: 0.0,
        additive: false,
    }
}

