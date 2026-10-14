#!/usr/bin/env python3
"""Compile selected original Player.cpp functions without game data or D3D.

The functions and BulletData.cpp are read from the user-supplied decompilation;
only graphics/audio/particle side effects are stubbed. No reference formulas
are replaced, and the original tree is never edited.
"""
from pathlib import Path
import os
import hashlib
import json
import re
import subprocess

here = Path(__file__).resolve().parent
root = here.parent.parent
source = Path(os.environ.get('TH06_DECOMP_SRC', root.parent / 'refs/th06-decomp/src'))
build = here / 'build'
build.mkdir(exist_ok=True)
player = (source / 'Player.cpp').read_text()
math = (source / 'ZunMath.hpp').read_text()

def function(text, signature):
    start = text.index(signature)
    opening = text.index('{', start)
    depth = 1
    cursor = opening + 1
    while depth:
        depth += (text[cursor] == '{') - (text[cursor] == '}')
        cursor += 1
    return text[start:cursor]

stub = r'''
#include <algorithm>
#include <cmath>
#include <cstdint>
#include <cstdio>
#include <cstring>
#include <string>
#include <vector>
using i16=int16_t; using u16=uint16_t; using i32=int32_t; using u32=uint32_t; using f32=float; using ZunBool=bool;
#define ZUN_PI ((f32)(3.14159265358979323846))
#define RADIANS(degrees) ((degrees * ZUN_PI / 180.0f))
#define ZUN_MIN(x,y) ((x)>(y)?(y):(x))
#define ARRAY_SIZE(x) (sizeof(x)/sizeof((x)[0]))
#define ARRAY_SIZE_SIGNED(x) ((i32)ARRAY_SIZE(x))
#define MAX_PLAYER_BULLETS 80
#define PLAYER_BOMB_REGION_COUNT 32
#define COLOR_WHITE 0xffffffff
#define NO_SOUND (-1)
#define SOUND_SHOOT 0
#define SOUND_TOTAL_BOSS_DEATH 20
#define SOUND_2 2
#define ITEM_RANDOM_ITEM (-1)
#define ITEM_SPAWNS 3
#define PARTICLE_EFFECT_UNK_3 3
#define PARTICLE_EFFECT_UNK_5 5
#define GAME_REGION_LEFT 0
#define GAME_REGION_TOP 0
#define TH_BUTTON_UP 16
#define TH_BUTTON_DOWN 32
#define TH_BUTTON_LEFT 64
#define TH_BUTTON_RIGHT 128
#define TH_BUTTON_FOCUS 4
#define IS_PRESSED(key) ((mask&(key))!=0)
#define FBR_STOP_SPAWNING (-2)
#define FBR_SPAWN_MORE (-1)
namespace th06 {
using FireBulletResult=i32;
enum { PLAYER_BULLET_TYPE_0,PLAYER_BULLET_TYPE_1,PLAYER_BULLET_TYPE_2,PLAYER_LASER };
enum { ORB_HIDDEN,ORB_UNFOCUSED,ORB_FOCUSING,ORB_FOCUSED,ORB_UNFOCUSING };
enum { PLAYER_BULLET_STATE_INACTIVE,PLAYER_BULLET_STATE_FIRED,PLAYER_BULLET_STATE_COLLIDED };
enum PlayerDirection { MOVEMENT_NONE,MOVEMENT_UP,MOVEMENT_DOWN,MOVEMENT_LEFT,MOVEMENT_RIGHT,MOVEMENT_UP_LEFT,MOVEMENT_UP_RIGHT,MOVEMENT_DOWN_LEFT,MOVEMENT_DOWN_RIGHT };
struct Vec3 {
    float x=0,y=0,z=0;
    float &operator[](int idx) { return (&x)[idx]; }
    Vec3 *AsD3dXVec() { return this; }
    Vec3 operator/(float f) const { return {x/f,y/f,z/f}; }
    Vec3 operator+(Vec3 o) const { return {x+o.x,y+o.y,z+o.z}; }
    Vec3 operator-(Vec3 o) const { return {x-o.x,y-o.y,z-o.z}; }
};
using D3DXVECTOR3=Vec3; using ZunVec3=Vec3;
struct ZunVec2 { float x=0,y=0; };
struct Timer { i32 current=0; operator i32() const { return current; } void operator=(int v){current=v;} void operator++(int){current++;} void operator--(int){current--;} bool HasTicked() const{return true;} };
struct Sprite { float widthPx=12,heightPx=12; };
enum { AnmColorOp_Modulate,AnmColorOp_Add };
enum { EnemyDeath_DespawnNoCallback,EnemyDeath_DisableInteraction,EnemyDeath_DropItemsOnly,EnemyDeath_SetHpTo1 };
struct AnmVm { i32 anmFileIndex=0,pendingInterrupt=0; Vec3 pos; float scaleY=1; Sprite *sprite=nullptr; struct {int colorOp=0;} flags; };
struct Enemy { i32 life=100; uint8_t unk_e41=0; AnmVm primaryVm; int lifeCallbackThreshold=0,timerCallbackThreshold=0,score=123,itemDrop=-2,deathParticle1=0,deathParticle2=0; Vec3 position; struct {int deathMode=0;bool isDamageable=true,isInteractable=true,isSlotOccupied=true,isBoss=false;} flags; int resets=0; };
void Enemy_ResetEffectArray(Enemy *e){e->resets++;}
using ItemType=int;using ItemState=int;
struct ItemManager {void SpawnItem(Vec3*,int,int){}} g_ItemManager;
struct BulletManager {void DespawnBullets(int,bool){}} g_BulletManager;
struct EnemyManager {int randomItemSpawnIndex=0,randomItemTableIndex=0;struct{bool isActive=false;} spellcardInfo;} g_EnemyManager;
int g_RandomItems[32]{};
struct Gui {bool bossPresent=true;} g_Gui;
struct PlayerBullet { AnmVm sprite; Vec3 position,size; ZunVec2 velocity,motion; float speed=0,angle=0; Timer lifetime; i16 damage=0,bulletState=0,bulletType=0,bulletFrame=0,spawnPositionIdx=0; };
struct CharacterPowerBulletData { i16 waitBetweenBullets,bulletFrame; ZunVec2 motion,size; f32 direction,velocity; u16 damage; uint8_t spawnPositionIdx,bulletType; i16 anmFileIdx,bulletSoundIdx; };
struct CharacterPowerData { i32 numBullets,power; CharacterPowerBulletData *bullets; };
struct PlayerBombInfo { bool isInUse=false; };
struct CharacterData { float orthogonalMovementSpeed=4,orthogonalMovementSpeedFocus=2,diagonalMovementSpeed=4,diagonalMovementSpeedFocus=2; };
struct Player {
    PlayerBullet bullets[80]{}; PlayerBombInfo bombInfo;
    Vec3 positionCenter{},orbsPosition[2]{},positionOfLastEnemyHit{-999,-999,0};
    Timer laserTimer[2]; i32 particleTimer=0;
    Vec3 bombRegionSizes[32]{},bombRegionPositions[32]{};
    i32 bombRegionDamages[32]{},bombRegionTotalDamages[32]{};
    i32 CalcDamageToEnemy(D3DXVECTOR3*,D3DXVECTOR3*,ZunBool*);
    int orbState=0; Timer focusMovementTimer; bool isFocus=false; void UpdateOrbs(bool);
    CharacterData characterData; PlayerDirection playerDirection=MOVEMENT_NONE;
    ZunVec2 previousSpeed{},speedMultiplierDuringBomb{1,1}; AnmVm playerSprite;
    Vec3 hitboxSize{},hitboxTopLeft{},hitboxBottomRight{},grabItemSize{},grabItemTopLeft{},grabItemBottomRight{};
    void UpdateMovement(int);
};
struct AnmManager { Sprite sprite; void SetAndExecuteScriptIdx(AnmVm *vm,int idx){vm->anmFileIndex=idx;vm->sprite=&sprite;} int ExecuteScript(AnmVm*){return 0;} } anm;
AnmManager *g_AnmManager=&anm;
struct SoundPlayer { int last=-1; void PlaySoundByIdx(int idx){last=idx;} } g_SoundPlayer;
using SoundIdx=int;
struct EffectManager { std::vector<int> spawned; void SpawnParticles(int idx,Vec3*,int count,u32){for(int i=0;i<count;i++)spawned.push_back(idx);} } g_EffectManager;
struct GameManager { i32 currentPower=0,score=0; void AddScore(int n){score+=n;} Vec3 gameRegionSize{384,448,0},playerMovementAreaTopLeftPos{8,16,0},playerMovementAreaSize{368,416,0}; ZunBool IsInBounds(f32,f32,f32,f32); } g_GameManager;
struct Supervisor { float effectiveFramerateMultiplier=1; } g_Supervisor;
u32 bits(float f){u32 result;memcpy(&result,&f,4);return result;}
}
'''
ids = []
for name,value in re.findall(r'#define (ANM_SCRIPT_PLAYER_\w+) \(ANM_SCRIPT_PLAYER_START \+ (\d+)\)', (source/'AnmIdx.hpp').read_text()):
    ids.append(f'#define {name} {value}')
raw_data = re.sub(r'^#include.*\n', '', (source/'BulletData.cpp').read_text(), flags=re.M)
# Keep all functions in the same th06 namespace as the original tables.
ref = '\nnamespace th06 {\n'
ref += function((source/'GameManager.cpp').read_text(),'ZunBool GameManager::IsInBounds') + '\n'
ref += function(math,'inline void ZunVec3_SetVecCorners') + '\n'
for signature in ['static FireBulletResult FireSingleBullet','i32 Player::CalcDamageToEnemy','static void UpdatePlayerBullets']:
    ref += function(player, signature) + '\n'
orb_start = player.index('    this->orbsPosition[0] = this->positionCenter;')
orb_end = player.index('    if (IS_PRESSED(TH_BUTTON_SHOOT)', orb_start)
ref += 'void Player::UpdateOrbs(bool focus) { this->isFocus=focus; float orbOffsetX,orbOffsetY;\n'+player[orb_start:orb_end]+'}\n'
movement_start = player.index('    speedX = 0.0f;', player.index('ZunResult Player::HandlePlayerInputs'))
ref += 'void Player::UpdateMovement(int mask) { float speedX,speedY;\n'+player[movement_start:orb_start]+'}\n'
enemy_manager = (source/'EnemyManager.cpp').read_text()
flash_start = enemy_manager.index('            if (curEnemy->unk_e41 != 0)')
flash_end = enemy_manager.index('\n        }\n        UpdateEffects(curEnemy);', flash_start)
ref += 'void EnemyDamageFeedback(Enemy *curEnemy,int enemyLifeBeforeDmg) {\n'+enemy_manager[flash_start:flash_end]+'}\n'
death_start = enemy_manager.index('                curEnemy->lifeCallbackThreshold = -1;', enemy_manager.index('if (curEnemy->life <= 0'))
death_end = enemy_manager.index('\n                if (curEnemy->deathCallbackSub >= 0)', death_start)
ref += 'void EnemyDeathSwitch(Enemy *curEnemy) { EnemyManager manager;EnemyManager *mgr=&manager;bool hitByBomb=false;int enemyIdx=0;\n'+enemy_manager[death_start:death_end]+'}\n'
ref += '}\n'
main = r'''
using namespace th06;
int main(int argc,char**argv) {
    std::string mode=argc>1?argv[1]:"tables";
    CharacterPowerData *characters[]={g_CharacterPowerDataReimuA,g_CharacterPowerDataReimuB,g_CharacterPowerDataMarisaA,g_CharacterPowerDataMarisaB};
    if(mode=="tables") {
        for(int c=0;c<4;c++) for(int r=0;r<9;r++) for(int i=0;i<characters[c][r].numBullets;i++) {
            auto &b=characters[c][r].bullets[i];
            printf("%d %d %d %d %d %u %u %u %u %u %u %d %d %d %d %d\n",c,r,i,b.waitBetweenBullets,b.bulletFrame,bits(b.motion.x),bits(b.motion.y),bits(b.size.x),bits(b.size.y),bits(b.direction),bits(b.velocity),b.damage,b.spawnPositionIdx,b.bulletType,b.anmFileIdx,b.bulletSoundIdx>=0);
        }
    } else if(mode=="death") {
        for(int mode=0;mode<4;mode++)for(int boss=0;boss<2;boss++) {
            Enemy e;e.life=-9;e.flags.deathMode=mode;e.flags.isBoss=boss;g_Gui.bossPresent=true;
            EnemyDeathSwitch(&e);
            printf("%d %d %d %d %d %d %d %d %d\n",mode,boss,e.life,e.flags.deathMode,e.flags.isDamageable,e.flags.isInteractable,e.flags.isSlotOccupied,g_Gui.bossPresent,e.resets);
        }
    } else if(mode=="deathfx") {
        for(int mode=0;mode<4;mode++)for(int drop : {-2,0}) {
            Enemy e;g_EffectManager.spawned.clear();e.itemDrop=drop;
            for(int phase=0;phase<2;phase++) {
                e.flags.deathMode=mode;EnemyDeathSwitch(&e);
                printf("%d %d %d %zu\n",mode,drop,phase,g_EffectManager.spawned.size());
            }
        }
    } else if(mode=="flash") {
        Enemy e;
        for(int frame=0;frame<26;frame++) {
            int before=e.life;
            if(frame<10||(frame>=12&&frame<24))e.life--;
            else if(frame==10)e.life+=10;
            g_SoundPlayer.last=-1;EnemyDamageFeedback(&e,before);
            printf("%d %d %d %d %d\n",frame,e.life,e.unk_e41,e.primaryVm.flags.colorOp,g_SoundPlayer.last);
        }
    } else if(mode=="movement") {
        for(int scenario=0;scenario<8;scenario++) for(int directions=0;directions<16;directions++) {
            Player p;p.positionCenter={100,100,0};
            const float ortho=scenario&4?5.0f:4.0f,focus=scenario&4?2.5f:2.0f;
            p.characterData={ortho,focus,ortho/sqrtf(2.0f),focus/sqrtf(2.0f)};
            const float mult=scenario&2?0.3f:1.0f;p.speedMultiplierDuringBomb={mult,mult};
            p.UpdateMovement(directions*16+(scenario&1?4:0));
            printf("%d %d %u %u %u %d\n",scenario,directions,bits(p.positionCenter.x),bits(p.positionCenter.y),bits(p.previousSpeed.x),p.playerSprite.anmFileIndex);
        }
    } else if(mode=="bounds") {
        const float nan=std::nanf("");
        const float cases[][6]={{-6,100,12,12,384,448},{-6.001f,100,12,12,384,448},{390,100,12,12,384,448},{390.001f,100,12,12,384,448},{100,-6,12,12,384,448},{100,-6.001f,12,12,384,448},{100,454,12,12,384,448},{100,454.001f,12,12,384,448},{390,100,12,12,380,444},{100,454,12,12,380,444},{nan,100,12,12,384,448},{100,nan,12,12,384,448}};
        for(int i=0;i<12;i++) { auto &c=cases[i]; g_GameManager.gameRegionSize={c[4],c[5],0}; printf("%d %d\n",i,g_GameManager.IsInBounds(c[0],c[1],c[2],c[3])); }
    } else if(mode=="damage") {
        for(int scenario=0;scenario<18;scenario++) {
            Player p;
            auto &b=p.bullets[0]; b.bulletState=1;b.bulletType=0;b.damage=48;
            b.position={100,100,0};b.size={12,12,1};b.velocity={2,-12};b.sprite.anmFileIndex=64;b.sprite.sprite=&anm.sprite;
            Vec3 pos={100,100,0},size={16,16,1};
            if(scenario==1)pos.x=114;
            if(scenario==2)pos.x=114.001f;
            if(scenario==3)pos={114,114,0};
            if(scenario==4){b.size.y=48;pos.y=132;b.sprite.anmFileIndex=66;}
            if(scenario==5)b.bulletState=2;
            if(scenario==6){b.bulletState=2;b.bulletType=1;}
            if(scenario>=7&&scenario<=10){b.bulletType=2;b.sprite.anmFileIndex=scenario-7+65;b.damage=18-(scenario-7);b.lifetime=(scenario==7?0:scenario==8?1:scenario==9?6:5);}
            if(scenario==11){b.bulletType=2;b.bulletState=2;b.sprite.anmFileIndex=97;b.damage=4;b.lifetime=6;b.size={32,32,1};}
            if(scenario==12){b.bulletType=3;b.damage=3;p.particleTimer=7;}
            if(scenario==13)p.bombInfo.isInUse=true;
            if(scenario==14){p.bombInfo.isInUse=true;b.bulletType=2;b.damage=2;b.sprite.anmFileIndex=65;}
            if(scenario>=15){b.position.x=500;p.bombRegionSizes[0]={32,32,1};p.bombRegionPositions[0]={100,100,0};p.bombRegionDamages[0]=8;p.particleTimer=3;p.bombInfo.isInUse=scenario!=16;if(scenario==17)pos.x=125;}
            g_EffectManager.spawned.clear();
            for(int hit=0;hit<4;hit++) {
                bool bomb=false;int damage=p.CalcDamageToEnemy(&pos,&size,&bomb);
                printf("%d %d %d %d %d %d %d %u %u %u %u %d %d %zu",scenario,hit,damage,bomb,p.particleTimer,b.bulletState,b.damage,bits(b.size.x),bits(b.size.y),bits(b.velocity.x),bits(b.velocity.y),b.sprite.anmFileIndex,p.bombRegionTotalDamages[0],g_EffectManager.spawned.size());
                for(auto id:g_EffectManager.spawned)printf(" %d",id);puts("");
            }
        }
    } else if(mode=="orbs") {
        Player p;p.positionCenter={192,384,0};
        for(int frame=0;frame<40;frame++){
            bool focus=frame<4||(frame>=8&&frame%5<2);g_GameManager.currentPower=(frame>=22&&frame<26)?0:8;
            p.UpdateOrbs(focus);
            printf("%d %d %d %u %u %u %u\n",frame,p.orbState,(i32)p.focusMovementTimer,bits(p.orbsPosition[0].x),bits(p.orbsPosition[0].y),bits(p.orbsPosition[1].x),bits(p.orbsPosition[1].y));
        }
    } else if(mode=="update") {
        for(int scenario=0;scenario<10;scenario++) {
            Player p;auto &b=p.bullets[0]; b.bulletState=1;b.bulletType=1;b.position={100,100,0};b.size={12,12,1};b.velocity={0,-3};b.speed=3;b.sprite.sprite=&anm.sprite;
            if(scenario==0)p.positionOfLastEnemyHit={110,80,0};
            if(scenario==1){p.positionOfLastEnemyHit={110,80,0};b.lifetime=39;}
            if(scenario==2){p.positionOfLastEnemyHit={110,80,0};b.lifetime=40;}
            if(scenario==3)b.bulletState=2;
            if(scenario==4)b.bulletType=2;
            if(scenario==5){b.bulletType=2;b.bulletState=2;}
            if(scenario>=6){b.bulletType=3;b.spawnPositionIdx=1;b.motion={2,0};p.orbsPosition[0]={120,320,0};p.laserTimer[0]=(scenario==6?71:scenario==7?2:scenario==8?1:0);}
            for(int frame=0;frame<4;frame++) {
                UpdatePlayerBullets(&p);
                printf("%d %d %u %u %u %u %u %u %u %d %d %d\n",scenario,frame,bits(b.position.x),bits(b.position.y),bits(b.velocity.x),bits(b.velocity.y),bits(b.speed),bits(b.size.y),bits(b.sprite.scaleY),(i32)b.lifetime,(i32)p.laserTimer[0],b.sprite.pendingInterrupt);
            }
        }
    }
}
'''
# The copied table source opens its own namespace.
output=build/'player_reference.cpp'
output.write_text(stub+'\n'+'\n'.join(ids)+'\n'+raw_data+'\n'+ref+'\n'+main)
subprocess.run(['clang++','-std=c++17','-O2','-ffp-contract=off','-Wno-unknown-pragmas',str(output),'-o',str(build/'player_reference')],check=True)
manifest = {
    'source_directory': str(source),
    'source_sha256': {name: hashlib.sha256((source/name).read_bytes()).hexdigest() for name in ('Player.cpp', 'BulletData.cpp', 'AnmIdx.hpp', 'ZunMath.hpp', 'GameManager.cpp', 'EnemyManager.cpp')},
    'generated_cpp_sha256': hashlib.sha256(output.read_bytes()).hexdigest(),
    'compiler': subprocess.run(['clang++', '--version'], capture_output=True, text=True, check=True).stdout.splitlines()[0],
    'float_contract': 'off',
    'coverage': {'table_entries': 193, 'damage_states': 72, 'movement_states': 40, 'orb_states': 40, 'bounds_states': 12, 'input_states': 128, 'enemy_feedback_states': 26, 'enemy_death_modes': 8, 'repeated_death_particles': 16},
}
(build/'manifest.json').write_text(json.dumps(manifest, indent=2)+'\n')
print(build/'player_reference')
