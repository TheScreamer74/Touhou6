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
    quit_enabled: bool,
    pre_input: bool,
    timer: u16,
    pub(crate) input: MenuInput,
}
impl Title {
    pub fn new(art: MenuArt) -> Self {
        // A browser has no application window to quit; native keeps the
        // original eighth entry and its keyboard shortcuts.
        Self::with_quit(art, !cfg!(target_arch = "wasm32"))
    }
    fn with_quit(mut art: MenuArt, quit_enabled: bool) -> Self {
        art.interrupt_all(1);
        Self {
            art,
            cursor: 0,
            quitting: None,
            pending: None,
            extra_unlocked: false,
            quit_enabled,
            pre_input: true,
            // RegisterChain assigns 60 after the added callback.
            timer: 60,
            input: MenuInput::default(),
        }
    }
    fn entry_count(&self) -> usize {
        if self.quit_enabled { 8 } else { 7 }
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
                let count = self.entry_count();
                self.cursor = (self.cursor + count - 1) % count;
                if self.cursor == 1 && !self.extra_unlocked {
                    self.cursor = 0;
                }
            } else if self.input.pressed(input, Key::Down) {
                self.art.sound("select00");
                self.cursor = (self.cursor + 1) % self.entry_count();
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
                    7 if self.quit_enabled => {
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
            if self.quit_enabled && accepts_selection && input.pressed(Key::Quit) {
                self.art.sound("cancel00");
                self.pending = None;
                action = TitleAction::None;
                self.start_quit();
            }
            if self.quit_enabled
                && accepts_selection
                && (input.pressed(Key::Bomb) || input.pressed(Key::Pause))
            {
                self.art.sound("cancel00");
                self.cursor = 7;
            }
        }
        self.art.tick();
        if let Some(t) = &mut self.quitting {
            *t = t.saturating_sub(1);
            if *t == 0 {
                action = self.pending.take().unwrap_or(if self.quit_enabled {
                    TitleAction::Quit
                } else {
                    TitleAction::None
                });
            }
        }
        self.timer = self.timer.saturating_add(1);
        (self.draw(), action)
    }
    pub fn draw(&mut self) -> Vec<DrawCmd> {
        let quit_enabled = self.quit_enabled;
        if self.pre_input {
            return self
                .art
                .draw(|idx| (quit_enabled || idx != 7).then_some(([0.0; 2], [1.0; 4])));
        }
        for idx in 0..self.entry_count() {
            self.art.highlight(idx, idx == self.cursor);
        }
        self.art.draw(|idx| {
            if !quit_enabled && idx == 7 {
                return None;
            }
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
        self.cursor = if self.quit_enabled {
            cursor
        } else {
            cursor.min(self.entry_count() - 1)
        };
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
        // MainMenu keeps these VMs alive through Options. Resetting them here
        // loses the rotation established by label1; label2 does not restore it.
        self.cursor = if self.quit_enabled {
            cursor
        } else {
            cursor.min(self.entry_count() - 1)
        };
        self.quitting = None;
        self.pending = None;
        self.input = MenuInput::default();
        self.art.interrupt_all(2);
        self.pre_input = false;
        self.timer = 1;
    }

    /// Title and Options are two states of the same original MainMenu chain.
    /// Transfer its live VMs so they keep advancing while Options owns them.
    pub(crate) fn swap_art(&mut self, art: &mut MenuArt) {
        std::mem::swap(&mut self.art, art);
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

#[cfg(test)]
mod tests {
    use super::*;

    fn check_title_rotation_survives_options(
        script: Vec<th06_formats::anm0::Instr>,
        sprites: Vec<th06_formats::anm0::Sprite>,
        angle: f32,
        quit: bool,
    ) {
        let mut title = Title::with_quit(MenuArt::title_script_fixture(script, sprites), quit);
        for _ in 0..120 {
            title.update(&Input::default());
        }
        title.update(&Input::from_buttons(1, 0));
        for _ in 0..120 {
            title.update(&Input::default());
        }
        assert!((title.art.test_rotation() - angle).abs() < 0.00001);
        let mut options = crate::options::OptionsMenu::new(MenuArt::title_test_fixture());
        title.swap_art(&mut options.art);
        options.begin();
        for _ in 0..120 {
            options.update(&Input::default(), &mut crate::options::Options::default());
        }
        title.swap_art(&mut options.art);
        title.resume_menu(6);
        for tick in 1..=360 {
            title.update(&Input::default());
            if [36, 120, 360].contains(&tick) {
                assert!(
                    (title.art.test_rotation() - angle).abs() < 0.00001,
                    "after Options tick={tick}"
                );
            }
        }
        // A demo returns through a new MainMenu chain. Its first calculation
        // must execute label1 with neutral input before a new confirmation.
        title.reset(0);
        title.update(&Input::default());
        title.update(&Input::from_buttons(1, 0));
        for _ in 0..120 {
            title.update(&Input::default());
        }
        assert!((title.art.test_rotation() - angle).abs() < 0.00001);
    }

    #[test]
    fn options_transfers_live_vms_and_preserves_startup_rotation_in_both_quit_modes() {
        let angle = -3.0 * std::f32::consts::PI / 8.0;
        let mut script = Vec::new();
        let mut offset = 0;
        for (opcode, args) in [
            (21, vec![]),
            (22, vec![1]),
            (9, vec![0, 0, angle.to_bits()]),
            (21, vec![]),
            (22, vec![2]),
            (21, vec![]),
            (22, vec![3]),
            (21, vec![]),
        ] {
            script.push(th06_formats::anm0::Instr {
                time: 0,
                opcode,
                args: args.iter().flat_map(|n| n.to_le_bytes()).collect(),
                offset,
            });
            offset += 4 + args.len() as u32 * 4;
        }
        for quit in [false, true] {
            check_title_rotation_survives_options(script.clone(), Vec::new(), angle, quit);
        }
    }

    #[test]
    #[ignore = "requires original title03.anm at TH06_TITLE03_ANM"]
    fn original_title03_subtitles_keep_diagonal_after_options_and_new_demo_chain() {
        let bytes = std::fs::read(std::env::var_os("TH06_TITLE03_ANM").unwrap()).unwrap();
        let anm = th06_formats::anm0::Anm0::parse(&bytes).unwrap();
        let entry = &anm.entries[0];
        for id in [1, 2] {
            let script = entry
                .scripts
                .iter()
                .find(|(n, _)| *n == id)
                .unwrap()
                .1
                .clone();
            for quit in [false, true] {
                check_title_rotation_survives_options(
                    script.clone(),
                    entry.sprites.clone(),
                    -3.0 * std::f32::consts::PI / 8.0,
                    quit,
                );
            }
        }
    }

    fn ready(quit_enabled: bool, cursor: usize) -> Title {
        let mut title = Title::with_quit(MenuArt::title_test_fixture(), quit_enabled);
        title.resume_menu(cursor);
        for _ in 0..20 {
            title.update(&Input::default());
        }
        title
    }

    fn press(title: &mut Title, buttons: u16) -> TitleAction {
        let action = title.update(&Input::from_buttons(buttons, 0)).1;
        title.update(&Input::default());
        action
    }

    #[test]
    fn web_navigation_wraps_without_quit_with_extra_locked_or_unlocked() {
        for unlocked in [false, true] {
            let mut title = ready(false, 0);
            title.set_extra_unlocked(unlocked);
            let down = if unlocked {
                vec![1, 2, 3, 4, 5, 6, 0]
            } else {
                vec![2, 3, 4, 5, 6, 0]
            };
            for cursor in down {
                assert_eq!(press(&mut title, 1 << 5), TitleAction::None);
                assert_eq!(title.cursor, cursor);
            }
            let up = if unlocked {
                vec![6, 5, 4, 3, 2, 1, 0]
            } else {
                vec![6, 5, 4, 3, 2, 0]
            };
            for cursor in up {
                assert_eq!(press(&mut title, 1 << 4), TitleAction::None);
                assert_eq!(title.cursor, cursor);
            }
        }
    }

    #[test]
    fn web_quit_row_is_hidden_during_startup_and_active_menu() {
        let mut title = Title::with_quit(MenuArt::title_test_fixture(), false);
        assert_eq!(title.draw().len(), 8); // Background plus seven visible rows.
        assert!(title.draw().iter().all(|cmd| cmd.tex != 8));
        title.resume_menu(6);
        assert_eq!(title.draw().len(), 8);
        assert!(title.draw().iter().all(|cmd| cmd.tex != 8));
        let mut native = ready(true, 0);
        assert_eq!(native.draw().len(), 9);
        assert!(native.draw().iter().any(|cmd| cmd.tex == 8));
    }

    #[test]
    fn web_cancel_and_quit_shortcuts_never_select_or_start_quit() {
        for buttons in [1 << 1, 1 << 3, 1 << 9] {
            let mut title = ready(false, 6);
            assert_eq!(press(&mut title, buttons), TitleAction::None);
            assert_eq!(title.cursor, 6);
            assert!(title.quitting.is_none());
            for _ in 0..61 {
                assert_eq!(title.update(&Input::default()).1, TitleAction::None);
            }
        }
    }

    #[test]
    fn web_quit_key_does_not_cancel_a_simultaneous_game_confirmation() {
        for confirm in [1, 1 << 12] {
            let mut title = ready(false, 0);
            title.update(&Input::from_buttons(confirm | (1 << 9), 0));
            assert_eq!(title.pending, Some(TitleAction::StartGame));
            for _ in 0..59 {
                assert_eq!(title.update(&Input::default()).1, TitleAction::None);
            }
            assert_eq!(title.update(&Input::default()).1, TitleAction::StartGame);
        }
    }

    #[test]
    fn web_menu_returns_normalize_a_stale_quit_cursor() {
        let mut title = ready(false, 0);
        title.reset(7);
        assert_eq!(title.cursor, 6);
        title.begin_startup(7);
        assert_eq!(title.cursor, 6);
        title.resume_menu(7);
        assert_eq!(title.cursor, 6);
        assert_eq!(press(&mut title, 1 << 5), TitleAction::None);
        assert_eq!(title.cursor, 0);
    }

    #[test]
    fn native_keeps_quit_navigation_cancel_and_original_exit_delay() {
        let mut title = ready(true, 0);
        press(&mut title, 1 << 4);
        assert_eq!(title.cursor, 7);
        press(&mut title, 1 << 5);
        assert_eq!(title.cursor, 0);
        press(&mut title, 1 << 3);
        assert_eq!(title.cursor, 7);
        title.update(&Input::from_buttons(1, 0));
        for _ in 0..59 {
            assert_eq!(title.update(&Input::default()).1, TitleAction::None);
        }
        assert_eq!(title.update(&Input::default()).1, TitleAction::Quit);
    }

    #[test]
    fn web_fade_fallback_cannot_emit_quit() {
        let mut title = ready(false, 0);
        title.quitting = Some(1);
        assert_eq!(title.update(&Input::default()).1, TitleAction::None);
    }
}
