/* Passive 1.02h menu/StageMenu observer. The StageMenu offsets follow
 * AsciiManager's size-checked layout: 2*0x110 + 256*0x60 + 0x24. */
#ifndef TH06_REFCAP_TRANSITION_TRACE_H
#define TH06_REFCAP_TRANSITION_TRACE_H
static FILE *g_transition_trace;
static void transition_trace_log(unsigned present)
{
    const unsigned char *supervisor = (const unsigned char *)0x6c6d18;
    const unsigned char *gm = (const unsigned char *)0x69bca0;
    const unsigned char *menu = (const unsigned char *)0x6d46c0;
    const unsigned char *pause = (const unsigned char *)0x481b44;
    const unsigned char *retry = (const unsigned char *)0x4822bc;
    const unsigned char *manager = *(const unsigned char **)0x6d3f18;
    int scene = *(const int *)(supervisor + 0x18c);
    if (!g_transition_trace) {
        const char *path = getenv("TH06CAP_TRANSITIONS");
        if (!path || !*path) return;
        g_transition_trace = fopen(path, "w");
        if (!g_transition_trace) return;
        fputs("present,scene,frame,stage,pause_state,pause_timer,retry_state,retry_timer,game_menu,retry,menu_state,cursor\n", g_transition_trace);
    }
    fprintf(g_transition_trace, "%u,%d,%d,%d,%u,%d,%u,%d,%u,%u,%d,%d\n",
        present, scene, scene == 2 && manager && gm[0x1821] ? *(const int *)manager : -1,
        *(const int *)(gm+0x1a34), *(const unsigned *)pause, *(const int *)(pause+4),
        *(const unsigned *)retry, *(const int *)(retry+4), gm[0x181f], gm[0x1820],
        *(const int *)(menu+0x81f0), *(const int *)(menu+0x81a0));
    fflush(g_transition_trace);
}
#endif
