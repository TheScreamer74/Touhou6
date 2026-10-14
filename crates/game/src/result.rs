//! ResultScreen.cpp: records, spellcards, name keyboard, final stats and replay save.
use crate::menu_art::{MenuArt, MenuFont, MenuInput};
use std::collections::HashMap;
use th06_engine::{DrawCmd, Engine, Input, Key, Texture};
use th06_formats::{
    replay::Replay,
    score::{HighScore, ScoreData},
};

#[derive(Clone, Debug, Default)]
pub struct RunStats {
    /// Original character/shot index: ReimuA, ReimuB, MarisaA, MarisaB.
    pub character: u8,
    pub difficulty: u8,
    pub score: u32,
    pub stage: u8,
    pub completed: bool,
    pub continues: u8,
    pub deaths: u32,
    pub bombs_used: u32,
    pub spellcards_captured: u32,
    pub point_items: u32,
    pub graze: u32,
    pub frames: u32,
    pub slowdown: f32,
    pub date: String,
}
#[derive(Debug, PartialEq, Eq)]
pub enum ResultAction {
    None,
    Back,
    SaveScore { name: String },
    SaveReplay { slot: usize, name: String },
    UnlockAll,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    Init,
    Menu,
    Scores,
    Spells,
    ScoreName,
    Stats,
    StatsLeaving,
    SaveQuestion,
    CannotSave,
    ChooseSlot,
    ReplayName,
    Overwrite,
    Leaving,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Table {
    Scores,
    Spells,
}
struct TextSprite {
    tex: usize,
    w: f32,
    h: f32,
}
struct ReplayInfo {
    name: String,
    date: String,
    character: u8,
    score: u32,
}
pub struct ResultScreen {
    art: MenuArt,
    font: MenuFont,
    input: MenuInput,
    state: State,
    last_table: Table,
    timer: u16,
    cursor: usize,
    difficulty: usize,
    character: usize,
    last_character: usize,
    last_spell_page: usize,
    keyboard: usize,
    name_cursor: usize,
    replay_slot: usize,
    name: [u8; 8],
    progress: ScoreData,
    slots: Vec<Option<ReplayInfo>>,
    slots_ready: bool,
    stats: Option<RunStats>,
    replay_available: bool,
    pending_score: Option<HighScore>,
    pending_rank: usize,
    character_names: [TextSprite; 4],
    spell_names: Vec<Option<TextSprite>>,
    unknown_spell: TextSprite,
    unlock_code: UnlockCode,
}
const KEYBOARD:&[u8;96]=b"ABCDEFGHIJKLMNOPQRSTUVWXYZ.,:;\xa5@abcdefghijklmnopqrstuvwxyz+-/*=%0123456789(){}[]<>#!?'\"$      --";
const SHORT_CHARACTERS: [&str; 4] = ["ReimuA ", "ReimuB ", "MarisaA", "MarisaB"];
const RIGHT_DIFFICULTY: [&str; 5] = [
    "     Easy",
    "   Normal",
    "     Hard",
    "  Lunatic",
    "    Extra",
];

#[derive(Default)]
struct UnlockCode {
    step: u8,
    previous: u16,
}
impl UnlockCode {
    fn update(&mut self, input: &Input, active: bool) -> bool {
        let buttons = input.buttons();
        let changed_mask = |mask| buttons & mask != 0 && buttons & mask != self.previous & mask;
        let mut unlocked = false;
        if !active {
            self.previous = buttons;
            return false;
        }
        if !input.held(Key::Focus) && !input.held(Key::Skip) {
            self.step = 0;
        } else if self.step == 10 {
            self.step = 0;
            unlocked = true;
        } else {
            let expected = if self.step < 5 {
                1 << 11
            } else if self.step < 7 {
                1 << 9
            } else {
                1 << 10
            };
            if changed_mask(expected) {
                self.step += 1;
            } else if changed_mask(1 | 2 | 8 | 512 | 1024 | 4096) {
                self.step = 0;
            }
        }
        self.previous = buttons;
        unlocked
    }
}

impl ResultScreen {
    pub fn new(
        engine: &Engine,
        tl: &HashMap<String, Vec<u8>>,
        inn: &HashMap<String, Vec<u8>>,
        textures: &mut Vec<Texture>,
    ) -> Self {
        let art = MenuArt::load(
            engine,
            tl,
            textures,
            "result.jpg",
            &[
                ("result00.anm", 0),
                ("result01.anm", 8),
                ("result02.anm", 15),
                ("result03.anm", 37),
            ],
        );
        let font = MenuFont::load(engine, inn, textures);
        let character_names = [
            "博麗 霊夢 (霊)",
            "博麗 霊夢 (夢)",
            "霧雨 魔理沙 (魔)",
            "霧雨 魔理沙 (恋)",
        ]
        .map(|name| bake(engine, textures, name));
        let unknown_spell = bake(engine, textures, "？？？？？");
        Self {
            art,
            font,
            input: MenuInput::default(),
            state: State::Init,
            last_table: Table::Scores,
            timer: 0,
            cursor: 1,
            difficulty: 1,
            character: 0,
            last_character: 0,
            last_spell_page: 0,
            keyboard: 0,
            name_cursor: 0,
            replay_slot: 0,
            name: [b' '; 8],
            progress: ScoreData::default(),
            slots: vec![],
            slots_ready: false,
            stats: None,
            replay_available: false,
            pending_score: None,
            pending_rank: 10,
            character_names,
            spell_names: vec![],
            unknown_spell,
            unlock_code: UnlockCode::default(),
        }
    }
    /// Pre-bake the 64 original spell names from ECL while the GPU texture
    /// collection is being built. Progress controls which names are revealed.
    pub fn load_spell_names(
        &mut self,
        engine: &Engine,
        textures: &mut Vec<Texture>,
        names: impl IntoIterator<Item = (i32, Vec<u8>)>,
    ) {
        self.spell_names = (0..64).map(|_| None).collect();
        for (id, name) in names {
            if (0..64).contains(&id) {
                self.spell_names[id as usize] = Some(bake(
                    engine,
                    textures,
                    &th06_engine::text::decode_shiftjis(&name),
                ));
            }
        }
    }
    pub fn begin_browse(&mut self, progress: &ScoreData) {
        self.unlock_code = UnlockCode::default();
        self.progress = progress.clone();
        self.slots_ready = false;
        self.stats = None;
        self.pending_score = None;
        self.cursor = 1;
        self.last_character = 0;
        self.last_spell_page = 0;
        self.character = 0;
        self.difficulty = 1;
        self.last_table = Table::Scores;
        self.art.reset();
        self.enter(State::Init, 1);
    }
    pub fn begin_run(&mut self, stats: RunStats, progress: &ScoreData, replay_available: bool) {
        self.progress = progress.clone();
        self.slots_ready = false;
        self.character = stats.character as usize / 2;
        self.difficulty = stats.difficulty as usize;
        self.last_table = Table::Scores;
        self.name = [b' '; 8];
        self.name_cursor = 0;
        self.keyboard = 0;
        self.cursor = 0;
        self.pending_score = None;
        let scores = score_table(&self.progress, stats.character, stats.difficulty, None);
        self.pending_rank = scores
            .iter()
            .position(|s| s.score <= stats.score)
            .unwrap_or(10);
        if self.pending_rank < 10 {
            self.pending_score = Some(HighScore {
                name: "        ".into(),
                score: stats.score,
                character: stats.character,
                difficulty: stats.difficulty,
                stage: if stats.completed { 99 } else { stats.stage },
            });
        }
        self.replay_available = replay_available;
        self.stats = Some(stats);
        self.art.reset();
        if self.pending_score.is_some() {
            self.enter(State::ScoreName, self.difficulty as u32 + 3);
        } else {
            self.enter(State::Stats, 2);
        }
    }
    fn enter(&mut self, state: State, interrupt: u32) {
        self.state = state;
        self.timer = 0;
        self.art.interrupt_all(interrupt);
    }
    fn leave(&mut self) {
        self.enter(State::Leaving, 2);
    }
    fn choose_slots(&mut self) {
        self.enter(State::ChooseSlot, 10);
    }
    pub fn update(&mut self, input: &Input, slots: &[Option<Replay>]) -> ResultAction {
        self.input.tick(input);
        if !self.slots_ready {
            self.slots = slots
                .iter()
                .take(15)
                .map(|r| {
                    r.as_ref().map(|r| ReplayInfo {
                        name: r.name.clone(),
                        date: r.date.clone(),
                        character: r.character,
                        score: r.score,
                    })
                })
                .collect();
            self.slots_ready = true;
        }
        let confirm = confirm(input);
        let back = input.pressed(Key::Bomb);
        let menu = input.pressed(Key::Pause);
        let mut action = ResultAction::None;
        // Original accepts the code only on the Extra score table, including
        // its entrance animation, before applying normal table navigation.
        if self
            .unlock_code
            .update(input, self.state == State::Scores && self.difficulty == 4)
        {
            self.progress.stages_cleared = [[99; 5]; 4];
            self.progress.stages_cleared_without_continues = [[99; 5]; 4];
            self.art.sound("extend");
            action = ResultAction::UnlockAll;
        }
        if self.timer == 20 {
            if self.state == State::Scores {
                self.character = self.cursor;
            }
            if self.state == State::Spells {
                self.last_spell_page = self.cursor;
            }
        }
        match self.state {
            State::Init => {
                if self.timer >= 20 {
                    self.state = State::Menu;
                    self.timer = 0;
                }
            }
            State::Menu => {
                move_vertical(&self.input, input, &mut self.cursor, 7, &mut self.art);
                if confirm {
                    if self.cursor < 5 {
                        self.difficulty = self.cursor;
                        self.cursor = self.last_character;
                        self.character = self.cursor;
                        self.last_table = Table::Scores;
                        self.enter(State::Scores, self.difficulty as u32 + 3);
                    } else if self.cursor == 5 {
                        self.cursor = self.last_spell_page;
                        self.last_table = Table::Spells;
                        self.enter(State::Spells, 8);
                    } else {
                        self.art.sound("cancel00");
                        self.leave();
                    }
                }
                if back || menu {
                    self.art.sound("cancel00");
                    self.cursor = 6;
                }
            }
            State::Scores if self.timer >= 30 => {
                if move_horizontal(&self.input, input, &mut self.cursor, 2, &mut self.art) {
                    self.timer = 0;
                    self.art.interrupt_all(self.difficulty as u32 + 3);
                }
                if back || menu {
                    self.art.sound("cancel00");
                    self.last_character = self.cursor;
                    self.cursor = self.difficulty;
                    self.enter(State::Init, 1);
                }
            }
            State::Spells if self.timer >= 30 => {
                if move_horizontal(&self.input, input, &mut self.cursor, 7, &mut self.art) {
                    self.timer = 0;
                    self.art.interrupt_all(8);
                }
                if back || menu {
                    self.art.sound("cancel00");
                    self.last_spell_page = self.cursor;
                    self.cursor = 5;
                    self.enter(State::Init, 1);
                }
            }
            State::ScoreName if self.timer >= 30 => {
                keyboard_move(&self.input, input, &mut self.keyboard, &mut self.art);
                if self.input.pressed(input, Key::Shoot) || self.input.pressed(input, Key::Enter) {
                    if self.keyboard == 95 {
                        self.art.sound("cancel00");
                        action = self.finish_score();
                    } else {
                        self.type_character();
                    }
                }
                if self.input.pressed(input, Key::Bomb) || self.input.pressed(input, Key::Pause) {
                    self.erase_character();
                }
                if menu && self.state == State::ScoreName {
                    self.art.sound("cancel00");
                    action = self.finish_score();
                }
            }
            State::Stats => {
                if self.timer <= 30 {
                    self.art.interrupt(37, 16);
                }
                if self.timer >= 90 && confirm {
                    self.state = State::StatsLeaving;
                    self.timer = 0;
                    self.art.interrupt(37, 2);
                }
            }
            State::StatsLeaving => {
                if self.timer >= 30 {
                    self.state = State::SaveQuestion;
                    self.timer = 59;
                }
            }
            State::SaveQuestion => {
                if self.timer == 60 {
                    let label = if self.stats.as_ref().is_some_and(|s| s.continues != 0) {
                        12
                    } else if !self.replay_available {
                        13
                    } else {
                        9
                    };
                    self.art.interrupt_all(label);
                    self.cursor = 0;
                    if label != 9 {
                        self.state = State::CannotSave;
                    }
                }
                if self.timer >= 80 {
                    move_horizontal(&self.input, input, &mut self.cursor, 2, &mut self.art);
                    if back || menu {
                        self.art.sound("cancel00");
                        self.leave();
                    } else if confirm {
                        if self.cursor == 0 {
                            self.art.sound("ok00");
                            self.choose_slots();
                        } else {
                            self.art.sound("cancel00");
                            self.leave();
                        }
                    }
                }
            }
            State::CannotSave => {
                if self.timer >= 20 && (confirm || back || menu) {
                    self.art.sound("cancel00");
                    self.leave();
                }
            }
            State::ChooseSlot if self.timer >= 20 => {
                move_vertical(&self.input, input, &mut self.cursor, 15, &mut self.art);
                self.replay_slot = self.cursor;
                if confirm {
                    self.art.sound("ok00");
                    let occupied = slots.get(self.replay_slot).is_some_and(Option::is_some);
                    self.enter(
                        if occupied {
                            State::Overwrite
                        } else {
                            State::ReplayName
                        },
                        if occupied { 11 } else { 15 },
                    );
                    self.art.interrupt(22 + self.replay_slot, 14);
                    self.cursor = 0;
                    self.keyboard = 0;
                    self.name_cursor = 0;
                }
                if back {
                    self.art.sound("cancel00");
                    self.enter(State::SaveQuestion, 2);
                }
            }
            State::ReplayName if self.timer >= 30 => {
                keyboard_move(&self.input, input, &mut self.keyboard, &mut self.art);
                if self.input.pressed(input, Key::Shoot) || self.input.pressed(input, Key::Enter) {
                    if self.keyboard == 95 {
                        self.art.sound("ok00");
                        action = ResultAction::SaveReplay {
                            slot: self.replay_slot,
                            name: name_string(&self.name),
                        };
                        self.leave();
                    } else {
                        self.type_character();
                    }
                }
                if self.input.pressed(input, Key::Bomb) || self.input.pressed(input, Key::Pause) {
                    self.erase_character();
                }
                if menu {
                    self.art.sound("ok00");
                    self.choose_slots();
                }
            }
            State::Overwrite if self.timer >= 20 => {
                move_horizontal(&self.input, input, &mut self.cursor, 2, &mut self.art);
                if back || menu {
                    self.art.sound("ok00");
                    self.choose_slots();
                } else if confirm {
                    if self.cursor == 0 {
                        self.enter(State::ReplayName, 15);
                        self.art.interrupt(22 + self.replay_slot, 14);
                    } else {
                        self.art.sound("ok00");
                        self.choose_slots();
                    }
                }
            }
            State::Leaving if self.timer >= 60 => action = ResultAction::Back,
            _ => {}
        }
        if self.state == State::ScoreName
            && let Some(score) = &mut self.pending_score
        {
            score.name = name_string(&self.name);
        }
        self.art.tick();
        self.timer = self.timer.saturating_add(1);
        action
    }
    pub fn take_sounds(&mut self) -> Vec<&'static str> {
        self.art.take_sounds()
    }
    pub fn reference_state(&self) -> (u8, u16, usize, u8) {
        let state = match self.state {
            State::Init => 0,
            State::Menu => 1,
            State::Leaving => 2,
            State::Scores => 3 + self.difficulty as u8,
            State::Spells => 8,
            State::ScoreName => 9,
            State::SaveQuestion => 10,
            State::CannotSave => 11,
            State::ChooseSlot => 12,
            State::ReplayName => 13,
            State::Overwrite => 14,
            State::Stats => 15,
            State::StatsLeaving => 16,
        };
        (state, self.timer, self.cursor, self.unlock_code.step)
    }
    fn type_character(&mut self) {
        self.art.sound("ok00");
        self.name[self.name_cursor.min(7)] = if self.keyboard == 94 {
            b' '
        } else {
            KEYBOARD[self.keyboard]
        };
        if self.name_cursor < 8 {
            self.name_cursor += 1;
            self.cursor = self.name_cursor;
            if self.name_cursor == 8 {
                self.keyboard = 95;
            }
        }
    }
    fn erase_character(&mut self) {
        self.art.sound("cancel00");
        let at = self.name_cursor.min(7);
        if self.name_cursor > 0 {
            self.name_cursor -= 1;
            self.cursor = self.name_cursor;
            self.name[at] = b' ';
        }
    }
    fn finish_score(&mut self) -> ResultAction {
        let name = name_string(&self.name);
        if let Some(score) = &mut self.pending_score {
            score.name = name.clone();
        }
        self.enter(State::Stats, 2);
        ResultAction::SaveScore { name }
    }
    pub fn draw(&self) -> Vec<DrawCmd> {
        let replay_state = matches!(
            self.state,
            State::SaveQuestion
                | State::CannotSave
                | State::ChooseSlot
                | State::ReplayName
                | State::Overwrite
        );
        let style = |idx: usize| {
            let mut offset = [0.0; 2];
            let mut tint = [1.0; 4];
            if matches!(self.state, State::Init | State::Menu) && (1..=7).contains(&idx) {
                if idx == self.cursor + 1 {
                    offset = [-4.0, -4.0];
                } else {
                    tint[3] = 176.0 / 255.0;
                }
            }
            if matches!(self.state, State::SaveQuestion | State::Overwrite)
                && (idx == 16 || idx == 17)
            {
                tint = color(if idx - 16 == self.cursor {
                    0xffff6060
                } else {
                    0xff606060
                });
            }
            Some((offset, tint))
        };
        let mut cmds = self.art.draw(style);
        if self.art.position(14)[0] < 640.0 {
            if self.last_table == Table::Scores {
                self.draw_scores(&mut cmds);
            } else {
                self.draw_spells(&mut cmds);
            }
        }
        if self.state == State::ScoreName || self.state == State::ReplayName {
            self.draw_keyboard(&mut cmds);
        }
        if replay_state {
            // The original renders the six question VMs a second time here.
            let mut overlay = self.art.draw(|idx| {
                if (15..21).contains(&idx) {
                    style(idx)
                } else {
                    None
                }
            });
            overlay.remove(0);
            cmds.extend(overlay);
            self.draw_replays(&mut cmds);
        }
        if matches!(self.state, State::Stats | State::StatsLeaving) {
            self.draw_stats(&mut cmds);
        }
        cmds
    }
    fn draw_scores(&self, cmds: &mut Vec<DrawCmd>) {
        let base = self.art.position(14);
        for shot in 0..2 {
            let index = self.character * 2 + shot;
            let name = &self.character_names[index];
            // DrawStringFormat2 centers within a 256px sprite using SJIS byte count.
            let sjis_bytes = if self.character == 0 { 14.0 } else { 16.0 };
            let alpha = if self.state == State::ScoreName
                && self.stats.as_ref().unwrap().character as usize % 2 != shot
            {
                0.5
            } else {
                1.0
            };
            draw_text_sprite(
                cmds,
                name,
                [
                    base[0] + shot as f32 * 320.0 + 128.0 - sjis_bytes * 4.0,
                    base[1],
                ],
                alpha,
            );
            let scores = score_table(
                &self.progress,
                index as u8,
                self.difficulty as u8,
                self.pending_score.as_ref(),
            );
            for (row, s) in scores.iter().take(10).enumerate() {
                let pos = [
                    base[0] + if shot == 0 { 0.0 } else { 336.0 },
                    base[1] + 36.0 + row as f32 * 18.0,
                ];
                let active = self.state == State::ScoreName
                    && self.stats.as_ref().unwrap().character as usize == index
                    && row == self.pending_rank;
                let tint = if self.state == State::ScoreName {
                    if shot == 0 {
                        if active {
                            0xfff0f0ff
                        } else if self.stats.as_ref().unwrap().character.is_multiple_of(2) {
                            0x80ffffc0
                        } else {
                            0x80ffc0c0
                        }
                    } else if active {
                        0xfffff0f0
                    } else if self.stats.as_ref().unwrap().character as usize % 2 == 1 {
                        0xc0c0c0ff
                    } else {
                        0x80c0c0ff
                    }
                } else if shot == 0 {
                    0xffffc0c0
                } else {
                    0xffc0c0ff
                };
                if active {
                    let mut cursor = [b' '; 8];
                    cursor[self.name_cursor.min(7)] = b'_';
                    self.font.text(
                        cmds,
                        pos,
                        [1.0; 2],
                        color(tint),
                        false,
                        &format!(
                            "{}{}",
                            if shot == 0 { "   " } else { "" },
                            name_string(&cursor)
                        ),
                    );
                }
                let mut name_pos = pos;
                if shot == 0 {
                    self.font.text(
                        cmds,
                        pos,
                        [1.0; 2],
                        color(tint),
                        false,
                        &format!("{:2}", row + 1),
                    );
                    name_pos[0] += 36.0;
                }
                let stage = if s.stage <= 6 {
                    s.stage.to_string()
                } else if s.stage == 7 {
                    "1".into()
                } else {
                    "C".into()
                };
                self.font.text(
                    cmds,
                    name_pos,
                    [1.0; 2],
                    color(tint),
                    false,
                    &format!("{:>8} {:9}({})", s.name, s.score, stage),
                );
            }
        }
    }
    fn draw_spells(&self, cmds: &mut Vec<DrawCmd>) {
        let base = self.art.position(14);
        let page = self.last_spell_page;
        for row in 0..10 {
            let idx = page * 10 + row;
            if idx >= 64 {
                break;
            }
            let Some(record) = self.progress.spellcards.get(idx) else {
                break;
            };
            let pos = [base[0], base[1] + 16.0 + row as f32 * 30.0];
            let tint = if record.attempts == 0 {
                0x80c0c0ff
            } else if record.successes == 0 {
                0xffc0a0a0
            } else {
                0xfff0f0ff - row as u32 * 0x080800
            };
            self.font.text(
                cmds,
                pos,
                [1.0; 2],
                color(tint),
                false,
                &format!("No.{:02}", idx + 1),
            );
            let name = if record.attempts == 0 {
                &self.unknown_spell
            } else {
                self.spell_names
                    .get(idx)
                    .and_then(Option::as_ref)
                    .unwrap_or(&self.unknown_spell)
            };
            draw_text_sprite(cmds, name, [pos[0] + 96.0, pos[1]], 1.0);
            self.font.text(
                cmds,
                [pos[0] + 368.0, pos[1]],
                [1.0; 2],
                color(tint),
                false,
                &format!("{:3}/{:3}", record.successes, record.attempts),
            );
        }
    }
    fn draw_keyboard(&self, cmds: &mut Vec<DrawCmd>) {
        for (idx, &ch) in KEYBOARD.iter().enumerate() {
            let active = idx == self.keyboard;
            let scale = if active {
                let phase = (self.timer % 32) as f32 / 32.0;
                if self.timer % 64 < 32 {
                    1.2 + 0.8 * phase
                } else {
                    2.0 - 0.8 * phase
                }
            } else {
                1.0
            };
            let offset = -(scale - 1.0) * 8.0;
            self.font.bytes(
                cmds,
                [
                    160.0 + (idx % 16) as f32 * 20.0 + offset,
                    356.0 + (idx / 16) as f32 * 18.0 + offset,
                ],
                [scale; 2],
                color(if active { 0xffffffc0 } else { 0x60c0c0c0 }),
                false,
                &[if idx == 94 {
                    0x80
                } else if idx == 95 {
                    0x81
                } else {
                    ch
                }],
            );
        }
    }
    fn draw_replays(&self, cmds: &mut Vec<DrawCmd>) {
        self.font.text(
            cmds,
            self.art.position(21),
            [1.0; 2],
            [1.0; 4],
            false,
            "No.   Name     Date     Player Score",
        );
        for slot in 0..15 {
            let pos = self.art.position(22 + slot);
            let c = color(if slot == self.replay_slot {
                0xffff8080
            } else {
                0xff808080
            });
            let text = if self.state == State::ReplayName {
                let stats = self.stats.as_ref().unwrap();
                format!(
                    "No.{:02} {:>8} {:>8} {:>7} {:9}",
                    slot + 1,
                    name_string(&self.name),
                    stats.date,
                    SHORT_CHARACTERS[stats.character as usize],
                    stats.score
                )
            } else if let Some(r) = self.slots.get(slot).and_then(Option::as_ref) {
                format!(
                    "No.{:02} {:>8} {:>8} {:>7} {:9}",
                    slot + 1,
                    r.name,
                    r.date,
                    SHORT_CHARACTERS[r.character as usize],
                    r.score
                )
            } else {
                format!("No.{:02} -------- --/--/-- -------         0", slot + 1)
            };
            self.font.text(cmds, pos, [1.0; 2], c, false, &text);
            if self.state == State::ReplayName {
                let mut name = [b' '; 8];
                name[self.name_cursor.min(7)] = b'_';
                self.font.text(
                    cmds,
                    pos,
                    [1.0; 2],
                    color(0xfff0f0ff),
                    false,
                    &format!("      {}", name_string(&name)),
                );
            }
        }
    }
    fn draw_stats(&self, cmds: &mut Vec<DrawCmd>) {
        let Some(stats) = &self.stats else {
            return;
        };
        let base = self.art.position(37);
        let tint = [1.0, 1.0, 1.0, self.art.alpha(37)];
        let completion = if stats.difficulty != 0 && stats.completed {
            "      100%".into()
        } else {
            format!(
                "    {:.2}%",
                stats.frames as f32
                    / if stats.difficulty < 4 {
                        89500.0
                    } else {
                        39600.0
                    }
                    * 100.0
            )
        };
        let lines = [
            format!("{:9}", stats.score),
            RIGHT_DIFFICULTY[stats.difficulty as usize].into(),
            completion,
            format!("{:9}", stats.continues),
            format!("{:9}", stats.deaths),
            format!("{:9}", stats.bombs_used),
            format!("{:9}", stats.spellcards_captured),
            format!("    {:.2}%", stats.slowdown),
        ];
        for (row, text) in lines.iter().enumerate() {
            self.font.text(
                cmds,
                [base[0] + 224.0, base[1] + 32.0 + row as f32 * 22.0],
                [1.0; 2],
                tint,
                false,
                text,
            );
        }
    }
}
fn score_table(
    progress: &ScoreData,
    character: u8,
    difficulty: u8,
    pending: Option<&HighScore>,
) -> Vec<HighScore> {
    let mut scores = vec![];
    if let Some(s) = pending.filter(|s| s.character == character && s.difficulty == difficulty) {
        scores.push(s.clone());
    }
    scores.extend(
        progress
            .high_scores
            .iter()
            .filter(|s| s.character == character && s.difficulty == difficulty)
            .cloned(),
    );
    for slot in 0..10 {
        scores.push(HighScore {
            name: "Nanashi ".into(),
            score: 1_000_000 - slot * 100_000,
            character,
            difficulty,
            stage: 1,
        });
    }
    scores.sort_by_key(|s| std::cmp::Reverse(s.score));
    scores.truncate(10);
    scores
}
fn bake(engine: &Engine, textures: &mut Vec<Texture>, text: &str) -> TextSprite {
    let (_, height) = {
        let image = th06_engine::text::render(text, 15.0, [255; 3], Some([0; 3]));
        (image.width, image.height)
    };
    let (tex, w, h) = crate::localization::bake_text(
        engine,
        textures,
        text,
        15.0,
        [255; 3],
        Some([0; 3]),
        [256, height],
    );
    TextSprite { tex, w, h }
}
fn draw_text_sprite(cmds: &mut Vec<DrawCmd>, sprite: &TextSprite, pos: [f32; 2], alpha: f32) {
    cmds.push(DrawCmd {
        clip: None,
        tex: sprite.tex,
        dst: [pos[0], pos[1], sprite.w, sprite.h],
        src: [0.0, 0.0, 1.0, 1.0],
        tint: [1.0, 1.0, 1.0, alpha],
        rot: 0.0,
        additive: false,
        color_add: false,
        depth: None,
    });
}
fn confirm(input: &Input) -> bool {
    input.pressed(Key::Shoot) || input.pressed(Key::Enter)
}
fn color(argb: u32) -> [f32; 4] {
    [
        ((argb >> 16) & 255) as f32 / 255.0,
        ((argb >> 8) & 255) as f32 / 255.0,
        (argb & 255) as f32 / 255.0,
        (argb >> 24) as f32 / 255.0,
    ]
}
fn name_string(name: &[u8]) -> String {
    name.iter()
        .map(|&b| if b == 0xa5 { '･' } else { b as char })
        .collect()
}
fn move_vertical(
    repeat: &MenuInput,
    input: &Input,
    cursor: &mut usize,
    count: usize,
    art: &mut MenuArt,
) {
    if repeat.pressed(input, Key::Up) {
        art.sound("select00");
        *cursor = (*cursor + count - 1) % count;
    }
    if repeat.pressed(input, Key::Down) {
        art.sound("select00");
        *cursor = (*cursor + 1) % count;
    }
}
fn move_horizontal(
    repeat: &MenuInput,
    input: &Input,
    cursor: &mut usize,
    count: usize,
    art: &mut MenuArt,
) -> bool {
    if repeat.pressed(input, Key::Left) {
        art.sound("select00");
        *cursor = (*cursor + count - 1) % count;
        true
    } else if repeat.pressed(input, Key::Right) {
        art.sound("select00");
        *cursor = (*cursor + 1) % count;
        true
    } else {
        false
    }
}
fn keyboard_move(repeat: &MenuInput, input: &Input, cursor: &mut usize, art: &mut MenuArt) {
    for (key, delta) in [
        (Key::Up, -16),
        (Key::Down, 16),
        (Key::Left, -1),
        (Key::Right, 1),
    ] {
        if repeat.pressed(input, key) {
            art.sound("select00");
            *cursor = keyboard_next(*cursor, delta);
        }
    }
}
fn keyboard_next(mut cursor: usize, delta: i32) -> usize {
    for _ in 0..96 {
        cursor = if delta.abs() == 16 {
            (cursor as i32 + delta).rem_euclid(96) as usize
        } else {
            cursor / 16 * 16 + (cursor as i32 % 16 + delta).rem_euclid(16) as usize
        };
        if KEYBOARD[cursor] != b' ' {
            break;
        }
    }
    cursor
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn extra_unlock_requires_modifier_sequence_and_one_later_tick() {
        let mut code = UnlockCode::default();
        for key in [
            Key::Home,
            Key::Home,
            Key::Home,
            Key::Home,
            Key::Home,
            Key::Quit,
            Key::Quit,
            Key::S,
            Key::S,
            Key::S,
        ] {
            assert!(!code.update(&Input::synthetic(&[Key::Focus, key], &[key]), true));
            if code.step != 10 {
                assert!(!code.update(&Input::synthetic(&[Key::Focus], &[]), true));
            }
        }
        assert_eq!(code.step, 10);
        assert!(code.update(&Input::synthetic(&[Key::Focus], &[]), true));
        assert_eq!(code.step, 0);
        assert!(!code.update(&Input::synthetic(&[Key::Home], &[Key::Home]), true));
        assert_eq!(code.step, 0);
        code.update(
            &Input::synthetic(&[Key::Skip, Key::Home], &[Key::Home]),
            false,
        );
        assert!(!code.update(&Input::synthetic(&[Key::Skip, Key::Home], &[]), true));
        assert_eq!(code.step, 0);
        code.update(&Input::synthetic(&[Key::Skip], &[]), true);
        code.update(
            &Input::synthetic(&[Key::Skip, Key::Home], &[Key::Home]),
            true,
        );
        code.update(
            &Input::synthetic(&[Key::Skip, Key::Shoot], &[Key::Shoot]),
            true,
        );
        assert_eq!(code.step, 0);
    }
    #[test]
    fn keyboard_routes_skip_spaces_and_preserve_row() {
        assert_eq!(keyboard_next(0, -1), 15);
        assert_eq!(keyboard_next(15, 1), 0);
        assert_eq!(keyboard_next(80, 16), 0);
        assert_eq!(keyboard_next(87, 1), 94);
        assert_eq!(KEYBOARD[30], 0xa5);
        assert_eq!(&KEYBOARD[94..], b"--");
    }
    #[test]
    fn original_default_scores_and_equal_score_insertion() {
        let progress = ScoreData::default();
        let table = score_table(&progress, 0, 1, None);
        assert_eq!(table.len(), 10);
        assert_eq!(table[0].score, 1_000_000);
        assert_eq!(table[9].score, 100_000);
        let candidate = HighScore {
            name: "TEST".into(),
            score: 100_000,
            character: 0,
            difficulty: 1,
            stage: 2,
        };
        assert_eq!(
            score_table(&progress, 0, 1, Some(&candidate))[9].name,
            "TEST"
        );
    }
}
