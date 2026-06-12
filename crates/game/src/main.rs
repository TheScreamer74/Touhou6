use std::collections::HashMap;
use std::path::Path;

use th06_engine::{compose_rgba, DrawCmd, Engine, SCREEN_H, SCREEN_W};
use th06_formats::anm0::{Anm0, Entry};
use th06_formats::pbg3::Pbg3;

/// All files from one PBG3 archive, keyed by entry name.
fn load_archive(path: &Path) -> HashMap<String, Vec<u8>> {
    let data = std::fs::read(path).expect("read archive");
    let archive = Pbg3::parse(&data).expect("parse PBG3");
    archive
        .entries
        .iter()
        .map(|e| (e.name.clone(), archive.extract(e).expect("extract")))
        .collect()
}

/// ANM texture names look like "data/title/title01.png"; archive entries
/// are flat basenames.
fn basename(path: &str) -> &str {
    path.rsplit('/').next().unwrap()
}

/// Static evaluation of a th06 ANM script to its settled state.
///
/// Scripts halt at "stop" opcodes (21, 24) and resume at the matching
/// interrupt label (22) when the game fires an interrupt — that is how the
/// title menu slides items in. We run to the first halt, then resume past
/// the requested interrupt label and run to the next halt, taking timed
/// moves (18/19/20) at their destination.
struct SpriteState {
    sprite: Option<u32>,
    x: f32,
    y: f32,
}

fn eval_script(instrs: &[th06_formats::anm0::Instr], interrupt: Option<u32>) -> SpriteState {
    let mut st = SpriteState { sprite: None, x: 0.0, y: 0.0 };

    fn run(st: &mut SpriteState, instrs: &[th06_formats::anm0::Instr], from: usize) {
        for i in &instrs[from..] {
            match i.opcode {
                0 => break,
                1 => st.sprite = Some(i.arg_u32(0)),
                17 => {
                    st.x = i.arg_f32(0);
                    st.y = i.arg_f32(1);
                }
                18 | 19 | 20 => {
                    st.x = i.arg_f32(0);
                    st.y = i.arg_f32(1);
                }
                21 | 24 => break,
                _ => {}
            }
        }
    }

    run(&mut st, instrs, 0);
    if let Some(label) = interrupt {
        if let Some(pos) = instrs
            .iter()
            .position(|i| i.opcode == 22 && i.arg_u32(0) == label)
        {
            run(&mut st, instrs, pos + 1);
        }
    }
    st
}

fn title_scene(entry: &Entry, tex_index: usize, interrupt: Option<u32>) -> Vec<DrawCmd> {
    let sprites: HashMap<u32, &th06_formats::anm0::Sprite> =
        entry.sprites.iter().map(|s| (s.index, s)).collect();
    let (tw, th) = (entry.width as f32, entry.height as f32);

    let mut cmds = Vec::new();
    for (_id, instrs) in &entry.scripts {
        let st = eval_script(instrs, interrupt);
        let Some(idx) = st.sprite else { continue };
        let Some(sp) = sprites.get(&idx) else { continue };
        cmds.push(DrawCmd {
            tex: tex_index,
            dst: [st.x, st.y, sp.width, sp.height],
            src: [sp.x / tw, sp.y / th, (sp.x + sp.width) / tw, (sp.y + sp.height) / th],
            tint: [1.0, 1.0, 1.0, 1.0],
        });
    }
    cmds
}

fn main() {
    let mut args = std::env::args().skip(1);
    let mut screenshot: Option<String> = None;
    let mut game_dir = String::from("../TH06 ~ The Embodiment of Scarlet Devil/kouma");
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--screenshot" => screenshot = Some(args.next().expect("--screenshot <out.png>")),
            "--game-dir" => game_dir = args.next().expect("--game-dir <path>"),
            other => panic!("unknown argument: {other}"),
        }
    }

    let tl = load_archive(&Path::new(&game_dir).join("TL.DAT"));

    let anm = Anm0::parse(&tl["title01.anm"]).expect("parse title01.anm");
    let entry = &anm.entries[0];

    let engine = Engine::new();

    // Texture 0: menu background (no alpha mask). Texture 1: menu sprites.
    let (bg_rgba, bg_w, bg_h) = compose_rgba(&tl["title00.jpg"], None);
    let bg_tex = engine.create_texture(&bg_rgba, bg_w, bg_h);
    let alpha = entry.alpha_name.as_deref().map(|n| tl[basename(n)].as_slice());
    let (rgba, w, h) = compose_rgba(&tl[basename(&entry.name)], alpha);
    let menu_tex = engine.create_texture(&rgba, w, h);

    let mut cmds = vec![DrawCmd {
        tex: 0,
        dst: [0.0, 0.0, SCREEN_W as f32, SCREEN_H as f32],
        src: [0.0, 0.0, 1.0, 1.0],
        tint: [1.0, 1.0, 1.0, 1.0],
    }];
    // Interrupt 2 = the game's "main menu entrance" signal.
    cmds.extend(title_scene(entry, 1, Some(2)));

    if let Some(out) = screenshot {
        let textures = [&bg_tex, &menu_tex];
        let pixels = engine.render_to_image(&cmds, &textures);
        image::save_buffer(&out, &pixels, SCREEN_W, SCREEN_H, image::ColorType::Rgba8)
            .expect("save screenshot");
        println!("wrote {out}");
    } else {
        engine.run_window("Touhou 6 ~ EoSD (Mac port)", cmds, vec![bg_tex, menu_tex]);
    }
}
