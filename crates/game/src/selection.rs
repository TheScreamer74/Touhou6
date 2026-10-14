//! MainMenu.cpp difficulty/character/shot/practice states and original ANM interrupts.
use crate::{
    CHARACTERS, Character,
    menu_art::{MenuArt, MenuFont, MenuInput},
    stage,
};
use th06_engine::{DrawCmd, Input, Key};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunMode {
    Story,
    Extra,
    Practice,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectionStep {
    Difficulty,
    Character,
    Shot,
    PracticeStage,
    TitleUnload,
    TitleStartup,
}
pub enum SelectionAction {
    None,
    Back {
        cursor: usize,
    },
    Start {
        character: Character,
        difficulty: stage::Difficulty,
        stage: usize,
        mode: RunMode,
    },
}

pub struct Selection {
    pub art: MenuArt,
    font: MenuFont,
    pub mode: RunMode,
    pub step: SelectionStep,
    pub difficulty: usize,
    pub character: usize,
    pub shot: usize,
    cursor: usize,
    timer: u16,
    pub(crate) input: MenuInput,
}
impl Selection {
    pub fn new(art: MenuArt, font: MenuFont) -> Self {
        Self {
            art,
            font,
            mode: RunMode::Story,
            step: SelectionStep::Difficulty,
            difficulty: 1,
            character: 0,
            shot: 0,
            cursor: 1,
            timer: 0,
            input: MenuInput::default(),
        }
    }
    pub fn begin(&mut self, mode: RunMode, default_difficulty: usize) {
        self.mode = mode;
        self.difficulty = if mode == RunMode::Extra {
            4
        } else {
            default_difficulty.min(3)
        };
        self.step = SelectionStep::Difficulty;
        self.cursor = if mode == RunMode::Extra {
            0
        } else {
            self.difficulty
        };
        // Difficulty loading retains MainMenu's timer at 60, then the update
        // increments it. Confirmation is available on the next calculation.
        self.timer = 61;
        self.art.reset();
        self.art
            .interrupt_all(if mode == RunMode::Extra { 18 } else { 6 });
    }
    fn character_vms(&mut self) {
        for c in 0..2 {
            for s in 0..2 {
                if c != self.character {
                    self.art.interrupt(6 + c * 2 + s, 0);
                }
            }
        }
    }
    fn shot_vms(&mut self) {
        for c in 0..2 {
            for s in 0..2 {
                if c != self.character {
                    self.art.interrupt(12 + c * 2 + s, 0);
                }
            }
        }
    }
    pub fn update(
        &mut self,
        input: &Input,
        extra_unlocked: [bool; 4],
        practice_stages: usize,
    ) -> SelectionAction {
        let action = self.update_inner(input, extra_unlocked, practice_stages);
        self.timer = self.timer.saturating_add(1);
        action
    }
    fn update_inner(
        &mut self,
        input: &Input,
        extra_unlocked: [bool; 4],
        practice_stages: usize,
    ) -> SelectionAction {
        if self.step == SelectionStep::TitleStartup {
            return SelectionAction::Back {
                cursor: self.cursor,
            };
        }
        self.input.tick(input);
        self.art.tick();
        if self.step == SelectionStep::TitleUnload {
            // MainMenu::STATE_CHARACTER_LOAD runs through timer 36 before
            // STATE_STARTUP reloads the title ANM on the following calc.
            if self.timer == 36 {
                self.step = SelectionStep::TitleStartup;
                self.timer = 0;
            }
            return SelectionAction::None;
        }
        // These three original states navigate during their entrance motion;
        // confirmation waits until frame 30. Character scrolling waits too.
        let up = self.input.pressed(input, Key::Up);
        let down = self.input.pressed(input, Key::Down);
        match self.step {
            SelectionStep::Difficulty if self.mode != RunMode::Extra => {
                if up {
                    self.art.sound("select00");
                    self.cursor = (self.cursor + 3) % 4;
                } else if down {
                    self.art.sound("select00");
                    self.cursor = (self.cursor + 1) % 4;
                }
                self.difficulty = self.cursor;
            }
            SelectionStep::Shot if up || down => {
                self.art.sound("select00");
                let next = 1 - self.shot;
                if self.mode != RunMode::Extra || extra_unlocked[self.character * 2 + next] {
                    self.shot = next;
                }
            }
            SelectionStep::PracticeStage => {
                let n = practice_stages.max(1);
                if up {
                    self.art.sound("select00");
                    self.cursor = (self.cursor + n - 1) % n;
                } else if down {
                    self.art.sound("select00");
                    self.cursor = (self.cursor + 1) % n;
                }
            }
            _ => {}
        }
        if self.timer < 30 {
            return SelectionAction::None;
        }
        let back = input.pressed(Key::Bomb) || input.pressed(Key::Pause);
        let confirm = input.pressed(Key::Shoot) || input.pressed(Key::Enter);
        match self.step {
            SelectionStep::Difficulty => {
                if back {
                    self.art.sound("cancel00");
                    self.art.interrupt_all(4);
                    self.step = SelectionStep::TitleUnload;
                    self.cursor = match self.mode {
                        RunMode::Story => 0,
                        RunMode::Extra => 1,
                        RunMode::Practice => 2,
                    };
                    self.timer = 0;
                    return SelectionAction::None;
                }
                if confirm {
                    self.art.sound("ok00");
                    if self.mode == RunMode::Extra
                        && !extra_unlocked[self.character * 2]
                        && !extra_unlocked[self.character * 2 + 1]
                    {
                        self.character = 1 - self.character;
                    }
                    self.art.interrupt_all(7);
                    self.art.interrupt(1 + self.difficulty, 8);
                    self.character_vms();
                    self.step = SelectionStep::Character;
                    self.cursor = self.character;
                    self.timer = 0;
                }
            }
            SelectionStep::Character => {
                if self.input.pressed(input, Key::Left) || self.input.pressed(input, Key::Right) {
                    let next = 1 - self.character;
                    if self.mode != RunMode::Extra
                        || extra_unlocked[next * 2]
                        || extra_unlocked[next * 2 + 1]
                    {
                        self.character = next;
                        self.art.sound("select00");
                        let (selected, other) = if self.input.pressed(input, Key::Left) {
                            (9, 12)
                        } else {
                            (10, 11)
                        };
                        for c in 0..2 {
                            for s in 0..2 {
                                self.art.interrupt(
                                    6 + c * 2 + s,
                                    if c == next { selected } else { other },
                                );
                            }
                        }
                    }
                }
                if back {
                    self.begin(self.mode, self.difficulty);
                    self.timer = 0;
                    self.art.sound("cancel00");
                } else if confirm {
                    self.art.sound("ok00");
                    if self.mode == RunMode::Extra
                        && !extra_unlocked[self.character * 2 + self.shot]
                    {
                        self.shot = 1 - self.shot;
                    }
                    self.art.interrupt_all(13);
                    self.art.interrupt(1 + self.difficulty, 0);
                    self.character_vms();
                    self.shot_vms();
                    self.step = SelectionStep::Shot;
                    self.cursor = self.shot;
                    self.timer = 0;
                }
            }
            SelectionStep::Shot => {
                if back {
                    self.art.sound("cancel00");
                    self.art.interrupt_all(7);
                    self.art.interrupt(1 + self.difficulty, 0);
                    self.character_vms();
                    self.step = SelectionStep::Character;
                    self.timer = 0;
                } else if confirm {
                    if self.mode == RunMode::Practice {
                        self.art.interrupt_all(19);
                        self.art.interrupt(1 + self.difficulty, 0);
                        self.character_vms();
                        self.shot_vms();
                        self.step = SelectionStep::PracticeStage;
                        self.cursor = 0;
                        self.timer = 0;
                    } else {
                        self.art.sound("ok00");
                        return self.start(if self.mode == RunMode::Extra { 6 } else { 0 });
                    }
                }
            }
            SelectionStep::PracticeStage => {
                if back {
                    self.art.sound("ok00");
                    self.art.interrupt_all(13);
                    self.art.interrupt(1 + self.difficulty, 0);
                    self.character_vms();
                    self.shot_vms();
                    self.step = SelectionStep::Shot;
                    self.timer = 0;
                } else if confirm {
                    self.art.sound("ok00");
                    return self.start(self.cursor);
                }
            }
            SelectionStep::TitleUnload | SelectionStep::TitleStartup => unreachable!(),
        }
        SelectionAction::None
    }
    fn start(&self, stage: usize) -> SelectionAction {
        SelectionAction::Start {
            character: CHARACTERS[self.character * 2 + self.shot],
            difficulty: [
                stage::Difficulty::Easy,
                stage::Difficulty::Normal,
                stage::Difficulty::Hard,
                stage::Difficulty::Lunatic,
                stage::Difficulty::Extra,
            ][self.difficulty],
            stage,
            mode: self.mode,
        }
    }
    pub fn reference_navigation(&self) -> (u8, usize, u16) {
        let (state, cursor) = match self.step {
            SelectionStep::Difficulty => (7, self.cursor),
            SelectionStep::Character => (9, self.character),
            SelectionStep::Shot => (11, self.shot),
            SelectionStep::PracticeStage => (17, self.cursor),
            SelectionStep::TitleUnload => (8, self.cursor),
            SelectionStep::TitleStartup => (0, self.cursor),
        };
        (state, cursor, self.timer)
    }
    pub fn take_sounds(&mut self) -> Vec<&'static str> {
        self.art.take_sounds()
    }
    pub fn draw(&self, practice_scores: &[i64], practice_stages: usize) -> Vec<DrawCmd> {
        let mut cmds = self.art.draw(|idx| {
            let mut a = 1.0;
            let mut offset = [0.0, 0.0];
            if self.step == SelectionStep::Difficulty && (1..=5).contains(&idx) {
                if (self.mode == RunMode::Extra) != (idx == 5) {
                    return None;
                }
                if idx == 1 + self.difficulty {
                    offset = [-6.0, -6.0];
                } else {
                    a = 96.0 / 255.0;
                }
            }
            if self.step == SelectionStep::Shot && (12..16).contains(&idx) {
                if (idx - 12) / 2 != self.character {
                    return None;
                }
                if (idx - 12) % 2 == self.shot {
                    offset = [-6.0, -6.0];
                } else {
                    a = 160.0 / 255.0;
                }
            }
            Some((offset, [1.0, 1.0, 1.0, a]))
        });
        if self.step == SelectionStep::PracticeStage {
            let alpha = (self.timer.min(30) as f32) / 30.0;
            for i in 0..practice_stages {
                let c = if i == self.cursor {
                    [192.0 / 255.0, 240.0 / 255.0, 240.0 / 255.0, alpha]
                } else {
                    [128.0 / 255.0, 192.0 / 255.0, 192.0 / 255.0, alpha * 0.5]
                };
                self.font.text(
                    &mut cmds,
                    [320.0, 200.0 + i as f32 * 24.0],
                    [1.0; 2],
                    c,
                    false,
                    &format!(
                        "STAGE {}  {:09}",
                        i + 1,
                        practice_scores.get(i).copied().unwrap_or(0)
                    ),
                );
            }
        }
        cmds
    }
}
