// ANM-interpreter oracle: runs the REAL decomp AnmManager::ExecuteScript over a
// bg anm's quad scripts and dumps per-frame VM state (sprite size, scale,
// rotation, colour, uv-scroll, visibility) to diff against the port's BgQuadVm.
// Usage: oracle_anm <stgNbg.anm> <scriptId> <frames>
#include <cstdio>
#include <cstdlib>
#include "AnmManager.hpp"
#include "Supervisor.hpp"
using namespace th06;

namespace th06 {
Supervisor g_Supervisor;
namespace utils {
float AddNormalizeAngle(float a, float b)
{
    a += b;
    while (a > 3.14159265f) a -= 6.28318531f;
    while (a < -3.14159265f) a += 6.28318531f;
    return a;
}
}
namespace FileSystem {
uint8_t *OpenPath(char *path, bool)
{
    FILE *f = fopen(path, "rb");
    if (!f) return nullptr;
    fseek(f, 0, SEEK_END); long n = ftell(f); fseek(f, 0, SEEK_SET);
    uint8_t *b = (uint8_t *)malloc(n); fread(b, 1, n, f); fclose(f);
    return b;
}
}
}

int main(int argc, char **argv)
{
    char *path = argv[1];
    i32 scriptId = atoi(argv[2]);
    i32 frames = atoi(argv[3]);

    g_AnmManager = new AnmManager();
    if (g_AnmManager->LoadAnm(0, path, 0) != ZUN_SUCCESS) { fprintf(stderr, "LoadAnm failed\n"); return 1; }

    AnmVm vm;
    vm.Initialize();
    g_AnmManager->SetAndExecuteScriptIdx(&vm, scriptId);

    for (i32 i = 0; i < frames; i++)
    {
        g_AnmManager->ExecuteScript(&vm);
        f32 wpx = vm.sprite ? vm.sprite->widthPx : 0.0f;
        f32 hpx = vm.sprite ? vm.sprite->heightPx : 0.0f;
        printf("%.0f %.0f %.4f %.4f %.5f %.5f %.5f %08x %.5f %.5f %d\n",
               wpx, hpx, vm.scaleX, vm.scaleY, vm.rotation.x, vm.rotation.y, vm.rotation.z,
               (unsigned)vm.color, vm.uvScrollPos.x, vm.uvScrollPos.y, (int)vm.flags.isVisible);
    }
    return 0;
}
