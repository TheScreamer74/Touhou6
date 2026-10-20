//! MainMenu::OnUpdateOptionsMenu and ColorMenuItem, using the original sprites.
use crate::localization::Language;
use crate::menu_art::{MenuArt, MenuInput};
use th06_engine::{DrawCmd, Input, Key};
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u8)]
pub enum Hitbox {
    Off = 0,
    #[default]
    Focus = 1,
    Always = 2,
}
impl Hitbox {
    pub fn from_u8(value: u8) -> Option<Self> {
        [Self::Off, Self::Focus, Self::Always]
            .get(value as usize)
            .copied()
    }
    fn cycle(self, delta: i32) -> Self {
        Self::from_u8((self as i32 + delta).rem_euclid(3) as u8).unwrap()
    }
}
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
    pub language: Language,
    pub hitbox: Hitbox,
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
            language: Language::Original,
            hitbox: Hitbox::Focus,
        }
    }
}
impl Options {
    pub fn encode(&self) -> Vec<u8> {
        vec![
            3,
            self.life_count,
            self.bomb_count,
            self.color_16bit as u8,
            self.music_mode,
            self.play_sounds as u8,
            self.windowed as u8,
            self.default_difficulty,
            self.hardware_texture_blending as u8,
            self.language as u8,
            self.hitbox as u8,
        ]
    }
    pub fn parse(b: &[u8]) -> Option<Self> {
        let legacy = b.len() == 8 && b.first() == Some(&1);
        let previous = b.len() == 9 && b.first() == Some(&2) && b[8] <= 1;
        let current =
            b.len() == 11 && b.first() == Some(&3) && b[8] <= 1 && b[9] <= 4 && b[10] <= 2;
        if (!legacy && !previous && !current) || b[1] > 4 || b[2] > 3 || b[4] > 2 || b[7] > 3 {
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
            hardware_texture_blending: (previous || current) && b[8] != 0,
            language: if current {
                Language::from_u8(b[9])?
            } else {
                Language::Original
            },
            hitbox: if current {
                Hitbox::from_u8(b[10])?
            } else {
                Hitbox::Focus
            },
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
        // The caller transfers the live title VMs; original MainMenu sends
        // interrupt3 without recreating them.
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
            self.cursor = (self.cursor + 10) % 11;
        } else if self.input.pressed(input, Key::Down) {
            self.art.sound("select00");
            self.cursor = (self.cursor + 1) % 11;
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
                self.cursor = 10;
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
                8 => options.language = options.language.cycle(delta),
                9 => options.hitbox = options.hitbox.cycle(delta),
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
                8 | 9 => {
                    if self.cursor == 8 {
                        options.language = options.language.cycle(1);
                    } else {
                        options.hitbox = options.hitbox.cycle(1);
                    }
                    self.art.sound("select00");
                    return OptionsAction::Changed;
                }
                10 => {
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
        self.draw_port_options(options)
    }
    fn draw_port_options(&self, options: &Options) -> Vec<DrawCmd> {
        use crate::localization::ui;
        let mut cmds = self.art.draw(|_| None);
        self.art.text(
            &mut cmds,
            [80.0, 54.0],
            1.6,
            [1.0; 4],
            &ui("menu.options", "Options"),
            480.0,
        );
        let labels = [
            ui("options.lives", "Lives"),
            ui("options.bombs", "Bombs"),
            ui("options.color", "Color depth"),
            ui("options.music", "Music"),
            ui("options.sounds", "Sound effects"),
            ui("options.window", "Display"),
            ui("options.defaults", "Restore defaults"),
            ui("options.controller", "Controller"),
            ui("ui.language", "Language"),
            ui("ui.hitbox", "Hitbox"),
            ui("menu.back", "Back"),
        ];
        let values = [
            (options.life_count + 1).to_string(),
            options.bomb_count.to_string(),
            if options.color_16bit {
                "16 bit"
            } else {
                "32 bit"
            }
            .into(),
            [ui("options.off", "Off"), "WAV / Ogg".into(), "MIDI".into()]
                [options.music_mode as usize]
                .clone(),
            if options.play_sounds {
                ui("options.on", "On")
            } else {
                ui("options.off", "Off")
            },
            if options.windowed {
                ui("options.windowed", "Windowed")
            } else {
                ui("options.fullscreen", "Fullscreen")
            },
            String::new(),
            String::new(),
            options.language.label().into(),
            [
                ui("hitbox.off", "Off"),
                ui("hitbox.focus", "Focus only"),
                ui("hitbox.always", "Always"),
            ][options.hitbox as usize]
                .clone(),
            String::new(),
        ];
        for row in 0..11 {
            let tint = if row == self.presentation_cursor {
                [1.0, 0.80, 0.85, 1.0]
            } else {
                [0.85, 0.88, 1.0, 0.8]
            };
            let y = 110.0 + row as f32 * 25.0;
            if row == self.presentation_cursor {
                self.art.text(&mut cmds, [57.0, y], 1.0, tint, "›", 20.0);
            }
            self.art
                .text(&mut cmds, [80.0, y], 1.0, tint, &labels[row], 265.0);
            self.art
                .text(&mut cmds, [366.0, y], 1.0, tint, &values[row], 205.0);
        }
        self.art.text(
            &mut cmds,
            [80.0, 410.0],
            0.85,
            [0.75, 0.80, 0.9, 1.0],
            &ui(
                "options.help",
                "↑↓ Select   ←→ Change   Z / Enter Confirm   X Back",
            ),
            490.0,
        );
        cmds
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
    #[test]
    fn previous_options_migrate_without_losing_original_preferences() {
        let v2 = [2, 4, 0, 1, 2, 0, 1, 3, 1];
        let migrated = Options::parse(&v2).unwrap();
        assert_eq!(
            (
                migrated.life_count,
                migrated.bomb_count,
                migrated.music_mode
            ),
            (4, 0, 2)
        );
        assert!(migrated.hardware_texture_blending);
        assert_eq!(
            (migrated.language, migrated.hitbox),
            (Language::Original, Hitbox::Focus)
        );
        for language in Language::ALL {
            for hitbox in [Hitbox::Off, Hitbox::Focus, Hitbox::Always] {
                let current = Options {
                    language,
                    hitbox,
                    ..migrated.clone()
                };
                assert_eq!(Options::parse(&current.encode()), Some(current));
            }
        }
        let mut invalid = migrated.encode();
        invalid[9] = 5;
        assert!(Options::parse(&invalid).is_none());
        invalid[9] = 1;
        invalid[10] = 3;
        assert!(Options::parse(&invalid).is_none());
    }
}
