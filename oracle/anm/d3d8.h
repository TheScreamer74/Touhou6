#pragma once
#include <cstdint>
typedef uint32_t D3DCOLOR;
typedef unsigned long DWORD;
typedef int D3DFORMAT;
struct IUnknownStub { long Release() { return 0; } };
struct IDirect3DTexture8 : IUnknownStub {};
struct IDirect3DSurface8 : IUnknownStub {};
struct IDirect3DVertexBuffer8 : IUnknownStub {};
struct IDirect3DDevice8 { template <class... A> long Anything(A...) { return 0; } };
typedef IDirect3DDevice8 *LPDIRECT3DDEVICE8;
typedef IDirect3DTexture8 *LPDIRECT3DTEXTURE8;
enum { D3DRS_ZFUNC = 23, D3DCMP_ALWAYS = 8, D3DCMP_LESSEQUAL = 4 };
enum { GAME_REGION_LEFT = 32, GAME_REGION_TOP = 16, GAME_REGION_WIDTH = 384, GAME_REGION_HEIGHT = 448 };
struct D3DXVECTOR4 { float x, y, z, w; };
struct D3DXIMAGE_INFO { unsigned Width, Height, Depth, MipLevels; };
enum { D3DFMT_UNKNOWN = 0, D3DFMT_A8R8G8B8 = 21, D3DFMT_A1R5G5B5 = 25, D3DFMT_R5G6B5 = 23, D3DFMT_R8G8B8 = 20, D3DFMT_A4R4G4B4 = 26 };
