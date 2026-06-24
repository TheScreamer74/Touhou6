#include "inttypes.hpp"
#include "ZunMath.hpp"
namespace th06 {
#pragma once


enum ItemType { ITEM_POWER_SMALL=0, ITEM_POWER_BIG=2, ITEM_POINT=1, ITEM_RANDOM_ITEM=-1, ITEM_POINT_BULLET=6 };
struct ItemManager { i32 randomItemSpawnIndex=0, randomItemTableIndex=0; void SpawnItem(D3DXVECTOR3*, ItemType, int) {}
    void OnUpdate(){} void OnDraw(){} };
extern ItemManager g_ItemManager;
}
