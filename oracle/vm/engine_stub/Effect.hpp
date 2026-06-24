#include "ZunMath.hpp"
#include "ZunTimer.hpp"
#include "inttypes.hpp"
namespace th06 {
#pragma once

struct Effect;
typedef i32 (*EffectUpdateCallback)(Effect *);

// Particle-effect indices into g_Effects (sequential, matching the decomp enum).
enum {
    PARTICLE_EFFECT_UNK_3 = 3,
    PARTICLE_EFFECT_UNK_5 = 5,
    PARTICLE_EFFECT_UNK_6 = 6,
    PARTICLE_EFFECT_UNK_8 = 8,
    PARTICLE_EFFECT_UNK_12 = 12,
    PARTICLE_EFFECT_UNK_19 = 19,
};

// Faithful enough for the oracle: every field the per-effect update callbacks
// read/write, plus the bookkeeping the manager needs. The RNG draws are exact;
// the cosmetic D3DXVECTOR3 motion is not modelled (effects aren't dumped).
struct Effect {
    D3DXVECTOR3 position, pos2, pos1, unk_11c, unk_128;
    f32 unk_15c = 0, angleRelated = 0;
    ZunTimer timer;
    EffectUpdateCallback updateCallback = nullptr;
    i8 inUseFlag = 0, effectId = 0, unk_17a = 0, unk_17b = 0;
};

struct EffectInfo {
    i32 anmIdx;
    EffectUpdateCallback updateCallback;
};
}
