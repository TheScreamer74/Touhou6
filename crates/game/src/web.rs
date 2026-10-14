//! Web entry point. The hosting page supplies a `{ filename: Uint8Array }`
//! object. Archives are extracted in memory and the game runs on a canvas.

use std::collections::HashMap;

use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use web_sys::HtmlCanvasElement;

use th06_engine::{Engine, SCREEN_H, SCREEN_W};
use th06_formats::pbg3::Pbg3;

use crate::{GameFiles, build_game};
use std::cell::RefCell;
thread_local! {
    static IMPORTS: RefCell<Vec<Vec<u8>>> = const {RefCell::new(Vec::new())};
    static REPLAYS_DIRTY: std::cell::Cell<bool> = const {std::cell::Cell::new(false)};
}

#[wasm_bindgen]
pub fn set_suspended(value: bool) {
    th06_engine::set_suspended(value);
}

#[wasm_bindgen]
pub fn import_score(bytes: &[u8]) -> Result<(), JsValue> {
    th06_formats::score::ScoreData::parse(bytes).map_err(|e| JsValue::from_str(&e))?;
    crate::save::store_bytes("score.dat", bytes).map_err(|e| JsValue::from_str(&e))?;
    IMPORTS.with(|queue| queue.borrow_mut().push(bytes.to_vec()));
    Ok(())
}
#[wasm_bindgen]
pub fn export_score() -> Vec<u8> {
    crate::save::load_bytes("score.dat")
        .unwrap_or_else(|| th06_formats::score::ScoreData::default().encode())
}
#[wasm_bindgen]
pub fn import_replay(bytes: &[u8], slot: u32) -> Result<(), JsValue> {
    let replay = th06_formats::replay::Replay::parse(bytes).map_err(|e| JsValue::from_str(&e))?;
    if slot >= crate::replay::SLOTS as u32 {
        return Err(JsValue::from_str("Invalid replay slot"));
    }
    crate::save::store_bytes(
        &crate::replay::path("replay", slot as usize),
        &replay.encode(),
    )
    .map_err(|e| JsValue::from_str(&e))?;
    REPLAYS_DIRTY.with(|flag| flag.set(true));
    Ok(())
}
#[wasm_bindgen]
pub fn export_replay(slot: u32) -> Result<Vec<u8>, JsValue> {
    if slot >= crate::replay::SLOTS as u32 {
        return Err(JsValue::from_str("Invalid replay slot"));
    }
    crate::save::load_bytes(&crate::replay::path("replay", slot as usize))
        .ok_or_else(|| JsValue::from_str("This replay slot is empty"))
}

/// Extract every entry of an in-memory PBG3 archive, keyed by entry name.
/// Returns an error (rather than panicking) so a wrong/corrupt upload surfaces
/// as a visible message instead of a blank canvas.
fn extract(raw: &[u8]) -> Result<HashMap<String, Vec<u8>>, String> {
    let archive = Pbg3::parse(raw).map_err(|_| "not a valid PBG3 (.DAT) archive".to_string())?;
    let mut out = HashMap::new();
    for e in &archive.entries {
        let bytes = archive
            .extract(e)
            .ok_or_else(|| format!("failed to extract {}", e.name))?;
        out.insert(e.name.clone(), bytes);
    }
    Ok(out)
}

/// Entry point invoked once the hosting page has loaded the game resources.
/// `files` is a plain object mapping each file's basename to a
/// `Uint8Array` of its bytes. Returns a Promise (async).
#[wasm_bindgen]
pub async fn start_game(files: js_sys::Object) -> Result<(), JsValue> {
    console_error_panic_hook::set_once();

    // Pull every uploaded file into a name -> bytes map.
    let mut raw: HashMap<String, Vec<u8>> = HashMap::new();
    for entry in js_sys::Object::entries(&files).iter() {
        let pair: js_sys::Array = entry.into();
        let Some(name) = pair.get(0).as_string() else {
            continue;
        };
        let bytes = js_sys::Uint8Array::new(&pair.get(1)).to_vec();
        raw.insert(
            name.rsplit(['/', '\\'])
                .next()
                .unwrap_or(&name)
                .to_ascii_lowercase(),
            bytes,
        );
    }

    // Extract a required archive, or fail with a readable message the page
    // shows — a wrong/partial/incompatible folder would otherwise panic deep in
    // build_game and leave a blank canvas.
    let need = |n: &str| -> Result<HashMap<String, Vec<u8>>, JsValue> {
        let bytes = raw
            .get(&n.to_ascii_lowercase())
            .filter(|b| !b.is_empty())
            .ok_or_else(|| {
                JsValue::from_str(&format!(
                    "Missing required file: {n}. Select the th06 folder containing \
                 TL.DAT, CM.DAT, ST.DAT, IN.DAT and bgm/."
                ))
            })?;
        extract(bytes).map_err(|e| JsValue::from_str(&format!("{n}: {e}")))
    };

    // `.pos` loop points, if the player uploaded them alongside the wavs.
    let mut bgm_pos: HashMap<String, (u32, u32)> = raw
        .iter()
        .filter(|(k, v)| k.starts_with("th06_") && k.ends_with(".pos") && v.len() >= 8)
        .map(|(k, v)| {
            let s = u32::from_le_bytes([v[0], v[1], v[2], v[3]]);
            let e = u32::from_le_bytes([v[4], v[5], v[6], v[7]]);
            (k.replace(".pos", ".wav"), (s, e))
        })
        .collect();

    let optional = |name: &str| {
        raw.get(&name.to_ascii_lowercase())
            .map(|b| extract(b))
            .transpose()
            .map_err(|e| JsValue::from_str(&format!("{name}: {e}")))
            .map(Option::unwrap_or_default)
    };
    let md = optional("MD.DAT")?;
    let inn = need("IN.DAT")?;
    for (name, bytes) in md
        .iter()
        .chain(inn.iter())
        .filter(|(n, b)| n.ends_with(".pos") && b.len() >= 8)
    {
        bgm_pos
            .entry(name.replace(".pos", ".wav"))
            .or_insert_with(|| {
                (
                    u32::from_le_bytes(bytes[..4].try_into().unwrap()),
                    u32::from_le_bytes(bytes[4..8].try_into().unwrap()),
                )
            });
    }
    let tl = need("TL.DAT")?;
    let cm = need("CM.DAT")?;
    let st = need("ST.DAT")?;
    let st_en = optional("th06e_ST.DAT")?;
    let ed = optional("ED.DAT")?;
    let music_comments = raw
        .get("musiccmt.txt")
        .or_else(|| md.get("musiccmt.txt"))
        .or_else(|| inn.get("musiccmt.txt"))
        .cloned()
        .unwrap_or_default();
    let bgm_keys: Vec<String> = raw
        .keys()
        .filter(|k| k.starts_with("th06_") && (k.ends_with(".wav") || k.ends_with(".ogg")))
        .cloned()
        .collect();
    let bgm = bgm_keys
        .into_iter()
        .map(|key| {
            let bytes = raw.remove(&key).unwrap();
            (key.replace(".ogg", ".wav"), bytes)
        })
        .collect();
    let game_files = GameFiles {
        tl,
        cm,
        st,
        inn,
        st_en,
        ed,
        music_comments,
        bgm,
        bgm_pos,
        midi: md
            .iter()
            .filter(|(name, _)| name.ends_with(".mid"))
            .map(|(name, bytes)| (name.clone(), bytes.clone()))
            .collect(),
        soundfont: raw.remove("gm.sf2"),
        text_font: raw
            .remove("msgothic.ttc")
            .or_else(|| raw.remove("msgothic.ttf")),
    };

    crate::validate_game_files(&game_files).map_err(|e| JsValue::from_str(&e))?;

    // Create the rendering canvas up front: WebGL needs it to exist before
    // the GPU adapter is requested.
    let document = web_sys::window()
        .expect("window")
        .document()
        .expect("document");
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

    let (engine, surface) = match Engine::new_web(canvas.clone()).await {
        Ok(result) => result,
        Err(error) => {
            canvas.remove();
            return Err(JsValue::from_str(&error));
        }
    };
    let initial_options = crate::save::load_bytes("th06-options.bin")
        .and_then(|bytes| crate::options::Options::parse(&bytes))
        .unwrap_or_else(|| crate::options::Options {
            language: crate::localization::Language::French,
            ..Default::default()
        });
    crate::localization::set_language(initial_options.language);
    let (textures, mut game) = build_game(&engine, &game_files, true);
    game.options = initial_options;
    game.configure_storage("");
    if crate::save::load_bytes("score.dat").is_none()
        && let Some(bytes) = raw.get("score.dat")
    {
        game.import_progress(bytes)
            .map_err(|e| JsValue::from_str(&e))?;
    }
    for slot in 0..crate::replay::SLOTS {
        let path = crate::replay::path("replay", slot);
        if crate::save::load_bytes(&path).is_none()
            && let Some(bytes) = raw.get(&format!("th6_{:02}.rpy", slot + 1))
        {
            game.import_replay(bytes, slot)
                .map_err(|e| JsValue::from_str(&e))?;
        }
    }
    drop(game_files);
    drop(raw);
    game.prepare_audio().await;
    game.start_title_bgm();
    let mut displayed_error = None;
    let mut displayed_language = None;
    let display_canvas = canvas.clone();
    engine.run_game_web(canvas, surface, "Touhou 6 ~ EoSD", textures, move |input| {
        IMPORTS.with(|queue| {
            for bytes in queue.borrow_mut().drain(..) {
                if let Ok(progress) = th06_formats::score::ScoreData::parse(&bytes) {
                    game.set_progress(progress);
                }
            }
        });
        if REPLAYS_DIRTY.with(|flag| flag.replace(false)) {
            game.replay_slots = crate::replay::load_slots(&game.replay_directory);
        }
        game.options.windowed = document.fullscreen_element().is_none();
        let frame = game.update(input);
        if displayed_language != Some(game.options.language) {
            displayed_language = Some(game.options.language);
            if let Some(window) = web_sys::window() {
                let code = JsValue::from_str(game.options.language.code());
                let _ = js_sys::Reflect::set(window.as_ref(), &JsValue::from_str("__th06GameLanguage"), &code);
                if let Ok(hook) = js_sys::Reflect::get(window.as_ref(), &JsValue::from_str("__th06OnLanguageChanged"))
                    .and_then(|value| value.dyn_into::<js_sys::Function>()) {
                    let _ = hook.call1(&JsValue::NULL, &code);
                }
            }
        }
        if let Some(window) = web_sys::window()
            && let Ok(hook) = js_sys::Reflect::get(window.as_ref(), &JsValue::from_str("__th06SubmitScore"))
                .and_then(|value| value.dyn_into::<js_sys::Function>()) {
            for result in game.take_community_results() {
                let _ = hook.call1(&JsValue::NULL, &JsValue::from_str(&result.to_string()));
            }
        }
        if let Some(windowed) = th06_engine::take_windowed_request() {
            let promise = if windowed {
                document.exit_fullscreen();
                None
            } else {
                Some(js_sys::Reflect::get(display_canvas.as_ref(), &JsValue::from_str("requestFullscreen"))
                    .and_then(|function| function.dyn_into::<js_sys::Function>())
                    .and_then(|function| function.call0(display_canvas.as_ref()))
                    .map(|value| js_sys::Promise::resolve(&value))
                    .unwrap_or_else(|error| js_sys::Promise::reject(&error)))
            };
            if let Some(promise) = promise {
                wasm_bindgen_futures::spawn_local(async move {
                    if wasm_bindgen_futures::JsFuture::from(promise).await.is_err()
                        && let Some(element) = web_sys::window()
                            .and_then(|window| window.document())
                            .and_then(|document| document.get_element_by_id("persistenceError")) {
                                element.set_text_content(Some("Le navigateur a refusé le plein écran. Utilisez le bouton Plein écran ou la touche F."));
                            }
                });
            }
        }
        if displayed_error != game.persistence_error {
            displayed_error.clone_from(&game.persistence_error);
            if let Some(element) = document.get_element_by_id("persistenceError") {
                element.set_text_content(displayed_error.as_ref().map(|error| {
                    format!("Sauvegarde impossible : {error}. Exportez vos données pour les conserver.")
                }).as_deref());
            }
        }
        frame
    });
    Ok(())
}
