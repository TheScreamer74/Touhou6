# Web build

The page loads the hosted game set through `assets.json` and starts the game
automatically. It no longer asks the player to select a game folder. Scores,
settings and replays stay in browser storage. The **Scores et replays** button
opens local import and export tools for `score.dat` and the 15 replay slots.
Audio starts after the first key press or click, as required by browsers.

## Build

```sh
wasm-pack build crates/game --release --target web --out-dir ../../web/pkg
```

The output is `web/pkg/th06.js` and `web/pkg/th06_bg.wasm`. Serve the directory
over HTTP; the module cannot be opened through `file://`.

```sh
cd web
python3 -m http.server 8080
```

## Hosted game files

`assets.json` is required. Its `files` object maps the names consumed by Rust
to same-origin URLs, relative to the page. The page downloads these files,
shows progress and starts the game without a launch button. If loading fails,
**Réessayer** reloads the page. Example manifest:

```json
{
  "files": {
    "TL.DAT": "res/TL.DAT",
    "CM.DAT": "res/CM.DAT",
    "ST.DAT": "res/ST.DAT",
    "IN.DAT": "res/IN.DAT",
    "ED.DAT": "res/ED.DAT",
    "MD.DAT": "res/MD.DAT",
    "th06e_ST.DAT": "res/th06e_ST.DAT",
    "th06_01.wav": "res/bgm/th06_01.ogg",
    "th06_02.wav": "res/bgm/th06_02.ogg",
    "th06_03.wav": "res/bgm/th06_03.ogg",
    "th06_04.wav": "res/bgm/th06_04.ogg",
    "th06_05.wav": "res/bgm/th06_05.ogg",
    "th06_06.wav": "res/bgm/th06_06.ogg",
    "th06_07.wav": "res/bgm/th06_07.ogg",
    "th06_08.wav": "res/bgm/th06_08.ogg",
    "th06_09.wav": "res/bgm/th06_09.ogg",
    "th06_10.wav": "res/bgm/th06_10.ogg",
    "th06_11.wav": "res/bgm/th06_11.ogg",
    "th06_12.wav": "res/bgm/th06_12.ogg",
    "th06_13.wav": "res/bgm/th06_13.ogg",
    "th06_14.wav": "res/bgm/th06_14.ogg",
    "th06_15.wav": "res/bgm/th06_15.ogg",
    "th06_16.wav": "res/bgm/th06_16.ogg",
    "th06_17.wav": "res/bgm/th06_17.ogg"
  }
}
```

The `.wav` keys identify tracks; their bytes may be Ogg for browser decoding.
Keep the original sample rate when converting music so `.pos` loop points
remain correct. The browser reads that rate from the WAV/Vorbis header before
decoding and converts loop frames to seconds; Web Audio may resample the buffer
to the output device rate (for example 48 kHz). `MD.DAT` contains `musiccmt.txt` (Shift-JIS) and the 17 `.pos`
files (two little-endian u32 sample offsets each). Loose `.pos` files and
`musiccmt.txt` are also supported. `ED.DAT` contains the six endings and the
staff roll. `th06e_ST.DAT` supplies translated stage text when present.

The actual original MIDI files are read from `MD.DAT`. To enable MIDI playback,
add a separately licensed General MIDI SoundFont as `"gm.sf2": "assets/gm.sf2"`.
The hosted build uses TimGM6mb; distribute its full GPL-2 license and attribution
beside the bank. The bank is separate from the MIT engine source and is not
committed here. MIDI renders in an isolated AudioWorklet; ScriptProcessor is the
compatibility fallback. See [audio/display details](../docs/PRESENTATION_FIDELITY.md).
Optional `msgothic.ttc`/`msgothic.ttf` files are recognized when supplied by the
host through the manifest; the default font is Noto Sans JP.

Downloads use two concurrent streams, report progress, abort together on an
error, and enforce a 64 MiB per-file / 1 GiB total input limit. The engine and
decoded audio need additional memory. Hosted files use the HTTP cache. The
loader does not store game resources in IndexedDB. Scores, settings and replays
use localStorage independently of the hosted files; imports and exports remain
local and do not upload anything to the host.

## Deployment and updates

Upload `index.html`, `midi-worklet.js`, `pkg/`, and `assets.json`
with its referenced files. Serve `.wasm` as `application/wasm`. To publish a new engine build, add
`build.json` beside the page:

```json
{"version":"git-commit-or-build-id"}
```

The page fetches this small file without caching and uses its version in the
JS, WASM and MIDI worklet URLs. Change the version only after both generated files have
been uploaded. An engine update does not invalidate HTTP asset URLs, scores
or replays. Serve `index.html` with revalidation or
`Cache-Control: no-cache`; `build.json` alone cannot refresh an already cached
old HTML page. Content-hashed asset URLs may be used independently in the
manifest when artwork or music changes.

The game stays public. An optional, independently hosted community interface is loaded from `/platform/community.js`. Its source and service belong to the host, outside this repository. The adapter exposes only language, suspension and completed-run metadata hooks; a missing shared interface does not prevent playing.

## Language and hitbox options

The in-game Options menu offers the original Japanese presentation and French,
English, German and Dutch. Text catalogs cover stage dialogues, spell names,
music titles/comments, endings and staff roles. Brand artwork and proper names
remain original. Language changes reuse texture slots, with only one active
language rasterized. New web profiles start in French; older options retain their
language choice and migrate without discarding gameplay settings.

The optional hitbox can be hidden, shown while focusing, or always shown during
live play. Its 2.5-pixel center marks the original collision box. It changes only
the overlay. It does not change collisions, scores or replay inputs.

The native capture tool accepts `--language ja|fr|en|de|nl` and
`--hitbox off|focus|always`. Use a scratch game directory for menu scenarios,
since normal Options interactions save their settings.

## Compatibility limits

Music playback supports original WAV tracks (or hosted Ogg conversions) and
original MIDI with a separately supplied SoundFont. Both 32-bit color and the
original limited-precision R5G6B5 mode are supported. Browser fullscreen depends on user activation and
browser permission; the F key and on-screen button request it directly.
Japanese text defaults to Noto Sans JP; a host-supplied MS Gothic font is
optional, with rasterization still differing from Windows GDI.

Replay files use the original T6RP format. Deterministic recording/playback is
regression-tested on all 96 routes. The bundled original stage-4 replay also
matches all 3,720 observed scalar states against the original executable. That
scoped comparison does not establish all-route or pixel/audio equivalence.
