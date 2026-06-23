#include "AnmVm.hpp"
#include "AnmIdx.hpp"
#include "inttypes.hpp"
namespace th06 {
#pragma once


// Sets a bullet/laser sprite. heightPx follows etama3: big-ball/fireball/dagger
// sprite indices (110..129) are 32px (>=30 test), everything else 16px.
struct AnmManager {
    AnmLoadedSprite spritePool[1024];
    void setSprite(AnmVm* vm, i32 spriteIdx){
        i32 i = spriteIdx & 1023;
        spritePool[i].heightPx = (spriteIdx>=110 && spriteIdx<=129) ? 32.0f : 16.0f;
        spritePool[i].widthPx = spritePool[i].heightPx;
        vm->sprite = &spritePool[i];
        vm->activeSpriteIndex = (i16)spriteIdx;
    }
    void SetActiveSprite(AnmVm* vm, i32 idx){ setSprite(vm, idx); }
    static i32 baseForScript(i32 scriptIdx){
        static const i32 BASE[10]={14,30,46,62,78,94,110,118,122,146};
        i32 t=scriptIdx-0x200; return (t>=0&&t<10)?BASE[t]:0;
    }
    void SetAndExecuteScriptIdx(AnmVm* vm, i32 scriptIdx){ vm->anmFileIndex=(i16)scriptIdx; vm->baseSpriteIndex=(i16)baseForScript(scriptIdx); setSprite(vm, vm->baseSpriteIndex); }
    void InitializeAndSetSprite(AnmVm* vm, i32 idx){ setSprite(vm, idx); }
    // etama3 spawn-in script durations (ANM_SCRIPT_BULLET3_SPAWN_* rel 14..20).
    static i32 spawnDur(i32 anmFileIndex){
        switch (anmFileIndex - 0x200) {
        case 14: case 17: return 10;            // FAST
        case 15: case 18: return 16;            // NORMAL
        case 16: case 19: case 20: return 32;   // SLOW / HUGE
        default: return 1000000;                // non-spawn scripts never "end"
        }
    }
    // Advance the vm's script clock; return nonzero once a spawn-in script ends
    // (so BulletManager::OnUpdate transitions the bullet to FIRED) — matching the
    // real anm timing. Other (looping) scripts return 0.
    i32 ExecuteScript(AnmVm* vm){
        vm->currentTimeInScript.Tick();
        return vm->currentTimeInScript.current >= spawnDur(vm->anmFileIndex) ? 1 : 0;
    }
    template<class...A> i32 ExecuteScript(A...){ return 0; }
    template<class...A> i32 Draw2(A...){ return 0; }
    template<class...A> i32 Draw3(A...){ return 0; }
    template<class...A> i32 Draw(A...){ return 0; }
    template<class...A> i32 LoadAnm(A...){ return 0; }
    template<class...A> void ReleaseAnm(A...){}
};
extern AnmManager* g_AnmManager;
}
