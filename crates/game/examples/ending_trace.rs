//! Deterministic END interpreter/capture fixture. It enters an ending directly;
//! it does not claim to verify the preceding story clear or score persistence.
use std::{collections::HashMap, io::Write, path::PathBuf};
use th06::{Character, Difficulty, ending::EndingAssets};
use th06_engine::{Engine, Input, Key};
use th06_formats::pbg3::Pbg3;

fn archive(path: PathBuf) -> HashMap<String, Vec<u8>> {
    let bytes = std::fs::read(path).expect("read supplied archive");
    let archive = Pbg3::parse(&bytes).expect("parse supplied archive");
    archive
        .entries
        .iter()
        .map(|entry| {
            (
                entry.name.clone(),
                archive.extract(entry).expect("extract supplied entry"),
            )
        })
        .collect()
}

fn main() {
    let mut values = std::env::args().skip(1);
    let mut args = Vec::new();
    let mut language = th06::localization::Language::Original;
    while let Some(value) = values.next() {
        if value == "--language" {
            let code = values.next().expect("--language <ja|fr|en|de|nl>");
            language = match code.as_str() {
                "ja" => th06::localization::Language::Original,
                "fr" => th06::localization::Language::French,
                "en" => th06::localization::Language::English,
                "de" => th06::localization::Language::German,
                "nl" => th06::localization::Language::Dutch,
                _ => panic!("invalid language: {code}"),
            };
        } else {
            args.push(value);
        }
    }
    assert_eq!(
        args.len(),
        8,
        "ending_trace <game-dir> <out-dir> <shot0..3> <difficulty0..3> <retries> <seen0|1> <plain|confirm|ctrl> <max-updates> [--language ja|fr|en|de|nl]"
    );
    let game = PathBuf::from(&args[0]);
    let output = PathBuf::from(&args[1]);
    std::fs::create_dir_all(&output).expect("create fixture output");
    // A failed or budget-limited rerun must never inherit an old @z receipt
    // or screenshots from a longer successful run in the same directory.
    let terminal = output.join("terminal.txt");
    if terminal.exists() {
        std::fs::remove_file(terminal).expect("remove previous terminal");
    }
    for entry in std::fs::read_dir(&output).expect("read fixture output") {
        let entry = entry.unwrap();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name
            .strip_prefix("tick_")
            .and_then(|name| name.strip_suffix(".png"))
            .is_some_and(|number| !number.is_empty() && number.bytes().all(|b| b.is_ascii_digit()))
        {
            std::fs::remove_file(entry.path()).expect("remove previous screenshot");
        }
    }
    let character = [
        Character::ReimuA,
        Character::ReimuB,
        Character::MarisaA,
        Character::MarisaB,
    ][args[2].parse::<usize>().unwrap()];
    let difficulty = Difficulty::ALL[args[3].parse::<usize>().unwrap()];
    let retries = args[4].parse().unwrap();
    let seen = args[5] == "1";
    let held: Vec<Key> = match args[6].as_str() {
        "plain" => vec![],
        "confirm" => vec![Key::Shoot],
        "ctrl" => vec![Key::Shoot, Key::Skip],
        _ => panic!("unknown ending controls"),
    };
    let frames = args[7].parse::<u32>().unwrap();
    let engine = Engine::new();
    th06_engine::set_color_16bit(false);
    let ed = archive(game.join("ED.DAT"));
    let inn = archive(game.join("IN.DAT"));
    th06::localization::set_language(language);
    th06::localization::begin_textures();
    let mut textures = Vec::new();
    let assets = EndingAssets::load(&engine, &ed, &inn["text.anm"], &mut textures).unwrap();
    let mut ending = assets.start(character, difficulty, retries, seen).unwrap();
    let mut states =
        std::io::BufWriter::new(std::fs::File::create(output.join("state.csv")).unwrap());
    let mut events =
        std::io::BufWriter::new(std::fs::File::create(output.join("events.csv")).unwrap());
    writeln!(states,"tick,file_prefix_fnv,pc,has_seen,script_time,pause,clear,minimum_pause,minimum_clear,long_delay,short_delay,text_line,text_color,background_y,scroll_speed,fade_kind,fade_timer,fade_duration").unwrap();
    writeln!(events, "tick,event,value").unwrap();
    for tick in 1..=frames {
        // Original main.cpp runs the draw chain before the calculation chain.
        let draw = ending.draw();
        th06::localization::refresh(&draw);
        // Supervisor::OnUpdate clears both input masks after registering a
        // new scene. Its first END calculation therefore receives no keys,
        // even when Ctrl/Z are physically held throughout the transition.
        let input = if tick == 1 {
            Input::synthetic(&[], &[])
        } else {
            Input::synthetic(&held, if tick == 2 { &held } else { &[] })
        };
        for event in ending.update(&input).unwrap() {
            use th06::ending::EndingEvent;
            match event {
                EndingEvent::Music(name) => writeln!(events, "{tick},music,{name}").unwrap(),
                EndingEvent::FadeMusic(seconds) => {
                    writeln!(events, "{tick},fade,{seconds}").unwrap()
                }
            }
        }
        if ending.finished() {
            std::fs::write(output.join("terminal.txt"), format!("{tick}\n")).unwrap();
            states.flush().unwrap();
            events.flush().unwrap();
            println!("ending reached @z at update {tick}");
            return;
        }
        writeln!(states, "{}", ending.reference_state(tick)).unwrap();
        if tick % 600 == 1 {
            engine.apply_texture_replacements(&mut textures);
            let references = textures.iter().collect::<Vec<_>>();
            let pixels = engine.render_to_image(&draw, &references, None);
            image::save_buffer(
                output.join(format!("tick_{tick:06}.png")),
                &pixels,
                640,
                480,
                image::ColorType::Rgba8,
            )
            .unwrap();
        }
    }
    panic!("ending never reached its required terminal within {frames} updates");
}
