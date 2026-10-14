//! MainMenu.cpp::ReplayHandling/DrawReplayMenu, original replay00 ANM.
use crate::menu_art::{MenuArt, MenuFont, MenuInput};
use std::collections::HashMap;
use th06_engine::{DrawCmd, Engine, Input, Key, Texture};
use th06_formats::replay::Replay;

#[derive(Debug, PartialEq, Eq)]
pub enum ReplayAction {
    None,
    Back,
    Play { slot: usize, stage: usize },
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    Files,
    Stages,
    Leaving,
    TitleStartup,
}
pub struct ReplayMenu {
    art: MenuArt,
    font: MenuFont,
    pub(crate) input: MenuInput,
    state: State,
    timer: u16,
    cursor: usize,
    chosen: usize,
}
const CHARACTERS: [&str; 4] = ["ReimuA ", "ReimuB ", "MarisaA", "MarisaB"];
const DIFFICULTIES: [&str; 5] = ["Easy   ", "Normal ", "Hard   ", "Lunatic", "Extra  "];
const STAGES: [&str; 7] = [
    "Stage1", "Stage2", "Stage3", "Stage4", "Stage5", "Stage6", "Extra ",
];
impl ReplayMenu {
    pub fn new(
        engine: &Engine,
        tl: &HashMap<String, Vec<u8>>,
        inn: &HashMap<String, Vec<u8>>,
        textures: &mut Vec<Texture>,
    ) -> Self {
        Self {
            art: MenuArt::load(engine, tl, textures, "select00.jpg", &[("replay00.anm", 0)]),
            font: MenuFont::load(engine, inn, textures),
            input: MenuInput::default(),
            state: State::Files,
            timer: 0,
            cursor: 0,
            chosen: 0,
        }
    }
    pub fn begin(&mut self) {
        self.art.reset();
        self.art.interrupt_all(15);
        self.state = State::Files;
        // STATE_REPLAY_LOAD retains its 60-tick title fade timer. Unlike
        // Files <- Stages, the initial load does not impose another delay.
        self.timer = 61;
        self.cursor = 0;
        self.chosen = 0;
    }
    pub fn update(&mut self, input: &Input, slots: &[Option<Replay>]) -> ReplayAction {
        if self.state == State::TitleStartup {
            // STATE_STARTUP executes Title's callback this calculation,
            // including its single Supervisor input-repeat tick.
            return ReplayAction::Back;
        }
        self.input.tick(input);
        let valid: Vec<usize> = slots
            .iter()
            .enumerate()
            .filter_map(|(i, r)| r.as_ref().map(|_| i))
            .collect();
        let mut action = ReplayAction::None;
        match self.state {
            State::Leaving if self.timer == 36 => {
                self.state = State::TitleStartup;
                self.timer = 0;
            }
            State::Files if self.timer >= 40 => {
                if !valid.is_empty() {
                    self.cursor = self.cursor.min(valid.len() - 1);
                    if self.input.pressed(input, Key::Up) {
                        self.art.sound("select00");
                        self.cursor = (self.cursor + valid.len() - 1) % valid.len();
                    } else if self.input.pressed(input, Key::Down) {
                        self.art.sound("select00");
                        self.cursor = (self.cursor + 1) % valid.len();
                    }
                    self.chosen = self.cursor;
                    if confirm(input) {
                        self.art.sound("ok00");
                        for i in 1..26 {
                            self.art.interrupt(i, 17);
                        }
                        self.art.interrupt(3 + self.chosen % 15, 16);
                        self.state = State::Stages;
                        self.timer = 0;
                        self.cursor = slots[valid[self.chosen]]
                            .as_ref()
                            .unwrap()
                            .stages
                            .iter()
                            .position(Option::is_some)
                            .unwrap();
                    }
                }
                if back(input) {
                    self.art.sound("cancel00");
                    self.state = State::Leaving;
                    self.timer = 0;
                    self.cursor = 0;
                    self.art.interrupt_all(4);
                }
            }
            State::Stages if self.timer >= 40 => {
                if let Some(replay) = valid
                    .get(self.chosen)
                    .and_then(|&slot| slots[slot].as_ref())
                {
                    let direction = if self.input.pressed(input, Key::Up) {
                        -1
                    } else if self.input.pressed(input, Key::Down) {
                        1
                    } else {
                        0
                    };
                    if direction != 0 {
                        self.art.sound("select00");
                        self.cursor = next_stage(replay, self.cursor, direction);
                    }
                    if confirm(input) {
                        action = ReplayAction::Play {
                            slot: valid[self.chosen],
                            stage: self.cursor + 1,
                        };
                    }
                }
                if back(input) {
                    self.art.sound("cancel00");
                    self.state = State::Files;
                    self.timer = 0;
                    self.cursor = self.chosen;
                    self.art.interrupt_all(15);
                }
            }
            _ => {}
        }
        self.art.tick();
        self.timer = self.timer.saturating_add(1);
        action
    }
    pub fn reference_navigation(&self) -> (u8, usize, u16) {
        let state = match self.state {
            State::Files => 13,
            State::Stages => 15,
            State::Leaving => 14,
            State::TitleStartup => 0,
        };
        (state, self.cursor, self.timer)
    }
    pub fn draw(&self, slots: &[Option<Replay>]) -> Vec<DrawCmd> {
        let mut cmds = self.art.draw(|idx| {
            if idx < 2 {
                Some(([0.0; 2], [1.0; 4]))
            } else {
                None
            }
        });
        self.font.text(
            &mut cmds,
            self.art.position(2),
            [1.0; 2],
            [1.0; 4],
            false,
            "No.   Name      Date     Player   Rank",
        );
        let valid: Vec<usize> = slots
            .iter()
            .enumerate()
            .filter_map(|(i, r)| r.as_ref().map(|_| i))
            .collect();
        let first = self.chosen - self.chosen % 15;
        for (row, &slot) in valid.iter().skip(first).take(15).enumerate() {
            let r = slots[slot].as_ref().unwrap();
            let c = if first + row == self.chosen {
                color(0xffff8080)
            } else {
                color(0xff808080)
            };
            let label = if slot < 15 {
                format!("No.{:02}", slot + 1)
            } else {
                "User ".into()
            };
            self.font.text(
                &mut cmds,
                self.art.position(3 + row),
                [1.0; 2],
                c,
                false,
                &format!(
                    "{} {:>8}  {:>8} {:>7}  {:>7}",
                    label,
                    r.name,
                    r.date,
                    CHARACTERS[r.character as usize],
                    if crate::localization::language() == crate::localization::Language::Original {
                        DIFFICULTIES[r.difficulty as usize].to_owned()
                    } else {
                        crate::localization::translate(DIFFICULTIES[r.difficulty as usize].trim())
                            .into_owned()
                    }
                ),
            );
        }
        if self.state == State::Stages
            && let Some(r) = valid.get(self.chosen).and_then(|&i| slots[i].as_ref())
        {
            self.font.text(
                &mut cmds,
                self.art.position(1),
                [1.0; 2],
                [1.0; 4],
                false,
                &format!("       {:.3}%", r.slowdown),
            );
            self.font.text(
                &mut cmds,
                self.art.position(18),
                [1.0; 2],
                [1.0; 4],
                false,
                "Stage  LastScore",
            );
            for (stage, label) in STAGES.iter().enumerate() {
                let value = r.stages[stage]
                    .as_ref()
                    .map_or("---------".into(), |s| format!("{:9}", s.score));
                self.font.text(
                    &mut cmds,
                    self.art.position(19 + stage),
                    [1.0; 2],
                    color(if stage == self.cursor {
                        0xffff8080
                    } else {
                        0xff808080
                    }),
                    false,
                    &format!(
                        "{} {}",
                        if crate::localization::language()
                            == crate::localization::Language::Original
                        {
                            (*label).to_owned()
                        } else if stage < 6 {
                            format!("{}{}", crate::localization::translate("Stage"), stage + 1)
                        } else {
                            crate::localization::translate("Extra").into_owned()
                        },
                        value
                    ),
                );
            }
        }
        cmds
    }
    pub fn take_sounds(&mut self) -> Vec<&'static str> {
        self.art.take_sounds()
    }
}
fn next_stage(replay: &Replay, cursor: usize, direction: i32) -> usize {
    let mut cursor = cursor;
    for _ in 0..7 {
        cursor = (cursor as i32 + direction).rem_euclid(7) as usize;
        if replay.stages[cursor].is_some() {
            return cursor;
        }
    }
    cursor
}
fn confirm(input: &Input) -> bool {
    input.pressed(Key::Shoot) || input.pressed(Key::Enter)
}
fn back(input: &Input) -> bool {
    input.pressed(Key::Bomb) || input.pressed(Key::Pause)
}
fn color(argb: u32) -> [f32; 4] {
    [
        ((argb >> 16) & 255) as f32 / 255.0,
        ((argb >> 8) & 255) as f32 / 255.0,
        (argb & 255) as f32 / 255.0,
        (argb >> 24) as f32 / 255.0,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stage_navigation_skips_unrecorded_stages_and_wraps() {
        let mut r = Replay {
            character: 0,
            difficulty: 1,
            date: String::new(),
            name: String::new(),
            score: 0,
            slowdown: 0.,
            stages: std::array::from_fn(|_| None),
        };
        let stage = th06_formats::replay::ReplayStage {
            score: 0,
            random_seed: 0,
            point_items: 0,
            power: 0,
            lives: 2,
            bombs: 3,
            rank: 16,
            power_item_count: 0,
            inputs: vec![],
        };
        r.stages[1] = Some(stage.clone());
        r.stages[5] = Some(stage);
        assert_eq!(next_stage(&r, 1, 1), 5);
        assert_eq!(next_stage(&r, 5, 1), 1);
        assert_eq!(next_stage(&r, 1, -1), 5);
    }
}
