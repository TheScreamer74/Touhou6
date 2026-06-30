#!/usr/bin/env bash
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"; cd "$HERE"
DECOMP="../../../refs/th06-decomp/src"; VM="../vm"
rm -rf build && mkdir build
cp "$DECOMP"/*.hpp "$DECOMP"/*.cpp build/
for h in "$VM"/engine_stub/*.hpp; do
  b=$(basename "$h")
  case "$b" in AnmManager.hpp|AnmVm.hpp|Stage.hpp) continue;; esac
  cp "$h" build/
done
cp d3d8.h TextHelper.hpp prelude.hpp build/
awk -f trim.awk build/AnmManager.cpp > build/AnmManager.trim && mv build/AnmManager.trim build/AnmManager.cpp
sed -i "" "s/(i32)vm->beginingOfScript->args/(intptr_t)vm->beginingOfScript->args/g; s/(i32)curInstr->args/(intptr_t)curInstr->args/g; s/(u32)curInstr->args/(uintptr_t)curInstr->args/g" build/AnmManager.cpp
sed -i "" "s/memcpy(vm->posInterpInitial, vm->pos,/memcpy(\&vm->posInterpInitial, \&vm->pos,/; s/memcpy(vm->posInterpInitial, vm->posOffset,/memcpy(\&vm->posInterpInitial, \&vm->posOffset,/" build/AnmManager.cpp
# clang rejects 'void AnmManager::Method' qualifier on inline defs inside the class
sed -i '' 's/\([a-zA-Z]\) AnmManager::\([A-Za-z]*(\)/\1 \2/g' build/AnmManager.hpp
sed -i '' 's|#define ZUN_ASSERT_SIZE(type, size) C_ASSERT(sizeof(type) == size);|#define ZUN_ASSERT_SIZE(type, size)|g; s|#define ZUN_ASSERT_SIZE(type, size) C_ASSERT(true);|#define ZUN_ASSERT_SIZE(type, size)|g' build/diffbuild.hpp
sed -i '' '1a\
#include "ZunBool.hpp"\
#include "diffbuild.hpp"
' build/ZunTimer.hpp
clang++ -std=c++17 -O2 -ffp-contract=off -ferror-limit=10 -Wno-address-of-temporary \
  -Wl,-dead_strip -I build -I "$VM/stub" -include build/prelude.hpp \
  oracle_anm_main.cpp texstubs.cpp build/AnmManager.cpp build/ZunTimer.cpp build/Rng.cpp -o /tmp/oracle_anm
echo "built /tmp/oracle_anm"
