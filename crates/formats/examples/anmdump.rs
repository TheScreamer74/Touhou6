use th06_formats::anm0::Anm0;

fn main() {
    let path = std::env::args().nth(1).expect("usage: anmdump <file.anm>");
    let data = std::fs::read(&path).expect("read");
    let anm = Anm0::parse(&data).expect("parse");
    for entry in &anm.entries {
        println!(
            "entry: {}x{} fmt={} name={:?} alpha={:?} sprites={} scripts={}",
            entry.width, entry.height, entry.format, entry.name, entry.alpha_name,
            entry.sprites.len(), entry.scripts.len()
        );
        for s in entry.sprites.iter().take(4) {
            println!("  sprite {}: ({}, {}) {}x{}", s.index, s.x, s.y, s.width, s.height);
        }
        for (id, instrs) in entry.scripts.iter() {
            println!("  script {id}:");
            for i in instrs {
                print!("    t={} op={} args=[", i.time, i.opcode);
                for c in i.args.chunks(4) {
                    if c.len() == 4 {
                        let v = u32::from_le_bytes(c.try_into().unwrap());
                        let f = f32::from_bits(v);
                        if f.abs() > 1e-6 && f.abs() < 1e6 {
                            print!("{f} ");
                        } else {
                            print!("{v} ");
                        }
                    }
                }
                println!("]");
            }
        }
    }
}
