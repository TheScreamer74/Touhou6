# Original display and MIDI modes

The port now supports the original 16-bit display option and actual MIDI
playback. These are separate from the WAV/Ogg path; the 44.1 kHz source-rate
loop correction remains in place for browsers running at 48 kHz.

## R5G6B5

`main.cpp` selects an R5G6B5 back buffer. This display choice does not change
texture formats: the separate advanced `force16bitColorMode` option controls
whether `AnmManager::LoadTexture` converts UNKNOWN/A8R8G8B8 to A4R4G4B4 and
R8G8B8 to R5G6B5. The comparison profiles leave that advanced option disabled.
Explicit ANM A4R4G4B4, R5G6B5 and A1R5G5B5 formats retain their own precision.

`engine::color565` stores integer 5/6/5 channel codes in an RGBA8 carrier.
It reads the previous destination, applies the original alpha or additive
blend, and quantizes the result after each overlapping quad. It also keeps
the original texture channel codes and interpolates their expanded values.
This avoids treating the mode as a filter applied only to the completed frame.
Only consecutive quads whose conservative screen rectangles do not overlap
can share a pass. Background ranges preserve the source's z-level/instance/quad
order, including interleaved alpha/additive VMs. ANM31 controls depth writes
independently of blend mode; the source defaults to depth writes in both modes. Background depth survives every pass. Presentation scales
the logical 640×480 buffer to the window.

GPU-dependent edge, interpolation and color-conversion rounding from a 2002
Direct3D driver cannot be proven identical on Metal/WebGL2. The renderer uses
upper-bit truncation for 5/6/5 destination quantization, as measured in the
original capture receipts, and the source texture format's high bits. This
implements the limited-precision mode without claiming bit identity with every
original GPU.

The original enemy hit flash uses the texture-stage `D3DTOP_ADD` color
operation. Both display paths add VM RGB to sampled texture RGB and clamp
each channel, while retaining texture-alpha × VM-alpha. This operation is
independent of the framebuffer's additive blend mode. The original default
software texture blending profile leaves `D3DTOP_MODULATE` active even when an
ANM VM requests ADD; the port only enables that RGB addition with the matching
hardware texture blending option.

## Frame history and effects

The default original options preserve the color back buffer. `GameWindow::Render`
clears color and depth only when the advanced clear-backbuffer or minimum-graphics
option is set; `GameManager::OnUpdate` ordinarily clears depth alone. Both port
color modes now keep a logical 640×480 color buffer across draw chains. Each
`Game::update` advances that buffer once, including every catch-up update and
every unselected screenshot-capture frame. Surface refreshes and screenshot
readbacks copy the completed image without drawing alpha layers again. This
preserves fading spellcard layers and the image behind stage-loading transitions
without making their accumulation depend on the monitor refresh rate.

`GameManager::OnUpdate` prepares the 3D camera before `Stage::OnUpdate` changes
its facing direction. The port snapshots that direction before stage calculation,
while drawing the current positions and fog. The global direction starts at zero
only when the game is constructed and survives stage changes and later runs.
Consequently the first background transform is degenerate and preserves the
Replay menu image for that draw; the following draw uses the stage's first
calculated facing. This follows callback order without shifting replay inputs or
the STD script clock. Paused calculations still prepare the camera.

Native drawing waits for completion after each calculation-chain frame. This
bounds temporary buffers retained by the GPU queue even when a replay writes
only occasional screenshots. Browser drawing uses asynchronous completion
callbacks and allows at most two calculation frames in flight; further updates
wait for a later animation callback without blocking the UI. The logical color
buffer is fixed-size and retains no list of previous images.

The WASM engine explicitly selects WebGL2 through wgpu's GLES backend.
wgpu25's default `GlFenceBehavior::Normal` inserts `fenceSync`, flushes and
polls `getSyncParameter(SYNC_STATUS)`. A later queue submission maintains the
device and fires completed callbacks. Surface blits keep submitting while
history calculations are deferred, so completion polling continues. The
two-chain limit covers original history draws; surface blits and their fences
are separate submissions. The normal polling path does not call `gl.finish`;
the GLES wait path uses a zero WebGL client-wait timeout. No browser queue or
memory pass is claimed from this source inspection alone.

`scripts/webgl_backpressure_probe.js` is a self-invoking browser diagnostic for
the settled 2D title. It observes `drawArrays` and `drawArraysInstanced`: the
engine emits six vertices per sprite quad, whereas the isolated surface blit
emits three. It rejects unknown geometry, unclassified submissions and absent
history chains. It temporarily masks fence completion for one second, measures
history and surface queues separately, then restores normal polling and checks
recovery. Execute its source with `await page.evaluate(probeSourceText)` on the
actual running WASM page; evaluating an uninvoked arrow function is not a test.
When browser evaluation is restricted to reading, a local copy of the page can
run the probe from a visible diagnostic button and show its JSON in a `pre`.
Keep that control outside the production HTML and record the copied page,
probe and tested WASM hashes separately.

The stage-specific `eff01.anm` through `eff07.anm` spellcard backgrounds use
original script zero, its alpha animation and ANM UV-scroll opcodes 27/28. Stages
6 and 7 use the original early background and replace it when the boss dialogue
loads the late effects sheet. Time stop writes the original D3DCOLOR bytes
alpha 0x60, red 0x80, green 0x30, blue 0x30 (`Stage_OnUpdate`; `COLOR_RED_BYTE_IDX`
is 2), a red tint; an earlier port swapped red and blue. In the replay-c0-d1-s5
time-stop checkpoint 10800 this lowered playfield MAE from 11.71 to 2.95.
Fidelity captures pass `--hitbox off`: the original draws no hitbox, while the
port option defaults to Focus. Lasers and their base glows are drawn by
`BulletManager_OnDraw` with Draw3 at unrounded D3D pixel-centre coordinates;
the port now adds the same +0.5 WebGPU half pixel as `sprite_geometry::world`.
Before, they sat half a pixel up and left: in replay-c1-d3-s6 checkpoint 7201
playfield MAE fell from 5.68 to 1.82. `loading.anm` appears on stage results and runs its
interrupt-driven reveal on the next stage; the stopped loading VM gates stage
background drawing as in `Gui::IsStageFinished`.

These claims describe implemented draw behavior and targeted verification.
Whole-frame comparison still includes font rasterization and geometry differences;
no claim of pixel-perfect presentation is made.

Particle color belongs to the running ANM VM. `SpawnParticles` assigns its tint
after initial script execution, and a running fade interpolates all four saved
color components. Thus the expanding Reimu A explosion restores the script's
saved white RGB on its next tick, rather than retaining a permanent blue
multiplier. The focused ANM regression check covers this assignment order.

Foreground sprites now retain their original depth comparison and ANM31
depth-write flag. Player and option RHW depths0.49/0.491 map to0.245/0.2455 in
the foreground viewport0..0.5; Draw2/Draw3 effects use the perspective depth
around0.445. Later additive effects therefore cannot bleach an opaque player
pixel that wrote the nearer depth. BulletManager's lasers, items and bullets
use its explicit ALWAYS scope, after EffectManager. GUI/menu commands retain
their independent overlay behavior. Each history draw clears depth, while
color history remains persistent.

Small score popups follow the original `AsciiManager` VM initialization:
`Initialize` clears the earlier top-left anchor, so each digit is centered at
the popup's `x - characterCount * 4`, advancing eight pixels per digit. Draw2
rounding and perspective at z0.1 apply before adding the playfield origin;
the wide sprite10 label retains its 48×8 dimensions. Its depth comparison is
LESSEQUAL with depth writes enabled. This later rendering correction changes
neither popup values nor timers and is validated on a newer pinned build.

The reference mapping is supported by the original Stage/Player/AnmManager
callbacks and Wine11's [RHW conversion](https://github.com/wine-mirror/wine/blob/wine-11.0/dlls/wined3d/glsl_shader.c)
and [viewport depth range](https://github.com/wine-mirror/wine/blob/wine-11.0/dlls/wined3d/ffp_gl.c).
The supplied etama4 sprites remain planes parallel to the screen: their
spell-bubble callback moves the center through3D, without tilting the quad.
The foreground also preserves the original viewport timing. GameManager prepares
its integer viewport before ScreenEffect updates the arcade region. Projected
Draw2/Draw3 sprites use that prepared camera; RHW player, bomb and item positions
use the newer arcade origin. GUI portraits and bomb darkening remain at absolute
screen coordinates. Per-command scissors reproduce the later Ascii viewport
switch and the fullscreen/arcade distinction of queued GUI strings. The GPU
clip test covers both framebuffer modes, clip changes, empty clips and depth
retention. In the stage6 bomb pilot, playfield MAE at frame962 fell from39.793
to1.488 without changing its scalar checkpoint.

Stage3's active camera-facing quads use the original projected centre and
projected-X scale, then an unrounded screen-aligned rectangle. Their XYZRHW
z is the projected viewport depth; Wine11 maps it through the viewport again,
while transformed fog uses that RHW z, as confirmed in `glsl_shader.c` and
`ffp_hlsl.c`. A numerical fixture checks the real STD4070 camera case, including
screen alignment and its33.490-pixel width/height. This is distinct from a
world XY plane viewed at an angle.

The final stage/Extra result table now shows `All Clear!`, the Player/Bomb stock
rows, the original rank/penalty/total row positions, and the player's option
sprites. The calculation of the awarded score is unchanged. Its displayed high score
follows the rolling GUI score; awarding the bonus no longer anticipates that
animation. The persistence event still receives the same real maximum score.

Uniform per-quad depth covers those sprites; this does not add a general
tilted-quad transformation to the 2D command API.

Boss UI visibility follows `Gui::bossPresent` or a nonzero
`bossHealthBarState`, independently of an occupied enemy slot or a nonempty
health bar. VM19 restarts `front.anm` script19 on entry and script20 on exit;
their 60-tick fades end at ANM_STATIC time61. The eased health bar starts only
after entry finishes, and its target, the global ECL life count and the displayed
timer survive enemy removal until the exit VM finishes. Text opacity changes
by four per calculation. Dialogue hides this panel, and the warning SFX uses
the same visibility condition. Spell portraits and name banners keep their own
independent VM lifetime.

The displayed timer is latched by `Enemy::HandleTimerCallback`, before its
timeout test and timer reset. This includes a boss initialized by the current
timeline calculation; looking up the boss before EnemyManager missed that
first value. The callback retains signed integer division and the original
threshold gate, independently of the GUI animation's time-stop gate.

Spell portraits and name banners execute in the common GUI calculation phase,
after Player priority7 and Enemy priority9. Their initial SetAndExecuteScript
already executes frame zero; Gui priority12 executes them again in the same
calculation that creates them. The Reimu bomb portrait therefore starts its
late scale animation on the original tick. Its screen bounds use the original
independent ties-to-even rounding of each edge.

## MIDI

The MIDI files are extracted from the supplied MD.DAT. The scheduler follows
`MidiOutput.cpp`: initial tempo 1,000,000, integer 1 ms timebase conversion,
track-order processing, running status, tempo changes and the global CC2/CC4
loop cursor snapshots. Loop restoration changes cursors/counts and timer
values; it does not reset channel controllers or kill sustained notes.
Fade-out scales each original CC7 channel volume linearly, as the original
MIDI backend does.

REA/Ghidra inspected the original `102h.exe`, SHA-256
`7f38496b31b8625196900a69cd1bfed243cab1f9612073e7881dc354b876fd39`,
function `MidiOutput::ProcessMsg` at `0x422900`. Evidence
`ev_8f7bc5e23e5c688923b990cb0b290051d7be7fc83f26548416bbcc7f34947d2b`
confirms the unusual `tempo = tempo * 257 + next_byte` expression, which the
port preserves rather than silently replacing with standard base-256 parsing.

The original game sent messages to the chosen Windows MIDI output device;
its timbre was device-dependent. The port uses the MIT-licensed RustySynth
General MIDI synthesizer with a separately supplied SoundFont. The hosted
build provides the freely licensed TimGM6mb bank, attribution and full GPL-2
license as separate data files. Its SHA-256 is
`c5378b62028c920cb11e4803327983fee2f2cdff5dc89c708e39da417e51c854`.
The bank comes from [Debian timgm6mb-soundfont 1.3-5](https://deb.debian.org/debian/pool/main/t/timgm6mb-soundfont/),
with [upstream source](https://github.com/musescore/musescore-old/commit/90c33ef9d87b3f5ff92efd3b07d89eb455fb1fef).
The bank is independent of the MIT engine source and is not included in Git.
That bank is not a recording of the game's WAV soundtrack and is not the
proprietary Windows GS bank. Unsupported device-specific SysEx extensions do
not become new synthesizer features. GM System On and Roland GS Reset are
recognized.

For native builds, provide `gm.sf2` beside the game's DAT files, or set
`TH06_SOUNDFONT=/absolute/path/to/bank.sf2`. Web hosts can provide `gm.sf2`
through the asset manifest. MIDI is enabled only when the bank and all 17
original MIDI files validate. The browser backend
instantiates the same compiled WASM module in an AudioWorklet and synthesizes
stereo on the audio rendering thread. Its 128-frame blocks use the actual
AudioContext sample rate. An Encoding API fallback loads before wasm-bindgen
because Chromium omits TextDecoder/TextEncoder from AudioWorkletGlobalScope.
The browser explicitly waits for audio-node installation before the first
title track, including when MIDI was persisted as the selected mode. The page
loads hosted assets and starts automatically; the browser unlocks audio on the
first key press or click. The game thread only sends bank/track/fade/stop
messages. A 2048-frame
ScriptProcessor provides compatibility playback when AudioWorklet cannot
start, or restarts a track if the isolated processor reports a failure. That
compatibility path remains subject to main-thread stalls.

## Sound effects

The engine now preserves all 32 SoundIdx variants over the original 26 files,
including each variant's DirectSound hundredths-of-a-decibel volume. Up to
three distinct variants enter the queue per calculation-chain run, in request
order; duplicate requests are discarded. A retrigger stops and restarts the
same variant's voice at sample zero, while different variants sharing a WAV
remain independent. The table's `unk` field does not actually implement a
priority in `PlaySoundByIdx` and is not invented as one here. Queue flushing
occurs after the complete calculation chain, including replay dialogue
acceleration, and honors the sound option.

## Japanese font

No MS Gothic font is supplied by the game archive or the current user system.
Native builds can explicitly load `msgothic.ttc`/`msgothic.ttf` beside the DAT
files or a font selected by `TH06_FONT`; hosted manifests recognize those two
filenames. Proprietary fonts are never bundled or hosted by default.
The bundled Noto Sans JP remains the fallback. Supplying the original font
matches its outlines but does not reproduce Windows GDI antialiasing or
synthetic bold pixel-for-pixel. The fixed text-sprite helper reproduces the
source canvas, Shift-JIS byte-count alignment and triangle downsampling used
by `TextHelper_RenderTextToTexture`.

## Checks

- Native and wasm engine compilation, engine Clippy and normal engine tests.
- GPU tests: overlapping alpha/additive draws quantize after every operation;
  A4R4G4B4/R5G6B5 source precision; background alpha/additive and depth.
- Real GPU color-operation check in both display modes: texture-stage RGB
  addition clamps correctly and keeps the ordinary alpha calculation.
- Real GPU history check in both display modes: empty draw chains preserve
  color, consecutive alpha layers accumulate once each, and repeated screenshot
  readbacks leave the completed image unchanged.
- Native Metal memory check: 3,000 unselected draw chains per display mode,
  after 100 warm-up frames per mode, completed in 19.25 seconds. Resident memory
  grew by 131,072 bytes from a 25,985,024-byte baseline in 32-bit mode and by
  638,976 bytes from a 48,889,856-byte baseline in 16-bit mode. Each mode's six
  measurements stabilized after the first 500 frames. Browser memory stability
  with the two-frame limit still requires a browser capture check.
- Targeted native S3 pilot with natural Replay-menu entry: 12 image pairs,
  no missing images or scalar checkpoint differences. Caching the pre-calculation
  camera reduced frame2 playfield MAE from 139.228 to 1.327; frame3 reached
  0.383. Reimu A effect color reduced frames3000–3002 playfield MAE from
  30.334/31.527/32.193 to 1.687/1.778/1.853. The pilot retained all pixels and
  still failed the strict image criteria; it predates the boss-UI correction
  and does not establish the final 96-scenario result.
- A second S3 pilot on source-certified binary `9c55284541d0cc4fba61ae1654c0767a73635791b3a6730d7429b770a0ac5217`
  includes boss-UI lifetime and portrait GUI-order fixes. All 12 selected scalar
  checkpoints still match. Portrait frames1000–1002 playfield MAE fell from
  7.298/7.389/7.291 to 1.154/1.185/1.193; frames1–3 and3000–3002 are unchanged.
  The capture completed in 8.39 seconds with peak resident memory606,339,072
  bytes and zero swaps. Strict image criteria still fail, including residual
  HUD/font differences. This targeted pilot does not prove the complete boss
  disappearance window or the final 96-scenario result.
- A read-only original GUI probe and native binary `b0538278d802901b956953859973b0d724127e04a70e2a93626cc13fbfd73b0c`
  match all nine GUI fields over 3,002 calculations exactly. The canonical
  SHA-256 includes calculation IDs, integer fields and IEEE754 binary32 bits
  for both bars; it applies no tolerance or temporal alignment. The original
  enters boss state1 at frame2009, state2 at2070, exit state3 at2920 and state0
  at2980. The original enemy slot remains occupied until3061. The native probe
  took 8.47 seconds with peak resident memory310,591,488 bytes and zero swaps.
  Original CSV/BMP evidence stays on the reference server; only its aggregate
  was returned. This confirms GUI lifetime and timer state for this window,
  independently of image fidelity and the complete scenario matrix.
- The S3 depth pilot on binary `4e84e9e14d5bfc3c429aec93fe6f35a99da8e5e935a98abc6a27fa0686c69096`
  retains all 12 image pairs and scalar checkpoint equality. Player/options ROI
  `(136,354,48,74)` MAE at frames3000–3002 fell from24.789/24.125/21.390 to
  2.129/2.162/2.286. Unchanged whole-playfield metrics fell from
  1.687/1.778/1.853 to1.186/1.278/1.410; full-frame metrics reached
  0.735/0.779/0.847. The corrected player stays visible through the explosion.
  This ROI supplements the full-frame/field/HUD/FPS comparisons; it masks no
  pixels in those reports. Strict image criteria still fail. First-entry and
  portrait PNGs are byte-identical to the preceding pilot; two early frames
  have only minor pixel changes. No new large outlier appeared in this pilot.
- All 17 original MIDI files run for 600,000 timer callbacks each and complete
  multiple CC4 loop cycles.
- All 17 tracks generate finite, non-silent stereo PCM through TimGM6mb at
  both 44,100 and 48,000 Hz, with MIDI fade-out exercised.
- Real browser AudioWorklet playback at 48,000 Hz continues generating
  nonzero PCM while the main thread is blocked for two seconds; see
  `midi-worklet-2026-10-12.json` for the measurement receipt.
- A real persisted-MIDI page reload starts the title through AudioWorklet
  after bank readiness, with no ScriptProcessor creation or browser warnings.
- SFX queue tests preserve three-variant capacity, shared-file independence,
  duplicate removal and original gain conversion.

Asset-dependent/GPU checks are explicitly ignored by the ordinary test run.
They can be run with `TH06_MIDI_DIR` pointing to extracted MD.DAT entries and
`TH06_SOUNDFONT` pointing to a legally supplied bank. GPU tests need a real
adapter; a sandbox without Metal access is not a rendering pass.

The complete campaign on pinned binary `4e84e9e1` records simulation and images
from the same executions. Its image failures remain explicit, including large
outliers found in dense cancellation scenes; they are not all attributable to
font rasterization. Later score-popup and WebGL changes have separate focused
receipts on a newer build. A successful simulation campaign, successful targeted
captures and whole-frame pixel parity are distinct results. Font/GDI, GM-bank
timbre and driver-dependent rendering limits remain as described above.

`scripts/run_capture_matrix.py` runs the image pass with one capture
process at a time and one comparison at a time. `--require-matrix --check-only`
checks all 96 legal scenario IDs, original execution/input receipts, image
checkpoint uniqueness and a pinned build's `build-source.json` before any GPU
launch. The matrix also requires the same version2 original collection
attestation used by state/audio comparisons and checks its execution, input,
scalar, pool, audio and terminal hashes before capture and comparison. The
aggregate retains a separate current local image-hash snapshot; primary image
hashes are not retroactive execution-time provenance. A fresh `--name` preserves
previous evidence. Its aggregate receipt and
per-scenario reports retain missing/scalar/image failures; capture completion
alone is not a fidelity pass.
Additional original screenshots retain a separate execution receipt in
`image-supplement.json`; preflight verifies its execution and image hashes,
unchanged input/EXE/terminal, matching complete scalar trace and final cadence.
The required matrix refuses a case without its actual terminal image.

`--complete-reference` records every replay-state observation, the original
pool checkpoint frames and the terminal event during that same render pass.
It is enabled automatically by `--require-matrix`; PNG and pool checkpoint
lists remain separate. Each case writes `execution.json` with the immutable
build, input, original collection and output hashes. The audio batch can read
these same executions with `--port-name` set to the capture directory name.
The aggregate reports `simulation_passed` and `image_passed` separately and
requires both for its overall `passed` value. Missing cases or an interrupted
matrix cannot produce a complete pass. Complete captures require a new output
name and fresh private storage; the default execution remains sequential.

To reproduce the browser isolation check, build the web package, serve the
repository root on localhost and open `/tools/midi-audio-probe.html`. Select
a legally supplied SoundFont and an original extracted MIDI file, click
**Start MIDI validation**, then **Block main thread for 2 seconds** after
playback starts. The page reports the actual sample rate, audio-thread frame
counter, nonzero PCM energy and processor errors. It neither uploads nor
stores the selected assets. Its isolated processor and `MidiSynth` are the
same files used by the game. The saved receipt used TimGM6mb and `th06_09.mid`;
the 102,400 measured frames include meter granularity around the 2-second
stall and the following 100 ms observation interval.
