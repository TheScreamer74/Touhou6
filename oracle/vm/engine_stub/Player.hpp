#include "ZunMath.hpp"
#include "inttypes.hpp"
#include <cmath>
namespace th06 {
#pragma once



struct BombInfo { i32 isInUse=0; };
struct Player {
    D3DXVECTOR3 positionCenter{192,400,0};
    D3DXVECTOR3 positionOfLastEnemyHit{0,0,0};
    BombInfo bombInfo;
    f32 AngleToPlayer(D3DXVECTOR3* p){ return std::atan2(positionCenter.y-p->y, positionCenter.x-p->x); }
    f32 AngleFromPlayer(D3DXVECTOR3* p){ return std::atan2(p->y-positionCenter.y, p->x-positionCenter.x); }
    i32 CalcKillBoxCollision(D3DXVECTOR3*, D3DXVECTOR3*){ return 0; }
    void CalcLaserHitbox(...){}
    i32 CheckGraze(...){ return 0; }
    i32 CalcDamageToEnemy(D3DXVECTOR3*, D3DXVECTOR3*, i32*){ return 0; }
};
extern Player g_Player;
}
