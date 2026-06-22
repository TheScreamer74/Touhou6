//! ECL virtual machine — a port of the original engine's enemy scripting,
//! following the th06 decompilation (EclManager.cpp / EnemyManager.cpp /
//! BulletManager.cpp) opcode by opcode. Coordinates are playfield-local
//! (384x448, origin top-left), matching the original.

use th06_formats::ecl::{Ecl, Instr};

pub const FIELD_W: f32 = 384.0;
pub const FIELD_H: f32 = 448.0;

/// The original 16-bit RNG (Rng.cpp).
pub struct Rng {
    seed: u16,
}

impl Rng {
    pub fn new(seed: u16) -> Self {
        Self { seed }
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
    pub fn f32_in_range(&mut self, range: f32) -> f32 {
        self.f32_zero_to_one() * range
    }
}

fn normalize_angle(mut a: f32) -> f32 {
    while a > std::f32::consts::PI {
        a -= std::f32::consts::TAU;
    }
    while a < -std::f32::consts::PI {
        a += std::f32::consts::TAU;
    }
    a
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
    pub pos: [f32; 2],
    pub angle: f32,
    pub speed: f32,
    /// etama3 sprite index (type base + color offset).
    pub sprite: u32,
    pub spawn_delay: u32,
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
    pub state: u8, // 0 warmup, 1 active, 2 despawning
    pub timer: i32,
    pub color: i32,
}

pub struct Enemy {
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
    pub max_life: i32,
    pub score: i32,
    boss_timer: i32,
    pub bullet_props: BulletProps,
    shoot_interval: i32,
    shoot_timer: i32,
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
    // ANM presentation
    pub anm_script: i32,
    pub anm_poses: [i16; 5], // default, far left, far right, left, right
    pub anm_pose_state: u8,  // 0xff = unset, 0 = default, 1 = left, 2 = right
    pub anm_dirty: bool,
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

impl Default for Enemy {
    fn default() -> Self {
        Self {
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
            rank_speed_low: 0.0,
            rank_speed_high: 0.0,
            rank_amount1_low: 0,
            rank_amount1_high: 0,
            rank_amount2_low: 0,
            rank_amount2_high: 0,
            life: 1,
            max_life: 1,
            score: 100,
            boss_timer: 0,
            bullet_props: BulletProps::default(),
            shoot_interval: 0,
            shoot_timer: 0,
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
            anm_script: -1,
            anm_poses: [-1; 5],
            anm_pose_state: 0xff,
            anm_dirty: false,
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
    SpellcardEnd,
    /// A non-timeout boss spell ran out its timer: capture is forfeit and the
    /// field is cleared (EnemyManager.cpp:408 — isCapturing=0, RemoveAllBullets).
    SpellTimeout,
    BulletCancel,
    BossSet(bool),
    EnemyDeath([f32; 2]),
    DropItem([f32; 2], i32),
}

/// Per-frame context the VM needs from the game.
pub struct World {
    pub rng: Rng,
    pub difficulty: u8, // 0 easy, 1 normal, 2 hard, 3 lunatic
    pub rank: i32,
    pub player_pos: [f32; 2],
    pub bullets: Vec<Bullet>,
    pub lasers: Vec<Laser>,
    pub events: Vec<WorldEvent>,
    pub pending_spawns: Vec<SpawnReq>,
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
}

impl World {
    fn alloc_laser(&mut self, laser: Laser) -> usize {
        if let Some(i) = self.lasers.iter().position(|l| !l.in_use) {
            self.lasers[i] = laser;
            i
        } else {
            self.lasers.push(laser);
            self.lasers.len() - 1
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
    pub fn spawn(ecl: &Ecl, world: &mut World, req: &SpawnReq) -> Option<Enemy> {
        let mut e = Enemy::default();
        if req.life >= 0 {
            e.life = req.life as i32;
        }
        e.pos = req.pos;
        e.invert_x = req.mirror;
        e.item_drop = req.item;
        e.ctx.pc = *ecl.sub_offsets.get(req.sub as usize)?;
        e.ctx.sub = req.sub as u16;
        e.ctx.time = 0;
        e.run_ecl(ecl, world);
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
            -10005 => self.ctx.fvars[0] as i32,
            -10006 => self.ctx.fvars[1] as i32,
            -10007 => self.ctx.fvars[2] as i32,
            -10008 => self.ctx.fvars[3] as i32,
            -10009 => self.ctx.ivars[4],
            -10010 => self.ctx.ivars[5],
            -10011 => self.ctx.ivars[6],
            -10012 => self.ctx.ivars[7],
            -10013 => world.difficulty as i32,
            -10014 => world.rank,
            -10022 => self.boss_timer,
            -10024 => self.life,
            -10025 => 0, // player shot type (Reimu A)
            other => other,
        }
    }

    /// Resolve a possibly-variable f32 argument. Variable IDs arrive as the
    /// integer bit pattern of small negative integers.
    fn get_f32(&self, raw: f32, world: &World) -> f32 {
        let as_int = raw as i32;
        if raw != raw.floor() || !(-10025..=-10001).contains(&as_int) {
            return raw;
        }
        match as_int {
            -10001 => self.ctx.ivars[0] as f32,
            -10002 => self.ctx.ivars[1] as f32,
            -10003 => self.ctx.ivars[2] as f32,
            -10004 => self.ctx.ivars[3] as f32,
            -10005 => self.ctx.fvars[0],
            -10006 => self.ctx.fvars[1],
            -10007 => self.ctx.fvars[2],
            -10008 => self.ctx.fvars[3],
            -10009 => self.ctx.ivars[4] as f32,
            -10010 => self.ctx.ivars[5] as f32,
            -10011 => self.ctx.ivars[6] as f32,
            -10012 => self.ctx.ivars[7] as f32,
            -10013 => world.difficulty as f32,
            -10014 => world.rank as f32,
            -10015 => self.pos[0],
            -10016 => self.pos[1],
            -10017 => self.pos[2],
            -10018 => world.player_pos[0],
            -10019 => world.player_pos[1],
            -10020 => 0.0,
            -10021 => self.angle_to_player(world),
            -10023 => {
                let dx = world.player_pos[0] - self.pos[0];
                let dy = world.player_pos[1] - self.pos[1];
                (dx * dx + dy * dy).sqrt()
            }
            _ => raw,
        }
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

    fn is_float_var(id: i32) -> bool {
        matches!(id, -10008..=-10005 | -10021..=-10015 | -10023)
    }

    fn angle_to_player(&self, world: &World) -> f32 {
        (world.player_pos[1] - self.pos[1]).atan2(world.player_pos[0] - self.pos[0])
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
                    self.axis_speed = [self.angle.cos() * self.speed, self.angle.sin() * self.speed];
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
                // EXINSREPEAT's per-frame callback (EclManager.cpp:1034).
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
            0 => {} // NOP
            1 => return Flow::Kill, // UNIMP: end of script
            2 => return self.jump(instr), // JUMP
            3 => {
                // JUMPDEC
                let v = self.get_i32(instr.arg_i32(2), world) - 1;
                self.set_var(instr.arg_i32(2), v, v as f32);
                if v > 0 {
                    return self.jump(instr);
                }
            }
            4 => {
                // SETINT
                let raw = instr.arg_i32(1);
                let v = self.get_i32(raw, world);
                self.set_var(instr.arg_i32(0), v, v as f32);
            }
            5 => {
                // SETFLOAT
                let v = self.get_f32(instr.arg_f32(1), world);
                self.set_var(instr.arg_i32(0), v as i32, v);
            }
            6 => {
                // SETINTRAND
                let range = self.get_i32(instr.arg_i32(1), world);
                let v = world.rng.u32_in_range(range.max(0) as u32) as i32;
                self.set_var(instr.arg_i32(0), v, v as f32);
            }
            7 => {
                // SETINTRANDMIN
                let range = self.get_i32(instr.arg_i32(1), world);
                let min = self.get_i32(instr.arg_i32(2), world);
                let v = world.rng.u32_in_range(range.max(0) as u32) as i32 + min;
                self.set_var(instr.arg_i32(0), v, v as f32);
            }
            8 => {
                // SETFLOATRAND
                let range = self.get_f32(instr.arg_f32(1), world);
                let v = world.rng.f32_in_range(range);
                self.set_var(instr.arg_i32(0), v as i32, v);
            }
            9 => {
                // SETFLOATRANDMIN
                let range = self.get_f32(instr.arg_f32(1), world);
                let min = self.get_f32(instr.arg_f32(2), world);
                let v = world.rng.f32_in_range(range) + min;
                self.set_var(instr.arg_i32(0), v as i32, v);
            }
            10 => self.set_var(instr.arg_i32(0), self.pos[0] as i32, self.pos[0]),
            11 => self.set_var(instr.arg_i32(0), self.pos[1] as i32, self.pos[1]),
            12 => self.set_var(instr.arg_i32(0), self.pos[2] as i32, self.pos[2]),
            13..=17 => self.alu_int(instr, world),   // int add/sub/mul/div/mod
            18 => {
                // MATHINC
                let v = self.get_i32(instr.arg_i32(0), world) + 1;
                self.set_var(instr.arg_i32(0), v, v as f32);
            }
            19 => {
                // MATHDEC
                let v = self.get_i32(instr.arg_i32(0), world) - 1;
                self.set_var(instr.arg_i32(0), v, v as f32);
            }
            20..=24 => self.alu_float(instr, world), // float add/sub/mul/div/mod
            25 => {
                // MATHATAN2: angle from point (arg1,arg2) to point (arg3,arg4).
                // Decomp result = atan2f(arg4 - arg2, arg3 - arg1)
                // (EnemyEclInstr.cpp:396, after the var_order shuffle).
                let x1 = self.get_f32(instr.arg_f32(1), world);
                let y1 = self.get_f32(instr.arg_f32(2), world);
                let x2 = self.get_f32(instr.arg_f32(3), world);
                let y2 = self.get_f32(instr.arg_f32(4), world);
                let v = (y2 - y1).atan2(x2 - x1);
                self.set_var(instr.arg_i32(0), v as i32, v);
            }
            26 => {
                // MATHNORMANGLE
                let v = normalize_angle(self.get_f32(instr.arg_f32(0), world));
                self.set_var(instr.arg_i32(0), v as i32, v);
            }
            27 => {
                let l = self.get_i32(instr.arg_i32(0), world);
                let r = self.get_i32(instr.arg_i32(1), world);
                self.ctx.cmp = if l == r { 0 } else if l < r { -1 } else { 1 };
            }
            28 => {
                let l = self.get_f32(instr.arg_f32(0), world);
                let r = self.get_f32(instr.arg_f32(1), world);
                self.ctx.cmp = if l == r { 0 } else if l < r { -1 } else { 1 };
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
                    s1 += rank_scale_f32(self.rank_speed_low, self.rank_speed_high, world.rank as f32);
                    if s1 < 0.3 {
                        s1 = 0.3;
                    }
                }
                let mut s2 = self.get_f32(speed2, world)
                    + rank_scale_f32(self.rank_speed_low, self.rank_speed_high, world.rank as f32) / 2.0;
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
                let pos = [self.pos[0] + self.shoot_offset[0], self.pos[1] + self.shoot_offset[1]];
                self.bullet_props.pos = pos;
                if !self.shooting_disabled {
                    let props = std::mem::take(&mut self.bullet_props);
                    spawn_bullet_pattern(world, &props);
                    self.bullet_props = props;
                }
            }
            76 => {
                // SHOOTINTERVAL
                self.shoot_interval = instr.arg_i32(0) + self.shoot_interval_rank(world.rank);
                self.shoot_timer = 0;
            }
            77 => {
                // SHOOTINTERVALDELAYED
                self.shoot_interval = instr.arg_i32(0) + self.shoot_interval_rank(world.rank);
                if self.shoot_interval != 0 {
                    self.shoot_timer = world.rng.u32_in_range(self.shoot_interval as u32) as i32;
                }
            }
            78 => self.shooting_disabled = true,
            79 => self.shooting_disabled = false,
            80 => {
                // SHOOTNOW
                self.bullet_props.pos =
                    [self.pos[0] + self.shoot_offset[0], self.pos[1] + self.shoot_offset[1]];
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
            83 => world.events.push(WorldEvent::BulletCancel),
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
                let mut angle = self.get_f32(instr.arg_f32(1), world);
                if instr.opcode == 86 {
                    angle += self.angle_to_player(world);
                }
                let laser = Laser {
                    in_use: true,
                    pos: [self.pos[0] + self.shoot_offset[0], self.pos[1] + self.shoot_offset[1]],
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
                if let Some(idx) = self.lasers[slot] {
                    if let Some(l) = world.lasers.get_mut(idx) {
                        l.angle += delta;
                    }
                }
            }
            89 => {
                // LASERROTATEFROMPLAYER
                let slot = instr.arg_i32(0).clamp(0, 31) as usize;
                let delta = self.get_f32(instr.arg_f32(1), world);
                if let Some(idx) = self.lasers[slot] {
                    let player = world.player_pos;
                    if let Some(l) = world.lasers.get_mut(idx) {
                        l.angle = (player[1] - l.pos[1]).atan2(player[0] - l.pos[0]) + delta;
                    }
                }
            }
            90 => {
                // LASEROFFSET
                let slot = instr.arg_i32(0).clamp(0, 31) as usize;
                let dx = instr.arg_f32(1);
                let dy = instr.arg_f32(2);
                if let Some(idx) = self.lasers[slot] {
                    if let Some(l) = world.lasers.get_mut(idx) {
                        l.pos = [self.pos[0] + dx, self.pos[1] + dy];
                    }
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
                if let Some(idx) = self.lasers[slot] {
                    if let Some(l) = world.lasers.get_mut(idx) {
                        if l.in_use && l.state < 2 {
                            l.state = 2;
                            l.timer = 0;
                        }
                    }
                }
            }
            134 => self.lasers = [None; 32], // LASERCLEARALL
            93 => {
                // SPELLCARDSTART (EclRawInstrSpellcardStartArgs): portrait sprite
                // at byte 0, id at byte 2, Shift-JIS name from byte 4.
                let sprite = instr.arg_i16(0) as i32;
                let id = instr.arg_i16(2) as i32;
                let name = instr.args.get(4..).map(|b| {
                    let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
                    b[..end].to_vec()
                }).unwrap_or_default();
                world.events.push(WorldEvent::SpellcardStart(id, sprite, name));
                world.events.push(WorldEvent::BulletCancel);
                self.rank_speed_low = -0.5;
                self.rank_speed_high = 0.5;
                self.rank_amount1_low = 0;
                self.rank_amount1_high = 0;
                self.rank_amount2_low = 0;
                self.rank_amount2_high = 0;
            }
            94 => world.events.push(WorldEvent::SpellcardEnd),
            95 => {
                // ENEMYCREATE
                world.pending_spawns.push(SpawnReq {
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
                });
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
            99 | 100 => {} // sub-slot anm, death anm
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
            102 => {} // spellcard visual effect
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
            118 => {} // EFFECTPARTICLE — effect system pending
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
            120 => {} // ANMFLAGROTATION
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
                world
                    .events
                    .push(WorldEvent::DropItem([self.pos[0], self.pos[1]], instr.arg_i32(0)));
            }
            125 => {} // STDUNPAUSE
            126 => self.spell_count = instr.arg_i32(0), // BOSSSETLIFECOUNT (gui)
            127 => {} // DEBUGWATCH
            128 | 129 => {} // ANMINTERRUPTMAIN / SLOT — anm interrupts pending
            op if std::env::var_os("TH06_TRACE_OP").is_some() => { eprintln!("unhandled ECL op {op}"); }
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
            _ => {}
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
                for b in world.bullets.iter_mut() {
                    match param {
                        0 => b.speed = 0.0,
                        1 => {
                            b.ex_flags |= 0x10;
                            b.ex_int0 = 220;
                            b.timer = 0;
                            let a = world.rng.f32_zero_to_one() * TAU - PI;
                            b.ex_accel = [a.cos() * 0.01, a.sin() * 0.01];
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
                        (self.star_player_pos[0] - self.star_enemy_pos[0]) * td0 + self.star_enemy_pos[0],
                        (self.star_player_pos[1] - self.star_enemy_pos[1]) * td0 + self.star_enemy_pos[1],
                    ];
                    let pp = pattern_pos + 0.5;
                    self.bullet_props.angle1 = (PI / 3.0) * pp;
                    let r = self.ctx.fvars[3];
                    for i in 0..5 {
                        let td = (var2 % 30) as f32 / 30.0;
                        let a0 = self.star_angle[i];
                        let a1 = self.star_angle[i + 1];
                        let t0 = [a0.cos() * r, a0.sin() * r];
                        let t1 = [a1.cos() * r, a1.sin() * r];
                        let tt = [(t1[0] - t0[0]) * td + t0[0], (t1[1] - t0[1]) * td + t0[1]];
                        self.bullet_props.pos = [base[0] + tt[0], base[1] + tt[1]];
                        let backup = self.bullet_props.speed1;
                        self.bullet_props.speed1 =
                            world.rng.f32_in_range(self.bullet_props.speed2) + self.bullet_props.speed1;
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
                // (isTimeStopped = u8Param). The param >= 2 bullet-redirect branch
                // needs per-bullet sprite-height tests our bullets lack — skipped.
                if param < 2 {
                    world.time_stopped = (param & 0xff) != 0;
                }
                self.ctx.ivars[2] = 0; // var2 = 0
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
                        self.bullet_props.pos =
                            [base_angle.cos() * r + 192.0, base_angle.sin() * r + 224.0];
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
                let remaining = if self.boss_timer >= 7200 { 0 } else { self.life };
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
            // idx 5,6,7,8,9,10,11,12,14,15 need subsystems this port lacks
            // (effects, anm-interrupts, per-bullet sprite-height tests,
            // laser-segment spawning); left unimplemented per the no-approx rule.
            _ => {}
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

    fn alu_int(&mut self, instr: &Instr, world: &World) {
        let out = instr.arg_i32(0);
        let a = self.get_i32(instr.arg_i32(1), world);
        let b = self.get_i32(instr.arg_i32(2), world);
        let v = match instr.opcode {
            13 => a.wrapping_add(b),
            14 => a.wrapping_sub(b),
            15 => a.wrapping_mul(b),
            16 => if b != 0 { a / b } else { 0 },
            _ => if b != 0 { a % b } else { 0 },
        };
        if Self::is_float_var(out) {
            self.set_var(out, v, v as f32);
        } else {
            self.set_var(out, v, v as f32);
        }
    }

    fn alu_float(&mut self, instr: &Instr, world: &World) {
        let out = instr.arg_i32(0);
        let a = self.get_f32(instr.arg_f32(1), world);
        let b = self.get_f32(instr.arg_f32(2), world);
        let v = match instr.opcode {
            20 => a + b,
            21 => a - b,
            22 => a * b,
            23 => if b != 0.0 { a / b } else { 0.0 },
            _ => if b != 0.0 { a % b } else { 0.0 },
        };
        self.set_var(out, v as i32, v);
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
        // Pose selection from horizontal motion (left/right banking).
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

    fn clamp(&mut self) {
        if self.clamp_pos {
            self.pos[0] = self.pos[0].clamp(self.lower_bound[0], self.upper_bound[0]);
            self.pos[1] = self.pos[1].clamp(self.lower_bound[1], self.upper_bound[1]);
        }
    }

    /// Bounds bookkeeping; returns false when the enemy should despawn.
    pub fn update_bounds(&mut self) -> bool {
        let margin = 32.0;
        let inside = self.pos[0] + margin >= 0.0
            && self.pos[0] - margin <= FIELD_W
            && self.pos[1] + margin >= 0.0
            && self.pos[1] - margin <= FIELD_H;
        if !self.has_been_in_bounds && inside {
            self.has_been_in_bounds = true;
        }
        !(self.has_been_in_bounds && !inside)
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
                // Timed out a damageable spell: forfeit capture + clear bullets.
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
    }

    pub fn on_death(&mut self, ecl: &Ecl, world: &mut World) {
        self.life_cb_threshold = -1;
        self.timer_cb_threshold = -1;
        world.events.push(WorldEvent::EnemyDeath([self.pos[0], self.pos[1]]));
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
        if self.death_mode != 3 {
            self.life = 0;
        }
        if self.is_boss && (self.death_mode == 0 || self.death_mode == 1) {
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

    /// Remaining seconds of the current boss attack, for the HUD timer.
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
pub const BULLET_BASE_SPRITE: [u32; 10] = [14, 30, 46, 62, 78, 94, 110, 118, 122, 146];

/// Faithful port of BulletManager::SpawnSingleBullet's aim-mode math.
pub fn spawn_bullet_pattern(world: &mut World, props: &BulletProps) {
    let aim_angle = (world.player_pos[1] - props.pos[1]).atan2(world.player_pos[0] - props.pos[0]);
    let tau = std::f32::consts::TAU;
    let pi = std::f32::consts::PI;
    for idx2 in 0..props.count2 as i32 {
        for idx1 in 0..props.count1 as i32 {
            let mut angle = 0.0f32;
            let mut speed = props.speed1
                - (props.speed1 - props.speed2) * idx2 as f32 / props.count2 as f32;
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
            angle = normalize_angle(angle);
            let base = BULLET_BASE_SPRITE
                .get(props.sprite.max(0) as usize)
                .copied()
                .unwrap_or(30);
            // Spawn-effect flags delay the live bullet slightly in the
            // original; approximated as a fixed delay.
            let spawn_delay = if props.flags & (2 | 4 | 8) != 0 { 8 } else { 0 };
            // Ex-behavior setup, ported from SpawnSingleBullet.
            let mut ex_accel = [0.0f32; 2];
            let mut ex_f = [0.0f32; 2];
            let mut ex_int0 = 0;
            let mut ex_int1 = 0;
            if props.flags & 0x10 != 0 {
                let dir = if props.ex_floats[1] <= -999.0 { angle } else { props.ex_floats[1] };
                ex_accel = [dir.cos() * props.ex_floats[0], dir.sin() * props.ex_floats[0]];
                ex_int0 = if props.ex_ints[0] > 0 { props.ex_ints[0] } else { 99999 };
            } else if props.flags & 0x20 != 0 {
                ex_f = [props.ex_floats[0], props.ex_floats[1]];
                ex_int0 = props.ex_ints[0];
            }
            if props.flags & 0x1c0 != 0 {
                // Direction-change group (0x40 rotate / 0x80 aim-at-player /
                // 0x100 absolute): shared dirChange* setup.
                ex_f = [
                    props.ex_floats[0],
                    if props.ex_floats[1] >= 0.0 { props.ex_floats[1] } else { speed },
                ];
                ex_int0 = props.ex_ints[0];
                ex_int1 = props.ex_ints[1];
            }
            world.bullets.push(Bullet {
                pos: props.pos,
                angle,
                speed,
                sprite: base + props.sprite_offset.max(0) as u32,
                spawn_delay,
                timer: 0,
                ex_flags: props.flags,
                ex_accel,
                ex_f,
                ex_int0,
                ex_int1,
                ex_count: 0,
                grazed: false,
            });
        }
    }
    if props.flags & 0x200 != 0 && props.sfx >= 0 {
        world.events.push(WorldEvent::Sfx(props.sfx));
    }
}
