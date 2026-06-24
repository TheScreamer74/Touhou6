# Oracle — byte-comparison against the decomp

Two harnesses that verify the port's danmaku matches the th06 decompilation
(`refs/th06-decomp`) exactly, instead of eyeballing recordings.

## `bullet_oracle.cpp` + `oracle_dump.rs` (`run.sh`)
The decomp's exact `SpawnSingleBullet` angle/speed math + RNG vs the port's real
`spawn_bullet_pattern`, over a battery of all 9 aim modes. **PASS** = byte-identical.

## `vm/` — full-VM oracle
Compiles the **real** decomp ECL sim (`EclManager`/`EnemyManager`/`EnemyEclInstr`/
`BulletManager`/`Rng`/`ZunTimer`) on clang via minimal engine stubs (`stub/`,
`engine_stub/`) + a few 64-bit/MSVC patches (`build.sh`), runs the actual
`ecldataN.ecl` with a fixed RNG seed + fixed player + no damage, and dumps every
live bullet per frame. The port does the same via `th06 --ecl-dump`. `compare.sh`
diffs them and prints the first divergent frame.

`vm/build/` and `vm/src/` (the patched decomp working copy) are git-ignored;
`build.sh` regenerates them from `refs/`.

### Finding (stage 5)
Bullet **patterns are faithful** — angles/speeds are byte-identical to the decomp.
The only divergence is bullet **position during the spawn-in effect**: the decomp
moves spawning bullets at velocity/2, /2.5, /3 (FAST/NORMAL/SLOW) for the spawn
anm-script's duration; the port approximates a fixed 8-frame delay at 1/2.5. This
gives a transient ~1px offset that then cascades. Enemy positions match exactly.
