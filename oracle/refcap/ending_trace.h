/* Optional ending-entry fixture and passive END interpreter observations.
 * endingsetup <shot0..3> <difficulty0..3> <retries0..3> <seen0..1> <Present>
 * explicitly changes entry state once, from an active replay. It does not
 * patch END code, timers, assets or subsequent input. Without this directive
 * this header never writes game memory. endingfile enables passive logging.
 */
#ifndef TH06_REFCAP_ENDING_TRACE_H
#define TH06_REFCAP_ENDING_TRACE_H
static FILE *g_ending_trace;
static unsigned g_ending_setup[5];
static int g_ending_setup_enabled, g_ending_entered, g_ending_done;
static unsigned g_ending_stop;
static unsigned g_ending_tick;

static int refcap_ending_config(const char *line)
{
    char path[64];
    if (sscanf(line, "stopendingend %u", &g_ending_stop) == 1) return 1;
    if (sscanf(line, "endingfile %63s", path) == 1) {
        g_ending_trace = fopen(path, "w");
        if (g_ending_trace)
            fputs("present,tick,file_prefix_fnv,pc,has_seen,script_time,pause,clear,minimum_pause,minimum_clear,long_delay,short_delay,text_line,text_color,background_y,scroll_speed,fade_kind,fade_timer,fade_duration\n", g_ending_trace);
        return 1;
    }
    if (sscanf(line, "endingsetup %u %u %u %u %u", &g_ending_setup[0],
               &g_ending_setup[1], &g_ending_setup[2], &g_ending_setup[3],
               &g_ending_setup[4]) == 5) {
        if (g_ending_setup[0] > 3 || g_ending_setup[1] > 3 ||
            g_ending_setup[2] > 3 || g_ending_setup[3] > 1) ExitProcess(85);
        g_ending_setup_enabled = 1;
        return 1;
    }
    return 0;
}

static void refcap_ending_present(unsigned present)
{
    unsigned char *gm = (unsigned char *)0x69bca0;
    unsigned char *supervisor = (unsigned char *)0x6c6d18;
    if (g_ending_setup_enabled && present == g_ending_setup[4]) {
        if (*(int *)(supervisor + 0x18c) != 2 || !*(unsigned *)(gm + 0x1c))
            ExitProcess(86); /* Never silently set up from a different scene. */
        unsigned shot = g_ending_setup[0], difficulty = g_ending_setup[1];
        gm[0x181d] = shot / 2;
        gm[0x181e] = shot % 2;
        gm[0x1818] = g_ending_setup[2];
        *(int *)(gm + 0x10) = difficulty;
        if (g_ending_setup[3]) {
            unsigned char *clrd = gm + 0x1030 + shot * 0x18;
            clrd[0x0c + difficulty] = clrd[0x11 + difficulty] = 99;
        }
        *(int *)(supervisor + 0x18c) = 10;
        g_ending_setup_enabled = 0;
    }
    if (!g_ending_trace || g_ending_done) return;
    const unsigned char *node = (const unsigned char *)0x69d918;
    const unsigned char *ending = NULL;
    for (unsigned i = 0; node && i < 128; i++) {
        if (*(const unsigned *)(node + 4) == 0x4109c0) {
            ending = *(const unsigned char **)(node + 0x1c);
            break;
        }
        node = *(const unsigned char **)(node + 0x14);
    }
    if (!ending) {
        if (g_ending_entered) {
            /* The object is deleted on @z; terminal gets its own sidecar. */
            FILE *terminal = fopen("ending-terminal.txt", "w");
            if (terminal) {
                fprintf(terminal, "%u,%u,%d\n", present, g_ending_tick + 1,
                        *(int *)(supervisor + 0x18c));
                fclose(terminal);
            }
            g_ending_done = 1;
            fflush(NULL);
            if (g_ending_stop) ExitProcess(0);
        }
        return;
    }
    g_ending_entered = 1;
    const unsigned char *data = *(const unsigned char **)(ending + 0x1114);
    const unsigned char *pc = *(const unsigned char **)(ending + 0x116c);
    unsigned hash = 2166136261u;
    for (unsigned i = 0; i < 64; i++) hash = (hash ^ data[i]) * 16777619u;
#define END_I(o) (*(const int *)(ending + (o)))
#define END_F(o) (*(const float *)(ending + (o)))
    fprintf(g_ending_trace, "%u,%u,%u,%u,%d,%d,%d,%d,%d,%d,%d,%d,%d,%u,%.9g,%.9g,%d,%d,%d\n",
        present, ++g_ending_tick, hash, (unsigned)(pc-data), END_I(0x1118),
        END_I(0x1124), END_I(0x1130), END_I(0x113c), END_I(0x1144),
        END_I(0x1140), END_I(0x1148), END_I(0x114c), END_I(0x1154),
        *(const unsigned *)(ending + 0x1158), END_F(0x0c), END_F(0x10),
        END_I(0x1168), END_I(0x1160), END_I(0x1164));
    fflush(g_ending_trace);
#undef END_I
#undef END_F
}
#endif
