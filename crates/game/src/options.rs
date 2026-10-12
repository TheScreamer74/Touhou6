//! MainMenu::OnUpdateOptionsMenu and ColorMenuItem, using the original sprites.
use crate::menu_art::{MenuArt, MenuInput};
use th06_engine::{DrawCmd, Input, Key};
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Options {
    pub life_count: u8,
    pub bomb_count: u8,
    pub color_16bit: bool,
    pub music_mode: u8,
    pub play_sounds: bool,
    pub windowed: bool,
    pub default_difficulty: u8,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            life_count: 2,
            bomb_count: 3,
            color_16bit: false,
            music_mode: 1,
            play_sounds: true,
            windowed: false,
            default_difficulty: 1,
        }
    }
}
impl Options {
    pub fn encode(&self) -> Vec<u8> {
        vec![
            1,
            self.life_count,
            self.bomb_count,
            self.color_16bit as u8,
            self.music_mode,
            self.play_sounds as u8,
            self.windowed as u8,
            self.default_difficulty,
        ]
    }
    pub fn parse(b: &[u8]) -> Option<Self> {
        if b.len() != 8 || b[0] != 1 || b[1] > 4 || b[2] > 3 || b[4] > 2 || b[7] > 3 {
            return None;
        }
        Some(Self {
            life_count: b[1],
            bomb_count: b[2],
            // The RGBA renderer cannot reproduce the original R5G6B5 buffer.
            color_16bit: false,
            music_mode: b[4].min(1),
            play_sounds: b[5] != 0,
            windowed: b[6] != 0,
            default_difficulty: b[7],
        })
    }
}
pub enum OptionsAction {
    None,
    Back,
    Changed,
    ControllerChanged,
}
pub struct OptionsMenu {
    pub art: MenuArt,
    cursor: usize,
    timer: u16,
    input: MenuInput,
    key_config: Option<crate::key_config::KeyConfig>,
}
impl OptionsMenu {
    pub fn new(art: MenuArt) -> Self {
        Self {
            art,
            cursor: 0,
            timer: 0,
            input: MenuInput::default(),
            key_config: None,
        }
    }
    pub fn begin(&mut self) {
        self.cursor = 0;
        self.timer = 0;
        self.key_config = None;
        self.art.reset();
        self.art.interrupt_all(3);
    }
    pub fn update(&mut self, input: &Input, options: &mut Options) -> OptionsAction {
        options.color_16bit = false;
        options.music_mode = options.music_mode.min(1);
        self.timer = self.timer.saturating_add(1);
        self.input.tick(input);
        self.art.tick();
        if let Some(config) = &mut self.key_config {
            if config.update(input, &mut self.art) {
                self.key_config = None;
                self.art.interrupt_all(3);
                self.cursor = 7;
                self.timer = 0;
                return OptionsAction::ControllerChanged;
            }
            return OptionsAction::None;
        }
        if self.input.pressed(input, Key::Up) {
            self.art.sound("select00");
            self.cursor = (self.cursor + 8) % 9;
        }
        if self.input.pressed(input, Key::Down) {
            self.art.sound("select00");
            self.cursor = (self.cursor + 1) % 9;
        }
        if self.timer < 32 {
            return OptionsAction::None;
        }
        if input.pressed(Key::Bomb) || input.pressed(Key::Pause) {
            self.cursor = 8;
            self.art.sound("cancel00");
        }
        let left = self.input.pressed(input, Key::Left);
        let right = self.input.pressed(input, Key::Right);
        if left || right {
            let delta = if left { -1 } else { 1 };
            match self.cursor {
                0 => options.life_count = (options.life_count as i32 + delta + 5) as u8 % 5,
                1 => options.bomb_count = (options.bomb_count as i32 + delta + 4) as u8 % 4,
                2 => return OptionsAction::None,
                // WAV and OFF are supported by both backends. MIDI requires a synthesizer.
                3 => options.music_mode = (options.music_mode + 1) % 2,
                4 => options.play_sounds = !options.play_sounds,
                5 => options.windowed = !options.windowed,
                _ => return OptionsAction::None,
            }
            self.art.sound("select00");
            return OptionsAction::Changed;
        }
        if input.pressed(Key::Shoot) || input.pressed(Key::Enter) {
            match self.cursor {
                6 => {
                    *options = Options::default();
                    return OptionsAction::Changed;
                }
                7 => {
                    self.art.sound("ok00");
                    self.key_config = Some(crate::key_config::KeyConfig::begin(&mut self.art));
                    return OptionsAction::None;
                }
                8 => {
                    self.art.sound("cancel00");
                    return OptionsAction::Back;
                }
                _ => {}
            }
        }
        OptionsAction::None
    }
    pub fn draw(&mut self, options: &Options) -> Vec<DrawCmd> {
        if let Some(config) = &self.key_config {
            return config.draw(&mut self.art);
        }
        // The original has six title01 label VMs and three title04 label VMs.
        let labels = [8, 9, 10, 11, 12, 72, 73, 74, 13];
        let mut selection = std::collections::HashMap::new();
        for (row, idx) in labels.iter().enumerate() {
            selection.insert(*idx, (row, row == self.cursor));
        }
        for (row, start, count, selected) in [
            (0, 14, 5, options.life_count as usize),
            (1, 19, 4, options.bomb_count as usize),
            (2, 23, 2, 0),
            (4, 25, 2, options.play_sounds as usize),
            (5, 75, 2, options.windowed as usize),
            (3, 77, 3, options.music_mode as usize),
        ] {
            for n in 0..count {
                selection.insert(start + n, (row, n == selected));
            }
        }
        for idx in 0..80 {
            self.art
                .highlight(idx, selection.get(&idx).map(|v| v.1).unwrap_or(false));
        }
        self.art.draw(|idx| {
            let mut offset = [0.0, 0.0];
            let mut alpha = 1.0;
            if let Some(&(row, selected)) = selection.get(&idx) {
                if selected {
                    offset = [-2.0, -2.0];
                }
                if row == self.cursor {
                    offset[0] -= 4.0;
                    offset[1] -= 4.0;
                } else {
                    alpha = 128.0 / 255.0;
                }
            }
            if idx == 10 || idx == 24 || idx == 79 {
                offset = [0.0; 2];
                alpha = 96.0 / 255.0;
            }
            Some((offset, [1.0, 1.0, 1.0, alpha]))
        })
    }
    pub fn take_sounds(&mut self) -> Vec<&'static str> {
        self.art.take_sounds()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn defaults_and_validation_match_original_ranges() {
        let o = Options::default();
        assert_eq!(o.life_count, 2);
        assert_eq!(o.bomb_count, 3);
        assert_eq!(Options::parse(&o.encode()), Some(o));
        let mut bad = Options::default().encode();
        bad[1] = 5;
        assert_eq!(Options::parse(&bad), None);
    }
    #[test]
    fn old_sixteen_bit_setting_is_normalized_to_supported_renderer() {
        let mut old = Options::default().encode();
        old[3] = 1;
        assert!(!Options::parse(&old).unwrap().color_16bit);
    }
    #[test]
    fn old_midi_setting_falls_back_to_available_wav_playback() {
        let mut old = Options::default().encode();
        old[4] = 2;
        assert_eq!(Options::parse(&old).unwrap().music_mode, 1);
    }
}
