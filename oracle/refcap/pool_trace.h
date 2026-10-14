/* Read-only 1.02h pool checkpoints. Offsets follow Player.hpp, Enemy.hpp,
 * BulletManager.hpp and ItemManager.hpp, whose 32-bit sizes are asserted by
 * the reference source. PC is an offset in the original ECL file, never an
 * address dependent on the process allocator. No game function is called. */
#ifndef TH06_REFCAP_POOL_TRACE_H
#define TH06_REFCAP_POOL_TRACE_H
static FILE *g_pool_trace;
static unsigned g_pool_stride;
static unsigned g_pool_frames[256][2];
static unsigned g_npool_frames;

static int refcap_pool_config(const char *line)
{
    char path[64]; unsigned stage, frame;
    if (sscanf(line, "poolfile %63s", path) == 1) {
        g_pool_trace = fopen(path, "w"); return 1;
    }
    if (sscanf(line, "poolstride %u", &frame) == 1) {
        g_pool_stride = frame; return 1;
    }
    if (sscanf(line, "poolframe %u %u", &stage, &frame) == 2) {
        if (g_npool_frames < 256) {
            g_pool_frames[g_npool_frames][0] = stage;
            g_pool_frames[g_npool_frames++][1] = frame;
        }
        return 1;
    }
    return 0;
}

#define POOL_F(p, o) (*(const float *)((p) + (o)))
#define POOL_I(p, o) (*(const int *)((p) + (o)))
#define POOL_H(p, o) (*(const short *)((p) + (o)))
static void refcap_pool_log(unsigned stage, unsigned frame)
{
    if (!g_pool_trace) return;
    int wanted = g_pool_stride && frame % g_pool_stride == 0;
    for (unsigned i = 0; i < g_npool_frames; i++)
        if (g_pool_frames[i][0] == stage && g_pool_frames[i][1] == frame) {
            wanted = 1;
            g_pool_frames[i][0] = 0;
        }
    if (!wanted) return;
    const unsigned char *ecl_file = *(const unsigned char **)0x487e50;
    fprintf(g_pool_trace, "{\"frame\":%u,\"stage\":%u,\"enemies\":[", frame, stage);
    const char *comma = "";
    for (unsigned i = 0; i < 256; i++) {
        const unsigned char *e = (const unsigned char *)0x4b79c8 + 8 + 0xec8 + i * 0xec8;
        if (!(e[0xe50] & 0x80)) continue;
        const unsigned char *pc = *(const unsigned char **)(e + 0x990);
        fprintf(g_pool_trace, "%s{\"slot\":%u,\"x\":%.9g,\"y\":%.9g,\"z\":%.9g,\"life\":%d,\"max_life\":%d,\"sub\":%u,\"pc\":%u,\"time\":%d,\"phase_timer\":%d}",
            comma, i, POOL_F(e,0xc6c), POOL_F(e,0xc70), POOL_F(e,0xc74), POOL_I(e,0xce4), POOL_I(e,0xce8),
            *(const unsigned short *)(e+0x9d8), (unsigned)(pc-ecl_file), POOL_I(e,0x99c), POOL_I(e,0xcf8));
        comma = ",";
    }
    fputs("],\"shots\":[", g_pool_trace); comma = "";
    for (unsigned i = 0; i < 80; i++) {
        const unsigned char *s = (const unsigned char *)0x6ca628 + 0xa28 + i * 0x158;
        if (!POOL_H(s,0x14e)) continue;
        fprintf(g_pool_trace, "%s{\"slot\":%u,\"x\":%.9g,\"y\":%.9g,\"vx\":%.9g,\"vy\":%.9g,\"width\":%.9g,\"height\":%.9g,\"angle\":%.9g,\"speed\":%.9g,\"state\":%d,\"timer\":%d,\"damage\":%d,\"type\":%d}",
            comma, i, POOL_F(s,0x110), POOL_F(s,0x114), POOL_F(s,0x128), POOL_F(s,0x12c), POOL_F(s,0x11c), POOL_F(s,0x120),
            POOL_F(s,0x13c), POOL_F(s,0x138), POOL_H(s,0x14e), POOL_I(s,0x148), POOL_H(s,0x14c), POOL_H(s,0x150));
        comma = ",";
    }
    fputs("],\"bullets\":[", g_pool_trace); comma = "";
    for (unsigned i = 0; i < 640; i++) {
        const unsigned char *b = (const unsigned char *)0x5a5ff8 + 0x5600 + i * 0x5c4;
        unsigned state = *(const unsigned short *)(b+0x5be);
        if (!state) continue;
        fprintf(g_pool_trace, "%s{\"slot\":%u,\"x\":%.9g,\"y\":%.9g,\"vx\":%.9g,\"vy\":%.9g,\"angle\":%.9g,\"speed\":%.9g,\"state\":%u,\"timer\":%d}",
            comma, i, POOL_F(b,0x560), POOL_F(b,0x564), POOL_F(b,0x56c), POOL_F(b,0x570), POOL_F(b,0x590), POOL_F(b,0x584), state, POOL_I(b,0x5a4));
        comma = ",";
    }
    fputs("],\"lasers\":[", g_pool_trace); comma = "";
    for (unsigned i = 0; i < 64; i++) {
        const unsigned char *l = (const unsigned char *)0x5a5ff8 + 0xec000 + i * 0x270;
        if (!POOL_I(l,0x258)) continue;
        fprintf(g_pool_trace, "%s{\"slot\":%u,\"x\":%.9g,\"y\":%.9g,\"angle\":%.9g,\"speed\":%.9g,\"start_offset\":%.9g,\"end_offset\":%.9g,\"width\":%.9g,\"state\":%u,\"timer\":%d}",
            comma, i, POOL_F(l,0x220), POOL_F(l,0x224), POOL_F(l,0x22c), POOL_F(l,0x240), POOL_F(l,0x230), POOL_F(l,0x234), POOL_F(l,0x23c), l[0x26c], POOL_I(l,0x264));
        comma = ",";
    }
    fputs("],\"items\":[", g_pool_trace); comma = "";
    for (unsigned i = 0; i < 512; i++) {
        const unsigned char *it = (const unsigned char *)0x69e268 + i * 0x144;
        if (!it[0x141]) continue;
        fprintf(g_pool_trace, "%s{\"slot\":%u,\"x\":%.9g,\"y\":%.9g,\"state\":%u,\"timer\":%d,\"type\":%d}",
            comma, i, POOL_F(it,0x110), POOL_F(it,0x114), it[0x143], POOL_I(it,0x13c), (signed char)it[0x140]);
        comma = ",";
    }
    fputs("]}\n", g_pool_trace);
    fflush(g_pool_trace);
}
#undef POOL_F
#undef POOL_I
#undef POOL_H
#endif
