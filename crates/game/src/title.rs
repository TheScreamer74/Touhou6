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
    timer: u16,
    input: MenuInput,
}
impl Title {
    pub fn new(mut art: MenuArt) -> Self {
        art.interrupt_all(2);
        Self {
            art,
            cursor: 0,
            quitting: None,
            pending: None,
            extra_unlocked: false,
            timer: 0,
            input: MenuInput::default(),
        }
    }
    pub fn set_extra_unlocked(&mut self, unlocked: bool) {
        self.extra_unlocked = unlocked;
    }
    pub fn update(&mut self, input: &Input) -> (Vec<DrawCmd>, TitleAction) {
        self.timer = self.timer.saturating_add(1);
        self.input.tick(input);
        let mut action = TitleAction::None;
        if self.quitting.is_none() {
            if self.input.pressed(input, Key::Up) {
                self.art.sound("select00");
                self.cursor = (self.cursor + 7) % 8;
                if self.cursor == 1 && !self.extra_unlocked {
                    self.cursor = 0;
                }
            }
            if self.input.pressed(input, Key::Down) {
                self.art.sound("select00");
                self.cursor = (self.cursor + 1) % 8;
                if self.cursor == 1 && !self.extra_unlocked {
                    self.cursor = 2;
                }
            }
            if self.timer >= 20 && (input.pressed(Key::Bomb) || input.pressed(Key::Pause)) {
                self.art.sound("cancel00");
                self.cursor = 7;
            }
            if self.timer >= 20 && (input.pressed(Key::Shoot) || input.pressed(Key::Enter)) {
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
                    action = TitleAction::None;
                }
            }
        }
        self.art.tick();
        if let Some(t) = &mut self.quitting {
            *t = t.saturating_sub(1);
            if *t == 0 {
                action = self.pending.take().unwrap_or(TitleAction::Quit);
            }
        }
        for idx in 0..8 {
            self.art.highlight(idx, idx == self.cursor);
        }
        let cmds = self.art.draw(|idx| {
            if idx < 8 {
                let active = idx == self.cursor;
                Some((
                    if active { [-4.0, -4.0] } else { [0.0, 0.0] },
                    [1.0, 1.0, 1.0, if active { 1.0 } else { 128.0 / 255.0 }],
                ))
            } else {
                Some(([0.0; 2], [1.0; 4]))
            }
        });
        (cmds, action)
    }
    pub fn reset(&mut self) {
        self.art.reset();
        self.art.interrupt_all(2);
        self.timer = 0;
        self.quitting = None;
        self.pending = None;
    }
    pub fn take_sounds(&mut self) -> Vec<&'static str> {
        self.art.take_sounds()
    }
    fn start_quit(&mut self) {
        self.art.interrupt_all(4);
        self.quitting = Some(61);
    }
}
