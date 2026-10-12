# Original display and MIDI modes

The port now supports the original 16-bit display option and actual MIDI
playback. These are separate from the WAV/Ogg path; the 44.1 kHz source-rate
loop correction remains in place for browsers running at 48 kHz.

## R5G6B5

`main.cpp` selects an R5G6B5 back buffer. `AnmManager::LoadTexture` selects
A4R4G4B4 for UNKNOWN/A8R8G8B8 and R5G6B5 for R8G8B8 in this mode; an explicit
A1R5G5B5 format retains its single-bit alpha.

`engine::color565` stores integer 5/6/5 channel codes in an RGBA8 carrier.
It reads the previous destination, applies the original alpha or additive
blend, and quantizes the result after each overlapping quad. It also keeps
the original texture channel codes and interpolates their expanded values.
This avoids treating the mode as a filter applied only to the completed frame.
Only consecutive quads whose conservative screen rectangles do not overlap
can share a pass. Background depth survives every pass. Presentation scales
the logical 640×480 buffer to the window.

GPU-dependent edge, interpolation and color-conversion rounding from a 2002
Direct3D driver cannot be proven identical on Metal/WebGL2. The renderer uses
nearest 5/6/5 destination quantization and the source texture format's high
bits. This implements the actual limited-precision mode, without claiming
bit identity with every original GPU.

The original enemy hit flash uses the texture-stage `D3DTOP_ADD` color
operation. Both display paths add VM RGB to sampled texture RGB and clamp
each channel, while retaining texture-alpha × VM-alpha. This operation is
independent of the framebuffer's additive blend mode.

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
`TH06_SOUNDFONT=/absolute/path/to/bank.sf2`. Browser folder imports recognize
`gm.sf2`; hosted manifests can provide the same name. MIDI is enabled only
when the bank and all 17 original MIDI files validate. The browser backend
instantiates the same compiled WASM module in an AudioWorklet and synthesizes
stereo on the audio rendering thread. Its 128-frame blocks use the actual
AudioContext sample rate. An Encoding API fallback loads before wasm-bindgen
because Chromium omits TextDecoder/TextEncoder from AudioWorkletGlobalScope.
The browser explicitly waits for audio-node installation before the first
title track, including when MIDI was persisted as the selected mode. The
game thread only sends bank/track/fade/stop messages. A 2048-frame
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
files or a font selected by `TH06_FONT`; browser folder imports recognize
those two filenames. Proprietary fonts are never bundled or hosted by default.
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
