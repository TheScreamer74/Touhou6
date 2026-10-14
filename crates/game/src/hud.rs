//! In-game HUD driven by `front.anm`, matching the decompilation's `Gui` —
//! each persistent HUD element is a front.anm script run as an `AnmRunner`
//! (the same VM that drives title/stage sprites). The scripts place graphic
//! labels in 640x480 screen space (x=432 sidebar) and animate the intro
//! emblems; dynamic values (score digits, star counts, power) are drawn over
//! this by `stage.rs`.

use std::collections::HashMap;

use th06_engine::DrawCmd;
use th06_formats::anm0::Entry;

use crate::anm_vm::AnmRunner;

pub type ScriptState = ([f32; 4], [f32; 2], [f32; 2], f32);

/// Supervisor::DrawFpsCounter: refresh its ASCII label every 500 milliseconds.
/// Count presentations, so extra calculation ticks do not inflate the result.
#[derive(Default)]
pub struct FpsCounter {
    last_time: Option<u32>,
    last_frames: u64,
    frames: u64,
    label: String,
}

impl FpsCounter {
    pub fn observe(&mut self, milliseconds: u32, presentations: u64) {
        let last_time = *self.last_time.get_or_insert_with(|| {
            self.last_frames = presentations.saturating_sub(1);
            milliseconds
        });
        self.frames += presentations.wrapping_sub(self.last_frames);
        self.last_frames = presentations;
        let elapsed = milliseconds.wrapping_sub(last_time);
        if elapsed >= 500 {
            let fps = self.frames as f32 / (elapsed as f32 / 1000.0);
            self.label = format!("{fps:.2}fps");
            self.frames = 0;
            self.last_time = Some(milliseconds);
        }
    }

    pub fn draw(&self, commands: &mut Vec<DrawCmd>) {
        crate::stage::draw_text(commands, [512.0, 464.0], 16.0, [1.0; 4], &self.label);
    }
}

pub struct Hud {
    /// front.anm sprite index -> pixel rect [x, y, w, h].
    sprites: HashMap<u32, [f32; 4]>,
    tex_slot: usize,
    tex_size: f32,
    runners: Vec<AnmRunner>,
    /// front.anm script id of each runner (parallel to `runners`).
    ids: Vec<u32>,
    boss_scripts: HashMap<u32, Vec<th06_formats::anm0::Instr>>,
    boss_state: u8,
    boss_opacity: u32,
}

impl Hud {
    pub fn new(front: &Entry, tex_slot: usize) -> Self {
        let sprites = front
            .sprites
            .iter()
            .map(|s| (s.index, [s.x, s.y, s.width, s.height]))
            .collect();
        let runners: Vec<_> = front
            .scripts
            .iter()
            .map(|(_, instrs)| AnmRunner::new(instrs.clone()))
            .collect();
        let ids = front.scripts.iter().map(|(id, _)| *id).collect();
        Self {
            sprites,
            tex_slot,
            tex_size: front.width as f32,
            runners,
            ids,
            boss_scripts: front
                .scripts
                .iter()
                .filter(|(id, _)| matches!(id, 19 | 20))
                .cloned()
                .collect(),
            boss_state: 0,
            boss_opacity: 0,
        }
    }

    pub fn tick(&mut self) {
        for (id, runner) in self.ids.iter().zip(&mut self.runners) {
            if *id != 19 {
                runner.tick();
            }
        }
    }

    /// Gui::UpdateStageElements owns VM19 separately from the other HUD VMs.
    /// Script19 enters; script20 leaves. Their ANM_STATIC instructions, rather
    /// than a guessed fade duration, determine bossHealthBarState transitions.
    pub(crate) fn tick_boss(&mut self, present: bool, dialogue: bool) {
        let Some(index) = self.ids.iter().position(|&id| id == 19) else {
            return;
        };
        if dialogue {
            self.runners[index].tick();
            return;
        }
        if present {
            if self.boss_state == 0 {
                if let Some(script) = self.boss_scripts.get(&19) {
                    self.runners[index] = AnmRunner::new(script.clone());
                }
                self.boss_state = 1;
                self.boss_opacity = 0;
            } else {
                self.runners[index].tick();
                if self.runners[index].ended() {
                    self.boss_state = 2;
                }
                self.boss_opacity = if self.boss_opacity < 252 {
                    self.boss_opacity + 4
                } else {
                    255
                };
            }
        } else if self.boss_state != 0 {
            if self.boss_state <= 2 {
                if let Some(script) = self.boss_scripts.get(&20) {
                    self.runners[index] = AnmRunner::new(script.clone());
                }
                self.boss_state = 3;
            }
            self.boss_opacity = self.boss_opacity.saturating_sub(4);
            self.runners[index].tick();
            if self.runners[index].ended() {
                self.boss_state = 0;
                self.boss_opacity = 0;
            }
        }
    }

    pub(crate) fn boss_state(&self) -> u8 {
        self.boss_state
    }

    pub(crate) fn boss_opacity(&self) -> f32 {
        self.boss_opacity as f32 / 255.0
    }

    pub fn tex(&self) -> usize {
        self.tex_slot
    }

    pub fn tex_size(&self) -> f32 {
        self.tex_size
    }

    /// Draw state of front.anm script `id`: its current sprite rect
    /// [x, y, w, h], self-placed pos, scale and alpha. The boss UI repositions
    /// and re-scales these sprites itself (`Gui::DrawGameScene`) but keeps the
    /// script's own scaleY/alpha (e.g. the health bar's 0.3 height + fade-in).
    pub fn script_state(&self, id: u32) -> Option<ScriptState> {
        let idx = self.ids.iter().position(|&i| i == id)?;
        let r = &self.runners[idx];
        let sprite = r.sprite?;
        let rect = *self.sprites.get(&sprite)?;
        Some((rect, r.pos, r.scale, r.alpha))
    }

    /// Draw front.anm script `id`'s current sprite, top-left anchored at
    /// (x, y), its width scaled by `scale_x` (1.0 = native). For the
    /// game-positioned HUD tiles/plates/stars (`Gui::DrawGameScene`).
    pub fn draw_sprite(&self, cmds: &mut Vec<DrawCmd>, id: u32, x: f32, y: f32, scale_x: f32) {
        if let Some(([sx, sy, sw, sh], _, scale, alpha)) = self.script_state(id) {
            let ts = self.tex_size;
            cmds.push(DrawCmd {
                clip: None,
                tex: self.tex_slot,
                dst: [x, y, sw * scale_x, sh * scale[1]],
                src: [sx / ts, sy / ts, (sx + sw) / ts, (sy + sh) / ts],
                tint: [1.0, 1.0, 1.0, alpha],
                rot: 0.0,
                additive: false,
                color_add: false,
                depth: None,
            });
        }
    }

    /// Emit the self-placing HUD sprites (labels + intro emblems). Elements the
    /// game positions each frame (stars, digits) are skipped here.
    pub fn draw(&self, cmds: &mut Vec<DrawCmd>) {
        let ts = self.tex_size;
        // Gui::DrawGameScene uses this explicit layering, independent of ANM
        // script order (the overlapping logo ring belongs above some glyphs).
        for id in [5, 0, 1, 3, 4, 2, 9, 10, 11, 12, 13, 14, 15] {
            let Some(index) = self.ids.iter().position(|&candidate| candidate == id) else {
                continue;
            };
            let r = &self.runners[index];
            if !r.visible() || !r.positioned {
                continue;
            }
            let Some(sprite) = r.sprite else { continue };
            let Some(&[x, y, w, h]) = self.sprites.get(&sprite) else {
                continue;
            };
            let sw = w * r.scale[0];
            let sh = h * r.scale[1];
            // op23 AnchorTopLeft -> pos is the top-left corner, else the centre.
            let geometry = crate::sprite_geometry::screen_rotated(
                r.pos,
                [sw, sh],
                if id <= 5 { r.rotation } else { 0.0 },
                r.corner,
            );
            cmds.push(DrawCmd {
                clip: None,
                tex: self.tex_slot,
                dst: geometry.dst,
                src: [x / ts, y / ts, (x + w) / ts, (y + h) / ts],
                tint: [r.color[0], r.color[1], r.color[2], r.alpha],
                // AnmManager::TranslateRotation rotates positive script angles
                // counterclockwise in screen space. Engine angles are clockwise.
                rot: geometry.rotation,
                additive: r.additive,
                color_add: r.color_add,
                depth: None,
            });
        }
    }
}

#[cfg(test)]
mod boss_tests {
    use super::*;
    use th06_formats::anm0::Instr;

    pub(super) fn front() -> Entry {
        let script = |alpha: u32, target: u32| {
            vec![
                Instr {
                    time: 0,
                    opcode: 3,
                    args: alpha.to_le_bytes().to_vec(),
                    offset: 0,
                },
                Instr {
                    time: 0,
                    opcode: 12,
                    args: [target.to_le_bytes(), 60_u32.to_le_bytes()].concat(),
                    offset: 8,
                },
                Instr {
                    time: 61,
                    opcode: 15,
                    args: Vec::new(),
                    offset: 20,
                },
            ]
        };
        Entry {
            width: 256,
            height: 256,
            format: 1,
            name: String::new(),
            alpha_name: None,
            sprites: Vec::new(),
            scripts: vec![(19, script(0, 255)), (20, script(255, 0))],
        }
    }

    #[test]
    fn boss_hud_restarts_on_entry_and_keeps_exit_until_original_anm_end() {
        let mut hud = Hud::new(&front(), 0);
        for _ in 0..200 {
            hud.tick();
        }
        assert_eq!(hud.boss_state(), 0);
        assert!(hud.runners[0].alpha < 0.02);
        hud.tick_boss(true, false);
        assert_eq!(hud.boss_state(), 1);
        assert_eq!(hud.boss_opacity(), 0.0);
        for _ in 0..60 {
            hud.tick_boss(true, false);
        }
        assert_eq!(hud.boss_state(), 1);
        hud.tick_boss(true, false);
        assert_eq!(hud.boss_state(), 2);

        hud.tick_boss(false, false);
        assert_eq!(hud.boss_state(), 3);
        assert!(hud.boss_opacity() > 0.0);
        for _ in 0..59 {
            hud.tick_boss(false, false);
        }
        assert_eq!(hud.boss_state(), 3);
        hud.tick_boss(false, false);
        assert_eq!(hud.boss_state(), 0);
        assert_eq!(hud.boss_opacity(), 0.0);
    }
}

#[cfg(test)]
mod fps_tests {
    use super::FpsCounter;

    #[test]
    fn fps_uses_original_half_second_window_and_presentations() {
        let mut fps = FpsCounter::default();
        for frame in 0..=30 {
            fps.observe(frame * 1000 / 60, u64::from(frame + 1));
        }
        assert_eq!(fps.label, "62.00fps");
        for frame in 31..=60 {
            let time = frame * 1000 / 60;
            fps.observe(time, u64::from(frame + 1));
            // Additional calculation callbacks do not add a presentation.
            fps.observe(time, u64::from(frame + 1));
        }
        assert_eq!(fps.label, "60.00fps");
    }

    #[test]
    fn original_millisecond_clock_wrap_is_preserved() {
        let mut fps = FpsCounter::default();
        fps.observe(u32::MAX - 249, 1);
        fps.observe(250, 31);
        assert_eq!(fps.label, "62.00fps");
    }
}
