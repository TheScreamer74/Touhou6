//! Original ANM menu assets, with file-local script and sprite indices.
use crate::anm_vm::AnmRunner;
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
}

/// AsciiManager::DrawStrings: ANM glyph geometry, 14-pixel advance and
/// 16-pixel line height. SP, END and the arrow are sprites, too.
pub struct MenuFont {
    normal: FontSheet,
    selected: FontSheet,
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
        Self { normal, selected }
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
                    });
                }
            }
            pos[0] += 14.0 * scale[0];
        }
    }
}

/// Supervisor.cpp's common 30-frame delay, then eight-frame input repeat.
#[derive(Default)]
pub struct MenuInput {
    previous: u16,
    held_frames: u16,
    repeating: bool,
}
impl MenuInput {
    pub fn tick(&mut self, input: &th06_engine::Input) {
        let buttons = input.buttons();
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
}

pub struct MenuArt {
    pub background: usize,
    vms: BTreeMap<usize, Vm>,
    sounds: Vec<&'static str>,
}

impl MenuArt {
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
                        },
                    );
                }
            }
        }
        Self {
            background,
            vms,
            sounds: Vec::new(),
        }
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
        );
        if let Some(target) = self.vms.get_mut(&dst) {
            target.sprites = sheet.0;
            target.tex = sheet.1;
            target.size = sheet.2;
            target.highlight_tex = sheet.3;
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
        let mut pos = [
            r.pos[0] + r.pos_offset[0] + offset[0],
            r.pos[1] + r.pos_offset[1] + offset[1],
        ];
        if !r.corner {
            pos[0] -= w / 2.0;
            pos[1] -= h / 2.0;
        }
        Some(DrawCmd {
            tex: vm.tex,
            dst: [pos[0], pos[1], w, h],
            src: [
                sp.x / vm.size[0],
                sp.y / vm.size[1],
                (sp.x + sp.width) / vm.size[0],
                (sp.y + sp.height) / vm.size[1],
            ],
            tint: [tint[0], tint[1], tint[2], tint[3] * r.alpha],
            rot: r.rotation,
            additive: r.additive,
            color_add: false,
        })
    }
    pub fn draw(
        &self,
        mut style: impl FnMut(usize) -> Option<([f32; 2], [f32; 4])>,
    ) -> Vec<DrawCmd> {
        let mut cmds = vec![DrawCmd {
            tex: self.background,
            dst: [0.0, 0.0, 640.0, 480.0],
            src: [0.0, 0.0, 1.0, 1.0],
            tint: [1.0; 4],
            rot: 0.0,
            additive: false,
            color_add: false,
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
            let [mut x, mut y] = [
                r.pos[0] + r.pos_offset[0] + offset[0],
                r.pos[1] + r.pos_offset[1] + offset[1],
            ];
            if !r.corner {
                x -= w * 0.5;
                y -= h * 0.5;
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
                tex: if vm.highlighted {
                    vm.highlight_tex.unwrap_or(vm.tex)
                } else {
                    vm.tex
                },
                dst: [x, y, w, h],
                src: [u0, v0, u1, v1],
                tint: [tint[0], tint[1], tint[2], tint[3] * r.alpha],
                rot: r.rotation,
                additive: r.additive,
                color_add: false,
            });
        }
        cmds
    }
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
        };
        art.sound("select00");
        art.sound("cancel00");
        art.reset();
        assert_eq!(art.take_sounds(), ["select00", "cancel00"]);
        assert!(art.take_sounds().is_empty());
    }

    #[test]
    fn title_waits_sixty_frames_before_loading_difficulty() {
        use crate::title::{Title, TitleAction};
        let art = MenuArt {
            background: 0,
            vms: BTreeMap::new(),
            sounds: Vec::new(),
        };
        let mut title = Title::new(art);
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
