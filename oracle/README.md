# Oracles — comparisons with the decompilation

The harnesses compare specific interpreter and rendering calculations with
Touhou 6 v1.02h's matching decompilation. They run native C++ reference code;
they do not launch the Windows executable or prove complete game equivalence.

## Locked reference and paths

The C++ harnesses target
[`happyhavoc/th06` commit bd90685fa40eaf2e16789dcf788843b304e6e02d](https://github.com/happyhavoc/th06/tree/bd90685fa40eaf2e16789dcf788843b304e6e02d).
The downloaded source archive SHA256 is
`f09629ea2d227bbc9430c03dc51f87e2cc408e6cf6337797f449fcfde225e3cf`.
Later upstream revisions merged RNG/timer files and replaced `diffbuild.hpp`.
An incompatible source layout fails with a readable profile error instead of
silently substituting reference calculations.

Set paths explicitly when the old sibling `refs/` and `res/` folders do not
exist. Paths may contain spaces.

```sh
export TH06_DECOMP_SRC=/absolute/path/to/th06/src
export TH06_RES_DIR=/absolute/path/to/game
export TH06_ORACLE_OUTPUT_DIR=/absolute/path/to/comparison-output
oracle/run.sh
oracle/anm/compare.sh 1 600
oracle/bg/compare.sh 5 1200
oracle/vtx/compare.sh 1 300
oracle/vm/compare.sh 5 6000
```

`TH06_ST_DAT` can override `$TH06_RES_DIR/ST.DAT`. The VM also needs the other
original archives in `TH06_RES_DIR`, because it runs the actual port. The
source and game data must be supplied separately. No downloader runs inside
the harnesses.

Build scripts use clang and macOS/BSD `sed`. Reference sources are copied to
ignored `build/` directories and overlaid with the existing graphics/audio
stubs and 64-bit pointer adaptations; upstream files are never edited.
Comparison inputs are freshly extracted from the selected archive. Results
are preserved in `TH06_ORACLE_OUTPUT_DIR`, or a new temporary directory when
that variable is absent. Differences and empty comparisons fail with a
nonzero exit status. `TH06_SKIP_ORACLE_BUILD=1` reuses an already built C++
reference binary; use it only with the same selected source.

## What each comparison covers

| Harness | Reference calculation and output |
|---|---|
| `run.sh` | Existing `SpawnSingleBullet`/RNG transcription against the port's real `spawn_bullet_pattern`: 139 bullets across all nine aim modes; angle and speed printed to six decimals |
| `anm/compare.sh` | Real `AnmManager::ExecuteScript` against the background ANM runner: sprite, flags, position, scale, rotation and color state per script/frame |
| `bg/compare.sh` | Real `Stage::OnUpdate`: background camera, facing, fog and script state per frame |
| `vtx/compare.sh` | Real `Draw3` quad transforms and camera projection against the port's projected corners |
| `vm/compare.sh` | Real ECL, EnemyManager and BulletManager execution: every active bullet's position, angle and speed to four decimals per frame |

ANM, BG, bullet and VM comparisons require identical formatted observations;
no new numeric tolerance is introduced. The vertex harness retains its
existing 1.5 px threshold and near-plane/off-screen exclusions, reporting the
number skipped and maximum observed difference. A skipped quad is not a
verified quad.

The VM fixes seed `0x1234`, Normal difficulty (Extra for stage 7), rank 16, Reimu A and player
position `(192,384)`, with no input, no damage and dialogue stubs. It includes
the two 16-bit item-table initialization draws from `EnemyManager::AddedCallback`.
Effect stubs preserve RNG draws, while presentation and audio are excluded.
Runs on other difficulties, with input/damage, or over other frame windows
need their own comparisons. A pass in one harness does not imply a pass in
another.

See [the dated validation report](../docs/REVERSE_ENGINEERING.md) for actual
executed cases, limitations and binary evidence.
