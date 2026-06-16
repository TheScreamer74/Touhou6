<div align="center">

# 東方紅魔郷
## Touhou 6 — the Embodiment of Scarlet Devil, for macOS

**A native Apple Silicon reimplementation of ZUN's 2002 classic,**
**built in Rust on Metal (and WebAssembly) — bring your own copy of the game.**

![Platform](https://img.shields.io/badge/platform-macOS%20%2B%20Web%20(WASM)-black?logo=apple)
![Language](https://img.shields.io/badge/language-Rust-b7410e?logo=rust)
![Renderer](https://img.shields.io/badge/renderer-wgpu%20%2F%20Metal%20%2F%20WebGL2-blueviolet)
![Status](https://img.shields.io/badge/status-all%206%20stages%20playable-crimson)

*"The border land was wrapped in Scarlet Magic.*
*Girls believe that you solve this mystery..."*

</div>

---

## What is this?

A from-scratch engine that reads the **original, unmodified game files** of
Touhou Koumakyou ~ the Embodiment of Scarlet Devil (v1.02h) and runs the game
natively on macOS. No Wine, no emulation, no patched binaries — the same
model as [devilutionX](https://github.com/diasurgical/devilutionX) or
[OpenRCT2](https://github.com/OpenRCT2/OpenRCT2).

You supply your own legally obtained copy of the game. This repository
contains **zero** copyrighted assets — only code.

## Status

| Milestone | State |
|---|---|
| PBG3 `.DAT` archive extraction | ✅ done |
| ANM sprite/animation format + VM | ✅ done |
| ECL / STD / MSG interpreters | ✅ done |
| Title screen + faithful character / shot-type select | ✅ done |
| Audio — BGM + sound effects | ✅ done |
| **All 6 stages playable** — trash, midbosses, bosses, dialogue, spellcards, 3D backgrounds | ✅ done |
| Stage progression carrying lives / bombs / power / score | ✅ done |
| All four shot types (Reimu A/B, Marisa A/B) — distinct shots & bombs | ✅ done |
| Faithful player — shot tiers, hitbox, graze, deathbomb | ✅ done |
| Faithful items — point-of-collection latch, bullet-cancel → point items, death/boss-kill scatter drops | ✅ done |
| Scoring, results screen, high-score save, pause menu | ✅ done |
| **Web build** (WebAssembly, bring-your-own-files upload, fullscreen) | ✅ done |
| Boss ex-instructions (`EXINSCALL`) — some spellcard patterns simplified | 🚧 partial |
| Per-character bomb visuals, HUD layout polish, game-over name entry | 🚧 in progress |
| Difficulty select (Easy/Hard/Lunatic), Extra stage, endings | 🔜 next |
| Replays, Practice+, JP/EN selector, gamepad | ⬜ |

Stages run the **original ECL/STD/MSG scripts** directly, with the player,
scoring and collision aligned opcode-by-opcode against the
[happyhavoc/th06](https://github.com/happyhavoc/th06) (now
[GensokyoClub/th06](https://github.com/GensokyoClub/th06)) matching
decompilation — so behavior matches the real thing, including the original RNG.

## Already better than 2002

- **Stable 60 Hz** — game logic runs on a fixed timestep, decoupled from
  display refresh. No more speed tied to your GPU. ProMotion-safe.
- **Runs in the browser** — a WebAssembly build (drop in your own game folder;
  files never leave your machine) with scaling + fullscreen.
- **Native resolution scaling & Metal rendering** via wgpu.
- **In-game pause menu** (resume / return to title / quit).
- **Crash reports** written to `logs/` (the original just vanished).
- Planned: rebindable keys & gamepads, practice any spellcard, replay
  rewind, instant restart.

## Building

```sh
# Rust toolchain (rustup.rs), then:
git clone https://github.com/TheScreamer74/Touhou6-Mac.git
cd Touhou6-Mac
cargo run -p th06 -- --game-dir "/path/to/your/th06/folder"
```

The game folder must contain the v1.02h data files
(`CM.DAT`, `IN.DAT`, `ST.DAT`, `TL.DAT`, `th06e_ST.DAT` and the `bgm/`
directory). The Windows `.exe` files are never executed — only the data is
read.

## Play in the browser

A WebAssembly build runs the same engine in a browser — you drop in your own
game folder and the files stay on your machine (nothing is uploaded). See
[`web/README.md`](web/README.md):

```sh
wasm-pack build crates/game --release --target web --out-dir ../../web/pkg
cd web && python3 -m http.server 8080   # open http://localhost:8080
```

## Controls

| Key | Action |
|---|---|
| Arrows | Move |
| `Z` | Shoot / confirm |
| `X` | Bomb / back |
| `Shift` | Focus (slow movement) |
| `Esc` | Pause / back |
| `F` | Fullscreen (web build) |

## Architecture

```
crates/
├── formats   PBG3 archives · ANM sprites · ECL · STD · MSG
├── engine    wgpu sprite renderer · fixed-timestep loop · input · rodio audio
└── game      ANM VM · ECL VM · title menu · stage logic · background · scenes
```

## Legal

This is an unofficial fan project, unaffiliated with Team Shanghai Alice.
Touhou Project and the Embodiment of Scarlet Devil are © ZUN / Team Shanghai
Alice. This repository distributes no game assets; it requires a legally
purchased copy of the original game to run. Made under the spirit of the
[Touhou Project fan-work guidelines](https://touhou-project.news/guidelines_en/).

## Credits

- **ZUN** — for the game, obviously. Buy his games.
- [happyhavoc/th06](https://github.com/happyhavoc/th06) — matching
  decompilation; our ground truth for game logic.
- [thtk / thpatch](https://github.com/thpatch/thtk) — file format
  documentation and tooling.
- [PyTouhou](https://pytouhou.linkmauve.fr) — pioneering open
  reimplementation, an invaluable map of the territory.

<div align="center">

*⑨ Even Cirno could build this. (She could not.)*

</div>
