#!/usr/bin/env bash
# Vertex oracle: compiles the REAL decomp AnmManager (ExecuteScript + Draw3 KEPT)
# with real D3DX matrix math and a device that captures the world matrix Draw3
# builds. Output: /tmp/oracle_vtx
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"; cd "$HERE"
source "$HERE/../common.sh"
oracle_check_source
VM="../vm"
rm -rf build && mkdir build
cp "$DECOMP"/*.hpp "$DECOMP"/*.cpp build/
for h in "$VM"/engine_stub/*.hpp; do
  b=$(basename "$h")
  case "$b" in AnmManager.hpp|AnmVm.hpp|Stage.hpp) continue;; esac
  cp "$h" build/
done
# our real d3dx/d3d8 (win over the vm/stub no-op ones via -I build first),
# plus the anm harness's TextHelper/prelude.
cp d3d8.h d3dx8math.h ../anm/TextHelper.hpp ../anm/prelude.hpp build/
awk -f trim.awk build/AnmManager.cpp > build/AnmManager.trim && mv build/AnmManager.trim build/AnmManager.cpp
# 64-bit pointer-arith casts (same as the anm oracle).
sed -i "" "s/(i32)vm->beginingOfScript->args/(intptr_t)vm->beginingOfScript->args/g; s/(i32)curInstr->args/(intptr_t)curInstr->args/g; s/(u32)curInstr->args/(uintptr_t)curInstr->args/g" build/AnmManager.cpp
sed -i "" "s/memcpy(vm->posInterpInitial, vm->pos,/memcpy(\&vm->posInterpInitial, \&vm->pos,/; s/memcpy(vm->posInterpInitial, vm->posOffset,/memcpy(\&vm->posInterpInitial, \&vm->posOffset,/" build/AnmManager.cpp
sed -i '' 's/\([a-zA-Z]\) AnmManager::\([A-Za-z]*(\)/\1 \2/g' build/AnmManager.hpp
sed -i '' 's|#define ZUN_ASSERT_SIZE(type, size) C_ASSERT(sizeof(type) == size);|#define ZUN_ASSERT_SIZE(type, size)|g; s|#define ZUN_ASSERT_SIZE(type, size) C_ASSERT(true);|#define ZUN_ASSERT_SIZE(type, size)|g' build/diffbuild.hpp
sed -i '' '1a\
#include "ZunBool.hpp"\
#include "diffbuild.hpp"
' build/ZunTimer.hpp
# Extend the engine-stub device (FakeDev) to capture the world matrix that Draw3
# hands to SetTransform(D3DTS_WORLD) -- the whole point of this oracle -- plus the
# GCOS_DONT_USE_VERTEX_BUF cfg flag Draw3 reads.
perl -0pi -e 's/#include "inttypes.hpp"/#include "inttypes.hpp"\n#include "d3dx8math.h"/' build/Supervisor.hpp
perl -0pi -e 's/enum \{ GCOS_USE_D3D_HW_TEXTURE_BLENDING=0 \};/enum { GCOS_USE_D3D_HW_TEXTURE_BLENDING=0, GCOS_DONT_USE_VERTEX_BUF=1 };\nextern D3DXMATRIX g_capturedWorld, g_capturedTexture;/' build/Supervisor.hpp
perl -0pi -e 's/struct FakeDev \{ template<class\.\.\.A> long SetRenderState\(A\.\.\.\)\{ return 0; \} \};/struct FakeDev { template<class...A> long SetRenderState(A...){return 0;} long SetTransform(unsigned long s,const D3DXMATRIX*m){if(s==256)g_capturedWorld=*m;else if(s==16)g_capturedTexture=*m;return 0;} template<class...A> long SetTexture(A...){return 0;} template<class...A> long SetVertexShader(A...){return 0;} template<class...A> long SetStreamSource(A...){return 0;} template<class...A> long DrawPrimitive(A...){return 0;} template<class...A> long DrawPrimitiveUP(A...){return 0;} template<class...A> long CreateVertexBuffer(A...){return 0;} };/' build/Supervisor.hpp

clang++ -std=c++17 -O2 -ffp-contract=off -ferror-limit=20 -Wno-address-of-temporary \
  -Wl,-dead_strip -I build -I "$VM/stub" -include build/prelude.hpp \
  oracle_vtx_main.cpp texstubs.cpp build/AnmManager.cpp build/ZunTimer.cpp build/Rng.cpp -o /tmp/oracle_vtx
echo "built /tmp/oracle_vtx"
