use th06_formats::anm0::Anm0;
use th06_formats::pbg3::Pbg3;
fn main() {
    let data = std::fs::read(&std::env::args().nth(1).unwrap()).unwrap();
    let arc = Pbg3::parse(&data).unwrap();
    let e = arc.entries.iter().find(|e| e.name == "etama3.anm").unwrap();
    let anm = Anm0::parse(&arc.extract(e).unwrap()).unwrap();
    let bases = [14u32,30,46,62,78,94,110,118,122,146];
    let names = ["pellet","ring","rice","ball","kunai","shard","bigball","fire","dagger","laser"];
    for ent in &anm.entries {
        for (t,&b) in bases.iter().enumerate() {
            if let Some(sp) = ent.sprites.iter().find(|s| s.index == b) {
                println!("type {t} ({}) base {b}: w={} h={}", names[t], sp.width, sp.height);
            }
        }
    }
}
