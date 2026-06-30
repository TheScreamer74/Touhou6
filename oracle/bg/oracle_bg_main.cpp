// BG-state oracle: runs the REAL decomp Stage::OnUpdate over a stage's .std and
// dumps the per-frame camera position, facing dir and fog, to diff against the
// port's background.rs. Usage: oracle_bg <stageN.std> <frames>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include "Stage.hpp"
#include "GameManager.hpp"
#include "Supervisor.hpp"
using namespace th06;

namespace th06 { Supervisor g_Supervisor; GameManager g_GameManager; }

int main(int argc, char **argv)
{
    FILE *f = fopen(argv[1], "rb");
    if (!f) { fprintf(stderr, "open %s failed\n", argv[1]); return 1; }
    fseek(f, 0, SEEK_END); long sz = ftell(f); fseek(f, 0, SEEK_SET);
    char *buf = (char *)malloc(sz); fread(buf, 1, sz, f); fclose(f);
    int frames = atoi(argv[2]);

    static FakeDev dev;
    g_Supervisor.d3dDevice = &dev;
    g_GameManager.isTimeStopped = 0;

    Stage *s = &g_Stage;
    memset(s, 0, sizeof(Stage));
    s->stdData = (RawStageHeader *)buf;
    s->beginningOfScript = (RawStageInstr *)(buf + s->stdData->scriptOffset);
    s->instructionIndex = 0;
    s->position.x = s->position.y = s->position.z = 0;
    s->spellcardState = NOT_RUNNING;
    s->skyFogInterpDuration = 0;
    s->skyFog.color = 0xff000000;
    s->skyFog.nearPlane = 200.0f;
    s->skyFog.farPlane = 500.0f;
    s->facingDirInterpFinal.x = 0;  s->facingDirInterpFinal.y = 0;  s->facingDirInterpFinal.z = 1;
    s->facingDirInterpInitial.x = 0; s->facingDirInterpInitial.y = 0; s->facingDirInterpInitial.z = 1;
    s->facingDirInterpDuration = 1;
    s->facingDirInterpTimer.InitializeForPopup();
    s->scriptTime.InitializeForPopup();
    s->unpauseFlag = 0;
    s->objectsCount = 0; // skip UpdateObjects quad work

    for (int i = 0; i < frames; i++)
    {
        Stage::OnUpdate(s);
        printf("%.3f %.3f %.3f %.5f %.5f %.5f %08x %.2f %.2f\n",
               s->position.x, s->position.y, s->position.z,
               g_GameManager.stageCameraFacingDir.x, g_GameManager.stageCameraFacingDir.y,
               g_GameManager.stageCameraFacingDir.z,
               (unsigned)s->skyFog.color, s->skyFog.nearPlane, s->skyFog.farPlane);
    }
    return 0;
}
