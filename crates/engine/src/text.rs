//! Japanese (Shift-JIS) text rasterisation — the port's stand-in for the
//! decomp's `TextHelper::RenderTextToTexture` (which used a GDI MS-Gothic font
//! via `TextOutA`). We can't use MS Gothic (proprietary) or GDI (Windows), so a
//! pure-Rust rasteriser (`ab_glyph`) draws a bundled free Gothic (Noto Sans JP,
//! SIL OFL) — a close substitute. Same recipe: a shadow pass at (+3,+2) then the
//! main text, both AA. Shift-JIS bytes are decoded with `encoding_rs`.
//!
//! Output is a tight RGBA8 bitmap; the caller uploads it as a texture and draws
//! it like any other sprite.

use ab_glyph::{Font, FontRef, Glyph, PxScale, ScaleFont};

/// Bundled JP Gothic (Bold, to match the decomp's `FW_BOLD`).
const FONT: &[u8] = include_bytes!("../assets/NotoSansJP-Bold.otf");

/// A rasterised text bitmap: tightly-sized RGBA8, top-left origin.
pub struct TextImage {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

fn font() -> FontRef<'static> {
    FontRef::try_from_slice(FONT).expect("bundled NotoSansJP is a valid font")
}

/// Decode Shift-JIS bytes to a String (lossy on invalid sequences).
pub fn decode_shiftjis(bytes: &[u8]) -> String {
    let (s, _, _) = encoding_rs::SHIFT_JIS.decode(bytes);
    s.into_owned()
}

/// Rasterise `text` (already decoded to Unicode) at `px` pixels, in `color`,
/// with an optional drop `shadow` at the decomp's (+3,+2)·(px/30) offset.
/// Returns a tight RGBA bitmap (fully transparent where no glyph covers).
pub fn render(text: &str, px: f32, color: [u8; 3], shadow: Option<[u8; 3]>) -> TextImage {
    let font = font();
    let scaled = font.as_scaled(PxScale::from(px));
    let ascent = scaled.ascent();
    let line_h = scaled.height();

    // The decomp's shadow offset is +3,+2 at 2×fontHeight (~30px); scale it.
    let (sdx, sdy) = shadow
        .map(|_| ((px / 30.0 * 3.0).round(), (px / 30.0 * 2.0).round()))
        .unwrap_or((0.0, 0.0));

    // Lay glyphs left-to-right at the font's natural advances; measure width.
    let mut caret = 0.0f32;
    let mut glyphs: Vec<(Glyph, ab_glyph::GlyphId)> = Vec::new();
    for ch in text.chars() {
        if ch == '\r' {
            continue;
        }
        let id = font.glyph_id(ch);
        let g = id.with_scale_and_position(px, ab_glyph::point(caret, ascent));
        glyphs.push((g, id));
        caret += scaled.h_advance(id);
    }

    let width = (caret + sdx).ceil().max(1.0) as u32 + 1;
    let height = (line_h + sdy).ceil().max(1.0) as u32 + 1;
    let mut rgba = vec![0u8; (width * height * 4) as usize];

    let mut blit = |gx: f32, gy: f32, col: [u8; 3]| {
        for (g, _) in &glyphs {
            let g = ab_glyph::Glyph {
                position: ab_glyph::point(g.position.x + gx, g.position.y + gy),
                ..g.clone()
            };
            if let Some(outline) = font.outline_glyph(g) {
                let bb = outline.px_bounds();
                outline.draw(|x, y, cov| {
                    let px_ = bb.min.x as i32 + x as i32;
                    let py_ = bb.min.y as i32 + y as i32;
                    if px_ < 0 || py_ < 0 || px_ as u32 >= width || py_ as u32 >= height {
                        return;
                    }
                    let idx = ((py_ as u32 * width + px_ as u32) * 4) as usize;
                    // Source-over: a covered pixel replaces toward `col` by coverage.
                    let a = (cov * 255.0) as u16;
                    let dst_a = rgba[idx + 3] as u16;
                    if a <= dst_a && dst_a == 255 {
                        return;
                    }
                    let na = a.max(dst_a);
                    rgba[idx] = col[0];
                    rgba[idx + 1] = col[1];
                    rgba[idx + 2] = col[2];
                    rgba[idx + 3] = na as u8;
                });
            }
        }
    };

    if shadow.is_some() {
        blit(sdx, sdy, shadow.unwrap());
    }
    blit(0.0, 0.0, color);

    TextImage { width, height, rgba }
}

/// Convenience: decode Shift-JIS then render.
pub fn render_shiftjis(bytes: &[u8], px: f32, color: [u8; 3], shadow: Option<[u8; 3]>) -> TextImage {
    render(&decode_shiftjis(bytes), px, color, shadow)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_japanese_nonempty() {
        // "月符「ムーンライトレイ」" — kanji + brackets + katakana.
        let img = render("月符「ムーンライトレイ」", 24.0, [255, 240, 240], Some([0, 0, 0]));
        assert!(img.width > 40 && img.height > 10, "sane size: {}x{}", img.width, img.height);
        // Some pixels must be opaque (glyphs drew).
        let opaque = img.rgba.chunks_exact(4).filter(|p| p[3] > 128).count();
        assert!(opaque > 100, "glyphs rasterised: {opaque} opaque px");
    }

    #[test]
    fn shiftjis_decode_roundtrip() {
        // Shift-JIS for "月" is 0x8C8E.
        assert_eq!(decode_shiftjis(&[0x8c, 0x8e]), "月");
    }
}
