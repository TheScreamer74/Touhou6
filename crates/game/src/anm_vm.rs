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
    pub scale: [f32; 2],
    /// Per-frame scale velocity (opcode 11 SetScaleSpeed), added to `scale`.
    scale_vel: [f32; 2],
    /// Z rotation in radians (opcode 9), integrated by `angle_vel` (opcode 10).
    pub rotation: f32,
    angle_vel: f32,
    /// opcode 12 Fade: (from_alpha, to_alpha, start_time, duration).
    fade: Option<(f32, f32, u16, u16)>,
    /// Anchor at top-left (opcode 23) instead of the sprite center.
    pub corner: bool,
    /// Horizontal mirror (opcode 7).
    pub flip_x: bool,
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
            scale: [1.0, 1.0],
            scale_vel: [0.0, 0.0],
            rotation: 0.0,
            angle_vel: 0.0,
            fade: None,
            corner: false,
            flip_x: false,
            positioned: false,
        };
        runner.exec_ready();
        runner
    }

    pub fn visible(&self) -> bool {
        !self.dead && self.sprite.is_some()
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
        self.scale[0] += self.scale_vel[0];
        self.scale[1] += self.scale_vel[1];
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
                1 => self.sprite = Some(i.arg_u32(0)),
                2 => self.scale = [i.arg_f32(0), i.arg_f32(1)],
                11 => self.scale_vel = [i.arg_f32(0), i.arg_f32(1)],
                3 => self.alpha = i.arg_u32(0) as f32 / 255.0,
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
                _ => {}
            }
            self.pc += 1;
        }
    }
}
