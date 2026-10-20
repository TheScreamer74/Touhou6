//! ECL virtual machine — a port of the original engine's enemy scripting,
//! following the th06 decompilation (EclManager.cpp / EnemyManager.cpp /
//! BulletManager.cpp) opcode by opcode. Coordinates are playfield-local
//! (384x448, origin top-left), matching the original.

use crate::anm_vm::AnmRunner;
use std::collections::HashMap;
use th06_formats::anm0::Instr as AnmInstr;
use th06_formats::ecl::{Ecl, Instr};

pub const FIELD_W: f32 = 384.0;
pub const FIELD_H: f32 = 448.0;

/// Player::AngleToPlayer shoots down when both positions coincide.
pub(crate) fn player_angle_from(player: [f32; 2], origin: [f32; 2]) -> f32 {
    let dx = player[0] - origin[0];
    let dy = player[1] - origin[1];
    if dx == 0.0 && dy == 0.0 {
        std::f32::consts::FRAC_PI_2
    } else {
        f64::from(dy).atan2(f64::from(dx)) as f32
    }
}

/// ZunMath::sincosmul retains the FSINCOS result until after multiplication;
/// rounding the trigonometric value to f32 first changes the stored velocity.
pub(crate) fn sincos_mul(angle: f32, multiplier: f32) -> [f32; 2] {
    [
        (f64::from(angle).cos() * f64::from(multiplier)) as f32,
        (f64::from(angle).sin() * f64::from(multiplier)) as f32,
    ]
}

/// The original 16-bit RNG (Global.cpp: Rng).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rng {
    seed: u16,
}

impl Rng {
    pub fn new(seed: u16) -> Self {
        Self { seed }
    }
    pub fn seed(&self) -> u16 {
        self.seed
    }
    pub fn u16(&mut self) -> u16 {
        let a = (self.seed ^ 0x9630).wrapping_sub(0x6553);
        self.seed = ((a & 0xc000) >> 14).wrapping_add(a.wrapping_mul(4));
        self.seed
    }
    pub fn u32(&mut self) -> u32 {
        ((self.u16() as u32) << 16) | self.u16() as u32
    }
    pub fn f32_zero_to_one(&mut self) -> f32 {
        self.u32() as f32 / u32::MAX as f32
    }
    pub fn u32_in_range(&mut self, range: u32) -> u32 {
        if range != 0 { self.u32() % range } else { 0 }
    }
    pub fn u16_in_range(&mut self, range: u16) -> u16 {
        if range != 0 { self.u16() % range } else { 0 }
    }
    pub fn f32_in_range(&mut self, range: f32) -> f32 {
        self.f32_zero_to_one() * range
    }
}

pub fn normalize_angle(mut a: f32) -> f32 {
    while a > std::f32::consts::PI {
        a -= std::f32::consts::TAU;
    }
    while a < -std::f32::consts::PI {
        a += std::f32::consts::TAU;
    }
    a
}

fn cross3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn normalize3(v: [f32; 3]) -> [f32; 3] {
    let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if len > 1e-6 {
        [v[0] / len, v[1] / len, v[2] / len]
    } else {
        [0.0, 0.0, 0.0]
    }
}

/// One ECL execution context (EnemyEclContext).
#[derive(Clone, Copy)]
struct Ctx {
    pc: u32,
    time: i32,
    sub: u16,
    ivars: [i32; 8],
    fvars: [f32; 4],
    cmp: i32,
    /// `currentContext.funcSetFunc`: index into the ex-instruction table that
    /// EXINSREPEAT runs every frame (while life > 0). -1 = none. Saved/restored
    /// with the call stack because it lives in EnemyEclContext.
    func_set_func: i32,
}

impl Default for Ctx {
    fn default() -> Self {
        Self {
            pc: 0,
            time: 0,
            sub: 0,
            ivars: [0; 8],
            fvars: [0.0; 4],
            cmp: 0,
            func_set_func: -1,
        }
    }
}

pub struct BulletProps {
    pub sprite: i16,
    pub sprite_offset: i32,
    pub pos: [f32; 2],
    pub angle1: f32,
    pub angle2: f32,
    pub speed1: f32,
    pub speed2: f32,
    pub count1: i16,
    pub count2: i16,
    pub aim_mode: u16,
    pub flags: u32,
    pub sfx: i32,
    pub ex_ints: [i32; 4],
    pub ex_floats: [f32; 4],
}

impl Default for BulletProps {
    fn default() -> Self {
        Self {
            sprite: 0,
            sprite_offset: 0,
            pos: [0.0, 0.0],
            angle1: 0.0,
            angle2: 0.0,
            speed1: 0.0,
            speed2: 0.0,
            count1: 1,
            count2: 1,
            aim_mode: 0,
            flags: 0,
            sfx: -1,
            ex_ints: [0; 4],
            ex_floats: [0.0; 4],
        }
    }
}

pub struct Bullet {
    /// Fixed BulletManager slot: iteration order is significant for ex-insns,
    /// cancel scores and the shared RNG after the allocation cursor wraps.
    pub pool_index: usize,
    pub pos: [f32; 2],
    pub angle: f32,
    pub speed: f32,
    /// Cached velocity: acceleration modifies it independently of `speed`.
    pub velocity: [f32; 2],
    pub type_index: usize,
    /// Original pre-executed bullet, spawn and cancellation ANM instances.
    pub visuals: Option<crate::projectiles::BulletVisuals>,
    /// etama3 sprite index (type base + color offset).
    pub sprite: u32,
    /// Template baseSpriteIndex stays fixed when the fireball animates.
    pub base_sprite: u32,
    /// Colour offset within the bullet type's sprite group (the `spriteOffset`
    /// the decomp tracks; ex-instructions test/overwrite it, e.g. Sakuya's
    /// redirect sets it to 5).
    pub sprite_offset: i32,
    /// Sprite height in px (the type's `heightPx`), used by ex-instructions that
    /// only affect "big" bullets (>= 30px).
    pub height: f32,
    /// Sprite width (px), for the off-screen IsInBounds cull (x axis).
    pub width: f32,
    pub spawn_delay: u32,
    /// Speed multiplier while `spawn_delay > 0` (the spawn-in slow-down).
    pub spawn_factor: f32,
    /// Frames spent out of bounds (BulletManager `unk_5c0`): normal bullets are
    /// culled on the first OOB frame; dir-change/bounce bullets get 256 frames.
    pub oob_count: i32,
    pub timer: i32,
    pub ex_flags: u32,
    /// flag 0x10: acceleration vector; 0x20: (speed delta, angle delta);
    /// dir-change group 0x40/0x80/0x100: (rotation, new speed) applied every
    /// `ex_int0` frames, `ex_int1` times — rotate / aim-at-player / absolute.
    pub ex_accel: [f32; 2],
    pub ex_f: [f32; 2],
    pub ex_int0: i32,
    pub ex_int1: i32,
    pub ex_count: i32,
    /// Set once the bullet has been grazed, so it scores graze only once.
    pub grazed: bool,
    /// BULLET_STATE_DESPAWNING (BulletManager.cpp:947): when > 0, the bullet is
    /// fading out — it drifts at half velocity and is removed when this reaches
    /// 0, running over the 12-frame donut anm. Set by a non-timeout-spell spell
    /// timeout (RemoveAllBullets(0)); 0 means a normal live bullet. (#14)
    pub despawn_timer: i32,
}

/// A laser beam (Laser in the original). The lit segment runs from
/// `start_offset` to `end_offset` along `angle` from `pos`.
pub struct Laser {
    pub in_use: bool,
    pub pos: [f32; 2],
    pub angle: f32,
    pub speed: f32,
    pub start_offset: f32,
    pub end_offset: f32,
    pub start_length: f32,
    pub width: f32,
    pub start_time: i32,
    pub duration: i32,
    pub despawn_duration: i32,
    pub hitbox_start: i32,
    pub hitbox_end: i32,
    pub flags: u16,
    /// Local etama3 laser script: 9 line / 10 round beam.
    pub script: u32,
    /// Drawing state evaluated before the timer increments.
    pub draw_width: f32,
    pub draw_alpha: f32,
    pub state: u8, // 0 warmup, 1 active, 2 despawning
    pub timer: i32,
    pub color: i32,
}

pub struct Enemy {
    pub pool_index: usize,
    ctx: Ctx,
    stack: [Ctx; 8],
    stack_depth: usize,
    pub pos: [f32; 3],
    pub hitbox: [f32; 2],
    axis_speed: [f32; 2],
    pub angle: f32,
    angular_velocity: f32,
    speed: f32,
    acceleration: f32,
    shoot_offset: [f32; 2],
    move_interp: [f32; 2],
    move_interp_start: [f32; 2],
    move_timer: i32,
    move_start_time: i32,
    // movement: 0 = axis velocity, 1 = angle/speed, 2 = interpolation
    movement_mode: u8,
    ease_type: u8,
    rank_speed_low: f32,
    rank_speed_high: f32,
    rank_amount1_low: i32,
    rank_amount1_high: i32,
    rank_amount2_low: i32,
    rank_amount2_high: i32,
    pub life: i32,
    /// EnemyManager::unk_e41 suppresses damage flashes for four ticks.
    damage_flash_timer: u8,
    pub max_life: i32,
    pub score: i32,
    boss_timer: i32,
    pub bullet_props: BulletProps,
    shoot_interval: i32,
    shoot_timer: i32,
    /// Remilia bat-wing EXINS (idx 6) state: the wing-flap timer (ramps the
    /// spawn rate) and the slowly-oscillating base wing angle.
    exins_func6_timer: i32,
    exins_func6_angle: f32,
    /// Remilia bat-transformation (idx 10) re-show delay: counts down 60 frames
    /// after a player bomb ends before she swaps back to her damageable bat form.
    exins_func10_timer: i32,
    interrupts: [i32; 8],
    run_interrupt: i32,
    lasers: [Option<usize>; 32],
    laser_store: usize,
    death_callback: i32,
    life_cb_threshold: i32,
    life_cb_sub: i32,
    timer_cb_threshold: i32,
    timer_cb_sub: i32,
    lower_bound: [f32; 2],
    upper_bound: [f32; 2],
    // flags
    pub occupied: bool,
    pub interactable: bool,
    pub collidable: bool,
    pub damageable: bool,
    pub is_boss: bool,
    pub invisible: bool,
    shooting_disabled: bool,
    invert_x: bool,
    clamp_pos: bool,
    disable_call_stack: bool,
    pub death_mode: u8,
    timeout_spell: bool,
    has_been_in_bounds: bool,
    pub item_drop: i16,
    /// ANMSETDEATH (op 100): death-effect anm indices. `death_anm1` is the
    /// bubble pop (no RNG); `death_anm2 + 4` is the splash particle index into
    /// g_Effects (RandomSplash → 2 RNG draws each). Used by the faithful death
    /// EffectManager spawns (#26).
    death_anm1: u8,
    death_anm2: u8,
    /// True once this enemy's death effects have been spawned (fire once).
    // ANM presentation
    pub anm_script: i32,
    pub anm_poses: [i16; 5], // default, far left, far right, left, right
    pub anm_pose_state: u8,  // 0xff = unset, 0 = default, 1 = left, 2 = right
    pub anm_dirty: bool,
    /// ECL op120 ANMFLAGROTATION: render the sprite rotated to the enemy's
    /// movement angle (EnemyManager.cpp:801, primaryVm.rotation.z = angle).
    pub rotate_anm: bool,
    /// ECL op128 ANMINTERRUPTMAIN: an interrupt label queued for the primary anm
    /// runner, applied before its next tick (primaryVm.pendingInterrupt).
    pub pending_interrupt: Option<i32>,
    /// ECL op99 ANMSETSLOT: per-slot sub-vm anm scripts (enemy->vms[8]). -1 = the
    /// slot is empty. `sub_dirty` marks a slot whose runner must be (re)built.
    /// Slots 0-3 render behind the primary sprite, 4-7 in front.
    pub sub_scripts: [i32; 8],
    pub sub_dirty: [bool; 8],
    /// ECL op129: per-slot pending ANM interruption, applied before ExecuteScript.
    pub sub_interrupts: [Option<i32>; 8],
    /// ECL op102 SPELLCARDEFFECT: indices into world.effects of this enemy's live
    /// spellcard bubbles (enemy->effectArray), plus their shared target radius
    /// (enemy->effectDistance). Driven each frame by `update_effects`.
    pub effect_slots: Vec<usize>,
    pub effect_distance: f32,
    pub boss_id: u8,
    /// Boss remaining-attack count shown by the HUD (ECL BOSSSETLIFECOUNT).
    pub spell_count: i32,
    /// Persistent state for ExInsShootStarPattern (g_StarAngleTable +
    /// g_EnemyPosVector/g_PlayerPosVector snapshots, which carry across the
    /// per-frame repeat calls).
    star_angle: [f32; 6],
    star_enemy_pos: [f32; 2],
    star_player_pos: [f32; 2],
}

impl Enemy {
    /// Read-only ECL context for comparison with the supplied 1.02h executable.
    pub fn reference_ecl_state(&self) -> (u16, u32, i32, i32) {
        (self.ctx.sub, self.ctx.pc, self.ctx.time, self.boss_timer)
    }
}

impl Default for Enemy {
    fn default() -> Self {
        Self {
            pool_index: usize::MAX,
            ctx: Ctx::default(),
            stack: [Ctx::default(); 8],
            stack_depth: 0,
            pos: [0.0; 3],
            hitbox: [12.0, 12.0],
            axis_speed: [0.0; 2],
            angle: 0.0,
            angular_velocity: 0.0,
            speed: 0.0,
            acceleration: 0.0,
            shoot_offset: [0.0; 2],
            move_interp: [0.0; 2],
            move_interp_start: [0.0; 2],
            move_timer: 0,
            move_start_time: 0,
            movement_mode: 0,
            ease_type: 0,
            // EnemyManager.cpp:83-84: a fresh enemy's bullet rank-speed influence
            // defaults to [-0.5, 0.5] (amounts 0). At rank 16 this contributes 0
            // (the midpoint), so it was invisible until dynamic rank moved off 16.
            rank_speed_low: -0.5,
            rank_speed_high: 0.5,
            rank_amount1_low: 0,
            rank_amount1_high: 0,
            rank_amount2_low: 0,
            rank_amount2_high: 0,
            life: 1,
            damage_flash_timer: 0,
            max_life: 1,
            score: 100,
            boss_timer: 0,
            bullet_props: BulletProps::default(),
            shoot_interval: 0,
            shoot_timer: 0,
            exins_func6_timer: 0,
            exins_func6_angle: 0.0,
            exins_func10_timer: 0,
            interrupts: [-1; 8],
            run_interrupt: -1,
            lasers: [None; 32],
            laser_store: 0,
            death_callback: -1,
            life_cb_threshold: -1,
            life_cb_sub: -1,
            timer_cb_threshold: -1,
            timer_cb_sub: -1,
            lower_bound: [0.0; 2],
            upper_bound: [FIELD_W, FIELD_H],
            occupied: true,
            interactable: true,
            collidable: true,
            damageable: true,
            is_boss: false,
            invisible: false,
            shooting_disabled: false,
            invert_x: false,
            clamp_pos: false,
            disable_call_stack: false,
            death_mode: 0,
            timeout_spell: false,
            has_been_in_bounds: false,
            item_drop: -1,
            death_anm1: 0,
            death_anm2: 0,
            anm_script: -1,
            anm_poses: [-1; 5],
            anm_pose_state: 0xff,
            anm_dirty: false,
            rotate_anm: false,
            pending_interrupt: None,
            sub_interrupts: [None; 8],
            sub_scripts: [-1; 8],
            sub_dirty: [false; 8],
            effect_slots: Vec::new(),
            effect_distance: 0.0,
            boss_id: 0,
            spell_count: 0,
            star_angle: [0.0; 6],
            star_enemy_pos: [0.0; 2],
            star_player_pos: [0.0; 2],
        }
    }
}

pub enum WorldEvent {
    Sfx(i32),
    /// Spell card declared: (spell id, raw Shift-JIS name bytes).
    SpellcardStart(i32, i32, Vec<u8>),
    SpellcardEnd {
        points: Vec<[f32; 2]>,
        bonus: i64,
    },
    /// A non-timeout boss spell ran out its timer: capture is forfeit and the
    /// field is cleared (EnemyManager.cpp:408 — isCapturing=0, RemoveAllBullets).
    SpellTimeout,
    BulletCancel(Vec<[f32; 2]>),
    ScorePopup([f32; 2], i64, [f32; 4]),
    BossSet(bool),
    BossLifeCount(i32),
    BossSeconds(i32),
    DropItem([f32; 2], i32),
    /// ECL op125 STDUNPAUSE: release the background camera park (Stage::unpauseFlag).
    Unpause,
}

/// Per-frame context the VM needs from the game.
pub struct World {
    pub rng: Rng,
    pub difficulty: u8, // 0 easy, 1 normal, 2 hard, 3 lunatic
    /// Internal difficulty (GameManager.rank): scales enemy bullet speed/count/
    /// interval. Driven by `sub_rank` via Increase/DecreaseSubrank, clamped to
    /// [min_rank, max_rank] (g_DifficultyInfo).
    pub rank: i32,
    pub sub_rank: i32,
    pub min_rank: i32,
    pub max_rank: i32,
    pub player_pos: [f32; 2],
    /// Player::positionCenter.z, initially 0.49 and 0.2 after a respawn.
    /// Sakuya's knife arc normalizes the complete three-dimensional vector.
    pub player_z: f32,
    pub bullets: Vec<Bullet>,
    pub next_bullet_index: usize,
    pub lasers: Vec<Laser>,
    pub events: Vec<WorldEvent>,
    /// Reserved fixed enemy slots, including children initializing inside ECL.
    pub enemy_slots: [bool; 256],
    pub pending_spawns: Vec<Enemy>,
    pub kill_trash: bool,
    pub boss_present: bool,
    pub power: i32,
    /// Player identity for boss ex-instructions: 0 Reimu / 1 Marisa, shot 0 A / 1 B.
    pub character: u8,
    pub shot_type: u8,
    /// Sakuya's time-stop (GameManager::isTimeStopped). While set, existing
    /// bullets freeze in place; toggled by EXINSCALL #4. The boss keeps moving
    /// and firing, so new bullets are laid down over the frozen field.
    pub time_stopped: bool,
    /// Player bomb in progress (g_Player.bombInfo.isInUse). Read by Remilia's
    /// bat-transformation ex-instruction (idx 10): while a bomb is active she
    /// becomes non-interactable and swaps to her dissipate animation.
    pub bomb_active: bool,
    /// EnemyManager::spellcardInfo.isActive: 0 inactive, 1 running, 2 timed out.
    pub spellcard_active: u8,
    /// Sprite height (px) per bullet type 0..9, from etama3.anm. Used by
    /// ex-instructions that only touch "big" bullets (>= 30px).
    pub bullet_heights: [f32; 10],
    /// Sprite width (px) per bullet type 0..9, from etama3.anm. Used (with
    /// height) by the off-screen IsInBounds cull.
    pub bullet_widths: [f32; 10],
    pub bullet_templates: Vec<crate::projectiles::BulletVisuals>,
    /// EffectManager (#26, #17): live particle effects. Each runs its g_Effects
    /// update callback (which draws the gameplay RNG on its first tick) and an
    /// etama4 ANM script for the visual. `update_effects` runs them between the
    /// enemy and bullet updates (chain prio 10) so the RNG advances exactly as
    /// the decomp's. `eff_scripts` are the etama4 per-script ANM instructions
    /// used to build each effect's runner.
    pub effects: Vec<Option<Effect>>,
    pub next_effect_index: usize,
    pub eff_scripts: HashMap<u32, Vec<AnmInstr>>,
    /// EnemyManager::randomItemSpawnIndex / randomItemTableIndex (drive the
    /// every-3rd random-item drop + its bigger death splash).
    pub random_item_spawn_index: i32,
    pub random_item_table_index: i32,
}

/// A live particle effect (EffectManager.cpp). `pos` is the drawn position
/// (`pos1`); `vel`/`accel` the splash motion (`unk_11c`/`unk_128`); `anchor`/
/// `dir` the Attract pivot (`position`/`pos2`). The `runner` plays the etama4
/// ANM script for the visual.
pub struct Effect {
    pub in_use: bool,
    /// Render position. For SpellBubble this is the Callback4 `pos1` (a 3D world
    /// point); for the 2D splashes it is the screen position.
    pub pos: [f32; 3],
    vel: [f32; 3],
    accel: [f32; 3],
    /// SpellBubble: the followed enemy position (`effect->position`).
    anchor: [f32; 3],
    /// SpellBubble: the swirl axis (`effect->pos2`).
    dir: [f32; 3],
    timer: i32,
    cb: EffCb,
    /// SpellBubble: radial sweep distance `unk_15c`, grows by 0.3/frame toward the
    /// enemy's effectDistance.
    sweep: f32,
    /// SpellBubble: `angleRelated`, advances by pi/100 each frame.
    angle_rel: f32,
    /// SpellBubble pop: -1 = live; >= 0 = fading out (`unk_17b`), freed at 16.
    pop: i32,
    /// True for the spellcard-declaration bubble (g_Effects 13, Callback4): drawn
    /// in 3D via the stage camera rather than as a flat 2D particle.
    pub spell_bubble: bool,
    pub runner: AnmRunner,
}

/// g_EffectsColorWithTextureBlending (BulletManager.cpp:21) — the default
/// spellcard-effect palette, indexed by SPELLCARDEFFECT's effectColorId.
pub const G_EFFECTS_COLOR: [u32; 28] = [
    0xff000000, 0xff303030, 0xff606060, 0xff500000, 0xff900000, 0xffff2020, 0xff400040, 0xff800080,
    0xffff30ff, 0xff000050, 0xff000090, 0xff2020ff, 0xff203060, 0xff304090, 0xff3080ff, 0xff005000,
    0xff009000, 0xff20ff20, 0xff206000, 0xff409010, 0xff80ff20, 0xff505000, 0xff909000, 0xffffff20,
    0xff603000, 0xff904010, 0xfff08020, 0xffffffff,
];

/// The g_Effects update-callback column (EffectManager.cpp:18).
#[derive(Clone, Copy, PartialEq)]
enum EffCb {
    None,
    RandomSplash,
    RandomSplashBig,
    Still,
    Attract,
    AttractSlow,
    /// EffectUpdateCallback4 (EffectManager.cpp:94): the spellcard-declaration
    /// bubble — a 3D swirl driven by the owning enemy's effectArray update.
    SpellBubble,
}

/// g_Effects[effect_idx] -> (etama4 local script id, update callback).
/// Scripts are etama4's local ids (ANM_SCRIPT_BULLET4_START + N). Effect 16 is
/// the spellcard background (a different anm); mapped to script 0 / None so it
/// neither draws RNG nor renders a particle.
fn g_effects(effect_idx: i32) -> (u32, EffCb) {
    match effect_idx {
        0 => (3, EffCb::None),            // bubble explosion small
        1 => (4, EffCb::None),            // bubble explosion spiral
        2 => (5, EffCb::None),            // bubble explosion normal
        3 => (6, EffCb::RandomSplashBig), // glow 1
        4 => (9, EffCb::RandomSplash),    // white particle
        5 => (10, EffCb::RandomSplash),
        6 => (11, EffCb::RandomSplash),
        7 => (12, EffCb::RandomSplash),
        8 => (13, EffCb::RandomSplash), // small particles
        9 => (14, EffCb::RandomSplash),
        10 => (15, EffCb::RandomSplash),
        11 => (16, EffCb::RandomSplash),
        12 => (17, EffCb::None),
        13..=15 => (18, EffCb::SpellBubble), // spellcard bubble cb4 (3D, no RNG)
        16 => (0, EffCb::None),              // spellcard bg (different anm); no particle
        17 => (7, EffCb::Attract),           // glow 2
        18 => (8, EffCb::AttractSlow),       // glow 3
        19 => (19, EffCb::Still),
        _ => (0, EffCb::None),
    }
}

impl World {
    pub fn reference_effects(&self, frame: i32) -> String {
        use std::fmt::Write;
        let mut out = String::new();
        for (slot, effect) in self.effects.iter().enumerate() {
            let Some(effect) = effect else { continue };
            let callback = match effect.cb {
                EffCb::None => 0,
                EffCb::RandomSplash => 1,
                EffCb::RandomSplashBig => 2,
                EffCb::Still => 3,
                EffCb::Attract => 5,
                EffCb::AttractSlow => 6,
                EffCb::SpellBubble => 4,
            };
            writeln!(
                out,
                "{frame},{slot},{},{callback},{},{:.9},{:.9},{:.9}",
                effect.timer,
                effect.runner.sprite.unwrap_or(u32::MAX),
                effect.pos[0],
                effect.pos[1],
                effect.pos[2]
            )
            .unwrap();
        }
        out
    }

    /// GameManager::IncreaseSubrank: add to subRank, carrying every 100 into a
    /// rank up, clamped to maxRank (GameManager.cpp:578).
    pub fn increase_subrank(&mut self, amount: i32) {
        self.sub_rank += amount;
        while self.sub_rank >= 100 {
            self.rank += 1;
            self.sub_rank -= 100;
        }
        if self.rank > self.max_rank {
            self.rank = self.max_rank;
        }
    }

    /// GameManager::DecreaseSubrank: subtract from subRank, borrowing every 100
    /// from a rank down, clamped to minRank (GameManager.cpp:593).
    pub fn decrease_subrank(&mut self, amount: i32) {
        self.sub_rank -= amount;
        while self.sub_rank < 0 {
            self.rank -= 1;
            self.sub_rank += 100;
        }
        if self.rank < self.min_rank {
            self.rank = self.min_rank;
        }
    }

    /// EffectManager::SpawnParticles (EffectManager.cpp:195) — allocate `count`
    /// effects of `effect_idx` at `pos`, each cycling `next_effect_index` to the
    /// next free slot. The initial ANM may draw a random sprite after allocation;
    /// movement callbacks draw on their first OnUpdate tick. `color` tints the
    /// etama4 sprite (white = unchanged).
    pub fn spawn_particles(&mut self, effect_idx: i32, pos: [f32; 3], count: i32, color: [f32; 4]) {
        let (script, cb) = g_effects(effect_idx);
        let Some(instrs) = self.eff_scripts.get(&script).cloned() else {
            return;
        };
        for _ in 0..count.max(0) {
            let mut runner = AnmRunner::new(instrs.clone());
            // SpawnParticles assigns vm.color after SetAndExecuteScript. A
            // fade already started by that script keeps its original RGBA
            // endpoints and can overwrite this tint on the next calculation.
            runner.set_rgba(color);
            let eff = Effect {
                in_use: true,
                pos,
                vel: [0.0; 3],
                accel: [0.0; 3],
                anchor: pos,
                dir: [0.0; 3],
                timer: 0,
                cb,
                sweep: 0.0,
                angle_rel: 0.0,
                pop: -1,
                spell_bubble: false,
                runner,
            };
            if self.alloc_effect(eff).is_none() {
                break;
            }
        }
    }

    /// SPELLCARDEFFECT (ECL op102): spawn one effect-13 bubble at `pos`, store its
    /// swirl axis `pos2` and tint, and return its slot so the owning enemy can
    /// drive it from its effectArray. Returns None if etama4 lacks the script.
    pub fn spawn_spell_bubble(
        &mut self,
        pos: [f32; 3],
        pos2: [f32; 3],
        color: [f32; 4],
    ) -> Option<usize> {
        let (script, cb) = g_effects(13);
        let instrs = self.eff_scripts.get(&script).cloned()?;
        let mut runner = AnmRunner::new(instrs);
        runner.set_rgba(color);
        let eff = Effect {
            in_use: true,
            pos,
            vel: [0.0; 3],
            accel: [0.0; 3],
            anchor: pos,
            dir: pos2,
            timer: 0,
            cb,
            sweep: 0.0,
            angle_rel: 0.0,
            pop: -1,
            spell_bubble: true,
            runner,
        };
        self.alloc_effect(eff)
    }

    /// Spawn one effect and seed its `unk_11c`/`unk_128` motion, as the bat-wing
    /// EXINS does (EnemyEclInstr.cpp:774). The Still callback then drifts it.
    pub fn spawn_particle_moving(
        &mut self,
        effect_idx: i32,
        pos: [f32; 3],
        color: [f32; 4],
        vel: [f32; 3],
        accel: [f32; 3],
    ) -> Option<usize> {
        let (script, cb) = g_effects(effect_idx);
        let instrs = self.eff_scripts.get(&script).cloned()?;
        let mut runner = AnmRunner::new(instrs);
        runner.set_rgba(color);
        let eff = Effect {
            in_use: true,
            pos,
            vel,
            accel,
            anchor: pos,
            dir: [0.0; 3],
            timer: 0,
            cb,
            sweep: 0.0,
            angle_rel: 0.0,
            pop: -1,
            spell_bubble: false,
            runner,
        };
        self.alloc_effect(eff)
    }

    fn alloc_effect(&mut self, mut effect: Effect) -> Option<usize> {
        for _ in 0..512 {
            let candidate = self.next_effect_index;
            self.next_effect_index = (candidate + 1) % 512;
            if self
                .effects
                .get(candidate)
                .is_none_or(|slot| slot.is_none())
            {
                if self.effects.len() <= candidate {
                    self.effects.resize_with(candidate + 1, || None);
                }
                if let Some((sprite, range)) = effect.runner.take_pending_random() {
                    effect
                        .runner
                        .set_sprite(sprite + self.rng.u16_in_range(range) as u32);
                }
                self.effects[candidate] = Some(effect);
                return Some(candidate);
            }
        }
        None
    }

    /// BulletManager::RemoveAllBullets(turnIntoItem=false): every live bullet
    /// enters the DESPAWNING fade (drifts at half velocity over the 12-frame
    /// donut anm) rather than vanishing instantly. Used by a non-timeout-spell
    /// spell timeout (EnemyManager.cpp:421). Already-fading bullets are skipped.
    pub fn despawn_all_bullets(&mut self) {
        for b in &mut self.bullets {
            if b.despawn_timer == 0 {
                b.begin_despawn();
            }
        }
        self.cancel_lasers(false, false, &mut Vec::new());
    }

    /// RemoveAllBullets(true) excludes bullets already running their despawn
    /// animation. Each live laser produces one item every 32 pixels.
    pub fn turn_bullets_into_points(&mut self) -> Vec<[f32; 2]> {
        let mut points = Vec::new();
        self.bullets.retain(|bullet| {
            if bullet.despawn_timer == 0 {
                points.push(bullet.pos);
                false
            } else {
                true
            }
        });
        self.cancel_lasers(true, false, &mut points);
        points
    }

    /// DespawnBullets also counts already fading bullets, and leaves their
    /// animation alive. Its escalating score counts bullets only, not lasers.
    pub fn despawn_bullets_for_bonus(&mut self, award_points: bool) -> (Vec<[f32; 2]>, i64) {
        let mut points = Vec::new();
        let mut score = 2000;
        let mut total = 0;
        for bullet in &mut self.bullets {
            if award_points {
                points.push(bullet.pos);
            }
            self.events.push(WorldEvent::ScorePopup(
                bullet.pos,
                score,
                if score >= 12800 {
                    [1.0, 1.0, 0.0, 1.0]
                } else {
                    [1.0; 4]
                },
            ));
            total += score;
            score = (score + 10).min(12800);
            if bullet.despawn_timer == 0 {
                bullet.begin_despawn();
            }
        }
        self.cancel_lasers(award_points, true, &mut points);
        (points, total)
    }

    fn cancel_lasers(
        &mut self,
        award_points: bool,
        include_origin: bool,
        points: &mut Vec<[f32; 2]>,
    ) {
        for laser in &mut self.lasers {
            if !laser.in_use {
                continue;
            }
            laser.hitbox_end = 0;
            if laser.state >= 2 {
                continue;
            }
            laser.state = 2;
            laser.timer = 0;
            laser.hitbox_end = 0;
            if award_points {
                // DespawnBullets emits each laser's origin immediately before
                // its sampled line. RemoveAllBullets samples only the line.
                // ItemManager slot allocation makes this ordering observable.
                if include_origin {
                    points.push(laser.pos);
                }
                let (sin, cos) = laser.angle.sin_cos();
                let mut offset = laser.start_offset;
                while laser.end_offset > offset {
                    points.push([laser.pos[0] + cos * offset, laser.pos[1] + sin * offset]);
                    offset += 32.0;
                }
            }
        }
    }

    /// EffectManager::OnUpdate (chain prio 10, EffectManager.cpp:250) — run each
    /// live effect's callback (which draws the gameplay RNG on its first tick),
    /// then tick its ANM runner; free the slot when the script ends. Runs between
    /// the enemy and bullet updates so the RNG advances exactly as the decomp's.
    pub fn update_effects(&mut self) {
        let pi = std::f32::consts::PI;
        let tau = std::f32::consts::TAU;
        for slot in &mut self.effects {
            let Some(e) = slot else { continue };
            let tick0 = e.timer == 0;
            match e.cb {
                EffCb::RandomSplash => {
                    if tick0 {
                        e.vel[0] = (self.rng.f32_zero_to_one() * 256.0 - 128.0) / 12.0;
                        e.vel[1] = (self.rng.f32_zero_to_one() * 256.0 - 128.0) / 12.0;
                        e.accel = [-e.vel[0] / 19.0, -e.vel[1] / 19.0, 0.0];
                    }
                    for k in 0..3 {
                        e.pos[k] += e.vel[k];
                        e.vel[k] += e.accel[k];
                    }
                }
                EffCb::RandomSplashBig => {
                    if tick0 {
                        e.vel[0] = (self.rng.f32_zero_to_one() * 256.0 - 128.0) * 4.0 / 33.0;
                        e.vel[1] = (self.rng.f32_zero_to_one() * 256.0 - 128.0) * 4.0 / 33.0;
                        e.accel = [-e.vel[0] / 20.0, -e.vel[1] / 20.0, 0.0];
                    }
                    for k in 0..3 {
                        e.pos[k] += e.vel[k];
                        e.vel[k] += e.accel[k];
                    }
                }
                EffCb::Still => {
                    for k in 0..3 {
                        e.pos[k] += e.vel[k];
                        e.vel[k] += e.accel[k];
                    }
                }
                EffCb::Attract | EffCb::AttractSlow => {
                    let span = if e.cb == EffCb::Attract { 60.0 } else { 240.0 };
                    if tick0 {
                        e.anchor = e.pos;
                        let a = self.rng.f32_zero_to_one() * tau - pi;
                        e.dir = [a.cos(), a.sin(), 0.0];
                    }
                    let r = 256.0 - e.timer as f32 * 256.0 / span;
                    for k in 0..3 {
                        e.pos[k] = r * e.dir[k] + e.anchor[k];
                    }
                }
                EffCb::SpellBubble => {
                    // EffectUpdateCallback4 (EffectManager.cpp:94): place the
                    // bubble on a circle perpendicular to its axis `pos2` (e.dir),
                    // radius `sweep`, rotated about that axis by `angle_rel`, then
                    // exaggerate depth (z*6). `anchor` is the followed enemy pos.
                    let n = normalize3(e.dir);
                    // posOffset = normalize(cross(n, (1,0,0))) = normalize(ny, -nx, 0)
                    let perp = normalize3([n[1], -n[0], 0.0]);
                    let mut off = [perp[0] * e.sweep, perp[1] * e.sweep, perp[2] * e.sweep];
                    // Rotate `off` about axis `n` by `angle_rel` via the unit
                    // quaternion (sin*n, cos): p' = p + 2w(n×p) + n×(2(n×p)).
                    let (s, c) = e.angle_rel.sin_cos();
                    let u = [n[0] * s, n[1] * s, n[2] * s];
                    let t = cross3([2.0 * u[0], 2.0 * u[1], 2.0 * u[2]], off);
                    let ut = cross3(u, t);
                    off = [
                        off[0] + c * t[0] + ut[0],
                        off[1] + c * t[1] + ut[1],
                        off[2] + c * t[2] + ut[2],
                    ];
                    off[2] *= 6.0;
                    e.pos = [
                        off[0] + e.anchor[0],
                        off[1] + e.anchor[1],
                        off[2] + e.anchor[2],
                    ];
                    if e.pop >= 0 {
                        e.pop += 1;
                        if e.pop >= 16 {
                            *slot = None;
                            continue;
                        }
                        let alpha = 1.0 - e.pop as f32 / 16.0;
                        e.runner.alpha = (alpha * 255.0) as i32 as f32 / 255.0;
                        let sc = 2.0 - alpha;
                        e.runner.scale = [sc, sc];
                    }
                }
                EffCb::None => {}
            }
            e.runner.tick();
            if let Some((sprite, range)) = e.runner.take_pending_random() {
                e.runner
                    .set_sprite(sprite + self.rng.u16_in_range(range) as u32);
            }
            e.timer += 1;
            if e.runner.ended() {
                *slot = None;
            }
        }
    }

    fn alloc_laser(&mut self, laser: Laser) -> usize {
        if let Some(i) = self.lasers.iter().position(|l| !l.in_use) {
            self.lasers[i] = laser;
            i
        } else if self.lasers.len() < 64 {
            self.lasers.push(laser);
            self.lasers.len() - 1
        } else {
            64 // original dummy slot; never participates in the update
        }
    }
}

pub struct SpawnReq {
    pub sub: i32,
    pub pos: [f32; 3],
    pub life: i16,
    pub item: i16,
    pub score: i32,
    pub mirror: bool,
}

fn rank_scale_i32(low: i32, high: i32, rank: i32) -> i32 {
    rank * (high - low) / 32 + low
}
fn rank_scale_f32(low: f32, high: f32, rank: f32) -> f32 {
    rank * (high - low) / 32.0 + low
}

impl Enemy {
    pub fn vacant() -> Self {
        Self {
            occupied: false,
            ..Default::default()
        }
    }

    pub fn spawn(ecl: &Ecl, world: &mut World, req: &SpawnReq) -> Option<Enemy> {
        let pc = *ecl.sub_offsets.get(req.sub as usize)?;
        let pool_index = world.enemy_slots.iter().position(|occupied| !occupied)?;
        world.enemy_slots[pool_index] = true;
        let mut e = Enemy {
            pool_index,
            ..Default::default()
        };
        if req.life >= 0 {
            e.life = req.life as i32;
        }
        e.pos = req.pos;
        e.invert_x = req.mirror;
        e.ctx.pc = pc;
        e.ctx.sub = req.sub as u16;
        e.ctx.time = 0;
        e.run_ecl(ecl, world);
        e.item_drop = req.item;
        world.enemy_slots[pool_index] = e.occupied;
        if req.life >= 0 {
            e.life = req.life as i32;
        }
        if req.score >= 0 {
            e.score = req.score;
        }
        e.max_life = e.life;
        Some(e)
    }

    fn call_sub(&mut self, ecl: &Ecl, sub: i32) {
        if let Some(&off) = ecl.sub_offsets.get(sub as usize) {
            self.ctx.pc = off;
            self.ctx.time = 0;
            self.ctx.sub = sub as u16;
        }
    }

    /// Resolve a possibly-variable i32 argument (negative magic IDs).
    fn get_i32(&self, raw: i32, world: &World) -> i32 {
        match raw {
            -10001 => self.ctx.ivars[0],
            -10002 => self.ctx.ivars[1],
            -10003 => self.ctx.ivars[2],
            -10004 => self.ctx.ivars[3],
            -10005 => self.ctx.fvars[0].to_bits() as i32,
            -10006 => self.ctx.fvars[1].to_bits() as i32,
            -10007 => self.ctx.fvars[2].to_bits() as i32,
            -10008 => self.ctx.fvars[3].to_bits() as i32,
            -10009 => self.ctx.ivars[4],
            -10010 => self.ctx.ivars[5],
            -10011 => self.ctx.ivars[6],
            -10012 => self.ctx.ivars[7],
            -10013 => world.difficulty as i32,
            -10014 => world.rank,
            -10015 => self.pos[0].to_bits() as i32,
            -10016 => self.pos[1].to_bits() as i32,
            -10017 => self.pos[2].to_bits() as i32,
            -10018 => world.player_pos[0].to_bits() as i32,
            -10019 => world.player_pos[1].to_bits() as i32,
            -10020 => 0,
            -10021 => self.angle_to_player(world).to_bits() as i32,
            -10023 => {
                let dx = world.player_pos[0] - self.pos[0];
                let dy = world.player_pos[1] - self.pos[1];
                (dx * dx + dy * dy).sqrt().to_bits() as i32
            }
            -10022 => self.boss_timer,
            -10024 => self.life,
            -10025 => world.character as i32 * 2 + world.shot_type as i32,
            other => other,
        }
    }

    /// EclGetVarFloat converts the numeric sentinel to an integer ID, then
    /// reinterprets the referenced 32 bits as a float without numeric conversion.
    fn get_f32(&self, raw: f32, world: &World) -> f32 {
        let id = raw as i32;
        if (-10025..=-10001).contains(&id) {
            f32::from_bits(self.get_i32(id, world) as u32)
        } else {
            raw
        }
    }

    /// EclSetVar copies the referenced four bytes for either SET opcode.
    fn set_var_bits(&mut self, id: i32, bits: u32) {
        self.set_var(id, bits as i32, f32::from_bits(bits));
    }

    fn set_var(&mut self, id: i32, int: i32, float: f32) {
        match id {
            -10001 => self.ctx.ivars[0] = int,
            -10002 => self.ctx.ivars[1] = int,
            -10003 => self.ctx.ivars[2] = int,
            -10004 => self.ctx.ivars[3] = int,
            -10005 => self.ctx.fvars[0] = float,
            -10006 => self.ctx.fvars[1] = float,
            -10007 => self.ctx.fvars[2] = float,
            -10008 => self.ctx.fvars[3] = float,
            -10009 => self.ctx.ivars[4] = int,
            -10010 => self.ctx.ivars[5] = int,
            -10011 => self.ctx.ivars[6] = int,
            -10012 => self.ctx.ivars[7] = int,
            -10015 => self.pos[0] = float,
            -10016 => self.pos[1] = float,
            -10017 => self.pos[2] = float,
            -10022 => self.boss_timer = int,
            -10024 => self.life = int,
            _ => {}
        }
    }

    fn angle_to_player(&self, world: &World) -> f32 {
        player_angle_from(world.player_pos, [self.pos[0], self.pos[1]])
    }

    /// Faithful port of EclManager::RunEcl.
    pub fn run_ecl(&mut self, ecl: &Ecl, world: &mut World) {
        let mut guard = 10_000;
        loop {
            guard -= 1;
            if guard == 0 {
                return;
            }
            if self.run_interrupt >= 0 {
                let instr = match ecl.instr_at(self.ctx.pc) {
                    Some(i) => i,
                    None => return,
                };
                self.ctx.pc += instr.offset_to_next as u32;
                if !self.disable_call_stack {
                    self.stack[self.stack_depth] = self.ctx;
                }
                let sub = self.interrupts[self.run_interrupt as usize & 7];
                self.call_sub(ecl, sub);
                if self.stack_depth < 7 {
                    self.stack_depth += 1;
                }
                self.run_interrupt = -1;
                continue;
            }

            let instr = match ecl.instr_at(self.ctx.pc) {
                Some(i) => i,
                None => {
                    self.despawn(world);
                    return;
                }
            };

            if self.ctx.time == instr.time {
                // skipForDifficulty: bit set = run on that difficulty.
                if instr.skip_for_difficulty & (1 << world.difficulty) != 0 {
                    match self.exec(ecl, world, &instr) {
                        Flow::Next => {}
                        Flow::Jumped => continue,
                        Flow::Kill => {
                            self.despawn(world);
                            return;
                        }
                    }
                }
                self.ctx.pc += instr.offset_to_next as u32;
                continue;
            }

            // No instruction due: movement + autofire, then tick.
            match self.movement_mode {
                1 => {
                    self.angle = normalize_angle(self.angle + self.angular_velocity);
                    self.speed += self.acceleration;
                    self.axis_speed = sincos_mul(self.angle, self.speed);
                }
                2 => {
                    self.move_timer -= 1;
                    let mut f = self.move_timer as f32 / self.move_start_time as f32;
                    if f >= 1.0 {
                        f = 1.0;
                    }
                    f = match self.ease_type {
                        0 => 1.0 - f,
                        1 => 1.0 - f * f,
                        2 => 1.0 - f * f * f * f,
                        3 => {
                            let g = 1.0 - f;
                            g * g
                        }
                        _ => {
                            let g = 1.0 - f;
                            g * g * g * g
                        }
                    };
                    self.axis_speed = [
                        f * self.move_interp[0] + self.move_interp_start[0] - self.pos[0],
                        f * self.move_interp[1] + self.move_interp_start[1] - self.pos[1],
                    ];
                    self.angle = self.axis_speed[1].atan2(self.axis_speed[0]);
                    if self.move_timer <= 0 {
                        self.movement_mode = 0;
                        self.pos[0] = self.move_interp_start[0] + self.move_interp[0];
                        self.pos[1] = self.move_interp_start[1] + self.move_interp[1];
                        self.axis_speed = [0.0, 0.0];
                    }
                }
                _ => {}
            }
            if self.life > 0 {
                if self.shoot_interval > 0 {
                    self.shoot_timer += 1;
                    if self.shoot_timer >= self.shoot_interval {
                        self.bullet_props.pos = [
                            self.pos[0] + self.shoot_offset[0],
                            self.pos[1] + self.shoot_offset[1],
                        ];
                        spawn_bullet_pattern(world, &self.bullet_props);
                        self.shoot_timer = 0;
                    }
                }
                // RunEcl chooses the pose after computing this tick's velocity,
                // including during SpawnEnemy's initial RunEcl. The following
                // EnemyManager bounds test therefore sees that initial sprite.
                self.update_anm_pose();
                // EXINSREPEAT's per-frame callback (EclManager.cpp:1080).
                if self.ctx.func_set_func >= 0 {
                    let idx = self.ctx.func_set_func;
                    self.exec_ex(idx, 0, world);
                }
            }
            self.ctx.time += 1;
            return;
        }
    }

    fn exec(&mut self, ecl: &Ecl, world: &mut World, instr: &Instr) -> Flow {
        match instr.opcode {
            0 => {}                       // NOP
            1 => return Flow::Kill,       // UNIMP: end of script
            2 => return self.jump(instr), // JUMP
            3 => {
                // JUMPDEC
                let v = self.get_i32(instr.arg_i32(2), world) - 1;
                self.set_var_bits(instr.arg_i32(2), v as u32);
                if v > 0 {
                    return self.jump(instr);
                }
            }
            4 | 5 => {
                // SET_INT and SET_FLOAT both copy raw variable/immediate bits.
                let bits = self.get_i32(instr.arg_i32(1), world) as u32;
                self.set_var_bits(instr.arg_i32(0), bits);
            }
            6 => {
                // SETINTRAND
                let range = self.get_i32(instr.arg_i32(1), world);
                let v = world.rng.u32_in_range(range.max(0) as u32) as i32;
                self.set_var_bits(instr.arg_i32(0), v as u32);
            }
            7 => {
                // SETINTRANDMIN
                let range = self.get_i32(instr.arg_i32(1), world);
                let min = self.get_i32(instr.arg_i32(2), world);
                let v = world.rng.u32_in_range(range.max(0) as u32) as i32 + min;
                self.set_var_bits(instr.arg_i32(0), v as u32);
            }
            8 => {
                // SETFLOATRAND
                let range = self.get_f32(instr.arg_f32(1), world);
                let v = world.rng.f32_in_range(range);
                self.set_var_bits(instr.arg_i32(0), v.to_bits());
            }
            9 => {
                // SETFLOATRANDMIN
                let range = self.get_f32(instr.arg_f32(1), world);
                let min = self.get_f32(instr.arg_f32(2), world);
                let v = world.rng.f32_in_range(range) + min;
                self.set_var_bits(instr.arg_i32(0), v.to_bits());
            }
            10 => self.set_var_bits(instr.arg_i32(0), self.pos[0].to_bits()),
            11 => self.set_var_bits(instr.arg_i32(0), self.pos[1].to_bits()),
            12 => self.set_var_bits(instr.arg_i32(0), self.pos[2].to_bits()),
            13..=17 | 20..=24 => self.alu(instr, world), // destination determines value type
            18 => {
                // MATHINC
                let v = self.get_i32(instr.arg_i32(0), world) + 1;
                self.set_var_bits(instr.arg_i32(0), v as u32);
            }
            19 => {
                // MATHDEC
                let v = self.get_i32(instr.arg_i32(0), world) - 1;
                self.set_var_bits(instr.arg_i32(0), v as u32);
            }
            25 => {
                // MATHATAN2: angle from point (arg1,arg2) to point (arg3,arg4).
                // Decomp result = atan2f(arg4 - arg2, arg3 - arg1)
                // (EnemyEclInstr.cpp:396, after the var_order shuffle).
                let x1 = self.get_f32(instr.arg_f32(1), world);
                let y1 = self.get_f32(instr.arg_f32(2), world);
                let x2 = self.get_f32(instr.arg_f32(3), world);
                let y2 = self.get_f32(instr.arg_f32(4), world);
                let v = f64::from(y2 - y1).atan2(f64::from(x2 - x1)) as f32;
                self.set_var_bits(instr.arg_i32(0), v.to_bits());
            }
            26 => {
                // MATHNORMANGLE
                let id = instr.arg_i32(0);
                let v = normalize_angle(f32::from_bits(self.get_i32(id, world) as u32));
                self.set_var_bits(id, v.to_bits());
            }
            27 => {
                let l = self.get_i32(instr.arg_i32(0), world);
                let r = self.get_i32(instr.arg_i32(1), world);
                self.ctx.cmp = if l == r {
                    0
                } else if l < r {
                    -1
                } else {
                    1
                };
            }
            28 => {
                let l = self.get_f32(instr.arg_f32(0), world);
                let r = self.get_f32(instr.arg_f32(1), world);
                self.ctx.cmp = if l == r {
                    0
                } else if l < r {
                    -1
                } else {
                    1
                };
            }
            29 => return self.cond_jump(instr, self.ctx.cmp < 0),
            30 => return self.cond_jump(instr, self.ctx.cmp <= 0),
            31 => return self.cond_jump(instr, self.ctx.cmp == 0),
            32 => return self.cond_jump(instr, self.ctx.cmp > 0),
            33 => return self.cond_jump(instr, self.ctx.cmp >= 0),
            34 => return self.cond_jump(instr, self.ctx.cmp != 0),
            35 => return self.call(ecl, instr),
            36 => {
                // RET
                if self.stack_depth > 0 {
                    self.stack_depth -= 1;
                }
                self.ctx = self.stack[self.stack_depth];
                return Flow::Jumped;
            }
            37..=42 => {
                // CALLLSS..CALLNEQ
                let lhs = self.get_i32(instr.arg_i32(3), world);
                let rhs = instr.arg_i32(4);
                let cond = match instr.opcode {
                    37 => lhs < rhs,
                    38 => lhs <= rhs,
                    39 => lhs == rhs,
                    40 => lhs > rhs,
                    41 => lhs >= rhs,
                    _ => lhs != rhs,
                };
                if cond {
                    return self.call(ecl, instr);
                }
            }
            43 => {
                // MOVEPOSITION (EclManager.cpp:316): set x,y,z then clamp.
                self.pos[0] = self.get_f32(instr.arg_f32(0), world);
                self.pos[1] = self.get_f32(instr.arg_f32(1), world);
                self.pos[2] = self.get_f32(instr.arg_f32(2), world);
                self.clamp();
            }
            44 => {
                // MOVEAXISVELOCITY
                self.axis_speed[0] = self.get_f32(instr.arg_f32(0), world);
                self.axis_speed[1] = self.get_f32(instr.arg_f32(1), world);
                self.movement_mode = 0;
            }
            45 => {
                // MOVEVELOCITY
                self.angle = self.get_f32(instr.arg_f32(0), world);
                self.speed = self.get_f32(instr.arg_f32(1), world);
                self.movement_mode = 1;
            }
            46 => {
                // MOVEANGULARVELOCITY
                self.angular_velocity = self.get_f32(instr.arg_f32(0), world);
                self.movement_mode = 1;
            }
            47 => {
                // MOVESPEED
                self.speed = self.get_f32(instr.arg_f32(0), world);
                self.movement_mode = 1;
            }
            48 => {
                // MOVEACCELERATION
                self.acceleration = self.get_f32(instr.arg_f32(0), world);
                self.movement_mode = 1;
            }
            49 => {
                // MOVERAND
                let a = instr.arg_f32(0);
                let b = instr.arg_f32(1);
                self.angle = world.rng.f32_in_range(b - a) + a;
            }
            50 => {
                // MOVERANDINBOUND
                let a = instr.arg_f32(0);
                let b = instr.arg_f32(1);
                self.angle = world.rng.f32_in_range(b - a) + a;
                let pi = std::f32::consts::PI;
                if self.pos[0] < self.lower_bound[0] + 96.0 {
                    if self.angle > pi / 2.0 {
                        self.angle = pi - self.angle;
                    } else if self.angle < -pi / 2.0 {
                        self.angle = -pi - self.angle;
                    }
                }
                if self.pos[0] > self.upper_bound[0] - 96.0 {
                    if self.angle < pi / 2.0 && self.angle >= 0.0 {
                        self.angle = pi - self.angle;
                    } else if self.angle > -pi / 2.0 && self.angle <= 0.0 {
                        self.angle = -pi - self.angle;
                    }
                }
                if self.pos[1] < self.lower_bound[1] + 48.0 && self.angle < 0.0 {
                    self.angle = -self.angle;
                }
                if self.pos[1] > self.upper_bound[1] - 48.0 && self.angle > 0.0 {
                    self.angle = -self.angle;
                }
            }
            51 => {
                // MOVEATPLAYER
                self.angle = self.angle_to_player(world) + instr.arg_f32(0);
                self.speed = self.get_f32(instr.arg_f32(1), world);
                self.movement_mode = 1;
            }
            52..=55 => {
                // MOVEDIRTIME*
                let duration = instr.arg_i32(0);
                let angle = self.get_f32(instr.arg_f32(1), world);
                let speed = instr.arg_f32(2);
                self.move_interp = [
                    angle.cos() * speed * duration as f32 / 2.0,
                    angle.sin() * speed * duration as f32 / 2.0,
                ];
                self.start_interp(duration);
                self.ease_type = (instr.opcode - 51) as u8;
            }
            56..=60 => {
                // MOVEPOSITIONTIME*
                let duration = instr.arg_i32(0);
                let x = self.get_f32(instr.arg_f32(1), world);
                let y = self.get_f32(instr.arg_f32(2), world);
                self.move_interp = [x - self.pos[0], y - self.pos[1]];
                self.start_interp(duration);
                self.ease_type = (instr.opcode - 56) as u8;
                self.axis_speed = [0.0, 0.0];
            }
            61..=64 => {
                // MOVETIME*
                let duration = instr.arg_i32(0);
                self.move_interp = [
                    self.angle.cos() * self.speed * duration as f32 / 2.0,
                    self.angle.sin() * self.speed * duration as f32 / 2.0,
                ];
                self.start_interp(duration);
                self.ease_type = (instr.opcode - 60) as u8;
            }
            65 => {
                // MOVEBOUNDSSET
                self.lower_bound = [instr.arg_f32(0), instr.arg_f32(1)];
                self.upper_bound = [instr.arg_f32(2), instr.arg_f32(3)];
                self.clamp_pos = true;
            }
            66 => self.clamp_pos = false,
            67..=75 => {
                // BULLET*
                let sprite = instr.arg_i16(0);
                let color = instr.arg_i16(2) as i32;
                let aim_mode = (instr.opcode - 67) as u16;
                let count1 = instr.arg_i32(1);
                let count2 = instr.arg_i32(2);
                let speed1 = instr.arg_f32(3);
                let speed2 = instr.arg_f32(4);
                let angle1 = instr.arg_f32(5);
                let angle2 = instr.arg_f32(6);
                let flags = instr.arg_i32(7);
                let c1 = self.get_i32(count1, world)
                    + rank_scale_i32(self.rank_amount1_low, self.rank_amount1_high, world.rank);
                let c2 = self.get_i32(count2, world)
                    + rank_scale_i32(self.rank_amount2_low, self.rank_amount2_high, world.rank);
                let mut s1 = self.get_f32(speed1, world);
                if s1 != 0.0 {
                    s1 += rank_scale_f32(
                        self.rank_speed_low,
                        self.rank_speed_high,
                        world.rank as f32,
                    );
                    if s1 < 0.3 {
                        s1 = 0.3;
                    }
                }
                let mut s2 = self.get_f32(speed2, world)
                    + rank_scale_f32(self.rank_speed_low, self.rank_speed_high, world.rank as f32)
                        / 2.0;
                if s2 < 0.3 {
                    s2 = 0.3;
                }
                let a1 = normalize_angle(self.get_f32(angle1, world));
                let a2 = self.get_f32(angle2, world);
                let sprite_offset = self.get_i32(color, world);
                let p = &mut self.bullet_props;
                p.sprite = sprite;
                p.aim_mode = aim_mode;
                p.count1 = c1.max(1) as i16;
                p.count2 = c2.max(1) as i16;
                p.speed1 = s1;
                p.speed2 = s2;
                p.angle1 = a1;
                p.angle2 = a2;
                p.flags = flags as u32;
                p.sprite_offset = sprite_offset;
                let pos = [
                    self.pos[0] + self.shoot_offset[0],
                    self.pos[1] + self.shoot_offset[1],
                ];
                self.bullet_props.pos = pos;
                if !self.shooting_disabled {
                    let props = std::mem::take(&mut self.bullet_props);
                    spawn_bullet_pattern(world, &props);
                    self.bullet_props = props;
                }
            }
            76 => {
                // SHOOTINTERVAL (EclManager.cpp:429): set to the arg, THEN add the
                // rank scaling computed from that new interval (not the old one).
                self.shoot_interval = instr.arg_i32(0);
                self.shoot_interval += self.shoot_interval_rank(world.rank);
                self.shoot_timer = 0;
            }
            77 => {
                // SHOOTINTERVALDELAYED (EclManager.cpp:434): same, plus a random
                // initial timer in [0, interval).
                self.shoot_interval = instr.arg_i32(0);
                self.shoot_interval += self.shoot_interval_rank(world.rank);
                if self.shoot_interval != 0 {
                    self.shoot_timer = world.rng.u32_in_range(self.shoot_interval as u32) as i32;
                }
            }
            78 => self.shooting_disabled = true,
            79 => self.shooting_disabled = false,
            80 => {
                // SHOOTNOW
                self.bullet_props.pos = [
                    self.pos[0] + self.shoot_offset[0],
                    self.pos[1] + self.shoot_offset[1],
                ];
                let props = std::mem::take(&mut self.bullet_props);
                spawn_bullet_pattern(world, &props);
                self.bullet_props = props;
            }
            81 => {
                // SHOOTOFFSET
                self.shoot_offset[0] = self.get_f32(instr.arg_f32(0), world);
                self.shoot_offset[1] = self.get_f32(instr.arg_f32(1), world);
            }
            82 => {
                // BULLETEFFECTS: ex parameters consumed by the next shots.
                let ints = [
                    self.get_i32(instr.arg_i32(0), world),
                    self.get_i32(instr.arg_i32(1), world),
                    self.get_i32(instr.arg_i32(2), world),
                    self.get_i32(instr.arg_i32(3), world),
                ];
                let floats = [
                    self.get_f32(instr.arg_f32(4), world),
                    self.get_f32(instr.arg_f32(5), world),
                    self.get_f32(instr.arg_f32(6), world),
                    self.get_f32(instr.arg_f32(7), world),
                ];
                self.bullet_props.ex_ints = ints;
                self.bullet_props.ex_floats = floats;
            }
            83 => {
                let points = world.turn_bullets_into_points();
                world.events.push(WorldEvent::BulletCancel(points));
            }
            84 => {
                // BULLETSOUND
                let sfx = instr.arg_i32(0);
                if sfx >= 0 {
                    self.bullet_props.sfx = sfx;
                    self.bullet_props.flags |= 0x200;
                } else {
                    self.bullet_props.flags &= !0x200;
                }
            }
            85 | 86 => {
                // LASERCREATE / LASERCREATEAIMED
                let origin = [
                    self.pos[0] + self.shoot_offset[0],
                    self.pos[1] + self.shoot_offset[1],
                ];
                let mut angle = self.get_f32(instr.arg_f32(1), world);
                if instr.opcode == 86 {
                    angle += player_angle_from(world.player_pos, origin);
                }
                let laser = Laser {
                    in_use: true,
                    pos: origin,
                    angle,
                    speed: self.get_f32(instr.arg_f32(2), world),
                    start_offset: self.get_f32(instr.arg_f32(3), world),
                    end_offset: self.get_f32(instr.arg_f32(4), world),
                    start_length: self.get_f32(instr.arg_f32(5), world),
                    width: instr.arg_f32(6),
                    start_time: instr.arg_i32(7),
                    duration: instr.arg_i32(8),
                    despawn_duration: instr.arg_i32(9),
                    hitbox_start: instr.arg_i32(10),
                    hitbox_end: instr.arg_i32(11),
                    flags: instr.arg_i32(12) as u16,
                    script: 9 + instr.arg_i16(0).max(0) as u32,
                    draw_width: instr.arg_f32(6),
                    draw_alpha: 1.0,
                    state: if instr.arg_i32(7) == 0 { 1 } else { 0 },
                    timer: 0,
                    color: instr.arg_i16(2) as i32,
                };
                let idx = world.alloc_laser(laser);
                self.lasers[self.laser_store & 31] = Some(idx);
            }
            87 => self.laser_store = self.get_i32(instr.arg_i32(0), world).max(0) as usize,
            88 => {
                // LASERROTATE
                let slot = instr.arg_i32(0).clamp(0, 31) as usize;
                let delta = self.get_f32(instr.arg_f32(1), world);
                if let Some(idx) = self.lasers[slot]
                    && let Some(l) = world.lasers.get_mut(idx)
                {
                    l.angle += delta;
                }
            }
            89 => {
                // LASERROTATEFROMPLAYER
                let slot = instr.arg_i32(0).clamp(0, 31) as usize;
                let delta = self.get_f32(instr.arg_f32(1), world);
                if let Some(idx) = self.lasers[slot] {
                    let player = world.player_pos;
                    if let Some(l) = world.lasers.get_mut(idx) {
                        l.angle = player_angle_from(player, l.pos) + delta;
                    }
                }
            }
            90 => {
                // LASEROFFSET
                let slot = instr.arg_i32(0).clamp(0, 31) as usize;
                let dx = instr.arg_f32(1);
                let dy = instr.arg_f32(2);
                if let Some(idx) = self.lasers[slot]
                    && let Some(l) = world.lasers.get_mut(idx)
                {
                    l.pos = [self.pos[0] + dx, self.pos[1] + dy];
                }
            }
            91 => {
                // LASERTEST: cmp = 0 while the laser lives.
                let slot = instr.arg_i32(0).clamp(0, 31) as usize;
                let alive = self.lasers[slot]
                    .and_then(|idx| world.lasers.get(idx))
                    .map(|l| l.in_use)
                    .unwrap_or(false);
                self.ctx.cmp = if alive { 0 } else { 1 };
            }
            92 => {
                // LASERCANCEL
                let slot = instr.arg_i32(0).clamp(0, 31) as usize;
                if let Some(idx) = self.lasers[slot]
                    && let Some(l) = world.lasers.get_mut(idx)
                    && l.in_use
                    && l.state < 2
                {
                    l.state = 2;
                    l.timer = 0;
                }
            }
            134 => self.lasers = [None; 32], // LASERCLEARALL
            93 => {
                // SPELLCARDSTART (EclRawInstrSpellcardStartArgs): portrait sprite
                // at byte 0, id at byte 2, Shift-JIS name from byte 4.
                // EclManager.cpp:752 leaves effectArray alive. Its bubbles are
                // owned by the enemy and can outlive a spell declaration.
                let sprite = instr.arg_i16(0) as i32;
                let id = instr.arg_i16(2) as i32;
                let name = instr
                    .args
                    .get(4..)
                    .map(|b| {
                        let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
                        b[..end].to_vec()
                    })
                    .unwrap_or_default();
                world
                    .events
                    .push(WorldEvent::SpellcardStart(id, sprite, name));
                world.spellcard_active = 1;
                let points = world.turn_bullets_into_points();
                world.events.push(WorldEvent::BulletCancel(points));
                self.rank_speed_low = -0.5;
                self.rank_speed_high = 0.5;
                self.rank_amount1_low = 0;
                self.rank_amount1_high = 0;
                self.rank_amount2_low = 0;
                self.rank_amount2_high = 0;
            }
            94 => {
                // SPELLCARDEND also leaves the enemy's effectArray intact.
                if world.spellcard_active != 0 {
                    let (points, bonus) = if world.spellcard_active == 1 {
                        world.despawn_bullets_for_bonus(true)
                    } else {
                        (Vec::new(), 0)
                    };
                    world
                        .events
                        .push(WorldEvent::SpellcardEnd { points, bonus });
                    world.spellcard_active = 0;
                }
            }
            95 => {
                // ENEMYCREATE
                let request = SpawnReq {
                    sub: instr.arg_i32(0),
                    pos: [
                        self.get_f32(instr.arg_f32(1), world),
                        self.get_f32(instr.arg_f32(2), world),
                        self.get_f32(instr.arg_f32(3), world),
                    ],
                    life: instr.arg_i16(16),
                    item: instr.arg_i16(18),
                    score: instr.arg_i32(5),
                    mirror: false,
                };
                // SpawnEnemy runs the child ECL immediately; deferring it until
                // the parent returns changes the shared random stream.
                if let Some(child) = Enemy::spawn(ecl, world, &request) {
                    world.pending_spawns.push(child);
                }
            }
            96 => world.kill_trash = true, // ENEMYKILLALL
            97 => {
                self.anm_script = instr.arg_i32(0);
                self.anm_dirty = true;
            }
            98 => {
                // ANMSETPOSES
                self.anm_poses = [
                    instr.arg_i16(0),
                    instr.arg_i16(2),
                    instr.arg_i16(4),
                    instr.arg_i16(6),
                    instr.arg_i16(8),
                ];
                self.anm_pose_state = 0xff;
            }
            99 => {
                // ANMSETSLOT: assign a sub-vm (enemy->vms[vmIdx]) an anm script.
                let vm = instr.arg_i32(0);
                let script = instr.arg_i32(1);
                if (0..8).contains(&vm) {
                    self.sub_scripts[vm as usize] = script;
                    self.sub_dirty[vm as usize] = true;
                }
            }
            100 => {
                // ANMSETDEATH (EclManager.cpp:422): 3 i8 death-effect anm indices.
                self.death_anm1 = instr.args.first().copied().unwrap_or(0);
                self.death_anm2 = instr.args.get(1).copied().unwrap_or(0);
            }
            101 => {
                // BOSSSET
                let v = instr.arg_i32(0);
                if v >= 0 {
                    self.is_boss = true;
                    self.boss_id = v as u8;
                    world.boss_present = true;
                    world.events.push(WorldEvent::BossSet(true));
                } else {
                    self.is_boss = false;
                    world.boss_present = false;
                    world.events.push(WorldEvent::BossSet(false));
                }
            }
            102 => {
                // SPELLCARDEFFECT (EclManager.cpp:552): spawn an effect-13 bubble
                // at the enemy, tinted by g_EffectsColor[colorId], store its swirl
                // axis pos2 + the shared target radius, and track it in effectArray.
                let color_id = instr.arg_i32(0).rem_euclid(28) as usize;
                let pos2 = [instr.arg_f32(1), instr.arg_f32(2), instr.arg_f32(3)];
                self.effect_distance = instr.arg_f32(4);
                let c = G_EFFECTS_COLOR[color_id];
                let color = [
                    ((c >> 16) & 0xff) as f32 / 255.0,
                    ((c >> 8) & 0xff) as f32 / 255.0,
                    (c & 0xff) as f32 / 255.0,
                    ((c >> 24) & 0xff) as f32 / 255.0,
                ];
                if let Some(slot) = world.spawn_spell_bubble(self.pos, pos2, color) {
                    self.effect_slots.push(slot);
                }
            }
            103 => {
                self.hitbox = [instr.arg_f32(0), instr.arg_f32(1)];
            }
            104 => self.collidable = instr.arg_i32(0) != 0,
            105 => self.damageable = instr.arg_i32(0) != 0,
            106 => world.events.push(WorldEvent::Sfx(instr.arg_i32(0))),
            107 => self.death_mode = instr.arg_i32(0) as u8,
            108 => self.death_callback = instr.arg_i32(0),
            109 => {
                // ENEMYINTERRUPTSET
                let sub = instr.arg_i32(0);
                let id = instr.arg_i32(1);
                self.interrupts[(id & 7) as usize] = sub;
            }
            110 => {
                // ENEMYINTERRUPT: the loop-top handler advances past this
                // instruction itself (single advance, like the original).
                self.run_interrupt = instr.arg_i32(0);
                return Flow::Jumped;
            }
            111 => {
                self.life = instr.arg_i32(0);
                self.max_life = self.life;
            }
            112 => self.boss_timer = instr.arg_i32(0),
            113 => self.life_cb_threshold = instr.arg_i32(0),
            114 => self.life_cb_sub = instr.arg_i32(0),
            115 => {
                self.timer_cb_threshold = instr.arg_i32(0);
                self.boss_timer = 0;
            }
            116 => self.timer_cb_sub = instr.arg_i32(0),
            117 => self.interactable = instr.arg_i32(0) != 0,
            118 => {
                // EFFECTPARTICLE (EclManager.cpp): spawn numParticles effects of
                // effectId at the enemy. Their RNG draws (RandomSplash etc.) land
                // in update_effects, like death effects. (#23/#30)
                let effect_id = instr.arg_i32(0);
                let count = instr.arg_i32(1);
                let c = instr.arg_i32(2) as u32; // ZunColor ARGB particleColor
                let color = [
                    ((c >> 16) & 0xff) as f32 / 255.0,
                    ((c >> 8) & 0xff) as f32 / 255.0,
                    (c & 0xff) as f32 / 255.0,
                    ((c >> 24) & 0xff) as f32 / 255.0,
                ];
                world.spawn_particles(effect_id, self.pos, count, color);
            }
            119 => {
                // DROPITEMS (exact port: big power first unless maxed)
                let n = instr.arg_i32(0);
                for i in 0..n {
                    let pos = [
                        self.pos[0] + world.rng.f32_in_range(144.0) - 72.0,
                        self.pos[1] + world.rng.f32_in_range(144.0) - 72.0,
                    ];
                    let kind = if world.power < 128 {
                        if i == 0 { 2 } else { 0 } // big power / small power
                    } else {
                        1 // point
                    };
                    world.events.push(WorldEvent::DropItem(pos, kind));
                }
            }
            120 => self.rotate_anm = instr.arg_i32(0) != 0, // ANMFLAGROTATION
            121 => {
                // EXINSCALL: run g_EclExInsn[arg0] once, with arg1 as i32Param.
                self.exec_ex(instr.arg_i32(0), instr.arg_i32(1), world);
            }
            122 => {
                // EXINSREPEAT: arg0 >= 0 arms the per-frame callback; < 0 clears.
                let idx = instr.arg_i32(0);
                self.ctx.func_set_func = if idx >= 0 { idx } else { -1 };
            }
            123 => {
                // TIMESET
                let v = self.get_i32(instr.arg_i32(0), world);
                self.ctx.time += v;
            }
            124 => {
                // DROPITEMID
                world.events.push(WorldEvent::DropItem(
                    [self.pos[0], self.pos[1]],
                    instr.arg_i32(0),
                ));
            }
            125 => world.events.push(WorldEvent::Unpause), // STDUNPAUSE
            126 => {
                self.spell_count = instr.arg_i32(0); // BOSSSETLIFECOUNT (global Gui value)
                world
                    .events
                    .push(WorldEvent::BossLifeCount(self.spell_count));
            }
            127 => {} // DEBUGWATCH
            // ANMINTERRUPTMAIN: queue an interrupt for the primary anm runner.
            128 => self.pending_interrupt = Some(instr.arg_i32(0)),
            129 => {
                let slot = instr.arg_i32(0);
                if (0..8).contains(&slot) {
                    self.sub_interrupts[slot as usize] = Some(instr.arg_i32(1));
                }
            }
            130 => self.disable_call_stack = instr.arg_i32(0) != 0,
            131 => {
                // BULLETRANKINFLUENCE
                self.rank_speed_low = instr.arg_f32(0);
                self.rank_speed_high = instr.arg_f32(1);
                self.rank_amount1_low = instr.arg_i32(2);
                self.rank_amount1_high = instr.arg_i32(3);
                self.rank_amount2_low = instr.arg_i32(4);
                self.rank_amount2_high = instr.arg_i32(5);
            }
            132 => self.invisible = instr.arg_i32(0) != 0,
            133 => {
                // BOSSTIMERCLEAR
                self.timer_cb_sub = self.death_callback;
                self.boss_timer = 0;
            }
            135 => self.timeout_spell = instr.arg_i32(0) != 0,
            op => {
                #[cfg(test)]
                panic!("unhandled ECL opcode {op}");
                #[cfg(not(test))]
                if std::env::var_os("TH06_TRACE_OP").is_some() {
                    eprintln!("unhandled ECL op {op}");
                }
            }
        }
        Flow::Next
    }

    /// Dispatch a per-boss ex-instruction (`g_EclExInsn[idx]`, EnemyEclInstr.cpp).
    /// `param` is `instr->args.exInstr.i32Param`; for EXINSREPEAT's per-frame
    /// calls the decomp passes a NULL instr, so the repeat-style handlers ignore
    /// it and `param` is 0.
    fn exec_ex(&mut self, idx: i32, param: i32, world: &mut World) {
        use std::f32::consts::{PI, TAU};
        if std::env::var_os("TH06_TRACE_EX").is_some() {
            eprintln!("EXINS idx={idx} param={param}");
        }
        match idx {
            0 => {
                // ExInsCirnoRainbowBallJank (Perfect Freeze): param 0 freezes
                // every live bullet, param 1 releases them with a small random
                // acceleration over 220 frames (ex flag 0x10).
                world.spawn_particles(12, self.pos, 1, [1.0; 4]);
                for b in world.bullets.iter_mut() {
                    if b.despawn_timer > 0 {
                        continue;
                    }
                    b.set_normal_color(0);
                    match param {
                        0 => {
                            b.speed = 0.0;
                            b.velocity = [0.0; 2];
                        }
                        1 => {
                            b.ex_flags |= 0x10;
                            b.ex_int0 = 220;
                            b.timer = 0;
                            let a = world.rng.f32_zero_to_one() * TAU - PI;
                            b.ex_accel = sincos_mul(a, 0.01);
                        }
                        _ => {}
                    }
                }
            }
            1 => {
                // ExInsShootAtRandomArea: fire the configured pattern from a
                // random point in a box (w=param, h=0.75w) around the enemy.
                let s = param as f32;
                let bx = world.rng.f32_in_range(s) + self.pos[0] - s / 2.0;
                let sy = s * 0.75;
                let by = world.rng.f32_in_range(sy) + self.pos[1] - sy / 2.0;
                self.bullet_props.pos = [bx, by];
                let props = std::mem::take(&mut self.bullet_props);
                spawn_bullet_pattern(world, &props);
                self.bullet_props = props;
            }
            2 => {
                // ExInsShootStarPattern: a moving five-pointed star, fired in
                // bursts. Driven by EXINSREPEAT (var2 = frame counter, var3 =
                // total frames, float3 = star radius).
                let var2 = self.ctx.ivars[2];
                let var3 = self.ctx.ivars[3];
                if var2 >= var3 {
                    self.ctx.func_set_func = -1;
                    return;
                }
                let step = 4.0 * PI / 5.0;
                if var2 == 0 {
                    self.star_enemy_pos = [self.pos[0], self.pos[1]];
                    self.star_player_pos = world.player_pos;
                    self.star_angle[0] = world.rng.f32_zero_to_one() * TAU - PI;
                    self.star_angle[1] = normalize_angle(self.star_angle[0] + step);
                }
                if var2 % 30 == 0 {
                    self.star_angle[0] = self.star_angle[1];
                    for i in 1..6 {
                        self.star_angle[i] = normalize_angle(self.star_angle[i - 1] + step);
                    }
                }
                if var2 % 6 == 0 {
                    let pattern_pos = var2 as f32 / var3 as f32;
                    let td0 = pattern_pos * 0.1;
                    let base = [
                        (self.star_player_pos[0] - self.star_enemy_pos[0]) * td0
                            + self.star_enemy_pos[0],
                        (self.star_player_pos[1] - self.star_enemy_pos[1]) * td0
                            + self.star_enemy_pos[1],
                    ];
                    let pp = pattern_pos + 0.5;
                    self.bullet_props.angle1 = (PI / 3.0) * pp;
                    let r = self.ctx.fvars[3];
                    for i in 0..5 {
                        let td = (var2 % 30) as f32 / 30.0;
                        let a0 = self.star_angle[i];
                        let a1 = self.star_angle[i + 1];
                        let t0 = sincos_mul(a0, r);
                        let t1 = sincos_mul(a1, r);
                        let tt = [(t1[0] - t0[0]) * td + t0[0], (t1[1] - t0[1]) * td + t0[1]];
                        self.bullet_props.pos = [base[0] + tt[0], base[1] + tt[1]];
                        let backup = self.bullet_props.speed1;
                        self.bullet_props.speed1 +=
                            world.rng.f32_in_range(self.bullet_props.speed2);
                        let props = std::mem::take(&mut self.bullet_props);
                        spawn_bullet_pattern(world, &props);
                        self.bullet_props = props;
                        self.bullet_props.speed1 = backup;
                        self.bullet_props.angle1 -= (PI / 6.0) * pp;
                    }
                    world.events.push(WorldEvent::Sfx(22)); // SOUND_16
                }
                self.ctx.ivars[2] += 1;
            }
            3 => {
                // ExInsPatchouliShottypeSetVars: pick the boss's pattern vars by
                // the player's character + shot type.
                const T: [[[i32; 3]; 2]; 2] = [[[0, 3, 1], [2, 3, 4]], [[1, 4, 0], [4, 2, 3]]];
                let v = T[world.character.min(1) as usize][world.shot_type.min(1) as usize];
                self.ctx.ivars[1] = v[0]; // var1
                self.ctx.ivars[2] = v[1]; // var2
                self.ctx.ivars[3] = v[2]; // var3
            }
            4 => {
                // ExInsStage56Func4: param < 2 toggles Sakuya's time-stop
                // (isTimeStopped = u8Param). param >= 2 is the "misdirection"
                // redirect: re-aim a capped number of big (>=30px) bullets and
                // swap them to colour offset 5.
                if param < 2 {
                    world.spawn_particles(12, self.pos, 1, [1.0; 4]);
                    world.time_stopped = (param & 0xff) != 0;
                } else {
                    let normal = world.difficulty <= 1; // <= NORMAL
                    let mut left: i32 = if normal { 14 } else { 52 };
                    let player = world.player_pos;
                    let rng = &mut world.rng;
                    for b in world.bullets.iter_mut() {
                        if b.despawn_timer > 0 {
                            continue;
                        }
                        if b.height >= 30.0 && b.sprite_offset != 5 && rng.u16().is_multiple_of(4) {
                            b.set_normal_color(5);
                            let dx = b.pos[0] - player[0];
                            let dy = b.pos[1] - player[1];
                            b.angle = if (dx * dx + dy * dy).sqrt() > 128.0 {
                                if normal {
                                    rng.f32_zero_to_one() * (3.0 * PI / 4.0) + PI / 4.0
                                } else {
                                    rng.f32_zero_to_one() * TAU
                                }
                            } else {
                                // AngleFromPlayer (player -> bullet) + π/2 + rand(2π).
                                player_angle_from(b.pos, player) + PI / 2.0 + rng.f32_in_range(TAU)
                            };
                            b.velocity = sincos_mul(b.angle, b.speed);
                            left -= 1;
                            if left == 0 {
                                break;
                            }
                        }
                    }
                }
                self.ctx.ivars[2] = 0; // var2 = 0
            }
            5 => {
                // ExInsStage5Func5: Sakuya's knife fan (sprite 8 = dagger), fired
                // every 9 frames in a player-relative rotated arc. Driven by
                // EXINSREPEAT (var2 = frame counter). EnemyEclInstr.cpp:655.
                let var2 = self.ctx.ivars[2];
                if var2 % 9 == 0 {
                    let pp = var2 / 9;
                    let normal = world.difficulty <= 1;
                    let mut props = BulletProps {
                        sprite: 8,
                        aim_mode: 0,
                        count1: if normal { 1 } else { 3 },
                        count2: 1,
                        angle2: PI / 6.0,
                        flags: 0,
                        speed1: 2.0,
                        sprite_offset: 3,
                        ..Default::default()
                    };
                    let mut mo = [
                        world.player_pos[0] - self.pos[0],
                        world.player_pos[1] - self.pos[1],
                    ];
                    let dz = world.player_z - self.pos[2];
                    let len = (mo[0] * mo[0] + mo[1] * mo[1] + dz * dz).sqrt();
                    let mut mi = if len > 0.0 {
                        // D3DXVec3Normalize (EXE0x43f13f) computes one
                        // rounded reciprocal, then multiplies each component.
                        let inverse_length = 1.0 / len;
                        [mo[0] * inverse_length, mo[1] * inverse_length]
                    } else {
                        [0.0, 0.0]
                    };
                    let mat_in_seed = if pp & 1 != 0 { -256.0 } else { 256.0 };
                    mi = [mi[0] * mat_in_seed, mi[1] * mat_in_seed];
                    let mat_out_seed = 0.5 - pp as f32 * 0.5 / 9.0;
                    mo = [mo[0] * mat_out_seed, mo[1] * mat_out_seed];
                    let bp_offset = [mo[0] + mi[0], mo[1] + mi[1]];
                    mi = [-mi[0], -mi[1]];
                    // Rotate by +pi/4, then by -pi/18 each of the 9 bullets.
                    let (c, s) = ((PI / 4.0).cos(), (PI / 4.0).sin());
                    mi = [mi[0] * c + mi[1] * s, -mi[0] * s + mi[1] * c];
                    let (co, so) = ((-PI / 18.0).cos(), (-PI / 18.0).sin());
                    let mut bullet_angle = -PI / 4.0;
                    for _ in 0..9 {
                        mi = [mi[0] * co + mi[1] * so, -mi[0] * so + mi[1] * co];
                        props.pos = [
                            mi[0] + self.pos[0] + bp_offset[0],
                            mi[1] + self.pos[1] + bp_offset[1],
                        ];
                        props.speed1 = 2.0;
                        if (pp & 1) != 0 && normal {
                            props.angle1 = bullet_angle;
                        }
                        props.sprite_offset = 3;
                        spawn_bullet_pattern(world, &props);
                        bullet_angle += PI / 18.0;
                    }
                    world.events.push(WorldEvent::Sfx(7)); // SOUND_7
                }
                self.ctx.ivars[2] += 1;
            }
            13 => {
                // ExInsStageXFunc13: a radial burst of `param` arms from the
                // playfield centre every 6 frames (float1 angle offset, float2
                // base angle, float3 radius; var3 = frame counter).
                let n = param;
                if n > 0 && self.ctx.ivars[3] % 6 == 0 {
                    let mut base_angle = self.ctx.fvars[2];
                    let r = self.ctx.fvars[3];
                    let off = self.ctx.fvars[1];
                    for _ in 0..n {
                        let radial = sincos_mul(base_angle, r);
                        self.bullet_props.pos = [radial[0] + 192.0, radial[1] + 224.0];
                        self.bullet_props.angle1 = base_angle + off;
                        let props = std::mem::take(&mut self.bullet_props);
                        spawn_bullet_pattern(world, &props);
                        self.bullet_props = props;
                        base_angle += TAU / n as f32;
                    }
                }
                self.ctx.ivars[3] += 1;
            }
            16 => {
                // ExInsFlandreFinalContextUpdate: derive pattern vars from
                // remaining life (clamped to 0 past the 7200-frame rage timer).
                let remaining = if self.boss_timer >= 7200 {
                    0
                } else {
                    self.life
                };
                if param == 0 {
                    self.ctx.fvars[3] = 2.0 - (remaining as f32) / 6000.0; // float3
                    self.ctx.ivars[5] = remaining * 240 / 6000 + 40; // var5
                } else {
                    let m = 320.0 - (remaining as f32) * 160.0 / 6000.0;
                    self.ctx.fvars[2] = world.rng.f32_in_range(m) + (192.0 - m / 2.0); // float2
                    let m = 128.0 - (remaining as f32) * 64.0 / 6000.0;
                    self.ctx.fvars[3] = world.rng.f32_in_range(m) + (96.0 - m / 2.0); // float3
                }
            }
            6 => self.ex_bat_wing(world),
            7 => {
                // ExInsStage6Func7 (EnemyEclInstr.cpp:795) — Remilia's laser-star
                // card: two counter-rotating rings of 8 arms, each arm laying 3
                // segments outward from a 32px-offset start point. param == 0
                // fires lasers (sprite 1); otherwise the enemy's current
                // bulletProps from each segment point. One f32 draw up front.
                let attack_type = param;
                let rand_mod = world.rng.f32_in_range(TAU);
                let normal = world.difficulty <= 1;
                for outer in 0..2 {
                    let (mut laser_angle, angle_diff) = if outer == 0 {
                        (-PI + rand_mod, PI / 4.0)
                    } else {
                        (7.0 * -PI / 8.0 + rand_mod, -PI / 4.0)
                    };
                    let mut pv = [[0.0f32; 2]; 8];
                    let length0 = 32.0f32;
                    for slot in pv.iter_mut() {
                        *slot = [
                            self.pos[0] + laser_angle.cos() * length0,
                            self.pos[1] + laser_angle.sin() * length0,
                        ];
                        laser_angle += PI / 4.0;
                    }
                    laser_angle = if outer == 0 {
                        -PI + rand_mod
                    } else {
                        7.0 * -PI / 8.0 + rand_mod
                    };
                    for inner in 0..3 {
                        let length = if inner < 2 { 112.0 } else { 480.0 };
                        for p in pv.iter_mut() {
                            if attack_type == 0 {
                                let sprite_offset = if normal { 2 } else { 8 };
                                let end = if normal { length } else { 440.0 };
                                let width = if normal { 28.0 } else { 20.0 };
                                world.alloc_laser(Laser {
                                    in_use: true,
                                    pos: *p,
                                    angle: laser_angle,
                                    speed: 0.0,
                                    start_offset: 0.0,
                                    end_offset: end,
                                    start_length: end,
                                    width,
                                    start_time: inner * 16 + 60,
                                    duration: 90 - inner * 16,
                                    despawn_duration: 16,
                                    hitbox_start: 50,
                                    hitbox_end: 16,
                                    flags: 2,
                                    script: 10,
                                    draw_width: width,
                                    draw_alpha: 1.0,
                                    state: 0, // start_time is always >= 60, so warmup
                                    timer: 0,
                                    color: sprite_offset,
                                });
                            } else {
                                self.bullet_props.pos = *p;
                                spawn_bullet_pattern(world, &self.bullet_props);
                            }
                            p[0] += laser_angle.cos() * length;
                            p[1] += laser_angle.sin() * length;
                            laser_angle += PI / 4.0;
                        }
                        laser_angle += angle_diff - TAU;
                    }
                }
            }
            8 => {
                // ExInsStage6Func8 (EnemyEclInstr.cpp:912) — Remilia "Scarlet
                // Meister"/"Red Magic" tier: for every live big (>=30px) bullet,
                // spawn one small bullet (sprite 3, colour offset 1) at its
                // position aimed in a random direction. One f32 draw per big
                // bullet; var3 records the count.
                let positions: Vec<[f32; 2]> = world
                    .bullets
                    .iter()
                    .filter(|b| b.despawn_timer == 0 && b.height >= 30.0)
                    .map(|b| b.pos)
                    .collect();
                let mut changed = 0;
                for pos in positions {
                    let props = BulletProps {
                        pos,
                        sprite: 3,
                        sprite_offset: 1,
                        angle1: world.rng.f32_in_range(TAU) - PI,
                        speed1: 0.0,
                        count1: 1,
                        count2: 1,
                        flags: 8,
                        aim_mode: 1,
                        ..Default::default()
                    };
                    spawn_bullet_pattern(world, &props);
                    changed += 1;
                }
                self.ctx.ivars[3] = changed;
            }
            9 => {
                // ExInsStage6Func9 (EnemyEclInstr.cpp:951) — Remilia "Scarlet
                // Gensokyo": every live small (<30px) stationary bullet is
                // turned into a slow accelerating one (colour offset 2, flag
                // 0x10 acceleration over 120 frames). The acceleration angle is
                // its distance from the boss scaled by pi/256, plus a single
                // shared random offset. One f32 draw total + a particle burst.
                let rand_mod = world.rng.f32_in_range(TAU) - PI;
                world.spawn_particles(12, self.pos, 1, [1.0; 4]);
                let (ex, ey) = (self.pos[0], self.pos[1]);
                for b in world.bullets.iter_mut() {
                    if b.despawn_timer != 0 || b.height >= 30.0 || b.speed != 0.0 {
                        continue;
                    }
                    b.ex_flags |= 0x10;
                    b.set_normal_color(2);
                    b.speed = 0.01;
                    b.timer = 0;
                    b.ex_int0 = 120;
                    let d2 = (ex - b.pos[0]) * (ex - b.pos[0]) + (ey - b.pos[1]) * (ey - b.pos[1]);
                    let dist = if d2 > 0.1 { d2.sqrt() } else { 0.0 };
                    let a = dist * PI / 256.0 + rand_mod;
                    b.ex_accel = sincos_mul(a, 0.01);
                }
            }
            11 => {
                // ExInsStage6Func11 (EnemyEclInstr.cpp:999) — like Func9 but each
                // converted bullet gets its own random acceleration angle (one
                // f32 draw per bullet), plus one discarded draw up front and a
                // particle burst.
                let _ = world.rng.f32_in_range(TAU) - PI;
                world.spawn_particles(12, self.pos, 1, [1.0; 4]);
                for b in world.bullets.iter_mut() {
                    if b.despawn_timer != 0 || b.height >= 30.0 || b.speed != 0.0 {
                        continue;
                    }
                    b.ex_flags |= 0x10;
                    b.set_normal_color(2);
                    b.speed = 0.01;
                    b.timer = 0;
                    b.ex_int0 = 120;
                    let a = world.rng.f32_in_range(TAU) - PI;
                    b.ex_accel = sincos_mul(a, 0.01);
                }
            }
            10 => self.ex_bat_transform(world),
            15 => {
                // ExInsStageXFunc15 (EnemyEclInstr.cpp:1145) — for every live big
                // (>=30px) bullet, every nearby (<64px) small (<30px) stationary
                // bullet is bent away from the boss and sent accelerating: its new
                // angle reflects its boss-relative angle about the big bullet's
                // boss-relative angle (factor 2.2). Then the bat transform runs.
                let mut total = 0;
                let big: Vec<[f32; 2]> = world
                    .bullets
                    .iter()
                    .filter(|b| b.despawn_timer == 0 && b.height >= 30.0)
                    .map(|b| b.pos)
                    .collect();
                let (ex, ey) = (self.pos[0], self.pos[1]);
                for bp in big {
                    total += 1;
                    let enemy_angle = f64::from(bp[1] - ey).atan2(f64::from(bp[0] - ex)) as f32;
                    for b in world.bullets.iter_mut() {
                        if b.despawn_timer != 0 || b.height >= 30.0 || b.speed != 0.0 {
                            continue;
                        }
                        let dx = b.pos[0] - bp[0];
                        let dy = b.pos[1] - bp[1];
                        if (dx * dx + dy * dy).sqrt() >= 64.0 {
                            continue;
                        }
                        b.ex_flags |= 0x10;
                        b.speed = 0.01;
                        b.timer = 0;
                        b.ex_int0 = 120;
                        let bullets_angle =
                            f64::from(b.pos[1] - ey).atan2(f64::from(b.pos[0] - ex)) as f32;
                        b.angle = (bullets_angle - enemy_angle) * 2.2 + enemy_angle;
                        b.ex_accel = sincos_mul(b.angle, 0.01);
                        b.set_normal_color(b.sprite_offset + 1);
                    }
                }
                self.ex_bat_transform(world);
                self.ctx.ivars[3] = total;
            }
            12 => {
                // ExInsStage4Func12 (EnemyEclInstr.cpp:1071): from each of the
                // enemy's first 8 live lasers, fire the current bulletProps from a
                // point 64px out along the laser's angle.
                for slot in 0..8 {
                    let Some(idx) = self.lasers[slot] else {
                        continue;
                    };
                    let Some(angle) = world.lasers.get(idx).filter(|l| l.in_use).map(|l| l.angle)
                    else {
                        continue;
                    };
                    // utils::Rotate is called with the same input/output
                    // vector. Its second assignment reads the newly written
                    // x, preserving the original aliasing quirk and handedness.
                    let cosine = f64::from(angle).cos() as f32;
                    let sine = f64::from(angle).sin() as f32;
                    let x = cosine * 64.0;
                    let y = -sine * x;
                    self.bullet_props.pos = [self.pos[0] + x, self.pos[1] + y];
                    spawn_bullet_pattern(world, &self.bullet_props);
                }
            }
            14 => {
                // ExInsStageXFunc14 (EnemyEclInstr.cpp:1110): walk each of the
                // enemy's first 8 live lasers from startOffset to endOffset in 48px
                // steps, firing the current bulletProps at each point. var3 counts
                // the lasers that fired.
                self.ctx.ivars[3] = 0;
                for slot in 0..8 {
                    let Some(idx) = self.lasers[slot] else {
                        continue;
                    };
                    let Some((angle, lpos, start, end)) = world
                        .lasers
                        .get(idx)
                        .filter(|l| l.in_use)
                        .map(|l| (l.angle, l.pos, l.start_offset, l.end_offset))
                    else {
                        continue;
                    };
                    let (sin, cos) = (angle.sin(), angle.cos());
                    let mut pm = start;
                    while end > pm {
                        self.bullet_props.pos = [cos * pm + lpos[0], sin * pm + lpos[1]];
                        spawn_bullet_pattern(world, &self.bullet_props);
                        pm += 48.0;
                    }
                    self.ctx.ivars[3] += 1;
                }
            }
            _ => {}
        }
    }

    /// ExInsBatWingEffect (EnemyEclInstr.cpp:737): Remilia's bat wings — a
    /// deep-blue glow-particle (effect 19) spray along an oscillating wing angle,
    /// ramping faster the longer it runs. Suppressed while she is invisible
    /// (mid-transition). Shared by the standalone idx 6 and the bat transform.
    fn ex_bat_wing(&mut self, world: &mut World) {
        use std::f32::consts::PI;
        if self.invisible {
            self.reset_effect_array(world);
            return;
        }
        let deg = PI / 180.0;
        self.exins_func6_angle += deg;
        if self.exins_func6_angle >= 45.0 * PI / 180.0 {
            self.exins_func6_angle -= 90.0 * PI / 180.0;
        }
        let t = self.exins_func6_timer;
        // EnemyManager memset leaves both ZunTimer.current/previous at0;
        // HasTicked is false on this callback's first invocation.
        let fire =
            t != 0 && (t > 120 || (t > 60 && t % 2 == 0) || (t > 30 && t % 4 == 0) || t % 8 == 0);
        if fire {
            let bam = t % 16;
            let bam = world.rng.u16_in_range((bam / 2) as u16) as i32 + bam / 2;
            let dist = (bam as f32 * 160.0) / 16.0 + 32.0;
            let final_angle = self.exins_func6_angle - (bam as f32 * PI) / 40.0;
            let dvy = (8.0 * bam as f32) / 60.0 - 4.0 / 15.0;
            let color = [48.0 / 255.0, 48.0 / 255.0, 1.0, 1.0]; // COLOR_DEEPBLUE 0xff3030ff
            for side in [1.0f32, -1.0] {
                let p = [
                    self.pos[0] + side * final_angle.cos() * dist,
                    self.pos[1] + final_angle.sin() * dist,
                    self.pos[2],
                ];
                // SpawnParticles executes its ANM (including random sprite)
                // before the EXINS draws the particle's horizontal velocity.
                let slot = world.spawn_particle_moving(19, p, color, [0.0; 3], [0.0; 3]);
                let vx = (world.rng.f32_zero_to_one() * 40.0 - 20.0) / 60.0;
                if let Some(effect) = slot.and_then(|i| world.effects[i].as_mut()) {
                    effect.vel = [vx, dvy, 0.0];
                    effect.accel = [-vx / 120.0, -dvy / 120.0, 0.0];
                }
            }
        }
        self.exins_func6_timer += 1;
    }

    /// ExInsHandleBatTransformation (EnemyEclInstr.cpp:1035): idx 10. Runs the
    /// bat-wing spray, then keys off the player bomb. While a bomb is active she
    /// swaps to her dissipate animation (ANM_SCRIPT_ENEMY_END, local 165),
    /// disables the directional-pose left slot (anmExLeft = -1) so it can't
    /// override that script, and becomes non-interactable; 60 frames after the
    /// bomb ends she swaps back to the damageable bat form (local 160) and
    /// restores the left pose (0xa1). No-ops once dead.
    fn ex_bat_transform(&mut self, world: &mut World) {
        if self.life <= 0 {
            return;
        }
        self.ex_bat_wing(world);
        if world.bomb_active {
            if self.anm_poses[3] >= 0 {
                self.anm_script = 165; // ANM_SCRIPT_ENEMY_END (0x1a5 - ANM_OFFSET_ENEMY)
                self.anm_dirty = true;
                self.anm_poses[3] = -1;
            }
            self.interactable = false;
            self.exins_func10_timer = 60;
        } else if self.exins_func10_timer > 0 {
            self.exins_func10_timer -= 1;
            if self.exins_func10_timer == 0 {
                if self.anm_poses[3] < 0 {
                    self.anm_script = 160; // ANM_OFFSET_ENEMY + 0xa0 (bat form)
                    self.anm_dirty = true;
                    self.anm_poses[3] = 0xa1;
                }
                self.interactable = true;
            }
        }
    }

    fn jump(&mut self, instr: &Instr) -> Flow {
        self.ctx.time = instr.arg_i32(0);
        self.ctx.pc = self.ctx.pc.wrapping_add(instr.arg_i32(1) as u32);
        Flow::Jumped
    }

    fn cond_jump(&mut self, instr: &Instr, cond: bool) -> Flow {
        if cond { self.jump(instr) } else { Flow::Next }
    }

    fn call(&mut self, ecl: &Ecl, instr: &Instr) -> Flow {
        let sub = instr.arg_i32(0);
        let var0 = instr.arg_i32(1);
        let float0 = instr.arg_f32(2);
        self.ctx.pc += instr.offset_to_next as u32;
        if !self.disable_call_stack {
            self.stack[self.stack_depth] = self.ctx;
        }
        self.call_sub(ecl, sub);
        if !self.disable_call_stack && self.stack_depth < 7 {
            self.stack_depth += 1;
        }
        self.ctx.ivars[0] = var0;
        self.ctx.fvars[0] = float0;
        Flow::Jumped
    }

    fn alu(&mut self, instr: &Instr, world: &World) {
        let out = instr.arg_i32(0);
        let operation = (instr.opcode - 13) % 7;
        // Both opcode families call the same EclMath functions. Their branch
        // depends on EclGetVar's destination type, rather than the opcode name.
        if matches!(out, -10008..=-10005 | -10017..=-10015) {
            let a = self.get_f32(instr.arg_f32(1), world);
            let b = self.get_f32(instr.arg_f32(2), world);
            let v = match operation {
                0 => a + b,
                1 => a - b,
                2 => a * b,
                3 => a / b,
                _ => a % b,
            };
            self.set_var_bits(out, v.to_bits());
        } else {
            let a = self.get_i32(instr.arg_i32(1), world);
            let b = self.get_i32(instr.arg_i32(2), world);
            let v = match operation {
                0 => a.wrapping_add(b),
                1 => a.wrapping_sub(b),
                2 => a.wrapping_mul(b),
                3 => a.checked_div(b).unwrap_or(0),
                _ => a.checked_rem(b).unwrap_or(0),
            };
            self.set_var_bits(out, v as u32);
        }
    }

    fn start_interp(&mut self, duration: i32) {
        self.move_interp_start = [self.pos[0], self.pos[1]];
        self.move_start_time = duration.max(1);
        self.move_timer = duration.max(1);
        self.movement_mode = 2;
    }

    fn shoot_interval_rank(&self, rank: i32) -> i32 {
        let low = self.shoot_interval / 5;
        rank_scale_i32(low, -low, rank)
    }

    pub fn frame_move(&mut self) {
        if self.invert_x {
            self.pos[0] -= self.axis_speed[0];
        } else {
            self.pos[0] += self.axis_speed[0];
        }
        self.pos[1] += self.axis_speed[1];
        self.clamp();
    }

    fn update_anm_pose(&mut self) {
        // EclManager::RunEcl, after movement/autofire and before EXINSREPEAT.
        if self.anm_poses[3] >= 0 {
            let state = if self.axis_speed[0] < 0.0 {
                1
            } else if self.axis_speed[0] > 0.0 {
                2
            } else {
                0
            };
            if state != self.anm_pose_state {
                let script = match state {
                    1 => self.anm_poses[3],
                    2 => self.anm_poses[4],
                    _ => match self.anm_pose_state {
                        0xff => self.anm_poses[0],
                        1 => self.anm_poses[1],
                        _ => self.anm_poses[2],
                    },
                };
                if script >= 0 {
                    self.anm_script = script as i32;
                    self.anm_dirty = true;
                }
                self.anm_pose_state = state;
            }
        }
    }

    pub fn clamp(&mut self) {
        if self.clamp_pos {
            self.pos[0] = self.pos[0].clamp(self.lower_bound[0], self.upper_bound[0]);
            self.pos[1] = self.pos[1].clamp(self.lower_bound[1], self.upper_bound[1]);
        }
    }

    /// Enemy::UpdateEffects (EnemyManager.cpp:744): keep each live spellcard
    /// bubble on the enemy, grow its sweep toward effectDistance, and spin it.
    pub fn update_effects(&self, world: &mut World) {
        for &slot in &self.effect_slots {
            let Some(Some(e)) = world.effects.get_mut(slot) else {
                continue;
            };
            if !e.in_use || !e.spell_bubble {
                continue;
            }
            e.anchor = self.pos;
            if e.sweep < self.effect_distance {
                e.sweep += 0.3;
            }
            e.angle_rel = normalize_angle(e.angle_rel + std::f32::consts::PI / 100.0);
        }
    }

    /// Enemy::ResetEffectArray (EnemyManager.cpp:127): trigger the fade-out (pop)
    /// on every tracked bubble and forget them. Run on death and at card change.
    pub fn reset_effect_array(&mut self, world: &mut World) {
        for &slot in &self.effect_slots {
            if let Some(Some(e)) = world.effects.get_mut(slot)
                && e.spell_bubble
                && e.pop < 0
            {
                e.pop = 0;
            }
        }
        self.effect_slots.clear();
    }

    /// Queue this enemy's death particle effects (EnemyManager.cpp:641-706 —
    /// the `SpawnParticles` calls only; drops/score/removal stay in collide()).
    /// `death_anm1` is the bubble pop (no RNG), `death_anm2 + 4` the splash
    /// (RandomSplash → 2 RNG each). Every death mode runs the after-switch pair;
    /// mode 3 also pops three bubbles first, modes 0/1/2 add the item splash.
    pub fn spawn_death_effects(&mut self, world: &mut World) {
        let a1 = self.death_anm1 as i32;
        let a2 = self.death_anm2 as i32 + 4;
        if self.death_mode == 3 {
            world.spawn_particles(a1, self.pos, 1, [1.0; 4]);
            world.spawn_particles(a1, self.pos, 1, [1.0; 4]);
            world.spawn_particles(a1, self.pos, 1, [1.0; 4]);
        } else if self.item_drop >= 0 {
            world.spawn_particles(a2, self.pos, 3, [1.0; 4]);
        } else if self.item_drop == -1 {
            // ITEM_RANDOM_ITEM: every 3rd drop gets the bigger splash.
            if world.random_item_spawn_index % 3 == 0 {
                world.spawn_particles(a2, self.pos, 6, [1.0; 4]);
                world.random_item_table_index += 1;
                if world.random_item_table_index >= 32 {
                    world.random_item_table_index = 0;
                }
            }
            world.random_item_spawn_index += 1;
        }
        // After the switch, for every death mode (EnemyManager.cpp:699-700).
        world.spawn_particles(a1, self.pos, 1, [1.0; 4]);
        world.spawn_particles(a2, self.pos, 4, [1.0; 4]);
    }

    /// Bounds bookkeeping; returns false when the enemy should despawn.
    /// `(w, h)` is the enemy's current `primaryVm.sprite` size in px — the decomp
    /// (EnemyManager.cpp:549) tests `GameManager::IsInBounds(pos, sprite->widthPx,
    /// heightPx)`: the sprite rect must overlap the current game region, which
    /// screen shake may shrink. `(0,0)`
    /// before the first ANMSETMAIN sets a sprite — then the enemy is still off
    /// screen at spawn, so `has_been_in_bounds` is never set and it can't despawn.
    pub fn ready_for_damage(&self) -> bool {
        self.occupied && self.has_been_in_bounds && !self.invisible
    }

    /// EnemyManager_OnUpdate applies this after damage/death handling, even
    /// for a non-interactable enemy. Only an actual HP decrease triggers ADD.
    pub fn update_damage_feedback(&mut self, life_before: i32) -> bool {
        if self.damage_flash_timer != 0 {
            self.damage_flash_timer -= 1;
            false
        } else if life_before > self.life {
            self.damage_flash_timer = 4;
            true
        } else {
            false
        }
    }

    pub fn update_bounds(&mut self, w: f32, h: f32, region: [f32; 2]) -> bool {
        let (hw, hh) = (w / 2.0, h / 2.0);
        let inside = !(hw + self.pos[0] < 0.0
            || self.pos[0] - hw > region[0]
            || hh + self.pos[1] < 0.0
            || self.pos[1] - hh > region[1]);
        if !self.has_been_in_bounds && inside {
            self.has_been_in_bounds = true;
        }
        !self.has_been_in_bounds || inside
    }

    pub fn handle_callbacks(&mut self, ecl: &Ecl, world: &mut World) {
        if self.life_cb_threshold >= 0 && self.life < self.life_cb_threshold {
            self.life = self.life_cb_threshold;
            let sub = self.life_cb_sub;
            self.call_sub(ecl, sub);
            self.life_cb_threshold = -1;
            self.timer_cb_sub = self.death_callback;
            self.reset_rank_influence();
            self.stack_depth = 0;
            world.kill_trash = true;
        }
        // HandleTimerCallback (EnemyManager.cpp:386). The boss timer is ticked
        // once per frame by the enemy loop (Enemy::Move's sibling at line 737),
        // not here — this only tests and acts on it.
        if self.timer_cb_threshold >= 0 && self.is_boss {
            // SetSpellcardSeconds precedes the timeout test and timer reset.
            // This also runs for a boss initialized by this calculation's
            // timeline, before GUI12; a Stage pre-enemy lookup misses its birth.
            world.events.push(WorldEvent::BossSeconds(
                (self.timer_cb_threshold - self.boss_timer) / 60,
            ));
        }
        if self.timer_cb_threshold >= 0 && self.boss_timer >= self.timer_cb_threshold {
            if self.life_cb_threshold > 0 {
                self.life = self.life_cb_threshold;
                self.life_cb_threshold = -1;
            }
            let sub = self.timer_cb_sub;
            self.call_sub(ecl, sub);
            self.timer_cb_threshold = -1;
            self.timer_cb_sub = self.death_callback;
            self.boss_timer = 0;
            if !self.timeout_spell {
                // Timed out a non-timeout spell (EnemyManager.cpp:416-421): fade
                // the field via RemoveAllBullets(0) inline — same frame as the
                // timeout, before the bullet update runs, so the half-velocity
                // drift starts on the right frame. The event carries the Stage-
                // side state (capture flag, lasers); its despawn is then a no-op.
                world.despawn_all_bullets();
                if world.spellcard_active != 0 {
                    world.spellcard_active = world.spellcard_active.wrapping_add(1);
                }
                world.events.push(WorldEvent::SpellTimeout);
            }
            self.reset_rank_influence();
            self.stack_depth = 0;
            world.kill_trash = true;
        }
    }

    /// Free the slot when an enemy goes off-screen or its ECL ends. The decomp
    /// callers (EnemyManager.cpp:553,567) set `isSlotOccupied = 0` regardless of
    /// death_mode before calling Enemy::Despawn, so the slot is always freed —
    /// without this a `death_mode != 0` enemy (e.g. the midboss after its
    /// death-callback ECL) stays occupied forever and the stage softlocks.
    pub fn despawn(&mut self, world: &mut World) {
        self.occupied = false;
        self.interactable = false;
        if self.is_boss {
            world.boss_present = false;
            world.time_stopped = false;
            world.events.push(WorldEvent::BossSet(false));
        }
        // Enemy::Despawn owns this cleanup (EnemyManager.cpp:475), including
        // non-boss enemies. Starting/ending a spell does not perform it.
        self.reset_effect_array(world);
    }

    pub fn on_death(&mut self, ecl: &Ecl, world: &mut World) {
        let mode = self.death_mode;
        if self.is_boss && (mode == 0 || mode == 1) {
            self.reset_effect_array(world);
        }
        self.life_cb_threshold = -1;
        self.timer_cb_threshold = -1;
        match self.death_mode {
            3 => {
                self.life = 1;
                self.damageable = false;
                self.death_mode = 0;
                world.boss_present = false;
            }
            1 => self.interactable = false,
            0 => self.occupied = false,
            _ => {}
        }
        if mode != 3 {
            self.life = 0;
        }
        if self.is_boss && (mode == 0 || mode == 1) {
            world.boss_present = false;
            world.time_stopped = false;
            world.events.push(WorldEvent::BossSet(false));
        }
        if self.death_callback >= 0 {
            self.reset_rank_influence();
            self.stack_depth = 0;
            let sub = self.death_callback;
            self.call_sub(ecl, sub);
            self.death_callback = -1;
        }
    }

    pub fn fire_interrupt(&mut self, id: i32) {
        self.run_interrupt = id;
    }

    /// Advance the boss timer one frame (EnemyManager.cpp:737 — every occupied
    /// enemy each frame, unless time is stopped). It drives the spell time
    /// limit, the `-10022` ECL var, and rage timers (e.g. Flandre's 7200).
    pub fn tick_boss_timer(&mut self) {
        self.boss_timer += 1;
    }

    /// ENEMYKILLALL / phase-transition trash kill (EnemyManager.cpp:360-378):
    /// drop life to 0 (the normal death pass finishes interactable enemies) and
    /// directly run the death callback for non-interactable ones.
    pub fn kill_as_trash(&mut self, ecl: &Ecl) {
        self.life = 0;
        if !self.interactable && self.death_callback >= 0 {
            let sub = self.death_callback;
            self.call_sub(ecl, sub);
            self.death_callback = -1;
        }
    }

    /// Per-enemy timer observation used by the original attack-oracle test.
    /// Runtime GUI timing is latched by HandleTimerCallback instead.
    #[cfg(test)]
    pub fn spell_seconds_left(&self) -> Option<i32> {
        if self.is_boss && self.timer_cb_threshold > 0 {
            Some(((self.timer_cb_threshold - self.boss_timer) / 60).max(0))
        } else {
            None
        }
    }

    fn reset_rank_influence(&mut self) {
        self.rank_speed_low = -0.5;
        self.rank_speed_high = 0.5;
        self.rank_amount1_low = 0;
        self.rank_amount1_high = 0;
        self.rank_amount2_low = 0;
        self.rank_amount2_high = 0;
    }
}

enum Flow {
    Next,
    Jumped,
    Kill,
}

/// Bullet type -> etama3 base sprite index (AnmIdx.hpp ANM_SPRITE_BULLET3_*:
/// pellet, ring-ball, rice, ball, kunai, shard, big-ball, fireball, dagger,
/// laser). Fireball is 118, not 110 — it is its own sprite group after big-ball.
pub const BULLET_BASE_SPRITE: [u32; 10] = [14, 30, 46, 62, 78, 94, 110, 118, 122, 0];

/// Faithful port of BulletManager::SpawnSingleBullet's aim-mode math.
pub fn spawn_bullet_pattern(world: &mut World, props: &BulletProps) {
    let aim_angle = player_angle_from(world.player_pos, props.pos);
    let tau = std::f32::consts::TAU;
    let pi = std::f32::consts::PI;
    'pattern: for idx2 in 0..props.count2 as i32 {
        for idx1 in 0..props.count1 as i32 {
            // SpawnSingleBullet searches the 640 slots before doing any
            // pattern math or RNG draw, advancing its circular cursor even on
            // occupied slots. Full pools stop this entire pattern.
            let mut slot = None;
            for _ in 0..640 {
                let candidate = world.next_bullet_index;
                world.next_bullet_index = (candidate + 1) % 640;
                if let Err(insert_at) = world
                    .bullets
                    .binary_search_by_key(&candidate, |bullet| bullet.pool_index)
                {
                    slot = Some((candidate, insert_at));
                    break;
                }
            }
            let Some((pool_index, insert_at)) = slot else {
                break 'pattern;
            };
            let mut angle = 0.0f32;
            let mut speed =
                props.speed1 - (props.speed1 - props.speed2) * idx2 as f32 / props.count2 as f32;
            match props.aim_mode {
                0 | 1 => {
                    // FAN_AIMED / FAN
                    if props.count1 & 1 != 0 {
                        angle += ((idx1 + 1) / 2) as f32 * props.angle2;
                    } else {
                        angle += (idx1 / 2) as f32 * props.angle2 + props.angle2 * 0.5;
                    }
                    if idx1 & 1 != 0 {
                        angle = -angle;
                    }
                    if props.aim_mode == 0 {
                        angle += aim_angle;
                    }
                    angle += props.angle1;
                }
                2 | 3 => {
                    // CIRCLE_AIMED / CIRCLE
                    if props.aim_mode == 2 {
                        angle += aim_angle;
                    }
                    angle += idx1 as f32 * tau / props.count1 as f32;
                    angle += idx2 as f32 * props.angle2 + props.angle1;
                }
                4 | 5 => {
                    // OFFSET_CIRCLE_AIMED / OFFSET_CIRCLE
                    if props.aim_mode == 4 {
                        angle += aim_angle;
                    }
                    angle += pi / props.count1 as f32;
                    angle += idx1 as f32 * tau / props.count1 as f32;
                    angle += props.angle1;
                }
                6 => {
                    // RANDOM_ANGLE
                    angle = world.rng.f32_in_range(props.angle1 - props.angle2) + props.angle2;
                }
                7 => {
                    // RANDOM_SPEED
                    speed = world.rng.f32_in_range(props.speed1 - props.speed2) + props.speed2;
                    angle += idx1 as f32 * tau / props.count1 as f32;
                    angle += idx2 as f32 * props.angle2 + props.angle1;
                }
                _ => {
                    // RANDOM
                    angle = world.rng.f32_in_range(props.angle1 - props.angle2) + props.angle2;
                    speed = world.rng.f32_in_range(props.speed1 - props.speed2) + props.speed2;
                }
            }
            // No bullet mirroring: SpawnSingleBullet (BulletManager.cpp:118)
            // never flips the angle. `invertX` only negates the enemy's X
            // movement (Enemy::Move), not its fire pattern.
            // SpawnSingleBullet keeps bulletAngle for EX_ACCELERATION even
            // though the stored direction and initial velocity are normalized.
            let pattern_angle = angle;
            angle = normalize_angle(angle);
            let base = BULLET_BASE_SPRITE
                .get(props.sprite.max(0) as usize)
                .copied()
                .unwrap_or(30);
            // Spawn-in effect (BulletManager.cpp:683-708): while spawning, the
            // bullet moves at velocity/2 (FAST flag 2), /2.5 (NORMAL flag 4) or
            // /3 (SLOW flag 8) for the spawn anm-script's length, then snaps to
            // full speed. Durations from etama3.anm: FAST 10 / NORMAL 16 / SLOW 32
            // frames; the big types (>=6, which use SPAWN_BIG_BALL_HUGE) are 32
            // for every state.
            let big = props.sprite >= 6;
            let (spawn_delay, spawn_factor) = if props.flags & 2 != 0 {
                (if big { 32 } else { 10 }, 0.5)
            } else if props.flags & 4 != 0 {
                (if big { 32 } else { 16 }, 1.0 / 2.5)
            } else if props.flags & 8 != 0 {
                (32u32, 1.0 / 3.0)
            } else {
                (0u32, 1.0)
            };
            // Ex-behavior setup, ported from SpawnSingleBullet.
            let mut ex_accel = [0.0f32; 2];
            let mut ex_f = [0.0f32; 2];
            let mut ex_int0 = 0;
            let mut ex_int1 = 0;
            if props.flags & 0x10 != 0 {
                let dir = if props.ex_floats[1] <= -999.0 {
                    pattern_angle
                } else {
                    props.ex_floats[1]
                };
                ex_accel = sincos_mul(dir, props.ex_floats[0]);
                ex_int0 = if props.ex_ints[0] > 0 {
                    props.ex_ints[0]
                } else {
                    99999
                };
            } else if props.flags & 0x20 != 0 {
                ex_f = [props.ex_floats[0], props.ex_floats[1]];
                ex_int0 = props.ex_ints[0];
            }
            if props.flags & 0x1c0 != 0 {
                // Direction-change group (0x40 rotate / 0x80 aim-at-player /
                // 0x100 absolute): shared dirChange* setup.
                ex_f = [
                    props.ex_floats[0],
                    if props.ex_floats[1] >= 0.0 {
                        props.ex_floats[1]
                    } else {
                        speed
                    },
                ];
                ex_int0 = props.ex_ints[0];
                ex_int1 = props.ex_ints[1];
            }
            if props.flags & 0xc00 != 0 {
                // Wall-bounce group (0x400 all edges / 0x800 top+sides):
                // dirChangeSpeed = exFloats[0] (or the spawn speed if < 0),
                // dirChangeMaxTimes = exInts[0] (BulletManager.cpp:363). Stored in
                // the same ex_f[1]/ex_int1 slots the per-frame bounce reads.
                ex_f[1] = if props.ex_floats[0] >= 0.0 {
                    props.ex_floats[0]
                } else {
                    speed
                };
                ex_int1 = props.ex_ints[0];
            }
            let bullet_type = props.sprite.clamp(0, 9) as usize;
            world.bullets.insert(
                insert_at,
                Bullet {
                    pool_index,
                    pos: props.pos,
                    angle,
                    speed,
                    velocity: sincos_mul(angle, speed),
                    type_index: bullet_type,
                    visuals: world
                        .bullet_templates
                        .get(bullet_type)
                        .cloned()
                        .map(|mut visual| {
                            visual.set_color(
                                props.sprite_offset.max(0) as u32,
                                world.bullet_heights[bullet_type],
                                bullet_type,
                            );
                            visual
                        }),
                    sprite: base + props.sprite_offset.max(0) as u32,
                    base_sprite: base,
                    spawn_factor,
                    oob_count: 0,
                    sprite_offset: props.sprite_offset.max(0),
                    height: world.bullet_heights[bullet_type],
                    width: world.bullet_widths[bullet_type],
                    spawn_delay,
                    timer: 0,
                    ex_flags: props.flags,
                    ex_accel,
                    ex_f,
                    ex_int0,
                    ex_int1,
                    ex_count: 0,
                    grazed: false,
                    despawn_timer: 0,
                },
            );
        }
    }
    if props.flags & 0x200 != 0 && props.sfx >= 0 {
        world.events.push(WorldEvent::Sfx(props.sfx));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extra_reflection_keeps_original_atan2_rounding() {
        // Original Marisa A / Extra, just before replay calculation 20209.
        // atan2f loses one bit here; the subsequent 2.2 reflection amplifies
        // that difference until the accelerated trajectory diverges.
        let mut world = world();
        for _ in 0..2 {
            spawn_bullet_pattern(&mut world, &BulletProps::default());
        }
        world.bullets[0].pos = [64.0, f32::from_bits(0x4333fff9)];
        world.bullets[0].height = 16.0;
        world.bullets[1].pos = [f32::from_bits(0x42eb9cd1), f32::from_bits(0x43178663)];
        world.bullets[1].height = 32.0;
        let mut enemy = Enemy {
            pos: [f32::from_bits(0x433ee3ca), 48.0, 0.0],
            life: 0, // The independently tested bat callback is irrelevant here.
            ..Default::default()
        };
        enemy.exec_ex(15, 0, &mut world);
        assert_eq!(world.bullets[0].angle.to_bits(), 0x40212067);
        assert_eq!(world.bullets[0].ex_flags & 0x10, 0x10);
        assert_eq!(world.bullets[0].ex_int0, 120);
    }

    #[test]
    fn boss_seconds_callback_latches_before_timeout_and_timer_increment() {
        let ecl = Ecl {
            data: Vec::new(),
            sub_offsets: Vec::new(),
            timeline_offset: 0,
        };
        for (threshold, timer, seconds) in [(1500, 1, 24), (60, 1, 0), (0, 60, -1)] {
            let mut world = world();
            world.time_stopped = true;
            let mut enemy = Enemy {
                is_boss: true,
                timer_cb_threshold: threshold,
                boss_timer: timer,
                ..Default::default()
            };
            let rng = world.rng.seed();
            enemy.handle_callbacks(&ecl, &mut world);
            assert!(
                matches!(world.events.first(), Some(WorldEvent::BossSeconds(n)) if *n == seconds)
            );
            assert_eq!(world.rng.seed(), rng);
            if timer < threshold {
                assert_eq!(enemy.boss_timer, timer);
            } else {
                assert_eq!(enemy.timer_cb_threshold, -1);
            }
        }
        for (is_boss, threshold) in [(true, -1), (false, 1500)] {
            let mut world = world();
            let mut enemy = Enemy {
                is_boss,
                timer_cb_threshold: threshold,
                ..Default::default()
            };
            enemy.handle_callbacks(&ecl, &mut world);
            assert!(
                !world
                    .events
                    .iter()
                    .any(|e| matches!(e, WorldEvent::BossSeconds(_)))
            );
        }
    }

    #[test]
    fn spawn_ecl_selects_pose_before_first_bounds_and_move_does_not_reselect() {
        // Extra/ReimuA first divergence11758: Flandre at x=-28.282 has a
        // 64px pose sprite already, and becomes the homing target this tick.
        fn instruction(data: &mut Vec<u8>, time: i32, opcode: u16, args: &[u8]) {
            data.extend(time.to_le_bytes());
            data.extend(opcode.to_le_bytes());
            data.extend(((12 + args.len()) as u16).to_le_bytes());
            data.extend([0, 31, 0, 0]);
            data.extend(args);
        }
        let mut data = Vec::new();
        let poses = [160i16, 163, 164, 161, 162, 0]
            .into_iter()
            .flat_map(i16::to_le_bytes)
            .collect::<Vec<_>>();
        instruction(&mut data, 0, 98, &poses);
        let mut motion = 120i32.to_le_bytes().to_vec();
        motion.extend(192f32.to_le_bytes());
        motion.extend(96f32.to_le_bytes());
        motion.extend(0f32.to_le_bytes());
        instruction(&mut data, 0, 57, &motion);
        instruction(&mut data, 120, 0, &[]);
        let ecl = Ecl {
            data,
            sub_offsets: vec![0],
            timeline_offset: 0,
        };
        let mut world = world();
        let mut enemy = Enemy::spawn(
            &ecl,
            &mut world,
            &SpawnReq {
                sub: 0,
                pos: [-32.0, 96.0, 0.0],
                life: 15000,
                item: -2,
                score: 0,
                mirror: false,
            },
        )
        .unwrap();
        assert_eq!((enemy.anm_script, enemy.anm_pose_state), (162, 2));
        enemy.frame_move();
        assert!((enemy.pos[0] - -28.282225).abs() < 0.00001);
        assert!(enemy.update_bounds(64.0, 80.0, [384.0, 448.0]));
        assert!(enemy.ready_for_damage());
        enemy.movement_mode = 0;
        enemy.axis_speed[0] = -1.0;
        enemy.frame_move();
        assert_eq!(enemy.anm_script, 162);
        enemy.run_ecl(&ecl, &mut world);
        assert_eq!((enemy.anm_script, enemy.anm_pose_state), (161, 1));
    }

    #[test]
    fn aiming_uses_double_atan2_before_rounding_to_float() {
        // The EXE's 0x428766 calls the double atan2 runtime entry, followed
        // by FSTP f32. A float atan2 implementation differs by one ULP here.
        assert_eq!(
            player_angle_from([150.62506, 127.82745], [0.0, 0.0]).to_bits(),
            0x3f342633
        );
    }

    #[test]
    fn normalize_angle_opcode_reads_integer_destination_id() {
        let mut enemy = Enemy::default();
        enemy.ctx.fvars[0] = 3.2;
        let mut world = world();
        let ecl = Ecl {
            data: Vec::new(),
            sub_offsets: Vec::new(),
            timeline_offset: 0,
        };
        let args = (-10005i32).to_le_bytes();
        enemy.exec(
            &ecl,
            &mut world,
            &Instr {
                time: 0,
                opcode: 26,
                offset_to_next: 16,
                skip_for_difficulty: 31,
                args: &args,
            },
        );
        assert_eq!(enemy.ctx.fvars[0], normalize_angle(3.2));
        assert!(enemy.ctx.fvars[0].is_finite());
    }

    #[test]
    fn set_opcodes_copy_float_register_bits_without_truncation() {
        let mut enemy = Enemy::default();
        enemy.ctx.fvars[0] = f32::from_bits(0xbf0bbdd8);
        let mut world = world();
        let ecl = Ecl {
            data: Vec::new(),
            sub_offsets: Vec::new(),
            timeline_offset: 0,
        };
        // Stage5 child25 uses SET_INT f0,f0 before its first bullet fan.
        for opcode in [4, 5] {
            let mut args = (-10006i32).to_le_bytes().to_vec();
            args.extend((-10005i32).to_le_bytes());
            enemy.exec(
                &ecl,
                &mut world,
                &Instr {
                    time: 0,
                    opcode,
                    offset_to_next: 20,
                    skip_for_difficulty: 31,
                    args: &args,
                },
            );
            assert_eq!(enemy.ctx.fvars[1].to_bits(), enemy.ctx.fvars[0].to_bits());
        }
        assert_eq!(
            enemy.get_i32(-10005, &world) as u32,
            enemy.ctx.fvars[0].to_bits()
        );
        enemy.ctx.ivars[0] = 1.25f32.to_bits() as i32;
        assert_eq!(enemy.get_f32(-10001.0, &world), 1.25);
    }

    #[test]
    fn arithmetic_opcode_families_use_destination_type() {
        let mut enemy = Enemy::default();
        let mut world = world();
        let ecl = Ecl {
            data: Vec::new(),
            sub_offsets: Vec::new(),
            timeline_offset: 0,
        };
        let mut args = (-10005i32).to_le_bytes().to_vec();
        args.extend(0.25f32.to_le_bytes());
        args.extend(0.5f32.to_le_bytes());
        for opcode in [13, 20] {
            enemy.exec(
                &ecl,
                &mut world,
                &Instr {
                    time: 0,
                    opcode,
                    offset_to_next: 24,
                    skip_for_difficulty: 31,
                    args: &args,
                },
            );
            assert_eq!(enemy.ctx.fvars[0], 0.75);
        }
        let mut args = (-10001i32).to_le_bytes().to_vec();
        args.extend(25i32.to_le_bytes());
        args.extend(50i32.to_le_bytes());
        for opcode in [13, 20] {
            enemy.exec(
                &ecl,
                &mut world,
                &Instr {
                    time: 0,
                    opcode,
                    offset_to_next: 24,
                    skip_for_difficulty: 31,
                    args: &args,
                },
            );
            assert_eq!(enemy.ctx.ivars[0], 75);
        }
    }

    #[test]
    fn projectile_velocity_keeps_original_sincos_multiply_precision() {
        // Actual 1.02h replay-c0-d1-s1 bullet slot344 at frame2880.
        let velocity = sincos_mul(f32::from_bits(0xc040b062), f32::from_bits(0x3fcd5555));
        assert_eq!(velocity.map(f32::to_bits), [0xbfcb9420, 0xbe564af5]);
        assert_eq!(
            player_angle_from([20.0, 30.0], [20.0, 30.0]),
            std::f32::consts::FRAC_PI_2
        );
    }

    #[test]
    fn sakuya_knife_fan_uses_original_three_dimensional_normalization() {
        // Passive original EXE probe at stage5 Normal replay frame9468.
        // positionCenter.z remains 0.49 before the first respawn.
        let mut world = world();
        world.difficulty = 1;
        world.player_pos = [194.0, 384.0];
        let mut enemy = Enemy {
            pos: [192.0, 112.0, 0.0],
            ..Default::default()
        };
        enemy.exec_ex(5, 0, &mut world);
        assert_eq!(world.bullets.len(), 9);
        let expected = [
            [0x423a0949, 0x4393b00e],
            [0x42a9fa78, 0x438863d8],
            [0x42fd9de3, 0x43809ae0],
            [0x432ab228, 0x43792364],
            [0x43575181, 0x4378cf65],
            [0x4381a8f2, 0x43801e29],
            [0x4396ae87, 0x43879834],
            [0x43aa15fb, 0x43929bac],
            [0x43bb485f, 0x43a0d2e4],
        ];
        assert_eq!(
            world
                .bullets
                .iter()
                .map(|b| b.pos.map(f32::to_bits))
                .collect::<Vec<_>>(),
            expected
        );
    }

    #[test]
    fn acceleration_retains_the_unwrapped_pattern_angle() {
        let mut world = world();
        let props = BulletProps {
            aim_mode: 1,
            count1: 1,
            count2: 1,
            angle1: 5.0 * std::f32::consts::PI / 4.0,
            speed1: 0.3,
            flags: 0x10,
            ex_ints: [100, 0, 0, 0],
            ex_floats: [0.015, -999.0, 0.0, 0.0],
            ..Default::default()
        };
        spawn_bullet_pattern(&mut world, &props);
        let bullet = &world.bullets[0];
        assert_eq!(bullet.angle, normalize_angle(props.angle1));
        assert_eq!(bullet.ex_accel, sincos_mul(props.angle1, 0.015));
        assert_ne!(bullet.ex_accel, sincos_mul(bullet.angle, 0.015));
    }

    #[test]
    fn aimed_laser_uses_shifted_spawn_position() {
        let mut world = world();
        world.player_pos = [192.0, 400.0];
        let ecl = Ecl {
            data: Vec::new(),
            sub_offsets: Vec::new(),
            timeline_offset: 0,
        };
        let mut enemy = Enemy {
            pos: [64.0, 100.0, 0.0],
            shoot_offset: [30.0, -20.0],
            ..Default::default()
        };
        let mut args = vec![0; 4];
        for value in [0.25f32, 1.0, 0.0, 100.0, 100.0, 16.0] {
            args.extend(value.to_le_bytes());
        }
        for value in [0i32, 60, 10, 0, 0, 0] {
            args.extend(value.to_le_bytes());
        }
        enemy.exec(
            &ecl,
            &mut world,
            &Instr {
                time: 0,
                opcode: 86,
                offset_to_next: (12 + args.len()) as i16,
                skip_for_difficulty: 31,
                args: &args,
            },
        );
        let laser = &world.lasers[0];
        assert_eq!(laser.pos, [94.0, 80.0]);
        assert_eq!(
            laser.angle,
            0.25 + player_angle_from(world.player_pos, laser.pos)
        );
    }

    #[test]
    #[ignore = "requires original decomp; build with TH06_DECOMP_SRC=... python3 oracle/player/build.py"]
    fn original_cpp_repeated_phase_deaths_emit_every_particle() {
        use std::fmt::Write;
        let mut actual = String::new();
        for mode in 0..4 {
            for drop in [-2, 0] {
                let mut world = world();
                for script in [3, 9] {
                    world.eff_scripts.insert(
                        script,
                        vec![AnmInstr {
                            time: 0,
                            opcode: 21,
                            args: Vec::new(),
                            offset: 0,
                        }],
                    );
                }
                let mut enemy = Enemy {
                    death_mode: mode,
                    item_drop: drop,
                    ..Default::default()
                };
                for phase in 0..2 {
                    enemy.spawn_death_effects(&mut world);
                    writeln!(
                        actual,
                        "{mode} {drop} {phase} {}",
                        world.effects.iter().flatten().count()
                    )
                    .unwrap();
                }
            }
        }
        let binary = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../oracle/player/build/player_reference");
        let output = std::process::Command::new(binary)
            .arg("deathfx")
            .output()
            .unwrap();
        assert!(output.status.success());
        assert_eq!(actual, String::from_utf8(output.stdout).unwrap());
    }

    #[test]
    #[ignore = "requires original decomp; build with TH06_DECOMP_SRC=... python3 oracle/player/build.py"]
    fn original_cpp_enemy_death_modes_keep_set_hp_to_one_alive() {
        use std::fmt::Write;
        let ecl = Ecl {
            data: Vec::new(),
            sub_offsets: Vec::new(),
            timeline_offset: 0,
        };
        let mut actual = String::new();
        for mode in 0..4 {
            for boss in 0..2 {
                let mut world = world();
                world.boss_present = true;
                let mut enemy = Enemy {
                    life: -9,
                    death_mode: mode,
                    is_boss: boss != 0,
                    occupied: true,
                    interactable: true,
                    damageable: true,
                    effect_slots: vec![999],
                    ..Default::default()
                };
                enemy.on_death(&ecl, &mut world);
                writeln!(
                    actual,
                    "{mode} {boss} {} {} {} {} {} {} {}",
                    enemy.life,
                    enemy.death_mode,
                    u8::from(enemy.damageable),
                    u8::from(enemy.interactable),
                    u8::from(enemy.occupied),
                    u8::from(world.boss_present),
                    u8::from(enemy.effect_slots.is_empty())
                )
                .unwrap();
            }
        }
        let binary = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../oracle/player/build/player_reference");
        let output = std::process::Command::new(binary)
            .arg("death")
            .output()
            .unwrap();
        assert!(output.status.success());
        assert_eq!(actual, String::from_utf8(output.stdout).unwrap());
    }

    #[test]
    #[ignore = "requires original decomp; build with TH06_DECOMP_SRC=... python3 oracle/player/build.py"]
    fn original_cpp_enemy_damage_feedback_matches_sound_and_color_cadence() {
        use std::fmt::Write;
        let mut enemy = Enemy {
            life: 100,
            ..Default::default()
        };
        let mut actual = String::new();
        for frame in 0..26 {
            let before = enemy.life;
            if frame < 10 || (12..24).contains(&frame) {
                enemy.life -= 1;
            } else if frame == 10 {
                enemy.life += 10;
            }
            let flash = enemy.update_damage_feedback(before);
            writeln!(
                actual,
                "{frame} {} {} {} {}",
                enemy.life,
                enemy.damage_flash_timer,
                u8::from(flash),
                if flash { 20 } else { -1 }
            )
            .unwrap();
        }
        let binary = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../oracle/player/build/player_reference");
        let output = std::process::Command::new(binary)
            .arg("flash")
            .output()
            .expect("build oracle/player/build.py first");
        assert!(output.status.success());
        assert_eq!(actual, String::from_utf8(output.stdout).unwrap());
    }

    #[test]
    #[ignore = "requires original decomp; build with TH06_DECOMP_SRC=... python3 oracle/enemy/build.py"]
    fn original_cpp_bat_wings_match_timer_cadence_and_particle_rng_order() {
        use std::fmt::Write;
        let mut actual = String::new();
        for random in 0..2 {
            let mut world = world();
            let mut script = vec![AnmInstr {
                time: 0,
                opcode: 1,
                args: 11u32.to_le_bytes().to_vec(),
                offset: 0,
            }];
            if random != 0 {
                script[0] = AnmInstr {
                    time: 0,
                    opcode: 16,
                    args: [11u32.to_le_bytes(), 8u32.to_le_bytes()].concat(),
                    offset: 0,
                };
            }
            script.push(AnmInstr {
                time: 0,
                opcode: 21,
                args: Vec::new(),
                offset: 12,
            });
            world.eff_scripts.insert(19, script);
            let mut enemy = Enemy {
                pos: [192.0, 96.0, 0.0],
                ..Default::default()
            };
            for frame in 0..140 {
                enemy.invisible = (130..135).contains(&frame);
                world.effects.clear();
                enemy.ex_bat_wing(&mut world);
                let effects: Vec<_> = world.effects.iter().flatten().collect();
                write!(
                    actual,
                    "{random} {frame} {} {} {} {}",
                    enemy.exins_func6_timer,
                    enemy.exins_func6_angle.to_bits(),
                    world.rng.seed(),
                    effects.len()
                )
                .unwrap();
                for effect in effects {
                    write!(
                        actual,
                        " {} {} {} {} {} {} {}",
                        effect.pos[0].to_bits(),
                        effect.pos[1].to_bits(),
                        effect.vel[0].to_bits(),
                        effect.vel[1].to_bits(),
                        effect.accel[0].to_bits(),
                        effect.accel[1].to_bits(),
                        effect.runner.sprite.unwrap()
                    )
                    .unwrap();
                }
                actual.push('\n');
            }
        }
        let binary = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../oracle/enemy/build/enemy_reference");
        let output = std::process::Command::new(binary)
            .output()
            .expect("build oracle/enemy/build.py first");
        assert!(output.status.success());
        assert_eq!(actual, String::from_utf8(output.stdout).unwrap());
    }

    fn world() -> World {
        World {
            rng: Rng::new(0x1234),
            difficulty: 4,
            rank: 16,
            sub_rank: 0,
            min_rank: 14,
            max_rank: 18,
            player_pos: [192.0, 384.0],
            player_z: 0.49,
            bullets: Vec::new(),
            next_bullet_index: 0,
            lasers: Vec::new(),
            events: Vec::new(),
            enemy_slots: [false; 256],
            pending_spawns: Vec::new(),
            kill_trash: false,
            boss_present: false,
            power: 128,
            character: 1,
            shot_type: 1,
            time_stopped: false,
            bomb_active: false,
            spellcard_active: 0,
            bullet_heights: [16.0; 10],
            bullet_widths: [16.0; 10],
            bullet_templates: Vec::new(),
            effects: Vec::new(),
            next_effect_index: 0,
            eff_scripts: HashMap::new(),
            random_item_spawn_index: 0,
            random_item_table_index: 0,
        }
    }

    #[test]
    fn spell_start_and_end_preserve_bubbles_until_enemy_despawns() {
        // Real102h MarisaA Extra keeps five sub32 bubbles when sub33 declares
        // its spell at replay11949. Prematurely popping them at11964 makes
        // room for three extra effects at25012 and consumes11extra RNG draws.
        // EclManager SPELLCARD_START/END do not call Enemy_ResetEffectArray;
        // Enemy::Despawn does, independently of the boss flag.
        for is_boss in [false, true] {
            let mut world = world();
            world.eff_scripts.insert(
                18,
                vec![AnmInstr {
                    time: 0,
                    opcode: 21,
                    args: Vec::new(),
                    offset: 0,
                }],
            );
            let mut enemy = Enemy {
                occupied: true,
                is_boss,
                ..Default::default()
            };
            for _ in 0..5 {
                enemy.effect_slots.push(
                    world
                        .spawn_spell_bubble([0.0; 3], [1.0, 0.0, 0.0], [1.0; 4])
                        .unwrap(),
                );
            }
            let ecl = Ecl {
                data: Vec::new(),
                sub_offsets: Vec::new(),
                timeline_offset: 0,
            };
            for opcode in [93, 94] {
                enemy.exec(
                    &ecl,
                    &mut world,
                    &Instr {
                        time: 0,
                        opcode,
                        offset_to_next: 16,
                        skip_for_difficulty: 31,
                        args: &[0; 4],
                    },
                );
                for _ in 0..16 {
                    world.update_effects();
                }
                assert_eq!(enemy.effect_slots.len(), 5);
                assert_eq!(world.effects.iter().flatten().count(), 5);
                assert!(
                    world
                        .effects
                        .iter()
                        .flatten()
                        .all(|effect| effect.pop == -1)
                );
            }
            enemy.despawn(&mut world);
            assert!(enemy.effect_slots.is_empty());
            for _ in 0..15 {
                world.update_effects();
            }
            assert_eq!(world.effects.iter().flatten().count(), 5);
            world.update_effects();
            assert_eq!(world.effects.iter().flatten().count(), 0);
        }
    }

    #[test]
    fn particle_random_sprite_draw_precedes_motion_rng_and_full_pool_draws_nothing() {
        let mut world = world();
        world.rng = Rng::new(34784);
        world.eff_scripts.insert(
            10,
            vec![
                AnmInstr {
                    time: 0,
                    opcode: 16,
                    args: [11u32.to_le_bytes(), 4u32.to_le_bytes()].concat(),
                    offset: 0,
                },
                AnmInstr {
                    time: 0,
                    opcode: 21,
                    args: Vec::new(),
                    offset: 12,
                },
            ],
        );
        world.spawn_particles(5, [100.0, 200.0, 0.0], 1, [1.0; 4]);
        let mut expected = Rng::new(34784);
        let sprite = 11 + expected.u16_in_range(4) as u32;
        assert_eq!(
            world.effects[0].as_ref().unwrap().runner.sprite,
            Some(sprite)
        );
        assert_eq!(world.rng, expected);
        expected.f32_zero_to_one();
        expected.f32_zero_to_one();
        world.update_effects();
        assert_eq!(world.rng, expected);
        assert_eq!(world.rng.seed(), 38085); // observed real EXE demo00 frame456
        world.spawn_particles(5, [0.0; 3], 511, [1.0; 4]);
        let full_rng = world.rng;
        world.spawn_particles(5, [0.0; 3], 1, [1.0; 4]);
        assert_eq!(world.rng, full_rng);
        assert_eq!(world.effects.iter().flatten().count(), 512);
    }

    #[test]
    fn enemy_create_initializes_child_inline_before_parent_rng_draw() {
        fn instruction(data: &mut Vec<u8>, time: i32, opcode: i16, args: &[u8]) {
            data.extend(time.to_le_bytes());
            data.extend(opcode.to_le_bytes());
            data.extend(((12 + args.len()) as i16).to_le_bytes());
            data.extend([0, 31, 0, 0]);
            data.extend(args);
        }
        let mut data = Vec::new();
        let mut spawn = Vec::new();
        spawn.extend(1i32.to_le_bytes());
        spawn.extend([0; 12]);
        spawn.extend(1i16.to_le_bytes());
        spawn.extend((-1i16).to_le_bytes());
        spawn.extend(100i32.to_le_bytes());
        instruction(&mut data, 0, 95, &spawn);
        let random_args = [(-10001i32).to_le_bytes(), 65535i32.to_le_bytes()].concat();
        instruction(&mut data, 0, 6, &random_args);
        instruction(&mut data, 1, 0, &[]);
        let child_offset = data.len() as u32;
        instruction(&mut data, 0, 6, &random_args);
        instruction(&mut data, 1, 0, &[]);
        let ecl = Ecl {
            data,
            sub_offsets: vec![0, child_offset],
            timeline_offset: 0,
        };
        let mut world = world();
        let mut expected = world.rng;
        let first = expected.u32_in_range(65535) as i32;
        let second = expected.u32_in_range(65535) as i32;
        let parent = Enemy::spawn(
            &ecl,
            &mut world,
            &SpawnReq {
                sub: 0,
                pos: [0.0; 3],
                life: 1,
                item: -1,
                score: 100,
                mirror: false,
            },
        )
        .unwrap();
        assert_eq!(
            (parent.pool_index, world.pending_spawns[0].pool_index),
            (0, 1)
        );
        assert_eq!(world.pending_spawns[0].ctx.ivars[0], first);
        assert_eq!(parent.ctx.ivars[0], second);
        assert_eq!(world.rng, expected);
        world.enemy_slots.fill(true);
        assert!(
            Enemy::spawn(
                &ecl,
                &mut world,
                &SpawnReq {
                    sub: 0,
                    pos: [0.0; 3],
                    life: 1,
                    item: -1,
                    score: 100,
                    mirror: false,
                }
            )
            .is_none()
        );
        assert_eq!(world.rng, expected);
    }

    #[test]
    fn slot_interrupt_129_and_player_shot_variable_use_original_values() {
        let mut world = world();
        let ecl = Ecl {
            data: Vec::new(),
            sub_offsets: Vec::new(),
            timeline_offset: 0,
        };
        let mut enemy = Enemy::default();
        let args = [2i32.to_le_bytes(), 7i32.to_le_bytes()].concat();
        let instr = Instr {
            time: 0,
            opcode: 129,
            offset_to_next: 20,
            skip_for_difficulty: 31,
            args: &args,
        };
        enemy.exec(&ecl, &mut world, &instr);
        assert_eq!(
            enemy.sub_interrupts,
            [None, None, Some(7), None, None, None, None, None]
        );
        assert_eq!(enemy.get_i32(-10025, &world), 3);
    }

    #[test]
    fn perfect_freeze_recolors_live_bullets_and_emits_pulse_without_touching_despawns() {
        let mut world = world();
        world.eff_scripts.insert(
            17,
            vec![AnmInstr {
                time: 0,
                opcode: 21,
                args: Vec::new(),
                offset: 0,
            }],
        );
        spawn_bullet_pattern(
            &mut world,
            &BulletProps {
                count1: 2,
                count2: 1,
                sprite_offset: 3,
                ..Default::default()
            },
        );
        world.bullets[1].despawn_timer = 1;
        let dead_speed = world.bullets[1].speed;
        let dead_sprite = world.bullets[1].sprite;
        let rng = world.rng;
        let mut enemy = Enemy::default();
        enemy.exec_ex(0, 0, &mut world);
        assert_eq!(world.bullets[0].sprite_offset, 0);
        assert_eq!(world.bullets[0].sprite, world.bullets[0].base_sprite);
        assert_eq!(world.bullets[0].velocity, [0.0; 2]);
        assert_eq!(
            (world.bullets[1].speed, world.bullets[1].sprite),
            (dead_speed, dead_sprite)
        );
        assert_eq!(world.effects.iter().flatten().count(), 1);
        assert_eq!(world.rng, rng);
        let mut expected = rng;
        expected.f32_zero_to_one();
        enemy.exec_ex(0, 1, &mut world);
        assert_eq!(world.rng, expected);
        assert_eq!(world.effects.iter().flatten().count(), 2);
        assert_eq!(world.bullets[0].ex_int0, 220);
        assert_eq!(world.bullets[1].ex_flags, 0);
    }

    #[test]
    fn misdirection_skips_despawning_bullets_without_drawing_rng() {
        let mut world = world();
        spawn_bullet_pattern(
            &mut world,
            &BulletProps {
                count1: 1,
                count2: 1,
                ..Default::default()
            },
        );
        world.bullets[0].height = 32.0;
        world.bullets[0].despawn_timer = 1;
        let rng = world.rng;
        Enemy::default().exec_ex(4, 2, &mut world);
        assert_eq!(world.rng, rng);
        assert_eq!(world.bullets[0].sprite_offset, 0);
    }

    #[test]
    fn timed_out_spell_end_does_not_cancel_replacement_pattern() {
        let mut world = world();
        world.spellcard_active = 2;
        spawn_bullet_pattern(
            &mut world,
            &BulletProps {
                count1: 1,
                count2: 1,
                ..Default::default()
            },
        );
        let ecl = Ecl {
            data: Vec::new(),
            sub_offsets: Vec::new(),
            timeline_offset: 0,
        };
        let mut enemy = Enemy::default();
        enemy.exec(
            &ecl,
            &mut world,
            &Instr {
                time: 0,
                opcode: 94,
                offset_to_next: 12,
                skip_for_difficulty: 31,
                args: &[],
            },
        );
        assert_eq!(world.spellcard_active, 0);
        assert_eq!(world.bullets[0].despawn_timer, 0);
        assert!(
            matches!(&world.events[0], WorldEvent::SpellcardEnd {points, bonus: 0} if points.is_empty())
        );
    }
}
