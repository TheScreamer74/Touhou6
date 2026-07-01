// Dump the port's per-frame bg state (camera/facing/fog) in the bg-state oracle
// format, to diff against /tmp/oracle_bg. Usage: bg_dump <ST.DAT> <stageN> <frames>
use th06::background::Background;
use th06_formats::anm0::Anm0;
use th06_formats::pbg3::Pbg3;
use th06_formats::std::Std;

fn main() {
    let dat = std::fs::read(std::env::args().nth(1).unwrap()).unwrap();
    let n: u32 = std::env::args().nth(2).unwrap().parse().unwrap();
    let frames: u32 = std::env::args().nth(3).unwrap().parse().unwrap();
    let arc = Pbg3::parse(&dat).unwrap();
    let get = |name: String| {
        let e = arc.entries.iter().find(|e| e.name == name).expect("entry");
        arc.extract(e).unwrap()
    };
    let std = Std::parse(&get(format!("stage{n}.std"))).unwrap();
    let bg = Anm0::parse(&get(format!("stg{n}bg.anm"))).unwrap();
    let mut background = Background::new(std, &bg.entries[0], 0);
    // Optional STDUNPAUSE injection (emulates ECL op125), matching the oracle's
    // UNPAUSE_FRAMES env, to diff the pause mechanism.
    let unpause: Vec<u32> = std::env::var("UNPAUSE_FRAMES")
        .ok()
        .map(|s| s.split(',').filter_map(|x| x.trim().parse().ok()).collect())
        .unwrap_or_default();
    for f in 0..frames {
        if unpause.contains(&f) {
            background.unpause();
        }
        background.tick();
        let (p, f, c, near, far) = background.dbg_state();
        println!(
            "{:.3} {:.3} {:.3} {:.5} {:.5} {:.5} {:08x} {:.2} {:.2}",
            p[0], p[1], p[2], f[0], f[1], f[2], c, near, far
        );
    }
}
