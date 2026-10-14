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
    /// Inverse of original cfg.opts.useSwTextureBlending (default: true).
    /// Advanced configuration, preserved by the main menu's Restore Defaults.
    pub hardware_texture_blending: bool,
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
            hardware_texture_blending: false,
        }
    }
}
impl Options {
    pub fn encode(&self) -> Vec<u8> {
        vec![
            2,
            self.life_count,
            self.bomb_count,
            self.color_16bit as u8,
            self.music_mode,
            self.play_sounds as u8,
            self.windowed as u8,
            self.default_difficulty,
            self.hardware_texture_blending as u8,
        ]
    }
    pub fn parse(b: &[u8]) -> Option<Self> {
        let legacy = b.len() == 8 && b[0] == 1;
        let current = b.len() == 9 && b[0] == 2 && b[8] <= 1;
        if (!legacy && !current) || b[1] > 4 || b[2] > 3 || b[4] > 2 || b[7] > 3 {
            return None;
        }
        Some(Self {
            life_count: b[1],
            bomb_count: b[2],
            color_16bit: b[3] != 0,
            music_mode: b[4],
            play_sounds: b[5] != 0,
            windowed: b[6] != 0,
            default_difficulty: b[7],
            hardware_texture_blending: current && b[8] != 0,
        })
    }
}
pub enum OptionsAction {
    None,
    Back,
    Changed,
    DefaultsRestored,
    ControllerChanged,
}
#[derive(Debug, PartialEq, Eq)]
pub enum OptionsAudio {
    Sound(&'static str),
    Music(u8),
}
pub struct OptionsMenu {
    pub art: MenuArt,
    cursor: usize,
    timer: u16,
    pub(crate) input: MenuInput,
    key_config: Option<crate::key_config::KeyConfig>,
    pub midi_available: bool,
    music_changes: Vec<(u8, Vec<&'static str>)>,
    presentation_options: Option<Options>,
    presentation_cursor: usize,
}
impl OptionsMenu {
    pub fn new(art: MenuArt) -> Self {
        Self {
            art,
            cursor: 0,
            timer: 0,
            input: MenuInput::default(),
            key_config: None,
            midi_available: false,
            music_changes: Vec::new(),
            presentation_options: None,
            presentation_cursor: 0,
        }
    }
    pub fn begin(&mut self) {
        self.cursor = 0;
        // The MainMenu update which enters Options also increments its timer.
        self.timer = 1;
        self.key_config = None;
        self.music_changes.clear();
        self.presentation_options = None;
        self.presentation_cursor = 0;
        self.art.reset();
        self.art.interrupt_all(3);
    }
    pub fn reference_navigation(&self) -> (u8, usize, u16) {
        self.key_config
            .as_ref()
            .map_or((3, self.cursor, self.timer), |config| {
                config.reference_navigation()
            })
    }
    pub fn update(&mut self, input: &Input, options: &mut Options) -> OptionsAction {
        let action = self.update_inner(input, options);
        self.timer = self.timer.saturating_add(1);
        action
    }
    fn update_inner(&mut self, input: &Input, options: &mut Options) -> OptionsAction {
        if !self.midi_available {
            options.music_mode = options.music_mode.min(1);
        }
        self.input.tick(input);
        self.art.tick();
        if let Some(config) = &mut self.key_config {
            if config.update(input, &mut self.art, &self.input) {
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
        } else if self.input.pressed(input, Key::Down) {
            self.art.sound("select00");
            self.cursor = (self.cursor + 1) % 9;
        }
        // OnUpdateOptionsMenu colors the VMs before changing settings. The
        // next Draw observes those colors, even when this tick restores them.
        self.presentation_options = Some(options.clone());
        self.presentation_cursor = self.cursor;
        if self.timer < 32 {
            return OptionsAction::None;
        }
        let left = self.input.pressed(input, Key::Left);
        let right = self.input.pressed(input, Key::Right);
        let mut changed = false;
        // The original checks left, return-menu, right, then confirm. Both
        // horizontal directions can act in one update; music must retain
        // those requests even when the final setting equals its old value.
        for direction in 0..2 {
            if direction == 1 && (input.pressed(Key::Bomb) || input.pressed(Key::Pause)) {
                self.cursor = 8;
                self.art.sound("cancel00");
            }
            if !(if direction == 0 { left } else { right }) {
                continue;
            }
            let delta = if direction == 0 { -1 } else { 1 };
            let mut music_mode = None;
            match self.cursor {
                0 => options.life_count = (options.life_count as i32 + delta + 5) as u8 % 5,
                1 => options.bomb_count = (options.bomb_count as i32 + delta + 4) as u8 % 4,
                2 => options.color_16bit = !options.color_16bit,
                3 => {
                    let count = if self.midi_available { 3 } else { 2 };
                    options.music_mode =
                        (options.music_mode as i32 + delta + count) as u8 % count as u8;
                    music_mode = Some(options.music_mode);
                }
                4 => options.play_sounds = !options.play_sounds,
                5 => options.windowed = !options.windowed,
                _ => continue,
            }
            self.art.sound("select00");
            if let Some(mode) = music_mode {
                self.music_changes.push((mode, self.art.take_sounds()));
            }
            changed = true;
        }
        if input.pressed(Key::Shoot) || input.pressed(Key::Enter) {
            match self.cursor {
                6 => {
                    // MainMenu::OnUpdateOptionsMenu resets gameplay/audio
                    // values, but preserves colorMode16bit and the menu's
                    // pending windowed choice. The original also restarts
                    // title music even if WAV was already selected.
                    options.life_count = 2;
                    options.bomb_count = 3;
                    options.music_mode = 1;
                    options.play_sounds = true;
                    options.default_difficulty = 1;
                    self.music_changes.push((1, self.art.take_sounds()));
                    return OptionsAction::DefaultsRestored;
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
        if changed {
            OptionsAction::Changed
        } else {
            OptionsAction::None
        }
    }
    pub fn take_music_modes(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.music_changes)
            .into_iter()
            .map(|change| change.0)
            .collect()
    }
    pub fn take_audio_events(&mut self) -> Vec<OptionsAudio> {
        let mut events = Vec::new();
        for (mode, sounds) in std::mem::take(&mut self.music_changes) {
            events.extend(sounds.into_iter().map(OptionsAudio::Sound));
            events.push(OptionsAudio::Music(mode));
        }
        events.extend(self.art.take_sounds().into_iter().map(OptionsAudio::Sound));
        events
    }
    pub fn draw(&mut self, options: &Options) -> Vec<DrawCmd> {
        if let Some(config) = &self.key_config {
            return config.draw(&mut self.art);
        }
        let options = self.presentation_options.as_ref().unwrap_or(options);
        let cursor = self.presentation_cursor;
        // The original has six title01 label VMs and three title04 label VMs.
        let labels = [8, 9, 10, 11, 12, 72, 73, 74, 13];
        let mut selection = std::collections::HashMap::new();
        for (row, idx) in labels.iter().enumerate() {
            selection.insert(*idx, (row, row == cursor));
        }
        for (row, start, count, selected) in [
            (0, 14, 5, options.life_count as usize),
            (1, 19, 4, options.bomb_count as usize),
            (2, 23, 2, options.color_16bit as usize),
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
                if row == cursor {
                    offset[0] -= 4.0;
                    offset[1] -= 4.0;
                } else {
                    alpha = 128.0 / 255.0;
                }
            }
            if idx == 79 && !self.midi_available {
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
    fn sixteen_bit_setting_is_preserved_for_original_color_compositor() {
        let mut old = Options::default().encode();
        old[3] = 1;
        assert!(Options::parse(&old).unwrap().color_16bit);
    }
    #[test]
    fn midi_setting_is_preserved_for_available_synthesizer() {
        let mut old = Options::default().encode();
        old[4] = 2;
        assert_eq!(Options::parse(&old).unwrap().music_mode, 2);
    }
    #[test]
    fn legacy_options_select_original_software_blending_default() {
        let old = [1, 2, 3, 0, 1, 1, 0, 1];
        assert!(!Options::parse(&old).unwrap().hardware_texture_blending);
        let options = Options {
            hardware_texture_blending: true,
            ..Options::default()
        };
        assert_eq!(Options::parse(&options.encode()), Some(options));
        assert!(Options::parse(&[]).is_none());
    }
}
