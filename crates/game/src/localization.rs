//! Optional port translations. Original script data, waits and score rules stay intact.
//! Text recipes reuse texture slots when the player changes language.
use std::borrow::Cow;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::sync::OnceLock;
use th06_engine::{
    DrawCmd, Engine, Texture,
    text::{self, SpriteTextAlign, TextImage},
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u8)]
pub enum Language {
    #[default]
    Original = 0,
    French = 1,
    English = 2,
    German = 3,
    Dutch = 4,
}
impl Language {
    pub const ALL: [Self; 5] = [
        Self::Original,
        Self::French,
        Self::English,
        Self::German,
        Self::Dutch,
    ];
    pub fn from_u8(value: u8) -> Option<Self> {
        Self::ALL.get(value as usize).copied()
    }
    pub fn code(self) -> &'static str {
        ["ja", "fr", "en", "de", "nl"][self as usize]
    }
    pub fn label(self) -> &'static str {
        [
            "日本語 / Original",
            "Français",
            "English",
            "Deutsch",
            "Nederlands",
        ][self as usize]
    }
    pub fn cycle(self, delta: i32) -> Self {
        Self::ALL[(self as i32 + delta).rem_euclid(5) as usize]
    }
}
thread_local! { static LANGUAGE: Cell<Language> = const { Cell::new(Language::Original) }; }
pub fn language() -> Language {
    LANGUAGE.with(Cell::get)
}
pub fn set_language(value: Language) {
    LANGUAGE.with(|language| language.set(value));
}
type Catalog = HashMap<String, String>;
static CATALOGS: OnceLock<[Catalog; 4]> = OnceLock::new();
fn catalogs() -> &'static [Catalog; 4] {
    CATALOGS.get_or_init(|| {
        [
            include_str!("../assets/locales/fr.json"),
            include_str!("../assets/locales/en.json"),
            include_str!("../assets/locales/de.json"),
            include_str!("../assets/locales/nl.json"),
        ]
        .map(|source| serde_json::from_str(source).expect("valid bundled translation catalog"))
    })
}
pub fn lookup_in(value: Language, source: &str) -> Option<&'static str> {
    if value == Language::Original {
        return None;
    }
    catalogs()[value as usize - 1]
        .get(source)
        .map(String::as_str)
}
pub fn translate_in(value: Language, source: &str) -> Cow<'_, str> {
    lookup_in(value, source).map_or(Cow::Borrowed(source), Cow::Borrowed)
}
pub fn translate(source: &str) -> Cow<'_, str> {
    translate_in(language(), source)
}
/// A UI key has a readable English fallback, including in Original mode.
pub fn ui(key: &str, fallback: &str) -> String {
    lookup_in(language(), key).unwrap_or(fallback).to_owned()
}

type CustomRenderer = fn(&[u8], &str, u32, u32, u32, Language) -> Vec<u8>;

#[derive(Clone)]
enum RecipeKind {
    Text {
        px: f32,
        color: [u8; 3],
        shadow: Option<[u8; 3]>,
    },
    Sprite {
        font_size: u32,
        alignment: SpriteTextAlign,
        color: [u8; 3],
    },
    Custom {
        color: u32,
        render: CustomRenderer,
    },
}
#[derive(Clone)]
struct Recipe {
    source: String,
    original: Option<Vec<u8>>,
    canvas: [u32; 2],
    slot: usize,
    kind: RecipeKind,
    format: u32,
}
struct Registry {
    recipes: Vec<Recipe>,
    applied: Language,
}
thread_local! {
    static REGISTRY: RefCell<Registry> = const { RefCell::new(Registry { recipes: Vec::new(), applied: Language::Original }) };
}
/// Called once before assembling a new Game's texture vector.
pub fn begin_textures() {
    REGISTRY.with(|registry| {
        *registry.borrow_mut() = Registry {
            recipes: Vec::new(),
            applied: language(),
        }
    });
    th06_engine::clear_texture_replacements();
}

pub(crate) fn canvas_image(
    image: TextImage,
    canvas: [u32; 2],
    alignment: SpriteTextAlign,
) -> TextImage {
    let [width, height] = canvas.map(|n| n.max(1));
    let mut rgba = vec![0; (width * height * 4) as usize];
    // Long translations shrink horizontally inside the same sprite rectangle.
    // A render taller than the original canvas (Noto vs GDI line height) is
    // shrunk vertically too, so descenders and the shadow are not cut off.
    let target_width = image.width.min(width);
    let target_height = image.height.min(height);
    let offset = match alignment {
        SpriteTextAlign::Left => 0,
        SpriteTextAlign::Center => (width - target_width) / 2,
        SpriteTextAlign::Right => width - target_width,
    };
    for y in 0..target_height {
        let source_y = if image.height > height {
            // Endpoint-inclusive: the first and last source rows both survive.
            let (from, to) = (image.height as u64 - 1, (target_height as u64 - 1).max(1));
            ((y as u64 * from + to / 2) / to) as u32
        } else {
            y
        };
        for x in 0..target_width {
            let source_x = if image.width > width {
                ((x as u64 * image.width as u64) / target_width as u64) as u32
            } else {
                x
            };
            let source = ((source_y * image.width + source_x) * 4) as usize;
            let dest = ((y * width + x + offset) * 4) as usize;
            rgba[dest..dest + 4].copy_from_slice(&image.rgba[source..source + 4]);
        }
    }
    TextImage {
        width,
        height,
        rgba,
    }
}
fn render_recipe(recipe: &Recipe, value: Language) -> TextImage {
    match recipe.kind {
        RecipeKind::Text { px, color, shadow } => canvas_image(
            text::render(&translate_in(value, &recipe.source), px, color, shadow),
            recipe.canvas,
            SpriteTextAlign::Left,
        ),
        RecipeKind::Sprite {
            font_size,
            alignment,
            color,
        } => {
            if value == Language::Original
                && let Some(bytes) = &recipe.original
            {
                return text::render_sprite_shiftjis_aligned(
                    bytes,
                    recipe.canvas[0],
                    recipe.canvas[1],
                    font_size,
                    alignment,
                    color,
                );
            }
            let image = text::render(
                &translate_in(value, &recipe.source),
                font_size as f32,
                color,
                Some([0; 3]),
            );
            canvas_image(image, recipe.canvas, alignment)
        }
        RecipeKind::Custom { color, render } => TextImage {
            width: recipe.canvas[0],
            height: recipe.canvas[1],
            rgba: render(
                recipe.original.as_deref().unwrap_or_default(),
                &recipe.source,
                color,
                recipe.canvas[0],
                recipe.canvas[1],
                value,
            ),
        },
    }
}
/// Layout of one registered text in one language: the natural rendered width
/// and height versus the original sprite canvas. Translations shrink
/// horizontally to the canvas (`canvas_image`) and are clipped vertically.
#[derive(Debug, Clone)]
pub struct TextFit {
    pub language: &'static str,
    pub source: String,
    pub text: String,
    pub canvas: [u32; 2],
    pub natural: [u32; 2],
    pub horizontal_scale: f32,
    pub vertically_clipped: bool,
    /// Non-transparent pixels below the canvas height that are cut off.
    pub clipped_ink_pixels: u32,
    /// Final sprite exactly as `render_recipe` bakes it, for squeezed or
    /// taller-than-canvas texts (width, height, RGBA).
    pub baked: Option<(u32, u32, Vec<u8>)>,
}

/// Measures every registered text recipe in each translated language without
/// touching the GPU textures. Custom (ending) recipes use the 15 px chunk metrics.
pub fn audit_text_fit() -> Vec<TextFit> {
    REGISTRY.with(|registry| {
        let registry = registry.borrow();
        let mut rows = Vec::new();
        for recipe in &registry.recipes {
            let (px, shadow) = match recipe.kind {
                RecipeKind::Text { px, shadow, .. } => (px, shadow),
                RecipeKind::Sprite { font_size, .. } => (font_size as f32, Some([0; 3])),
                // Ending chunks (`ending::chunk_text`) render at 15 px with a
                // shadow; staff credit lines are approximated the same way.
                RecipeKind::Custom { .. } => (15.0, Some([0; 3])),
            };
            for value in [
                Language::French,
                Language::English,
                Language::German,
                Language::Dutch,
            ] {
                let translated = translate_in(value, &recipe.source).into_owned();
                let image = text::render(&translated, px, [255; 3], shadow);
                let [width, height] = recipe.canvas.map(|n| n.max(1));
                rows.push(TextFit {
                    language: value.code(),
                    source: recipe.source.clone(),
                    text: translated,
                    canvas: recipe.canvas,
                    natural: [image.width, image.height],
                    horizontal_scale: (width as f32 / image.width.max(1) as f32).min(1.0),
                    vertically_clipped: image.height > height,
                    clipped_ink_pixels: image
                        .rgba
                        .chunks_exact(4)
                        .skip((height.min(image.height) * image.width) as usize)
                        .filter(|pixel| pixel[3] != 0)
                        .count() as u32,
                    baked: (image.width > width || image.height > height).then(|| {
                        let baked = render_recipe(recipe, value);
                        (baked.width, baked.height, baked.rgba)
                    }),
                });
            }
        }
        rows
    })
}

fn register(engine: &Engine, textures: &mut Vec<Texture>, mut recipe: Recipe) -> (usize, f32, f32) {
    recipe.slot = textures.len();
    let image = render_recipe(&recipe, language());
    textures.push(engine.create_texture_with_format(
        &image.rgba,
        image.width,
        image.height,
        recipe.format,
    ));
    let result = (recipe.slot, image.width as f32, image.height as f32);
    REGISTRY.with(|registry| registry.borrow_mut().recipes.push(recipe));
    result
}
pub fn bake_text(
    engine: &Engine,
    textures: &mut Vec<Texture>,
    source: &str,
    px: f32,
    color: [u8; 3],
    shadow: Option<[u8; 3]>,
    canvas: [u32; 2],
) -> (usize, f32, f32) {
    register(
        engine,
        textures,
        Recipe {
            source: source.to_owned(),
            original: None,
            canvas,
            slot: 0,
            kind: RecipeKind::Text { px, color, shadow },
            format: 1,
        },
    )
}
pub fn bake_shiftjis(
    engine: &Engine,
    textures: &mut Vec<Texture>,
    bytes: &[u8],
    px: f32,
    color: [u8; 3],
    shadow: Option<[u8; 3]>,
    canvas: [u32; 2],
) -> (usize, f32, f32) {
    bake_text(
        engine,
        textures,
        &text::decode_shiftjis(bytes),
        px,
        color,
        shadow,
        canvas,
    )
}
pub fn bake_custom(
    engine: &Engine,
    textures: &mut Vec<Texture>,
    bytes: &[u8],
    color: u32,
    canvas: [u32; 2],
    render: CustomRenderer,
) -> (usize, f32, f32) {
    bake_custom_key(
        engine,
        textures,
        bytes,
        &text::decode_shiftjis(bytes),
        color,
        canvas,
        render,
    )
}
#[allow(clippy::too_many_arguments)]
pub fn bake_custom_key(
    engine: &Engine,
    textures: &mut Vec<Texture>,
    bytes: &[u8],
    source_key: &str,
    color: u32,
    canvas: [u32; 2],
    render: CustomRenderer,
) -> (usize, f32, f32) {
    register(
        engine,
        textures,
        Recipe {
            source: source_key.to_owned(),
            original: Some(bytes.to_vec()),
            canvas,
            slot: 0,
            kind: RecipeKind::Custom { color, render },
            format: 1,
        },
    )
}
#[allow(clippy::too_many_arguments)]
pub fn bake_sprite_shiftjis_aligned(
    engine: &Engine,
    textures: &mut Vec<Texture>,
    bytes: &[u8],
    width: u32,
    height: u32,
    font_size: u32,
    alignment: SpriteTextAlign,
    color: [u8; 3],
    format: u32,
) -> (usize, f32, f32) {
    let bytes = bytes.split(|byte| *byte == 0).next().unwrap_or_default();
    register(
        engine,
        textures,
        Recipe {
            source: text::decode_shiftjis(bytes),
            original: Some(bytes.to_vec()),
            canvas: [width, height],
            slot: 0,
            kind: RecipeKind::Sprite {
                font_size,
                alignment,
                color,
            },
            format,
        },
    )
}
/// Refreshes only translated textures, keeping every texture index and size stable.
pub fn refresh() -> bool {
    let value = language();
    REGISTRY.with(|registry| {
        let mut registry = registry.borrow_mut();
        if registry.applied == value {
            return false;
        }
        for recipe in &registry.recipes {
            let image = render_recipe(recipe, value);
            th06_engine::queue_texture_replacement(
                recipe.slot,
                image.rgba,
                image.width,
                image.height,
                recipe.format,
            );
        }
        registry.applied = value;
        true
    })
}

#[derive(Clone, Default)]
pub struct LatinFont {
    tex: usize,
    glyphs: HashMap<char, (u32, u32, f32)>,
    width: u32,
    height: u32,
}
thread_local! { static LATIN_FONT: RefCell<Option<LatinFont>> = const { RefCell::new(None) }; }
impl LatinFont {
    pub fn load(engine: &Engine, textures: &mut Vec<Texture>) -> Self {
        let characters: Vec<char> = (32..=255)
            .filter_map(char::from_u32)
            .chain("œŒ–—…’‘“”←→↑↓›日本語".chars())
            .collect();
        let width = 16 * 24;
        let height = characters.len().div_ceil(16) as u32 * 24;
        let mut rgba = vec![0; (width * height * 4) as usize];
        let mut glyphs = HashMap::new();
        for (idx, ch) in characters.into_iter().enumerate() {
            let image = text::render(&ch.to_string(), 16.0, [255; 3], Some([0; 3]));
            let (x, y) = (idx as u32 % 16 * 24, idx as u32 / 16 * 24);
            for py in 0..image.height.min(24) {
                for px in 0..image.width.min(24) {
                    let dest = (((y + py) * width + x + px) * 4) as usize;
                    let source = ((py * image.width + px) * 4) as usize;
                    rgba[dest..dest + 4].copy_from_slice(&image.rgba[source..source + 4]);
                }
            }
            glyphs.insert(ch, (x, y, (image.width as f32 - 2.0).max(4.0)));
        }
        let tex = textures.len();
        textures.push(engine.create_texture(&rgba, width, height));
        let font = Self {
            tex,
            glyphs,
            width,
            height,
        };
        LATIN_FONT.with(|stored| *stored.borrow_mut() = Some(font.clone()));
        font
    }
    pub fn measure(&self, source: &str) -> f32 {
        source
            .chars()
            .map(|ch| {
                self.glyphs
                    .get(&ch)
                    .or_else(|| self.glyphs.get(&'?'))
                    .map_or(8.0, |g| g.2)
            })
            .sum()
    }
    pub fn draw(
        &self,
        cmds: &mut Vec<DrawCmd>,
        origin: [f32; 2],
        scale: [f32; 2],
        tint: [f32; 4],
        source: &str,
    ) {
        let mut pos = origin;
        for ch in source.chars() {
            if ch == '\n' {
                pos[0] = origin[0];
                pos[1] += 20.0 * scale[1];
                continue;
            }
            let Some(&(x, y, advance)) = self.glyphs.get(&ch).or_else(|| self.glyphs.get(&'?'))
            else {
                continue;
            };
            cmds.push(DrawCmd {
                clip: None,
                tex: self.tex,
                dst: [pos[0], pos[1], 24.0 * scale[0], 24.0 * scale[1]],
                src: [
                    x as f32 / self.width as f32,
                    y as f32 / self.height as f32,
                    (x + 24) as f32 / self.width as f32,
                    (y + 24) as f32 / self.height as f32,
                ],
                tint,
                rot: 0.0,
                additive: false,
                color_add: false,
                depth: None,
            });
            pos[0] += advance * scale[0];
        }
    }
}
/// Lets the game's original ASCII renderer retain numbers and untranslated labels.
fn translated_ascii(source: &str) -> Option<String> {
    if language() == Language::Original {
        return None;
    }
    if let Some(value) = lookup_in(language(), source) {
        return Some(value.to_owned());
    }
    const PHRASES: &[&str] = &[
        "Player Penalty",
        "Point Item",
        "Lunatic",
        "Normal",
        "Extra",
        "Easy",
        "Hard",
        "STAGE",
        "Stage",
        "Power",
        "Graze",
        "Player",
        "Bomb",
        "Rank",
        "Total",
        "BONUS",
    ];
    let mut output = String::new();
    let mut offset = 0;
    let mut changed = false;
    while offset < source.len() {
        let tail = &source[offset..];
        let phrase = PHRASES.iter().find(|phrase| {
            tail.starts_with(**phrase)
                && (offset == 0 || !source.as_bytes()[offset - 1].is_ascii_alphabetic())
                && tail
                    .as_bytes()
                    .get(phrase.len())
                    .is_none_or(|byte| !byte.is_ascii_alphabetic())
        });
        if let Some(phrase) = phrase {
            let key = if *phrase == "STAGE" { "Stage" } else { phrase };
            if let Some(value) = lookup_in(language(), key) {
                output.push_str(value);
                changed = true;
            } else {
                output.push_str(phrase);
            }
            offset += phrase.len();
        } else {
            let ch = tail.chars().next().unwrap();
            output.push(ch);
            offset += ch.len_utf8();
        }
    }
    changed.then_some(output)
}
pub fn draw_translated_text(
    cmds: &mut Vec<DrawCmd>,
    pos: [f32; 2],
    size: f32,
    tint: [f32; 4],
    source: &str,
) -> bool {
    let Some(translated) = translated_ascii(source) else {
        return false;
    };
    LATIN_FONT.with(|stored| {
        let font = stored.borrow();
        let Some(font) = font.as_ref() else {
            return false;
        };
        let scale = size / 16.0;
        let right = if pos[0] < 416.0 { 416.0 } else { 640.0 };
        let fit = ((right - pos[0] - 4.0).max(1.0) / (font.measure(&translated) * scale).max(1.0))
            .min(1.0);
        font.draw(cmds, pos, [scale * fit, scale], tint, &translated);
        true
    })
}
pub fn draw_label(
    cmds: &mut Vec<DrawCmd>,
    pos: [f32; 2],
    size: f32,
    tint: [f32; 4],
    key: &str,
    fallback: &str,
    max_width: f32,
) -> bool {
    LATIN_FONT.with(|stored| {
        let font = stored.borrow();
        let Some(font) = font.as_ref() else {
            return false;
        };
        let source = ui(key, fallback);
        let scale = size / 16.0;
        let fit = (max_width / (font.measure(&source) * scale).max(1.0)).min(1.0);
        font.draw(cmds, pos, [scale * fit, scale], tint, &source);
        true
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn language_identifiers_and_unknown_fallback_are_stable() {
        for value in Language::ALL {
            assert_eq!(Language::from_u8(value as u8), Some(value));
        }
        assert_eq!(Language::from_u8(5), None);
        assert_eq!(Language::Original.cycle(-1), Language::Dutch);
        assert_eq!(Language::Dutch.cycle(1), Language::Original);
        assert_eq!(
            translate_in(Language::German, "unregistered text"),
            "unregistered text"
        );
        assert_eq!(translate_in(Language::Original, "menu.start"), "menu.start");
    }
    #[test]
    fn every_catalog_contains_language_and_hitbox_options() {
        for value in &Language::ALL[1..] {
            assert!(lookup_in(*value, "ui.language").is_some());
            assert!(lookup_in(*value, "ui.hitbox").is_some());
        }
    }
    #[test]
    fn dynamic_stage_labels_keep_numeric_values_and_word_boundaries() {
        set_language(Language::French);
        assert_eq!(
            translated_ascii("Stage 6  Power 128"),
            Some("Niveau 6  Puissance 128".into())
        );
        assert_eq!(translated_ascii("Bombard 123"), None);
        set_language(Language::Original);
        assert_eq!(translated_ascii("Stage 6  Power 128"), None);
    }
    #[test]
    fn tall_render_keeps_its_bottom_rows_inside_the_canvas() {
        // An 18-row render in a 16-row sprite: the last source row (descenders,
        // shadow) must still reach the canvas instead of being cut off.
        let mut rgba = vec![0; 4 * 18 * 4];
        for x in 0..4 {
            rgba[(17 * 4 + x) * 4..(17 * 4 + x) * 4 + 4].copy_from_slice(&[255; 4]);
        }
        let image = canvas_image(
            TextImage {
                width: 4,
                height: 18,
                rgba,
            },
            [4, 16],
            SpriteTextAlign::Left,
        );
        assert_eq!((image.width, image.height), (4, 16));
        assert!(
            image.rgba[15 * 4 * 4..]
                .chunks_exact(4)
                .all(|p| p[3] == 255)
        );
    }

    #[test]
    fn long_text_fits_without_changing_texture_dimensions() {
        let image = canvas_image(
            text::render("Sehr langer übersetzter Text", 16.0, [255; 3], None),
            [64, 20],
            SpriteTextAlign::Right,
        );
        assert_eq!(
            (image.width, image.height, image.rgba.len()),
            (64, 20, 64 * 20 * 4)
        );
        assert!(image.rgba.chunks_exact(4).any(|pixel| pixel[3] != 0));
    }
}
