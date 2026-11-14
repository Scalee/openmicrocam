# rgn-gop — mi_rgn, libmi_rgn, the mhal RGN block (GOP, OSD blender, cover) and fbdev.ko
<!-- ev:rgn-gop -->

The overlay side of the media stack. The hardware is the **GOP** (graphics output plane: a layer engine that fetches up to
eight windows, "GWINs", from memory and lays them over a video path), the **OSD blender** of each scaler output that mixes
a GOP into the video, and the **cover** engines that paint solid rectangles. Ten GOPs exist: six over the scaler (SCL),
DIVP and LDC outputs and four over the display paths. Three pieces of vendor software drive them:

- `mhal.ko` block **RGN** — the HAL: `MHAL_RGN_*` (42 exports) over a Drv cache layer and a HAL register layer.
- `mi_rgn.ko` + `libmi_rgn.so` — the MI region API (`MI_RGN_*`, `/dev/mi_rgn`): OSD regions (canvases in memory) and
  covers attached to channel-ports of SCL / LDC, composed per frame into the GOP of that output, programmed through the
  frame's CMDQ from two hooks mi_scl calls.
- `fbdev.ko` — the Linux framebuffer (`/dev/fb0` GUI, fb1 hardware cursor) on the display GOPs, through `MHAL_RGN_Gop*`
  and `MHAL_DISP_*`.

Other documents: the user-side ioctl wrapper, payload packing, return codes, error-code format and module ids —
`mi-user-abi.md` (§2 "The device fd", §3 "The ioctl call", §4 "Return values and error codes"); the kernel-side device frame
(`/dev/mi_*`, ioctl dispatch, `/proc/mi_modules/<module>/*` debug files, mi_sys exports, MMA heaps, private heaps, BDMA
operations, `logo2free`) — `mi_sys-2-interfaces.md` §1, §10, §6, §7; the mi_sys registration contract and bind capability —
`mi_sys-1-engine.md` §2, §3.3; the mi_scl side of the two RGN hooks (where it calls them, what it fills) — `mi-capture.md` §6.5 and §6.12;
mhal's block map, module parameters, init order and device-tree properties — `mhal-core.md` §2.1, §5, §3.2, §7; the CMDQ interface table and
engine ids — `mhal-scl-jpe-cmdq.md` §2.3, §2.1; RIU addressing — `kernel-platform.md` §3.1 "Static I/O map and RIU addressing"; the
`MHAL_DISP_*` calls fbdev makes and the display paths (OP0, colour engines) — `mhal-disp.md` §2.1, §12.1, `mi_disp-mi_hdmi.md` §1; the
`mmap.ini` grammar — `config_tool.md` §2.1; the file rows — `firmware-image.md` §3.3, §3.5, §3.6; the fb client — `kjd_lvgl.md` §8.

Tags: **[R]** read from the binary, a decompile, disassembly or a file; **[S]** stated by a source; **[I]** inference. A heading
tagged [R] means every statement under it is [R] unless marked otherwise. Struct and field names the binaries do not carry
are SigmaStar-style names and count as [I]; offsets, sizes, constants and control flow are [R].

Binaries (dump of 2026-10-16): `/config/lib/libmi_rgn.so` (9.5 KB); `/config/modules/4.9.227/mi_rgn.ko` (131 KB file, 50
KB code, 96 functions, 9 exports); `…/modules/4.9.227/mhal.ko` block RGN (`.text` 0x0a6800–0x0aaeb0);
`…/modules/4.9.227/fbdev.ko` (vermagic `4.9.227 SMP preempt mod_unload ARMv7 thumb2 p2v8`). All four are the MI build of
2022-07-21 (`project_commit.7251436 sdk_commit.80327e1`).

Readings on the running camera are tagged [R live, date]; where one contradicts an earlier static reading, the live
reading is the fact. Addresses: mi_rgn = `.text` offset + 0x10028; the mhal RGN block = `.text` offset + 0x40000028.
Reference sources: OpenIPC u-boot `drivers/mstar/rgn/{drv,hal/mercury6}` (mhal RGN, same chip, older revision);
jockyw2001/sdk `misc/fbdev/mstar_fbdev/` (fbdev, 2018); the SSD20x user-space fb header `sstarFb.h`
(GuoFlying/ssd2xx-demo). No public source and no M6 header exist for mi_rgn / libmi_rgn (searched) [R]; SigmaStar's RGN API
document describes a newer API revision (extra `u16SocId`, `bInputPort`, FRAME / LINE types) and is the source of names
only.

## 1. The component on this camera [R]
<!-- ev:rgn-gop.1 -->

- **mi_rgn / libmi_rgn: loaded, never initialised.** `mi_rgn.ko` is loaded only as a symbol dependency of `mi_scl.ko`.
  Nobody opens `/dev/mi_rgn` and nobody links `libmi_rgn.so` (prog_uvc has no RGN call; kjd_lvgl calls no MI function); no
  module imports `mi_rgn_Init`. mi_scl calls `mi_rgn_Process` and `mi_rgn_NotifyFrameDone` for every frame and both return 0
  at once, because the driver instance is never obtained (§3.5). The hooks become live as soon as any process runs
  `MI_RGN_Init` (§3.4); from then on they program the SCL output's GOP through the frame's CMDQ.
- **fbdev: used.** kjd_lvgl draws its GUI into `/dev/fb0`: ARGB8888 1600×900 on MHAL GOP type 9 (DISP0_UI = HAL GOP 7,
  banks 0x11A4–0x11A6), stretched to the output timing (1920×1080 or 3840×2160) and kept full-screen by the timing follow
  (§5.4, §5.5). It uses only `FBIOBLANK`, `FBIOGET_FSCREENINFO`, `FBIOGET_VSCREENINFO` and `mmap` — none of fbdev's own
  ioctls. prog_uvc does not open a framebuffer. fbdev's GOP writes go straight to RIU (no CMDQ is ever installed for the
  display GOPs) and are latched by the GOP double buffer (§4.3). The fb1 hardware cursor, fbdev's debug commands and
  `GUI_SETALPHA_ARGB1555` are unused (§5.7, §5.9).
- **mhal RGN: the GOP part is used** (fbdev's 15 `MHAL_RGN_*` calls, §4.6); the OSD blender, cover and CMDQ machinery
  run only for mi_rgn and so stay idle.

What the scaler-port overlay hardware does, as probed live [R: live 2026-10-26/27]: up to 8 windows per GOP and frame, each scanned from its own buffer at its own position,
shown or hidden, with per-pixel alpha or one constant alpha for the whole window; up to 4 solid cover rectangles per port
under the whole overlay, without memory; no layer stretch on the scaler GOPs; the constant alpha replaces the pixel alpha;
windows do not blend with each other. One pixel format per GOP (§4.4). The overlay is part of the SCL output, so whatever
consumes that port carries it: regions on the port bound into the H.264 / MJPEG encoders show in the USB stream and in
recorded MP4 files, and a region on the upscaler's output (SCL dev 1 chn 0 port 0) shows sharp at output pixels.

## 2. libmi_rgn.so — the user library [R]

### 2.1 Frame
- `NEEDED libc.so.6` only; imports `MI_SYS_Mmap`, `MI_SYS_Munmap`, `MI_SYS_FlushInvCache` and the shared fd table
  `g_mi_fd_table` / `g_last_fd_index` / `g_mi_fd_semalock` (all from libmi_sys), `CamOsTsem*` (libcam_os_wrapper). Version
  string `project_commit.7251436 sdk_commit.80327e1 build_time.20220721223320`.
- Device open (static helper at 0xb34, the common pattern of `mi-user-abi.md` §2.1): a per-process cached fd (`fd`, `pid`);
  if fd is −1 or the pid changed, open `/dev/mi_rgn` `O_RDWR`; if the new fd is ≤ the last entry of `g_mi_fd_table` it is
  duplicated above it with `fcntl(F_DUPFD, table[last])` and the original closed; appended to the table (assert ≤ 0x22
  entries, `rgn_api.c:0x23`); `FD_CLOEXEC` set. A second string `/dev/mi/rgn` exists but appears only in the error print.
- Every call: the 16-byte wrapper `{u32 len; u32 pad; u64 ptr}` → `ioctl(fd, cmd, &wrapper)` (`mi-user-abi.md` §3.1); `-1`
  from ioctl prints `[%s] failed to ioctl 0x%08lx!(%s)` and returns −1 (Init / InitDev return the −1 too); otherwise the MI
  code from the kernel is returned. **The first argument of every export (`u16 SocId` in the newer API) is ignored.**
- Process-local canvas map: a `.bss` table of 1024 × `{u32 va; u32 len}` (0x2000 B, cleared by Init / InitDev), one entry
  per region handle, guarded by a Tsem created in Init.

### 2.2 Exports → ioctls (type `'i'`, nr = index into the kernel handler table of §3.2)

| export (args after the ignored socId) | cmd | payload (offset: field) | user-side logic |
|---|---|---|---|
| `MI_RGN_Init(pstPaletteTable)` | 0x40086900 `_IOW 0 8` | +0 palette pointer (u32), +4 junk (the payload is built 4 B below the wrapper and overlaps its `len`) | `CamOsTsemInit(map lock, 1)`; clear the canvas map after the ioctl |
| `MI_RGN_InitDev(pstInitParam)` | same 0x40086900 | +0 = `pstInitParam->pstPaletteTable` | opens the device first (fail → returns **0x1F**), then as Init |
| `MI_RGN_DeInit()` | 0x6901 `_IO 1` | — (arg 0) | `CamOsTsemDeinit(map lock)` |
| `MI_RGN_DeInitDev()` | 0x6901 | — | as DeInit, then closes this process's fd and removes it from `g_mi_fd_table` |
| `MI_RGN_Create(hRgn, pstAttr)` | 0x40146902 `_IOW 2 0x14` | +0 handle, +4 `MI_RGN_Attr_t` (16 B) | — |
| `MI_RGN_Destroy(hRgn)` | 0x40046903 `_IOW 3 4` | +0 handle | — (the canvas map entry is **not** unmapped) |
| `MI_RGN_GetAttr(hRgn, pstAttr)` | 0xc0146904 `_IOWR 4 0x14` | +0 handle, +4 Attr (in/out) | copies Attr back when the result is 0 |
| `MI_RGN_SetBitMap(hRgn, pstBitmap)` | 0x40146905 `_IOW 5 0x14` | +0 handle, +4 `MI_RGN_Bitmap_t` (16 B) | — (the kernel copies the pixels from user space itself) |
| `MI_RGN_AttachToChn(hRgn, pstChnPort, pstChnAttr)` | 0x403c6906 `_IOW 6 0x3c` | +0 handle, +4 `MI_RGN_ChnPort_t` (16 B), +0x14 `MI_RGN_ChnPortParam_t` (40 B) | — |
| `MI_RGN_DetachFromChn(hRgn, pstChnPort)` | 0x40146907 `_IOW 7 0x14` | +0 handle, +4 ChnPort | — |
| `MI_RGN_SetDisplayAttr(hRgn, pstChnPort, pstChnAttr)` | 0x403c6908 `_IOW 8 0x3c` | as Attach | — |
| `MI_RGN_GetDisplayAttr(hRgn, pstChnPort, pstChnAttr)` | 0xc03c6909 `_IOWR 9 0x3c` | as Attach (param zero-filled before) | copies the 40-B param back on 0 |
| `MI_RGN_GetCanvasInfo(hRgn, pstCanvasInfo)` | 0xc028690a `_IOWR 10 0x28` | +0 handle, +4 pad, +8 `MI_RGN_CanvasInfo_t` (32 B, out) | on 0: if the map entry is empty, `MI_SYS_Mmap(phy, height × stride, &va, cached=1)` and store `{va, len}`; fail → **0xA0032011**; the `virtAddr` returned is the user va |
| `MI_RGN_UpdateCanvas(hRgn)` | 0x4004690b `_IOW 11 4` | +0 handle | if mapped: `MI_SYS_FlushInvCache(va, len)`, `MI_SYS_Munmap`, clear the entry (either failing → 0xA0032011 without the ioctl); then the ioctl |
| `MI_RGN_ScaleRect(pstChnPort, pstCanvasSize, pstScreenSize)` | 0x4020690c `_IOW 12 0x20` | +0 ChnPort (16 B), +0x10 canvas size {u32 W, u32 H}, +0x18 screen size {u32 W, u32 H} | — |

`MI_RGN_ScaleRect`: after the ignored socId the export takes three pointers and copies 16 B, 8 B and 8 B from them; the
kernel hands `(ChnPort, size, size)` to `scale_rect`, which stores them as the channel-port's canvas and screen sizes
(§3.4). It is a setter, so the write-only `_IOW` is consistent. The parameter names are [I].

### 2.3 Data types of this build (sizes and offsets [R] from the kernel's use; names [I])

| type | size | layout |
|---|---|---|
| `MI_RGN_HANDLE` | 4 | u32, valid 0…0x3FF (1024 handles) |
| `MI_RGN_Type_e` | 4 | 0 OSD, 1 COVER (anything else rejected by Create) |
| `MI_RGN_PixelFormat_e` | 4 | 0 ARGB1555, 1 ARGB4444, 2 I2, 3 I4, 4 I8, 5 RGB565, 6 ARGB8888 (bits per pixel 16, 16, 2, 4, 8, 16, 32) |
| `MI_RGN_Attr_t` | 16 | +0 eType, +4 ePixelFmt, +8 u32Width, +0xC u32Height (OSD only; a COVER create stores them unchecked) |
| `MI_RGN_Bitmap_t` | 16 | +0 ePixelFormat, +4 u32Width, +8 u32Height, +0xC pData (user pointer) |
| `MI_RGN_ChnPort_t` | 16 | +0 eModId (`MI_ModuleId_e`), +4 s32DevId, +8 s32ChnId, +0xC s32PortId |
| `MI_RGN_ChnPortParam_t` | 40 | +0 bShow (u8 in a u32 slot), +4 stPoint {u32 X, u32 Y}, +0xC union (28 B) |
| — COVER part | | +0xC u32Layer, +0x10 stSize {u32 W, u32 H}, +0x18 u32Color |
| — OSD part | | +0xC u32Layer, +0x10 eAlphaMode (0 PIXEL, 1 CONSTANT; > 1 rejected), +0x14 alpha para (u8 BgAlpha, u8 FgAlpha / u8 ConstantAlpha), +0x18 ColorInvert {+0 u8 bEnable, +4 eMode, +8 u16 LumaThreshold, +0xA u16 WDivNum, +0xC u16 HDivNum} |
| `MI_RGN_CanvasInfo_t` | 32 | +0 u64 phyAddr, +8 u32 virtAddr, +0xC u32 Width, +0x10 u32 Height, +0x14 u32 Stride, +0x18 ePixelFmt, +0x1C pad |
| `MI_RGN_PaletteTable_t` | 1024 | 256 × {u8 Alpha, u8 Red, u8 Green, u8 Blue} (the kernel copies exactly 0x400 B) |

The `CanvasInfo` layout is fixed by three readers [R]: the kernel copies region words 10…17 out (§3.3), `SetBitMap` clips
width / height against words 13 / 14 and uses word 15 as the stride, and the library maps `Height × Stride` from payload
words +0x10 / +0x14.

The ColorInvert meaning of the 16 B at OSD +0x18 is [I] from the checker's name `_MI_RGN_IMPL_CheckOsdColorInvAttr`, the
dump headers (`BColorInv`, `WDiv`, `HDiv`) and the info print `attachChn: WDiv=%d HDiv=%d` [R]; its rules are in §3.4.

## 3. mi_rgn.ko — kernel module

### 3.1 Build, load, exports, function names [R]
<!-- ev:rgn-gop.3.1 -->
- Build `project_commit.7251436 sdk_commit.80327e1 build_time.20220721223316`, with a SigmaStar developer's e-mail as author,
  license GPL v2, `depends=mi_common`; source path in a string `alkaid/sdk/interface/src/rgn/../../../impl/rgn/mi_rgn_drv.c`;
  three compilation units: `mi_rgn_ioctl` / device frame, `mi_rgn_impl.c`, `mi_rgn_drv.c`.
- Imports: CamOs (Tsem, atomics, threads, mem, strtoul, copy-from-user), MI_DEVICE / MI_COMMON, mi_sys (`RegisterDev`,
  `RegistCommand`, `UnRegisterDev`, `PrivateDevChnHeapAlloc/Free`, `MMA_Alloc/Free`, `Vmap/UnVmap`, `VFlushInvCache`,
  `MemsetPa`, `BufFillPa`, `BufBlitPa`, `GetCurPts`), 24 `MHAL_RGN_*` (the rows marked mi_rgn in §4.6), kernel
  `filp_open` / `vfs_write` / `filp_close` (dumps to file), wait queues, procfs.
- `init_module`: `/proc/mi_modules/mi_rgn/{debug_level, debug_file, debug_func, module_version_file}` (mode 0640, the
  common mi_device ops of `mi_sys-2-interfaces.md` §1.6), print `module [rgn] init`, **`mi_rgn_SetEnv()`**,
  `MI_DEVICE_Register(desc)` → `/dev/mi_rgn` with 13 ioctl handlers (§3.2), ProcessInit = `mi_rgn_process_init` (printk
  only), **ProcessExit = `mi_rgn_process_exit` → `MI_RGN_IMPL_DeInit()`**: closing `/dev/mi_rgn` in any process tears down
  every region of every process. Parameters `debug_level` (uint), `debug_file`, `debug_func` (string).
- `cleanup_module`: removes the four proc files and the directory, `MI_DEVICE_Unregister`.
- `mi_rgn_SetEnv()` (runs at insmod, exported): `CamOsTsemInit` of three binary semaphores — the region-list lock, the
  memory-accounting lock and the driver-instance lock — and `CamOsAtomicSet(0)` of the instance refcount and of
  `_gatInitDrvRefCnt`. Nothing else; no hardware is touched at load.
- 9 exports (`__ksymtab`): `mi_rgn_SetEnv`, `mi_rgn_Init`, `mi_rgn_Deinit`, `mi_rgn_Process`, `mi_rgn_NotifyFrameDone`,
  `mi_rgn_DrvGetInstance`, `mi_rgn_DrvInit`, `mi_rgn_DrvDeinit`, `mi_rgn_MemCntPrint`. Among the modules on the camera only
  mi_scl imports any (`mi_rgn_Process`, `mi_rgn_NotifyFrameDone`). The other GLOBAL but not exported symbols
  (`MI_RGN_IMPL_*`, `hSysDrvHandle`, `_gatInitDrvRefCnt`, `mi_rgn_DrvInitBufWork/DeinitBufWork`, `mi_rgn_MemAlloc/FreeCnt`)
  are internal.
- mi_rgn keeps local symbols for most functions; the 22 functions without a symbol carry names from the `__func__` strings
  their debug / BUG_ON prints reference:

| address | name | address | name |
|---|---|---|---|
| 0x106d4 | `_MI_RGN_IMPL_GetWindowListData` | 0x16560 | `_mi_rgn_cal_frontbuffer` (+ inlined `_mi_rgn_drv_calculate_pos_distance`) |
| 0x107f0 | `_MI_RGN_IMPL_CheckOsdPosAttr` | 0x16f20 | `_mi_rgn_drv_get_chport_list_head` |
| 0x109a8 | `_MI_RGN_IMPL_CheckCoverAttr` | 0x17b54 | `_mi_rgn_drv_do_clear_each_hashmap` |
| 0x11494 | `_MI_RGN_IMPL_CheckOsdColorInvAttr` | 0x17bb0 | `_mi_rgn_drv_window_wait_for_buffer_ready` |
| 0x11648 | `_MI_RGN_IMPL_CheckOsdAlphaAttr` (BUG_ON arm only) | 0x17c10 | `_mi_rgn_drv_misys_buf_del` (worker part) |
| 0x11668 / 0x116f4 | inlined `mi_rgn_MemAllocCnt` / `mi_rgn_MemFreeCnt` copies | 0x18214 | `_mi_rgn_drv_do_clear_wq_and_fb_each_hashmap` (+ `_buf_cancel_del_work`) |
| 0x13168 | `_MI_RGN_IMPL_GetDumpListHead` | 0x186b4 | `_mi_rgn_drv_pre_queue_del_work` |
| 0x15794 | `_mi_rgn_drv_swap_frontbuffer_info` | 0x18818 | `_mi_rgn_drv_buf_fill` |
| 0x16138 | `_mi_rgn_drv_init_buf` | 0x18b58 | `_mi_rgn_drv_blit_buffer` |
| 0x162f8 | `_mi_rgn_drv_caculate_frontbuffer_size` | 0x18e58 | `_mi_rgn_drv_do_draw_work` |
| | | 0x19ed4 | `_mi_rgn_drv_rebuild_frontbuffer` with inlined `_adjust_buf_opt`, `_check_frontbuffer_resize_flag`, `_create_draw_work`, `_enqueue_buf_blit_opt`, `_enqueue_buf_init_opt` |

`_mi_rgn_drv_rebuild_frontbuffer` (0x19ed4, file 0x9eac…0xb723, 6 KB) is a static function without a symbol; objdump attributes it to
`_mi_rgn_drv_window_update+…`.

### 3.2 ioctl handlers and error codes [R]
`MI_DEVICE_WrapperFunction(nr)` returns `table[nr]` for nr < 13 (table in `.data`, relocations read). Every handler locks
the single module mutex, stores it in `ctx+8` (mi_common unlocks it after the call, `mi_sys-2-interfaces.md` §1.3) and
calls (kbuf = the payload of §2.2):

| nr | handler → IMPL call |
|---|---|
| 0 | `MI_RGN_IMPL_Init(kbuf[0] = user palette pointer)` |
| 1 | `MI_RGN_IMPL_DeInit()` |
| 2 | `MI_RGN_IMPL_Create(kbuf[0], kbuf+4)` |
| 3 | `MI_RGN_IMPL_Destroy(kbuf[0])` |
| 4 | `MI_RGN_IMPL_GetAttr(kbuf[0], kbuf+4)` (output written into kbuf, copied back) |
| 5 | `MI_RGN_IMPL_SetBitMap(kbuf[0], kbuf+4)` |
| 6 | `MI_RGN_IMPL_AttachToChn(kbuf[0], kbuf+4, kbuf+0x14)` |
| 7 | `MI_RGN_IMPL_DetachFromChn(kbuf[0], kbuf+4)` |
| 8 | `MI_RGN_IMPL_SetDisplayAttr(kbuf[0], kbuf+4, kbuf+0x14)` |
| 9 | `MI_RGN_IMPL_GetDisplayAttr(kbuf[0], kbuf+4, kbuf+0x14)` |
| 10 | `MI_RGN_IMPL_GetCanvasInfo(kbuf[0], kbuf+8)` |
| 11 | `MI_RGN_IMPL_UpdateCanvas(kbuf[0])` |
| 12 | `MI_RGN_IMPL_ScaleRect(kbuf, kbuf+0x10, kbuf+0x18)` |

Error codes `0xA003_20xx` (module id 3 = `E_MI_MODULE_ID_RGN`; format: `mi-user-abi.md` §4.2): 01 INVALID_DEVID (used for a
handle ≥ 0x400 **and** for an unknown handle), 03 ILLEGAL_PARAM, 06 NULL_PTR, 08 NOT_SUPPORT, 09 NOT_PERM, 0C NOMEM,
10 SYS_NOTREADY (not initialised), 11 BADADDR, 12 BUSY (already initialised / already attached / not attached /
refcount) [R: constants; names from `mi_common_datatype.h`]. Below, "→ 03" means `0xA0032003`, and so on.

### 3.3 Object model [R: allocation sizes and field use; names I]
<!-- ev:rgn-gop.3.3 -->
- **Region** (0x50 B, `CamOsMemAlloc`, kept in one global doubly linked list, no hash): +0x08 list of attachments, +0x10
  handle, +0x14 Attr (type, fmt, w, h — the 16 B as given), +0x28 canvas `CanvasInfo` (32 B: phy u64, kernel va, w, h,
  stride, fmt — so h at +0x38, stride at +0x3c), +0x48 u8 "canvas lent to user" flag. COVER regions have no
  canvas.
- **Canvas** (OSD only): allocated by Create through the buffer manager as `rgn-bb_<handle>`. Stride =
  `_mi_rgn_drv_calpixstride(fmt, w)` = bytes per row (I2 w/4, I4 w/2, I8 w, 16-bit formats w×2, ARGB8888 w×4) rounded up to
  `MHAL_RGN_GetChipCapability(PITCH_ALIGNMENT, fmt)`; size = stride × h; `mi_sys_PrivateDevChnHeapAlloc(hSysDrv, dev 0,
  chn 0, name, size, &buf, cached = 1)`, fallback `mi_sys_MMA_Alloc(heap NULL, name, size)`; then cleared
  (`_mi_rgn_drv_init_buf`: I2 / I4 / I8 → 0, other formats → the colour-key word `0x23232323 | mask`, §3.9).
- **Attachment** (0xC-B list node in the region) → **window** (0x80 B, owned by the driver): +0x08 u8 OSD ordinal, +0x0C
  ChnPort, +0x1C ChnPortParam (40 B), +0x44 region Attr, +0x58 copy of the region CanvasInfo, +0x78 u8 "canvas dirty",
  +0x79 u8 "attr dirty".
- **Channel-port head** H (0x2F0 B, `CamOsMemAlloc`, zeroed; one per (module, dev, chn, port) that ever had a window or
  ever went through `mi_rgn_Process`). The lookup `_mi_rgn_drv_get_chport_list_head` (0x00016f20) finds
  or creates it and returns **P = H + 0x18**; all module code works with P. H+0x00 / +0x04 are the hash-bucket links of a
  256-bucket table, bucket = `(chn × 0x9E370001) >> 24` (golden-ratio hash); H+0x08…+0x14 = `{module, dev, chn, port}`
  (P−0x10…P−4). **The lookup compares module, dev and port but not chn**, so two channels of the same dev / port that fall
  into one bucket share one head [R: compares; I: consequence — a latent vendor bug].

Head layout, P-relative [R unless tagged]:

| P + | type | field (name from asserts / logs where one exists) | written by → read by |
|---|---|---|---|
| 0x000 | list | OSD window list (append order) | window_create / destroy → rebuild, window_update |
| 0x008 | list | cover window list (sorted by layer) | window_create / setattr → window_update |
| 0x010 | u16 | cover count (max 4) | create / destroy |
| 0x012 | u16 | OSD count (max 0x80) | create / destroy → rebuild grouping test |
| 0x014 | u16 | externally supplied front buffer in use (`update_front_buffer` 1, `clear_front_buffer` 0; both without caller in the module) [I: name] | → window_update "nothing to show" test, rebuild one-window-per-GWIN test |
| 0x018 | u32×2 | canvas size w, h (ScaleRect) | scale_rect → window_update, dump `CanvasWidth/Height` |
| 0x020 | u32×2 | canvas position x, y | scale_rect |
| 0x028 | u32×2 | screen size w, h | scale_rect → window_update, dump `ScreenWidth/Height` |
| 0x030 | u32×2 | screen position x, y | scale_rect |
| 0x038 | u8 | "scale set" flag (1 after ScaleRect) | scale_rect → window_update |
| 0x039 + g | u8 ×8 | per-GWIN display-off switch (`setDispOnOff`; 0 = shown) | set_disp_on_off → window_update |
| 0x044 | u32 | canvas write in progress (begin write 1, end write 0) | check_front_buffer_busy / blitosd → create, setattr |
| 0x048 | u8 | need rebuild | create / setattr / destroy / end write → rebuild (start test), cleared at its end |
| 0x049 | u8 | rebuilt, waiting for the swap | buf_done → swap (cleared) |
| 0x04a | u8 | realtime flip (`setRealtimeFlip`) | → create / setattr / destroy / end write; dump `RealtimeFlip` |
| 0x04b | u8 | delayed-delete pass armed | swap sets 1 → window_update walks the list at +0x2a0, clears it when the list is empty |
| 0x04c | u8 | delete delay (frames; from `cmdInfo` +0x12 at every Process) | window_update → rebuild (`_mi_rgn_drv_pre_queue_del_work` argument) |
| 0x04d | u8 | `setFbListMaxCnt` override on (argument a) | set_fb_list_cnt → window_update |
| 0x04e | u8 | `u8SetMulListCnt` — buffers per GWIN slot: 1 (default, set at creation) or 2, from `cmdInfo` +0x11 at every Process unless overridden; override value = argument b | → rebuild, window_update |
| 0x04f | u8 | queued draw works of this head | rebuild +1 when it queues a list → executor 0; window_update compares it with +0x4e |
| 0x050 + 0xC·g | 0xC ×8 | `astFbMultiDataHead[g]`: +0 list of the slot's buffers (first = the one on screen, last = back buffer; §3.7), +8 u8 `u8MultiListCnt`, +9 u8 flags (bit 0 = slot changed by this rebuild, consumed by the swap) | rebuild → swap |
| 0x0b0 + 0x38·g | 0x38 ×8 | front-buffer record g (a GWIN's current buffer): +0x00 CanvasInfo (u64 phys, VA, w, h, stride, fmt), +0x20 alpha attr (8 B), +0x28 point `{x, y}`, +0x30 u8 valid (externally supplied: update 1 / clear 0), +0x31 u8 `bShow` | swap → window_update, `get_frontbufferinfo` (dump) |
| 0x270 + g | u8 ×8 | buffer count of slot g (dump `BufCnt`) | rebuild / swap → get_frontbufferinfo |
| 0x278 + g | u8 ×8 | OSD windows in slot g (dump `OsdCnt`) | rebuild / swap → get_frontbufferinfo |
| 0x280 | Tsem | head semaphore | all |
| 0x2a0 | list | delayed-delete list (node: +8 u8 frames left, +0x10 0x20-byte buffer record) | rebuild / swap → window_update |
| 0x2a8 / 0x2ac | fn / arg | vsync callback (`notify_vsync`; no caller in the module) | → end of window_update |
| 0x2b0 / 0x2b4 | u8 / u32 | colour-key enable (0) / value (`0x23232323 \| mask` at creation) | set_color_key (no caller) → window_update |
| 0x2b8 | u64 | **BlitStartPts**: `mi_sys_GetCurPts` when the executor starts this head's draw work | executor |
| 0x2c0 | u64 | **BlitEndPts**: when it finished | executor |
| 0x2c8 | u64 | **EnqueuePts**: when the rebuild queued the work | rebuild |
| 0x2d0 | u8 | **draw-work timeout** flag (§3.8) | window_update → executor |
| 0x2d2 | u16 | **BlitTimeouCnt** (sic, dump column): +1 at each timeout, never reset | window_update → get_chnportinfo (dump) |
| 0x2d4…0x2d7 | — | no access found [R search; I: padding] | |

### 3.4 IMPL API semantics [R]
<!-- ev:rgn-gop.3.4 -->
All IMPL calls take the region-list lock (a Tsem) inside the module mutex. Not initialised (IMPL flag clear) → 10 (`0xA0032010`) for
every call except Init.

- **Init(palette)**: palette NULL → 06; already initialised → 12. `mi_sys_RegisterDev(devinfo{module 3, no channels},
  procops{OnDumpDevAttr, OnHelp})` (fail → 01), 16 proc commands (§3.10), `access_ok(palette, 0x400)` (fail → 06), copy
  1024 B in, `mi_rgn_DrvInit()` (§3.6), start the draw-work thread, obtain the driver instance (function tables of §3.6;
  sets the instance flag), `set_palette(copy)`, instance refcount +1, fill the 0x24-B capability record (below), free the
  copy, set initialised. **No per-process state**: a second process gets 12.
- **DeInit**: not initialised → 12. For each region: destroy every window, free each attachment, free the canvas (OSD),
  free the region (prints `rgn... destroy` lines). Instance refcount −1 → at 0 clear the function tables and the instance
  flag (`mi_rgn_Process` is a no-op again). `mi_rgn_DrvdeinitBufWork` (drain the draw work, free every channel-port head's
  buffers, stop the thread), `mi_rgn_DrvDeinit` (§3.6), print the memory counters, `mi_sys_UnRegisterDev`.
- **Create(h, attr)**: h ≥ 0x400 → 01; exists → 01. `attr.eType` OSD: h in [osdMinH, osdMaxH] = [1, 4096], w in [1, 4096],
  fmt < 7 and `MHAL_RGN_GetChipCapability(FMT_SUPPORT, fmt)` ≠ 0 — on this chip only ARGB1555, ARGB4444, I2, I4, I8 (§4.6);
  RGB565 and ARGB8888 regions are refused — I2 needs w % 4 = 0, I4 needs w even (all else → 03); allocate the canvas (fail
  → 0C). COVER: nothing checked, nothing allocated. Other types → 03. The node is appended to the list.
- **Destroy(h)**: unknown → 01. Destroys every window of the region (detaches it from all channels), frees the canvas.
- **GetAttr(h, attr)**: returns the 16 B stored at Create.
- **SetBitMap(h, bmp)**: bmp NULL or `pData` NULL → 06; COVER → 03; canvas currently lent (GetCanvasInfo without
  UpdateCanvas) → returns **0** without doing anything; bmp fmt > 6 or width 0 → also returns **0** (logged only); bmp h in
  [1, 4096], w in [1, 4096], the I2 / I4 width rules; bmp fmt ≠ region fmt → 03; w, h clipped to the canvas;
  `access_ok(pData, h × rowbytes)` (fail → 03); for every attached window `check_front_buffer_busy` (= begin write, §3.6),
  `mi_sys_Vmap(canvas phy, stride × h, cached)` (fail → 0C), copy h rows of `(w × bpp + 7) / 8` bytes from user space (the
  source row pitch is the same row length: the bitmap is tightly packed), `mi_sys_VFlushInvCache`, `mi_sys_UnVmap`, then
  `blitosd_front_buffer` (= end write) for every window.
- **GetCanvasInfo(h, info)**: OSD only (else 03); begin write on every window; returns the CanvasInfo (the kernel va field
  is the stored one, normally 0 — the library replaces it with its own mapping); sets the "lent" flag. User space writes
  the pixels through its mapping.
- **UpdateCanvas(h)**: OSD only; not lent → **09**; end write on every window; clears the flag.
- **AttachToChn(h, chnport, param)**: param or chnport NULL → 06; unknown h → 01; already attached to that exact ChnPort
  (module, dev, chn, port compared) → 12. OSD checks: colour-invert block (`bEnable` ≠ 0: WDivNum in [⌈w/64⌉, min(w, 256)],
  HDivNum likewise for h, WDiv × HDiv ≤ 2048, else 03); eAlphaMode ≤ 1 (else 03); position x ≤ 4095, y ≤ 4095, I2 x % 4 = 0,
  I4 x even (else 03). COVER checks: w in [1, 8192], h in [1, 8192], x ≤ 8191, y ≤ 8191 (else 03). Then `window_create`
  (§3.6); on success the attachment is linked and `attachChn...` logged.
- **DetachFromChn(h, chnport)**: not attached → 12; `window_destroy`, unlink.
- **SetDisplayAttr(h, chnport, param)**: the checks of Attach (for OSD the colour-invert sizes against the region), not
  attached → 12, then `window_setattr` (§3.6). **GetDisplayAttr**: not attached → 12; copies the stored 40-B param.
- **ScaleRect(chnport, sizeFrom, sizeTo)**: under the lock `scale_rect(chnport, from, pos 0, to, pos 0)`: every dimension
  ≤ 4096 (else 03); if the heights differ the source width must be ≤ 1920 (a vertical scale is refused for canvases wider
  than 1920; 03); the source is clamped to the destination; stored in the channel-port head (P+0x18…+0x38) with the "scale
  set" flag. From then on the OSD coordinates of that channel-port are taken in canvas units and the canvas / screen pair
  goes to `MHAL_RGN_GopSetBaseWindow` at every frame (§3.5 step 8). mhal writes stretch ratios only on the DISP_UI GOPs
  (§4.4), so on the SCL GOPs nothing is stretched: confirmed live 2026-10-27 — canvas 1920×1080 → screen 3840×2160, a
  480×270 region showed 480×270 at the frame's (0, 0) and a move to canvas (960, 540) landed at frame (960, 540); the ratio
  registers 0x6A / 0x6C of GOP 2 (phys 0x1F250ED4 / 0x1F250ED8) read back 0 at once after a direct write and the 0xFE
  latch [R: live]. The ioctl is write-only, so nothing is returned (§2.2).

**Capability record** (0x24 B, filled at Init, printed by `getcap`): +0…+6 per-format support = `GetChipCapability(0,
fmt)`; +7 overlap support (cap 1); +8 x-position overlap support (cap 2); +9 channels 64; +0xA covers per channel-port 4;
+0xB OSDs per channel-port 128; +0xC front buffers (= HW GWIN count, cap 6); +0xE / +0x10 / +0x12 / +0x14 cover min w, min
h, max w, max h = 1, 1, 8192, 8192; +0x16…+0x1C OSD min w, min h, max w, max h = 1, 1, 4096, 4096; +0x20 colour-key value
`0x23232323 | mask`. Region handles 0…1023. The `cat` of the proc file shows it as read live [R live 2026-10-27]: OSD
formats 0–4 supported, 5 and 6 not; regions up to 4096×4096; «HW mode: 8 frontbuffs», «X pos overlap: Support»,
«Overlap: Support».

Live behaviour of the API [R: live 2026-10-26/27]:
- Format 1 is ARGB4444 with per-pixel alpha (alpha 8 of 15 shows the picture through), format 0 ARGB1555; codes 0–4 as in
  §2.3. A region attached from a freshly started process showed about 4 s after the command (process start and MI opens).
- One OSD region attached to SCL dev 0 port 0 (HDMI path) and port 2 (USB path) at once shows in both outputs; position and
  size are in each port's own pixels.
- `SetDisplayAttr` with only a new x / y (no canvas change) applies at the next output frame: a 128×128 ARGB4444 region on
  dev 0 port 2 moved once per frame at 4K45 showed a new position in 440 of 448 frames (the rest off by one frame from the
  mover's own timing); at 1080p120 a 64×64 region showed 1198 of 1199 positions in one frame each, also with several I8
  regions moving at once; the call takes ~0.02 ms. An x beyond the frame is accepted and the region clipped.
- `SetBitMap` fills an I8 region's whole canvas from tightly packed rows in one call (nothing is mapped into the process).
  `UpdateCanvas` returns in ~0.5–2 ms.
- Canvas updates (SetBitMap or GetCanvasInfo / UpdateCanvas) need more than two output frames between them; closer ones are
  dropped silently: 960×1080 at 4K45 updated 15/s and 20/s (3 and 2.25 frames apart) all showed, 30/s about half; at 4K40
  20/s (exactly 2 frames) half. Larger regions apply slower (a 3.9 MB region ~10/s, an 8.3 MB full-frame I8 region
  ~3–4/s). With updates paced at least three frames apart, the last of a burst of changes 40 ms apart is what shows.
  Taking the canvas again (GetCanvasInfo = begin write, §3.6) before the next frame has ended clears the "need rebuild"
  mark the previous UpdateCanvas set, so that update is lost: a GE drawing loop that re-took the canvas at once showed its
  first state and then nothing at 4K45; paced to 15 updates a second it moved. An I8 1920×1080 region attached in a nearly
  full heap never showed, with no error returned; a whole-frame 3840×2160 ARGB4444 region (16.6 MB) brought about one update
  in three to the picture where a 3840×1080 one (8.3 MB) brought every one; at 1080p60 the updates reached at most every
  second frame (30 per second) [R]. Why large regions lose updates is not settled: an earlier reading of ours explains it by
  a new front buffer allocated per update with up to two old ones kept and the allocation failing silently [I]; the decision table read later (§3.7) allocates only
  when the slot has room or the size or format changed and otherwise reuses the back buffer, and a failed allocation leaves
  "need rebuild" set and retries at the next frame [R]. The two readings disagree for a same-size update.
- Memory: each shown region holds two media-heap chunks named `rgn…`, its canvas and one front buffer (two small regions
  0.21 MB, five regions of a 4K menu 4.44 MB; a whole-frame 4K ARGB4444 region 33 MB).

### 3.5 Per-frame path: `mi_rgn_Process` and `mi_rgn_NotifyFrameDone` [R]
Both exports take the driver-instance Tsem; **if the instance flag is clear they return 0 at once** — the state of this
camera (§1). Otherwise they call `ops.window_update(cmdInfo, cmdq)` resp. `ops.buf_done(chnPort)`; a non-zero driver result
becomes 0xA0032003. `cmdInfo` NULL / `chnPort` NULL → BUG_ON.

`cmdInfo` as mi_rgn reads it (values as mi_scl fills them at its call site in `mi_scl.c`) [R; names I]: +0 module (0x22
SCL, 0x17 LDC), +4 dev, +8 chn, +0xC port, +0x10 u8 bEnable (SCL: 1), +0x11 u8 double-buffer flag (SCL: 1), +0x12 u8
delete-delay frames (SCL: 0), +0x14 u32 frame width, +0x18 frame height, +0x1C x, +0x20 y (window of the frame the OSD lives
in; SCL: 0, 0), +0x28 path type (0 = SCL output port, 1 = LDC), +0x2C port index.

`window_update` (= Process), under the channel-port Tsem (the head is created if it does not exist yet):
1. **Swap**: if a rebuild finished (P+0x49, §3.7), the newly built front buffers become the current GWIN records
   (`_mi_rgn_drv_swap_frontbuffer_info`, §3.7); replaced buffers are on the delayed-delete list. When that pass is armed
   (P+0x4b), entries whose counter is 0 are freed (`mi_sys_PrivateDevChnHeapFree`), the others are decremented.
2. If `bEnable`: CMDQ id = 2 (LDC) for module 0x17, 0 (VPE) for 0x22 (others: 0 + log); **`MHAL_RGN_SetupCmdQ(cmdq, id)`**
   — every following MHAL register write of this call goes into the frame's command queue (§4.3).
3. **Covers** (path type 0 only): cover engine = `{0:0, 1:1, 2:2, 3:3, 4:5, 5:6}[port index]` (`.rodata` table at
   `.rodata`+0xCA0 for indexes 1–5; 0 otherwise) = `MHAL_RGN_CoverType_e` VPE_PORT0…3, VPE_PORT4, VPE_PORT5 (DIVP skipped).
   For each cover window in layer order with `bShow`: the rectangle is in **1/8192 units of the frame**: `x' = (x·W + 4096)
   >> 13`, `y' = (y·H + 4096) >> 13`, `w' = (w·W + 4096) >> 13`, `h'` likewise, `w'` / `h'` clipped to the frame;
   `CoverSetWindow(engine, n, x', y', w', h')`, `CoverSetColor(engine, n, u32Color)`, `CoverEnable(engine, n)` with n
   counting the shown covers 0…3; the remaining layers up to 3 get `CoverDisable`.
4. **Geometry**: without ScaleRect, canvas = screen = (W, H) and position = (x, y) of `cmdInfo`; with it, the stored canvas
   / screen sizes are used.
5. **GOP** = `{0:0, 1:1, 2:2, 3:3, 4:6, 5:7}[port index]` (`.rodata`+0xCB4) for path 0 (`MHAL_RGN_GopType_e` VPE_PORT0…3,
   VPE_PORT4, VPE_PORT5); path 1 with index 0 → GOP 5 (LDC_PORT0). If the channel-port has no OSD window and no externally
   supplied front buffer (P+0x14): `GopGwinDisable` + `GopGwinSetWindow(…, 0, 0, 0, 0, 0)` for every GWIN, then step 8.
   The port index comes from mi_scl, so channels of one SCL device share a GOP: SCL dev 1's two channels (1/0/0 and
   1/1/0) report GOP 3 [R live 2026-10-27]. The GWIN registers are queued into each frame's CMDQ, so with windows on both
   channels each output showed its own windows correctly: the 8 GWINs are per frame, not per port pair [R live 2026-10-27].
6. **OSD routing**: `MHAL_RGN_GetGopBindOsd(&mask)`; if the GOP's bit is clear: GOP 5 → `mask = mask & ~0x90 | 0x20`, GOP 7
   → `mask = mask & ~0x30 | 0x80`, `MHAL_RGN_SetGopBindOsd(mask)` (bits = `MHAL_RGN_GopOsdFlag_e`; one OSD blender is shared
   between DIP 0x10, LDC 0x20 and PORT5 0x80, §4.2); any other GOP with a clear bit → logged, all its GWINs disabled.
7. **GWINs**: for each front-buffer record i < GWIN count that is shown, non-empty and not switched off by `setDispOnOff`:
   intersect the record's rectangle with the screen rectangle; if non-empty, on the next free GWIN g:
   - first GWIN only: `GopSetColorkey(gop, keyEnable, R, G, B)` with the channel-port colour key converted from the record's
     format (1555: 5-bit fields expanded `v<<3 | v>>2`; 4444: nibbles doubled; I2 / I4 / I8: the low byte for R = G = B;
     565; 8888: bytes) — §3.9;
   - alpha mode PIXEL: `GopSetAlphaType(gop, g, PIXEL=1, 0)`, `GopSetArgb1555AlphaVal(gop, g, ALPHA1, FgAlpha)`,
     `GopSetArgb1555AlphaVal(gop, g, ALPHA0, BgAlpha)`; CONSTANT: `GopSetAlphaType(gop, g, CONSTANT=0, ConstantAlpha)`;
   - `GopGwinSetPixelFormat(gop, g, fmt)`, `GopGwinSetWindow(gop, g, w∩, h∩, record stride, x∩ − screenX, y∩ − screenY)`;
   - start address = record phy + stride·(y∩ − recordY) + bytes of (x∩ − recordX) pixels, rounded down to 16 B →
     `GopGwinSetBuffer(gop, g, addr)`; the rounding remainder in pixels → `GopGwinSetBufferXoffset(gop, g, xoff)`;
     `GopGwinEnable(gop, g)`.

   Unused GWINs: disable + zero window.
8. `GopSetBaseWindow(gop, src {0, 0, canvasW, canvasH}, dst {0, 0, screenW, screenH})`.
9. Store the double-buffer flag (P+0x4e, unless the `setFbListMaxCnt` override is on) and the delete delay (P+0x4c) from
   `cmdInfo`; set or clear the draw-work timeout (§3.8); call the channel-port vsync callback if one was set (none is set
   anywhere in this module).

`buf_done` (= NotifyFrameDone, called by mi_scl when the frame's output buffer is done): if "need rebuild" (P+0x48) is set,
rebuild the front buffers (§3.7) and mark "rebuilt" (P+0x49; the swap happens at the next Process); wake the draw-work
thread if work is queued. The swap at the next Process waits for the channel-port's queued draw work only when the
`setFbListMaxCnt` override (§3.10) is on; otherwise it swaps at once [R]. So an OSD change normally becomes visible one to
two frames after the API call: rebuilt after frame N, drawn by the BDMA worker, swapped into the GWINs when frame N+1 is
programmed [I]. With "realtime flip" set (proc command only) the rebuild runs in the API call itself [R].

### 3.6 Driver layer (`mi_rgn_drv.c`) [R]
**Instance**: `mi_rgn_DrvGetInstance(wm, bm)` fills two function tables. Window manager (24 slots, in order): set_palette,
get_palette, get_capability, window_create, window_destroy, window_setattr, window_getattr, window_getchport,
blitosd_front_buffer (= end write), check_front_buffer_busy (= begin write), get_frontbufferinfo, get_chnportinfo,
set_disp_on_off, set_fb_list_cnt, scale_rect, window_update (Process), buf_done (NotifyFrameDone), set_color_key,
win_cal_debug, get_draw_work_pts, set_realtime_flip, update_front_buffer, clear_front_buffer, notify_vsync. Buffer manager
(4): misys_buf_alloc, misys_buf_del, calpixstride, buf_set_color_key_mask. Only the IMPL layer and the proc commands call
them; `set_color_key`, `update_front_buffer` / `clear_front_buffer` and `notify_vsync` have no caller in this module
[R: no references besides the table; I: left for other in-kernel users — no module on the camera imports
`mi_rgn_DrvGetInstance`].

**`mi_rgn_DrvInit`** (refcounted by `_gatInitDrvRefCnt`, work only on the first call): GWIN count N =
`GetChipCapability(HW_GWIN_CNT)` (forced 0 if > 8); `MHAL_RGN_CoverInit()`; `MHAL_RGN_GopInitByType(0)`; for every GOP 0…7
and every format the chip supports (0–4): `GopSetAlphaZeroOpaque(gop, bEn 0, bConstAlpha = (fmt == RGB565), fmt)`; Tsems for
the hash table, the draw-work queue and the statistics; wait queue `_gBufWorkQCond`; the 1-KiB palette shadow cleared.
**`mi_rgn_DrvDeinit`** (last reference): `MHAL_RGN_SetupCmdQ(NULL, 0/1/2)` (back to direct register writes),
`GopGwinDisable(gop, g)` for GOP 0…7 × g < N, `MHAL_RGN_GopDeinitByType(0)`, `MHAL_RGN_CoverDeinit()`, free every
channel-port head (`_mi_rgn_drv_do_clear_wq_and_fb_each_hashmap`, `_mi_rgn_drv_do_clear_each_hashmap`).
**Draw-work thread** `RGN BUF WQ` (`CamOsThreadCreate {prio 99, stack 0xC00}`): sleeps on the wait queue in 10-jiffy steps
while the queue is empty, then runs `_mi_rgn_drv_do_draw_work(all)` (§3.8).

**Windows** (`window_create` / `_destroy` / `_setattr` / `_getattr` / `_getchport`): OSD windows are appended to the
channel-port's OSD list (128 max, else 0xA0032003); a new or changed OSD window marks the channel-port "need rebuild"
unless a canvas write is in progress; covers are kept sorted by `u32Layer` (4 max, else 03) and need no buffers. With
"realtime flip" set, a create / setattr / destroy rebuilds at once and runs the draw work in the caller.

**Canvas write bracket**: begin write (`check_front_buffer_busy`) clears "need rebuild", marks "writing" (P+0x44) and runs
the channel-port's queued draw work synchronously (no blit reads the canvas while user space writes it); end write
(`blitosd_front_buffer`) clears "writing", marks "need rebuild" and the window "canvas dirty". The consequence for a canvas
taken again before the next frame is in §3.4 (live).

### 3.7 Front buffers: composition into the GWINs [R]
<!-- ev:rgn-gop.3.7 -->
A GOP has N hardware GWINs (N = `HW_GWIN_CNT` = 8), so the OSD windows of a channel-port are composed into at most N
**front buffers**, one per GWIN slot. The composition runs in `_mi_rgn_drv_rebuild_frontbuffer` (from NotifyFrameDone, or
in the API call with realtime flip).

**Objects.**
- *Slot list* `astFbMultiDataHead[g]` (P+0x50+0xC·g, g < N): a list of *front-buffer descriptors* (0x50 B: +0x08 list of
  window descriptors, +0x10 buffer record {u64 phys, VA, w, h, stride, fmt} = 0x1C B, +0x30 alpha attr, +0x38 point {x0,
  y0}, +0x41 u8 bShow, +0x48 u8 window count, +0x49 u8 `bUpdate` = OR of the windows' "canvas dirty", +0x4a u8 `bSetAttr`
  = OR of their "attr dirty"); **first element = on screen ("Use"), last = back buffer ("Back")**; +8 u8 `u8MultiListCnt`;
  +9 bit 0 "slot changed".
- *Window descriptor* (0x58 B, `pChnPortListDataDesc`): +0x08 OSD ordinal, +0x10…+0x2f copy of the window's region
  CanvasInfo (canvas phys at +0x10), +0x30…+0x57 copy of its 40-byte `ChnPortParam` (+0x30 bShow, +0x34 x, +0x38 y, +0x3c
  layer). Of the `ChnPortParam` copy only these four fields are read by the decision code; the rest is not mapped.
- *New* descriptor of slot g (`pstTmpFbMultiData[g]`): built by the grouping step, absent when there are fewer groups than
  slots. Building it copies and **clears** the windows' two dirty bytes (window +0x78 / +0x79).
- *Draw work* (0x14 B `pstBufDrawWork`: list node, op list, +0x10 P) with ops (0x58 B `pstbufOptData`: +0x08 type 0 init /
  2 blit, +0x10 destination buffer record, +0x30 source canvas record, +0x50 / +0x54 destination x, y).

**Grouping** (which windows share a front buffer). If the chip reports overlap support (`GetChipCapability(1)`) **and** the
OSD count (P+0x12) + P+0x14 ≤ N, each window gets its own New descriptor. Otherwise `_mi_rgn_cal_frontbuffer`: windows are
placed one by one (first the one nearest the origin, then the farthest, then the rest); a window opens a new group while
fewer than N groups exist, else joins the group with the smallest squared distance between window centres
(`_mi_rgn_drv_calculate_pos_distance`). After each placement groups whose bounding boxes overlap are merged; groups that
overlap in x but not in y are merged too unless `XPOS_OVERLAP_SUPPORT`. The inner arithmetic of the placement helper
(`_mi_rgn_cal_frontbuffer`) and of the geometry helper (`_mi_rgn_drv_caculate_frontbuffer_size`) is not read beyond what is given here.

**Group geometry** (`_mi_rgn_drv_caculate_frontbuffer_size`): all windows of a group must have the same pixel format (else
0xA0032008 and the rebuild is abandoned); x0 = min x rounded down to `XPOS_ALIGNMENT`; y0 = min y; width = `((maxRight − x0)
+ (minX − x0) − 1 + a) / a × a` with a = `WIDTH_ALIGNMENT` (the `(minX − x0)` term is in the vendor formula as written);
height = maxBottom − y0; right / bottom clipped to 4096; alpha mode / para from the first window; "shown" = OR of the
windows' bShow.

**Order of work** (`_mi_rgn_drv_rebuild_frontbuffer`; the debug-log texts `.LC74…LC111` name every branch):
1. Start only when "need rebuild" (P+0x48) is set; zero four 8-entry scratch arrays (New descriptors, Back, Use, new buffer
   records).
2. Grouping and geometry as above; abandon on a format mix.
3. **Trim**: for every slot, while `u8MultiListCnt > u8SetMulListCnt` (P+0x4e) drop the **last** descriptor (its buffer to
   the delayed-delete list, `Cut the mul list left: %d`). `room[g] = (u8MultiListCnt < u8SetMulListCnt)`.
   `u8SetMulListCnt == 0` → `Mul list cnt setting is zero!`, rebuild abandoned.
4. **Decide** per slot (table below): flags *drop* (put the slot's existing buffers on the delayed-delete list), *alloc*,
   *init* (queue op 0), *blit* (queue ops 2), *changed* (the slot gets the New descriptor; flag bit 0).
5. **Act** per slot: drop → every descriptor of the slot list goes away, buffers to the delayed-delete list with the delay
   P+0x4c (`_mi_rgn_drv_pre_queue_del_work`); init → one op 0 on New's buffer; blit → one op 2 per window descriptor of New
   whose bShow ≠ 0, **or every window when New holds only one** (a hidden single window is still copied), destination offset
   `(win.x − x0, win.y − y0)`; then the list rotation below. Unchanged slots: New is freed.
6. Clear P+0x48; if any op was queued: append the work to the global queue (draw-work thread), P+0x4f += 1, EnqueuePts :=
   now (P+0x2c8); else free it.

Any allocation failure (`_mi_rgn_drv_misys_buf_alloc fail.`) frees **all** buffers allocated by this rebuild and all New
descriptors and returns without clearing P+0x48 — the rebuild is retried at the next `NotifyFrameDone`; nothing is
reported to the caller.

**Per-slot decision table.** New is compared with the slot's Back (last) and Use (first) descriptors (with one buffer per
slot they are the same element). "offsets equal" = `firstwin.x − x0` is the same in New, Back and Use. Branch targets where
Ghidra's structure misleads were checked in the disassembly (the "new buffer empty" exit at 0xae0c→0xa5f0 and the reuse
path at 0xb0ca→0xae3a→0xa5f0 both end the slot **without** allocating).

| # | condition (in test order) | log text | drop | buffer | init | blit | changed |
|---|---|---|---|---|---|---|---|
| 1 | no New for this slot | `New buf is empty, and Mul buf need to del it if exist .` | yes | — | — | — | yes (slot empties) |
| 2 | New, slot list empty | `New buf is not empty, Must blit!.` | — | **alloc** | no | yes | yes |
| 3 | Back's w, h or fmt ≠ New's | `Need to alloc buffer. 0: w %d h %d 1: w %d h %d!` / `New buf is different, need del old first…` | yes | **alloc** | no | yes | yes |
| 4 | same size; Use's window count ≠ New's | `List count not match cur is %d new is %d need init and blit!!` | — | see A | yes | yes | yes |
| 5 | same size, same count, New `bUpdate` | `bUpdate flag is raise, must blit!` (+ `And buf cnt is larger than 1 must init buf!` when New has > 1 window; + `[Update] Xpos has offset gap…` when `x0 ≠ firstwin.x` and New's firstwin.x ≠ Use's) | — | see A | if > 1 window or the x-gap case | yes | yes |
| 6 | as 5 but `bSetAttr`, New has ≥ 2 windows | `bSetAttr flag is raise!` + `Buf cnt is larger than 1 must init buf and blit!` | — | see A | yes | yes | yes |
| 7 | `bSetAttr`, one window, offsets equal | `Need update pos, bShow and alphaAttr to current used buf.` | — | **descriptor-only**: New's alpha, point, bShow and the window's `ChnPortParam` are copied into both Use and Back; no buffer work | — | — | **no** |
| 8 | `bSetAttr`, one window, offsets differ | `[Setattr] Xpos has offset gap and x pos is changed so need init buf.` | — | see A | yes | yes | yes |
| 9 | no flag, Use has windows, some window's canvas phys differs from Back's or Use's at the same list position (or a list is shorter) | `Back buf compared with the old/current one is different!!! need blit!!` (+ `And buf cnt is larger than 1 must init buf` with > 1 window; else + `Xpos has offset gap…` when offsets differ) | — | see A | if > 1 window or offsets differ | yes | yes |
| 10 | otherwise (incl. Use with no windows) | `Nothing to do!!` | — | — | — | — | no |

**A — buffer for rows 4–6, 8, 9** (same size): `room[g]` true → `Lack buf, and alloc buf!`: **alloc** a fresh buffer and the
init flag is **cleared** (a fresh buffer is cleared at allocation); `room[g]` false → **reuse Back's buffer**: New takes
Back's phys and VA, stride recomputed by `calpixstride(fmt, w)`, init / blit as decided.

**Allocation** (`_mi_rgn_drv_misys_buf_alloc`): name `rgnfb<mod>_<dev>_<chn>_<port>_<g>`, private heap of device 0 channel 0,
fallback MMA heap `mma_heap_name0`, uncached; stride by `calpixstride`. The new buffer is cleared **synchronously** with
`mi_sys_MemsetPa` (`_mi_rgn_drv_init_buf`): formats 2–4 (I2 / I4 / I8) to 0, formats 0, 1, 5, 6 to `0x23232323 | mask`, any
other format BUG. A newly allocated buffer therefore gets **no** op 0; op 0 is queued only for a reused buffer with the
init flag (supersedes step 3, "a new buffer … op 0 init then blits").

**Multi-buffer list rotation.**
- Rebuild, changed slot: if `room[g]` → New appended at the **tail**, count + 1; if not → the Back descriptor (the tail) is
  freed **without** freeing its pixel buffer (New owns it now, case A) or with it already dropped (rows 1, 3), then New
  appended at the tail. Flag bit 0 set. With `u8SetMulListCnt` = 2 the list is `[Use (on screen), New]` after a rebuild;
  with 1 it is `[New]`, i.e. New reuses the buffer that is on screen [R; I: drawing into the visible GWIN buffer while the
  BDMA works, tearing possible].
- **Swap** (`_mi_rgn_drv_swap_frontbuffer_info`, 0x15794; at the next Process after `NotifyFrameDone` set P+0x49;
  with the `setFbListMaxCnt` override on it is skipped while this head still has draw work queued): every front-buffer
  record P+0xb0+0x38·k whose +0x30 byte (externally supplied buffer) is 0 is cleared, the BufCnt / OsdCnt bytes
  (P+0x270…+0x27f) are zeroed; for each slot with bit 0 the **tail is moved to the front** and bit 0 cleared; then the
  first descriptor of every non-empty slot is copied (0x38 bytes from descriptor +0x08: buffer record, alpha, point, bShow)
  into the next free record, skipping records with the +0x30 byte set, with BufCnt := the slot's `u8MultiListCnt` and
  OsdCnt := its window count. Order: when some slot's first descriptor composes ≥ 2 windows, slot order; otherwise the
  slots are first sorted by their window's layer (window descriptor +0x3c) **descending**, ties by OSD ordinal (+0x08)
  descending — record 0 gets the highest layer, so the GWIN index follows the layer [R disassembly 0x5aaa…0x5bd2; the sort
  is missing from the Ghidra output]. Finally P+0x49 := 0, P+0x4b := 1 (arms the delayed-delete pass). At steady state with
  2 buffers the displayed buffer and the back buffer alternate on every content change; position-only changes (row 7)
  never cost a buffer or a blit.
- Overlapping windows on one GOP do not blend: where two overlap only the upper one shows (the higher `u32Layer`), and its
  transparent pixels show the video, not the window below [R live 2026-10-26/27]. Which GWIN index the GOP itself draws on
  top is not read from the hardware side.
- The front buffer is what the GOP scans: its physical address is listed by `dumpchport` (§3.10), and pixels written there
  directly appear in the next frames without any driver call [R live 2026-10-27]; erasing and redrawing in that buffer
  while it is scanned shows a band rolling down the frame [R live, seen by the user].

### 3.8 Draw-work executor and statistics [R]
<!-- ev:rgn-gop.3.8 -->
The executor `_mi_rgn_drv_do_draw_work` runs a head's ops: init as `mi_sys_MemsetPa` (index formats 0, others the key word),
fill as `mi_sys_BufFillPa` (`_mi_rgn_drv_buf_fill`), blit as `mi_sys_BufBlitPa` (`_mi_rgn_drv_blit_buffer`) — all BDMA
(`mi_sys-2-interfaces.md` §7.8) — with MI pixel codes ARGB1555 5, ARGB4444 6, I2 7, I4 8, I8 9, RGB565 4, ARGB8888 1. Op 1
*fill* (a rectangle in a colour) exists in the executor but no builder in this module emits it [R: only ops 0 and 2 are
built]. `BlitStartPts` (P+0x2b8) is stamped with `mi_sys_GetCurPts` when the head's work starts.

**Timeout**: window_update sets P+0x2d0 := 1 when the `setFbListMaxCnt` override is off and the queued works (P+0x4f) ≥
`u8SetMulListCnt` (P+0x4e), else 0; on 1 it logs `[MI_RGN]: Warning!!! Draw Work Timeout... [ModId…]` (`.LC70`, source line
0x10b4) and increments `BlitTimeouCnt` (P+0x2d2). Beside it, `if (count != 0) count++` increments the queued-work counter
only when it is already non-zero [R decompile; I: probably meant as a frame counter].

After a head's draw work the executor sets P+0x4f := 0 and `BlitEndPts` (P+0x2c0); **only if P+0x2d0 is set** it appends
`{mod, dev, chn, port, P+0x2b8, P+0x2c0, P+0x2c8}` to a global ring of 20 records × 0x28 bytes (index byte wraps at 0x14);
then it clears P+0x2b8…+0x2cf. So `dumpDrawWorkPts` (columns `EnqueuePts`, `BlitStartPts`, `BlitEndPts`, `Bs-Eq`, `Be-Bs`, in
ns) lists the last 20 works **that ran after a timeout warning**, not the last 20 works (supersedes 
step 4).

### 3.9 Palette, colour key, alpha, colour invert [R]
<!-- ev:rgn-gop.3.9 -->
- **Palette**: one global 256-entry table; `set_palette` stores it with **entry 0 forced to {A 0xFF, R = G = B = 0x23 |
  mask}** and writes all 256 entries to **every GOP 0…7** (`MHAL_RGN_GopSetPalette(gop, A, R, G, B, idx)`, 2048 calls). It is
  used by I2 / I4 / I8 regions; the API offers no way to change it after Init.
- **Background and colour key**: buffers are cleared to `0x23232323 | mask` (index 0 for the index formats). Each
  channel-port head is created with key value `0x23232323 | mask` and key enable 0; only `set_color_key` would enable it and
  nothing in this build calls it, so every GOP gets `GopSetColorkey(gop, 0, …)` = key off. The mask is set only by the proc
  command `setcolorkeymask <hex>` (default 0).
- **What shows where nothing is drawn** [R: live 2026-10-26]:
  with one pixel format on the port, I8, I4 and I2 regions show index 0 transparent and their palette alpha working (a
  zeroed palette passed to Init, no colour-key call); I4 / I2 pack the low bits first. ARGB1555 `0x2323` has the alpha bit
  0 (→ BgAlpha in PIXEL mode); ARGB4444 `0x2323` has alpha 2/15 [I: a faint background, not checked live]. This supersedes
  an earlier static reading of ours that palette entry 0, forced to alpha 0xFF, would make the index formats'
  background opaque grey; which step of that reading is wrong is not found. The opaque grey seen once came from mixing an
  I2 region with ARGB4444 regions on one port (dev 0 port 2): the video under the I2 region turned opaque grey and the
  ARGB4444 colours went wrong; without the I2 region all was right — the format register is shared by all 8 GWINs of the
  GOP (§4.4).
- **Alpha**: per window `eAlphaMode` PIXEL (ARGB1555 alpha bit → Fg / Bg alpha; other formats use their own alpha) or
  CONSTANT (one 8-bit alpha for the GWIN); a front-buffer group takes it from its first window. The constant alpha
  **replaces** the per-pixel alpha [R live 2026-10-27]: with constant 128 the window's transparent pixels show their RGB
  (black) at 50 % over the video ((88,88,88) against video (152,152,152)) and a strip with pixel alpha 1…15 comes out
  uniform; with 255 the whole window rectangle is opaque black where it was transparent.
- **Colour invert** (`bEnable`, mode, luma threshold, W / H divisions) is checked, stored and dumped only: mi_rgn imports
  none of `MHAL_RGN_GopSetColorInverse*` / `Gop{Write,Read}ColorInverseData`, and those have no hardware effect in this
  mhal anyway (§4.6) — this build never programs a colour-invert block.

### 3.10 Proc interface and dump files [R]
mi_sys device commands, `echo <cmd> <args> > /proc/mi_modules/mi_rgn/mi_rgn0` (the per-device file of
`mi_sys-2-interfaces.md` §10.3). `cat` = capability + region list + channel-port list + colour-invert table + buffer
counters (`_MI_RGN_IMPL_GetDumpListHead`, `_MI_RGN_IMPL_GetWindowListData`). Commands (argument count):

| command | args | action |
|---|---|---|
| `getcap` | 0 | capability record (§3.4) |
| `dumprgn` | 0 | region list |
| `dumpchport` | 0 | per channel-port: canvas / screen sizes, front buffers (with their physical addresses), OSD / cover windows, alpha |
| `bufcnt` | 0 | kmalloc / MMA bytes (§3.11) |
| `dumpRgnBuf` | 2: handle, dir | canvas to a file (below) |
| `dumpFrontBuf` | 5: mod dev chn port dir | front buffers to files (below) |
| `dumpPalette` | 0 | the palette |
| `dumpDrawWorkPts` | 0 | the timeout ring of §3.8 |
| `setDispOnOff` | 6: mod dev chn port gwin on | per-GWIN display switch (P+0x39 + g) |
| `setFbListMaxCnt` | 6: mod dev chn port a b | override on = a, buffers per slot = b (P+0x4d / P+0x4e) |
| `forceUpdate` | 1: handle | begin + end write on every window of an OSD region |
| `dumpColorInvert` | 0 | colour-invert table |
| `setRgnScale` | 12: mod dev chn port, canvas w h, canvas x y, screen w h, screen x y | as ScaleRect with positions |
| `setcolorkeymask` | 1: hex | the colour-key mask (§3.9) |
| `debugRgnWin` | 4: mod dev chn port | `win_cal_debug`, a log line only |
| `setRealtimeFlip` | 5: mod dev chn port flag | realtime flip (P+0x4a) |

`help` prints a format-test line (`Hello 6 …`).

**Dump files**: both commands `filp_open(name, 0x42 = O_RDWR | O_CREAT, 0644)` — **no O_TRUNC** — and one `vfs_write` at
offset 0 of the raw pixel bytes from a cached `mi_sys_Vmap`; no header, no row padding removed; an existing longer file
keeps its old tail.
- `dumpRgnBuf <handle> <dir>`: OSD regions only (type 0; COVER and unknown handles do nothing); file
  `<dir>/Rgn<h>_canvasInfo_fmt<f>_<w>X<h>_stride<s>`, length `stride × h` from the region's CanvasInfo (+0x3c × +0x38); Vmap
  failure → `0xA003200D`. Under the region-list Tsem.
- `dumpFrontBuf <mod> <dev> <chn> <port> <dir>`: only when the module is initialised; for every GWIN slot g < the
  capability's front-buffer count with a non-empty descriptor: `<dir>/Mod<m>_Dev<d>_Chn<c>_Port<p>_frontBuf<g>_fmt<f>_
  <w>X<h>_stride<s>` (written via `filp_open` / `vfs_write`), length `stride × h` of the slot's current buffer
  (P+0xb0+0x38·g). The pixel formats in both names are the `MI_RGN_PixelFormat_e` codes of §2.3 [I: the same enum the region
  attributes use].

### 3.11 Memory accounting [R]
Every allocation is recorded (`mi_rgn_MemAllocCnt(ptr, size, isKmalloc)`, a 16-B node in a list; kmalloc and MMA byte
totals); `mi_rgn_MemFreeCnt` removes it and BUG_ONs on an unknown pointer; `mi_rgn_MemCntPrint` (exported) prints the
leftovers at DeInit. A debug aid only.

## 4. mhal.ko block RGN — GOP, OSD blender, cover

### 4.1 Layers and source match [R]
<!-- ev:rgn-gop.4.1 -->
Four layers as in the u-boot tree: `MHAL_RGN_*` (`mhal_rgn.c`, 42 exports) → `Drv{Gop,Osd,Cover,Rgn}*` (`drv_*.c`; caches
every setting per GOP / GWIN and skips unchanged writes, §4.8) → `Hal{Gop,Osd,Cover,Rgn}*` (`hal_*.c`, register writes) →
`HalRgnWrite2Byte[Msk]` (RIU or CMDQ). Linux glue: clocks via the DT node `sigmastar,gop` (`mhal-core.md` §7),
`/proc/mrgn/{dbglv, gop, osd, cover}` (§4.10, created by `RgnSysfsInit`, `mhal-core.md` §3.2).

190 functions decompiled (181 named + 9 static without symbols); 160 names also exist in u-boot `rgn/drv` +
`rgn/hal/mercury6`. Binary-only: `MHAL_RGN_Gop{InitByType, DeinitByType, SetCscConfig, SetOutputFormat, CheckAvailable}`,
`MHAL_RGN_CoverCheckAvailable` and their Drv / HAL counterparts (`DrvGop{RgnInitByType, InitCscConfig, SetCscConfig,
SetOutputFormat, CheckAvailable}`, `DrvCoverCheckAvailable`, `HalGopSetOutputFormat`, `HalGopTransGopIdToType`,
`_DrvGopGetForceUpdate`), the cache `_tGopLocCscSetting` (§4.8), the proc writer (`rgn_gop_store`, `RgnProcGopStore`,
`_RgnPorcGopCsc`, `_RgnProcSplit`, `_RgnProcParsingCommand`) and the seq_file readers (`RgnDebug{Gop,Osd,Cover,Dbglv}Show`,
`rgn_dbglv_store`). The 9 unnamed statics (Ghidra names, address = `.text` + 0x40000028) are the source's
`_HalGopGetBindMode` (0x400a9574), `_HalGopGetGwinBaseAddr` (0x400a95c0; its `__func__` string confirms it),
`_HalGopIdTransCmdqID` (0x400a969e), the GWIN shift-table read (`gop_gwin_shift_read` (0x400a96f0)), `_HalGopGetBaseAddr` (0x400a9748),
`_HalGopWriteDoubleBuffer` (0x400a97e0), `_HalGopGetGwinSize` (0x400a9898), an error helper (`gop_error` (0x400a9724)) and
`_DrvGopExecQueryFunc` / `_DrvOsdExecQueryFunc` (0x400a6b2c, `_DrvOsdExecQueryFunc`) [R: bodies match the source functions].
The source-only `DrvRgnUseCmdq` is in the binary too, as a tail jump to `HalRgnUseCmdq` at `.text` 0xa7c98 that Ghidra did
not split off.

Comparison: a constant-set comparison of the 145 functions present in both (→, the source preprocessed by `pp-src.sh`) found 20 identical; the other differences are
mostly `__LINE__` shifts (the binary's source is ~0x20–0x40 lines longer per file), switches compiled to tables, and byte-
vs word-register offsets. Every GOP / OSD / cover HAL function was read; the 47 Drv-layer and 38 MHAL-layer functions
present in both trees (`drv/src/{drv_gop, drv_osd, drv_cover, drv_rgn_common, mhal_rgn}.c`) were read side by side with
the source — all 85 do what the source does except the differences of §4.11. The
binary is a later revision than the u-boot source. Everything below describes **the binary**.

### 4.2 IDs and their maps [R]
- **MHAL GOP type** (`MHAL_RGN_GopType_e`): 0–3 VPE_PORT0–3, 4 DIVP_PORT0, 5 LDC_PORT0, 6 VPE_PORT4, 7 VPE_PORT5,
  8 DISP0_CUR, 9 DISP0_UI, 10 DISP1_CUR, 11 DISP1_UI. → **HAL GOP id** through `geGopIdFromOsd[12]`, rebuilt from the
  OSD-sharing mask `geGopOsdFlag` (module parameter `rgn_module.geGopOsdFlag`, `mhal-core.md` §5; `.data` default **0xF5F**;
  reset to 0xF5F at init if not one of 0xF5F, 0xF6F, 0xFCF): types 0–3 → 0–3; 4 → 5 if bit 0x10 (DIP); 5 → 5 if 0x20 (LDC);
  6 → 4 if 0x40; 7 → 5 if 0x80; 8–11 → 6–9 if 0x100…0x800; otherwise 10 = invalid and the MHAL call returns −1. With the
  default 0xF5F DIVP owns HAL GOP 5, and LDC_PORT0 and VPE_PORT5 have no GOP. The three accepted masks differ only in bits
  0x10–0x80, i.e. only in who owns HAL GOP 5.
- **HAL GOP banks** (`_HalGopGetBaseAddr`; 24-bit RIU addresses = bank × 0x100 + byte offset; RIU: `kernel-platform.md`
  §3.1): 0 0x1281, 1 0x1284, 2 0x1287, 3 0x1291, 4 0x1294, 5 0x1297, 6 0x11A1, 7 0x11A4, 8 0x11B1, 9 0x11B4. Each GOP = 3
  banks: global (+0x000), GWIN 0–3 (+0x100), GWIN 4–7 (+0x200); GWIN g's register block at `(g < 4 ? 0x100 : 0x200) +
  0x20 × (g & 3)`. GOPs 0–5 are the SCL / DIVP / LDC OSD GOPs with 8 GWINs ("type 0"), 6–9 the display GOPs with 1 GWIN
  ("type 1": 6 = DISP0 cursor, 7 = DISP0 UI, 8 / 9 = DISP1); 7 and 9 ("DISP_UI") use an older register layout.
- **CMDQ id** per GOP (`_HalGopIdTransCmdqID`): 0–4 → 0 VPE; 5 → 1 DIVP if flag 0x10, 2 LDC if 0x20, else 0; 6–9 → 3 DISP.
  Cover: HAL 0–5 → 0, 6 (DIVP) → 1. Palette and OSD-bind writes always use id 4 = direct RIU.
- **OSD blender id** (from the MHAL GOP type, `_DrvOsdTransId` → `HalOsdTransId`): 0, 1, 2 → 0, 1, 2; 3 → 5; DIVP → 3;
  LDC → 4; PORT4 → 6; PORT5 → 7; DISP types → 8–11. Blender banks: 0 0x1214, 1 0x1215, 2 0x1216, 3 (DIVP) 0x1219, 4 (LDC)
  0x12F0, 5 0x1217, 6 0x1218, 7 0x1219; the display ids have none (enable is skipped).
- **Cover** (`MHAL_RGN_CoverType_e` 0–3 VPE_PORT0–3, 4 DIVP, 5 VPE_PORT4, 6 VPE_PORT5) → HAL 0–3, 6, 4, 5 → banks 0x1214,
  0x1215, 0x1216, 0x1217, 0x1218 (HAL 4), 0x1219 (HAL 5 and 6 share it); `HalCoverTransHalId` gives HAL 4 = PORT4, 5 =
  PORT5, 6 = DIVP. 4 cover windows (layers) per engine.
- **Pixel format**: MHAL (`0 ARGB1555, 1 ARGB4444, 2 I2, 3 I4, 4 I8, 5 RGB565, 6 ARGB8888, 7 UV8Y8`) → Drv
  (`DrvGopGwinSrcFmtType_e`, §4.8) → HAL source format (`0 I8, 1 I4, 2 I2, 3 ARGB1555, 4 ARGB4444, 5 RGB1555, 6 RGB565,
  7 2266, 8 ARGB8888, 9 ABGR8888, 10 UV7Y8, 11 UV8Y8, 12 RGBA5551, 13 RGBA4444`): ARGB1555 → 3, ARGB4444 → 4, I2 → 2, I4 → 1,
  I8 → 0, RGB565 → 6, ARGB8888 → 8, UV8Y8 → 11. Register value: the SCL GOPs take HAL codes 0–4 only (I8, I4, I2, ARGB1555,
  ARGB4444 — anything else is logged "not support" and **0xFF** is written); the DISP_UI GOPs translate through a 14-entry
  table `{4, 13, 14, 6, 2, 0, 1, 3, 5, 7, 8, 9, 10, 11}`. Bytes per pixel for the pitch arithmetic: 32-bit ×4, 16-bit ×2,
  I8 ×1, I4 /2, I2 /4.

### 4.3 Register access, CMDQ and the double buffer [R]
- `HalRgnWrite2Byte(addr24, v, cmdqId)` / `Write2ByteMsk(addr24, v, mask, cmdqId)`: if cmdqId < 4, a CMDQ interface for that
  id is installed and the "use CMDQ" flag is on (set by `HalRgnInit`, cleared around the Deinit sequences), the write is
  appended to a per-id 32-entry buffer `{u32 addr24, u16 value, u16 mask}` (0x100 B; "Cnt overflow" printed at 32 — the
  buffer is not bounds-checked beyond the print); otherwise **direct RIU**: `*(u16 *)(0xFD000000 + addr24 × 2)`
  (read-modify-write for the masked form). `HalRgnRead2Byte` always reads RIU.
- Every MHAL call ends with `DrvRgnUpdateReg` → `HalRgnUpdateReg`: for each id with queued entries, call the installed
  interface's function at +0x1C (`MHAL_CMDQ_CmdqInterface_t`, `mhal-scl-jpe-cmdq.md` §2.3) with (buffer, count), then clear the
  count.
- `MHAL_RGN_SetupCmdQ(iface, type)` (type 0 VPE, 1 DIVP, 2 LDC, 3 DISP; else −1): copies the 0x98-B interface; NULL clears
  it. Only mi_rgn calls it (VPE / LDC, per SCL frame, §3.5); **nobody installs a DISP interface**, so fbdev's display-GOP
  writes are always direct RIU writes.
- **Double buffer** (`_HalGopWriteDoubleBuffer`, after every GOP HAL write): GOP global reg 0x7F (byte +0xFE): `[11:8] = 0`,
  then `[11:8, 3:0] = _eGopWrRegMd` (`.data` 0x100 = "all banks double write"), then `[11:8] = 0` again. The GOP registers
  are thus double-buffered: a write lands in a shadow and is committed by this latch sequence; the event at which the
  hardware then takes the committed values is not read [I: the frame start (vsync) of its path; fb0's pan, which waits
  for vsync, flips cleanly, §5.10]. The cover's `_HalCoverWriteDoubleBuffer` is an empty function in this build.

### 4.4 GOP registers written (binary; byte offsets from the GOP or GWIN base) [R]
<!-- ev:rgn-gop.4.4 -->
GOP global:

| reg (byte) | written by | content |
|---|---|---|
| 0x00 | UpdateParam (init), BindOsd, SetOutputFormat, SetAlphaInvert, SetColorKey | bit 0 soft reset (pulsed 1→0 at init); mask 0x440A = bind mode from `_HalGopGetBindMode`: SCL GOPs `0x4002` (HS_MASK + VS_INV), GOP 5 in LDC mode `0x4000`, DISP cursor `0x4000`, DISP_UI `0x000A` (VS_INV + progressive), plus `out_fmt << 10` (bit 10: 0 RGB, 1 YUV; per-GOP `geOutFormat[]`, `.data` default 1,1,1,1,1,1,0,0,0,0); bit 11 RGB colour-key enable; bit 5 YUV colour-key enable (DISP_UI only); bit 15 alpha invert ("alpha zero opaque") |
| 0x04 | init | `[7:6] = 2` |
| 0x06 / 0x08 / 0x0A | palette write | 0x06 = G<<8 \| B, 0x08 = A<<8 \| R, 0x0A `[7:0]` = index, then 0x0A bit 8 pulsed (write strobe), `[13:12] = 0`; bracketed by 0x7F bit 9 set / clear; on GOP 5 the index-0 write also sets 0x20 `[14:11]` as BindOsd does |
| 0x20 | init (DISP_UI) / BindOsd (GOP 5) | DISP_UI `[11:0] = 0x501`; GOP 5 `[14:11]` = 0x2000 (DIP / VPE) or 0x4000 (LDC) |
| 0x32 (reg 0x19) | init | `[7:0]` DMA threshold = `gGopDmaThd` (module parameter `drv_rgn_os.gGopDmaThd`) or 0x40 when it is 0 |
| 0x48 / 0x4A | colour key RGB | 0x48 = G<<8 \| B, 0x4A `[7:0]` = R |
| 0x4C / 0x4E | colour key YUV (DISP_UI) | same layout |
| 0x60 / 0x62 / 0x64 / 0x68 | stretch window | width (DISP_UI: width / 2), height, x, y (16 bit) |
| 0x6A / 0x6C / 0x70 / 0x72 / 0x74 | stretch ratios (DISP_UI only) | H ratio = (src<<12) / dst, V ratio, H init, V init (= remainder), 0x74 = 0; `[12:0]`; DISP_UI refuses down-scaling (logged, ratios not written) |
| 0xE0 | init (not DISP_UI) | `[15:14] = 0` |
| 0xFE (reg 0x7F) | after every write | double-buffer latch (§4.3) |
| *0x113742* | init | **bank 0x1137 (DISP GP_CTRL) reg 0x21 = 0** on every GOP init — not in the u-boot source |

The SCL GOPs have no stretch: written live, their 0x6A / 0x6C read back 0 at once, while the DISP_UI GOP 7 holds 0x1000
(1:1) there [R: live 2026-10-27].

GWIN (block of §4.2):

| reg | content |
|---|---|
| +0x00 | bit 0 enable; `[7:4]` source format — **written to all 8 GWINs of an SCL GOP at once** (one format per GOP; mixing formats on one port breaks the picture, §3.9), GWIN 0 only on the display GOPs; alpha-type bit (value and mask checked in the disassembly): bit 1 (SCL) / bit 14 (DISP_UI) set = per-pixel alpha, clear = constant |
| +0x02 / +0x04 | buffer address in 16-B units: `[15:0] = addr[19:4]`, `[11:0] = addr[31:20]` |
| +0x06…+0x0C (SCL) | H start, H end, V start, V end in pixels, ends inclusive (`x + w − 1`) |
| +0x08…+0x10 (DISP_UI) | H start / end in 16-B units of bytes (`pix2byte(x) >> 4`, end exclusive), V start (+0x0C), V end (+0x10) |
| +0x0E (SCL) / +0x12 (DISP_UI) | pitch `[10:0]` = bytes per line >> 4 |
| +0x10 (SCL) | `[7:0]` constant alpha **and** ARGB1555 alpha-0 value (the same field), `[13:8]` X offset in pixels within the first 16 B |
| +0x12 (SCL) | `[7:0]` ARGB1555 alpha-1 value |
| +0x14 (DISP_UI) | `[7:0]` constant alpha (ARGB1555 alpha values not supported on DISP_UI) |

Bandwidth [R: live 2026-10-27]: a GWIN costs memory bandwidth by its size, not by what is drawn in
it. With USB 4K25 MJPEG beside a 4K25 H.264 recording: no overlay 260 frames in 10.36 s (25.1 fps); one 3840×2160 ARGB4444
GWIN on the USB port 210 in 10.47 s (20 fps); one on each port 209 in 11.16 s (19 fps); the same picture as windows over the
drawn parts only (~4.4 MB of I8, or five ARGB4444 windows) ran at 25.0–25.1 fps [R; I: the GOP fetches every pixel of a
GWIN each frame, transparent or not — 16.6 MB × 25 per whole-frame layer].

### 4.5 OSD blender, OSD bind, cover, clocks [R]
<!-- ev:rgn-gop.4.5 -->
- OSD enable (`HalOsdSetOsdEnable`): blender bank + 0xA0 (reg 0x50) bit 0; the LDC blender (0x12F0) at +0xE2 (reg 0x71)
  bit 0, plus bit 1 and bit 6 set when disabling, cleared when enabling. Display ids skipped. `MHAL_RGN_GopGwinEnable`
  enables the GWIN and the blender of that MHAL type; `GopGwinDisable` disables the blender only when the GWIN being
  disabled is the GOP's last enabled one (`DrvGopGetGwinEnableCount`).
- OSD bind (`HalOsdSetBindGop`): bank 0x1210 (SCL) byte 0x10 (reg 0x08) bit 12 = 1 when `geGopOsdFlag & 0x20` (LDC uses the
  shared OSD), else 0. `MHAL_RGN_SetGopBindOsd(mask)`: mask must be 0xF5F, 0xF6F or 0xFCF (else returns 0xFF), store,
  rebuild the id map, set the bank-0x1210 bit, re-run `HalGopBindOsd` for all ten GOPs. `GetGopBindOsd` returns the mask.
- **Cover** window w (0–3) of an engine: base + 0x10·w: +0x40 bit 0 enable; +0x42 H start, +0x44 H end (= x + w), +0x46 V
  start, +0x48 V end (= y + h); +0x4A `[7:0]` colour byte 0, `[15:8]` byte 1; +0x4C `[7:0]` byte 2 (in 16-bit register
  numbers: regs 0x20–0x26 + 8·w). The u32 colour of `MHAL_RGN_CoverSetColor` is split as bytes 0 / 1 / 2 = bits 7:0 / 15:8
  / 23:16 without conversion; the hardware takes them as the port's YUV: byte 0 U, byte 1 Y, byte 2 V, full-range BT.601 as
  the frames decode (0x0000FF → (0,49,227), 0x00FF00 → (74,255,27), 0xFF0000 → (180,0,0) in RGB are exactly U, Y, V = 255
  alone) [R: live 2026-10-27]. A cover lies **under** the whole overlay: an OSD window over
  it shows unchanged (magenta (236,17,233) over the cover against (238,16,234) over video); it needs no memory, and show /
  hide, move and recolour go through `SetDisplayAttr` [R live 2026-10-27]. `CoverInit` disables and zeroes all 4 windows of
  all 7 engines.
- **Clocks**: `HalGopSetClkEnable` (SCL GOPs only): CLKGEN 0x1038 reg 0x64 (`0x1038C8`), for GOP 5 in LDC mode reg 0x53
  (`0x1038A6`, the same register as the LDC clock, `mhal-core.md` §2.5), bit 0 = gate: cleared if set, the previous state
  returned; `HalGopSetClkDisable` only reads (the disabling write is commented out in the source too).
  `MHAL_RGN_GopSetPalette` turns the GOP clock on through the CCF (`of_find_compatible_node("sigmastar,gop")`, clock index 1
  SCL / 2 LDC / 3 DISP, `CamClkPrepare/Enable`) when it writes index 0 and off again at index 255 (static `bForceClk[]`, one
  byte per MHAL GOP type), so a full 256-entry palette upload runs with the clock forced on.

### 4.6 The exported API [R]
All return 0 / −1 (`0xFFFFFFFF`) unless noted; an invalid MHAL id → −1 before anything is written.

| export | does | users |
|---|---|---|
| `GopInit` / `GopDeinit` | legacy: init / deinit all GOPs (`DrvGopRgnInit`) | — |
| `GopInitByType(t)` | first user of either type: `DrvRgnInit` (CMDQ buffers), OSD init (all blenders off, bind bit), id map, clocks; then init every HAL GOP of type t (0 = GOPs 0–5, 1 = 6–9): soft reset, bind mode, DMA threshold, stretch 0, both colour keys off, every GWIN disabled with format I8, address and window 0, constant alpha 0xFF, ARGB1555 alpha-0 0 / alpha-1 0xFF; CSC cache cleared; state bit 1 (t 0) / bit 2 (t 1) of `geRgnInit`; `CoverInit` uses bit 0 | mi_rgn (0), fbdev (1) |
| `GopDeinitByType(t)` | re-init type t's GOPs with direct writes, clear its bit; the last one: OSD init, flag 0xF5F, `DrvRgnDeinit` | mi_rgn, fbdev |
| `SetGopBindOsd(mask)` / `GetGopBindOsd(&mask)` | §4.5 | mi_rgn |
| `SetupCmdQ(iface, type)` | §4.3 | mi_rgn |
| `GopSetPalette(gop, A, R, G, B, idx)` | one palette entry (RIU, clock bracket at idx 0 / 255) | mi_rgn, fbdev |
| `GopSetBaseWindow(gop, src{x,y,w,h}, dst{x,y,w,h})` | stretch window = src; ratios from src / dst on DISP_UI only | mi_rgn, fbdev |
| `GopSetMirFlip(gop, mode)` | **no-op** in this build (`DrvGopSetMirror/Flip` have no HAL call) | — |
| `GopGwinSetPixelFormat(gop, g, fmt)` | §4.2 format, all GWINs of the GOP | mi_rgn, fbdev |
| `GopGwinSetWindow(gop, g, w, h, pitch, x, y)` | GWIN position / size + pitch | mi_rgn, fbdev |
| `GopGwinSetBuffer(gop, g, u64 addr)` | buffer address (low 32 bits used; `HalGopMiuSelect` is empty) | mi_rgn, fbdev |
| `GopGwinSetBufferXoffset(gop, g, x)` | +0x10 `[13:8]` (not on DISP_UI) | mi_rgn |
| `GopGwinEnable(gop, g)` / `GopGwinDisable(gop, g)` | §4.5 | mi_rgn, fbdev |
| `GopSetColorkey(gop, en, R, G, B)` | RGB key (reg 0x00 bit 11, 0x48 / 0x4A) | mi_rgn, fbdev |
| `GopSetAlphaType(gop, g, 0 CONSTANT / 1 PIXEL, val)` | GWIN alpha bit + constant (types > 1 → −1) | mi_rgn, fbdev |
| `GopSetArgb1555AlphaVal(gop, g, 0/1, val)` | ARGB1555 alpha-0 / alpha-1 value (not DISP_UI) | mi_rgn, fbdev |
| `GopSetAlphaZeroOpaque(gop, bEn, bConstAlpha, fmt)` | reg 0x00 bit 15 = bEn, inverted on DISP_UI unless (fmt ARGB1555 and not constant alpha); one bit per GOP, so the last call wins | mi_rgn, fbdev |
| `GopSetColorInverse{Enable, Param, Update}`, `Gop{Write,Read}ColorInverseData` | only the enable byte is stored (`_tOsdpLocSettings[id].bColorInv`); the parameter is not stored because the HAL parameter check is empty; Update / Write / Read store nothing; **no hardware effect** (§4.9) | — |
| `GopSetVideoTimingInfo(gop, cfg)` | stored only (HAL query slot 0 empty) | — |
| `GopSetCscConfig(gop, {type, n, coef*})` | needs n = 10 coefficients and type 0–2; stored; HAL slot 1 empty → no effect (§4.8) | — |
| `GopSetOutputFormat(gop, yuv)` | `geOutFormat[gop]` + reg 0x00 bit 10 | fbdev |
| `GopCheckAvailable(gop)` / `CoverCheckAvailable(c)` | 0 if the id maps to hardware | — |
| `GetChipCapability(cap, fmt)` | 0 FMT_SUPPORT: fmt < 5 (**ARGB1555, ARGB4444, I2, I4, I8 only**); 1 OVERLAP 1; 2 XPOS_OVERLAP 1; 3 WIDTH_ALIGN / 4 XPOS_ALIGN: 1 for fmt < 5 else −1; 5 PITCH_ALIGN 16; 6 HW_GWIN_CNT 8; else −1 | mi_rgn, fbdev |
| `CoverInit` / `CoverDeinit` | all cover windows off; refcount bit | mi_rgn |
| `CoverSetWindow(c, layer, x, y, w, h)`, `CoverSetColor(c, layer, u32)`, `CoverEnable/Disable(c, layer)` | §4.5 | mi_rgn |

fbdev's 15 calls are the rows marked fbdev; mi_rgn's 24 are the rows marked mi_rgn. fbdev addresses MHAL GOP type
**8 + layer** and initialises the display GOPs with `GopInitByType(1)` (§5.4) [R: fbdev.c `MHAL_RGN_GopSetBaseWindow(layer
+ 8, …)`, `GopInitByType(1)`]. On a display GOP only GWIN 0 exists for the Drv
layer (`_DrvGopTransGwinId`, §4.11 item 1).

### 4.7 GWIN overlap, latching, cursor, size and per-frame cost
What the sources settle about the GOP's windows, question by question.

**Overlap: which GWIN is on top, and limits.** The capability table says overlap and x-overlap are supported (caps 1 and 2 =
1) [R]. No priority or z-order register of the GOP is written anywhere in this mhal and none appears in the register list of
§4.4; whether one exists is unknown. Live, overlapping windows of one GOP do not blend and the window with the higher
`u32Layer` is the one shown [R live 2026-10-26/27]; mi_rgn's swap gives the highest layer GWIN record 0 when every slot
holds one window (§3.7) [R]. Together: the GWIN with the lower index is on top [I — from the two readings; the hardware
priority rule itself is not read]. Limits: no per-scanline or count limit on overlapping GWINs is known from the sources
(unknown); the alignment caps are 1 pixel for x and width in every supported format (caps 3, 4) [R]; the address is 16-byte
aligned with the remainder given as an X offset in pixels, field `[13:8]` = up to 63 pixels (§4.4) [R]; mi_rgn itself adds
x % 4 = 0 for I2 and even x for I4 (§3.4) [R].

**Base address and position: latching and flips.** Every GWIN register write (address, window, pitch, enable, format,
alpha) is followed by the double-buffer latch of reg 0x7F (§4.3), so the GOP's registers are double-buffered in hardware
[R: the latch sequence; I: its meaning "commit the shadow at the next frame start" — the latch event is not read]. On the
SCL GOPs mi_rgn queues all of a frame's GWIN writes into that frame's CMDQ (§3.5), so they are applied together when the
CMDQ runs for that frame [R: queueing; I: before the frame is scanned]. No per-GWIN flip, ready or busy bit exists in the
written register set and none is known (unknown whether the hardware has one). Pointing a GWIN at another buffer every
frame: live, per-frame position changes through the CMDQ (a descriptor-only update, §3.7 row 7) applied at the next
output frame without tearing (440 of 448, 1198 of 1199 frames, §3.4) [R live 2026-10-26]; a per-frame change of the buffer
address alone was not measured; tearing was seen only when the scanned buffer itself was rewritten (§3.7) [R live]. That an
address change through the frame's CMDQ is equally tear-free is [I] (same registers, same double buffer).

**Hardware cursor or sprite.** The only cursor GOPs are the display ones: MHAL types 8 DISP0_CUR and 10 DISP1_CUR = HAL GOPs
6 and 8, one GWIN each, with fbdev's fb1 path on top of them (§5.7) [R]. No SCL-side GOP and nothing of the scaler register
set in these sources is a cursor or sprite engine: the SCL GOPs 0–5 are plain 8-GWIN OSD GOPs [R for what is written;
unknown for SCL registers outside the RGN block]. Routing a DISP cursor GOP to an SCL output: the id map sends types 8–11
only to HAL GOPs 6–9 and the three accepted `geGopOsdFlag` masks differ only in who owns HAL GOP 5 (§4.2); the OSD blenders of
the SCL ports exist only for HAL GOPs 0–5 and the display ids have no blender bank [R]. No path in this mhal routes a
display GOP into an SCL output; whether the hardware could is unknown.

**Minimum size, alignment, cost per frame.** Sizes: the HAL writes the window as given (SCL ends `x + w − 1`, inclusive)
and no minimum check is recorded for it in the sources [R as far as read]; mi_rgn requires OSD w, h ≥ 1 (I2 w % 4 = 0, I4 w even) and covers w, h ≥ 1 [R]; a hardware minimum is not known. Alignment:
x and width 1 pixel (caps 3, 4), pitch in 16-byte units (`PITCH_ALIGN` 16, field `[10:0]` → at most 2047 × 16 bytes),
address in 16-byte units with the pixel X offset [R]. Cost: a GWIN programmed by mi_rgn costs eight or nine MHAL calls
(colour key on the first GWIN only, alpha type, two ARGB1555 alpha values in PIXEL mode, format, window, buffer, X offset,
enable — §3.5 step 7);
each changed register is one queued CMDQ entry plus the three writes of the reg-0x7F latch [R: the write sequence; the
count per field is not tabulated]; unchanged values cost nothing because the Drv cache skips them (§4.8) [R]. The per-id
queue holds 32 entries and is flushed at the end of every MHAL call (§4.3) [R]. The time this takes in the CMDQ engine is
not measured; the API call that moves a region costs ~0.02 ms of CPU [R live 2026-10-26, §3.4].

### 4.8 Drv caches [R]
<!-- ev:rgn-gop.4.8 -->
The three caches have exactly the source layout (`_DrvGopLocalSettingsConfig_t` 0x218, `_DrvOsdLocalSettingsConfig_t`
0x94, `_DrvCoverLocalSettingsConfig_t` 0x3A — the cross compiler gives the same sizes from the u-boot mercury6 headers as
the binary's strides) and every offset the binary uses falls on a source field. The
binary has one cache the source does not: `_tGopLocCscSetting`.

**`_tGopLocSettings[10]`** — 0x218 B per GOP, indexed by the Drv GOP id (= HAL GOP id 0–9: `_DrvGopTransId` copies the id
and only asks `HalGopCheckIdSupport`). Every setter except `DrvGopSetGwinSizeWithoutFmt` and `DrvGopSetVideoTimingInfo`
compares the new value with the cached one and skips the HAL write when equal unless `_DrvGopGetForceUpdate` returns 1;
that helper asks HAL GOP query slot 3, which is never filled (§4.9), so it always returns 0 and the skip is unconditional.
"Not need to exec" is printed for a skipped call when `gbrgndbglv` bit 2 is set.

| off | size | source field | written by (binary) | read by |
|---|---|---|---|---|
| 0x000 | 4 | `eGopOutFmtSet` (0 RGB, 1 YUV) | `_DrvGopInit` (the value `HalGopUpdateParam` leaves in its config), `DrvGopSetOutputFormat` (raw argument) | nothing in Drv |
| 0x004 | 4 | `eGopDispModeSet` | `_DrvGopInit` = 1 (PROGRESS) | nothing |
| 0x008 | 8 | `tGopSrcStrWinSet` {x, y, w, h} u16 | `_DrvGopInit` = 0, `DrvGopSetStretchWindowSize` | that setter (compare), `/proc/mrgn/gop` SrcX…SrcH |
| 0x010 | 8 | `tGopDstStrWinSet` {x, y, w, h} | same | same, DstX…DstH |
| 0x018 | 4 | `tGopColorKeySet` {bEn, R, G, B} u8 | `_DrvGopInit` = 0 (one u32 store), `DrvGopSetColorKey` | that setter, proc CKeyEn / R / G / B |
| 0x01C | 1 | `u8GWinNum` | `_DrvGopInit` = 8 (also for the one-GWIN display GOPs) | nothing |
| 0x01D | 3 | padding | — | — |
| 0x020 | 4 | `eSrcFmt` (GOP level) | never written | never read |
| 0x024 | 1 | `bMirror` | `_DrvGopInit` = 0 (u16 store covering 0x24–0x25) | nothing (`DrvGopSetMirror` only prints "Not support") |
| 0x025 | 1 | `bFlip` | same | nothing (`DrvGopSetFlip` likewise) |
| 0x026 | 2 | padding | — | — |
| 0x028 | 8 × 0x3C | `tGwinLocSet[8]` (table below) | | |
| 0x208 | 14 | `tVideoTiming` {Htotal, Vtotal, HdeStart, VdeStart, Width, Height u16; bInterlaceMode, bYuvOutput u8} | `DrvGopSetVideoTimingInfo` (stored, then query slot 0 — empty) | nothing |
| 0x216 | 2 | padding | — | — |

GWIN entry g at 0x028 + 0x3C·g:

| off in entry (abs. for g 0) | size | source field | written by | read by |
|---|---|---|---|---|
| +0x00 (0x28) | 1 | `bGwinEn` | `_DrvGopGwinInit` = 0, `DrvGopRgnSetEnable` | `DrvGopGetGwinEnableCount` (count of GWINs with value 1 + the last such index), `DrvGopRgnSetEnable` (compare), proc (row printed only when non-zero) |
| +0x04 (0x2C) | 4 | `tGwinGenSet.eGwinId` | `_DrvGopGwinInit` = g | nothing |
| +0x08 (0x30) | 4 | `tGwinGenSet.eSrcFmt` (Drv code, below) | `_DrvGopGwinInit` = 4 (I8), `DrvGopSetSrcFmt` | `DrvGopSetSrcFmt` (compare), `DrvGopSetGwinSizeWithoutFmt` (bytes per pixel for `HalGopSetGwinSize`), proc Fmt |
| +0x0C (0x34) | 8 | `tGwinGenSet.tDisplayWin` {x, y, w, h} | `_DrvGopGwinInit` = 0, `DrvGopSetGwinSizeWithoutFmt` (always, no compare) | proc X / Y / W / H |
| +0x14 (0x3C) | 4 | `tGwinGenSet.u32BaseAddr` | `_DrvGopGwinInit` = 0, `DrvGopUpdateBase` (the address after `HalGopMiuSelect`, which is empty) | `DrvGopUpdateBase` (compare), proc MemAddr |
| +0x18 (0x40) | 2 | `tGwinGenSet.u16Base_XOffset` | `_DrvGopGwinInit` = 0, `DrvGopUpdateBufferXoffset` | that setter (compare), proc XOft |
| +0x1C (0x44) | 4 | `tGwinGenSet.u32Base_YOffset` | `_DrvGopGwinInit` = 0 | nothing |
| +0x20 (0x48) | 4 | `tGwinAblSet.eAlphaType` (0 CONSTANT, 1 PIXEL) | `_DrvGopGwinInit` = 0, `DrvGopSetAlphaBlending` (when `ePixelAlphaType` = 2 NOUSE) | that setter (compare), proc AType |
| +0x24 (0x4C) | 4 | `tGwinAblSet.ePixelAlphaType` | never written | never read |
| +0x28 (0x50) | 3 | `tGwinAblSet.tAlphaVal` {constant, ARGB1555 alpha0, alpha1} u8 | only through the three pointers below | proc CVal / AVal0 / AVal1 (through the pointers) |
| +0x2C (0x54) | 4 | `u32MemPitch` | `_DrvGopGwinInit` = 0, `DrvGopSetMemPitchDirect` | that setter (compare), proc MemH |
| +0x30 (0x58) | 4 | `p8ConstantAlphaVal` → +0x28 | `_DrvGopGwinInit` via `HalGopSetAlphaPointVal(2, &tAlphaVal, …)` | `*p` = 0xFF at init; `DrvGopSetAlphaBlending` stores the constant only when the type is CONSTANT |
| +0x34 (0x5C) | 4 | `p8Argb1555Alpha0Val` → +0x28 | `HalGopSetAlphaPointVal(0, …)` | `*p` = 0 at init; `DrvGopSetAlphaBlending` with type 0 (ALPHA0) |
| +0x38 (0x60) | 4 | `p8Argb1555Alpha1Val` → +0x2A | `HalGopSetAlphaPointVal(1, …)` (pointer = &tAlphaVal + 2) | `*p` = 0xFF at init; type 1 (ALPHA1) |

`HalGopSetAlphaPointVal(type, base, &ptr)`: type 0 and 2 → `ptr = base`, type 1 → `ptr = base + 2`, other types leave `ptr`
unchanged. So the "constant alpha" and "ARGB1555 alpha-0" pointers both point at byte +0x28 of the entry: one shared cache
byte, matching the shared register field GWIN +0x10 `[7:0]` (§4.4). The source does the same
(`E_HAL_GOP_GWIN_ALPHA_DEFINE_CONST` = 2, `…ALPHA0` = 0; source `hal_gop.c` / `drv_gop.c` and the binary agree). The byte
+0x29 (`u8Argb1555Alpha0Val`) is never addressed [I: no pointer or direct offset reaches it].

Drv source-format codes (`DrvGopGwinSrcFmtType_e`, cached at +0x08) and their HAL codes (`_DrvGopTranSrcFmt`): 0 RGB1555→5,
1 RGB565→6, 2 ARGB4444→4, 3 2266→7, 4 I8→0, 5 ARGB8888→8, 6 ARGB1555→3, 7 ABGR8888→9, 8 UV7Y8→10, 9 UV8Y8→11, 10 RGBA5551→12,
11 RGBA4444→13, 13 I4→1, 14 I2→2; any other value prints "Wrong enType" and calls **`CamOsPanic`** (kernel panic) before the
unreachable "SRC NOTSUPPORT" / 0xFF return. The source's `_DrvGopTranSrcFmt` default does the same (print,
`CamOsPanic("")`) [R: preprocessed `drv_gop.c`].

**`_tGopLocCscSetting`** — binary-only, 10 × 0x18 B + one byte:

| off | size | content | written by | read by |
|---|---|---|---|---|
| 0x18·n + 0x00 | 4 | CSC type as given (0 none, 1 r2y, 2 y2r; 3 rejected "CscType(3) is Not Support") | `DrvGopInitCscConfig` = 0, `DrvGopSetCscConfig` | `DrvGopSetCscConfig` (compare) |
| 0x18·n + 0x04 | 20 | the 10 u16 coefficients | `DrvGopInitCscConfig` = 0 (memset 0x14), `DrvGopSetCscConfig` (copied as five u32) | `DrvGopSetCscConfig` (compare; skip when type and all 10 equal) |
| 0xF0 | 1 | "initialised" bit per GOP n | `DrvGopInitCscConfig` | `DrvGopInitCscConfig`: zero entry n only when its bit is clear |

The byte at 0xF0 takes `1 << n`, so GOPs 8 and 9 never keep their bit and are re-zeroed on every init [I].
`DrvGopSetCscConfig(n, {type, u8 count, coef*})` requires count = 10 ("CoefNum(%d) != 10" otherwise), then forwards `{mode = 2
if type 2, 1 if type 1, else 0; count; coef*}` (0xC B) through HAL GOP query slot 1, which is empty — no register effect.
The only callers are `MHAL_RGN_GopSetCscConfig` and the proc writer (`echo "csc <gop> <r2y|y2r|other> <c0> … <c9>" >
/proc/mrgn/gop`: exactly 13 tokens, gop decimal, coefficients hex; any other token count prints "csc [GopId] [Type]
[Coef]"). mhal's init runs `DrvGopInitCscConfig(0..9)` once at `RgnSysfsInit` (`mhal-core.md` §3.2).

**`_tOsdpLocSettings[12]`** — 0x94 B per OSD blender, indexed by the HAL OSD id (0–11, §4.2):

| off | size | source field | written by | read by |
|---|---|---|---|---|
| 0x00 | 1 | `bOsdEn` | `DrvOsdInit` = 0, `DrvOsdSetEnable` (skips when equal) | `DrvOsdSetEnable`, `/proc/mrgn/osd` En |
| 0x01 | 1 | `bBypassEn` | `DrvOsdInit` = 1 (u16 0x0100 store over 0x00–0x01) | nothing; its HAL call `HalOsdSetOsdBypassForScEnable` is an empty function |
| 0x02 | 1 | `bColorInv` | `DrvOsdSetColorInverseEn` (stored, then query slot 0 — empty) | nothing |
| 0x03 | 1 | padding | — | — |
| 0x04 | 8 × 0x12 | `stColorInvCfg[8]` (`E_DRV_OSD_AE0…7`): {Th1, Th2, Th3, X, Y, W, H, BlkNumX, BlkNumY} u16 | `DrvOsdSetColorInverseParam` would copy entry `eAeId` only after query slot 1 (parameter check) returned OK; slot 1 is empty, so **never written** | nothing |

All 12 HAL ids are initialised by `DrvOsdInit` (it walks MHAL ids 0–11 through `_DrvOsdTransId`), also the four display ids
whose blender has no register bank. Same layout and use as the source; the source too stores `stColorInvCfg` only after a
successful check (`drv_osd.c` `DrvOsdSetColorInverseParam`).

**`_tCoverLocSettings[7]`** — 0x3A B per cover engine, indexed by the HAL cover id 0–6:

| off | size | source field | written by | read by |
|---|---|---|---|---|
| 0x00 | 1 | `bDbEn` (double-buffer enable) | `DrvCoverRgnInit` = 0 | nothing (`_HalCoverWriteDoubleBuffer` is empty, §4.3) |
| 0x01 | 1 | padding | — | — |
| 0x02 + 0x0E·w | 1 | window w (0–3) `bBwinEn` | `DrvCoverRgnInit` = 0, `DrvCoverSetEnableWin` | that setter (compare), `/proc/mrgn/cover` (row printed only when set) |
| +0x02 (abs. 0x04 + 0x0E·w) | 8 | `tWinSet` {x, y, w, h} u16 | init 0, `DrvCoverSetWinSize` (skip when all four equal) | that setter, proc X / Y / W / H |
| +0x0A (abs. 0x0C + 0x0E·w) | 3 | `tWinColorSet` {R, G, B} u8 | init 0, `DrvCoverSetColor` (skip when equal) | that setter, proc R / G / B |
| +0x0D | 1 | padding | — | — |

The u8 R / G / B names are the source's; the three bytes go unchanged to the cover colour registers, where the hardware
reads them as U, Y, V (§4.5).

### 4.9 HAL query tables [R]
`_DrvGopExecQueryFunc` (0x400a6b2c) / `_DrvOsdExecQueryFunc` (0x400a7898) call `HalGopQuery` / `HalOsdQuery` with
`{u32 slot, u32 id, void *cfg, u32 cfgSize, out: u32 ret, fn pfForSet, …, fn pfForGet at +0x1C}`; `ret` = 0 when the slot
holds a function, else 3 (NOTSUPPORT); on 0 they call `pfForSet(id, cfg)` then, if non-NULL, the +0x1C function with (id,
+0x18). Same scheme as the source's `_DrvGopExecQueryFunc` (source `drv_gop.c:67`).
- GOP table `gGoppfForSet` (`HalGopInit` clears 0x10 B, then sets): slot 0 video timing — empty; 1 CSC — empty; 2 output
  format = `HalGopSetOutputFormat`; 3 force update — empty; 4 GOP type = `HalGopTransGopIdToType` (stored at +0x10, just past
  the 0x10 B the memset clears; `HalGopQuery` accepts slots < 5). The source has one slot (`E_HAL_QUERY_SETVIDEOTIMINGINFO`,
  empty).
- OSD table `gOsdpfForSet` (`HalOsdInit` clears 0x18 B = 6 slots; nothing is ever stored): 0 colour-invert enable,
  1 colour-invert parameter check, 2 colour-invert parameter, 3 update, 4 write data, 5 read data — all empty [R: slot
  numbers from the Drv callers; names from the source enum order `E_HAL_QUERY_OSD_CI_*`, S: source `hal_osd_if.h`].
- Cover table `gCovpfForSet`: one slot, never filled.

### 4.10 Debug interface `/proc/mrgn/` [R]
Created once by `RgnSysfsInit`, mode 0444 although writers exist. Every file (seq_file readers) starts and ends with the
line `=====================RGN Info=====================`.
- **`dbglv`**: `dbglv=%hhx` (the byte `gbrgndbglv`: bit 0 GOP / cover traces, bit 1 debug-level "Fail ID", bit 2 register
  traces and "Not need to exec" / "Not support" prints, bit 3 every masked write printed). The writer `rgn_dbglv_store`
  sets it.
- **`gop`** — for each HAL GOP 0–9 (all ten, initialised or not): `----------------------GOP_%d----------------------`, then
  the header `%-8s%-8s%-6s…` = `BindID CKeyEn R G B SrcX SrcY SrcW SrcH DstX DstY DstW DstH` and one row
  `%-8s%-8d0x%-4x0x%-4x0x%-4x%-6d×8`: BindID = the name of the first MHAL type t (0–11) with `geGopIdFromOsd[t]` = this GOP
  (`RgnOsdTransId`), names 0 Port0, 1 Port1, 2 Port2, 3 Port3, 4 DIP, 5 LDC, 6 Port4, 7 Port5, 8 CUR0, 9 UI0, 10 CUR1, 11 UI1,
  no match `UNKNOWN`; CKeyEn / R / G / B = cache 0x18–0x1B; Src* / Dst* = cache 0x08–0x16. Then a blank line, the header
  `GwinId Fmt MemH MemAddr XOft X Y W H AType CVal AVal0 AVal1` (`%-8s%-9s%-6s%-12s%-6s×9`) and one row per GWIN 0–7 **whose
  `bGwinEn` is set**: `%-8d%-9s%-6d0x%-10x%-6d%-6d%-6d%-6d%-6d%-6d0x%-4x0x%-4x0x%-4x` = g, format name, `u32MemPitch`,
  `u32BaseAddr`, `u16Base_XOffset`, display window x / y / w / h, `eAlphaType`, `*p8ConstantAlphaVal`,
  `*p8Argb1555Alpha0Val`, `*p8Argb1555Alpha1Val`. Format names by Drv code: 1 RGB565, 2 ARGB4444, 4 I8, 5 ARGB8888,
  6 ARGB1555, 9 UV8Y8, 13 I4, 14 I2, anything else `UNKNOWN`. CVal and AVal0 always show the same byte (the shared pointer
  target of §4.8). The writer takes `csc …` (§4.8), without effect.
- **`osd`** — header `%-7s%-4s%-7s%-8s` = `OSDId En bUsed BindId`; one row per MHAL OSD id 0–11 that `_DrvOsdTransId` accepts
  (all twelve): the name (same list as `gop`), `bOsdEn` of the mapped HAL OSD id (`%-4d`), bUsed = bit t of `geGopOsdFlag`
  (0 / 1), BindId = one ASCII digit `'0' + geGopIdFromOsd[t]`, or `UNBIND` when that entry is 10 (a two-byte string built on
  the stack; `RgnDebugOsdShow`, disassembly `.text` 0xa7f6c, 0xa7fe6–0xa800a).
- **`cover`** — for each HAL cover id 0–6: `----------------------COVER_%s----------------------` with the label 0 Port0,
  1 Port1, 2 Port2, 3 Port3, 4 DIP, 5 LDC, 6 Port4; header `BwinId X Y W H R G B` (`%-8s%-6s×7`); one row per window 0–3
  **whose `bBwinEn` is set**: `%-8d%-6d%-6d%-6d%-6d0x%-4x0x%-4x0x%-4x`. The labels do not match the id map (HAL 4 = PORT4,
  5 = PORT5, 6 = DIVP, §4.2), so the section titled "DIP" shows the VPE port-4 engine, "LDC" the port-5 engine and "Port4"
  the DIVP engine [I: label table against the id map, both read].

### 4.11 Differences from the u-boot mercury6 source (behaviour, not line numbers) [R]
1. **`_DrvGopTransGwinId`** — source: any GOP accepts GWIN 0–7; binary: GWIN < 8 for HAL GOPs 0–5, GWIN < 1 for the display
   GOPs 6–9. Every GWIN-level Drv call (enable, format, window, pitch, base, X offset, alpha) on a display GOP with GWIN ≥ 1
   fails with "Fail ID"; the MHAL call still returns 0 for the MHAL functions that ignore the Drv result (`GopGwinEnable` /
   `Disable`, `GopGwinSetWindow`, `GopGwinSetBuffer`, `GopGwinSetBufferXoffset`, `GopSetAlphaType`,
   `GopSetArgb1555AlphaVal`); `GopGwinSetPixelFormat` returns −1.
2. Type-split init (`GopInitByType` / `DeinitByType`, `DrvGopRgnInitByType`, `HalGopTransGopIdToType`) with a two-bit
   refcount in `geRgnInit` (value 1 cover, 2 GOP or GOP type 0, 4 GOP type 1), so fbdev (display GOPs) and mi_rgn (SCL GOPs)
   init and deinit independently; the source has only `GopInit` / `GopDeinit` for all GOPs. `MHAL_RGN_GopDeinit` keeps the
   source order (clear the bit, flag 0xF5F, id map, Deinit if last, `DrvGopClkUnregister`, `DrvRgnUpdateReg`).
3. Per-GOP output format `geOutFormat[]` + `MHAL_RGN_GopSetOutputFormat`; the source hard-codes YUV for SCL / LDC and RGB
   for the display GOPs in `_HalGopGetBindMode`.
4. GOP init writes bank 0x1137 reg 0x21 = 0 (DISP GP_CTRL).
5. CSC: `_tGopLocCscSetting`, `DrvGopInitCscConfig` / `SetCscConfig`, `MHAL_RGN_GopSetCscConfig` and the proc `csc` writer
   exist but do nothing (empty HAL slot); `DrvGopRgnInit` (legacy all-GOP init) calls `DrvGopInitCscConfig(n)` before
   `_DrvGopInit(n)`, and `_DrvGopInit` ends with another `DrvGopInitCscConfig(n)` — the source has neither.
6. The HAL GOP query table has five slots (2 output format and 4 GOP type filled; 0, 1, 3 empty); the source has one.
   Force-update hook: every GOP setter that skips unchanged values (`DrvGopRgnSetEnable`, `UpdateBufferXoffset`,
   `UpdateBase`, `SetMemPitchDirect`, `SetSrcFmt`, `SetAlphaBlending`, `SetColorKey`, `SetStretchWindowSize`) also asks
   `_DrvGopGetForceUpdate` (slot 3) and writes anyway on 1; the slot is empty, so the behaviour equals the source. OSD and
   cover setters have no such hook (as in the source).
7. **`MHAL_RGN_CoverDeinit` order** — source: re-init the covers with direct writes, clear the cover bit of `geRgnInit`,
   `DrvRgnDeinit()` if no user is left, then `DrvRgnUpdateReg()`; binary: re-init, **`DrvRgnUpdateReg()` first**, then clear
   the bit and `DrvRgnDeinit()` if it was the last user [R]. [I: the cover re-init runs with `DrvRgnUseCmdq(0)` (direct
   RIU), so the early flush matters only for writes another caller had queued.]
8. Clock gate: the source also opens the display GOPs' gate (CLKGEN 0x1038 reg 0x52, `REG_DISP_TO_GOP_CLK_BASE` 0x1038A4);
   the binary skips clock handling for GOPs 6–9 entirely (SCL 0x1038C8 and LDC 0x1038A6 as in the source). GOP 5's LDC / DIP
   choice follows `geGopOsdFlag` at run time in both.
9. Prints: `DrvGopSetMirror` / `DrvGopSetFlip` return 1 silently in the source, the binary also prints "Not support" when
   `gbrgndbglv` bit 2 is set; `DrvGopUpdateBase` with a failing `_DrvGopTransId` returns 0 silently in the source, the binary
   prints a debug-level "Fail ID" when bit 1 is set and returns 0; `_MHAL_RGN_GopTransId` prints `"%s %d: ID %lx %x"` (source
   `"%s %d: ID %x %x"`) under bit 2 — format letter only.
10. Placement only: the source's `geRgnInit` sits at `geGopIdFromOsd + 0x30` and the palette's static `bForceClk[]` at
    `geGopIdFromOsd + 0x34`; the `_MHAL_RGN_OsdTransId` / `_DrvOsdTransId` calls inside the colour-invert and GWIN enable /
    disable MHAL functions are inlined as "id ≠ 12"; the source's switch-based bank lookups are tables with the same values
    (cover banks 0x1214–0x1219, OSD banks as §4.2); `HalRgnWrite2Byte` maps RIU at 0xFD000000 (kernel) where u-boot uses
    0x1F000000.

The `__LINE__` arguments of the debug prints are ~0x20–0x40 higher in the binary throughout; with the items above read, no
Drv / MHAL function hides another difference [R: all 85 shared Drv / MHAL functions read].

## 5. fbdev.ko — Linux framebuffer on the display GOPs

### 5.1 Build and source match [R]
<!-- ev:rgn-gop.5.1 -->
SigmaStar "misc_fbdev", the MI build 2022-07-21 (`project_commit.7251436 sdk_commit.80327e1`, the same as the mi_*.ko),
license "GPL v2", no `depends` on mi_* in modinfo, but it imports mhal and mi_sys symbols, so it loads after them (insmod
order: `rootfs-customer.md` §3.2). 92 functions, 85 named and 7 unnamed. The closest public source is the older jockyw2001/sdk
2018 `misc/fbdev/mstar_fbdev/` (MStar / SigmaStar SDK): after renaming `mstar`→`sstar`, the named functions exist there
(`sstar_fb_ioctl/mmap/set_par/pan_display/check_var/setcolreg/blank`, `sstar_FB_SetGwinInfo/SetStretchWin/EnableGwin/
SetDst/BeginTransaction/EndTransaction/...`, `_fb_hwcursor_*`, iniparser) except the 20 of §5.5. No newer public copy was
found (GitHub code search on its unique strings: only this repo). The 2018 source drives the GOP through Utopia
(`apiGOP.h`, `apiGFX.h`, `apiPNL.h`), the binary through `MHAL_RGN_Gop*` / `MHAL_DISP_*` (source `#include`s against the
binary imports): the adapter layer was rewritten, so no compiled comparison was made and the module is specified from the
code directly (literal pools and relocations resolved). The user-space ABI header with the same ioctl set (older struct sizes in one place, §5.3, §5.8):
the SSD20x `sstarFb.h` (GuoFlying/ssd2xx-demo).

### 5.2 Parameters, `fbdev.ini` and the ini parser [R]
- **`default_config_path_file`** (charp, default `/config/fbdev.ini`; the camera's boot passes the same,
  `firmware-image.md` §2): `filp_open` + `vfs_read` of the whole file in `init_module`; "read /config/fbdev.ini failed!"
  aborts the probe.
- **`default_reserved_mem_name`** (charp, default **"fb"**, `.data+0x28`): used once, §5.6.
- **`misc_module_version`** (charp, read-only version string).
- `/config/fbdev.ini` on the camera (dump `miservice/fbdev.ini`): `[FB_DEVICE]` FB_HWLAYER_ID 1, FB_HWWIN_ID 0,
  FB_HWLAYER_DST 3 (OP0), FB_HWWIN_FORMAT 5 (ARGB8888), FB_HWLAYER_OUTPUTCOLOR 0 (RGB), FB_WIDTH 1600, FB_HEIGHT 900,
  FB_TIMMING_WIDTH 1920, FB_TIMMING_HEIGHT 1080, FB_MMAP_NAME E_MMAP_ID_FB, FB_BUFFER_LEN 8192 (KiB); `[FB_CURSOR]` layer 0,
  format 6 (ARGB1555), output YUV, mmap E_MMAP_ID_HW_CURSOR; `[LAYER_ZORDER]` `LAYER_ZORDER0`…`4`. Memory names are looked
  up in `/config/mmap.ini` via `MI_SYSCFG_GetMmapInfo`, with an MMA fallback (§5.6).
- The iniparser copy (`alloc_and_init_ini_tree`, `get_section`, `get_key_value`, `foreach_section/key`, `get_section_num`,
  `get_key_num`, `dump_ini`, `release_ini_tree`, `print_section/key_value`; two unnamed helpers) is the 2018 `iniparser.c`
  (section `[name]` lines, `key = value` pairs, `#` comments). Keys read: `FB_HWLAYER_ID`, `FB_HWWIN_ID`, `FB_HWLAYER_DST`,
  `FB_HWWIN_FORMAT`, `FB_HWLAYER_OUTPUTCOLOR`, `FB_WIDTH`, `FB_HEIGHT`, `FB_TIMMING_WIDTH`, `FB_TIMMING_HEIGHT`,
  `FB_MMAP_NAME`, `FB_BUFFER_LEN` (KiB) for `[FB_DEVICE]`, the layer / format / output / mmap keys of `[FB_CURSOR]`, and the
  five `LAYER_ZORDERn`.

### 5.3 Device nodes and ioctls [R]
<!-- ev:rgn-gop.5.3 -->
`/dev/fb0` (GUI layer) and fb1 (hardware cursor) via `register_framebuffer`; `fb_ops` = `sstar_fb_open/release/check_var/
set_par/setcolreg/blank/pan_display/fillrect/copyarea/imageblit(cfb_*)/mmap/ioctl/destroy`. The standard ioctls go through
fbmem; the driver's own (type 'F') are handled in `sstar_fb_ioctl` (switch constants resolved with
 →). The handler first rejects type ≠ 'F'
(`-ENOTTY`) and size > 0x34 (`-EINVAL`), and copies the argument in for every write-direction command except
`FBIO_WAITFORVSYNC`.

| cmd | name (sstarFb.h) | size in binary | header size |
|---|---|---|---|
| 0x40044620 | FBIO_WAITFORVSYNC (Linux standard) → `sstar_FB_WaitForVsync` | 4 | — |
| 0x80084660 / 0x40084661 | FBIOGET/SET_SCREEN_LOCATION (`MI_FB_Rectangle_t` 4×u16) | 8 | 8 |
| 0x80014662 / 0x40014663 | FBIOGET/SET_SHOW (MI_BOOL) | 1 | 1 |
| 0x80074664 / 0x40074665 | FBIOGET/SET_GLOBAL_ALPHA (§5.8) | **7** | 6 |
| 0x80044666 / 0x40044667 | FBIOGET/SET_COLORKEY (`MI_FB_ColorKey_t`) | 4 | 4 |
| 0x80344668 / 0x40344669 | FBIOGET/SET_DISPLAYLAYER_ATTRIBUTES (`MI_FB_DisplayLayerAttr_t`) | 52 | 52 |
| 0x80344670 / 0x40344671 | FBIOGET/SET_CURSOR_ATTRIBUTE (`MI_FB_CursorAttr_t`, §5.7) — compared in the binary as `0x80344668 + 8` / `0x40344669 + 8` | 52 | 52 |

`FBIOSET_SCREEN_LOCATION` turns the timing follow (§5.5) off before it moves the window. Colour formats YUV422 (9) and 12
reject colour keys ("can not set colorkey prop").

### 5.4 Layers and what it does in hardware [R]
fbdev has no MMIO of its own; everything goes through mhal and mi_sys. An fb layer id L (ini `FB_HWLAYER_ID`) is MHAL GOP
type **L + 8** (clamped to 9 when L + 8 > 11): the GUI fb0 is layer 1 = MHAL type 9 DISP0_UI = HAL GOP 7 (banks
0x11A4–0x11A6); the cursor layer 0 = type 8 DISP0_CUR = HAL GOP 6. Its GOP writes go straight to RIU (no CMDQ for DISP,
§4.3) and are latched by GOP reg 0x7F.

| step | calls |
|---|---|
| probe / init | `MHAL_DISP_DeviceGetHwCount`, `MHAL_DISP_GetDevIrqById` + `CamOsIrqRequest` (vsync ISR, §5.5), `MHAL_DISP_DeviceGetInstance`; ini parse; `MI_SYSCFG_GetMmapInfo`; `register_framebuffer` ×2 |
| layer init | `MHAL_RGN_GopInitByType(1)` — initialises all four display GOPs (`sstar_FB_DeinitContext` undoes it with `GopDeinitByType(1)`, §5.5); `GopSetOutputFormat`, `GopSetAlphaType`, `GopSetAlphaZeroOpaque` |
| set_par / pan | `MHAL_RGN_GetChipCapability` (alignment, `sstar_FB_GetAlignment`), `GopGwinSetPixelFormat`, `GopGwinSetBuffer` (phys = mmap base + yoffset × stride), `GopGwinEnable/Disable`; `mi_sys_FreeMMAPReservedArea` on the first pan (§5.6) |
| device config | `MHAL_DISP_DeviceSetConfig` = INTERCFG **COLORID** (`eType` 2, `u8ColorId = enable ≠ 0`) for the device of the layer: it selects which of the two OP2 colour engines gets the device's CSC settings; nothing is routed (meaning: `mhal-disp.md` §6.3, §12.1) |
| scaling | stretch window 1600×900 → the current output timing via `GopSetBaseWindow` (DISP_UI up-scales, §4.4) |
| timing follow | the vsync ISR reads `MHAL_DISP_DeviceGetDisplayInfo`; on a timing change `GOPTimingSync_WorkHandler` re-applies `GopSetBaseWindow` — this keeps the GUI full-screen when HDMI switches 1080p ↔ 4K |
| colour key / palette | `GopSetColorkey`, `GopSetPalette`, `GopSetArgb1555AlphaVal` |
| memory | `mi_sys_MMA_Alloc` / `mi_sys_Vmap` at open when `mmap.ini` has no area, `mi_sys_Miu2Cpu_BusAddr` for mmap |

### 5.5 Functions absent from the 2018 source [R]
20 by name check (the binary's named functions not in jockyw2001 2018):

| function | behaviour |
|---|---|
| `_sstar_FB_VsyncISR(irq, ctx)` | registered per DISP device (`CamOsIrqRequest` in `sstar_FB_Init`); `MHAL_DISP_DeviceGetInstance(ctx.dev)` (none → "[_sstar_FB_VsyncISR 191]" error); atomically increments the device's vsync counter (ctx+0x38) and `wake_up`s its wait queue; reads `MHAL_DISP_DeviceGetDisplayInfo`; for each of the 4 auto-sync slots that belongs to this device and is enabled, when the stored output width / height differ from the current timing: store them, keep the horizontal offset (timing width − width), `queue_work(system_wq, GOPTimingSync_WorkHandler)` |
| `GOPTimingSync_WorkHandler(work)` | for slot gop < 4: copy the new timing width / height into the GOP's base-window record and `MHAL_RGN_GopSetBaseWindow(gop + 8, &window, &timing)` |
| `sstar_FB_AutoSyncOPTimingON(gop, timing*)` / `OFF(gop)` | arm a slot: `INIT_WORK(GOPTimingSync_WorkHandler)`, slot.gop = gop, enabled = 1, timing pointer; OFF clears `enabled` |
| `sstar_FB_WaitForVsync(gop)` | device = gop >> 1 (0 when out of range); sleeps (interruptible) until that device's vsync counter changes; backs `FBIO_WAITFORVSYNC` |
| `sstar_FB_SetOutputColor(layer, yuv)` | `MHAL_RGN_GopSetOutputFormat(gop, yuv != 0)` |
| `sstar_FB_SetPalette(layer)` | `MHAL_RGN_GopSetPalette(gop)` (palette from the fb cmap) |
| `sstar_FB_SetAlphaOpaque(layer, on)` | `MHAL_RGN_GopSetAlphaZeroOpaque(gop, on, 0, 0)` — ARGB1555 pixels with A = 0 shown opaque |
| `sstar_FB_SupportSetArgb1555Alpha(layer)` | true for layers 0 and 2 (the GOPs that take per-value alpha for ARGB1555) |
| `sstar_FB_SupportScaleUp(layer)` | true for layers 1 and 3 (the DISP_UI GOPs) |
| `sstar_FB_GetAlignment(layer, fmt, &align)` | layer < 3: `MHAL_RGN_GetChipCapability(type[layer], mhalfmt[fmt − 1])` with type = {5, 3, 4} [I: PITCH_ALIGN, WIDTH_ALIGN, XPOS_ALIGN of §4.6] and, by fb colour format (fmt 1–9): RGB565 → 5, ARGB4444 → 1, 3 → 8, I8 → 4, ARGB8888 → 6, ARGB1555 → 0, 7 → 8, 8 → 8, YUV422 → 7, other → 8 (`.rodata+0x36`, `+0x3F`); false when the capability is −1 |
| `sstar_FB_DeInit()` | frees the two vsync IRQs (`CamOsIrqFree`) if requested |
| `sstar_FB_DeinitContext()` | `MHAL_RGN_GopDeinitByType(1)` |
| `sstarfb_suspend` | `DeinitContext` |
| `sstarfb_resume` | `InitContext(1)`; for every fb with memory: re-apply var / fix (`fb_reapply_var` (0x10fa0)), `check_var`, window, `CreateFBInfo`, `EnableGwin(shown)`, `_fb_hwcursor_show(1)` |
| `sstar_FB_GOP_SetForceWrite` | empty |
| `_fb_hwcursor_show(on)` | `EnableGwin(cursor layer, on)` when the cursor is initialised |
| `MI_DEVICE_GetSelfDir` | `proc_mkdir("fb", MI_DEVICE_GetProcModuleDir())` |
| `init_module`, `cleanup_module` | new module glue (ini parse, `MI_SYSCFG_GetMmapInfo`, chrdev / class / device, `register_framebuffer`) |

### 5.6 Frame-buffer memory and the boot-logo release [R]
<!-- ev:rgn-gop.5.6 -->
- `E_MMAP_ID_FB` (and `E_MMAP_ID_HW_CURSOR`) are absent from the camera's `mmap.ini`, so
  `sstar_fb_open` allocates `FB_BUFFER_LEN` KiB with `mi_sys_MMA_Alloc` at the first open, maps it (`mi_sys_Vmap`), clears it
  and sets up up to three buffers (`yres_virtual = min(3, len / (w·h·bpp)) · h`) [R code; I: this path is taken because of
  the missing section — confirmed live: fb0's memory comes from the MMA heap, R live 2026-10-26]. Live, fb0 reports
  `smem_len` 8519680 B with FB_BUFFER_LEN 8192 and 16908288 B after a reload with 16384 — 128 KiB more than the setting in
  both cases [R live 2026-10-26]; where the extra 128 KiB comes from is not read [I: the cursor's two 64 KiB icon buffers,
  §5.7]. `rmmod fbdev` and `insmod` with another `FB_BUFFER_LEN` work on the running system, `/dev/fb0` keeping its node
  [R live 2026-10-26].
- **Boot-logo release**: on the **first `pan_display`** after `EndTransaction`, when the flag at `.data+0x24` (initially 1)
  is set, fbdev calls `mi_sys_FreeMMAPReservedArea(default_reserved_mem_name)` and clears the flag — once per module
  lifetime; no other read of the variable exists (scan of all `.data` base loads). That frees the `mmap_reserved_config`
  entry of that name (the boot-logo frame kept by U-Boot / the kernel) to the page allocator — the same operation as
  `logo2free <name>` written to the mi_sys file `reserve_2_free` (`mi_sys-2-interfaces.md` §10.1). On the camera the bootargs reserve `mmap_reserved=fb,…,sz=0x800000` and the
  boot log shows «Freeing fb memory: 8192K» [R live 2026-10-27]; fb0 itself never lives in that area.

### 5.7 Hardware cursor (fb1) [R]
`FBIOGET_CURSOR_ATTRIBUTE` 0x80344670 / `FBIOSET_CURSOR_ATTRIBUTE` 0x40344671 take a 52-byte `MI_FB_CursorAttr_t`. Binary layout
(offsets in the 52 bytes): 0 XPos, 4 YPos, 8 HotSpotX, 12 HotSpotY (u32 each), 16 `stAlpha` (7 bytes, §5.8), 23 `stColorKey`
(4), 27 `bShown`, 28 `stCursorImageInfo` {28 Width, 32 Height, 36 Pitch, 40 eColorFmt, 44 data pointer}, 48
`u16CursorAttrMask` [R: GET builds exactly these offsets].
- **GET**: "fb%d hwcursor has not been initialized!" and an error if the cursor layer was never set up; else returns
  position, hotspot, alpha, colour key, shown flag, icon width / height / pitch / format; data pointer and mask 0.
- **SET** (mask bits ICON 1, POS 2, ALPHA 4, SHOW 8, HIDE 0x10, COLORKEY 0x20; mask 0 → return 0); an ICON larger than
  **128 × 128** → "Icon size … larger than max size[128,128]" and an error. Under the cursor mutex: the first use initialises
  the cursor layer — `sstar_FB_Init` at the current output timing, mirror off, output colour from the ini (YUV), MIU
  select, colour key off, blending const 0xFF, multi-alpha off, hidden; for ARGB1555 layers that support it Alpha0 = Alpha1
  = 0xFF; maps the cursor memory (`mi_sys_Vmap(phys, 0x20000)`), two **64 KiB icon buffers** (A, B), clears them. Layer 0
  does not scale up → stretch window = output timing, H / V scale 1:1, H start from the timing. HIDE → `EnableGwin(0)`. ICON
  → when size or format changed, recreate the GWIN info (`DestroyFBInfo` / `CreateFBInfo` with the width rounded down to
  even), copy `Height` rows of `Width × bpp` bytes from the user `data` pointer (`copy_from_user`, row stride = the layer
  pitch) into the **back buffer**, compute the visible rectangle with the hotspot and screen clipping (`fb_cursor_clip` (0x10dbe)),
  point the GWIN at the back buffer and flip the buffer index. POS → recompute the rectangle at the new position (from the
  front buffer when there is no new icon). SHOW → `EnableGwin(1)`. COLORKEY → transparent colour on / off,
  `SetTransClr_8888(0xFF:R:G:B)`. ALPHA → as `FBIOSET_GLOBAL_ALPHA` (§5.8). Then one `SetGwinInfo` if the window or buffer
  changed and `EndTransaction`; the new state is stored.
- Cursor memory: `[FB_CURSOR] FB_MMAP_NAME` is looked up in `/config/mmap.ini` ("parse_hwcursorInfo use %s in mmap.ini…");
  without it ("Can not find %s section…" / "did not config memory in fbdev.ini, Use OSD Bottom…") `setHwcursrPhyAddr`
  takes the **last 128 KiB of the GUI framebuffer** (A = end − 128 KiB, B = end − 64 KiB, MIU 1 when the address >
  0x80000000). The camera's `mmap.ini` has no `E_MMAP_ID_HW_CURSOR` section, so the cursor would live at the top of fb0's
  memory [I: the lookup's result path not traced further].
- `CURSOR_SHOW on|off` (§5.9) toggles the cursor GWIN; `_fb_hwcursor_show(1)` runs at each fb open / resume, so an
  initialised cursor reappears with the GUI.
- On the camera: **unused** — kjd_lvgl uses no 'F' ioctl and nothing initialises the cursor, so `_fb_hwcursor_show` does
  nothing.

### 5.8 `MI_FB_GlobalAlpha_t` — 7 bytes [R: `FBIOSET_GLOBAL_ALPHA` handler, cursor GET offsets]

| byte | field | use in `FBIOSET_GLOBAL_ALPHA` (0x40074665) |
|---|---|---|
| 0 | `bAlphaEnable` | 0 → `SetBlending(const, 0xFF)` (opaque), nothing else |
| 1 | `bAlphaChannel` | 0 → constant alpha = byte 5 (+ multi-alpha off when the layer had it); 1 → pixel alpha (`SetBlending(pixel, byte 5)`, multi-alpha on if supported) |
| **2** | **field present only in this build, name unknown** [I: an "alpha-zero-opaque" flag] | ARGB1555 on a layer without per-value ARGB1555 alpha and pixel alpha on: `SetAlphaOpaque(byte 2)`; when set, byte 4 is inverted (~Alpha1) before `SetBlending(const 0, …)` and `SetBlending(pixel, …)` |
| 3 | `u8Alpha0` | ARGB1555 + supported: `SetAlpha0` |
| 4 | `u8Alpha1` | ARGB1555 + supported: `SetAlpha1` |
| 5 | `u8GlobalAlpha` | constant / pixel alpha value |
| 6 | `u8Reserved` | stored only |

The public header has the six fields without byte 2; a same-size call with the header struct shifts Alpha0 / Alpha1 /
GlobalAlpha by one byte. The stored copy lives at par+0x38…+0x3E; a SET equal to it returns 0 without touching the hardware.

### 5.9 Debug commands [R]
Registered at the **first `open`** of any fb (not at load): `mi_sys_RegisterDev` (proc directory `fb` under the MI module
directory via `MI_DEVICE_GetSelfDir`, "MI_Moduledev_RegisterDev Enter", the handle printed as "gGfxDevHdl"), dump callback
`_MI_FBDEV_IMPL_OnDumpDevAttr` (prints every fb's layer, GWIN, buffer, format, alpha, colour key and cursor state), help
callback `_MI_FBDEV_IMPL_OnHelp` (empty), and six `mi_sys_RegistCommand`s (written to the MI proc device file; argument
count as registered):

| command | args | handler | action |
|---|---|---|---|
| `GUI_SHOW` | 2: fb, on/off | `_MI_Fbdev_IMPL_ShowOSD` | fb index (decimal, must be within the registered fb range) → `lock_fb_info`, `EnableGwin(on)` |
| `CURSOR_SHOW` | 1: on/off | `_MI_Fbdev_IMPL_ShowCursor` | cursor GWIN on / off; "The Hwcurosr did not initialize yet!!" when not set up |
| `GUI_SET_CLRKEY` | 4: fb, R, G, B (hex) | `_MI_Fbdev_IMPL_SetClrKey` | colour key on with 0xFF:R:G:B |
| `GUI_DISABLE_CLRKEY` | 1: fb | `_MI_Fbdev_IMPL_DisableClrKey` | colour key off |
| `GUI_SETALPHA_ARGB1555` | 3 | `_MI_Fbdev_IMPL_SetAlpha` | **empty stub, returns 0** |
| `CURSOR_DUMP` | 1: directory | `_MI_Fbdev_IMPL_DumpCursor` | creates `<dir>/CursorData.raw` (`O_WRONLY, O_CREAT, O_TRUNC`, 0777) and writes the 64 KiB icon buffers with `vfs_write`; an error print when the cursor is not initialised |

### 5.10 fb0 as observed live [R: live 2026-10-26]
<!-- ev:rgn-gop.5.10 -->
- The UI layer (GOP 7) lies over every DISP input-port window: an opaque band written into fb0 crossed the video window and
  two picture windows unbroken on the monitor.
- Every open of `/dev/fb0` reports 1600×900 32 bpp again, yet the geometry set last (`FBIOPUT_VSCREENINFO`) stays on the
  screen after the setting process exits. The layer keeps its geometry, palette and contents through a teardown and rebuild
  of DISP and HDMI by another process.
- fb0 accepts 3840×2160 with an 8-bit palette (`FBIOPUT_VSCREENINFO` bpp 8, `FBIOPUTCMAP` with per-entry alpha): 8.3 MB,
  fits the 8 MiB setting; on a 2160p30 monitor a 1-px line stays 1 px in a write-back capture. A 1920×1080 layer on the same
  monitor is up-scaled with smoothing (a 1-px line spreads over 2–3 px). The driver also accepts 3840×2160 ARGB4444 (16.6 MB),
  more than its memory — it does not check `line_length × yres ≤ smem_len`.
- Palette alpha (`FBIOPUTCMAP` transp): written as a × 0x101 it blends exactly (eight white entries 0.12…1.00 measured
  0.11…1.00 over the video at 1440p); written in the high byte only (a << 8, the usual fbdev convention) every entry below
  0xffff shows nothing [R]; so the driver takes the low byte [I].
- Pages: `yres_virtual = 2 × yres` is accepted (ypanstep 1) and `FBIOPAN_DISPLAY` flips — a write-back shows the page panned
  to; the call returns after ~16 ms on a 60 Hz timing and after one refresh (~33 ms) on 30 Hz, i.e. it waits for vsync;
  `FBIO_WAITFORVSYNC` works too. With `FB_BUFFER_LEN` 16384 two 3840×2160 8-bit pages flip on a 2160p30 monitor.
