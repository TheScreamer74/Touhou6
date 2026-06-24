use th06_formats::anm0::Anm0;
use th06_formats::pbg3::Pbg3;
fn main() {
    let args: Vec<String> = std::env::args().collect();
    let data = std::fs::read(&args[1]).expect("read CM.DAT");
    let arc = Pbg3::parse(&data).expect("pbg3");
    let e = arc.entries.iter().find(|e| e.name == "etama3.anm").expect("etama3");
    let bytes = arc.extract(e).expect("extract");
    let anm = Anm0::parse(&bytes).expect("anm");
    for ent in &anm.entries {
        for (id, instrs) in &ent.scripts {
            if (11..=20).contains(id) {
                // duration = time of the script-end (opcode 0) instr, else max time
                let end = instrs.iter().find(|i| i.opcode == 0).map(|i| i.time)
                    .unwrap_or_else(|| instrs.iter().map(|i| i.time).max().unwrap_or(0));
                println!("script {id}: dur={end} ({} instrs)", instrs.len());
            }
        }
    }
}
