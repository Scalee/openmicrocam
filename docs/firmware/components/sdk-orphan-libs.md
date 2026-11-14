# sdk-orphan-libs — the unused SDK libraries of `/config/lib`
<!-- ev:sdk-orphan-libs -->

Nine user-space libraries of `/config/lib` that nothing on this camera loads (§1.2): six SigmaStar MI libraries and three
stock codec / parser libraries.

- **libmi_divp**, **libmi_gyro**, **libmi_vpe** — thin ioctl wrappers of an older (2020-11) generation for `/dev/mi_divp`
  (DIVP scaling / deinterlace / TNR channel), `/dev/mi_gyro` (gyro / accelerometer) and `/dev/mi_vpe` (the i6 / i6e
  ISP + scaler channel API). No kernel module on the camera provides any of the three devices.
- **libmi_sed** — SED, "smart encode detector": user-space glue that watches one video port, finds moving regions (through
  libmi_vdf's MD or through IVE SAD + histogram) and pushes them into H.264 / H.265 encoder channels as ROI QP rectangles
  and smart-detect motion statistics. No device of its own.
- **libmi_vdf** — VDF, "video detection framework": a user-space MI module that registers itself with mi_shadow as module
  VDF (id 1), takes frames on its input ports through mi_sys binds, runs the MD / OD / VG algorithm libraries on them and
  hands the results back as mi_sys output buffers.
- **libmi_wlan** — Wi-Fi control: every operation is a shell command line composed from a JSON description and run with
  `system()` / `popen()`, the tools' output parsed back. No kernel module, no device.
- **libcjson** (cJSON 1.7.7), **libg711** (Sun `g711.c` + three wrappers), **libg726** (Sun G.72x reference coders + a
  bit-packing wrapper) — stock code.

Other documents: the user-side ioctl wrapper of the later library generation, its fd helper, payload packing, return
convention, MI error-code format and module ids — `mi-user-abi.md` §2 "The device fd", §3 "The ioctl call", §4 "Return
values and error codes"; the list of unused libraries in `/config/lib` — `mi-user-abi.md` §1.3 "The other libraries in
`/config/lib` (unused)"; the inventory rows — `firmware-image.md` §3.5 "`/config/lib` (SDK user libraries)"; mi_shadow
(kernel module and `libmi_shadow.so`, the registration parameters VDF fills, the event numbers, the "not ready" code) —
`mi_vdisp-mi_shadow.md` §7 "mi_shadow — kernel internals", §8 "libmi_shadow.so — user API and event thread"; the MD / OD /
VG algorithm libraries libMD, libOD, libVG and libmi_ive — `ive-video-analytics.md`; the mhal DIVP HAL a `mi_divp.ko`
would drive — `mhal-core.md` §8 "DIVP block"; libmi_isp's `MI_ISP_EnableUserspace3A` and its 0x18-byte ISP API header —
`mi-capture.md` §5.2 "Dispatch and routing", §5.4 "Callbacks the 3A and IQ-server libraries import"; mi_venc's
`SetSmartDetInfo` (ioctl 0x50) — `mi_venc.md` §6.4 "Per-codec parameters — direct, no state change"; the DT node
`i2c4/gyro@d0` — `kernel-platform.md` §8 "I2C", §14 "Device tree".

**Tags.** [R] read from the binary, a decompile, disassembly or a file (of the dump or a downloaded source tree); [S]
stated by a source; [I] inference. A heading tagged [R] covers every statement under it unless marked otherwise.

**Binaries** [R]: `/config/lib/<lib>.so` (= `/config/lib` on the camera). Comparison material: the i6e kernel tree's
`drivers/sstar/gyro` , the `MI_ModuleId_e` enum , copies of the same libraries in other firmware dumps.

**Addresses.** libmi_divp / libmi_gyro / libmi_vpe: file offsets. libmi_wlan: Ghidra addresses (file offset + 0x10000),
code and data alike. libmi_vdf: code addresses are Ghidra's (vaddr + 0x10000), data addresses are ELF vaddrs. libmi_sed,
libcjson, libg711, libg726: as the decompiles show them.

Struct and field names the binaries do not carry (`MI_DIVP_ChnAttr_t`, `MI_VPE_ChannelAttr_t`, `MI_SED_ChnAttr_t` and
their fields) follow the public SigmaStar i6 / i6e SDK headers as remembered — no public header for DIVP / VPE / VDF / SED /
WLAN / GYRO was found — and count as [I]; their sizes and offsets are [R].

## 1. The libraries on this camera [R]

### 1.1 Identity

| library | size | .text | exports (funcs) | NEEDED | build stamp (`sstar_mi_so_version_string` or own) | same file elsewhere |
|---|---|---|---|---|---|---|
| libmi_divp | 9 664 | 0x948 | 13 | libc | `mi_divp … project_commit.aa6e4b0 sdk_commit.33b0b04 build_time.20201124143518` | anjoy copy differs (same size) |
| libmi_gyro | 9 664 | 0xa30 | 14 | libc | `mi_gyro … aa6e4b0 / 33b0b04 build_time.20201124143508` | anjoy copy differs (same size) |
| libmi_vpe | 13 776 | 0x1a78 | 40 | libc (+ `MI_ISP_*Userspace3A` undefined) | `mi_vpe … aa6e4b0 / 33b0b04 build_time.20201124143522` | anjoy copy differs (same size) |
| libmi_sed | 22 180 | 0x2848 | 14 | libc (+ CamOs, MI_SYS, MI_VDF, MI_VENC, MI_IVE undefined) | `mi_sed … project_commit.7251436 sdk_commit.80327e1 build_time.20220721223310` | anjoy copy differs (same size) |
| libmi_vdf | 67 464 | 0x9d50 | 32 + 6 data | libc (+ CamOs, MI_SYS, MI_SHADOW undefined; MI_MD/OD/VG weak) | `mi_vdf … 7251436 / 80327e1 build_time.20220721223310`; `__DATE__/__TIME__` "Jul 21 2022" "22:33:10" | anjoy (2021-11-02): same code; siyi (2021-11-26), runcam (2022-06-06): different code |
| libmi_wlan | 39 112 | 0x5e60 | 28 + 5 data | libc (+ cJSON undefined) | `mi_wlan … 7251436 / 80327e1 build_time.20220721223310` | — |
| libcjson | 26 048 | 0x374c | 74 | libc, ld-linux-armhf; SONAME `libcjson.so` | none (version from code, §9.1) | — |
| libg711 | 5 648 | 0x568 | 3 + 2 tables | libc | none | **identical** in siyi, runcam, anjoy |
| libg726 | 9 512 | 0x159c | 3 | libc | none | **identical** in anjoy |

- "Same code" for the anjoy copies of divp, gyro, vpe, sed, vdf: `cmp` differs only in `.note.gnu.build-id` (file 0x104 /
  0x124, 20 bytes), the version string and (vdf) the `__DATE__` / `__TIME__` strings. So the same SDK sources were rebuilt
  unchanged between 2020-11-10 and 2020-11-24 (divp, gyro, vpe) and between 2021-11-02 and 2022-07-21 (sed, vdf) [I].
- The 2020-11 group (divp, gyro, vpe) is an **older generation** of the MI wrapper than the libraries `mi-user-abi.md` §1.1
  describes (§2) [R imports].

### 1.2 No user, no provider
<!-- ev:sdk-orphan-libs.1.2 -->
- None of the nine appears as NEEDED in any ELF of the dump, and their names occur in no other file of the dump (`grep -a`
 over and the kernel `Image`). The only mention is mi_sys.ko's module-name table (`mi_ive, mi_vdf,
  mi_venc, mi_rgn, mi_ai, mi_ao, mi_vif, mi_divp, mi_vpe, …, mi_wlan, mi_ipu, mi_mipitx, mi_gyro, mi_jpd, mi_wbc`), which is
  the `MI_ModuleId_e` order: IVE 0, VDF 1, VENC 2, RGN 3, AI 4, AO 5, VIF 6, **VPE 7**, VDEC 8, SYS 9, FB 10, HDMI 11,
  **DIVP 12**, GFX 13, VDISP 14, DISP 15, OS 16, IAE 17, MD 18, OD 19, SHADOW 20, WARP 21, UAC 22, LDC 23, SD 24, PANEL 25,
  CIPHER 26, SNR 27, **WLAN 28**, IPU 29, MIPITX 30, **GYRO 31**, JPD 32, ISP 33, SCL 34, WBC 35. SED has no id (commented
  out in the header).
- The libraries are not self-contained: libmi_sed and libmi_vdf use CamOs / MI_SYS / MI_SHADOW / MI_VENC / MI_IVE symbols
  without a NEEDED entry for the libraries that define them — libmi_sed needs libmi_ive and libmi_vdf, libmi_vdf needs
  libmi_shadow and the MD / OD / VG algorithm libraries; libmi_wlan imports cJSON (libcjson, §9.1) with no NEEDED entry.
  They load only into a process that already links those libraries [R dynamic section; I consequence].
- The three 2020 wrappers are on the camera (e.g. `/config/lib/libmi_divp.so`), but no `/dev/mi_divp`, `/dev/mi_vpe`,
  `/dev/mi_gyro` provider exists: there is no mi_divp / mi_vpe / mi_gyro.ko, and the kernel has no gyro driver (no `gyro`
 symbol in) although the DT has an enabled `i2c4/gyro@d0` node (`compatible = "sstar,gyro"`, `reg = <0x69>`, `gyro_int` / `gyro_fsync` = 0xffff). On the camera every
  call of these three libraries therefore ends in "failed to open" and an ioctl on fd −1 (§2.1) [I from the code path].
- The camera has no Wi-Fi chip and no JSON description for libmi_wlan.

## 2. The 2020 wrapper generation (divp, gyro, vpe) — shared mechanics [R]

### 2.1 Device fd
Each of the three libraries has a private `{int fd; pid_t pid}` pair in `.data`, both initialised to −1 (file 0x20b8 in
divp / gyro, 0x30c8 in vpe). One unnamed open helper runs before every ioctl:
1. if `fd == -1` **or** `pid != getpid()`: `fd = open("/dev/mi_<m>", O_RDWR)`; `pid = getpid()`;
   `fcntl(fd, F_SETFD, fcntl(fd, F_GETFD) | FD_CLOEXEC)` — done even when the open failed. An fd inherited over `fork()` is
   not closed: the child opens a second one.
2. if `fd < 0`: print `[<m>] failed to open /dev/mi/<m>!(<strerror>)` (the printed name is `/dev/mi/<m>`, the opened one
   `/dev/mi_<m>`, the same quirk as the later generation, `mi-user-abi.md` §2.1).
3. return fd. A failed open stores −1, so the next call tries again.

`<m>` = `divp`, `gyro`, `vpe`. Unlike the later generation, the libraries prog_uvc uses (`mi-user-abi.md` §2.1): no lock
(no `CamOsTsem` `g_mi_fd_semalock`), no `g_mi_fd_table`, no `F_DUPFD` renumbering.

### 2.2 ioctl argument and result
- The ioctl argument is the 16-byte wrapper `{u32 len; u32 pad (uninitialised); u64 ptr (sign-extended)}` of
  `mi-user-abi.md` §3.1; `cmd = _IOC(dir, 'i', nr, len)`; payload = arguments packed in order (`mi-user-abi.md` §3.2),
  output copied back as stated per row.
- The function returns the `ioctl()` result unchanged; `−1` additionally prints `[<m>] failed to ioctl 0x%08x!(<strerror>)`.
  The local "default" value (0x1F in divp / vpe, 0xA0000000 in gyro) is dead except where a row says otherwise.

### 2.3 InitDev / DeInitDev
- `MI_<M>_InitDev()` (`MI_DIVP_InitDev`, `MI_Gyro_InitDev`, `MI_VPE_InitDev`): runs the open helper; returns 0, or **0x1F**
  (bare number, not an MI code) when fd < 0. No ioctl.
- `MI_<M>_DeInitDev()` (`MI_DIVP_DeInitDev`, `MI_Gyro_DeInitDev`, `MI_VPE_DeInitDev`): if `fd < 0` or `pid != getpid()` →
  print `[<m>] failed to close /dev/mi/<m>, fd is Invalid !`, return −1. Else `r = close(fd)`; `r == 0` → `fd = −1` (pid
  left as is); `r < 0` → print `failed to close … !(<strerror>)`. Returns `r`. No reference count: any DeInitDev closes the
  fd for every user in the process.
- The kernel side would see "release" only here or at process exit [I].

## 3. libmi_divp.so — DIVP channel (scaling / deinterlace / TNR), 13 functions [R unless tagged]
<!-- ev:sdk-orphan-libs.3 -->

Device `/dev/mi_divp` (no provider on the camera, §1.2). Error module id 12 (`0xA00C…`) would come from the kernel module;
the library makes no error code itself. `MI_DIVP_CHN` is a u32 (one 4-byte slot).

| function (args) | nr | dir, payload | payload layout | copy-back |
|---|---|---|---|---|
| `MI_DIVP_CreateChn(chn, const ChnAttr *a)` | 0 | W 0x24 | +0 chn, +4 `ChnAttr` (0x20) | — |
| `MI_DIVP_DestroyChn(chn)` | 1 | W 4 | chn | — |
| `MI_DIVP_SetChnAttr(chn, const ChnAttr *a)` | 2 | W 0x24 | as nr 0 | — |
| `MI_DIVP_GetChnAttr(chn, ChnAttr *a)` | 3 | WR 0x24 | +0 chn, rest zero | `a` ← +4 (0x20) if ret == 0 |
| `MI_DIVP_StartChn(chn)` | 4 | W 4 | chn | — |
| `MI_DIVP_StopChn(chn)` | 5 | W 4 | chn | — |
| `MI_DIVP_SetOutputPortAttr(chn, const OutputPortAttr *a)` | 6 | W 0x14 | +0 chn, +4 `OutputPortAttr` (0x10) | — |
| `MI_DIVP_GetOutputPortAttr(chn, OutputPortAttr *a)` | 7 | WR 0x14 | +0 chn | `a` ← +4 (0x10) if ret == 0 |
| `MI_DIVP_RefreshChn(chn)` | 8 | W 4 | chn | — |
| `MI_DIVP_StretchBuf(const DirectBuf *src, const WindowRect *crop, const DirectBuf *dst)` | 9 | W 0x6c | +0 src (0x30), +0x30 dst (0x30), +0x60 crop (8; zero when `crop == NULL`), +0x68 u32 = 0 | — |
| `MI_DIVP_StretchBufEx(src, crop, dst, u32 x)` | 9 | W 0x6c | as StretchBuf, +0x68 = `x` | — |
| `MI_DIVP_InitDev()`, `MI_DIVP_DeInitDev()` | — | — | §2.3 | — |

- StretchBuf / StretchBufEx return **0x1F** without an ioctl when `src` or `dst` is NULL; `crop` may be NULL. Both use the
  same ioctl; the kernel tells them apart only by the +0x68 word. What +0x68 selects is not visible in the library and is
  not known: it would take a kernel `mi_divp.ko`, which is neither on the camera nor in any downloaded tree [R: searched].
- Struct layouts (sizes and offsets [R]; field names [I], public i6 / i6e `mi_divp_datatype.h` as remembered):

| struct (size) | layout |
|---|---|
| `MI_DIVP_ChnAttr_t` (0x20) | +0 u32 MaxWidth, +4 u32 MaxHeight, +8 enum TnrLevel, +0xc enum DiType, +0x10 enum RotateType (`MI_SYS_Rotate_e`), +0x14 `MI_SYS_WindowRect_t` CropRect {u16 x, y, w, h}, +0x1c u8 bHorMirror, +0x1d u8 bVerMirror, +0x1e pad |
| `MI_DIVP_OutputPortAttr_t` (0x10) | +0 enum CompMode (`MI_SYS_CompressMode_e`), +4 enum PixelFormat (`MI_SYS_PixelFormat_e`), +8 u32 Width, +0xc u32 Height |
| `MI_DIVP_DirectBuf_t` (0x30) | +0 enum PixelFormat, +4 u32 Width, +8 u32 Height, +0xc u32 Stride[3], +0x18 u64 PhyAddr[3] |
| `MI_SYS_WindowRect_t` (8) | u16 x, y, width, height |

## 4. libmi_gyro.so — gyro / accelerometer sensor, 14 functions [R unless tagged]

Device `/dev/mi_gyro`; error module id 31 (GYRO). Every function's first argument is a u8 device index, sent as payload
byte +0. The payload is `memset` to zero first and **outputs are copied back unconditionally** (also after a failed ioctl,
when they read as 0 unless the kernel wrote part of the buffer). Returns the ioctl result (§2.2).

| function (args) | nr | dir, size | payload (+offset) | outputs |
|---|---|---|---|---|
| `GyroSensor_SetSampleRateDiv(u8 dev, u8 div)` | 0 | W 2 | +0 dev, +1 div | — |
| `GyroSensor_SetGyroRange(u8 dev, u32 range)` | 1 | W 8 | +0 dev, +4 range | — |
| `GyroSensor_GetGyroRange(u8 dev, u32 *range)` | 2 | WR 8 | +0 dev | `*range` ← +4 |
| `GyroSensor_GetGyroSensitivity(u8 dev, u16 *num, u16 *den)` | 3 | WR 6 | +0 dev | `*num` ← +2, `*den` ← +4 |
| `GyroSensor_ReadGyro_XYZ(u8 dev, s16 *x, s16 *y, s16 *z)` | 4 | WR 8 | +0 dev | +2, +4, +6 |
| `GyroSensor_SetAccelRange(u8 dev, u32 range)` | 5 | W 8 | +0 dev, +4 range | — |
| `GyroSensor_GetAccelRange(u8 dev, u32 *range)` | 6 | WR 8 | +0 dev | `*range` ← +4 |
| `GyroSensor_GetAccelSensitivity(u8 dev, u16 *num, u16 *den)` | 7 | WR 6 | +0 dev | +2, +4 |
| `GyroSensor_ReadAccel_XYZ(u8 dev, s16 *x, s16 *y, s16 *z)` | 8 | WR 8 | +0 dev | +2, +4, +6 |
| `GyroSensor_ReadTemp(u8 dev, s16 *t)` | 9 | WR 4 | +0 dev | `*t` ← +2 |
| `GyroSensor_EnableDev(u8 dev, u8 a, u8 b)` | 0xa | W 3 | +0 dev, +1 a, +2 b | — |
| `GyroSensor_ReadFifoData(u8 dev, u16 *cnt, void *buf)` | 0xb | WR 0x204 | +0 dev | `*cnt` ← +2 (u16), `buf` ← +4 (0x200 bytes, always all) |
| `MI_Gyro_InitDev()`, `MI_Gyro_DeInitDev()` | — | — | §2.3 | — |

- Signedness and the meaning of `a` / `b` come from the kernel gyro core of the i6e / i6c trees, which has the same twelve
  operations [R source]: sensitivity = `{u16 num; u16 den}`, xyz = three `short`, temp = `short`, dev mode = `{char
  fifo_mode (0/1); u8 fifo_type}` with `fifo_type` bits ZA 0x01, YA 0x02, XA 0x04, ZG 0x10, YG 0x20, XG 0x40, TEMP 0x80 — so
  `EnableDev(dev, fifo_mode, fifo_type)` [I: by position]. The FIFO buffer is raw sensor FIFO bytes; its layout is described
  by the kernel's `gyro_arg_fifo_info` (start / end byte per axis, bytes per sample, endianness, max count), which this
  library never asks for [R: no such ioctl].
- **The library does not talk to that kernel driver**: the i6e / i6c `drivers/sstar/gyro` char device takes the struct
  directly (no 16-byte wrapper, no device byte) and numbers its commands 0 SetSampleRateDiv, 1 SetGyroRange,
  2 SetAccelRange, 3 GetGyroRange, 4 GetGyroSensitivity, 5 GetAccelRange, 6 GetAccelSensitivity, 7 ReadFifoData,
  8 ReadGyroXYZ, 9 ReadAccelXYZ, 0xa ReadTemp, 0xb SetDevMode — a different order from the table above. libmi_gyro expects
  an MI-layer `mi_gyro` module (MI wrapper ABI, module id 31) in front of the sensor core [I]; neither exists on the camera
  (§1.2).

## 5. libmi_vpe.so — VPE (the i6 / i6e ISP + scaler channel API), 40 functions [R unless tagged]

Device `/dev/mi_vpe`; error module id 7 (VPE). The dead default value `0xA0078012` carries module 7, level field 8,
err 0x12 (read as `MI_DEF_ERR(VPE, ERROR, 0x12 BUSY)` [I]; what level 8 means is not settled, `mi-user-abi.md` §4.2). VPE is
the API of the older chips in which one "VPE" channel = ISP + scaler; on M6 the same job is split into ISP and SCL
(mi_isp, mi_scl) [I]. Arguments `chn`, `port` are u32 slots. Rows without a note are plain wrappers (§2.2): payload =
`{chn[, port], struct}`, copy-back only when ret == 0.

### 5.1 ioctl table

| function (args) | nr | dir, size | payload | copy-back |
|---|---|---|---|---|
| `MI_VPE_CreateChannel(chn, const ChannelAttr *a)` | 0 | W 0x7c | re-packed, §5.2; then calls `MI_ISP_EnableUserspace3A` (§5.3) | — |
| `MI_VPE_DestroyChannel(chn)` | 1 | W 4 | chn; **before** the ioctl calls `MI_ISP_DisableUserspace3A(chn)` | — |
| `MI_VPE_GetChannelAttr(chn, ChannelAttr *a)` | 2 | WR 0x70 | +0 chn, +4 `*a` (0x6c, sent in as well) | `a` ← +4 (0x6c) |
| `MI_VPE_SetChannelAttr(chn, const ChannelAttr *a)` | 3 | W 0x70 | +0 chn, +4 `*a` (0x6c, **not** re-packed) | — |
| `MI_VPE_StartChannel(chn)` / `StopChannel(chn)` | 4 / 5 | W 4 | chn | — |
| `MI_VPE_SetChannelParam(chn, const ChannelPara *p)` | 6 | W 0x20 | +0 chn, +4 `*p` (0x1c) | — |
| `MI_VPE_GetChannelParam(chn, ChannelPara *p)` | 7 | WR 0x20 | +0 chn, +4 `*p` (sent in) | `p` ← +4 (0x1c) |
| `MI_VPE_SetChannelCrop(chn, const WindowRect *r)` | 8 | W 0xc | +0 chn, +4 rect (8) | — |
| `MI_VPE_GetChannelCrop(chn, WindowRect *r)` | 9 | WR 0xc | +0 chn, +4 rect (sent in) | `r` ← +4 (8) |
| `MI_VPE_GetChannelRegionLuma(chn, const RegionInfo *ri, u32 *luma, s32 timeout)` | 0xa | WR 0x54 | +0 chn, +4 `ri->pRects[0..n)` (n·8 bytes), +0x34 n = `ri->u32RegionNum`, +0x38 outputs, +0x50 timeout | `luma` ← +0x38 (n·4 bytes) |
| `MI_VPE_SetChannelRotation(chn, u32 rot)` | 0xb | W 8 | chn, rot (`MI_SYS_Rotate_e`) | — |
| `MI_VPE_GetChannelRotation(chn, u32 *rot)` | 0xc | WR 8 | chn | `*rot` ← +4 |
| `MI_VPE_EnablePort(chn, port)` / `DisablePort` | 0xd / 0xe | W 8 | chn, port | — |
| `MI_VPE_SetPortMode(chn, port, const PortMode *m)` | 0xf | W 0x18 | chn, port, `*m` (0x10) | — |
| `MI_VPE_GetPortMode(chn, port, PortMode *m)` | 0x10 | WR 0x18 | chn, port | `m` ← +8 (0x10) |
| `MI_VPE_SetPortCrop(chn, port, const WindowRect *r)` | 0x11 | W 0x10 | chn, port, rect | — |
| `MI_VPE_GetPortCrop(chn, port, WindowRect *r)` | 0x12 | WR 0x10 | chn, port | `r` ← +8 |
| `MI_VPE_SetPortShowPosition(chn, port, const WindowRect *r)` | 0x13 | W 0x10 | chn, port, rect | — |
| `MI_VPE_GetPortShowPosition(chn, port, WindowRect *r)` | 0x14 | WR 0x10 | chn, port | `r` ← +8 |
| `MI_VPE_SetIspApiData(const u32 hdr[5], void *data)` | 0x15 | W 0x18 | +0 hdr[0..4] (0x14), +0x14 `data` (user pointer) | — |
| `MI_VPE_GetIspApiData(const u32 hdr[5], void *data)` | 0x16 | **W** 0x18 | as 0x15 | none: the kernel writes through `data` |
| `MI_VPE_SkipFrame(chn, u32 n)` | 0x17 | W 8 | chn, n | — |
| `MI_VPE_Alloc_IspDataBuf(u32 size, void **va)` | 0x18 | WR 8 | +0 size | `*va` ← +4 **always** (0 on failure) |
| `MI_VPE_Free_IspDataBuf(void *va)` | 0x19 | W 4 | va | — |
| `MI_VPE_LDCBegViewConfig(chn)` / `LDCEndViewConfig(chn)` | 0x1a / 0x1b | W 4 | chn | — |
| `MI_VPE_LDCSetViewConfig(chn, const void *cfg, u32 size)` | 0x1c | W 0xc | chn, va, size — §5.4 | — |
| `MI_VPE_LoadPortZoomTable(chn, port, const ZoomTable *t)` | 0x1f | W 0x10 | chn, port, t[0] (u32 count), t[1] (u32 user pointer to the entries) | — |
| `MI_VPE_StartPortZoom(chn, port, const ZoomAttr *z)` | 0x20 | W 0x14 | chn, port, `*z` (0xc) | — |
| `MI_VPE_StopPortZoom(chn, port)` | 0x21 | W 8 | chn, port | — |
| `MI_VPE_GetPortCurZoomAttr(chn, port, ZoomAttr *z)` | 0x22 | WR 0x14 | chn, port | `z` ← +8 (0xc) **always** |
| `MI_VPE_EnableInputPort(chn, port)` / `DisableInputPort` | 0x23 / 0x24 | W 8 | chn, port | — |
| `MI_VPE_CreateMultiChannelStitch(chn, const StitchAttr *a)` | 0 | W 0x7c | re-packed, §5.2 | — |
| `MI_VPE_CallBackTask_Register(...)`, `_Unregister(...)` | — | — | **stubs: return 0**, read no argument | — |
| `MI_VPE_InitDev()`, `MI_VPE_DeInitDev()` | — | — | §2.3 | — |

Unused nr in the table: 0x1d, 0x1e (no function sends them). `GetChannelRegionLuma` copies `n·8` bytes into a 0x30-byte
area with no bound check: n > 6 overruns the stack frame; n ≤ 6 is the implied limit [I].

### 5.2 Channel create payload (nr 0, 0x7c) and the two attr structs
`MI_VPE_ChannelAttr_t` (caller side, 0x6c) and the stitch attr (`MI_VPE_MultiChannelStitchAttr_t`-like, 0x70) are both
re-packed into one kernel layout [R offsets; names I, from the i6e `mi_vpe_datatype.h` as remembered]:

| kernel +off | type | from ChannelAttr (CreateChannel) | from stitch attr (CreateMultiChannelStitch) |
|---|---|---|---|
| 0x00 | u32 | chn | chn |
| 0x04 | u16 | +0x00 MaxW | +0x00 |
| 0x06 | u16 | +0x02 MaxH | +0x02 |
| 0x08 | u32 | +0x04 PixFmt | +0x04 |
| 0x0c | u32 | +0x08 HDRType | +0x08 |
| 0x10..0x14 | 5 × u8 | +0x10..+0x14 bNrEn, bEdgeEn, bEsEn, bContrastEn, bUvInvert | 0 |
| 0x18 | u32 | +0x18 RunningMode | +0x10 |
| 0x1c | 0x48 bytes | +0x1c IspInitPara (`{u32 Revision; u32 Size; u8 Data[64]}` [I]) | +0x14 |
| 0x64 | u8 | +0x64 bEnLdc | +0x5c |
| 0x68 | u32 | +0x68 ChnPortMode | +0x60 |
| 0x6c | u32 | **constant 1** | +0x64 |
| 0x70 | u32 | +0x0c SensorBindId | +0x0c |
| 0x74 | u32 | 0 | +0x68 |
| 0x78 | u8 | 0 | +0x6c |

So a plain channel is a stitch of one: +0x6c is the number of stitched inputs (1) and +0x74 / +0x78 the stitch parameters
[I]. `SetChannelAttr` / `GetChannelAttr` (nr 3 / 2) send the 0x6c caller struct unchanged at +4 — a kernel must accept the
two layouts on different nr. The caller byte +0x15 (and +0x16 / +0x17, padding [I]) is sent by Set / Get but dropped by
Create.

### 5.3 User-space 3A hook — ABI mismatch with the camera's libmi_isp
After the create ioctl (whatever its result) `MI_VPE_CreateChannel` calls `MI_ISP_EnableUserspace3A(chn, &pair)` with
`pair = {attr+0x04 PixFmt, attr+0x0c SensorBindId}` on the stack; `DestroyChannel` calls `MI_ISP_DisableUserspace3A(chn)`
with one argument. The camera's libmi_isp defines both as `(dev, chn)` taking two scalars (`CUS3A_Init`,
`CUS3A_EnableUserspaceAE/AWB/AF(dev, chn, on)`, agent registration; `mi-capture.md` §5.2). Linked together, `chn` of VPE
becomes the ISP device and a **stack address** becomes the ISP channel (≥ 16 → the agent registration fails with 6, the
CUS3A calls get an out-of-range channel); Disable passes whatever is in r1. libmi_vpe is therefore built for the i6e-era
libmi_isp, not for this one [I from the two signatures].

### 5.4 LDC view config (nr 0x1c)
`MI_VPE_LDCSetViewConfig(chn, cfg, size)`: `MI_VPE_Alloc_IspDataBuf(size, &va)` (the kernel allocates `size` bytes and
returns a user address), `memcpy(va, cfg, size)`, ioctl 0x1c `{chn, va, size}`, `MI_VPE_Free_IspDataBuf(va)`; returns the
0x1c result. No NULL check: when the alloc fails `va` = 0 and the `memcpy` writes to address 0 — on the camera (no
`/dev/mi_vpe`) this call crashes the process [R code; I consequence]. Typical sequence: `LDCBegViewConfig(chn)`, one or
more `LDCSetViewConfig`, `LDCEndViewConfig(chn)` [I from the names]. The cfg bytes are opaque to the library.

### 5.5 Struct sizes the library fixes [R sizes; I names]
`MI_VPE_ChannelPara_t` 0x1c; `MI_VPE_PortMode_t` 0x10 (`{u16 Width; u16 Height; PixelFormat; CompressMode; …}` [I]);
`MI_SYS_WindowRect_t` 8; `MI_VPE_ZoomAttr_t` 0xc; `MI_VPE_ZoomTable_t` = `{u32 count; ZoomEntry *entries}` (the entry size
is the kernel's business, not visible here); `MI_VPE_RegionInfo_t` = `{WindowRect *pRects; u32 u32RegionNum}`; ISP API
header 5 × u32 (0x14) — one word shorter than mi_isp's 0x18 header (`mi-capture.md` §5.2). The field names inside
`ChannelPara` (0x1c), `PortMode` (0x10) and `ZoomAttr` (0xc) are not known: no mi_vpe header is on hand [R: find].

## 6. libmi_sed.so — SED "smart encode detector" (motion → encoder ROI / smart-detect), 14 functions [R unless tagged]

Pure user-space glue (no device of its own, source file `sed_api.c`). Imports, all unresolved by NEEDED (§1.2): CamOs
mutex / thread / print, `MI_SYS_{Bind,UnBind}ChnPort`, `ChnOutputPort{Get,Put}Buf`, `FlushInvCache`, `MMA_Alloc/Free`,
`Mmap/Munmap`, `MI_VDF_{Init,Uninit,CreateChn,DestroyChn,Run,Stop,EnableSubWindow,GetResult,PutResult}`,
`MI_VENC_{GetChnAttr,SetRoiCfg,SetSmartDetInfo}`, `MI_IVE_{Create,Destroy,Sad,Hist}`; libc pthread / clock / prctl. Two
compiler helpers are linked in: `udiv32` (0x136e8) = unsigned 32-bit divide, `sdiv64` (0x13960) / `sdiv64_core` (0x13a0c) = signed 64-bit
divide.

### 6.1 Error codes
Module id **0x1D** (29: SED's id in the older enum, IPU's in the M6 one — §1.2), level 2: `0xA01D2002` INVALID_CHNID,
`…2003` ILLEGAL_PARAM, `…2004` EXIST, `…2005` UNEXIST, `…2006` NULL_PTR, `0xA01D2013` (−0x5fe2dfed) "not started",
`0xA01D2014` (−0x5fe2dfec) CHN_NOT_STOPED. Borrowed from other modules: `0xA0022002` (VENC INVALID_CHNID) for a channel id
≥ 64 in StartDetector, StopDetector, DestroyChn, Attach and Detach (and for a VENC id > 32); `0xA0002003` / `0xA000200C`
(IVE ILLEGAL_PARAM / NOMEM) from the image helpers. Bare numbers: Attach returns 0x12 when all 8 slots are used, Detach 3
when the VENC channel is not attached. MI_VDF errors pass through; 0x15 from VDF is reported as "maybe someone called
MI_VDF_Uninit".

### 6.2 State
`.bss` 0x15188: **64 channel records of 0xeb0 bytes** (.bss size 0x3ac18); 0x15180 u8 "initialised"; 0x4fd88 u32 debug
level (`MI_SED_SetDbgLevel`; prints at levels 6 and 8 only); 0x4fd8c u8 "VDF usable" (set to 1 by every accepted
CreateChn, cleared when the first-call `MI_VDF_Init` fails; gates the later `MI_VDF_Stop` / `MI_VDF_Uninit` calls). The
first CreateChn after load clears all records and sets every VENC slot to −1. Channel record:

| +off | content |
|---|---|
| 0x00 | `CamOsMutex_t` (record lock) |
| 0x1c | u32 own channel id |
| 0x20 / 0x21 | u8 bCreate / u8 bStart |
| 0x24 | u32 vencChn[8] (−1 = free; values ≤ 32) |
| 0x44 | copy of the 0x38-byte `ChnAttr` (§6.3) |
| 0x7c | u8 number of MD regions in the last VDF result |
| 0x80 | the VDF region list, 12 bytes per region (≤ 255) |
| 0xc80 | u32 rect count reported by `GetRect` |
| 0xc84 | rect[16] `{u32 left, top, width, height}` in input-picture pixels |
| 0xd88 | u32 IVE handle (type 2) |
| 0xd90 | `MI_IVE_Image_t` current frame (0x38, type U8C1, w × h) |
| 0xdc8 | `MI_IVE_MemInfo_t` histogram buffer `{u64 phy; u8 *vir; u32 size = 0x400}` |
| 0xdd8 | `MI_IVE_Image_t` previous frame |
| 0xe14 | u16 sadRangeRatio[16] |
| 0xe38 | CamOs thread handle |
| 0xe58 / 0xe70 | `pthread_mutex_t` / `pthread_cond_t` (CLOCK_MONOTONIC) — the wake-up pair |

### 6.3 `MI_SED_ChnAttr_t` (0x38) [R offsets and checks; names I]

| +off | field | checks / use |
|---|---|---|
| 0x00 | u32 width | ≠ 0, multiple of 16 |
| 0x04 | u32 height | ≠ 0, multiple of 4 |
| 0x08 | u32 src frame rate | ≠ 0 |
| 0x0c | u32 dst frame rate | bind only |
| 0x10 | `MI_SYS_ChnPort_t` input `{eModId, u32 dev, chn, port}` | eModId must be **34 (SCL)** — the message still says "only support … vpe" |
| 0x20 | u32 algorithm | 0 = MD through VDF, 1 = CNN object detect, 2 = IVE SAD motion; ≥ 3 → ILLEGAL_PARAM |
| 0x24 | u32 per-algorithm | type 0: VDF channel (< 16, assert); type 1: `u32MaxVariableBufSize` (must be ≠ 0); type 2: IVE handle |
| 0x28 | u32 | type 0: MD sensitivity (low byte) |
| 0x2c, 0x30 | u32 | copied, never read |
| 0x34 | s32 ROI QP | relative QP of every ROI (`bAbsQp = 0`) |

### 6.4 Functions
- **`MI_SED_CreateChn(chn < 64, attr)`**: NULL → 0xA01D2006; attr checks (§6.3); "VDF usable" = 1; on the first call
  only: clear all records, VENC slots = −1, and if this channel is algorithm 0 `MI_VDF_Init()` (failure printed, "VDF
  usable" = 0). So a first channel of algorithm 2 followed by an algorithm-0 channel never initialises VDF [I
  consequence]. Already created → EXIST. Copies attr, inits cond (monotonic) / mutex / CamOs mutex, bCreate = 1. Then per
  algorithm:
  - 0: `_MI_SED_CreateVDFChn(rec, attr.vdfChn)` (§6.6); failure → record cleared, error returned.
  - 1: prints "Current chip cannot support this type[1]" and **returns 0** with the record created and no thread.
  - 2: `_MI_SED_MotionDetectInit`: `MI_IVE_Create(handle)`; allocate the current and previous images (U8C1, stride =
    width, w × h) and the 0x400-byte histogram buffer (§6.7); any failure frees what was made, destroys the handle, clears
    the record and returns the error (NOMEM 0xA000200C for the histogram).
  - then `CamOsThreadCreate(&rec->thread, NULL, _MI_SED_WorkThread, rec)`, return 0.
- **`MI_SED_StartDetector(chn)`**: not created → UNEXIST; started → CHN_NOT_STOPED. Algorithm 0:
  `MI_SYS_BindChnPort(0, &input, &{VDF=1, dev 0, vdfChn, port 0}, fps, fps)` with `fps = srcFps & (dstFps << 16)` — a
  bitwise AND, so 0 for any sane pair (a bug for `|` [I]); then `MI_VDF_Run()` (on failure, `MI_VDF_Uninit()` if "VDF
  usable"); then `MI_VDF_EnableSubWindow(vdfChn, 0, 0, 1)` — on failure it **unbinds and returns the unbind result** (0 on
  success, so a failed start can report 0). Then bStart = 1 and the work thread is woken (broadcast).
- **`MI_SED_StopDetector(chn)`**: not created → UNEXIST, not started → 0xA01D2013. Algorithm 0: `MI_VDF_Stop()` (if "VDF
  usable"), `MI_VDF_EnableSubWindow(vdfChn, 0, 0, 0)`, unbind; bStart = 0.
- `MI_VDF_Run` and `MI_VDF_Stop` are called with no argument: Run gets r0 = 0 = the bind result, so MD; Stop gets whatever
  r0 holds [R disassembly; I consequence].
- **`MI_SED_DestroyChn(chn)`**: id ≥ 64 → 0xA0022002; not created → UNEXIST. Under the record lock: stop if started;
  bCreate = 0; wake and `CamOsThreadJoin` the thread; algorithm 0 → `MI_VDF_DestroyChn(vdfChn)`; algorithm 2 → free
  histogram, both images, `MI_IVE_Destroy`; destroy cond / mutex / CamOs mutex; clear the record. If no record is left
  created: `MI_VDF_Uninit()` (if "VDF usable"), "initialised" (0x15180) = 0. Returns 0 whenever the channel existed.
- **`MI_SED_AttachToVencChn(chn, vencChn ≤ 32)`**: first free of the 8 slots ← vencChn (no duplicate check); full → 0x12.
  **`MI_SED_DetachFromVencChn(chn, vencChn)`**: first slot equal → −1; none → 3. Neither takes the lock.
- **`MI_SED_GetRect(chn, out)`**: not created → UNEXIST; out (0x104) = `{u32 count = rec+0xc80; rect[16]}`, no lock.
- **`MI_SED_SetDbgLevel(level)`**: stores, prints, returns 0.
- Exported helpers: `_MI_SED_AllocateMemory(size, u64 *phy, void **vir)`, `_MI_SED_FreeMemory(phy, vir, size)`,
  `_MI_SED_GetImageSize(type, stride, height, u32 size[3])`, `_MI_SED_FreeImage(img)`, `_MI_SED_FlushInvCacheImage(img)`,
  `_MI_SED_WorkThread(rec)` (§6.5).

### 6.5 Work thread `_MI_SED_WorkThread(rec)`
Asserts `rec` and `rec->bCreate`; names itself with `prctl(PR_SET_NAME)`: algorithm 0 `SED_IVE_DETCTOR`, 1
`SED_CNN_WorkThread`, 2 `SED_Motion_WorkThread`. Loop while bCreate:
1. while not bStart: wait on the cond for up to **1 s** (monotonic), re-check bStart / bCreate.
2. one detection step — algorithm 0: `_MI_SED_GetVDFResult` (§6.6); 2: `_MI_SED_DoMotionDetect` (§6.7); 1: constant 8
   (never succeeds).
3. step ≠ 0 → wait up to **10 ms** on the cond, loop. Step == 0 → for every attached VENC slot: `_MI_SED_SetVencParam`
   (§6.8) and, for algorithm 2 only, `_MI_SED_SetSmartDetInfo`; then loop at once (no sleep after a successful step).
Exit when bCreate drops (DestroyChn).

### 6.6 Algorithm 0: MD through libmi_vdf
`_MI_SED_CreateVDFChn(rec, vdfChn < 16)` fills a zeroed 0x178-byte `MI_VDF_ChnAttr_t` and calls `MI_VDF_CreateChn` [R
values; meaning I, matched against libmi_vdf's reading of the struct in §7.3]:

| +off | value | meaning [I] |
|---|---|---|
| 0x00 | 0 | work mode MD |
| 0x04 / 0x05 / 0x06 | 1 / 4 / 0 | MD enable, result-buffer count 4, VDF interval 0 |
| 0x18 / 0x1a | u16 8 / u16 2 | CCL control: initial area threshold 8, step 2 |
| 0x1c / 0x1e | u16 width / u16 height | MD static params: picture size |
| 0x20 | u8 1 | colour = Y only |
| 0x24 | u32 width | stride |
| 0x28 / 0x2c | 1 / 1 | macro-block 8×8; SAD output 8-bit |
| 0x30 | u8 4 | ROI point count |
| 0x32…0x41 | (0,0) (w−1,0) (w−1,h−1) (0,h−1) | ROI = whole picture, u16 x/y pairs |
| 0x54 | 1 | MD algorithm mode SAD |
| 0x58 | u8 attr.sensitivity | dynamic: sensitivity |
| 0x5a | u16 2000 | dynamic: learn rate |
| 0x5c | u32 16 | dynamic: MD threshold |

`_MI_SED_GetVDFResult(rec)`: `MI_VDF_GetResult(vdfChn, res[0x160], 0)`; on 0 and `res+0x10` ≠ 0: `p = *(res+0x28)`;
`n = p[0]` (u8) → `rec+0x7c`. n == 0 → the 16 rects are zeroed. Else the n 12-byte regions at `*(p+4)` `{u32 pixelCount;
u16 x0, x1, y0, y1}` are copied to `rec+0x80`, bubble-sorted by pixelCount descending, every region lying inside an
earlier (larger) one is zeroed, and the first 16 non-zero regions become rects `{x0, y0, x1−x0, y1−y0}`, the rest zeroed.
Always `MI_VDF_PutResult(vdfChn, res)`; its error is returned. A GetResult error (no new result) → 10 ms wait.

### 6.7 Algorithm 2: IVE SAD motion statistics (`_MI_SED_DoMotionDetect(chn)`)
Under the record lock; asserts the input module is 34.
1. `MI_SYS_ChnOutputPortGetBuf(&input, &buf, &h)`; on error return it.
2. `bw = w/8`, `bh = h/8`; allocate two U8C1 images bw × bh (SAD and threshold outputs) from MMA heap `mma_heap_name0`
   (`_MI_SED_AllocateMemory`: `MI_SYS_MMA_Alloc(0, "mma_heap_name0", size, &phy)` + `MI_SYS_Mmap(phy, size, &vir, cached)`).
3. copy the Y plane (`w·h` bytes from the buffer's first virtual address) into the current image; flush.
4. `MI_IVE_Sad(handle, cur, prev, sad, thr, {eMode = 8×8, eOutCtrl = 8BIT_SAD, u16Thr 0, min 0, max 0}, 0)`; flush SAD;
   `MI_IVE_Hist(handle, sad, &hist, 0)`; flush the 256 × u32 histogram.
5. 16 bands with bin edges **0, 10, 15, 20, 25, 30, 40, 50, 60, 70, 80, 90, 100, 120, 140, 160, 256** (.rodata 0x4740):
   `ratio[i] = clamp(⌊(Σ hist[edge_i … edge_{i+1}−1]) / (bw·bh) · 256⌋, 0, 256)` (float divide, fixed-point convert with
   8 fraction bits) → `rec+0xe14 + 2i`.
6. free both temporary images; on success copy the current Y plane into the previous image; `PutBuf`; unlock.
On the first call the previous image holds whatever the MMA allocation contained: the library never clears it [R: no
memset; I consequence].

### 6.8 Pushing results into VENC
`_MI_SED_SetVencParam(rec, vencChn)` (vencChn > 32 → assert): `MI_VENC_GetChnAttr(0, vencChn, attr)` (error only
printed); encoder size = attr +0x18 / +0x1c (`u32PicWidth/Height` of the H.264 / H.265 attr; codec type at +0, 2 = H.264,
3 = H.265 — other codecs print "Only H.264/H.265 are supported." and use size 0; RC mode at +0x28). For each of the 16
rects in order, index i:
- width 0 → nothing sent; height 0 → `{i, bEnable 0}`;
- else scaled to the encoder: `left = (x·encW/w) & 0xffc0`, `top = (y·encH/h) & 0xffc0`,
  `width = ((rw·encW/w) & ~1) + 63 & ~63` clipped to `(encW − left) & ~63`, height likewise;
  `MI_VENC_RoiCfg_t = {u32Index i, bEnable 1, bAbsQp 0, s32Qp attr.qp, rect}`;
- `MI_VENC_SetRoiCfg(0, vencChn, &cfg)` unless the RC mode is H264FIXQP (4) or H265FIXQP (10); `rec+0xc80 = i+1`.

`_MI_SED_SetSmartDetInfo` (algorithm 2): `MI_VENC_SetSmartDetInfo(0, vencChn, {eSmartDetType = 1 MD_DET,
u16SadRangeRatio[16] = rec+0xe14, u8ProtectFrmNum = 0})`. mi_venc consumes this as the AVBR still-percent override (ioctl
0x50, `mi_venc.md` §6.4).

## 7. libmi_vdf.so — VDF "video detection framework" (MD / OD / VG host), 32 functions [R unless tagged]

User-space MI module (source `vdf_api.c`) that registers itself with mi_shadow as module **VDF (id 1)**, receives frames on
its input ports through mi_sys binds, runs one of three algorithm libraries on them, and hands results back as mi_sys
output buffers. The algorithms are **not** in this library: `MI_MD_*` (libMD_LINUX), `MI_OD_*` (libOD_LINUX), `MI_VG_*`
(libVG_LINUX) are weak undefined symbols (their APIs: `ive-video-analytics.md` §9.1, §10.1, §11.2). Undefined, no NEEDED: CamOs (rwsem, mutex,
thread, mem, time, print), `MI_SHADOW_*` (14), `MI_SYS_ChnOutputPort{Get,Put}Buf`, `MI_SYS_SetChnOutputPortDepth`.

Error module id 1: `0xA0012003` ILLEGAL_PARAM, `…2012` EXIST ("has created"), `…2013` NOT_START ("eStatus is not
E_MI_VDF_STATUS_START"), `…2015` "not inited" / "has destroyed" / algorithm init failure, `−0x5ffedfea` = `0xA0012016`
"module vdf has inited!" (Init called twice), `…2017` shadow port / channel enable failed, `…2018` shadow disable failed;
GetChnAttr returns a bare −1 when the algorithm's Get call fails (§7.4); SetChnAttr (VG) returns a bare 0x15 when
`MI_VG_Init` fails.

### 7.1 Global state
- **`g_stVdfSrcMgr`** (.bss 0x203e0, 0x1fa0 bytes): +0 u8 bInit; +1 / +2 / +3 u8 work mode MD / OD / VG running
  (`MI_VDF_Run/Stop`); +4…+7 u8 exit flags of the four threads (YUV, MD, OD, VG; set by Uninit); +8 / +9 / +0xa u8 "new
  image" flags of the MD / OD / VG workers (§7.5); +0xc / +0x10 / +0x14 u32 MD / OD / VG library version (filled on the
  first channel of each mode); **+0x18 16 channel records × 0x1d4**; +0x1d58 16 `CamOsMutex_t` × 0x1c (per-channel lock);
  +0x1f18 `pthread_mutex_t`, +0x1f30 `pthread_cond_t` (monotonic) — the "image queued" wake-up; +0x1f60 string
  `"[<__DATE__> <__TIME__>]"`.
- **`g_mutexMoudleVDF`** (0x203bc): `CamOsRwsem` — write-locked by every API call, read-locked by the worker threads.
- **`g_hVDFDev`** (0x2025c): the shadow device handle. **`g_stVDFRegDevInfo`** (0x20270, 0x5c): the registration
  descriptor (§7.2).
- **`g_stVdfProcDebug`** (0x202cc, 0xF0 bytes, zeroed by Init except +0 := 2): +0 u32 debug level; +4 u8 EnCheckPoint;
  +5 u8 EnSelfCheck; +6 u8 DumpAll; +8 u32 dump count[16] (per channel); +0x48 dump path; +0xC8…+0xD8 check-point counters
  `MDTaskChk, ODTaskChk, VGTaskChk, YUVTaskChk, FuncChk`; +0xDC…+0xE8 run counters `MdTaskRun, OdTaskRun, VgTaskRun,
  YuvTaskRun` (names from the dump headers). What the debug level gates: §7.7 `log_level`.
- **Channel record** (offsets from `g_stVdfSrcMgr + chn·0x1d4`; counter names from the `cat` output, §7.8):

| +off | field |
|---|---|
| 0x18 | u32 eStatus: 0 none, 1 created, 2 started, 3 stopped; > 3 asserts |
| 0x1c | algorithm handle (phandle) |
| 0x20 | u8 frame counter (FrameCnt) |
| 0x21 | u8 frame interval (FrameInv) = attr `u8VDFIntvl` |
| 0x24 | u32 Y image size (YImgSize) = stride·height of the last frame |
| 0x28 | InGetCnt: input frames obtained |
| 0x2c | InGetFailCnt: `MI_SHADOW_GetInputPortBuf` failures |
| 0x30 | InDropCnt: every dropped input frame (no address, `u8Enable` ≠ 1, size mismatch, interval skip, output-buffer error other than "not ready") |
| 0x34 | InDoneCnt: input frames finished after processing |
| 0x38 / 0x3c | last count of the input fps window / InFps (float) |
| 0x40 | OutGetCnt: output buffers obtained |
| 0x44 | OutGetFailCnt: output-buffer failures |
| 0x48 | OutDropCnt: results rewound (algorithm failure) |
| 0x4c | OutDoneCnt: results finished |
| 0x50 / 0x54 | last count of the output fps window / OutFps (float) |
| 0x58 / 0x5c / 0x60 / 0x64 | RstGetCnt (GetResult ok) / RstGetFailCnt (GetBuf failures) / RstDropCnt (bad mode) / RstPutCnt (PutResult) |
| 0x68 | image queue `{list head, …, count}`; the count at +0x70 (ImageQCnt) |
| 0x74 | **copy of the caller's `MI_VDF_ChnAttr_t` (0x178)** |

### 7.2 Registration with mi_shadow (`g_stVDFRegDevInfo`) [R: 0x15dac, GOT slots]
`MI_VDF_Init` fills `MI_SHADOW_RegisterDevParams_t` (field meanings [S]: `mi_vdisp-mi_shadow.md` §8.2) and passes it to
`MI_SHADOW_RegisterDev(&desc, &g_hVDFDev)`:

| off | field (mi_shadow) | VDF value |
|---|---|---|
| +0x00 / +0x04 | eModuleId / u32DevId | 1 (VDF) / 0 |
| +0x08 / +0x0C / +0x10 | input ports / output ports / channels | 1 / 1 / 16 |
| +0x14 … +0x20 | OnBind/UnBind Input/Output Port | NULL (mi_shadow's thread answers 0) |
| +0x24 | OnDumpDevAttr (event 5) | **`_MI_VDF_OnDumpVdfAllAttr`** |
| +0x28 / +0x2C / +0x30 | OnDumpChannelAttr / InputPortAttr / OutPortAttr (6 / 7 / 8) | NULL |
| +0x34 | OnHelp (9) | `_MI_VDF_OnHelp` |
| +0x38 | `log_level` (0x0B) | `_MI_VDF_CmdDebugLevel` |
| +0x3C | `sem_check` (0x0C) | `_MI_VDF_CmdCheckPoint` |
| +0x40 | `auto_check` (0x0D) | `_MI_VDF_CmdSelfChecking` |
| +0x44 | `en_workmode` (0x0E) | `_MI_VDF_CmdWorkModeEnable` |
| +0x48 | `en_chn` (0x0F) | `_MI_VDF_CmdChnEnable` |
| +0x4C | `dump_all` (0x10) | `_MI_VDF_CmdDumpAllEnable` |
| +0x50 | `dump_image` (0x11) | `_MI_VDF_CmdDumpImage` |
| +0x54 / +0x58 | bindUsr / dumpUsr | 0 / `pDumpBuf` (CamOsMemAlloc 0x8000) |

The slot order is mi_shadow's event order. The four exported `_MI_VDF_OnDump{Dev,Channel,InputPort,OutPort}Attr` are **not
registered**: they are unused exports that only print "u32DevId=%u" at debug level ≥ 4 and return 0. A `cat` of the proc
node (event 5) runs `_MI_VDF_OnDumpVdfAllAttr`; `echo help` runs `_MI_VDF_OnHelp`. Every callback receives `(handle,
u32DevId, …)`; the command callbacks `(handle, devId, u8 argc, char **argv)` with `argv[0]` = the command word (mi_shadow:
8 strings of ≤ 32 bytes, `mi_vdisp-mi_shadow.md` §8.2).

### 7.3 `MI_VDF_ChnAttr_t` (0x178) — what the library reads and writes [R offsets / checks; names I]
`+0x00 u32 enWorkMode`: 0 MD, 1 OD, 2 VG (anything else: assert in CreateChn, ILLEGAL_PARAM elsewhere). The rest is a union
starting at +4; `u8Enable` +4, `u8BufCnt` +5 (> 8 → 8 with a warning; output port depth = (cnt, cnt+2)), `u8VDFIntvl` +6
(process only every N-th frame: frame counter mod N == 0; 0 = every frame), `u16RstBufSize` +8 (**written by CreateChn**).

| mode | layout used |
|---|---|
| MD | +0xc/+0x10/+0x14 `stSubResultSize {u32 StatusLen, SadLen, ObjLen}` (**written**); +0x18 CCL ctrl `{u16 InitAreaThr, u16 Step}`; +0x1c static params → `MI_MD_Init(arg1)`: +0x1c u16 width, +0x1e u16 height, +0x20 u8 colour, +0x24 u32 stride, +0x28 mb_size (0 4×4, 1 8×8, 2 16×16), +0x2c sad_out_ctrl (0 16-bit, 1 8-bit), +0x30 ROI `{u8 num; {u16 x, y}[8]}`, +0x54 md_alg_mode (0 FG, 1 SAD, 2 FRAMEDIFF; else ILLEGAL_PARAM); +0x58 dynamic params → `MI_MD_Init(arg2)` / `MI_MD_SetParam`: +0x58 u8 sensitivity, +0x5a u16 learn_rate, +0x5c u32 md_thr, +0x60 u32 obj_num_max |
| OD | +0xc static params → `MI_OD_Init(arg1)` (stride at +0x10 is checked; the window division `div` at +0x18, §7.6); +0x4c OD params → `MI_OD_Init(arg2)` / `MI_OD_SetAttr`: +0x4c thd_tamper, +0x50 tamper_blk_thd, +0x54 min_duration; RstBufSize = 0x20 |
| VG | +0xa u16 width, +0xc u16 height (→ `MI_VG_Init(attr+0xc0, w, h)`), +0xe u16 stride (checked), +0x10 float object-size threshold (asserted in [1e−6, 100]) → `MI_VG_SetObjSizeThd`, +0x14 u8 scene (≤ 1) → `SetScene`, +0x15 u8 function (≤ 3) → `SetFunction`; if the u8 at +0x148 == 2: line mode, +0x16 u16 line count (1…4) → `SetLineNumber`, lines at +0x18 (0x20 bytes each) → `SetLineAndDir(i)`; else region mode, +0xb8 u32 (≤ 2) and region at +0x98 → `SetRegionAndDir`; RstBufSize = 0x154 |

MD sizes computed by CreateChn: `blocks = (w >> (mb+2)) · (h >> (mb+2))`; StatusLen = blocks; ObjLen = 0xbf4; FG: SadLen =
0, RstBufSize = (blocks + 0xc37) & ~3; SAD / FRAMEDIFF: SadLen = blocks · (2 − sad_out_ctrl), RstBufSize = (blocks + SadLen
+ 0xc43) & ~3. The stride check: the input-port extra config `{u16 0x10, u16 4 / 8 / 16 for mb 0 / 1 / 2}` (OD, VG:
`{0x10, 4}`) goes to `MI_SHADOW_SetInputPortBufExtConf`; then `stride % (first u16 after the call)` must be 0 or the process
asserts ("stride(%d) not align up (%d)").

### 7.4 API functions
<!-- ev:sdk-orphan-libs.7.4 -->
Every call write-locks the module rwsem, then the channel mutex; `chn` ≥ 16 or a NULL pointer → ILLEGAL_PARAM; not
inited → 0xA0012015.
- **`MI_VDF_Init()`**: second call → 0xA0012016. Clears the manager, debug level = 2, prints the
  `===== libmi_vdf.so is running on platform [:<date> <time>] =====` banner, inits the 16 channel mutexes, fills the shadow
  descriptor (§7.2), allocates the 0x8000 dump buffer (assert), `MI_SHADOW_RegisterDev(&desc, &g_hVDFDev)` (failure → its
  error, nothing started), bInit = 1, starts thread `vdf_GetYuvImage_task` and three threads `vdf_CalcRst_<n>_task`
  (n = 0 MD, 1 OD, 2 VG; the thread argument is n), inits the monotonic cond + mutex.
- **`MI_VDF_Uninit()`**: not inited → 0 (warning). Sets the four exit flags, joins the four threads, destroys cond / mutex,
  `MI_VDF_DestroyChn` on all 16, frees the dump buffer, destroys mutexes, `MI_SHADOW_UnRegisterDev`, clears the manager,
  destroys the rwsem. Returns 0.
- **`MI_VDF_CreateChn(chn, attr)`**: status ≠ 0 → 0xA0012012. Record cleared, attr copied to +0x74, queue initialised;
  `MI_SHADOW_EnableInputPort(dev, chn, 0)`, `EnableOutputPort(dev, chn, 0)`, `EnableChannel(dev, chn)` (failure →
  0xA0012017); library version of the mode stored once; per mode: `MI_SYS_SetChnOutputPortDepth(0, {VDF, 0, chn, 0}, cnt,
  cnt+2)`, stride check, then MD `MI_MD_Init(&attr.static, &attr.dyn)` / OD `MI_OD_Init(&attr+0xc, &attr+0x4c)` +
  `MI_OD_SetAttr` + `MI_OD_AlarmReset(h, 1, 5)` / VG attribute setup + `MI_VG_Init`; a NULL handle → 0xA0012015 (the shadow
  ports stay enabled); interval copied; RstBufSize / sub-sizes written (§7.3). Status = 1. Returns 0.
- **`MI_VDF_DestroyChn(chn)`**: status 0 → 0 ("has destroyed"). `MI_SHADOW_DisableChannel`, `DisableInputPort`,
  `DisableOutputPort` (failure → 0xA0012018); drains the image queue (each image: `MI_SHADOW_FinishBuf(dev, hInputBuf)` +
  free); `MI_MD/OD/VG_Uninit(handle)`; handle = 0, status = 0.
- **`MI_VDF_SetChnAttr(chn, attr)`**: work mode must equal the created one (else ILLEGAL_PARAM with the "has been created in
  enWorkMode…" message); status 0 → 0xA0012015. MD: `MI_MD_SetParam(h, {sensitivity, learn_rate, md_thr, obj_num_max})`,
  stored on success. OD: `MI_OD_SetAttr(h, {+0x4c, +0x50, +0x54})`, stored on success. VG: `MI_VG_Uninit`, copy
  attr+4…+0x177, redo the VG setup and `MI_VG_Init` (NULL → bare 0x15). Algorithm errors are only printed; returns 0.
- **`MI_VDF_GetChnAttr(chn, attr)`** [R: disassembly 0x17bda–0x17c4e]: VG copies the stored 0x178 bytes. MD: a zeroed
  16-byte `MI_MD_param_t` on the stack goes to `MI_MD_GetParam(handle, &p)`; then attr +0x58 u8 := `p.sensitivity` (p+0),
  +0x5A u16 := `p.learn_rate` (p+2), +0x5C…+0x63 := `p.md_thr`, `p.obj_num_max` (p+4…+0xB); `p.LSD_open` (p+0xC) is not
  copied. libMD's `GetParam` fills those fields from the handle's current parameters and always returns 0
  (0x17d40), so the values are the ones last accepted by `MI_MD_Init` / `MI_MD_SetParam`.
  OD: a zeroed 12-byte `MI_OD_param_t` goes to `MI_OD_GetAttr(handle, &p)`; attr +0x4C / +0x50 / +0x54 (+0x4C…+0x57) := `thd_tamper`,
  `tamper_blk_thd`, `min_duration`. libOD keeps them as bytes and widens each to u32 (0x14860), so the values read back
  are the clamped ones (thd_tamper ≤ 10, tamper_blk_thd ≤ windows², min_duration mod 256; `ive-video-analytics.md` §10.5). In
  both modes nothing else of the caller's struct is written (`enWorkMode`, static parameters, sizes stay as the caller left
  them). A failing Get → bare `−1` (0xFFFFFFFF), and the channel mutex and the module rwsem stay **locked** on that path
  [R: the two `return 0xffffffff` leave before `CamOsMutexUnlock` / `CamOsRwsemUpWrite`; I: every later VDF call then
  blocks].
- **`MI_VDF_EnableSubWindow(chn, u32 unused1, u32 unused2, u32 bEnable)`**: bEnable > 1 asserts. Enabling needs the
  channel's work mode to be running (`MI_VDF_Run(mode)` first, `g_stVdfSrcMgr[1+mode]`); otherwise the call is skipped with
  "<MD/OD/VG> bEnable = FALSE, cant set EnableSubWindow = 1, skip it!" and returns ILLEGAL_PARAM. Disabling always
  proceeds. Enable: shadow input / output port + channel enable → status 2 (failure 0xA0012017). Disable: shadow disables
  (failure 0xA0012018), drain queue, statistics cleared, status 3. Stores bEnable in attr `u8Enable` (+4).
- **`MI_VDF_Run(mode)`** / **`MI_VDF_Stop(mode)`**: mode ≥ 3 → ILLEGAL_PARAM; Run sets `running[mode]`; Stop drains the
  queues of every channel of that mode whose `u8Enable` is set, clears their statistics, clears `running[mode]`. (How
  libmi_sed calls them: §6.4.)
- **`MI_VDF_GetResult(chn, result, s32 timeout)`**: status ≠ 2 → 0xA0012013. `MI_SYS_ChnOutputPortGetBuf({VDF, 0, chn, 0},
  &buf, &h)` (timeout not used [R: not passed]); failure → returned (counted). Builds `result` from the raw buffer (§7.6),
  `result->handle (+4) = h`; the raw size must equal RstBufSize or the process asserts (assert text
  `stBufInfo.stRawData.u32BufSize == g_stVdfSrcMgr.stChnInfo[VdfChn].stAttr.stOdAttr.u16RstBufSize`). Bad md_alg_mode /
  work mode → buffer put back, ILLEGAL_PARAM.
- **`MI_VDF_PutResult(chn, result)`**: status ≠ 2 → 0xA0012013; `handle == 0` → 0 (print); else
  `MI_SYS_ChnOutputPortPutBuf(handle)`, 0.
- **`MI_VDF_GetLibVersion(chn, u32 *ver)`**: the channel's algorithm library version (`MI_MD/OD/VG_GetLibVersion`).
- **`MI_VDF_GetDebugInfo(chn, info)`**: VG only — `MI_VG_GetDebugInfo(h, info+4)` and a print of its fields; MD / OD print
  "E_MI_VDF_WORK_MODE_(MD/OD) no support!" and return an error.

### 7.5 Threads and data flow [R: `vdf_GetYuvImage_task` 0x136d8, `vdf_CalcRst_task` 0x13d78]
- **`vdf_GetYuvImage_task`** (one): until exit flag +4: for each channel with status 2 — refresh the input fps once per
  window (below); `MI_SHADOW_GetInputPortBuf(dev, chn, 0, &bufInfo (0x108), &hIn)`; on success allocate a 0x20-byte image
  record `{u32 phy; u8 *vir (asserted ≠ NULL); u64 pts; u32 hIn; list node}`, Y size = stride[0]·height → record +0x24; if
  the channel's mode is running → append to the channel queue, set **all three** "new image" flags (+8 / +9 / +0xA) to 1
  and broadcast the cond (+0x1F30) under its mutex (+0x1F18); otherwise `MI_SHADOW_FinishBuf(dev, hIn)` and free. A pass
  with no buffer from any channel → `CamOsMsSleep(100)`; otherwise `MI_SHADOW_WaitOnInputTaskAvailable(dev, 100)`.
- **`vdf_CalcRst_task(n)`** (n = 0 MD, 1 OD, 2 VG), loop: take the module rwsem (read), **clear flag n**, leave if not
  initialised or exit flag +5+n is 1; mode n not running → release, `CamOsMsSleep(100)`, repeat (the fps window resets to
  1 s). Otherwise one pass over the 16 channels; each of mode n with status 2: refresh the output fps once per window; pop
  **one** image (none → next channel); image without handle or address → finish + free; frame counter +1, skip (finish +
  free) unless `counter % interval == 0`; `MI_SHADOW_GetOutputPortBuf(dev, chn, 0, {raw, size = RstBufSize}, 0, &out,
  &hOut)` (vir ≠ NULL and size == RstBufSize asserted); then
  - MD: `MI_MD_Run(h, image)`; SAD / FRAMEDIFF: `MI_MD_ComputeImageSAD(h, image, sadDesc)`; `MI_MD_CCL(h, &attr.ccl,
    objDesc)`; `MI_MD_GetMBResult(h, statusDesc, &size)` (size ≤ StatusLen asserted); header (pts, enable) written;
  - OD: `MI_OD_Run(h, image)`, then `_MI_VDF_SetReslut_OD` (§7.6);
  - VG: `MI_VG_Run(h, image.vir)`; `MI_VG_GetResult(h, rst)` (§7.6);
  - success → `MI_SHADOW_FinishBuf(dev, hOut)` (the result becomes readable by `MI_VDF_GetResult`); failure →
    `MI_SHADOW_RewindBuf(dev, hOut)`;
  - optional YUV dump: while the channel's dump count > 0, write the Y plane to `/<path>/vdf[<chn>]_image[<i>]_<ms>.yuv`
    (`fopen "w+"`) and decrement the count (one Y plane per processed frame);
  - finally `MI_SHADOW_FinishBuf(dev, hIn)` and free the image record.

  After the pass, if any channel of mode n still has queued images (queue count, record +0x70) the loop restarts at once;
  else the rwsem is released and the worker waits on the cond with a **1 s** absolute timeout (`CLOCK_MONOTONIC`, now +
  1 000 000 000 ns) until flag n is set or `pthread_cond_timedwait` returns non-zero. Flag set before the wait → no wait;
  spurious returns re-check the flag. So a worker wakes on any channel's image (all modes' flags are set), on the 1 s
  timeout, or when its mode starts.
- `MI_SHADOW_GetOutputPortBuf` failing with an error whose low 12 bits are 0x01C ("not ready", `mi_vdisp-mi_shadow.md`
  §7.4) puts the image back at the queue head and sleeps 10 ms with the channel mutex released; any other failure drops
  the image (+0x30, +0x44).
- fps: both threads refresh their fps once per window; the window is **1 s until the first frame has been counted, then
  3 s** (two locals become 3); fps = frames in the window / window length.

### 7.6 `MI_VDF_Result_t` (0x160) and the raw result buffers
<!-- ev:sdk-orphan-libs.7.6 -->
`+0 enWorkMode` (caller's), `+4 u32 handle` (mi_sys output buffer, for PutResult), `+8` mode data. Algorithm-side structs
(`MI_MD_*`, `MI_OD_*`, `MI_VgResult_t`): `ive-video-analytics.md` §9.1, §10.1, §11.2.

**MD** (`MI_VDF_MdResult_t`): +8 u64 pts, +0x10 u8 bEnable / valid, +0x14 `{StatusLen, SadLen, ObjLen}`, +0x20 status
pointer, +0x24 SAD pointer (NULL for FG), +0x28 object (CCL) pointer — all pointing into the mapped raw buffer. If the sizes
in the buffer differ from the channel's, the result is zeroed, +0x10 = 1 and the channel's sizes are put in
("stSubResultSize is not match"). Raw buffer: `0x00` header (0x24 bytes, copied to result+8) · [SAD / FRAMEDIFF: `0x28` SAD
descriptor `{u8 *data; stride; enOutCtrl}` (0xc) · `0x34` SAD data (SadLen)] · object descriptor (0x14) `{u8 u8RegionNum;
…; +4 region *astRegion; +8 u8 indexOfMaxObj; areaOfMaxObj; areaOfTotalObj}` · regions (ObjLen = 0xbf4 = **255 × 12 bytes**
`{u32 area; u16 x0, x1, y0, y1}`) · status descriptor `{u8 *data}` · status data (StatusLen = one byte per macro-block).
GetResult rewrites the descriptor pointers to the user mapping.

**OD** — `MI_VDF_GetResult` copies the raw buffer (0x20 bytes) unchanged to result +8. The buffer is written by
`_MI_VDF_SetReslut_OD` after `MI_OD_Run` returned ≥ 0 (0 prints "No OD detected!" at info level; the result is written
either way; < 0 → output buffer rewound):

| raw | result | field |
|---|---|---|
| +0x00 | +0x08 | u8 valid := 1 |
| +0x01 / +0x02 | +0x09 / +0x0A | u8 windows per axis n (both): attr `div` (attr +0x18) 0 → 1, 1 → 2, 2 → 3 |
| +0x03 | +0x0B | u8, never written (printed as "DataLen") |
| +0x04…+0x07 | +0x0C | not written |
| +0x08 | +0x10 | u64 pts of the input frame |
| +0x10…+0x18 | +0x18 | u8 state[3][3]: byte +0x10 + 3·i + j = `MI_OD_GetWindowResult(h, i, j)` for i, j < n (libOD names the arguments `col, row`): 0 normal, 1 tamper, 2 no feature, 0xFF fail; unused cells stay 0 |
| +0x19…+0x1F | | not written |

Another `div` value prints "Get the wrong window Divs(%d)" and writes nothing, but the buffer is still finished, so the
caller receives a result with stale contents [R; I: the mi_sys buffer is not cleared by VDF].

**VG** — the raw buffer (0x154 bytes) is copied unchanged to result +8. It is `MI_VgResult_t`, filled directly by
`MI_VG_GetResult(h, raw)` after `MI_VG_Run(h, image.vir)` returned 0 (layout [S],
fill [R: libVG `IvaVG_GetResult` 0x139a8]): +0x00 s32 `alarm[4]` (only `line_number` entries copied; the rest keep the
buffer's old bytes), +0x10 s32 `alarm_cnt`, +0x14 `bounding_box[20]` × `{s32 up, down, left, right}` (min y, max y, min x,
max x; zeroed first). In the result these sit at +0x08, +0x18, +0x1C. The VG result carries **no pts**. A non-zero
`GetResult` prints "Get VG alarm error" and the buffer is still finished; `MI_VG_Run` ≠ 0 prints "Run VG error" and rewinds
it. `alarm_cnt` > 20 overruns the array (`ive-video-analytics.md` §11.6).

### 7.7 Proc commands (`echo <cmd> … > /proc/mi_modules/mi_shadow/mi_vdf0`, through mi_shadow) [R: `_MI_VDF_Cmd*` 0x1252c–0x12f90]
<!-- ev:sdk-orphan-libs.7.7 -->
`help` → `_MI_VDF_OnHelp`. All command handlers need argc ≥ 2 (`dump_image` ≥ 4); fewer → "Unsupport command count:
%d.commond:" + the words, a usage line, `0xA0012003` (`log_level`: 0) [R]. (An earlier reading of ours gives the limit as fewer than 3 words for every command; which reading holds for the
one-argument commands is not settled.)
- **`log_level <n>`** → `g_stVdfProcDebug` +0: n < 6 is stored; **6 is rejected** although the help text offers 0–6 ("VDF
  SetDebugLevel Attr: [0-6]"); returns 0 even on a bad value. The help names 0 CLOSE, 1 FATAL, 2 ERROR, 3 WARN, 4 INFO,
  5 DEBUG, 6 TRACE; the code prints errors at ≥ 1, warnings ≥ 2, info ≥ 3, debug ≥ 4, entry / exit ≥ 5 — one step below
  the help's names.
- **`sem_check <0|1>`** → +4 (EnCheckPoint). Its only reader is the dump: when set, `cat` adds the "CheckPoint Debug info"
  section (`[en_dbg]` DebugLevel / EnCheckPoint / EnSelfCheck / DumpAll, `[ck_pnt]` the five check counters, `[run_cnt]` the
  four run counters). **Nothing in the library ever writes +0xC8…+0xEB** (no store with those offsets in.text; only Init's
  `memset`), so those rows always print 0 [I: unless written through a computed address the search missed — not ruled out
  exhaustively].
- **`auto_check <0|1>`** → +5 (EnSelfCheck). When set, `cat` adds the "SelfChecking Debug info" section
  (`_MI_VDF_SelfChecking`), per created channel: (1) Y image size (+0x24) == stride × height of the attr (MD: stride,
  height; OD: inImgStride, inImgH; VG: stride, height); (2) **calls `MI_SHADOW_SetInputPortBufExtConf(dev, chn, 0, {0x10,
  4 / 8 / 16})` again** (MD by mb_size, OD / VG 4) and checks stride mod the first returned u16 == 0; (3) `u8Enable` 1 ⇔
  status 2 (started), 0 ⇔ status 3 (stopped), else error; warning when `u8Enable` ≠ the work mode's running flag; (4) for
  started channels |InFps − OutFps| < 2 → ok, OutFps larger → error, InFps larger → warning "all WorkMode Task Stop, only
  get YImage" when no mode runs, else error. Then "Others Check": for each worker n = 0…2 the run counter is read, 100 ms
  slept, read again — mode not running: unchanged = ok; running: unchanged = "Run status fail, maybe block!!!"; the YUV
  thread (3) likewise. With the counters never written, every running worker and the YUV thread report "maybe block" [I].
  The section sleeps 4 × 100 ms inside the dump callback while mi_shadow waits only 100 ms for its answer
  (`mi_vdisp-mi_shadow.md` §7.2) [I: the kernel times out and prints the previous dump buffer]. An interval `u8VDFIntvl` > 1
  lowers OutFps by design, so check (4) reports an error for such channels [I].
- **`dump_all <0|1>`** → +4, +5 **and** +6 together. +6 adds "DepLibs Debug info": per created channel, VG →
  `MI_VG_GetDebugInfo` (Version, DbgState, BgState, DbgObjSize and the four point arrays dfp / dsp / dfdp / dsdp of
  `MI_VgDebug_t`); MD and OD → "Libs no support.".
- **`en_workmode <md|od|vg> <0|1>`** → `MI_VDF_Run/Stop(mode)`.
- **`en_chn <chn> <0|1>`** → `MI_VDF_EnableSubWindow(chn, 0, 0, b)`.
- **`dump_image <chn> <num> <path>`**: path → +0x48 by `memcpy(strlen)` **without a terminator** (a shorter path keeps the
  tail of a longer earlier one [I]; a NULL word gives "/mnt"); num = 0 → usage, `0xA0012003`; chn ≥ 16 → silently 0; else
  count[chn] := num — the YUV dump of §7.5.

### 7.8 `cat` output (`_MI_VDF_OnDumpVdfAllAttr`, event 5) [R: format strings and the fields they print]
Written into the 0x8000-byte dump buffer (offsets wrap at 0x7FFF), sections in order:
1. "Dump all private info for VDF": `VDF Verison:` + the `"[<date> <time>]"` string (manager +0x1F60).
2. "Dump DEV info": `bInited, MDEnable, ODEnable, VGEnable, YuvTaskExit, MDTaskExit, ODTaskExit, VGTaskExit` = manager
   bytes +0…+7, `DbgLevel`.
3. Per created channel (status ≠ 0), "CHN common info": `ChnId, eStatus (+0x18), phandle (+0x1C), FrameCnt (+0x20),
   FrameInv (+0x21), YImgSize (+0x24), ImageQCnt (+0x70)`.
4. "Input Port info": `ChnId, InGetCnt (+0x28), InGetFailCnt (+0x2C), InDropCnt (+0x30), InDoneCnt (+0x34), InFps
   (+0x3C, float)`.
5. "Output Port info": `ChnId, OutGetCnt (+0x40), OutGetFailCnt (+0x44), OutDropCnt (+0x48), OutDoneCnt (+0x4C),
   RstGetCnt (+0x58), RstGetFailCnt (+0x5C), RstDropCnt (+0x60), RstPutCnt (+0x64), OutFps (+0x54, float)`.
6. "MD Attr info … [Ver:<MD lib version>]" per MD channel: `[c_param]` Enable, MdBufCnt, VDFIntvl, RstBufSize,
   SubRstObjLen, SubRstSadLen, SubRstStsLen, ccl_InitArea, ccl_Step; `[s_param]` width, height, stride, color, mb_size,
   sad_out_ctrl, roi_md_num, md_alg_mode and the ROI points; `[d_param]` sensitivity, learn_rate, md_thr, obj_num_max.
7. "OD Attr info … [Ver:…]" per OD channel: Enable, OdBufCnt, VDFIntvl, RstBufSize; inImgW, inImgH, inImgStride,
   nClrType, alpha, div, roi_od_num, M, MotionSensit (header order) and the ROI points; thd_tamper, tamper_blk_thd,
   min_duration.
8. "VG Attr info … [Ver:…]" per VG channel: Enable, VgBufCnt, VDFIntvl, RstBufSize; width, height, stride, obj_size_thd,
   indoor, func_state, line_number; then either the lines (`p/pd` point pairs, line mode) or region direction + the four
   region points.
9. Only with +4: "CheckPoint Debug info" (§7.7). Only with +6: "DepLibs Debug info" (§7.7). Only with +5: "SelfChecking
   Debug info" (§7.7).

## 8. libmi_wlan.so — Wi-Fi control through shell commands described in a JSON file, 28 functions [R unless tagged]

No kernel module, no device: every operation is a shell command line **composed from a JSON description** and run with
`system()` / `popen()`; outputs of `iwlist` / `wpa_cli` / `hostapd_cli`-style tools are parsed back ("resolver"). Imports
cJSON (§9.1), libc string / stdio, `system`, `popen/pclose`, `setenv`, `signal`. Function names
below are the `__func__` strings the code prints with; functions without a name are named by what they do [I].

### 8.1 Objects, state, handles, errors
- **State word** (`.data` 0x19198, initial 1): bit0 not initialised, bit1 initialised, **bit2 (4) AP opened, bit3 (8) STA
  opened, bit4 (0x10) AP connected, bit5 (0x20) STA connected**.
- **`WLAN_DBG_LEVEL`** (exported, `.data` 0x1915c, initial 3; set from the JSON by Init, §8.5). The prints test > 1
  (errors), > 2 (warnings), > 3 (info). One global `pthread_mutex_t` (0x193cc) serialises the API.
- **Resolver object** (`.data` 0x19160, 0x28 bytes, filled by `InitResolver`): +0 / +4 cJSON hooks `{malloc_fn, free_fn}`
  (two stubs 0x161d4 / 0x161d8 that tail-jump to libc `malloc` / `free`); +8 `FILE *` of the JSON file; +0xc cJSON root;
  +0x10 file text buffer; +0x14 debug level from the JSON; +0x18 `resolverInit`, +0x1c `resolverParse`, +0x20
  `resolverDeInit`, +0x24 `resolve`. `InitResolver` also fills the two parser tables of §8.7.
- **Composer object** (`.data` 0x19188, 0x10 bytes, filled by `InitComposer`): +0 pointer to `gstWlanJson.wifi` (below);
  +4 `composerInit`, +8 `composerCmd`, +0xc `composerDeinit` (returns 0, does nothing).
- **`gstWlanJson`** (`.bss` 0x193e4): +0 top-level `selfie`; then the `wifi` block J (0x193e8): J[0] `<module>.selfie`,
  J[1] `init`, J[2] `deinit`, J[3] `universal`, J[4] `individual`. **`gpRefs`** (`.bss` 0x193fc) = the reference dictionary
  while a set is being composed (§8.4), else NULL.
- **`pVarFence` / `varFence`**: an exported guard pair (`pVarFence` = &`varFence`, `.rodata` 0x7d18) that terminates the
  variadic argument lists of `composerCmd`, `__CMD_option_refs` and `resolve`.
- **`AP_HANDLE`** (exported, `.rodata` 0x7ca0) = `0x41506170` ("paPA"): the handle that means "the access point"; any other
  handle value means the station — a station handle is the supplicant's network id (§8.6).
- **Error codes**, module 0x1C (28, WLAN): `0xA01C2003` NULL argument / bad password length / a `binary` that is not an
  array; `…2004` already open; `…2006` JSON item missing; `…2007` not open / not connected; `…2011` DHCP lease file not
  readable (§8.8); `…2015` not initialised; `…201F` a shell command failed (non-zero exit), an empty version string, a
  resolver failure, or `MI_WLAN_Init` while bit0 is clear. `…2016` ("initialised twice") is in the code but unreachable
  (§8.2). Error paths for NULL arguments call `pthread_mutex_unlock` on the mutex they never locked.

### 8.2 API
Every call composes into a stack buffer with unbounded `sprintf`: 0x100 bytes for Init, DeInit, Open, Close, Disconnect,
GetStatus, GetWlanChipVersion; 0x800 for Scan and Connect.

| function | needs | does |
|---|---|---|
| `MI_WLAN_Init(const char *jsonPath)` | bit0 set (else `0xA01C201F`), then bit1 clear (else `0xA01C2016`) | init mutex; `InitResolver`, `InitComposer`, `resolverInit(res, path)`, `resolverParse(res)`, `composerInit(res, comp)` — **none of the three return values is checked** [R: disassembly 0x119b8–0x119cc]; `WLAN_DBG_LEVEL := res+0x14`; `composerCmd(0, buf[0x100], fence)` (INIT) and `mi_sys_LaunchSystemCmd(buf)`; success → bit1 set, bit0 cleared; failure → mutex destroyed, error returned |
| `MI_WLAN_DeInit()` | bit1 | compose + run cmd 3 (DEINIT); resolver and composer de-init (always); success → state = 1 |
| `MI_WLAN_Open(const OpenParam *p)` | bit1; `p` ≠ NULL | `p->eNetworkType` (u32 at +0): **1 = AP → bit 4, otherwise (0 = infra / STA) → bit 8**; already set → 0xA01C2004; cmd 1 (OPEN) with the type; success → bit set |
| `MI_WLAN_Close(const OpenParam *p)` | bit1 | not open → 0 (nothing run); cmd 2 (CLOSE); success → bit cleared |
| `MI_WLAN_Connect(const u32 *phWlan, const ConnectParam *c)` | bit1; open for that side | password (`c+0x84`, C string) must be empty or 8…63 chars; `*phWlan == AP_HANDLE` → AP side (open bit 4, sets 0x10) else STA (8, 0x20); cmd 5 (CONNECT); a station handle < 0 gets the new network id written back (§8.6) |
| `MI_WLAN_Disconnect(u32 hWlan)` | bit1; connected for that side | not connected → 0; cmd 7 (DISCONNECT) with `{isAp, handle}`; clears the connected bit |
| `MI_WLAN_Scan(void *unused, ScanResult *r)` | bit1; **STA open** (bit 8) else 0xA01C2007 | `r[0x3700] = 0x40` (capacity 64 cells); cmd 4 (SCAN) + the awk filter below, output parsed into `r` (§8.8); **returns 0 whatever the parser returns** [R: 0x11754] |
| `MI_WLAN_GetStatus(u32 hWlan, Status *s)` | bit1; open for that side (else 0xA01C2007) | cmd 8 (STATUS) for AP or STA; output parsed into `s` (§8.8); **returns 0 whatever the parser returns** [R: 0x12984] |
| `MI_WLAN_GetWlanChipVersion(char *out)` | bit1 | cmd 9 (VERSION) → the text `"<name> <version> "` from `<module>.selfie` (§8.6); empty → 0xA01C201F; `strcpy` to `out` (no length) |

- A successful Init clears bit0 and sets bit1, so **a second Init returns `0xA01C201F`**; `0xA01C2016` is unreachable [R:
  0x11960–0x1196e].
- A missing or broken JSON composes an empty command; `system("")` exits 0 and Init reports success [R path; I: `system("")`
  of glibc returns the shell's 0].
- Helpers: `mi_sys_LaunchSystemCmd(cmd)`: prints at level ≥ 4, sets SIGCHLD to default around `system(cmd)`, returns 0 or
  0xA01C201F; `mi_sys_SystemCmdWithReturn(cmd)`: the same but returns the exit status 1…255 (0 or a signal → 0xA01C201F).
- The scan filter appended to the scan command (`.rodata` 0x74f4): `| busybox awk ' /Cell/ {…} /ESSID/ ; /Mode/ {…}
  /Channel/ {…} /Qual/ {…} /Encr/ {…} /Group Cipher/ {…} /Pairwise Ciphers/ {…} /Authentication Suites/ {…}; '` (iwlist
  output); the AP station query uses `| busybox awk ' /dot11RSNAStatsSTAAddress/ {…} /connected_time/ {…}'` (hostapd_cli
  all_sta; 0x67 bytes appended to the 0x100 buffer).

### 8.3 `composerCmd(u32 cmd, char *out, …)` (0x13464) and the flag word
- Variadic. The arguments are collected into a local array up to and including the terminator `pVarFence`; n = number of
  arguments before it. Then `composerFlag(cmd, &flags, args, n)`, then `compose(cmd, out, flags, J, args, n)` (§8.6);
  **compose's return value is dropped** — composerCmd always returns 0.
- After compose: cmd 4 (SCAN) and cmd 10 (CHANNEL) append `args[1]` + `" "`; n = 0 prints "wrong scan mode" / "lack of
  channel id" and appends nothing. cmd < 10 otherwise prints "direct cmd %d" (info); cmd > 10 "unknown cmd %d".
- Command ids and arguments per API call (all are pointers unless said):

| cmd | caller | args |
|---|---|---|
| 0 INIT | Init | — |
| 1 OPEN / 2 CLOSE | Open / Close | `&eNetworkType` |
| 3 DEINIT | DeInit | — |
| 4 SCAN | Scan (0x11754) | `&mode (= 0)`, the iwlist awk filter string (`.rodata` 0x74f4) |
| 5 CONNECT | Connect (0x11880) | `&isAp`, `ConnectParam *c`, `phWlan`, `c+4` (SSID), `c+0x84` (password), the `resolve` function pointer |
| 6 | — | no compose case ("unknown cmd" path, returns 0); 6 exists only as the resolver's "read one integer" mode (§8.7) |
| 7 DISCONNECT | Disconnect | `&isAp`, `&hWlan` |
| 8 STATUS | GetStatus | `&isAp` |
| 9 VERSION | GetWlanChipVersion | — |
| 10 CHANNEL | **no caller** in the library | (would take a channel string as args[1]) |

- **`composerFlag`** builds the flag word: network type → `0x100` infra (type 0) / `0x200` AP (type 1); types 2…5 print
  " not support yet!!" and then " unkown network type", ≥ 6 only the latter; no bit is set for them. OPEN and CLOSE fall
  through into the STATUS case, so their type is evaluated twice (the warnings print twice). CONNECT: `*phWlan < 0` →
  `0x20000`, else `0x10000`; security `c->+0` == 4 → bit `1`; == 8 or 16 → bit `2`; anything else prints " unkown security
  type" (bits 0). DISCONNECT: `hWlan < 0` → `0x20000` else `0x10000` (not used by compose).
- Security values 4 / 8 / 16 match the SigmaStar `MI_WLAN_Security_e` bit order UNKNOWN 1, NONE 2, WEP 4, WPA 8, WPA2 16
  [I: memory of the SDK header; no header on hand].

### 8.4 Command-line grammar: the three builders
- **`__CMD_option_refs(char *out, cJSON *item, …)`** (0x13610): variadic list of cJSON items ending at the fence (or
  at a NULL item). A non-array item prints `"<string> "`. An array prints each element as `"<string> "`; an element that
  is a string of ≥ 2 characters **starting with `$`** is a **reference**: the string is split at `:` and, starting from
  `gpRefs`, each token minus its first character is looked up as an object key (`"$a:$b"` → `gpRefs.a.b`); if the walk
  ends on an item, that item's string is printed instead, otherwise the element itself. With `gpRefs == NULL` the element
  is printed as is (info "gpRefs==NULL").
- **`_CMD_SetWrapper(out, set, refs, values, sep)`** (0x137d4; a copy without `values`, `_CMD_SetWrapper_novalues` (0x13cc8), serves the
  sets that take no run-time values). `set` = `{ "binary": [b0, b1, …], "option0": [...], "option1": [...], … }`:
  `binary` missing → `0xA01C2006` ("binSet = null"), not an array → `0xA01C2003`; `gpRefs := refs` (always the `universal`
  object); for each i: `option<i>` must exist (else `0xA01C2006`), it is duplicated; **every element of it that contains a
  `%` is replaced, in order, by the next element of `values`** (one running index over all options of the set; replacement
  through `cJSON_ReplaceItemInArray`); then `"<bi> <option_i…> "` is appended through `__CMD_option_refs`; if `sep` ≠ NULL
  and the last non-blank character of the text so far is not `&`, `sep` is appended (so a background command `… &` gets no
  ` ; `; the separator also follows the last command). At the end `gpRefs := NULL`.
- **`_CMD_ScriptWrapper(out, script, selfie)`** (0x13b14): `script` = `{ "name": "<file>", "option": [...],
  ["parser": "<interpreter>"] }`; `parser` is taken from the script object, else from `<module>.selfie` (neither →
  `0xA01C2006`); `scriptPath` from `<module>.selfie` (missing → `0xA01C2006`); output `"<parser> <scriptPath>/<name>
  <option…> "` (the path joined with `%s/%s`). Options are printed without reference resolution (gpRefs is NULL here).

### 8.5 JSON description [R key names and where each is looked up; meaning I]
<!-- ev:sdk-orphan-libs.8.5 -->
Every `action` is looked up in **`individual`**; **`universal`** serves only as the `$` reference dictionary (§8.4) and for
plain keys: `iface`, `hostapd_conf_real`, `dhcp-leasefile` (speaks of four plain keys but names
these three; a fourth is not identified):
```
{ "selfie": { "wifi": "<module>", "debugLevel": "fatal|error|warn|info|debug|entry" },
  "<module>": {
    "selfie":   { "scriptPath": "<dir>", "parser": "<interpreter>", "name": "<chip>", "version": "<ver>" },
    "init":     { "env": { "name": [..], "value": [..] }, "script": <script> },
    "deinit":   { "script": <script> },
    "universal":  { "iface": "<if>", "hostapd_conf_real": "<file>", "dhcp-leasefile": "<file>", <ref targets…> },
    "individual": { "action": {
        "open":       { "deviceup":   { "infra": <set>, "ap": <set> }, "serviceup":   { "infra": <set>, "ap": <set> } },
        "close":      { "devicedown": { "infra": <set>, "ap": <set> }, "servicedown": { "infra": { "script": <script> }, "ap": { "script": <script> } } },
        "scan":       <set>,
        "connect":    { "infra": { "add": <set>, "setup": { "ssid": <set>, "password": { "keyoff": <set>, "keyon": { "wep": <set>, "wpa": <set> } } },
                                   "enable": <set>, "dhcpc": <set>, "save": <set> },
                        "ap": <set> },
        "disconnect": { "infra": <set>, "ap": <set> },
        "status":     { "infra": <set>, "ap": { "all_sta": <set> } },
        "channel":    { "binary": "<cmd>", "option": [..] } } } } }
```
- `<set>` = `{ "binary": [...], "option0": [...], … }` (§8.4 `_CMD_SetWrapper`); `<script>` = `{ "name", "option",
  ["parser"] }` (§8.4 `_CMD_ScriptWrapper`). In option elements, `%` marks a slot for the next run-time value (SSID,
  password, network id) and `$a:$b` a reference into `universal`.
- `composerInit` requires the top `selfie`, its `wifi` string, the `<module>` object and its `selfie`, `init`, `deinit`,
  `universal`, `individual`, and the top `debugLevel` (each missing → `0xA01C2006`, ignored by Init, §8.2).
- **Debug levels** (`_getDebugLevel`, stored in `WLAN_DBG_LEVEL`): `fatal` 1, `error` 2, `warn` 3, `info` 4, `debug` 5,
  `entry` 6, any other string 3. 5 and 6 print nothing more than 4.

### 8.6 What each command composes (`compose`, 0x143a8)
`<set>` below = `_CMD_SetWrapper` output with refs = `universal`; `;` = the `" ; "` separator; t = `infra` (flag 0x100) or
`ap` (0x200); neither flag → both sets NULL, their wrappers fail and nothing is written.
- **0 INIT**: for every pair i of `init.env.name[i]` / `value[i]`: if the variable already exists the new value is
  `"<old>:<new>"` (PATH-style append), else `<new>`; `setenv(name, value, 1)`. Both not arrays → "name/value should be
  array"; fewer names than values → "lack of env value"; **`env` missing → nothing at all is composed** (the init script is
  skipped too). Then `init.script` through ScriptWrapper.
- **1 OPEN**: `open.deviceup.t` `;` `open.serviceup.t` `;`.
- **2 CLOSE**: `close.devicedown.t` `;` then the script `close.servicedown.t.script` (ScriptWrapper).
- **3 DEINIT**: `deinit.script`.
- **4 SCAN**: `scan` (no separator) + the awk filter from composerCmd.
- **5 CONNECT infra** (`__CMD_CONNECT_infra`): all of `add`, `setup`, `setup.ssid`, `setup.password`, `enable`, `dhcpc`,
  `save` must exist (each missing → error print, compose returns 0 with what it has).
  1. Only when `*phWlan < 0` (flag 0x20000, "new network"): `add` `;` is composed alone and run through `resolve(6, …)` —
     `popen`, the first output line through `strtol(…, 10)` — and the number is **written back to `*phWlan`** (the station
     handle is the supplicant's network id). If that fails (no output line) compose returns 0 and the caller runs the
     `add` text once more with `system()` [R; I: a second network is added].
  2. The buffer is then rewritten from its start: values = `["<id>", "\"<ssid>\""]` → `setup.ssid` `;` (the SSID quoted as
     `"\"%s\""`, the network id as `%d`).
  3. values become `["<id>", "\"<password>\"", "<id>"]`; password empty → `setup.password.keyoff`; else `keyon.wep`
     (security bit 1) / `keyon.wpa` (bit 2); with neither bit "unknown security" is printed and the `password` object itself
     is passed, which has no `binary` → nothing is added; then `;`.
  4. values = `["<id>", "<id>", "<id>"]` → `enable` `;`; then `save` `;` and `dhcpc` `;` (no values).
- **5 CONNECT ap** (`__CMD_CONNECT_ap_apply_ssid_pwd`): the file named by `universal.hostapd_conf_real` is opened with mode
  **`"a"` (append)**; open failure → "Could not open configuration file '%s' for reading.", `0xA01C2006`. It appends
  `"\nssid=<ssid>"` (when the SSID pointer is non-NULL), then for a password of 8…63 characters `"\nwpa_passphrase=<pwd>"`,
  for `"NONE"` (any case) `"\nwpa_key_mgmt=NONE"`, for anything else nothing and `0xA01C2003`; then `connect.ap` `;` (the hostapd keys written: `ssid`, `wpa_passphrase`, `wpa_key_mgmt`). Since
  `MI_WLAN_Connect` accepts only an empty password or 8…63 characters, the `NONE` branch cannot be reached and an empty
  password ends in `0xA01C2003` with an empty command — `system("")` succeeds and the AP is marked connected [R paths; I:
  outcome]. Every call appends again; the file grows.
- **7 DISCONNECT**: infra: values `["<hWlan>"]` → `disconnect.infra` `;`; ap: `disconnect.ap` `;`.
- **8 STATUS** (`_CMD_STATUS`): `status.infra` with `;` (`_CMD_STATUS_infra`), or `status.ap.all_sta` without a separator
  (`_CMD_STATUS_ap`). The NULL check of `infra` / `ap` comes after the wrapper call (the wrapper only prints).
- **9 VERSION** (`_CMD_VERSION`): `__CMD_option_refs(out, selfie.name, selfie.version)` → `"<name> <version> "` — no shell
  command; MI_WLAN_GetWlanChipVersion returns this text. **The call passes no fence** [R: 0x13f40–0x13f46, r3 and the stack
  are whatever the caller left]: the list walk goes on into r3 and the caller's stack words until it meets a NULL or the
  fence value [I: it works only by chance].
- **10 CHANNEL** (`_CMD_CHANNEL`): `"<individual.action.channel.binary> <universal.iface> <channel.option…> "` + the
  channel argument from composerCmd. Unreachable from the API (§8.3).

### 8.7 JSON loader, `resolve` and the output parsers
- **`resolverInit(res, path)`** (0x16a9c): zeroes res+8…+0x17, sets the hooks, `fopen(path, "r")` (failure → info
  "json file open fail", `0xA01C2006`), `cJSON_InitHooks(res)`. **`resolverParse(res)`**: no file → "should open json file
  first", `0xA01C2006`; else reads the whole file (`fseek` / `ftell`, `calloc(size + 1)`), `cJSON_Minify`, `cJSON_Parse` →
  res+0xc (a parse failure leaves NULL there; composerInit then fails with "resolver should parse in ahead").
  **`resolverDeInit`**: `cJSON_Delete(root)` and `free(text)` if a root exists; clears +0xc / +0x10; **the `FILE *` is
  never closed** (one leaked stream per Init).
- **`resolve(cmd, char *cmdline, void *out, …)`** (variadic, fence-terminated): cmd 6 → `popen(cmdline, "r")`, first line
  (≤ 0x80) → `strtol(…, 10)` into `*(long *)out`, no line → `0xA01C201F`; cmd 4 → scan parser (§8.8; `out[0x3700] := 0x40`
  if it is 0); cmd 8 → status parser with args `(isAp, composer)`; anything else → "unknown cmd", `0xA01C201F`.
- **Line dispatch.** Each output line is first stripped of leading spaces (`strspn` with `" "` — tabs are not skipped).
  - Scan table (`.bss` 0x19400, 50 entries): index = `toupper(line[1]) + toupper(line[2]) − 0x82`. Parsers
    `_SCAN_PARSER_{CELL, CHANNEL, FREQUENCY, QUALITY, ENCRYPTIONKEY, ESSID, MODE}` under the keys `Cell` (E+L), `Channel`
    (H+A), `Frequency` (R+E), `Quality` (U+A), `key:` (E+Y), `ESSID` (S+S), `Mode` (O+D). The awk filter's other lines
    (`Group Cipher…`, `Pairwise Ciphers…`, `Authentication Suites…`) hash to empty slots and are ignored, so **the
    security suites are never parsed**.
  - STA status table (`.bss` 0x194c8, 25 entries): index = `toupper(line[0]) − 'A'`: `a` address, `b` bssid, `f` freq,
    `g` group_cipher, `i` ip_address, `k` key_mgmt, `m` mode, `p` pairwise_cipher, `s` ssid, `w` wpa_state — parsers
    `_STATUS_STA_PARSER_{ADDRESS, BSSID, FREQ, GROUPCIPHER, IPADDRESS, KEYMGMT, MODE, PAIRCIPHER, SSID, STATE}`. Lines
    whose first letter matches but whose key differs (`id=`, `p2p_device_address=`) fail the parser's `sscanf` and change
    nothing.
  - Neither index is range-checked: a line starting with a digit, `_` or punctuation indexes outside the table [R; I: reads
    a neighbouring .bss word and calls it if non-zero].
  - AP status: `_CMD_STATUS_ap_sta` (station lines) and `_CMD_STATUS_ap_lease` (DHCP lease file), §8.8. The parsers sit in
    0x15dd8 … 0x1691c.

### 8.8 Struct layouts as the library reads and fills them [R offsets and values; names and field sizes I]
Field sizes are the distance to the next written offset (no header exists to confirm them); strings are copied with
unbounded `sscanf("%s")` / `strcpy`.

**`OpenParam`** (Open / Close): +0 u32 `eNetworkType` (0 infra, 1 AP; 2…5 "not support yet") — nothing else is read.

**`ConnectParam`** (Connect): +0x00 u32 security (4 WEP, 8 / 16 WPA / WPA2, §8.3); +0x04 char SSID[0x80]; +0x84 char
password[] (C string, length 0 or 8…63). Nothing past the password is read. The handle argument: `AP_HANDLE` = AP;
otherwise a station network id, `< 0` = "add a new network" and the new id is written back (§8.6).

**`ScanResult`** (Scan, ≥ 0x3701 bytes): 64 cells × 0xDC, then +0x3700 u8 (in: capacity, set to 64 by the library; out:
count). Cell:

| off | field | source line (after the awk filter) |
|---|---|---|
| +0x00 | u16 cell number | `Cell %hd %s` |
| +0x04 | float frequency (GHz) | `Frequency:%f` (the `Frequency:… (Channel n)` line, cut by awk at the blank) |
| +0x0C / +0x0D / +0x0E | u8 quality, u8 quality max, s8 signal level (dBm) | `Quality=%hhd/%hhd Signal level=%hhd` |
| +0x0F | u8 encrypted: 1 for `key:on`, else 0 | `key:%s` |
| +0x10 | char ESSID[0x80]: the line from offset 6 with its last character (the newline) removed — **the double quotes of `ESSID:"…"` are kept** | `ESSID…` |
| +0x90 | u8 channel | `Channel:%hhd` |
| +0x91 | char BSSID[0x27] (MAC text) | second `%s` of the Cell line |
| +0xB8 | u32 mode: 4 `master`, 5 `slave`, 6 anything else — compared case-sensitively, so iwlist's `Master` / `Managed` give 6 [I: iwlist spelling] | `Mode:%s` |
| +0xBC…+0xDB | not written | |

Count: a `Cell` line starts the next cell; lines before the first cell are dropped; parsing stops at EOF or when the cell
index reaches capacity − 1 (so at most 63 cells). The stored count is the **index** of the last cell, i.e. **one less than
the cells parsed** at EOF (one network → count 0 and `0xA01C201F`; two → 1), and 63 when the capacity stop hits [R: 0x16360
`uxtb r7, r4` before each `fgets`, `strb r7` at 0x16406]. MI_WLAN_Scan ignores the parser's return value (§8.2).

**STA `Status`** (GetStatus on a station handle, 0x12C bytes, zeroed, then copied out whole):

| off | field | source (`status.infra` output, `key=value` lines) |
|---|---|---|
| +0x00 | char BSSID[0x30] | `bssid=%s` |
| +0x30 | s32 frequency (MHz) | `freq=%d` |
| +0x34 | char SSID[0x84]: the line from offset 5, last character removed | `ssid=` |
| +0xB8 | u32 mode: 0 `station`, 1 anything else | `mode=%s` |
| +0xC0 / +0xC4 | u32 group / pairwise cipher bits: 8 = contains `TKIP`, 0x10 = contains `CCMP` | `group_cipher=%s`, `pairwise_cipher=%s` |
| +0xCC | char key_mgmt[0xC] | `key_mgmt=%s` |
| +0xD8 | u32 state: DISCONNECTED 0, INTERFACE_DISABLED 1, INACTIVE 2, SCANNING 3, AUTHENTICATING 4, ASSOCIATING 5, ASSOCIATED 6, GROUP_HANDSHAKE 8, COMPLETED 9, any other text 7 (= wpa_supplicant's `enum wpa_states`, where 7 is 4WAY_HANDSHAKE [I]) | `wpa_state=%s` |
| +0xDC | char own MAC[0x30] | `address=%s` |
| +0x10C | char IP[0x14] | `ip_address=%s` |
| +0x120 | s32 signal 0…100 | `/proc/net/wireless` (below) |
| +0xBC, +0xC8, +0x124…+0x12B | not written | |

Signal: the interface name is the token after `-i` in the composed status command line (no `-i` → `strtok(NULL, …)`
continues an earlier tokenisation [I: undefined result]); the matching `<if>:` line of `/proc/net/wireless` is found
(leading blanks skipped) and its third value after the colon (the level, which must contain a `.`) is read as dBm:
≤ −100 → 0, −99…−51 → 2·(dBm + 100), ≥ −50 → 100. No match or no dot → 0.

**AP `Status`** (GetStatus on `AP_HANDLE`, 0x4C08 bytes): 64 station records × 0x130, then +0x4C00 u16 count. Command =
`status.ap.all_sta` + the second awk filter (§8.2). Output lines are scanned for `dot11RSNAStatsSTAAddress` then
`connected_time`, alternately, one station per pair; EOF ends it; the bound (64) is checked **after** a record is written,
so a 65th station is written past the array [R; I: needs > 64 stations]. Record: +0x000 char host name[0x100], +0x100 char
IP[0x10], +0x110 char MAC[0x18] (`dot11RSNAStatsSTAAddress=%s`), +0x128 s64 connected time in s (`connected_time=%lld`).
Then the lease file `universal.dhcp-leasefile` (missing key → `0xA01C2006`; open failure → `0xA01C2011`; the station part
is copied out either way) is read line by line, split at blanks: token 2 is compared with each station's MAC; on a match
token 3 → IP, token 4 → host name — the dnsmasq lease line `<expiry> <mac> <ip> <name> <client-id>` [I: format].

## 9. libcjson, libg711, libg726 — stock code [R unless tagged]

### 9.1 libcjson.so = cJSON **1.7.7**
- `cJSON_Version()` returns `sprintf("%i.%i.%i", 1, 7, 7)`. The 74 exports are exactly the CJSON_PUBLIC set of 1.7.7:
  compared with cJSON 1.7.18 the only names missing here are `cJSON_GetNumberValue`, `cJSON_ParseWithLength`,
  `cJSON_ParseWithLengthOpts`, `cJSON_SetValuestring` — all added after 1.7.7; nothing extra. The `.rodata` strings (`%1.15g`,
  `%lg`, `%1.17g`, `u%04x`, `null`, `false`, `true`) and the `cJSON` node layout (`valuestring` at +0x10) are upstream's.
- Upstream `cJSON.c` v1.7.7 built with Thumb-2 `-fPIC -fstack-protector-strong` gives at `-O0` a `.text` of 0x34ec against
  the vendor's 0x374c (gcc 15 against gcc 9.1); every other optimisation level is ≥ 30 % smaller — so the vendor library is
  **upstream 1.7.7 at -O0**, with no sign of local changes [I from size, symbol set and strings; no instruction-level diff].
  Licence MIT [S: upstream].
- Its only importer is libmi_wlan (§8).

### 9.2 libg711.so = Sun Microsystems `g711.c` (public domain) + three wrappers
- Internals [R decompile + disassembly]: `linear2alaw`, `alaw2linear`, `linear2ulaw` (bias 0x84), `ulaw2linear`,
  `alaw2ulaw`, `ulaw2alaw`, the single 16-bit segment table `seg_end = {0xFF, 0x1FF, 0x3FF, 0x7FF, 0xFFF, 0x1FFF, 0x3FFF,
  0x7FFF}` (`.data` 0x11130) and Sun's 128-byte `_u2a` (0x11030) / `_a2u` (0x110b0) tables (exported). This is the older Sun
  variant that works on 16-bit linear directly (no 13 / 14-bit pre-shift, no µ-law clip).
- **One deviation from Sun's source**: `alaw2linear` has no `switch (seg)` — it always computes
  `t = (((a ^ 0x55) & 0xf) << 4) + 0x108; t <<= seg − 1` (16-bit). For segment 0 the shift count is −1, which ARM's register
  shift takes as 255 → **every segment-0 A-law code decodes to 0** instead of ±(t + 8) [R disassembly at 0x590]. µ-law
  decoding is correct. −32768 input to `linear2ulaw` overflows the 16-bit negate [I].
- Exports: `G711Encoder(const s16 *pcm, u8 *out, int n, int law)`, `G711Decoder(s16 *pcm, const u8 *in, int n, int law)`,
  `G711Covert(u8 *out, const u8 *in, int n, int dir)` (sic): `law` / `dir` 0 = A-law (Covert: A→µ), non-zero = µ-law
  (Covert: µ→A); one byte per sample, no return value. Identical file in three other firmware dumps (§1.1).

### 9.3 libg726.so = Sun G.72x reference codec (+ 16 kbit/s) with a bit-packing wrapper
- State struct (caller-allocated, ≥ 0x4c bytes) [R offsets]: +0 bit rate, +4 bits per code (2 / 3 / 4 / 5), +8 Sun
  `g72x_state` {`long yl` = 34816 (0x8800); `short yu` = 544 (0x220), `dms`, `dml`, `ap`, `a[2]`, `b[6]`, `pk[2]`, `dq[6]` =
  32, `sr[2]` = 32; `char td`} — Sun's `g72x_init_state` values exactly; +0x3c u32 bit buffer, +0x40 bit count, +0x44
  encoder, +0x48 decoder function pointer.
- `g726_init(state, rate)`: rate ∉ {16000, 24000, 32000, 40000} → returns NULL; else initialises, picks the 2 / 3 / 4 /
  5-bit coder pair, clears the bit buffer, returns `state`.
- `g726_encode(state, u8 *out, const s16 *pcm, int nsamples)`: per sample `code = enc(state, pcm >> 2)` (14-bit linear);
  `buf = buf << bits | code`, bytes emitted **MSB first** whenever ≥ 8 bits are pending; leftover bits stay in the state for
  the next call (never flushed); returns bytes written.
- `g726_decode(state, s16 *pcm, const u8 *in, int nbytes)`: refills the bit buffer a byte at a time, takes `bits` from the
  top, `pcm = dec(state, code)`; returns samples written.
- Tables in `.rodata` (int arrays) [R values]: 16 kbit `dqlntab {116, 365, 365, 116}`, `witab {−704, 14048, 14048, −704}`,
  `fitab {0, 0xE00, 0xE00, 0}`, `qtab {261}`; 24 kbit = Sun `g723_24.c`; 32 kbit = Sun `g721.c` (`witab` … 35904); 40 kbit =
  Sun `g723_40.c`. The 16 kbit tables are the G.726 2-bit values also used by spandsp [I]. Same file in the anjoy dump.

## 10. What is not known
- **libmi_divp**: what the `StretchBufEx` word at payload +0x68 selects (§3) — it would take a kernel `mi_divp.ko`, which is
  not on the camera and in none of the downloaded trees.
- **libmi_vpe**: the field names inside `ChannelPara` (0x1c), `PortMode` (0x10), `ZoomAttr` (0xc) (§5.5) — sizes are known,
  no mi_vpe header is on hand; the entry size of the zoom table.
- **libmi_wlan**: the field sizes of §8.8 are distances between written offsets [I]; no header confirms them.
- **libmi_vdf**: "never written" for the check-point / run counters of `g_stVdfProcDebug` (§7.7) rests on the absence of any
  store with those offsets; a store through a computed address was not ruled out exhaustively.
- **libcjson**: no instruction-level comparison with upstream 1.7.7 exists (§9.1).
- None of the nine libraries has a user on the camera, so none of their behaviour has been observed at run time.
