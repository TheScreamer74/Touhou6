// Full-VM oracle harness: runs the REAL decomp ECL sim (EclManager + EnemyManager
// + EnemyEclInstr + BulletManager, compiled via the stubs) over the actual
// ecldata5.ecl, with a fixed RNG seed + fixed player position + no damage, and
// dumps every live bullet (pos, angle, speed) per frame. The Rust side runs the
// port identically; a diff reveals any execution-layer divergence.
#include "AnmManager.hpp"
#include "AsciiManager.hpp"
#include "BulletManager.hpp"
#include "Chain.hpp"
#include "EclManager.hpp"
#include "EffectManager.hpp"
#include "Enemy.hpp"
#include "EnemyManager.hpp"
#include "FileSystem.hpp"
#include "GameErrorContext.hpp"
#include "GameManager.hpp"
#include "Gui.hpp"
#include "ItemManager.hpp"
#include "Player.hpp"
#include "Rng.hpp"
#include "Stage.hpp"
#include "Supervisor.hpp"
#include "ZunBool.hpp"
#include "ZunMath.hpp"
#include <algorithm>
#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <string>
#include <vector>

using namespace th06;

// The decomp's manager globals (g_BulletManager, g_EclManager, g_EnemyManager,
// g_Rng, g_PlayerAngle, ...) are DIFFABLE_STATIC, which is a real definition in
// non-DIFFBUILD mode — so the compiled .cpp files already define them. We only
// define the stub-header globals here.
namespace th06 {
Player g_Player;
GameManager g_GameManager;
Gui g_Gui;
Stage g_Stage;
Supervisor g_Supervisor;
SoundPlayer g_SoundPlayer;
ItemManager g_ItemManager;
EffectManager g_EffectManager;
GameErrorContext g_GameErrorContext;
Chain g_Chain;
AnmManager g_AnmManagerInst;
AnmManager* g_AnmManager = &g_AnmManagerInst;
AsciiManager g_AsciiManager;

// utils the ECL code calls (utils.cpp itself is Windows-bound; copy the 3 math fns).
namespace utils {
f32 AddNormalizeAngle(f32 a, f32 b) {
    int i = 0;
    a += b;
    while (a > ZUN_PI) { a -= ZUN_2PI; if (i++ > 16) break; }
    while (a < -ZUN_PI) { a += ZUN_2PI; if (i++ > 16) break; }
    return a;
}
void Rotate(D3DXVECTOR3* out, D3DXVECTOR3* in, f32 angle) {
    f32 c = cosf(angle), s = sinf(angle);
    f32 x = in->x, y = in->y;
    out->x = x * c - y * s;
    out->y = x * s + y * c;
    out->z = in->z;
}
void DebugPrint2(const char*, ...) {}
} // namespace utils

namespace FileSystem {
uint8_t* OpenPath(char* path, bool) {
    FILE* f = fopen(path, "rb");
    if (!f) return nullptr;
    fseek(f, 0, SEEK_END);
    long n = ftell(f);
    fseek(f, 0, SEEK_SET);
    uint8_t* buf = (uint8_t*)malloc(n);
    fread(buf, 1, n, f);
    fclose(f);
    return buf;
}
} // namespace FileSystem
} // namespace th06

int main(int argc, char** argv) {
    const char* eclPath = argc > 1 ? argv[1] : "ecldata5.ecl";
    int frames = argc > 2 ? atoi(argv[2]) : 16000;

    g_Rng.seed = 0x1234;
    g_Rng.generationCount = 0;
    g_GameManager.difficulty = NORMAL;
    g_GameManager.rank = 16;
    g_Supervisor.effectiveFramerateMultiplier = 1.0f;
    g_Player.positionCenter = D3DXVECTOR3(192.0f, 408.0f, 0.0f);

    g_BulletManager.InitializeToZero();
    BulletManager::AddedCallback(&g_BulletManager); // init bulletTypeTemplates
    g_EnemyManager.Initialize();

    // Custom 64-bit ECL loader: the file uses 32-bit offsets, but EclRawHeader
    // declares them as 8-byte pointers, so the decomp's Load misreads on 64-bit.
    // Header: subCount@0 (u16), timelineOffset@4 (u32), subOffsets@16+i*4 (u32).
    // Instruction/timeline bodies are layout-identical, so RunEcl works once the
    // sub/timeline pointers point into the buffer.
    uint8_t* buf = th06::FileSystem::OpenPath((char*)eclPath, false);
    if (!buf) { fprintf(stderr, "failed to read %s\n", eclPath); return 1; }
    auto u16at = [&](int o){ return (uint16_t)(buf[o] | (buf[o+1] << 8)); };
    auto u32at = [&](int o){ return (uint32_t)(buf[o] | (buf[o+1]<<8) | (buf[o+2]<<16) | ((uint32_t)buf[o+3]<<24)); };
    int subCount = u16at(0);
    static EclRawInstr* subTable[4096];
    for (int i = 0; i < subCount && i < 4096; i++)
        subTable[i] = (EclRawInstr*)(buf + u32at(16 + i*4));
    g_EclManager.eclFile = (EclRawHeader*)buf;
    g_EclManager.subTable = subTable;
    g_EclManager.timeline = (EclTimelineInstr*)(buf + u32at(4));

    for (int fr = 0; fr < frames; fr++) {
        g_Player.positionCenter = D3DXVECTOR3(192.0f, 408.0f, 0.0f);
        EnemyManager::OnUpdate(&g_EnemyManager);
        EffectManager::OnUpdate(&g_EffectManager); // chain prio 10 (between enemy 9 and bullet 11)
        BulletManager::OnUpdate(&g_BulletManager);

        std::vector<std::string> lines;
        if (getenv("DUMP_ENEMIES")) {
            for (int i = 0; i < 256; i++) {
                Enemy* e = &g_EnemyManager.enemies[i];
                if (!e->flags.isSlotOccupied) continue;
                char buf[96];
                snprintf(buf, sizeof buf, "%.4f %.4f", e->position.x, e->position.y);
                lines.push_back(buf);
            }
        } else
        for (int i = 0; i < 640; i++) {
            Bullet* b = &g_BulletManager.bullets[i];
            if (b->state == BULLET_STATE_UNUSED) continue;
            char buf[96];
            snprintf(buf, sizeof buf, "%.4f %.4f %.4f %.4f", b->pos.x, b->pos.y, b->angle, b->speed);
            lines.push_back(buf);
        }
        std::sort(lines.begin(), lines.end());
        printf("F%d %zu\n", fr, lines.size());
        for (auto& l : lines) printf(" %s\n", l.c_str());
    }
    return 0;
}
