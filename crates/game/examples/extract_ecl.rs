use th06_formats::pbg3::Pbg3;
fn main() {
    let args: Vec<String> = std::env::args().collect();
    let dat = &args[1];   // path to ST.DAT
    let name = &args[2];  // e.g. ecldata5.ecl
    let out = &args[3];
    let data = std::fs::read(dat).expect("read dat");
    let arc = Pbg3::parse(&data).expect("parse pbg3");
    let e = arc.entries.iter().find(|e| &e.name == name).expect("entry");
    let bytes = arc.extract(e).expect("extract");
    std::fs::write(out, &bytes).unwrap();
    println!("wrote {} ({} bytes)", out, bytes.len());
}
