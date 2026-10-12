# Reference analysis and validation — 2026-10-12

The port uses the original game data and matching decompilation as its source
of behavior. Focused binary observations, reference comparisons and runtime
checks have different scopes; none is a whole-game equivalence proof.

## REA and original executable

REA was prepared in an isolated temporary tooling directory. Its stable npm
package is `reverse-engineer-anything@6.4.0`; the WebAssembly inspection
capability was taken from the explicitly pinned source commit
`b3dec005bcb89b5afe6e0aa41e5251f8a3a31de7`. Node 24.11.0, Ghidra 12.1.4,
JDK 24.0.1 and WABT 1.0.42 were used. The macOS ARM64 Ghidra native decompiler
was built from the official archive's included C++ source. No personal MCP
configuration was changed. The original executable was subsequently run in an
isolated Wine/Xvfb reference environment on the authorized server; see the replay
comparison below.

The original `102h.exe` supplied with the game has SHA256
`7f38496b31b8625196900a69cd1bfed243cab1f9612073e7881dc354b876fd39`.
The public REA CLI's `function` command, with provider `ghidra`, successfully
inspected `Ending::ParseEndFile` at `0x40f7c0`. Evidence ID:
`ev_84a19ffb89ed6f638e594c58df7491eb63ed1ccc744701ec4ec4c2c4c5e370c8`.
Its decompiler observations corroborate the source's two-byte text reads,
32-byte half-lines, 16 ANM slots, text script base `0x708`, shadow `0xc0d0d0`,
minimum-wait branches and `@F` switch falling through to sprite clearing.
Decompiler output is an observation, rather than original source text.
The invocation is reproducible with the prepared CLI:

```sh
rea function "/path/to/102h.exe" 0x40f7c0 --provider ghidra --format json --full-output
```

Related source-map addresses for v1.02h:

| Function | Address |
|---|---|
| ReadEndFileParameter | `0x40f4d0` |
| FadingEffect | `0x40f550` |
| ParseEndFile | `0x40f7c0` |
| LoadEnding | `0x4106d0` |
| Ending_OnUpdate | `0x4109c0` |
| Ending_OnDraw | `0x410a70` |
| Ending_AddedCallback | `0x410b10` |

REA also inspected the deployed HTML/JavaScript: two files, 14664 AST nodes,
zero parse failures, complete inventory but 3716 semantic unknowns. The exact
9,132,905-byte initially deployed WASM exceeded REA's default 8 MiB input budget. Raising
only that local tooling cap to 16 MiB allowed input processing, but WABT's WAT
output then exceeded the unchanged 32 MiB output budget. The initial run therefore
did not complete WASM inspection. The official Ghidra conformance verifier encountered
an upstream error-message assertion mismatch; provider doctor and actual
function decompilations succeeded, rather than the complete verifier suite.

The follow-up tooling patch raises the separate binary/text budgets to 16/256 MiB
and reports the configured limits directly. Both the full-game build and the
music-loop correction now pass complete static inspection: 13 sections and exact
WAT output matching an independent WABT invocation (121,217,568 and 121,677,166
bytes respectively). A generated 12 MiB module also passes, exercising both old
limits without game assets. The 21 adapter tests and 42 real CLI/MCP checks pass.
This is complete decoding of the selected module, not recovered Rust source or
a proof that its behavior matches Windows. The JavaScript semantic unknowns
above still require separate investigation. See [the tooling repair](REA_TOOLING.md)
and [capacity receipts](rea-capacity-2026-10-12.json).
The Ghidra follow-up also passes its complete host-native lane after repairing
the stale verifier assertion and compiling the distribution's missing ARM64
GNU demangler from its bundled source: 370 successful calls and 52 checked
rejections, with complete cleanup. [Receipt](rea-ghidra-2026-10-12.json).

## Music-loop correction

Web Audio's `decodeAudioData` resamples music to the device rate. The previous
code divided original `.pos` frame counts by the decoded buffer rate, which
shifted loop times when a 44.1 kHz track was decoded at 48 kHz. The title track
returned to its loop 4.811 seconds too early. The corrected engine reads the
encoded WAV/Vorbis rate and converts the original frame counts to seconds before
the asynchronous decode. Generation IDs also discard stale A → B → A decodes.

All 17 hosted Vorbis files retain the corresponding original WAV's frame count
and source rate; every `.pos` range is inside its track. Thirty-four browser
render checks (17 tracks at 44.1 and 48 kHz) straddled the loop boundary and
matched the expected decoded PCM alignment within 0.49 output samples. Six
focused audio tests and all 60 workspace unit checks pass. This checks loop
timing and rendered output, without claiming subjective listening results or
lossless Vorbis equivalence. [Browser receipt](audio-loop-2026-10-12.json).
The actual corrected engine was also observed using a 48 kHz context and buffer
while keeping the title's original loop at 6.000340136–59.214625850 seconds.
The asset-free local browser harness is reproducible with
`scripts/verify_music_loops.py --help` and the separately supplied game files.

REA workflow sources: [installation](https://github.com/morluto/rea/blob/b3dec005bcb89b5afe6e0aa41e5251f8a3a31de7/docs/installation.md),
[CLI](https://github.com/morluto/rea/blob/b3dec005bcb89b5afe6e0aa41e5251f8a3a31de7/docs/cli.md),
[skill](https://github.com/morluto/rea/blob/b3dec005bcb89b5afe6e0aa41e5251f8a3a31de7/.agents/skills/reverse-engineer-anything/SKILL.md).

## Original endings

Nine unit tests cover ending selection, clear/pause wait ordering, Ctrl's
five logical steps, two 32-byte text slots, script switching, scroll clamping
and fades advancing per draw. An integration check loads original ED/IN
resources, follows all six endings into `staff00.end`, observes `th06_16` then
`th06_17`, displays final staff sprite 16, reaches `@z`, and ends at background
y = 0. Metal screenshots were inspected for artwork, positions and overflow.
The existing Noto text renderer remains a visible difference from GDI.

## Locked C++ oracle reference

The native harnesses target
[`happyhavoc/th06` commit bd90685fa40eaf2e16789dcf788843b304e6e02d](https://github.com/happyhavoc/th06/tree/bd90685fa40eaf2e16789dcf788843b304e6e02d),
before upstream merged the RNG/timer files expected by the harness.
The downloaded source archive SHA256 is
`f09629ea2d227bbc9430c03dc51f87e2cc408e6cf6337797f449fcfde225e3cf`.
Source copying, existing rendering stubs and 64-bit pointer adaptations are
local to generated build directories. Interpreter formulas and comparison
tolerances were not rewritten. The harness-only `utils::Rotate` stub was corrected
to the original function's handedness before the projectile comparisons. `Catk.characterShotType` in the engine stub was
corrected to the five-entry array declared by the reference.

The standalone bullet battery compares 139 emitted projectiles over all nine
angle modes with the port's real `spawn_bullet_pattern`: comparison output is
byte-identical. Background ANM state was compared over 600 frames per script
for stages 1–7: 71 scripts / 42600 states were identical at the harness's existing
formatted precision. Background camera/fog state matched over 1200 frames on
each of stages 1–7.
At frame 300, vertex comparisons on all seven stages checked 7577 quads with
maximum difference 0.010 px, within the existing 1.5 px threshold; 390 quads
were excluded by the unchanged near-plane/off-screen guards.
Full-VM runs use seed 0x1234, Normal rank 16, Reimu A, player (192,384),
no input, invulnerability and dialogue stubs. Two item-table initialization
RNG draws are included on both sides. The original player spawn is y = 384
(`Player.cpp`: `448 - 64`), and the port's debug mode explicitly sets the seed.
The partial VM harness stubs item motion/collection/loss and graze behavior;
those systems can change rank in the actual game. A difference beyond those
omissions cannot be assigned to the ECL port from this harness alone. Results and unresolved observations are recorded in [validation-2026-10-12.json](validation-2026-10-12.json).

The stage 5 VM run executed 6000 frames. Its first strict formatted difference
is at frame 1202: one x coordinate is 8.0908 versus 8.0909 (0.0001 px).
At frame 3574 a bullet speed is 3.2313 versus 3.2000, followed by a count
difference at frame 3608. The C++ harness's `ItemManager::SpawnItem` and
`OnUpdate` are no-ops, while the port applies original item-loss subrank changes
(`ItemManager.cpp`:145). The strict comparison is therefore reported as a
divergence under a partial harness, not a complete pass or proof of an ECL bug.
The report fingerprints both executables, the ECL input and a working-tree
source manifest whose before/after hashes matched during execution.

## Original executable replay comparison

The supplied, unmodified v1.02h executable runs in an isolated Wine 11.0 WoW64 /
Xvfb environment on Ubuntu 22.04 x86_64. The Direct3D8 capture proxy reads manager
state at Present; the reference uses deterministic timing and keyboard hooks,
with god mode, practice unlocks and forced-stage patches disabled. Audio is
disabled only in this reference copy's configuration. Original game assets and
executables stay outside Git and outside the public release.

All 3,720 observations of the original title's bundled Reimu A / Lunatic /
stage-4 demo match the port: position (0.0002 px tolerance), score, stocks, power,
graze, rank/subrank, RNG, projectile/item counts and hit-particle counter.
Comparison uses ReplayManager frame IDs. The capture and CLI also handle
dialogue calculation-chain restarts. This exposed and corrected missing effect sprite RNG, periodic rank
phase, viewport-dependent culling and overlapping screen-shake chains. It checks
these runtime observations for this replay, without claiming all-route Windows
identity or a pixel/audio comparison.
[Receipt](replay-reference-2026-10-12.json),
[reproduction](../oracle/refcap/README.md).

## Player, projectiles and effects

The player attack pool now follows the original 80 slots and all 193 table
entries. Collision uses inclusive rectangles; Marisa A missile explosions retain
and weaken their damage, while Marisa B beams persist, pulse and use their actual
ANM lifetimes. Focus orb transitions, homing, bomb damage, portraits, name bars,
flash textures and bomb firing locks follow the original functions and resources.
Five compiled source oracles compare table values, damage, movement, orb states
and dynamic viewport bounds using exact float bits. See
[the player harness](../oracle/player/README.md).

Enemy projectiles preserve spawn/fade ANM lifetimes, cached velocity acceleration,
first-tick timing, original laser state fallthrough and midpoint collision behavior.
The source oracle compares 864 scenarios over 85 frames (73,440 observations),
including graze cadence, rank, scoring, cancellation and laser alpha. Thirty
original-asset spawn/fade combinations also pass. Graze popups, effect sprite RNG
and screen-shake viewport dependencies are covered separately. See
[the projectile harness](../oracle/projectiles/README.md).

## Complete route regression

The original-asset battery exercises all 96 legal stage/difficulty/shot
combinations: stages 1–5 in Easy/Normal/Hard/Lunatic, stage 6 in Normal/Hard/Lunatic,
and Extra with each of the four shots. An invulnerable automated player reaches
`MSG` termination / `NextStage` in every case. Each recorded T6RP is parsed and
replayed, comparing every simulation and draw-command hash. This is an internal
determinism/coverage test; it does not compare all 96 routes to Windows. The latest
run checked 1,375,027 live and 1,375,027 replay frames (2,750,054 combined) in
141.30 seconds. [Route receipt](routes-2026-10-12.json).

Reproduce with extracted ST/CM/IN data as documented in the ignored test:

```sh
TH06_GOD=1 TH06_REFERENCE_DIR=/path/to/extracted-reference \
  cargo test -p th06 original_assets_all_routes_and_replays_match_every_frame -- --ignored --nocapture
```

## Remaining fidelity limits

| Area | Scope and remaining limit |
|---|---|
| Replay equivalence | Internal recording/playback is checked on all 96 routes; original executable observations cover the bundled stage-4 replay only |
| Japanese font | MS Gothic can be supplied separately; the hosted fallback is Noto Sans JP, and rasterization differs from Windows GDI |
| MIDI timbre | Original MIDI messages and loop timing are supported; the hosted free TimGM6mb bank differs from proprietary Windows MIDI banks |
| GPU output | Original 16-bit texture/back-buffer precision is implemented; driver-specific edge and conversion rounding can differ on Metal/WebGL2 |

Actual MIDI, its isolated browser audio thread, original SFX variants, the
R5G6B5 renderer and their focused checks are documented in
[presentation fidelity](PRESENTATION_FIDELITY.md). The Windows font/bank files are
not part of the supplied game archive and are not bundled.

The current browser check loads 25 hosted assets (including the separate MIDI
bank), starts the game, enables 16-bit rendering and exercises Marisa B shots
and Master Spark. A saved MIDI setting also starts through AudioWorklet after a
real reload. The preceding build additionally checked replay import and durable
score/replay storage. This checks startup and those flows;
it is not a browser playthrough of every stage/character/difficulty.

These checks do not compare every rendering effect, complete GUI behavior,
all difficulty/character combinations, or Windows audio/font behavior. See
[oracle/README.md](../oracle/README.md) for reproducible commands and inputs.
