# Perfect Freeze white-index correction — ticket #10

Independent MCP Codex review of bb2489e..cfe402d found one actionable P2:
Perfect Freeze used color0, but original BulletManager.hpp defines0as gray and
15as white. The earlier MAE2.83 measurement was real but used gray; it did not
prove the original color. A named BULLET_WHITE=15 now matches the decompiled
callback, and the regression test checks offset15 and baseSprite+15.

The correction is committed separately as e2464f9. Fresh captures using the
cumulative title/white binary eda900fe… show stage2 frames9000–9002 falling
from2.8288–2.8348 MAE to0.7737–0.7798, versus the same historical original BMPs.
The initial pre-fix maximum was7.8145. Stage3 and stage5 control images are
byte-identical to8675ef1; all9scalar CSV and9pool JSONL snapshots are unchanged.
HUD/FPS remain unchanged. The adjacent receipt preserves every image hash and
regional metric. The cumulative source snapshot is recorded by its immutable
build manifest and includes the separate title-state fix; these numbers are
not attributed to a binary built from e2464f9 alone.

135 release tests pass (18ignored by default), including corrected Freeze
color/despawn/RNG/pulse assertions. The original-title test was separately run
for the title work; no extra FPS/playfield behavior is inferred from it.
The native build and WASM library check pass. This remains targeted native
verification, with residual pixels and no fresh96-case campaign or deployment.
