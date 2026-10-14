/* Optional read-only GUI observations for the locked TH06 1.02h executable.
 * Layout: Gui.hpp size0x2c, GuiImpl.vms[26], AnmVm.hpp size0x110.
 * Present observes the post-calculation state, like replay-state.csv; it is
 * not the earlier state that produced the image presented at this checkpoint.
 * No hooks, memory writes, game function calls or clock calls are added. */
#ifndef TH06_REFCAP_GUI_TRACE_H
#define TH06_REFCAP_GUI_TRACE_H

static FILE *g_gui_trace;

static int refcap_gui_config(const char *line)
{
    char path[64];
    if (sscanf(line, "guifile %63s", path) != 1) return 0;
    g_gui_trace = fopen(path, "w");
    if (g_gui_trace)
        fputs("present,frame,stage,boss_present,health_bar_state,bar_target,bar_display,opacity,lives,seconds,last_draw_seconds,dialogue,vm19_time,vm19_script,vm19_sprite,vm19_alpha,vm19_stopped\n", g_gui_trace);
    return 1;
}

static void refcap_gui_log(unsigned present, int stage, int frame, int stage_scene)
{
    if (!g_gui_trace || !stage_scene || frame < 0) return;
    const unsigned char *gui = (const unsigned char *)0x69bc30;
    const unsigned char *impl = *(const unsigned char **)(gui + 4);
    if (!impl) return;
    const unsigned char *vm = impl + 19 * 0x110;
    fprintf(g_gui_trace, "%u,%d,%d,%u,%u,%.9g,%.9g,%u,%d,%d,%d,%d,%d,%d,%d,%u,%u\n",
        present, frame, stage, gui[0x20], impl[26 * 0x110],
        *(const float *)(gui + 0x24), *(const float *)(gui + 0x28),
        *(const unsigned *)(gui + 0x10), *(const int *)(gui + 0x14),
        *(const int *)(gui + 0x18), *(const int *)(gui + 0x1c),
        *(const int *)(impl + 0x253c), *(const int *)(vm + 0x38),
        *(const short *)(vm + 0xb4), *(const short *)(vm + 0xb0),
        vm[0x7f], (*(const unsigned short *)(vm + 0x80) >> 13) & 1);
    fflush(g_gui_trace);
}

#endif
