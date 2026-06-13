use th06_formats::std::Std;
fn main() {
    let path = std::env::args().nth(1).expect("usage: stddump <file.std>");
    let d = std::fs::read(&path).expect("read");
    let s = Std::parse(&d).expect("parse");
    println!("objects={} instances={} script_instrs={}", s.objects.len(), s.instances.len(), s.script.len());
    for (i, o) in s.objects.iter().enumerate().take(6) {
        println!("obj {i}: z={} pos={:?} size={:?} quads={}", o.z_level, o.pos, o.size, o.quads.len());
        for q in o.quads.iter().take(3) {
            println!("    quad anm={} pos={:?} size={:?}", q.anm_script, q.pos, q.size);
        }
    }
    println!("-- instances (first 8) --");
    for inst in s.instances.iter().take(8) {
        println!("  id={} pos={:?}", inst.id, inst.pos);
    }
    println!("-- script (first 12) --");
    for s in s.script.iter().take(12) {
        println!("  frame={} op={} args={:?}", s.frame, s.opcode, s.args);
    }
}
