#include "inttypes.hpp"
namespace th06 {
#pragma once

enum SoundIdx { NO_SOUND=-1, SOUND_2=2, SOUND_7=7, SOUND_SHOOT=8, SOUND_16=22, SOUND_TOTAL_BOSS_DEATH=30 };
struct SoundPlayer { void PlaySoundByIdx(SoundIdx, int) {} };
extern SoundPlayer g_SoundPlayer;
}
