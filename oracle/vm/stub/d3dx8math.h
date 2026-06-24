// Minimal d3dx8math.h stub for compiling the decomp's ECL sim on clang/macOS.
// Provides just the vector type + operators + functions the ECL/bullet code uses.
#pragma once
#include <cmath>

typedef struct D3DXVECTOR2 {
    float x, y;
    D3DXVECTOR2() : x(0), y(0) {}
    D3DXVECTOR2(float a, float b) : x(a), y(b) {}
} D3DXVECTOR2;

typedef struct D3DXVECTOR3 {
    float x, y, z;
    D3DXVECTOR3() : x(0), y(0), z(0) {}
    D3DXVECTOR3(float a, float b, float c) : x(a), y(b), z(c) {}
    D3DXVECTOR3 operator+(const D3DXVECTOR3& o) const { return D3DXVECTOR3(x+o.x, y+o.y, z+o.z); }
    D3DXVECTOR3 operator-(const D3DXVECTOR3& o) const { return D3DXVECTOR3(x-o.x, y-o.y, z-o.z); }
    D3DXVECTOR3 operator-() const { return D3DXVECTOR3(-x, -y, -z); }
    D3DXVECTOR3 operator*(float s) const { return D3DXVECTOR3(x*s, y*s, z*s); }
    D3DXVECTOR3 operator/(float s) const { return D3DXVECTOR3(x/s, y/s, z/s); }
    D3DXVECTOR3& operator+=(const D3DXVECTOR3& o) { x+=o.x; y+=o.y; z+=o.z; return *this; }
    D3DXVECTOR3& operator-=(const D3DXVECTOR3& o) { x-=o.x; y-=o.y; z-=o.z; return *this; }
    D3DXVECTOR3& operator*=(float s) { x*=s; y*=s; z*=s; return *this; }
    float operator[](int i) const { return (&x)[i]; }
    float& operator[](int i) { return (&x)[i]; }
} D3DXVECTOR3;

inline D3DXVECTOR3 operator*(float s, const D3DXVECTOR3& v) { return v * s; }

typedef struct D3DXMATRIX { float m[4][4]; } D3DXMATRIX;

inline float D3DXVec3Length(const D3DXVECTOR3* v) {
    return std::sqrt(v->x*v->x + v->y*v->y + v->z*v->z);
}
inline D3DXVECTOR3* D3DXVec3Normalize(D3DXVECTOR3* out, const D3DXVECTOR3* v) {
    float len = D3DXVec3Length(v);
    if (len > 0.0f) { out->x = v->x/len; out->y = v->y/len; out->z = v->z/len; }
    else { out->x = out->y = out->z = 0.0f; }
    return out;
}
inline D3DXMATRIX* D3DXMatrixRotationZ(D3DXMATRIX* m, float) { return m; }
inline D3DXMATRIX* D3DXMatrixRotationX(D3DXMATRIX* m, float) { return m; }
inline D3DXMATRIX* D3DXMatrixRotationY(D3DXMATRIX* m, float) { return m; }
inline D3DXMATRIX* D3DXMatrixMultiply(D3DXMATRIX* o, const D3DXMATRIX*, const D3DXMATRIX*) { return o; }
inline D3DXMATRIX* D3DXMatrixIdentity(D3DXMATRIX* m) { return m; }
#define D3DCOLOR_RGBA(r,g,b,a) ((D3DCOLOR)(((a)<<24)|((r)<<16)|((g)<<8)|(b)))
#include <cstdlib>
