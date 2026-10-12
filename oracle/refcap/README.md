# Reference-capture harness (`oracle/refcap`, #9)

Runs the **real TH06 1.02h** as a deterministic, scripted, frame-dumping oracle
— the pixel-accurate ground truth for background/gameplay parity (#10). No game
file is modified; bring your own legal copy.

## How

A proxy `d3d8.dll` (cross-compiled with mingw-w64, dropped next to `102h.exe`)
is loaded by the game instead of the system one (DLL search order; under Wine
via `WINEDLLOVERRIDES="d3d8=n,b"`). It:

- forwards `Direct3DCreate8` to the real d3d8, vtable-hooking
  `IDirect3D8::CreateDevice` → `IDirect3DDevice8::Present`;
- on every Present, copies the back buffer to a sysmem surface and writes
  `capture/frame_%06u.bmp` (24bpp; X8R8G8B8/A8R8G8B8/R5G6B5/X1R5G5B5 handled);
- IAT-patches the exe (in `DllMain`, before `WinMain`):
  - `timeGetTime` → fake clock (+1 ms per call from the main thread). This
    **fixes the RNG seed** (`Supervisor.cpp:330` seeds `g_Rng` from the first
    call) and drives the frame limiter (`GameWindow.cpp`, `FRAME_TIME=1000/60`)
    deterministically — one logic tick per Present, at uncapped real speed;
  - `DirectInput8Create` → fails, so `Controller::GetInput` falls back to the
    `GetKeyboardState` path (`Supervisor.cpp:364-377`);
  - `GetKeyboardState` → scripted per-frame key states.

Same input script → same frame-for-frame run, every time. Calibrate the menu
navigation once; "stage N at capture frame F" is then stable.

### Read-only replay state comparison

`statefile replay-state.csv` logs the original ReplayManager frame ID, stage,
player position, stocks, power, score, graze, rank, RNG seed and live projectile/
item counts and the player hit-particle counter. It reads the size-checked 1.02h layouts and changes no game state.
`stopafter 6500` terminates this isolated reference process after that Present.
Use a separate copy of the supplied game and leave `god`, `practice` and
`forcestage` disabled. With no scripted input, the original title starts its
bundled `demo00.rpy` automatically. The trace can contain repeated demo sessions;
the comparison script selects the first.

```sh
cargo run -p th06 --release -- --game-dir /path/to/game \
  --replay /path/to/extracted/demo00.rpy --frames 4000 \
  --replay-dump port-state.csv
python3 scripts/verify_replay_reference.py replay-state.csv port-state.csv \
  --output replay-comparison.json
```

Optional `detailfile bullets.csv` and `effectfile effects.csv` read live pool
slots during the inclusive `detailstart` / `detailend` frame range. The port's
`--replay-details bullets.csv` and `--replay-effects effects.csv` expose matching
debug observations. Callback/sprite fields in the original effect trace are
raw pointers; resolve them through the original layout/map before comparison.

The one-tick-per-Present description below applies to ordinary gameplay.
During skippable replay dialogue the original restarts its calculation chain
until `ReplayManager.frameId % 3 == 2`, performing up to three ticks per Present.
Compare by replay frame ID. A scalar match verifies the listed observations;
it does not establish pixel identity, audio identity or every possible route.

## Use

```sh
./build.sh                    # -> d3d8.dll (needs brew mingw-w64)
./run.sh [game_dir]           # deploys dll+config, runs under Wine
./convert.sh <capdir> out.mp4 # BMP sequence -> 60fps mp4 (needs ffmpeg)
```

`run.sh` defaults to the repo-adjacent `TH06 ~ The Embodiment of Scarlet
Devil/kouma` and a dedicated `~/.wine-th06` prefix (created on first run;
`brew install --cask wine-stable`).

Config = `th06cap.txt` in the game dir (see `th06cap.example.txt`):
`capdir`/`capstart`/`capend` select the dumped frame range; `key <start> <end>
<name>` holds a key for logic frames `[start, end)`; `tap <first> <period>
<count> <name>` fires `<count>` short presses every `<period>` frames (robustly
walks the menu chain by re-confirming defaults — prefer this over hand-timed
`key` for menu nav); `realtime 1` paces at ~60 Hz for watching. The proxy logs
to `th06cap.log` (patches, backbuffer format, capture start; `TH06CAP_TIMING=1`
adds per-Present clock/poll lines).

## Determinism

The clock is derived purely from the Present count (frame `k` reads
`base + k*(1000/60)`), so a given input script produces a **byte-identical**
frame sequence every run. Verified: two independent runs md5-match across menu,
stage-entry and mid-stage frames. That is the point — "stage N, frame F" is
stable, reproducible ground truth.

Speed: without `realtime 1`, the game runs uncapped (fast wall-clock) but each
captured frame is exactly **one** logic tick — patterns are frame-exact
regardless of how fast it looks live. Watch at true 60 Hz with `realtime 1`, or
just play the converted mp4.

Two real-time busy-waits have no Present inside and would deadlock a purely
frame-locked clock — the menu-music delay (`MainMenu.cpp:1019`, 3000 ms) and BGM
load (`SoundPlayer.cpp:212`, 100 ms). A monotone "creep" term (1 ms per poll
once a frame is polled >64×) advances the clock only inside those spins, exiting
them instantly and deterministically.

### Timing verified (`TH06CAP_TIMING=1`)

Audited against the decomp game loop, and confirmed in-run:

- **1 logic tick : 1 Present.** In windowed mode the loop ticks the calc chain
  once (`curFrame==0`), then Presents once and resets `curFrame`
  (`GameWindow.cpp`, `I_HAVE_NO_CLUE...`); the frame limiter's `do/while` only
  *consumes* elapsed time, it never multi-ticks. The Present-count frame index
  therefore equals the logic frame.
- **`effectiveFramerateMultiplier == 1.0` always.** Windowed sets
  `framerateMultiplier = 1.0`, so the loop takes the `else` branch
  (`GameWindow.cpp:147`) — never the 0.5/0.8 slowdown. `ZunTimer` advances whole
  frames; patterns can't run fractional or desync.
- **Clock advances exactly `1000/60` ms per Present** (logged:
  `100000, 100016, 100033, 100050, …`), except one deterministic creep jump at
  the menu-music wait — which happens **before any stage**.
- **No duplicate presents:** consecutive captured stage frames are all distinct.

So capture frame F within a stage is exactly F−(constant menu offset) logic
frames in, at a true 60 Hz cadence.

## Notes

- `cfg.cfg` as shipped is already right for capture: windowed=1, 32-bit
  colour, frameskip=0 (decomp `GameConfiguration`, Supervisor.hpp:56).
- Audio may be silent under a gstreamer-less Wine — irrelevant to capture.
- Wine startup is occasionally flaky (a d3d/init crash before the title); it is
  intermittent, not caused by the proxy — just relaunch. Once past the title the
  run is stable to 3000+ frames.
- Comparing with the port: the port's stage frame 0 = `--scene stage` start;
  find the capture frame where the stage fade-in begins and diff from there.
- Stage 1 has been validated 1:1 against the port (banks symmetric, colour
  matches) — confirms the `background.rs` transform fixes against ground truth.

### Reaching later stages (stage 4+ capture)

`god 1` no-ops `Player::Die` (0x427770), so a `key <n> 999999 Z` hold-shoot run
never dies. But a *stationary* player barely damages bosses — they only end on
spell timeouts — so grinding to stage 4 takes ~15 min / ~25 000 logic frames
(EoSD stages are ~2-3 min each). Use `capstride` to scan such a run cheaply.

The efficient path for st4+ is **Practice mode** (`practice 1`), which unlocks the
stage-select: `MainMenu::ChoosePracticeLevel` offers stages up to
`g_GameManager.clrd[charShotType].difficultyClearedWithoutRetries[difficulty]`
(g_GameManager 0x69bca0, `clrd` at +0x1030, `difficultyClearedWithoutRetries` at
+0x11 — offsets verified via `offsetof` against the `ZUN_ASSERT_SIZE` values).
`ParseClrd` reloads it from `score.dat` on each menu entry, so `practice 1` pokes
all four `clrd` entries to 6 **every Present** (like the god patch), keeping all
stages unlocked while the menu is up.

The stage-select cursor navigation is timing-fragile (the char/shot screen only
accepts Z after an intro), so instead use **`forcestage N`**: it writes
`g_GameManager.currentStage = N-1` (offset +0x1a34, offsetof-verified; the value
`MainMenu.cpp:837` sets from the cursor) every Present, so a Practice run *starts*
at stage N regardless of the cursor. Just walk into Practice with dense Z taps and
`forcestage` picks the stage:

```
practice 1
forcestage 4
god 1
key 240 248 DOWN     # Start -> Extra
key 300 308 DOWN     # Extra -> Practice Start
tap 380 45 16 Z      # dense Z walks practice/difficulty/character/shot/select
```

Verified: this reliably lands in **stage 4** (Voile library). A real-vs-port
stage-4 comparison confirmed the port's background colour/brightness is faithful
(playfield luminance 5.1 vs 4.5 — both dark red-dominant), closing the #10
stage-4 item. (Wine startup is intermittently crashy; retry the launch.)
