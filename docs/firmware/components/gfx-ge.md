# gfx-ge — mi_gfx, libmi_gfx and the mhal GE block
<!-- ev:gfx-ge -->

The 2D side of the media stack. The hardware is the **GE** (graphics engine): a blitter that fills rectangles, draws
lines, copies, scales, mirrors, rotates, colour-keys and blends between surfaces in DRAM. Its 16-bit registers lie in RIU
bank 0x1409; commands go through the engine's command FIFO and, optionally, a virtual command queue (VCMDQ) in DRAM;
completion is signalled by tags written through the queue and a level interrupt. Three pieces of vendor software drive it:

- `mhal.ko` block **GE** — SigmaStar/MStar Utopia "graphic" compiled for M6: the GFX API (`apiGFX.c`, `mapiGFX.c`), the
  GE driver (`drvGE.c`, `osalGE.c`) and the GE HAL (the Infinity2 `hal/i2/ge` code at the M6 register base), plus
  binary-only glue (module set-up, device tree, clock, interrupt API, sysfs).
- `mi_gfx.ko` — the MI GFX module (`/dev/mi_gfx`, 14 ioctls, 14 kernel exports) over 33 functions of that block.
- `libmi_gfx.so` — the MI GFX user library (17 exports, `MI_GFX_*`).

Other documents: the user-side ioctl wrapper, payload packing, fd handling, return codes, error-code format and module
ids — `mi-user-abi.md` (§2 "The device fd", §3 "The ioctl call", §4 "Return values and error codes", §6 "Debug level");
the kernel-side device frame (`MI_DEVICE_Register`, ioctl dispatch, `/proc/mi_modules/<module>/*` debug files, mi_sys
exports, MMA heaps) — `mi_sys-2-interfaces.md` (§1 "mi_common.ko — the device frame of every MI module", §6 "Exports to
other kernel modules", §7.2 "MMA heaps and the chunk manager", §10 "procfs"); the mi_sys registration contract and the
0x298-byte info record — `mi_sys-1-engine.md` §2 "Registration contract"; mhal's block map, `camdriver_init` order, the
device-node pattern of `mge`, device-tree properties per node — `mhal-core.md` (§2.1 "Block table", §3.2
"`camdriver_init` order", §4 "Character devices", §7 "Device-tree properties mhal reads, per node"); RIU addressing,
the clock tree (`CLK_ge`), the device-tree node `ge0`, the MIU client table — `kernel-platform.md` (§3.1 "Static I/O map
and RIU addressing", §5.1 "The DT clock tree", §12.2 "Client → group table", §14 "Device tree"); the `mmap.ini` grammar
and items — `config_tool.md` §2.1, §2.5; mi_disp's rotation path that calls this module — `mi_disp-mi_hdmi.md` §4.11.2; the file rows —
`firmware-image.md` §3.3 "SigmaStar media stack", §3.5 "`/config/lib`".

Tags: **[R]** read from the binary, a decompile, disassembly or a file; **[S]** stated by a source; **[I]** inference. A
heading tagged [R] means every statement under it is [R] unless marked otherwise. Struct and field names the binaries do
not carry are SigmaStar/Utopia-style names and count as [I]; offsets, sizes, constants and control flow are [R]. "Same as
I2" means the body is the I2 source's: only the register base, line numbers in prints and inlining differ.

Binaries (dump of 2026-10-16) [R]: `/config/lib/libmi_gfx.so` (9.5 KB, stripped, 17 exports, no SONAME);
`/config/modules/4.9.227/mi_gfx.ko` (39.6 KB file, 12 KB code, 66 functions, full local symbols, no debug info);
`…/modules/4.9.227/mhal.ko` block GE (`.text` 0x02d340–0x036a78). Build string of both MI files: `project_commit.7251436
sdk_commit.80327e1`, build time `20220721223318` in the library and `20220721223315` in the module.

The same `mi_gfx.ko` / `libmi_gfx.so` occur in the firmware of other Mercury6 products (Anjoy MC-H12S, SIYI A8 mini,
RunCam Split-H). Reference source: Utopia `graphic/` of the Infinity2 (I2) SDK, published as `jockyw2001/sdk` on GitHub,
`src/mhal/i2/utpa/modules/graphic/` (`api/gfx/apiGFX.c`, `api/gfx/mapiGFX.c`, `drv/ge/drvGE.c`, `drv/ge/drvGE.h`,
`drv/ge/osalGE.c`, `hal/{i2,k6,kastor}/ge/halGE.c`, `hal/i2/ge/halGE.h`, `regGE.h`). That tree lacks `apiGFX.h`,
`apiGFX_v2.h`, `utopia.h` and `halCHIP.h`; no public `mi_gfx*.h` header and no public M6 GE HAL source were found.

## 1. The component on this camera [R]
<!-- ev:gfx-ge.1 -->

- **Unused.** No program on the camera links `libmi_gfx.so`: it lies in `/config/lib`, but no file of the dump contains
  the string `libmi_gfx` (`grep -rl` over; the library has no SONAME a dependency could name). Nobody
  opens `/dev/mi_gfx`. Nothing opens the mhal node `/dev/mge` or writes its sysfs file `geclk`: no file of the dump but
  `mhal.ko` contains either string.
- **The one kernel consumer is mi_disp.** Of all kernel modules only `mi_disp.ko` imports anything of the stack —
  `MI_GFX_BitBlit` and `MI_GFX_WaitAllDone` [R `nm -u`]; its modinfo `depends=` names mi_gfx. mi_disp calls them from its
  video-layer **rotation path** (`_MI_DISP_RotateByGfx`, rotating into MMA buffers named `"disp_rotate"`), taken only when
  a video layer has a rotate mode; the app never calls `SetVideoLayerRotateMode` (disp ioctl nr 0x2b), so the path never
  runs. No other module imports `MI_GFX_*` or `MApi_GFX*`; fbdev drives its GOP through mhal RGN and DISP, not through the
  GE (`rgn-gop.md` §5).
- **The rotation path works only while `/dev/mi_gfx` is held open.** mi_disp never calls `MI_GFX_Open`; `MI_GFX_BitBlit`
  and `MI_GFX_WaitAllDone` return 0xA00D2220 ("not opened") unless some process holds `/dev/mi_gfx` open or another module
  called the `MI_GFX_Open` export (§3.3, §3.7).
- No register state of the GE under the vendor stack has been read on the camera; the meanings left open below (reg
  0x1e bits 5–7 and regs 0x24 / 0x25, §4.7; bit 0 of 0x1F001720, §4.8; the destination colour-key polarity, §4.10) rest on
  static code only.

## 2. libmi_gfx.so — the user library [R]
<!-- ev:gfx-ge.2 -->

### 2.1 Frame
- `NEEDED` is only `libc.so.6`. The library also imports `g_mi_fd_table`, `g_last_fd_index`, `g_mi_fd_semalock`,
  `CamOsTsemDown`, `CamOsTsemUp` and `_MI_PRINT_GetDebugLevel`, which only libmi_sys.so (and libcam_os_wrapper through
  it) define, so it works only in a process that has already loaded libmi_sys [I: with no SONAME dependency, a plain
  `-lmi_gfx` link without `-lmi_sys` fails at load].
- fd helper `_MI_WRAPPER_DEVICE_Open` (0x10b4c, source file `gfx_api.c`) is the common helper of `mi-user-abi.md` §2.1
  unchanged: `/dev/mi_gfx`, `O_RDWR`, re-open after `fork`, `F_DUPFD` above the top table slot,
  `assert(g_last_fd_index <= 0x22)` (`gfx_api.c` line 0x18), `FD_CLOEXEC`. The string `/dev/mi/gfx` exists and is used
  only in the close messages.
- The ioctl argument is the 16-byte wrapper `{u32 u32Len; u32 pad; u64 u64Ptr}` (`mi-user-abi.md` §3.1).
- **The first argument of every export is a device id and is ignored** (never read, never sent) [R: the first argument unused in
  all 17] [I: `MI_GFX_DEV GfxDevId`, the newer SigmaStar API signature].
- An ioctl returning −1 (errno set) makes the export print `failed to ioctl 0x%08lx!(%s)` and return **−1** (not an MI
  code); otherwise the export returns the ioctl's return value (the MI code of the kernel handler).
- Argument errors return **0xA00D2003** (module id 0xD) [R constant; I: a `MI_ERR_GFX_NULL_PTR` / `ILLEGAL_PARAM`-style
  name]; the message is printed only when `/proc/mi_modules/mi_gfx/debug_level` ≥ 1 (`mi-user-abi.md` §6).
- Open and close:
  - `MI_GFX_Open(dev)` and `MI_GFX_CreateDev(dev)` are the same code: call the fd helper; return 0, or **0x1F** when the fd
    is negative. **Neither sends ioctl 0 (`Open`)** — the kernel opens the GE from `open()` of the node itself (§3.3).
  - `MI_GFX_Close(dev)` and `MI_GFX_DestroyDev(dev)` are the same code: the shared close helper of `mi-user-abi.md` §2.2
    (close the cached fd if this pid opened it, release the top `g_mi_fd_table` slot, fd = −1); **no counter** — the first
    Close closes the fd for every user in the process. Return 0, −1 if the fd is invalid or not this pid's, or the negative
    `close()` result. Neither sends ioctl 1.
- Data export `sstar_mi_so_version_string` = `Sigmastar Module mi_gfx version: project_commit.7251436
  sdk_commit.80327e1 build_time.20220721223318`.

### 2.2 Exports → ioctls
Payload offsets are bytes into the buffer the wrapper points at; the structs `Surface` (0x18), `Rect` (0x10), `Line`
(0x14), `Opt` (0x54) and `Palette` (0x404) are laid out in §3.4. The library reads a surface's width and height at
+0xc / +0x10 to build a default rectangle.

| export (after `dev`) | ioctl (nr) | payload | library logic |
|---|---|---|---|
| `WaitAllDone(u8 bWaitAll, u16 u16Fence)` | 0x40046903 `_IOW` (3) | +0 u8 bWaitAll, +2 u16 fence (4 B) | none |
| `QuickFill(Surface *dst, Rect *rect, u32 colour, u16 *pFence)` | 0xC0306904 `_IOWR` (4) | +0 Surface; +0x18 Rect; +0x28 u32 colour; +0x2c u16 fence (out) | `dst == NULL` → 0xA00D2003; `rect == NULL` → `{0, 0, dst.w, dst.h}`; `*pFence` written only if ret == 0 and `pFence != NULL` |
| `DrawLine(Surface *dst, Line *line, u16 *pFence)` | 0xC0306905 `_IOWR` (5) | +0 Surface; +0x18 Line (0x14); +0x2c u16 fence (out) | either pointer NULL → 0xA00D2003; fence as QuickFill |
| `GetARGB8888To1555AlphaThreshold(u8 *p)` | 0x80016906 `_IOR` (6) | +0 u8 | `p == NULL` → 0xA00D2003; a local preset to 1 (irrelevant: the kernel fills its own buffer); `*p` only if ret == 0 |
| `SetARGB8888To1555AlphaThreshold(u8 v)` | 0x40016907 `_IOW` (7) | +0 u8 | none |
| `BitBlit(Surface *src, Rect *srcRect, Surface *dst, Rect *dstRect, Opt *opt, u16 *pFence)` | 0xC0A86908 `_IOWR` (8) | memset 0, then +0 src Surface; +0x18 src Rect; +0x28 dst Surface; +0x40 dst Rect; +0x50 Opt (0x54); +0xa4 u16 fence (out) | `src` or `dst` NULL → 0xA00D2003; a NULL rect → `{0, 0, surface.w, surface.h}`; `opt == NULL` → all-zero Opt (which the kernel rejects, §3.5); fence as QuickFill |
| `SetPalette(u32 eFmt, Palette *pal)` | 0x44086909 `_IOW` (9) | +0 Palette (0x404 copied); +0x404 u32 eFmt (0x408 B) | 0xA00D2003 if `pal == NULL`, `eFmt >= 4`, `pal.end` (u16 +0x402) > 0xff, or `pal.start` (u16 +0x400) > `pal.end` |
| `GetARGB1555To8888AlphaValue(u8 *p)` | 0x8001690A `_IOR` (0xa) | +0 u8 | as nr 6 |
| `SetARGB1555To8888AlphaValue(u8 v)` | 0x4001690B `_IOW` (0xb) | +0 u8 | none |
| `GetInitialScalingCoeff(u16 *p)` | 0x8002690C `_IOR` (0xc) | +0 u16 | as nr 6 |
| `SetInitialScalingCoeff(u16 v)` | 0x4002690D `_IOW` (0xd) | +0 u16 | `v` must be 1…999, else 0xA00D2003 |
| `mi_gfx_PendingDone(u16 fence)` | 0x40026902 `_IOW` (2) | +0 u16 | none (lower-case name: an internal export) |
| `Open`, `CreateDev`, `Close`, `DestroyDev` | — | — | §2.1 |

ioctls 0 (`Open`) and 1 (`Close`) have no user-space caller.

## 3. mi_gfx.ko — the kernel module [R: decompile + `objdump -dr`; `.text` offsets]
<!-- ev:gfx-ge.3 -->

### 3.1 Load, device, state
- `init_module` (`.init.text`) creates the common procfs set under `/proc/mi_modules/mi_gfx/` (`debug_level`,
  `debug_file`, `debug_func`, `module_version_file` = `Sigmastar Module mi_gfx version: project_commit.7251436
  sdk_commit.80327e1 build_time.20220721223315`; frame in `mi_sys-2-interfaces.md` §10.1), prints `module [gfx] init`,
  then `MI_DEVICE_Register(&dev)` → `/dev/mi_gfx`. `cleanup_module` removes the five proc entries and unregisters.
  modinfo: `depends=mi_common`, parameters `debug_level` (uint), `debug_file`, `debug_func` (string) — diagnostics only.
  No GE work happens at load.
- Device struct `.data+0x60`: ioctl table `.data+0x24` (14 entries, §3.4), `process_init` = `mi_gfx_process_init`,
  `process_exit` = `mi_gfx_process_exit`. Module mutex = `.data+0x0` (`struct mutex`, statically initialised).
- State at `.data+0x98` (0x10 bytes; initial values from `.data`; names [I]):

| offset | field | initial |
|---|---|---|
| +0 | u8 `bInited` | 0 |
| +1 | u8 `u8Argb8888To1555Thr` | 0 |
| +2 | u8 `u8Argb1555To8888Alpha` | 0 |
| +4 | u16 `u16ScalingCoeff` | **500** |
| +6 | u8 `u8InitCnt` | 0 |
| +8 | u64 `vqPhys` (MMA buffer of the command queue; 0 if none) | 0 |

- `.bss+0x210`: +0 u32 GE format of the last QuickFill / DrawLine destination; +0xc `g_ge_isr_wait_queue` (wait-queue
  head); +0x24 mi_sys device handle; +0x28 u8 `bSupportIrq`; +0x2c u32 `irq`. Two more `.bss` words record the GE source
  and destination format of the last BitBlit. The three format words serve only `getcap` (§3.8). Names [I].
- `.data+0xa8 gfx_isr_waitqueue` is a second, statically initialised wait-queue head that **no code uses** (its only
  relocations are its own list pointers) — unused.

### 3.2 Colour formats and result codes
**Format table** MI colour format → GE value (Utopia `GFX_Buffer_Format`), `.rodata+0x1f4`, 22 words. The index order
is the order of `E_GE_FMT_*` in `drvGE.h` (names [R source], numeric GE values [R table]):

| MI | name | GE | MI | name | GE | MI | name | GE |
|---|---|---|---|---|---|---|---|---|
| 0 | I1 | 0x00 | 8 | ARGB4444 | 0x0a | 16 | ABGR1555 | 0x13 |
| 1 | I2 | 0x01 | 9 | ARGB1555_DST | 0x0c | 17 | ABGR4444 | 0x14 |
| 2 | I4 | 0x02 | 10 | YUV422 | 0x0e | 18 | BGRA4444 | 0x15 |
| 3 | I8 | 0x04 | 11 | ARGB8888 | 0x0f | 19 | BGR565 | 0x16 |
| 4 | FaBaFgBg2266 | 0x06 | 12 | RGBA5551 | 0x10 | 20 | RGBA8888 | 0x1d |
| 5 | 1ABFgBg12355 | 0x07 | 13 | RGBA4444 | 0x11 | 21 | BGRA8888 | 0x1e |
| 6 | RGB565 | 0x08 | 14 | ABGR8888 | 0x1f | ≥22 | (any) | 0x0f (ARGB8888) |
| 7 | ARGB1555 | 0x09 | 15 | BGRA5551 | 0x12 | | | |

Every **destination** buffer (QuickFill, DrawLine, BitBlit dst) maps MI 7 (ARGB1555) to GE **0x0c (ARGB1555_DST)**
instead of 0x09; sources and colour keys use the table as is.

`_MI_GFX_IMPL_GetFmtBpp(mi_fmt)` (inlined in the stretch patch): 0–3 → 1; 4–10, 12, 13, 15–19 → 2; 11, 14, 20, 21 → 4;
other → 0xffff. [I: the index formats count as 1 byte per pixel here, so I1 / I2 / I4 strides are over-estimated.]

**GE result → MI code**, table `.rodata+0x24c` (the GE API returns `GFX_Result`; 1 = success). Result names follow the
`E_GE_*` order of `drvGE.h` [R order; I names]:

| GFX_Result | MI code | GFX_Result | MI code |
|---|---|---|---|
| 2 NON_ALIGN_ADDRESS | 0xA00D2223 | 7 DRV_FAIL_BLTADDR | 0xA00D2229 |
| 3 NON_ALIGN_PITCH | 0xA00D2224 | 8 DRV_FAIL_OVERLAP | 0xA00D2225 |
| 4 INVALID_PARAMETERS | 0xA00D2003 | 9 DRV_FAIL_STRETCH | 0xA00D2226 |
| 5 DRV_NOT_SUPPORT | 0xA00D2221 | 10 DRV_FAIL_ITALIC | 0xA00D2227 |
| 6 DRV_FAIL_FORMAT | 0xA00D2222 | 11 DRV_FAIL_LOCKED | 0xA00D2228 |

0 (`GFX_FAIL`) and any value > 11 → 0xA00D2003. The module has a private code range 0x220–0x229 (not in
`mi_common_datatype`): **0xA00D2220 = not opened** (`bInited == 0`) and the nine GE driver failures above; 0xA00D2221 is
also returned directly by SetPalette for a non-I8 format. 0xA00D2003 = bad argument everywhere.

### 3.3 Open and Close — the GE comes up on the first open of `/dev/mi_gfx` [R: `MI_GFX_IMPL_Open` 0x134c, `MI_GFX_IMPL_Close` 0x16ac]
Callers of Open: `mi_gfx_process_init` (every `open()` of the node, under mi_common's device mutex; then prints
`client [%d] connected, module:gfx`), ioctl 0 and the kernel export `MI_GFX_Open`. Callers of Close: `mi_gfx_process_exit`
(every `release()`), ioctl 1 and the export `MI_GFX_Close`. So `u8InitCnt` counts open file descriptors plus explicit Open
calls.

`MI_GFX_IMPL_Open()` (always returns 0):
1. `u8InitCnt++`; if `bInited` → return.
2. `__init_waitqueue_head(&g_ge_isr_wait_queue)`; `vqPhys = 0`.
3. `MI_SYSCFG_GetMmapInfo("E_MMAP_ID_GE_VQ", &p)` (an `mmap.ini` item; this camera's `mmap.ini` has none —
   `config_tool.md` §2.5, §2.6):
   - found: `cfg.u8Miu = p->u8MiuNo` (+2), `cfg.u32VCmdQSize = p->u32Size` (+8), `cfg.vcmdqAddr = p->u32Addr` (+4; high
     word 0);
   - not found: `mi_sys_MMA_Alloc(NULL, "gfx_vq", 0x20000, &vqPhys)`; success → `cfg.u8Miu = 0`, size 0x20000, address
     `vqPhys`; failure → `printk("Get GE VQ buffer from mma fail.\n")` and size 0 (no queue).
4. `MApi_GFX_Init(&cfg)` with the packed 16-byte `GFX_Config` (§4.4):
   `{+0 u8 u8Miu; +1 u8 u8Dither = 0; +2 u32 u32VCmdQSize; +6 u64 u64VCmdQAddr; +0xe u8 bIsHK = 1; +0xf u8 bIsCompt = 1}`.
   `u8Miu` (and with it the MIU number of `E_MMAP_ID_GE_VQ`) never reaches the GE driver; the queue's MIU comes only from
   the address (§4.8).
5. VCMDQ: size < 0x1000 → `MApi_GFX_EnableVCmdQueue(0)`. Else size code = 7 (≥ 512 KiB), 6 (≥ 256 K), 5 (≥ 128 K),
   4 (≥ 64 K), 3 (≥ 32 K), 2 (≥ 16 K), 1 (≥ 8 K), 0 (≥ 4 K) → `MApi_GFX_SetVCmdBuffer(u64 addr, code)`; result 1 →
   `MApi_GFX_EnableVCmdQueue(1)`, else `MApi_GFX_EnableVCmdQueue(0)`. The MMA case (128 KiB) is code 5. What these
   calls leave in the registers: §4.5.
6. `MApi_GFX_EnableAlphaBlending(0)`, `MApi_GFX_SetDFBBldOP(0xb, 6)`, `MApi_GFX_EnableDFBBlending(1)`.
7. `bInited = 1`, `u8Argb8888To1555Thr = 1` (one halfword store of 0x0101; no GE call sets the threshold here).
8. `MI_Moduledev_RegisterDev()` (§3.8).
9. `bSupportIrq = MApi_GE_SupportInterrupt()` (always 1, §4.2); if set, `MAPI_GE_GetIrqNum(&irq)`; if `irq != 0`:
   `MApi_GE_EnableInterrupt()`, `CamOsIrqRequest(irq, mi_GE_ISR, "mi_GE_ISR", &state)`; a failure (−1) is only printed.

`mi_GE_ISR`: `MApi_GE_ClearInterrupt()`, `__wake_up(&g_ge_isr_wait_queue, TASK_NORMAL, 0, NULL)`, return 1
(`IRQ_HANDLED`).

`MI_GFX_IMPL_Close()`: `u8InitCnt--` **before** the inited check (so a Close without Open wraps the u8 counter); not
inited → 0xA00D2220. When the counter reaches 0: if `bSupportIrq && irq` → `MApi_GE_DisableInterrupt(0)`,
`CamOsIrqFree(irq, &state)`; `MApi_GFX_EnableVCmdQueue(0)`; `MApi_GE_Exit()`; if `vqPhys != 0` →
`mi_sys_MMA_Free(vqPhys)`; `mi_sys_UnRegisterDev(handle)` (inlined `MI_Moduledev_UnRegisterDev`, name from its log
string); `bInited = 0`. Returns 0.

### 3.4 ioctl table and payload structs [R]
Dispatch is the common one (`'i'`, nr < 14, 16-byte wrapper, copy-back on `_IOC_READ` whatever the result;
`mi_sys-2-interfaces.md` §1.3). Every handler except SetPalette takes the module mutex and stores it in `ctx+8`
(mi_common unlocks); the handler returns the IMPL return value (r0 passes through).

| nr | handler | payload (size) | IMPL call | notes |
|---|---|---|---|---|
| 0 | `MI_GFX_IOCTL_Open` | none (`_IO`) | `MI_GFX_IMPL_Open()` | §3.3 |
| 1 | `MI_GFX_IOCTL_Close` | none | `MI_GFX_IMPL_Close()` | §3.3 |
| 2 | `MI_GFX_IOCTL_PendingDone` | u16 fence (2) | `MI_GFX_IMPL_PendingDone(fence)` | inited → 0, else 0xA00D2220; **does nothing else** (fence ignored) |
| 3 | `MI_GFX_IOCTL_WaitAllDone` | u8 bWaitAll, +2 u16 fence (4) | `MI_GFX_IMPL_WaitAllDone(b, fence)` | §3.6 |
| 4 | `MI_GFX_IOCTL_QuickFill` | 0x30 | `IMPL_QuickFill(&p[0], &p[0x18], p.colour, &f)` | `p+0x2c = f` only if ret == 0 |
| 5 | `MI_GFX_IOCTL_DrawLine` | 0x30 | `IMPL_DrawLine(&p[0], &p[0x18], &f)` | same |
| 6 | `…GetARGB8888To1555AlphaThreshold` | u8 (1) | `IMPL_Get…(&v)` (local preset 1) | `*p = v` only if ret == 0 |
| 7 | `…SetARGB8888To1555AlphaThreshold` | u8 | `IMPL_Set…(v)` | |
| 8 | `MI_GFX_IOCTL_BitBlit` | 0xA8 | `IMPL_BitBlit(&p[0], &p[0x18], &p[0x28], &p[0x40], &p[0x50], &f)` | `p+0xa4 = f` only if ret == 0 |
| 9 | `MI_GFX_IOCTL_SetPalette` | 0x408 | copies 0x404 to the stack, `IMPL_SetPalette(p.eFmt, pal, pal.start, pal.end)` | **no mutex** |
| 0xa | `…GetARGB1555To8888AlphaValue` | u8 | as 6 (preset 1) | |
| 0xb | `…SetARGB1555To8888AlphaValue` | u8 | as 7 | |
| 0xc | `…GetScalingCoefficient` | u16 (2) | as 6 (preset **500**) | |
| 0xd | `…SetScalingCoefficient` | u16 | as 7 | |

Payload structs (layout [R] from the readers; names [I], SigmaStar `MI_GFX_*` style):
- `Surface` (0x18): +0 u64 `phyAddr`, +8 u32 `eColorFmt` (MI, §3.2), +0xc u32 `u32Width`, +0x10 u32 `u32Height`,
  +0x14 u32 `u32Stride` (bytes).
- `Rect` (0x10): +0 s32 `s32Xpos`, +4 s32 `s32Ypos`, +8 u32 `u32Width`, +0xc u32 `u32Height`.
- `Line` (0x14): +0 s16 `fromX`, +2 s16 `fromY`, +4 s16 `toX`, +6 s16 `toY`, +8 u16 `u16Width`, +0xa u8 `bColorGradient`
  (+0xb pad), +0xc u32 `colourFrom`, +0x10 u32 `colourTo`.
- `Palette` (0x404): +0 u32 `argb[256]`, +0x400 u16 `start`, +0x402 u16 `end`; the ioctl adds +0x404 u32 `eFmt`.
- `Opt` (0x54):

| off | field | used as |
|---|---|---|
| 0x00 | `Rect stClipRect` | clip; must have w, h ≠ 0 (§3.5) |
| 0x10 | u8 `bSrcColorKey` (+3 pad) | 1 = source colour key on |
| 0x14 | u32 `eSrcCKeyOp` | 0…5 (else 0 and a warning) |
| 0x18 | u32 `eSrcCKeyFmt` | MI format → GE by the table |
| 0x1c / 0x20 | u32 `srcKeyFrom` / `srcKeyTo` | key range |
| 0x24…0x34 | destination colour key, the same five fields | op also clamped to 0…5 |
| 0x38 | u32 `eSrcDfbBldOp` | 1…10 kept, anything else → 0 |
| 0x3c | u32 `eDstDfbBldOp` | same |
| 0x40 | u32 `eMirror` | 0 none, 1 horizontal, 2 vertical, 3 both |
| 0x44 | u32 `eRotate` | 0…3 = 0/90/180/270 [I]; > 3 only warned, passed on |
| 0x48 | u32 `u32DfbBldFlags` | bit mask, §3.7 step 4 |
| 0x4c | u32 `u32GlobalSrcConstColor` | DFB constant colour |
| 0x50 | u32 `u32GlobalDstConstColor` [I name] | **never read** by the module |

### 3.5 Argument checks (all fail with 0xA00D2003) [R]
- **Surface valid** (`_MI_GFX_CheckSurface`): `1 ≤ w ≤ 0x1fff` and `1 ≤ h ≤ 0x1fff`. No check of address, stride or
  format.
- **Rect in surface** (`_MI_GFX_CheckRect(s, r)`): surface valid; `r.w ≠ 0`, `r.h ≠ 0`; `0 ≤ r.x < s.w`,
  `r.x + r.w ≤ s.w`, `0 ≤ r.y < s.h`, `r.y + r.h ≤ s.h`.
- **Clip** (`_MI_GFX_CheckClipRect(dst, dstRect, clip)`): `dstRect.w/h ≠ 0`, `clip.w/h ≠ 0`; the intersection
  `x0 = max(dstRect.x, clip.x)`, `x1 = min(dstRect.x+w−1, clip.x+w−1)` (the same for y) must be non-empty (signed
  compare); dst surface valid; the intersection must lie inside the dst surface (the rules of CheckRect). The dst rect
  itself is **not** required to lie inside the surface — only its clipped part.
- **Line** (`_MI_GFX_CheckLine(s, l)`): surface valid; from == to is only logged (allowed); `0 ≤ fromX < s.w`,
  `0 ≤ fromY < s.h`, `0 ≤ toX ≤ s.w`, `0 ≤ toY ≤ s.h` (the end point may lie one past the edge).
- A library `BitBlit` with `opt == NULL` sends an all-zero Opt, whose zero clip fails the clip check → **0xA00D2003**:
  every BitBlit needs an Opt with a clip rectangle. The "zero clip = whole surface" branch inside `_MI_GFX_IMPL_BitBlit`
  (§3.7 step 8) is therefore unreachable.

### 3.6 Operations [R]
Common pattern: check `bInited` (0xA00D2220), NULL pointers and geometry (0xA00D2003), program the GE state, issue the
draw between `MApi_GFX_BeginDraw()` / `MApi_GFX_EndDraw()`, and on success return 0 with
`*pFence = MApi_GFX_SetNextTAGID()` (taken **after** the draw command, before EndDraw); on failure map the GFX result
(§3.2). Clip and buffer structs passed to mhal [R offsets; I names after Utopia]: `GFX_Point {u16 x; u16 y}`,
`GFX_BufferInfo {+0 u64 addr; +8 u32 width; +0xc u32 height; +0x10 u32 pitch; +0x14 u32 fmt}`,
`GFX_Block {u16 x, y, w, h}`.

**QuickFill(s, r, colour, &fence)** (`MI_GFX_IMPL_QuickFill` 0x18fc): CheckRect(s, r) (a NULL fence pointer also gives
0xA00D2003) → `EnableAlphaBlending(1)`, `SetDFBBldOP(1, 0)`, `SetDFBBldFlags(0)`, `EnableDFBBlending(1)`,
`SetSrcColorKey(0, 4, 0xf, &0, &0)`, `SetDstColorKey(0, 4, 0xf, &0, &0)`, `BeginDraw`, `SetClip({0,0}, {s.w, s.h})`,
`SetDstBufferInfo(&{s.addr, s.w, s.h, s.stride, dstfmt(s.fmt)}, 0)`, `RectFill(&info)` with
`info = {+0 u16 x, +2 y, +4 w, +6 h = r; +8 (not written); +0xc u32 fmt; +0x10 u32 colourStart = colour;
+0x14 u32 colourEnd = colour; +0x18 u32 flag = 0}`. The colour is passed in the destination's own pixel-format value
(no conversion) [I: the GE expands it per format, §4.14 `GE_ColorExpand`].

**DrawLine(s, l, &fence)** (`MI_GFX_IMPL_DrawLine` 0x2938): CheckLine → the same blend / colour-key reset as QuickFill,
`BeginDraw`, `SetClip({0,0}, {s.w, s.h})`, `SetDstBufferInfo`, `DrawLine(&info)` with `info = {+0 u16 x1, +2 y1,
+4 x2, +6 y2; +8 u32 fmt; +0xc u32 colourFrom; +0x10 u32 colourTo; +0x14 u32 width = l.u16Width; +0x18 u32 flag =
(l.bColorGradient ≠ 0)}`.

**SetPalette(eFmt, pal, start, end)** (0x284c): inited; `eFmt ≠ 3` (I8) → **0xA00D2221**; `BeginDraw`,
`MApi_GFX_SetPaletteOpt(&pal.argb[start], start, end)`, `EndDraw`. (The library allows eFmt 0…3; the kernel only 3.)

**Alpha threshold, alpha value, scaling coefficient.** Get returns the stored copy (inited required; NULL →
0xA00D2003). Set calls the GE once and stores the value only if the GE returned 1:
`SetARGB8888To1555AlphaThreshold(v)` → `MApi_GFX_SetAlphaThr_ARGB1555(v)`; `SetARGB1555To8888AlphaValue(v)` →
`MApi_GFX_SetAlpha_ARGB1555(v)`; `SetScalingCoefficient(v)` → `MApi_GFX_SetScallingCoeff(v)` (vendor spelling; the
library limits v to 1…999, the kernel handler does not check). A GE failure → 0xA00D2003. The stored defaults (threshold
1, alpha 0, coefficient 500) are **not** written to the GE at Open — they are only what Get reports until a Set.

**WaitAllDone(bWaitAll, fence)** (0x17fc): inited; `bWaitAll == 1` → `fence = MApi_GFX_SetNextTAGID()` (a new tag
after everything queued); then
- no IRQ (`bSupportIrq == 0`): `MApi_GFX_WaitForTAGID(fence)`, return 0 — on M6 that call does not wait (§4.7);
- IRQ (always on M6): if `MApi_GE_TagIsFinish(fence)` → 0; else a `prepare_to_wait_event(&g_ge_isr_wait_queue,
  TASK_UNINTERRUPTIBLE)` loop with `schedule_timeout`, **20 jiffies in total** (HZ 100 → 200 ms), left when the tag is
  finished or the time is up — **it returns 0 in both cases** (a timeout is not reported).

**PendingDone(fence)**: no-op apart from the inited check (§3.4).

### 3.7 BitBlit and the stretch patch [R: `MI_GFX_IMPL_BitBlit` 0x25e0, `_MI_GFX_IMPL_BitBlit` 0x1d18, `_Patch_00_A` 0x2410]
`MI_GFX_IMPL_BitBlit(src, srcRect, dst, dstRect, opt, &fence)`:
1. `src` / `srcRect` NULL → 0xA00D2003; CheckRect(src, srcRect) (inline copy); CheckClipRect(dst, dstRect, &opt->clip)
   (catches `dst`, `dstRect`, `opt` NULL too).
2. `_Patch_00_A(src, srcRect, dst, dstRect, opt, &fence, &tmpPhys)` — may replace `src` / `srcRect` by a pre-shrunk copy.
3. Patch returned 0 → `_MI_GFX_IMPL_BitBlit(…)` with the (possibly replaced) source; if a temporary buffer exists,
   `MI_GFX_IMPL_WaitAllDone(0, fence)` and `mi_sys_MMA_Free(tmpPhys)`. Patch returned non-zero → free the temporary
   buffer if any and return the patch's result.

This outer function has **no inited check**; `_MI_GFX_IMPL_BitBlit` (and the patch's own blit) checks `bInited` first →
0xA00D2220 without an Open. Without Open only the patch's MMA allocation and free can happen.

**Stretch patch "00_A"** — taken when the source is shrunk horizontally by more than 4× and at most 5×:
`srcRect.w > dstRect.w && srcRect.w > 4·dstRect.w && srcRect.w ≤ 5·dstRect.w`, and `srcRect.w − 5 ≥ 5`:
- `tmpW = (srcRect.w − 5) / 5`, `tmpH = srcRect.h`, `bpp = GetFmtBpp(src.fmt)`; temporary surface
  `{phys from mi_sys_MMA_Alloc(NULL, "MI_GFX_BitBlit_Patch_00", bpp·tmpW·tmpH), src.fmt, tmpW, tmpH, stride bpp·tmpW}`.
  Allocation failure → return −1 (`0xFFFFFFFF`, not an MI code).
- Blit `src/srcRect` → `tmp/{0,0,tmpW,tmpH}` with an internal Opt: clip `{0,0,tmpW,tmpH}`, no colour keys,
  `eSrcDfbBldOp = 1`, `eDstDfbBldOp = 0` (plain copy), mirror / rotate 0, flags 0, both constant colours 0xff000000; then
  `WaitAllDone(0, tmpFence)`.
- Replace the caller's `src` by the temporary surface and `srcRect` by `{0,0,tmpW,tmpH}`; return the temporary blit's
  result.
- The second (user) blit then scales `tmpW → dstRect.w`, a slight up-scale. [I: a work-around for a GE limit or defect on
  horizontal down-scaling between 4× and 5×; vertical ratios and ratios > 5× are not patched.]
- `_Patch_00_B(&tmpPhys, fence, bWait)` (0x25ac: optional `WaitAllDone(0)`, then `mi_sys_MMA_Free`) has **no caller** —
  unused.

**`_MI_GFX_IMPL_BitBlit(src, srcRect, dst, dstRect, opt, &fence)`** — the GE programming, in order:
1. `bInited` (0xA00D2220); all six pointers non-NULL (0xA00D2003). `BeginDraw`.
2. Blend ops: `s = opt.eSrcDfbBldOp`, `d = opt.eDstDfbBldOp`, each replaced by 0 unless 1…10.
3. Source colour key: `opt.bSrcColorKey == 1` → `SetSrcColorKey(1, op ≤ 5 ? op : 0, GEfmt(opt.eSrcCKeyFmt), &from,
   &to)`; else `SetSrcColorKey(0, 4, 0xf, &0, &0)`. Destination colour key: the same with the dst fields and
   `SetDstColorKey`. The six ops: §4.10.
4. Unless **both raw ops equal 0xb** (left as they are — the GE keeps the blend state of the previous operation):
   `EnableAlphaBlending(1)`, `SetDFBBldOP(s, d)`, `EnableDFBBlending(1)`, `SetDFBBldConstColor(opt.srcConst)` and
   `SetDFBBldFlags(f)` with `f = (flags & 0x3) | (flags & 0x4 if srcConst ≠ 0) | (flags & 0x8) |
   (flags & 0x10 if srcConst ≠ 0) | (flags & 0xe0)` (bits 2 and 4 dropped when the constant colour is 0; bits > 7
   dropped).
5. `SetRotate(opt.eRotate)`.
6. Mirror: 1 → `srcRect.x += srcRect.w − 1`, `SetMirror(1, 0)`; 2 → `srcRect.y += srcRect.h − 1`, `SetMirror(0, 1)`;
   3 → both, `SetMirror(1, 1)`; else `SetMirror(0, 0)`. (The adjusted rect is written into the payload buffer; neither
   the library nor the kernel export copies more than the fence back, so callers never see it.)
7. `SetSrcBufferInfo(&{src.addr, src.w, src.h, src.stride, GEfmt(src.fmt)}, 0)`;
   `SetDstBufferInfo(&{dst…, dstfmt(dst.fmt)}, 0)` (ARGB1555 → 0x0c).
8. Clip: `SetClip({clip.x, clip.y}, {clip.x + clip.w − 1, clip.y + clip.h − 1})` (u16 each). The all-zero-clip branch
   (`{0,0}`–`{max(src.w,dst.w)−1, max(src.h,dst.h)−1}`) is unreachable (§3.5).
9. `MApi_GFX_BitBlt(&{srcBlock = {srcRect.x, .y, .w, .h}, dstBlock = {dstRect…}} (8×u16), drawFlag)` with
   `drawFlag = 1` when `srcRect.w ≠ dstRect.w || srcRect.h ≠ dstRect.h` (scaling), else 0. Width and height are **not**
   swapped for 90° / 270° rotation [I from the code: the caller gives the dst block already rotated].
10. Result 1 → `*fence = SetNextTAGID()`, `EndDraw`, 0; else `EndDraw` and the mapped error.

### 3.8 mi_sys registration, `getcap`, exports, imports [R]
- `MI_Moduledev_RegisterDev()` (called from Open): the 0x298-byte info record (`mi_sys-1-engine.md` §2.2) all zero
  except **module id 0xD** — no devices, channels or ports: mi_gfx takes no part in binding or buffer passing. procfs ops =
  `{OnDumpDevAttr = _MI_GFX_IMPL_OnDumpDevAttr, others NULL}`; `handle = mi_sys_RegisterDev(&info, NULL, &ops,
  MI_COMMON_RealGetSelfDir(selfdir))` → proc file `/proc/mi_modules/mi_gfx/mi_gfx0` [I: the name rule of
  `mi_sys-2-interfaces.md` §10.3], and `mi_sys_RegistCommand("getcap", 0, _MI_GFX_IMPL_GetCapability, handle)`.
  Unregistered at the last Close, so the proc file exists only while the GE is open.
- `getcap` and the device dump both print, through the mi_sys print callback: a banner `-----…Gfx Dev Attr-----…`, the
  header `"%9s%24s%20s%25s%21s%22s%23s\n"` with `InitCnt, ARGB1555AlphaThreshold, ARGB1555AlphaValue, ScalingCoefficient,
  QuickFillColorFormt, BitblitSrcColorFormt, BitblitDstColorFormt`, then `%9d` InitCnt, `%24d` 8888→1555 threshold,
  `%20d` 1555→8888 alpha, `%20d/1000` coefficient, `%21d` last QuickFill / DrawLine dst GE format, `%22d` last BitBlit
  src GE format, `%22d\n` last BitBlit dst GE format (recorded before the ARGB1555 → `_DST` substitution).
- **Kernel exports (14)**: `MI_GFX_Open`, `MI_GFX_Close`, `MI_GFX_WaitAllDone`, `MI_GFX_QuickFill`, `MI_GFX_DrawLine`,
  `MI_GFX_BitBlit`, `MI_GFX_SetPalette`, `MI_GFX_GetARGB8888To1555AlphaThreshold` / `Set…`,
  `MI_GFX_GetARGB1555To8888AlphaValue` / `Set…`, `MI_GFX_GetInitialScalingCoeff` / `Set…`, `mi_gfx_PendingDone`. Each
  packs its arguments exactly like the library (§2.2, the same NULL-rect defaults and the same argument checks, including
  SetPalette's and SetInitialScalingCoeff's 1…999), fetches `table[nr]` through the local
  `MI_DEVICE_WrapperFunction(cmd)` (nr < 0xe, else NULL — called without a check) and calls it with a zeroed fake ctx (no
  `copy_from_user`), unlocking the mutex the handler stored. `MI_GFX_CreateDev` / `MI_GFX_DestroyDev` exist as local
  functions returning 0 and are **not** exported.
- **Imports from mhal: 33** — 26 `MApi_GFX_*` (`BeginDraw, BitBlt, DrawLine, EnableAlphaBlending, EnableDFBBlending,
  EnableVCmdQueue, EndDraw, Init, RectFill, SetAlpha_ARGB1555, SetAlphaThr_ARGB1555, SetClip, SetDFBBldConstColor,
  SetDFBBldFlags, SetDFBBldOP, SetDstBufferInfo, SetDstColorKey, SetMirror, SetNextTAGID, SetPaletteOpt, SetRotate,
  SetScallingCoeff, SetSrcBufferInfo, SetSrcColorKey, SetVCmdBuffer, WaitForTAGID`) and 7 GE calls
  (`MApi_GE_ClearInterrupt`, `MApi_GE_DisableInterrupt`, `MApi_GE_EnableInterrupt`, `MApi_GE_Exit`,
  `MApi_GE_SupportInterrupt`, `MApi_GE_TagIsFinish`, `MAPI_GE_GetIrqNum`); what each does: §4.13. From mi_sys / mi_common:
  `MI_SYSCFG_GetMmapInfo`, `mi_sys_MMA_Alloc` / `Free`, `mi_sys_RegisterDev` / `UnRegisterDev` / `RegistCommand`,
  `MI_DEVICE_Register` / `Unregister` / `GetProcModuleDir`, `MI_COMMON_Print` / `RealGetSelfDir`,
  `MI_SYS_LOG_IMPL_PrintLog`, the four `mi_device_*_ops`; kernel: CamOs IRQ, wait queues, mutex, proc.

## 4. mhal.ko block GE — Utopia GFX API, GE driver, GE HAL [R]
<!-- ev:gfx-ge.4 -->

### 4.1 Extent, origin, agreement with the I2 source
<!-- ev:gfx-ge.4.1 -->
- `.text` 0x02d340–0x036a78 (35.4 KB). Function counts [R]: the symbol table gives 403 named
  and 13 unnamed functions; code analysis finds 416 there, 395 with symbols and 21 static ones without (§4.14).
- It is SigmaStar/MStar **Utopia "graphic"** compiled for M6. Of the 395 named functions **314** are functions of the I2
  sources and 81 have no source (all specified in §4.2, §4.7, §4.9, §4.13, §4.14); the 314 split as `apiGFX.c` 89,
  `mapiGFX.c` 66, `drvGE.c` 87, `halGE.c` 70, `osalGE.c` 2 [R: list]. Code size per source file [R: by function-to-file attribution]: apiGFX 7.3 KB
  (95 functions), mapiGFX 11.5 KB (66), drvGE 9.4 KB (87), osalGE 2 functions, halGE 4.0 KB (70), no source 3.3 KB (83).
  The two attributions differ for apiGFX (95 vs 89) and for the no-source set. `mhal-core.md` §2.6 gives 335 of 403 as
  matched to the Utopia family (from the mhal map). Built with `MS_DEBUG` on: the HAL wait loops print and dump / reset
  after 10 rounds [R strings].
- The HAL is the I2 `hal/i2/ge` code with the M6 register base: 16-bit words of RIU bank **0x1409**, kernel address
  `0xFD000000 + 0x281200 + reg·4`, physical `0x1F281200 + reg·4` (§4.6). Register numbering = `hal/i2/ge/regGE.h` (checked
  for every register named in this document). The mhal map has the same bank and IRQ (`mhal-core.md` §2.5).
- Agreement with I2 [R]: the 159 named HAL, driver and OSAL functions plus 5 unnamed ones (those with a source name in an
  `-O2` build) were compared with the I2 `halGE.c`, `drvGE.c`, `osalGE.c` compiled for Thumb-2 and decompiled the same
  way, on called functions and folded constants with GE register addresses normalised to the register number; 101 of
  the 164 have identical sets, the rest were read as normalised diffs and against source and disassembly. The 155 API
  and mapi functions were compared on the preprocessed source text (callees, register numbers, constants, the command
  and sub-command each issues, the case content of every dispatcher switch) and read pair by pair where those differed.
  The data objects present in both (`_GE_Reg_Backup`, `g_GeChipPro`) were compared byte for byte. The I2 header that
  defines `HAL_MIU1_BASE` / `HAL_MIU2_BASE` (`halCHIP.h`) is not in the tree, so the MIU split 0x80000000 / 0xC0000000
  (§4.8) is the binary's value and is **not** checked against I2. A first, header-less constant comparison of all 314
  found 21 identical constant sets; its other differences are mostly noise from
  stubbed headers (`MsTypes` / `apiGFX.h` macros and enum values unexpanded; the binary shows RIU offsets 0x2812xx where
  the source writes `GE_REG(n)`).
- Not compared at expression level: the arithmetic inside the largest `_U02` bodies (`MApi_GFX_TextOut_U02`,
  `TriFill_U02`, `SpanFill_U02`, `ClearFrameBuffer*_U02`, `DrawBitmap_U02`, `GFX_ConvertRGB2DBFmt`) — any difference
  there keeps the same callees and constants — and the 20.12 fixed-point operations of the trapezoid helpers
  (`GE_TrapezoidFill` / `Blit` with their `…Ex` inlined, `GE_TrapezoidOverlap` inside `MDrv_GE_BitBltEX_Trape`), which
  the binary writes as shifts and masks; their callees are the same. `apiGFX.c` / `mapiGFX.c` do not compile without the
  absent `apiGFX.h` / `apiGFX_v2.h` / `utopia.h`.
- **Everything else in the 314 source-matched functions is the I2 code.** The M6 changes, each in its place: global
  register base at bank 0x1409 and hard-coded I/O bases (§4.6); no yield in the HAL waits (§4.7); non-waiting
  `GE_WaitTAGID` (§4.7); `GE_Init` warning, single `GE_SetVCmdBuffer`, once-per-boot interrupt mask (§4.8); scaling
  coefficient in `GE_Divide2Fixed` / `GE_CalcBltScaleRatio` (§4.9); colour keys with six ops, inverted destination
  polarity and exclusive enables (§4.10); `GE_GetFmtCaps` adds RGBA8888 / BGRA8888 (§4.11); thread-blind nesting in
  `GE_BeginDraw` and no API mutex (§4.3); static shared state instead of SHM, fixed `MDrv_GE_GetConfig`, reduced
  `MDrv_GE_Exit` (§4.4); direct `Ioctl_GFX_*` calls without Utopia or `memset`, `GFX_UTOPIA2K` callbacks, interrupt
  commands 0x14–0x18 and MISC 0xf–0x12, `E_MISC_EXIT` returning 0 (§4.12); empty STR and no TLB path in the buffer setters
  (§4.15); plus the 81 no-source functions.

### 4.2 Module glue: device, device tree, IRQ, clock, sysfs, OS wrappers, Utopia stubs (no source) [R]
<!-- ev:gfx-ge.4.2 -->
- **`_DrvGeModuleInit`** — step 12 of `camdriver_init`, run once through `find_exec_symbol` (`mhal-core.md` §3.2). It
  follows the pattern of steps 12–17 there: the class pointer is 0 on this only call, so it takes the class branch
  (`msys_get_sysfs_class()`, else `CamClassCreate("m_ge_class")`) and creates the device **`mge`** (`CamDeviceCreate`,
  static `MKDEV(250,240)`; major 250 is never reserved) with the sysfs file of `DrvGeSysfsInit` (`geclk`, below), but
  **adds no cdev** [I: `/dev/mge` opens with −ENXIO]. The cdev would have no file operations of interest anyway [I: no
  `mge` fops function in the block]. It then finds the DT node **`compatible = "sstar,ge0"`** (`of_find_compatible_node`),
  takes `_DrvGeGetIrqNum(node, 0)` = `CamOfIrqToResource(node, 0)` → `GE_SetIsrNum(irq)` (stores it and sets "valid";
  failure prints `Can't Get SCL_IRQ` [sic]) and `DrvGeOsSetDeviceNode(node)` (kept for the clock calls).
- The GE platform driver **"mge"** (`DrvGeModuleInit` with `DrvGeModuleProbe`) is never registered: it is an initcall
  entry, and none of mhal's initcalls runs (`mhal-core.md` §3.1). `camdriver_exit` calls `DrvGeModuleExit`, which prints
  and unregisters that never-registered driver; the `mge` device is not removed at exit (`mhal-core.md` §3.3).
- **Device tree** (node table `kernel-platform.md` §14): `ge0 { compatible =
  "sstar,ge0"; clocks = <&CLK_ge>; clock-names = "CKG_ge"; interrupts = <GIC_SPI 0x76 IRQ_TYPE_LEVEL_HIGH>; }`. `CLK_ge`
  is a `sstar,composite-clock` at register 0x1F207144 (CLKGEN bank 0x1038 reg 0x51), mux bits [4:2] over 8 parents, gate
  bit 0 (parents: `kernel-platform.md` §5.1). mhal reads `interrupts[0]` and `clocks[*]` of the node
  (`mhal-core.md` §7). The GE's MIU client id: `kernel-platform.md` §12.2.
- **IRQ** = the DT's SPI 0x76 resolved through the GIC [I: so `MAPI_GE_GetIrqNum` returns the Linux virq].
  `MApi_GE_SupportInterrupt` → `MDrv_GE_SupportInterrupt` returns **1 unconditionally**, so mi_gfx always takes the
  interrupt path on M6 (§3.3, §3.6). `GE_GetIsrNum` returns 0 and prints if `_DrvGeModuleInit` never stored a number.
  The IRQ number, its "valid" flag and the once-per-boot flag of `GE_Init` are three statics without symbols at
  `_gGE_PM_MapBase + 4`, `+ 8`, `+ 9` (§4.8) [R offsets; I: separate objects].
- **Clocks** (through the stored node; each loops over all clocks of the node, here one):
  `DrvDeOsSetClk(rate)` [sic "De"] — `rate == 0`: set the parent to the clock's parent #0, prepare + enable;
  `rate ≠ 0`: `CamClkSetRate(clk, rate)`, prepare + enable. `DrvGeOsSetClkOn()` — parent #0 + enable, prints the parent
  count; **no caller** (no relocation to it in mhal.ko) — unused. `DrvGeOsSetClkOff()` — disable + unprepare.
  `MDrv_GE_Init` calls `DrvDeOsSetClk(0)`, `MDrv_GE_Exit` calls `DrvGeOsSetClkOff()`. A second clock write in the HAL
  (`GE_SetClock`, register 0x1F001720): §4.8.
- **sysfs `geclk`** on the `mge` device [I: path `/sys/class/…/mge/geclk` from the class above]: read
  (`check_geclk_show`) prints `GE Now_clk equal miu clk` (rate 0) or `GE Now_clk <rate>`, then the list 216000000,
  172800000, 144000000, 320000000, 384000000, 432000000, 240000000 [I: the rates of `CLK_ge` parents #1–#7]. Write
  (`check_geclk_store`) `"<id> clk=<rate>"` (decimal; the id is parsed and ignored): rate ≠ 0 → stored,
  `DrvDeOsSetClk(…)`, `Geclk %d success`; anything else prints the usage (`echo [id] [clk=xxx] >geclk`,
  `0 (->clk_miu)`, the rate list with 172800000 twice). [R strings; I: the write passes the stored rate — the decompile
  drops the argument.]
- **`DrvGeOs*`** (CamOs wrappers): `Init` (flags), `Create` / `Start Timer` (CamOsTimer, re-armed by
  `_DrvGeOsTimerNotify`, which then calls the stored callback), `Create` / `Destroy` / `Obtain` / `Release Semaphore`
  (CamOsTsem), `… Mutex` (CamOsMutex; Obtain / Release return true on success), `CreateTask` (CamOsThread, priority
  0x32, optional wake-up), `DestroyTask`. The interrupt hooks `Attach` / `Detach` / `Enable` / `Disable Interrupt` and
  `RestoreAllInterrupts` return 1, `DisableAllInterrupts` returns 0 — all stubs. `DrvGeModuleSuspend` returns 0 (stub).
  `HAL_GE_exit` returns 1. `DrvGeOsGetMmioBase(…, 0)` returns the RIU window 0xFD000000 / 4 MiB and `(…, 1)` 0xFD200000 /
  2 MiB; it has **no caller** in mhal.ko — unused (the I/O bases come from `OSAL_GE_GetMapBase*`, §4.6).
- **Utopia framework**: the 14 `Utopia*` functions in the block are **stubs returning 0** (`UtopiaInstanceCreate`,
  `UtopiaInstanceDelete`, `UtopiaInstanceGetModule`, `UtopiaModuleCreate`, `UtopiaModuleRegister`,
  `UtopiaModuleSetupFunctionPtr`, `UtopiaModuleSetupSTRFunctionPtr`, `UtopiaModuleSetSTRPrivate`,
  `UtopiaModuleAddResourceStart` / `End`, `UtopiaResourceCreate`, `UtopiaResourceRegister`), except
  `UtopiaOpen(…, &inst)`, which zeroes one static 0x700-byte private area and returns a static instance pointing at it,
  and `UtopiaInstanceGetPrivate(inst, &priv)` = `*inst`. So there is exactly one GE instance per kernel and no real
  Utopia core; `GFXRegisterToUtopia` (module "GFX" with `GFXOpen` / `GFXClose` / `GFXIoctl` / `GFXStr`, STR private size
  0x604, resource "ge0") and `GFXOpen` (`UtopiaInstanceCreate(0x700, …)`) do nothing useful. The API layer calls the
  `Ioctl_GFX_*` functions directly (§4.12), never `GFXIoctl`.

### 4.3 Call path, fire-info record, locking [R]
- **Path.** `MApi_GFX_*` / `MApi_GE_*` (apiGFX.c) → if the global instance `GFXFireInfo + 0xac` (`pGEInstance`) is NULL:
  print `should call MApi_GFX_Init first`, return **0x104** (`GFX_INIT_FAIL`); else call the matching `Ioctl_GFX_*`
  function of mapiGFX.c **directly** (§4.12) → `UtopiaInstanceGetPrivate` → `MDrv_GE_*` (drvGE.c) on the driver
  context `priv + 0x1c` → HAL `GE_*` (halGE.c) on the HAL context `drvctx + 0x5c` → registers.
- **State setters only record.** `SetSrc/DstBufferInfo`, `SetClip`, `SetDFBBld*`, `EnableDFBBlending`,
  `EnableAlphaBlending`, `SetSrc/DstColorKey`, `SetMirror`, `SetRotate` and the other `MApi_GFX_Set*` of the
  "fire-info" family store into `GFXFireInfo` and set a dirty bit; nothing reaches the GE until a draw call. The draw
  ioctls (`Ioctl_GFX_BitBlt`, `_RectFill`, `_LineDraw`, `_Bitmap`, `_Font`, …) first apply the dirty state with
  `GFX_SetFireInfo` (= `GFX_SetFireInfo`, the source's bit order), then program the primitive. Dirty bits, as the
  setters write them and `GFX_SetFireInfo` applies them:

| bit | value | state |
|---|---|---|
| 0 | 0x1 | source buffer |
| 1 | 0x2 | destination buffer |
| 2 | 0x4 | DFB |
| 3 | 0x8 | alpha blend (DFB + alpha setters write 0xc) |
| 4 | 0x10 | source colour key |
| 5 | 0x20 | destination colour key |
| 6 | 0x40 | alpha compare |
| 7 | 0x80 | clip |
| 8 | 0x100 | source mirror |
| 9 | 0x200 | destination mirror |
| 10 | 0x400 | rotate |
| 11 | 0x800 | CSC |
| 12 | 0x1000 | stretch colour-key type |
| 13 | 0x2000 | nearest |
| 14 | 0x4000 | dither |

- **No API mutex.** `GFX_FireInfoLock` / `GFX_FireInfoUnLock` (I2: `GEWrapper_Mutex` around every fire-info access)
  are absent; the fire-info setters store and set the dirty bit unprotected.
- **Driver lock.** `_API_GE_ENTRY` (0x4002f45e: `GE_Get_Resource(ctx, 1)`, nesting counter) and `_API_GE_RETURN`
  (0x4002f48c) bracket each ioctl. `GE_BeginDraw` (0x40032694) checks the context magic **0x55AABEEF** (ctx +0x694)
  and that the mutex id at ctx +0x67c is ≥ 0; if the nesting counter (ctx +0x684) is > 0 it **just increments it,
  whatever the thread**; otherwise it takes the global `_gstDrvGeMutex` through `DrvGeOsObtainMutex` (retrying with a
  print until it succeeds), increments, and, if another context id owned the HAL last, runs `GE_Restore_HAL_Context`.
  The owner field ctx +0x680 is never written. `GE_EndDraw` (0x4003262e) has no thread check, keeps the count check
  (prints `Assert in %s,%d` and `No Mutex to release` when the count is < 1, then continues), decrements, and releases
  the global mutex when the old count was < 2. I2 instead records the owner thread (`s32GELock` =
  `MsOS_GetOSThreadID()`, ctx +0x680), re-enters only for that thread and makes any other thread wait on the context
  mutex (`MsOS_ObtainMutex`). [I: two threads entering the driver at once are therefore not serialised once one of them
  holds the lock; above it, mi_gfx's ioctl handlers take the module mutex, except SetPalette (§3.4).]
- `MApi_GFX_BeginDraw` / `EndDraw` are `Ioctl_GFX_MISC` sub-commands 6 / 7, which call `MApi_GFX_BeginDraw_U02` /
  `EndDraw_U02` → `GE_BeginDraw` / `GE_EndDraw`. Like I2 they have **no init check**: before `MApi_GFX_Init` they pass a
  NULL instance to `Ioctl_GFX_MISC`, whose `UtopiaInstanceGetPrivate(NULL, …)` reads address 0 [R; I: a kernel oops — in
  I2, `UtopiaIoctl` would have rejected the NULL instance].
- The binary's `MDrv_GE_*` functions have no `GE_ENTRY` / `GE_RETURN` lock calls (the source macros are compiled empty in
  this configuration).

### 4.4 Init, configuration structs, exit [R]
The structures (disassembly of `MApi_GFX_Init`, `.text` 0x2d7bc, for the byte layout):

| struct | where | layout |
|---|---|---|
| `GFX_Config` (16 B, packed; built by mi_gfx, §3.3) | argument of `MApi_GFX_Init` | +0 u8 `u8Miu`, +1 u8 `u8Dither`, +2 u32 `u32VCmdQSize`, +6 u64 `u64VCmdQAddr`, +0xE u8 `bIsHK`, +0xF u8 `bIsCompt` |
| `GFX_Init_Config` (14 B, packed, on `MApi_GFX_Init`'s stack) | `GFX_INIT_ARGS.pGFX_Init` | +0 u32 VCmdQ size, +4 u64 VCmdQ address, +0xC u8 bIsHK, +0xD u8 bIsCompt |
| `GFX_INIT_ARGS` (12 B, packed) | the "ioctl" argument | +0 u32 = 0, +4 pointer to `GFX_Init_Config`, +8 u32 size = 0xE |
| `GE_Config` (0x18 B, natural alignment; `drvGE.h`) | `Ioctl_GFX_Init`'s stack, zeroed first | +0 u8 `u8Miu`, +1 u8 `u8Dither`, +4 u32 `u32VCmdQSize`, +8 u64 `PhyVCmdQAddr`, +0x10 u8 `bIsHK`, +0x11 u8 `bIsCompt` |

`MApi_GFX_Init` → `Ioctl_GFX_Init` → `MDrv_GE_Init` → `GE_Init`:
1. **`MApi_GFX_Init(cfg)`** copies `u32VCmdQSize`, both halves of the address, `bIsHK` and `bIsCompt` into
   `GFX_Init_Config`; **`u8Miu` and `u8Dither` are not copied**. If the instance pointer (`GFXFireInfo + 0xAC`) is NULL
   it calls `UtopiaOpen` (the static instance, §4.2; failure: print "GFXOpen Fail", return); then it calls
   `Ioctl_GFX_Init` directly (failure: print "GFXIoctl Fail"). It has none of the I2 source's callback registration
   (`_GFX_RegisterAllCBFunc`), wrapper mutex (`MsOS_CreateMutex("GEWrapper_Mutex")`) or `atexit(MApi_GE_Exit)`.
2. **`Ioctl_GFX_Init`**: NULL `pGFX_Init` → print "GFX init FAIL", return 1. Else `memset(GE_Config, 0, 0x18)` and copy
   size, address, `bIsHK`, `bIsCompt` — so `GE_Config.u8Miu = 0` and `u8Dither = 0` always. The `u32Size` field of
   `GFX_INIT_ARGS` is not checked (the source's `CheckSize` is compiled empty in this build — no compare in the binary).
   First time only (private byte +0x34 clear): `MDrv_GE_Init(inst, &GE_Config, priv + 0x1C)` and
   `ctx->pBufInfo.tlbmode` (+0x38) = 0 (`E_GE_TLB_NONE`); then `_API_GE_ENTRY`, `MDrv_GE_Chip_Proprity_Init(ctx,
   priv + 0x20)` (a pointer to the `g_GeChipPro` capability table, §4.11), `MDrv_GE_SetOnePixelMode(ctx,
   !chipProperty->byte3)`, set the init byte, `_API_GE_RETURN(ctx, 0)`, return 0. Same order and fields as the source.
3. **`MDrv_GE_Init(inst, GE_Config, &ctx)`**:
   - `DrvDeOsSetClk(0)` first (GE clock on, parent #0, §4.2);
   - the shared state is the static `g_drvGECtxShared` (0x223 B, `.bss`), `memset` on every init (I2 maps it through
     `MsOS_SHM_GetId("GE driver", 0x223)`); the HAL context (0x618 B) is zeroed too;
   - **`shared.bGE_DirectToReg = 1`** (§4.6);
   - from `GE_Config` it takes `bIsCompt` → local context `bIsComp` (+0x6D2) and HAL `bIsComp` (+0x2AC); size,
     address, `bIsHK`, `bIsCompt` → the shared state (`g_drvGECtxShared` +0x214…+0x221, which `MDrv_GE_GetConfig` reads
     back; the same bytes appear in the decompile as words 0x90–0x93 and byte 0x24D of the `_gstDrvGeMutex` area [I: one
     object addressed from two symbols]); it never reads `u8Miu` / `u8Dither`;
   - context-id counter (it wraps to id 1 as in I2; M6 writes the packed u32 counter bytewise);
   - I/O bases from `OSAL_GE_GetMapBase` / `OSAL_GE_GetMapBase2` (both 0xFD000000, §4.6);
   - the mutex: `DrvGeOsCreateMutex(&_gstDrvGeMutex)` (I2: `MsOS_CreateMutex` into ctx +0x67c); M6 stores the value of
     the first word of `_gstDrvGeMutex` into ctx +0x6bc and +0x67c and fails with `%s Fail` if it is negative;
   - `GE_BeginDraw`; default source / destination format value 9 [I: ARGB1555] at ctx +0x18 / +0x1c; `GE_Init` (§4.8);
     `GE_InitCtxHalPalette` (palette at HAL context +0x214, as I2); `GE_SetActiveCtrlMiu1` (§4.8);
     `MDrv_GE_SetVCmd_W_Thread(4)` / `MDrv_GE_SetVCmd_R_Thread(4)` (TH thresholds 4 / 4, §4.5); mark the palette
     context valid; `GE_EndDraw`.
4. **`GE_Init`**: §4.8.

**`MDrv_GE_GetConfig`**: if `g_drvGECtxShared.bInit` (+0x222) is set it copies VCMDQ size / address, `bIsHK`,
`bIsCompt` (+0x214…+0x221) to the caller and returns `E_GE_OK`, else `E_GE_FAIL`. (I2 queries / creates the SHM, then
`memset`s the shared state and tests `bInit`, so it always fails.)

**`MApi_GE_Exit()`** → `Ioctl_GFX_MISC` sub-command 1 (`E_MISC_EXIT`) → **`MDrv_GE_Exit`**: `DrvGeOsSetClkOff()`,
release the global mutex if the nesting count (+0x6c4 of the instance private area) is > 0 (count set to 0),
`HAL_GE_exit` (no-op), destroy the mutex; the API clears the instance pointer. (I2: `GE_AtExit()`.) The VCMDQ registers
are left as they were; mi_gfx turns the queue off before (§3.3).

### 4.5 Virtual command queue set-up: `Ioctl_GFX_SetConfig` sub-command 7 (`E_GFX_SET_VQ`) [R]
**One shared record.** The four API functions that configure the queue do not build their own payload: they update
fields of one 15-byte packed record at `GFXFireInfo + 0xB0` (the source's global `GFXSerVQ`, placed inside
`GFXFireInfo` in this build) and pass the **whole** record each time (`GFX_SETCONFIG_ARGS` {+0 u32 0, +4 u32 cmd = 7,
+8 pointer = `GFXFireInfo + 0xB0`, +0xC u32 size = 0xF}):

| record offset (abs.) | field | written by |
|---|---|---|
| +0x0 (0xB0) | u8 `bEnable` | `MApi_GFX_EnableVCmdQueue(b)` |
| +0x1 (0xB1) | u64 `u32Addr` (low word at 0xB1, high at 0xB5) | `MApi_GFX_SetVCmdBuffer(u64 addr, code)` |
| +0x9 (0xB9) | u32 `enBufSize` (GFX size code) | `MApi_GFX_SetVCmdBuffer` |
| +0xD (0xBD) | u8 `u8R_Threshold` | `MApi_GE_SetVCmd_R_Thread(t)` |
| +0xE (0xBE) | u8 `u8W_Threshold` | `MApi_GE_SetVCmd_W_Thread(t)` |

The record is in `.bss` (zero at module load) and nothing else writes it (only these four stores in the block). Every
one of these API calls returns 0x104 before `MApi_GFX_Init`, else 1 when `Ioctl_GFX_SetConfig` returned 0, and 0 (with
`%s fail [LINE:%d]`) otherwise.

**Case 7, in this order** (no size check; the case always reports success, sub-results are dropped; same order and
fields as the source's `E_GFX_SET_VQ` case in `mapiGFX.c`):
1. `MDrv_GE_EnableVCmdQueue(ctx, bEnable)`: `GE_WaitIdle`; CFG (reg 1) bit 1 set or cleared.
2. `MApi_GFX_SetVCmdBuffer_U02(inst, addr, enBufSize)`: the GFX code is accepted as is when 0–8 (inlined
   `MApi_GFX_MapVCMQSZ`, identity); any other value prints `[GFXDBG]… ERROR: 00000004` (only while the private word at +4
   is < 2 — a debug-level field [I]) and becomes 0; then `MDrv_GE_SetVCmdBuffer` → `GE_WaitIdle`,
   `GE_SetVCmdBuffer(hal, addr, code)` (§4.8).
3. `MDrv_GE_SetVCmd_R_Thread(ctx, u8R_Threshold)`: `GE_WaitIdle`; TH (reg 3) [11:8] = threshold.
4. `MDrv_GE_SetVCmd_W_Thread(ctx, u8W_Threshold)`: `GE_WaitIdle`; TH [7:4] = threshold.

**What mi_gfx's Open leaves** (§3.3: `MApi_GFX_Init`, then `MApi_GFX_SetVCmdBuffer(addr, code)`, then
`MApi_GFX_EnableVCmdQueue(1)`) [R: the steps; I: the composition]:
- `MApi_GFX_Init` → `MDrv_GE_Init` sets the thresholds to 4 / 4 (TH [11:8] = [7:4] = 4) and, for a queue ≥ 8 KiB,
  enables and programs the queue (`GE_Init`, §4.8).
- `SetVCmdBuffer` sends the record {enable 0 (still the `.bss` zero), addr, code, R 0, W 0}: the queue is **disabled**,
  base and size rewritten, **both thresholds set to 0**.
- `EnableVCmdQueue(1)` sends {1, addr, code, 0, 0}: queue enabled again, base and size written a third time, thresholds 0.
- End state: VCMDQ on with mi_gfx's address and size code, TH [11:4] = 0 — the driver's 4 / 4 defaults do not survive,
  because mi_gfx never calls the threshold setters. `MApi_GFX_EnableVCmdQueue(0)` at Close re-sends the record with the
  stored address, so the base / size registers are rewritten while the queue is turned off.
- With mi_gfx's MMA fallback (0x20000 B) the queue is already enabled and programmed inside `MApi_GFX_Init`. A 4 KiB
  queue (size code 0, which mi_gfx accepts) is not enabled by `GE_Init` (that needs ≥ 8 KiB) but is by the later
  sub-command 7, and is programmed as 8 KiB (§4.8) [I: a 4 KiB buffer would be overrun].

### 4.6 HAL register access and the register map [R]
**Base.** The I2 `GE_REG(r)` is `RIU[(r·2) + 0x2800]` on the context's `va_mmio_base` (byte offset 0x5000 + 4·r). M6
reads and writes `*(_gGE_PM_MapBase) + 0x281200 + 4·r` — a global base, not the context field. `GE_Set_IOMap_Base`
stores the base both into the context (`va_mmio_base`, as I2) and into `_gGE_PM_MapBase` (a 4-byte `.bss` object);
`MDrv_GE_Init` passes it 0xFD000000, so the GE bank is at kernel address 0xFD281200 (physical 0x1F281200, RIU bank
0x1409). This affects `GE_WriteReg`, `GE_ReadReg`, `GE_WaitCmdQAvail`, `GE_WaitIdle`, `GE_Init_RegImage`, `GE_Reset` and
`GE_DumpReg`. Context-relative accesses that remain: `GE_SetActiveCtrlMiu1` and `GE_SetClock` (§4.8).

**I/O bases.** `OSAL_GE_GetMapBase` and `OSAL_GE_GetMapBase2` both return the constants **0xFD000000, size 0x400000**
(I2: `MDrv_MMIO_GetBASE` for `MS_MODULE_GE` and `MS_MODULE_PM`). So `va_mmio_base` and `va_mmio_base2` are both
0xFD000000 (the RIU window ↔ physical 0x1F000000, `kernel-platform.md` §3.1).

**Register image.** The HAL context keeps `u16RegGETable[0x80]` (ctx +0x10); the shared state keeps a copy of CFG.
- `GE_WriteReg(r, v)`: `r < 0x80` → store in the image (r == 1 also into the shared CFG copy); `r ≥ 0x80` → print
  `Reg Index [%d]is out of GE_TABLE_REGNUM` (line number 0x302 in the print; I2: 0x2a2) and store nothing in the image.
  Then, because **`bGE_DirectToReg == 1`** (set by `MDrv_GE_Init`, always on M6), `GE_WaitCmdQAvail(1)` and write the
  hardware register at once — **every** write is immediate (through the GE command FIFO), an index ≥ 0x80 included, which
  lands at `0x281200 + 4·r`, past the 0x80-word bank (as in I2).
- The non-direct mode — write only 0–5, 7–0xf, 0x28–0x2a, 0x2d–0x2f, 0x32 at once; buffer the rest; on a write to 0x60
  `CMD` wait idle, copy image regs 6, 7, 0x10–0x27, 0x2b, 0x2c, 0x30, 0x31, 0x33–0x5f, 0x61–0x7f to the hardware, then reg
  0x60 last — is in the code (same partial-write list as I2) but **unused** on M6.
- `GE_ReadReg(r)`: `r ≥ 0x80` → wait idle, hardware reg `r − 0x80`; `r == 0` → hardware bit 0 + image bits 15:1;
  `r == 1` → the shared CFG copy; 2–5, 7–0xf, 0x28, 0x29, 0x32 → hardware; 0x2a → hardware bits 2:0 + image rest;
  everything else (including 0x1e and 0x12) → **the image**. Same no-FIFO register list and masks as I2.
- `GE_Init_RegImage` and `_GE_Reg_Backup` (the 99-byte register list the context restore walks, byte-identical to I2)
  are the I2 code.

**Register names** (`regGE.h` of I2; hex register numbers of bank 0x1409):

| reg | name | reg | name |
|---|---|---|---|
| 0x00 | EN | 0x2b | DFB flags / B const |
| 0x01 | CFG | 0x2c | G / R const |
| 0x02 | DBG | 0x2d–0x2f | CLUT L / H / CTRL |
| 0x03 | TH | 0x30 | SRC_PITCH |
| 0x04 | VCMDQ_STAT | 0x32 | TAG |
| 0x05 | BIST_STAT | 0x33 | DST_PITCH |
| 0x07 | STAT | 0x34 | FMT |
| 0x08–0x0f | MIU protect | 0x55–0x58 | CLIP L / R / T / B |
| 0x10 | ROP2 | 0x59 | ROT / stretch-colour-key mode |
| 0x11 | BLEND | 0x5e / 0x5f | BLT dst x / y offset (scale init) |
| 0x12 | ALPHA | 0x60 | CMD |
| 0x13 | ALPHA_CONST | 0x61 / 0x62 / 0x63 | line delta / style / length |
| 0x14–0x1b | source / destination colour-key ranges | 0x64 / 0x65 | BLT src dx / dy |
| 0x1c | OP_MODE | other regs of 0x61–0x7f | further primitive parameters (italic among them; numbers not listed) |
| 0x1d | ATEST_TH | 0x68–0x6d | vertices |
| 0x1e | named SRCMASK_GB in regGE.h; used as **interrupt control** (§4.7) | 0x6e / 0x6f | source w / h |
| 0x1f | YUV_MODE | 0x70–0x7f | colour + gradients |
| 0x20 / 0x21 | SRC_BASE | | |
| 0x24 / 0x25 | no name in regGE.h (§4.7) | | |
| 0x26 / 0x27 | DST_BASE | | |
| 0x28 / 0x29 | VCMDQ_BASE L / H | | |
| 0x2a | VCMDQ_SIZE (bits 2:0) / DFB_BLD_OP | | |

Bits used in this document: CFG bit 0 = CMDQ on, bit 1 = VCMDQ on, bit 14 (cleared by `MDrv_GE_FillRect`, set by
`MDrv_GE_BitBltEX` when `bSupportSpiltMode`, §4.15), bit 15 (0x8000) = power saving; STAT bit 0 = busy, [7:3] == 0x10 =
second-level queue empty, [15:11] = command-FIFO level the waits compare against [I: free slots]; TH [7:4] / [11:8] = VCMDQ write /
read thresholds; EN 0x40 / 0x80 / 0x08 / 0x10 = source colour key / destination colour key / source alpha key /
destination alpha key enables (§4.10).

### 4.7 Waits, reset, tags, interrupt control [R]
**Waits** (the `MS_DEBUG` variants: after 0x80000 spins a print, after 10 prints `GE_DumpReg` + `GE_Reset`). The M6
loops spin without any call; I2 calls `GE_YIELD()` = `MsOS_YieldTask()` in `GE_WaitCmdQAvail` and `GE_WaitIdle`.
- `GE_WaitCmdQAvail(n)`: CFG bit 1 (VCMDQ) clear → spin while `STAT[15:11] < min(n, 8)`; set → `max = 512 <<
  VCMDQ_SIZE[2:0]`, `n = min(n, max)`, spin while `STAT[15:11] < n`, then while `VCMDQ_STAT | (BIST_STAT bit0 << 16)
  ≥ max − n`.
- `GE_WaitIdle`: write TAG (0x32) with the shared tag value, `GE_WaitCmdQAvail(8)`, spin until `STAT[7:3] == 0x10`
  (second-level queue empty), then until STAT bit 0 (busy) clears.
- `GE_Reset` (0x400359dc): save EN and CFG, write both 0, restore both. `GE_DumpReg` (0x40035a18): print regs
  0–0x7f.

**Tags (fences).** `GE_GetNextTAGID(ctx, 1)`: shared 16-bit counter + 1, skipping 0. `MDrv_GE_SetNextTAGID(&tag)` /
`MDrv_GE_SetTAGID(tag)`: write TAG (0x32) **4 times** with that value [I: it travels through the command queue, so the
register reaches the value when the engine has drained everything before it]. `GE_Tag_Finish(tag)` (no source):
`hw = TAG`; finished if `tag − hw` is "not ahead" in 16-bit serial arithmetic (difference < 0x8000), else finished only if
STAT bit 0 (busy) is clear.

**`GE_WaitTAGID(tag)` on M6 does not wait**: if `TAG < tag` it calls the interrupt control with 5 (clear + unmask),
writes reg 0x24 = 0 and reg 0x25 = `tag` four times each, then interrupt control 10 (mask) and returns. The I2 body
instead polls TAG with `MsOS_DelayTask` and 16-bit serial arithmetic. So `MApi_GFX_WaitForTAGID` returns before the
operation ends; only the IRQ path of mi_gfx (§3.6) really waits. What regs 0x24 / 0x25 do is not known — regGE.h of I2
has no name for them, and no M6 reference exists [I: they arm a "tag reached" interrupt or a queue barrier].

**Interrupt control** (`GE_IntCtrl(ops)`, no source; reg 0x1e read from the image, written through):
`ops` bit 0 → write `v | 0x0c00` (pulse **clear**, bits 11:10), continue with `v & ~0x0c00`; bit 1 → `v |= 0x00c0`
(**mask**, bits 7:6); bit 2 → `v &= ~0x00c0` (unmask); bit 3 → `v |= 0x0020`; bit 4 → `v &= ~0x0020`; write `v`.
`GE_Enable_INT` = 0x15 (clear, unmask, bit 5 off), `GE_Disable_INT` = 0x0a (mask, bit 5 on), `GE_Clear_INT` = 0x01.
`MApi_GE_EnableInterrupt` / `DisableInterrupt` / `ClearInterrupt` → `Ioctl_GFX_*` → `MDrv_GE_*` → these. `GE_Init`
leaves reg 0x1e = 0x00e0 (all masked) when its once-per-boot step runs (§4.8). The meaning of bit 5 and of bits 6 and 7
taken separately is not known [I: bit 5 is a second enable / inhibit].

### 4.8 `GE_Init`, `GE_ResetState`, `GE_SetVCmdBuffer`, the clock and MIU writes [R]
**`GE_Init(hal, &GE_Config)`** (the source's structure with `GE_VQADDR_LOCK_PATCH` defined), in M6 order:
1. `GE_WaitIdle`; `GE_SetClock(1)`.
2. If CFG bit 1 (VCMDQ) is already set: print ` warning!!! Virtual Command queue has been activated!! ` and skip to step
   3 (I2: an empty `else`). Otherwise: CFG |= 1 (CMDQ); FMT (0x34) = 0x0909 (ARGB1555 source and destination); **if
   `u32VCmdQSize` ≥ 0x2000 (8 KiB)**: CFG |= 2 (VCMDQ) **then** `GE_SetVCmdBuffer(addr,
   _GE_ConvertVQSize2VQSizeEnum(size))` — **once** (I2 calls it twice and sets CFG bit 1 again after it); then
   `GE_ResetState`.
3. Both branches: TH (reg 3) |= 0x0d; CFG |= 0x8000 (power saving); reg 0x1e = 0x00c0; `GE_EnableDynaClkGate(1)` (CFG
   |= 0x8000 again; prints its name).
4. Binary-only tail, once per boot: if the byte at `_gGE_PM_MapBase + 9` is 0, read reg 0x1e and, **only if its bit 5 is
   clear**, run interrupt control 10 (mask, bit 5 on → reg 0x1e = 0x00e0); then set the byte to 1.

**`GE_ResetState`** (same as I2): wait idle; EN = 1; TH = 0; reg 0x62 (line style) = 0x100; reg 0x59 = 0x40; BLEND
(0x11) = 0xff00.

**`GE_SetVCmdBuffer(hal, u64 phys, code)`** (same as I2):
- `code ≥ 8` (1024 K) → return 5 (not supported), nothing written.
- MIU of the address: < 0x8000_0000 → MIU0, < 0xC000_0000 → MIU1, anything else (including a non-zero high word) → 2.
  Register address = `_GFXAPI_PHYS_ADDR_IN_MIU(addr) & 0x7fffffff` (bit 31 cleared), bit 31 set only for MIU1 (the source's
  `GE_ConvertAPIAddr2HAL`, `MIU_SELETE_OFFSET` 31). The source's `GE_SetVQBufMIUId` call has no counterpart in the
  binary.
- **Five times**: VCMDQ_BASE_H (0x29) = address >> 16, VCMDQ_BASE_L (0x28) = address & 0xFFFF
  (`GE_VQADDR_ENHANCE_LOCK_TIMES` = 5 [R: loop count]).
- VCMDQ_SIZE (0x2a) [2:0] = 0 for codes 0 / 1 (4 K and 8 K both → the 8 K setting), `code − 1` for codes 2–7 (16 K → 1 …
  512 K → 6) — the second `GE_VQ_*` table of `halGE.h` (8K 0 … 1024K 7). The GFX / driver size codes are `E_GE_VCMD_4K` =
  0 … `E_GE_VCMD_1024K` = 8 (`drvGE.h`). The queue holds `512 << VCMDQ_SIZE` 16-byte commands (= 8 KiB << VCMDQ_SIZE),
  so mi_gfx's 128 KiB (code 5 → 4) is programmed as 128 KiB.

Helpers of the address conversion (same as I2): `_GFXAPI_MIU_ID`, `_GFXAPI_PHYS_ADDR_IN_MIU`,
`_GFXAPI_PHYS_ADDR_2_API` (prints `!!!!!! get miu1 base error!!!!!!` on every call, from the I2 `_GET_MIU_MASK_SHIFT()`
macro; address mask 0x7ffffff), `_GE_ConvertVQSize2VQSizeEnum`. The MIU split values come from the binary only (§4.1).

**Clock register in the HAL.** `GE_SetClock(on)` toggles bit 0 at `va_mmio_base + 0x1720` (`va_mmio_base` = ctx +8 =
0xFD000000, §4.6), i.e. kernel 0xFD001720 = **physical 0x1F001720 = RIU bank 0x000B reg 0x48**; on → clear, off → set.
This is the I2 `CLK_REG(CHIP_GE_CLK)` = `RIU[0x0B00 + 0x48·2]` kept unchanged (same as I2). On M6 the GE clock of the
device tree is CLKGEN bank 0x1038 reg 0x51 (`CLK_ge`, §4.2), set by `DrvDeOsSetClk`. What bit 0 of 0x1F001720 controls
on M6 is not known [I: a leftover write that hits another register on M6].

**MIU client select.** `GE_SetActiveCtrlMiu1`: `va_mmio_base + 0xdf4 |= 0x8000` (same as I2 `MIU1_REG(MIU1_GEGROUP) |=
MIU1_GE_CLIENT`) [I: arithmetic — physical 0x1F000DF4, bank 0x0006 reg 0x7d].

### 4.9 Scaling, the scaling coefficient, ARGB1555 alpha [R]
**Scale ratio** (`GE_CalcBltScaleRatio(src w/h, dst w/h)`, used for every stretched BitBlt; identical to I2 except the
initial-phase division): x / y ratio = `GE_Divide2Fixed(src, dst, shiftRangeMin, 12, 0)`, or 0xFFFFFFFF when
`src ≥ dst << g_GeChipPro.u8ShiftRangeMin` (more down-scaling than supported); initial phase
`init_x = GE_Divide2Fixed(src − dst, dst, 0, 12, coeff)` with sign bit 12 set when `src < dst`, the coefficient read from
`g_GeChipPro + 0x12`; the same for y; if the context's Y-scaling patch flag is set and `src h < 6` → `init_y = 0x1000`.
I2 computes the initial phase as `GE_Divide2Fixed(src − dst, (dst & 0x7fff) << 1, 0, 12)`.

`GE_Divide2Fixed(x, y, nInt, nFrac, coeff)` (the fifth argument is new on M6): sign-magnitude of the 16-bit inputs,
`q = (|x| << nFrac) / |y|`, but with `coeff ≠ 0`: `q = (coeff · (|x| << nFrac)) / (|y| · 1000)`; negate if the signs
differ; mask to `1 + nInt + nFrac` bits. Every caller other than the initial-phase division passes 0 (the two ratio
calls, `MDrv_GE_DrawLine`, `GE_NormalizeTrapezoidDef`, `GE_CalcColorDelta`). The default coefficient **500**
reproduces the I2 division by `2·dst`; other values make the initial phase tunable.

`_GE_SetBltScaleRatio2HW`: reg 0x64 = x ratio low, 0x65 = y ratio low, 0x5e / 0x5f = init x / y, then 0x5e / 0x5f again
with bit 15 = ratio bit 16 (`(image & 0x7fff) | hi << 15`). `GE_SetBltScaleRatio` and `_GE_SetBltScaleRatio2HW` are the
I2 code.

**Scaling coefficient (no source).** `MApi_GFX_SetScallingCoeff(u16)` → `Ioctl_GFX_MISC` sub 0x11 →
`MDrv_SetBltScaleCoeff` → `GE_SetBltScaleCoeff(v)`: `v` outside 1…999 → 500; `g_GeChipPro.u16ScaleCoeff = v` — a global,
no register; used by the next stretch. `MApi_GFX_GetScallingCoeff` (sub 0x12) reads it. The coefficient is the halfword
at `.data` 0x1a02 (default `f4 01`), right **after** the 0x12-byte `g_GeChipPro` object, which the code addresses as
`g_GeChipPro + 0x12` [R]; it is a separate static the symbol table does not name [I]. Both sub-commands do their work and
then return `MapRet(GFX_FAIL)` = 1 (§4.12), so `MApi_GFX_SetScallingCoeff` / `GetScallingCoeff` print `%s fail
[LINE:%d]` on every call and still return 1 — the call works; mi_gfx's Set stores its copy only on a return of 1 (§3.6).

**ARGB1555 alpha threshold (no source).** `MApi_GFX_SetAlphaThr_ARGB1555(u8)` (MISC sub 0x0f) →
`MDrv_GE_SetAlphaThr_ARGB1555`: ALPHA (0x12) = `(image & 0xff00) | v`; `Get…` (sub 0x10) returns the ALPHA low byte from
the image. `MApi_GFX_SetAlpha_ARGB1555` (sub 0x0d) is a source function [I: it sets the ALPHA high byte, the alpha used
for ARGB1555 sources, per source]. All of them return 1 except "not initialised" (0x104).

### 4.10 Colour keys [R]
`MDrv_GE_SetSrcColorKey` / `MDrv_GE_SetDstColorKey` have **six** ops on M6 (I2: 0 `E_GE_CK_EQ`, 1 `E_GE_CK_NE`,
2 `E_GE_ALPHA_EQ`, 3 `E_GE_ALPHA_NE`); this is why `MApi_GFX_MapCKOP` maps 0–5 and mi_gfx clamps the op to 0…5:

| op | source key: OP_MODE (0x1c) / EN (0) bit | destination key: OP_MODE / EN bit |
|---|---|---|
| 0 | bit 0 clear / 0x40 (SCK) | **bit 1 set** / 0x80 (DCK) |
| 1 | bit 0 set / 0x40 | **bit 1 clear** / 0x80 |
| 2 | bit 2 clear / 0x08 (ASCK) | **bit 3 set** / 0x10 (DSCK) |
| 3 | bit 2 set / 0x08 | **bit 3 clear** / 0x10 |
| 4 (new) | bits 0, 2 clear / 0x48 | bits 1, 3 set / 0x90 |
| 5 (new) | bits 0, 2 set / 0x48 | bits 1, 3 clear / 0x90 |
| other | OP_MODE unchanged / none | unchanged / none |

- The source-key polarity equals I2 (EQ = bit clear); the **destination-key polarity is inverted** against I2 (I2: EQ
  clears OP_MODE bit 1 / bit 3). Whether that is a fix for M6 hardware or a different meaning of the destination bits is
  not known: static code does not tell.
- Enable: M6 writes `EN = (EN & ~0x48) | en` (source) / `(EN & ~0x90) | en` (destination) — the colour and alpha keys of
  one side exclude each other — and disable clears both bits of the side; I2 only ORs in / clears the one bit.
- Thresholds: source key LTH = `ck_low` (0x16 / 0x17), HTH = `ck_high` (0x14 / 0x15), as I2; the destination key keeps
  the I2 mix — LTH_L (0x1a) and HTH_L (0x18) = `ck_low & 0xffff`, LTH_H (0x1b) and HTH_H (0x19) = `ck_high >> 16`.
- These rules apply to every caller, including the unused families (§4.15).

### 4.11 Formats, format capabilities, chip properties [R]
**GE format table** (`GE_GetFmt`, 32 entries, byte-identical to I2): `00 01 02 ff 04 ff 06 07 08 09 0a ff 0c ff 0e 0f
10 13 12 11 14 15 16 ff ff ff ff ff ff 1d 1f 1e`; 0xff → `GE_FMT_GENERIC`.

**`GE_GetFmtCaps`** (Ghidra stops on the destination switch; read from the disassembly at `.text` 0x362fc). Format numbers
are `E_MS_FMT_*` of the I2 `MsTypes.h`. M6 adds **RGBA8888 (0x1d) and BGRA8888 (0x1e)** to the 4-byte class on both
sides (I2: ARGB8888 0xf and ABGR8888 0x1f only); every other class and alignment value equals I2:

| side | formats | base / pitch / stretch align | tile base | tile width | tile height |
|---|---|---|---|---|---|
| source | I1, I2, I4, I8 | 1 / 1 / 1 | 0x80 | `{8,4,2,0,1}[fmt]` | 16 |
| source | 16-bit (6–0xa, 0xe, 0x10, 0x11) | 2 / 2 / 2 | 0x80 | 16 | 16 |
| source | 32-bit (0xf, 0x1f, 0x1d, 0x1e) | 4 / 4 / 4 | 0x80 | 8 | 16 |
| destination | I8 | 1 / 1 / 1 | 8 | 1 | — |
| destination | 16-bit (6–0xa, 0xc, 0xe, 0x10, 0x11) | 2 / 2 / 2 | 8 | 16 | 16 |
| destination | 32-bit (0xf, 0x1f, 0x1d, 0x1e) | 4 / 4 / 4 | 8 | 8 | 16 |
| either | anything else | 4 / 4 / 4, `fmt = 0xffff` | 0 | 0 | 0 |

The "anything else" case returns `E_GE_FAIL_FORMAT` (6). `GE_NonOnePixelModeCaps` is the I2 code.

**`g_GeChipPro`** (0x12 bytes, `.data`, byte-identical to I2): `WordUnit` 0x10, `bSupportFourePixelMode` 1,
`bFourPixelModeStable` 1, `bSupportMultiPixel` 0, `bSupportSpiltMode` 1, `bSupportTLBMode` 0,
`bSupportTwoSourceBitbltMode` 0, `BltDownScaleCaps` = range max 1, range min 0x20, continuous range min 1, full range 1,
shift max 0, shift min 5, shift continuous min 0; `MIUSupportMaxNUM` 2. The scaling coefficient that follows it: §4.9.
`MDrv_GE_Chip_Proprity_Init` (pointer to the table) and `MDrv_GE_SetOnePixelMode` are the I2 code.

### 4.12 API layer and dispatchers [R]
<!-- ev:gfx-ge.4.12 -->
The apiGFX.c / mapiGFX.c bodies are the I2 bodies with the systematic changes below; no function has a different
algorithm except the ones listed in this section.
- **Utopia removed from the call path** (all 76 API functions that issue a command). I2: `memset(args); args.e…Type =
  E_…; UtopiaIoctl(pGEInstance, MAPI_CMD_GFX_…, &args)`. M6: the argument block `{u32 0, u32 sub-command, void *info,
  u32 size}` is filled on the stack (no `memset`) and the matching `Ioctl_GFX_*` of mapiGFX.c is called directly with the
  instance from `GFXFireInfo + 0xac`. For all 76 the M6 target is the function the I2 command number dispatches to through
  `GFXIoctl` [R].
- **Results.** `Ioctl_* == 0` → `GFX_SUCCESS` (1), else print `%s fail [LINE:%d]` and `GFX_FAIL` (0) — or the I2
  per-function value where the source returns success anyway (e.g. `SetAlpha_ARGB1555`, `BeginDraw`).
  `APIGFX_CHECK_INIT` → `should call MApi_GFX_Init first`, **0x104 = `GFX_INIT_FAIL`**; `APIGFX_CHECK_NULL` →
  `Input Parameter is NULL, please to check!`, also 0x104 (the I2 macro returns `GFX_INIT_FAIL` too);
  `MApi_GFX_RegisterGetFontCB` / `RegisterGetBMPCB` with NULL → 4 = `GFX_INVALID_PARAMETERS`.
  `MapRet` (0x4002f2f0, GFX result → Utopia status, called on the result of every `Ioctl_GFX_*`): 1 → 0, 2…5 → 2,
  anything else → 1 (5-byte table `00 02 02 02 02`).
- **Callbacks.** `MApi_GFX_RegisterGetFontCB` / `RegisterGetBMPCB` are built as the I2 `GFX_UTOPIA2K` variant: the
  function pointer is stored in the API globals (`GFXFireInfo + 0xc0` for the font callback) and no `MAPI_CMD_GFX_CB`
  is issued. `Ioctl_GFX_CB` itself (sub 1 → private +0xc, sub 2 → private +8) still exists but nothing calls it — unused.
- **`GFXIoctl`** (the Utopia dispatcher, registered by `GFXRegisterToUtopia` but unreachable because the Utopia core is
  stubbed, §4.2) — unused: I2 commands 0–0x12 in this order — 0 INIT, 1 GET_INFO, 2 LINEDRAW, 3 RECTFILL, 4 BITBLT,
  5 SET_ABL, 6 SET_CONFIG, 7 GET_CONFIG, 8 BITMAP, 9 FONT, 0xa POWERSTATE, 0xb CB, 0xc MISC, 0xd TWOSRCBITBLT, 0xe TLB,
  0xf GET_CAPS, 0x10 DRAW_OVAL, 0x11 TRIFILL, 0x12 SPANFILL; 0x13 → default; M6 adds 0x14 EnableInterrupt,
  0x15 DisableInterrupt, 0x16 ClearInterrupt, 0x17 TagIsFinish, 0x18 SupportInterrupt [R: binary case numbers; names from
  the I2 case of the same callee].
- **`Ioctl_GFX_MISC`** returns 0 for `E_MISC_EXIT` (I2 returns `GFX_SUCCESS` = 1 there [I: which the I2 API then reports
  as a failure]). M6 adds sub-commands 0xf–0x12 (alpha threshold get / set, scaling coefficient get / set, §4.9).

**Sub-command numbers** (the enum header `apiGFX_v2.h` is absent; numbers are the binary's case labels, names the I2 case
with the same body; the I2 enum order equals the case order in every switch):

| ioctl | numbers → name |
|---|---|
| `Ioctl_GFX_MISC` | 1 EXIT (before the switch), 2 CLEAR_FRAME_BY_WORLD, 3 CLEAR_FRAME, 4 LINE_PATTERN_RESET, 5 LINE_PATTERN, 6 BEGINE_DRAW, 7 END_DRAW, 8 DEBUG_LEVEL, 9 SET_HK, 0xa GET_HK, 0xb PATCH_MODE (stores the byte at private +0x12), 0xc STR_BLT_SCK_TYPE, 0xd SET_ALB_1555, 0xe GET_ALB_1555, M6: 0xf SET_ALPHA_THR_1555, 0x10 GET_ALPHA_THR_1555, 0x11 SET_SCALE_COEFF, 0x12 GET_SCALE_COEFF (names of the M6 ones [I]) |
| `Ioctl_GFX_SetConfig` | 0 POWERON, 1 FLUSHQUEUE, 2 TAGID, 3 NEXTTAGID, 4 WAITFORTAGID, 5 ONEPIXELMODE, 6 DITHER, 7 VQ (§4.5), 8 MIRROR (private +0x13 / +0x14), 9 DST_MIRROR (+0x15 / +0x16), 0xa ROTATE (+0x10), 0xb ITALIC, 0xc NEARESTMODE (+0x11), 0xd PALETTEOPT, 0xe CLIP, 0xf WRITEPROTECT, 0x10 ALPHA_COMPARE, 0x11 CSC, 0x12 BURSTMIULEN |
| `Ioctl_GFX_GetInfo` | 0 BUFFERINFO, 1 TAGID, 2 NEXTTAGID, 3 CLIP, 4 INTENSITY, 5 DBGINFO |
| `Ioctl_GFX_SetABL` | 0 ABL, 1 COLORKEY, 2 ROP, 3 DFB, 4 INTENSITY |
| `Ioctl_GFX_TLB` | 0 SET_MODE (ctx +0x38), 1 SET_ADDR (ctx +0x40 / +0x48), 2 FLUSH_TABLE (byte ctx +0x50), 3 TAG (`MDrv_GE_SetTLBTag` → HAL stub 5) |
| `MApi_GFX_GetGECaps_U02` | 0 MULTI_PIXEL, 1 BLT_DOWN_SCALE, 2 DFB_BLENDING, 3 ALIGN, 4 TLB |
| `Ioctl_GFX_Font` (from the callers) | 1 LOADTEXTOUT (`MApi_GFX_TextOut`, size 0x114), 2 CHARACTEROUT (size 0xc) |

Argument sizes the API passes (the `u32Size` field; `CheckSize` is compiled empty): GET_HK 1, EXIT 0x10,
SET / GET_ALB_1555 1, ONEPIXELMODE 1, ITALIC 4, PALETTEOPT 8, TAGID 4, WAITFORTAGID 4, VQ 0xf, FLUSHQUEUE 4, POWERON 1,
WRITEPROTECT 0x15, SetABL INTENSITY 0xc, ROP 5, GetInfo INTENSITY 8, TAGID 2, DBGINFO 0x124, GetCaps 0xc,
CLEAR_FRAME(_BY_WORLD) 0x10 [R: apicheck].

- **Removed or merged** (no symbol, no body): `GFX_FireInfoLock` / `UnLock`, `_GFX_RegisterAllCBFunc`,
  `MApi_GFX_GetLibVer`, `msAPI_GE_ClearFrameBufferByWord`, the legacy wrappers `MDrv_GE_PE_SetIntensity`,
  `MDrv_GE_SetAlpha`, `MDrv_GE_SetNearestMode`, `MDrv_GE_SetPaletteOpt`, `MDrv_GE_SetPatchMode` of apiGFX.c, `GFXMdbIoctl`
  (the MDB debug ioctl, not in the preprocessed I2 text either), and the whole `utopia_adaption/gfx/GFX_adp.c` (user-space
  ioctl adaption). Inlined into their callers: `MApi_GFX_MapACmp`, `MapDFBBldFlag`, `MapDFBBldOP`, `MapStrSCKType`,
  `MapVCMQSZ`, `MapYUVOp`, `GFX_CheckInClipWindow`, `GFX_TextOutEx`.
- **Kept as `#if 0` stubs of I2** that return `GFX_SUCCESS` without doing anything: `MApi_GFX_TrapezoidFill`,
  `MApi_GFX_EnableTrapezoidAA`, `MApi_GFX_EnableTrapSubPixCorr`, `MApi_GFX_SetDFBBldSrcColorMask`, `MApi_GFX_GetStatus`
  (`*pStatus = TRUE`). So the driver's trapezoid fill is not reachable from the API; the trapezoid blit is, through the
  `E_GE_FLAG_TRAPEZOID_X/Y` BitBlit flags (`MDrv_GE_BitBltEX` → `MDrv_GE_BitBltEX_Trape`, `CONFIG_GFX_TRAPZOID`
  defined).
- `MApi_GFX_QueryDFBBldCaps` / `GetAlignCaps` are `MApi_GFX_GetGECaps(2 / 3, …)` (sizes 2 / 0xe);
  `MDrv_GE_EnableAlphaBlending` / `MDrv_GE_SetAlphaBlending` (legacy names inside apiGFX.c) are the init check plus the
  `MApi_GFX_*` call.
- `GFX_ConvertRGB2PCFmt` and `MApi_GFX_DrawLine_U02` call the unnamed colour → register-halves helper `GE_ColorExpand`
  (§4.14) where I2 inlines the per-format switch (same arithmetic [R: constants]). `MApi_GFX_TriFill_U02` /
  `SpanFill_U02` / `TextOut_U02` / `ClearFrameBuffer*_U02` differ only by the expanded DDA, 64-bit division
  (`__do_div64`) and inlined helpers [R: callees and constants].

### 4.13 The 33 mhal calls mi_gfx makes [R]
<!-- ev:gfx-ge.4.13 -->
"Record" = stored in `GFXFireInfo` with a dirty bit, applied by the next draw (§4.3). "MISC n" / "SetConfig n" = the
sub-command number the API passes to `Ioctl_GFX_MISC` / `Ioctl_GFX_SetConfig`. Registers: hex numbers of bank 0x1409;
for the draw calls the set is the transitive closure over the block's call graph — an upper bound (it includes branches
for other formats, TLB, patterns). Every API returns 0x104 before `MApi_GFX_Init`.

| call (mi_gfx use) | path | hardware effect |
|---|---|---|
| `MApi_GFX_Init(&cfg)` (Open) | §4.4 | clock on; regs 0, 1, 3, 0x11, 0x1e, 0x28 / 0x29 / 0x2a, 0x2f, 0x34, 0x59, 0x62 (§4.8 `GE_Init`) |
| `MApi_GFX_SetVCmdBuffer(u64 addr, code)` (Open) | stores addr / code in the shared VQ record (`GFXFireInfo + 0xb1` / `+0xb9`), SetConfig 7 with the whole record (§4.5) | CFG bit 1 per the record's enable, VCMDQ base / size (0x28, 0x29, 0x2a), TH thresholds |
| `MApi_GFX_EnableVCmdQueue(b)` (Open / Close) | stores b in the record (`GFXFireInfo + 0xb0`), SetConfig 7 (§4.5) | the same set |
| `MApi_GFX_EnableAlphaBlending(b)`, `SetDFBBldOP(src, dst)`, `EnableDFBBlending(b)`, `SetDFBBldFlags(f)`, `SetDFBBldConstColor(argb)` | record (DFB / alpha bits) | at the draw: BLEND 0x11, ALPHA 0x12, ALPHA_CONST 0x13, DFB op / flags / const 0x2a–0x2c |
| `MApi_GFX_SetSrcColorKey(en, op, fmt, &from, &to)`, `SetDstColorKey(…)` | record | at the draw: 0x14–0x1b, OP_MODE 0x1c, EN (§4.10) |
| `MApi_GFX_SetClip(&v0, &v1)` | record | at the draw: CLIP 0x55–0x58 |
| `MApi_GFX_SetSrcBufferInfo(&info, 0)`, `SetDstBufferInfo(&info, 0)` | record | at the draw: base 0x20 / 0x21, 0x26 / 0x27, pitch 0x30 / 0x33, FMT 0x34 |
| `MApi_GFX_SetMirror(x, y)`, `SetRotate(angle)` | record | at the draw: 0x59 and the direction bits of CMD |
| `MApi_GFX_BeginDraw()` / `EndDraw()` | MISC 6 / 7 → `GE_BeginDraw` / `GE_EndDraw` (§4.3) | none (mutex, context restore) |
| `MApi_GFX_RectFill(&info)` | `Ioctl_GFX_RectFill` | W 0, 1, 0x11–0x1c, 0x1f, 0x20 / 21, 0x26 / 27, 0x2a–0x2f, 0x30, 0x33, 0x34, 0x55–0x59, 0x5c / 5d, 0x60, 0x68–0x6b, 0x6e / 6f, 0x70–0x7f |
| `MApi_GFX_DrawLine(&info)` | `Ioctl_GFX_LineDraw` | the same base set + 0x61, 0x63 (line delta, length), 0x68–0x6b, colours / gradients 0x70–0x7e |
| `MApi_GFX_BitBlt(&rect, drawFlag)` | `Ioctl_GFX_BitBlt` | the same base set + 0x5e / 5f (scale init), 0x64 / 65 (scale ratio), 0x68–0x6f; stretch path §4.9 |
| `MApi_GFX_SetPaletteOpt(argb*, start, end)` | SetConfig PALETTEOPT (a NULL pointer → returns 0); the sub-command number disagrees between two readings — 0xd by the case labels of `Ioctl_GFX_SetConfig` (§4.12), 8 by the reading of the mi_gfx path | CLUT 0x2d / 0x2e / 0x2f per entry (`GE_SetPalette`) |
| `MApi_GFX_SetNextTAGID()` → tag | SetConfig 3 → `MDrv_GE_SetNextTAGID` | TAG 0x32 ×4 (§4.7) |
| `MApi_GFX_WaitForTAGID(tag)` | SetConfig 4 → `MDrv_GE_WaitTAGID` | **no wait on M6**: 0x1e, 0x24 / 0x25 (§4.7) |
| `MApi_GFX_SetAlpha_ARGB1555(a)` | MISC 0xd (source function) | the alpha value used for ARGB1555 sources [I: reg 0x12 high byte, per source] |
| `MApi_GFX_SetAlphaThr_ARGB1555(t)` | MISC 0xf | ALPHA 0x12 low byte |
| `MApi_GFX_SetScallingCoeff(c)` | MISC 0x11 | none (global, §4.9); prints `%s fail` on every call, returns 1 |
| `MApi_GE_SupportInterrupt()` | → 1 | none |
| `MAPI_GE_GetIrqNum(&irq)` | `GE_GetIsrNum` | none; returns 1 |
| `MApi_GE_EnableInterrupt()` / `DisableInterrupt()` / `ClearInterrupt()` | `GE_Enable_INT` / `GE_Disable_INT` / `GE_Clear_INT` | reg 0x1e (§4.7) |
| `MApi_GE_TagIsFinish(tag)` → bool | `GE_Tag_Finish` | reads TAG 0x32, STAT 7 |
| `MApi_GE_Exit()` | MISC 1 → `MDrv_GE_Exit` | clock off (§4.4) |

Which registers each recorded setter ends up in (rows 4–8) is taken from the I2 source of the setters [I]; the paths,
sub-command numbers and the draw-call register sets are [R]. mi_gfx's MI-level operations therefore reach the hardware
only in `RectFill` / `DrawLine` / `BitBlt` / `SetPaletteOpt` / alpha / VCMDQ / interrupt calls; the rest is bookkeeping
inside the API layer.

### 4.14 Unnamed and no-source functions [R]
The 21 functions without symbols, identified by `__func__` strings, callers, shape and — where marked "compiled" — by
identical callee and constant sets against I2 bodies built with `-fno-inline`:

| function | identified as | evidence |
|---|---|---|
| 0x4002e7c0 | tail of `MApi_GFX_GetInfo` (its error print) | `__func__` string |
| 0x4002f2f0 | `MapRet` (GFX result → Utopia status, §4.12) | called on the result of every `Ioctl_GFX_*` |
| 0x4002f304 | `MApi_GFX_MapBLDCOEF` (16-entry permutation: 0–3 → 0–3, 4–7 → 8–0xb, 8–0xb → 4–7, 0xc → 0xf, 0xd–0xf → 0xc–0xe) | first map in the ABL branch of `GFX_SetFireInfo`, as in the source |
| 0x4002f366 | `MApi_GFX_MapABLCOEF` and `MApi_GFX_MapROP2` (identity 0–0xf) | second map of the ABL branch; `MApi_GFX_SetROP2_U02` calls the same function; the two I2 maps are identical identity maps [I: merged by gcc's identical-code folding] |
| 0x4002f3c8 | `MApi_GFX_MapCKOP` (identity 0–5; the I2 source has 4 ops, M6 6, §4.10) | called by `MApi_GFX_SetSrc/DstColorKey_U02` |
| 0x4002f3f8 | `GFX_RectBltFlags` — draw-flag builder from the context flags: italic 0x400, source mirror H / V 0x1000 / 0x2000, destination mirror H / V 0x4000 / 0x8000, rotation, nearest; the flag values in the code are 0x400, 0x1000, 0x2000, 0x4000, 0x10000, 0x100000 [I: rotation and nearest are the last two] | the I2 function's fields and order; called by the BitBlt / TwoSourceBitBlt / DrawBitmap / TextOut / CharacterOut `_U02` functions |
| 0x4002f45e / `_API_GE_RETURN` | `_API_GE_ENTRY` / `_API_GE_RETURN` | `__func__` strings |
| 0x4002f564 | colour → two 16-bit register halves per GE format (I8 index doubled; 2266 / 12355; RGB565, ARGB1555 / RGBA5551 (alpha → 0xff00), ARGB4444 / RGBA4444 nibble expansion; others pass through) [I: a static helper; I2 inlines it] | callers `MApi_GFX_DrawLine_U02`, `GFX_ConvertRGB2PCFmt` |
| 0x400311c6 | `GFX_SetFireInfo` | the same 15-bit order as the source (§4.3) |
| 0x40032558 | `GE_FastClip` — 1 if the primitive lies wholly outside the context clip (ctx +0x674…+0x67a); the draw is then skipped | compiled; callers `MDrv_GE_DrawLine`, `_FillRect`, `_BitBltEX` |
| 0x4003259c | `GE_Restore_PaletteContext` with `GE_SetPalette` inlined — if the palette is dirty or owned by another context: 256 × (CLUT_H 0x2e, CLUT_L 0x2d, CLUT_CTRL 0x2f = index \| 0x100) | compiled; register pattern |
| 0x4003262e / `GE_BeginDraw` | `GE_EndDraw` / `GE_BeginDraw` | `__func__` strings (§4.3) |
| 0x40032776 | `GE_CalcColorDelta` (`GE_Divide2Fixed(…, 7, 12)`, alpha `(…, 4, 11)`) | compiled |
| 0x40032882 | `GE_NormalizeTrapezoidDef` | compiled; callers `MDrv_GE_FillTrapezoid`, `_BitBltEX_Trape` |
| 0x4003388c / `GE_TrapezoidFill_part` (0x4003466c) | `GE_TrapezoidFill` with `GE_TrapezoidFillEx` inlined / `GE_TrapezoidBlit` with `GE_TrapezoidBlitEx` inlined (they differ from the source only in fixed-point constants written as shifts) | compiled; they call `MDrv_GE_FillRect` / `MDrv_GE_BitBltEX` |
| 0x400359dc / `GE_DumpReg` | `GE_Reset` / `GE_DumpReg` | register pattern, strings (§4.7) |
| 0x40035f88 | interrupt control (no source) | §4.7 |

The 81 named functions without an I2 source: the interrupt API (`MApi_GE_EnableInterrupt`, `DisableInterrupt`,
`ClearInterrupt`, `TagIsFinish`, `SupportInterrupt`, `MAPI_GE_GetIrqNum` and the six `Ioctl_GFX_*` behind them),
`MDrv_GE_*Interrupt` / `TagFinish` / `SupportInterrupt`, `GE_Enable_INT` / `GE_Disable_INT` / `GE_Clear_INT`,
`GE_Tag_Finish`, `GE_GetIsrNum` / `GE_SetIsrNum` (§4.2, §4.7); the alpha threshold and the scaling coefficient
(`MDrv_GE_SetAlphaThr_ARGB1555`, `MDrv_SetBltScaleCoeff`, `GE_SetBltScaleCoeff`, §4.9); `DrvGeOs*`, `DrvDeOsSetClk`, the
module init, sysfs, the Utopia stubs, `HAL_GE_exit`, `DrvGeModuleSuspend` (§4.2).

### 4.15 The rest of the block — unused on this camera [R]
Everything not on mi_gfx's path is the stock Utopia GFX feature set that no module calls — unused: fonts, bitmaps and
text (`MApi_GFX_TextOut`, `CharacterOut`, `DrawBitmap`, font and bitmap handles, `Ioctl_GFX_Font`, `_Bitmap`, `_CB`),
trapezoid fill and blit, triangle and span fill, oval, two-source BitBlt, frame-buffer clears, line patterns, ROP2,
alpha compare, YUV / CSC (`SetDC_CSC_FMT`, `YUV_MODE`), stretch colour-key type, italic, dither, nearest mode, tile modes,
the TLB / MMU interface (`MDrv_GE_SetTLB*`, `Ioctl_GFX_TLB`), MIU protect, burst length, one-pixel mode, capability
queries (`GE_GetFmtCaps`, `GE_NonOnePixelModeCaps`, `Ioctl_GFX_GetCaps` / `GetInfo`), STR suspend / resume (`GFXStr`,
`GE_Restore_HAL_Context`), debug (`MApi_GFX_SetDbgLevel`, `GetStatus`). For these the I2 files are the specification,
with every M6 difference of this section and of §4.1's list applying to them too (colour-key ops and polarity, the
non-waiting `GE_WaitTAGID`, direct register writes, the absent API mutex, …).

**HAL stubs** (stubs in I2 too): the functions returning `E_GE_NOT_SUPPORT` (5) — TLB (`GE_SetTLBMode`,
`GE_GetTLBSRC/DSTADDR`, `GE_SetTLBSrc/DstBaseAddr`, `GE_FlushTLBTable`, `GE_SetTLBTag`, `GE_StopFlushTLB`),
`GE_SetSrcTile` / `GE_SetDstTile`, the trapezoid HAL (`GE_FillTrapezoid`, `GE_BitBltEX_Trape`, `GE_EnableTrapezoidAA`,
`GE_EnableTrapSubPixCorr`), `GEWD_ReadReg` / `GEWD_WriteReg`. So this GE has **no TLB / MMU, no tile mode and no
hardware trapezoid**; the driver's trapezoid paths (`MDrv_GE_FillTrapezoid`, `MDrv_GE_BitBltEX_Trape`) are software
splits into rectangles and blits (§4.14 `GE_TrapezoidFill` / `Blit`).

**HAL functions that are the I2 code** although the header-less constant sets differed (compiler forms only):
`GE_Restore_HAL_Context`, `GE_SetRotate`, `GE_SetBlend`, `GE_SetAlpha`, `GE_SetDFBBldFlags` (it also keeps the I2 dead
code: it computes the DFB_BLD_OP source-colour-mask value and never writes it — the binary reads reg 0x2a and drops the
result), `GE_WriteProtect`, `HAL_GE_SetBurstMiuLen`; the rest of the HAL functions named in §4.6–§4.11 are marked there.
Of the 72 HAL functions (70 + `GE_Reset` / `GE_DumpReg`), 46 have identical callee and constant sets; every function of
the I2 `halGE.c` exists in the block.

**Driver: STR (suspend to RAM).** `MDrv_GESTR_WaitIdle` → return 0, `MDrv_GESTR_SetPalette` → return 0,
`MDrv_GE_STR_SetPowerState` → return 1 — all three empty on M6 (I2: register save / restore through
`UtopiaModuleGetSTRPrivate`, palette reload, clock register at `CHIP_GE_CLK`). `MDrv_GESTR_DumpReg` is absent.
`MDrv_GE_SetPowerState` (the per-context path) is I2 with `GE_VQADDR_LOCK_PATCH`: suspend saves CFG, the VCMDQ base,
YUV_MODE and the VCMDQ size into the context (+0x6b0…+0x6bc); resume writes CFG, the VCMDQ base 5 times, YUV_MODE, the
size, then `GE_Init` with a zeroed config, `GE_InitCtxHalPalette`, `GE_SetActiveCtrlMiu1` (unconditional).

**Driver: buffer setters without TLB.** I2 `MDrv_GE_SetSrcBuffer` / `_SetDstBuffer` translate the address through the
TLB table (`MsOS_MPool_Mapping`, `MsOS_MPool_PA2KSEG1`, `GE_Get_MIU_INTERVAL`) when `bSupportTLBMode` and the TLB mode
say so; the M6 bodies have only the plain path (MIU id, `GE_ConvertAPIAddr2HAL`, YUV_MODE MIU-high bit 13 / 14, base,
pitch, tile stub). With `bSupportTLBMode = 0` (§4.11) the I2 branch was dead anyway, so the behaviour is unchanged.

**Driver functions that are the I2 code** after reading: `MDrv_GE_FillRect` (built with `GE_SPLIT_MODE_PATCH`: CFG bit 14
cleared when `bSupportSpiltMode`; the four-pixel-mode source w / h and FMT copy when `bFourPixelModeStable`),
`MDrv_GE_DrawLine` (line-last patch: V1 moved by 2, length + 1, one primitive per width step), `MDrv_GE_BitBltEX` (sets
CFG bit 14 when `bSupportSpiltMode`, one-pixel-mode switch around the primitive, dither off when the source and
destination formats match or are the ARGB1555 / ARGB1555_DST pair, CMD = flags | 0x40), `MDrv_GE_DrawOval`
(`SUPPORT_LINE_LAST_PATCH` branch: LINE_STYLE 0x13f then 0x3f), `MDrv_GE_BitBltByTwoSourceBuffer` (it drives the GEWD
stubs, so it does nothing useful on M6), `MDrv_GE_FillTrapezoid`, `MDrv_GE_BitBltEX_Trape` (software split;
`GE_FastClipTrapezoid` and `GE_TrapezoidOverlap` inlined), `MDrv_GE_SetIntensity` / `GetIntensity`, `SetAlphaTest`,
`Set` / `GetPalette`, `Set` / `GetClipWindow`, `SetYUVMode`, `SetStrBltSckType`, `SetOnePixelMode`,
`Chip_Proprity_Init`, `GetInfo`, `Get` / `Free_Resource`. Of the 90 driver functions compared (87 named + 3 unnamed),
55 have identical callee and constant sets. Driver logging: I2 `ULOGE` / `ULOGD` / `ULOGI` / `ULOGF` / `MS_DEBUG_MSG` →
`CamOsPrintf("[Utopia][GE DRV]: …")` with the same texts; the `__LINE__` values differ (the M6 file is longer).
