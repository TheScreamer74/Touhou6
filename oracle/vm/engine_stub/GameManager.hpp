#include "ZunMath.hpp"
#include "inttypes.hpp"
namespace th06 {
#pragma once


enum Difficulty { EASY=0, NORMAL=1, HARD=2, LUNATIC=3, EXTRA=4 };
struct Catk { char name[64]; u8 nameCsum; i16 numSuccess, numAttempts; i32 captureScore; u8 characterShotType; };
struct GameManager {
    i32 difficulty=NORMAL, rank=16, score=0, character=0, shotType=0;
    i32 isTimeStopped=0, currentPower=0, counat=0, spellcardsCaptured=0;
    bool isInReplay=false;
    Catk catk[64];
    D3DXVECTOR3 arcadeRegionTopLeftPos{32,16,0};
    D3DXVECTOR3 arcadeRegionSize{384,448,0};
    i32 CharacterShotType(){ return character*2+shotType; }
    // GameManager::IsInBounds: the (w x h) sprite must overlap [0,384]x[0,448].
    i32 IsInBounds(f32 x, f32 y, f32 width, f32 height){
        if (width/2.0f + x < 0.0f) return 0;
        if (x - width/2.0f > arcadeRegionSize.x) return 0;
        if (height/2.0f + y < 0.0f) return 0;
        if (y - height/2.0f > arcadeRegionSize.y) return 0;
        return 1;
    }
    void AddScore(i32 s){ score+=s; }
    // Real gameplay init (GameManager.cpp:293): the player movement area is the
    // 384x448 playfield inset by 8/16px margins, NOT the full playfield. Enemy
    // random spawns (RunEclTimeline case 4-7) draw against this, so the stub
    // must match or every random-spawn x/y diverges from the port.
    D3DXVECTOR3 playerMovementAreaSize{368,416,0};
    // livesRemaining = 2 matches a fresh Normal game and the port's --ecl-dump;
    // the survival subrank tick interval depends on it (EnemyManager.cpp:166-170).
    i32 livesRemaining=2, currentStage=0;
    // Dynamic rank (#3): g_DifficultyInfo[NORMAL] = rank 16, range [10,32].
    i32 subRank=0, minRank=10, maxRank=32;
    void IncreaseSubrank(i32 amount){
        subRank += amount;
        while (subRank >= 100){ rank++; subRank -= 100; }
        if (rank > maxRank) rank = maxRank;
    }
    void DecreaseSubrank(i32 amount){
        subRank -= amount;
        while (subRank < 0){ rank--; subRank += 100; }
        if (rank < minRank) rank = minRank;
    }
};
extern GameManager g_GameManager;
}
