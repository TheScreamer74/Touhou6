use std::path::PathBuf;

use th06_formats::pbg3::Pbg3;

fn main() {
    let mut args = std::env::args().skip(1);
    let input = args.next().expect("usage: extract <archive.DAT> [out_dir]");
    let out_dir = args.next().map(PathBuf::from);

    let data = std::fs::read(&input).expect("read archive");
    let archive = Pbg3::parse(&data).expect("parse PBG3");

    println!("{} entries:", archive.entries.len());
    for entry in &archive.entries {
        println!("  {:>10}  {}", entry.size, entry.name);
        if let Some(dir) = &out_dir {
            let contents = archive.extract(entry).expect("decompress");
            assert_eq!(contents.len(), entry.size as usize, "{}: size mismatch", entry.name);
            std::fs::create_dir_all(dir).expect("create out dir");
            std::fs::write(dir.join(&entry.name), contents).expect("write file");
        }
    }
}
