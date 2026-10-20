# Playfield spike diagnosis — 20 October 2026

Board ticket #10 required a fresh native capture in each of three clusters,
identification against the decompiled original, and a correction or a precise
separate defect. Nine original BMPs and their execution receipts were copied
with human authorization and checked against the historical campaign hashes.
The original process was not rerun. Proprietary pixels, replays and archives
remain private and are not included here.

| Case / frames | Full-frame MAE before | Full-frame MAE after | Finding |
| --- | --- | --- | --- |
| replay-c0-d1-s2 / 9000–9002 | 6.1961–7.8145 | 2.8288–2.8348 | Perfect Freeze recoloring and pulse absent |
| replay-c0-d2-s3 / 14400–14402 | 3.8697–3.9551 | unchanged, identical PNG bytes | Boss sub-VM X rotation discarded; separate #12 |
| replay-c1-d2-s5 / 14400–14402 | 4.8829–4.9035 | 0.5361–0.5384 | Misdirection updated logical sprite but left rendered ANM blue |

All nine CSV scalar checkpoints and all nine JSONL pool snapshots are
byte-identical between the pinned baseline and candidate. HUD and FPS metrics
are unchanged. These snapshots omit bullet color and effect VM geometry, so
their equality did not detect the visual defects. Residual pixel errors remain;
the result is not pixel equality or a recertification of the full 96-case matrix.

## Corrected: bullet recoloring and Cirno pulse

`EnemyEclInstr.cpp`, `ExInsCirnoRainbowBallJank` (lines 17–49), spawns particle
12, ignores inactive/despawning bullets, sets the normal bullet color to white
and calls `SetActiveSprite` before freezing or releasing motion. The port
previously performed only the motion update. The patch restores the pulse,
white sprite and despawn exclusion.

`ExInsStage56Func4` (lines 186–268) sets eligible large bullets to green and
updates `sprites.spriteBullet`. The port changed `sprite_offset`/`sprite`, but
`Stage::draw` selects `bullet.visuals.normal.sprite` whenever an ANM exists.
That left the visible stars blue. A shared `Bullet::set_normal_color` now updates
the logical sprite and the normal ANM sprite together, preserving its clock and
the independently cached spawn/despawn animations. The same stale-ANM writes
in callbacks 9, 11 and 15 use this helper as well. No movement or RNG formula
is changed by those recolor assignments.

The added tests verify that a recolor reaches the runner used for drawing,
preserves the existing animation schedule and other state animations, and that
Perfect Freeze emits one pulse per call while leaving despawns and their RNG
consumption alone. `cargo test -p th06 --release`: 132 passed, 17 ignored;
`cargo build -p th06 --release`, formatting and diff whitespace checks pass.
Ignored tests require separate original-data/oracle fixtures and were not run.

## Separate #12: stage 3 boss halo perspective

The halo is an enemy sub-VM, not an EffectManager spell bubble. The original
`ecldata3.ecl` sub32 calls sub10; sub10 at time2 assigns slot4 to script66
(instruction offset4824). `stg3enm.anm` script66 sets sprite44, autorotation,
scale0.3 increasing to6.3 over60frames, rotation `(-pi/3,0,0)` and disabled
depth writes. The port's ANM opcodes9/10 retain only Z, and
`sprite_cmd_from`/`sprite_geometry::world` handle a rectangle rotated in Z.

Original `EnemyManager_OnDraw` (EnemyManager.cpp:709 onward) calls Draw2.
`AnmManager::Draw2` delegates to Draw3 when any rotation component is nonzero;
Draw3 (AnmManager.cpp:748 onward) composes X, Y and then Z rotations before
perspective projection. Keeping X/Y and projecting the four corners, including
depth and perspective-correct UVs, is the separate repair. Scaling an axis by
cosine alone would be an approximation. Texture normalization was investigated:
this atlas is256x256, so its dimensions alone do not explain the ellipse.

This patch deliberately leaves that geometry untouched. The three stage3 PNGs
remain byte-identical to baseline, establishing the boundary of this fix.

## Provenance and limits

The adjacent JSON receipt includes every original, baseline and candidate image
hash, per-region metrics, binary hashes, and checkpoint comparisons. Native
runs used menu entry, 32-bit color, the same replay input, hitbox off and the
silent capture path. Baseline binary SHA256:
`af6455963aabf949265e87f7e731ad418a10bc91882846ba31f5686f349e9f2e`.
Candidate SHA256:
`b85deab23ad50ee4ec74fbe2e47945c1cfa2e7bb258cb589664cbdee0db3a3fe`.

The implementation branch is based on the ongoing full-game feature history,
including the separately reviewed FPS entry fix. Remote main predates that
history; merging the whole branch would also include roughly202 prior files
and is not represented as this bounded patch. No main merge, browser/WASM
rebuild, server deployment, fresh original execution or full campaign is
claimed by these native results.
