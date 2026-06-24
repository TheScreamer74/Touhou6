#include "inttypes.hpp"
namespace th06 {
#pragma once

enum { SUPERVISOR_STATE_GAMEMANAGER_REINIT=0 };
enum { GCOS_USE_D3D_HW_TEXTURE_BLENDING=0 };
struct FakeDev { template<class...A> long SetRenderState(A...){ return 0; } };
struct SupCfg { int opts=0; };
struct Supervisor { f32 effectiveFramerateMultiplier=1.0f, framerateMultiplier=1.0f;
    void TickTimer(i32* frames, f32* subframes){
        if (framerateMultiplier <= 0.99f){ *subframes += effectiveFramerateMultiplier; if(*subframes>=1.0f){ *frames+=1; *subframes-=1.0f; } }
        else { *frames += 1; }
    }
    FakeDev* d3dDevice=nullptr;
    int curState=0;
    int hasD3dHardwareVertexProcessing=0;
    SupCfg cfg; };
extern Supervisor g_Supervisor;
}
