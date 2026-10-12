#!/usr/bin/env bash
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
DECOMP="${TH06_DECOMP_SRC:-$HERE/../../../refs/th06-decomp/src}"
OUT="${TH06_ORACLE_OUTPUT_DIR:-$(mktemp -d "${TMPDIR:-/tmp}/th06-projectiles.XXXXXX")}"
mkdir -p "$OUT"
python3 - "$DECOMP" "$OUT" <<'PY'
from pathlib import Path
import hashlib,sys,re
source,out=map(Path,sys.argv[1:])
player=(source/'Player.cpp').read_text()
bullet=(source/'BulletManager.cpp').read_text()
global_file=source/('Global.cpp' if (source/'Global.cpp').exists() else 'utils.cpp')
global_source=global_file.read_text()
def function(text,signature):
    begin=text.index(signature); brace=text.index('{',begin); level=1; end=brace+1
    while level:
        level += (text[end]=='{')-(text[end]=='}'); end+=1
    return text[begin:end]
parts=[function(player,name) for name in ('i32 Player::CalcKillBoxCollision(', 'i32 Player::CheckGraze(', 'i32 Player::CalcLaserHitbox(', 'void Player::ScoreGraze(')]
parts.append('namespace utils {\n'+function(global_source,'void Rotate(')+'\n}')
start=bullet.index('    Laser *curLaser = &mgr->lasers[0];',bullet.index('BulletManager::OnUpdate') if 'BulletManager::OnUpdate' in bullet else bullet.index('BulletManager_OnUpdate'))
end=bullet.index('    mgr->time++;',start)
laser_block=re.sub(r'laserColor = ([^;]+);',r'laserColor = original_ftol(\1);',bullet[start:end])
parts.append('void update_lasers(BulletManager *mgr) {\n i32 idx,laserColor;f32 length;D3DXVECTOR3 laserSize,laserCenter;\n'+laser_block+'\n}')
(out/'original_functions.inc').write_text('\n\n'.join(parts))
shake=(source/'ScreenEffect.cpp').read_text()
header=(source/'Global.hpp').read_text()
shake_parts=[function(global_source,name) for name in ('u16 Rng::GetRandomU16(', 'u32 Rng::GetRandomU32(')]
shake_parts.append(function(header,'u32 GetRandomU32InRange(').replace('u32 GetRandomU32InRange(', 'u32 Rng::GetRandomU32InRange('))
shake_parts.append(function(shake,'static ChainCallbackResult ScreenEffect_ShakeScreen('))
(out/'original_shakes.inc').write_text('\n\n'.join(shake_parts))
with (out/'reference-sources.sha256').open('w') as f:
    for path in (source/'Player.cpp',source/'BulletManager.cpp',global_file,source/'Global.hpp',source/'ScreenEffect.cpp'): f.write(hashlib.sha256(path.read_bytes()).hexdigest()+'  '+str(path)+'\n')
PY
clang++ -std=c++17 -O2 -ffp-contract=off -Wno-unknown-pragmas -I "$OUT" "$HERE/reference.cpp" -o "$OUT/projectiles-reference"
"$OUT/projectiles-reference" > "$OUT/laser-reference.txt"
clang++ -std=c++17 -O2 -ffp-contract=off -I "$OUT" "$HERE/shakes.cpp" -o "$OUT/shakes-reference"
"$OUT/shakes-reference" > "$OUT/shakes-reference.txt"
printf '%s\n' "$OUT/laser-reference.txt"
