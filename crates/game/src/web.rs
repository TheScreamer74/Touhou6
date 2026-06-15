//! Web entry point. The browser uploads the player's own game folder; the
//! bytes never leave their machine. JS hands us a `{ filename: Uint8Array }`
//! object, we extract the PBG3 archives in-memory and run the game on a
//! canvas. No files are bundled, fetched, or served.

use std::collections::HashMap;

use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::HtmlCanvasElement;

use th06_engine::{Engine, SCREEN_H, SCREEN_W};
use th06_formats::pbg3::Pbg3;

use crate::{build_game, GameFiles};

/// Extract every entry of an in-memory PBG3 archive, keyed by entry name.
fn extract(raw: &[u8]) -> HashMap<String, Vec<u8>> {
    let archive = Pbg3::parse(raw).expect("parse PBG3");
    archive
        .entries
        .iter()
        .map(|e| (e.name.clone(), archive.extract(e).expect("extract")))
        .collect()
}

/// Entry point invoked from JS once the player has selected their game
/// folder. `files` is a plain object mapping each file's basename to a
/// `Uint8Array` of its bytes. Returns a Promise (async).
#[wasm_bindgen]
pub async fn start_game(files: js_sys::Object) {
    console_error_panic_hook::set_once();

    // Pull every uploaded file into a name -> bytes map.
    let mut raw: HashMap<String, Vec<u8>> = HashMap::new();
    for entry in js_sys::Object::entries(&files).iter() {
        let pair: js_sys::Array = entry.into();
        let Some(name) = pair.get(0).as_string() else { continue };
        let bytes = js_sys::Uint8Array::new(&pair.get(1)).to_vec();
        raw.insert(name, bytes);
    }

    let get = |n: &str| raw.get(n).cloned().unwrap_or_default();
    // Loose BGM wavs (th06_0N.wav) sit beside the archives, not inside them.
    let bgm: HashMap<String, Vec<u8>> = raw
        .iter()
        .filter(|(k, _)| k.starts_with("th06_") && k.ends_with(".wav"))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();

    let game_files = GameFiles {
        tl: extract(&get("TL.DAT")),
        cm: extract(&get("CM.DAT")),
        st: extract(&get("ST.DAT")),
        inn: extract(&get("IN.DAT")),
        st_en: extract(&get("th06e_ST.DAT")),
        bgm,
    };

    // Create the rendering canvas up front: WebGL needs it to exist before
    // the GPU adapter is requested.
    let document = web_sys::window().expect("window").document().expect("document");
    let canvas: HtmlCanvasElement = document
        .create_element("canvas")
        .expect("create canvas")
        .dyn_into()
        .expect("canvas element");
    canvas.set_width(SCREEN_W);
    canvas.set_height(SCREEN_H);
    canvas.set_id("th06-canvas");
    canvas.set_tab_index(0); // focusable, so it receives keyboard input
    document
        .body()
        .expect("body")
        .append_child(&canvas)
        .expect("append canvas");
    let _ = canvas.focus();

    let (engine, surface) = Engine::new_web(canvas.clone()).await;
    let (textures, mut game) = build_game(&engine, &game_files, true);
    game.start_title_bgm();
    engine.run_game_web(canvas, surface, "Touhou 6 ~ EoSD", textures, move |input| game.update(input));
}
