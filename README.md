<div align="center">

# 東方紅魔郷
## Touhou 6 — the Embodiment of Scarlet Devil, for macOS & the Web

**Rust reimplementation for Apple Silicon and WebAssembly.**
**Reads the original v1.02h game data.**

![Platform](https://img.shields.io/badge/platform-macOS%20%2B%20Web%20(WASM)-black?logo=apple)
![Language](https://img.shields.io/badge/language-Rust-b7410e?logo=rust)
![Renderer](https://img.shields.io/badge/renderer-wgpu%20%2F%20Metal%20%2F%20WebGL2-blueviolet)
![Status](https://img.shields.io/badge/status-7%20stages%20implemented-crimson)

</div>

## Project

The engine reads Touhou Koumakyou ~ the Embodiment of Scarlet Devil v1.02h
archives and interprets their ECL, STD, ANM, MSG and END scripts. The Windows
executable is not run. Supply your own game files for local play, or use a web
host configured with its own asset manifest. Personal browser files are never
uploaded.

Implementation follows the [matching decompilation](https://github.com/GensokyoClub/th06).
[REA](https://github.com/morluto/rea) and Ghidra were also used to inspect the
original executable and record function evidence. This does not establish
whole-game binary or visual equivalence; see [validation](docs/REVERSE_ENGINEERING.md).

## Supported features

| Area | Implemented support |
|---|---|
| Stages | Stages 1–6 and Extra, including enemies, bosses, dialogue and 3D backgrounds |
| Difficulty | Easy, Normal, Hard, Lunatic and Extra; difficulty-specific script masks and rank limits |
| Characters | Reimu A/B and Marisa A/B, their shot tiers and bombs |
| Run flow | Stage progression, pause, continues, game over, Practice and clear results |
| Endings | Six END routes selected by character, difficulty and continues, followed by the staff roll |
| Records | High scores, clear/spell-card records and original `score.dat` persistence |
| Replays | T6RP recording, save/load slots and original replay parsing/playback |
| Menus | Difficulty/character selection, Result, Practice, Replay, Music Room and Options |
| Audio | 17 BGM tracks, loop points and sound effects; WAV playback and web Ogg decoding |
| Input | Keyboard and gamepad, configurable controller mappings |
| Web | Hosted asset manifest or local folder selection, local file cache, fullscreen and score/replay import/export |

The usual six-stage run ends after stage 5 on Easy. Extra has its own clear
flow and unlock records. END artwork and staff animations require `ED.DAT`;
Music Room comments and the original loop markers are supplied by `MD.DAT`.
The translated stage archive `th06e_ST.DAT` is optional.

## Build and play

```sh
git clone https://github.com/TheScreamer74/Touhou6.git
cd Touhou6
cargo run -p th06 --release -- --game-dir "/path/to/th06"
```

Use the v1.02h installation containing `CM.DAT`, `IN.DAT`, `ST.DAT`, `TL.DAT`,
`ED.DAT`, `MD.DAT` and `bgm/th06_01.wav` through `th06_17.wav`. No game data or
Windows executable is included in the source distribution.

For WebAssembly:

```sh
wasm-pack build crates/game --release --target web --out-dir ../../web/pkg
cd web
python3 -m http.server 8080
```

Open `http://localhost:8080`. The [web guide](web/README.md) documents hosted
`assets.json`, engine cache versioning, personal file storage and save exports.

## Controls

| Key | Action |
|---|---|
| Arrows | Move |
| `Z` | Shoot / confirm |
| `Enter` | Confirm menus |
| `X` | Bomb / back |
| `Shift` | Focus |
| `Esc` | Pause / back |
| `Ctrl` | Skip eligible dialogue and previously seen endings |
| `F` | Fullscreen in the browser |

## Validation and limits

Unit and integration checks cover format validation, run selection, persistence
and END command timing. Original ED resources were loaded and all six endings
were exercised through the last staff animation and script termination.
The C++ oracles compare the real decompilation's interpreter calculations with
the Rust port; the [oracle guide](oracle/README.md) identifies the locked source
revision, input settings and comparison precision.

Some gameplay details still differ from the original: player-shot collision
uses simplified hit tests, Marisa A missile explosions and Marisa B beams do
not reproduce every damage rule, and bullet cancellation/graze effects and
laser graze timing remain partial. Replay roundtrips establish the port's own
determinism; playback equivalence to the Windows game has not been established.
The [dated report](docs/REVERSE_ENGINEERING.md) records these limits and the
actual oracle results.

The current renderer uses 32-bit color. Original 16-bit display mode and MIDI
synthesis are unavailable. Japanese text uses the bundled Noto Sans JP font
instead of Windows GDI/MS Gothic, so text rasterization differs. Browser audio,
GPU availability and storage depend on the browser. Focused oracle and binary
checks are evidence for the tested paths, rather than a claim of 100% fidelity
across every character, difficulty, frame and visual effect.

## Architecture

```text
crates/
├── formats   PBG3 · ANM · ECL · STD · MSG · score.dat · T6RP
├── engine    wgpu · fixed timestep · keyboard/gamepad · audio · Japanese text
└── game      interpreters · stages · menus · endings · records · replays
oracle/       native C++ reference comparisons
web/          browser loader and static deployment guide
```

## Credits and legal

This is an unofficial fan project, unaffiliated with Team Shanghai Alice.
Touhou Project and the Embodiment of Scarlet Devil are © ZUN / Team Shanghai
Alice. Game assets are supplied separately. See the
[Touhou Project fan-work guidelines](https://touhou-project.news/guidelines_en/).

- **ZUN** — original game, music and artwork.
- [happyhavoc/th06](https://github.com/happyhavoc/th06) and
  [GensokyoClub/th06](https://github.com/GensokyoClub/th06) — matching decompilation.
- [REA](https://github.com/morluto/rea) — evidence-oriented reverse engineering tools.
- [thtk / thpatch](https://github.com/thpatch/thtk) — format tooling and documentation.
- [PyTouhou](https://pytouhou.linkmauve.fr) — earlier open reimplementation.
