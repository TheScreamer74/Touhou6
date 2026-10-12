# Player attack reference comparison

This harness compiles selected **original** `Player.cpp` functions and the
complete `BulletData.cpp` from a user-supplied Touhou 6 decompilation. It does not
run or distribute the game, and never modifies the reference tree.

```sh
TH06_DECOMP_SRC=/absolute/path/to/th06/src python3 oracle/player/build.py
cargo test -p th06 original_cpp -- --ignored --nocapture
```

`build.py` extracts `FireSingleBullet`, `Player::CalcDamageToEnemy`,
`UpdatePlayerBullets`, the movement/orb transition blocks in `HandlePlayerInputs`, and
`GameManager::IsInBounds` from `GameManager.cpp`.
It also compiles the original damage-feedback block from `EnemyManager.cpp`.
The graphics/audio calls are stubs. Damage tests record particle IDs and counts;
movement tests keep ANM instances alive to isolate the movement calculation.
The actual ANM interpreter has its own independent oracle and the asset-dependent
player test checks real collision/laser animation termination.

The comparisons use integer values and the **exact float bit patterns**:

- All 193 entries in all 36 character/power tables, including collision sizes,
  degree conversion, ANM variant, laser periods and sound suppression.
- 72 successive damage states: inclusive rectangle edges/corners, needles,
  retained and repeatedly weakened Marisa A explosions, persistent Marisa B
  lasers, bomb weakening, bomb hit attribution and hit-particle cadence.
- 40 movement states: homing before/after its 40-frame cutoff, collided homing
  shots, missile acceleration, collided missile drift, and anchored laser height.
- 40 orb transition states, including focus reversals and power loss/recovery.
- 128 movement-input states: both characters, all 16 direction combinations,
  focus, and the bomb movement multiplier. This includes opposing-key priority
  and the original diagonal-normalization/multiplier order.
- 12 bounds states, including inclusive edges, the region reduced by screen
  shake, and the original ordered comparisons for NaN positions.
- 26 enemy damage-feedback states: one ADD-color/SoundIdx20 tick followed by
  four suppressed ticks, including unchanged and restored HP.

These are scoped function comparisons. They do not claim Windows replay or
complete game equivalence. Original game archives are required for the separate
asset-dependent tests and are never included in this directory.
