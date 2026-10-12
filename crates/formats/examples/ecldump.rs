use th06_formats::ecl::Ecl;

fn main() {
    let path = std::env::args().nth(1).expect("usage: ecldump <file.ecl>");
    let data = std::fs::read(&path).expect("read");
    let ecl = Ecl::parse(data).expect("parse");
    println!(
        "subs: {}  timeline at 0x{:x}",
        ecl.sub_offsets.len(),
        ecl.timeline_offset
    );

    println!("-- timeline (first 25) --");
    let mut off = ecl.timeline_offset;
    for _ in 0..25 {
        let Some(t) = ecl.timeline_at(off) else { break };
        if t.time < 0 {
            println!("  end");
            break;
        }
        print!(
            "  t={:5} op={:2} arg0={:4} size={}",
            t.time, t.opcode, t.arg0, t.size
        );
        if t.opcode <= 7 && t.args.len() >= 12 {
            print!(
                "  pos=({}, {}, {})",
                t.arg_f32(0),
                t.arg_f32(4),
                t.arg_f32(8)
            );
            if t.args.len() >= 20 {
                print!(
                    " life={} item={} score={}",
                    t.arg_u16(12),
                    t.arg_u16(14) as i16,
                    t.arg_i32(16)
                );
            }
        }
        println!();
        off += t.size as u32;
    }

    println!("-- sub 0 (first 20 instrs) --");
    let mut off = ecl.sub_offsets[0];
    for _ in 0..20 {
        let Some(i) = ecl.instr_at(off) else { break };
        println!(
            "  t={:5} op={:3} next={:3} skip={:08b} args={}b",
            i.time,
            i.opcode,
            i.offset_to_next,
            i.skip_for_difficulty,
            i.args.len()
        );
        if i.offset_to_next <= 0 {
            break;
        }
        off += i.offset_to_next as u32;
    }
}
