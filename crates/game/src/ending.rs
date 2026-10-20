//! END script player and staff roll (Ending.cpp, EoSD 1.02h).
//!
//! Commands, waits, two-byte text reads, script switching and five-step Ctrl
//! skipping follow Ending::ParseEndFile (0x40f7c0). Fades advance once per
//! rendered frame, as in Ending_OnDraw (0x410a70), rather than once per skipped
//! script step. Positions and artwork come from ED.DAT and IN.DAT/text.anm.

use crate::anm_vm::AnmRunner;
use crate::stage::{Character, Difficulty};
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::Arc;
use th06_engine::{DrawCmd, Engine, Input, Key, Texture, compose_rgba, text};
use th06_formats::anm0::{Anm0, Instr, Sprite};

const SLOT_COUNT: usize = 16;
const TEXT_BASE: u32 = 0x100; // Original 0x700, relative to staff base 0x600.
const ENDING_TEXT_SCRIPT: u32 = TEXT_BASE + 8;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EndingError(pub String);
impl fmt::Display for EndingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for EndingError {}
fn invalid(message: impl Into<String>) -> EndingError {
    EndingError(message.into())
}

/// Ending_AddedCallback: Easy or any continue selects the character's bad end.
pub fn ending_script(character: Character, difficulty: Difficulty, continues: u8) -> &'static str {
    if difficulty == Difficulty::Easy || continues != 0 {
        if character.is_marisa() {
            "end10b.end"
        } else {
            "end00b.end"
        }
    } else {
        match character {
            Character::ReimuA => "end00.end",
            Character::ReimuB => "end01.end",
            Character::MarisaA => "end10.end",
            Character::MarisaB => "end11.end",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum EndingEvent {
    /// The engine registers WAVs by basename; original scripts name MID files.
    Music(String),
    /// Supervisor::FadeOutMusic takes seconds (despite its local variable name).
    FadeMusic(f32),
}

#[derive(Clone, Copy, Default)]
struct Controls {
    confirm_pressed: bool,
    confirm_held: bool,
    skip: bool,
}
impl From<&Input> for Controls {
    fn from(input: &Input) -> Self {
        Self {
            confirm_pressed: input.pressed(Key::Shoot) || input.pressed(Key::Enter),
            confirm_held: input.held(Key::Shoot) || input.held(Key::Enter),
            skip: input.held(Key::Skip),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Action {
    Background(String),
    Animation {
        slot: usize,
        script: u32,
        sprite: u32,
    },
    Text {
        slot: usize,
        bytes: Vec<u8>,
        color: u32,
        source_key: String,
    },
    Clear,
    Event(EndingEvent),
    TickAnimations,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FadeKind {
    None,
    InBlack,
    OutBlack,
    InWhite,
    OutWhite,
}
struct Fade {
    kind: FadeKind,
    timer: i32,
    duration: i32,
}
impl Default for Fade {
    fn default() -> Self {
        Self {
            kind: FadeKind::None,
            timer: 0,
            duration: 0,
        }
    }
}
impl Fade {
    fn draw(&mut self) -> [f32; 4] {
        let white = matches!(self.kind, FadeKind::InWhite | FadeKind::OutWhite);
        let alpha = match self.kind {
            FadeKind::None => 0,
            FadeKind::InBlack | FadeKind::InWhite if self.timer >= self.duration => {
                self.kind = FadeKind::None;
                0
            }
            FadeKind::OutBlack | FadeKind::OutWhite if self.timer >= self.duration => 255,
            FadeKind::InBlack | FadeKind::InWhite => {
                let alpha = 255 - self.timer * 255 / self.duration;
                self.timer += 1;
                alpha
            }
            FadeKind::OutBlack | FadeKind::OutWhite => {
                let alpha = self.timer * 255 / self.duration;
                self.timer += 1;
                alpha
            }
        };
        let rgb = if white { 1.0 } else { 0.0 };
        [rgb, rgb, rgb, alpha as f32 / 255.0]
    }
}

struct EndVm {
    files: Arc<HashMap<String, Arc<[u8]>>>,
    file: String,
    data: Arc<[u8]>,
    pc: usize,
    has_seen: bool,
    finished: bool,
    script_time: u32,
    pause: i32,
    clear: i32,
    minimum_pause: i32,
    minimum_clear: i32,
    long_delay: i32,
    short_delay: i32,
    text_line: usize,
    text_color: u32,
    background: Option<String>,
    background_y: f32,
    scroll_speed: f32,
    fade: Fade,
}

fn basename(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or(path)
}
fn path_key(path: &str) -> String {
    basename(path).to_ascii_lowercase()
}

impl EndVm {
    fn new(
        files: Arc<HashMap<String, Arc<[u8]>>>,
        file: &str,
        has_seen: bool,
    ) -> Result<Self, EndingError> {
        let file = path_key(file);
        let data = files
            .get(&file)
            .ok_or_else(|| invalid(format!("ED.DAT: missing {file}")))?
            .clone();
        Ok(Self {
            files,
            file,
            data,
            pc: 0,
            has_seen,
            finished: false,
            script_time: 0,
            pause: 0,
            clear: 0,
            minimum_pause: 0,
            minimum_clear: 0,
            long_delay: 8,
            short_delay: 0,
            text_line: 0,
            text_color: 0,
            background: None,
            background_y: 0.0,
            scroll_speed: 0.0,
            fade: Fade::default(),
        })
    }

    fn error(&self, message: &str) -> EndingError {
        invalid(format!("{} at byte {}: {message}", self.file, self.pc))
    }

    fn byte(&self) -> Result<u8, EndingError> {
        self.data
            .get(self.pc)
            .copied()
            .ok_or_else(|| self.error("truncated END script (missing @z)"))
    }

    fn string(&mut self) -> Result<String, EndingError> {
        let start = self.pc;
        while self.byte()? != 0 {
            self.pc += 1;
        }
        let result = String::from_utf8_lossy(&self.data[start..self.pc]).into_owned();
        while self.data.get(self.pc) == Some(&0) {
            self.pc += 1;
        }
        Ok(result)
    }

    fn number(&mut self) -> Result<i32, EndingError> {
        // atoi accepts whitespace/sign and stops at the first non-digit.
        let text = self.string()?;
        let text = text.trim_start();
        let (sign, tail) = if let Some(tail) = text.strip_prefix('-') {
            (-1i64, tail)
        } else {
            (1, text.strip_prefix('+').unwrap_or(text))
        };
        let mut n = 0i64;
        for byte in tail.bytes().take_while(u8::is_ascii_digit) {
            n = n.saturating_mul(10).saturating_add((byte - b'0') as i64);
        }
        Ok((n * sign).clamp(i32::MIN as i64, i32::MAX as i64) as i32)
    }

    fn next_line(&mut self) -> Result<(), EndingError> {
        while !matches!(self.byte()?, b'\r' | b'\n') {
            self.pc += 1;
        }
        while matches!(self.data.get(self.pc), Some(b'\r' | b'\n')) {
            self.pc += 1;
        }
        Ok(())
    }

    fn switch(&mut self, filename: &str) -> Result<(), EndingError> {
        self.file = path_key(filename);
        self.data = self
            .files
            .get(&self.file)
            .ok_or_else(|| invalid(format!("ED.DAT: missing {}", self.file)))?
            .clone();
        self.pc = 0;
        self.long_delay = 8;
        self.pause = 0;
        self.script_time = 0;
        // Ending_AddedCallback has already set this run's clear record. The
        // @F scan of all normal-difficulty records therefore finds a clear.
        self.has_seen = true;
        Ok(())
    }

    fn text_action(
        &self,
        second: bool,
        bytes: Vec<u8>,
        actions: &mut Vec<Action>,
    ) -> Result<(), EndingError> {
        let slot = self.text_line * 2 + usize::from(second);
        if slot >= SLOT_COUNT {
            return Err(self.error("text exceeds the 16 original ANM slots"));
        }
        actions.push(Action::Text {
            slot,
            bytes,
            color: self.text_color,
            source_key: format!("end:{}:{}", self.file, self.pc),
        });
        Ok(())
    }

    fn frame_end(&mut self) {
        self.script_time = self.script_time.wrapping_add(1);
        self.background_y -= self.scroll_speed;
        if self.background_y <= 0.0 {
            self.background_y = 0.0;
            self.scroll_speed = 0.0;
        }
    }

    fn step(&mut self, input: Controls, actions: &mut Vec<Action>) -> Result<(), EndingError> {
        if self.finished {
            return Ok(());
        }
        if self.clear > 0 {
            self.clear -= 1;
            if self.minimum_clear != 0 {
                self.minimum_clear -= 1;
            } else if input.confirm_pressed || (self.has_seen && input.skip) {
                self.clear = 0;
            }
            if self.clear <= 0 {
                actions.push(Action::Clear);
                self.text_line = 0;
            } else {
                self.frame_end();
                return Ok(());
            }
        }
        if self.pause > 0 {
            self.pause -= 1;
            if self.minimum_pause != 0 {
                self.minimum_pause -= 1;
            } else if input.confirm_pressed || (self.has_seen && input.skip) {
                self.pause = 0;
            }
            self.frame_end();
            return Ok(());
        }
        let mut text_bytes = Vec::with_capacity(32);
        let mut second = false;
        // Original files contain finite command runs. A malformed loop through
        // @F should produce an error instead of blocking the rendering thread.
        for _ in 0..65_536 {
            match self.byte()? {
                b'@' => {
                    self.pc += 1;
                    let opcode = self.byte()?;
                    self.pc += 1;
                    let mut wait = false;
                    match opcode {
                        b'a' => {
                            let slot = self.number()?;
                            let script = self.number()?;
                            let sprite = self.number()?;
                            if !(0..SLOT_COUNT as i32).contains(&slot) || script < 0 || sprite < 0 {
                                return Err(self.error("invalid staff ANM slot/script/sprite"));
                            }
                            actions.push(Action::Animation {
                                slot: slot as usize,
                                script: script as u32,
                                sprite: sprite as u32,
                            });
                        }
                        b'b' => {
                            let name = path_key(&self.string()?);
                            self.background = Some(name.clone());
                            actions.push(Action::Background(name));
                        }
                        b'c' => self.text_color = self.number()? as u32,
                        b'm' => {
                            let path = self.string()?;
                            let name = basename(&path)
                                .rsplit_once('.')
                                .map_or(basename(&path), |(stem, _)| stem);
                            actions
                                .push(Action::Event(EndingEvent::Music(name.to_ascii_lowercase())));
                        }
                        b'M' => actions
                            .push(Action::Event(EndingEvent::FadeMusic(self.number()? as f32))),
                        b'r' => {
                            self.clear = self.number()?;
                            self.minimum_clear = self.number()?;
                            wait = true;
                        }
                        b'w' => {
                            self.pause = self.number()?;
                            self.minimum_pause = self.number()?;
                            wait = true;
                        }
                        b's' => {
                            self.long_delay = self.number()?;
                            self.short_delay = self.number()?;
                        }
                        b'v' => self.background_y = self.number()? as f32,
                        b'V' => {
                            let distance = self.number()?;
                            let duration = self.number()?;
                            if duration == 0 {
                                return Err(self.error("zero background scroll duration"));
                            }
                            self.scroll_speed = distance as f32 / duration as f32;
                        }
                        b'0' | b'1' | b'2' | b'3' => {
                            self.fade.kind = match opcode {
                                b'0' => FadeKind::InBlack,
                                b'1' => FadeKind::OutBlack,
                                b'2' => FadeKind::InWhite,
                                _ => FadeKind::OutWhite,
                            };
                            self.fade.timer = 0;
                            self.fade.duration = self.number()?;
                        }
                        b'F' => {
                            let filename = self.string()?;
                            self.switch(&filename)?;
                            text_bytes.clear();
                            second = false;
                            actions.push(Action::Clear);
                            // LoadEnding replaces the pointer; generic next_line
                            // below consequently skips the new file's first line.
                        }
                        b'R' => actions.push(Action::Clear),
                        b'z' => {
                            self.finished = true;
                            return Ok(());
                        }
                        _ => { /* Unknown commands are skipped by the original. */ }
                    }
                    self.next_line()?;
                    if wait {
                        self.frame_end();
                        return Ok(());
                    }
                }
                0 | b'\r' | b'\n' => {
                    if !text_bytes.is_empty() {
                        self.text_action(second, text_bytes, actions)?;
                    }
                    while matches!(self.data.get(self.pc), Some(0 | b'\r' | b'\n')) {
                        self.pc += 1;
                    }
                    let delay = if input.confirm_held {
                        self.short_delay
                    } else {
                        self.long_delay
                    };
                    self.pause = delay;
                    self.minimum_pause = delay;
                    self.text_line += 1;
                    self.frame_end();
                    return Ok(());
                }
                _ => {
                    let pair = self
                        .data
                        .get(self.pc..self.pc + 2)
                        .ok_or_else(|| self.error("truncated two-byte text read"))?;
                    text_bytes.extend_from_slice(pair);
                    self.pc += 2;
                    if text_bytes.len() >= 32 {
                        self.text_action(second, std::mem::take(&mut text_bytes), actions)?;
                        if second {
                            self.frame_end();
                            return Ok(());
                        }
                        second = true;
                    }
                }
            }
        }
        Err(self.error("END command loop exceeds the parser budget"))
    }

    fn update(&mut self, input: Controls) -> Result<Vec<Action>, EndingError> {
        let mut actions = Vec::new();
        for step in 0..5 {
            self.step(input, &mut actions)?;
            if self.finished {
                break;
            }
            actions.push(Action::TickAnimations);
            if !self.has_seen || !input.skip || step == 4 {
                break;
            }
        }
        Ok(actions)
    }
}

struct Image {
    tex: usize,
    width: f32,
    height: f32,
}
struct SpriteImage {
    sprite: Sprite,
    image: Image,
    localized_texture: Option<usize>,
}
#[derive(Hash, PartialEq, Eq)]
struct TextKey {
    source_key: String,
    bytes: Vec<u8>,
    color: u32,
    width: u32,
    height: u32,
}
struct Resources {
    files: Arc<HashMap<String, Arc<[u8]>>>,
    scripts: HashMap<u32, Vec<Instr>>,
    sprites: HashMap<u32, SpriteImage>,
    backgrounds: HashMap<String, Image>,
    text: HashMap<TextKey, usize>,
    white: usize,
}

/// Textures are uploaded once; each playthrough gets independent END/ANM state.
pub struct EndingAssets {
    resources: Arc<Resources>,
}

impl EndingAssets {
    pub fn load(
        engine: &Engine,
        ed: &HashMap<String, Vec<u8>>,
        text_anm: &[u8],
        textures: &mut Vec<Texture>,
    ) -> Result<Self, EndingError> {
        if ed.is_empty() {
            return Err(invalid(
                "ED.DAT is required to play endings and the staff roll",
            ));
        }
        let source: HashMap<String, &[u8]> = ed
            .iter()
            .map(|(name, data)| (path_key(name), data.as_slice()))
            .collect();
        let files: HashMap<String, Arc<[u8]>> = source
            .iter()
            .filter(|(name, _)| name.ends_with(".end"))
            .map(|(name, data)| (name.clone(), Arc::from(*data)))
            .collect();
        for name in [
            "end00.end",
            "end01.end",
            "end10.end",
            "end11.end",
            "end00b.end",
            "end10b.end",
        ] {
            if !files.contains_key(name) {
                return Err(invalid(format!("ED.DAT: missing {name}")));
            }
        }
        let mut resources = Resources {
            files: Arc::new(files),
            scripts: HashMap::new(),
            sprites: HashMap::new(),
            backgrounds: HashMap::new(),
            text: HashMap::new(),
            white: textures.len(),
        };
        textures.push(engine.create_texture(&[255; 4], 1, 1));
        for (name, base) in [("staff01.anm", 0), ("staff02.anm", 7), ("staff03.anm", 14)] {
            let bytes = source
                .get(name)
                .ok_or_else(|| invalid(format!("ED.DAT: missing {name}")))?;
            load_anm(engine, &source, bytes, base, textures, &mut resources)?;
        }
        load_anm(
            engine,
            &source,
            text_anm,
            TEXT_BASE,
            textures,
            &mut resources,
        )?;
        let (chunks, colors, backgrounds) = inventory(&resources.files)?;
        for filename in backgrounds {
            let bytes = source
                .get(&filename)
                .ok_or_else(|| invalid(format!("ED.DAT: missing background {filename}")))?;
            let (rgba, width, height) = compose_rgba(bytes, None);
            let tex = textures.len();
            textures.push(engine.create_texture_with_format(&rgba, width, height, 4));
            resources.backgrounds.insert(
                filename,
                Image {
                    tex,
                    width: width as f32,
                    height: height as f32,
                },
            );
        }
        let dimensions: HashSet<(u32, u32)> = resources
            .sprites
            .iter()
            .filter(|(id, _)| **id >= TEXT_BASE)
            .map(|(_, image)| (image.sprite.width as u32, image.sprite.height as u32))
            .collect();
        for (source_key, bytes) in chunks {
            for &color in &colors {
                for &(width, height) in &dimensions {
                    if width == 0 || height == 0 {
                        return Err(invalid("text.anm contains an empty text sprite"));
                    }
                    let (tex, _, _) = crate::localization::bake_custom_key(
                        engine,
                        textures,
                        &bytes,
                        &source_key,
                        color,
                        [width, height],
                        raster_localized_text,
                    );
                    resources.text.insert(
                        TextKey {
                            source_key: source_key.clone(),
                            bytes: bytes.clone(),
                            color,
                            width,
                            height,
                        },
                        tex,
                    );
                }
            }
        }
        Ok(Self {
            resources: Arc::new(resources),
        })
    }

    /// `has_seen` must be read before updating this run's clear record.
    pub fn start(
        &self,
        character: Character,
        difficulty: Difficulty,
        continues: u8,
        has_seen: bool,
    ) -> Result<EndingPlayer, EndingError> {
        let vm = EndVm::new(
            self.resources.files.clone(),
            ending_script(character, difficulty, continues),
            has_seen,
        )?;
        Ok(EndingPlayer {
            resources: self.resources.clone(),
            vm,
            slots: std::array::from_fn(|_| None),
            previous_confirm: 0,
        })
    }
}

fn load_anm(
    engine: &Engine,
    files: &HashMap<String, &[u8]>,
    data: &[u8],
    base: u32,
    textures: &mut Vec<Texture>,
    resources: &mut Resources,
) -> Result<(), EndingError> {
    let anm = Anm0::parse(data).map_err(|error| invalid(format!("ending ANM: {error:?}")))?;
    for entry in anm.entries {
        let (rgba, width, height) = if entry.name == "@" {
            (
                vec![0; (entry.width * entry.height * 4) as usize],
                entry.width,
                entry.height,
            )
        } else {
            let name = path_key(&entry.name);
            let data = files
                .get(&name)
                .ok_or_else(|| invalid(format!("ED.DAT: missing {name}")))?;
            let mask = entry
                .alpha_name
                .as_ref()
                .map(|name| {
                    files
                        .get(&path_key(name))
                        .copied()
                        .ok_or_else(|| invalid(format!("ED.DAT: missing mask {name}")))
                })
                .transpose()?;
            compose_rgba(data, mask)
        };
        let tex = textures.len();
        textures.push(engine.create_texture_with_format(&rgba, width, height, entry.format));
        for sprite in entry.sprites {
            let id = base + sprite.index;
            let localized_texture = if let Some(key) = credit_key(id) {
                let cropped = crop_staff_sprite(&rgba, width, height, &sprite)?;
                Some(
                    crate::localization::bake_custom_key(
                        engine,
                        textures,
                        &cropped,
                        key,
                        id,
                        [sprite.width as u32, sprite.height as u32],
                        raster_staff,
                    )
                    .0,
                )
            } else {
                None
            };
            resources.sprites.insert(
                id,
                SpriteImage {
                    sprite,
                    localized_texture,
                    image: Image {
                        tex,
                        width: width as f32,
                        height: height as f32,
                    },
                },
            );
        }
        for (id, mut script) in entry.scripts {
            for instruction in &mut script {
                if matches!(instruction.opcode, 1 | 16) && instruction.args.len() >= 4 {
                    let sprite = instruction.arg_u32(0).wrapping_add(base);
                    instruction.args[..4].copy_from_slice(&sprite.to_le_bytes());
                }
            }
            resources.scripts.insert(base + id, script);
        }
    }
    Ok(())
}

// These IDs are the original staff01/02/03.anm sprite indices, with the
// 0/7/14 offsets used by Ending_AddedCallback. Branding, version/date and
// proper-name-only sprites keep their supplied artwork in every language.
fn credit_key(sprite: u32) -> Option<&'static str> {
    Some(match sprite {
        4 => "credits.staff",
        5 => "credits.programmer",
        6 => "credits.program_support",
        7 => "credits.graphic_designer",
        8 => "credits.music_composer",
        9 => "credits.special_thanks",
        12 => "credits.test_players",
        13 => "credits.and_you",
        14 => "credits.extra_stage",
        16 => "credits.extra_challenge",
        _ => return None,
    })
}

fn crop_staff_sprite(
    rgba: &[u8],
    width: u32,
    height: u32,
    sprite: &Sprite,
) -> Result<Vec<u8>, EndingError> {
    let [x, y, w, h] = [sprite.x, sprite.y, sprite.width, sprite.height];
    if [x, y, w, h]
        .iter()
        .any(|value| !value.is_finite() || *value < 0.0 || value.fract() != 0.0)
        || w == 0.0
        || h == 0.0
        || x + w > width as f32
        || y + h > height as f32
        || rgba.len() != width as usize * height as usize * 4
    {
        return Err(invalid("staff ANM sprite is outside its texture"));
    }
    let (x, y, w, h) = (x as usize, y as usize, w as usize, h as usize);
    let mut cropped = Vec::with_capacity(w * h * 4);
    for row in y..y + h {
        let start = (row * width as usize + x) * 4;
        cropped.extend_from_slice(&rgba[start..start + w * 4]);
    }
    Ok(cropped)
}

fn raster_staff(
    original: &[u8],
    source_key: &str,
    sprite: u32,
    width: u32,
    height: u32,
    language: crate::localization::Language,
) -> Vec<u8> {
    if language == crate::localization::Language::Original {
        return original.to_vec();
    }
    let Some(translated) = crate::localization::lookup_in(language, source_key) else {
        return original.to_vec();
    };
    // staff01/02 have black ink; staff03 has green ink. Preserve the supplied
    // ink color instead of introducing a new credit-screen palette.
    let color = original
        .chunks_exact(4)
        .max_by_key(|pixel| pixel[3])
        .map(|pixel| [pixel[0], pixel[1], pixel[2]])
        .unwrap_or([0; 3]);
    let mut rgba = vec![0; (width * height * 4) as usize];
    // The final handwritten phrase in staff03 is not unambiguously readable.
    // Its original last 32 rows remain intact; only the confirmed two-line
    // challenge instruction above it is translated.
    let translated_height = if sprite == 16 {
        height.saturating_sub(32)
    } else {
        height
    };
    if sprite == 16 {
        let start = (width * translated_height * 4) as usize;
        if original.len() == rgba.len() {
            rgba[start..].copy_from_slice(&original[start..]);
        }
    }
    let lines: Vec<_> = translated.lines().collect();
    let band = translated_height / lines.len().max(1) as u32;
    for (index, line) in lines.iter().enumerate() {
        let image = text::render(line, 16.0, color, None);
        let image_height = image.height.min(band);
        let centered = crate::localization::canvas_image(
            image,
            [width, image_height],
            text::SpriteTextAlign::Center,
        );
        let top = index as u32 * band + band.saturating_sub(image_height) / 2;
        for row in 0..image_height {
            let src = (row * width * 4) as usize;
            let dst = ((top + row) * width * 4) as usize;
            rgba[dst..dst + (width * 4) as usize]
                .copy_from_slice(&centered.rgba[src..src + (width * 4) as usize]);
        }
    }
    rgba
}

type Inventory = (HashSet<(String, Vec<u8>)>, HashSet<u32>, HashSet<String>);
fn inventory(files: &HashMap<String, Arc<[u8]>>) -> Result<Inventory, EndingError> {
    let mut chunks = HashSet::new();
    let mut colors = HashSet::from([0]);
    let mut backgrounds = HashSet::new();
    for (filename, data) in files {
        let mut vm = EndVm::new(Arc::new(files.clone()), filename, false)?;
        while vm.pc < data.len() {
            match vm.byte()? {
                b'@' => {
                    vm.pc += 1;
                    let opcode = vm.byte()?;
                    vm.pc += 1;
                    match opcode {
                        b'c' => {
                            colors.insert(vm.number()? as u32);
                        }
                        b'b' => {
                            backgrounds.insert(path_key(&vm.string()?));
                        }
                        b'z' => break,
                        _ => {}
                    }
                    vm.next_line()?;
                }
                0 | b'\r' | b'\n' => {
                    vm.pc += 1;
                }
                _ => {
                    let mut bytes = Vec::new();
                    while vm.pc < data.len() && !matches!(vm.byte()?, 0 | b'\r' | b'\n') {
                        let pair = data
                            .get(vm.pc..vm.pc + 2)
                            .ok_or_else(|| vm.error("truncated text pair"))?;
                        bytes.extend_from_slice(pair);
                        vm.pc += 2;
                        if bytes.len() == 32 {
                            chunks.insert((
                                format!("end:{filename}:{}", vm.pc),
                                std::mem::take(&mut bytes),
                            ));
                        }
                    }
                    if !bytes.is_empty() {
                        chunks.insert((format!("end:{filename}:{}", vm.pc), bytes));
                    }
                }
            }
        }
    }
    Ok((chunks, colors, backgrounds))
}

fn raster_text(bytes: &[u8], color: u32, width: u32, height: u32) -> Vec<u8> {
    // COLORREF is BGR in the integer representation. TextHelper renders a
    // 30px font, then linearly scales a (2*w-2)x28 rectangle into w x 16.
    // Use the port's established Japanese font renderer for that same layout.
    let image = text::render_shiftjis(
        bytes,
        30.0,
        [color as u8, (color >> 8) as u8, (color >> 16) as u8],
        Some([208, 208, 192]),
    );
    let mut rgba = vec![0; (width * height * 4) as usize];
    for y in 0..height.min(16) {
        for x in 0..width {
            let sx = (x as f32 + 0.5) * (width * 2 - 2) as f32 / width as f32 - 0.5;
            let sy = (y as f32 + 0.5) * 28.0 / 16.0 - 0.5;
            let x0 = sx.floor() as i32;
            let y0 = sy.floor() as i32;
            let fx = sx - x0 as f32;
            let fy = sy - y0 as f32;
            for channel in 0..4 {
                let mut value = 0.0;
                for (dx, dy, weight) in [
                    (0, 0, (1.0 - fx) * (1.0 - fy)),
                    (1, 0, fx * (1.0 - fy)),
                    (0, 1, (1.0 - fx) * fy),
                    (1, 1, fx * fy),
                ] {
                    let (px, py) = (x0 + dx, y0 + dy);
                    if px >= 0 && py >= 0 && px < image.width as i32 && py < image.height as i32 {
                        value += image.rgba
                            [((py as u32 * image.width + px as u32) * 4) as usize + channel]
                            as f32
                            * weight;
                    }
                }
                rgba[((y * width + x) * 4) as usize + channel] = value.round() as u8;
            }
        }
    }
    rgba
}

fn raster_localized_text(
    bytes: &[u8],
    source_key: &str,
    color: u32,
    width: u32,
    height: u32,
    language: crate::localization::Language,
) -> Vec<u8> {
    if language == crate::localization::Language::Original {
        return raster_text(bytes, color, width, height);
    }
    let source = text::decode_shiftjis(bytes);
    let translated = crate::localization::lookup_in(language, source_key)
        .map(std::borrow::Cow::Borrowed)
        .unwrap_or_else(|| crate::localization::translate_in(language, &source));
    let image = text::render(
        &translated,
        15.0,
        [color as u8, (color >> 8) as u8, (color >> 16) as u8],
        Some([208, 208, 192]),
    );
    crate::localization::canvas_image(image, [width, height], text::SpriteTextAlign::Left).rgba
}

struct Slot {
    runner: AnmRunner,
    text_texture: Option<usize>,
}
pub struct EndingPlayer {
    resources: Arc<Resources>,
    vm: EndVm,
    slots: [Option<Slot>; SLOT_COUNT],
    previous_confirm: u16,
}
impl EndingPlayer {
    pub fn finished(&self) -> bool {
        self.vm.finished
    }

    /// Read-only observation matching refcap's original END interpreter fields.
    /// The file prefix identifies script switches independently of heap pointers.
    pub fn reference_state(&self, tick: u32) -> String {
        let vm = &self.vm;
        let fingerprint = vm
            .data
            .iter()
            .take(64)
            .fold(2_166_136_261u32, |hash, byte| {
                (hash ^ u32::from(*byte)).wrapping_mul(16_777_619)
            });
        let fade_kind = match vm.fade.kind {
            FadeKind::None => 0,
            FadeKind::InBlack => 1,
            FadeKind::OutBlack => 2,
            FadeKind::InWhite => 3,
            FadeKind::OutWhite => 4,
        };
        format!(
            "{tick},{fingerprint},{},{},{},{},{},{},{},{},{},{},{},{:.9},{:.9},{fade_kind},{},{}",
            vm.pc,
            vm.has_seen as u8,
            vm.script_time,
            vm.pause,
            vm.clear,
            vm.minimum_pause,
            vm.minimum_clear,
            vm.long_delay,
            vm.short_delay,
            vm.text_line,
            vm.text_color,
            vm.background_y,
            vm.scroll_speed,
            vm.fade.timer,
            vm.fade.duration,
        )
    }

    pub fn update(&mut self, input: &Input) -> Result<Vec<EndingEvent>, EndingError> {
        let mut events = Vec::new();
        let confirm = input.buttons() & 0x1001;
        let controls = Controls {
            // WAS_PRESSED(TH_BUTTON_SELECTMENU) compares the combined Z/Enter
            // mask. Releasing one while the other stays held also changes it.
            confirm_pressed: confirm != 0 && confirm != self.previous_confirm,
            ..input.into()
        };
        self.previous_confirm = confirm;
        for action in self.vm.update(controls)? {
            match action {
                Action::Background(name) => {
                    if !self.resources.backgrounds.contains_key(&name) {
                        return Err(invalid(format!("missing ending background {name}")));
                    }
                }
                Action::Animation {
                    slot,
                    script,
                    sprite,
                } => {
                    let instructions =
                        self.resources.scripts.get(&script).ok_or_else(|| {
                            invalid(format!("missing ending ANM script {script}"))
                        })?;
                    if !self.resources.sprites.contains_key(&sprite) {
                        return Err(invalid(format!("missing ending sprite {sprite}")));
                    }
                    let mut runner = AnmRunner::new(instructions.clone());
                    runner.sprite = Some(sprite);
                    self.slots[slot] = Some(Slot {
                        runner,
                        text_texture: None,
                    });
                }
                Action::Text {
                    slot,
                    bytes,
                    color,
                    source_key,
                } => {
                    let script = ENDING_TEXT_SCRIPT + slot as u32;
                    let instructions = self.resources.scripts.get(&script).ok_or_else(|| {
                        invalid(format!("text.anm: missing script {}", script - TEXT_BASE))
                    })?;
                    let runner = AnmRunner::new(instructions.clone());
                    let sprite = runner
                        .sprite
                        .and_then(|id| self.resources.sprites.get(&id))
                        .ok_or_else(|| invalid("ending text script has no sprite"))?;
                    let key = TextKey {
                        source_key,
                        bytes,
                        color,
                        width: sprite.sprite.width as u32,
                        height: sprite.sprite.height as u32,
                    };
                    let tex = self.resources.text.get(&key).copied().ok_or_else(|| {
                        invalid("ending text was not present in the asset inventory")
                    })?;
                    self.slots[slot] = Some(Slot {
                        runner,
                        text_texture: Some(tex),
                    });
                }
                Action::Clear => self.slots = std::array::from_fn(|_| None),
                Action::Event(event) => events.push(event),
                Action::TickAnimations => {
                    for slot in self.slots.iter_mut().flatten() {
                        slot.runner.tick();
                        // Staff and ending text scripts are deterministic. A
                        // random opcode must use the shared game RNG, never a
                        // private arbitrary seed for this presentation screen.
                        if slot.runner.take_pending_random().is_some() {
                            return Err(invalid("ending ANM requires a shared RNG draw"));
                        }
                    }
                }
            }
        }
        Ok(events)
    }

    pub fn draw(&mut self) -> Vec<DrawCmd> {
        let mut cmds = Vec::new();
        if let Some(background) = self
            .vm
            .background
            .as_ref()
            .and_then(|name| self.resources.backgrounds.get(name))
        {
            let top = self.vm.background_y as i32 as f32;
            cmds.push(DrawCmd {
                projected: None,
                clip: None,
                tex: background.tex,
                dst: [0.0, 0.0, 640.0, 480.0],
                src: [
                    0.0,
                    top / background.height,
                    640.0 / background.width,
                    (top + 480.0) / background.height,
                ],
                tint: [1.0; 4],
                rot: 0.0,
                additive: false,
                color_add: false,
                depth: None,
            });
        }
        for slot in self.slots.iter().flatten() {
            let runner = &slot.runner;
            if !runner.visible() {
                continue;
            }
            let Some(sprite) = runner.sprite.and_then(|id| self.resources.sprites.get(&id)) else {
                continue;
            };
            let sp = &sprite.sprite;
            let (width, height) = (sp.width * runner.scale[0], sp.height * runner.scale[1]);
            let geometry = crate::sprite_geometry::screen(
                [
                    runner.pos[0] + runner.pos_offset[0],
                    runner.pos[1] + runner.pos_offset[1],
                ],
                [width, height],
                runner.corner,
            );
            let texture = slot.text_texture.or(sprite.localized_texture);
            let mut src = if texture.is_some() {
                [0.0, 0.0, 1.0, 1.0]
            } else {
                [
                    sp.x / sprite.image.width,
                    sp.y / sprite.image.height,
                    (sp.x + sp.width) / sprite.image.width,
                    (sp.y + sp.height) / sprite.image.height,
                ]
            };
            if runner.flip_x {
                src.swap(0, 2);
            }
            if runner.flip_y {
                src.swap(1, 3);
            }
            cmds.push(DrawCmd {
                projected: None,
                clip: None,
                tex: texture.unwrap_or(sprite.image.tex),
                dst: geometry.dst,
                src,
                tint: [
                    runner.color[0],
                    runner.color[1],
                    runner.color[2],
                    runner.alpha,
                ],
                rot: 0.0,
                additive: runner.additive,
                color_add: false,
                depth: None,
            });
        }
        let tint = self.vm.fade.draw();
        if tint[3] != 0.0 {
            cmds.push(DrawCmd {
                projected: None,
                clip: None,
                tex: self.resources.white,
                dst: [0.0, 0.0, 640.0, 480.0],
                src: [0.0, 0.0, 1.0, 1.0],
                tint,
                rot: 0.0,
                additive: false,
                color_add: false,
                depth: None,
            });
        }
        cmds
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vm(data: &[u8], seen: bool) -> EndVm {
        EndVm::new(
            Arc::new(HashMap::from([("test.end".into(), Arc::from(data))])),
            "test.end",
            seen,
        )
        .unwrap()
    }

    #[test]
    fn original_ending_selection() {
        for character in [
            Character::ReimuA,
            Character::ReimuB,
            Character::MarisaA,
            Character::MarisaB,
        ] {
            let bad = if character.is_marisa() {
                "end10b.end"
            } else {
                "end00b.end"
            };
            assert_eq!(ending_script(character, Difficulty::Easy, 0), bad);
            assert_eq!(ending_script(character, Difficulty::Lunatic, 1), bad);
        }
        assert_eq!(
            ending_script(Character::ReimuA, Difficulty::Normal, 0),
            "end00.end"
        );
        assert_eq!(
            ending_script(Character::ReimuB, Difficulty::Hard, 0),
            "end01.end"
        );
        assert_eq!(
            ending_script(Character::MarisaA, Difficulty::Normal, 0),
            "end10.end"
        );
        assert_eq!(
            ending_script(Character::MarisaB, Difficulty::Lunatic, 0),
            "end11.end"
        );
    }

    #[test]
    fn clear_wait_respects_minimum_and_continues_in_the_expiring_frame() {
        let mut vm = vm(b"@r10\0\x32\0\r\n@mth06_17.mid\0\r\n@z\r\n", false);
        vm.update(Controls::default()).unwrap();
        let pressed = Controls {
            confirm_pressed: true,
            ..Controls::default()
        };
        assert_eq!(vm.update(pressed).unwrap(), vec![Action::TickAnimations]);
        assert_eq!(vm.update(pressed).unwrap(), vec![Action::TickAnimations]);
        let actions = vm.update(pressed).unwrap();
        assert_eq!(
            actions,
            vec![
                Action::Clear,
                Action::Event(EndingEvent::Music("th06_17".into()))
            ]
        );
        assert!(vm.finished);
    }

    #[test]
    fn pause_expiration_always_yields_one_frame() {
        let mut vm = vm(b"@w1\0\x30\0\r\n@z\r\n", false);
        vm.update(Controls::default()).unwrap();
        vm.update(Controls::default()).unwrap();
        assert!(!vm.finished);
        vm.update(Controls::default()).unwrap();
        assert!(vm.finished);
    }

    #[test]
    fn two_byte_reads_split_into_two_32_byte_slots() {
        let mut script = b"@s0\0\x30\0\r\n".to_vec();
        script.extend([0x82, 0xa0].repeat(32));
        script.extend(b"\r\n@z\r\n");
        let mut vm = vm(&script, false);
        let actions = vm.update(Controls::default()).unwrap();
        assert_eq!(
            actions
                .iter()
                .filter(|action| matches!(action, Action::Text { .. }))
                .count(),
            2
        );
        assert_eq!(
            actions[0],
            Action::Text {
                slot: 0,
                bytes: [0x82, 0xa0].repeat(16),
                color: 0,
                source_key: "end:test.end:40".into(),
            }
        );
        assert_eq!(
            actions[1],
            Action::Text {
                slot: 1,
                bytes: [0x82, 0xa0].repeat(16),
                color: 0,
                source_key: "end:test.end:72".into(),
            }
        );
        let (inventoried, _, _) = inventory(&vm.files).unwrap();
        let emitted = actions
            .iter()
            .filter_map(|action| match action {
                Action::Text {
                    bytes, source_key, ..
                } => Some((source_key.clone(), bytes.clone())),
                _ => None,
            })
            .collect::<HashSet<_>>();
        assert_eq!(
            inventoried, emitted,
            "END inventory and playback must identify the same chunks"
        );
        assert_eq!(vm.text_line, 0); // 64-byte flush yields before the newline.
        vm.update(Controls::default()).unwrap();
        assert_eq!(vm.text_line, 1);
        vm.update(Controls::default()).unwrap();
        assert!(vm.finished);
    }

    #[test]
    fn staff_original_pixels_and_unreadable_final_phrase_are_preserved() {
        use crate::localization::Language;
        let original = (0..256 * 80 * 4)
            .map(|index| (index % 251) as u8)
            .collect::<Vec<_>>();
        assert_eq!(
            raster_staff(
                &original,
                "credits.extra_challenge",
                16,
                256,
                80,
                Language::Original
            ),
            original
        );
        for language in &Language::ALL[1..] {
            let rendered =
                raster_staff(&original, "credits.extra_challenge", 16, 256, 80, *language);
            assert_eq!(rendered.len(), original.len());
            assert_eq!(&rendered[256 * 48 * 4..], &original[256 * 48 * 4..]);
            assert_ne!(&rendered[..256 * 48 * 4], &original[..256 * 48 * 4]);
        }
    }

    #[test]
    fn staff_translations_fit_the_original_sprite_rectangles_at_16_pixels() {
        use crate::localization::{Language, lookup_in};
        for sprite in 0..=16 {
            let Some(key) = credit_key(sprite) else {
                continue;
            };
            for language in &Language::ALL[1..] {
                let translated = lookup_in(*language, key).expect("credit translation");
                let lines = translated.lines().collect::<Vec<_>>();
                assert_eq!(lines.len(), if sprite == 16 { 2 } else { 1 });
                for line in lines {
                    let image = text::render(line, 16.0, [0; 3], None);
                    assert!(image.width <= 256, "{} {key}: {line}", language.code());
                    assert!(image.height <= if sprite == 16 { 24 } else { 32 });
                }
            }
        }
        for branded_or_name in [0, 1, 2, 3, 10, 11, 15] {
            assert!(credit_key(branded_or_name).is_none());
        }
    }

    #[test]
    fn staff_crop_preserves_the_original_texels_and_checks_bounds() {
        let rgba = (0..4 * 4 * 4).map(|index| index as u8).collect::<Vec<_>>();
        let mut sprite = Sprite {
            index: 0,
            x: 1.0,
            y: 1.0,
            width: 2.0,
            height: 2.0,
        };
        let cropped = crop_staff_sprite(&rgba, 4, 4, &sprite).unwrap();
        assert_eq!(cropped, [&rgba[20..28], &rgba[36..44]].concat());
        sprite.x = 3.0;
        assert!(crop_staff_sprite(&rgba, 4, 4, &sprite).is_err());
    }

    #[test]
    fn ctrl_runs_five_steps_only_after_an_ending_was_seen() {
        for (seen, steps) in [(false, 1), (true, 5)] {
            let mut vm = vm(b"@w100\0\x31\x30\x30\0\r\n@z\r\n", seen);
            let actions = vm
                .update(Controls {
                    skip: true,
                    ..Controls::default()
                })
                .unwrap();
            assert_eq!(vm.script_time, steps);
            assert_eq!(
                actions
                    .iter()
                    .filter(|action| **action == Action::TickAnimations)
                    .count(),
                steps as usize
            );
        }
    }

    #[test]
    fn switch_skips_first_line_and_preserves_scroll_state() {
        let files = Arc::new(HashMap::from([
            (
                "test.end".into(),
                Arc::from(&b"@v10\0\r\n@V8\0\x34\0\r\n@Fstaff.end\0\r\n"[..]),
            ),
            (
                "staff.end".into(),
                Arc::from(&b"@mignored.mid\0\r\n@mactual.mid\0\r\n@w1\0\x30\0\r\n@z\r\n"[..]),
            ),
        ]));
        let mut vm = EndVm::new(files, "test.end", false).unwrap();
        let actions = vm.update(Controls::default()).unwrap();
        assert!(vm.has_seen);
        assert_eq!(vm.background_y, 8.0);
        assert_eq!(vm.script_time, 1);
        assert_eq!(
            actions,
            vec![
                Action::Clear,
                Action::Event(EndingEvent::Music("actual".into())),
                Action::TickAnimations
            ]
        );
    }

    #[test]
    fn scroll_clamps_at_zero_and_stops() {
        let mut vm = vm(b"@v3\0\r\n@V8\0\x34\0\r\n@w10\0\x31\x30\0\r\n@z\r\n", false);
        vm.update(Controls::default()).unwrap();
        assert_eq!(vm.background_y, 1.0);
        vm.update(Controls::default()).unwrap();
        assert_eq!(vm.background_y, 0.0);
        assert_eq!(vm.scroll_speed, 0.0);
    }

    #[test]
    fn fades_advance_per_draw_including_zero_duration() {
        let mut vm = vm(b"@22\0\r\n@w100\0\x31\x30\x30\0\r\n@z\r\n", true);
        vm.update(Controls {
            skip: true,
            ..Controls::default()
        })
        .unwrap();
        assert_eq!(vm.fade.timer, 0);
        assert_eq!(vm.fade.draw(), [1.0, 1.0, 1.0, 1.0]);
        assert_eq!(vm.fade.draw()[3], 128.0 / 255.0);
        assert_eq!(vm.fade.draw()[3], 0.0);
        assert_eq!(vm.fade.kind, FadeKind::None);
        let mut zero = Fade {
            kind: FadeKind::OutBlack,
            timer: 0,
            duration: 0,
        };
        assert_eq!(zero.draw(), [0.0, 0.0, 0.0, 1.0]);
    }

    #[test]
    fn invalid_script_is_an_error_instead_of_a_synthetic_ending() {
        let mut vm = vm(b"@r100", false);
        assert!(
            vm.update(Controls::default())
                .unwrap_err()
                .0
                .contains("truncated")
        );
    }

    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    #[ignore = "requires original ED.DAT and IN.DAT extracted in TH06_REFERENCE_DIR"]
    fn original_assets_all_six_endings_reach_staff_roll_and_finish() {
        let base =
            std::env::var("TH06_REFERENCE_DIR").unwrap_or_else(|_| "/tmp/th06-reference".into());
        let ed = std::fs::read_dir(format!("{base}/ed"))
            .unwrap()
            .map(|entry| {
                let entry = entry.unwrap();
                (
                    entry.file_name().to_string_lossy().into_owned(),
                    std::fs::read(entry.path()).unwrap(),
                )
            })
            .collect::<HashMap<_, _>>();
        let text = std::fs::read(format!("{base}/inn/text.anm")).unwrap();
        let engine = Engine::new();
        let mut textures = Vec::new();
        let assets = EndingAssets::load(&engine, &ed, &text, &mut textures).unwrap();
        if let Ok(directory) = std::env::var("TH06_ENDING_SCREENSHOTS") {
            std::fs::create_dir_all(&directory).unwrap();
            let references = textures.iter().collect::<Vec<_>>();
            let mut player = assets
                .start(Character::ReimuA, Difficulty::Normal, 0, false)
                .unwrap();
            for _ in 0..500 {
                player.update(&Input::default()).unwrap();
                player.draw();
            }
            let image = engine.render_to_image(&player.draw(), &references, None);
            image::save_buffer(
                format!("{directory}/ending.png"),
                &image,
                640,
                480,
                image::ColorType::Rgba8,
            )
            .unwrap();
            player.vm = EndVm::new(assets.resources.files.clone(), "staff00.end", false).unwrap();
            player.slots = std::array::from_fn(|_| None);
            for _ in 0..1_200 {
                player.update(&Input::default()).unwrap();
                player.draw();
            }
            let image = engine.render_to_image(&player.draw(), &references, None);
            image::save_buffer(
                format!("{directory}/staff.png"),
                &image,
                640,
                480,
                image::ColorType::Rgba8,
            )
            .unwrap();
        }
        let input = Input::synthetic(&[Key::Shoot, Key::Skip], &[]);
        for (character, difficulty) in [
            (Character::ReimuA, Difficulty::Easy),
            (Character::MarisaA, Difficulty::Easy),
            (Character::ReimuA, Difficulty::Normal),
            (Character::ReimuB, Difficulty::Normal),
            (Character::MarisaA, Difficulty::Normal),
            (Character::MarisaB, Difficulty::Normal),
        ] {
            let mut player = assets.start(character, difficulty, 0, true).unwrap();
            let mut music = Vec::new();
            let mut staff_animations = HashSet::new();
            for _ in 0..10_000 {
                for event in player.update(&input).unwrap() {
                    if let EndingEvent::Music(name) = event {
                        music.push(name);
                    }
                }
                if player.vm.file == "staff00.end" {
                    for slot in player.slots.iter().flatten() {
                        if let Some(sprite) = slot.runner.sprite {
                            staff_animations.insert(sprite);
                        }
                    }
                }
                let cmds = player.draw();
                assert!(cmds.iter().all(|cmd| cmd.tex < textures.len()));
                if player.finished() {
                    break;
                }
            }
            assert!(
                player.finished(),
                "{} did not terminate",
                ending_script(character, difficulty, 0)
            );
            assert_eq!(music, ["th06_16", "th06_17"]);
            assert!(
                staff_animations.contains(&16),
                "last staff sprite was never displayed"
            );
            assert_eq!(player.vm.background_y, 0.0);
        }
    }
}
