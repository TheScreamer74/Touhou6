/* Read-only 1.02h menu/config observations. Included by d3d8proxy.c after
 * windows.h/stdio.h. Addresses come from the locked reference globals.csv;
 * offsets from its 32-bit MainMenu/Supervisor/GameConfiguration layouts.
 * No memory writes, input changes, clock calls or game function calls. */
#ifndef TH06_REFCAP_MENU_TRACE_H
#define TH06_REFCAP_MENU_TRACE_H

static FILE *g_menu_trace;
static FILE *g_result_trace;

static int refcap_menu_config(const char *line)
{
    char path[64];
    if (sscanf(line, "resultfile %63s", path) == 1) {
        g_result_trace = fopen(path, "w");
        if (g_result_trace)
            fputs("present,state,timer,cursor,cheat_step,progress\n", g_result_trace);
        return 1;
    }
    if (sscanf(line, "menufile %63s", path) != 1) return 0;
    g_menu_trace = fopen(path, "w");
    if (g_menu_trace)
        fprintf(g_menu_trace, "present,supervisor_state,menu_state,cursor,timer,life_count,bomb_count,color_16bit,music_mode,play_sounds,default_difficulty,windowed,pending_windowed,pending_color_16bit\n");
    return 1;
}

static void refcap_menu_log(unsigned present)
{
    const unsigned char *supervisor = (const unsigned char *)0x6c6d18;
    const unsigned char *menu = (const unsigned char *)0x6d46c0;
    const unsigned char *cfg = supervisor + 0x114;
    if (g_result_trace) {
        const unsigned char *node = (const unsigned char *)0x69d918;
        for (unsigned i = 0; node && i < 128; i++) {
            if (*(const unsigned *)(node + 4) == 0x42d98e) {
                const unsigned char *result = *(const unsigned char **)(node + 0x1c);
                if (!result) break;
                fprintf(g_result_trace, "%u,%d,%d,%d,%d,", present,
                    *(const int *)(result + 8), *(const int *)(result + 4),
                    *(const int *)(result + 0x10), *(const int *)(result + 0x30));
                for (unsigned c = 0; c < 4; c++)
                    for (unsigned d = 0; d < 10; d++)
                        fprintf(g_result_trace, "%02x", *((const unsigned char *)0x69bca0 + 0x1030 + c * 0x18 + 0x0c + d));
                fputc('\n', g_result_trace);
                fflush(g_result_trace);
                break;
            }
            node = *(const unsigned char **)(node + 0x14);
        }
    }
    if (!g_menu_trace) return;
    fprintf(g_menu_trace, "%u,%d,%d,%d,%d,%u,%u,%u,%u,%u,%u,%u,%u,%u\n",
        present, *(const int *)(supervisor + 0x18c),
        *(const int *)(menu + 0x81f0), *(const int *)(menu + 0x81a0),
        *(const int *)(menu + 0x81f4),
        cfg[24], cfg[25], cfg[26], cfg[27], cfg[28], cfg[29], cfg[30],
        menu[0x8231], menu[0x8230]);
    fflush(g_menu_trace);
}

#endif
