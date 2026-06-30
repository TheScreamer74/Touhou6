// Emit a C header mapping each enemy anm GLOBAL script id (ANM_OFFSET_ENEMY 0x100
// + local) to its sprite (w,h) in px, for the ecl-dump oracle's bottom-edge
// despawn test. Both stgNenm.anm and stgNenm2.anm load at offset 0x100.
// Usage: enemy_anm_sizes <ST.DAT> <stage>
use std::collections::HashMap;
use th06_formats::anm0::Anm0;
use th06_formats::pbg3::Pbg3;

fn main() {
    let dat = std::fs::read(std::env::args().nth(1).unwrap()).unwrap();
    let n: u32 = std::env::args().nth(2).unwrap().parse().unwrap();
    let arc = Pbg3::parse(&dat).unwrap();
    let mut sizes: HashMap<u32, (f32, f32)> = HashMap::new();
    for file in [format!("stg{n}enm.anm"), format!("stg{n}enm2.anm")] {
        let Some(e) = arc.entries.iter().find(|e| e.name == file) else { continue };
        let anm = Anm0::parse(&arc.extract(e).unwrap()).unwrap();
        for ent in &anm.entries {
            let spr: HashMap<u32, [f32; 2]> =
                ent.sprites.iter().map(|s| (s.index, [s.width, s.height])).collect();
            for (id, instrs) in &ent.scripts {
                // first op1 (SetActiveSprite) sprite of the script
                if let Some(idx) = instrs.iter().find(|i| i.opcode == 1).map(|i| i.arg_u32(0)) {
                    if let Some(d) = spr.get(&idx) {
                        sizes.insert(0x100 + id, (d[0], d[1]));
                    }
                }
            }
        }
    }
    println!("// generated: enemy anm script id -> sprite (w,h), stage {n}");
    println!("static inline bool anm_enemy_size(int id, float *w, float *h) {{");
    println!("    switch (id) {{");
    let mut ids: Vec<_> = sizes.keys().copied().collect();
    ids.sort();
    for id in ids {
        let (w, h) = sizes[&id];
        println!("    case {id}: *w = {w:.1}f; *h = {h:.1}f; return true;");
    }
    println!("    default: return false;");
    println!("    }}");
    println!("}}");
}
