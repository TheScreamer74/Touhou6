# Original executable scenario comparisons

This harness runs the supplied **TH06 1.02h executable** in an isolated Wine
installation. It records simulation state, object pools, screenshots and audio
requests from that executable. Bring your own game files; neither original
assets nor generated screenshots belong in Git or the public deployment.

The reference executable SHA-256 is
`7f38496b31b8625196900a69cd1bfed243cab1f9612073e7881dc354b876fd39`.
Wine rendering and its installed fonts are part of the reference environment;
a Wine image is not proof of identical output on every Windows graphics driver.

## Instrumentation and timing

A `d3d8.dll` proxy forwards Direct3D8 calls and observes the back buffer at
Present. It writes unscaled 640×480 BMPs, replay-frame scalar CSVs, fixed-slot
object pools, scene changes and the configured checkpoints. Binary layouts
and detour prologues are checked for the supported executable.

The original executable file is unchanged. The proxy replaces its imported
clock and keyboard functions in memory so a declared input script is
reproducible. DirectInput creation fails deliberately, selecting the original
keyboard fallback. Every such hook is instrumentation, not an unmodified
Windows execution. Audio hooks separately observe MIDI messages, WAV requests,
SFX variants and dispatch. Some Wine jobs require an output-device shim;
these receipts establish requests and dispatch, not physical speaker output.

`key <start> <end> <name>` and `tap <first> <period> <count> <name>` use Present
indices. Ordinary windowed gameplay calculates once per Present. Replay
dialogue can restart the calculation chain and calculate up to three times.
**Compare replay-frame IDs, not a fixed offset from Present.** The game draws
before calculating, so the image associated with a post-calculation checkpoint
shows the preceding state. The port capture records both checkpoint and draw
frame/scene. Newly registered scenes receive neutral input on their first
calculation, matching `Supervisor::OnUpdate`.

The synthetic clock advances from Present count, with a declared creep term
for original busy-waits that cannot Present. This stabilizes simulation; it
does not measure real-time performance. Fullscreen 16-bit captures also record
the actual back-buffer format and runtime configuration. The original windowed
mode uses the desktop back-buffer format even when its color option is 16-bit.
The separate advanced texture-precision and software-blending flags must be
reported independently.

## Complete stage inputs

`scenarios.py` accepts checksum-valid original T6RP files and launches them
through the real Replay menu. It does not transplant a stage or player state.
Each job has a private game copy, Wine prefix, display, score/config and logs.
A declared invulnerability hook permits full-stage coverage; separate cases
must cover deaths, bombs, continues and results without that hook.

The predefined matrix contains 96 legal stage/difficulty/shot combinations:
Easy stages 1–5, Normal/Hard/Lunatic stages 1–6, and Extra, with all four shots.
`docs/fidelity-scenarios.json` defines required coverage; it is not a pass
receipt. Scenario execution records EXE/proxy/input/config hashes and requires
a genuine original terminal. A single-stage replay cannot establish the
following stage transition: use an actual multistage input from
`make_transition_replay.py` / `merge_replays.py` for that scenario.

```sh
python3 oracle/refcap/scenarios.py --help
python3 scripts/collect_scenario_references.py --help
```

The collector downloads completed private evidence only, without game assets
or Wine prefixes. Wait for a transfer to finish before extracting its archive.

Freeze the source, build the native port, and pin it before comparisons:

```sh
cargo build -p th06 --release
python3 scripts/pin_port_build.py
python3 scripts/run_port_references.py --help
```

`run_port_references.py` uses the same replay and declared overrides, private
storage, and a pinned executable. Its strict comparator checks every observed
scalar, required pool checkpoint and terminal. RNG, counts and score are exact;
positions allow 0.0002 pixels. Missing endpoints, holes, duplicate IDs, nonfinite
values and incomplete references fail. An executable exiting is insufficient.

## Images

```sh
python3 scripts/capture_port_scenario.py --help
python3 scripts/compare_capture_scenario.py --help
python3 scripts/compare_capture_images.py --help
```

The capture tool requests the checkpoint IDs actually reached by the original
and freezes its executable. Explicit reruns remove old generated images and
receipts, preventing an interrupted rerun from inheriting a previous pass.
Whole-frame, playfield, HUD and FPS measurements retain text and transitions;
there is no image alignment, resizing or hidden masking. The comparison report
has an explicit `passed` field. The scenario comparator CLI returns exit code 1
when images are missing, scalar checkpoints differ, or pixel criteria fail; it
still writes the full measurement report. Consumers should inspect the report
for the scope and cause of any difference.

Direct `--replay` entry omits the preceding Replay menu draw. That first image
remains in the measurements and is reported as an entry-boundary mismatch;
menu-entry scenarios are needed to compare the actual transition.

## Endings, menus and sound

`ending_scenarios.py` declares a one-time ending entry from an active original
Replay. It sets the selected ending conditions, then observes the original END
interpreter through `@z` without modifying its timers, scripts or artwork.
It verifies END behavior after entry, not the preceding story clear or score
persistence. `ending_trace` reproduces this scope in the port.

```sh
cargo build -p th06 --release --example ending_trace
python3 oracle/refcap/ending_scenarios.py --help
python3 scripts/verify_ending_reference.py --help
python3 scripts/verify_options_reference.py --help
python3 scripts/verify_audio_reference.py --help
```

END comparisons require every tick and a newly generated terminal. Menu scripts
live under `scenarios/`; Options comparisons use its displayed pending window
mode, since the original defers committing that value until leaving the menu.
Audio comparisons distinguish event order/timing, SFX dispatch/gain, MIDI
messages, and rendered PCM/loop boundaries. Matching MIDI messages does not
establish timbre equality with a missing Windows MIDI bank. The hosted Japanese
font and free SoundFont are documented in `docs/PRESENTATION_FIDELITY.md`.
Before the required audio matrix, run
`scripts/attest_original_audio_reference.py <private-scenario-directory>`.
It checks the exact legal96 remote audio/state/pool/input/terminal/receipt
snapshot against the local mirror without rewriting historical execution
receipts. The previous dated attestation is preserved when collecting a new
snapshot. Both the audio batch and `run_port_references.py --require-matrix`
or `--name port-final` require the current version2 attestation. Original CSV
and pool hashes are checked before native execution and again immediately
before comparison; execution/comparison/aggregate receipts bind its SHA.
An older native receipt without that binding requires an explicit rerun.
`scripts/attest_reference_assets.py` separately compares current DAT/WAV
snapshots. The batch binds that attestation and checks the current native
storage assets with streaming hashes cached across resolved symlinks. This
later snapshot does not add asset hashes to historical execution receipts.
Use `verify_audio_reference.py batch <directory> --port-name <diagnostic-name>`
to inspect a candidate capture directory; all provenance, clock and ordering
checks remain enabled, and the batch still requires one immutable binary.

Natural pause, continue, Practice and result paths can use `transition_trace.h`
and `scripts/verify_transition_reference.py`. Original Present `p` corresponds
to completed native update `p+1`; this is a fixed input/draw convention, never
an offset inferred from matching events. Required intervals reject missing
observations and compare the nine transition fields without tolerance. The
optional stage comparison also checks every passive gameplay observation,
including the final calculation before leaving the stage.

For a declared highscore/save fixture, `scorefixture <present> <score>` writes
only `GameManager.score` before that input calculation. Native diagnostics use
`--score-fixture <present>:<score>`. This does not modify Result state/timers,
recording, lives, RNG or the rolling GUI score. The receipt must state the
fixture and initial storage; an empty replay slot and an occupied slot follow
different original paths. `scripts/verify_saved_reference.py` verifies both
file checksums and boundaries, then compares replay metadata, stage headers,
all inputs through the first sentinel and the declared highscore records.
Ignored inputs after the first sentinel are reported separately, since the
original can call `StopRecordingReplay` twice. Byte equality is a separate
property from interpreted playback equality.

When original saved files remain on the reference server, run the same
`verify_saved_reference.py` parser there in memory and collect its checksum,
execution, parser and canonical semantic hashes. The canonical object contains
the replay metadata, including its exact creation date, all interpreted stage
inputs and the declared highscore records. `scripts/verify_saved_aggregate.py`
compares that aggregate with actual native saved files without recreating any
original file. Bind the received JSON to its recorded hash:

```sh
python3 scripts/verify_saved_aggregate.py original-aggregate.json native-saved \
  --expected-original-sha256 "$TH06_ORIGINAL_AGGREGATE_SHA256" \
  --output saved-comparison.json
```

Generate fresh original and native saves on the same date. Their creation dates
remain part of the exact comparison; older dated references stay separate.

`scenarios.py --diagnostic-gui` enables `guifile gui.csv`, a read-only Present
observation of boss visibility, health-bar state/target/display, opacity, lives,
timer, dialogue and VM19 animation state. It adds no hooks or game/clock calls.
The native capture CLI writes the matching fields to `boss-ui.csv`.
`scripts/compare_boss_ui.py` compares post-calculation `(stage, frame)` IDs;
these fields describe the state after the earlier bitmap draw. GUI lifetime
must be measured directly: an enemy slot may remain occupied after
`Gui::bossPresent` clears. Separate probe execution and proxy hashes preserve
the provenance of earlier reference captures.

## Legacy manual capture

`build.sh`, `run.sh` and `convert.sh` remain available for exploratory captures.
The config accepts `capdir`, `capstart`, `capend`, `capstride`, `capreplay`,
`statefile`, `poolfile`, `menufile` and the scripted keys above. `stopafter`
limits an isolated job. See `th06cap.example.txt` and the proxy parser.

Legacy `practice` unlock and `forcestage` writes are exploratory conveniences;
they must be declared and cannot stand in for natural progression, unlock or
menu fidelity. Prefer real replay/menu inputs for proof. Wine initialization
failures are recorded as failed jobs and require explicit reruns.
