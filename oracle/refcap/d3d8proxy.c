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
static int g_realtime;              /* Sleep per Present so a human can watch */
static volatile LONG g_frame;       /* Present count == logic frame */
static FILE *g_log;

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
        } else if (sscanf(line, "realtime %u", &s) == 1) {
            g_realtime = (int)s;
        } else if (sscanf(line, "key %u %u %63s", &s, &e, b) == 3) {
            if (g_nkeys < (int)(sizeof(g_keys) / sizeof(g_keys[0]))) {
                g_keys[g_nkeys].start = s;
                g_keys[g_nkeys].end = e;
                g_keys[g_nkeys].vk = vk_from_name(b) & 0xff;
                g_nkeys++;
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
    if (f >= g_capstart && f < g_capend)
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
        g_log = fopen("th06cap.log", "w");
        logf_("refcap proxy attached\n");
        load_config();
        CreateDirectoryA(g_capdir, NULL);
        patch_iat("WINMM.dll", "timeGetTime", (void *)my_timeGetTime);
        patch_iat("USER32.dll", "GetKeyboardState", (void *)my_GetKeyboardState);
        patch_iat("DINPUT8.dll", "DirectInput8Create",
                  (void *)my_DirectInput8Create);
    }
    return TRUE;
}
