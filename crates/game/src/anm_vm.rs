//! Frame-stepped interpreter for th06 ANM scripts.
//!
//! Each runner owns one script instance: it executes instructions when the
//! script clock reaches their time, halts at stop opcodes (21/24) and
//! resumes at interrupt labels (22) when the game fires an interrupt.

use th06_formats::anm0::Instr;

#[derive(Clone, Copy)]
enum Ease {
    Linear,
    Decel,
    Accel,
}

#[derive(Clone, Copy)]
struct Move {
    from: [f32; 2],
    to: [f32; 2],
    start: u16,
    duration: u16,
    ease: Ease,
}

pub struct AnmRunner {
    instrs: Vec<Instr>,
    pc: usize,
    clock: u16,
    halted: bool,
    dead: bool,
    moving: Option<Move>,
    pub sprite: Option<u32>,
    pub pos: [f32; 2],
    pub alpha: f32,
    /// RGB tint (opcode 4 SetColor), 0..1 per channel. Decomp keeps it in the
    /// same D3DCOLOR as alpha; here it is split out. Default white (no tint).
    pub color: [f32; 3],
    pub scale: [f32; 2],
    /// Scale interpolation (AnmManager scaleInterp*): opcode 30 (ScaleTime)
    /// ramps `scale` from `scale_init` toward `scale_final` over `scale_end`
    /// frames from `scale_start`; opcode 11 (SetScaleSpeed) just presets the
    /// target with `scale_end = 0` (no ramp), matching the decomp.
    scale_final: [f32; 2],
    scale_init: [f32; 2],
    scale_end: u16,
    scale_start: u16,
    /// Z rotation in radians (opcode 9), integrated by `angle_vel` (opcode 10).
    pub rotation: f32,
    angle_vel: f32,
    /// opcode 12 Fade: (from_alpha, to_alpha, start_time, duration).
    fade: Option<(f32, f32, u16, u16)>,
    /// Anchor at top-left (opcode 23) instead of the sprite center.
    pub corner: bool,
    /// Horizontal mirror (opcode 7).
    pub flip_x: bool,
    /// Vertical mirror (opcode 8 FlipY).
    pub flip_y: bool,
    /// Explicit hide (opcode 29 SetVisibility 0); cleared when a sprite is set
    /// (SetActiveSprite / SetRandomSprite set isVisible=1 in the decomp).
    hidden: bool,
    /// SetAutoRotate (opcode 26): the draw rotates this sprite to its owner's
    /// angle instead of using the runner's own rotation.
    pub auto_rotate: bool,
    /// SetRandomSprite (opcode 16): (base, range) awaiting an RNG draw. The
    /// decomp resolves the sprite as `base + GetRandomU16InRange(range)`
    /// (AnmManager.cpp:1066), drawing from the shared game RNG; the owner draws
    /// it right after ticking so the RNG stream stays in step with the decomp.
    pending_random: Option<(u32, u16)>,
    /// op13/14 blend mode: true = additive (One) glow, e.g. the etama4 particle
    /// glows and laser base balls. Drawn with SrcAlpha/One instead of alpha.
    pub additive: bool,
    /// True once the script set its own position (opcode 17) — distinguishes
    /// self-placing HUD labels from elements the game positions each frame.
    pub positioned: bool,
}

impl AnmRunner {
    pub fn new(instrs: Vec<Instr>) -> Self {
        let mut runner = Self {
            instrs,
            pc: 0,
            clock: 0,
            halted: false,
            dead: false,
            moving: None,
            sprite: None,
            pos: [0.0, 0.0],
            alpha: 1.0,
            color: [1.0, 1.0, 1.0],
            scale: [1.0, 1.0],
            scale_final: [0.0, 0.0],
            scale_init: [1.0, 1.0],
            scale_end: 0,
            scale_start: 0,
            rotation: 0.0,
            angle_vel: 0.0,
            fade: None,
            corner: false,
            flip_x: false,
            flip_y: false,
            hidden: false,
            auto_rotate: false,
            pending_random: None,
            additive: false,
            positioned: false,
        };
        runner.exec_ready();
        runner
    }

    pub fn visible(&self) -> bool {
        !self.dead && self.sprite.is_some() && !self.hidden
    }

    /// Take a pending SetRandomSprite (opcode 16) request: (base, range). The
    /// owner resolves the sprite as `base + rng.u16_in_range(range)` right after
    /// ticking, keeping the shared game RNG in step with the decomp.
    pub fn take_pending_random(&mut self) -> Option<(u32, u16)> {
        self.pending_random.take()
    }

    /// The script reached its end opcode (0) — the decomp frees the owning
    /// effect when `ExecuteScript` reports this.
    pub fn ended(&self) -> bool {
        self.dead
    }

    /// Jump to the section after interrupt label `n` and resume.
    pub fn interrupt(&mut self, n: u32) {
        if self.dead {
            return;
        }
        if let Some(idx) = self
            .instrs
            .iter()
            .position(|i| i.opcode == 22 && !i.args.is_empty() && i.arg_u32(0) == n)
        {
            self.pc = idx + 1;
            self.clock = self.instrs[idx].time;
            self.halted = false;
            self.moving = None;
            self.exec_ready();
        }
    }

    pub fn tick(&mut self) {
        if self.dead {
            return;
        }
        self.clock = self.clock.saturating_add(1);
        self.exec_ready();
        self.rotation += self.angle_vel;
        if self.scale_end > 0 {
            let t = self.clock.saturating_sub(self.scale_start);
            if t >= self.scale_end {
                self.scale = self.scale_final;
                self.scale_end = 0;
            } else {
                let f = t as f32 / self.scale_end as f32;
                self.scale = [
                    self.scale_init[0] + (self.scale_final[0] - self.scale_init[0]) * f,
                    self.scale_init[1] + (self.scale_final[1] - self.scale_init[1]) * f,
                ];
            }
        }
        if let Some((from, to, start, dur)) = self.fade {
            let t = self.clock.saturating_sub(start);
            if t >= dur {
                self.alpha = to;
                self.fade = None;
            } else {
                self.alpha = from + (to - from) * (t as f32 / dur as f32);
            }
        }
        if let Some(m) = self.moving {
            let t = self.clock.saturating_sub(m.start);
            if t >= m.duration {
                self.pos = m.to;
                self.moving = None;
            } else {
                let f = t as f32 / m.duration as f32;
                let f = match m.ease {
                    Ease::Linear => f,
                    Ease::Decel => 1.0 - (1.0 - f) * (1.0 - f),
                    Ease::Accel => f * f,
                };
                self.pos = [
                    m.from[0] + (m.to[0] - m.from[0]) * f,
                    m.from[1] + (m.to[1] - m.from[1]) * f,
                ];
            }
        }
    }

    fn exec_ready(&mut self) {
        let mut budget = 1000; // guards against zero-frame jump loops
        while !self.halted && !self.dead && self.pc < self.instrs.len() {
            budget -= 1;
            if budget == 0 {
                break;
            }
            let i = &self.instrs[self.pc];
            if i.time > self.clock {
                break;
            }
            match i.opcode {
                0 => {
                    // Script end: the sprite is removed.
                    self.dead = true;
                }
                15 => {
                    // Freeze in place, still visible.
                    self.halted = true;
                }
                1 => {
                    // SetActiveSprite also sets isVisible=1 (AnmManager.cpp:1059).
                    self.sprite = Some(i.arg_u32(0));
                    self.hidden = false;
                }
                2 => self.scale = [i.arg_f32(0), i.arg_f32(1)],
                // SetScaleSpeed: preset the interp target with no ramp (endTime 0).
                11 => {
                    self.scale_final = [i.arg_f32(0), i.arg_f32(1)];
                    self.scale_end = 0;
                }
                // ScaleTime: ramp scale from the current value to the target over
                // arg(2) frames (AnmManager scaleInterp, AnmManager.cpp:1113).
                30 => {
                    self.scale_final = [i.arg_f32(0), i.arg_f32(1)];
                    self.scale_end = i.arg_u32(2) as u16;
                    self.scale_init = self.scale;
                    self.scale_start = self.clock;
                }
                3 => self.alpha = i.arg_u32(0) as f32 / 255.0,
                4 => {
                    // SetColor: RGB from the arg's low 24 bits (D3DCOLOR 0xAARRGGBB),
                    // alpha kept separately (op3).
                    let v = i.arg_u32(0);
                    self.color = [
                        ((v >> 16) & 0xff) as f32 / 255.0,
                        ((v >> 8) & 0xff) as f32 / 255.0,
                        (v & 0xff) as f32 / 255.0,
                    ];
                }
                5 => {
                    // Jump: arg is a byte offset from script start; the
                    // clock snaps to the target instruction's time.
                    let target = i.arg_u32(0);
                    if let Some(idx) = self.instrs.iter().position(|x| x.offset == target) {
                        self.pc = idx;
                        self.clock = self.instrs[idx].time;
                        continue;
                    }
                }
                7 => self.flip_x = !self.flip_x,
                8 => self.flip_y = !self.flip_y,
                9 => self.rotation = i.arg_f32(2),
                10 => self.angle_vel = i.arg_f32(2),
                12 => {
                    self.fade = Some((self.alpha, i.arg_u32(0) as f32 / 255.0, i.time, i.arg_u32(1) as u16));
                }
                17 => {
                    self.pos = [i.arg_f32(0), i.arg_f32(1)];
                    self.moving = None;
                    self.positioned = true;
                }
                18 | 19 | 20 => {
                    let ease = match i.opcode {
                        18 => Ease::Linear,
                        19 => Ease::Decel,
                        _ => Ease::Accel,
                    };
                    self.moving = Some(Move {
                        from: self.pos,
                        to: [i.arg_f32(0), i.arg_f32(1)],
                        start: i.time,
                        duration: i.arg_u32(3).max(1) as u16,
                        ease,
                    });
                }
                21 | 24 => {
                    self.pc += 1;
                    self.halted = true;
                    return;
                }
                23 => self.corner = true,
                13 => self.additive = true,
                14 => self.additive = false,
                // SetAutoRotate: enemy sub-vm sprites with this flag are drawn
                // rotated to the owning enemy's angle (EnemyManager.cpp:792).
                26 => self.auto_rotate = i.arg_u32(0) != 0,
                // SetRandomSprite: defer the RNG draw to the owner (resolve_random)
                // so the shared game-RNG stream stays ordered like the decomp.
                // Also sets isVisible=1 (AnmManager.cpp:1064).
                16 => {
                    self.pending_random = Some((i.arg_u32(0), i.arg_u32(1) as u16));
                    self.hidden = false;
                }
                // SetVisibility: isVisible = args[0].
                29 => self.hidden = i.arg_u32(0) == 0,
                _ => {}
            }
            self.pc += 1;
        }
    }
}
