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
`capdir`/`capstart`/`capend` select the dumped frame range, `key <start> <end>
<name>` holds a key for logic frames `[start, end)`. The proxy logs to
`th06cap.log` (patches applied, device/backbuffer format, capture start).

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

## Notes

- `cfg.cfg` as shipped is already right for capture: windowed=1, 32-bit
  colour, frameskip=0 (decomp `GameConfiguration`, Supervisor.hpp:56).
- Audio may be silent under a gstreamer-less Wine — irrelevant to capture.
- Wine startup is occasionally flaky (a d3d/init crash before the title); it is
  intermittent, not caused by the proxy — just relaunch. Once past the title the
  run is stable to 3000+ frames.
- Comparing with the port: the port's stage frame 0 = `--scene stage` start;
  find the capture frame where the stage fade-in begins and diff from there.
