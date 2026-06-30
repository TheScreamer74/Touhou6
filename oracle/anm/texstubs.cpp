// No-op texture methods (anm interpreter doesn't touch pixels).
#include "AnmManager.hpp"
namespace th06 {
ZunResult AnmManager::LoadTexture(i32, char *, i32, D3DCOLOR) { return ZUN_SUCCESS; }
ZunResult AnmManager::CreateEmptyTexture(i32, u32, u32, i32) { return ZUN_SUCCESS; }
ZunResult AnmManager::LoadTextureAlphaChannel(i32, char *, i32, D3DCOLOR) { return ZUN_SUCCESS; }
void AnmManager::ReleaseTexture(i32) {}
}
