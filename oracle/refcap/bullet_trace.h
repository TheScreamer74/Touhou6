/* Optional passive fixed-slot projectile boundary diagnostics for 1.02h. */
#ifndef TH06_REFCAP_BULLET_TRACE_H
#define TH06_REFCAP_BULLET_TRACE_H
static FILE *g_bullet_trace;
static void refcap_bullet_log(unsigned present, int scene, int frame)
{
    const char *path = getenv("TH06CAP_BULLET_BOUNDARY");
    if (!path || !*path || scene != 2 || frame < 13000 || frame > 16000) return;
    if (!g_bullet_trace) {
        g_bullet_trace = fopen(path, "w");
        if (!g_bullet_trace) return;
        fputs("present,frame,slot,x,y,vx,vy,width,height,sprite,flags,state,oob,timer,region_w,region_h\n",g_bullet_trace);
    }
    const unsigned char *gm = (const unsigned char *)0x69bca0;
    const unsigned char *b = (const unsigned char *)0x5a5ff8 + 0x5600 + 6 * 0x5c4;
    const unsigned char *sprite = *(const unsigned char **)(b+0xc0);
    unsigned state = *(const unsigned short *)(b+0x5be);
    fprintf(g_bullet_trace,"%u,%d,6,%.9g,%.9g,%.9g,%.9g,%.9g,%.9g,%d,%u,%u,%u,%d,%.9g,%.9g\n",
        present,frame,*(const float *)(b+0x560),*(const float *)(b+0x564),
        *(const float *)(b+0x56c),*(const float *)(b+0x570),
        state && sprite ? *(const float *)(sprite+0x30) : 0.0f,
        state && sprite ? *(const float *)(sprite+0x2c) : 0.0f,
        *(const short *)(b+0xb0),*(const unsigned short *)(b+0x5b8),state,
        *(const unsigned short *)(b+0x5c0),*(const int *)(b+0x5a4),
        *(const float *)(gm+0x1a44),*(const float *)(gm+0x1a48));
    fflush(g_bullet_trace);
}
#endif
