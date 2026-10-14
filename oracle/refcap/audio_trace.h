/* Optional read-only audio-call tracing for the supplied 1.02h executable.
 * Include after g_frame; call audio_trace_install() from PROCESS_ATTACH and
 * audio_trace_present() from Present. TH06CAP_AUDIO selects the output CSV.
 * Every detour verifies the complete, instruction-aligned original prologue.
 * No replacement audio backend, game data or timing input is used by tracing.
 */
#ifndef TH06_AUDIO_TRACE_H
#define TH06_AUDIO_TRACE_H
#include <stdint.h>

static FILE *audio_trace_file;
static CRITICAL_SECTION audio_trace_lock;
static char audio_trace_track[256];
static char audio_trace_pending_track[256];

static void audio_trace_row(const char *event, const char *value, const char *detail)
{
    if (!audio_trace_file) return;
    unsigned char *gm = (unsigned char *)0x69bca0;
    /* CutChain leaves the replay pointer behind after a stage has ended. */
    unsigned char *replay = *(int *)(0x6c6d18 + 0x18c) == 2
        ? *(unsigned char **)0x6d3f18 : NULL;
    EnterCriticalSection(&audio_trace_lock);
    fprintf(audio_trace_file, "%u,%d,%d,%s,%s,%s\n", (unsigned)g_frame,
        replay && gm[0x1821] ? *(int *)replay : -1, *(int *)(gm + 0x1a34),
        event, value ? value : "", detail ? detail : "");
    LeaveCriticalSection(&audio_trace_lock);
}

/* All copied instructions below are register/stack operations, without
 * relative branches. Lengths and bytes were read from the SHA-256-identified
 * original 102h.exe, rather than guessed from a compiler's usual prologue. */
static void *audio_trace_detour(uintptr_t address, const unsigned char *prologue,
                               size_t length, void *replacement)
{
    unsigned char *target = (unsigned char *)address;
    if (length < 5 || memcmp(target, prologue, length)) return NULL;
    unsigned char *copy = VirtualAlloc(NULL, length + 5, MEM_COMMIT | MEM_RESERVE,
                                      PAGE_EXECUTE_READWRITE);
    if (!copy) return NULL;
    memcpy(copy, target, length);
    copy[length] = 0xe9;
    *(int32_t *)(copy + length + 1) = (int32_t)((target + length) - (copy + length + 5));
    DWORD previous;
    if (!VirtualProtect(target, length, PAGE_EXECUTE_READWRITE, &previous)) {
        VirtualFree(copy, 0, MEM_RELEASE);
        return NULL;
    }
    target[0] = 0xe9;
    *(int32_t *)(target + 1) = (int32_t)((unsigned char *)replacement - (target + 5));
    memset(target + 5, 0x90, length - 5);
    VirtualProtect(target, length, previous, &previous);
    FlushInstructionCache(GetCurrentProcess(), target, length);
    return copy;
}

typedef void (__thiscall *AudioVoid)(void *);
typedef int (__thiscall *AudioInt)(void *);
typedef int (__thiscall *AudioPath)(void *, const char *);
typedef int (__thiscall *AudioIndex)(void *, int);
typedef void (__thiscall *AudioSound)(void *, int, int);
typedef int (__thiscall *AudioFade)(void *, float);
static AudioSound audio_real_request;
static AudioVoid audio_real_flush;
static AudioPath audio_real_play;
static AudioInt audio_real_stop;
static AudioFade audio_real_fade;
static AudioIndex audio_real_midi_index;
static AudioIndex audio_real_wave_play;
static AudioVoid audio_real_wave_stop;

static void __fastcall audio_request(void *self, void *unused, int index, int arg)
{
    (void)unused;
    char value[32];
    snprintf(value, sizeof(value), "%d", index);
    audio_trace_row("SFX_REQUEST", value, "");
    audio_real_request(self, index, arg);
}
static void __fastcall audio_flush(void *self, void *unused)
{
    (void)unused;
    unsigned char *player = self;
    if (*(void **)(player + 0x610) && *(unsigned char *)0x6c6e48) {
        for (unsigned i = 0; i < 3; i++) {
            int index = *(int *)(player + 0x620 + i * 4);
            if (index < 0) break;
            if (index < 32 && *(void **)(player + 0x208 + index * 4)) {
                char value[32];
                snprintf(value, sizeof(value), "%d", index);
                audio_trace_row("SFX_PLAY", value, "");
            }
        }
    }
    audio_real_flush(self);
}
static int __fastcall audio_play(void *self, void *unused, const char *path)
{
    (void)unused;
    const char *name = strrchr(path, '/');
    const char *back = strrchr(path, '\\');
    if (back && (!name || back > name)) name = back;
    name = name ? name + 1 : path;
    strncpy(audio_trace_pending_track, name, sizeof(audio_trace_pending_track) - 1);
    audio_trace_row("BGM_REQUEST", name, "");
    return audio_real_play(self, path);
}
static int __fastcall audio_stop(void *self, void *unused)
{
    (void)unused;
    audio_trace_row("BGM_STOP_REQUEST", "", "");
    return audio_real_stop(self);
}
static int __fastcall audio_fade(void *self, void *unused, float seconds)
{
    (void)unused;
    char value[64];
    snprintf(value, sizeof(value), "%.9g", seconds);
    audio_trace_row("BGM_FADE", value, "");
    return audio_real_fade(self, seconds);
}
static int __fastcall audio_midi_index(void *self, void *unused, int index)
{
    (void)unused;
    char value[32];
    snprintf(value, sizeof(value), "%d", index);
    audio_trace_row("BGM_MIDI_INDEX", value, "");
    return audio_real_midi_index(self, index);
}
static int __fastcall audio_wave_play(void *self, void *unused, int looping)
{
    (void)unused;
    char value[32];
    snprintf(value, sizeof(value), "%d", looping);
    strcpy(audio_trace_track, audio_trace_pending_track);
    audio_trace_row("BGM_PLAY", audio_trace_track, value);
    return audio_real_wave_play(self, looping);
}
static void __fastcall audio_wave_stop(void *self, void *unused)
{
    (void)unused;
    if (*(void **)((unsigned char *)self + 0x62c))
        audio_trace_row("BGM_STOP", audio_trace_track, "");
    audio_real_wave_stop(self);
}

static void audio_trace_install(void)
{
    const char *path = getenv("TH06CAP_AUDIO");
    if (!path || !*path) return;
    audio_trace_file = fopen(path, "w");
    if (!audio_trace_file) return;
    InitializeCriticalSection(&audio_trace_lock);
    fprintf(audio_trace_file, "present,frame,stage,event,value,detail\n");
#define AUDIO_DETOUR(slot, addr, bytes, hook) do { \
    static const unsigned char expected[] = bytes; \
    slot = (void *)audio_trace_detour(addr, expected, sizeof(expected) - 1, (void *)hook); \
    if (!slot) { fprintf(audio_trace_file, "0,-1,0,HOOK_ERROR,%x,\n", addr); fflush(audio_trace_file); } \
} while (0)
    AUDIO_DETOUR(audio_real_request, 0x4311e0, ("\x55\x8b\xec\x83\xec\x0c"), audio_request);
    AUDIO_DETOUR(audio_real_flush, 0x431270, ("\x55\x8b\xec\x83\xec\x0c"), audio_flush);
    AUDIO_DETOUR(audio_real_play, 0x424b5d, ("\x55\x8b\xec\x81\xec\x30\x02\x00\x00"), audio_play);
    AUDIO_DETOUR(audio_real_stop, 0x424d38, ("\x55\x8b\xec\x51\x89\x4d\xfc"), audio_stop);
    AUDIO_DETOUR(audio_real_fade, 0x424d82, ("\x55\x8b\xec\x83\xec\x10"), audio_fade);
    AUDIO_DETOUR(audio_real_midi_index, 0x424ae4, ("\x55\x8b\xec\x51\x51"), audio_midi_index);
    AUDIO_DETOUR(audio_real_wave_play, 0x430ec0, ("\x55\x8b\xec\x83\xec\x0c"), audio_wave_play);
    AUDIO_DETOUR(audio_real_wave_stop, 0x430f80, ("\x55\x8b\xec\x83\xec\x10"), audio_wave_stop);
#undef AUDIO_DETOUR
}

#include "midi_trace.h"
#include "wave_trace.h"
static void audio_trace_present(void)
{
    if (audio_trace_file) fflush(audio_trace_file);
    midi_trace_present();
    wave_trace_present();
}
#endif
