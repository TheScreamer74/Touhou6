// Vertex-oracle port side: build the bg for stage N, tick to <frame>, and emit
//   - <quadlist>: "F <frame> FACE <fx fy fz>" + one "anmScript px py pz sx sy"
//     per drawn quad (the raw inputs the decomp's Draw3 needs; STD parse shared).
//   - stdout: the port's projected screen corners (tl,tr,br,bl) per quad.
// Diff stdout vs /tmp/oracle_vtx (oracle/vtx) to check the transform path.
// Usage: bg_vtx_dump <ST.DAT> <stageN> <frame> <quadlist_out>
use th06::background::Background;
use th06_formats::anm0::Anm0;
use th06_formats::pbg3::Pbg3;
use th06_formats::std::Std;

fn main() {
    let dat = std::fs::read(std::env::args().nth(1).unwrap()).unwrap();
    let n: u32 = std::env::args().nth(2).unwrap().parse().unwrap();
    let frame: u32 = std::env::args().nth(3).unwrap().parse().unwrap();
    let quadlist_out = std::env::args().nth(4).unwrap();
    let arc = Pbg3::parse(&dat).unwrap();
    let get = |name: String| {
        let e = arc.entries.iter().find(|e| e.name == name).expect("entry");
        arc.extract(e).unwrap()
    };
    let std = Std::parse(&get(format!("stage{n}.std"))).unwrap();
    let bg = Anm0::parse(&get(format!("stg{n}bg.anm"))).unwrap();
    let mut background = Background::new(std, &bg.entries[0], 0);
    for _ in 0..frame {
        background.tick();
    }
    let (facing, quads) = background.dbg_quad_geom();

    let mut ql = format!(
        "F {} FACE {:.5} {:.5} {:.5}\n",
        frame, facing[0], facing[1], facing[2]
    );
    for (anm, pos, size, corners, min_w) in &quads {
        ql.push_str(&format!(
            "{} {:.4} {:.4} {:.4} {:.4} {:.4}\n",
            anm, pos[0], pos[1], pos[2], size[0], size[1]
        ));
        let c = corners;
        // 9th column = min clip-w (near-plane guard; the comparator skips quads
        // with a corner at/behind the camera, where screen projection explodes).
        println!(
            "{:.2} {:.2} {:.2} {:.2} {:.2} {:.2} {:.2} {:.2} {:.3}",
            c[0][0], c[0][1], c[1][0], c[1][1], c[2][0], c[2][1], c[3][0], c[3][1], min_w
        );
    }
    std::fs::write(&quadlist_out, ql).unwrap();
}
