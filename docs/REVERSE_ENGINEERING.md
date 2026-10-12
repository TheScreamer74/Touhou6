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
configuration was changed, and no original Windows target was executed.

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
9,132,905-byte deployed WASM exceeds REA's default 8 MiB input budget. Raising
only that local tooling cap to 16 MiB allowed input processing, but WABT's WAT
output then exceeded the unchanged 32 MiB output budget. No complete deployed
WASM recovery is claimed. The official Ghidra conformance verifier encountered
an upstream error-message assertion mismatch; provider doctor and actual
function decompilations succeeded, rather than the complete verifier suite.

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
tolerances were not rewritten. `Catk.characterShotType` in the engine stub was
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

## Complete route regression

The final original-asset battery exercised all 96 legal stage/difficulty/shot
combinations: stages 1–5 in Easy/Normal/Hard/Lunatic, stage 6 in Normal/Hard/Lunatic,
and Extra with each of the four shots. An invulnerable automated player reached
`MSG` termination / `NextStage` in every case. Each recorded T6RP was parsed and
replayed; 1,482,262 replay frames matched the recorded simulation and draw-command
hashes (2,964,524 simulation frames total). The hash includes RNG, enemies,
projectiles, items, score and player death/spawn/invulnerability state. This is an
internal determinism/coverage test, not comparison to the Windows executable.

Reproduce with extracted ST/CM/IN data as documented in the ignored test:

```sh
TH06_GOD=1 TH06_REFERENCE_DIR=/path/to/extracted-reference \
  cargo test -p th06 original_assets_all_routes_and_replays_match_every_frame -- --ignored --nocapture
```

## Remaining fidelity limits

| Area | Observed limitation |
|---|---|
| Player shots | Hits use a circular test and remove each shot on first impact; original `Player::CalcDamageToEnemy` uses boxes and retains Marisa A missile explosions with declining damage |
| Marisa B beams | Collision and damage remain simplified columns |
| Bullet cancellation | Bomb cancellation removes bullets immediately; the original cancellation/impact fade is not fully reproduced |
| Graze | Original `ScoreGraze` rank +6, particles and caps are partial; laser graze is counted each frame instead of the original modulo-12 cadence |
| Replay equivalence | Internal recording/playback determinism is tested; equivalence to Windows replay execution has not been established |
| Presentation/audio | Noto Sans JP replaces GDI/MS Gothic; MIDI synthesis and 16-bit display mode are unavailable |

The integration browser check reported by the root task loaded 24 hosted assets
(about 51 MB, including 17 Ogg tracks), started the game, exercised Options/SFX,
and imported the original `CM/demo00.rpy`. This checks startup and those flows;
it is not a browser playthrough of every stage/character/difficulty.

These checks do not compare every rendering effect, complete GUI behavior,
all difficulty/character combinations, or Windows audio/font behavior. See
[oracle/README.md](../oracle/README.md) for reproducible commands and inputs.
