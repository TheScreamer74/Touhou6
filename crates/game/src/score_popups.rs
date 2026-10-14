//! AsciiManager's two score popup pools (512 normal / 3 bullet-point).
use th06_engine::{DepthState, DepthTest, DrawCmd};
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
            let mut x = popup.pos[0] - popup.digits.len() as f32 * 4.0;
            for digit in popup.digits.iter().rev() {
                if let Some(sprite) = entry.sprites.iter().find(|sprite| sprite.index == *digit) {
                    // InitializeVms sets anchorTopLeft before vm1.Initialize,
                    // which resets it to CENTER. DrawPopupsWithHwVertexProcessing
                    // then uses Draw2 at z=.1, including its rounding/projection.
                    // ascii.anm's label 10 is 48x8; ordinary digits are 8x8.
                    let mut dst = crate::sprite_geometry::world(
                        [x, popup.pos[1]],
                        [sprite.width, sprite.height],
                        0.0,
                        false,
                        0.1,
                    )
                    .dst;
                    dst[0] += origin[0];
                    dst[1] += origin[1];
                    cmds.push(DrawCmd {
                        clip: None,
                        tex,
                        dst,
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
                        depth: Some(DepthState {
                            z: crate::sprite_geometry::world_depth(0.1),
                            test: DepthTest::LessEqual,
                            write: true,
                        }),
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
    fn original_centered_popup_digits_and_wide_label_use_world_depth() {
        use th06_formats::anm0::Sprite;
        // The supplied ascii.anm has ten 8x8 digits and a 48x8 sprite 10.
        // Source DrawPopups sets VM centers to x-count*4,y, then advances x by 8.
        // These projected bounds use the original 224/tan(15deg) camera and
        // z=.1. Half-integer centers exercise x87 nearest-even rounding.
        let mut sprites: Vec<_> = (0..10)
            .map(|index| Sprite {
                index,
                x: index as f32 * 8.0,
                y: 0.0,
                width: 8.0,
                height: 8.0,
            })
            .collect();
        sprites.push(Sprite {
            index: 10,
            x: 80.0,
            y: 0.0,
            width: 48.0,
            height: 8.0,
        });
        let entry = Entry {
            width: 256,
            height: 256,
            format: 4,
            name: "ascii.png".into(),
            alpha_name: None,
            sprites,
            scripts: Vec::new(),
        };
        let mut popups = ScorePopups::default();
        popups.spawn(false, [192.5, 224.5], 123, [1.0; 4]);
        popups.spawn(false, [192.5, 224.5], -1, [1.0; 4]);
        let mut cmds = Vec::new();
        popups.draw(&mut cmds, &entry, 7, [32.0, 16.0]);
        assert_eq!(cmds.len(), 4);
        for (actual, expected) in cmds[0]
            .dst
            .into_iter()
            .zip([208.00197, 236.00053, 7.999043, 7.999043])
        {
            assert!((actual - expected).abs() < 0.0001);
        }
        for (actual, expected) in cmds[3]
            .dst
            .into_iter()
            .zip([196.0034, 236.00053, 47.99426, 7.999043])
        {
            assert!((actual - expected).abs() < 0.0001);
        }
        assert_eq!(cmds[0].src, [8.0 / 256.0, 0.0, 16.0 / 256.0, 8.0 / 256.0]);
        for command in cmds {
            let depth = command.depth.unwrap();
            assert!((depth.z - 0.4446435).abs() < 0.000001);
            assert_eq!(depth.test, DepthTest::LessEqual);
            assert!(depth.write);
        }
    }

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
