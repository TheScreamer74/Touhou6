//! AsciiManager's two score popup pools (512 normal / 3 bullet-point).
use th06_engine::DrawCmd;
use th06_formats::anm0::Entry;

#[derive(Clone)]
struct Popup {
    pos: [f32; 2],
    digits: Vec<u32>,
    color: [f32; 4],
    timer: u8,
}

pub struct ScorePopups {
    slots: Vec<Option<Popup>>,
    next: [usize; 2],
}
impl Default for ScorePopups {
    fn default() -> Self {
        Self {
            slots: vec![None; 515],
            next: [0; 2],
        }
    }
}
impl ScorePopups {
    pub fn spawn(&mut self, player: bool, pos: [f32; 2], mut value: i64, color: [f32; 4]) {
        let pool = usize::from(player);
        let count = if player { 3 } else { 512 };
        let index = self.next[pool] + if player { 512 } else { 0 };
        self.next[pool] = (self.next[pool] + 1) % count;
        let mut digits = Vec::with_capacity(8);
        if value < 0 {
            digits.push(10);
        } else {
            while value > 0 {
                digits.push((value % 10) as u32);
                value /= 10;
            }
            if digits.is_empty() {
                digits.push(0);
            }
        }
        self.slots[index] = Some(Popup {
            pos,
            digits,
            color,
            timer: 0,
        });
    }
    pub fn tick(&mut self) {
        for slot in &mut self.slots {
            if let Some(popup) = slot {
                popup.pos[1] -= 0.5;
                popup.timer += 1;
                if popup.timer > 60 {
                    *slot = None;
                }
            }
        }
    }
    pub fn draw(&self, cmds: &mut Vec<DrawCmd>, entry: &Entry, tex: usize, origin: [f32; 2]) {
        for popup in self.slots.iter().flatten() {
            let mut x = origin[0] + popup.pos[0] - popup.digits.len() as f32 * 4.0;
            for digit in popup.digits.iter().rev() {
                if let Some(sprite) = entry.sprites.iter().find(|sprite| sprite.index == *digit) {
                    cmds.push(DrawCmd {
                        tex,
                        dst: [x, origin[1] + popup.pos[1], sprite.width, sprite.height],
                        src: [
                            sprite.x / entry.width as f32,
                            sprite.y / entry.height as f32,
                            (sprite.x + sprite.width) / entry.width as f32,
                            (sprite.y + sprite.height) / entry.height as f32,
                        ],
                        tint: popup.color,
                        rot: 0.0,
                        additive: false,
                        color_add: false,
                    });
                }
                x += 8.0;
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn original_popup_pools_wrap_independently_and_expire_after_sixty_ticks() {
        let mut p = ScorePopups::default();
        p.spawn(true, [10.0, 20.0], 123, [1.0; 4]);
        for _ in 0..513 {
            p.spawn(false, [0.0; 2], -1, [1.0; 4]);
        }
        assert_eq!(p.next, [1, 1]);
        assert_eq!(p.slots[512].as_ref().unwrap().digits, [3, 2, 1]);
        assert_eq!(p.slots[0].as_ref().unwrap().digits, [10]);
        for _ in 0..60 {
            p.tick();
        }
        assert_eq!(p.slots[512].as_ref().unwrap().pos, [10.0, -10.0]);
        p.tick();
        assert!(p.slots.iter().all(Option::is_none));
    }
}
