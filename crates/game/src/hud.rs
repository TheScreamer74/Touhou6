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

pub struct Hud {
    /// front.anm sprite index -> pixel rect [x, y, w, h].
    sprites: HashMap<u32, [f32; 4]>,
    tex_slot: usize,
    tex_size: f32,
    runners: Vec<AnmRunner>,
    /// front.anm script id of each runner (parallel to `runners`).
    ids: Vec<u32>,
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
        Self { sprites, tex_slot, tex_size: front.width as f32, runners, ids }
    }

    pub fn tick(&mut self) {
        for r in &mut self.runners {
            r.tick();
        }
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
    pub fn script_state(&self, id: u32) -> Option<([f32; 4], [f32; 2], [f32; 2], f32)> {
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
                tex: self.tex_slot,
                dst: [x, y, sw * scale_x, sh * scale[1]],
                src: [sx / ts, sy / ts, (sx + sw) / ts, (sy + sh) / ts],
                tint: [1.0, 1.0, 1.0, alpha],
                rot: 0.0,
                additive: false,
            });
        }
    }

    /// Emit the self-placing HUD sprites (labels + intro emblems). Elements the
    /// game positions each frame (stars, digits) are skipped here.
    pub fn draw(&self, cmds: &mut Vec<DrawCmd>) {
        let ts = self.tex_size;
        for (r, &id) in self.runners.iter().zip(&self.ids) {
            // Boss frame / health bar (scripts 19/20/21) are placed by the boss
            // UI in stage.rs each frame, not self-placed here.
            if matches!(id, 19 | 20 | 21) {
                continue;
            }
            if !r.visible() || !r.positioned {
                continue;
            }
            let Some(sprite) = r.sprite else { continue };
            let Some(&[x, y, w, h]) = self.sprites.get(&sprite) else { continue };
            let sw = w * r.scale[0];
            let sh = h * r.scale[1];
            // op23 AnchorTopLeft -> pos is the top-left corner, else the centre.
            let dst = if r.corner {
                [r.pos[0], r.pos[1], sw, sh]
            } else {
                [r.pos[0] - sw / 2.0, r.pos[1] - sh / 2.0, sw, sh]
            };
            cmds.push(DrawCmd {
                tex: self.tex_slot,
                dst,
                src: [x / ts, y / ts, (x + w) / ts, (y + h) / ts],
                tint: [1.0, 1.0, 1.0, r.alpha],
                rot: r.rotation,
                additive: false,
            });
        }
    }
}
