#!/usr/bin/env python3
"""Compile the supplied original BatWing callback with observable VM/RNG stubs."""
import hashlib
import json
import os
from pathlib import Path
import subprocess

root = Path(__file__).resolve().parent
source = Path(os.environ['TH06_DECOMP_SRC'])
build = root / 'build'
build.mkdir(exist_ok=True)


def function(text, signature):
    start = text.index(signature)
    opening = text.index('{', start)
    depth, cursor = 1, opening + 1
    while depth:
        depth += (text[cursor] == '{') - (text[cursor] == '}')
        cursor += 1
    return text[start:cursor]


global_source = (source / 'Global.cpp').read_text()
stub = r'''
#include <cmath>
#include <cstdint>
#include <cstdio>
#include <cstring>
#include <vector>
using u16=uint16_t;using u32=uint32_t;using i32=int32_t;using f32=float;
#define ZUN_PI ((f32)3.14159265358979323846)
#define RADIANS(degrees) ((degrees)*ZUN_PI/180.0f)
#define PARTICLE_EFFECT_UNK_19 19
#define COLOR_DEEPBLUE 0xff3030ff
struct Vec {
 float x=0,y=0,z=0;
 Vec operator-() const{return {-x,-y,-z};}
 Vec operator/(float n) const{return {x/n,y/n,z/n};}
};
using D3DXVECTOR3=Vec;
struct Timer {
 int current=0,previous=0;operator int()const{return current;}
 bool HasTicked()const{return current!=previous;}
 void operator++(int){previous=current;current++;}
};
struct Rng {
 u16 seed=0x1234;u32 generationCount=0;
 u16 GetRandomU16();u32 GetRandomU32();float GetRandomF32ZeroToOne();
 u16 GetRandomU16InRange(u16 range){return range?GetRandomU16()%range:0;}
 float GetRandomF32InRange(float range){return GetRandomF32ZeroToOne()*range;}
}g_Rng;
struct Effect {Vec position,unk_11c,unk_128;u32 color;int sprite;};
struct Manager {
 bool random=false;std::vector<Effect> effects;
 Effect* SpawnParticles(int,Vec*pos,int,u32 color){
  Effect e;e.position=*pos;e.color=color;e.sprite=random?11+g_Rng.GetRandomU16InRange(8):11;
  effects.push_back(e);return &effects.back();
 }
}g_EffectManager;
struct Enemy {struct{bool isInvisible=false;}flags;Vec position={192,96,0};float exInsFunc6Angle=0;Timer exInsFunc6Timer;};
struct EclRawInstr{};
void Enemy_ResetEffectArray(Enemy*){}
unsigned bits(float value){unsigned out;std::memcpy(&out,&value,4);return out;}
'''
body = '\n'.join(function(global_source, signature) for signature in (
    'u16 Rng::GetRandomU16(void)', 'u32 Rng::GetRandomU32(void)',
    'f32 Rng::GetRandomF32ZeroToOne(void)'))
body += '\n' + function((source / 'EnemyEclInstr.cpp').read_text(),
                         'void ExInsBatWingEffect(Enemy *enemy, EclRawInstr *instr)')
main = r'''
int main(){
 for(int random=0;random<2;random++){
  Enemy e;g_Rng.seed=0x1234;g_EffectManager.random=random;
  for(int frame=0;frame<140;frame++){
   e.flags.isInvisible=frame>=130&&frame<135;
   g_EffectManager.effects.clear();ExInsBatWingEffect(&e,nullptr);
   printf("%d %d %d %u %u %zu",random,frame,e.exInsFunc6Timer.current,bits(e.exInsFunc6Angle),g_Rng.seed,g_EffectManager.effects.size());
   for(auto &fx:g_EffectManager.effects)printf(" %u %u %u %u %u %u %d",bits(fx.position.x),bits(fx.position.y),bits(fx.unk_11c.x),bits(fx.unk_11c.y),bits(fx.unk_128.x),bits(fx.unk_128.y),fx.sprite);
   puts("");
  }
 }
}
'''
generated = build / 'enemy_reference.cpp'
generated.write_text(stub + body + main)
subprocess.run(['clang++', '-std=c++17', '-O2', '-ffp-contract=off',
                str(generated), '-o', str(build / 'enemy_reference')], check=True)
receipt = {'source_sha256': {name: hashlib.sha256((source/name).read_bytes()).hexdigest()
                            for name in ('EnemyEclInstr.cpp', 'Global.cpp', 'Global.hpp', 'ZunTimer.hpp')},
           'generated_cpp_sha256': hashlib.sha256(generated.read_bytes()).hexdigest(),
           'coverage': '280 callback observations, initial zero timer, ramp cadence, invisible freeze, ANM random-sprite-before-motion ordering',
           'stubs': 'SpawnParticles records arguments; optional random sprite draws one U16 as ANM opcode16; ResetEffectArray is unobserved'}
(build / 'manifest.json').write_text(json.dumps(receipt, indent=2)+'\n')
print(build / 'enemy_reference')
