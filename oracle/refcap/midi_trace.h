/* Original executable MIDI scheduler probe. TH06CAP_MIDI points to a private
 * directory containing extracted th06_01.mid ... th06_17.mid. It runs the
 * actual original constructor, ParseFile, LoadTracks, OnTimerElapsed and
 * SetFadeOut instructions on isolated objects, with the MIDI-device boundary
 * replaced by an event sink. No game object, input or music data is changed.
 * Requires audio_trace_detour, from audio_trace.h. Invoke midi_trace_present
 * after the game's first Present, once the original allocator is initialized.
 */
#ifndef TH06_MIDI_TRACE_H
#define TH06_MIDI_TRACE_H
#include <mmsystem.h>

static int midi_trace_active;
static FILE *midi_trace_events;
static unsigned midi_trace_callback;
static unsigned midi_trace_track;
static unsigned midi_trace_messages;
typedef int (__thiscall *MidiShort)(void *, unsigned char, unsigned char, unsigned char);
typedef int (__thiscall *MidiLong)(void *, MIDIHDR *);
static MidiShort midi_trace_real_short;
static MidiLong midi_trace_real_long;

static int __fastcall midi_trace_short(void *self, void *unused,
                                     unsigned char status, unsigned char a, unsigned char b)
{
    (void)unused;
    if (!midi_trace_active) return midi_trace_real_short(self, status, a, b);
    fprintf(midi_trace_events, "%u,%u,%u,%u,%u,\n", midi_trace_track,
            midi_trace_callback, status, a, b);
    midi_trace_messages++;
    return 0;
}
static int __fastcall midi_trace_long(void *self, void *unused, MIDIHDR *header)
{
    (void)unused;
    if (!midi_trace_active) return midi_trace_real_long(self, header);
    fprintf(midi_trace_events, "%u,%u,240,0,0,", midi_trace_track, midi_trace_callback);
    for (unsigned i = 0; i < header->dwBufferLength; i++)
        fprintf(midi_trace_events, "%02x", (unsigned char)header->lpData[i]);
    fputc('\n', midi_trace_events);
    midi_trace_messages++;
    /* Device rejects the packet after recording it. The original caller then
     * releases the header immediately; its parser/scheduler state is unchanged. */
    return 1;
}
static uint64_t midi_trace_hash(uint64_t hash, const void *bytes, unsigned size)
{
    const unsigned char *p = bytes;
    while (size--) { hash ^= *p++; hash *= UINT64_C(1099511628211); }
    return hash;
}
static void midi_trace_present(void)
{
    static int done;
    const char *directory = getenv("TH06CAP_MIDI");
    if (done || !directory || !*directory || g_frame < 1) return;
    done = 1;
    static const unsigned char short_prologue[] = {0x55, 0x8b, 0xec, 0x83, 0xec, 0x08};
    static const unsigned char long_prologue[] = {0x55, 0x8b, 0xec, 0x51, 0x89, 0x4d, 0xfc};
    midi_trace_real_short = (MidiShort)audio_trace_detour(0x421be0, short_prologue,
        sizeof(short_prologue), (void *)midi_trace_short);
    midi_trace_real_long = (MidiLong)audio_trace_detour(0x421b90, long_prologue,
        sizeof(long_prologue), (void *)midi_trace_long);
    if (!midi_trace_real_short || !midi_trace_real_long) return;
    midi_trace_events = fopen("midi-events.csv", "w");
    FILE *summary = fopen("midi-summary.csv", "w");
    if (!midi_trace_events || !summary) return;
    fprintf(midi_trace_events, "track,callback,status,a,b,sysex\n");
    fprintf(summary, "track,callbacks,messages,clock_hash\n");
    for (midi_trace_track = 1; midi_trace_track <= 17; midi_trace_track++) {
        char path[MAX_PATH];
        snprintf(path, sizeof(path), "%s/th06_%02u.mid", directory, midi_trace_track);
        FILE *input = fopen(path, "rb");
        if (!input) { fprintf(summary, "%u,0,0,missing\n", midi_trace_track); continue; }
        fseek(input, 0, SEEK_END);
        long size = ftell(input);
        rewind(input);
        unsigned char *data = malloc((size_t)size);
        if (!data || fread(data, 1, (size_t)size, input) != (size_t)size) {
            fclose(input); free(data); continue;
        }
        fclose(input);
        unsigned char *object = VirtualAlloc(NULL, 0x300, MEM_COMMIT | MEM_RESERVE, PAGE_READWRITE);
        ((void *(__thiscall *)(void *))0x421df0)(object);
        *(void **)(object + 0x94 + 31 * 4) = data;
        ((int (__thiscall *)(void *, int))0x422140)(object, 31);
        *(void **)(object + 0x94 + 31 * 4) = NULL;
        free(data);
        ((void (__thiscall *)(void *))0x4223c0)(object); /* LoadTracks */
        midi_trace_messages = 0;
        midi_trace_active = 1;
        uint64_t hash = UINT64_C(14695981039346656037);
        for (midi_trace_callback = 0; midi_trace_callback < 603000; midi_trace_callback++) {
            if (midi_trace_callback == 600000)
                ((unsigned (__thiscall *)(void *, unsigned))0x422630)(object, 2000);
            ((void (__thiscall *)(void *))0x422680)(object);
            hash = midi_trace_hash(hash, object + 0x120, 4); /* tempo, LE u32 */
            hash = midi_trace_hash(hash, object + 0x128, 8); /* playcount1, LE u64 */
            hash = midi_trace_hash(hash, object + 0x130, 8); /* playcount2, LE u64 */
        }
        midi_trace_active = 0;
        fprintf(summary, "%u,%u,%u,%016llx\n", midi_trace_track,
                midi_trace_callback, midi_trace_messages, (unsigned long long)hash);
        fflush(summary);
        fflush(midi_trace_events);
        ((void (__thiscall *)(void *))0x4220b0)(object); /* ClearTracks */
        VirtualFree(object, 0, MEM_RELEASE);
    }
    fclose(summary);
    fclose(midi_trace_events);
    midi_trace_events = NULL;
}
#endif
