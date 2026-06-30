#pragma once
#include "ZunMath.hpp"
enum { D3DRS_FOGCOLOR = 34, D3DRS_FOGSTART = 36, D3DRS_FOGEND = 37 };
// Used only by dead-stripped RenderObjects; stub returns the dest unchanged.
template <class... A> inline D3DXVECTOR3 *D3DXVec3Project(D3DXVECTOR3 *o, A...) { return o; }
namespace th06 { struct ZunRect { float left, top, right, bottom; }; }
enum { GAME_REGION_LEFT = 32, GAME_REGION_TOP = 16, GAME_REGION_WIDTH = 384, GAME_REGION_HEIGHT = 448 };
