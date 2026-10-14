//! Original MainMenu key configuration: raw controller buttons and SwapMapping.
use crate::menu_art::{MenuArt, MenuInput};
use th06_engine::{DrawCmd, Input, Key};
pub const DEFAULT_MAPPING: [u16; 9] = [
    0,
    1,
    0,
    u16::MAX,
    u16::MAX,
    u16::MAX,
    u16::MAX,
    u16::MAX,
    u16::MAX,
];
pub struct KeyConfig {
    pub mapping: [u16; 9],
    cursor: usize,
    timer: u16,
    last_button: u16,
}
impl KeyConfig {
    pub fn begin(art: &mut MenuArt) -> Self {
        art.interrupt_all(5);
        for idx in 54..72 {
            art.copy_sheet(idx, 0);
        }
        let mapping = th06_engine::controller_mapping();
        let mut navigation = mapping;
        navigation[4] = u16::MAX;
        navigation[5] = u16::MAX;
        th06_engine::set_controller_mapping(navigation);
        Self {
            mapping,
            cursor: 0,
            timer: 1,
            last_button: u16::MAX,
        }
    }
    pub fn update(&mut self, input: &Input, art: &mut MenuArt, menu_input: &MenuInput) -> bool {
        let back = self.update_inner(input, art, menu_input);
        self.timer = self.timer.saturating_add(1);
        back
    }
    fn update_inner(&mut self, input: &Input, art: &mut MenuArt, menu_input: &MenuInput) -> bool {
        if menu_input.pressed(input, Key::Up) {
            art.sound("select00");
            self.cursor = (self.cursor + 10) % 11;
        } else if menu_input.pressed(input, Key::Down) {
            art.sound("select00");
            self.cursor = (self.cursor + 1) % 11;
        }
        if self.timer < 32 {
            return false;
        }
        let buttons = input.gamepad_buttons();
        let button = if buttons == 0 {
            u16::MAX
        } else {
            buttons.trailing_zeros() as u16
        };
        if button != u16::MAX && button != self.last_button {
            art.sound("ok00");
            if self.cursor < 9 {
                assign(&mut self.mapping, self.cursor, button);
            }
        }
        self.last_button = button;
        if input.pressed(Key::Shoot) || input.pressed(Key::Enter) {
            match self.cursor {
                9 => self.mapping = DEFAULT_MAPPING,
                10 => {
                    art.sound("cancel00");
                    th06_engine::set_controller_mapping(self.mapping);
                    return true;
                }
                _ => {}
            }
        }
        false
    }
    pub fn reference_navigation(&self) -> (u8, usize, u16) {
        (5, self.cursor, self.timer)
    }
    pub fn draw(&self, art: &mut MenuArt) -> Vec<DrawCmd> {
        for idx in 34..72 {
            let row = if idx < 45 {
                idx - 34
            } else if idx < 54 {
                idx - 45
            } else {
                (idx - 54) / 2
            };
            art.highlight(idx, row == self.cursor);
        }
        for i in 0..18 {
            let mapping = self.mapping[i / 2];
            if mapping != u16::MAX {
                art.sprite(
                    54 + i,
                    if i % 2 == 0 {
                        mapping as u32 / 10
                    } else {
                        mapping as u32 % 10
                    },
                );
            }
        }
        art.draw(|idx| {
            if !(34..72).contains(&idx) {
                return Some(([0.0; 2], [1.0; 4]));
            }
            let row = if idx < 45 {
                idx - 34
            } else if idx < 54 {
                idx - 45
            } else {
                (idx - 54) / 2
            };
            if idx >= 45 && self.mapping[row] == u16::MAX {
                return None;
            }
            let active = row == self.cursor;
            Some((
                if active { [-4.0, -4.0] } else { [0.0, 0.0] },
                [1.0, 1.0, 1.0, if active { 1.0 } else { 128.0 / 255.0 }],
            ))
        })
    }
}
fn assign(mapping: &mut [u16; 9], row: usize, button: u16) {
    let old = mapping[row];
    let share = row == 0 || row == 2;
    for (idx, current) in mapping.iter_mut().enumerate() {
        if share && (idx == 0 || idx == 2) {
            continue;
        }
        if *current == button {
            *current = old;
        }
    }
    mapping[row] = button;
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shoot_and_focus_may_share_but_bomb_swaps() {
        let mut map = DEFAULT_MAPPING;
        assign(&mut map, 2, 0);
        assert_eq!(map[0], map[2]);
        assign(&mut map, 1, 0);
        assert_eq!(map[0], 1);
        assert_eq!(map[2], 1);
        assert_eq!(map[1], 0);
    }
}
