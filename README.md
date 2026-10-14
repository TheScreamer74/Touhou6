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
executable is not run. Supply your own game files for native play. The web
version loads the host's asset manifest and starts automatically; scores,
settings and replays stay on the player's device.

Implementation follows the [matching decompilation](https://github.com/GensokyoClub/th06).
[REA](https://github.com/morluto/rea) and Ghidra were also used to inspect the
original executable and record function evidence. This does not establish
whole-game binary or visual equivalence; see [validation](docs/REVERSE_ENGINEERING.md).

## Supported features

| Area | Implemented support |
|---|---|
| Stages | Stages 1–6 and Extra, including enemies, bosses, dialogue and 3D backgrounds |
| Difficulty | Easy, Normal, Hard, Lunatic and Extra; difficulty-specific script masks and rank limits |
| Characters | Reimu A/B and Marisa A/B; original shot tables, homing, needles, retained missile explosions, laser pulses, orbs and four bombs |
| Projectiles | Original spawn/cancel animations, rectangle collision, graze rules, laser lifecycle and graze timing, particle effects and score popups |
| Run flow | Stage progression, pause, continues, game over, Practice and clear results |
| Endings | Six END routes selected by character, difficulty and continues, followed by the staff roll |
| Records | High scores, clear/spell-card records and original `score.dat` persistence |
| Replays | T6RP recording, save/load slots and original replay parsing/playback |
| Menus | Difficulty/character selection, Result, Practice, Replay, Music Room and Options |
| Audio | 17 BGM tracks and original loop markers; WAV/web Ogg playback, MIDI synthesis with a supplied SoundFont, original sound-effect variants and gains |
| Display | 32-bit and original R5G6B5 mode; supplied Japanese font or bundled fallback |
| Input | Keyboard and gamepad, configurable controller mappings |
| Web | Automatic launch with hosted assets, fullscreen and local score/replay import/export |

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

Configure `assets.json` and its referenced game files, then open
`http://localhost:8080`; the game starts automatically. The
[web guide](web/README.md) documents the manifest, engine cache versioning and
save exports. Browser audio starts after the first key press or click.

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

Player attacks now use all 193 original table entries and the original damage
rectangles, retained Marisa A explosions, Marisa B laser timers and reversible
orb focus transitions. Focused C++ comparisons also cover enemy bullet/laser
collision, cancellation, graze cadence and state transitions. Real assets
exercise all four complete bomb cycles, impact animations and laser tiers.
The replay regression completes all 96 legal stage/difficulty/shot routes and
compares every recorded/playback frame with invulnerability enabled. The
96 routes cover 1,375,027 live frames plus 1,375,027 replay frames (2,750,054
observations). These roundtrips verify the port's internal determinism with
real assets; they do not establish Windows playback identity. The [dated report](docs/REVERSE_ENGINEERING.md)
records the separately executed binary comparisons and their actual coverage.

The R5G6B5 renderer reproduces limited texture/framebuffer precision, but driver
rounding may differ from the original Direct3D hardware. MIDI timbre depends on
the supplied SoundFont, just as the original depended on its MIDI device.
Native builds accept `TH06_SOUNDFONT` or `gm.sf2` beside the DAT files; web hosts
can supply `gm.sf2` through the asset manifest. Japanese text accepts
`msgothic.ttc`/`msgothic.ttf` beside the DAT files or through the hosted manifest;
native builds also support `TH06_FONT`. The bundled Noto Sans JP remains the
fallback. Font outlines alone do not
reproduce Windows GDI rasterization. See [display/audio details](docs/PRESENTATION_FIDELITY.md).
Browser audio, GPU availability and storage depend on the browser. Focused
comparisons are evidence for their tested paths, rather than a claim of 100%
fidelity across every character, difficulty, frame and visual effect.

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
