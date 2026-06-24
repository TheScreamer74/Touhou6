// Reference oracle: the decomp's exact bullet angle/speed math + RNG, compiled
// standalone, dumping (angle, speed) per spawned bullet for a fixed battery of
// shooter configs. The Rust side (oracle_dump.rs) runs the same battery through
// spawn_bullet_pattern; diffing the two outputs proves the port matches the
// decomp's BulletManager::SpawnSingleBullet (BulletManager.cpp:82-177) + Rng.cpp.
//
// Transcribed verbatim from the decomp; no D3D / engine deps.
#include <cstdint>
#include <cstdio>
#include <cmath>

typedef int32_t i32;
typedef uint32_t u32;
typedef uint16_t u16;
typedef float f32;

static const f32 ZUN_PI = 3.14159265358979323846f;
static const f32 ZUN_2PI = ZUN_PI * 2.0f;

// --- Rng.cpp (exact) ---
struct Rng {
    u16 seed;
    u32 generationCount;
    u16 GetRandomU16() {
        u16 a = (u16)((this->seed ^ 0x9630) - 0x6553);
        this->seed = (u16)((((a & 0xc000) >> 14) + a * 4) & 0xFFFF);
        this->generationCount++;
        return this->seed;
    }
    u32 GetRandomU32() { return ((u32)GetRandomU16() << 16) | GetRandomU16(); }
    f32 GetRandomF32ZeroToOne() { return (f32)GetRandomU32() / (f32)0xFFFFFFFF; }
    f32 GetRandomF32InRange(f32 range) { return GetRandomF32ZeroToOne() * range; }
};
static Rng g_Rng;

// --- utils::AddNormalizeAngle (utils.cpp:48, exact) ---
static f32 AddNormalizeAngle(f32 a, f32 b) {
    i32 i = 0;
    a += b;
    while (a > ZUN_PI) { a -= ZUN_2PI; if (i++ > 16) break; }
    while (a < -ZUN_PI) { a += ZUN_2PI; if (i++ > 16) break; }
    return a;
}

enum AimMode { FAN_AIMED, FAN, CIRCLE_AIMED, CIRCLE, OFFSET_CIRCLE_AIMED, OFFSET_CIRCLE,
               RANDOM_ANGLE, RANDOM_SPEED, RANDOM };

struct Props {
    i32 aimMode, count1, count2;
    f32 speed1, speed2, angle1, angle2;
};

// SpawnSingleBullet's angle/speed math (BulletManager.cpp:118-174), exact.
static void SpawnSingleBullet(const Props& p, i32 bulletIdx1, i32 bulletIdx2, f32 angle,
                              f32* outAngle, f32* outSpeed) {
    f32 bulletAngle = 0.0f;
    f32 bulletSpeed = p.speed1 - (p.speed1 - p.speed2) * bulletIdx2 / p.count2;
    switch (p.aimMode) {
    case FAN_AIMED:
    case FAN:
        if ((p.count1 & 1) != 0)
            bulletAngle = ((bulletIdx1 + 1) / 2) * p.angle2 + bulletAngle;
        else
            bulletAngle = (bulletIdx1 / 2) * p.angle2 + p.angle2 * 0.5f + bulletAngle;
        if ((bulletIdx1 & 1) != 0) bulletAngle *= -1.0f;
        if (p.aimMode == FAN_AIMED) bulletAngle += angle;
        bulletAngle += p.angle1;
        break;
    case CIRCLE_AIMED:
        bulletAngle += angle;
    case CIRCLE:
        bulletAngle += bulletIdx1 * ZUN_2PI / p.count1;
        bulletAngle += bulletIdx2 * p.angle2 + p.angle1;
        break;
    case OFFSET_CIRCLE_AIMED:
        bulletAngle += angle;
    case OFFSET_CIRCLE:
        bulletAngle += ZUN_PI / p.count1;
        bulletAngle += bulletIdx1 * ZUN_2PI / p.count1;
        bulletAngle += p.angle1;
        break;
    case RANDOM_ANGLE:
        bulletAngle = g_Rng.GetRandomF32InRange(p.angle1 - p.angle2) + p.angle2;
        break;
    case RANDOM_SPEED:
        bulletSpeed = g_Rng.GetRandomF32InRange(p.speed1 - p.speed2) + p.speed2;
        bulletAngle += bulletIdx1 * ZUN_2PI / p.count1;
        bulletAngle += bulletIdx2 * p.angle2 + p.angle1;
        break;
    case RANDOM:
        bulletAngle = g_Rng.GetRandomF32InRange(p.angle1 - p.angle2) + p.angle2;
        bulletSpeed = g_Rng.GetRandomF32InRange(p.speed1 - p.speed2) + p.speed2;
    }
    *outAngle = AddNormalizeAngle(bulletAngle, 0.0f);
    *outSpeed = bulletSpeed;
}

// SpawnBulletPattern loop order (BulletManager.cpp:540): idx1 over count2 (outer),
// idx2 over count1 (inner), call SpawnSingleBullet(props, idx2, idx1, angle).
static void SpawnPattern(const Props& p, f32 aimAngle) {
    for (i32 idx1 = 0; idx1 < p.count2; idx1++) {
        for (i32 idx2 = 0; idx2 < p.count1; idx2++) {
            f32 a, s;
            SpawnSingleBullet(p, idx2, idx1, aimAngle, &a, &s);
            printf("%.6f %.6f\n", a, s);
        }
    }
}

int main() {
    g_Rng.seed = 0x1234;
    g_Rng.generationCount = 0;
    f32 aimAngle = 1.2345f; // a fixed "AngleToPlayer"
    // Battery: every aim mode, with a few count/angle/speed configs.
    Props battery[] = {
        {FAN_AIMED, 1, 1, 3.0f, 1.0f, 0.1f, 0.3f},
        {FAN_AIMED, 4, 1, 3.0f, 1.0f, 0.1f, 0.3f},
        {FAN, 5, 2, 3.0f, 1.0f, 0.0f, 0.26f},
        {CIRCLE_AIMED, 12, 1, 2.5f, 2.5f, 0.0f, 0.1f},
        {CIRCLE, 16, 3, 2.0f, 1.0f, 0.2f, 0.05f},
        {OFFSET_CIRCLE_AIMED, 8, 1, 2.0f, 2.0f, 0.0f, 0.0f},
        {OFFSET_CIRCLE, 24, 1, 1.5f, 1.5f, 0.1f, 0.0f},
        {RANDOM_ANGLE, 6, 1, 2.0f, 2.0f, 1.0f, -1.0f},
        {RANDOM_SPEED, 8, 2, 3.0f, 1.0f, 0.0f, 0.2f},
        {RANDOM, 10, 1, 3.0f, 1.0f, 1.0f, -1.0f},
    };
    for (auto& p : battery) SpawnPattern(p, aimAngle);
    return 0;
}
