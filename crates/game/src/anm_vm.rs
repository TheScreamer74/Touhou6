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
    elapsed: u16,
    duration: u16,
    ease: Ease,
}

#[derive(Clone)]
pub struct AnmRunner {
    instrs: Vec<Instr>,
    pc: usize,
    clock: u16,
    halted: bool,
    dead: bool,
    static_sprite: bool,
    pending_interrupt: Option<u32>,
    moving: Option<Move>,
    pub sprite: Option<u32>,
    pub pos: [f32; 2],
    pub pos_offset: [f32; 2],
    use_pos_offset: bool,
    pub alpha: f32,
    /// RGB tint (opcode 4 SetColor), 0..1 per channel. Decomp keeps it in the
    /// same D3DCOLOR as alpha; here it is split out. Default white (no tint).
    pub color: [f32; 3],
    pub scale: [f32; 2],
    /// Scale interpolation (AnmManager scaleInterp*): opcode 30 (ScaleTime)
    /// ramps `scale` from `scale_init` toward `scale_final` over `scale_end`
    /// frames; opcode 11 is the per-frame scale speed when no ramp is active.
    scale_final: [f32; 2],
    scale_init: [f32; 2],
    scale_end: u16,
    scale_time: u16,
    /// Z rotation in radians (opcode 9), integrated by `angle_vel` (opcode 10).
    pub rotation: f32,
    angle_vel: f32,
    /// opcode 12 Fade snapshots the complete D3DCOLOR, including RGB. An owner
    /// changing VM color afterward does not change those interpolation endpoints.
    fade: Option<([f32; 4], [f32; 4], u16, u16)>,
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
    /// D3DTOP_ADD combines texture RGB with the VM color, preserving the
    /// normal texture-alpha × VM-alpha calculation.
    pub color_add: bool,
    /// ANM31 disables depth writes while keeping the current depth comparison.
    pub z_write_disable: bool,
    /// Texture-coordinate scrolling from ANM opcodes27/28.
    pub uv_scroll: [f32; 2],
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
            static_sprite: false,
            pending_interrupt: None,
            moving: None,
            sprite: None,
            pos: [0.0, 0.0],
            pos_offset: [0.0, 0.0],
            use_pos_offset: false,
            alpha: 1.0,
            color: [1.0, 1.0, 1.0],
            scale: [1.0, 1.0],
            scale_final: [0.0, 0.0],
            scale_init: [1.0, 1.0],
            scale_end: 0,
            scale_time: 0,
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
            color_add: false,
            z_write_disable: false,
            uv_scroll: [0.0; 2],
            positioned: false,
        };
        // SetAndExecuteScript executes frame zero immediately, including the
        // interpolation work, then advances currentTimeInScript.
        runner.tick();
        runner
    }

    pub fn visible(&self) -> bool {
        !self.dead && self.sprite.is_some() && !self.hidden
    }

    /// ANM's stop flag remains set until an interrupt resumes the script.
    pub fn stopped(&self) -> bool {
        self.halted
    }

    /// AnmManager::SetActiveSprite, also used by Gui's MSG expression opcode.
    pub fn set_sprite(&mut self, sprite: u32) {
        self.sprite = Some(sprite);
        self.hidden = false;
    }

    /// Assign the owner's D3DCOLOR after SetAndExecuteScript, as effects do.
    pub fn set_rgba(&mut self, rgba: [f32; 4]) {
        self.color = [rgba[0], rgba[1], rgba[2]];
        self.alpha = rgba[3];
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
        self.dead || self.static_sprite
    }

    /// Jump to the section after interrupt label `n` and resume.
    pub fn interrupt(&mut self, n: u32) {
        if !self.dead && !self.static_sprite && n != 0 {
            self.pending_interrupt = Some(n);
        }
    }

    pub fn tick(&mut self) {
        if self.dead || self.static_sprite {
            return;
        }
        self.exec_ready();
        if self.dead || self.static_sprite {
            return;
        }
        self.rotation = crate::ecl_vm::normalize_angle(self.rotation + self.angle_vel);
        if self.scale_end > 0 {
            self.scale_time = self.scale_time.saturating_add(1);
            let t = self.scale_time;
            if t >= self.scale_end {
                self.scale = self.scale_final;
                self.scale_end = 0;
                self.scale_final = [0.0; 2];
            } else {
                let f = t as f32 / self.scale_end as f32;
                self.scale = [
                    self.scale_init[0] + (self.scale_final[0] - self.scale_init[0]) * f,
                    self.scale_init[1] + (self.scale_final[1] - self.scale_init[1]) * f,
                ];
            }
        } else {
            self.scale[0] += self.scale_final[0];
            self.scale[1] += self.scale_final[1];
        }
        if let Some((from, to, elapsed, dur)) = self.fade {
            let t = elapsed.saturating_add(1);
            let factor = (t as f32 / dur as f32).min(1.0);
            // ExecuteScript interpolates all four saved 8-bit components,
            // truncating each result. This can restore the saved RGB after an
            // owner assigns another tint while the fade is running.
            self.set_rgba(std::array::from_fn(|k| {
                ((from[k] * 255.0 + (to[k] - from[k]) * 255.0 * factor) as i32).clamp(0, 255) as f32
                    / 255.0
            }));
            self.fade = (t < dur).then_some((from, to, t, dur));
        }
        if let Some(mut m) = self.moving {
            let t = m.elapsed;
            let target = if self.use_pos_offset {
                &mut self.pos_offset
            } else {
                &mut self.pos
            };
            if t >= m.duration {
                *target = m.to;
                self.moving = None;
            } else {
                let f = t as f32 / m.duration as f32;
                let f = match m.ease {
                    Ease::Linear => f,
                    Ease::Decel => 1.0 - (1.0 - f) * (1.0 - f),
                    Ease::Accel => 1.0 - (1.0 - f).powi(4),
                };
                *target = [
                    f * m.to[0] + (1.0 - f) * m.from[0],
                    f * m.to[1] + (1.0 - f) * m.from[1],
                ];
                m.elapsed = m.elapsed.saturating_add(1);
                self.moving = Some(m);
            }
        }
        if !self.halted {
            self.clock = self.clock.saturating_add(1);
        }
    }

    fn exec_ready(&mut self) {
        if let Some(interrupt) = self.pending_interrupt.take() {
            let mut fallback = None;
            let mut found = None;
            for (idx, instr) in self.instrs.iter().enumerate() {
                if matches!(instr.opcode, 0 | 15) {
                    break;
                }
                if instr.opcode == 22 && !instr.args.is_empty() {
                    let label = instr.arg_u32(0);
                    if label == interrupt {
                        found = Some(idx);
                        break;
                    }
                    if label == u32::MAX {
                        fallback = Some(idx);
                    }
                }
            }
            if let Some(idx) = found.or(fallback) {
                self.pc = idx + 1;
                if let Some(next) = self.instrs.get(self.pc) {
                    self.clock = next.time;
                }
                self.halted = false;
                self.hidden = false;
            }
        }
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
                    // ANM_STATIC ends execution while preserving its last sprite.
                    self.halted = true;
                    self.static_sprite = true;
                    return;
                }
                1 => {
                    // SetActiveSprite also sets isVisible=1 (AnmManager.cpp:1059).
                    self.sprite = Some(i.arg_u32(0));
                    self.hidden = false;
                }
                2 => self.scale = [i.arg_f32(0), i.arg_f32(1)],
                // Per-frame scale speed (AnmManager::ExecuteScript).
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
                    self.scale_time = 0;
                }
                3 => self.alpha = (i.arg_u32(0) & 0xff) as f32 / 255.0,
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
                    let from = [self.color[0], self.color[1], self.color[2], self.alpha];
                    let mut to = from;
                    to[3] = (i.arg_u32(0) & 0xff) as f32 / 255.0;
                    self.fade =
                        (i.arg_u32(1) as u16 != 0).then_some((from, to, 0, i.arg_u32(1) as u16));
                }
                17 => {
                    let pos = [i.arg_f32(0), i.arg_f32(1)];
                    if self.use_pos_offset {
                        self.pos_offset = pos;
                    } else {
                        self.pos = pos;
                    }
                    self.positioned = true;
                }
                18..=20 => {
                    let ease = match i.opcode {
                        18 => Ease::Linear,
                        19 => Ease::Decel,
                        _ => Ease::Accel,
                    };
                    let duration = i.arg_u32(3) as u16;
                    self.moving = (duration != 0).then_some(Move {
                        from: if self.use_pos_offset {
                            self.pos_offset
                        } else {
                            self.pos
                        },
                        to: [i.arg_f32(0), i.arg_f32(1)],
                        elapsed: 0,
                        duration,
                        ease,
                    });
                }
                21 | 24 => {
                    if i.opcode == 24 {
                        self.hidden = true;
                    }
                    self.halted = true;
                    return;
                }
                25 => self.use_pos_offset = i.arg_u32(0) != 0,
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
                27 | 28 => {
                    let axis = (i.opcode - 27) as usize;
                    self.uv_scroll[axis] += i.arg_f32(0);
                    // Original performs a single wrap, rather than rem_euclid.
                    if self.uv_scroll[axis] >= 1.0 {
                        self.uv_scroll[axis] -= 1.0;
                    } else if self.uv_scroll[axis] < 0.0 {
                        self.uv_scroll[axis] += 1.0;
                    }
                }
                // SetVisibility: isVisible = args[0].
                29 => self.hidden = i.arg_u32(0) == 0,
                31 => self.z_write_disable = i.arg_u32(0) & 1 != 0,
                _ => {}
            }
            self.pc += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn z_write_disable_uses_original_single_bit_and_resets_with_script() {
        assert!(!AnmRunner::new(Vec::new()).z_write_disable);
        for (argument, expected) in [(0, false), (1, true), (2, false), (3, true)] {
            assert_eq!(
                AnmRunner::new(vec![instr(31, &[argument])]).z_write_disable,
                expected
            );
        }
    }

    fn instr(opcode: u8, args: &[u32]) -> Instr {
        Instr {
            time: 0,
            opcode,
            args: args.iter().flat_map(|arg| arg.to_le_bytes()).collect(),
            offset: 0,
        }
    }

    #[test]
    fn scale_speed_applies_on_initial_execute_and_every_frame() {
        let mut vm = AnmRunner::new(vec![
            instr(2, &[0, 0]),
            instr(11, &[0.05f32.to_bits(), 0.05f32.to_bits()]),
            instr(21, &[]),
        ]);
        assert_eq!(vm.scale, [0.05, 0.05]);
        vm.tick();
        assert_eq!(vm.scale, [0.1, 0.1]);
    }

    #[test]
    fn fade_restores_saved_rgba_after_effect_owner_overrides_color() {
        let mut vm = AnmRunner::new(vec![instr(12, &[0, 40]), instr(21, &[])]);
        assert_eq!(vm.alpha, 248.0 / 255.0);
        // EffectManager::SpawnParticles assigns NEONBLUE after executing the
        // initial script. Its running fade still holds white RGB endpoints.
        vm.set_rgba([64.0 / 255.0, 64.0 / 255.0, 1.0, 1.0]);
        vm.tick();
        assert_eq!(vm.color, [1.0; 3]);
        assert_eq!(vm.alpha, 242.0 / 255.0);
    }

    #[test]
    fn offset_position_and_fourth_power_easing_match_execute_script() {
        let mut vm = AnmRunner::new(vec![
            instr(17, &[12f32.to_bits(), 20f32.to_bits(), 0]),
            instr(25, &[1]),
            instr(20, &[16f32.to_bits(), 0, 0, 4]),
            instr(21, &[]),
        ]);
        assert_eq!(vm.pos, [12.0, 20.0]);
        assert_eq!(vm.pos_offset, [0.0, 0.0]);
        vm.tick();
        assert_eq!(vm.pos_offset, [10.9375, 0.0]);
        for _ in 0..3 {
            vm.tick();
        }
        assert_eq!(vm.pos_offset, [16.0, 0.0]);
        assert_eq!(vm.pos, [12.0, 20.0]);
    }

    #[test]
    fn invisible_halt_interrupt_uses_fallback_and_restores_visibility() {
        let mut vm = AnmRunner::new(vec![
            instr(1, &[1]),
            instr(24, &[]),
            instr(22, &[u32::MAX]),
            instr(1, &[3]),
            instr(21, &[]),
            instr(0, &[]),
        ]);
        assert!(!vm.visible());
        vm.interrupt(7);
        vm.tick();
        assert!(vm.visible());
        assert_eq!(vm.sprite, Some(3));
    }

    #[test]
    fn static_sprite_finishes_execution_and_preserves_visible_sprite() {
        let mut vm = AnmRunner::new(vec![
            instr(1, &[4]),
            instr(11, &[1f32.to_bits(), 1f32.to_bits()]),
            instr(15, &[]),
            instr(0, &[]),
        ]);
        assert!(vm.ended());
        assert!(vm.visible());
        let scale = vm.scale;
        vm.tick();
        vm.interrupt(1);
        vm.tick();
        assert_eq!(vm.scale, scale);
    }
}
