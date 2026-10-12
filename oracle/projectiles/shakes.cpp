// Compile the original screen-shake callback and RNG with minimal chain stubs.
#include <cstdint>
#include <cstdio>
#include <cstring>
#include <vector>
using i32 = int32_t;
using u16 = uint16_t;
using u32 = uint32_t;
using f32 = float;
constexpr float GAME_REGION_POS_X=32, GAME_REGION_POS_Y=16;
constexpr float GAME_REGION_WIDTH=384, GAME_REGION_HEIGHT=448;
enum ChainCallbackResult { CHAIN_CALLBACK_RESULT_CONTINUE_AND_REMOVE_JOB=0, CHAIN_CALLBACK_RESULT_CONTINUE=1 };
struct Vec2 { float x=0,y=0; };
struct { bool isTimeStopped=false; Vec2 gameRegionScreenPos{32,16}, gameRegionSize{384,448}; } g_GameManager;
struct Rng {
    u16 seed=0;
    u32 generationCount=0;
    u16 GetRandomU16();
    u32 GetRandomU32();
    u32 GetRandomU32InRange(u32);
} g_Rng;
struct ScreenEffect { i32 timer=0,effectLength=0,genericParam=0,shakinessParam=0; };
#include "original_shakes.inc"
static u32 bits(float value) { u32 result; std::memcpy(&result,&value,4);return result; }
int main() {
    for(int scenario=0;scenario<8;scenario++) {
        g_GameManager={};g_Rng={};g_Rng.seed=static_cast<u16>(16878+scenario*71);
        std::vector<ScreenEffect> effects;
        for(int frame=0;frame<60;frame++) {
            if(frame==0)effects.push_back({0,40,2,6});
            if(frame==2)effects.push_back({0,16,8,0});
            if(frame==4)effects.push_back({0,scenario&1?7:16,scenario&2?20:8,0});
            g_GameManager.isTimeStopped=(scenario&4)&&frame>=8&&frame<13;
            for(auto it=effects.begin();it!=effects.end();) {
                if(ScreenEffect_ShakeScreen(&*it)==CHAIN_CALLBACK_RESULT_CONTINUE_AND_REMOVE_JOB)it=effects.erase(it);
                else ++it;
            }
            printf("%d %d %u %zu %u %u %u %u\n",scenario,frame,g_Rng.seed,effects.size(),
                bits(g_GameManager.gameRegionScreenPos.x),bits(g_GameManager.gameRegionScreenPos.y),
                bits(g_GameManager.gameRegionSize.x),bits(g_GameManager.gameRegionSize.y));
        }
    }
}
