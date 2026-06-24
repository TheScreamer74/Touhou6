#pragma once
#include <cstdint>
typedef uint32_t D3DCOLOR;
typedef unsigned long DWORD;
struct IDirect3DTexture8;
struct IDirect3DSurface8;
struct IDirect3DDevice8;
struct IDirect3DVertexBuffer8;
typedef IDirect3DDevice8* LPDIRECT3DDEVICE8;
typedef IDirect3DTexture8* LPDIRECT3DTEXTURE8;
enum { D3DRS_ZFUNC=23, D3DCMP_ALWAYS=8 };
enum { D3DCMP_LESSEQUAL=4 };
