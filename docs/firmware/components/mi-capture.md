# mi-capture — mi_sensor, mi_vif, mi_isp, mi_scl and their user libraries
<!-- ev:mi-capture -->

The four MI kernel modules of the capture side and the user libraries that drive them. `mi_sensor.ko` passes sensor
calls through to mhal's SNR block; `mi_vif.ko` (video input), `mi_isp.ko` (image signal processor) and `mi_scl.ko`
(scaler) are **mi_sys device drivers**: their ioctls validate and store the configuration, and the hardware is driven from
the mi_sys task callbacks and the interrupt handlers, which wake the mi_sys worker with `mi_sys_NotifyWakeupISREvent`.
User side: `libmi_sensor.so`, `libmi_vif.so`, `libmi_isp.so` (with its IQ / AE / AWB / AF API, the routing of that API
to libcus3a and the API-bin and calibration loaders) and `libmi_scl.so`. Also here: the two hooks mi_scl calls in mi_rgn.

Other documents: the user-side ioctl wrapper, payload packing, return codes, error-code format, module ids and the
fd lifetime of every library — `mi-user-abi.md` §2, §3, §4; the kernel-side device frame (`/dev/mi_*`, ioctl dispatch, per-module procfs
and debug parameters, mi_sys exports) — `mi_sys-2-interfaces.md`; the registration contract, bind types and bind
capability, buffer geometry and ext-conf, the sideband message, worker threads and CMDQ objects — `mi_sys-1-engine.md`; the
sensor ABI and the `MHal_SNR_*` side — `mhal-csi-snr.md`; the `MHal_VIF_*` side — `mhal-vif.md`; the ISP interface under
mi_isp (`MHalVpeIsp*`) — `isp-2-ispmid-mload-ispscl.md` §3; ISP API ids and payloads — `isp-3-iq-api.md`; the user-space 3A
(libcus3a, libispalgo) — `isp-5-3a.md`, `isp-6-userspace.md`; the SCL / CMDQ side (`MHalVpeScl*`) —
`mhal-scl-jpe-cmdq.md`; mi_rgn — `rgn-gop.md`; the app's calls — `prog_uvc.md`.

Tags: **[R]** read from the binary, a decompile or disassembly; **[S]** stated by a source; **[I]** inference. A heading
tagged [R] means every statement under it is [R] unless marked otherwise.

Binaries: `/config/modules/4.9.227/mi_{sensor,vif,isp,scl}.ko` (MI build 2022-07-21, `project_commit.7251436
sdk_commit.80327e1`); `/config/lib/libmi_{sensor,vif,isp,scl}.so` (the same code as the static copies linked into
prog_uvc). A Ghidra `.text` address = objdump `.text` address + 0x10028 for all four modules. **mi_vif decompile:**
wherever Ghidra forms a pointer into the module state by adding a large constant (`iVar + 0x20bec`, `+ 0x2083c`, `+
0x252cc`), the constant is 0x20780 too large (Ghidra folded the base symbol `g_stVifModRes`, `.bss + 0x258`, into the
literal); plain array indexing is correct; every mi_vif offset in this document is the corrected one. Source line
numbers quoted below are the `__LINE__` values the modules pass to their log calls.

Struct names are from the public M6 / Tiramisu headers (skyipc_tiramisu `mi_vif_datatype.h` and others).

## 1. Common to the four modules

### 1.1 The capture chain on this camera
<!-- ev:mi-capture.1.1 -->
The app (`prog_uvc.md`) uses: mi_sensor; VIF group 0, dev 0, port 0, bound **REALTIME** to ISP dev 0 channel 0; ISP
output port 0 bound **REALTIME** to SCL dev 0 (mask 7 = HWSCL0..2) channel 0; SCL output ports bound FRAME_BASE (or HW_RING) to
VENC and DISP (all binds: `mi_sys-1-engine.md` §3.4) [R]. With REALTIME on both sides of the ISP, VIF writes nothing to
DRAM and the ISP reads the sensor stream directly: no capture-side DRAM buffer exists before SCL's outputs [I from the code
paths]. ISP and SCL are programmed per frame through the mi_sys CMDQ (hardware command queue) [R]; the CMDQ object handed to
their callbacks comes from mi_sys's `cmdQBufSize=3072` pool [I]. The sensor is not an mi_sys data source: mi_sensor never
binds, VIF reads the MIPI data itself [R]. The frame-mode paths (VIF frame queue, ISP frame input, SCL devices 1–3) are not
reached on this pipeline [I].

What SigmaStar's MI documentation states for Tiramisu (= Mercury6) [S]: ISP "Max Pixel Rate" 4K30 (MI ISP page, chip-difference
table) and SCL "Max Pixel Rate" 4K30, six SCL hardware scalers (MI SCL page; the six `HWSCL0..5` of §6 [R]); VIF port 0 has no
crop or scaling on Tiramisu, port 1 scaling is meant for YUV input; and a VIF that receives without errors does not guarantee
that the ISP has no FIFO overflow (Sensor Porting page, Tiramisu_IPC) [S]. These are stated
ceilings of the documentation, not limits read from the binaries.

### 1.2 Device frame, locking, error codes [R]
Device node, ioctl dispatch and the procfs directory are mi_common's (`mi_sys-2-interfaces.md` §1). One mutex per module
serialises all ioctls (mi_common holds it across the handler); VIF, ISP and SCL add per-device or per-channel rwsems shared
with the worker callbacks. Error codes are `0xA0MMxxxx` with module field 0x1B (sensor), 0x06 (VIF), 0x07 (ISP), 0x22 (SCL);
the format is `mi-user-abi.md` §4.2's.

### 1.3 Chip identification [R]
`MI_VIF_IMPL_Init`, `MI_ISP_IMPL_Init` and `MI_SCL_IMPL_Init` match `Chip_Get_Platform_Name()` against a static table and store
the index together with `CamOsChipRevision()`. In mi_vif the table is at `.rodata + 0x504`: 7 entries `{u32 index; char
*name}` (8 bytes each), names in `.rodata.str1.4`:

| chipIdx | platform name |
|---|---|
| 0 | `I5` |
| 1 | `I6` |
| 2 | `I2M` |
| 3 | `I6E` |
| 4 | `I6b0` |
| 5 | **`M6`** |
| 6 | `P3` |

No match → chipIdx 7. This camera is M6, chipIdx 5. The only chipIdx branch any of the three modules reaches on this pipeline's
code paths is the VIF port-1 errata (§3.11), which needs chipIdx 5 and revision 0. The value `CamOsChipRevision()` returns on
this chip is not known from static data (a camera reading; chip id and revision: `kernel-platform.md` §3.2); the same reading decides
mhal SCL's revision-2 init branch (`mhal-scl-jpe-cmdq.md` §3.13).

### 1.4 Registration with mi_sys [R]
Each module registers its devices with `mi_sys_RegisterDev(info, usrData, procfs_ops, dir)`; the info record (0x298 bytes)
and the pass callback slots are `mi_sys-1-engine.md` §2.2–§2.3.
What the four modules fill in:

| field (info offset) | mi_sensor | mi_vif | mi_isp | mi_scl |
|---|---|---|---|---|
| module id (+0x00) | 0x1B | 6 | 0x21 | 0x22 |
| dev id (+0x04) | 0 | dev 0..15 (master devs only, §3.6) | 0 | 0..3 |
| input ports (+0x08) | 0 | 0 | 1 | 1 |
| output ports (+0x0c) | 1 | 2 | 3 | 6 |
| channels (+0x10) | 1 | 1 | 32 | 34 |
| passes (+0x14) | 1 | 1 | 1 | 1 |
| worker priority (+0x1c) | — | `g_threadPriority` (98) | `g_threadPriority` (98) | `g_threadPriority` (98) |
| concurrent-task limit (+0xc4 = slot 0x8c) | — | — | 0x80 | 0x88 with CMDQ on, else 1 |
| callbacks | none (capability slots NULL) | §3.5 | §4.4 | §6.4 |

Callback return codes: VIF output task 0 queued / 1 done / 2 retry / 3 drop; ISP and SCL input task 0 accepted / 2 retry /
3 drop; capability and bind callbacks return 0. `RejectCmdq` (ISP, SCL) writes the module's CMDQ buffer size into its third
argument and returns 0 when the device's CMDQ flag is 1, else returns 1.

### 1.5 Bind capability (what each module fills, slots 0x40 / 0x44) [R]
Only the type mask (+0x0c of the capability record) is written; bits FRAME_BASE 1, REALTIME 4, 8, HW_RING 0x10
(`mi_sys-1-engine.md` §3.1, §3.3):

| module | input side | output side |
|---|---|---|
| mi_vif | not set (no input ports) | 1; **5** when the own port is 0 and the peer module is ISP 0x21 or SCL 0x22 |
| mi_isp | 1; **5** when the peer is VIF (module 6) port 0 | **4** (REALTIME only) for port 0, **1** for ports 1/2 |
| mi_scl | by channel type: 0 or 2 → **4**, 1 or 3 → **5**, 4 → **1**, any other type → nothing written (bind refused) + log; dev > 3, device not created, chn > 0x21 or channel not created → nothing written, log, returns `0xA0222000` (ignored by mi_sys; the empty mask refuses the bind) | **0x1D** for every port |
| mi_sensor | capability slots NULL | capability slots NULL |

The meaning of SCL's output bit 8 is not known.

### 1.6 Ext-conf values the modules pass [R]
`mi_sys_Set{Input,Output}PortBufExtConf(h, chn, pass, port, &ext)`, `ext = {u16 strideAlign; u16 lumaLineAlign; u16
chromaLineAlign; u8 flag}` (`mi_sys-1-engine.md` §4.2). The constant word 0x00020010 = {16, 2} is what Ghidra renders as
"`{ptr, 1}`" / "`{"cCnt", 1}`" in the decompiles.

| caller | call | value |
|---|---|---|
| mi_vif `EnableOutputPort` | SetOutputPortBufExtConf | {16, 2, 1, 0} |
| mi_isp `CreateChannel` (input) / `EnableOutputPort` | SetInputPortBufExtConf / SetOutputPortBufExtConf | {16, 2, 1, 0} |
| mi_scl `EnableOutputPort` | SetOutputPortBufExtConf | an earlier reading: `{u16 align 0x10 (0x20 for pixel 0x11/0x12), u16 2, u8 1}`; `mi_sys-1-engine.md` §4.2 reads the two output sites as {align, lines, 1, 0} with per-port values, or {0x20 for pixel formats < 2 else 0x10, 2, n, flag} |
| mi_scl `CreateChannel` (input) | SetInputPortBufExtConf | {a, 2, c, 0} computed, or {16, 2, n, …} (`mi_sys-1-engine.md` §4.2) |
| mi_scl `OnPreProcessInputTask` | SetOutputPortBufExtConf | from the consumer's sideband (§6.6) |

### 1.7 Proc commands [R]
A command is `echo "<cmd> <a1> <a2> …" > /proc/mi_modules/mi_<m>/mi_<m><dev>` (`mi_sys-2-interfaces.md` §10.3), registered
with `mi_sys_RegistCommand(name, argc, fn, handle)`. The handler is `fn(handle, u32 devId, u8 argc, u8 **argv, void *usr)`;
`argv[0]` is the command word, numeric arguments are parsed with `CamOsStrtoul(argv[i], 0, 10)` (decimal) [R: 2018
`mi_sys_internal.h` and the decompiles]. Reading the file (`cat`) runs the module's procfs dump callbacks. Every proc
command of the four modules is a debug aid; nothing on the camera writes them [I].

## 2. mi_sensor.ko [R]

12.6 KB of code, 51 functions, no exports. A pass-through to `MHal_SNR_*` (`mhal-csi-snr.md` §3.3) plus a per-pad cache used
by the procfs views and a few getters. No threads, IRQs or buffers; never binds.

### 2.1 Load, open, close
| event | what happens |
|---|---|
| insmod | params `debug_*` only; procfs as every MI module; `MI_DEVICE_Register` → `/dev/mi_sensor`. No mhal call. |
| each `open` (ProcessInit) | `MI_SENSOR_IMPL_Init`: open counter (u8) ++; on the first: `mi_sys_RegisterDev(devinfo{module 0x1B, words 3–5 = 1}, NULL, procfs{DumpPadAttr, DumpPlaneAttr, -, -, Help}, dir)`, `mi_sys_RegistCommand("setmirrorflip", 3, …)`, `("setfps", 2, …)` |
| each `release` (ProcessExit) | counter −−; on reaching 0: for each pad 0..3 with `bEnable`: `MI_SENSOR_IMPL_Disable(pad)`; `mi_sys_UnRegisterDev`; zero the cache. **Closing the last fd switches the sensor off.** |
| rmmod | procfs removed, `MI_DEVICE_Unregister`. |

### 2.2 ioctls
`/dev/mi_sensor`, 17 entries, table order = nr (`.data` relocations); pad id = `MI_SNR_PADID`, 0..3. A pad > 3 is logged and
**replaced by 4** (not rejected), which mhal then fails → `0xA01B201F`. mhal failure → `0xA01B201F` (FAILED); NULL output
pointer → `0xA01B2006` (cannot happen through the ioctl path).

| nr | name (payload) | kernel action | app uses |
|---|---|---|---|
| 0 | Enable `{pad}` | `MHal_SNR_SensorEnable(pad)`; ok → `bEnable=1` | yes |
| 1 | Disable `{pad}` | if `bEnable`: `MHal_SNR_SensorDisable(pad)`, `bEnable=0`; not enabled → 0 (only a log) | yes |
| 2 | GetPadInfo `{pad; MI_SNR_PADInfo_t 0x34 out}` | `MHal_SNR_GetInfo(pad, &h)` (0x50 buffer); reorder (§2.3), cache at pad+0x178 | yes |
| 3 | GetPlaneInfo `{pad; plane; MI_SNR_PlaneInfo_t 0x48 out}` | pad<4 and plane<4 else FAILED; `MHal_SNR_GetPlaneInfo(pad, plane, &h)`; reorder, cache | yes |
| 4 | GetFps `{pad; u32 fps out}` | `MHal_SNR_GetFPS(pad, &fps)` (unclamped pad) | no |
| 5 | SetFps `{pad; u32 fps}` | `MHal_SNR_SetFPS(pad, fps)`; cache | no |
| 6/7 | Get/SetAnadecSrcAttr `{pad; plane; MI_SNR_Anadec_SrcAttr_t 0x14}` | `MHal_SNR_GetAnadecSrcAttr` / `MHal_SNR_SetAnadecSrcAttr(pad, plane, 0x18 buf)` — analog decoders, unused | no |
| 8 | QueryResCount `{pad; u32 cnt out}` | `MHal_SNR_QueryResCount(pad, &cnt)` | yes |
| 9 | GetRes `{pad; u8 idx; MI_SNR_Res_t 0x34 @+8 out}` | `MHal_SNR_GetRes(pad, idx, &h)` (0x40 buffer), reorder | yes |
| 0xa | GetCurRes `{pad; u8 idx out; Res @+8 out}` | `MHal_SNR_GetCurRes(pad, &idx, &h)` | no |
| 0xb | SetRes `{pad; u8 idx}` | `MHal_SNR_SetRes(pad, idx)`; cache idx | yes |
| 0xc | SetOrien `{pad; u8 mirror; u8 flip}` | `MHal_SNR_SetOrien(pad, mirror, flip)` | no |
| 0xd | GetOrien `{pad; u8 mirror out; u8 flip out}` | `MHal_SNR_GetOrien(pad, &m, &f)` | no |
| 0xe | SetPlaneMode `{pad; u8 bHdr}` | `MHal_SNR_SetPlaneMode(pad, bHdr)`; cache | yes (0) |
| 0xf | GetPlaneMode `{pad; u8 out}` | cache only (pad 4 → FAILED) | no |
| 0x10 | CustFunction `{pad; u32 cmd; u32 size; u32 uptr; u32 dir}` | `buf = CamOsMemAlloc(size)`; `copy_from_user(buf, uptr, size)`; `MHal_SNR_CustFunction(pad, cmd, buf)`; dir 1 → `copy_to_user(uptr, buf, size)`; dir ≥ 2 → returns ILLEGAL_PARAM **after** executing | no |

[S] (SigmaStar Tiramisu MI_SENSOR document `sensor_zh` §2.1, §2.6, §2.12, §3.4, §5.2): `MI_SNR_SetPlaneMode`
and `MI_SNR_SetRes` are to be called before `MI_SNR_Enable`; after Enable SetRes is not to be called alone — a resolution
change goes through the switch flow (Disable, SetRes, Enable); `MI_SNR_SetFps` is bounded by the max / min fps of the index
SetRes chose and takes whole fps or fps × 1000; in `MI_SNR_Res_t` `stOutputSize` is the sensor's raw size and `stCropRect` the
area it really outputs. The ioctls above check none of the order (SetRes and SetFps go to mhal whatever `bEnable` is) [R].

### 2.3 Field reordering between mhal and MI structs
- **PadInfo**: mhal `{+0 eIntfMode, +4 eHDRMode, +8 u32PlaneCount, +0xc union attr (0x24), +0x30 u8 bEarlyInit}` →
  MI `{u32PlaneCount, eIntfMode, eHDRMode, attr, bEarlyInit}`.
- **PlaneInfo**: mhal `{+0 capRect (4×u16), +8 mhal pixel fmt, +0xc ePixPrecision, +0x10 u32PlaneID, +0x14 eHdrSrc,
  +0x18 name[32], +0x38 shutterUs, +0x3c gainX1024, +0x40 compGain}` → MI `{PlaneID, name, capRect, eBayerId, ePrecision,
  eHdrSrc, shutter, gain, compGain, ePixel}`, with **`eBayerId = fmt − 0x2a`** (values > 0xB → 0xC = MAX) and
  `ePixel = fmt` unchanged. The same 0x2A base is used by mi_vif and mi_isp for the Bayer formats (§3.4).
- **Res**: mhal (0x40) `{+0 cropW, +4 cropH, +8 maxFps, +0xc minFps, +0x10 cropX, +0x14 cropY, +0x18 outW, +0x1c outH,
  +0x20 desc[32]}` (32-bit slots; MI takes the low 16 bits of the geometry) → `MI_SNR_Res_t` (packed, 0x34).

### 2.4 State
4 pads × 0x1EC bytes: `+0 bEnable`, `+4 fps`, `+8` 4 planes × 0x5C (PlaneInfo 0x48 + Anadec attr 0x14), `+0x178` PadInfo
(intf word +0x17c), `+0x1ac` res count, `+0x1b0` current Res, `+0x1e4` current idx, `+0x1e5` set idx, `+0x1e6/+0x1e7`
mirror/flip, `+0x1e8` plane mode. Read back by GetPlaneMode, the procfs views and the two proc commands; `bEnable` also by
Disable and the last-close cleanup.

### 2.5 Proc commands and views
Registered on the first `open` under mi_sys device (module 0x1B, dev 0); commands are written to
`/proc/mi_modules/mi_sensor/mi_sensor0`:

| command | args | checks and action | result |
|---|---|---|---|
| `setmirrorflip` | pad, bMirror, bFlip | pad ≥ 4 → "not support Padid", `0xA01B2008`; pad not enabled (state byte +0) → "not enable", `0xA01B2001`; else `MHal_SNR_SetOrien(pad, m, f)` (failure only logged), then `MHal_SNR_GetOrien(pad, &m, &f)`, read-back stored in the cache (+0x1e6/+0x1e7) | 0 even if the set failed |
| `setfps` | pad, fps | same pad checks; `MI_SENSOR_IMPL_SetFps(pad, fps)` (= ioctl 5: `MHal_SNR_SetFPS` + cache) | failure → "setfps %d fail", `0xA01B201F` |

`help` prints `setmirrorflip [PadId, bMirror, bFlip];` and `setfps [Padid, fps];`.

Views (`cat /proc/mi_modules/mi_sensor/mi_sensor0`): both iterate the 4 pads and print only pads with `bEnable` = 1; live
values are re-read from mhal through the IMPL getters, the per-interface attribute block from the cached PadInfo.
- **DumpPadAttr**: header `PadId PlaneMode bEnable bmirror bflip fps ResCnt intfmode hdrmode planecnt`; interface names
  `BT656, DIG_CAM, BT1120_ST, BT1120_INTE, MIPI, MAX, ERROR` (PadInfo intf word, cache +0x17c). Then the attribute block:
  - MIPI (intf 4): `HdrHwMode DataFmt HdrVchNum HsyncMode LaneNum LPackType0 LPackType1 samDelay` from cache +0x194, +0x188,
    +0x198, +0x18c, +0x184, +0x19c, +0x1a0, +0x190.
  - BT656 (intf 0): `BitSwap ClkEdge Multinum … PcPol … HDelay VDelay PcDelay` from +0x1a4, +0x1a0, +0x184, +0x18c, +0x190,
    +0x188, +0x198, +0x194, +0x19c.
  - parallel / other: `ParallelAttr … PcPol … HDelay VDelay PcDelay` from +0x188, +0x18c, +0x184, +0x194, +0x190, +0x198.
  Then every resolution (`GetRes`, columns `strResDesc CropX CropY CropW CropH OutW OutH MaxFps MinFps`) and the current one
  (`GetCurRes`).
- **DumpPlaneAttr**: for each enabled pad, planes **0..2** (the loop stops at 3): `GetPlaneInfo`, columns `Padid Planeid
  SnrName BayerId ePixPrec eHdrSrc CropX CropY CropW CropH`; Bayer-id names `RG GR BG GB R0 G0 B0 G1 G2 I0 G3 I1 MAX`,
  precision `8BPP 10BPP 12BPP 14BPP 16BPP MAX`.

### 2.6 Imports
mhal: `MHal_SNR_SensorEnable`, `SensorDisable`, `GetInfo`, `GetPlaneInfo`, `GetFPS`, `SetFPS`, `QueryResCount`, `GetRes`,
`GetCurRes`, `SetRes`, `SetOrien`, `GetOrien`, `SetPlaneMode`, `GetAnadecSrcAttr`, `SetAnadecSrcAttr`, `CustFunction`.
mi_sys: `mi_sys_RegisterDev`, `UnRegisterDev`, `RegistCommand`, `RecodeTimeStamp`, log and procfs ops. mi_common:
`MI_DEVICE_Register`, `MI_DEVICE_Unregister`, `MI_DEVICE_GetProcModuleDir`, `MI_COMMON_Print`, `MI_COMMON_RealGetSelfDir`.
CamOs: `CamOsMemAlloc`, `CamOsMemRelease`, `CamOsStrtoul`.

### 2.7 libmi_sensor.so [R]
<!-- ev:mi-capture.2.7 -->
19 exports; payloads follow the packing of `mi-user-abi.md` §3.2, no argument checks in user space, Get variants copy back only on
0 unless stated (per-function check:, raw field maps).
The 8 calls of the app (Enable, Disable, GetPadInfo, GetPlaneInfo, QueryResCount, GetRes, SetRes, SetPlaneMode) are rows of
the table of `mi-user-abi.md` §10; the other 11:

| function | nr (size) | payload / copy-back |
|---|---|---|
| `MI_SNR_GetFps(pad, u32 *fps)` / `SetFps(pad, fps)` | 4 / 5 (8) | {pad, fps}; Get: `*fps = +4` on 0 (the current value is sent in too) |
| `MI_SNR_GetAnadecSrcAttr` / `MI_SNR_SetAnadecSrcAttr(pad, plane, MI_SNR_Anadec_SrcAttr_t *a)` | 6 / 7 (0x1c) | {pad, plane, a (0x14)}; Get sends `a` in and copies 0x14 back on 0 |
| `MI_SNR_GetCurRes(pad, u8 *idx, MI_SNR_Res_t *r)` | 0xa (0x3c) | {pad}; on 0: `*r = +8 (0x34)`, `*idx = +4` |
| `MI_SNR_SetOrien(pad, u8 mirror, u8 flip)` / `GetOrien(pad, u8 *m, u8 *f)` | 0xc / 0xd (8) | {pad, +4 mirror, +5 flip, +6 = 0}; Get copies +4/+5 on 0 |
| `MI_SNR_GetPlaneMode(pad, bool *hdr)` | 0xf (8) | {pad, +4 *hdr}; `*hdr = +4` on 0 |
| `MI_SNR_CustFunction(pad, u32 cmd, u32 size, void *data, u32 dir)` | 0x10 (0x14) | {pad, cmd, size, data **pointer**, dir} — the kernel copies `size` bytes at `data` in, and back when dir = 1 |
| `MI_SNR_InitDev(void)` / `DeInitDev(void)` | — | open / close the fd (`mi-user-abi.md` §2) |

## 3. mi_vif.ko [R]

46 KB of code, 97 functions, 16 exports. The video-input block: up to 4 groups (receivers: MIPI, parallel, BT656, BT1120),
4 devices per group (16 devs, `dev = group·4 + i`), 2 output ports per dev, and 8 hardware pipes that devs are mapped onto.
Port 0 carries the full dev picture; port 1 is a crop path. On this camera only dev 0 port 0, bound REALTIME to ISP, is used;
VIF then writes nothing to DRAM. The DRAM frame path (`MHal_VIF_QueueFrameBuffer` / `DequeueFrameBuffer`) runs only for
FRAME_BASE binds and for port 1.

### 3.1 Load, open, close
| event | what happens |
|---|---|
| insmod | params (below); procfs; `MI_VIF_IMPL_Insmod` (fills a 0x40-byte table with 0xFF); `MI_DEVICE_Register` → `/dev/mi_vif` |
| first `open` (`MI_VIF_IMPL_Init`, u8 open counter) | clear module state, `CamOsMutexInit`; chipIdx and revision (§1.3); mark the 8 pipes free; **`MHal_VIF_Init()`** (failure = BUG); **`MHal_VIF_RegisterGetPtsCallback(mi_sys_GetCurPts)`** |
| last `release` (`MI_VIF_IMPL_DeInit`) | for every created group / enabled dev: disable the dev, `mi_sys_UnBindChnPort` + `DisableOutputPort` for each enabled port, `DestroyDevGroup`; **`MHal_VIF_Deinit()`** |

Parameters (defaults read from `__param`): `g_u32DevNeedUsePipeId[16]` (default {255, 0…}: preferred HW pipe per dev, ≥ 8 =
auto), `g_u32DevDebugLevel[16]` (0; per-dev trace bits; a separate object, `_LANCHOR1` = `.bss + 0x218`), `g_buseReset` (1),
`g_bFBCMode` (1), `g_bSupportIrq` (1), `g_threadPriority` (98, info +0x1c).

### 3.2 Module state `g_stVifModRes` (`.bss + 0x258`, 20 128 bytes)
<!-- ev:mi-capture.3.2 -->
- +0x00 u8 open counter; +0x4E90 u32 chipIdx, +0x4E94 u32 chip revision, +0x4E98 u8 port-1 errata flag (§3.11), +0x4E9A u16
  forced FIFO-full count, +0x4E9C u32 its dev id.
- **Group** *g* (0..3) at `g·0x12D0`: +0x08 u8 id, +0x09 u8 created, +0x0A u8 1, +0x12B0 stored `MI_VIF_GroupAttr_t` (intf
  +0x12B0, work mode +0x12B4, **HDR type +0x12B8**, clkEdge +0x12BC, mclk +0x12C0, scan +0x12C4, **stitch mask +0x12C8**),
  **+0x12D0 u32 number of groups in the stitch mask** (= sub-plane count of a buffer, §3.6).
- **Device** block at `g·0x12D0 + i·0x4A8` (0x4A8 bytes). The pointer passed to mi_sys as `usrData` and to the mhal event
  callbacks is **D = block + 0x10**; offsets below are relative to D. Code that addresses the block itself indexes from
  the block instead (block offset = D offset + 0x10: enabled +0x74, mi_sys handle +0x78, port-0 enabled +0xb9, lock-owner
  trace +0x3d0..+0x41c, pipe +0x460, preferred pipe +0x464, masked flag +0x465):
  - D+0x00 / +0x20 / +0x40 callback lists {next, prev} for frame start / frame end / line hit; D+0x08 / +0x28 / +0x48 their
    spinlocks.
  - D+0x60 dev id; D+0x64 u8 enabled; D+0x68 mi_sys handle; D+0x6C `MI_VIF_DevAttr_t` (pixel +0x6C, rect +0x70, field +0x78,
    H2T1P +0x7C).
  - port *p* (0..1), offsets + 0x180·p: +0xA9 u8 enabled, +0xAC applied `MI_VIF_OutputPortAttr_t`, +0xC0 staged attr (cap rect
    +0xBC..+0xC2 u16, dest +0xC4 / +0xC6, pixel +0xC8, rate +0xCC — offsets of the applied copy), +0xD8 FBC flag, +0x110
    atomic in-flight count, +0x114 u16 queue depth ("init atom", default 1), +0x118 peer `MI_SYS_ChnPort_t`, +0x128 bound peer
    module id, +0x138 bind type, +0x144 u8 first enqueue done, +0x148 extra line count, +0x150 frames ready, +0x158 / +0x160
    fps window (jiffies / u64 ns), +0x168 / +0x169 measured fps (whole, hundredths), +0x170 delivered count, +0x174 dump
    count, +0x178 dump path, +0x1F8 u8 erase flag, +0x1F9 / +0x1FA fps threshold (whole, hundredths), +0x200 / +0x208 / +0x210
    PTS window (last / max / min u64), +0x218 u64 "addr0" counter, +0x228 u64 queue-full counter.
  - D+0x3A8 rwsem; D+0x3C0..+0x40C lock-owner trace fields (debug); D+0x450 u32 pipe (0xFF = none); D+0x454 u8 preferred
    pipe; D+0x455 u8 input masked by mi_vif ("needs unmask"); D+0x45C timer object; D+0x474 u8 keep timer running; D+0x478
    frame-start counter; D+0x47C task sequence; D+0x480..+0x4A0 callback counters; D+0x494 last polled frame; D+0x498 master dev
    id (stitched devs); D+0x49C no-buffer streak; D+0x4A4 u8 pipe class (0).
- **Pipes** (8): slot at +0x4B48 + 100·p: +0x00 u8 owner (0xFF = free; set to the bind-param low byte when a dev claims it),
  +0x01 u8 claimed [R], +0x04 the 8-byte mhal pipe config (its first word
  non-zero also means "already configured": the enqueue skips re-configuration), +0x10 + 0x28·port the 0x28-byte mhal port
  config (§3.8). `MHal_VIF_PipeSetConfig(dev, pipe, slot+0x04)` and `MHal_VIF_PortSetConfig(pipe, port, slot+0x10+0x28·port)`
  point here (the decompile's inflated constants 0x252CC / 0x252D8 for pipe 0).

Concurrency: the module mutex (ioctls), the per-dev rwsem (ioctls write, task callbacks read), a CamOs mutex for pipe
allocation, spinlocks for the callback lists.

### 3.3 ioctls
`/dev/mi_vif`, 16 entries, nr order = table order; payloads from `mi-user-abi.md` §3.2 and the M6 header
`disp/skyipc_tiramisu_include_mi_vif_datatype.h` (names below). Errors `0xA006xxxx`; VIF-specific `0x80` INVALID_PORTID, `0x81`
DEVNOTENABLE, `0x82` DEVNOTDISABLE, `0x84` PORTNOTDISABLE (= `E_MI_ERR_MAX+1…`).

| nr | name (payload) | kernel action | app |
|---|---|---|---|
| 0 | CreateDevGroup `{group; MI_VIF_GroupAttr_t 0x1c}` | §3.5 | yes |
| 1 | CreateDevGroupExt `{group; GroupAttr; MI_VIF_GroupExtAttr_t 0x14}` | §3.7 | no |
| 2 | DestroyDevGroup `{group}` | refuses while a dev of the group is enabled; for each group in the stitch mask `MHal_VIF_GroupReset(g, 0)`, `MHal_VIF_GroupDisable(g)`; release pipes; `mi_sys_UnRegisterDev` ×4 | at exit |
| 3 | GetDevGroupAttr `{group; GroupAttr out}` | stored copy | no |
| 4 | SetDevAttr `{dev; MI_VIF_DevAttr_t 0x14}` | §3.5 | yes |
| 5 | GetDevAttr | stored copy | no |
| 6 | EnableDev `{dev}` | §3.5 | yes |
| 7 | DisableDev `{dev}` | §3.14 | at exit |
| 8 | GetDevStatus `{dev; MI_VIF_DevStatus_t 0x14 out}` | group id / created / enabled + `MHal_VIF_GetVIFDev2SensorPadMux` (§3.7) | no |
| 9 | SetOutputPortAttr `{dev; port; MI_VIF_OutputPortAttr_t 0x14}` | §3.5 | yes |
| 0xa | GetOutputPortAttr | stored copy | no |
| 0xb | EnableOutputPort `{dev; port}` | §3.5 | yes |
| 0xc | DisableOutputPort `{dev; port}` | §3.14 | at exit |
| 0xd | Query `{dev; port; MI_VIF_DevPortStat_t 0x1c out}` | `bEnable`, `u32IntCnt` = frame-start count, `u32FrameRate` = measured fps byte, `u32PicWidth/Height` = port dest size; `u32LostInt` / `u32VbFail` from `MHal_VIF_PortStatQuery(pipe, port, buf)` (0 if no pipe) | yes (status print) |
| 0xe/0xf | CallBackTaskRegister / UnRegister `{dev; MI_VIF_CallBackParam_t *}` | §3.12 | no |

The 16 exported `MI_VIF_*` functions are the same IMPL calls for in-kernel users; no module on the camera imports them
(`nm -u` over all `.ko`).

### 3.4 Pixel format → mhal format and precision
Accepted MI pixel formats: YUV422 0, 0xE..0x10, Bayer 0x14..0x4F. `vif_pixfmt_to_mhal` (0x10b30) maps them to mhal `(fmt, precision)`: YUV
kept as is with precision 4; Bayer `p = pix − 0x14` → **fmt = 0x2A + p % 12, precision = p / 12** (the 0x2A base mi_sensor
subtracts, §2.3; mi_isp uses the same rule, §4.7). mhal codes: `mhal-vif.md` §9.3.

### 3.5 Bring-up (the app's calls)
1. **CreateDevGroup(g, attr)**: g < 4 (else ILLEGAL_PARAM); attr checks intf ≤ 4, work mode ≤ 2, clk edge ≤ 2, HDR ≤ 4 (else
   returns 0x1F — not an MI code); `u32GroupStitchMask == 0` → `1<<g`. Already created → 0. For each of the group's 4 devs:
   reset its state, preferred pipe = `g_u32DevNeedUsePipeId[dev]` (≥ 8 → none), and register the dev with mi_sys:
   `mi_sys_RegisterDev(devinfo{module 6, devId = dev, +0xc 2, +0x10 1, +0x14 1, +0x1c g_threadPriority, callbacks
   EnqueueOutputTask, BarrierOutputTask, CheckOutputTaskStatus, DequeueOutputTask, OnPollingAsyncOutputTaskConfig,
   OnGetOutputPortBindCapability, OnBindInputPort, OnUnBindInputPort, OnBindOutputPort, OnUnBindOutputPort}, D, procfs, dir)`
   + the 11 proc commands (§3.13). Then for each group in the stitch mask: **`MHal_VIF_GroupSetConfig(g, &{intf, work, hdr,
   clkEdge, 0×6, mclk, scanMode}, &ops{alloc, free, map, unmap, flush})`** — `ops` = wrappers of
   `mi_sys_PrivateDevChnHeapAlloc` / `mi_sys_PrivateDevChnHeapFree`, `mi_sys_Vmap` / `mi_sys_UnVmap` / `mi_sys_VFlushInvCache`
   that mhal uses for its own buffers; the 0x30-byte group config and which fields mhal reads (intf, work mode, HDR mode, clock
   edge only): `mhal-vif.md` §9.2 — and **`MHal_VIF_GroupEnable(g)`**. Store attr, mark created.
2. **SetDevAttr(dev, attr)**: dev < 16; group created and dev not enabled (else DEVNOTDISABLE `…82`); field ≤ 3, H2T1P bool,
   pixel as §3.4; rect w/h ≠ 0; H2T1P only with YUV. For every dev of the stitch mask: **`MHal_VIF_DevSetConfig(dev, &{u16 x,
   u16 y, u16 w, u16 h (h/2 if field == 3), fmt, precision, interlaced = field ∈ {1,2}})`** (layout `mhal-vif.md` §9.3).
3. **EnableDev(dev)**: for each dev in the stitch mask with a non-zero rect: **`MHal_VIF_DevEnable(dev)`**; register the
   mhal event callbacks (§3.10); mark enabled. Then `mi_sys_EnableChannel(hDev, 0)` (failure = BUG).
4. **SetOutputPortAttr(dev, port, attr)**: port < 2; dev enabled (else `…81`), port not enabled (else `…84`); frame-rate enum
   ≤ 4; pixel as §3.4; **port 0 must be the full dev rect, unscaled** (cap = dev rect, dest = cap); port 1 = crop inside the
   dev rect with dest ≤ cap, Bayer only. Stored for all devs of the stitch mask; not sent to mhal yet.
5. **EnableOutputPort(dev, port)**: dev enabled, dest size set; port 1: the dev's input field must not be scan mode 3 (else
   `0xA0062080` INVALID_PORTID) and the errata check of §3.11; `mi_sys_EnableOutputPort(hDev, 0, 0, port)` (failure = BUG),
   `mi_sys_SetOutputPortBufExtConf(hDev, 0, 0, port, &{16, 2, 1, 0})`, reset the port's in-flight counters, mark enabled; if
   the dev is running, wake the mi_sys worker (`mi_sys_NotifyWakeupISREvent`).
6. **Bind VIF(0,0,0) → ISP(0,0) REALTIME** (an mi_sys call): mi_sys asks `OnGetOutputPortBindCapability` (§1.5), then
   `OnBindOutputPort(cur, peer, bindParam)`: for port 0 + REALTIME the preferred pipe becomes 0 when the peer is ISP, 2 when
   SCL; peer and bind param are stored. `OnBindInputPort` / `OnUnBindInputPort` are empty (VIF has no input).

### 3.6 Stitch groups and buffer shape
`CreateDevGroup(g, attr)` with `u32GroupStitchMask` ≠ 0 (default `1<<g`) makes *g* the **master**. For every group *u* in the
mask: `MHal_VIF_GroupSetConfig(u, &cfg, &ops)` + `MHal_VIF_GroupEnable(u)`; the dev slots of *u* get pipe 0xFF and the
preferred pipe from `g_u32DevNeedUsePipeId[g·4 + i]` (indexed by the **master** dev, not *u*); the master's plane count
(+0x12D0) is incremented per stitched group. Only the master's four devices are registered with mi_sys; stitched groups have no
mi_sys presence. Attr and the created flag are stored only for the master. Every later per-dev call fans out over the mask:
SetDevAttr → `MHal_VIF_DevSetConfig(i + 4u)` for each *u*; EnableDev → `DevEnable` + the four event callbacks per *u*, each
stitched dev block marked enabled and stamped with the master dev id at D+0x498; Enqueue and Check iterate every *u*.

**Buffer shape** requested in `OnPollingAsyncOutputTaskConfig` (the `MI_SYS_BufConf_t` written at meta +0x18 + 0x78·port,
`mi_sys-1-engine.md` §4.2):
- plane count 1 → `type 1 FRAME {width = port dest w, height = port dest h + extra lines, format = port pixel fmt, compress =
  the port FBC flag}`.
- plane count *n* > 1 → `type 3 MULTIPLANE {u8 subPlaneNum = n, width, height, format, compress}`, one sub-plane per stitched
  group in mask order.
- **HDR height**: when the group HDR type (+0x12B8) ≠ 0 **and** a per-port flag (word 0x23 of the port record) is 1 [I: the FRAME_BASE /
  line-interleave case], the requested height is `(dest h + extra)·2` — both exposures stacked in one buffer, the second half at
  `phy + stride·h/2`; otherwise dest h + extra. "Extra lines" is the port's +0x148 value.
- The port FBC flag (+0xD8) is 0 for a REALTIME bind; otherwise `g_bFBCMode` (default 1), gated on the mhal precision being 10-
  or 12-bit Bayer. Compress 4 means "pack to 8 bit" (the Bayer-with-compress-4 size rule of `mi_sys-1-engine.md` §4.2).

### 3.7 `CreateDevGroupExt` and the sensor pad mux
Ioctl 1, not called by prog_uvc: group < 4 else `0xA0062003`; copies the 0x14-byte `MI_VIF_GroupExtAttr_t` onto the stack
`{eSensorPadID u32, u32PlaneID[4]}` and calls **`MHal_VIF_SetVIFDev2SensorPadMux(group, &ext)`** (the VIF → sensor routing
registers, `mhal-vif.md` §9.1), then runs the ordinary `CreateDevGroup(group, attr)`. The ext attr is not stored in
`g_stVifModRes`. `GetDevStatus` (ioctl 8) reads it back from mhal with `MHal_VIF_GetVIFDev2SensorPadMux`, filling
`MI_VIF_DevStatus_t` {GroupId, bGroupCreated, bDevEn, eSensorPadID, u32PlaneID}. `MI_VIF_GroupExtAttr_t` (M6 header):
`eSensorPadID` 0..3 or 0xFF, `u32PlaneID[MI_VIF_MAX_GROUP_DEV_CNT=4]` (HDR: 0 = short, 1 = long exposure). Without the ext call
the default pad mux is the one mhal sets inside the ordinary group set-up [I].

### 3.8 Runtime: the output-task callbacks
<!-- ev:mi-capture.3.8 -->
VIF has no input tasks; mi_sys's ASYNC thread polls it for output tasks (`mi_sys-1-engine.md` §6.5) and the mi_sys worker is
woken by the VIF interrupts (§3.10). REALTIME ports queue no buffer: one mi_sys task per sensor frame carries only PTS and fps.
A DRAM frame is a buffer of type 3, or type 1 with phys layout 2. `MI_SYS_BufInfo_t` as VIF reads it (M6 `mi_sys_datatype.h`):
+0x00 u64 PTS, +0x08 u64 sideband, +0x10 type, +0x20 frame union — pixel +0x24, compress +0x28, scan +0x2C, field +0x30,
**phys layout +0x34 (0 REALTIME, 1 RINGBUF, 2 NORMAL)**, w +0x38 u16, h +0x3A u16, pVirAddr[3] +0x3C, **phyAddr[3] u64
+0x48**, stride[3] u32 +0x60, u32BufSize +0x6C, content crop +0x7C. Multi-plane: u8 count +0x20, sub-plane *k* at +0x28 + 0x38·k
(pixel +0, u64 frameId +8, w +0x10, h +0x12, phy[2] u64 +0x20, stride[2] u16 +0x30, size +0x34).

1. **OnPollingAsyncOutputTaskConfig(meta, D)** (once per round; meta word 0 must be 0 and the dev enabled, else return 0x1C
   "nothing wanted"): per port — staged attr → applied; port disabled → skip; in-flight (+0x110) ≥ the queue depth (+0x114) →
   skip; the frame-start counter ≤ last polled (D+0x494) while the port's first-enqueue byte (+0x144) is set → skip (at most one
   request per sensor frame); else meta +4+port = 1 and the buf-conf of §3.6 (buffer type, width, height, pixel, a mode word 4
   / 0 = compress). Returns 0 if any port asked; after 100 empty rounds it logs once and returns 0x1C.
2. **EnqueueOutputTask(task)** (`_MI_VIF_IMPL_EnqueueOutputTaskDev`): result 2 (retry) while the dev is still input-masked by
   mi_vif and port 0's buffer is REALTIME whose **output-port sideband message** (`mi_sys_GetOutputPortSidebandMsg(h, chn, 0, 0,
   &u64)`) is not yet `top-3-bits == 4 && bit 0 == 1` — the ISP posts `0x80000000_00000001` on its input port when it enqueues
   its first realtime task (§4.7), 0 at start / stop (sideband type 4: `mi_sys-1-engine.md` §5.1; meaning [I], bit pattern [R]).
   Then, per group *u* of the stitch mask: allocate a pipe if the dev has none (§3.9) and call **`MHal_VIF_PipeSetConfig(dev,
   pipe, cfg)`**; for each enabled port not yet configured: mask the input (`MHal_VIF_GetInputMask` / `MHal_VIF_InputMask(dev,
   1)`), fill the mhal port config (below), call **`MHal_VIF_PortSetConfig(pipe, port, &cfg)`**, set D+0x455. FRAME_BASE ports
   with a DRAM buffer also call `MHal_VIF_QueueFrameBuffer(dev, port, &q)`, `q = {u64 phy[2], u64 stride-ish, w, h, …}` from the
   `MI_SYS_BufInfo_t` (for the HDR split the second plane is `phy + size`); when the port erase flag (+0x1F8) is set the buffer is
   zero-filled first (`mi_sys_Vmap` / `memset` / `mi_sys_UnVmap`). After all ports of all devs are configured the input is
   unmasked (`vif_unmask_input` (0x11810), §3.11). Returns 0 (queued), or 2 / 3 on the paths above; the no-buffer streak (D+0x49C) throttles
   repeated logs.
3. **CheckOutputTaskStatus** (`_MI_VIF_IMPL_CheckOutputTaskStatusDev`): REALTIME — the task is done (1) once the frame-start
   counter has passed the task's frame number. FRAME_BASE — per port with a DRAM buffer: `MHal_VIF_QueryFrames(dev, port, &n)`;
   when frames are ready it stores the count (+0x150), `MHal_VIF_DequeueFrameBuffer(dev, port, &deq)`, copies the dequeued PTS
   and addresses into the buffer info, runs the fps window, enforces the PTS window (+0x200..+0x214: a frame outside [min, max]
   is dropped), and when every expected port delivered records the first-frame timestamp and runs the optional dump (§3.13).
   Done / drop counts go to +0x170 / D+0x4A0. FIFO-full recovery, when the module counter (+0x4E9A) is non-zero: mask all enabled
   devs, **`MHal_VIF_DevReset(dev, 2)`**, reconfigure, unmask.
4. **BarrierOutputTask** → 0 if the dev is enabled, else `0xA0062006` / `0xA0062008`. **DequeueOutputTask** decrements the
   in-flight counters and returns 1 (there was a buffer) or 3 (none).

**mhal pipe and port configs as mi_vif fills them** (layouts and mhal's use: `mhal-vif.md` §9.4, §9.5). Pipe config (8 bytes): +0 enable, +4 HDN_EN = the dev attr H2T1P flag. Port config (0x28
bytes): +0x00 enable (1), +0x04 FBC flag, +0x08 / +0x0A / +0x0C / +0x0E u16 crop x / y / w / h (h halved for a field-3 input),
+0x10 output height (halved for field 3), +0x14 output width, +0x18 work mode = bind type ≠ 4 (0 realtime, 1 frame), +0x1C
mhal pixel format, +0x20 frame-rate code, +0x24 both-field flag (field == 3). When the port is already configured and the work
mode at +0x18 differs, mi_vif only logs. An earlier reading gave another layout (u8 FBC at +0x01, crop at +0x04, out h
+0x0C, out w +0x0E, work mode +0x10, pixel +0x14, rate +0x18, both-field +0x1C); the stores in the code match the one above.

[S] (SigmaStar Tiramisu MI_VIF document `vif_zh` §1.2.1, §2.10): «Tiramisu系列芯片：不支持eFrameRate 设置» — the
port's frame-rate field has no effect on Tiramisu chips; frame-rate reduction is done by the bind (`MI_SYS_BindChnPort2`
rates). [R] mi_vif.ko contains `_MI_VIF_IMPL_AdjustFuncRewindStatus` with the warning `rewind over cnt %d`; the function is not
read. [S] (SigmaStar FAE, 2022-08): the line `_MI_VIF_IMPL_AdjustFuncRewindStatus: rewind over cnt 100 … force
sleep` means VIF's downstream is too slow and VIF has no buffer to put the received data in.

### 3.9 HW pipe allocation
A dev needs one of the 8 HW pipes before its ports can be programmed; the choice is made in EnqueueOutputTask when D+0x450 is
0xFF:
- **Preferred pipe** (D+0x454, set in CreateDevGroup from `g_u32DevNeedUsePipeId[dev]`, default 255 = auto, overridden by
  `OnBindOutputPort`: port 0 REALTIME to ISP → 0, to SCL → 2): if 0..7, that pipe is taken directly unless its slot is already
  claimed (+0x01 = 1 → log, pipe stays 0xFF). On this camera dev 0 is bound REALTIME to ISP, so **pipe 0** is used and no search
  runs.
- **Auto-search** (preferred = 0xFF): under a module-wide mutex the 8 pipe slots are scanned and a free, stitch-compatible pipe
  is claimed; "free" = the slot's pipe-config first word (slot+0x04, i.e. +0x4B4C + 100·p) is 0, "compatible" = the owner byte
  matches the dev or is 0xFF. the pipe with the **lowest priority value** wins; the ranks are **8, 7, 6 for pipes 0–2 and 0 for pipes 3–7**; together: pipes 3–7 are taken
  before 2, 1, 0 [I]. chipIdx does not enter the search (no read of +0x4E90 in `_MI_VIF_IMPL_EnqueueOutputTaskDev`).
- With the group in HDR mode (+0x12B8 ≠ 0) the following pipe slot is claimed as well: its owner byte (slot+0x64), claimed flag
  (+0x65), pipe-config word (+0x68) and port-0 config word (+0x74) are set, the last three to 1 [R]; the second pipe carries the second exposure [I].
- Once chosen, the slot is marked, D+0x450 is set and `MHal_VIF_PipeSetConfig(dev, pipe, &cfg)` runs; on failure the slot and
  D+0x450 are released back to 0xFF. mhal accepts realtime only on pipes 0–2, port 0 (`mhal-vif.md` §9.5).

### 3.10 mhal interrupt callbacks
Registered in EnableDev: `MHal_VIF_RegisterEventCallback(dev, type, fn, 0, D)` for type 0 frame start, 1 frame end, 2 line
count, 4 FIFO full; mhal delivers the line-count event with type 3 (`DrvVif_DoVifLineCntCb`) although it was registered as
type 2 (`mhal-vif.md` §9.6).
Each `fn` takes D as its last argument and acts only while the dev is enabled (D+0x64):
- **FrameStart** → `vif_on_frame_start` (0x13238): frame-start counter D+0x478 ++, `mi_sys_NotifyWakeupISREvent(mi_sys handle of the master
  dev, 0)`, then the frame-start callback list (`vif_call_fs_list(list head)` (0x11d84) calls each `{pfn, u64Data}` under the spinlock). A trace
  byte in `g_u32DevDebugLevel[dev]` gates timing prints.
- **FrameEnd** → wake the master dev's mi_sys worker, run the frame-end list. mhal calls it only in frame mode (WDMA done),
  never in realtime mode (`mhal-vif.md` §11.1).
- **LineCount** → wake, run the line-hit list. mhal raises it on line count 1, which stays masked on this camera
  (`mhal-vif.md` §11.1).
- **FifoFull** → when the port's bind type (+0x138) is not 4 (not REALTIME) and the static flag `.LANCHOR2` (`.data + 0x9c`, a
  build constant, 0 in this build) is set: module FIFO-full counter (+0x4E9A) ++ and the dev recorded (+0x4E9C); then wake the
  worker. The recovery runs in CheckOutputTaskStatus (§3.8).

### 3.11 Input unmask and the chipIdx-5 revision-0 port-1 errata
`MI_VIF_IMPL_EnableOutputPort(dev, 1)` sets the module errata flag `g_stVifModRes[0x4E98] = 1` when **chipIdx == 5 and chip
revision == 0**; DisableDev clears it. The flag changes only the input-unmask step `vif_unmask_input(dev)`:
- flag 0: unmask just this dev — `MHal_VIF_GetInputMask(dev)`, and if masked `MHal_VIF_InputMask(dev, 0)`; clear D+0x455.
- flag 1: the unmask is **deferred until every enabled port of every enabled dev in the module has been configured** —
  `vif_unmask_input` scans all 16 dev slots, both ports each, and only when no enabled-but-unconfigured port remains does it unmask
  the inputs of all enabled devs at once. This serialises the VIF inputs that revision-0 silicon cannot bring up independently
  [I].
This camera is chipIdx 5; whether the flag is ever set depends on `CamOsChipRevision()` (§1.3) and on port 1 being enabled,
which the app does not do.

### 3.12 Callback-task ioctls 0xe / 0xf
`CallBackTaskRegister` / `UnRegister {dev; MI_VIF_CallBackParam_t *}` — an in-kernel hook; libmi_vif never sends these ioctls
(§3.16). The handler **dereferences the user-supplied pointer (word 1 of the argument) from kernel context** and copies the 0x14-byte
`MI_VIF_CallBackParam_t {eCallBackMode, eIrqType, MI_VIF_CALLBK_FUNC pfn, u64 u64Data}` onto the stack. Register:
`CamOsMemAlloc(0x1c)`, store `{pfn, u64Data}`, pick the dev's list by `eIrqType` (0 frame start D+0x00, 1 frame end D+0x20, 2
line hit D+0x40), initialise the list head on first use, splice the node in under the list spinlock. Unregister: walk the chosen
list, match by the stored function pointer (word 2 of the argument), unlink and free. The lists run from the mhal interrupt callbacks
(§3.10), so a registered `pfn` is called in interrupt context.

### 3.13 Frame dump and proc commands
**Frame dump** (`vif_frame_dump` (0x1348c), armed by the `dump` command): for each port with a buffer, in HDR mode (+0x12B8 ≠ 0) two files
`…_Hdr1…raw` / `…_Hdr2…raw` (the two exposure halves), else one; the name encodes `mi_vifDev%dPort%d_%dx%d_Pixel%d_stride%d
[_count]`; `vif_dump_write_file` (0x13358) does `filp_open(O_CREAT|O_RDWR, 0777)` / `vfs_write` / `filp_close` with the kernel address space
switched (`DataSynchronizationBarrier` around a `KERNEL_DS` set).

The 11 proc commands of `mi_vif<dev>`, registered per master dev in CreateDevGroup with the dev's mi_sys handle (`help` prints
all eleven with their argument lists):

| command | args | effect |
|---|---|---|
| `dump` | dev, port, [path], [count] | store the path at port +0x178 and the count at +0x174 (default 1); the next CheckOutputTaskStatus writes the frames |
| `initatom` | dev, port, n | the port's queue depth u16 (+0x114) — how many buffers may be in flight before OnPollingAsync stops asking |
| `usetimer` | dev, bUse | bUse = 1: `CamOsTimerInit` + `CamOsTimerAdd(10 ms, D+0x45C, TimerFunc)` — a software frame pulse that calls `mi_sys_NotifyWakeupISREvent` every 10 ms and stops after 6 ticks unless D+0x474 (keep) is set; bUse = 0: `CamOsTimerDelete`. Stands in for the HW frame IRQ without a sensor |
| `erasebuffer` | dev, port, bErase | port +0x1F8: zero-fill FRAME_BASE buffers before queueing (§3.8) |
| `devmask` | dev, bMask | `MHal_VIF_InputMask(dev, bMask)` directly |
| `enport` | dev, port, bEn | bEn ≠ 1 → DisableOutputPort; then EnableOutputPort |
| `cfgport` | dev, port, cropX, cropY, cropW, cropH, destW, destH, pixel | DisableOutputPort, SetOutputPortAttr with the given `MI_VIF_OutputPortAttr_t`, EnableOutputPort |
| `resethw` | dev, type | `MHal_VIF_DevReset(dev, type)` for type 0 / 1 / 2; type 3 = reset 1 then 0 (type 0 is a no-op in mhal, `mhal-vif.md` §9.1) |
| `fpsth` | dev, port, FpsTh, FpsFloatTh | port +0x1F9 / +0x1FA — fps threshold below which CheckOutputTaskStatus logs a warning |
| `ptsth` | dev, port, ptsMax, ptsMin | port +0x208 / +0x210 — the PTS window a FRAME_BASE frame must fall in, else it is dropped |
| `fifofull` | dev, cnt | set the module FIFO-full counter (+0x4E9A) and dev (+0x4E9C) — simulates the FIFO-full recovery |

### 3.14 Shutdown
DisableOutputPort: mask the input, `MHal_VIF_PortSetConfig(pipe, port, disabled cfg)`, wait (≤ 50 rounds) for in-flight tasks,
`mi_sys_DisableOutputPort` (failure = BUG). DisableDev: the same for both ports, `MHal_VIF_PipeSetConfig` (release),
**`MHal_VIF_DevDisable(dev)`**, free the pipe, clear the errata flag, `mi_sys_DisableChannel(hDev, 0)`.

### 3.15 Imports and exports
mhal: `MHal_VIF_Init`, `Deinit`, `GroupSetConfig`, `GroupEnable`, `GroupDisable`, `GroupReset`, `DevSetConfig`, `DevEnable`,
`DevDisable`, `DevReset`, `PipeSetConfig`, `PortSetConfig`, `InputMask`, `GetInputMask`, `RegisterEventCallback`,
`RegisterGetPtsCallback`, `PortStatQuery` (all on the camera's path); frame path only: `QueueFrameBuffer`, `DequeueFrameBuffer`,
`QueryFrames`; ext path only: `SetVIFDev2SensorPadMux`, `GetVIFDev2SensorPadMux`. mi_sys: `RegisterDev`, `UnRegisterDev`,
`EnableChannel`, `DisableChannel`, `EnableOutputPort`, `DisableOutputPort`, `SetOutputPortBufExtConf`, `NotifyWakeupISREvent`,
`GetCurPts`, `GetOutputPortSidebandMsg`, `UnBindChnPort`, `PrivateDevChnHeapAlloc`, `PrivateDevChnHeapFree`, `Vmap`, `UnVmap`,
`VFlushInvCache`, `RegistCommand`, `RecodeTimeStamp`. CamOs: mutex, rwsem, spinlock, atomics, timer (the `usetimer` command
only), `Chip_Get_Platform_Name`, `CamOsChipRevision`. Exports: 16 `MI_VIF_*`, no importer.

### 3.16 libmi_vif.so [R]
<!-- ev:mi-capture.3.16 -->
16 exports; no argument checks in user space, packing and copy-back as `mi-user-abi.md` §3.2 (per-function check). The app's calls are rows of the table of `mi-user-abi.md` §10; the other 7:

| function | nr (size) | payload / copy-back |
|---|---|---|
| `MI_VIF_CreateDevGroupExt(group, MI_VIF_GroupAttr_t *a, MI_VIF_GroupExtAttr_t *e)` | 1 (0x34) | {group, a (0x1c), e (0x14)}; on 0 the library's create count +1 (`mi-user-abi.md` §2.2) |
| `MI_VIF_GetDevGroupAttr(group, *a)` | 3 (0x20) | {group}; `*a = +4 (0x1c)` on 0 |
| `MI_VIF_GetDevAttr(dev, *a)` | 5 (0x18) | {dev}; 0x14 back on 0 |
| `MI_VIF_GetDevStatus(dev, *s)` | 8 (0x18) | {dev, s in}; 0x14 back on 0 |
| `MI_VIF_GetOutputPortAttr(dev, port, *a)` | 0xa (0x1c) | {dev, port}; 0x14 back on 0 |
| `MI_VIF_CallBackTask_Register` / `MI_VIF_CallBackTask_UnRegister(…)` | none | return 0 and do nothing (the kernel ioctls 0xe / 0xf are never sent) |

## 4. mi_isp.ko [R]

58 KB of code, 139 functions, 25 exports. One ISP device (dev 0) with up to 32 channels; each channel owns 1..8 mhal "ISP
instances" (one per bit of the sensor bind mask; the app has one). 1 input port and 3 output ports per channel. On this camera
channel 0 takes VIF dev 0 REALTIME on its input and feeds SCL REALTIME from output port 0, and is programmed per frame through
the mi_sys CMDQ. The mhal side of every `MHalVpeIsp*` call: `isp-2-ispmid-mload-ispscl.md` §3.

### 4.1 Load, open, close
| event | what happens |
|---|---|
| insmod | params `g_bUseHwReset` (0), `g_u32DefaultDropNum` (10), `g_ispMaxChn` (16), `g_threadPriority` (98, info +0x1c), `debug_*`; procfs plus **`/proc/isp/isp_debug`** (the write handler parses up to 5 numbers and does nothing with them; read / open are stubs); `MI_DEVICE_Register` → `/dev/mi_isp` |
| first `open` (`MI_ISP_IMPL_Init`, counter) | chipIdx and revision (§1.3); init 32 channel rwsems, 2 Tsems; start the kernel thread **`Isp_DumpTaskThread`** (prio 0x1e, §4.11) |
| last `release` (`MI_ISP_IMPL_DeInit`) | `mi_sys_UnBindChnPort` for the bound ports, stop channels and ports, destroy channels and the device, stop the dump thread |

### 4.2 Channel block (0x6E0 bytes)
Channel-table slot `*(char**)(0x250a4 + chn·4)` (NULL = not created; byte 0 = 1 when created). Per-channel rwsem index
= `(chn_lo + dev·0x20)·0x28` into `g_stIspChnnRwMutex`. Fields: +0x04 dev, +0x08 stitch / pass index, +0xFC the mhal input
config (layout `isp-2-ispmid-mload-ispscl.md` §3.2), +0x138 rotation config, +0x148..+0x184 a six-slot lock-trace ring
(debug), +0x1C8 skip-until frame, +0x1F0 input bind module (6 = VIF realtime), +0x210 input mode (1 = frame mode), +0x628 drop
counter, +0x630 frame counter, +0x658 u8 fill-input-0xAA debug, +0x6A0..+0x6D6 zoom state (§4.8), +0x6B4 sensor-switch
frame threshold and +0x6BC switch / zoom state (§4.8, §4.11).

### 4.3 ioctls
`/dev/mi_isp`, 25 entries in nr order; dev must be 0, else `0xA0078008`.

| nr | name (payload) | kernel action | app |
|---|---|---|---|
| 0 | CreateDevice `{dev; u32 stitchMask}` | §4.4 | yes |
| 1 | DestoryDevice `{dev}` | refuses while channels exist; `MHalVpeIspEnableIrq(ctx, irq, 0)`, `CamOsIrqFree`, **`MHalVpeIspDeInit()`**, `mi_sys_UnRegisterDev`, free per-channel IQ buffers | at exit |
| 2 | CreateChannel `{dev; chn; m6_isp_chn 0x50}` | §4.4 | yes |
| 3 | DestroyChannel `{dev; chn}` | `MHalVpeDestroyIspInstance(inst)` for each instance, free the channel block | at exit |
| 4/5 | Set/GetInputPortCrop `{dev; chn; rect}` | set: store (w, h ≠ 0) as user crop; get: the effective input crop (user crop, else filled from the incoming frame in OnPreProcessInputTask) | get: yes |
| 6/7 | Set/GetChnParam `{dev; chn; m6_isp_para 0x10}` | §4.4 | set: yes |
| 8/9 | Start/StopChannel `{dev; chn}` | §4.4 / stop: sideband message, `mi_sys_BlockingDisableInputPort(h, chn, 0, 0, 1000 ms)`, `mi_sys_DisableChannel`, wait in 20 ms steps for in-flight tasks | yes |
| 0xa/0xb | Set/GetOutputPortParam `{dev; chn; port; {rect crop; pixfmt}}` | port ≤ 2; crop x, y rounded up to even, w, h down to even, crop on = w, h ≠ 0; pixel stored **only if 0 (YUYV) or 0xB (NV12)**, other values silently ignored | set: yes |
| 0xc/0xd | Enable/DisableOutputPort `{dev; chn; port}` | enable: port 2 only when port 0 or 1 is on (+ `mi_sys_SetOutputPortBufClearConf(…, 2, 1)`); `mi_sys_SetOutputPortBufExtConf(h, chn, 0, port, &{16, 2, 1, 0})`, `mi_sys_EnableOutputPort(h, chn, 0, port)` (failure = BUG). disable: `mi_sys_BlockingDisableOutputPort` | yes |
| 0xe/0xf | Alloc/Free_IQDataBuf `{u32 size; u32 uva out}` / `{u32 uva; …}` | §4.6; only libmi_iqserver calls them | no |
| 0x10/0x11 | Set/GetIQApiData `{isp_api_hdr 0x18; u32 pData}` | §4.6 | **yes (all IQ calls)** |
| 0x12/0x13 | CallBackTaskRegister / Unregister `{MI_ISP_CallBackParam_t *}` | §4.10; never sent (libmi_isp returns before the ioctl, §5.5) | no |
| 0x14 | SkipFrame `{dev; chn; n}` | §4.9 | no |
| 0x15–0x18 | LoadPortZoomTable, StartPortZoom, StopPortZoom, GetPortCurZoomAttr | §4.8 | no |

Exports `MI_ISP_*` (25): the same IMPL calls for in-kernel users; no `.ko` on the camera imports them. Exported and called by
mi_isp itself: `MI_ISP_PASS_SetEnqueueMsg(chn, bEnable)` (§4.7).

### 4.4 Bring-up (the app's calls)
1. **CreateDevice(0, mask)**: already created → 0. Fill the info record {module 0x21, dev 0, +8 1 input port, +0xc 3 output
   ports, +0x10 32 channels, +0x14 1 pass, +0x1c priority, +0xc4 0x80, callbacks GetInterBindInputPortPassId,
   GetInterBindOutputPortPassId, OnPreProcessInputTask, OnPrepareOutputTaskMetaInfo, EnqueueInputTask, BarrierInputTask,
   CheckInputTaskStatus, DequeueInputTask, CheckOutputTaskStatus, DequeueOutputTask, RejectCmdq, OnGetInputPortBindCapability,
   OnGetOutputPortBindCapability, OnBindInputPort, OnUnBindInputPort, OnBindOutputPort, OnUnBindOutputPort,
   OnGetPassInputPortNum, GetPassOutputPortNum, ResetDevPass}; `mi_sys_RegisterDev(...)` (failure → `0xA0078010`
   SYS_NOTREADY) + the 16 proc commands (§4.12). Then **`MHalVpeIspInit()`**, **`MHalVpeIspGetIrqNum(0, &irq)`**,
   **`CamOsIrqRequest(irq, _MI_ISP_IMPL_Isr, "ISP-IRQ", dev)`**, **`MHalVpeIspEnableIrq(ctx, irq, 1)`** (failure → returns 3,
   not an MI code); store the stitch mask; per-channel Tsems; a waitqueue for the no-CMDQ path.
2. **CreateChannel(0, chn, attr)**: chn < 32; exists → 0. Instance count = bit count of `attr.sensorId` (0 → 1). Per instance
   *i*: **`MHalVpeCreateIspInstance(&ops{MiSysAlloc, MiSysFree, MiSysMap, MiSysUnMap, MiSysFlashCache}, &{u32 0, u32 chn+i},
   &inst)`** (ops wrap `mi_sys_PrivateDevChnHeapAlloc` / `mi_sys_PrivateDevChnHeapFree`, `mi_sys_Vmap` / `mi_sys_UnVmap` /
   `mi_sys_VFlushInvCache`; failure → `0xA0078004`); if `attr.iqVer.size != 0`: **`MHalVpeIspSnrEarlyInitParam(inst, &{rev,
   size, data[size]})`** (no effect in mhal: `isp-2-ispmid-mload-ispscl.md` §3.1). `mi_sys_SetInputPortBufExtConf(h, chn, 0, 0,
   &{16, 2, 1, 0})`; keep attr; defaults (the ports' peer module 0x24 = none).
3. **SetChnParam(0, chn, para)**: HDR ≤ 4, 3DNR level < 8, mirror / flip ≤ 1, rotation ≤ 3; **rotation or flip need 3DNR ≠
   0** (else ILLEGAL_PARAM `…8003`); refused while the channel is started (`…8008`). When 3DNR or rotation is being switched
   off: **`MHalVpeIspRotationConfig(inst, rotCfg)`** and **`MHalVpeIspInputConfig(inst, inCfg)`** at once; otherwise stored and
   applied by the next EnqueueInputTask. How mi_isp derives the rotation code from mirror / flip / rotate (`isp_rotation_code` (0x10ed8)) is not
   read.
4. **SetOutputPortParam / EnableOutputPort** (port 0, NV12): stored, mi_sys port enabled (§4.3).
5. **StartChannel(0, chn)**: first time `mi_sys_EnableInputPort(h, chn, 0, 0)`, `mi_sys_EnableChannel(h, chn)` (failures = BUG);
   `mi_sys_SetInputPortSidebandMsg(h, chn, 0, 0, 0x80000000:00000000)` (type 4, bit 0 clear).
6. **Binds** (mi_sys): capabilities §1.5. `OnBindInputPort`: REALTIME is accepted only from VIF and only once per device (it
   switches the device to realtime mode); stores peer and bind param. `OnBindOutputPort` stores peer and param per port.

[S] (SigmaStar Tiramisu MI_ISP document `isp_zh` §2.11, §3.7): one ISP device; output port 0 binds only
REALTIME, to MI_SCL; «Tiramisu系列芯片3DNR level最大为2» — the 3DNR level is at most 2 on Tiramisu chips (higher = stronger,
more buffers), where the kernel check of step 3 accepts < 8. [S] (SigmaStar M6 MI FAQ,
`Tiramisu_XLS00V024-20220523/customer/faq/M6/MI.html`): «MI_ISP_IQ_SetNR3D前需要设置3dnr等级，只支持bayer格式» — the 3DNR
level must be set before `MI_ISP_IQ_SetNR3D`, which works only on Bayer input. [S] (SigmaStar FAE, SSD268G, 2023-02): ISP → DISP
can be bound directly, only from ISP device 0 port 1, with the port's output format YUV420SP. [S] (SigmaStar FAE,
«verified on SSD268»): an ISP channel attribute `u32SensorBindId` set to the invalid id makes the channel run without AE.

### 4.5 Interrupt
**`_MI_ISP_IMPL_Isr`**: `st = MHalVpeIspCheckIrq(ctx, irq)` (this call runs mhal's whole ISP interrupt decode,
`isp-2-ispmid-mload-ispscl.md` §3.1); `MHalVpeIspClearIrq`; bit 2 → frame-start callback list; bit 1 → read the CMDQ tag into a
slot; bits 0|1 → frame-done counter, frame-done callback list, `mi_sys_GetCurPts` into the device, wake the no-CMDQ waitqueue,
**`mi_sys_NotifyWakeupISREvent(hDev, 0)`**.

### 4.6 IQ API data path (kernel side)
The only part of the user IQ calls that reaches the kernel (routing in libmi_isp: §5.2).
`SetIQApiData(hdr, pData)`: chn = `hdr.u32Channel` < 32 (else `0xA0078004`); per-channel set-Tsem. If `pData` is the user address
of a buffer from AllocIQDataBuf (16-bucket hash, key = user VA), that buffer's kernel mapping is used (zero copy); else
`hdr.u32DataLen` bytes are copied from `pData` into a per-channel **80 KiB** (0x14000) kernel bounce buffer (allocated on first
use). Then **`MHalIspApiSet(hdr.u32Dev, hdr.u32Channel, hdr.u32CtrlID, hdr.u32DataLen, buf)`**, whose result is returned as
the MI code. `GetIQApiData`: the same lookup (the user data is copied in first, so in-parameters inside the struct reach mhal),
**`MHalIspApiGet(…)`**, `mi_sys_VFlushInvCache`, copy back to `pData` when the bounce buffer was used. `u32HeadSize` is not
checked, and no length check was seen: a data length > 80 KiB would overflow the bounce buffer.
`AllocIQDataBuf(size)`: `mi_sys_MMA_Alloc("mma_heap_name0", "IspApiDataBuf", size, &phy)`, `mi_sys_Mmap(phy, size, &uva)` into the
calling process, `mi_sys_Vmap` for the kernel; insert, return `uva`. `FreeIQDataBuf(uva)`: the reverse. The only caller on the
camera is libmi_iqserver (the IQ-tool TCP server, `isp-6-userspace.md` §6); prog_uvc and the 3A libraries do not import the
wrappers, so on the app's path SetIQApiData always takes the bounce-buffer branch.

### 4.7 Runtime (mi_sys worker, one input task per frame)
- **OnPreProcessInputTask**: computes the effective input crop from the incoming frame (unless user-set); advances a running
  zoom (§4.8); reads the next sensor id for sensor switching (§4.11); the optional 0xAA fill of a frame-mode input (§4.11).
- **RejectCmdq**: the CMDQ space the ISP needs (`n·0x154C0 + 0x16400` bytes), or "no CMDQ" (§1.4).
- **EnqueueInputTask(cmdq, inTask, outTask)** (`_MI_ISP_IMPL_EnqueueInputTask`): the channel must exist; frames below the
  skip-until number are dropped (§4.9); read the input frame descriptor (`isp_read_input_desc` (0x125f0)); if the input format or size changed: input pixel → mhal (fmt, precision)
  by the rule of §3.4, fill the input config (fmt, precision, crop w/h, input type 1 / 2 / 3 = YUV frame / Bayer frame / REALTIME
  bind, 3DNR, pass id) and call **`MHalVpeIspRotationConfig`** + **`MHalVpeIspInputConfig(inst, cfg)`** per instance; if "port
  0 is not tied to SCL" changed: **`MHalVpeIspConfigAckTie1(inst, notSclRealtime)`**. Then per instance:
  **`MHalVpeIspNextFramePTS(inst, cmdq, &pts)`**, build the 0xF8-byte process descriptor (input buffer if frame mode; per output
  port: enable, crop, size, pixel, u64 plane addresses and strides of the mi_sys output buffer — none for the REALTIME port 0;
  layout `isp-2-ispmid-mload-ispscl.md` §3.2), optional `mi_sys_SetMiuProtect` on the output buffers (debug), and
  **`MHalVpeIspProcess(inst, cmdq, &desc)`** — refused when the CMDQ has < 0x975 free entries. On the first realtime task the
  input-port sideband message `0x80000000_00000001` is set through `MI_ISP_PASS_SetEnqueueMsg` (the signal VIF waits for, §3.8).
  Returns 0 accepted / 2 retry / 3 drop.
- **CheckInputTaskStatus**: with CMDQ — compare the task's 16-bit CMDQ tag with the CMDQ's current tag (vtable +0x14);
  without — compare the frame counter. **BarrierInputTask** (no CMDQ) waits ≤ 200 jiffies for the frame-done IRQ.
- **OnPrepareOutputTaskMetaInfo / CheckOutputTaskStatus / DequeueOutputTask / DequeueInputTask**: finish the task, PTS and
  metadata into the output buffers, fps statistics; task-dump copies (`mi_sys_MemcpyPa`, debug); while a zoom table runs, the
  current zoom window goes into each output buffer's sideband (§4.8); the sensor switch (§4.11).
- **ResetDevPass** → `MHalVpeIspInputClose`.

### 4.8 Port zoom, ioctls 0x15–0x18
Digital zoom: a user-loaded table of crop windows that the ISP walks frame by frame. Gated by a module flag the word at 0x25020 (set
when the feature is enabled); prog_uvc sends none of these ioctls.
- **0x15 LoadPortZoomTable `{dev; chn; u32 pData; u32 count}`**: the channel must exist; frees any previous table (unmap,
  `mi_sys_PrivateDevChnHeapFree`, releasing a MIU-protect block if one was set); allocates `count·10` bytes
  (`_MI_ISP_IMPL_MiSysAlloc(0, "ISPZoomTable", count·10, &phy)`), `mi_sys_Vmap`s it and copies the user table in
  (`CamOsCopyFromUpperLayer`); phy at +0x6A0, VA at +0x6B8, count at +0x6A8. **Each entry is 10 bytes** (a crop window).
  Refused unless the zoom state (+0x6BC) is idle (0 or 3).
- **0x16 StartPortZoom `{dev; chn; u32 from, to, ?}`**: from, to < count; stores start / from / to at +0x6C0 / +0x6C4 / +0x6C8,
  direction at +0x6CC (1 = up when from < to, 2 = from > to, 3 = equal), sets running (+0x6BC = 1, +0x6B6 = 1). Per frame
  OnPreProcessInputTask steps the current index (+0x6AC) from `from` toward `to` and applies the entry's window as the input crop.
- **0x17 StopPortZoom `{dev; chn}`**: if running (+0x6BC == 1) → 3 (stopped, table kept); else NOT_SUPPORT.
- **0x18 GetPortCurZoomAttr `{dev; chn; out}`**: returns {start +0x6C0, from +0x6C4, to +0x6C8, current index +0x6AC}.
- While a table is loaded and running (+0x6B8 ≠ 0, +0x6A8 ≠ 0, +0x6BC = 1), `_MI_ISP_IMPL_OnPrepareOutputTaskMetaInfo` writes
  the current window (+0x6D0 x, +0x6D2 y, +0x6D4 w, +0x6D6 h) and the buffer's pixel format into each output buffer's sideband as
  type 1 + ACKED; mi_scl takes it as its input crop and input format (bit layout: `mi_sys-1-engine.md` §5.1).

### 4.9 SkipFrame, ioctl 0x14
`SkipFrame {dev 0; chn < 0x20; u32 n}`: the channel must exist; under the channel rwsem the skip-until frame (+0x1C8) = frame
counter (+0x630) + n. EnqueueInputTask drops frames whose number is below it (the drop result). Not used by prog_uvc.

### 4.10 Callback-task ioctls 0x12 / 0x13
`CallBackTaskRegister` / `Unregister {MI_ISP_CallBackParam_t *}` — an in-kernel hook with two **module-global** lists (not per
channel) keyed by IRQ type: type 0 frame start (head the word at 0x1d03c), type 1 frame done (head the word at 0x1d038). Register allocates
a 0x20-byte node `{mode, irqType, f2, pfn, data, link{next, prev}}` and splices it in under a spinlock; unregister matches by the
stored pointer and frees. The ISR runs the lists (§4.5). These ioctls are never sent on this camera.

### 4.11 Multi-instance channels, sensor switching, frame-mode input, dump thread
None of these runs on the single-sensor REALTIME path.
- **Multi-instance (stitch) channels**: every per-frame call (InputConfig, Process, …) iterates the instances CreateChannel made,
  one per set bit of `attr.sensorId` (§4.4 step 2); with one sensor the loop runs once.
- **Sensor switching** (time-multiplex of several sensors onto one ISP): in DequeueOutputTask, when the switch state (+0x6BC ==
  2) and the frame threshold +0x6B4 is reached and the input is a VIF realtime bind (+0x1F0 == 6): `MHal_VIF_InputMask(vifDev,
  1)`, `MHal_SNR_SwitchSensor(padMode, sensorId-from-table)`, `MHal_VIF_InputMask(vifDev, 0)`; OnPreProcessInputTask reads the next
  sensor id with `MHal_SNR_GetSwitchSensorId`.
- **Frame-mode input** (the ISP reads a DRAM frame instead of the VIF stream, input mode +0x210 == 1): the debug byte +0x658 makes
  OnPreProcessInputTask `memset(mappedInputFrame, 0xAA, size/2)` before processing — a fill pattern for bring-up without a
  sensor. On this camera the input type is 3 (REALTIME) and no DRAM input buffer exists.
- **Dump thread** `Isp_DumpTaskThread` (started in Init, prio 0x1e): sleeps on a semaphore; when woken by the `dumptaskfile`
  command it walks a job list, `mi_sys_Vmap`s each recorded task buffer and writes it with `filp_open` / `vfs_write` to the given
  path (kernel address space switched as in §3.13), then frees the job.

### 4.12 Proc commands of `mi_isp0`
16 commands, registered in CreateDevice; `/proc/isp/isp_debug` is the no-op of §4.1.

| command | args | effect |
|---|---|---|
| `debugmutex` | 0 | dump the per-channel lock-trace ring (+0x148..+0x184) |
| `stopchnl` | dev, chn | force-stop a channel (bypasses the normal StopChannel checks) |
| `dumptaskfile` | dev, chn, port, path, cnt, fmt | queue a task-buffer dump for the dump thread (§4.11) |
| `setatom` | dev, chn | the channel's initial output-queue depth |
| `clearbuf` | dev, chn, port, …, … | drop the channel's pending output buffers |
| `debuglv` | dev, chn, level | per-channel trace-level bits |
| `skipframe` | chn-with-bank, cnt | `MI_ISP_IMPL_SkipFrame` (§4.9) — the argument packs dev in bits 5+ |
| `protectport` | dev, chn, port, bOn | toggle `mi_sys_SetMiuProtect` on the port's output buffers |
| `clearbindq` | dev, chn, port | flush the bound input queue |
| `fpsth` | dev, chn, FpsTh, FpsFloatTh | fps-warning threshold |
| `ptsth` | dev, chn, ptsMax, ptsMin | PTS window |
| `enableport` | dev, chn, port, bEn | re-run EnableOutputPort / DisableOutputPort |
| `sendoneframe` | dev, chn, ? | push one frame through (frame-mode bring-up) |
| `dnrlevel` | dev, level | `_MI_ISP_IMPL_DebugSet3DNROnOff` — force the 3DNR level |
| `resethw` | 0 | reset the ISP hardware |
| `startzoom` | dev, chn, from, to | `_MI_ISP_IMPL_DebugPortZoom` → StartPortZoom (§4.8) |

### 4.13 Imports and exports
mhal: `MHalVpeIspInit`, `MHalVpeIspDeInit`, `MHalVpeIspGetIrqNum`, `MHalVpeIspEnableIrq`, `MHalVpeIspCheckIrq`,
`MHalVpeIspClearIrq`, `MHalVpeCreateIspInstance`, `MHalVpeDestroyIspInstance`, `MHalVpeIspSnrEarlyInitParam`,
`MHalVpeIspInputConfig`, `MHalVpeIspRotationConfig`, `MHalVpeIspConfigAckTie1`, `MHalVpeIspNextFramePTS`, `MHalVpeIspProcess`,
`MHalVpeIspInputClose`, `MHalIspApiSet`, `MHalIspApiGet`; sensor-switch path only: `MHal_SNR_SwitchSensor`,
`MHal_SNR_GetSwitchSensorId`, `MHal_VIF_InputMask`. mi_sys: `RegisterDev`, `UnRegisterDev`, `EnableChannel`, `DisableChannel`,
`EnableInputPort`, `BlockingDisableInputPort`, `EnableOutputPort`, `BlockingDisableOutputPort`, `SetInputPortBufExtConf`,
`SetOutputPortBufExtConf`, `SetOutputPortBufClearConf`, `SetInputPortSidebandMsg`, `NotifyWakeupISREvent`, `GetCurPts`,
`UnBindChnPort`, `PrivateDevChnHeapAlloc`, `PrivateDevChnHeapFree`, `Vmap`, `UnVmap`, `VFlushInvCache`, `MMA_Alloc`, `MMA_Free`,
`Mmap`, `Munmap`, `Miu2Cpu_BusAddr`, `SetMiuProtect`, `MemcpyPa`, log and proc. CamOs: IRQ, thread, Tsem, rwsem, spinlock,
atomics, `CamOsMsSleep`, `CamOsCopyFromUpperLayer`. Exports: 25 `MI_ISP_*`, no importer.

## 5. libmi_isp.so [R]

402 exported functions plus 2 data symbols: **265 API wrappers** (30 of them called by the app), **23 direct ioctl wrappers**
(11 called by the app), **90 `call_MI_ISP_*` helpers** of the API-bin loader and **24 others** — dispatchers, loaders, the agent
table (1 called by the app). The app's calls are rows of the table of `mi-user-abi.md` §10. Every API wrapper was tabulated with its direction, ctrl id and data length (read from the header literal the function
stores), and every `call_*` helper with its struct size and target; against the size the kernel checks for each id, **all
240 ids the kernel knows have matching sizes** [R].
Ids and data sizes of the app's calls: column `payload_layout` of the table of `mi-user-abi.md` §10; ids and payloads of every API
class: `isp-3-iq-api.md`. Inside prog_uvc these functions are statically linked copies of the same code.

### 5.1 API wrapper
Every `MI_ISP_<grp>_{Set,Get}<Name>(dev, chn, void *data)` builds a 0x18-byte header on the stack and calls
`_MI_ISP_SetIspApiData(&hdr, data)` / `_MI_ISP_GetIspApiData(&hdr, data)`; its return is the wrapper's:
```c
struct isp_api_hdr {
    u32 u32HeadSize;  /* +0x00 = 0x18 */
    u32 u32DataLen;   /* +0x04 = sizeof(data struct), e.g. 0x4c Contrast, 0x5b8 AWB attr */
    u32 u32CtrlID;    /* +0x08 API id: 0x10xx IQ, 0x14xx AE, 0x18xx AWB, 0x1cxx AF, 0x2exx CUS3A, 0x30xx debug */
    u32 u32Channel;   /* +0x0c = 2nd argument */
    u32 u32Dev;       /* +0x10 = 1st argument */
    u32 u32Reserved;  /* +0x14 = 0 */
};
```
(`MI_ISP_AWB_QueryInfo`: id 0x1801, 0x18 bytes.)

### 5.2 Dispatch and routing
`_MI_ISP_SetIspApiData` / `_MI_ISP_GetIspApiData`:
1. `fn = agent_set[dev*16 + chn]` (resp. `agent_get`); if fn != NULL and `fn(hdr, data) != 0xA0078008` → return its result.
2. else `MI_ISP_SetIQApiData` / `MI_ISP_GetIQApiData`: ioctl **mi_isp 0x10 / 0x11**, payload 0x1c = `{isp_api_hdr (0x18), u32
   pData}` — the kernel fetches / stores `u32DataLen` bytes at the **user pointer pData** itself (§4.6; both ioctls are `_IOW`
   because only the header block is copied in).

`MI_ISP_RegisterIspApiAgent(dev, chn < 16, set_fn, get_fn)` fills the table (chn ≥ 16 → returns 6). **`MI_ISP_CreateChannel`
always registers libcus3a's `Cus3A_SetIspApiData` / `Cus3A_GetIspApiData`**, through `MI_ISP_EnableUserspace3A`: `CUS3A_Init`,
`CUS3A_EnableUserspaceAE` / `CUS3A_EnableUserspaceAWB` / `CUS3A_EnableUserspaceAF(dev, chn, 1)` — even if the ioctl failed.
`CUS3A_EnableUserspaceAF` registers libcus3a's built-in Sigma AF adaptor (`isp-6-userspace.md` §2.7). `MI_ISP_DestroyChannel` unregisters and
calls `CUS3A_Release(dev)`. `CUS3A_GetAlgoStatus(dev, chn, u32[3])` only reads three ready flags from libcus3a's state (no ioctl).

libcus3a routes by id:

| ids | goes to |
|---|---|
| 0x1400–0x17FF, 0x3000 | `Cus3A_ProcAeCtrl` |
| 0x1800–0x1BFF, 0x240F, 0x2C03–0x2C06, 0x3001 | `Cus3A_ProcAwbCtrl` |
| 0x1C00–0x1FFF, 0x3003 | `Cus3A_ProcAfCtrl` |
| 0x3002 and the other ids in 0x2FFF–0x3100 | swallowed: 0 returned, nothing sent |
| everything else (incl. 0x2400–0x27FF other than 0x240F, 0x2C00–0x2DFF other than 0x2C03–0x2C06) | `0xA0078008` → the kernel ioctl 0x10 / 0x11 → `MHalIspApiSet` / `MHalIspApiGet` |

The Proc functions return −1 (→ `0xA0078008`, the kernel) only while their algorithm is not running; once it runs, every id of its
range returns 0 to the application and the algorithm's own status (size mismatch, unsupported id) is discarded. So the AE, AWB and
AF ids are served in user space (libcus3a → libispalgo, inside prog_uvc's process) while the algorithm runs and by the kernel
otherwise; IQ ids (0x10xx) always reach mi_isp.ko. Counts over the 265 wrappers: 160 kernel, 47 AE, 23 AWB, 35 AF. Which
id each algorithm accepts: `isp-5-3a.md` §2.16, §3.11, §4.8.

### 5.3 Ids the libraries send that the kernel does not accept
Not in the kernel's switches → "Not support - Ctrl ID", status 0xF (`isp-3-iq-api.md` §1.3): 0x101A FPN, 0x1020 EnSysMcnrMemory, 0x1025
NR3D_P1, 0x1026 WDRCurveAdv, 0x2802 IQind, 0x2803 APIBypassMode (imported by libmi_iqserver), **0x66 `MI_ISP_GetIspRoot`**; plus
0x1805 AWB CTWeight and 0x2C03 / 0x2C04 / 0x2C06 AWB calibration whenever libcus3a's AWB is not running. `MI_ISP_GetIspRoot` is
accepted by neither libcus3a nor the kernel, so it always fails on this firmware and libcus3a uses its fallback directory
`/config/iqfile`; the one other way to serve 0x66, a command filter registered through `MS_CAM_RegIspCmdFilter`, is never set
up on this firmware [R: no caller in mhal.ko, the symbol in no other file of the dump] (isp-6-userspace.md §2.8).

### 5.4 Callbacks the 3A and IQ-server libraries import
All are API wrappers of §5.1 (only `CUS3A_Enable` has logic of
its own); payload contents are the kernel's (`isp-3-iq-api.md`).

| export | dir | id | bytes | importer |
|---|---|---|---|---|
| `MI_ISP_AE_GetAeHwAvgStats` | Get | 0x2E01 | 0xB408 | libcus3a |
| `MI_ISP_AE_GetHisto0HwStats` / `Histo1` | Get | 0x2E02 / 0x2E03 | 0x100 | libcus3a |
| `MI_ISP_CUS3A_GetAeInitStatus` | Get | 0x2E04 | 0x6C | libcus3a |
| `MI_ISP_CUS3A_GetAeStatus` | Get | 0x2E05 | 0x41 | libcus3a |
| `MI_ISP_CUS3A_SetAeParam` | Set | 0x2E06 | 0x38 | libcus3a |
| `MI_ISP_AWB_GetAwbHwAvgStats` | Get | 0x2E07 | 0x8708 | libcus3a |
| `MI_ISP_CUS3A_Enable` | Set | 0x2E08 | 3 | libcus3a — prints `"[%s] AE = %d, AWB = %d, AF = %d"` first; the kernel handles 0x2E08 before the channel check |
| `MI_ISP_CUS3A_GetAwbStatus` | Get | 0x2E09 | 0x29 | libcus3a |
| `MI_ISP_CUS3A_SetAwbParam` | Set | 0x2E0A | 0x18 | libcus3a |
| `MI_ISP_AWB_GetAwbHwAvgStatsShort` | Get | 0x2E19 | 0x8708 | libcus3a |
| `MI_ISP_AE_GetRgbIrHistoHwStats` | Get | 0x2E1E | 0x200 | libcus3a |
| `MI_ISP_CUS3A_GetAfStatus` | Get | 0x2E20 | 0x10 | libcus3a |
| `MI_ISP_CUS3A_SetAfParam` | Set | 0x2E21 | 0xC | libcus3a |
| `MI_ISP_CUS3A_SetAFWindow` / `SetAFFilter_Geo` / `SetAFFilterSq` / `SetAFRoiMode` | Set | 0x2E0E / 0x2E0F / 0x2E10 / 0x2E11 | 0x14 / 0x8C / 0x34 / 8 | libispalgo, libsigma3a |
| `MI_ISP_CUS3A_SetAFYParam` / `SetAFSource` / `SetAFPreFilter` / `SetAFYMap` / `SetAFLdg` | Set | 0x2E24 / 25 / 26 / 27 / 28 | 3 / 4 / 0xC / 0x1C / 0x28 | libispalgo |
| `MI_ISP_CUS3A_SetAFFilter` | Set | 0x2E0F | takes a 0x54-byte struct | libispalgo, libsigma3a — copies the 0x54 bytes into a 0x8C stack buffer and calls `SetAFFilter_Geo`; **bytes 0x54–0x8B are uninitialised stack**; `GetAFFilter` = `GetAFFilter_Geo` into a 0x8C buffer, the first 0x54 bytes copied out |
| `MI_ISP_GetIspRoot` | Get | 0x66 | 0x40 (path) | libcus3a — fails, §5.3 |
| `MI_ISP_CUS3A_GetImageResolution` / `EnableISPOutImage` (Get) / `GetISPOutImageCount` / `CaptureHdrRawImage` | Get / Get / Get / Set | 0x2E13 / 0x2E14 / 0x2E15 / 0x2E16 | 0x10 / 0x14 / 4 / 0x18 | libmi_iqserver |
| `MI_ISP_ReadSensorData` / `WriteSensorData` | Get / Set | 0x2E18 | 0x18 | libmi_iqserver |
| `MI_ISP_IQ_GetAPIBypassMode` / `MI_ISP_IQ_SetAPIBypassMode` | Get / Set | 0x2803 | 8 | libmi_iqserver — the kernel refuses (§5.3) |
| `MI_ISP_GetChnParam`, `MI_ISP_Alloc_IQDataBuf`, `MI_ISP_Free_IQDataBuf`, `MI_ISP_SET_ALL`, `MI_ISP_GET_ALL` | — | §5.5–§5.6 | | libmi_iqserver |
| `MI_ISP_EnableUserspace3A` / `MI_ISP_DisableUserspace3A` | — | §5.2 | | libmi_vpe (not used on the camera) |

The 3A libraries' other libmi_sys imports (`MI_SYS_MMA_Alloc` / `Free`, `MI_SYS_Mmap` / `Munmap`): `mi_sys-2-interfaces.md` §5.

### 5.5 Direct ioctl wrappers outside the app's table
The direct wrappers not in the table of `mi-user-abi.md` §10; `/dev/mi_isp`, kernel actions §4.3.

| function | nr (size) | payload | copy-back |
|---|---|---|---|
| `MI_ISP_SetInputPortCrop(dev, chn, MI_SYS_WindowRect_t *r)` | 4 (0x10) | dev, chn, rect (8) | — |
| `MI_ISP_GetChnParam(dev, chn, *p)` | 7 (0x18) | dev, chn | +8 (0x10) when 0 |
| `MI_ISP_GetOutputPortParam(dev, chn, port, *p)` | 0xb (0x18) | dev, chn, port | 3 words from +0xc when 0 |
| `MI_ISP_Alloc_IQDataBuf(u32 size, void **uva)` | 0xe (8, `_IOWR`) | size, +4 = 0 | `*uva = +4` always |
| `MI_ISP_Free_IQDataBuf(void *uva)` | 0xf (8) | uva | — |
| `MI_ISP_SetIQApiData(hdr*, data*)` / `GetIQApiData` | 0x10 / 0x11 (0x1c) | §5.2 | — |
| `MI_ISP_SkipFrame(dev, chn, n)` | 0x14 (0xc) | dev, chn, n | — |
| `MI_ISP_LoadPortZoomTable(dev, chn, table*)` | 0x15 (0x10) | dev, chn, two words of the table struct (entry count + user pointer to the entries [I]; the kernel reads them as `pData`, `count`, §4.8) | — |
| `MI_ISP_StartPortZoom(dev, chn, attr*)` | 0x16 (0x14) | dev, chn, 3 words | — |
| `MI_ISP_StopPortZoom(dev, chn)` | 0x17 (8) | dev, chn | — |
| `MI_ISP_GetPortCurZoomAttr(dev, chn, attr*)` | 0x18 (0x14) | dev, chn | 3 words from +8 always |
| `MI_ISP_CallBackTask_Register` / `MI_ISP_CallBackTask_Unregister(…)` | none | prints `"[%s] kernel mode support"`, returns **0xA0078012** | — |

No user-space binary on the camera imports the zoom, skip, crop or callback wrappers.

### 5.6 Dispatchers and helpers
- **`MI_ISP_SET_ALL(dev, chn, id, size, data)` / `MI_ISP_GET_ALL`** (source file `isp_cus3a_api.c`): ids 0x1000–0x1FFF and
  0x2400–0x2DFF go first to `MI_ISP_ALGO_SET_ALL` / `MI_ISP_ALGO_GET_ALL` (which return −1 for other ids); then ids
  0x2E0E…0x2E28 and 0x3000 / 0x3001 / 0x3003 are matched with a required size and the matching CUS3A / debug wrapper is called;
  wrong size → `"[%s][%s] APIID:%d error param"` and the earlier result. **`MI_ISP_ALGO_SET_ALL` / `MI_ISP_ALGO_GET_ALL`**
  (`isp_api_iq.c`): one switch over 97 Set / 113 Get ids; each case checks the size and calls the matching `MI_ISP_IQ_*` /
  `MI_ISP_AE_*` / `MI_ISP_AWB_*` / `MI_ISP_AF_*` wrapper (in Ghidra's text the AE cases appear as 0…0x13 because the compiler
  switched on `id − 0x1403`). The sizes agree with the wrappers. Special
  Get cases: 0x103A → `IQ_GetDayNightDetection`, 0x103B → `IQ_QueryDayNightInfo`, 0x2C05 → `AWB_GetHWStats`; special Set: 0x103A
  → `IQ_SetDayNightDetection`, 0x2417 → `ALGO_API_CmdLoadCaliData`. These are the entry points of libmi_iqserver (the IQ tool,
  `isp-6-userspace.md` §6).
- `MI_ISP_IQ_SetDayNightDetection(dev, chn, p)` = `AE_SetDayNightDetection & AWB_SetDayNightDetection` (bitwise AND of the two
  results — 0 if either returned 0); `IQ_QueryDayNightInfo` likewise with the `Query` pair; `IQ_GetDayNightDetection` sends only
  the AWB id 0x1807.
- `MI_ISP_AWB_GetHWStats(dev, chn, out)`: `calloc(0x8708)`, `AWB_GetHWStats_Cus` (Get 0x2E07), copies 0x8700 bytes from +8 to
  `out` (drops the 8-byte block-count header).
- The 90 `call_MI_ISP_<X>(dev, chn, void **pp)` helpers: `calloc(size)`, copy `*pp` (size bytes), call `MI_ISP_<X>`, free; every
  size equals the wrapper's.
- **`MI_ISP_API_Reset(dev, chn, item)`** calls a setter with an all-zero struct: 1 FPN, 2 RGBMatrix, 3 Saturation, 4 OBC, 5
  OBC_P1, 6 FalseColor, 7 CrossTalk, 8 NRDeSpike, 9 Sharpness, 10 NR3D, 0xB Brightness, 0xC Lightness, 0xD Contrast, 0xE
  RGBGamma, 0xF ColorToGray, 0x10 NRLuma, 0x11 Defog, 0x13 DynamicDP, 0x14 HSV, 0x15 WDR, 0x16 RGBIR, 0x17 YUVGamma, 0x19
  COLORTRANS, 0x1A HDR, 0x1B Effect, 0x1C PFC, 0x1D NRChroma, 0x1E DEMOSAIC, 0x1F NR3D_P1, 0x20 WDRCurveAdv, 0x23 LSC, 0x24
  LSC_CTRL, 0x26 ALSC_CTRL; other items return an uninitialised value.
- `MI_ISP_API_DUMP_PARAM()` is empty; `MI_ISP_ALGO_API_DisableAllApi()` returns 0.

### 5.7 API bin loader
`MI_ISP_API_CmdLoadBinFile` (prog_uvc `-i /customer/iq_file_gc8613.bin`) works in user space only: it reads the file into memory,
then `MI_ISP_ALGO_API_CmdLoadBinFile(dev, chn, buf, key)`:
- header (8 × u32): `+4` version (hi16 major, lo16 minor), `+8` data length, `+0xc` checksum, `+0x10` key, `+0x18` low byte =
  checksum enable. The "CRC32" (`MI_ISP_API_CRC32`) is a **plain byte sum** of the data (`~(-1 - Σbytes)`).
- version vs `MI_ISP_IQ_GetVersionInfo` (an IQ get through §5.2): major mismatch → −1, minor mismatch → warning only; the `key`
  argument must be 0 or equal to the header key, else −1.
- records from `+0x20`: `{u32 key; u32 apiId; u32 len; data[len]}`; record data = `{u16 op; u16 n; u32 off[n]; …}`, pointer
  i = `data + 4 + 4n + off[i]`. `MI_ISP_API_EXCUTE_CMD` looks up `(apiId, op)` in a static table and calls the matching
  `call_MI_ISP_IQ_Set*` (copies the struct, then `MI_ISP_IQ_Set*` → §5.2). Loading the bin is a sequence of ordinary IQ / AE /
  AWB set calls; nothing bin-specific reaches the kernel. Bin format as a whole: `isp-3-iq-api.md` §6.

### 5.8 Calibration loader
**`MI_ISP_API_CmdLoadCaliData(dev, chn, item, path)`** reads the whole file and calls **`MI_ISP_ALGO_API_CmdLoadCaliData(dev,
chn, buf, len, item)`**: header `u32[5] {ver, dataSize, checksum, res0, res1}`; if `ver & 0xFC` the checksum is
`MI_ISP_API_CRC32(buf + 0x14, dataSize − 0x14)` (the byte sum of §5.7) and is enforced when `(ver & 0xFFFF) >> 8 != 0`
(mismatch → "Checksum error", −1). Item → Set id with the whole buffer as data: 0 AWB → 0x240F, 1 OBC → 0x2410, 2 SDC → 0x2414,
3 ALSC → 0x2415, 4 LSC → 0x2417; other → "eCaliItem is not support!", −1. Header: `u32DataLen` = the file length,
`u32Reserved` = item for AWB, 0 otherwise. Prints "Load calibration bin Success/Fail".

## 6. mi_scl.ko [R]

61 KB of code, 121 functions, 20 exports. Four SCL devices, each a set of hardware scalers (`HWSCL0..5` = mask bits 0..5) with a
fixed input role; up to 34 channels per device (0x20 / 0x21 reserved for dev 3); 6 output ports per channel, port *i* mapped to
the *i*-th HW scaler of the device mask. The app uses **dev 0, mask 7 (HWSCL0..2), channel 0, port 0**, input REALTIME from ISP
port 0, output FRAME_BASE (or HW_RING) to VENC / DISP. Per frame SCL is programmed through the mi_sys CMDQ like the ISP. The mhal
side of every `MHalVpeScl*` call: `mhal-scl-jpe-cmdq.md` §3.

### 6.1 Devices and roles
Role = lowest bit of a per-device role mask (dev0 1, dev1 2, dev2 4, dev3 0x18); the instance type is the role number [R from
CreateDevice / CreateChannel / OnBindInputPort; the purposes I]. Devices 1 and 3 answer `NeedIndividualWorkThread` with 1, so
mi_sys gives them their **own worker thread**; dev 0 and dev 2 run on the shared thread.

| dev | type | input accepted | worker | purpose |
|---|---|---|---|---|
| 0 | 0 | REALTIME only, from ISP port 0 | shared | the camera's path |
| 1 | 1 | FRAME_BASE, or REALTIME from module 0x20 (JPD [I]) | own | decode / offline scale |
| 2 | 2 | REALTIME from VIF | shared | VIF-direct scale |
| 3 | 3 (4 with rotation) | type 3: FRAME_BASE or REALTIME from 0x20; type 4: FRAME_BASE only | own | `StretchBuf`, rotation; created with mask 0x20, creates internal channels 0x20 / 0x21 itself |

Module 0x20 is JPD by the module-id table of `mi-user-abi.md` §4.2 [I]; mi_jpd's output kinds include an EMI ring with a row handshake
with SCL (`mi_jpd-mi_vdec.md` §1.7).

### 6.2 Load, open, close
| event | what happens |
|---|---|
| insmod | params `u32CMDQ_SIZE` (91 136), `g_bUseHwReset` (1), `g_HwIssueReadMorePoint` (64), `g_threadPriority` (98), `debug_*`; procfs + `/proc/scl/scl_debug` (§6.11); `MI_SCL_IMPL_DevInitParam` (§6.10); `MI_DEVICE_Register` → `/dev/mi_scl` |
| first `open` (`MI_SCL_IMPL_Init`, counter) | chipIdx and revision (§1.3); channel rwsems; Tsem; `CamOsMemCacheCreate("mi_scl_internal_cachep", 0x478)` (per-channel task descriptor); thread **`SCL_DumpTaskThread`** (debug); **`MHalVpeSclInit(&ops{MiSysAlloc, MiSysFree, MiSysMap, MiSysUnMap, MiSysFlashCache}, 0)`** (ops wrap `mi_sys_MMA_Alloc` / `mi_sys_MMA_Free`, `mi_sys_Vmap` / `mi_sys_UnVmap` / `mi_sys_VFlushInvCache`) |
| last `release` (`MI_SCL_IMPL_DeInit`) | `mi_sys_UnBindChnPort` for the bound ports, stop / destroy channels and devices, **`MHalVpeSclDeInit()`**, destroy the mem cache, stop the thread |

### 6.3 ioctls
`/dev/mi_scl`, 15 entries in nr order.

| nr | name (payload) | kernel action | app |
|---|---|---|---|
| 0 | CreateDevice `{dev; MI_SCL_DevAttr_t{u32NeedUseHWOutPortMask}}` | §6.4 | yes |
| 1 | DestoryDevice `{dev}` | `MHalVpeSclEnableIrq(ctx)` (off), `CamOsIrqFree` per HW scaler, `mi_sys_UnRegisterDev` | at exit |
| 2 | CreateChannel `{dev; chn; MI_SCL_ChannelAttr_t{u32Reserved}}` | §6.4 | yes |
| 3 | DestroyChannel `{dev; chn}` | `MHalVpeSclInstanceDetach`, `MHalVpeDestroySclInstance`, free | at exit |
| 4/5 | Set/GetInputPortCrop `{dev; chn; rect}` | only for types 0 / 2; stored, used as pre-crop | no |
| 6/7 | Set/GetChnParam `{dev; chn; MI_SCL_ChnParam_t{eRot}}` | §6.7 | yes (rot 0) |
| 8/9 | Start/StopChannel `{dev; chn}` | start: first time `mi_sys_EnableInputPort(h, chn, 0)`, `mi_sys_EnableChannel` (failures = BUG), sideband message `0x80000000:0`. stop: sideband message, `mi_sys_BlockingDisableInputPort`, `mi_sys_DisableChannel`, 20 ms waits | yes |
| 0xa/0xb | Set/GetOutputPortParam `{dev; chn; port; MI_SCL_OutPortParam_t 0x18}` | port ≤ 5 and mapped to a HW scaler; pixel ∈ {0..3, 0xA..0xC, 0xE..0x12} (bit mask 0x7DC0F), mirror / flip ≤ 1 (not on type 4), output w, h ≠ 0 (else ILLEGAL_PARAM `…2003`); crop w or h = 0 → crop off; stored, "changed" counters bumped | set: yes |
| 0xc/0xd | Enable/DisableOutputPort `{dev; chn; port}` | enable: the port must have a HW scaler (else `0xA0222002` INVALID_PORTID); `mi_sys_EnableOutputPort(h, chn, 0, port)` (failure = BUG); `mi_sys_SetOutputPortBufExtConf` (§1.6). disable: `mi_sys_BlockingDisableOutputPort` | yes |
| 0xe | StretchBuf `{… 0x70}` (`MI_SCL_DirectBuf_t` src / dst + crop) | §6.8 | no |

Exports: 15 `MI_SCL_*` (in-kernel twins of the ioctls, no importer) and the five `mi_scl_*` of §6.8, imported by `mi_vdec.ko`.

### 6.4 Bring-up
1. **CreateDevice(dev, mask)**: dev < 4, `1 ≤ mask ≤ 0xFFFE`, dev 3 needs mask 0x20, HW scalers not used by another device (else
   ILLEGAL_PARAM); already created with the same mask → 0. `mi_sys_RegisterDev(devinfo{module 0x22, dev, +8 1, +0xc 6, +0x10 34
   channels, +0x14 1, +0x1c priority, +0xc4 0x88 when CMDQ is on, callbacks GetInterBindInputPortPassId,
   GetInterBindOutputPortPassId, ConfigSramBuf, AllowPreemptionSram, AllowMoveableSram, OnPreProcessInputTask,
   OnPrepareOutputTaskMetaInfo, EnqueueInputTask, BarrierInputTask, CheckInputTaskStatus, DequeueInputTask,
   CheckOutputTaskStatus, DequeueOutputTask, NeedIndividualWorkThread (dev 1, 3), RejectCmdq, the bind callbacks,
   OnGetPassInputPortNum, GetPassOutputPortNum, ResetDevPass}, …)` (failure → `0xA022200C`) + the 16 proc commands (§6.11).
   For each role bit: **`MHalVpeSclGetIrqNum(&irq, &ctx)`**, **`CamOsIrqRequest(irq, _MI_SCL_IMPL_Isr, "SCL-IRQ", …)`**,
   **`MHalVpeSclEnableIrq(ctx)`**. Dev 3 then creates, starts and enables port 0 of channels 0x20 and 0x21 itself
   (`mi_sys_SetChnOutputPortDepth(…, 1, 2)`). The SRAM callbacks: `mi_sys-1-engine.md` §9.
2. **CreateChannel(dev, chn, attr)**: chn ≤ 0x21, not existing (else `0xA0222007`); allocate the 0xC98-byte channel block and a
   0x478-byte task descriptor from the cache; **`MHalVpeCreateSclInstance(&inst)`**; type = device role (channel +0x18);
   **`MHalVpeSclInstanceAttach(inst, &{0x02D00500, role cfg[3]})`**; map ports to HW scalers (`hwMask` = 1 << scaler index at
   port +0x12c, `hwId` 0..5 at +0x130; unmapped → 0xFFFF / 0xB); `mi_sys_SetInputPortBufExtConf(h, chn, 0, 0, …)` (§1.6).
3. **SetOutputPortParam / EnableOutputPort / StartChannel** as in §6.3.
4. **Binds**: capabilities §1.5. `OnBindInputPort` for REALTIME checks the peer (type 0: ISP port 0; types 1 / 3: module 0x20;
   type 2: VIF) and stores peer and bind param; `OnBindOutputPort` stores the peer module (port +0x1c0).

### 6.5 Runtime (mi_sys worker; per frame one input task with up to 6 output buffers)
- **OnPreProcessInputTask**: input geometry and format from the incoming frame (or the ISP realtime descriptor); output buffer
  requests; the sideband handling of §6.6.
- **EnqueueInputTask(cmdq, inTask, outTask)** → input DMA / size config when it changed (`MHalVpeSclDmaPortConfig`,
  `MHalVpeSclPortSizeConfig`; `MHalVpeSclRotationConfig` for type 4), then the per-frame worker `scl_frame_worker` (0x180a0): for each enabled
  port that got a buffer **`MHalVpeSclPreCropConfig`** (input crop), **`MHalVpeSclDmaPortConfig(inst, {u64 plane addresses,
  strides, pixel, compress})`**, **`MHalVpeSclPortSizeConfig(inst, {w, h})`**, **`MHalVpeSclPortMirrorFlipConfig`**, then
  **`mi_rgn_Process(&{module 0x22, dev, chn, port, …, w, h, pixel, …}, cmdq)`** (OSD hook, §6.12); after the ports: optional
  **`MHalVpeSclSendSwMessage(inst, {cmdq, …})`** and **`MHalVpeSclProcess(inst, cmdq, taskDesc)`**.
- **RejectCmdq** reports the CMDQ space SCL needs (§1.4).
- **CheckInputTaskStatus / BarrierInputTask**: CMDQ tag compare as in the ISP (§4.7); without CMDQ `MHalVpeSclSetWaitDone(inst,
  …)`.
- **CheckOutputTaskStatus**: `MHalVpeSclGetActiveBuf(taskDesc)` tells which outputs were written; PTS into the buffers;
  task-dump copies (`mi_sys_MemcpyPa`, debug).
- **DequeueOutputTask**: for every port that had a buffer **`mi_rgn_NotifyFrameDone(&{0x22, dev, chn, port})`**;
  `mi_sys_SetMiuProtect` (debug).
- **ISR `_MI_SCL_IMPL_Isr`**: `st = MHalVpeSclCheckIrq(ctx)`, `MHalVpeSclClearIrq`; per-scaler done bits 4..9 and frame bits →
  counters, `mi_sys_GetCurPts`, **`mi_sys_NotifyWakeupISREvent(hDev, 0)`**; CMDQ tag slot as in the ISP.
- **ResetDevPass**, and `g_bUseHwReset`: `MHalVpeSclReset`.
- On its input port SCL sets the sideband type-4 enqueue message `{1, 0x80000000}` when the first realtime task is enqueued
  (`_MI_SCL_IMPL_SetEnqueueMsg`), `{0, 0x80000000}` at Start / StopChannel; no module reads it [R: search of the module
  decompiles, `mi_sys-1-engine.md` §5.1; I: none exists].
- The field meanings of the mhal blocks mi_scl fills — the 0x478-byte task descriptor, the DMA / size / crop config blocks
  (offsets in `scl_frame_worker`) and the attach config words (`0x02D00500`, role cfg[3]) — are not known from mi_scl; the mhal side
  is `mhal-scl-jpe-cmdq.md` §3.

### 6.6 Sideband messages SCL reads
`OnPreProcessInputTask` reads the zoom window type 1 on its input buffer (§4.8: the window becomes the channel's input crop, the
format its input format, then the word is cleared) and, per enabled output port, the consumer's message with
`mi_sys_GetOutputPortSidebandMsg`: type 3 PREFER_ALIGN from VENC rounds the port's output size up to the requested alignment and
sets the port's ext-conf; type 2 "deliver tiled frames" from DISP is honoured only on a port mapped to **HWSCL5** with NV12 output
(ext-conf `{0x80, 0x40, 0x40, 0}`, ACK stamped in `OnPrepareOutputTaskMetaInfo` as hi word 0x50000000); SCL does not rotate. The
values of the VENC alignment table behind type 3 are not resolved. The full rules: `mi_sys-1-engine.md` §5.3. Dev 0 maps only HWSCL0..2, so a rotating display layer fed by it gets no ACK
(`mi_sys-1-engine.md` §5.5).

### 6.7 Rotation and instance type 4
`SetChnParam {dev; chn; MI_SCL_ChnParam_t{eRot}}`: the channel must be stopped. `eRot` 0 → stored at channel +0x10, the
instance stays the device's base type. `eRot` 1..3 (90 / 180 / 270°) is accepted **only on dev 3** and only for input pixel formats
that can be rotated (a pixel-format test against 0xc / 0xf / 0x10: NV12 / NV21 / YUV422SP and a few more); it switches the instance to
**type 4** by detach / re-attach (`MHalVpeSclInstanceDetach` / `MHalVpeSclInstanceAttach` with the type-4 role config) at channel
start. Any other dev with eRot ≠ 0, or an unrotatable format, returns `0xA0222007` NOT_SUPPORT. The app's channel uses eRot 0.

### 6.8 StretchBuf and the `mi_scl_*` exports
- **ioctl 0xe StretchBuf `{MI_SCL_DirectBuf_t src; dst; crop}` (0x70 bytes)**: a memory-to-memory scale on **dev 3** through
  `mi_sys_DoDirectBufTask`. Allocates a 0xC78-byte direct-buffer task, validates src / dst (non-zero w, h and stride; pixel in the
  SCL set), computes the stride alignment per pixel (0x20 for YUV422 planar 0x11 / 0x12, else 0x10 / 0x8 / 0x4), fills the task
  and submits it. Returns `0xA0222003` on a bad rect, `0xA0222000` when dev 3 is not ready. prog_uvc never calls it.
- **`mi_scl_StretchBufByVdec`, `mi_scl_CreateDevice`, `mi_scl_DestroyDevice`, `mi_scl_Init`, `mi_scl_DeInit`** — thin wrappers
  over the IMPL functions, **imported by `mi_vdec.ko`** to scale decoded frames (the MP4 player): mi_vdec calls `mi_scl_Init` /
  `mi_scl_CreateDevice` to stand up dev 3 and `mi_scl_StretchBufByVdec(src, dstWin, dstBuf, extra)` per decoded frame — only
  while output port 1 has a consumer, which the app never sets up (`mi_jpd-mi_vdec.md` §2.4).
  `StretchBufByVdec` differs from `StretchBuf` by taking the source as either a type-1 (FRAME) or a type-4 (rotated) descriptor
  and a 4th argument (the vdec context); it drives dev 3's internal channels 0x20 / 0x21 and needs the type-3 / type-4 scale.

### 6.9 Ports 3–5
A channel has 6 output ports; port *p* maps to HW scaler *p* of the device's `NeedUseHWOutPortMask`. Dev 0 is created with mask
7, so only ports 0..2 have a scaler; EnableOutputPort on ports 3..5 returns `0xA0222002` INVALID_PORTID. On a device created with
a wider mask ports 3..5 behave like ports 0..2 (same SetOutputPortParam / Enable / bind path), mapped to HWSCL3..5.

### 6.10 `DevInitParam` and `EarlyInitDevParam`
- **`MI_SCL_IMPL_DevInitParam`** runs at insmod, per device: CMDQ on, CMDQ size 0x20000, IRQ mode 0.
- **`MI_SCL_IMPL_EarlyInitDevParam(cmd, args)`** is a token parser **with no caller**: it compares `cmd` against `help`,
  `encmdq`, `debuglv`, `irqmode` and writes the per-device CMDQ-enable byte (`0x2605c + dev·0x210`), debug level
  (`_LANCHOR1 + dev·0x84`) or IRQ mode (`0x26064 + dev·0x84`, parsed hex). It is shaped like a boot / insmod parameter hook,
  but nothing on the camera invokes it, so the DevInitParam defaults stand.

### 6.11 Proc commands of `mi_scl<dev>` and `/proc/scl/scl_debug`
16 commands per device, registered in CreateDevice. `/proc/scl/scl_debug` parses words and does nothing (read is a stub), like
the ISP's `isp_debug`.

| command | args | effect |
|---|---|---|
| `debugmutex` | 0 | dump the per-channel lock trace |
| `setprecrop` | dev, chn, x, y, w, h | force the input pre-crop (`_MI_SCL_IMPL_DebugSetPreCop`) |
| `stopchnl` | dev, chn | force-stop a channel |
| `dumptaskfile` | dev, chn, port, path, cnt | queue a task-buffer dump for the dump thread |
| `setatom` | dev | initial output-queue depth |
| `setmirrorflip` | dev, chn, port, mode | force per-port mirror / flip |
| `clearbuf` | dev, chn, port, ? | drop pending output buffers |
| `debuglv` | dev, level | per-device trace bits |
| `enableport` | dev, chn, port | re-run EnableOutputPort |
| `protectport` | dev, chn, port, bOn | toggle MIU protect on output buffers |
| `clearbindq` | dev, chn | flush the bound input queue |
| `fpsth` | dev, chn, FpsTh, FpsFloatTh | fps-warning threshold |
| `ptsth` | dev, chn, ptsMax, ptsMin | PTS window |
| `irqmode` | dev, mode | IRQ vs. poll mode (`_MI_SCL_IMPL_DebugIrqMode`) |
| `pattern` | dev, chn, port, ?, ? | generate an output test pattern (`_MI_SCL_IMPL_DebugGenOutPattern`) |
| `resethw` | dev | `MHalVpeSclReset` |

### 6.12 Imports, and the two mi_rgn hooks
mhal: `MHalVpeSclInit`, `MHalVpeSclDeInit`, `MHalVpeCreateSclInstance`, `MHalVpeDestroySclInstance`, `MHalVpeSclInstanceAttach`,
`MHalVpeSclInstanceDetach`, `MHalVpeSclGetIrqNum`, `MHalVpeSclEnableIrq`, `MHalVpeSclCheckIrq`, `MHalVpeSclClearIrq`,
`MHalVpeSclPreCropConfig`, `MHalVpeSclDmaPortConfig`, `MHalVpeSclPortSizeConfig`, `MHalVpeSclPortMirrorFlipConfig`,
`MHalVpeSclRotationConfig`, `MHalVpeSclSendSwMessage`, `MHalVpeSclProcess`, `MHalVpeSclGetActiveBuf`, `MHalVpeSclSetWaitDone`,
`MHalVpeSclReset`; imported but referenced only by debug code: `MHalVpeSclGetQpHistogram` [I]. mi_sys: as the ISP (§4.13)
plus `DoDirectBufTask` (StretchBuf) and `SetChnOutputPortDepth` (dev 3). CamOs: as the ISP plus `CamOsMemCache*`.

mi_rgn: exactly two functions, `mi_rgn_Process(cmdInfo*, cmdq)` and `mi_rgn_NotifyFrameDone(chnPort*)`. Both act only once some
process has initialised RGN through the `MI_RGN_*` API (`MI_RGN_Init`), which nothing on the camera does (prog_uvc has
no RGN call, kjd_lvgl no MI call, no `.ko` imports `mi_rgn_Init`); until then they return 0. Their behaviour: `rgn-gop.md` §3.5.

### 6.13 libmi_scl.so [R]
<!-- ev:mi-capture.6.13 -->
15 exports; no argument checks in user space (per-function check). The app's calls are rows of
the table of `mi-user-abi.md` §10; the other 4:

| function | nr (size) | payload / copy-back |
|---|---|---|
| `MI_SCL_SetInputPortCrop(dev, chn, rect*)` / `GetInputPortCrop` | 4 / 5 (0x10) | {dev, chn, rect 8}; Get copies the rect **always** (also after −1) |
| `MI_SCL_GetChnParam(dev, chn, MI_SCL_ChnParam_t *p)` | 7 (0xc) | {dev, chn}; `*p = +8` always |
| `MI_SCL_StretchBuf(MI_SCL_DirectBuf_t *src, MI_SYS_WindowRect_t *srcCrop, MI_SCL_DirectBuf_t *dst, u32 filter)` | 0xe (0x70) | +0 src (0x30), +0x30 crop (8), +0x38 dst (0x30), +0x68 filter, +0x6c = 0 |

