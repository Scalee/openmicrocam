# isp-6-userspace — libcus3a, libsigma3a, libmi_iqserver, libfbc_decode, isp_api.xml
<!-- ev:isp-6-userspace -->

The user-space side of the ISP that runs inside prog_uvc. **libcus3a** is the CUS3A framework: it registers AE / AWB / AF
algorithms per ISP channel, runs them in its own thread on the frame events of `/dev/isp_fe`, moves statistics and results
between the kernel and the algorithms, and serves the AE / AWB / AF API ids while an algorithm runs; its built-in "Sigma"
adaptor drives libispalgo. **libsigma3a** is an older, unused copy of that adaptor. **libmi_iqserver** is the IQ Tool
service: a TCP / serial server through which SigmaStar's Windows IQ Tool reads and writes ISP parameters and pulls frames.
**libfbc_decode** is a C++ software model of the ISP frame-buffer compressor whose decoder the IQ server uses for compressed
raw frames. **isp_api.xml** is the parameter description the IQ server hands to the tool. Not here: the ISP API ids, the
CUS3A ids (0x2Exx) and their payloads, the API bin format — isp-3-iq-api.md; the AE / AWB / AF algorithms of libispalgo
and their in-kernel copy, the algorithms' structs (`AeInitParam_t`, `AeInput_t`, `AeOutput_t`, `AwbInput_t`,
`AwbOutput_t`), the AE / AWB / AF id handlers — isp-5-3a.md; the IQ file format — isp-4-iq-engine.md §13; the kernel's
statistics engines, `/dev/isp_fe` and the per-frame flow in mhal — isp-2-ispmid-mload-ispscl.md §10, §11, §6; libmi_isp
(the agent table that calls libcus3a, `MI_ISP_EnableUserspace3A`, the wrappers both libraries import) — mi-capture.md §5;
mi_vif's FBC switch `g_bFBCMode` — mi-capture.md §3; the VENC structs the IQ server moves — mi_venc.md; prog_uvc's
start-up, its `-q` flag and the static copies as a whole — prog_uvc.md; the file inventory — firmware-image.md §3.

Tags: **[R]** read from the binary, its disassembly or decompile, a data section or a file, or obtained by running the
vendor's own code in an emulator on the host; **[S]** stated by a source (vendor header, forum, another file
quoted as such); **[I]** inference. A heading tagged [R] means every statement under it is [R] unless marked otherwise.

Binaries (all in, i.e. `/config/lib` on the camera): `libcus3a.so`,
`libsigma3a.so`, `libmi_iqserver.so`, `libfbc_decode.so`; served file `/config/iqfile/isp_api.xml`.
prog_uvc carries static copies of libcus3a, libmi_iqserver and libfbc_decode.

Addresses: libcus3a and libsigma3a — offsets of the `.so`. libmi_iqserver — **file virtual addresses** of the `.so`
(the values `readelf` prints); its static copy in prog_uvc is written "prog_uvc 0x…" (prog_uvc file vaddrs).
libfbc_decode — addresses with image base 0x10000 (file vaddr + 0x10000), except where "file vaddr" is written.

## 1. The parts and how they run on this camera
<!-- ev:isp-6-userspace.1 -->

| part | file, size | on the camera | role |
|---|---|---|---|
| libcus3a | `libcus3a.so`, 26 592 B, stripped, needs only libc [R] | code runs as prog_uvc's static copy ([S]) | CUS3A framework + built-in "Sigma" adaptor; user AE / AWB / AF (§2–§4) |
| libsigma3a | `libsigma3a.so`, 10 048 B [R] | nothing links or `dlopen`s it ([S]) — **unused** | older copy of the Sigma adaptor, ABI-incompatible with this libispalgo (§5) |
| libmi_iqserver | `libmi_iqserver.so`, 50.5 KB [R] | the `.so` is loaded by nothing; prog_uvc's static copy runs when prog_uvc is started with `-q` [R] | IQ Tool service, TCP 9876 and serial (§6) |
| libfbc_decode | `libfbc_decode.so`, 162.7 KB [R] | the `.so` is loaded by nothing; prog_uvc's static copy is called only from the IQ server's raw-picture paths [R] | FBC 1x8 decoder (and the whole compressor model) (§7) |
| isp_api.xml | `/config/iqfile/isp_api.xml`, 164 700 B [R] | read only by the IQ server, served verbatim [R] | ISP parameter description for the IQ Tool (§8) |

### 1.1 User 3A on this camera
<!-- ev:isp-6-userspace.1.1 -->
- [R] prog_uvc's `MI_ISP_CreateChannel` → libmi_isp `MI_ISP_EnableUserspace3A` → libcus3a `CUS3A_Init` +
  `CUS3A_EnableUserspaceAE/AWB/AF` (mi-capture.md §5.2) registers the built-in Sigma adaptor for AE, AWB **and AF** and tells
  the kernel `{1, 1, 1}` (0x2E08). The kernel sets `nFlagCus3A = 0x1A`, so the in-kernel `DoAe` / `DoAwb` never run
  (isp-2-ispmid-mload-ispscl.md §1.1).
- [R] A SCHED_RR thread `3A_Proc_0` waits on `/dev/isp_fe` and per frame runs AE (reads 0x2E01 / 02 / 03 / 1E / 05, writes
  0x2E06), AWB (reads 0x2E07 / 09, writes 0x2E0A — the algorithm itself computes only every third frame) and AF (isp_fe modes 3
  and 2, writes 0x2E21) (§2.4, §2.5, §4). The algorithms are libispalgo's `DoAe` / `DoAwb` / `DoAf` (isp-5-3a.md §1.2), initialised
  from `/config/iqfile/iqfile0.bin`, a symlink to the **IMX307** IQ file (§2.8).
- AF event [R chain; I: that the simple-Bayer path is not taken on this pipeline and the CUS3A bypass mask stays 0]:
  prog_uvc `ST_VideoModuleInit` fills the 0x50-byte `MI_ISP_CreateChannel` attribute with only word 0 = 1 << sensor pad and
  zeros, so the application sets no control bits. ispmid `CameraIsp_InitChannelFmt` (called
  from `MhalCameraOpen`) sets the channel's `nCtrlFlag` (ch+4, bit meanings in isp-2-ispmid-mload-ispscl.md §2.2): if the
  per-channel override word `g_tIspMidHandler[ch + 0x7c]` has bit 31 set, its low 31 bits (bit 31 then cleared); else on the
  channel's first open (ch+0x19778 = 0) **0x7f** (IQ, menuload / WDR, AE, AWB, AF all on); else the old value; then YUV
  input modes (attribute word 5 = 4 / 5) → 10, ch+0x19730 = 0 → AE bit 4 cleared, no bound sensor → bits 4–6 cleared
 ; `CameraIsp_InitIspAttribute` clears bits 4–6 (and the CUS3A bits) on its
  "nIspSimpleBayerMode" path. The only writer of the override word is `CameraDebugSetControlFlag(ch, bit,
  on)`, reached from `ispmid_fwrite` for the text command "ctlflag <ch> <bit> <on>"; "ctlflag" occurs in no
  file of the dump except mhal.ko. So the AF bit 6 is set, and `CameraIsp_EnableChannel3A(ch, nFlagCus3A)` sets the user AF
  event bit (ch+0x196c4 bit 4) because nFlagCus3A bit 4 is set by the 0x2E08 AF = 1 and the CUS3A bypass mask
  (`g_tIspMidHandler+0x33c` bit 2) is clear. The bypass mask is changed only by `CameraIspSetCus3ABypass`, whose callers are
  not traced [I: default 0]. Not confirmed on the camera: ispmid's log line "nCtrlFlag = 0x%x" (printed by `MhalCameraOpen`
  when the ispmid log mask enables it) would show it.
- So the user AF registered by `CUS3A_EnableUserspaceAF` runs every frame [R chain]: `AfInit` pushes AF window / filter
  defaults to the kernel (§4), 0x2E20 fails and is ignored (no VCM driver), and with init positions 0 the AF emits Change =
  1, NextPos = 320 every frame after a 30-frame start delay; its search state machine never runs (isp-5-3a.md §4.9, AF on this
  camera). 0x2E21 is written every frame and the kernel drops it (no VCM driver) — nothing moves.
- [R] AE / AWB / AF API ids from the application are served by libispalgo inside prog_uvc while the algorithm runs and by the
  kernel before (§3).

### 1.2 IQ server on this camera
<!-- ev:isp-6-userspace.1.2 -->
- [R] Nothing in the product uses the service; it starts because `/customer/demo.sh` passes prog_uvc's `-q` flag ([S]), which calls `MI_IQSERVER_Open()`. It listens on TCP **9876** (one client); the serial transport stays off because
  nothing creates `/tmp/isp_uart` (§6.8).
- [R] `MI_IQSERVER_SetCustFunc` and `MI_IQSERVER_SetDataPath` are never called by prog_uvc (prog_uvc has no static copy of
  `SetDataPath` and no call site of either), so command 1 (custom) is inert — the default handler only prints a message and
  returns 0 — and the served file is always `/config/iqfile/isp_api.xml`.
- [R] prog_uvc binds VIF → ISP REALTIME ([S]), so raw injection (API 0x3201, `/tmp/raw_data.raw`) is
  refused (§6.7), and sensor-raw pictures come from the ISP's raw-store DMA, not from VIF buffers (§6.6). [I] Both raw paths
  choose the WDMA source under a REALTIME bind, and the WDMA source never calls `FBD_Execute`, so on this pipeline the FBC
  decoder is not reached even when the tool asks for raw frames.
- [S] (SigmaStar Tiramisu MI_IQSERVER document `iqserver_zh`, same in the 2022-04 SSC35XG set; ISP tuning SOP
  `iq_zh` §14): the documented API is `MI_IQSERVER_Open / Close / SetDataPath / SetCustFunc`; «IQ server目前只能创建一个» — only
  one IQ server can exist; `SetDataPath` names the directory of `isp_api.xml`; `SetCustFunc` passes tool data to an app
  callback. The Tiramisu pages name no port and no transport and describe only an Ethernet connection (the demo `mixer -q`
  starts the server). [S] (the MI_IQSERVER page of a newer SigmaStar family, not Tiramisu): adds
  `MI_IQSERVER_SetProtocol` and `_SetCmdData`, «IQ server默认端口为9876» (default port 9876), and that the server must run in
  the same process as the ISP pipeline and be opened once. [S] (SigmaStar FAE, 2022-08, SSD268G): connecting IQ
  Tool over USB is not done through UVC; the board runs an RNDIS network over USB and the tool connects over it.

## 2. libcus3a — the CUS3A framework [R]

### 2.1 Build, exports, globals
Undefined symbols it expects from the process: `MI_ISP_*` (libmi_isp), `MI_SYS_MMA_*` / `MI_SYS_Mmap` / `Munmap` (libmi_sys),
`CamOs*` (libcam_os_wrapper), and from libispalgo `AeInit` / `DoAe` / `AeRelease`, `AwbInit` / `DoAwb` / `AwbRelease`,
`AfInit` / `DoAf` / `AfRelease`, `DrvAlgo_IF_ApiSet` / `ApiGet`, `IspLoadIqCfg` / `IspFreeIqCfg`. Version strings:
`CUS3A_GetVersion` writes `"CUS3A_V1.5"` and returns 0x10001; `CUS3A_Init` prints `Sigmastar Module mi_cus3a version:
project_commit.7251436 sdk_commit.80327e1 build_time.20220721223309`, `Jul 21 2022`.

46 exported functions + 4 objects. Globals (all in .bss unless marked, sized for **2 devices × 16 channels**):

| symbol | size | content |
|---|---|---|
| `gCus3AChInfo` | 0x1300 = 2 × 0x980 | per device 16 channel records of 0x98 (§2.2) |
| `gCus3AMutexLock` | 0x38 = 2 × 0x1c | one CamOs mutex per device — serialises the thread, the registration calls and the API ctrl path |
| `g_thread` | 8 | one pthread per device |
| `.data+0x1619c[2]` | 8 | per-device stop flag (set by `CUS3A_Release`, polled by the thread) |
| `.bss 0x205ec[2]` | 8 | per-device init reference count |
| `.bss 0x3451c` | 2 × 0x11c0 | the built-in Sigma adaptor contexts, 0x11c per (device, channel) (§2.7) |
| statistics scratch | | **one** set for the whole process: AE averages 0x205f4 (46 092 B incl. the 8-byte {BlkX, BlkY} header), histogram 0 0x2ba00 (256 B), histogram 1 0x2bb00 (256 B), RGB-IR histogram 0x34308 (512 B), AWB averages 0x17ee4 (34 568 B), AWB short-frame averages 0x2bc00 (34 568 B), AF statistics 0x162e4 (0x1c00 B), AF info record 0x34508 |

The scratch buffers are shared by both devices while the lock is per device; a second ISP device running 3A at the same time
would race on them [I] (this camera has one device).

### 2.2 Channel record and algorithm interface
Channel record, 0x98 bytes at `gCus3AChInfo + dev·0x980 + ch·0x98`:

| offset | type | meaning |
|---|---|---|
| +0x00, +0x01, +0x02 | u8 ×3 | AE / AWB / AF "registered through the deprecated per-type API" — the bytes the deprecated `CUS3A_AE/AWB/AFRegInterface` send to `MI_ISP_CUS3A_Enable` |
| +0x08 + 0x40·a | 0x3c | adaptor slot `a` (0 = built-in SigmaStar 3A, 1 = customer 3A): AE interface at +0x08, AWB interface at +0x1c, AF interface at +0x30, each `{pdata, init, release, run, ctrl}` (5 words, below) |
| +0x84 / +0x85 / +0x86 | u8 | current adaptor index for AE / AWB / AF |
| +0x87 / +0x88 / +0x89 | u8 | AE / AWB / AF "running" = the algorithm's `init` was called and `release` not yet |

The algorithm interface, one per type (layout [R]; field names after the public SigmaStar `ISP_AE_INTERFACE` family [I]):
```c
typedef struct {
    void *pdata;                                         /* +0x00 algorithm context, passed back as 1st argument */
    int  (*init)(void *pdata, void *init_param);          /* +0x04 AE: ISP_AE_INIT_PARAM (0x6c); AWB: none (NULL); AF: ISP_AF_INIT_PARAM (0x118) */
    void (*release)(void *pdata);                         /* +0x08 */
    void (*run)(void *pdata, const void *info, void *result); /* +0x0c once per frame event */
    int  (*ctrl)(void *pdata, int ctrl_id, void *param);  /* +0x10 API id or 0x69 = camera re-open */
} ISP_ALGO_INTERFACE;   /* 0x14 */
```
AWB `init` is called with `pdata` only (no init parameter; `ProcRoutine` passes one argument).

### 2.3 Registration and life cycle
<!-- ev:isp-6-userspace.2.3 -->
- `CUS3A_Init(dev)`: reference-counted per device; the first call zeroes the device's 16 records, clears the stop flag, creates
  the mutex and a detached thread `Cus3A_ProcRoutine(dev)` named `3A_Proc_<dev>`, scheduling `SCHED_RR` (policy 2) priority 95.
  Thread creation failure → the count is undone, the mutex destroyed, the pthread error returned.
- `CUS3A_Release(dev)`: decrements; at 0 sets the stop flag, `pthread_join`s the thread, zeroes the records, destroys the
  mutex. Algorithms still running are **not** released here (only the thread's own exit path closes `/dev/isp_fe`).
- `CUS3A_RegInterfaceEX(dev, ch, adaptor, type, iface)` (type 0 AE, 1 AWB, 2 AF; other types: no-op, returns 0): under the
  lock, if `adaptor` is the type's current adaptor and the algorithm is running → its `release(pdata)` and running = 0; then
  the slot (`+0x08 / +0x1c / +0x30 + 0x40·adaptor`) is overwritten with `*iface`, or zeroed when `iface` is NULL. Returns 0
  always.
- `CUS3A_SetAlgoAdaptor(dev, ch, adaptor, type)`: no-op if unchanged; else releases the running algorithm of the old adaptor,
  marks it not running and stores the new index. The thread initialises the new adaptor at the next frame event (§2.4).
- `CUS3A_GetAlgoAdaptor(dev, ch, type)` → byte `+0x84 + type`.
- `CUS3A_EnableUserspaceAE/AWB/AF(dev, ch, on)`: on → `Sigma3AGetAe/Awb/AfInterface(dev, ch, &iface)` (§2.7),
  `CUS3A_RegInterfaceEX(dev, ch, 0, type, &iface)`, then `MI_ISP_CUS3A_Enable(dev, ch, {1, 1, 1})` (0x2E08, three bytes from
  .rodata 0x5a20) — every call enables all three in the kernel; returns the RegInterfaceEX result. off → `RegInterfaceEX(dev,
  ch, 0, type, NULL)` only (the kernel enable bits stay set).
- `CUS3A_CreateChannel(dev, ch)` = EnableUserspaceAE, AWB, AF with on = 1 (stops at the first failure, printing `Failed to
  active native AE/AWB/AF.`); `CUS3A_DestroyChannel` = the three with on = 0. libmi_isp's `MI_ISP_EnableUserspace3A` calls
  `CUS3A_Init` and the three `CUS3A_EnableUserspace*` directly (mi-capture.md §5.2) — the same effect.
- Deprecated API: `CUS3A_RegInterface(dev, ch, ae, awb, af)`: for each non-NULL interface `RegInterfaceEX(dev, ch, 1, type,
  iface)` + `SetAlgoAdaptor(dev, ch, 1, type)`, for each NULL one `SetAlgoAdaptor(dev, ch, 0, type)` (back to SigmaStar); then
  `MI_ISP_CUS3A_Enable {1,1,1}`; returns −1 if the enable fails. `CUS3A_AERegInterface` / `AWBRegInterface` /
  `AFRegInterface(dev, ch, iface)` release the running algorithm of the **current** adaptor, zero that adaptor's slot, copy
  `iface` into it (NULL: leave zeroed), set running = 0 and the "registered" byte (+0 / +1 / +2) to 1 (0 for NULL), then send
  `MI_ISP_CUS3A_Enable` with the three registered bytes — so with this API only registered types are switched to user space.
- `CUS3A_GetAlgoStatus(dev, ch, u32 out[3])` → running flags +0x87 / +0x88 / +0x89 as 0 / 1. prog_uvc polls it before
  loading the API bin ([S]); ids reach the user algorithm only while it runs (§3.1).

### 2.4 The 3A thread `Cus3A_ProcRoutine(dev)`
```
fd = open("/dev/isp_fe", O_RDWR)
while (!stop[dev]):
    write(fd, {dev=0, mode=0, 0, 0, 0}, 20)          # command: "event" read mode (the dev word is always 0 here)
    if poll(fd, POLLIN, 1000 ms) < 1: continue        # timeout: loop; poll error: print "err exit" and leave
    read(fd, ev, 20)  ev = {frameMask, aeMask, awbMask, afMask, reopenMask}
    for ch in 0..15:  (lock per edge)
        AE:  aeMask bit set, AE not running, slot init != NULL →
                 MI_ISP_CUS3A_GetAeInitStatus (0x2E04, 108 B); failure → print "Sensor have NOT been initialized!", stay stopped;
                 else init(pdata, copy of the 108 bytes with Size = 0x6c); running = 1
             (running is also set to 1 when init is NULL)
             aeMask bit clear while running → release(pdata), running = 0
        AWB: awbMask bit set, not running → init(pdata) if present; running = 1.   clear while running → release(pdata), running = 0
        AF:  afMask bit set, not running → §2.5 AF init (always marks running);   clear while running → release(pdata), running = 0
        reopenMask bit → print "CUS3A camera reopen CH%d"; ctrl(pdata, 0x69, NULL) for each running algorithm
    for ch in 0..15 with frameMask bit set:  (lock held for each algorithm)
        AE running  → ProcAE (§2.5)
        AWB running → ProcAWB (§2.5)
        AF running and AF run != NULL → ProcAF (§2.5)
close(fd); print "[Cus3A_ProcRoutine] exit."
```
The stop flag is checked between channels. The thread never sleeps other than in `poll`, so the algorithms run back-to-back
in the order AE → AWB → AF on the frame-start wake-up. The isp_fe command modes, the event masks and when the kernel raises
each event: isp-2-ispmid-mload-ispscl.md §11.

### 2.5 One frame: what is read, what the algorithm gets, what is written
**ProcAE** (static `0x1e40`; needs AE running and `run` ≠ NULL):
1. `MI_ISP_AE_GetAeHwAvgStats` (0x2E01) into the AE scratch — failure prints `Cus3A_ProcAE,919 error!` and skips the frame;
   `MI_ISP_AE_GetHisto0HwStats` (0x2E02) — failure skips; `GetHisto1HwStats` (0x2E03) and `GetRgbIrHistoHwStats` (0x2E1E) —
   failure zero-fills that buffer and goes on; `MI_ISP_CUS3A_GetAeStatus` (0x2E05, 65-byte packed `CusAEInfo_t`) — failure
   skips.
2. Builds the algorithm input **`ISP_AE_INFO` (0x48, natural alignment)**: +0x00 Size = **0x41** (the size of the packed
   kernel struct, not of this one), +0x04 hist1 → histogram-0 scratch, +0x08 hist2 → histogram-1 scratch, +0x0c AvgBlkX,
   +0x10 AvgBlkY, +0x14 avgs → AE scratch + 8 (block data after the {BlkX, BlkY} header), +0x18 Shutter, +0x1c SensorGain,
   +0x20 IspGain, +0x24 ShutterHDRShort, +0x28 SensorGainHDRShort, +0x2c IspGainHDRShort, +0x30 PreAvgY, +0x34 u8 HDRCtlMode,
   +0x38 FNx10, +0x3c CurFPS, +0x40 PreWeightY, +0x44 pRGBIRHist → RGB-IR scratch. (Values from 0x2E05; the kernel packs
   FNx10 at +0x35 etc.)
3. Pre-fills **`ISP_AE_RESULT` (0x3c)**: +0x00 Size 0x3c, +0x04 Change 0, +0x08 Shutter, +0x0c SensorGain, +0x10 IspGain,
   +0x14 ShutterHdrShort, +0x18 SensorGainHdrShort, +0x1c IspGainHdrShort (all = current values), +0x20 u4BVx16384 0,
   +0x24 AvgY = PreAvgY, +0x28 HdrRatio 0, +0x2c FNx10 = current, +0x30 DebandFPS 0, +0x34 WeightY 0, +0x38 0.
4. `run(pdata, &info, &result)`; then `MI_ISP_CUS3A_SetAeParam` (0x2E06) with the 56-byte kernel `CusAEResult_t` = result
   words +0x00…+0x34 (Size written as 56; result word +0x38 is dropped). Failure prints `Cus3A_ProcAE,1023 error!`.
   If `run` is NULL the current values are written back unchanged with Change = 0.

**ProcAWB** (needs AWB running and `run` ≠ NULL):
1. `MI_ISP_AWB_GetAwbHwAvgStats` (0x2E07) into the AWB scratch, then `MI_ISP_CUS3A_GetAwbStatus` (0x2E09, 41-byte packed
   `CusAWBInfo_t`); either failure prints `Cus3A_ProcAWB,<line> error!` and skips. If HDRMode ≠ 0:
   `MI_ISP_AWB_GetAwbHwAvgStatsShort` (0x2E19) into the short scratch (failure skips).
2. **`ISP_AWB_INFO` (0x2c)**: +0x00 Size 0x2c, +0x04 AvgBlkX, +0x08 AvgBlkY, +0x0c CurRGain, +0x10 CurGGain, +0x14 CurBGain,
   +0x18 avgs → AWB scratch + 8, +0x1c u8 HDRMode, +0x20 pAwbStatisShort → short scratch + 8 when HDR, else the constant **8**
   (NULL + 8, not NULL), +0x24 u4BVx16384, +0x28 WeightY.
3. **`ISP_AWB_RESULT` (0x18)** pre-filled: Size 24, Change 0, R / G / B gain = current gains, ColorTmp 0.
4. `run(pdata, &info, &result)`; `MI_ISP_CUS3A_SetAwbParam` (0x2E0A) with {Size field = **56** (copy of the AE value; the
   kernel checks the header length 24, not this field), Change, R, G, B, ColorTmp}.

**AF init** (static `0x1d90`; AF mask edge, AF not running, slot init ≠ NULL): `MI_ISP_CUS3A_GetAfStatus` (0x2E20, 16 B
`CusAFInfo_t` {Size, MinPos, MaxPos, CurPos}) — **its return value is ignored** (without a VCM driver it fails and leaves
zeros); `ISP_AF_INIT_PARAM` (0x118, zero except) +0x00 Size 0x118, +0x104 CurPos, +0x108 MinPos, +0x10c MaxPos, +0x110 = 1,
+0x114 = MaxPos − MinPos; `init(pdata, &param)`; returns 0, so AF is always marked running.

**ProcAF** (AF running, `run` ≠ NULL): `write(fd, {dev, 3, ch, 0, 0})` + `read(fd, 16)` = `CusAFInfo_t` (isp_fe mode 3);
`write(fd, {dev, 2, ch, 0, 0})` + `read(fd, AF scratch, 0x1c00)` (mode 2 = AF statistics, 7168 B); **`ISP_AF_INFO`** =
{+0x00 = 4, +0x04 → AF statistics scratch, +0x08 CurPos, +0x0c MinPos, +0x10 MaxPos}; **`ISP_AF_RESULT`** zeroed {Size,
Change, NextPos}; `run(pdata, &info, &result)`; `MI_ISP_CUS3A_SetAfParam` (0x2E21) with {12, Change, NextPos}. The kernel moves
the VCM only if a VCM driver exists; this camera has none (§1.1).

**After the writes, in the kernel** [R calls; timing from isp-2-ispmid-mload-ispscl.md §6.2]:
```
frame N start (ISP IRQ)  → FrameStartProc: … IspmidFrameEvent(ch)          [statistics of frame N−1 are in the copy]
Cus3A_ProcRoutine wakes  → read(isp_fe) frame bit → ProcAE, ProcAWB, ProcAF as above
kernel, on 0x2E06:       sensor shutter work at frame cur + (D − shutter delay), gain at cur + (D − gain delay),
                         digital gain at cur + D − 1 (D = max of the two delays) — executed by FrameStartProc of that frame
                         into its CMDQ batch (ISP gain) or straight over I2C from the ISP driver thread (sensor, via SNR)
kernel, on 0x2E0A:       gains stored; next FrameStartProc → DoIqPerFrame / DoIq → WB-gain shadow → batch → VSYNC
```
Units the kernel expects in the results [I]: shutter in µs, gains 1× = 1024, ISP gain ≥ 1024; a result written within the
frame is used `D` frames later for the sensor and at the next frame for the ISP gains. The sensor's shutter / gain delays that
set `D` come from the sensor driver at open (sensor-gc8613.md §4.1).

### 2.6 API data routing `Cus3A_SetIspApiData(hdr, data)` / `Cus3A_GetIspApiData`
Registered by libmi_isp as the agent of every channel (mi-capture.md §5.2). `hdr` = the 0x18-byte `isp_api_hdr`. Builds a ctrl
parameter block `{u32 dir (0 set, 1 get); u32 —; u32 DataLen; u32 CtrlID; u32 channel; u32 —; s32 result = 0; void *data}`
(0x20) and routes by `CtrlID`:

| id range | goes to |
|---|---|
| 0x1400–0x17FF, 0x3000 | `Cus3A_ProcAeCtrl(dev, ch, id, &blk)` |
| 0x1800–0x1BFF, **0x240F**, 0x2C03–0x2C06, 0x3001 | `Cus3A_ProcAwbCtrl` |
| 0x1C00–0x1FFF, 0x3003 | `Cus3A_ProcAfCtrl` |
| other 0x2FFF–0x3100 (incl. 0x2FFF, 0x3002, 0x3004–0x3100) | swallowed: returns 0, nothing done |
| other 0x2400–0x27FF, other 0x2C00–0x2DFF, everything else | returns **0xA0078008** → libmi_isp issues the ioctl (kernel) |

`Cus3A_Proc{Ae,Awb,Af}Ctrl`: under the device lock, if the type is **running** and its current adaptor's `ctrl` ≠ NULL →
`ctrl(pdata, id, &blk)` and return 0; else −1 → the caller returns 0xA0078008. **The ctrl's own return value and the status it
puts into `blk.result` are discarded**: once the algorithm runs, every id in its range returns 0 to the application whether the
algorithm accepted it or not (size mismatch, unknown id, NULL handle all look like success). Before the algorithm runs (between
`MI_ISP_CreateChannel` and the first isp_fe AE / AWB event) the same ids go to the kernel instead (§3).

### 2.7 The built-in SigmaStar adaptor (`Sigma*` in libcus3a)
`Sigma3AGetAe/Awb/AfInterface(dev, ch, &iface)` all return the same context `ctx = .bss 0x3451c + dev·0x11c0 + ch·0x11c`
(stores dev at ctx+0x114, ch at ctx+0x118) with the type's four functions. Context layout (the libispalgo structs named here:
isp-5-3a.md §2.2, §3.1, §4.1):

| offset | content |
|---|---|
| +0x000 | AE handle (`AeInit` result) |
| +0x004 … +0x093 | `AeOutput_t` of `DoAe` (0x90 B; ctx offset = out offset + 4): +0x8 Change, +0xc Shutter, +0x10 SensorGain, +0x14 IspGain, +0x18…+0x20 HDR-short triple, +0x24 BV, +0x28 CurY×10, +0x2c AvgY×10, +0x30 short-set Y×10 ("WeightY"), +0x88 DebandFPS, +0x8c u16 FNx10 |
| +0x094 | `AEDEBUG_t` {+0x94 enable = 1, +0x98 debug level} — the `0x3000` customer AE ctrl reads / writes +0x98 |
| +0x09c | AWB handle (`AwbInit`) |
| +0x0a0 … | `AwbOutput_t` of `DoAwb` (+0xc2 u16 R gain, +0xc4 Gr, +0xc8 B, +0xca colour temperature) |
| +0x0e4 | `AWBDEBUG_t` {+0xe4 AWB enable = 1, …, +0xf4 debug level} — the `0x3001` customer AWB ctrl reads / writes +0xf4 |
| +0x0f8 | AF handle (`AfInit`, lazy) |
| +0x0fc | 8 bytes zeroed at AF init |
| +0x104 | AF debug pair {1, 0} (`.rodata 0x50c8`); the `0x3003` customer AF ctrl reads / writes +0x108 |
| +0x10c | AE frame counter; +0x110 AWB frame counter |

- **SigmaAeInit(ctx, ISP_AE_INIT_PARAM *p)** (p = the kernel's 108-byte `CusAEInitParam_t`): prints the limits; builds the
  libispalgo `AeInitParam_t` (0x5c) from it; sets ctx+0x94 = 1, +0x98 = 0, output FNx10 = 18, CurY×10 = 100 and output
  shutter / gains = 300 µs, 1024, 1024, HDR 30 µs, 1024, 1024; `LoadIqFile(&cfg, 0)` (§2.8); `ctx[0] = AeInit(cfg,
  &initParam, &aeInput{AvgBlkX = p+0x64, AvgBlkY = p+0x68, +0x50 → ctx+0x94}, ctx+4)`; `FreeIqFile`; frame counter = 0.
- **SigmaAeRun(ctx, info, result)**: result ← Size 0x3c, Change 0, previous output values (shutter … HDR, BV, AvgY =
  CurY×10/10); frame counter++; if the AE handle exists builds `AeInput_t` (0x58) — +0x04 hist1, +0x08 hist2, +0x0c
  pRGBIRHist, +0x10 / +0x14 AvgBlkX / Y, +0x18 avgs, +0x1c 0, +0x20 u16 FNx10 (18 if 0), +0x24…+0x38 shutter / gains / HDR,
  +0x3c previous CurY×10, +0x40 PreAvgY×10, +0x44 / +0x48 **left uninitialised** (stack), +0x4c u8 HDRCtlMode, +0x50 →
  ctx+0x94, +0x54 CurFPS — calls `DoAe(ctx[0], &in, ctx+4)` and copies out: Change, Shutter, SensorGain, IspGain, the HDR
  triple, BV, AvgY = out+0x28/10, FNx10 = u16 out+0x88, DebandFPS = out+0x84, WeightY = out+0x2c/10, HdrRatio 0. `AeOutput_t`
  +0x8c (= the AE's GMBlendRatio + 1, the AdaptiveGamma weight on the in-kernel path, isp-5-3a.md §2.2) is not forwarded, so on the
  user-3A path nothing feeds the IQ engine's AdaptiveGamma weight (ch+0x114). Whether AdaptiveGamma is enabled in this camera's
  IQ file / API bin is not known.
  The unit of `AeInput_t`+0x54 on this path — libcus3a passes the 0x2E05 "current sensor FPS" (`CusAEInfo_t`+0x3c) — is not
  known (isp-5-3a.md §2.12, run-time flicker detector).
- **SigmaAeCtrl(ctx, id, blk)**: id 0x69 → print `[ae_ctrl] camera reopen`, return 0. Set: 0x3000 → ctx+0x98 = *data; else
  `blk.result = DrvAlgo_IF_ApiSet(id, ctx[0], 0, 0, 0, blk.DataLen, blk.data)`. Get: 0x3000 → *data = ctx+0x98; else
  `DrvAlgo_IF_ApiGet(…)`. NULL ctx / blk or no AE handle → −1 (ignored by the caller, §2.6).
- **SigmaAeRelease** → `AeRelease(ctx[0])`, ctx[0] = 0.
- **SigmaAwbInit(ctx)**: ctx+0xe4 = 1, +0xf4 = 0; `AwbInput_t` zeroed (0x48) except {+0x14 = 0x04000400, +0x18 u16 = 0x400};
  `LoadIqFile(&cfg, 0)`; `ctx+0x9c = AwbInit(cfg or NULL when the file is missing, &in, ctx+0xa0)`; counter = 0.
- **SigmaAwbRun(ctx, info, result)**: result ← {Size 24, Change 0, R = u16 ctx+0xc2, G = u16 ctx+0xc4, B = u16 ctx+0xc8,
  ColorTmp = u16 ctx+0xca}; counter++; **the algorithm runs only when the counter before the increment is a multiple of 3**
  (`n·0xAAAAAAAB ≤ 0x55555555`) — AWB every third frame — and only with an AWB handle. `AwbInput_t` (0x48) on the stack:
  +0x00 Size 0x48, +0x04 frame count (= counter after the increment), +0x08 BV (info+0x24), +0x0c WeightY (info+0x28),
  +0x10 u16 0, +0x12 u8 HDRMode (info+0x1c), +0x14…+0x1b **uninitialised**, +0x1c crop {1, 1, 1920, 1080} (.rodata 0x4b48),
  +0x2c block {u16 128, u16 90}, +0x30 {u16 4, u16 2}, +0x34 {u16 10, u16 8} (block count, sample size, ratio — the same
  defaults as the kernel's AWB window, isp-2-ispmid-mload-ispscl.md §10.2), +0x38 uninitialised, +0x3c avgs (info+0x18),
  +0x40 short avgs (info+0x20), +0x44 → ctx+0xe4 (`AWBDEBUG_t`). After `DoAwb(ctx[0x9c], &in, ctx+0xa0)` the result is
  rewritten with Change = **1** and the new gains / colour temperature. On the two frames out of three where AWB does not run
  the previous gains go out with Change = 0, which the kernel ignores.
- **SigmaAwbCtrl**: as AE with 0x3001 ↔ ctx+0xf4 and `DrvAlgo_IF_ApiSet/Get(id, 0, ctx+0x9c, 0, 0, len, data)`.
- **SigmaAfInit(ctx)**: AF handle = 0, 8 bytes at ctx+0xfc = 0, prints dev / ch — the real init is lazy: **SigmaAfRun(ctx,
  info, result)** calls `_SigmaAfInit(ctx, info)` while the handle is 0: ctx+0x104 = {1, 0} (AF debug pair, .rodata 0x50c8);
  `AfInit(NULL, &{MinPos, MaxPos, 1, MaxPos − MinPos, CurPos, dev, ch}, &{…, +0x08 30, +0x0c stats, +0x10 CurPos, +0x14 →
  ctx+0x104}, &out)` — no AF configuration file (cfg = NULL), so the AF runs on its built-in defaults (isp-5-3a.md §4.2). Every run:
  `DoAf(h, &{0, 0, 30, stats = info+4, CurPos = info+8, → ctx+0x104}, &out{u32, NextPos})`; result Change = 1, NextPos = out
  word 1.
- **SigmaAfCtrl**: as AE with 0x3003 ↔ ctx+0x108 and the AF handle as the 4th `DrvAlgo_IF_*` argument. **SigmaAfRelease** →
  `AfRelease`.

### 2.8 IQ file of the user 3A: `LoadIqFile(&cfg, n)` / `FreeIqFile`
Directory `/config/iqfile`, replaced by `MI_ISP_GetIspRoot(0, 0, buf)` when that succeeds (prints `change isproot : %s`);
opens `<dir>/iqfile<n>.bin` (n = 0 always), reads it whole, `IspLoadIqCfg(&cfg, buf, size)` (libispalgo's parser of the IQ
file, isp-4-iq-engine.md §13), frees the buffer; failure → cfg = NULL, −1. **FreeIqFile** → `IspFreeIqCfg`.
- `MI_ISP_GetIspRoot` (API id 0x66) fails on every call: libcus3a does not route 0x66 (§2.6), the mhal ISP API switch has no
  case for it, and the one way it could still be served — a command filter registered through `MS_CAM_RegIspCmdFilter` — is
  never set up: that function only stores a pointer pair, nothing inside mhal.ko calls it, and no other file of the dump
  (modules, libraries, applications) contains the symbol name [R: string search; the compressed kernel image is not searchable
  this way, I: built-ins do not call module symbols]. So the directory is always `/config/iqfile`.
- `/config/iqfile/iqfile0.bin` (= `miservice/iqfile/iqfile0.bin` in the dump) is a symlink to `imx307_iqfile.bin`, the same
  39 135-byte file the kernel carries built in as `iq_bin` (isp-4-iq-engine.md §13.5); `iqfile0…3.bin` are all symlinks to it. So
  the user AE / AWB start from the **IMX307** IQ tuning, and the API bin's AE / AWB records loaded afterwards (§3.2) override
  part of it.

### 2.9 Helpers
- `CUS3A_AeAvgDownSample(in, out, inX, inY, outX, outY)` (3.6 KB, NEON-vectorised): returns −1 if the output grid is larger
  than the input in either direction (outX > inX or outY > inY); else for every output block (i, j) averages the input blocks
  x ∈ [⌊i·inX/outX⌋, ⌊(i+1)·inX/outX⌋), y ∈ [⌊j·inY/outY⌋, ⌊(j+1)·inY/outY⌋), separately for the four bytes {R, G, B, Y},
  with rounding `(sum + n/2) / n` (indices handled as u8, sums as u16); returns 0. Not called inside libcus3a (a service for
  customer AE code that wants fewer blocks).
- `pAllocDmaBuffer(name, size, &phys, &miu, cached)` → `MI_SYS_MMA_Alloc(0, NULL, size, &addr)` + `MI_SYS_Mmap(addr, size,
  &va, cached)`, phys = miu = addr, returns va (NULL on failure, freeing the allocation); `FreeDmaBuffer(name, miu, va, size)`
  → `MI_SYS_Munmap` + `MI_SYS_MMA_Free`. Not called inside libcus3a.
- `Cus3AOpenIspFrameSync(&fd)` / `Cus3ACloseIspFrameSync(fd)` / `Cus3AWaitIspFrameSync(fd, timeout_ms)`: the single-fd variant
  of the public header's two-fd API; Wait writes the mode-0 command, polls, reads the 20-byte event and returns **its first
  word** (frame mask), 0 on timeout. Not used by the thread (it has its own fd).
- `__aeabi_idiv` (0x154d8) = the ARM EABI integer division helper (`__aeabi_idiv` with divide-by-zero → `raise(SIGFPE)`).

## 3. Which ids the user-space 3A accepts (libcus3a + libispalgo)

### 3.1 The rule [R]
<!-- ev:isp-6-userspace.3.1 -->
An `MI_ISP_*` AE / AWB / AF call from the application goes `_MI_ISP_SetIspApiData` → libcus3a agent (§2.6):
1. Ranges 0x1400–0x1FFF, 0x240F, 0x2C03–0x2C06, 0x3000 / 0x3001 / 0x3003 go to the algorithm **if it is running**; then the
   call returns **0 to the application whatever happened**. Not running → 0xA0078008 → mi_isp ioctl → kernel (AE / AWB ids
   stored in the idle in-kernel AE / AWB objects, AF ids zero the caller's buffer and fail with 6; isp-3-iq-api.md §4.1).
2. Inside the built-in adaptor: 0x3000 / 0x3001 / 0x3003 are the debug-level words of §2.7; every other id is
   `DrvAlgo_IF_ApiSet/Get(id, aeH, awbH, afH, NULL, DataLen, data)` of libispalgo, whose switch is
  :

| ids | Set | Get |
|---|---|---|
| AE 0x1403–0x1413, 0x1415, 0x1416, 0x1418 | handler, size checked (same sizes as the kernel) | — |
| AE 0x1401–0x1419 (all 25) | — | handler |
| AE 0x1401, 0x1402, 0x1414, 0x1417, 0x1419 Set | **not supported** (0xF) | |
| AWB 0x1802–0x1804, 0x1806, 0x1807, 0x1809 | handler | — |
| AWB 0x1801–0x1804, 0x1806–0x1809 | — | handler |
| AWB 0x1801, 0x1808 Set; 0x1805 both | not supported | |
| AF 0x1C01–0x1C09, 0x1C29–0x1C2F | AF method if the AF handle ≠ NULL | AF state (listed as inline copies; below) |
| AF 0x1C30 | not supported | `pAF_QueryInfo` (method +0) |
| 0x240F | `DrvAlgo_IF_CALIv30_AWB_CaliApply` (no size check) | not supported |
| 0x2C03 | `AWB_SetGoldenLightArea` (84 B) | `AWB_GetGoldenLightArea` (84 B) |
| 0x2C04 / 0x2C05 / 0x2C06 | not supported | `AWB_GetCTStats` (23 044 B) / `AWB_GetHWStats` (34 560 B) / `AWB_GetCurLightArea` (80 B) |
| anything else in the routed ranges | not supported (0xF) | not supported |

AF Get: 0x1C2B (AFAccWeight) calls method +0x68 (Set: +0x64) and 0x1C30 method +0; the other AF Get rows, which the switch
table lists as inline copies of the AF state, were not rechecked [I: also method calls]. Compared with the kernel copy the id
set and every size are the same except [R, script diff]: the user copy **adds** 0x2C03 Set / Get and 0x2C04–0x2C06 Get (the
kernel has no case for them), and **lacks** the IQ-handle calibration ids 0x2410 / 0x2414 / 0x2415 / 0x2417
(`CALIv30_OB/SDC/ALSC/LSC_CaliApply`) — those are outside libcus3a's routed ranges, so they reach the kernel. Get 0x1419 /
0x1808 are inline in the kernel and named functions in the user copy (same behaviour). The handlers themselves: isp-5-3a.md §2.16, §3.11, §4.8.

### 3.2 On this camera [R for the routing, I where marked]
<!-- ev:isp-6-userspace.3.2 -->
- The API bin `iq_file_gc8613.bin` carries 0x1403–0x1416 (18 AE ids), 0x1802–0x1806 / 0x1809 (5 AWB), 0x2C03, 0x1C01–0x1C09
  and 0x1C29–0x1C2F (16 AF) and 0x3000 / 0x3001 / 0x3003. prog_uvc loads it only after
  `CUS3A_GetAlgoStatus` reports AE and AWB running ([S]), so every AE / AWB record reaches libispalgo and is
  accepted (all have a Set case and the sizes match). 0x3000 / 0x3001 / 0x3003 set the three debug levels (§2.7).
- AF records: accepted only if AF is running **and** `SigmaAfRun` has already created the AF handle (lazy, §2.7); prog_uvc does
  not wait for AF, so whether the AF records are applied or silently dropped (adaptor returns −1, app sees 0) or sent to the
  kernel (AF not yet running → zeroed + error 6) depends on the order of isp_fe events [I]. Without a VCM it has no visible
  effect.
- The application's own AE / AWB calls (exposure mode, manual exposure, flicker, target, AWB attr / query — [S])
  are all in the accepted set; a wrong size is reported to libispalgo's log (`[ERROR - ISP API SET] Structure mismatch`) but
  returns 0 to the application.
- Before the user 3A runs (first frames after `MI_ISP_CreateChannel`), the same calls land in the kernel objects and are lost
  when the user algorithm starts (its own state comes from `AeInit` / `AwbInit`) [I].

## 4. CUS3A ids called per frame [R]
libispalgo itself issues CUS3A calls **only from its AF hardware setters** (`pAF_SetHWRoiMode/Win/FilterAttr/FilterSq/YParam/
Source/PreFilter/YMap/Ldg` → 0x2E11, 16 × 0x2E0E, 0x2E0F, 0x2E10, 0x2E24, 0x2E25, 0x2E26, 0x2E27, 0x2E28 with the dev / ch
stored in the AF object at +0x29ac / +0x29b0); they run at `AfInit` (`AF_InitParameters` calls all nine through the method
table) and when an AF id is Set — never per frame. Everything per frame comes from libcus3a's thread (§2.4–§2.5):

| per frame event, channel ch | ids |
|---|---|
| AE running | Get 0x2E01, 0x2E02, 0x2E03, 0x2E1E, 0x2E05; Set 0x2E06 |
| AWB running | Get 0x2E07, 0x2E09 (+ 0x2E19 when HDR); Set 0x2E0A — **every frame**, although the built-in AWB computes only every third frame (Change = 0 on the other two) |
| AF running | `/dev/isp_fe` read modes 3 (16 B) and 2 (0x1C00 B) — not CUS3A ids; Set 0x2E21 |
| once at AE start | Get 0x2E04 |
| once at AF start | Get 0x2E20 (result ignored) |
| `CUS3A_EnableUserspace*` / `RegInterface*` | Set 0x2E08 |

## 5. libsigma3a.so — an older copy of the built-in adaptor (unused) [R unless tagged]
16 exports, NEEDED only libc although it calls `AeInit` / `DoAe` / `AeRelease` / `AwbInit` / `DoAwb` / `AwbRelease` /
`DrvAlgo_IF_ApiSet` / `ApiGet` / `IspLoadIqCfg` / `IspFreeIqCfg` (libispalgo) and `MI_ISP_CUS3A_SetAFRoiMode` /
`SetAFWindow` / `SetAFFilter` (libmi_isp) — it only loads into a process that already has both. It is the Sigma adaptor of
§2.7 in an earlier CUS3A generation [R differences below, I "earlier"]:
- **No device argument**: `Sigma3AGetAeInterface(ch, &iface)` / `Sigma3AGetAwbInterface(ch, &iface)` (no AF getter although
  `SigmaAf*` exist); context = `.bss 0x1224c + ch·0xf8`,.bss 0x3ec = room for **4 channels**; one global frame counter per
  type (.bss 0x12240 / 0x12248) shared by all channels.
- `SigmaAeInit(ctx, p)` reads the same 108-byte `CusAEInitParam_t` but builds the **M5-layout** `AeInitParam_t` of
  the Mercury5 `isp_3a_if.h` (Size written as 100, `sensor_id[0]` copied, shutter / step / min / max at
  +0x24…+0x30, sensor gain +0x34, gain min forced to 1024 at +0x38, gain max +0x3c, ISP gain / max +0x40 / +0x44, FNx10
  +0x48, fps +0x4c) and pre-fills an M5-layout `AeOutput_t` (FNx10 = 18 at out+8, shutter 300 at out+0xc, gains 1024, HDR
  shutter 30, CurY 100 at out+0x28).
- `SigmaAeRun` passes `DoAe` an **81-byte packed** `AeInput_t` (Size 0x51; FNx10 at +0x1c taken from the packed CUS3A-1.1
  `ISP_AE_INFO` offset +0x35, shutter … HDR gains at +0x20…+0x34, previous CurY at +0x38, PreAvgY×10 at +0x3c, HDR mode at
  +0x48, debug pointer at the unaligned +0x49, CurFPS at +0x4d) and reads the result back from the M5 layout.
- `SigmaAwbRun` runs every third call (global counter), `AwbInput_t` Size 0x40 with a fixed crop 1920×1080, blocks 128×90.
- `SigmaAfInit` writes the AF set-up straight to the kernel with the **two-argument** CUS3A calls
  (`MI_ISP_CUS3A_SetAFRoiMode(0, {0,1})`, 16 × `SetAFWindow(0, win)` from a.rodata table, `SetAFFilter(0, …)`); `SigmaAfRun` /
  `SigmaAfCtrl` are empty.
- `LoadIqFile(&cfg, n)` opens `/config/iqfile/iqfile<n>.bin` (fixed path, no ISP-root lookup); `FreeIqFile` = `IspFreeIqCfg`.
- `SigmaAeCtrl` / `SigmaAwbCtrl` print `[ae_ctrl] CtrlID = %d, Dir = %d` and forward to `DrvAlgo_IF_ApiSet/Get` with the old
  7-argument order (no customer ids 0x3000 / 0x3001).

This firmware's `DoAe` reads `AeInput_t` with FNx10 as a u16 at +0x20 and the shutter at +0x24 (isp-5-3a.md §2.2): fed by
libsigma3a it would take the sensor gain for the shutter and so on — the library cannot work with this firmware's libispalgo
[I from the two layouts].

## 6. libmi_iqserver — the IQ Tool service [R unless tagged]
<!-- ev:isp-6-userspace.6 -->

The service lets a PC tuning tool (SigmaStar's Windows "IQ Tool") inspect and change the camera's ISP settings and pull frames
at several pipeline stages. Subject: `libmi_iqserver.so` (50.5 KB, ARM EABI5, stripped except its exports; 48 exported FUNC
symbols; BuildID 3f3e849b…; version string "Sigmastar Module mi_iqserver version: project_commit.7251436
sdk_commit.80327e1 build_time.20220721223314"); `libmi_iqserver.so` needs only `libc.so.6` and imports `FBD_Execute`
unresolved. The same code is linked statically into prog_uvc.

In short: a TCP server on port **9876** (one client at a time) and an optional serial server on `/dev/ttyS0` at 115200 8N1
(§6.8) carry the same message format (8-byte header + payload, §6.2) and the same eight commands (`CAMERA_EXT_CMD_TYPE`):
1 custom, 2 set-mode (no-op), 3 get-mode, 4 get-picture, 5 set-API, 6 get-API, 7 upload-file, 8 download-file. Set / Get-API
carry a 16-bit API id plus a struct and pass them to `MI_ISP_SET_ALL` / `MI_ISP_GET_ALL` — the same id and struct space as
`isp_api.xml` and the API bin (isp-3-iq-api.md §2, §6); about twenty further ids are handled inside the service (§6.5.3). Pictures
come from four stages: scaler output (YUV), ISP output (before the scaler), sensor raw (single or burst), JPEG (§6.6).

### 6.1 Lifecycle
**Starting and stopping.** `MI_IQSERVER_Open()` (0x3b3c; prog_uvc 0x43994) takes no arguments and always returns 0. It
installs the default custom-command handler if none was set, clears the response-buffer state and the stop flags, starts the
TCP listener thread ("IQSERVER_LISTEN"), and then checks the serial switch: if `/tmp/isp_uart` enables it, it freezes the
serial-console shell so the port is free and starts the serial listener thread ("IQSERVER_UART_LISTEN") (§6.8).

`MI_IQSERVER_Close()` (0x3c78) sets the three stop flags (TCP command thread, serial command thread, both listeners), prints
"iqserver closed" and returns 0. It does not join the threads or close sockets; each thread notices its flag at its next
1-second poll timeout and exits. A `sh` frozen by Open is never resumed (no SIGCONT in the binary).

Two setup calls, not used by prog_uvc (§1.2):
- `MI_IQSERVER_SetCustFunc(f)` (0x3ab0): sets the handler of command 1. `f == NULL` → returns 2.
- `MI_IQSERVER_SetDataPath(p)` (0x3ad0, Ghidra 0x13ad0): sets the directory `isp_api.xml` is read from. `p == NULL` → 2. Writes
  `sprintf(path, "%s/", p)` into a 32-byte buffer with no length check [I: overflow if `p` ≥ 31 bytes], then
  `access(path, R_OK)`: readable → 0, else prints "setDataPath failed" and returns 1 (the path stays changed). Default path is
  `/config/iqfile/`.

**TCP listener.** `thread_listen_handle` (0x3248) creates the listening socket with `mst_ListenSockCreate(INADDR_ANY, 9876)`:
`socket(AF_INET, SOCK_STREAM)`, `SO_REUSEADDR`, `bind` to `0.0.0.0:9876`, `listen(backlog 1)`, then `TCP_NODELAY`,
`SO_KEEPALIVE`, `SO_SNDBUF = SO_RCVBUF = 4096`. It loops on a 1-second select; on an incoming connection it `accept`s
(retrying ECONNABORTED). If a client is already being served it closes the new socket with "iq server only for one client!";
otherwise it starts `thread_cmd_handle` for the new socket and marks a client active. A select or accept error closes and
re-creates the listening socket on the next turn. The thread exits when the listener stop flag is set.

**Threads and concurrency.** One TCP command thread (`thread_cmd_handle`) runs per connected client, but only one client is
accepted at a time, so at most one is alive. The serial path has its own listener (`thread_uart_listen_handle`) and command
thread (`process_uart_cmd`). TCP and serial command threads can run at once; they share all the global state (current
device / channel, sensor pad, VENC channel, SCL info, raw-capture settings, §6.4) with no locking (no mutex in the binary), so a
concurrent TCP + serial session would race [I].

### 6.2 Message format [R]
All fields are little-endian.
```
Request:   u32 cmd        // 1..8
           u32 len        // number of payload bytes that follow; must be >= 2
           u8  payload[len]

Response:  u32 code        // 0 = ok, 1 = error
           u32 len         // number of payload bytes that follow
           u8  payload[len]
```
The response code is the only error signal — there is no text. One request is in flight at a time: the server reads the next
8-byte header only after it has sent the current response in full.

### 6.3 Command thread and dispatch
**Command thread loop** (`thread_cmd_handle`, 0x2e18; prog_uvc 0x42cfe). On accept, the socket is set to
`SO_SNDTIMEO = SO_RCVTIMEO = 3 s`, `SO_SNDBUF = SO_RCVBUF = 8192`, and non-blocking (`FIONBIO`). The thread allocates a
command buffer and a response buffer of 0xc800 (51200) bytes each and an ISP data buffer via
`MI_ISP_Alloc_IQDataBuf(0xc800)`. Then per request:
1. Poll with a 1-second select; nothing readable → loop. Read exactly 8 header bytes. The low-level read
   (`mst_NoBlock_Recv`) loops `recv`, retrying EAGAIN / EINTR every 10 ms up to 500 consecutive times; a 0-byte read or
   ECONNRESET means the peer closed. Fewer than 1 byte → the connection is treated as closed and the thread ends. 1–7 bytes →
   "sock ret != len", the request is dropped with no response.
2. `len < 2` → send `{code 1, len 0}`.
3. `cmd == 7` (upload): the payload is streamed straight to `/tmp/raw_data.raw` (§6.7). For every other command the command
   buffer is grown to `len` if needed (`MI_IQSERVER_CheckBuffSize`: realloc by free-then-malloc; failure ends the session) and
   `len` bytes are read; a short read → `{1, 0}`.
4. Dispatch to `MI_IQSERVER_ProcessCmd(cmd, len, &cmdBuf, &respBuf, &ispBuf)` (0x2cf4; prog_uvc 0x42bd8). Each buffer is a
   descriptor `{void *p; u32 cap; u32 used}`. The handler sets `respBuf.used = 0` first, then fills it.
5. Reply. A non-zero handler return → `{1, 0}` ("Parse Command error ret = …"). Return 0 → `{0, respBuf.used}`, then, if
   `used != 0`, the payload: one `send` of the response buffer, or — when the handler swapped the response descriptor to the
   multi-buffer head (below) — a `SendResponseDataArray`, after which the normal buffer is restored. The response buffer is
   zeroed after every request. The low-level send (`mst_NoBlock_Send`) loops `send`, retrying EAGAIN / EINTR with a per-socket
   pause (`SleepTime`, §6.4) up to 4000 consecutive times.

**Command table** (`MI_IQSERVER_ProcessCmd` switches on `cmd`; the enum matches the MStar `iqserver_type.h`):

| cmd | name | request payload | response payload |
|---|---|---|---|
| 1 | custom | `u16 id; u16 -; u32 n; u8 pad[8]; u8 data[n]` → `g_CustFunc(id, n, data)` | none; status = handler return (default handler prints "please call MI_IQSERVER_SetCustFunc()", returns 0) |
| 2 | set-mode | any | none; status 0 |
| 3 | get-mode | `u32 0` | 1 byte: 1 if the VIF group of the current sensor pad is in an HDR mode (`GetHdrMode`, §6.6.1), else 0 |
| 4 | get-picture | `u32 mode; u32 -` (len ≥ 8) | the picture bytes (§6.6) |
| 5 | set-API | API request (§6.5) | none |
| 6 | get-API | API request (§6.5) | API reply (§6.5) |
| 7 | upload-file | 16-byte header + file bytes | none (§6.7) |
| 8 | download-file | `u16 fileId; …` | the file bytes (§6.7) |
| other | — | — | status −1 |

Commands 4, 5 and 6 act on the current ISP device and channel globals (`g_ispDev`, `g_ispChn`, both default 0), set through
API ids 104 and 101 (§6.5.3).

**Multi-buffer picture responses.** Raw and ISP-output pictures are too large to copy into the 51200-byte response buffer, so
the picture handler parks the normal response descriptor aside (`iqsvr_buff`) and points the response at a head node
(`iqsvr_buff_array`), building a linked chain of source buffers:
```
struct node {            // 28 bytes; extra nodes are malloc(0x1c)
    void *vaddr;         // +0x00  frame data (mmapped MMA buffer, or MI_SYS buffer virtual address)
    u32   cap;           // +0x04
    u32   size;          // +0x08  bytes to send from this node
    u32   handle;        // +0x0c  MI_SYS buffer handle, or MIU physical address of an MMA buffer
    u32   kind;          // +0x10  0 = MI_SYS output buffer; 1 = one MMA buffer; 2 = one MMA buffer per node
    struct node *next;   // +0x14  send order
    struct node *prev;   // +0x18
};
```
`SendResponseDataArray(fd, head, total)` (0x8538, Ghidra 0x18538; prog_uvc 0x48390) walks `next`: for a `kind == 0` node (a VIF
output buffer) it first runs `FBD_Execute(node.vaddr, parked response buffer, W, H)` (§7) with W / H from the two raw-geometry
globals (§6.4) and sends the decoded `W × H × 2` bytes; otherwise it sends `node.size` bytes from `node.vaddr`. There is a 1 ms
pause between nodes. Then, walking `prev` from the last node, it releases each buffer according to the **head** node's kind
(0 → `MI_SYS_ChnOutputPortPutBuf`; 2 → `MI_SYS_Munmap` + `MI_SYS_MMA_Free` per node; 1 → one unmap / free of the head after the
walk) and frees the extra nodes. A sent total that does not match `total` prints "already send length". So each burst frame is
one node, sent in order and released in reverse [I].

### 6.4 Global state
All state is process-global (no per-connection context). Names are from the export table or inferred from use; addresses are
`.so` file vaddrs (prog_uvc addresses where given).

| addr | name | init | meaning |
|---|---|---|---|
| 0x1c2bc | `iqserver_data_path` (char[32]) | "/config/iqfile/" | directory of `isp_api.xml`; changed only by `SetDataPath` |
| 0x1c3b8 / 0x1c3bc | `vifIdleChn` / `vifIdleDev` | 8 / 8 | VIF chn / dev feeding the ISP, written by the raw paths and never read (no read xref) |
| 0x1c3c0 | `SleepTime` | 1000 | µs between send retries (listen socket 1000, connect socket 10000) |
| 0x1c346 / 0x1c348 | u16 / u16 | 0x0780 / 0x0438 | raw frame **height** / **width** passed to the FBC decoder; set before each raw capture (height doubled for HDR) |
| 0x1c509 | u8 | 0 | a TCP client is being served (a second client is refused) |
| 0x1c50a | `ISUartThreadRuning` (u8) | 0 | a serial session is active |
| 0x1c50c | u32 | 0 | stop flag of the TCP command thread (set by Close) |
| 0x1c510 | u32 | 0 | stop flag of both listener threads (set by Close) |
| 0x1c514 | u8 | 0 | stop flag of the serial command thread (set by Close) |
| 0x1c518 | `g_CustFunc` | default handler at Open | handler of command 1 |
| 0x1c51c / 0x1c520 | `g_ispChn` / `g_ispDev` | 0 / 0 | ISP channel / device all commands act on (API ids 101 / 104) |
| 0x1c524 / 0x1c528 / 0x1c52c | pthread_t | | command, listener, serial-command thread ids |
| 0x1c530 / 0x1c534 (prog_uvc 0xba658 / 0xba65c) | u32 / u8 | 0 | raw-injection thread running / stop flag (API 0x3202) |
| 0x1c538 | u32 | 0 | HDR raw toggle: alternates long / short exposure between captures |
| 0x1c53c | `g_rawCount` | 0 | frames per raw burst (API 0x3203; 0 = until the source runs dry) |
| 0x1c540 | `g_sensorPad` | 0 | sensor pad used to locate the VIF device (API 0x24) |
| 0x1c544 | `g_vencChn` | 0 | VENC channel for the VENC API ids (device fixed at 0) |
| 0x1c548..0x1c553 (prog_uvc 0xba670 / 74 / 78) | u32 ×3 | 0 | raw-injection width, height, format (API 0x3201) |
| 0x1c554 | pthread_t | | raw-injection thread |
| 0x1c558 | `vifGroupId` | 0 | VIF group of the sensor pad (raw paths) |
| 0x1c55c | `g_SclInfo` {dev, chn, port} | 0,0,0 | SCL port for scaler-output picture / JPEG / resolution query (API 0x3205) |
| 0x1c568 | `iqsvr_buff_array` (28 B) | 0 | head node of a multi-buffer response (§6.3) |
| 0x1c584 | `iqsvr_buff` (12 B) | 0 | parking slot for the normal response buffer while a node chain is sent |

### 6.5 Reading and writing ISP parameters (commands 5 and 6)
The tool reads a parameter group, shows it, lets the user change it and writes it back. Each group is identified by a 16-bit
API id — the ids `isp_api.xml` lists and the API bin stores (isp-3-iq-api.md §2, §6).

#### 6.5.1 Request layout
Both commands take the same payload:
```
u16 apiId;
u16 n;            // number of offsets; must be >= 1
u32 off[n];       // only off[0] is used
u8  data[];       // the struct begins at data + off[0]
```

#### 6.5.2 Set / Get path
**Set-API** (`MI_IQSERVER_SetApi`, 0x788c): `apiId == 0` or `len < 4` is rejected (−1). It computes the struct pointer
`base + 4 + 4n + off[0]`, copies the struct into the ISP data buffer, and calls `MI_ISP_SET_ALL(dev, chn, apiId, len − 4 − 4n,
ispBuf)`. Its return is the status, **unless** one of the service-local ids (§6.5.3) overrides it — those run after
`MI_ISP_SET_ALL`, so the ISP receives every id too, and for ids the ISP does not know the local handler's status replaces the
ISP failure. (`n` is written into a 5-slot stack array, so `n > 5` overruns it [I].)

**Get-API** (`MI_IQSERVER_GetApi`, 0x6da4): `len < 2` → −1. It always calls `MI_ISP_GET_ALL(dev, chn, apiId, &outLen, ispBuf)`
first; on success it builds the reply from the returned struct, and a service-local id may then replace it.

Reply format (also used by the local ids):
```
u16 apiId;
u16 1;            // (= request n for id 0x2803)
u32 0;
u8  data[dataLen];
// respBuf.used = 8 + dataLen
```
Status is the ISP call's return unless a local id sets it. The ISP data buffer is 51200 bytes, so a struct larger than that
cannot be set or read. [I] Any id the ISP knows round-trips with exactly the struct layout `isp_api.xml` describes — which is
why the tool can drive the full IQ parameter set from the XML alone. Which ids the ISP accepts and what each does:
isp-3-iq-api.md §2, §3 here for the AE / AWB / AF ids.

#### 6.5.3 Service-local API ids
Handled inside the service in addition to (Set) or instead of (Get) the ISP call. `d0, d1, …` are the 32-bit words of the
struct (for Set at `base + 4 + 4n + off[0]`; for Get read as `base + 4 + 4n + 4k`, ignoring `off`). "L ≡ x" is the required
total `len`; a mismatch gives −1 and "APIID:%d error param".

| id | Set (cmd 5) | Get (cmd 6) |
|---|---|---|
| 0x20, 0x2c01, 0x2e18 | sensor I2C write: L ≡ 4(n+1)+24, the five offset slots non-zero (n ≥ 5); `MI_ISP_WriteSensorData` with `CusSensorI2cParam_t {u32 slaveAddr, regLen, dataLen, i2cSpeed, reg, data}` | sensor I2C read: words d0..d5 as `CusSensorI2cParam_t`; `MI_ISP_ReadSensorData`; reply = u32 `data` (12-byte reply) |
| 0x24 | `g_sensorPad = d0` (L ≡ 4(n+1)+4) | reply `g_sensorPad` |
| 0x65 (101) | `g_ispChn = d0` | reply `g_ispChn` |
| 0x68 (104) | `g_ispDev = d0` | reply `g_ispDev` |
| 100 | — | chip id from `/sys/devices/soc0/machine`: first word → INFINITY 0x6d65031b, INFINITY3 0x6d65031c, INFINITY2 0x6d65031d, INFINITY5 0x6d65031e, INFINITY6 0x6d650320, INFINITY6E 0x6d650321, INFINITY6B0 0x6d650322, **MERCURY6 0x6d650323** (= `ID_CHIP_M6`, isp-3-iq-api.md §6.1), PIONEER3 0x6d650324; unknown → 0 |
| 106 | — | resolution; d0 selects: 0 or 5 → SCL port `g_SclInfo` output size; 1–3 → VIF output-port-0 capture size of the sensor pad's VIF device; 4 or >5 → −1. Reply `{u32 w, u32 h}` |
| 0x2801 | — | sensor raw info of the pad's VIF device, output port 0: `{u32 width, u32 height, u32 bayerId, u32 precision}` (24-byte reply; §6.6.2) |
| 0x2803 | API bypass: L ≡ 4(n+1)+8; ispBuf = `{d1, d0}`; `MI_ISP_IQ_SetAPIBypassMode` | L ≥ 4; ispBuf = `{0, d0}`; `MI_ISP_IQ_GetAPIBypassMode`; reply n = request n, data word = ispBuf[0], used = (n+2)·4 |
| 0x3102 | `g_vencChn = d0` | reply `g_vencChn` |
| 0x3103 | `MI_VENC_GetChnAttr(0, g_vencChn)`; d0 = RC mode; struct size by mode (1,8 → 24; 2,5,9,11 → 28; 4,10 → 20; other → "not support"); L ≡ 4(n+1)+8+size → 36 bytes replace the attr's RC block (+0x28..+0x4b) and `MI_VENC_SetChnAttr` | `MI_VENC_GetChnAttr` → reply the 36-byte RC block at attr +0x28 |
| 0x3104 | `MI_VENC_GetChnAttr`; size by the attr's RC mode (1,8 → 32; 2,9 → 28; 5,11 → 40; other → 0); L ≡ 4(n+1)+16+size → `MI_VENC_SetRcParam(0, g_vencChn, ispBuf)` | `MI_VENC_GetRcParam` → 56-byte reply |
| 0x3201 | raw-injection format `{w, h, fmt}` (L ≡ 4(n+1)+12); refused when the ISP input is bound REALTIME (§6.7) | — |
| 0x3202 | d0 == 0 → stop raw injection (set stop flag); else clear it | — |
| 0x3203 | `g_rawCount = d0` | — |
| 0x3205 | `g_SclInfo = {d0, d1, d2}` (SCL dev, chn, port); a wrong L only prints the error, the value is still taken | — |

- The kernel refuses 0x2803 (not in its switches, mi-capture.md §5.3), so the bypass Set / Get fail on this firmware [I].
- The 0x3103 block is `MI_VENC_RcAttr_t` (36 bytes at `MI_VENC_ChnAttr_t` +0x28…+0x4b: `eRcMode`, the mode union, `pRcAttr`),
  the 0x3104 block `MI_VENC_RcParam_t` (56 bytes) of the M6 header `mi_venc_datatype.h` [S]; every size the service checks
  matches that header's union members [R sizes vs S header]; RC mode 3 (H264ABR) and the MJPEG modes 6 / 7 fall to "not
  support" / size 0. Field lists: mi_venc.md §6.2, §6.3.
- [I] These ids cover what the MStar header groups as COMMON (0x20 I2C, 0x24 sensor pad), MAINTOOL / PLUGIN (0x2801 raw info,
  0x2803 bypass, 0x2c01 I2C), CUS3A (0x2e18 I2C), and the encoder / raw tool pages (0x31xx VENC, 0x32xx raw capture).

#### 6.5.4 Serial escape of API requests
While a serial session is active, Set-API first scans the request for the 10-byte marker `"~2~2~2~2~2"`
(`findSubIndexOf(buf, '~', '2', len)`, needing 11 bytes of room) and, if present, replaces every `"~2"` by `"2"`
(`strim_special_character`, shortening `len`); then the same for `"~1"`. [I] This un-escapes a framing sequence the PC tool
inserts on the serial link; the TCP path never calls it.

### 6.6 Pictures (command 4)
`MI_IQSERVER_GetPicture(g_ispDev, g_ispChn, cmdBuf, len, respBuf)` (0x51f8, Ghidra 0x151f8; prog_uvc 0x45050). Request:
`u32 mode` (len ≥ 8). mode > 5 → "Error parameter mode failed", −1. mode 4 → −1 (no handler). There is no picture header in
the response — the payload is the frame bytes; the tool knows the geometry from API ids 106 / 0x2801 [I].

#### 6.6.1 Pipeline helpers
- `GetVifBySensorPadID(pad)` (0x3e04): scans `MI_VIF_GetDevStatus(d)` for d = 0…15 (20-byte status: +0 group id, +8 enabled,
  +12 sensor pad); returns the first enabled device carrying that pad, with its group; none → −1.
- `GetHdrMode()` (0x4288): `MI_VIF_GetDevGroupAttr` of the sensor pad's VIF group → `eHDRType` (0 on any failure). Used by
  command 3 and by the HDR branch of the raw paths.
- `EnableScl(dev, chn, port, align)` (0x4140): if the SCL output port has no size set, sets it to the VIF device's output-port
  capture size (1920×1080 if VIF fails), pixel format 11 (NV12), zero crop; with `align == 1` width is rounded up to 16 and
  height to 2; then `MI_SCL_EnableOutputPort`.
- `FindVencOnSclPort` (0x4080): for VENC device 0 and 8, channels 0…32, port 0, `MI_SYS_GetBindbyDest`; returns the bind type
  of the first channel fed by that SCL dev / chn / port, and for REALTIME (4) also its {dev, chn}.
- Pixel format → precision: a VIF output pixel format `f` must be a bayer code `0x14 + 12·p + id` (p = 0…4 for 8 / 10 / 12 /
  14 / 16 bit, id = 0…11 bayer pattern); otherwise "get vif pixel precision fail" → −1.

#### 6.6.2 Raw geometry and bayer id
API id 0x2801 and the raw capture paths derive the frame geometry from the VIF device the current sensor pad maps to:
`MI_VIF_GetOutputPortAttr(vifdev, 0, &attr)` → width = 16-bit field at attr+4, height = 16-bit field at attr+6 (the capture
rectangle's width / height [I: M6 `MI_VIF_OutputPortAttr_t` order]), and `bayerId` / `precision` by decoding the pixel format
as in §6.6.1. Before each raw capture the width is stored in the global 0x1c348 and the height in 0x1c346 (§6.4; Ghidra labels
`data_1c348` / `data_1c346`), which the FBC decoder reads back (§6.3); the height is doubled when a field of
`MI_VIF_GetDevGroupAttr` is non-zero [R; I: the HDR / interlaced line doubling].

#### 6.6.3 mode 0 — scaler output (YUV)
`GetSCLOutput` (0x4d28) on `g_SclInfo`: it refuses a port whose VENC consumer is REALTIME (4) or RING (0x10) ("… is Realtime
or Ring."); otherwise `EnableScl(…, align 0)`, `MI_SYS_SetChnOutputPortDepth(port, 2, 4)`, `MI_SYS_GetFd`, and up to 3
one-second waits for a buffer. It copies the buffer's `u32BufSize` bytes from the plane-0 virtual address (NV12 as the scaler
produces it) into the response, puts the buffer back, and disables the port again if it had no size before. The port depth
stays (2, 4) afterwards.

#### 6.6.4 mode 2 / mode 1 — sensor raw, single frame or burst
A single frame is mode 2 ("GetSensorRawData" / "GetRawDataCommon"); a burst of `g_rawCount` frames is mode 1
(`MI_IQSERVER_GetSensorRawStream` / "GetRawStreamCommon", inside the same function). Two sources, chosen by the ISP input bind
type (`MI_SYS_GetBindbyDest`):
- **ISP input not REALTIME** — the frame comes from the VIF output port. Single frame: VIF output-port depth set
  (`MI_SYS_SetChnOutputPortDepth(0, {mod 6, dev, chn, port}, 2, 4)`), the client buffer must hold W·H·2 bytes
  (`MI_IQSERVER_CheckBuffSize`), then up to 70 tries × 10 ms of `MI_SYS_ChnOutputPortGetBuf`. It prints `compress mode %d`
  with the buffer's `eCompressMode` and, **if it is 4 (`E_MI_SYS_COMPRESS_MODE_TO_8BIT`, [S] M6 `mi_sys_datatype.h`)**, calls
  `FBD_Execute(pVirAddr[0], client_buf, W, H)` (FBD call site prog_uvc 0x46228) — the buffer's stride is ignored. Otherwise
  the buffer is copied and, for 8 / 10 / 12-bit precision, unpacked to 16 bit (§6.6.7); 14-bit is sent as is, 16-bit prints
  "Don't need to convert image". (The conversion step runs for a non-zero bayer id.) `MI_SYS_ChnOutputPortPutBuf`; W·H·2
  bytes go back to the tool — after `FBD_Execute` 12-bit values << 4 (§7.3.1). Burst: sets the VIF port depth, records W / H
  in the same globals (H doubled under the same condition), and **keeps** each VIF buffer it gets (`GetBuf` loop, up to the
  requested count) in the node chain as entries {vir addr, size, size, buf handle, kind 0, next, prev}; it does not look at
  `eCompressMode`. When the response is sent, `SendResponseDataArray` (FBD call site prog_uvc 0x48428) runs every kind-0 node
  through `FBD_Execute(node.vaddr, iqsvr_buff, W, H)` (§6.3). So a burst taken while VIF FBC is off (`mi_vif g_bFBCMode=0`) is
  still "decoded" and arrives as garbage; the single-frame path checks the mode, the burst path does not [I].
- **ISP input REALTIME (this camera)** — the VIF buffers are owned by the ISP, so the service captures from the ISP's raw-store
  DMA ("WDMA"): `MI_ISP_CUS3A_GetImageResolution` for geometry (`CusImageResolution_t {u32 width, height, node,
  pixelDepth}`), allocates an uncached MMA buffer (`pAllocDmaBuffer`), then either `MI_ISP_CUS3A_CaptureHdrRawImage` (HDR,
  alternating long / short via the toggle at 0x1c538) or `MI_ISP_CUS3A_EnableISPOutImage` with node = sensor-raw, polling
  `MI_ISP_CUS3A_GetISPOutImageCount` for up to 1 s. The captured frame is bit-unpacked to 16 bit if needed and attached as a
  node (kind 2, its own MMA buffer per node). This path never calls `FBD_Execute`.

For a burst this repeats `g_rawCount` times (or until the source yields no more frame), each frame a separate node, all sent in
order and released in reverse (§6.3).

#### 6.6.5 mode 3 — ISP output (before the scaler)
`GetIspOutput`: `MI_ISP_CUS3A_GetImageResolution` for geometry, one MMA buffer of `w × h × 2`, then
`MI_ISP_CUS3A_EnableISPOutImage` with node = ISP-out, polling the count for up to 1 s; the buffer is handed back as one node
(kind 1). This is the demosaiced / processed image at the ISP output tap.

#### 6.6.6 mode 5 — JPEG
`GetJpeg` (0x45b8): it locates the SCL module feeding the ISP (handling an LDC stage in between), then builds a temporary JPEG
path — `MI_VENC_CreateChn(8, 0x20, …)` as a JPEG channel, `MI_SYS_BindChnPort2` from the SCL port to it (REALTIME),
`MI_VENC_StartRecvPic`. It waits on the encoder fd (select, limited retries), `MI_VENC_Query` for a ready stream,
`MI_VENC_GetStream`, concatenates the stream packs into the response buffer, `MI_VENC_ReleaseStream`, then tears the channel
down (`StopRecvPic`, `UnBindChnPort`, `DestroyChn`) and disables the SCL port if it created it. If the JPEG VENC device is
already bound REALTIME it bails with "jpeg venc deivcie is already realtime mode bind!".

#### 6.6.7 Raw bit-unpacking
`ConvertRawImageTo16Bits` (0x4338) / `ConvertOneLineTo16Bits` (0x3cd0 0x13cd0) expand packed sensor raw to
one 16-bit sample per pixel, bottom-up (last line first). Supported packings: 8-bit (byte → `<< 8`), 10-bit (4 pixels per 5
bytes), 12-bit (2 pixels per 3 bytes); any other width → "unsupport bit width". Row size is rounded per packing: 8-bit
align16(W) bytes, 10-bit 64-pixel groups × 0x50 bytes (⌈W/64⌉·80), 12-bit 32-pixel groups × 0x30 bytes (⌈W/32⌉·48). The
compressed case (VIF compress mode 4) is handled by the FBC decoder instead.

**Where the FBC data comes from.** [S] (forum sources): on SigmaStar the VIF writes raw
above 8 bpp as 8-bit FBC by default (`mi_vif` parameter `g_bFBCMode`, `FBC` in `/proc/isp/isp_debug`). This camera's mi_vif has
`g_bFBCMode` = 1, but the port FBC flag is 0 for a REALTIME bind (mi-capture.md §3.6). [I] The VIF FBC hardware is the producer
of the 1x8 format of §7.3, and the encoder of §7.4 is the vendor's model of it; whether the hardware's mode choices equal the
model's is not established — a decoder needs only §7.3.

### 6.7 File transfer (commands 8 and 7)
<!-- ev:isp-6-userspace.6.7 -->
**Download — serving `isp_api.xml` (command 8).** `MI_IQSERVER_DownloadFile` (0x67f4, Ghidra 0x167f4; prog_uvc 0x4664c, called
at prog_uvc 0x46676). The request payload begins with `u16 fileId`. Only `fileId == 2` (XML_DATA in the MStar
`FILE_TRANSFER_ID` enum) is handled; any other id → −1. File id 1 (CALIBRATION_DATA) exists in the MStar enum but has no
handler. It calls `IQSERVER_load_xml_data` (0x931c, Ghidra 0x1931c; prog_uvc 0x49080), which builds the path
`<iqserver_data_path>isp_api.xml` (default `/config/iqfile/isp_api.xml`), `access`es, `open`s, `stat`s and `mmap`s the file,
copies it into a fresh `malloc`'d buffer, unmaps, and returns it with its size; a missing file → "isp_api.xml not found!", −1.
The client buffer is checked with `MI_IQSERVER_CheckBuffSize`. The reply is `{u16 2; u16 1; u32 0; u8 xml[size]}`
(`respBuf.used = size + 8`); the bytes are the file verbatim, re-read on every request. The device never parses the XML (§8).

**Upload (command 7) and raw injection.**
- Reception: in the TCP command thread a frame with `cmd == 7` and `len ≥ 2` is received straight into a file by
  `mst_NoBlock_Recv_WriteTo_File` (prog_uvc 0x48d24): `fopen("/tmp/raw_data.raw", "wb")`; the first chunk must be > 16 bytes,
  its first 16 bytes are a header kept in memory and the rest is written; following chunks are appended until `len` bytes
  arrived; `recv` returning 0, `ECONNRESET` or another error than `EINTR` / `EAGAIN`, or 500 × 10 ms of `EAGAIN`, removes the
  file and fails.
- `MI_IQSERVER_UploadFile` (command 7 in `MI_IQSERVER_ProcessCmd`): checks that the header's `u32 size` (+4) equals
  `get_file_size("/tmp/raw_data.raw")` (else "data length(%d) is not equal to buff content size(%d)"); with header
  `u16 type == 3` and no injection thread running, starts `PutVpeRawDataThread` (prog_uvc 0x46790).
- Raw parameters come earlier from `MI_IQSERVER_SetApi`: API id **0x3201** stores width, height, format (12 bytes) — but only
  if the ISP channel's input is **not** bound REALTIME: `MI_SYS_GetBindbyDest(ISP 0x21, dev, chn)` bind type 4 → "realtime not
  support inject raw data!" and failure. API id **0x3202**: value 0 sets the stop flag, non-zero clears it.
- `PutVpeRawDataThread`: marks itself running, opens the file, `MI_ISP_GetChnParam` for the HDR mode (HDR on → the height
  field doubled); unbinds **VIF(6,0,0,0) → ISP(0x21,0,chn,0)**, sets the VIF output depth (1, 4); then every 30 ms until the
  stop flag: takes and drops one VIF frame, gets an ISP input buffer (`MI_SYS_ChnInputPortGetBuf`, buffer config: type 1,
  pts = `time()`, raw width / height / format from SetApi 0x3201) and fills it with the next frame-size slice of the file,
  wrapping to the start at EOF (file size not a multiple of the frame size → "raw data file size not match frame size." and
  stop). At the end: close, `remove("/tmp/raw_data.raw")`, clear the running flag and **rebind VIF → ISP with
  `MI_SYS_BindChnPort` 30→30** (the plain bind call, not the REALTIME `BindChnPort2` of start-up).
- On this camera the feature cannot run: prog_uvc binds VIF → ISP REALTIME ([S]), so 0x3201 is refused and
  the thread never gets raw parameters [R + I].

### 6.8 Serial transport
<!-- ev:isp-6-userspace.6.8 -->
**Enabling.** `getIspUartDebugEnable` (prog_uvc 0x438d8): `fopen("/tmp/isp_uart", "r")`; absent → 0. Reads one line
(≤ 0x100), `sscanf("%[^=]=%s\n", key, value)`; not two fields → 0. Two fields → 2, or 1 when `key == "nIspUartEnable"` and
`value == "1"`. **Any `key=value` line enables it** (the caller tests ≠ 0), e.g. `nIspUartEnable=0` returns 2. Nothing on the
camera creates `/tmp/isp_uart` (no writer in prog_uvc, kjd_lvgl or the scripts — [S]); [I] it is a developer
switch: a `nIspUartEnable=1` line in it before prog_uvc starts turns the console UART into an IQ Tool link.

**Start.** `MI_IQSERVER_Open`, after starting the TCP listener: if enabled, prints "isp uart debug enable.", finds the pid
of the first process whose `/proc/<pid>/status` Name is exactly `sh` (helper prog_uvc 0x437b8: walks `/proc`, directories
only, first line parsed with `"%*s %s"`), prints "sh pid is %d", and if found sends it **SIGSTOP** (signal 19) — the
serial-console shell freezes (§6.1) — and starts `thread_uart_listen_handle` (0x3750; prog_uvc 0x43634). That thread opens
`/dev/ttyS0` with `mst_OpenUartDevice("/dev/ttyS0")` = `open(O_RDWR|O_NOCTTY|O_NONBLOCK)`, 115200 baud
(`cfset[io]speed(B115200)`), raw 8N1 (`CS8|CREAD|CLOCAL`, no parity, canonical / echo off, `VMIN = VTIME = 0`),
`tcflush(TCIFLUSH)`; polls (`mst_UartSelect(fd, 3)`) for the 13-byte handshake `"iq uart start"`; on it prints "uart connect
start." and starts `process_uart_cmd`.

**Serial command loop** (`process_uart_cmd`, 0x33d0; prog_uvc 0x432b4): buffers 0xc800 B and an IQ data buffer from
`MI_ISP_Alloc_IQDataBuf`; the same command set and the same `MI_IQSERVER_ProcessCmd` dispatcher as TCP, with these
differences:
- Header and payload are read with `mst_UartRead` (a polling `read` with a retry budget of about len/110 × 10 ms) and responses
  are written with `mst_UartWrite`; `SendResponseHeader` / `SendResponseData` take a protocol argument (0 = socket `send`,
  2 = UART `write`).
- There is no special case for command 7, so no file reception over serial; `UploadFile` then fails its size check unless a
  stale `/tmp/raw_data.raw` happens to match.
- A header shorter than 8 bytes prints "uart close." and the request is dropped.
- A multi-buffer picture response still calls `SendResponseDataArray`, which sends through socket `send()`; [I] over a tty that
  fails (ENOTSOCK), so raw / ISP-output pictures are a TCP-only feature.
- While a serial session is active, Set-API un-escapes `"~2"` / `"~1"` (§6.5.4).

The serial path exits when the serial-command stop flag (set by Close) is seen; the listener keeps the port and re-opens it
after a failed read.

## 7. libfbc_decode — the FBC 1x8 model and decoder

### 7.1 What the library is [R unless tagged]
<!-- ev:isp-6-userspace.7.1 -->
- A **C++ software model ("C-model") of the ISP's frame-buffer compressor**, built from a generic IP-simulation framework,
  plus one C entry point `FBD_Execute` that uses only its decoder. ARMv7-A Thumb-2, VFPv3-D16 hard-float, C++ with libstdc++
  `__cxx11` ABI. Class / module names from the export list: `Compression1x8` (the FBC encoder + decoder), `IPBase` (base class of a simulated IP
  block: named parameters bound to INI keys / register fields), `Frame` and `Mat<PIXEL>` (image containers), `RegisterTable`
  (register-map file parser), `IniFileIO`, `CIniFile`, `CTkifstream`, minIni (`ini_gets`…`ini_putf`), `RandomTop`
  (random-parameter / random-image test generator), `DataDumper`, `CPPM2`, PPM / RAW / CRW / YUV file readers and writers.
- Version string printed by `FBD_Execute`: `"FBD start... v1.5"`.
- Dynamic dependencies: `NEEDED` only `libgcc_s.so.1` and `libc.so.6`. Its 131 undefined symbols include libstdc++
  (`std::__cxx11::basic_string`, iostreams, `_Rb_tree_*`, `random_device`, `operator new/delete`) and libm (`log2`, `pow`)
  **without** a `NEEDED` entry for `libstdc++.so.6` / `libm.so.6`: the library only resolves when the loading process already
  has them.
- [I] The framework parts (INI / register-table loading, random tests, PPM dumps, golden-reference text dumps) are the vendor's
  PC-side verification harness compiled for ARM unchanged; on the camera only `FBD_Execute` → decoder can run.
- [S] (forum video notes): the Windows IQ Tool's `libFbcDecode.dll` / `FBC_DLL.dll` are the PC counterparts of this
  library [I: same name, same role].

### 7.2 The .so is not loaded; the code runs as a static copy inside prog_uvc [R]
<!-- ev:isp-6-userspace.7.2 -->
- No binary or library in the dump lists `libfbc_decode.so` (or `libmi_iqserver.so`) as `NEEDED`, and no file contains the
  string `libfbc_decode` (byte search over the dump), so nothing `dlopen`s it by name either.
- prog_uvc carries a static copy of both libraries. The libfbc_decode copy is matched object by object with verdict A
  (immediates equal) for every placed function, e.g. (.so side in file vaddrs) `FBD_Execute` prog_uvc 0x62e54 ↔.so 0x14374,
  `Compression1x8::DecompressFrame` 0x6849c ↔ 0x1e1b8, `Decode_bitstream` 0x67fe4 ↔ 0x1dd00, `setSWinit` 0x63340 ↔ 0x1905c
 . Region sizes in prog_uvc sometimes cover several.so functions (e.g. 0x670a0 = 1592 B =
  `decodeTruncate` + `decodeEdge2` + `decodeEdge3`) because the region list merges neighbours, not because the code differs
  [R sizes add up].
- prog_uvc calls `FBD_Execute` from exactly two places (Thumb disassembly): `bl 0x62e54` at 0x46228 (inside the static
  `MI_IQSERVER_GetPicture` copy, 0x45050) and at 0x48428 (static `SendResponseDataArray`, 0x48390) — the same two call sites
  the.so `libmi_iqserver` has (§6.6.4). So the decoder is reached only through prog_uvc's IQ server (`-q`).

### 7.3 The compressed format ("FBC 1x8", Bayer raw 12 bit → 8 bit) [R]
Read from `Compression1x8::Decode_bitstream` (0x2dd00) and the ten `decode*` functions (0x2cafc–0x2db78), `DecompressFrame`
(0x2e1b8), `FBD_Execute` (0x24374) and the `Frame` accessors, and checked by running the vendor code: a Python decoder written
from this section was compared with `Compression1x8::Decode_bitstream` executed in unicorn on random blocks of
every mode under all 16 combinations of the four decoder options (300 and 3000 blocks per mode and option set, four times as
many in curve mode — about 500 000 blocks in all), and with `FBD_Execute` end to end on random frames 8×1, 16×2, 21×3, 37×2,
64×4 at the default options — no mismatch.

#### 7.3.1 Frame
- The compressed frame is a plain byte array, **one byte per pixel**, row stride = width, no header, no line or frame markers,
  no tile index. Each row is cut into blocks of **8 horizontally adjacent pixels = 8 bytes = 64 bits**, starting at x = 0.
  Blocks are independent: no prediction across blocks or rows, no state carried.
- Bit numbering below: **bit i = bit (i mod 8) of byte (i div 8)** of the block, i.e. the block read as a little-endian u64.
  Every field is read LSB first: field `U(off, n)` = `(block >> off) & (2^n − 1)`.
- Each block decodes to 8 values of 12 bits (0…4095). `FBD_Execute` stores each value **<< 4** into a u16 (MSB-aligned 16-bit
  raw, 0…65520).
- Row end: if width is not a multiple of 8, the last block of a row is still read as 8 bytes, but byte positions x ≥ width are
  taken from x = width − 1 of the same row (`Frame::GetPixel` clamps x to [0, w−1] and y to [0, h−1]); only outputs x < width
  are written. [I] A real encoder would pad that block the same way or align the stride to 8: the model's own compressor
  allocates `align8(width)` bytes per row (`allocateCompressedFrame` 0x2ddb8), while `FBD_Execute` assumes stride = width — the
  two agree only when width % 8 == 0.

#### 7.3.2 Mode tag
The mode is a prefix code in the top bits, read from bit 63 downward:

| bits 63, 62, … | mode | payload bits |
|---|---|---|
| `0` | Hadamard (transform) | 0–62 |
| `1 1` | Truncate | 0–61 |
| `1 0 0 0` | Edge3 | 0–59 |
| `1 0 0 1` | DPCM | 0–59 |
| `1 0 1 0` | Edge2 | 0–59 |
| `1 0 1 1 0` | Edge4 | 0–58 |
| `1 0 1 1 1 0` | Edge5 | 0–57 |
| `1 0 1 1 1 1 0` | Edge6 | 0–56 |
| `1 0 1 1 1 1 1 0` | Edge7 | 0–55 |
| `1 0 1 1 1 1 1 1` | MinMax | 0–55 |

Exception: when the option `curve_mode_en` (§7.3.6) is set, the tag is ignored and the block is 8 independent 8-bit curve
codes.

#### 7.3.3 Common reconstruction of an n-bit sample
Several modes code a pixel as its top n bits. Such a field v is expanded to 12 bits as `v · 2^(12−n) + half`, where
`half = 2^(11−n)` if `round_md == 0 and (round_zero == 0 or v != 0)`, else 0 (`round_md` = option `reg_comp_de_tr_round_md`,
`round_zero` = `reg_comp_de_tr_round_zero`, both 0 by default, so by default every such field gets the half step). Examples:
7 bit → 32v + 16, 8 bit → 16v + 8, 9 bit → 8v + 4, 10 bit → 4v + 2. No clamp is needed (the result stays below 4096).

#### 7.3.4 The ten modes
**Hadamard** (tag `0`):

| bits | field |
|---|---|
| 0–6, 7–13, …, 42–48 | seven 7-bit two's-complement coefficients; the field at bits 7k…7k+6 is coefficient c[6−k] (bits 42–48 = c[0], bits 0–6 = c[6]) |
| 49–60 | DC, 12 bit unsigned |
| 61, 62 | q = b61 + 2·b62 |

`out[r] = clamp0_4095( (8·DC + Σ_{j=1..7} W[r][j] · (c[j−1] << (8−q))) >> 3 )`, arithmetic shift, where W is the
sequency-ordered 8×8 Walsh–Hadamard matrix stored at file vaddr 0x251b8 (`rodata_251b8`; row r = output pixel r, column j =
sequency j; W is symmetric):
```
r0: + + + + + + + +     r4: + - - + + - - +
r1: + + + + - - - -     r5: + - - + - + + -
r2: + + - - - - + +     r6: + - + - - + - +
r3: + + - - + + - -     r7: + - + - + - + -
```
(column 0 is all +1 and multiplies 8·DC). The left shift of a negative coefficient is a plain two's-complement shift.

**Truncate** (tag `1 1`): pixel i is a 7-bit field at

| pixel | 0 | 1 | 2 | 3 | 4 | 5 | 6 | 7 |
|---|---|---|---|---|---|---|---|---|
| bits | 51–57 | 44–50 | 37–43 | 30–36 | 23–29 | 16–22 | 9–15 | 0–6 |

reconstructed per §7.3.3 (32v + 16). Bits 7, 8, 58, 59, 60, 61 are unused unless option `reg_tr_ext_en` is set; then bit 60 =
parity p and four pixels get one more LSB, becoming 8-bit fields (16v + 8): p = 0 → pixels 1, 3, 5, 7 take extra LSB from bits
59, 58, 8, 7; p = 1 → pixels 0, 2, 4, 6 take them from bits 59, 58, 8, 7 (in that order). Bit 61 is never read.

**DPCM** (tag `1 0 0 1`):

| bits | field |
|---|---|
| 0–2 | s, shift 0…7 |
| 3–5 | P, index of the pixel that equals the base |
| 6–11, 12–17, …, 42–47 | seven 6-bit deltas d, assigned to pixels 7, 6, 5, …, 0 in that order, skipping pixel P |
| 48–59 | base, 12 bit |

`out[P] = base`, every other `out[j] = min(4095, (d_j << s) + base)`. The rounding options do not apply.

**MinMax** (tag `1 0 1 1 1 1 1 1`):

| bits | field |
|---|---|
| 0–15 | eight 2-bit selectors; bits 2i, 2i+1 belong to pixel 7−i |
| 16–27 | A, 12 bit |
| 28–39 | B, 12 bit |
| 40–55 | not read |

selector 0 → `(A + B) >> 1`, 1 → B, 2 or 3 → A.

**Edge2 … Edge7** (tags `1010`, `1000`, `10110`, `101110`, `1011110`, `10111110`): a run of N "smooth" pixels coded as small
deltas over a base, the remaining 8 − N pixels coded individually by their top bits. A direction bit D chooses which end of the
block holds the smooth run: position k = 0…7 is pixel k when D = 1 and pixel 7 − k when D = 0. Layout, all fields packed from
bit 0 upward in this order:

| mode | N smooth (positions 0…N−1) | delta width | base | individual pixels (positions N…7), widths in order | D bit | s bits | unused |
|---|---|---|---|---|---|---|---|
| Edge2 | 2 | 3 | 9 bit, × 8 | 7, 7, 7, 7, 7, 7 | 57 | 58–59 | — |
| Edge3 | 3 | 3 | 10 bit, × 4 | 7, 8, 8, 8, 7 | 57 | 58–59 | — |
| Edge4 | 4 | 3 | 10 bit, × 4 | 8, 9, 9, 8 | 56 | 57–58 | — |
| Edge5 | 5 | 3 | 10 bit, × 4 | 10, 10, 10 | 55 | 56–57 | — |
| Edge6 | 6 | 4 | 10 bit, × 4 | 10, 10 | 54 | 55–56 | — |
| Edge7 | 7 | 4 | 10 bit, × 4 | 10 | 53 | 54–55 | 48–52 |

So for Edge3: deltas at bits 0–2, 3–5, 6–8; base 9–18; individual 19–25, 26–33, 34–41, 42–49, 50–56. Smooth pixel at position
k: `min(4095, (delta_k << s) + base·mult)`, s = b(s0) + 2·b(s0+1) (0…3), mult 8 for Edge2 and 4 otherwise. Individual pixels:
§7.3.3 with their width.

#### 7.3.5 Decoder options and their defaults
The decoder reads only five fields of the `Compression1x8` object; `FBD_Execute` takes all of them from `setSWinit` (0x2905c),
so the defaults below are what the camera's IQ server uses (values printed by running `setSWinit` in the emulator):

| object offset | INI / register name (constructor `bind`) | default | effect in the decoder |
|---|---|---|---|
| +0xc0 | `reg_comp_de_tr_round_md` | 0 | 1 = never add the half step (§7.3.3, Truncate) |
| +0xc4 | `reg_comp_de_tr_round_zero` | 0 | 1 = no half step for a zero field |
| +0xd4 | `reg_tr_ext_en` | 0 | Truncate extra-LSB scheme |
| +0x134 | `reg_curve_mode_en` | 0 | curve mode (§7.3.6) instead of the tag modes |
| +0x1f0 / +0x22c / +0x268 | `reg_comp_lut_out`[15] / `reg_comp_lut_in`[15] / `reg_uncomp_lut_slope`[16] | §7.3.6 | curve tables (inverse direction) |

`Decode_bitstream` also sets +0x2ec = 1 whenever it decodes in curve mode; nothing in the library reads +0x2ec (no other read
in the decompile). +0x2ec is the field in which the encoder records its chosen mode number (1 = Curve, §7.4.2).

#### 7.3.6 Curve mode (`reg_curve_mode_en` = 1; off on the camera)
Each byte x of the block is decoded alone by `curveUncompress` (0x28fa4), a 15-knot piecewise-linear map with Q8 slopes, then
clamped to 0…4095:
- `x ≤ in[0]` → `y = interp(0, x, slope[0])`;
- else the first i in 1…14 with `x ≤ in[i]` → `y = interp(out[i−1], x − in[i−1], slope[i])`;
- none → `y = interp(out[14], x − in[14], slope[15])`;
- `interp(a, b, c) = a + ((b·c + 128) >> 8)` for c > 0, else `a − ((b·(−c) + 128) >> 8)` (64-bit product, `curve_interp` (0x28ea0)).

Default tables (decoder side): in = 32, 128, 220, 255 × 12; out = 32, 512, 2048, 4095 × 12; slope = 256, 1280, 4274, 14973,
0 × 12. That is identity up to 32, then slopes ≈ 5, 16.7, 58.5. The encoder side (`curveCompress`, 0x28f64) uses in = 32, 512,
2048, 4095 × 12; out = 32, 128, 220, 255 × 12; Q12 slopes 4096, 819, 245, 70, 0 × 12, clamp to 8 bit.

#### 7.3.7 Frame decode, as `FBD_Execute` does it
```
for y in 0..h-1:
  for x0 in 0, 8, 16, … while x0 < w:
    block = bytes src[y*w + min(x0+j, w-1)] for j = 0..7       # little-endian u64
    v[0..7] = decode_block(block)                              # 12-bit values
    for j in 0..7: if x0+j < w: dst[y*w + x0 + j] = v[j] << 4  # u16
```

### 7.4 The encoder side of the model (`Compression1x8` compress path) — unused on the camera [R unless tagged]
`FBD_Execute` never calls it; the camera's compressed frames come from the VIF hardware (§6.6.7). Read from the decompile
(0x28e98–0x2e4b8) and checked by running the vendor code: a Python port written from this section was compared
with `Compression1x8::EncodePixel8` executed in unicorn on structured random 8-pixel blocks (random, small range, smooth run +
edge on either side, flat, two-level, extremes, ramps) under 31 parameter sets (defaults, each option on its own, threshold
changes, curve, two `mode_definition` remaps, 12 random `mode_en` masks), comparing the output block, the chosen mode and every
object field listed below — 93 000 blocks, 0 mismatches. [I] The model's encoder is the vendor's reference for the hardware
compressor; that the VIF hardware makes the same choices is not shown, only that its output must be decodable by §7.3.

#### 7.4.1 Frame loop (`CompressFrame` 0x2dde0, `allocateCompressedFrame` 0x2ddb8)
- Input: a `Frame` of int pixels, **12-bit values expected** [I: every error and clamp works against 0xfff]. Output frame width
  = `align8(w)` (`allocateCompressedFrame`, when Enable = 1), bytes 0…255.
- Per row y, per 8-pixel block: the 8 pixels are copied; **a partial last block is padded with zeros** (the decoder side,
  §7.3.1, instead repeats the last byte). Block → `EncodePixel8` → 64 bits → 8 output bytes, bit order as §7.3.1.
- Row parity flag passed to the Truncate logic: `parity = (y & 1) ^ (cfa ∈ {1, 3})`, cfa = `TGlobalParam+0x44`
  (`Compression1x8::execute` passes it as the "CFA" argument). [I] With the usual enum order (0 RGGB, 1 GRBG, 2 BGGR, 3 GBRG)
  this marks the green pixels of each row as the ones that get the Truncate extra bit (§7.3.4).
- DPCM swing parity +0x3e4: if `sw_dpcm_swing_en` (+0x2ac) = 0 it is set to `reg_dpcm_swing_rounder_en` (+0xc8), else to
  `frame_index & 1` (alternates per frame).
- Enable (+0xa4) = 0 → the output is a plain copy of the input (same for `DecompressFrame`).

#### 7.4.2 Per block: statistics and the ten error estimates (`EncodeCalculateSAD` 0x2ab50)
min / argmin / max of the 8 pixels (+0x2d8 / +0x2dc / +0x2e4; first strict minimum, start 0xfff; max start 0). Then each mode's
reconstruction is simulated and its **maximum** absolute error stored (only the maxima drive the choice; the sums are kept but
never read):

| mode no. (+0x2ec) | mode | max error / sum | how the encoder codes it |
|---|---|---|---|
| 0 | Hadamard | +0x358 / +0x2f0 | T = W·p (§7.3.4 matrix). q = largest of 3, 2, 1, 0 such that every T[1..7] lies in [−2^(14−q), 2^(14−q)); DC = (T0 + 4) >> 3 (0xfff if T0 + 4 ≥ 0x8000); c[k] = (T[k+1] + 2^(7−q)) >> (8−q), positive values first clamped to 0x3fff >> q. Error from the §7.3.4 reconstruction (negative recon → error = p). |
| 1 | Curve | — | only when `reg_curve_mode_en`; each pixel → `curveCompress` (§7.3.6) |
| 2 | MinMax | +0x35c / +0x2fc | mid = (max + min) >> 1, t = ceil((max − mid)/2); \|p − mid\| < t → selector 0, else p < mid → 1 (min), else 2 (max). A = max, B = min. |
| 3 | DPCM | +0x364 / +0x2f8 | range r = max − min; s = 0 if r < 64, else 1…6 for r < 128, 256, 512, 1024, 2048, larger; s ≤ 3 + `reg_dpcm_stepsize_expansion_en`. rounder = [0,1,2,4,8,16,32][s], or with `reg_dpcm_swing_rounder_en` the row [0,0,1,3,7,15,31] when swing parity = 1. base = min; with `reg_dpcm_pmin_lsb_update_en` base's low s bits are replaced by the rounded mean of the pixels' low s bits (±2^s correction to stay within the rounder window, clamp 0…4095). d = trunc((p − base + rounder) / 2^s), ≤ 63. Error only computed if r < 64 << (3 + expansion), else max = 0xfff. Asserts `Diffmin[Pminpos] == 0` (file `Compression1x8.cpp` line 793) — [I] reachable only when base is clamped. |
| 4 | Truncate | +0x360 / +0x2f4 | error \|(p & 31) − 16\| (7-bit fields), or \|(p & 15) − 8\| for the pixels getting the extra bit when `reg_tr_ext_en` |
| 5…10 | Edge2…Edge7 | +0x368…+0x37c / +0x300…+0x314 | below |

EdgeN estimate (N = mode − 3; per-mode slots at +4·(N−2) from: step +0x384, direction +0x39c, right-side spread +0x3b4,
left-side spread +0x3cc): L = pixels 0…N−1, R = pixels 8−N…7; base_X = min(X) & ~7 (Edge2) or & ~3; spread_X = max(X) −
base_X; D = 1 (smooth run on the left) if spread_L < spread_R else 0; m = min(spread_L, spread_R). Candidate only if m < 64
(3-bit deltas) or < 128 (4-bit, Edge6 / 7) **and** m ≤ `reg_edge_diffmin_th`; otherwise max error 0xfff. Step = 1, 2, 4, 8 for
m < t, 2t, 4t, else (t = 8 for 3-bit, 16 for 4-bit deltas). Smooth pixel delta = trunc((p − base + step/2) / step), capped at 7
or 15; individual pixels' error per §7.3.3 width.

#### 7.4.3 Mode choice (`Encode_modeselect` 0x2abf8)
Let E[m] = max error of mode m, or 0xfff when `reg_mode_en[m]` = 0; ne = min(E0, E2, E3, E4); em = min(E5…E10).
- `reg_compress_edgefisrt_enable` = 1 (default): if DPCM is the ne-minimum and ne ≤ `reg_compress_edgefisrt_finalth` → DPCM;
  else if MinMax is it and ≤ finalth → MinMax; else if Hadamard is it and ≤ finalth → Hadamard. Otherwise, if em ≤
  `reg_compress_edgemin_th`, try Edge7 (needs E10 ≤ min(E5…E9)), then Edge6…Edge2 (need E = em), and take the first whose two
  sides are **not** "similar": similar = spread_L ≤ spread_R·ratio and spread_R ≤ ratio·spread_L, ratio =
  `reg_compress_edgemin_ratio` (default 1, i.e. only equal spreads are rejected). Failing all, fall back to the ne-minimum in
  the order DPCM, MinMax, Hadamard, else Truncate.
- = 0: global minimum over all modes, ties broken DPCM, MinMax, Edge7, Edge6, …, Edge2, Hadamard, Truncate.
- any other value: like 0 but against ne only (edge modes win only on an exact tie with ne).
- Then: `reg_curve_mode_en` → mode 1. Finally remap: if `reg_mode_definition[mode]` ≠ 1 the mode becomes
  `reg_mode_definition[mode]` (default table 0…10 = identity; quirk: a remap to 1 is impossible).

Defaults (`setSWinit`): edgefirst 1, finalth 2000, edgemin_th 2000, edgemin_ratio 1, edge_diffmin_th 200, every `reg_mode_en`
= 1 (including index 1, which the selection never reads).

#### 7.4.4 Packing (`Encode_bitstream` 0x2ca5c → `encode*`)
Exactly the inverse of §7.3.4 (verified bit for bit): Hadamard writes c[6]…c[0], DC, q, tag 0. MinMax writes the selectors of
pixels 7…0, A = max, B = min, zeros in 40–55. DPCM writes s as `log2(step)` (`Compression1x8::log2` 0x2b260: index of the
lowest set bit for 1…64, else 0), argmin as P, deltas of pixels 7…0 skipping P, base. Truncate writes `v >> 5` per pixel and,
with `reg_tr_ext_en`, bit 4 of the four extra-bit pixels into bits 59, 58, 8, 7 and the parity into bit 60. With
`reg_comp_enc_trun_rnd` (+0xbc) individually coded pixels are first rounded: +16 (7-bit) / +8 (8-bit, Truncate extra-bit
pixels) / +2^(11−w) (Edge fields of width w), clamped to 0…4095; without it they are truncated. Edge modes write the deltas,
`min(run) >> (12 − base bits)`, the individual fields, D, the step code (1→0, 2→1, 4→2, 8→3) and the tag; Edge7 zeroes bits
48–52.

#### 7.4.5 Other encoder-side members
- `encodeCurve` (0x28fe4) / `decodeCurve` (0x29024): 8 pixels ↔ 8 bytes through `curveCompress` / `curveUncompress`.
- `compressDecompressFrame` (0x2e430): allocate the compressed frame, `CompressFrame`, `DecompressFrame` — the lossy round trip
  as one IP step.
- `execute` (0x2e4b8, vtable slot of `IPBase::execute`): returns at once if the constructor's bool (+0x2b0) is set, else
  `compressDecompressFrame(in[0], out[0], cfa = TGlobalParam+0x44, frame = TGlobalParam+0x4)`.
- `randomSpecificParam` (0x28e8c) returns 0 (nothing randomised); `init` (0x28e98) tail-calls `IPBase::loadParam`;
  `isEnabled` (0x2f408) returns +0xa4 != 0; `setEnable` (0x2f418) writes +0xa4.
- Golden-reference dumps (only when +0x98 ≥ 1, never on the camera): `%s/%s_golden_ref_com_input_%04d.txt` and
  `%s/%s_com_input_%04d.raw` (input pixels as 2 bytes LE), `%s/%s_golden_ref_com_%04d.txt` (compressed bytes,
  `DataDumper::dumpNByteAlignedData(…, 8, 8)`), `%s/%s_golden_ref_dec_input_%04d.txt` (each block as `%02x_…` bytes 7…0),
  `%s/%s_golden_ref_dec_%04d.txt` (decoded frame, `dumpNByteAlignedData(…, 12, 12)`); first `%s` = the std::string at +0x64,
  second = lower-case name (`fbc`).

### 7.5 `FBD_Execute`, the `Compression1x8` object, `IPBase` and the parameter machinery [R unless tagged]

#### 7.5.1 `FBD_Execute`
`FBD_Execute(const u8 *src, u16 *dst, int w, int h)` (0x24374), returns 1 always:
1. two `Frame`s on the stack (`Frame(CS_TYPE 0, CFA_TYPE_HW 0, depth 16, CFA_TYPE_RGBIR 8)`), a `Compression1x8` built on the
   stack with name `"FBC"` and skip = false;
2. `puts("FBD start... v1.5")`, `puts("FBD create buffer")`, both frames `Mat<PIXEL>::create(w, h)` (int pixels,
   zero-filled);
3. `src` copied byte by byte into the input frame (stride w);
4. `puts("FBD in process...")`; object +0x98 = 0 (no dumps), +0xa4 = 1 (Enable); `setSWinit()`; `DecompressFrame(in, out,
   frame 0)`;
5. output copied to `dst` as u16 (stride w); frames released, `puts("FBD end!")`, destructor.

Nothing is read from files or the environment: the decoder configuration is fixed by `setSWinit` (§7.3.5, §7.4.3). There is
no size check — `dst` must hold w·h u16, `src` w·h bytes; the MI_SYS buffer stride is ignored by the caller (§6.6.4). It
allocates 2 × w·h × 4 bytes of int frames on the heap (3840×2160: ≈ 66 MB) [R sizes; I the figure]. All output goes to
stdout via `puts`. With the camera's options (round_md = round_zero = tr_ext = curve = 0) the decoder needs no parameters and
no table except the Walsh matrix (§7.3.4). [I] If the VIF's FBC output stride differs from w, the IQ Tool's images are skewed;
for widths that are a multiple of 8 the question does not arise; not checked on hardware.

#### 7.5.2 Object layout

| offset | member | notes |
|---|---|---|
| +0x00 | vptr (`vtable for Compression1x8` 0x37e88 + 8) | slots +0x00 dtor, +0x04 deleting dtor, +0x08 `init`, +0x0c `allocateOutputFrame` (IPBase), +0x10 `execute`, +0x14 `dumpRegister` (IPBase), +0x18 `setEnable`, +0x1c `isEnabled`, +0x20 `randomSpecificParam`, +0x24 `getRegisterAddress`, +0x28 `getRegisterValue`, +0x2c `getRegisterValueSequence`, +0x30 `checkParam`, +0x34 `saveParam` (the last five IPBase's) [R vtable words + relocations] |
| +0x04 | `std::map<void*, IPBase::TVarInfo>` (header +0x08) | parameter bindings, key = address of the bound field |
| +0x1c | `std::map<void*, void*>` (header +0x20) | owned buffers; `IPBase::release` deletes every mapped value |
| +0x34, +0x4c, +0x64, +0x7c | four `std::string` | +0x4c = IP name ("FBC"), +0x64 = dump directory for golden-ref files; +0x34, +0x7c not used by Compression1x8 (no reference) |
| +0x98 | int dump level | golden-ref dumps when ≥ 1 and frame index ≥ 0 |
| +0x9c | int | index used by `RandomTop::isIPTestEn` |
| +0xa0 | byte | zeroed by the constructor, otherwise unused |
| +0xa4 … +0x2ac | bound parameters | §7.5.3 |
| +0x2b0 | bool skip | constructor argument; `execute` does nothing when set |
| +0x2b4 … +0x3e4 | per-block encoder state | §7.4.2 (Hadamard coefficients +0x2b4…+0x2d4, mode +0x2ec, sums +0x2f0…+0x314, DPCM deltas +0x318, MinMax selectors +0x338, maxima +0x358…+0x37c, steps +0x380…+0x398, directions +0x39c…, spreads +0x3b4…+0x3e0, swing parity +0x3e4) |

`TVarInfo` (map node payload from +0x14): section `std::string`, key `std::string`, element size (4), element count, bool "hex"
(only used when saving: `0x%x` instead of `%d`) (`IPBase::bind` 0x30b3c, `saveParam`).

#### 7.5.3 Parameters (constructor `bind` calls, defaults from `setSWinit`)
Constructor 0x2e560 `bind` calls; defaults from running `setSWinit` in the emulator. INI section = the IP name (`FBC`) except
for the enable flag. All are int, element size 4.

| offset | section / key | count | default | used by |
|---|---|---|---|---|
| +0xa4 | `Enable` / `reg_fbc_en` (`IPBase::getEnableKey` = "reg_" + lower(name) + "_en") | 1 | ctor 0; `FBD_Execute` 1 | both directions (0 = copy) |
| +0xa8 | `FBC` / `reg_compress_edgefisrt_enable` | 1 | 1 | mode choice |
| +0xac | `reg_compress_edgefisrt_finalth` | 1 | 2000 | mode choice |
| +0xb0 | `reg_compress_edgemin_th` | 1 | 2000 | mode choice |
| +0xb4 | `reg_compress_edgemin_ratio` | 1 | 1 | mode choice |
| +0xb8 | `reg_edge_diffmin_th` | 1 | 200 | Edge estimates |
| +0xbc | `reg_comp_enc_trun_rnd` | 1 | 0 | encoder rounding |
| +0xc0 | `reg_comp_de_tr_round_md` | 1 | 0 | **decoder** |
| +0xc4 | `reg_comp_de_tr_round_zero` | 1 | 0 | **decoder** |
| +0xc8 | `reg_dpcm_swing_rounder_en` | 1 | 0 | DPCM encoder |
| +0xcc | `reg_dpcm_pmin_lsb_update_en` | 1 | 0 | DPCM encoder |
| +0xd0 | `reg_dpcm_stepsize_expansion_en` | 1 | 0 | DPCM encoder |
| +0xd4 | `reg_tr_ext_en` | 1 | 0 | Truncate, **decoder** and encoder |
| +0xd8 | `reg_tr_ext_rb` | 1 | 0 | Truncate encoder (flips the extra-bit pixels) |
| +0xdc | `reg_mode_en` | 11 | all 1 | mode choice |
| +0x108 | `reg_mode_definition` | 11 | 0…10 | mode remap |
| +0x134 | `reg_curve_mode_en` | 1 | 0 | **decoder** and encoder |
| +0x138 / +0x174 / +0x1b0 | `reg_comp_lut_in` / `reg_comp_lut_out` / `reg_comp_lut_slope` | 15 / 15 / 16 | §7.3.6 | `curveCompress` |
| +0x1f0 / +0x22c / +0x268 | `reg_comp_lut_out` / `reg_comp_lut_in` / `reg_uncomp_lut_slope` | 15 / 15 / 16 | §7.3.6 | `curveUncompress` (**decoder**) |
| +0x2ac | `FBC` / `sw_dpcm_swing_en` | 1 | 0 | per-frame swing parity |

Quirk: `reg_comp_lut_in` and `reg_comp_lut_out` are each bound twice, to the compress and to the swapped uncompress copy;
`IPBase::bind` keys the map by field address, so both bindings exist and `loadParam` fills both from the one INI key — the
uncompress curve is the mirror of the compress curve, only the slopes are separate.

#### 7.5.4 `IPBase` methods (the generic simulated-IP base class)
None is called on the camera except `bind`, `getEnableKey`, `getName`, `getNameLower` (constructor, dumps) and `release`
(destructor).

| method | behaviour |
|---|---|
| `bind(void*, pair<section,key>, size, hex, count)` 0x30b3c | adds a `TVarInfo`; a second bind of the same address prints `Parameter "%s/%s" bind to a variable that was bound before!` |
| `setDefaultParam(RegisterTable&)` 0x2fcd8 | for every binding, copies the register-table entry's default list (+0x40 vector) into the field (count elements, last value repeated) |
| `loadParam(IniFileIO&, RegisterTable&, errors&)` 0x305f0 | `setDefaultParam`, then for every binding: entry must exist in the register table and the key in the INI file, scalar via `getVariable` (default = table default), arrays via `getArray` (comma-separated, hex if the token contains `x`); range check against entry min / max; messages "Cannot find parameter name…in the register table!/…in the INI file!", "Failed to get parameter…(Array length mismatch ?)", "…is out of range! %d is not in [%d, %d]!" appended to `errors`; returns 1 if all fine |
| `saveParam(IniFileIO&, RegisterTable&, errors&, bool)` 0x30c80 | writes every binding back with `setVariable` / `setArray` (hex per `TVarInfo`), "Failed to save parameter…" |
| `checkParam(RegisterTable&, errors&)` 0x30378 | range check only |
| `randomParam(RegisterTable&, bool, errors&)` 0x2fde4 | each binding := `randomVal(lo, hi)` from the entry's random range (+0x38 / +0x3c) or, with the bool, the normal range (+0x30 / +0x34) |
| `getParamInfo` 0x2f5a8, `getParamRange` 0x2f72c | entry lookup for a bound field; range (random or normal) |
| `getRegisterAddress(RegisterTable&, unordered_set<int>&, …)` 0x2ffa0 | collects the register addresses of every binding's register fields (`TRegEntity` list, entry +0x50) |
| `getRegisterValue(RegisterTable&, map<int,int>&, …)` 0x31040 | packs every bound value into register words with `setBits` (address → value) |
| `getRegisterValueSequence` 0x2f404, `dumpRegister` 0x2f400 | empty stubs (return 0 / nothing) |
| `run(in, out, gp, dir, frame)` 0x2f91c | dumps inputs (`Frame::dumpFrame` as `<name>_in` or `<name>_p%d_in`), allocates output frames with the inputs' CFA fields, calls virtual `execute` (vtable +0x10), copies the CFA fields back, dumps `<name>_out` |
| `execute` 0x2f4f0 (base) | output[0] = clone of input[0] |
| `allocateOutputFrame` 0x2f578 | push a `cloneFrame` of input[0] |
| `init` 0x31398, `setEnable` 0x3139c, `isEnabled` 0x313a0, `randomSpecificParam` 0x313a4 | base stubs (0 / nothing) |
| `getName` / `getNameLower` / `getNameUpper` / `getSectionNameLower` 0x2f870–0x2f8e4 | the name string, lower / upper-cased |
| `release` 0x2f540, `~IPBase` 0x313a8 / 0x31414 | free owned buffers, clear maps |

`setBits(int value, int msb?, int lsb?, int& word, …)` 0x2f480 and `readCFAColor(int x, int y, int cfa)` 0x2f4b4 are free
helpers (bit-field insert; colour index of a Bayer position) [R shape; I argument names].

### 7.6 The rest of the library: support classes and free functions (all unused on the camera)
<!-- ev:isp-6-userspace.7.6 -->
Reachability [R, static]: following direct calls and PLT stubs that resolve to the library's own exports, `FBD_Execute`
reaches 41 functions (file vaddrs): the decoder (§7.3), `Frame` ctor / `GetPixel`
/ `C` / `clone` / `operator=`, `Mat<PIXEL>::create/release`, the constructor and destructor with
`IPBase::bind/getEnableKey/getName/getNameLower/release` and their `std::map` helpers, `DataDumper::dumpNByteAlignedData/
packLine` (dump path, off at run time), and one `std::string` helper. Everything in this section outside that list is
**unused on the camera** (no other caller in prog_uvc or the dump; the RandomTop / PPM / IniFileIO parts are unreferenced in
prog_uvc too). Behaviour is [R] where a string, call or constant was read, [I] where it rests on the symbol name and code shape
only.

#### 7.6.1 Image containers
- `Mat<PIXEL>` (vtable 0x37e5c, 16 B): +4 width, +8 height, +0xc `int*` data (row-major), +0x10 `int*` refcount, +0x14 / +0x18
  x / y origin offsets. `create(w, h)` 0x24314 releases, allocates w·h ints zeroed (throws `bad_array_new_length` beyond
  0x1ffffffe elements), refcount 1; `release` 0x24254 frees at refcount 1 else decrements; `~Mat` 0x24284 / 0x242a4.
  `Mat<short>::create` 0x241dc: same with 2-byte elements. [R]
- `Frame : Mat<PIXEL>` (vtable 0x37e6c, 0x2c B): +0x1c `CS_TYPE` (2 = YUV, [R] `toYUV422` check), +0x20 bit depth, +0x24
  `CFA_TYPE_HW`, +0x28 `CFA_TYPE_RGBIR`. Constructors 0x20984 (CS, CFA, depth, RGBIR; no buffer) and 0x22240 (w, h, CS, depth,
  CFA, RGBIR; allocates). [R]
- Accessors `GetPixel`, `Y`, `Cb`, `Cr`, `U`, `V`, `C`, `R`, `G`, `B` (const and non-const, 0x20c34–0x20d10): all return the
  address / value of the int at the clamped (x + origin, y + origin) — one plane for every channel name (the 2-byte ones are
  branches into the shared body). [R]
- Conversions: `toYUV422` 0x20d14 (prints `[Err] not CS_YUV type ! ` unless CS = 2), `RGB2YUV` 0x210d0, `YUV2RGB` 0x21104
  (empty), `convertDepth(int)` 0x21108 (rescale to a new bit depth [I]), `Bayer2RGB(src, dst, cfa)` 0x20dc8 (5×5 neighbourhood
  with `mirroredPos` edge mirroring and `readCFAColor` [R calls]; demosaic [I]), `YUV422To444` 0x2238c,
  `splitChannel(Mat<short>*×3, Frame)` 0x21208, `toMat(Mat<short>&, int)` 0x21268, `crop(x, y, w, h)` 0x225fc (origin / size
  view [I]), `clone` 0x22324 / `cloneFrame` 0x2229c (deep copy / new heap Frame), `operator=` 0x21328.
- File I/O: `readPPM(name, …)` 0x2135c (appends `.ppm`, `ParsePPMFile_all`), `readCRW` 0x21670 (`.CR2` / `.CRW`,
  `ParseCRWFile`), `readRAWYC` 0x219f0 (`.raw`, `ParseYUV422File`), `readRAW` 0x21c8c (`.raw`, `ParseRAWFile` with a packing
  mode), `dumpFrame(dir, name, frame)` 0x209b8 (`%s/%s.bin`, 16-bit samples, "Failed to open file "), `dumpFramePat` 0x22e8c
  (text, header `//format : x y Y U V`), `storeAllTypePPM` / `storePPMYUV` / `storePPM_8b` / `storePPM_12b` / `storePPM` /
  `store` 0x2271c–0x240c4 (binary `P6` PPM; "store PPM : ", "Error : StorePPM Failed").

#### 7.6.2 Free file helpers (0x1c534–0x1dbb4)
`error_parse_file(name)` prints `Error : Parsing file <name>` to cerr and `exit(-1)`; `get_file_token(ifstream&)` reads one
alphanumeric token, skipping `#` comment lines (PPM header tokenizer); `string2dec` = decimal digits → int; `ParsePPMFile_all`
/ `ParsePPMFile` (P-format header via the tokenizer, 8- or 16-bit big-endian samples scaled to the requested depth);
`ParseYUV422File(name, w, h, out)` (2-byte samples, low 10 bits); `ParseRAWFile(name, w, h, out, mode)` (u16 samples
re-packed by mode: 0 nibble rotate, 1 >> 4, 2 byte swap >> 4, others bit-field extractions; prints "Width " / "Height ");
`ParseCRWFile(name, out, mode, w, h)` — [R] allocates a 40 000 000-byte buffer and converts it **without opening `name`**
(reads uninitialised memory; dead code). `StorePPMField`, `StorePPMFrame` (`%s.ppm`), `StorePPM` (8-bit), `StorePPM16B`
(16-bit, values << 6) write through `CPPM2`.

#### 7.6.3 `CPPM2` (0x255f0–0x25a2c)
An `ofstream` wrapper: `newppm(name, w, h, maxval)` / `newppm16b` open the file and write a PPM header (`P6`, w h, maxval − 1),
`appendpixel(r, g, b)` / `appendpixel16b` write one pixel (16-bit big-endian [I]) and count columns, `closeppm` closes ("ppm
file open error!" on failure). [R]

#### 7.6.4 `DataDumper` (0x25ad8–0x26100) — golden-reference text dumps
`packLine(frame, y, bits, buf, …)` packs one row's samples LSB first, `bits` each, into bytes;
`dumpNByteAlignedData(file, frame, bits, align)` writes every row so packed as `%02x` hex text, one line per row (used by
`CompressFrame` / `DecompressFrame` dumps with 8 / 8 and 12 / 12); `packLineTwoRow` / `dumpPackedDataTwoRow` (two rows
interleaved), `dumpPackedDataTileMode(file, frame, bits, align, tile_w, tile_h)`, `dumpPlainData` (int array or frame, plain).
[R strings / calls; I exact line layout]

#### 7.6.5 INI handling — three independent implementations
- **minIni** (CompuPhase, C, file-based, `INI_BUFFERSIZE` 4096): `ini_gets`, `ini_getl` (hex if the 2nd char is `x`),
  `ini_getf` (float), `ini_getbool` (Y / 1 / T → 1, N / 0 / F → 0), `ini_getsection`, `ini_getkey`, `ini_browse`, `ini_puts`
  (rewrites via a `<file>~` temp file and rename), `ini_putl`, `ini_putf`, plus its statics (`save_strncpy`, `writekey`,
  `skiptrailing`, `striptrailing`, `cleanstring`, `getkeystring`, `cache_flush`, `writesection`, `close_rename` — named by their
  assert strings). Comments `;` / `#`, key separators `=` and `:`. [R asserts and code]
- **`IniFileIO`** (0x245b8–0x251ec), a C++ front end over minIni, object 0x1030 B: +0 file name `std::string`, +0x18 4 KB text
  buffer, +0x1018 `map<section, set<key>>` filled by the constructor (enumerates every section ≤ 64 chars and key with
  `ini_getsection` / `ini_getkey`). `getVariable` (bool / int / double / string, default given as text, optional
  lower-casing), `getArray` (int: tokens split on `IniFileIO::m_seperator` = `","`, a token containing `x` parsed as hex;
  double), `setVariable` / `setArray` (`%d` or `0x%x`, `, `-joined, ≤ 4 KB), `isSectionExist`, `isKeyExist` (map lookups).
  Used by `IPBase::loadParam/saveParam` and `RandomTop`. [R]
- **`CIniFile` + `CTkifstream`** (0x1dbb8–0x20744): a tokenizing ifstream (`NextToken`: identifiers `[A-Za-z0-9_]`, numbers
  via `strtoul` base 10 or 16 after `0x`, `"…"` strings, `;` comments; token types 1 / 3 / 4 / 5 / 6) and an INI parser on top
  (`ParseOptions`, sections in `map<string, map<string, vector<TOption>>>`, the unnamed section is
  `CIniFile::_GlobalSectionName` = `"@GlobalSection"` set by `.init_array`; `SetSection`, `GetOption`, `GetFirstOption` /
  `GetNextOption`, `ShowAll`, stream operators, `firstopt`, `setsection` manipulators). [R] Nothing in the library calls it — an
  exported utility for a host program.

#### 7.6.6 `RegisterTable` (0x26100–0x289e4)
`read(table_file, bank_file)`: `readBankName2Addr(bank_file)` builds a bank-name → address map ("Bank name "%s" address is empty
in %s", "…is duplicated in %s"), then the table is read line by line, **19 tab-separated columns** per row, trimmed of spaces:
col 1 string, 2–5 `%d`, 6 comma-separated int list (the defaults), 7 bool, 8 bank name, 9–10 hex (optional `h` prefix), 11–12
small ints, 13 string, 14–16 ints, 17 hex (`h`) or binary (`b` prefix), 18 an assignment rule parsed by `parseAssignRule` (needs
`=`: "…: Missing '='", "WARNING! Fail to parse register assignment rule."); unknown bank → "ERROR! Address of bank "%s" is not
found!". Entries (`TParamInfo`, 0x5c B: section / key strings, +0x30 / +0x34 normal range, +0x38 / +0x3c random range, +0x40
default `vector<int>`, +0x50 `vector<TRegEntity>`) are indexed by (section, key) in a map (`lookupParam` → index, −1 if absent;
`getEntry(i)` = base + 0x5c·i); `getBankAddr(name)` (−1 if absent); `parseLoc("[a:b]…")` splits on `[`, `:`, `]` into ints.
[R strings, sscanf formats, offsets; I column meanings beyond the defaults / ranges]

#### 7.6.7 `RandomTop` (0x31cb0–0x34099) — random regression-test driver
Globals (`_INIT_9`): a `std::random_device` and an MT19937 state seeded from it (0x6c078965 recurrence, 624 words) plus a
`uniform_real_distribution(0, 1)`; `setRandSeed(seed)` re-seeds the MT state, `genRandSeed` draws from the random_device.
`randomVal(lo, hi)` → uniform int via that distribution [R shape]; `random_lutX` / `check_randlutX` make and check a random
monotone LUT with `rand()`. `RandomTop::loadSetting(ini, log, IPBase**)` reads section `GlobalControl`: `default`,
`default_setting_from_ini_en`, `pattern_start`, `pattern_num`, `module_en_random`, `normal_range`, `dst_path` (default `gold`),
`dump_frame_path` (default `gold`), `imgsize_random`, `img_width`, `img_height`, `data_depth`, `frame_number`, `ISP_en`,
`OSD_en`, and `random_<module>_en` per IP; `randomEnable`, `isRandomEnabled` (opens the INI and returns 0),
`useDefaultFromINI`, `isIPTestEn`, `randomIPParam` (→ `IPBase::randomParam` + virtual `setEnable`), `randomInputImage` (random
size / depth and input file pick; "randomInputImage runs too long", "image_size = ", …), `randomGlobalParameter`,
`reportError` (`[RandomTop] %s` to the log file). [R strings and calls]

#### 7.6.8 Runtime glue and template instances
`.init_array`: `_INIT_2` … `_INIT_9` — one `std::ios_base::Init` per translation unit, `CIniFile::_GlobalSectionName`, the
RandomTop RNG globals. `_DT_INIT` / `entry` / 0x1b1e0–0x1b264 are the usual crt (`__gmon_start__`, TM clone
table, `__cxa_finalize`). `__aeabi_uidiv` (0x34288) / `__aeabi_idiv_b` (0x34500) / `__aeabi_idiv_c` (0x34506) = libgcc `__aeabi_uidiv` / `__aeabi_idiv` style
division (signed truncating, the DPCM / Edge `trunc` in §7.4), `curve_interp` / `curve_interp_b` (0x28f24) = the curve interpolation
helpers (§7.3.6). The remaining exports are `std::` template instantiations (`_Rb_tree` insert / erase / find / copy for the
maps above, `vector::_M_realloc_insert/emplace_back` for `TOption`, `TRegEntity`, `TParamInfo`, `int`, `Frame*`, `string`,
`TImageEntity`, `basic_string::_M_construct`, `stringbuf` dtors, `random_device` ctor / dtor, `generate_canonical`,
`operator+`) with library semantics.

### 7.7 Export index [R]
<!-- ev:isp-6-userspace.7.7 -->
All 342 export rows collapse to 322 distinct (address, name) entries; each is specified in §7.3–§7.6 [R]; whether
`FBD_Execute` reaches each one is noted there.

| section | what | entries | reached from `FBD_Execute` |
|---|---|---|---|
| 7.3, 7.5.1 | decoder + `FBD_Execute` | 16 | all 16 statically (`decodeCurve` / `curveUncompress` run only with curve mode on) |
| 7.4 | encoder side of `Compression1x8` | 38 | constructor / destructor only |
| 7.5.2–7.5.4, 7.6 | IPBase, RTTI / vtables, Frame / Mat, file helpers, CPPM2, DataDumper, three INI layers, RegisterTable, RandomTop | 227 | the 41-function list of §7.6 |
| 7.6.8 | `std::` instantiations | 41 | the map helpers used by `bind` / the destructor |

Unused on the camera: everything not reached from `FBD_Execute` (§7.6), plus inside the reached set the dump code (+0x98 = 0),
the Enable-0 copy path and curve mode (§7.3.5 defaults).

## 8. isp_api.xml [R unless tagged]
- File: `/config/iqfile/isp_api.xml` = `/config/iqfile/isp_api.xml`, 164 700 bytes, CRLF text, md5
  `0309415a196ed4365957eef6fd5e3ce7`, root `<ISP_ITEM Author="Smart Camera ISP" Version="1.2" Name="Tiramisu">`. `/config` is
  the read-only squashfs miservice partition (mtd3, `mount -t squashfs /dev/mtdblock3 /config`, rootfs-customer.md §2.3,
  firmware-image.md §3.6), so it ships with the SDK release image next to the imx307 IQ files (not with the gc8613 IQ file in
  `/customer`). Content: 99 groups, the field-name source for the IQ API structs (isp-3-iq-api.md §3.1).
- Reader and server: the IQ server only (§6.7) — path `iqserver_data_path` + `"isp_api.xml"`, `iqserver_data_path` a 32-byte
  global initialised to `"/config/iqfile/"` in the.data of both libmi_iqserver and prog_uvc, fixed on the camera (§1.2).
  Served verbatim on command 8 with file id 2, re-read on every
  request. Nothing else on the camera reads it (byte search of the dump: only prog_uvc and libmi_iqserver.so contain the name).
- The device never parses the XML; the tool builds its whole GUI, the group list and every struct layout from it, so the
  layouts it describes must agree with what `MI_ISP_GET_ALL` returns for each id [I].
- [S] (forum video notes): the Windows IQ Tool first asks the board for this XML and falls back to a local
  `<platform>_isp_api.xml`.
