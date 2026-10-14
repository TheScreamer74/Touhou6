//! AsciiManager.cpp StageMenu retry state machine; no gameplay ticks while open.
use crate::menu_art::MenuArt;
use th06_engine::{DrawCmd, Input, Key};
pub enum RetryAction {
    None,
    Continue,
    Finish,
}
pub struct RetryMenu {
    art: MenuArt,
    yes: bool,
    selected: Option<bool>,
    timer: u16,
    retries: u8,
    active: bool,
    opening: bool,
    presentation_yes: bool,
}
impl RetryMenu {
    pub fn new(art: MenuArt) -> Self {
        Self {
            art,
            yes: false,
            selected: None,
            timer: 0,
            retries: 0,
            active: false,
            opening: false,
            presentation_yes: false,
        }
    }
    pub fn begin(&mut self, retries: u8) {
        self.retries = retries;
        self.active = true;
        self.opening = true;
        self.presentation_yes = false;
        self.timer = 0;
        self.selected = None;
        self.yes = false;
        self.art.reset();
    }
    pub fn diagnostic_state(&self) -> (u8, u16) {
        if !self.active || self.opening {
            return (0, 0);
        }
        let state = match self.selected {
            Some(true) => 3,
            Some(false) => 4,
            None if self.yes => 1,
            None => 2,
        };
        (state, self.timer)
    }

    pub fn update(&mut self, input: &Input) -> RetryAction {
        if self.opening {
            for i in [6, 7, 8, 9] {
                self.art.interrupt(i, 1);
            }
        }
        // Opening changes state to No and falls through the Yes styling.
        self.presentation_yes = self.yes || self.opening;
        self.opening = false;
        if let Some(yes) = self.selected {
            if self.timer >= if yes { 30 } else { 20 } {
                self.active = false;
                self.timer = 0;
                return if yes {
                    RetryAction::Continue
                } else {
                    RetryAction::Finish
                };
            }
            self.art.tick();
            self.timer = self.timer.saturating_add(1);
            return RetryAction::None;
        }
        let selected_yes = self.yes;
        if self.timer >= if self.yes { 4 } else { 30 } {
            if input.pressed(Key::Up) || input.pressed(Key::Down) {
                self.yes = !self.yes;
            }
            if input.pressed(Key::Shoot) || input.pressed(Key::Enter) {
                // The switch keeps the original state's selection when a
                // direction and confirmation arrive on the same callback.
                self.selected = Some(selected_yes);
                self.timer = 0;
                for i in [6, 7, 8, 9] {
                    self.art.interrupt(i, 2);
                }
            }
        }
        self.art.tick();
        self.timer = self.timer.saturating_add(1);
        RetryAction::None
    }
    pub fn draw(&mut self) -> Vec<DrawCmd> {
        for idx in [6, 7] {
            self.art.scale(
                idx,
                if (idx == 6) == self.presentation_yes {
                    [1.7; 2]
                } else {
                    [1.5; 2]
                },
            );
        }
        // DrawNoRotation uses XYZRHW screen coordinates; the viewport only clips.
        // OnDrawRetryMenu also does not apply the VM's selected-item posOffset.
        let mut cmds = self.art.draw(|idx| {
            if ![6, 7, 8, 9].contains(&idx) {
                return None;
            }
            let active =
                (idx == 6 && self.presentation_yes) || (idx == 7 && !self.presentation_yes);
            Some((
                [0.0; 2],
                if idx == 6 || idx == 7 {
                    if active {
                        [1.0, 128.0 / 255.0, 128.0 / 255.0, 1.0]
                    } else {
                        [128.0 / 255.0; 4]
                    }
                } else {
                    [1.0; 4]
                },
            ))
        });
        // MenuArt's backdrop is replaced by the frozen stage underneath.
        cmds.remove(0);
        if self.selected.is_none()
            && let Some(cmd) = self.art.extra_sprite(
                9,
                30 - self.retries as u32,
                [8.0 * self.art.scale_of(9)[0], 0.0],
                [1.0; 4],
            )
        {
            cmds.push(cmd);
        }
        cmds
    }
    pub fn take_sounds(&mut self) -> Vec<&'static str> {
        self.art.take_sounds()
    }
}
