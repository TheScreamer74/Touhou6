#pragma once
#include "diffbuild.hpp"
#include "inttypes.hpp"
#include <Windows.h>
#include <d3dx8math.h>
#include <cmath>

struct ZunVec2 {
    f32 x, y;
    f32 VectorLength() { return std::sqrt(x*x + y*y); }
    f64 VectorLengthF64() { return (f64)VectorLength(); }
    D3DXVECTOR2 *AsD3dXVec() { return (D3DXVECTOR2 *)this; }
};

struct ZunVec3 {
    f32 x, y, z;
    D3DXVECTOR3 *AsD3dXVec() { return (D3DXVECTOR3 *)this; }
    static void SetVecCorners(ZunVec3 *tl, ZunVec3 *br, const D3DXVECTOR3 *c, const D3DXVECTOR3 *s) {
        tl->x = c->x - s->x/2.0f; tl->y = c->y - s->y/2.0f;
        br->x = s->x/2.0f + c->x; br->y = s->y/2.0f + c->y;
    }
};

#define ZUN_MIN(x, y) ((x) > (y) ? (y) : (x))
#define ZUN_PI ((f32)(3.14159265358979323846))
#define ZUN_2PI ((f32)(ZUN_PI * 2.0f))
#define RADIANS(degrees) ((degrees * ZUN_PI / 180.0f))

// fsincos: ST0=cos, ST1=sin -> first store is cos, second is sin.
#define sincos(in, out_sine, out_cosine) do { (out_cosine) = cosf(in); (out_sine) = sinf(in); } while(0)

static inline void fsincos_wrapper(f32 *out_sine, f32 *out_cosine, f32 angle) {
    *out_cosine = cosf(angle); *out_sine = sinf(angle);
}
static inline void sincosmul(D3DXVECTOR3 *out_vel, f32 input, f32 multiplier) {
    out_vel->x = cosf(input) * multiplier;
    out_vel->y = sinf(input) * multiplier;
}
static inline f32 invertf(f32 x) { return 1.f / x; }
static inline f32 rintf_(f32 v) { return std::rint(v); }
