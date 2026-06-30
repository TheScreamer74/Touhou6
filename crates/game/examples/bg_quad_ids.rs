use th06_formats::anm0::Anm0;
use th06_formats::pbg3::Pbg3;
fn main() {
    let dat = std::fs::read(std::env::args().nth(1).unwrap()).unwrap();
    let n: u32 = std::env::args().nth(2).unwrap().parse().unwrap();
    let arc = Pbg3::parse(&dat).unwrap();
    let e = arc.entries.iter().find(|e| e.name == format!("stg{n}bg.anm")).unwrap();
    let anm = Anm0::parse(&arc.extract(e).unwrap()).unwrap();
    let ids: Vec<String> = anm.entries[0].scripts.iter().map(|(id, _)| id.to_string()).collect();
    println!("{}", ids.join(" "));
}
