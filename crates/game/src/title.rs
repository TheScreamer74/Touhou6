//! Title screen: ANM-driven menu with keyboard navigation.
//!
//! title01.anm scripts 0-7 are the main menu items (Start, Extra Start,
//! Practice Start, Replay, Score, Music Room, Option, Quit); 8+ belong to
//! the option submenu. Interrupt 2 slides the main menu in, 3 slides it
//! out / the option menu in.

use std::collections::HashMap;

use th06_engine::{DrawCmd, Input, Key};
use th06_formats::anm0::{Entry, Sprite};

use crate::anm_vm::AnmRunner;

const MAIN_ITEMS: usize = 8;
const START_ITEM: usize = 0;
const QUIT_ITEM: usize = 7;

#[derive(PartialEq)]
pub enum TitleAction {
    None,
    StartGame,
    Quit,
}

pub struct Title {
    runners: Vec<AnmRunner>,
    sprites: HashMap<u32, Sprite>,
    tex_size: [f32; 2],
    bg_tex: usize,
    menu_tex: usize,
    cursor: usize,
    quitting: Option<u16>,
}

impl Title {
    pub fn new(entry: &Entry, bg_tex: usize, menu_tex: usize) -> Self {
        let mut runners: Vec<AnmRunner> = entry
            .scripts
            .iter()
            .map(|(_, instrs)| AnmRunner::new(instrs.clone()))
            .collect();
        // The game fires interrupt 2 when the title menu appears.
        for r in &mut runners {
            r.interrupt(2);
        }
        Self {
            runners,
            sprites: entry.sprites.iter().map(|s| (s.index, s.clone())).collect(),
            tex_size: [entry.width as f32, entry.height as f32],
            bg_tex,
            menu_tex,
            cursor: 0,
            quitting: None,
        }
    }

    pub fn update(&mut self, input: &Input) -> (Vec<DrawCmd>, TitleAction) {
        let mut action = TitleAction::None;
        if self.quitting.is_none() {
            if input.pressed(Key::Up) {
                self.cursor = (self.cursor + MAIN_ITEMS - 1) % MAIN_ITEMS;
            }
            if input.pressed(Key::Down) {
                self.cursor = (self.cursor + 1) % MAIN_ITEMS;
            }
            if input.pressed(Key::Bomb) || input.pressed(Key::Pause) {
                if self.cursor == QUIT_ITEM {
                    self.start_quit();
                } else {
                    self.cursor = QUIT_ITEM;
                }
            }
            if input.pressed(Key::Shoot) || input.pressed(Key::Enter) {
                match self.cursor {
                    START_ITEM => action = TitleAction::StartGame,
                    QUIT_ITEM => self.start_quit(),
                    _ => {} // other entries not implemented yet
                }
            }
        }

        for r in &mut self.runners {
            r.tick();
        }

        if let Some(frames) = &mut self.quitting {
            *frames -= 1;
            if *frames == 0 {
                action = TitleAction::Quit;
            }
        }

        (self.draw(), action)
    }

    /// Re-arm the entrance animation (returning from a game run).
    pub fn reset(&mut self) {
        for r in &mut self.runners {
            r.interrupt(2);
        }
        self.cursor = 0;
        self.quitting = None;
    }

    fn start_quit(&mut self) {
        // Interrupt 3 plays the slide-out animation; quit when it is done.
        for r in &mut self.runners {
            r.interrupt(3);
        }
        self.quitting = Some(40);
    }

    fn draw(&self) -> Vec<DrawCmd> {
        let mut cmds = vec![DrawCmd {
            tex: self.bg_tex,
            dst: [0.0, 0.0, th06_engine::SCREEN_W as f32, th06_engine::SCREEN_H as f32],
            src: [0.0, 0.0, 1.0, 1.0],
            tint: [1.0, 1.0, 1.0, 1.0],
            rot: 0.0,
            additive: false,
        }];
        let [tw, th] = self.tex_size;
        for (i, r) in self.runners.iter().enumerate() {
            if !r.visible() {
                continue;
            }
            let Some(sp) = r.sprite.and_then(|idx| self.sprites.get(&idx)) else {
                continue;
            };
            let w = sp.width * r.scale[0];
            let h = sp.height * r.scale[1];
            let [mut x, mut y] = r.pos;
            if !r.corner {
                x -= w / 2.0;
                y -= h / 2.0;
            }
            // Selected menu item drawn at full brightness, the rest dimmed
            // (the original darkens unselected entries the same way).
            let lit = i < MAIN_ITEMS && i == self.cursor;
            let c = if i < MAIN_ITEMS && !lit { 0.55 } else { 1.0 };
            cmds.push(DrawCmd {
                tex: self.menu_tex,
                dst: [x, y, w, h],
                src: [sp.x / tw, sp.y / th, (sp.x + sp.width) / tw, (sp.y + sp.height) / th],
                tint: [c, c, c, r.alpha],
                rot: 0.0,
                additive: false,
            });
        }
        cmds
    }
}
