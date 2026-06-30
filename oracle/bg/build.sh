#!/usr/bin/env bash
# BG-state oracle: compiles the REAL decomp Stage::OnUpdate (+ UpdateObjects) —
# STD camera/fog/facing interpolation — with the vm engine-stubs and real
# Stage.hpp. Draw/IO Stage methods are trimmed (they drag in D3D/FileSystem).
# Output: /tmp/oracle_bg
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
cd "$HERE"
DECOMP="../../../refs/th06-decomp/src"
VM="../vm"
[ -d "$DECOMP" ] || { echo "decomp not found"; exit 1; }

rm -rf build && mkdir build
cp "$DECOMP"/*.hpp "$DECOMP"/*.cpp build/
for h in "$VM"/engine_stub/*.hpp; do
    b=$(basename "$h"); [ "$b" = "Stage.hpp" ] && continue; cp "$h" build/
done
cp ScreenEffect.hpp d3dconsts.hpp build/
grep -q stageCameraFacingDir build/GameManager.hpp || \
  sed -i '' 's|    i32 livesRemaining=0, currentStage=0;|    i32 livesRemaining=0, currentStage=0;\
    D3DXVECTOR3 stageCameraFacingDir{0,0,1};|' build/GameManager.hpp

awk -f trim.awk build/Stage.cpp > build/Stage.trim && mv build/Stage.trim build/Stage.cpp

sed -i '' 's|#define ZUN_ASSERT_SIZE(type, size) C_ASSERT(sizeof(type) == size);|#define ZUN_ASSERT_SIZE(type, size)|g; s|#define ZUN_ASSERT_SIZE(type, size) C_ASSERT(true);|#define ZUN_ASSERT_SIZE(type, size)|g' build/diffbuild.hpp
sed -i '' '1a\
#include "ZunBool.hpp"\
#include "diffbuild.hpp"
' build/ZunTimer.hpp
# 64-bit: pointer arithmetic cast in UpdateObjects' quad walk.
sed -i '' 's|(i32)&objQuad->type|(intptr_t)\&objQuad->type|g' build/Stage.cpp

clang++ -std=c++17 -O2 -ffp-contract=off -ferror-limit=8 -Wno-address-of-temporary \
    -Wl,-dead_strip -I "$VM/stub" -I build -include build/d3dconsts.hpp \
    oracle_bg_main.cpp build/Stage.cpp build/ZunTimer.cpp -o /tmp/oracle_bg
echo "built /tmp/oracle_bg"
