#!/usr/bin/env bash
# Builds the full-VM oracle: compiles the REAL decomp ECL sim (EclManager,
# EnemyManager, EnemyEclInstr, BulletManager, Rng, ZunTimer) on clang/macOS by
# copying the decomp source into build/, overlaying minimal engine stubs
# (engine_stub/ + stub/), and applying a handful of 64-bit / MSVC patches.
# Output: /tmp/oracle_vm. Run: ./build.sh && /tmp/oracle_vm <ecldata5.ecl> <frames>
set -euo pipefail
cd "$(dirname "$0")"
DECOMP="../../../refs/th06-decomp/src"
[ -d "$DECOMP" ] || { echo "decomp not found at $DECOMP"; exit 1; }

rm -rf build && mkdir build
cp "$DECOMP"/*.hpp "$DECOMP"/*.cpp build/
cp engine_stub/*.hpp build/        # overlay full stub headers

cd build
# --- patches to the real decomp sources ---
# 1. ZUN_ASSERT_SIZE -> no-op (it expands to a Windows C_ASSERT not always in scope)
sed -i '' 's|#define ZUN_ASSERT_SIZE(type, size) C_ASSERT(sizeof(type) == size);|#define ZUN_ASSERT_SIZE(type, size)|g; s|#define ZUN_ASSERT_SIZE(type, size) C_ASSERT(true);|#define ZUN_ASSERT_SIZE(type, size)|g' diffbuild.hpp
# 2. missing includes
sed -i '' '1a\
#include "ZunBool.hpp"\
#include "diffbuild.hpp"
' ZunTimer.hpp
for f in BulletManager.hpp EnemyManager.hpp; do grep -q '#include "Chain.hpp"' "$f" || sed -i '' '1a\
#include "Chain.hpp"
' "$f"; done
for f in EnemyEclInstr.cpp BulletManager.cpp EnemyManager.cpp; do sed -i '' '1i\
#include "ZunBool.hpp"\
#include "AnmIdx.hpp"\
#include "AnmManager.hpp"\
#include "EffectManager.hpp"\
#include "ItemManager.hpp"\
#include "Chain.hpp"
' "$f"; done
# 3. 64-bit: the decomp casts pointers to (int)/(i32) for ECL addressing
sed -i '' 's/(int)this->/(intptr_t)this->/g; s/(int)instruction/(intptr_t)instruction/g' EclManager.cpp
sed -i '' 's/(i32)this->/(intptr_t)this->/g; s/(int)this->/(intptr_t)this->/g' EnemyManager.cpp
cd ..
clang++ -std=c++17 -O2 -ffp-contract=off -ferror-limit=0 -Wno-address-of-temporary \
    -I stub -I build oracle_main.cpp \
    build/EclManager.cpp build/EnemyEclInstr.cpp build/BulletManager.cpp \
    build/EnemyManager.cpp build/Rng.cpp build/ZunTimer.cpp -o /tmp/oracle_vm
echo "built /tmp/oracle_vm"
