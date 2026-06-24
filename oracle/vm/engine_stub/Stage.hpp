#include "inttypes.hpp"
namespace th06 {
#pragma once

enum StageSpellcardState { NOT_RUNNING=0, RUNNING=1 };
struct Stage { i32 spellcardState=0, ticksSinceSpellcardStarted=0, unpauseFlag=0; };
extern Stage g_Stage;
}
