// Dump the port BgQuadVm state for one bg anm script, oracle_anm format.
// Usage: bg_quad_dump <ST.DAT> <stageN> <scriptId> <frames>
use std::collections::HashMap;
use th06::background::dbg_quad_run;
use th06_formats::anm0::Anm0;
use th06_formats::pbg3::Pbg3;

fn main() {
    let dat = std::fs::read(std::env::args().nth(1).unwrap()).unwrap();
    let n: u32 = std::env::args().nth(2).unwrap().parse().unwrap();
    let sid: u32 = std::env::args().nth(3).unwrap().parse().unwrap();
    let frames: u32 = std::env::args().nth(4).unwrap().parse().unwrap();
    let arc = Pbg3::parse(&dat).unwrap();
    let e = arc.entries.iter().find(|e| e.name == format!("stg{n}bg.anm")).unwrap();
    let anm = Anm0::parse(&arc.extract(e).unwrap()).unwrap();
    let ent = &anm.entries[0];
    let sprites: HashMap<u32, [f32; 2]> =
        ent.sprites.iter().map(|s| (s.index, [s.width, s.height])).collect();
    let (_, instrs) = ent.scripts.iter().find(|(id, _)| *id == sid).expect("script");
    for line in dbg_quad_run(instrs.clone(), &sprites, frames) {
        println!("{line}");
    }
}
