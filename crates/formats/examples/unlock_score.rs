fn main() {
    let mut s = th06_formats::score::ScoreData::default();
    s.stages_cleared = [[99; 5]; 4];
    s.stages_cleared_without_continues = [[99; 5]; 4];
    let out = std::env::args().nth(1).expect("out path");
    std::fs::write(&out, s.encode()).unwrap();
    let back = th06_formats::score::ScoreData::parse(&std::fs::read(&out).unwrap()).unwrap();
    assert_eq!(back.stages_cleared, [[99; 5]; 4]);
    println!("ok {}", std::fs::metadata(&out).unwrap().len());
}
