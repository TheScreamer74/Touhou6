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
}

impl Hud {
    pub fn new(front: &Entry, tex_slot: usize) -> Self {
        let sprites = front
            .sprites
            .iter()
            .map(|s| (s.index, [s.x, s.y, s.width, s.height]))
            .collect();
        let runners = front
            .scripts
            .iter()
            .map(|(_, instrs)| AnmRunner::new(instrs.clone()))
            .collect();
        Self { sprites, tex_slot, tex_size: front.width as f32, runners }
    }

    pub fn tick(&mut self) {
        for r in &mut self.runners {
            r.tick();
        }
    }

    /// Emit the self-placing HUD sprites (labels + intro emblems). Elements the
    /// game positions each frame (stars, digits) are skipped here.
    pub fn draw(&self, cmds: &mut Vec<DrawCmd>) {
        let ts = self.tex_size;
        for r in &self.runners {
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
            });
        }
    }
}
