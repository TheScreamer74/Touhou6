#include "inttypes.hpp"
namespace th06 {
#pragma once

struct Gui {
    i32 eclSetLives=0; i32 bossPresent=0;
    void ShowSpellcard(i32, char*) {} void EndEnemySpellcard() {}
    void ShowSpellcardBonus(i32) {} void ShowBonusScore(i32) {}
    i32 SpellcardSecondsRemaining(){ return 0; }
    void SetBossHealthBar(f32) {} void SetSpellcardSeconds(i32) {}
    bool HasCurrentMsgIdx(){ return false; }
    bool BossPresent(){ return bossPresent!=0; }
    void MsgRead(i32){} bool MsgWait(...){ return false; }
};
extern Gui g_Gui;
}
