# Enemy ANM X/Y perspective — ticket #12

The stage3 boss halo uses enemy sub-VM4 / stg3enm script66. Its X rotation is
-pi/3; EnemyManager replaces only Z when autorotation is enabled. Previously
the interpreter discarded X/Y and the renderer could only rotate a flat
rectangle in screen space. The halo was circular instead of a tilted ellipse.

The interpreter now retains X/Y rotations and angular velocities. Enemy main
and sub-VM drawing selects projected corners when either tilt component is
nonzero, composing X, Y and Z in the order used by original Draw3. The original
256-unit base quad is scaled by sprite/texture dimensions. Translation,
corner anchoring and the half-pixel offset remain in their original spaces.

`DrawCmd.projected` carries homogeneous pixel coordinates, full clip Z and
camera W. The GPU receives W instead of a pre-divided affine approximation,
so it clips at the original near/far planes and interpolates texture UVs with
perspective. The projected path uses viewport depth0..0.5, preserving the
foreground depth range without weakening far clipping. Both the normal and
R5G6B5 renderers retain command order, blend mode, alpha test, scissor, depth
test and depth writes. R5G6B5 overlap bounds divide by W conservatively; a quad
crossing the camera receives full-screen bounds.

Screen shake rebases each corner using its own camera distance. Sprites with
X/Y zero keep the existing dst/rot calculation, W=1 and viewport depth0..1.
The new `projected: None` fields in unrelated constructors are mechanical
initialization of that unchanged path. Backgrounds retain their separate
existing 3D renderer. This change wires the new path to enemy ANMs; it does not
claim to migrate every other sprite owner to Draw3.

## Verification

Nine fresh silent native captures were compared with the same authorized,
hash-verified historical original images used for #10:

| Case | Frames | Full-frame MAE before | After | State |
| --- | --- | --- | --- | --- |
| replay-c0-d2-s3 | 14400–14402 | 3.8697–3.9551 | 0.6522–0.6683 | Halo tilt restored |
| replay-c0-d1-s2 | 9000–9002 | 2.8288–2.8348 | identical PNG bytes | Control |
| replay-c1-d2-s5 | 14400–14402 | 0.5361–0.5384 | identical PNG bytes | Control |

All nine scalar CSV checkpoints and all nine pool JSONL snapshots remain
byte-identical to the #10 candidate. HUD/FPS metrics are unchanged. The adjacent
JSON receipt includes all image hashes and regional metrics. The three original
processes were not rerun; the captures are fresh native comparisons against
historical originals, not fresh original execution.

`cargo test -p th06 -p th06-engine --release`:156 passed,31 ignored by default.
Three targeted GPU tests were also explicitly run and passed in both color
modes: the new perspective-UV/depth/far-clipping test, the existing sprite
depth/alpha/reset test, and the existing clip-change/empty-clip test. New unit
coverage checks X/Y velocity after the owner replaces Z, X→Y→Z composition,
and per-corner camera rebasing during shake.

Release native build, formatting, diff whitespace and the WASM **library**
check pass. An initial unrestricted `cargo check --target wasm32-unknown-unknown`
attempt also selected the native capture binary and failed on native-only
image/Engine APIs. The web target is the cdylib (`--lib`); its check passed.
No WASM package build, browser runtime or deployment is inferred from that
compile check.

Baseline binary SHA256:
`b85deab23ad50ee4ec74fbe2e47945c1cfa2e7bb258cb589664cbdee0db3a3fe`.
Candidate SHA256:
`1b0eac4d90d015a800fa3f7304006b16f9e25c0ef8f78d5096ea9fbcbb823908`.
After adding the final test, a fresh release build reproduced the exact binary
used by these captures. `scripts/pin_port_build.py` saved its source receipt;
all runtime source hashes were rechecked. Source fingerprint:
`9e9711fee29079327f352beeecee65013388b23de7895e25b6efc0bcdf115d88`.

Residual pixels remain (notably existing shot/HUD texel differences); this is
not a claim of exact pixels or complete96-case recertification. Proprietary
images, replays and archives remain private, outside Git and Board attachments.
The implementation is on a feature-history branch based on cfe402d (#10).
Main predates that history; no main merge or live deployment is claimed.
