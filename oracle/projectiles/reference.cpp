// Minimal platform/graphics stubs around functions extracted from the selected
// original decompilation. No collision/state formulas are transcribed here.
#include <algorithm>
#include <cmath>
#include <cstdint>
#include <cstdio>
using i32 = int32_t;
using f32 = float;
using ZunBool = bool;
constexpr int MAX_ENEMY_LASERS = 1;
constexpr int PARTICLE_EFFECT_UNK_8 = 8;
constexpr int COLOR_WHITE = -1, SOUND_GRAZE = 0;
constexpr int LASER_STATE_START_DELAY = 0, LASER_STATE_ACTIVE = 1, LASER_STATE_DESPAWNING = 2;
constexpr int LASER_FLAG_FADE_IN_OUT = 1;
#define ZUN_MIN(a,b) std::min(a,b)
#define ARRAY_SIZE_SIGNED(a) (int(sizeof(a)/sizeof((a)[0])))
struct D3DXVECTOR3 {
    float x=0, y=0, z=0;
    D3DXVECTOR3 operator+(const D3DXVECTOR3 &v) const { return {x+v.x,y+v.y,z+v.z}; }
    D3DXVECTOR3 operator-(const D3DXVECTOR3 &v) const { return {x-v.x,y-v.y,z-v.z}; }
    D3DXVECTOR3 operator/(float v) const { return {x/v,y/v,z/v}; }
};
namespace utils { void Rotate(D3DXVECTOR3*,D3DXVECTOR3*,float); }
struct PlayerRect { D3DXVECTOR3 pos, size; };
enum { PLAYER_STATE_ALIVE, PLAYER_STATE_SPAWNING, PLAYER_STATE_DEAD, PLAYER_STATE_INVULNERABLE };
struct Player {
    PlayerRect bombProjectiles[16];
    D3DXVECTOR3 positionCenter, hitboxSize{1.25f,1.25f,5.0f}, hitboxTopLeft, hitboxBottomRight;
    struct { bool isInUse=false; } bombInfo;
    int playerState=PLAYER_STATE_ALIVE;
    i32 CalcKillBoxCollision(D3DXVECTOR3*,D3DXVECTOR3*);
    i32 CheckGraze(D3DXVECTOR3*,D3DXVECTOR3*);
    i32 CalcLaserHitbox(D3DXVECTOR3*,D3DXVECTOR3*,D3DXVECTOR3*,float,bool);
    void ScoreGraze(D3DXVECTOR3*);
    void Die() { playerState=PLAYER_STATE_DEAD; }
};
Player g_Player;
struct GameManager {
    int grazeInStage=0, grazeInTotal=0, score=0, rank=16, subRank=0;
    void AddScore(int amount) { score += amount; }
    void IncreaseSubrank(int amount) { subRank += amount; while(subRank>=100){rank++;subRank-=100;}rank=std::min(rank,32); }
} g_GameManager;
struct { void SpawnParticles(int,D3DXVECTOR3*,int,int) {} } g_EffectManager;
struct { struct { int flag3=0; } flags; } g_Gui;
struct { void PlaySoundByIdx(int) {} } g_SoundPlayer;
struct { float effectiveFramerateMultiplier=1; } g_Supervisor;
struct Sprite { float widthPx=16,heightPx=256; } sprite;
struct AnmVm { float scaleX=1,scaleY=1; D3DXVECTOR3 rotation; uint32_t color=0xffffffff; Sprite *sprite=&::sprite; };
struct { void ExecuteScript(AnmVm*) {} } anmManager;
auto *g_AnmManager=&anmManager;
struct Laser {
    bool inUse=false;
    D3DXVECTOR3 pos;
    float endOffset=0,startOffset=0,startLength=0,width=0,speed=0,angle=0;
    int startTime=0,hitboxStartTime=0,duration=0,despawnDuration=0,hitboxEndDelay=0,flags=0,state=0,timer=0;
    AnmVm vm;
};
struct BulletManager { Laser lasers[1]; } manager;
constexpr float ZUN_HALF_PI=1.57079632679489661923f;
// Compatibility shim verified in original __ftol2 at 0x45ba78: FISTP qword,
// then caller keeps EAX. ARM's native saturating cast differs on NaN/infinity.
static i32 original_ftol(float value) {
    int64_t integer=(!std::isfinite(value)||value < -9223372036854775808.0 || value >= 9223372036854775808.0)
        ? INT64_MIN : static_cast<int64_t>(value);
    return static_cast<i32>(integer);
}
#include "original_functions.inc"

int main() {
    int id=0;
    for(int flags : {0,1}) for(int start : {0,20,60}) for(int duration : {0,25})
    for(int despawn : {0,16}) for(int end : {0,16}) for(float angle : {0.0f,0.7f,1.57079632679489661923f})
    for(int location : {0,1,2}) for(int bomb : {0,1}) {
        g_Player={};g_GameManager={};manager={};
        auto &l=manager.lasers[0];l.inUse=true;l.pos={100,100,0};l.endOffset=200;l.startOffset=16;
        l.startLength=200;l.width=24;l.speed=0.5f;l.angle=angle;l.startTime=start;l.hitboxStartTime=10;
        l.duration=duration;l.despawnDuration=despawn;l.hitboxEndDelay=end;l.flags=flags;l.state=start==0?1:0;
        float along=location==2?215.0f:110.0f, across=location==0?0.0f:30.0f;
        g_Player.positionCenter={100+cosf(angle)*along-sinf(angle)*across,100+sinf(angle)*along+cosf(angle)*across,0};
        g_Player.hitboxTopLeft=g_Player.positionCenter-g_Player.hitboxSize;
        g_Player.hitboxBottomRight=g_Player.positionCenter+g_Player.hitboxSize;
        g_Player.bombInfo.isInUse=bomb;
        if(bomb)g_Player.playerState=PLAYER_STATE_INVULNERABLE;
        for(int frame=0;frame<85;frame++) {
            update_lasers(&manager);
            printf("%d %d %d %d %d %d %d %d %d %d %.6f %u %.6f %.6f\n",id,frame,l.state,l.timer,l.inUse,
                g_Player.playerState,g_GameManager.grazeInTotal,g_GameManager.score,g_GameManager.rank,g_GameManager.subRank,
                l.vm.scaleX*16.0f,l.vm.color>>24,l.startOffset,l.endOffset);
        }
        id++;
    }
}
