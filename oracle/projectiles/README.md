# Enemy projectile reference

`build.sh` extracts the actual `Player::{CheckGraze,CalcKillBoxCollision,
CalcLaserHitbox,ScoreGraze}` functions, `utils::Rotate`, and the laser section
of `BulletManager::OnUpdate` from the supplied decompilation. These are compiled
with minimal graphics/audio/manager stubs; collision formulas are not copied
into a hand-written reference. No original game data is required.

It also extracts `ScreenEffect_ShakeScreen` and the original RNG functions for
480 observations of overlapping shakes, their insertion/expiration order,
viewport coordinates, RNG state and time-stop behavior. `Global.cpp` inserts
equal-priority chains in registration order, and each chain consumes its own
RNG draws even when a later chain overwrites the viewport. This reproduces the
Fantasy Seal overlap that first changed the executable replay RNG at frame 2475.

```sh
TH06_DECOMP_SRC=/path/to/th06/src \
TH06_ORACLE_OUTPUT_DIR=/tmp/projectile-comparison \
  oracle/projectiles/compare.sh
```

The source profile must include those function names, as present in the
v1.02h decompilation. Outputs include original source SHA256s and all reference
observations. The Rust test compares all 864 combinations over 85 frames
(73,440 observations): warmup 0/20/60, active 0/25, despawn 0/16, end hitbox
0/16, width/alpha flags, three angles, three player locations, and active bombs.
It observes state, timer, allocation, player death, graze, score, rank/subrank,
drawn width/alpha, and beam offsets. The original midpoint hitbox bug and
state-switch fallthrough are preserved.

The one numeric compatibility adapter is explicit: the original x86
`__ftol2` converts via a 64-bit `FISTP`, and the caller keeps its low 32 bits.
ARM's float-to-int cast differs for NaN/infinity. The adapter reproduces this
original behavior, including fade lasers whose `startTime` is zero, instead
of letting the host's native cast choose their alpha.

The original executable was inspected with REA/Ghidra, SHA256
`7f38496b31b8625196900a69cd1bfed243cab1f9612073e7881dc354b876fd39`:

- `BulletManager_OnUpdate`, `0x4149d0`: REA evidence
  `ev_9491a464aef0c1c94c414fbf842958a12856aff65c8672c385df1918740275b1`.
- Alpha conversion calls at `0x416003` and `0x4162eb` call `0x45ba78`,
  then compare the low dword with 255 and shift it by 24 bits.
- The original `__ftol2`, `0x45ba78..0x45baec`, was independently disassembled
  from that PE: `FISTP qword [esp+0x10]` at `0x45ba87`, low dword loaded into
  EAX at `0x45ba93`. Integer-indefinite has a zero low dword.

Separate asset-dependent Rust coverage verifies every original bullet type's
three spawn modes and cancellation ANM lifetimes (30 combinations), including
etama4's 24-frame bubble spawn and 16-frame fade. Ordinary tests verify first
fired tick zero, cached acceleration velocity, spawn fallthrough, bomb fades,
independent graze caps, popup pool wrap/lifetime, and effect sprite RNG before
particle motion. The last RNG fixture reproduces the actual demo00 frame-456
observation: seed 34784 advances five 16-bit draws to 38085.

The executable comparison also exposed a viewport dependency at demo00 frame
2416: bomb screen shake shortens `GameManager::gameRegionSize`, and both bullet
culling and missed-item removal read that shortened region on the next gameplay
tick. Regression coverage checks the cropped bullet edge and the item's exact
16-pixel removal margin, including the retained 256-tick allowance for bullets
with direction changes.

These focused checks do not establish complete Windows replay compatibility.
The executable replay comparison is a separate validation.
