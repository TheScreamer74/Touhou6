/* Original SoundPlayer queue and CStreamingSound fade probe. Only enabled by
 * TH06CAP_WAVE=1 in an isolated capture. COM boundary sinks record the actual
 * original Stop / SetCurrentPosition / Play / SetVolume calls. No original
 * game's sound buffers or SoundPlayer object are used. */
#ifndef TH06_WAVE_TRACE_H
#define TH06_WAVE_TRACE_H
static FILE *wave_trace_file;
static unsigned wave_trace_case, wave_trace_tick;
typedef struct WaveTraceBuffer { void **vtable; int index; } WaveTraceBuffer;
static HRESULT __stdcall wave_trace_stop(WaveTraceBuffer *buffer)
{
    fprintf(wave_trace_file, "%u,%u,STOP,%d,0\n", wave_trace_case, wave_trace_tick, buffer->index);
    return 0;
}
static HRESULT __stdcall wave_trace_position(WaveTraceBuffer *buffer, DWORD position)
{
    fprintf(wave_trace_file, "%u,%u,POSITION,%d,%lu\n", wave_trace_case, wave_trace_tick, buffer->index, position);
    return 0;
}
static HRESULT __stdcall wave_trace_play(WaveTraceBuffer *buffer, DWORD reserved, DWORD priority, DWORD flags)
{
    (void)reserved; (void)priority;
    fprintf(wave_trace_file, "%u,%u,PLAY,%d,%lu\n", wave_trace_case, wave_trace_tick, buffer->index, flags);
    return 0;
}
static HRESULT __stdcall wave_trace_volume(WaveTraceBuffer *buffer, LONG volume)
{
    fprintf(wave_trace_file, "%u,%u,VOLUME,%d,%ld\n", wave_trace_case, wave_trace_tick, buffer->index, volume);
    return 0;
}
static void wave_trace_present(void)
{
    static int done;
    if (done || !getenv("TH06CAP_WAVE") || g_frame < 1) return;
    done = 1;
    wave_trace_file = fopen("wave-calls.csv", "w");
    FILE *variants = fopen("sound-variants.csv", "w");
    if (!wave_trace_file || !variants) return;
    fprintf(wave_trace_file, "case,tick,event,voice,value\n");
    fprintf(variants, "index,file,db100\n");
    for (unsigned index = 0; index < 32; index++) {
        unsigned char *entry = (unsigned char *)0x478528 + index * 8;
        fprintf(variants, "%u,%u,%d\n", index, *(unsigned *)entry, *(short *)(entry + 4));
    }
    fclose(variants);
    void *vtable[21] = {0};
    vtable[12] = wave_trace_play;
    vtable[13] = wave_trace_position;
    vtable[15] = wave_trace_volume;
    vtable[18] = wave_trace_stop;
    WaveTraceBuffer buffers[32];
    for (int index = 0; index < 32; index++) {
        buffers[index].vtable = vtable;
        buffers[index].index = index;
    }
    unsigned char player[0x638] = {0};
    *(void **)(player + 0x610) = player; /* non-null manager, never dereferenced */
    for (int index = 0; index < 32; index++)
        *(void **)(player + 0x208 + index * 4) = &buffers[index];
    for (int index = 0; index < 3; index++) *(int *)(player + 0x620 + index * 4) = -1;
    unsigned char old_sounds = *(unsigned char *)0x6c6e48;
    *(unsigned char *)0x6c6e48 = 1;
    wave_trace_case = 0;
    static const int requests[][5] = {{7,7,15,22,28}, {7,15,22,22,28}, {28,28,28,28,28}};
    for (wave_trace_tick = 0; wave_trace_tick < 3; wave_trace_tick++) {
        for (unsigned index = 0; index < 5; index++)
            ((void (__thiscall *)(void *, int, int))0x4311e0)(player, requests[wave_trace_tick][index], 0);
        ((void (__thiscall *)(void *))0x431270)(player);
    }
    wave_trace_tick = 3;
    ((void (__thiscall *)(void *, int, int))0x4311e0)(player, 10, 0);
    *(unsigned char *)0x6c6e48 = 0;
    ((void (__thiscall *)(void *))0x431270)(player);
    wave_trace_tick = 4;
    ((void (__thiscall *)(void *, int, int))0x4311e0)(player, 11, 0);
    *(unsigned char *)0x6c6e48 = 1;
    ((void (__thiscall *)(void *))0x431270)(player);
    *(unsigned char *)0x6c6e48 = old_sounds;
    static const float durations[] = {0.0f, 0.001f, 0.016666667f, 0.5f, 1.0f, 2.0f, 3.5f, 10.0f};
    for (wave_trace_case = 1; wave_trace_case <= 8; wave_trace_case++) {
        unsigned char streaming[0x40] = {0};
        WaveTraceBuffer *buffer_pointer = &buffers[0];
        *(void **)(streaming + 4) = &buffer_pointer;
        *(void **)(player + 0x62c) = streaming;
        ((void (__thiscall *)(void *, float))0x424e41)(player, durations[wave_trace_case - 1]);
        unsigned total = (unsigned)(durations[wave_trace_case - 1] * 60.0f);
        for (wave_trace_tick = 1; wave_trace_tick <= total + 3; wave_trace_tick++)
            ((unsigned (__thiscall *)(void *))0x43b4d0)(streaming);
    }
    fclose(wave_trace_file);
    wave_trace_file = NULL;
}
#endif
