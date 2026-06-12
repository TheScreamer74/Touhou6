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
    /// Anchor at top-left (opcode 23) instead of the sprite center.
    pub corner: bool,
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
            corner: false,
        };
        runner.exec_ready();
        runner
    }

    pub fn visible(&self) -> bool {
        !self.dead && self.sprite.is_some()
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
        while !self.halted && !self.dead && self.pc < self.instrs.len() {
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
                3 => self.alpha = i.arg_u32(0) as f32 / 255.0,
                17 => {
                    self.pos = [i.arg_f32(0), i.arg_f32(1)];
                    self.moving = None;
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
