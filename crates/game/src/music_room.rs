//! MusicRoom.cpp: original ten-row list and 32-byte split comment lines.
use crate::{
    GameFiles,
    menu_art::{MenuArt, MenuFont},
};
use th06_engine::{DrawCmd, Engine, Input, Key, Texture};
struct Text {
    tex: usize,
    w: f32,
    h: f32,
}
struct Track {
    path: String,
    title: Text,
    description: Vec<(Text, [f32; 2])>,
}
pub struct MusicRoom {
    art: MenuArt,
    font: MenuFont,
    tracks: Vec<Track>,
    cursor: usize,
    selected: usize,
    offset: usize,
    timer: u16,
}
pub enum MusicAction {
    None,
    Back,
    Play(String),
    PlayAndBack(String),
}
impl MusicRoom {
    pub fn load(engine: &Engine, files: &GameFiles, textures: &mut Vec<Texture>) -> Self {
        let art = MenuArt::load(
            engine,
            &files.tl,
            textures,
            "music.jpg",
            &[("music00.anm", 0)],
        );
        let font = MenuFont::load(engine, &files.inn, textures);
        let mut tracks = Vec::new();
        for (path, title, description) in parse_comments(&files.music_comments) {
            let (tex, w, h) = crate::localization::bake_shiftjis(
                engine,
                textures,
                &title,
                15.0,
                [255, 224, 192],
                Some([128, 32, 48]),
                [496, 20],
            );
            let title = Text { tex, w, h };
            let mut desc = Vec::new();
            for (line, bytes) in description.iter().enumerate().take(8) {
                let (tex, w, h) = crate::localization::bake_custom(
                    engine,
                    textures,
                    bytes,
                    0,
                    [496, 20],
                    raster_comment,
                );
                desc.push((Text { tex, w, h }, [96.0, 320.0 + line as f32 * 16.0]));
            }
            tracks.push(Track {
                path,
                title,
                description: desc,
            });
        }
        Self {
            art,
            font,
            tracks,
            cursor: 0,
            selected: 0,
            offset: 0,
            timer: 0,
        }
    }
    pub fn begin(&mut self) {
        self.timer = 0;
        self.cursor = 0;
        self.selected = 0;
        self.offset = 0;
        self.art.start();
    }
    pub fn update(&mut self, input: &Input) -> MusicAction {
        self.timer = self.timer.saturating_add(1);
        self.art.tick();
        if self.timer < 8 {
            return MusicAction::None;
        }
        let back = input.pressed(Key::Bomb) || input.pressed(Key::Pause);
        let n = self.tracks.len();
        if n == 0 {
            return if back {
                MusicAction::Back
            } else {
                MusicAction::None
            };
        }
        if input.pressed(Key::Up) {
            self.cursor = (self.cursor + n - 1) % n;
        }
        if input.pressed(Key::Down) {
            self.cursor = (self.cursor + 1) % n;
        }
        if self.cursor < self.offset {
            self.offset = self.cursor;
        }
        if self.cursor >= self.offset + 10 {
            self.offset = self.cursor - 9;
        }
        if input.pressed(Key::Shoot) || input.pressed(Key::Enter) {
            self.selected = self.cursor;
            let path = self.tracks[self.selected].path.clone();
            return if back {
                MusicAction::PlayAndBack(path)
            } else {
                MusicAction::Play(path)
            };
        }
        if back {
            MusicAction::Back
        } else {
            MusicAction::None
        }
    }
    pub fn draw(&self) -> Vec<DrawCmd> {
        let mut cmds = self.art.draw(|_| Some(([0.0; 2], [1.0; 4])));
        if self.tracks.is_empty() {
            self.font.text(
                &mut cmds,
                [96.0, 102.0],
                [1.0; 2],
                [1.0; 4],
                false,
                "Missing musiccmt.txt",
            );
            self.font.text(
                &mut cmds,
                [96.0, 126.0],
                [1.0; 2],
                [1.0; 4],
                false,
                "X: return",
            );
            return cmds;
        }
        for i in self.offset..(self.offset + 10).min(self.tracks.len()) {
            let selected = self.cursor == i;
            let c = if selected {
                [1.0; 4]
            } else {
                [128.0 / 255.0, 128.0 / 255.0, 128.0 / 255.0, 224.0 / 255.0]
            };
            let y = 102.0 + (i - self.offset) as f32 * 18.0;
            Self::text(&mut cmds, &self.tracks[i].title, [93.0, y], c);
            self.font.text(
                &mut cmds,
                [48.0, y],
                [1.0; 2],
                c,
                false,
                &format!("{:2}.", i + 1),
            );
            if selected {
                self.font
                    .bytes(&mut cmds, [33.0, y], [1.0; 2], c, false, &[0x7f]);
            }
        }
        for (text, pos) in &self.tracks[self.selected].description {
            Self::text(&mut cmds, text, *pos, [1.0; 4]);
        }
        cmds
    }
    pub fn take_sounds(&mut self) -> Vec<&'static str> {
        self.art.take_sounds()
    }
    fn text(cmds: &mut Vec<DrawCmd>, text: &Text, pos: [f32; 2], tint: [f32; 4]) {
        cmds.push(DrawCmd {
            projected: None,
            clip: None,
            tex: text.tex,
            dst: [pos[0], pos[1], text.w, text.h],
            src: [0.0, 0.0, 1.0, 1.0],
            tint,
            rot: 0.0,
            additive: false,
            color_add: false,
            depth: None,
        });
    }
}
fn raster_comment(
    bytes: &[u8],
    source_key: &str,
    _: u32,
    width: u32,
    height: u32,
    language: crate::localization::Language,
) -> Vec<u8> {
    use th06_engine::text;
    if language != crate::localization::Language::Original {
        return crate::localization::canvas_image(
            text::render(
                &crate::localization::translate_in(language, source_key),
                15.0,
                [192, 224, 255],
                Some([0, 0, 48]),
            ),
            [width, height],
            text::SpriteTextAlign::Left,
        )
        .rgba;
    }
    let mut rgba = vec![0; (width * height * 4) as usize];
    for half in 0..2 {
        let start = half * 32;
        if start >= bytes.len() {
            continue;
        }
        let image = text::render_shiftjis(
            &bytes[start..bytes.len().min(start + 32)],
            15.0,
            [192, 224, 255],
            Some([0, 0, 48]),
        );
        let offset = half as u32 * 248;
        for y in 0..height.min(image.height) {
            for x in 0..image.width.min(width.saturating_sub(offset)) {
                let source = ((y * image.width + x) * 4) as usize;
                let dest = ((y * width + offset + x) * 4) as usize;
                rgba[dest..dest + 4].copy_from_slice(&image.rgba[source..source + 4]);
            }
        }
    }
    rgba
}
/// The source leaves the title's CR/LF in place, producing an empty first comment line.
fn parse_comments(bytes: &[u8]) -> Vec<(String, Vec<u8>, Vec<Vec<u8>>)> {
    let mut out = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] != b'@' {
            at += 1;
            continue;
        }
        at += 1;
        let line = |at: &mut usize| {
            let start = *at;
            while *at < bytes.len() && !b"\r\n".contains(&bytes[*at]) {
                *at += 1;
            }
            bytes[start..*at].to_vec()
        };
        let path = line(&mut at);
        while at < bytes.len() && b"\r\n".contains(&bytes[at]) {
            at += 1;
        }
        let title = line(&mut at);
        let mut desc = Vec::new();
        for _ in 0..8 {
            if at >= bytes.len() || bytes[at] == b'@' {
                break;
            }
            desc.push(line(&mut at));
            while at < bytes.len() && b"\r\n".contains(&bytes[at]) {
                at += 1;
            }
        }
        if !path.is_empty() && !title.is_empty() {
            out.push((String::from_utf8_lossy(&path).into_owned(), title, desc));
        }
    }
    out
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_source_comment_offset() {
        let parsed = parse_comments(
            b"@bgm/th06_01.mid\r\nTitle\r\nDescription\r\n@bgm/th06_02.mid\r\nSecond\r\nDetails",
        );
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].1, b"Title");
        assert_eq!(parsed[0].2[0], b"");
        assert_eq!(parsed[0].2[1], b"Description");
    }

    #[test]
    fn shift_jis_at_byte_inside_a_comment_is_not_a_track_header() {
        let parsed = parse_comments(
            b"@bgm/th06_01.mid\r\nTitle\r\n\x81\x40Comment\r\n@bgm/th06_02.mid\r\nSecond\r\nDetails",
        );
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[1].0, "bgm/th06_02.mid");
        assert_eq!(parsed[0].2[1], b"\x81\x40Comment");
    }
}
