//! Original ANM menu assets, with file-local script and sprite indices.
use crate::anm_vm::AnmRunner;
use crate::localization::{self, Language, LatinFont};
use std::collections::{BTreeMap, HashMap};
use th06_engine::{DrawCmd, Engine, Texture, compose_rgba};
use th06_formats::anm0::{Anm0, Sprite};

struct Vm {
    runner: AnmRunner,
    sprites: HashMap<u32, Sprite>,
    tex: usize,
    size: [f32; 2],
    hidden: bool,
    labels: Vec<u32>,
    script: Vec<th06_formats::anm0::Instr>,
    highlight_tex: Option<usize>,
    highlighted: bool,
    sheet_name: String,
}

/// AsciiManager::DrawStrings: ANM glyph geometry, 14-pixel advance and
/// 16-pixel line height. SP, END and the arrow are sprites, too.
pub struct MenuFont {
    normal: FontSheet,
    selected: FontSheet,
    latin: LatinFont,
}
struct FontSheet {
    tex: usize,
    size: [f32; 2],
    sprites: HashMap<u32, Sprite>,
}
impl MenuFont {
    pub fn load(
        engine: &Engine,
        files: &HashMap<String, Vec<u8>>,
        textures: &mut Vec<Texture>,
    ) -> Self {
        let mut sheet = |anm_name: &str| {
            let anm = Anm0::parse(&files[anm_name]).expect("ASCII ANM");
            let entry = &anm.entries[0];
            let name = entry.name.rsplit('/').next().unwrap();
            let mask = entry
                .alpha_name
                .as_ref()
                .and_then(|n| files.get(n.rsplit('/').next().unwrap()));
            let (rgba, w, h) = compose_rgba(&files[name], mask.map(|b| b.as_slice()));
            let tex = textures.len();
            textures.push(engine.create_texture_with_format(&rgba, w, h, entry.format));
            FontSheet {
                tex,
                size: [w as f32, h as f32],
                sprites: entry.sprites.iter().map(|s| (s.index, s.clone())).collect(),
            }
        };
        let normal = sheet("ascii.anm");
        let selected = sheet("asciis.anm");
        let latin = LatinFont::load(engine, textures);
        Self {
            normal,
            selected,
            latin,
        }
    }
    pub fn text(
        &self,
        cmds: &mut Vec<DrawCmd>,
        pos: [f32; 2],
        scale: [f32; 2],
        color: [f32; 4],
        selected: bool,
        text: &str,
    ) {
        if let Some(translated) = localization::lookup_in(localization::language(), text)
            .or_else(|| localization::lookup_in(localization::language(), text.trim()))
        {
            let fit = ((640.0 - pos[0] - 4.0).max(1.0)
                / (self.latin.measure(translated) * scale[0]).max(1.0))
            .min(1.0);
            self.latin
                .draw(cmds, pos, [scale[0] * fit, scale[1]], color, translated);
            return;
        }
        if localization::language() != Language::Original && !text.is_ascii() {
            self.latin.draw(cmds, pos, scale, color, text);
            return;
        }
        let bytes: Vec<u8> = text
            .chars()
            .map(|c| {
                if c == '･' {
                    0xa5
                } else if c as u32 <= 255 {
                    c as u8
                } else {
                    b'?'
                }
            })
            .collect();
        self.bytes(cmds, pos, scale, color, selected, &bytes);
    }
    pub fn bytes(
        &self,
        cmds: &mut Vec<DrawCmd>,
        origin: [f32; 2],
        scale: [f32; 2],
        color: [f32; 4],
        selected: bool,
        text: &[u8],
    ) {
        let mut pos = origin;
        for &ch in text {
            if ch == 0 {
                break;
            }
            if ch == b'\n' {
                pos[0] = origin[0];
                pos[1] += 16.0 * scale[1];
                continue;
            }
            if ch > b' ' {
                let mut sheet = if selected {
                    &self.selected
                } else {
                    &self.normal
                };
                let mut idx = (ch - b' ') as u32 + if selected { 10 } else { 11 };
                // The original JP keyboard's 0xa5 byte indexes the next global
                // sheet, exactly as GetSprite does in AsciiManager.
                if !selected && idx >= 119 {
                    idx -= 119;
                    sheet = &self.selected;
                }
                if let Some(sp) = sheet.sprites.get(&idx) {
                    let [w, h] = sheet.size;
                    cmds.push(DrawCmd {
                        clip: None,
                        tex: sheet.tex,
                        dst: [pos[0], pos[1], sp.width * scale[0], sp.height * scale[1]],
                        src: [
                            sp.x / w,
                            sp.y / h,
                            (sp.x + sp.width) / w,
                            (sp.y + sp.height) / h,
                        ],
                        tint: if selected { [1.0; 4] } else { color },
                        rot: 0.0,
                        additive: false,
                        color_add: false,
                        depth: None,
                    });
                }
            }
            pos[0] += 14.0 * scale[0];
        }
    }
}

/// Supervisor.cpp's common 30-frame delay, then eight-frame input repeat.
#[derive(Clone, Copy, Default)]
pub struct MenuInput {
    previous: u16,
    before: u16,
    held_frames: u16,
    repeating: bool,
}
impl MenuInput {
    pub fn tick(&mut self, input: &th06_engine::Input) {
        let buttons = input.buttons();
        self.before = self.previous;
        self.repeating = false;
        if buttons == self.previous {
            if self.held_frames >= 30 {
                self.repeating = self.held_frames.is_multiple_of(8);
                if self.held_frames >= 38 {
                    self.held_frames = 30;
                }
            }
            self.held_frames += 1;
        } else {
            self.held_frames = 0;
        }
        self.previous = buttons;
    }
    pub fn pressed(&self, input: &th06_engine::Input, key: th06_engine::Key) -> bool {
        input.pressed(key) || (input.held(key) && self.repeating)
    }
    /// WAS_PRESSED_WEIRD on a combined mask also notices a released member
    /// when another member remains held; individual pressed-key tests do not.
    pub fn pressed_mask(&self, input: &th06_engine::Input, mask: u16) -> bool {
        let current = input.buttons() & mask;
        current != 0 && (current != self.before & mask || self.repeating)
    }
}

pub struct MenuArt {
    pub background: usize,
    vms: BTreeMap<usize, Vm>,
    sounds: Vec<&'static str>,
    latin: LatinFont,
}

impl MenuArt {
    #[cfg(test)]
    pub(crate) fn title_test_fixture() -> Self {
        let vms = (0..8)
            .map(|idx| {
                let script = vec![
                    th06_formats::anm0::Instr {
                        time: 0,
                        opcode: 1,
                        args: 0u32.to_le_bytes().to_vec(),
                        offset: 0,
                    },
                    th06_formats::anm0::Instr {
                        time: 0,
                        opcode: 15,
                        args: Vec::new(),
                        offset: 8,
                    },
                ];
                let runner = AnmRunner::new(script.clone());
                (
                    idx,
                    Vm {
                        runner,
                        sprites: HashMap::from([(
                            0,
                            Sprite {
                                index: 0,
                                x: 0.0,
                                y: 0.0,
                                width: 1.0,
                                height: 1.0,
                            },
                        )]),
                        tex: idx + 1,
                        size: [1.0; 2],
                        hidden: false,
                        labels: vec![1, 2, 4],
                        script,
                        highlight_tex: None,
                        highlighted: false,
                        sheet_name: "title-test-fixture".into(),
                    },
                )
            })
            .collect();
        Self {
            background: 0,
            vms,
            sounds: Vec::new(),
            latin: LatinFont::default(),
        }
    }
    pub fn load(
        engine: &Engine,
        files: &HashMap<String, Vec<u8>>,
        textures: &mut Vec<Texture>,
        background: &str,
        sheets: &[(&str, usize)],
    ) -> Self {
        let (rgba, w, h) = compose_rgba(&files[background], None);
        let background = textures.len();
        textures.push(engine.create_texture_with_format(&rgba, w, h, 4));
        let mut vms = BTreeMap::new();
        for &(name, offset) in sheets {
            let anm = Anm0::parse(&files[name]).expect("menu ANM");
            for entry in anm.entries {
                let name = entry.name.rsplit('/').next().unwrap();
                if name == "@" {
                    continue;
                }
                let mask = entry
                    .alpha_name
                    .as_ref()
                    .and_then(|n| files.get(n.rsplit('/').next().unwrap()));
                let (rgba, w, h) = compose_rgba(&files[name], mask.map(|b| b.as_slice()));
                let tex = textures.len();
                textures.push(engine.create_texture_with_format(&rgba, w, h, entry.format));
                let highlight_tex = if name == "title01.png" || name == "title04.png" {
                    let alt = name.replace(".png", "s.png");
                    // title01s/title04s retain their base sheet's alpha mask.
                    files.get(&alt).map(|bytes| {
                        let (rgba, w, h) = compose_rgba(bytes, mask.map(|b| b.as_slice()));
                        let idx = textures.len();
                        textures.push(engine.create_texture_with_format(&rgba, w, h, entry.format));
                        idx
                    })
                } else {
                    None
                };
                for (id, script) in entry.scripts {
                    let labels = script
                        .iter()
                        .filter(|i| i.opcode == 22)
                        .map(|i| i.arg_u32(0))
                        .collect();
                    vms.insert(
                        offset + id as usize,
                        Vm {
                            runner: AnmRunner::new(script.clone()),
                            sprites: entry.sprites.iter().map(|s| (s.index, s.clone())).collect(),
                            tex,
                            size: [entry.width as f32, entry.height as f32],
                            hidden: true,
                            labels,
                            script,
                            highlight_tex,
                            highlighted: false,
                            sheet_name: name.to_string(),
                        },
                    );
                }
            }
        }
        let latin = LatinFont::load(engine, textures);
        Self {
            background,
            vms,
            sounds: Vec::new(),
            latin,
        }
    }
    /// Port feature text uses Unicode glyphs, including Western accents.
    pub fn text(
        &self,
        cmds: &mut Vec<DrawCmd>,
        pos: [f32; 2],
        scale: f32,
        tint: [f32; 4],
        source: &str,
        max_width: f32,
    ) {
        let width = self.latin.measure(source) * scale;
        let sx = scale * (max_width / width.max(1.0)).min(1.0);
        self.latin.draw(cmds, pos, [sx, scale], tint, source);
    }

    /// SoundPlayer's menu cues, drained once after updating the active scene.
    pub fn sound(&mut self, name: &'static str) {
        self.sounds.push(name);
    }
    pub fn take_sounds(&mut self) -> Vec<&'static str> {
        std::mem::take(&mut self.sounds)
    }

    pub fn reset(&mut self) {
        for vm in self.vms.values_mut() {
            vm.runner = AnmRunner::new(vm.script.clone());
            vm.hidden = true;
            vm.highlighted = false;
        }
    }
    pub fn start(&mut self) {
        self.reset();
        for vm in self.vms.values_mut() {
            vm.hidden = false;
        }
    }
    pub fn highlight(&mut self, idx: usize, highlighted: bool) {
        if let Some(vm) = self.vms.get_mut(&idx) {
            vm.highlighted = highlighted;
        }
    }
    pub fn scale(&mut self, idx: usize, scale: [f32; 2]) {
        if let Some(vm) = self.vms.get_mut(&idx) {
            vm.runner.scale = scale;
        }
    }
    pub fn sprite(&mut self, idx: usize, sprite: u32) {
        if let Some(vm) = self.vms.get_mut(&idx) {
            vm.runner.sprite = Some(sprite);
        }
    }
    pub fn interrupt_all(&mut self, label: u32) {
        for vm in self.vms.values_mut() {
            vm.hidden = !vm.labels.contains(&label) && !vm.labels.contains(&u32::MAX);
            vm.runner.interrupt(label);
        }
    }
    pub fn interrupt(&mut self, idx: usize, label: u32) {
        if let Some(vm) = self.vms.get_mut(&idx) {
            vm.hidden =
                label == 0 || (!vm.labels.contains(&label) && !vm.labels.contains(&u32::MAX));
            vm.runner.interrupt(label);
        }
    }
    pub fn tick(&mut self) {
        for vm in self.vms.values_mut() {
            vm.runner.tick();
        }
    }
    /// Text in the original menus follows ANM VMs, including offscreen exit motion.
    pub fn position(&self, idx: usize) -> [f32; 2] {
        self.vms.get(&idx).map_or([0.0; 2], |v| {
            [
                v.runner.pos[0] + v.runner.pos_offset[0],
                v.runner.pos[1] + v.runner.pos_offset[1],
            ]
        })
    }
    pub fn alpha(&self, idx: usize) -> f32 {
        self.vms.get(&idx).map_or(0.0, |v| v.runner.alpha)
    }
    pub fn scale_of(&self, idx: usize) -> [f32; 2] {
        self.vms.get(&idx).map_or([1.0; 2], |v| v.runner.scale)
    }
    pub fn copy_sheet(&mut self, dst: usize, src: usize) {
        let Some(source) = self.vms.get(&src) else {
            return;
        };
        let sheet = (
            source.sprites.clone(),
            source.tex,
            source.size,
            source.highlight_tex,
            source.sheet_name.clone(),
        );
        if let Some(target) = self.vms.get_mut(&dst) {
            target.sprites = sheet.0;
            target.tex = sheet.1;
            target.size = sheet.2;
            target.highlight_tex = sheet.3;
            target.sheet_name = sheet.4;
        }
    }
    pub fn extra_sprite(
        &self,
        idx: usize,
        sprite: u32,
        offset: [f32; 2],
        tint: [f32; 4],
    ) -> Option<DrawCmd> {
        let vm = self.vms.get(&idx)?;
        let r = &vm.runner;
        let sp = vm.sprites.get(&sprite)?;
        let (w, h) = (sp.width * r.scale[0], sp.height * r.scale[1]);
        let pos = [
            r.pos[0] + r.pos_offset[0] + offset[0],
            r.pos[1] + r.pos_offset[1] + offset[1],
        ];
        let geometry = crate::sprite_geometry::screen_rotated(pos, [w, h], r.rotation, r.corner);
        Some(DrawCmd {
            clip: None,
            tex: vm.tex,
            dst: geometry.dst,
            src: [
                sp.x / vm.size[0],
                sp.y / vm.size[1],
                (sp.x + sp.width) / vm.size[0],
                (sp.y + sp.height) / vm.size[1],
            ],
            tint: [tint[0], tint[1], tint[2], tint[3] * r.alpha],
            rot: geometry.rotation,
            additive: r.additive,
            color_add: false,
            depth: None,
        })
    }
    pub fn draw(
        &self,
        mut style: impl FnMut(usize) -> Option<([f32; 2], [f32; 4])>,
    ) -> Vec<DrawCmd> {
        let mut cmds = vec![DrawCmd {
            clip: None,
            tex: self.background,
            dst: [0.0, 0.0, 640.0, 480.0],
            src: [0.0, 0.0, 1.0, 1.0],
            tint: [1.0; 4],
            rot: 0.0,
            additive: false,
            color_add: false,
            depth: None,
        }];
        for (&idx, vm) in &self.vms {
            let r = &vm.runner;
            if vm.hidden || !r.visible() {
                continue;
            }
            let Some((offset, tint)) = style(idx) else {
                continue;
            };
            let Some(sp) = r.sprite.and_then(|n| vm.sprites.get(&n)) else {
                continue;
            };
            let [w, h] = [sp.width * r.scale[0], sp.height * r.scale[1]];
            let [tw, th] = vm.size;
            let pos = [
                r.pos[0] + r.pos_offset[0] + offset[0],
                r.pos[1] + r.pos_offset[1] + offset[1],
            ];
            let geometry =
                crate::sprite_geometry::screen_rotated(pos, [w, h], r.rotation, r.corner);
            if localization::language() != Language::Original
                && let Some((key, fallback)) = translated_sprite_label(&vm.sheet_name, sp.index)
            {
                let label = localization::ui(key, fallback);
                let lines: Vec<_> = label.lines().collect();
                if vm.sheet_name == "result03.png" {
                    for (line, source) in lines.iter().enumerate() {
                        self.text(
                            &mut cmds,
                            [
                                geometry.dst[0],
                                geometry.dst[1]
                                    + if line == 0 {
                                        0.0
                                    } else {
                                        32.0 + (line - 1) as f32 * 22.0
                                    },
                            ],
                            if line == 0 { 1.3 } else { 1.0 },
                            [tint[0], tint[1], tint[2], tint[3] * r.alpha],
                            source,
                            if line == 0 { 256.0 } else { 220.0 },
                        );
                    }
                    continue;
                }
                let nominal = (h / (lines.len().max(1) as f32 * 22.0)).clamp(0.65, 1.6);
                for (line, source) in lines.iter().enumerate() {
                    self.text(
                        &mut cmds,
                        [
                            geometry.dst[0],
                            geometry.dst[1] + line as f32 * 22.0 * nominal,
                        ],
                        nominal,
                        [tint[0], tint[1], tint[2], tint[3] * r.alpha],
                        source,
                        w,
                    );
                }
                continue;
            }
            let (u0, u1) = if r.flip_x {
                ((sp.x + sp.width) / tw, sp.x / tw)
            } else {
                (sp.x / tw, (sp.x + sp.width) / tw)
            };
            let (v0, v1) = if r.flip_y {
                ((sp.y + sp.height) / th, sp.y / th)
            } else {
                (sp.y / th, (sp.y + sp.height) / th)
            };
            cmds.push(DrawCmd {
                clip: None,
                tex: if vm.highlighted {
                    vm.highlight_tex.unwrap_or(vm.tex)
                } else {
                    vm.tex
                },
                dst: geometry.dst,
                src: [u0, v0, u1, v1],
                tint: [tint[0], tint[1], tint[2], tint[3] * r.alpha],
                rot: geometry.rotation,
                additive: r.additive,
                color_add: false,
                depth: None,
            });
            if localization::language() != Language::Original
                && (vm.sheet_name == "slpl00b.png" || vm.sheet_name == "slpl01b.png")
            {
                let original = cmds.pop().expect("character portrait command");
                let mut upper = original;
                upper.dst[3] = 94.0 * r.scale[1];
                upper.src[3] = (sp.y + 94.0) / th;
                cmds.push(upper);
                let mut ratings = original;
                ratings.dst = [
                    geometry.dst[0] + 176.0 * r.scale[0],
                    geometry.dst[1] + 148.0 * r.scale[1],
                    80.0 * r.scale[0],
                    62.0 * r.scale[1],
                ];
                ratings.src = [
                    (sp.x + 176.0) / tw,
                    (sp.y + 148.0) / th,
                    (sp.x + 256.0) / tw,
                    (sp.y + 210.0) / th,
                ];
                cmds.push(ratings);
                let marisa = vm.sheet_name == "slpl01b.png";
                let labels = [
                    localization::ui(
                        if marisa {
                            "character.marisa_title"
                        } else {
                            "character.reimu_title"
                        },
                        if marisa {
                            "Ordinary magician"
                        } else {
                            "Shrine maiden of Hakurei"
                        },
                    ),
                    localization::ui(
                        if marisa {
                            "character.marisa"
                        } else {
                            "character.reimu"
                        },
                        if marisa {
                            "Marisa Kirisame"
                        } else {
                            "Reimu Hakurei"
                        },
                    ),
                    localization::ui("character.speed", "Speed"),
                    localization::ui("character.range", "Attack range"),
                    localization::ui("character.power", "Attack power"),
                ];
                for (line, source) in labels.iter().enumerate() {
                    let y = [96.0, 122.0, 150.0, 168.0, 186.0][line];
                    self.text(
                        &mut cmds,
                        [
                            geometry.dst[0] + 38.0 * r.scale[0],
                            geometry.dst[1] + y * r.scale[1],
                        ],
                        if line < 2 {
                            r.scale[0]
                        } else {
                            0.85 * r.scale[0]
                        },
                        [tint[0], tint[1], tint[2], tint[3] * r.alpha],
                        source,
                        if line < 2 {
                            210.0 * r.scale[0]
                        } else {
                            136.0 * r.scale[0]
                        },
                    );
                }
            }
        }
        cmds
    }
}

/// These are text-only sprites. Portraits, emblems and decorative art retain
/// their original images; every replacement follows its source ANM VM.
fn translated_sprite_label(sheet: &str, sprite: u32) -> Option<(&'static str, &'static str)> {
    let labels: &[(&str, &str)] = match sheet {
        "title01.png" => {
            return match sprite {
                10 => Some(("menu.start", "Start")),
                11 => Some(("menu.extra", "Extra")),
                12 => Some(("menu.replay", "Replays")),
                13 => Some(("menu.score", "Records")),
                14 => Some(("menu.quit", "Quit")),
                15 => Some(("menu.options", "Options")),
                25 => Some(("menu.music", "Music Room")),
                26 => Some(("menu.practice", "Practice")),
                _ => None,
            };
        }
        "title04.png" => &[
            ("controller.shoot", "Shoot / Confirm"),
            ("controller.bomb", "Bomb / Cancel"),
            ("controller.focus", "Focus"),
            ("controller.up", "Up"),
            ("controller.down", "Down"),
            ("controller.left", "Left"),
            ("controller.right", "Right"),
            ("controller.button", "Button"),
            ("menu.back", "Back"),
            ("controller.pause", "Pause"),
            ("options.defaults", "Restore defaults"),
            ("options.off", "Off"),
            ("options.window", "Display"),
            ("options.fullscreen", "Fullscreen"),
            ("options.windowed", "Windowed"),
            ("options.wav", "WAV"),
            ("options.midi", "MIDI"),
            ("options.controller", "Controller"),
            ("controller.skip", "Skip"),
        ],
        "select01.png" => &[
            ("difficulty.easy_card", "Easy\nFor new players"),
            ("difficulty.normal_card", "Normal\nThe standard challenge"),
            ("selection.mode", "Select difficulty"),
        ],
        "select02.png" => &[
            ("difficulty.hard_card", "Hard\nArcade difficulty"),
            ("difficulty.lunatic_card", "Lunatic\nFor the fearless"),
        ],
        "select05.png" => &[("difficulty.extra_card", "Extra\nA special challenge")],
        "select03.png" => &[
            ("selection.player", "Select player"),
            ("selection.shot", "Select shot type"),
        ],
        "select04.png" => &[
            ("shot.reimu_a", "Spirit: Homing Amulet\nBomb: Fantasy Seal"),
            (
                "shot.reimu_b",
                "Dream: Persuasion Needle\nBomb: Evil-Sealing Circle",
            ),
            (
                "shot.marisa_a",
                "Magic: Magic Missile\nBomb: Stardust Reverie",
            ),
            ("shot.marisa_b", "Love: Illusion Laser\nBomb: Master Spark"),
        ],
        "result00.png" => &[
            ("result.records", "Score and spell card records"),
            ("result.easy", "Easy records"),
            ("result.normal", "Normal records"),
            ("result.hard", "Hard records"),
            ("result.lunatic", "Lunatic records"),
            ("result.extra", "Extra records"),
            ("result.spells", "Spell card records"),
            ("menu.back", "Back"),
        ],
        "result01.png" => &[
            ("difficulty.easy", "Easy"),
            ("difficulty.normal", "Normal"),
            ("difficulty.hard", "Hard"),
            ("difficulty.lunatic", "Lunatic"),
            ("difficulty.extra", "Extra"),
            ("result.spells", "Spell cards"),
        ],
        "result02.png" => &[
            ("result.save_replay", "Save this replay?"),
            ("ui.yes", "Yes"),
            ("ui.no", "No"),
            (
                "result.continue_no_replay",
                "A replay cannot be saved\nafter using a continue.",
            ),
            (
                "result.refresh_no_replay",
                "A replay cannot be saved\nabove 60 Hz.",
            ),
            ("result.overwrite", "Overwrite this replay?"),
        ],
        "result03.png" => &[(
            "result.stats",
            "Your results\nFinal score\nDifficulty\nGame completed\nContinues used\nMisses\nBombs used\nSpell cards captured\nSlowdown rate",
        )],
        "music00.png" => &[("menu.music", "Music Room")],
        "replay00.png" => &[
            ("replay.title", "Watch your replays"),
            ("replay.stage", "Stage"),
            ("difficulty.easy", "Easy"),
            ("difficulty.normal", "Normal"),
            ("difficulty.hard", "Hard"),
            ("difficulty.lunatic", "Lunatic"),
            ("difficulty.extra", "Extra"),
            ("result.score", "Final score"),
            ("result.score", "Score"),
            ("result.slowdown", "Slowdown rate"),
        ],
        "ascii.png" => {
            return match sprite {
                115 => Some(("ui.yes", "Yes")),
                116 => Some(("ui.no", "No")),
                117 => Some(("retry.continue", "Continue?")),
                118 => Some(("retry.remaining", "Remaining")),
                _ => None,
            };
        }
        _ => return None,
    };
    labels.get(sprite as usize).copied()
}

#[cfg(test)]
mod tests {
    use super::*;
    use th06_engine::{Input, Key};

    #[test]
    fn original_menu_repeat_starts_after_delay_then_every_eight_frames() {
        let mut repeat = MenuInput::default();
        let mut triggered = Vec::new();
        for frame in 0..58 {
            let input = Input::from_buttons(1 << 5, if frame == 0 { 0 } else { 1 << 5 });
            repeat.tick(&input);
            if repeat.pressed(&input, Key::Down) {
                triggered.push(frame);
            }
        }
        assert_eq!(triggered, [0, 33, 41, 49, 57]);
    }

    #[test]
    fn menu_cues_survive_scene_animation_reset_and_are_drained_once() {
        let mut art = MenuArt {
            background: 0,
            vms: BTreeMap::new(),
            sounds: Vec::new(),
            latin: LatinFont::default(),
        };
        art.sound("select00");
        art.sound("cancel00");
        art.reset();
        assert_eq!(art.take_sounds(), ["select00", "cancel00"]);
        assert!(art.take_sounds().is_empty());
    }

    #[test]
    fn title_registration_sets_context_cursor_and_selection_startup_keeps_timer_one() {
        let art = MenuArt {
            background: 0,
            vms: BTreeMap::new(),
            sounds: Vec::new(),
            latin: LatinFont::default(),
        };
        let mut title = crate::title::Title::new(art);
        title.reset(5);
        assert_eq!(title.reference_navigation(), (1, 5, 60));
        title.reset(0);
        assert_eq!(title.reference_navigation(), (1, 0, 60));
        title.begin_startup(2);
        assert_eq!(title.reference_navigation(), (1, 2, 1));
        let _ = title.update(&Input::from_buttons(1, 0));
        assert_eq!(title.reference_navigation(), (1, 2, 2));
    }

    #[test]
    fn difficulty_cancel_retains_unload_then_startup_present() {
        use crate::selection::{RunMode, Selection, SelectionAction};
        for (mode, expected_cursor) in [
            (RunMode::Story, 0),
            (RunMode::Extra, 1),
            (RunMode::Practice, 2),
        ] {
            let art = MenuArt {
                background: 0,
                vms: BTreeMap::new(),
                sounds: Vec::new(),
                latin: LatinFont::default(),
            };
            let font = MenuFont {
                normal: FontSheet {
                    tex: 0,
                    size: [1.0; 2],
                    sprites: HashMap::new(),
                },
                selected: FontSheet {
                    tex: 0,
                    size: [1.0; 2],
                    sprites: HashMap::new(),
                },
                latin: LatinFont::default(),
            };
            let mut selection = Selection::new(art, font);
            selection.begin(mode, 2);
            assert!(matches!(
                selection.update(&Input::from_buttons(2, 0), [true; 4], 6),
                SelectionAction::None
            ));
            assert_eq!(selection.reference_navigation(), (8, expected_cursor, 1));
            for timer in 2..=36 {
                assert!(matches!(
                    selection.update(&Input::default(), [true; 4], 6),
                    SelectionAction::None
                ));
                assert_eq!(
                    selection.reference_navigation(),
                    (8, expected_cursor, timer)
                );
            }
            assert!(matches!(
                selection.update(&Input::default(), [true; 4], 6),
                SelectionAction::None
            ));
            assert_eq!(selection.reference_navigation(), (0, expected_cursor, 1));
            assert!(
                matches!(selection.update(&Input::default(), [true; 4], 6), SelectionAction::Back { cursor } if cursor == expected_cursor)
            );
        }
    }

    #[test]
    fn original_main_menu_held_input_survives_difficulty_load_and_cancel() {
        use crate::selection::{RunMode, Selection, SelectionAction};
        use crate::title::{Title, TitleAction};
        // Genuine 102h.exe natural keyboard probes: selection-entry-held and
        // selection-cancel-held. These are MainMenu state changes, so the
        // Supervisor repeat counter is neither reset nor ticked twice.
        for held_direction in [false, true] {
            let art = || MenuArt {
                background: 0,
                vms: BTreeMap::new(),
                sounds: Vec::new(),
                latin: LatinFont::default(),
            };
            let sheet = || FontSheet {
                tex: 0,
                size: [1.0; 2],
                sprites: HashMap::new(),
            };
            let mut title = Title::new(art());
            let mut selection = Selection::new(
                art(),
                MenuFont {
                    normal: sheet(),
                    selected: sheet(),
                    latin: LatinFont::default(),
                },
            );
            let mut in_selection = false;
            let mut previous = 0;
            for present in 0..=413 {
                let buttons = if (180..184).contains(&present) {
                    1 << 9 // Q dismisses PRE_INPUT.
                } else if held_direction && (240..340).contains(&present) {
                    (1 << 5) | 1 // Down + Z held through difficulty loading.
                } else if !held_direction && (240..244).contains(&present) {
                    1
                } else if !held_direction && (340..450).contains(&present) {
                    2 // X held through cancel and title startup.
                } else {
                    0
                };
                let input = Input::from_buttons(buttons, previous);
                previous = buttons;
                if in_selection {
                    if let SelectionAction::Back { cursor } =
                        selection.update(&input, [false; 4], 6)
                    {
                        title.begin_startup(cursor);
                        title.input = selection.input;
                        let _ = title.update(&input);
                        in_selection = false;
                    }
                } else {
                    let (_, action) = title.update(&input);
                    let mode = match action {
                        TitleAction::StartGame => Some(RunMode::Story),
                        TitleAction::Practice => Some(RunMode::Practice),
                        _ => None,
                    };
                    if let Some(mode) = mode {
                        selection.begin(mode, 1);
                        selection.input = title.input;
                        in_selection = true;
                    }
                }
                if held_direction && present == 305 {
                    assert_eq!(selection.reference_navigation(), (7, 2, 66));
                }
                if !held_direction && present == 410 {
                    assert_eq!(title.reference_navigation(), (1, 0, 35));
                }
                if !held_direction && present == 413 {
                    assert_eq!(title.reference_navigation(), (2, 0, 1));
                }
            }
        }
    }

    #[test]
    fn restore_defaults_preserves_original_display_choices_and_restarts_music() {
        use crate::options::{Options, OptionsAction, OptionsMenu};
        let art = MenuArt {
            background: 0,
            vms: BTreeMap::new(),
            sounds: Vec::new(),
            latin: LatinFont::default(),
        };
        let mut menu = OptionsMenu::new(art);
        menu.midi_available = true;
        menu.begin();
        let mut options = Options {
            life_count: 4,
            bomb_count: 0,
            color_16bit: true,
            music_mode: 2,
            play_sounds: false,
            windowed: true,
            default_difficulty: 3,
            hardware_texture_blending: true,
            ..Options::default()
        };
        for _ in 0..32 {
            menu.update(&Input::default(), &mut options);
        }
        for _ in 0..6 {
            menu.update(&Input::from_buttons(1 << 5, 0), &mut options);
            menu.update(&Input::default(), &mut options);
        }
        assert!(matches!(
            menu.update(&Input::from_buttons(1, 0), &mut options),
            OptionsAction::DefaultsRestored
        ));
        assert_eq!(menu.take_music_modes(), [1]);
        assert_eq!(options.life_count, 2);
        assert_eq!(options.bomb_count, 3);
        assert_eq!(options.music_mode, 1);
        assert!(options.play_sounds);
        assert_eq!(options.default_difficulty, 1);
        assert!(options.color_16bit);
        assert!(options.windowed);
        assert!(options.hardware_texture_blending);
        // Restoring an already-default audio mode still has its own action:
        // the dispatcher must stop/restart title playback in this case too.
        menu.update(&Input::default(), &mut options);
        assert!(matches!(
            menu.update(&Input::from_buttons(1, 0), &mut options),
            OptionsAction::DefaultsRestored
        ));
        assert_eq!(menu.take_music_modes(), [1]);
    }

    #[test]
    fn simultaneous_options_inputs_follow_original_order_and_retain_music_requests() {
        use crate::options::{Options, OptionsAction, OptionsMenu};
        let art = MenuArt {
            background: 0,
            vms: BTreeMap::new(),
            sounds: Vec::new(),
            latin: LatinFont::default(),
        };
        let mut menu = OptionsMenu::new(art);
        menu.midi_available = true;
        menu.begin();
        let mut options = Options::default();
        for _ in 0..32 {
            menu.update(&Input::default(), &mut options);
        }
        for _ in 0..3 {
            menu.update(&Input::from_buttons(1 << 5, 0), &mut options);
            menu.update(&Input::default(), &mut options);
        }
        menu.update(&Input::from_buttons((1 << 6) | (1 << 7), 0), &mut options);
        assert_eq!(options.music_mode, 1);
        assert_eq!(menu.take_music_modes(), [0, 1]);
        menu.update(&Input::default(), &mut options);
        // UP wins over DOWN: music row3 becomes color row2.
        menu.update(&Input::from_buttons((1 << 4) | (1 << 5), 0), &mut options);
        menu.update(&Input::default(), &mut options);
        menu.update(&Input::from_buttons(1 << 7, 0), &mut options);
        assert!(options.color_16bit);
        assert!(menu.take_music_modes().is_empty());
        menu.begin();
        for _ in 0..32 {
            menu.update(&Input::default(), &mut options);
        }
        // LEFT changes lives before cancel selects Exit; confirm then exits.
        assert!(matches!(
            menu.update(&Input::from_buttons((1 << 6) | 2 | 1, 0), &mut options),
            OptionsAction::Back
        ));
        assert_eq!(options.life_count, 1);
    }

    #[test]
    fn options_music_requests_preserve_sound_interleaving_and_silent_defaults() {
        use crate::options::{Options, OptionsAudio, OptionsMenu};
        let art = MenuArt {
            background: 0,
            vms: BTreeMap::new(),
            sounds: Vec::new(),
            latin: LatinFont::default(),
        };
        let mut menu = OptionsMenu::new(art);
        menu.midi_available = true;
        menu.begin();
        let mut options = Options::default();
        for _ in 0..32 {
            menu.update(&Input::default(), &mut options);
        }
        for _ in 0..3 {
            menu.update(&Input::from_buttons(1 << 5, 0), &mut options);
            menu.update(&Input::default(), &mut options);
            menu.take_audio_events();
        }
        menu.update(&Input::from_buttons((1 << 6) | (1 << 7), 0), &mut options);
        assert_eq!(
            menu.take_audio_events(),
            [
                OptionsAudio::Sound("select00"),
                OptionsAudio::Music(0),
                OptionsAudio::Sound("select00"),
                OptionsAudio::Music(1)
            ]
        );
        for _ in 0..3 {
            menu.update(&Input::from_buttons(1 << 5, 0), &mut options);
            menu.update(&Input::default(), &mut options);
            menu.take_audio_events();
        }
        for _ in 0..2 {
            menu.update(&Input::from_buttons(1, 0), &mut options);
            assert_eq!(menu.take_audio_events(), [OptionsAudio::Music(1)]);
            menu.update(&Input::default(), &mut options);
        }
    }

    #[test]
    fn title_waits_sixty_frames_before_loading_difficulty() {
        use crate::title::{Title, TitleAction};
        let art = MenuArt {
            background: 0,
            vms: BTreeMap::new(),
            sounds: Vec::new(),
            latin: LatinFont::default(),
        };
        let mut title = Title::new(art);
        // The first confirm dismisses PRE_INPUT. Selection begins only after
        // the active title menu's separate twenty-frame input gate.
        assert_eq!(
            title.update(&Input::from_buttons(1, 0)).1,
            TitleAction::None
        );
        for _ in 0..20 {
            title.update(&Input::default());
        }
        assert_eq!(
            title.update(&Input::from_buttons(1, 0)).1,
            TitleAction::None
        );
        for _ in 0..59 {
            assert_eq!(title.update(&Input::default()).1, TitleAction::None);
        }
        assert_eq!(title.update(&Input::default()).1, TitleAction::StartGame);
        assert_eq!(title.take_sounds(), ["ok00"]);
    }
}
