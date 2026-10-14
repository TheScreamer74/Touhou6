//! MainMenu.cpp title navigation and original software-color sprite variants.
use crate::menu_art::{MenuArt, MenuInput};
use th06_engine::{DrawCmd, Input, Key};
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TitleAction {
    None,
    StartGame,
    Extra,
    Practice,
    Replay,
    Score,
    Music,
    Options,
    Quit,
}
pub struct Title {
    art: MenuArt,
    cursor: usize,
    quitting: Option<u16>,
    pending: Option<TitleAction>,
    extra_unlocked: bool,
    pre_input: bool,
    timer: u16,
    pub(crate) input: MenuInput,
}
impl Title {
    pub fn new(mut art: MenuArt) -> Self {
        art.interrupt_all(1);
        Self {
            art,
            cursor: 0,
            quitting: None,
            pending: None,
            extra_unlocked: false,
            pre_input: true,
            // RegisterChain assigns 60 after the added callback.
            timer: 60,
            input: MenuInput::default(),
        }
    }
    pub fn set_extra_unlocked(&mut self, unlocked: bool) {
        self.extra_unlocked = unlocked;
    }
    pub fn update(&mut self, input: &Input) -> (Vec<DrawCmd>, TitleAction) {
        self.input.tick(input);
        let mut action = TitleAction::None;
        if self.pre_input {
            const START_INPUT: u16 = 1 | 2 | 8 | 512 | 1024 | 4096;
            if self.timer >= 30 && self.input.pressed_mask(input, START_INPUT) {
                self.pre_input = false;
                self.timer = 0;
                self.art.interrupt_all(2);
            } else {
                self.art.tick();
                self.timer = self.timer.saturating_add(1);
                return (self.draw(), action);
            }
        }
        if self.quitting.is_none() {
            let accepts_selection = self.timer >= 20;
            if self.input.pressed(input, Key::Up) {
                self.art.sound("select00");
                self.cursor = (self.cursor + 7) % 8;
                if self.cursor == 1 && !self.extra_unlocked {
                    self.cursor = 0;
                }
            } else if self.input.pressed(input, Key::Down) {
                self.art.sound("select00");
                self.cursor = (self.cursor + 1) % 8;
                if self.cursor == 1 && !self.extra_unlocked {
                    self.cursor = 2;
                }
            }
            if accepts_selection && (input.pressed(Key::Shoot) || input.pressed(Key::Enter)) {
                self.art
                    .sound(if self.cursor == 7 { "cancel00" } else { "ok00" });
                action = match self.cursor {
                    0 => TitleAction::StartGame,
                    1 if self.extra_unlocked => TitleAction::Extra,
                    2 => TitleAction::Practice,
                    3 => TitleAction::Replay,
                    4 => TitleAction::Score,
                    5 => TitleAction::Music,
                    6 => TitleAction::Options,
                    7 => {
                        self.start_quit();
                        TitleAction::None
                    }
                    _ => TitleAction::None,
                };
                if !matches!(action, TitleAction::None | TitleAction::Options) {
                    self.pending = Some(action);
                    self.art.interrupt_all(4);
                    self.quitting = Some(61);
                    self.timer = 0;
                    action = TitleAction::None;
                }
            }
            if accepts_selection && input.pressed(Key::Quit) {
                self.art.sound("cancel00");
                self.pending = None;
                action = TitleAction::None;
                self.start_quit();
            }
            if accepts_selection && (input.pressed(Key::Bomb) || input.pressed(Key::Pause)) {
                self.art.sound("cancel00");
                self.cursor = 7;
            }
        }
        self.art.tick();
        if let Some(t) = &mut self.quitting {
            *t = t.saturating_sub(1);
            if *t == 0 {
                action = self.pending.take().unwrap_or(TitleAction::Quit);
            }
        }
        self.timer = self.timer.saturating_add(1);
        (self.draw(), action)
    }
    pub fn draw(&mut self) -> Vec<DrawCmd> {
        if self.pre_input {
            return self.art.draw(|_| Some(([0.0; 2], [1.0; 4])));
        }
        for idx in 0..8 {
            self.art.highlight(idx, idx == self.cursor);
        }
        self.art.draw(|idx| {
            if idx < 8 {
                let active = idx == self.cursor;
                Some((
                    if active { [-4.0, -4.0] } else { [0.0, 0.0] },
                    [1.0, 1.0, 1.0, if active { 1.0 } else { 128.0 / 255.0 }],
                ))
            } else {
                Some(([0.0; 2], [1.0; 4]))
            }
        })
    }
    pub fn reset(&mut self, cursor: usize) {
        self.art.reset();
        self.art.interrupt_all(1);
        self.cursor = cursor;
        self.timer = 60;
        self.pre_input = true;
        self.quitting = None;
        self.pending = None;
        self.input = MenuInput::default();
    }
    /// Returning from selection re-enters STATE_STARTUP in the same MainMenu
    /// chain. Its timer was reset to zero, then incremented to one.
    pub fn begin_startup(&mut self, cursor: usize) {
        self.reset(cursor);
        self.timer = 1;
    }
    /// Options returns directly to the active title menu, without startup.
    pub fn resume_menu(&mut self, cursor: usize) {
        self.reset(cursor);
        self.art.interrupt_all(2);
        self.pre_input = false;
        self.timer = 1;
        self.cursor = cursor;
    }

    pub fn reference_navigation(&self) -> (u8, usize, u16) {
        let state = if self.pre_input {
            1
        } else if self.quitting.is_some() {
            match self.pending {
                Some(TitleAction::StartGame | TitleAction::Extra | TitleAction::Practice) => 6,
                Some(TitleAction::Replay) => 12,
                Some(TitleAction::Score) => 10,
                Some(TitleAction::Music) => 16,
                _ => 4,
            }
        } else {
            2
        };
        (state, self.cursor, self.timer)
    }
    pub fn take_sounds(&mut self) -> Vec<&'static str> {
        self.art.take_sounds()
    }
    fn start_quit(&mut self) {
        self.art.interrupt_all(4);
        self.quitting = Some(61);
        self.timer = 0;
    }
}
