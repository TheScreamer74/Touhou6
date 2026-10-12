/*
 * Reference-capture proxy d3d8.dll for the real TH06 1.02h (#9).
 *
 * Dropped next to 102h.exe (loaded instead of the system d3d8 by DLL search
 * order; under Wine with WINEDLLOVERRIDES="d3d8=n,b"). Makes the real game a
 * deterministic, scripted, frame-dumping oracle without touching its files:
 *
 *  - Direct3DCreate8 forwards to the system d3d8, then vtable-hooks
 *    IDirect3D8::CreateDevice -> IDirect3DDevice8::Present.
 *  - Present: copies the back buffer to a sysmem surface (CopyRects), converts
 *    to 24bpp and writes capture/frame_%06u.bmp; increments the frame counter.
 *  - IAT patches on the exe (applied in DllMain, before WinMain):
 *      timeGetTime      -> fake clock: base + 1ms per call from the main
 *                          thread. Fixes the RNG seed (Supervisor.cpp:330
 *                          seeds g_Rng from the first call) and makes the
 *                          frame limiter (GameWindow.cpp, FRAME_TIME=1000/60)
 *                          tick deterministically at uncapped real speed.
 *      DirectInput8Create -> fails, so Controller::GetInput falls back to the
 *                          GetKeyboardState path (Supervisor.cpp:364-377).
 *      GetKeyboardState -> scripted per-frame key states from th06cap.txt.
 *
 * Config (th06cap.txt in the game dir; '#' comments):
 *   capdir   <dir>            output dir for BMPs (created; default "capture")
 *   capstart <frame>          first Present to dump (default: never)
 *   capend   <frame>          one past the last Present to dump
 *   key <start> <end> <name>  hold key for logic frames [start, end)
 *                             names: Z X Q S SHIFT CTRL ESC RETURN UP DOWN
 *                             LEFT RIGHT HOME, or a raw 0xNN VK code
 *
 * Frame indexing: the game runs one logic tick per Present, so the input
 * frame counter == the capture frame index == Present count.
 */
#define CINTERFACE
#define COBJMACROS
#include <windows.h>
#include <d3d8.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

/* ---- state ---- */

#define FAKE_TIME_BASE 100000u

typedef struct KeyRange {
    unsigned start, end; /* [start, end) in logic frames */
    unsigned vk;
} KeyRange;

static KeyRange g_keys[256];
static int g_nkeys;
static char g_capdir[MAX_PATH] = "capture";
static unsigned g_capstart = 0xffffffffu;
static unsigned g_capend;
static unsigned g_capstride = 1;   /* dump every Nth frame in [start,end) */
static int g_realtime;              /* Sleep per Present so a human can watch */
static int g_timing;                /* log per-Present timing (env TH06CAP_TIMING) */
static int g_god;                   /* no-op Player::Die (reach later stages) */
static int g_practice;              /* force all stages unlocked for Practice */
static int g_forcestage;            /* if >0, force currentStage = g_forcestage-1 */
static volatile LONG g_frame;       /* Present count == logic frame */
static FILE *g_log;
static FILE *g_state;
static unsigned g_stopafter;
static FILE *g_detail;
static FILE *g_effects;
static unsigned g_detailstart;
static unsigned g_detailend = 0xffffffffu;

/* 1.02h globals/layout from globals.csv and the size-checked decomp headers.
 * Read only: logging must not change the replay, RNG, collision or score. */
static void log_replay_state(unsigned present)
{
    unsigned char *gm = (unsigned char *)0x69bca0;
    unsigned char *player = (unsigned char *)0x6ca628;
    unsigned char *manager = *(unsigned char **)0x6d3f18;
    if (!g_state || !manager || !gm[0x1821]) return;
    unsigned bullets = 0, lasers = 0, items = 0;
    unsigned char *bm = (unsigned char *)0x5a5ff8;
    for (unsigned i = 0; i < 640; i++) {
        unsigned char *b = bm + 0x5600 + i * 0x5c4;
        unsigned state = *(unsigned short *)(b + 0x5be);
        bullets += state != 0;
        if (state && g_detail && (unsigned)*(int *)manager >= g_detailstart && (unsigned)*(int *)manager <= g_detailend)
            fprintf(g_detail, "%d,%u,%.9g,%.9g,%.9g,%.9g,%.9g,%.9g,%u,%d\n", *(int *)manager, i,
                *(float *)(b+0x560), *(float *)(b+0x564), *(float *)(b+0x590), *(float *)(b+0x584),
                *(float *)(b+0x56c), *(float *)(b+0x570), state, *(int *)(b+0x5a4));
    }
    if (g_detail) fflush(g_detail);
    for (unsigned i = 0; i < 64; i++)
        lasers += *(int *)(bm + 0x5600 + 640 * 0x5c4 + i * 0x270 + 0x258) != 0;
    for (unsigned i = 0; i < 512; i++)
        items += *((unsigned char *)0x69e268 + i * 0x144 + 0x141) != 0;
    if (g_effects && (unsigned)*(int *)manager >= g_detailstart && (unsigned)*(int *)manager <= g_detailend) {
        for (unsigned i = 0; i < 512; i++) {
            unsigned char *e = (unsigned char *)0x487fe0 + 8 + i * 0x17c;
            if (!e[0x178]) continue;
            fprintf(g_effects, "%d,%u,%d,%u,%u,%.9g,%.9g,%.9g\n", *(int *)manager, i,
                *(int *)(e+0x16c), *(unsigned *)(e+0x174), *(unsigned *)(e+0xc0),
                *(float *)(e+0x110), *(float *)(e+0x114), *(float *)(e+0x118));
        }
        fflush(g_effects);
    }
    fprintf(g_state, "%u,%d,%d,%.6f,%.6f,%d,%d,%d,%u,%u,%u,%d,%d,%u,%u,%u,%u,%u\n",
        present, *(int *)manager, *(int *)(gm + 0x1a34),
        *(float *)(player + 0x440), *(float *)(player + 0x444),
        (signed char)gm[0x181a], (signed char)gm[0x181b], *(int *)(gm + 0x1810),
        *(unsigned *)(gm + 4), *(unsigned *)(gm + 0x14), *(unsigned *)(gm + 0x18),
        *(int *)(gm + 0x1a70), *(int *)(gm + 0x1a7c), *(unsigned short *)0x69d8f8,
        bullets, lasers, items, player[0x9e4]);
    fflush(g_state);
}

/* Player::Die in 102h.exe (config/mapping.csv: 0x427770, __thiscall void(Player*)).
 * ImageBase 0x400000, no dynamic base, so this VA is directly patchable. */
#define ADDR_PLAYER_DIE 0x427770
/* g_GameManager (config/globals.csv: 0x69bca0). clrd at +0x1030, stride 0x18
 * (Clrd), difficultyClearedWithoutRetries at +0x11 (verified via offsetof against
 * the ZUN_ASSERT_SIZE values). MainMenu::ChoosePracticeLevel offers stages up to
 * that byte, so forcing it unlocks the practice stage-select. */
#define ADDR_GAMEMANAGER 0x69bca0
#define OFF_CLRD 0x1030
#define CLRD_STRIDE 0x18
#define OFF_DCWOR 0x11 /* difficultyClearedWithoutRetries */
#define OFF_DCWR 0x0c  /* difficultyClearedWithRetries */
/* GameManager.currentStage at +0x1a34 (offsetof-verified). Practice sets it from
 * the stage-select cursor (MainMenu.cpp:837); forcing it makes any confirm start
 * the target stage regardless of cursor — sidesteps the fragile DOWN×N nav. */
#define OFF_CURRENT_STAGE 0x1a34

static void force_practice_unlock(void)
{
    /* g_GameManager is a writable global; ParseClrd reloads clrd from score.dat
     * on menu entry, so re-assert every Present. 6 = all 6 story stages. */
    for (int c = 0; c < 4; c++) {
        unsigned char *clrd =
            (unsigned char *)(ADDR_GAMEMANAGER + OFF_CLRD + c * CLRD_STRIDE);
        memset(clrd + OFF_DCWR, 6, 5);
        memset(clrd + OFF_DCWOR, 6, 5);
    }
}

static void logf_(const char *fmt, ...)
{
    if (!g_log)
        return;
    va_list ap;
    va_start(ap, fmt);
    vfprintf(g_log, fmt, ap);
    va_end(ap);
    fflush(g_log);
}

/* ---- config ---- */

static unsigned vk_from_name(const char *s)
{
    static const struct { const char *n; unsigned vk; } tbl[] = {
        {"Z", 'Z'}, {"X", 'X'}, {"Q", 'Q'}, {"S", 'S'},
        {"SHIFT", VK_SHIFT}, {"CTRL", VK_CONTROL}, {"ESC", VK_ESCAPE},
        {"RETURN", VK_RETURN}, {"ENTER", VK_RETURN},
        {"UP", VK_UP}, {"DOWN", VK_DOWN}, {"LEFT", VK_LEFT},
        {"RIGHT", VK_RIGHT}, {"HOME", VK_HOME},
    };
    for (size_t i = 0; i < sizeof(tbl) / sizeof(tbl[0]); i++)
        if (!_stricmp(s, tbl[i].n))
            return tbl[i].vk;
    return (unsigned)strtoul(s, NULL, 0); /* raw 0xNN */
}

static void load_config(void)
{
    FILE *f = fopen("th06cap.txt", "r");
    if (!f) {
        logf_("no th06cap.txt; capture disabled, keys empty\n");
        return;
    }
    char line[256];
    while (fgets(line, sizeof(line), f)) {
        char a[64], b[64];
        unsigned s, e;
        if (line[0] == '#' || line[0] == '\n')
            continue;
        if (sscanf(line, "capdir %63s", a) == 1) {
            strncpy(g_capdir, a, sizeof(g_capdir) - 1);
        } else if (sscanf(line, "capstart %u", &s) == 1) {
            g_capstart = s;
        } else if (sscanf(line, "capend %u", &e) == 1) {
            g_capend = e;
        } else if (sscanf(line, "capstride %u", &s) == 1) {
            g_capstride = s ? s : 1;
        } else if (sscanf(line, "realtime %u", &s) == 1) {
            g_realtime = (int)s;
        } else if (sscanf(line, "stopafter %u", &s) == 1) {
            g_stopafter = s;
        } else if (sscanf(line, "statefile %63s", a) == 1) {
            g_state = fopen(a, "w");
            if (g_state) fprintf(g_state, "present,frame,stage,x,y,lives,bombs,power,score,graze_stage,graze_total,rank,subrank,rng,bullets,lasers,items,particle_timer\n");
        } else if (sscanf(line, "detailfile %63s", a) == 1) {
            g_detail = fopen(a, "w");
            if (g_detail) fprintf(g_detail, "frame,slot,x,y,angle,speed,vx,vy,state,timer\n");
        } else if (sscanf(line, "effectfile %63s", a) == 1) {
            g_effects = fopen(a, "w");
            if (g_effects) fprintf(g_effects, "frame,slot,timer,callback,sprite,x,y,z\n");
        } else if (sscanf(line, "detailstart %u", &s) == 1) {
            g_detailstart = s;
        } else if (sscanf(line, "detailend %u", &s) == 1) {
            g_detailend = s;
        } else if (sscanf(line, "god %u", &s) == 1) {
            g_god = (int)s;
        } else if (sscanf(line, "practice %u", &s) == 1) {
            g_practice = (int)s;
        } else if (sscanf(line, "forcestage %u", &s) == 1) {
            g_forcestage = (int)s;
        } else if (sscanf(line, "key %u %u %63s", &s, &e, b) == 3) {
            if (g_nkeys < (int)(sizeof(g_keys) / sizeof(g_keys[0]))) {
                g_keys[g_nkeys].start = s;
                g_keys[g_nkeys].end = e;
                g_keys[g_nkeys].vk = vk_from_name(b) & 0xff;
                g_nkeys++;
            }
        } else {
            /* tap <first> <period> <count> <name>: <count> presses (4 frames
             * each) every <period> frames from <first> -- robustly walks the
             * menu chain by re-confirming defaults, tolerant of nav timing. */
            unsigned first, period, count;
            if (sscanf(line, "tap %u %u %u %63s", &first, &period, &count, b) ==
                4) {
                unsigned vk = vk_from_name(b) & 0xff;
                for (unsigned i = 0; i < count &&
                     g_nkeys < (int)(sizeof(g_keys) / sizeof(g_keys[0]));
                     i++) {
                    g_keys[g_nkeys].start = first + i * period;
                    g_keys[g_nkeys].end = first + i * period + 4;
                    g_keys[g_nkeys].vk = vk;
                    g_nkeys++;
                }
            }
        }
    }
    fclose(f);
    logf_("config: capdir=%s capstart=%u capend=%u keys=%d\n",
          g_capdir, g_capstart, g_capend, g_nkeys);
}

/* ---- IAT patching (the exe's import table) ---- */

static void patch_iat(const char *dllname, const char *func, void *newfn)
{
    BYTE *mod = (BYTE *)GetModuleHandleA(NULL);
    IMAGE_DOS_HEADER *dos = (IMAGE_DOS_HEADER *)mod;
    IMAGE_NT_HEADERS *nt = (IMAGE_NT_HEADERS *)(mod + dos->e_lfanew);
    IMAGE_DATA_DIRECTORY dir =
        nt->OptionalHeader.DataDirectory[IMAGE_DIRECTORY_ENTRY_IMPORT];
    if (!dir.VirtualAddress)
        return;
    IMAGE_IMPORT_DESCRIPTOR *imp =
        (IMAGE_IMPORT_DESCRIPTOR *)(mod + dir.VirtualAddress);
    for (; imp->Name; imp++) {
        if (_stricmp((const char *)(mod + imp->Name), dllname))
            continue;
        IMAGE_THUNK_DATA *orig =
            (IMAGE_THUNK_DATA *)(mod + imp->OriginalFirstThunk);
        IMAGE_THUNK_DATA *iat = (IMAGE_THUNK_DATA *)(mod + imp->FirstThunk);
        for (; orig->u1.AddressOfData; orig++, iat++) {
            if (orig->u1.Ordinal & IMAGE_ORDINAL_FLAG)
                continue;
            IMAGE_IMPORT_BY_NAME *ibn =
                (IMAGE_IMPORT_BY_NAME *)(mod + orig->u1.AddressOfData);
            if (strcmp((const char *)ibn->Name, func))
                continue;
            DWORD prot;
            VirtualProtect(&iat->u1.Function, sizeof(void *),
                           PAGE_EXECUTE_READWRITE, &prot);
            iat->u1.Function = (DWORD_PTR)newfn;
            VirtualProtect(&iat->u1.Function, sizeof(void *), prot, &prot);
            logf_("patched %s!%s\n", dllname, func);
        }
    }
}

/* ---- hooks: time, input ---- */

static DWORD g_mainTid;
static DWORD g_lastPollFrame, g_pollsThisFrame, g_creepMs;

static DWORD WINAPI my_timeGetTime(void)
{
    /* Clock derived exactly from the Present count: frame k reads
     * base + k*(1000/60). The frame limiter (GameWindow.cpp, FRAME_TIME =
     * 1000/60) then passes exactly once per Present — never a double logic
     * tick — and the slowdown compensation (effectiveFramerateMultiplier,
     * GameWindow.cpp:119-140) computes exactly 1.0, so game logic ticks
     * whole frames like a healthy 60 fps run. First call (before any
     * Present) returns the base: the fixed RNG seed (Supervisor.cpp:330).
     *
     * Busy-wait escape: the game has real-time waits with no Present inside
     * (menu-music delay MainMenu.cpp:1019 = 3000ms; BGM load
     * SoundPlayer.cpp:212 = 100ms) which would deadlock a frame-locked
     * clock. If one frame is polled implausibly often (normal frames poll
     * ~20x), a monotone creep term advances 1ms per poll until the wait
     * exits — instant in real time and deterministic (creep gained = exactly
     * the wait threshold). Main-thread only, so sound threads can't perturb
     * the pacing. */
    if (GetCurrentThreadId() == g_mainTid) {
        DWORD f = (DWORD)g_frame;
        if (f != g_lastPollFrame) {
            g_lastPollFrame = f;
            g_pollsThisFrame = 0;
        }
        if (++g_pollsThisFrame > 64)
            g_creepMs++;
    }
    return FAKE_TIME_BASE + (DWORD)((double)g_frame * (1000.0 / 60.0)) +
           g_creepMs;
}

static BOOL WINAPI my_GetKeyboardState(PBYTE state)
{
    unsigned f = (unsigned)g_frame;
    memset(state, 0, 256);
    for (int i = 0; i < g_nkeys; i++)
        if (f >= g_keys[i].start && f < g_keys[i].end)
            state[g_keys[i].vk] = 0x80;
    return TRUE;
}

static HRESULT WINAPI my_DirectInput8Create(HINSTANCE h, DWORD ver,
                                            REFIID riid, LPVOID *out,
                                            IUnknown *outer)
{
    (void)h; (void)ver; (void)riid; (void)outer;
    if (out)
        *out = NULL;
    return E_FAIL; /* Supervisor falls back to GetKeyboardState */
}

/* ---- BMP writer (24bpp, top-down) ---- */

#pragma pack(push, 1)
typedef struct {
    WORD bfType;
    DWORD bfSize;
    WORD r1, r2;
    DWORD bfOffBits;
    DWORD biSize;
    LONG biWidth, biHeight;
    WORD biPlanes, biBitCount;
    DWORD biCompression, biSizeImage;
    LONG biXPels, biYPels;
    DWORD biClrUsed, biClrImportant;
} BmpHeader;
#pragma pack(pop)

static BYTE *g_rgb; /* reused per-frame 24bpp buffer */
static unsigned g_rgbcap;

static void write_bmp(unsigned frame, const BYTE *px, UINT w, UINT h,
                      INT pitch, D3DFORMAT fmt)
{
    unsigned rowbytes = w * 3, imgsize = rowbytes * h;
    if (g_rgbcap < imgsize) {
        free(g_rgb);
        g_rgb = (BYTE *)malloc(imgsize);
        g_rgbcap = g_rgb ? imgsize : 0;
        if (!g_rgb)
            return;
    }
    for (UINT y = 0; y < h; y++) {
        const BYTE *src = px + (size_t)y * pitch;
        BYTE *dst = g_rgb + (size_t)y * rowbytes;
        if (fmt == D3DFMT_X8R8G8B8 || fmt == D3DFMT_A8R8G8B8) {
            for (UINT x = 0; x < w; x++) {
                dst[x * 3 + 0] = src[x * 4 + 0];
                dst[x * 3 + 1] = src[x * 4 + 1];
                dst[x * 3 + 2] = src[x * 4 + 2];
            }
        } else if (fmt == D3DFMT_R5G6B5) {
            const WORD *s16 = (const WORD *)src;
            for (UINT x = 0; x < w; x++) {
                WORD p = s16[x];
                dst[x * 3 + 0] = (BYTE)((p & 0x1f) << 3 | (p & 0x1f) >> 2);
                dst[x * 3 + 1] = (BYTE)((p >> 5 & 0x3f) << 2 | (p >> 5 & 0x3f) >> 4);
                dst[x * 3 + 2] = (BYTE)((p >> 11) << 3 | (p >> 11) >> 2);
            }
        } else if (fmt == D3DFMT_X1R5G5B5 || fmt == D3DFMT_A1R5G5B5) {
            const WORD *s16 = (const WORD *)src;
            for (UINT x = 0; x < w; x++) {
                WORD p = s16[x];
                dst[x * 3 + 0] = (BYTE)((p & 0x1f) << 3 | (p & 0x1f) >> 2);
                dst[x * 3 + 1] = (BYTE)((p >> 5 & 0x1f) << 3 | (p >> 5 & 0x1f) >> 2);
                dst[x * 3 + 2] = (BYTE)((p >> 10 & 0x1f) << 3 | (p >> 10 & 0x1f) >> 2);
            }
        } else {
            return; /* unsupported format; logged at first capture */
        }
    }
    char path[MAX_PATH];
    snprintf(path, sizeof(path), "%s\\frame_%06u.bmp", g_capdir, frame);
    FILE *f = fopen(path, "wb");
    if (!f)
        return;
    BmpHeader hd;
    memset(&hd, 0, sizeof(hd));
    hd.bfType = 0x4d42; /* BM */
    hd.bfOffBits = sizeof(hd);
    hd.bfSize = hd.bfOffBits + imgsize;
    hd.biSize = 40;
    hd.biWidth = (LONG)w;
    hd.biHeight = -(LONG)h; /* negative = top-down, matching D3D rows */
    hd.biPlanes = 1;
    hd.biBitCount = 24;
    hd.biSizeImage = imgsize;
    fwrite(&hd, sizeof(hd), 1, f);
    fwrite(g_rgb, imgsize, 1, f);
    fclose(f);
}

/* ---- d3d8 proxy + vtable hooks ---- */

typedef IDirect3D8 *(WINAPI *PFN_Direct3DCreate8)(UINT);

static HRESULT(STDMETHODCALLTYPE *real_CreateDevice)(
    IDirect3D8 *, UINT, D3DDEVTYPE, HWND, DWORD, D3DPRESENT_PARAMETERS *,
    IDirect3DDevice8 **);
static HRESULT(STDMETHODCALLTYPE *real_Present)(IDirect3DDevice8 *,
                                                CONST RECT *, CONST RECT *,
                                                HWND, CONST RGNDATA *);

static void hook_slot(void **slot, void *newfn, void **saved)
{
    DWORD prot;
    if (*slot == newfn)
        return; /* already hooked (device re-created) */
    VirtualProtect(slot, sizeof(void *), PAGE_EXECUTE_READWRITE, &prot);
    *saved = *slot;
    *slot = newfn;
    VirtualProtect(slot, sizeof(void *), prot, &prot);
}

static IDirect3DSurface8 *g_sysSurf;
static D3DSURFACE_DESC g_sysDesc;

static void capture_backbuffer(IDirect3DDevice8 *dev, unsigned frame)
{
    IDirect3DSurface8 *bb = NULL;
    if (FAILED(IDirect3DDevice8_GetBackBuffer(dev, 0, D3DBACKBUFFER_TYPE_MONO,
                                              &bb)) || !bb)
        return;
    D3DSURFACE_DESC desc;
    IDirect3DSurface8_GetDesc(bb, &desc);
    if (!g_sysSurf || g_sysDesc.Width != desc.Width ||
        g_sysDesc.Height != desc.Height || g_sysDesc.Format != desc.Format) {
        if (g_sysSurf)
            IDirect3DSurface8_Release(g_sysSurf);
        g_sysSurf = NULL;
        if (FAILED(IDirect3DDevice8_CreateImageSurface(
                dev, desc.Width, desc.Height, desc.Format, &g_sysSurf))) {
            IDirect3DSurface8_Release(bb);
            return;
        }
        g_sysDesc = desc;
        logf_("capture: %ux%u fmt=%d\n", desc.Width, desc.Height, desc.Format);
    }
    if (SUCCEEDED(IDirect3DDevice8_CopyRects(dev, bb, NULL, 0, g_sysSurf,
                                             NULL))) {
        D3DLOCKED_RECT lr;
        if (SUCCEEDED(IDirect3DSurface8_LockRect(g_sysSurf, &lr, NULL,
                                                 D3DLOCK_READONLY))) {
            write_bmp(frame, (const BYTE *)lr.pBits, desc.Width, desc.Height,
                      lr.Pitch, desc.Format);
            IDirect3DSurface8_UnlockRect(g_sysSurf);
        }
    }
    IDirect3DSurface8_Release(bb);
}

static HRESULT STDMETHODCALLTYPE my_Present(IDirect3DDevice8 *dev,
                                            CONST RECT *src, CONST RECT *dst,
                                            HWND wnd, CONST RGNDATA *dirty)
{
    unsigned f = (unsigned)g_frame;
    log_replay_state(f);
    if (g_stopafter && f >= g_stopafter) ExitProcess(0);
    if (g_practice)
        force_practice_unlock();
    if (g_forcestage > 0)
        *(int *)(ADDR_GAMEMANAGER + OFF_CURRENT_STAGE) = g_forcestage - 1;
    if (g_timing)
        logf_("PRESENT f=%u polls=%u creep=%u clock=%u\n", f, g_pollsThisFrame,
              g_creepMs,
              FAKE_TIME_BASE + (unsigned)((double)f * (1000.0 / 60.0)) +
                  g_creepMs);
    if (f >= g_capstart && f < g_capend && (f - g_capstart) % g_capstride == 0)
        capture_backbuffer(dev, f);
    InterlockedIncrement(&g_frame);
    if (g_realtime)
        Sleep(16); /* watchable pacing; doesn't affect the fake clock */
    return real_Present(dev, src, dst, wnd, dirty);
}

static HRESULT STDMETHODCALLTYPE my_CreateDevice(
    IDirect3D8 *d3d, UINT adapter, D3DDEVTYPE type, HWND wnd, DWORD flags,
    D3DPRESENT_PARAMETERS *pp, IDirect3DDevice8 **out)
{
    HRESULT hr = real_CreateDevice(d3d, adapter, type, wnd, flags, pp, out);
    logf_("CreateDevice hr=%08lx windowed=%d\n", (unsigned long)hr,
          pp ? pp->Windowed : -1);
    if (SUCCEEDED(hr) && out && *out)
        hook_slot((void **)&(*out)->lpVtbl->Present, (void *)my_Present,
                  (void **)&real_Present);
    return hr;
}

IDirect3D8 *WINAPI Direct3DCreate8(UINT sdk_version)
{
    char path[MAX_PATH + 16];
    GetSystemDirectoryA(path, MAX_PATH);
    strcat(path, "\\d3d8.dll");
    HMODULE real = LoadLibraryA(path);
    if (!real) {
        logf_("LoadLibrary(%s) failed\n", path);
        return NULL;
    }
    PFN_Direct3DCreate8 proc =
        (PFN_Direct3DCreate8)GetProcAddress(real, "Direct3DCreate8");
    if (!proc)
        return NULL;
    IDirect3D8 *d3d = proc(sdk_version);
    logf_("Direct3DCreate8(%u) -> %p\n", sdk_version, (void *)d3d);
    if (d3d)
        hook_slot((void **)&d3d->lpVtbl->CreateDevice, (void *)my_CreateDevice,
                  (void **)&real_CreateDevice);
    return d3d;
}

/* ---- entry ---- */

BOOL WINAPI DllMain(HINSTANCE hinst, DWORD reason, LPVOID reserved)
{
    (void)hinst; (void)reserved;
    if (reason == DLL_PROCESS_ATTACH) {
        g_mainTid = GetCurrentThreadId();
        g_timing = getenv("TH06CAP_TIMING") != NULL;
        g_log = fopen("th06cap.log", "w");
        logf_("refcap proxy attached\n");
        load_config();
        CreateDirectoryA(g_capdir, NULL);
        patch_iat("WINMM.dll", "timeGetTime", (void *)my_timeGetTime);
        patch_iat("USER32.dll", "GetKeyboardState", (void *)my_GetKeyboardState);
        patch_iat("DINPUT8.dll", "DirectInput8Create",
                  (void *)my_DirectInput8Create);
        if (g_god) {
            /* Overwrite Die()'s prologue with `ret` (0xC3). thiscall, `this` in
             * ecx, no stack args -> no cleanup needed. Player never dies/loses a
             * life, so a hold-shoot auto-run reaches later stages. */
            BYTE *die = (BYTE *)ADDR_PLAYER_DIE;
            DWORD prot;
            if (VirtualProtect(die, 1, PAGE_EXECUTE_READWRITE, &prot)) {
                *die = 0xC3;
                VirtualProtect(die, 1, prot, &prot);
                logf_("god: patched Player::Die @ %p -> ret\n", (void *)die);
            } else {
                logf_("god: VirtualProtect failed @ %p\n", (void *)die);
            }
        }
    }
    return TRUE;
}
