// Render sample JP strings to PNGs to eyeball the rasteriser.
use th06_engine::text;
fn save(name: &str, s: &str, px: f32) {
    let img = text::render(s, px, [255, 240, 240], Some([0, 0, 0]));
    image::save_buffer(name, &img.rgba, img.width, img.height, image::ColorType::Rgba8).unwrap();
    println!("wrote {name} {}x{}", img.width, img.height);
}
fn main() {
    let out = std::env::args().nth(1).unwrap_or_else(|| ".".into());
    save(&format!("{out}/jp_spell.png"), "月符「ムーンライトレイ」", 28.0);
    save(&format!("{out}/jp_stage.png"), "夢幻夜行絵巻　～ Mystic Flier", 22.0);
    save(&format!("{out}/jp_dialog.png"), "って、あんた誰？", 24.0);
}
