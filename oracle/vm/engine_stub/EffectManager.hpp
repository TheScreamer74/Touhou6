#include "Effect.hpp"
#include "Rng.hpp"
#include "inttypes.hpp"
namespace th06 {
#pragma once

#ifndef COLOR_WHITE
#define COLOR_WHITE 0xffffffff
#endif

// Faithful (RNG-exact) port of EffectManager. The per-effect update callbacks
// draw g_Rng on the effect's first OnUpdate tick exactly as EffectManager.cpp
// does; the cosmetic D3DXVECTOR3 motion they also compute is omitted (effects
// are not part of the bullet/enemy dump). SpawnParticles + OnUpdate replicate
// the slot bookkeeping and call order so RNG stays in sync with the real game.
struct EffectManager {
    Effect effects[513];
    i32 nextIndex = 0, activeEffects = 0;
    i32 randomItemSpawnIndex = 0, randomItemTableIndex = 0;

    // EffectManager.cpp:51 EffectCallbackRandomSplash — 2 draws on first tick.
    static i32 RandomSplash(Effect *e) {
        if (e->timer == 0 && e->timer.HasTicked()) {
            g_Rng.GetRandomF32ZeroToOne();
            g_Rng.GetRandomF32ZeroToOne();
        }
        return 1; // EFFECT_CALLBACK_RESULT_DONE
    }
    // EffectManager.cpp:68 EffectCallbackRandomSplashBig — 2 draws.
    static i32 RandomSplashBig(Effect *e) {
        if (e->timer == 0 && e->timer.HasTicked()) {
            g_Rng.GetRandomF32ZeroToOne();
            g_Rng.GetRandomF32ZeroToOne();
        }
        return 1;
    }
    // EffectManager.cpp:152/173 EffectCallbackAttract / AttractSlow — 1 draw.
    static i32 Attract(Effect *e) {
        if (e->timer == 0 && e->timer.HasTicked()) {
            g_Rng.GetRandomF32ZeroToOne();
        }
        return 1;
    }
    // Still / Callback4 / NULL entries draw no RNG.
    static i32 NoRng(Effect *) { return 1; }

    // g_Effects (EffectManager.cpp:18) callback column — only the callback
    // matters for RNG. NULL/Still/Callback4 -> NoRng (0 draws).
    static EffectUpdateCallback callbackFor(i32 idx) {
        switch (idx) {
        case 3:  return RandomSplashBig;
        case 4: case 5: case 6: case 7: case 8: case 9: case 10: case 11:
            return RandomSplash;
        case 17: case 18:
            return Attract;
        default:
            return NoRng; // 0,1,2 (bubbles), 12, 13-15 (cb4), 16 (sc bg), 19 (still)
        }
    }

    // EffectManager.cpp:195 SpawnParticles — allocates `count` free slots,
    // cycling nextIndex; no RNG here.
    Effect *SpawnParticles(i32 effectIdx, D3DXVECTOR3 *pos, i32 count, u32 /*color*/) {
        const i32 N = (i32)(sizeof(effects) / sizeof(effects[0]));
        Effect *effect = &this->effects[this->nextIndex];
        i32 idx;
        for (idx = 0; idx < N - 1; idx++) {
            this->nextIndex++;
            if (this->nextIndex >= N - 1) {
                this->nextIndex = 0;
            }
            if (effect->inUseFlag) {
                effect = (this->nextIndex == 0) ? &this->effects[0] : effect + 1;
                continue;
            }
            effect->inUseFlag = 1;
            effect->effectId = (i8)effectIdx;
            effect->pos1 = *pos;
            effect->updateCallback = callbackFor(effectIdx);
            effect->timer.InitializeForPopup();
            effect->unk_17a = 0;
            effect->unk_17b = 0;
            count--;
            if (count == 0)
                break;
            effect = (this->nextIndex == 0) ? &this->effects[0] : effect + 1;
        }
        return effect;
    }

    // EffectManager.cpp:250 OnUpdate — run each callback (draws RNG on tick 0),
    // then tick. Effects are freed after a fixed lifetime; the pool never fills
    // in practice, so the exact lifetime is RNG-irrelevant.
    static void OnUpdate(EffectManager *mgr) {
        const i32 N = (i32)(sizeof(mgr->effects) / sizeof(mgr->effects[0]));
        Effect *effect = &mgr->effects[0];
        mgr->activeEffects = 0;
        for (i32 i = 0; i < N - 1; i++, effect++) {
            if (effect->inUseFlag == 0)
                continue;
            mgr->activeEffects++;
            if (effect->updateCallback != nullptr && (effect->updateCallback)(effect) != 1) {
                effect->inUseFlag = 0;
            }
            effect->timer.Tick();
            if (effect->timer.current >= 32) {
                effect->inUseFlag = 0;
            }
        }
    }
};
extern EffectManager g_EffectManager;
}
