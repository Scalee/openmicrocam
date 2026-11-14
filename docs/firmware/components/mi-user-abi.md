# mi-user-abi — the user-space ABI shared by the MI libraries
<!-- ev:mi-user-abi -->

The MI user libraries (`/config/lib/libmi_*.so`) are thin wrappers over the `/dev/mi_<mod>` character devices of SigmaStar's
MI kernel modules on the Mercury6 (M6) chip. This document holds what all of them share: the library set and its users,
the device fd and its lifetime, the 16-byte ioctl wrapper, the `'i'` numbering and payload packing, return values and the MI
error-code format with its module ids, poll fds on `/dev/mi_poll`, the debug-level reader; and the three support libraries
in full: `libcam_os_wrapper.so` (user-space CamOs, 160 exports), `libcam_fs_wrapper.so` (user-space CamFs, 5 exports) and
`libmi_common.so` (MI_OS layer, 40 exports). Two companion tables carry every export one row each.

Tags: **[R]** read from a binary, a decompile, disassembly, a header or a file; **[S]** stated by a source; **[I]**
inference. A heading tagged [R] means every statement under it is [R] unless marked otherwise.

Binaries: `/config/lib/*.so` (the camera's `/config/lib`; the same code as the static copies linked into prog_uvc,
`prog_uvc.md` §1.3).

Shared facts owned elsewhere: the kernel half of the wrapper (`MI_DEVICE_Register` / `MI_DEVICE_Ioctl`), per-module procfs
and debug parameters, the libmi_sys exports outside the shared table, `/dev/mi_poll`'s kernel side —
`mi_sys-2-interfaces.md` §1, §10, §5, §4; kernel-side CamOs — `kernel-platform.md` §13; each module's own ioctl table, payloads and
user-space logic — the module documents (`mi-capture.md`, `mi_venc.md`, `mi_disp-mi_hdmi.md`, `mi_jpd-mi_vdec.md`,
`audio.md`, `ldc-1-stack.md`); the unused SDK libraries of the 2020 generation — `sdk-orphan-libs.md`.

Struct names are those of the public M6 / Tiramisu headers (Xsyt `m6/MI/include`, `skyipc_tiramisu`; the ISP device
API from divinus `m6_isp.h`); sizes are what the ioctl carries [R].

## 1. The libraries and their users [R]

### 1.1 The set
The used MI libraries and the support libraries, with their dynamic exports (functions + data objects):

| library | exports | specified in |
|---|---|---|
| libmi_sys | 47 + 3 data (`g_mi_fd_table`, `g_last_fd_index`, `g_mi_fd_semalock`, §2.1) | shared mechanics here; the 34 outside the table of §10: `mi_sys-2-interfaces.md` §5 |
| libmi_sensor | 19 | `mi-capture.md` §2.7 |
| libmi_vif | 16 | `mi-capture.md` §3.16 |
| libmi_isp | 402 + 2 data (`fpSetIspApiData`, `fpGetIspApiData`) | `mi-capture.md` §5 |
| libmi_scl | 15 | `mi-capture.md` §6.13 |
| libmi_venc | 78 | `mi_venc.md` §13 |
| libmi_disp | 58 | `mi_disp-mi_hdmi.md` §5 |
| libmi_hdmi | 16 | `mi_disp-mi_hdmi.md` §3 (EDID parser `MI_HDMI_GetSinkInfo`, 13.7 KB of code) |
| libmi_jpd | 14 | `mi_jpd-mi_vdec.md` §1.12 |
| libmi_vdec | 33 | `mi_jpd-mi_vdec.md` §2.20 |
| libcam_os_wrapper | 160 + 1 data (`cam_os_wrapper_version_string`) | §7 |
| libcam_fs_wrapper | 5 | §8 |
| libmi_common | 40 | §9 |

libmi_ldc, libmi_ai and libmi_ao follow the same mechanics; their wrapper rows are machine-extracted, their logic is in `ldc-1-stack.md` §3 (libmi_ldc: `MI_LDC_GetChnParam`, ioctl 0xA plus a conversion;
`MI_LDC_DestroyChannel` maps every failure to `0xA017201F`; fd close: §2.2) and
`audio.md` §2.6, §2.4 (libmi_ao's per-channel 0xC800-byte user-space queue in front of the MHAL DMA ring; `MI_AO_SendFrame` accepts
channel 0 only; `MI_AI_GetFrame`).

Build: every MI library of this firmware carries the same commits, `project_commit.7251436 sdk_commit.80327e1`, built
2022-07-21; libmi_common's exported `sstar_mi_so_version_string` is `"Sigmastar Module mi_common version: project_commit.7251436
sdk_commit.80327e1 build_time.20220721223312"`. libcam_os_wrapper is older (§7.1).

### 1.2 Users on the camera
<!-- ev:mi-user-abi.1.2 -->
- **prog_uvc** is the only program that calls MI functions; kjd_lvgl calls none. It calls 155 MI / CUS3A functions (the list
 of; the rows of the table of §10) and seven VENC functions outside that
  list (H.264 / H.265 slice split get / set, input source config, max stream count, JPEG parameters — `mi_venc.md` §5.1, §13). Against
  an independent naming of prog_uvc's ioctls 121 function / ioctl pairs agree; the other direct ones (ISP
  SetChnParam / StopChannel, SYS GetBuf / PutBuf(Pa), AI EnableChn) are unnamed there and are named here from
  the `.so` exports; HDMI Init / DeInit (`_IO`, §3.1) and the IQ / AE / AWB calls (indirect, `mi-capture.md` §5) are absent
  there by construction. libmi_ldc, libmi_ai and libmi_ao are not reached with the shipped command line.
- **The kept blobs** libcus3a and libispalgo (and the unused libsigma3a, §1.3) call back into the MI libraries: `MI_ISP_CUS3A_*`,
  `MI_ISP_AE_Get*HwStats` / `MI_ISP_AWB_Get*HwStats`, `MI_ISP_GetIspRoot` (libmi_isp, `mi-capture.md` §5.4) and
  `MI_SYS_MMA_Alloc` / `MI_SYS_MMA_Free` / `MI_SYS_Mmap` / `MI_SYS_Munmap` — their only libmi_sys imports
  (`mi_sys-2-interfaces.md` §5). `MI_ISP_GetIspRoot` sends ISP API id 0x66, which neither libcus3a nor the kernel accepts; the
  call fails and libcus3a uses its fallback directory [I from both routing tables; `mi-capture.md` §5.3]. libmi_iqserver
  imports libmi_isp and libmi_sys exports as well. libcus3a (46 exports), libispalgo (345), libfbc_decode (320) and
  libmi_iqserver (48, the IQ-tool server on TCP 9876) are specified in `isp-6-userspace.md` §2, §7, §6 (libispalgo: `isp-5-3a.md` §1.2).
- **No ELF outside `/config/lib` imports CamOs**; prog_uvc carries 16 statically linked CamOs functions
 . What the libraries import from libcam_os_wrapper: §7.7.

### 1.3 The other libraries in `/config/lib` (unused)
Not loaded by anything on the camera: the SDK libraries libmi_divp, libmi_gfx, libmi_gyro, libmi_ipu, libmi_ive,
libmi_mipitx, libmi_panel, libmi_rgn, libmi_sed, libmi_shadow, libmi_vdf, libmi_vdisp, libmi_vpe, libmi_wlan; the algorithm
libraries libAEC, libAED, libAPC, libBF, libMD, libOD, libSRC, libSSL, libVG (`_LINUX` builds) and libsigma3a; libcjson,
libg711, libg726 (`firmware-image.md` §3.5). libmi_divp (`/config/lib/libmi_divp.so`) is unused because no `mi_divp.ko`
provides `/dev/mi_divp`. The libraries are not self-contained: libmi_sed needs libmi_ive and libmi_vdf, libmi_vdf needs libmi_shadow and the MD / OD / VG algorithm
libraries, none through `NEEDED`; libmi_wlan imports cJSON. libmi_vpe calls `MI_ISP_EnableUserspace3A(chn, &pair)`, which
does not match the camera's libmi_isp signature `(dev, chn)`. libcjson is cJSON 1.7.7, libg711 Sun's `g711.c` (its
`alaw2linear` decodes A-law segment 0 to 0), libg726 Sun's G.72x with the 16 kbit/s tables. Their specification:
`sdk-orphan-libs.md`, `audio-algorithms.md`, `ive-video-analytics.md`.

## 2. The device fd [R]

### 2.1 Open: one fd per library, opened lazily
Each library has a static `{int fd = -1; pid_t pid}` and one helper (`_MI_WRAPPER_DEVICE_Open`, source file `<mod>_api.c`)
called before **every** ioctl:
1. if `fd != -1 && pid == getpid()` → return fd (after `fork()` the child reopens).
2. `CamOsTsemDown(&g_mi_fd_semalock)`; `fd = open("/dev/mi_<mod>", O_RDWR)` — `<mod>` = sys, sensor, vif, isp, scl, venc,
   disp, hdmi, jpd, vdec, ldc, ai, ao. The strings `/dev/mi/<mod>` also exist in each `.so`; the open path does not use them
   (the close helper prints them, §2.2).
3. if `fd > 0`: if `g_last_fd_index >= 0` and `g_mi_fd_table[g_last_fd_index] > fd`, replace fd by
   `fcntl(fd, F_DUPFD, g_mi_fd_table[g_last_fd_index])` and close the original (so each new MI fd is numbered above the
   previous one); `assert(g_last_fd_index <= 0x22)` (35 slots); `g_mi_fd_table[++g_last_fd_index] = fd`.
4. `pid = getpid()`; set `FD_CLOEXEC`; `CamOsTsemUp`.
5. on open failure the helper prints `failed to open` and returns −1; the ioctl that follows fails with `EBADF` → −1 (§4).

`g_mi_fd_table[35]`, `g_last_fd_index` (initial value −1 in `.data`) and `g_mi_fd_semalock` (a `CamOsTsem_t`) are **defined in
libmi_sys.so** and imported by all the other MI libraries. The semaphore is initialised only by `MI_SYS_Init`; before that
`CamOsTsemDown` only prints an error and does not lock (§7.3).

This helper is the one of the 2022 libraries prog_uvc uses. The 2020 wrapper generation also present on the camera
(libmi_divp, libmi_gyro, libmi_vpe, §1.3) has a private fd / pid pair, no lock and no shared table.

The "InitDev" exports (`MI_DISP_InitDev`, `MI_SNR_InitDev`, `MI_HDMI_InitDev`, …) only open the fd: they return 0, or **0x1F**
— a bare number, not an MI code — when the open fails. `MI_HDMI_InitDev` also sends ioctl 0 and starts the hot-plug poll
thread (`mi_disp-mi_hdmi.md` §3.2).

### 2.2 Close: per-library create counts
Eight libraries close their fd, through one shared close helper (the same code in each, source file `<mod>_api.c`): under
`g_mi_fd_semalock`, if the cached fd is ≥ 0 and was opened by this pid → `close(fd)`; on success, if
`g_mi_fd_table[g_last_fd_index] == fd` the top slot is set to −1 and `g_last_fd_index--` (only the top slot is ever released,
so a close out of order leaves a stale entry in the table); cached fd = −1. Failure prints `failed to close %s, fd is
Invalid !` / `failed to close %s!(%s)`, with `/dev/mi/<mod>` as the name.

| library | close happens in | counter |
|---|---|---|
| libmi_isp | `MI_ISP_DestoryDevice` | `u16` count, +1 on `MI_ISP_CreateDevice` returning 0, −1 on `DestoryDevice` returning 0; close when it reaches 0 (also when the ioctl failed and the count is already 0) |
| libmi_scl | `MI_SCL_DestroyDevice` | same scheme with `MI_SCL_CreateDevice` |
| libmi_vif | `MI_VIF_DestroyDevGroup` | +1 on `CreateDevGroup` **and** `CreateDevGroupExt` |
| libmi_vdec | `MI_VDEC_DestroyDev` | +1 on `MI_VDEC_CreateDev` |
| libmi_venc | `MI_VENC_DestroyDev` | +1 on `MI_VENC_CreateDev` (dev < 10) |
| libmi_disp | `MI_DISP_DeInitDev` | none: closes unconditionally |
| libmi_sensor | `MI_SNR_DeInitDev` | none |
| libmi_hdmi | `MI_HDMI_DeInitDev` | none (also clears the hot-plug poll thread's run flag) |

libmi_sys and libmi_jpd never close their fd. libmi_ldc closes it in `MI_LDC_DestroyDevice` at the last device.

### 2.3 What the kernel does on close
The kernel modules act on release of the fd: closing the last `/dev/mi_sensor` fd powers the sensor down, closing
`/dev/mi_hdmi` switches HDMI off, closing any `/dev/mi_venc` fd destroys all encoder channels (`mi-capture.md` §2.1,
`mi_disp-mi_hdmi.md` §2.1, `mi_venc.md` §1.1).

## 3. The ioctl call [R]

### 3.1 The argument is a 16-byte wrapper, not the struct
```c
struct mi_ioctl_wrap {          /* what the 3rd ioctl() argument points to */
    u32 u32Len;                 /* +0  payload size == _IOC_SIZE(cmd) */
    u32 pad;                    /* +4  left uninitialised by the libraries */
    u64 u64Ptr;                 /* +8  user pointer to the payload, sign-extended from 32 bits */
};
```
`cmd` = `_IOC(dir, 'i', nr, sizeof(payload))` (magic `'i'` = 0x69); the size field describes the **payload**, not the
wrapper. dir: `_IOW` = the payload is only read by the kernel, `_IOR` = only written, `_IOWR` = both. The kernel side
copies the wrapper in (`copy_from_user`), then `u32Len` bytes from `u64Ptr`, and copies back for `_IOR` / `_IOWR`
(`mi_sys-2-interfaces.md` §1.3). Exceptions: `MI_HDMI_Init` / `DeInit` use `_IO` with argument 0 (no wrapper: `ioctl(fd, 0x6900, 0)` /
`ioctl(fd, 0x6901, 0)`); `ioctl(pollfd, 0, id)` on `/dev/mi_poll` passes a value (§5).

### 3.2 Payload = the function's arguments packed in order
- Scalar arguments become u32 slots (u8 / BOOL take one byte, the rest of the 4-byte slot is zeroed); u64 arguments are
  8-aligned; a pointer-to-struct argument is copied **by value** into the payload.
- Output structs are copied from the payload back to the caller **only when the ioctl returned 0**. Exceptions are marked
  in the tables: "copy-back: always" = copied even when the ioctl failed, i.e. stale stack data after −1
  (`MI_SYS_ChnOutputPortGetBuf`, `output` column of the table of §10); "copy-back: unless −1" = copied whenever the syscall did not fail, an MI error code included (`MI_SCL_GetChnParam`,
  `MI_SCL_GetInputPortCrop`, `MI_VDEC_GetUserData`).
- Functions whose only argument is an output struct (`MI_SYS_GetVersion`, `MI_SYS_GetCurPts`) point the wrapper straight at
  the caller's buffer.
- Arguments that are not sent: the `u16SocId` first argument of the MI_SYS calls (M6 header) — `MI_SYS_Init` sends a
  constant 0, the bind and depth calls drop it; the device id of `MI_VDEC_CreateDev` / `MI_VDEC_CreateChn` (libmi_vdec never
  sends it); the attribute of `MI_LDC_CreateDevice`.
- The plain wrappers do **no argument checks in user space** (libmi_jpd is the exception); where a library writes a payload
  field itself, its row in the tables says so — e.g. the `+0x120 u32 flags` word of sys ioctl 0x13:
  `MI_SYS_ChnOutputPortGetBuf` writes `0xA0000000` (bit 31 = map the buffer into the process),
  `MI_SYS_ChnOutputPortGetBufPa` (GetBufPa) writes 0 (`mi_sys-2-interfaces.md` §3.3; the payload's `+0x11c` and
  `+0x124..+0x127` are not named). The functions with their own user-space logic are in the module documents.

The rule holds for every simple wrapper of the 13 libraries; the exceptions are the rows of the tables that say otherwise.

### 3.3 Numbers per device used by prog_uvc's calls
All type `'i'`; nr ranges overlap between devices. nr and payload size in brackets:

| device | nr used by prog_uvc's calls (payload size) |
|---|---|
| mi_sys | 0 Init(4) 1 Exit(4) 3 BindChnPort2(0x30) 4 UnBind(0x30) 6 GetVersion(0x80) 7 GetCurPts(8) 8 InitPtsBase(8) 9 SyncPts(8) 0x13 OutputPortGetBuf(0x128) 0x14 PutBuf(4) 0x15 SetDepth(0x18); poll: 0x17 (0x14) 0x18 CloseFd(4) 0x19 (0x14) 0x1a (0x14) |
| mi_sensor | 0 Enable 1 Disable 2 GetPadInfo(0x38) 3 GetPlaneInfo(0x50) 8 QueryResCount(8) 9 GetRes(0x3c) 0xb SetRes(8) 0xe SetPlaneMode(8) |
| mi_vif | 0 CreateDevGroup(0x20) 2 Destroy 4 SetDevAttr(0x18) 6/7 Enable/DisableDev 9 SetOutputPortAttr(0x1c) 0xb/0xc En/DisOutputPort 0xd Query(0x24) |
| mi_isp | 0 CreateDevice(8) 1 Destroy 2 CreateChannel(0x58) 3 DestroyChannel 5 GetInputPortCrop(0x10) 6 SetChnParam(0x18) 8/9 Start/Stop 0xa SetOutputPortParam(0x18) 0xc/0xd En/DisOutputPort(0xc) **0x10 SetIQApiData(0x1c) 0x11 GetIQApiData(0x1c)** |
| mi_scl | 0 CreateDevice(8) 1 Destroy 2 CreateChannel(0xc) 3 Destroy 6 SetChnParam(0xc) 8/9 Start/Stop 0xa/0xb Set/GetOutputPortParam(0x24) 0xc/0xd En/DisOutputPort |
| mi_venc | 2 CreateChn(0x54) 3 DestroyChn 5 StartRecvPic 7 StopRecvPic 8 Query(0x30) 0xb GetStream(0x54) 0xc ReleaseStream(0x50) 0x10 RequestIdr(0xc) 0x14 GetFd(0xc) 0x42 GetRingPoolHandle(0xc) 0x43 RingPool_GetRingPoolInfo(0x18) 0x45 RingPool_GetHeadRecord(0x50) 0x48 RingPool_ReleaseHeadRecord(0xc) |
| mi_disp | 0/1 Enable/Disable 2 SetPubAttr(0x38) 6/7 En/DisVideoLayer 8/9 Set/GetVideoLayerAttr(0x18) 0xa/0xb (Un)BindVideoLayer 0x14 SetInputPortAttr(0x14) 0x16/0x17 En/DisInputPort 0x1a/0x1b Pause/Resume 0x1f SetInputPortSyncMode(0xc) 0x2a ClearInputPortBuffer(0xc) |
| mi_hdmi | 0 Init(_IO) 1 DeInit(_IO) 2/3 Open/Close 4/5 Set/GetAttr(0x34) 6/7 Start/Stop 9 ForceGetEdid(0x20c) |
| mi_jpd | 2 CreateChn(0x18) 3 DestroyChn 5/6 Start/Stop 9 GetStreamBuf(0x40) 0xa PutStreamBuf(0x30) 0xb Drop(0x30) 0xe (0x18) 0xf (0xc) |
| mi_vdec | 0 CreateDev(4) 2 CreateChn(0x28) 5 StartChn(4) 7 GetChnStat(0x24) 0xc SendStream(0x28) 0x14/0x15 Set/GetOutputPortAttr(8) |
| mi_ldc, mi_ai, mi_ao | rows of the table of §10 (not reached with the shipped command line) |

The ioctls of the exports prog_uvc does not call, with the kernel names of every nr, are in the module documents.

## 4. Return values and error codes [R]

### 4.1 Return convention
- `ioctl()` returns −1 (syscall-level failure) → the library prints `[func] failed to ioctl 0x%08lx!(strerror)` and returns
  **−1**.
- Otherwise the ioctl's return value **is** the `MI_S32` result: 0 = OK, else an MI error code. MI codes are large negative
  ints outside glibc's −4095..−1 errno window, so they pass through `ioctl()` unchanged; the kernel's `unlocked_ioctl`
  returns the MI code itself, not −errno [I, from the libraries testing `ret != 0` separately from `ret == -1`; kernel side:
  `mi_sys-2-interfaces.md` §1.3].
- Some failure paths in all modules return small non-MI values (1, 3, 0x1F); the libraries pass them through as the result.
  The InitDev exports return a bare 0x1F when the open fails (§2.1); `MI_OS_ThreadCreate` returns 12 (§9).

### 4.2 Code format and module ids
<!-- ev:mi-user-abi.4.2 -->
`0xA0000000 | module<<16 | level<<12 | err` (`MI_DEF_ERR`, `mi_common_datatype.h`); level 2 = ERROR (0x2000) in most codes.
The codes `0xA0078008`, `0xA0078012` and `0xA00F8012` carry 8 in the level field [R values]. `0xA0078008` (err 8) is
NOT_SUPPORT; `0xA00F8012` is recorded as DISP's not-supported code, though err 0x12 is BUSY in the table
of §4.3 — which meaning the level-8 codes carry is not settled.

| module id | module | codes seen |
|---|---|---|
| 0x02 | VENC | `0xA002…` (`0xA0022001/2/3/6/9`, `0xA002201F`) |
| 0x06 | VIF | |
| 0x07 | ISP (its mi_sys module id is 0x21; the codes carry 0x07) | `0xA007…`; `0xA0078008` = the ISP agent's "not mine" answer (`mi-capture.md` §5.2); `0xA0078012` from `MI_ISP_CallBackTask_*` |
| 0x08 | VDEC | |
| 0x09 | SYS | `0xA009201F` (poll-fd helpers, §5) |
| 0x0B | HDMI | `0xA00B21E8` (`MI_HDMI_GetSinkInfo`, `mi_disp-mi_hdmi.md` §3.3) |
| 0x0F | DISP | `0xA00F8012` (not supported) |
| 0x10 | libmi_common | `0xA010201F` FAILED, `0xA0102019` timeout (§9) |
| 0x17 | LDC | `0xA017201F` (`MI_LDC_DestroyChannel`) |
| 0x1B | SNR | |
| 0x20 | JPD | |
| 0x22 | SCL | |

### 4.3 Error numbers
Common `MI_ErrCode_e`, with the uses the libraries themselves make of them:

| err | name | use in the libraries |
|---|---|---|
| 1 | INVALID_DEVID | |
| 2 | INVALID_CHNID | |
| 3 | ILLEGAL_PARAM | |
| 4 | EXIST | `CreateChn` of VENC and JPD map it to 0 |
| 6 | NULL_PTR | |
| 8 | NOT_SUPPORT | |
| 9 | NOT_PERM | VENC mutex failure |
| 0x0C | NOMEM | |
| 0x0F | BUF_FULL | JPD: libmi_jpd sleeps 30 ms and returns it to the caller (`mi_jpd-mi_vdec.md` §1.12) |
| 0x10 | SYS_NOTREADY | |
| 0x12 | BUSY | VENC `StopRecvPic`, retried |
| 0x1F | FAILED | |

**SCL uses its own enum** (M6 `mi_scl_datatype.h`): 0 INVALID_DEVID, 1 INVALID_CHNID, 2 INVALID_PORTID, 3 ILLEGAL_PARAM,
4 EXIST, 5 UNEXIST, 6 NULL_PTR, 7 NOT_SUPPORT, 8 NOT_PERM, 9 NOMEM, 0xA NOBUF, 0xB BUF_EMPTY, 0xC NOTREADY, 0xD BUSY.

## 5. Poll fds on `/dev/mi_poll` [R]

`/dev/mi_poll` (a character device created by `mknod` in demo.sh, `rootfs-customer.md` §3.2; kernel side
`mi_sys-2-interfaces.md` §4) gives one fd per waitable object: the owning module hands out an id and the poll fd is bound to
it with `ioctl(pollfd, 0, id)` (cmd 0, **the argument is the id by value**). The application then `select()`s / `poll()`s
the fd.

- **`MI_SYS_GetFd(ChnPort*, s32 *fd)`**: sys ioctl 0x19 `{ChnPort (0x10), s32 fd}` — if it returns 0 an fd is already
  registered → return it. Else `open("/dev/mi_poll", O_RDWR)`; sys 0x17 `{ChnPort, u32 id (out)}`; `ioctl(pollfd, 0, id)`;
  sys 0x1a `{ChnPort, s32 pollfd}` registers it; return pollfd. Any failure → close, return `0xA009201F`.
- **`MI_SYS_CloseFd(fd)`**: sys 0x18 `{s32 fd}`, then `close(fd)`; a close error ORs `0xA009201F` into the result.
- **`MI_VENC_GetFd(dev, chn)`**: cached per channel; else open `/dev/mi_poll`, venc 0x14 `{dev, chn, u32 id (out)}`,
  `ioctl(pollfd, 0, id)`, cache and return the fd (−1 on failure; the return is the fd, not an MI code). prog_uvc does not
  poll it; it calls `MI_VENC_GetStream(…, 40 ms)`.

## 6. Debug level [R]

`_MI_PRINT_GetDebugLevel(name)` (exported by libmi_sys, imported by libmi_venc, vdec, jpd, hdmi, ai, ao, gfx, ldc, shadow)
reads an integer from `/proc/mi_modules/mi_<name>/debug_level` (0 if the file is missing). The libraries gate their own
error prints on it (≥ 1 errors, ≥ 5 info such as HDMI's `Thread alrady running...`); it has no effect on behaviour. The
default `debug_level` of `mi_hdmi` on the running camera (`/proc/mi_modules/mi_hdmi/debug_level`) is not known (it
decides whether libmi_hdmi's ERR prints appear in its log).

## 7. libcam_os_wrapper.so — user-space CamOs (160 exports) [R]

### 7.1 Identity and build
- Version string `cam_os_wrapper_version_string` (exported data) = `"v1.0.37"`; `CamOsVersion()` returns the module string
  `"Sigmastar Module version: kernel_commit.ec6ea55 build_time.20201214193424 wrapper.v1.0.37"`.
- `NEEDED`: libdl, libpthread, libgcc_s, libc, ld-linux-armhf. Built from `cam_os_wrapper.c` with `CAM_OS_LINUX_USER`.
  Sources of the same file family: OpenIPC/linux `sigmastar-infinity6e` `drivers/sstar/cam_os_wrapper/` (v1.0.35) and
  `…infinity6c/…` (v2.0.43). The i6c `HISTORY` lists v1.0.36 (LPAE `CamOsPhysAddr_t`, `CamOsMemPhysToMiu` / `MiuToPhys`, drop
  `CamOsPhyMemMap` / `UnMap`) and v1.0.37 (`CamOsCondition###` macros, header only).
- The binary is **v1.0.35 plus the v1.0.36 changes**, with `CamOsTcond*` still present (removed in v1.0.39) [R exports, I
  version placement], together with the IDR functions of `src/cam_os_informal_idr.c` (exported `_CamOsIdr*`).
- The kernel's CamOs of this firmware is a different, later version (`kernel-platform.md` §13).

### 7.2 Agreement with the source
Of the 160 exports, **77 decompile identically** (after normalising addresses and local names) to the i6e or the i6c build
of the extracted user branch. The rest behave as the source except for the differences of §7.4; the remaining differences
are compiler-only (fortified `__*_chk` calls in the reference build, control-flow shape, `__isoc99_` vs `__isoc23_` scanf).
The classes of the exports are (`identical to source (i6e)`, `identical to
source (i6c)`, `source (i6c / informal_idr)`, `source behaviour, compiler-only diff`, `differs from source (mi-user-abi.md
§7.4)`). Of the functions classed "source behaviour, compiler-only diff", `CamOsHexdump`,
`CamOsListSort`, `_CamOsFindFirstZeroBit` and the 19 `CamOsAtomic*` were read line by line against the disassembly (§7.6);
of the others about two thirds were read side by side with the source, the rest are judged from the normalised decompile
diff only. Behaviourally the library is the i6e v1.0.35 user branch (`cam_os_wrapper.c` + `cam_os_informal_idr.c`) with
the §7.4 differences [I from the comparison].

### 7.3 Objects, magic, return codes, time and threads
- Every lock-like object starts with `u32 magic = 0x55AA5AA5` followed by the native object. Init refuses an object already
  carrying the magic (`already inited`, −1); NULL → −2. Lock / Unlock / Down / Up on an object without the magic either
  lazily inits (Mutex, Spin) or prints `not inited` and fails (Tsem, Rwsem, Tcond) — `CamOsTsemDown` then only prints and
  returns without locking (§2.1). Init of Mutex / Tsem / Rwsem / Tcond / Spin is serialised by one global pthread mutex in
  `.bss`. Deinit clears the magic.
- Callers embed these objects in their own state (the MI libraries, libcus3a), so the sizes are ABI. Bytes this library uses
  inside the caller's object (the public header reserves more: Mutex 11 words, Tsem 16, Rwsem 20, Tcond 20, Timer 8, Idr 20
  [R header]):

| object | layout used by this library | used size |
|---|---|---|
| CamOsMutex_t | magic, `pthread_mutex_t` (default attr) | 0x1c |
| CamOsTsem_t | magic, `sem_t` (`sem_init(pshared=1, value)`) | 0x14 |
| CamOsRwsem_t | magic, `pthread_rwlock_t` | 0x24 |
| CamOsTcond_t | magic, `pthread_mutex_t` at +4, `pthread_cond_t` at +0x20 (condattr clock = CLOCK_MONOTONIC) | 0x50 |
| CamOsSpinlock_t | magic, `pthread_spinlock_t` (`pthread_spin_init(…, 0)`) | 8 |
| CamOsAtomic_t | one `s32` (§7.6) | 4 |
| CamOsIdr_t | `{void **entries; u32 *bitmap; u32 entry_num}` | 0xc |
| CamOsListHead_t | `{next, prev}` (offsets 0 / 4) | 8 |
| thread handle (returned by ThreadCreate) | `{pthread_t; entry; arg; u32 tid; CamOsTsem_t entry_sem}` (calloc 0x24) | 0x24 |

- Return codes `CamOsRet_e`: 0 OK, −1 FAIL, −2 PARAM_ERR, −3 ALLOCMEM_FAIL, −4 TIMEOUT, −5 RESOURCE_BUSY (`EAGAIN` / `EBUSY`
  on trylock), −6 INTERRUPTED.
- Time: `CamOsMsSleep` rounds up to whole 10 ms (`usleep(((ms+9)/10)*10000)`); `CamOsUsSleep` = `usleep`;
  `CamOsGetTimeOfDay` / `SetTimeOfDay` convert µs ↔ ns (`CamOsTimespec_t` = {sec, nsec}); `CamOsGetMonotonicTime` =
  `clock_gettime(CLOCK_MONOTONIC)`; `CamOsGetTimeInMs` = monotonic ms; `CamOsTimeDiff` unit 0 s, 1 ms, 2 µs, 3 ns;
  `CamOsMsDelay` / `UsDelay` busy-wait on the monotonic clock; `CamOsTsemTimedDown` uses CLOCK_REALTIME + `sem_timedwait`,
  `CamOsTcondTimedWait` CLOCK_MONOTONIC; `CamOsThreadGetID` = `syscall(224 /*gettid*/)`.
- Thread priority (`CamOsThreadAttrb_t {u32 nPriority; u32 nStackSize; char *szName}`): 1..70 = nice level
  (`19 − p·19/50` for p < 51, else `50 − p`; applied with `setpriority` on the tid after the entry trampoline posts the entry
  semaphore, waited up to 1000 ms); 71..100 = SCHED_RR, priority `(p·93 − 6603)/23 + 1` for p < 95, else min(p, 99);
  0 = default. Stack size applied if non-zero; name via `pthread_setname_np` (≤ 15 chars).

### 7.4 Where the binary differs from the i6e v1.0.35 source
1. **Direct memory (`/dev/msys` + `/dev/mem`)** — `CamOsDirectMemAlloc(name, size, &va, &miu, &phys)`: one global list
   (lazily initialised under the global mutex) of `{list; u32 miu; u32 va; MSYS_DMEM_INFO *info; char *name; u32 len}` nodes
   (0x1c B). A name already in the list → −2 and all three outputs zeroed. Else open `/dev/msys` and `/dev/mem` (flags
   0x101002 = `O_RDWR|O_SYNC`), `malloc(0x40)` an **`MSYS_DMEM_INFO` of 0x40 bytes**: `+0 VerChk_Version = 0x4D530100,
   +4 char name[32] (snprintf "%s"), +0x24 u32 length, +0x28 u64 phys (out), +0x30 u64 kvirt, +0x38 u32 option,
   +0x3c VerChk_Size = 0x40` — the i6c layout (`name[32]`) with a 64-bit kvirt (i6e has `name[16]`, 0x30 B; i6c 0x3c B).
   ioctl `0x5310` (`IOCTL_MSYS_REQUEST_DMEM`, `_IO('S',0x10)`, argument = the struct pointer, no wrapper); if the kernel
   returns a smaller `length` than asked → `0x5311` RELEASE and fail. Then `0x5313` (`PHYS_TO_MIU`) with an
   `MSYS_ADDR_TRANSLATION_INFO {ver 0x4D530100; u64 addr = phys; VerChk_Size 0x10}` → MIU address; then
   `mmap(NULL, length, RW, MAP_SHARED, /dev/mem fd, phys)`. Outputs: `*va`, `*miu` (address after translation), `*phys`.
   A failure after REQUEST always releases with `0x5311`; success and failures are reported on stderr.
   `CamOsDirectMemRelease(va_or_miu, size)`: finds the node whose miu or va equals the argument, `munmap(va, size)` (the
   requested size if it equals the recorded length, else the recorded length), `0x5311` with the stored info, frees node and
   info. `CamOsDirectMemStat()` prints every node. `CamOsDirectMemPhysToMiu` / `MiuToPhys` and `CamOsMemPhysToMiu` /
   `MiuToPhys` = one `0x5313` / `0x5314` ioctl each on a fresh `/dev/msys` fd, 32-bit address (0 on failure). These functions
   are in the i6e source too, with the 0x30-byte struct and without the list node's `len` field.
2. **`CamOsMemMap(phys, size)`** = `mmap(NULL, size, RW, MAP_SHARED, open("/dev/mem"), phys)` directly; the source's
   `/dev/msys` MIU→phys ioctl and size check are absent. `CamOsMemUnmap` = `munmap` with an error print.
3. **`CamOsThreadCreate`** allocates a 0x24-byte handle (i6e: 0x50), layout in §7.3; the entry trampoline stores the tid,
   posts the entry semaphore, then tail-calls the user entry. `CamOsThreadGetName` accepts a NULL / zero handle and then names
   the calling thread (the i6e source refuses).
4. **Work queues** (`CamOsWorkQueueCreate` / `Destroy` / `Add` / `Cancel`; absent from the i6e user branch, present in
   i6c): the i6c design with a **CamOsTcond** instead of a CamOsTsem as the wake-up object, so the queue object is 0x78 B
   (i6c 0x38): `+0 Tcond, +0x50 thread handle, +0x54 work array (n × 0x20), +0x58 n, +0x60 u64 sequence, +0x68 spinlock,
   +0x70 atomic running`. Work slot (0x20): `+4 fn, +8 arg, +0xc state (0 free, 1 queued, 2 timer pending, 3 running),
   +0x10 u64 seq, +0x18 timer`. Worker thread (priority 50 attr, name = queue name): `TcondTimedWaitInterruptible(5000 ms)`,
   then runs queued slots in sequence order. `Add(wq, fn, arg, delay_ms)` with delay 0 queues and signals; with delay ≠ 0 it
   calls `CamOsTimerInit` / `TimerAdd`, which are **stubs in user space** (§7.5), so delayed work never runs. The locking
   calls `CamOsSpinLockIrqSave` / `SpinUnlockIrqRestore` are stubs too, so the queue is **unlocked** and prints a "not
   support" line on every operation. Unused: no ELF of the camera imports them.
5. **`CamOsPropertySet` / `Get(key, value[, default])`**: `dlopen("libat.so")`, `dlsym("property_set" / "property_get")`,
   call, `dlclose`; −1 if the library or symbol is missing (absent from the i6e user branch). libat.so is not on the camera
   (`firmware-image.md` §3.5).
6. **`CamOsCallStack()`**: `backtrace(64)`, prints `backtrace() returned %d addresses`, `backtrace_symbols`, `puts` each,
   `exit(1)` if the symbols cannot be built.
7. **Tcond family** (absent from i6c, which dropped it): `CamOsTcondInit` (condattr CLOCK_MONOTONIC), `Deinit`, `Signal`,
   `SignalAll`, `Wait`, `TimedWait(ms)` (−4 on ETIMEDOUT), `WaitInterruptible` = `Wait` returning 0,
   `TimedWaitInterruptible` = `TimedWait`; `TcondWaitActive` is a stub.
8. **Info readers** (same as the source): `CamOsChipId` parses `/sys/devices/virtual/mstar/msys/CHIP_ID` (`Chip_ID: 0x%X`);
   `CamOsChipRevision` parses `CHIP_VERSION` (`Chip_Version: %u`) **and returns value + 1**; `CamOsDramInfo` parses
   `/sys/devices/system/miu/miu0/dram_info` (type, size MB → `nBytes = MB << 20`, freq, miupll, data rate, bus width);
   `CamOsPhysMemSize` = log2(MB) as `CamOsMemSize_e`.

### 7.5 User-space stubs and plain libc mappings
The jiffies family (`CamOsGetJiffies`, `GetHz`, `JiffiesToMs`, `MsToJiffies`, `JiffiesDiff`), `CamOsMemPhysToVirt` /
`VirtToPhys` and `CamOsMemFromUserModeMap` / `Unmap` exist in the source only for the kernel; here they are stubs like the
rest of this list. Stub body = `CamOsPrintf("%s not support in LINUX USER, LR:0x%08X\n", name)`, then return:
- **0**: GetJiffies, GetHz, JiffiesToMs, MsToJiffies, JiffiesDiff, DirectMemPhysToVirt, DirectMemVirtToPhys, MemPhysToVirt,
  MemVirtToPhys, MemFromUserModeMap, MemCacheAlloc, MemCacheAllocAtomic, TimerDelete, TimerDeleteSync, IrqRequest,
  InInterrupt.
- **−1**: ThreadSchedule, ThreadWakeUp, ThreadStop, ThreadShouldStop, TcondWaitActive, SpinLockIrqSave, SpinUnlockIrqRestore,
  MemCacheCreate, TimerInit, TimerAdd, TimerModify.
- **void**: MemFlush, MemFlushExt, MemInvalidate, MiuPipeFlush, MemFromUserModeUnmap, MemCacheDestroy, MemCacheFree, IrqFree,
  IrqEnable, IrqDisable, SmpMemoryBarrier.

`CamOsMemoryBarrier` is an empty function; `CamOsPanic` prints and calls `abort()`.

Plain libc mappings: MemAlloc / MemCalloc / MemRealloc (and the `Atomic` variants) = malloc / calloc / realloc; MemRelease =
free (NULL-safe); CopyFromUpperLayer / CopyToUpperLayer = memcpy returning 0; Strtol / Strtoul / Strtoull, StrError;
Printf / Scanf / GetChar / Snprintf on stdout / stdin (Printf = `vfprintf(stdout)`, Snprintf = `vsnprintf`); Mutex = the
pthread mutex calls; Rwsem DownRead / DownWrite / UpRead / UpWrite = `pthread_rwlock_rdlock` / `wrlock` / `unlock`; Tsem
Down / Up = `sem_wait` / `sem_post`; MathDivU64 / MathDivS64 (quotient returned, remainder through the optional pointer);
IDR (`entry_num` default 0x4000; `_CamOsIdrAlloc` = first zero bit in [start, end], or in [start, entry_num−1] when
end ≤ start, found with `_CamOsFindFirstZeroBit`, §7.6).

### 7.6 Atomics, hexdump, list sort, bit search
<!-- ev:mi-user-abi.7.6 -->
All four are the source behaviour; none is imported by any ELF of the dump (unused on the camera); `_CamOsFindFirstZeroBit`
is called internally by `_CamOsIdrAlloc` (0x674c).

**`CamOsAtomic*`** (19 functions, 0x5af0–0x6128). `CamOsAtomic_t` = one `s32`.
Every read-modify-write is `dmb ish; 1: ldrex; <op>; strex; cmp; bne 1b; dmb ish` (the GCC `__sync_*` full-barrier
expansion), so each op is a sequentially consistent atomic:

| function | result | NULL handle |
|---|---|---|
| `Read`, `Set` | plain `ldr` / `str` (no barrier) | Read → 0, Set → nothing |
| `AddReturn`, `SubReturn`, `IncReturn`, `DecReturn` | new value | 0, silent |
| `SubAndTest`, `IncAndTest`, `DecAndTest` | `new == 0` (1/0, via `clz >> 5`) | 0, silent |
| `AddNegative` | `new < 0` (sign bit) | 0, silent |
| `CompareAndSwap(a, old, new)` | 1 if the value was `old` and `new` was stored, else 0 (no store) | 0, silent |
| `AndFetch`, `OrFetch`, `XorFetch`, `NandFetch` | new value; Nand stores `~(old & v)` | 0 + `"%s null atomic handle, LR:0x%08X\n"` |
| `FetchAnd`, `FetchOr`, `FetchXor`, `FetchNand` | old value | 0 + the same print |

Identical to the source (`CamOsAtomicRead` … `CamOsAtomicFetchXor`, `CAM_OS_WARN` with LR).

**`CamOsHexdump(buf, size)`** (0x3710): prints `"\nOffset(h)  00 01 02 03 04 05 06 07  08 09 0A 0B 0C 0D 0E 0F\n"` + 59
`-` + `"\n"`, then one line per 16-byte aligned row of the buffer's own addresses: `"%08X  "` row address (the first row
starts at `buf & ~0xF`, the bytes before `buf` shown as `"   "` and a blank ASCII cell), an extra `" "` before every
8-aligned byte, `"%02X "` per byte (byte read unsigned), and an ASCII column in an 80-byte line buffer: `[59] = ' '`,
`[60] = ' '`, `[61] = '|'`, cells `[62 + addr%16]` = the byte if 0x20–0x7E else `'.'`, `[78] = '|'`, `[79] = 0`; the line is
printed with `"%s\n"` at `addr%16 == 15` or at the last byte (the rest of a short last row padded with `"   "` + blank cells,
plus `" "` if fewer than 9 bytes of the row were filled). Ends with `"\n"`. Same as the source line for line, including the
`snprintf` bounds `0x50 - cx`.

**`CamOsListSort(priv, head, cmp)`** (0x6518): Linux `list_sort` (the pre-5.x bottom-up variant). Empty list → return;
`part[21]` zeroed (0x54 bytes); the ring is cut (`head->prev->next = NULL`); each element is merged up through the occupied
levels with `_CamOsListMerge` (local, 0x35e0; ties take `a` — stable); a level reaching 20 prints `"%s list too long,
LR:0x%08X\n"` and is clamped to 19; the levels below `max_lev` are merged into the rest; the last merge with `part[max_lev]`
restores the `prev` links and closes the ring (`tail->next = head`, `head->prev = tail`), calling `cmp(priv, x, x)` every
256th element of the trailing walk (the `cond_resched` hook; the compiler peeled the first iteration, so the u8 counter
starts at 2 — the same count).

**`_CamOsFindFirstZeroBit(bitmap, size, offset)`** (0x6444): returns the index of the first 0 bit at or after `offset` in a
32-bit-word bitmap of `size` bits, or `size` if none. `offset >= size` → `size`; a partial first word is OR-ed with the bits
below `offset`; whole words equal to `0xFFFFFFFF` are skipped; a partial last word is OR-ed with `~0 << remaining`; the bit
index inside the word is `ffs((x + 1) & ~x) − 1`, computed by a 16/8/4/2/1 binary search (checked for all 32 positions). Same as the source with `CAM_OS_BITS_PER_LONG` = 32.

### 7.7 What the camera imports
<!-- ev:mi-user-abi.7.7 -->
The MI libraries and the kept blobs (libcus3a, libispalgo) import only: `CamOsTsem*` Init / Deinit / Down / Up (all MI libraries; only
libmi_sys initialises, `g_mi_fd_semalock`), `CamOsMutex*` Init / Lock / Unlock / Destroy (libmi_venc, libmi_jpd, libcus3a), Rwsem Init /
DownRead / DownWrite / UpRead / UpWrite (libmi_ai, libmi_ao), MsSleep, UsSleep, GetTimeOfDay, MemAlloc / MemCalloc /
MemRelease, Printf / Snprintf, MathDivU64 / MathDivS64 (libispalgo). Per-function rows: the `camos` rows of
the table of §10. Every other export is unused on this camera (no importer among `/config/lib`, prog_uvc, kjd_lvgl; all 147 ELF files of the dump).

## 8. libcam_fs_wrapper.so — user-space CamFs (5 exports) [R]

The same source family (`drivers/sstar/cam_fs_wrapper/src/cam_fs_wrapper.c`, `CAM_OS_LINUX_USER` branch); no version
string.

| export | behaviour |
|---|---|
| `CamFsOpen(CamFsFd *pFd, path, flags, mode)` | `*pFd = open(path, flags, mode)`; **always returns 0 (CAM_FS_OK)**, even when open fails — the source's `>= 0` test is on a pointer-typed fd and was folded away by the compiler; callers have to check `*pFd` |
| `CamFsClose(fd)` | `close`; 0 or −1 |
| `CamFsRead(fd, buf, n)` / `CamFsWrite` / `CamFsSeek(fd, off, whence)` | tail jumps to `read` / `write` / `lseek`, raw return value |

The only importer is libmi_ipu.so, itself unused on this camera (`ipu-host.md` §1); so CamFs is unused.

## 9. libmi_common.so — the MI_OS layer (40 exports) [R]
<!-- ev:mi-user-abi.9 -->

Version: §1.1. **No ELF on the camera imports any libmi_common symbol** (all 147 ELF files of the dump), and prog_uvc does not link it statically (none of its names is in): the library is unused. Error code on failure **`0xA010201F`**
(`MI_DEF_ERR(module 0x10, level 2, err 0x1F FAILED)`), timeout `0xA0102019`. Failures print `"[%s] failed!(%s)\n"` with
`strerror`. The objects are native pthread / semaphore objects owned by the caller (no magic, no wrapper — unlike CamOs,
§7.3).

| export | behaviour |
|---|---|
| `MI_OS_Printf(fmt, …)` | `vfprintf(stdout)`; returns 0 |
| `MI_OS_MsSleep(ms)` | `usleep(ms*1000)` (no rounding, unlike `CamOsMsSleep`) |
| `MI_OS_GetTimeOfDay(timespec*)` / `SetTimeOfDay` | `clock_gettime` / `clock_settime(CLOCK_REALTIME)` |
| `MI_OS_GetMonotonicTime(timespec*)` | `clock_gettime(CLOCK_MONOTONIC)` |
| `MI_OS_ThreadCreate(void **h, attr{u16 prio; u16 stack}, fn, arg)` | joinable attr; prio 1..99 → `pthread_attr_setschedparam` (the policy is not changed, so it only takes effect for an RT-inheriting caller [I]); stack if ≠ 0; handle = malloc(0x10) `{pthread_t; fn; arg; u8 stop}`; entry trampoline: `pthread_once(pthread_key_create)`, `pthread_setspecific(key, h)`, `fn(arg)`. malloc failure → 12 (ENOMEM, not an MI code); create failure → `0xA010201F` |
| `MI_OS_ThreadStop(h)` | `h->stop = 1; pthread_join` (the handle is not freed) |
| `MI_OS_ThreadShouldStop(u8 *out)` | `*out = ((handle*)pthread_getspecific(key))->stop` — valid only inside a thread made by ThreadCreate |
| `MI_OS_ThreadSetName(name)` / `GetName(buf)` | `prctl(PR_SET_NAME / PR_GET_NAME)` on the calling thread |
| `MI_OS_ThreadGetId(long *tid)` | `syscall(gettid)` |
| `MI_OS_MutexInit` / `Destroy` / `Lock` / `Unlock(pthread_mutex_t*)` | the pthread call |
| `MI_OS_SemaphoreInit(sem, val)` | `sem_init(pshared 0)` |
| `MI_OS_SemaphoreDestroy` / `Up` / `Down` / `GetValue(sem[, int*])` | `sem_destroy` / `post` / `wait` / `getvalue` |
| `MI_OS_SemaphoreTimedDown(sem, ms)` | CLOCK_REALTIME deadline + `sem_timedwait`; ETIMEDOUT → `0xA0102019`. The carry test is `nsec > 1e9`: exactly 1e9 is passed un-normalised → `EINVAL` → `0xA010201F` |
| `MI_OS_WaitCondInit(obj)` | obj = `{pthread_mutex_t (0x18); pthread_cond_t}`; mutex init, cond with CLOCK_MONOTONIC |
| `MI_OS_WaitCondSignal(obj)` | lock, `pthread_cond_signal`, unlock |
| `MI_OS_WaitCond(obj)` / `WaitCondTimed(obj, ms)` | `pthread_cond_wait` / `timedwait` (CLOCK_MONOTONIC deadline, the same carry quirk); **the caller must hold the mutex**; a timeout returns `0xA010201F` (not distinguished) |
| `MI_OS_MemAlloc(size, void **p)` / `MemCalloc(n, size, void **p)` / `MemRelease(p)` | malloc / calloc / free, always 0 |
| `MI_OS_MathDivU64(u64 a, u64 b, u64 *rem, u64 *quot)` | `lldiv` (signed) |
| `MI_OS_AtomicRead(int *a, int *out)` / `AtomicSet(a, v)` | DMB + load / LDREX-STREX store |
| `MI_OS_AtomicAddReturn(v, a, int *out)`, `SubReturn`, `IncReturn(a, out)`, `DecReturn` | LDREX / STREX, `*out` = new value |
| `MI_OS_AtomicSubAndTest(v, a, bool *out)`, `IncAndTest(a, out)`, `DecAndTest(a, out)` | `*out = (new == 0)` |
| `MI_OS_AtomicAddNegative(v, a, bool *out)` | `*out = (new < 0)` |

All return 0 unless stated.

## 10. The functions prog_uvc calls [R unless a row's tag says otherwise]
<!-- ev:mi-user-abi.10 -->

171 rows: the 155 MI / CUS3A functions prog_uvc calls (modules ai, ao, cus3a, disp, hdmi, iqserver, isp, jpd, ldc, scl,
sensor, sys, vdec, venc, vif) and 16 rows of the CamOs functions the libraries need (`camos`). `so_addr` = the export's
address in the `.so`; `ioctl` = the command word(s) in hex, `decoded` = as `_IOWR('i', nr, size)`; `payload_layout` =
offsets and fields of the payload (struct names from the M6 / Tiramisu headers); `output` = what is copied back to the
caller, and when (§3.2); `user_space_logic` = what the library does besides the ioctl (a document name points to the module
document holding the logic); `extra_return_codes` = codes the library returns itself (§4). The other exports of the 13
libraries (909 in all) are specified in their module documents; [`mi-exports.tsv`](../data/mi-exports.tsv) lists all 909
with the document that specifies each, its class, ioctl and the libraries on the camera that import it.

| module | function | library | so_addr | device | ioctl | decoded | payload_layout | output | user_space_logic | extra_return_codes | tag |
|---|---|---|---|---|---|---|---|---|---|---|---|
| cus3a | CUS3A_GetAlgoStatus | libcus3a | 0x00013784 | - |  |  | no ioctl | u32[3] = {AE, AWB, AF} ready flags | reads three bytes (+0x87/+0x88/+0x89) of the libcus3a per-(dev,chn) state (stride 0x98 per chn, 0x980 per dev) |  | R |
| ai | MI_AI_SetPubAttr | libmi_ai | 0x00012090 | /dev/mi_ai | 0x40386900 | _IOW('i',0x0,0x38) | +0 AudioDev +4 stAttr(0x34) | - | rw-sem; _MI_AI_InitModule on first use |  | R |
| ai | MI_AI_Enable | libmi_ai | 0x00012454 | /dev/mi_ai | 0x40046902 | _IOW('i',0x2,0x4) | +0 AudioDev | - | rw-sem |  | R |
| ai | MI_AI_Disable | libmi_ai | 0x0001260c | /dev/mi_ai | 0x40046903 | _IOW('i',0x3,0x4) | +0 AudioDev | - | rw-sem |  | R |
| ai | MI_AI_EnableChn | libmi_ai | 0x000127dc | /dev/mi_ai | 0x40086904 | _IOW('i',0x4,0x8) | +0 AudioDev +4 AiChn | - | also MI_SYS_GetFd on the AI output port |  | R |
| ai | MI_AI_DisableChn | libmi_ai | 0x00012bb4 | /dev/mi_ai | 0x40086905 | _IOW('i',0x5,0x8) | +0 AudioDev +4 AiChn | - | also MI_SYS_CloseFd |  | R |
| ai | MI_AI_GetFrame | libmi_ai | 0x00012ee8 | /dev/mi_ai | 0xc00c6910,0xc0106911 | _IOWR('i',0x10,0xc) _IOWR('i',0x11,0x10) | 0x10 (0xc), 0x11 (0x10); data via MI_SYS_ChnOutputPortGetBuf | frame + optional AEC frame | select() on the port fd with timeout, MI_SYS_ChnOutputPortGetBuf, optional file dump; detail: audio.md |  | R |
| ai | MI_AI_ReleaseFrame | libmi_ai | 0x00013b78 | /dev/mi_ai |  |  | no own ioctl | - | MI_SYS_ChnOutputPortPutBuf |  | R |
| ai | MI_AI_SetVqeVolume | libmi_ai | 0x000143e4 | /dev/mi_ai | 0x400c6909 | _IOW('i',0x9,0xc) | +0 AudioDev +4 AiChn +8 s32VolumeDb | - | - |  | R |
| ai | MI_AI_SetMute | libmi_ai | 0x00014d4c | /dev/mi_ai | 0x400c690f | _IOW('i',0xf,0xc) | +0 AudioDev +4 AiChn +8 bEnable:u8 | - | - |  | R |
| ao | MI_AO_SetPubAttr | libmi_ao | 0x00012070 | /dev/mi_ao | 0x40386900 | _IOW('i',0x0,0x38) | +0 AudioDev +4 stAttr(0x34) | - | rw-sem; _MI_AO_InitModule |  | R |
| ao | MI_AO_Enable | libmi_ao | 0x00012490 | /dev/mi_ao | 0x40046902 | _IOW('i',0x2,0x4) | +0 AudioDev | - | allocates lib-side buffers (CamOsMemAlloc) |  | R |
| ao | MI_AO_Disable | libmi_ao | 0x00012744 | /dev/mi_ao | 0x40046903 | _IOW('i',0x3,0x4) | +0 AudioDev | - | frees them |  | R |
| ao | MI_AO_EnableChn | libmi_ao | 0x00012908 | /dev/mi_ao | 0x40086904,0x40086905 | _IOW('i',0x4,0x8) _IOW('i',0x5,0x8) | +0 AudioDev +4 AoChn (also 0x05 on error path) | - | buffer alloc, optional dump file |  | R |
| ao | MI_AO_DisableChn | libmi_ao | 0x00012fd8 | /dev/mi_ao | 0x40086905 | _IOW('i',0x5,0x8) | +0 AudioDev +4 AoChn | - | - |  | R |
| ao | MI_AO_SendFrame | libmi_ao | 0x0001461c | /dev/mi_ao |  |  | no direct ioctl | - | polls MI_AO_QueryChnStat for free space, memcpy into the per-channel 0xC800-byte user-space queue of libmi_ao (fed to the MHAL DMA ring); channel 0 only; detail: audio.md |  | R |
| disp | MI_DISP_Enable | libmi_disp | 0x000117a8 | /dev/mi_disp | 0x40046900 | _IOW('i',0x0,0x4) | +0 DispDev | - | - |  | R |
| disp | MI_DISP_Disable | libmi_disp | 0x00011808 | /dev/mi_disp | 0x40046901 | _IOW('i',0x1,0x4) | +0 DispDev | - | - |  | R |
| disp | MI_DISP_SetPubAttr | libmi_disp | 0x00011868 | /dev/mi_disp | 0x40386902 | _IOW('i',0x2,0x38) | +0 DispDev +4 stPubAttr:MI_DISP_PubAttr_t(0x34) | - | - |  | R |
| disp | MI_DISP_EnableVideoLayer | libmi_disp | 0x00011a5c | /dev/mi_disp | 0x40046906 | _IOW('i',0x6,0x4) | +0 DispLayer | - | - |  | R |
| disp | MI_DISP_DisableVideoLayer | libmi_disp | 0x00011abc | /dev/mi_disp | 0x40046907 | _IOW('i',0x7,0x4) | +0 DispLayer | - | - |  | R |
| disp | MI_DISP_SetVideoLayerAttr | libmi_disp | 0x00011b1c | /dev/mi_disp | 0x40186908 | _IOW('i',0x8,0x18) | +0 DispLayer +4 stLayerAttr:MI_DISP_VideoLayerAttr_t(0x14) | - | - |  | R |
| disp | MI_DISP_GetVideoLayerAttr | libmi_disp | 0x00011b90 | /dev/mi_disp | 0xc0186909 | _IOWR('i',0x9,0x18) | +0 DispLayer +4 stLayerAttr(0x14, out) | copy +4 (0x14) if ret==0 | - |  | R |
| disp | MI_DISP_BindVideoLayer | libmi_disp | 0x00011c14 | /dev/mi_disp | 0x4008690a | _IOW('i',0xa,0x8) | +0 DispLayer +4 DispDev | - | - |  | R |
| disp | MI_DISP_UnBindVideoLayer | libmi_disp | 0x00011c74 | /dev/mi_disp | 0x4008690b | _IOW('i',0xb,0x8) | +0 DispLayer +4 DispDev | - | - |  | R |
| disp | MI_DISP_SetInputPortAttr | libmi_disp | 0x00011e68 | /dev/mi_disp | 0x40146914 | _IOW('i',0x14,0x14) | +0 DispLayer +4 LayerInputPort +8 stInputPortAttr:MI_DISP_InputPortAttr_t(0xc) | - | - |  | R |
| disp | MI_DISP_EnableInputPort | libmi_disp | 0x00011f58 | /dev/mi_disp | 0x40086916 | _IOW('i',0x16,0x8) | +0 DispLayer +4 LayerInputPort | - | - |  | R |
| disp | MI_DISP_DisableInputPort | libmi_disp | 0x00011fb8 | /dev/mi_disp | 0x40086917 | _IOW('i',0x17,0x8) | +0 DispLayer +4 LayerInputPort | - | - |  | R |
| disp | MI_DISP_PauseInputPort | libmi_disp | 0x000120ec | /dev/mi_disp | 0x4008691a | _IOW('i',0x1a,0x8) | +0 DispLayer +4 LayerInputPort | - | - |  | R |
| disp | MI_DISP_ResumeInputPort | libmi_disp | 0x0001214c | /dev/mi_disp | 0x4008691b | _IOW('i',0x1b,0x8) | +0 DispLayer +4 LayerInputPort | - | - |  | R |
| disp | MI_DISP_SetInputPortSyncMode | libmi_disp | 0x000122cc | /dev/mi_disp | 0x400c691f | _IOW('i',0x1f,0xc) | +0 DispLayer +4 LayerInputPort +8 eMode | - | - |  | R |
| disp | MI_DISP_ClearInputPortBuffer | libmi_disp | 0x00012800 | /dev/mi_disp | 0x400c692a | _IOW('i',0x2a,0xc) | +0 DispLayer +4 LayerInputPort +8 bClrAll:u8 (+9..0xb zero) | - | - |  | R |
| hdmi | MI_HDMI_Init | libmi_hdmi | 0x00010fe4 | /dev/mi_hdmi | 0x00006900 | _IO('i',0x0,0x0) | _IO, no wrapper: ioctl(fd, 0x6900, 0) | - | on ret==0 and not yet started: store pstInitParam->pfnHdmiEventCallback, start a detached thread (arg pCallBackArgs) that calls MI_HDMI_GetAttr(0) every 500 ms and invokes the callback(0, bConnect!=1 ? 1:0 i.e. E_MI_HDMI_EVENT_HOTPLUG/NOHOTPLUG, 0, args) when bConnect changes -> mi_disp-mi_hdmi.md |  | R |
| hdmi | MI_HDMI_DeInit | libmi_hdmi | 0x000110c0 | /dev/mi_hdmi | 0x00006901 | _IO('i',0x1,0x0) | _IO, no wrapper: ioctl(fd, 0x6901, 0) | - | stops the hot-plug thread flag [I] |  | R/I |
| hdmi | MI_HDMI_Open | libmi_hdmi | 0x00011118 | /dev/mi_hdmi | 0x40046902 | _IOW('i',0x2,0x4) | +0 eHdmi | - | - |  | R |
| hdmi | MI_HDMI_Close | libmi_hdmi | 0x00011178 | /dev/mi_hdmi | 0x40046903 | _IOW('i',0x3,0x4) | +0 eHdmi | - | - |  | R |
| hdmi | MI_HDMI_SetAttr | libmi_hdmi | 0x000111d8 | /dev/mi_hdmi | 0x40346904 | _IOW('i',0x4,0x34) | +0 eHdmi +4 stAttr:MI_HDMI_Attr_t(0x30) | - | - |  | R |
| hdmi | MI_HDMI_GetAttr | libmi_hdmi | 0x00011258 | /dev/mi_hdmi | 0xc0346905 | _IOWR('i',0x5,0x34) | +0 eHdmi +4 stAttr(0x30, out) | copy +4 (0x30) if ret==0 | - |  | R |
| hdmi | MI_HDMI_Start | libmi_hdmi | 0x000113d8 | /dev/mi_hdmi | 0x40046906 | _IOW('i',0x6,0x4) | +0 eHdmi | - | - |  | R |
| hdmi | MI_HDMI_Stop | libmi_hdmi | 0x00011438 | /dev/mi_hdmi | 0x40046907 | _IOW('i',0x7,0x4) | +0 eHdmi | - | - |  | R |
| hdmi | MI_HDMI_ForceGetEdid | libmi_hdmi | 0x00011498 | /dev/mi_hdmi | 0xc20c6909 | _IOWR('i',0x9,0x20c) | +0 eHdmi +4 stEdid:MI_HDMI_Edid_t(0x208: bEdidValid u8 +3 pad, u32Edidlength, au8Edid[512]; out) | memcpy(caller,+4,0x208) if ret==0 | NULL check | A00B2003 NULL; A00B21E7 on any ioctl failure (-1 or nonzero) | R |
| hdmi | MI_HDMI_GetSinkInfo | libmi_hdmi | 0x00011590 | /dev/mi_hdmi |  |  | reuses 0x09 (ForceGetEdid payload, 0x20c) | fills MI_HDMI_SinkInfo_t (0xa8) | memset sink info; MI_HDMI_GetAttr(0) -> bConnected; if connected: ioctl 0x09 to read raw EDID, then parses EDID in user space (block 0: header, established/standard timings, 4 DTDs, checksum; block 1 CEA-861: video/audio/speaker/vendor-specific data blocks, HDMI vs DVI) -> mi_disp-mi_hdmi.md; parser: mi_disp-mi_hdmi.md |  | R/I |
| iqserver | MI_IQSERVER_Open | libmi_iqserver | 0x00013b3c | - |  |  | no ioctl of its own | - | starts the IQ-tool server thread (TCP; port 9876, firmware-image.md) that drives MI_ISP/SCL/VENC/SYS/VIF APIs; with /tmp/isp_uart present also a UART mode on /dev/ttyS0 (sends SIGSTOP to the process holding it). Tuning only |  | R |
| iqserver | MI_IQSERVER_Close | libmi_iqserver | 0x00013c78 | - |  |  | no ioctl | - | sets the stop flag of the server threads |  | R |
| isp | MI_ISP_CreateDevice | libmi_isp | 0x000177fc | /dev/mi_isp | 0x40086900 | _IOW('i',0x0,0x8) | +0 DevId +4 stDevAttr:u32 u32DevStitchMask [I name, divinus m6_isp.h] | - | - |  | R |
| isp | MI_ISP_DestoryDevice | libmi_isp | 0x00017870 | /dev/mi_isp | 0x40046901 | _IOW('i',0x1,0x4) | +0 DevId | - | - |  | R |
| isp | MI_ISP_CreateChannel | libmi_isp | 0x000179c0 | /dev/mi_isp | 0x40586902 | _IOW('i',0x2,0x58) | +0 DevId +4 ChnId +8 stChnAttr(0x50) [layout: divinus m6_isp.h, size-checked in 53] | - | after the ioctl (also when it fails) calls MI_ISP_EnableUserspace3A(dev,chn): CUS3A_Init, CUS3A_EnableUserspaceAE/AWB/AF(dev,chn,1), MI_ISP_RegisterIspApiAgent(dev,chn,Cus3A_SetIspApiData,Cus3A_GetIspApiData) -> mi-capture.md §5.2 |  | R |
| isp | MI_ISP_DestroyChannel | libmi_isp | 0x00017a40 | /dev/mi_isp | 0x40086903 | _IOW('i',0x3,0x8) | +0 DevId +4 ChnId | - | calls MI_ISP_DisableUserspace3A(dev,chn): CUS3A_EnableUserspace*(...,0), unregister agent, CUS3A_Release(dev) |  | R |
| isp | MI_ISP_GetInputPortCrop | libmi_isp | 0x00017b10 | /dev/mi_isp | 0xc0106905 | _IOWR('i',0x5,0x10) | +0 DevId +4 ChnId +8 stCrop:MI_SYS_WindowRect_t(8: u16 x,y,w,h, out) | copy +8 (8) if ret==0 | - |  | R |
| isp | MI_ISP_SetChnParam | libmi_isp | 0x00017b80 | /dev/mi_isp | 0x40186906 | _IOW('i',0x6,0x18) | +0 DevId +4 ChnId +8 stChnParam(0x10) | - | - |  | R |
| isp | MI_ISP_StartChannel | libmi_isp | 0x00017c68 | /dev/mi_isp | 0x40086908 | _IOW('i',0x8,0x8) | +0 DevId +4 ChnId | - | - |  | R |
| isp | MI_ISP_StopChannel | libmi_isp | 0x00017cc8 | /dev/mi_isp | 0x40086909 | _IOW('i',0x9,0x8) | +0 DevId +4 ChnId | - | - |  | R |
| isp | MI_ISP_SetOutputPortParam | libmi_isp | 0x00017d28 | /dev/mi_isp | 0x4018690a | _IOW('i',0xa,0x18) | +0 DevId +4 ChnId +8 PortId +0xc stOutParam(0xc: crop u16x4 + ePixelFormat) | - | - |  | R |
| isp | MI_ISP_EnableOutputPort | libmi_isp | 0x00017e14 | /dev/mi_isp | 0x400c690c | _IOW('i',0xc,0xc) | +0 DevId +4 ChnId +8 PortId | - | - |  | R |
| isp | MI_ISP_DisableOutputPort | libmi_isp | 0x00017e78 | /dev/mi_isp | 0x400c690d | _IOW('i',0xd,0xc) | +0 DevId +4 ChnId +8 PortId | - | - |  | R |
| isp | MI_ISP_API_CmdLoadBinFile | libmi_isp | 0x000182e0 | /dev/mi_isp |  |  | no own ioctl | - | reads the whole file (fopen/fseek/ftell/calloc/fread), MI_ISP_ALGO_API_DisableAllApi (no-op), MI_ISP_ALGO_API_CmdLoadBinFile: header check + per-record MI_ISP_IQ_Set*/AE/AWB calls -> mi-capture.md §5.7 | -1 file open fail / bad checksum / major version mismatch / key mismatch | R |
| isp | MI_ISP_IQ_SetColorToGray | libmi_isp | 0x00019820 | /dev/mi_isp | 0x401c6910 | _IOW('i',0x10,0x1c) | IQ API: hdr {+0 u32HeadSize=0x18 +4 u32DataLen=0x4 +8 u32CtrlID=0x1004 +0xc u32Channel +0x10 u32Dev +0x14 0} + data pointer; data = caller struct (0x4 B) | - | _MI_ISP_SetIspApiData: kernel (agent returns 0xA0078008) -> mi-capture.md §5.2 |  | R |
| isp | MI_ISP_IQ_GetColorToGray | libmi_isp | 0x00019888 | /dev/mi_isp | 0x401c6911 | _IOW('i',0x11,0x1c) | IQ API: hdr {+0 u32HeadSize=0x18 +4 u32DataLen=0x4 +8 u32CtrlID=0x1004 +0xc u32Channel +0x10 u32Dev +0x14 0} + data pointer; data = caller struct (0x4 B) | kernel/agent writes the caller struct | _MI_ISP_GetIspApiData: kernel (agent returns 0xA0078008) -> mi-capture.md §5.2 |  | R |
| isp | MI_ISP_IQ_SetContrast | libmi_isp | 0x000198c0 | /dev/mi_isp | 0x401c6910 | _IOW('i',0x10,0x1c) | IQ API: hdr {+0 u32HeadSize=0x18 +4 u32DataLen=0x4c +8 u32CtrlID=0x1005 +0xc u32Channel +0x10 u32Dev +0x14 0} + data pointer; data = caller struct (0x4c B) | - | _MI_ISP_SetIspApiData: kernel (agent returns 0xA0078008) -> mi-capture.md §5.2 |  | R |
| isp | MI_ISP_IQ_GetContrast | libmi_isp | 0x00019928 | /dev/mi_isp | 0x401c6911 | _IOW('i',0x11,0x1c) | IQ API: hdr {+0 u32HeadSize=0x18 +4 u32DataLen=0x4c +8 u32CtrlID=0x1005 +0xc u32Channel +0x10 u32Dev +0x14 0} + data pointer; data = caller struct (0x4c B) | kernel/agent writes the caller struct | _MI_ISP_GetIspApiData: kernel (agent returns 0xA0078008) -> mi-capture.md §5.2 |  | R |
| isp | MI_ISP_IQ_SetLightness | libmi_isp | 0x00019960 | /dev/mi_isp | 0x401c6910 | _IOW('i',0x10,0x1c) | IQ API: hdr {+0 u32HeadSize=0x18 +4 u32DataLen=0x4c +8 u32CtrlID=0x1007 +0xc u32Channel +0x10 u32Dev +0x14 0} + data pointer; data = caller struct (0x4c B) | - | _MI_ISP_SetIspApiData: kernel (agent returns 0xA0078008) -> mi-capture.md §5.2 |  | R |
| isp | MI_ISP_IQ_GetLightness | libmi_isp | 0x000199c8 | /dev/mi_isp | 0x401c6911 | _IOW('i',0x11,0x1c) | IQ API: hdr {+0 u32HeadSize=0x18 +4 u32DataLen=0x4c +8 u32CtrlID=0x1007 +0xc u32Channel +0x10 u32Dev +0x14 0} + data pointer; data = caller struct (0x4c B) | kernel/agent writes the caller struct | _MI_ISP_GetIspApiData: kernel (agent returns 0xA0078008) -> mi-capture.md §5.2 |  | R |
| isp | MI_ISP_IQ_SetBrightness | libmi_isp | 0x00019a00 | /dev/mi_isp | 0x401c6910 | _IOW('i',0x10,0x1c) | IQ API: hdr {+0 u32HeadSize=0x18 +4 u32DataLen=0x4c +8 u32CtrlID=0x1006 +0xc u32Channel +0x10 u32Dev +0x14 0} + data pointer; data = caller struct (0x4c B) | - | _MI_ISP_SetIspApiData: kernel (agent returns 0xA0078008) -> mi-capture.md §5.2 |  | R |
| isp | MI_ISP_IQ_GetBrightness | libmi_isp | 0x00019a68 | /dev/mi_isp | 0x401c6911 | _IOW('i',0x11,0x1c) | IQ API: hdr {+0 u32HeadSize=0x18 +4 u32DataLen=0x4c +8 u32CtrlID=0x1006 +0xc u32Channel +0x10 u32Dev +0x14 0} + data pointer; data = caller struct (0x4c B) | kernel/agent writes the caller struct | _MI_ISP_GetIspApiData: kernel (agent returns 0xA0078008) -> mi-capture.md §5.2 |  | R |
| isp | MI_ISP_IQ_SetSaturation | libmi_isp | 0x00019c98 | /dev/mi_isp | 0x401c6910 | _IOW('i',0x10,0x1c) | IQ API: hdr {+0 u32HeadSize=0x18 +4 u32DataLen=0x1a0 +8 u32CtrlID=0x100a +0xc u32Channel +0x10 u32Dev +0x14 0} + data pointer; data = caller struct (0x1a0 B) | - | _MI_ISP_SetIspApiData: kernel (agent returns 0xA0078008) -> mi-capture.md §5.2 |  | R |
| isp | MI_ISP_IQ_GetSaturation | libmi_isp | 0x00019d08 | /dev/mi_isp | 0x401c6911 | _IOW('i',0x11,0x1c) | IQ API: hdr {+0 u32HeadSize=0x18 +4 u32DataLen=0x1a0 +8 u32CtrlID=0x100a +0xc u32Channel +0x10 u32Dev +0x14 0} + data pointer; data = caller struct (0x1a0 B) | kernel/agent writes the caller struct | _MI_ISP_GetIspApiData: kernel (agent returns 0xA0078008) -> mi-capture.md §5.2 |  | R |
| isp | MI_ISP_IQ_SetDefog | libmi_isp | 0x00019d40 | /dev/mi_isp | 0x401c6910 | _IOW('i',0x10,0x1c) | IQ API: hdr {+0 u32HeadSize=0x18 +4 u32DataLen=0x1c +8 u32CtrlID=0x100b +0xc u32Channel +0x10 u32Dev +0x14 0} + data pointer; data = caller struct (0x1c B) | - | _MI_ISP_SetIspApiData: kernel (agent returns 0xA0078008) -> mi-capture.md §5.2 |  | R |
| isp | MI_ISP_IQ_GetDefog | libmi_isp | 0x00019dc0 | /dev/mi_isp | 0x401c6911 | _IOW('i',0x11,0x1c) | IQ API: hdr {+0 u32HeadSize=0x18 +4 u32DataLen=0x1c +8 u32CtrlID=0x100b +0xc u32Channel +0x10 u32Dev +0x14 0} + data pointer; data = caller struct (0x1c B) | kernel/agent writes the caller struct | _MI_ISP_GetIspApiData: kernel (agent returns 0xA0078008) -> mi-capture.md §5.2 |  | R |
| isp | MI_ISP_IQ_SetSharpness | libmi_isp | 0x0001a280 | /dev/mi_isp | 0x401c6910 | _IOW('i',0x10,0x1c) | IQ API: hdr {+0 u32HeadSize=0x18 +4 u32DataLen=0x1328 +8 u32CtrlID=0x1012 +0xc u32Channel +0x10 u32Dev +0x14 0} + data pointer; data = caller struct (0x1328 B) | - | _MI_ISP_SetIspApiData: kernel (agent returns 0xA0078008) -> mi-capture.md §5.2 |  | R |
| isp | MI_ISP_IQ_GetSharpness | libmi_isp | 0x0001a2f0 | /dev/mi_isp | 0x401c6911 | _IOW('i',0x11,0x1c) | IQ API: hdr {+0 u32HeadSize=0x18 +4 u32DataLen=0x1328 +8 u32CtrlID=0x1012 +0xc u32Channel +0x10 u32Dev +0x14 0} + data pointer; data = caller struct (0x1328 B) | kernel/agent writes the caller struct | _MI_ISP_GetIspApiData: kernel (agent returns 0xA0078008) -> mi-capture.md §5.2 |  | R |
| isp | MI_ISP_IQ_SetWDR | libmi_isp | 0x0001a510 | /dev/mi_isp | 0x401c6910 | _IOW('i',0x10,0x1c) | IQ API: hdr {+0 u32HeadSize=0x18 +4 u32DataLen=0x668 +8 u32CtrlID=0x1016 +0xc u32Channel +0x10 u32Dev +0x14 0} + data pointer; data = caller struct (0x668 B) | - | _MI_ISP_SetIspApiData: kernel (agent returns 0xA0078008) -> mi-capture.md §5.2 |  | R |
| isp | MI_ISP_IQ_GetWDR | libmi_isp | 0x0001a580 | /dev/mi_isp | 0x401c6911 | _IOW('i',0x11,0x1c) | IQ API: hdr {+0 u32HeadSize=0x18 +4 u32DataLen=0x668 +8 u32CtrlID=0x1016 +0xc u32Channel +0x10 u32Dev +0x14 0} + data pointer; data = caller struct (0x668 B) | kernel/agent writes the caller struct | _MI_ISP_GetIspApiData: kernel (agent returns 0xA0078008) -> mi-capture.md §5.2 |  | R |
| isp | MI_ISP_IQ_SetHSV | libmi_isp | 0x0001a850 | /dev/mi_isp | 0x401c6910 | _IOW('i',0x10,0x1c) | IQ API: hdr {+0 u32HeadSize=0x18 +4 u32DataLen=0x134c +8 u32CtrlID=0x1018 +0xc u32Channel +0x10 u32Dev +0x14 0} + data pointer; data = caller struct (0x134c B) | - | _MI_ISP_SetIspApiData: kernel (agent returns 0xA0078008) -> mi-capture.md §5.2 |  | R |
| isp | MI_ISP_AE_QueryExposureInfo | libmi_isp | 0x0001ba60 | /dev/mi_isp | 0x401c6911 | _IOW('i',0x11,0x1c) | IQ API: hdr {+0 u32HeadSize=0x18 +4 u32DataLen=0x23c +8 u32CtrlID=0x1402 +0xc u32Channel +0x10 u32Dev +0x14 0} + data pointer; data = caller struct (0x23c B) | kernel/agent writes the caller struct | _MI_ISP_GetIspApiData: agent Cus3A_*IspApiData (user-space 3A) first, kernel if it returns 0xA0078008 -> mi-capture.md §5.2 |  | R |
| isp | MI_ISP_AE_SetExpoMode | libmi_isp | 0x0001bb40 | /dev/mi_isp | 0x401c6910 | _IOW('i',0x10,0x1c) | IQ API: hdr {+0 u32HeadSize=0x18 +4 u32DataLen=0x4 +8 u32CtrlID=0x1404 +0xc u32Channel +0x10 u32Dev +0x14 0} + data pointer; data = caller struct (0x4 B) | - | _MI_ISP_SetIspApiData: agent Cus3A_*IspApiData (user-space 3A) first, kernel if it returns 0xA0078008 -> mi-capture.md §5.2 |  | R |
| isp | MI_ISP_AE_GetExpoMode | libmi_isp | 0x0001bba8 | /dev/mi_isp | 0x401c6911 | _IOW('i',0x11,0x1c) | IQ API: hdr {+0 u32HeadSize=0x18 +4 u32DataLen=0x4 +8 u32CtrlID=0x1404 +0xc u32Channel +0x10 u32Dev +0x14 0} + data pointer; data = caller struct (0x4 B) | kernel/agent writes the caller struct | _MI_ISP_GetIspApiData: agent Cus3A_*IspApiData (user-space 3A) first, kernel if it returns 0xA0078008 -> mi-capture.md §5.2 |  | R |
| isp | MI_ISP_AE_SetManualExpo | libmi_isp | 0x0001bbe0 | /dev/mi_isp | 0x401c6910 | _IOW('i',0x10,0x1c) | IQ API: hdr {+0 u32HeadSize=0x18 +4 u32DataLen=0x10 +8 u32CtrlID=0x1405 +0xc u32Channel +0x10 u32Dev +0x14 0} + data pointer; data = caller struct (0x10 B) | - | _MI_ISP_SetIspApiData: agent Cus3A_*IspApiData (user-space 3A) first, kernel if it returns 0xA0078008 -> mi-capture.md §5.2 |  | R |
| isp | MI_ISP_AE_GetManualExpo | libmi_isp | 0x0001bc50 | /dev/mi_isp | 0x401c6911 | _IOW('i',0x11,0x1c) | IQ API: hdr {+0 u32HeadSize=0x18 +4 u32DataLen=0x10 +8 u32CtrlID=0x1405 +0xc u32Channel +0x10 u32Dev +0x14 0} + data pointer; data = caller struct (0x10 B) | kernel/agent writes the caller struct | _MI_ISP_GetIspApiData: agent Cus3A_*IspApiData (user-space 3A) first, kernel if it returns 0xA0078008 -> mi-capture.md §5.2 |  | R |
| isp | MI_ISP_AE_SetTarget | libmi_isp | 0x0001bdd0 | /dev/mi_isp | 0x401c6910 | _IOW('i',0x10,0x1c) | IQ API: hdr {+0 u32HeadSize=0x18 +4 u32DataLen=0x84 +8 u32CtrlID=0x1407 +0xc u32Channel +0x10 u32Dev +0x14 0} + data pointer; data = caller struct (0x84 B) | - | _MI_ISP_SetIspApiData: agent Cus3A_*IspApiData (user-space 3A) first, kernel if it returns 0xA0078008 -> mi-capture.md §5.2 |  | R |
| isp | MI_ISP_AE_GetTarget | libmi_isp | 0x0001be38 | /dev/mi_isp | 0x401c6911 | _IOW('i',0x11,0x1c) | IQ API: hdr {+0 u32HeadSize=0x18 +4 u32DataLen=0x84 +8 u32CtrlID=0x1407 +0xc u32Channel +0x10 u32Dev +0x14 0} + data pointer; data = caller struct (0x84 B) | kernel/agent writes the caller struct | _MI_ISP_GetIspApiData: agent Cus3A_*IspApiData (user-space 3A) first, kernel if it returns 0xA0078008 -> mi-capture.md §5.2 |  | R |
| isp | MI_ISP_AE_GetExposureLimit | libmi_isp | 0x0001bfb8 | /dev/mi_isp | 0x401c6911 | _IOW('i',0x11,0x1c) | IQ API: hdr {+0 u32HeadSize=0x18 +4 u32DataLen=0x20 +8 u32CtrlID=0x1409 +0xc u32Channel +0x10 u32Dev +0x14 0} + data pointer; data = caller struct (0x20 B) | kernel/agent writes the caller struct | _MI_ISP_GetIspApiData: agent Cus3A_*IspApiData (user-space 3A) first, kernel if it returns 0xA0078008 -> mi-capture.md §5.2 |  | R |
| isp | MI_ISP_AE_SetFlicker | libmi_isp | 0x0001c258 | /dev/mi_isp | 0x401c6910 | _IOW('i',0x10,0x1c) | IQ API: hdr {+0 u32HeadSize=0x18 +4 u32DataLen=0x4 +8 u32CtrlID=0x140e +0xc u32Channel +0x10 u32Dev +0x14 0} + data pointer; data = caller struct (0x4 B) | - | _MI_ISP_SetIspApiData: agent Cus3A_*IspApiData (user-space 3A) first, kernel if it returns 0xA0078008 -> mi-capture.md §5.2 |  | R |
| isp | MI_ISP_AE_GetFlicker | libmi_isp | 0x0001c2c0 | /dev/mi_isp | 0x401c6911 | _IOW('i',0x11,0x1c) | IQ API: hdr {+0 u32HeadSize=0x18 +4 u32DataLen=0x4 +8 u32CtrlID=0x140e +0xc u32Channel +0x10 u32Dev +0x14 0} + data pointer; data = caller struct (0x4 B) | kernel/agent writes the caller struct | _MI_ISP_GetIspApiData: agent Cus3A_*IspApiData (user-space 3A) first, kernel if it returns 0xA0078008 -> mi-capture.md §5.2 |  | R |
| isp | MI_ISP_AWB_QueryInfo | libmi_isp | 0x0001c7f8 | /dev/mi_isp | 0x401c6911 | _IOW('i',0x11,0x1c) | IQ API: hdr {+0 u32HeadSize=0x18 +4 u32DataLen=0x18 +8 u32CtrlID=0x1801 +0xc u32Channel +0x10 u32Dev +0x14 0} + data pointer; data = caller struct (0x18 B) | kernel/agent writes the caller struct | _MI_ISP_GetIspApiData: agent Cus3A_*IspApiData (user-space 3A) first, kernel if it returns 0xA0078008 -> mi-capture.md §5.2 |  | R |
| isp | MI_ISP_AWB_SetAttr | libmi_isp | 0x0001c828 | /dev/mi_isp | 0x401c6910 | _IOW('i',0x10,0x1c) | IQ API: hdr {+0 u32HeadSize=0x18 +4 u32DataLen=0x5b8 +8 u32CtrlID=0x1802 +0xc u32Channel +0x10 u32Dev +0x14 0} + data pointer; data = caller struct (0x5b8 B) | - | _MI_ISP_SetIspApiData: agent Cus3A_*IspApiData (user-space 3A) first, kernel if it returns 0xA0078008 -> mi-capture.md §5.2 |  | R |
| isp | MI_ISP_AWB_GetAttr | libmi_isp | 0x0001c898 | /dev/mi_isp | 0x401c6911 | _IOW('i',0x11,0x1c) | IQ API: hdr {+0 u32HeadSize=0x18 +4 u32DataLen=0x5b8 +8 u32CtrlID=0x1802 +0xc u32Channel +0x10 u32Dev +0x14 0} + data pointer; data = caller struct (0x5b8 B) | kernel/agent writes the caller struct | _MI_ISP_GetIspApiData: agent Cus3A_*IspApiData (user-space 3A) first, kernel if it returns 0xA0078008 -> mi-capture.md §5.2 |  | R |
| jpd | MI_JPD_CreateChn | libmi_jpd | 0x00010e64 | /dev/mi_jpd | 0x40186902 | _IOW('i',0x2,0x18) | +0 DevId(must be 0) +4 ChnId(<16) +8 stChnAttr(0x10) | - | on success: init per-channel state (0x38 B) + CamOsMutexInit; 0xA0202004 EXIST -> 0 | A0202001/2/6 | R |
| jpd | MI_JPD_DestroyChn | libmi_jpd | 0x0001102c | /dev/mi_jpd | 0x40086903 | _IOW('i',0x3,0x8) | +0 DevId(sent as 0) +4 ChnId | - | lock; MI_SYS_Munmap of the cached bitstream buffer; ioctl; unlock; destroy mutex |  | R |
| jpd | MI_JPD_StartChn | libmi_jpd | 0x00011310 | /dev/mi_jpd | 0x40086905 | _IOW('i',0x5,0x8) | +0 DevId +4 ChnId | - | - |  | R |
| jpd | MI_JPD_StopChn | libmi_jpd | 0x00011420 | /dev/mi_jpd | 0x40086906 | _IOW('i',0x6,0x8) | +0 DevId +4 ChnId | - | - |  | R |
| jpd | MI_JPD_GetStreamBuf | libmi_jpd | 0x000117ec | /dev/mi_jpd | 0xc00c690f,0xc0406909,0xc018690e | _IOWR('i',0xf,0xc) _IOWR('i',0x9,0x40) _IOWR('i',0xe,0x18) | 0x0f {+0 0 +4 chn +8 u32 bufId(out)}(0xc); 0x0e {+0 bufId +8 u64 phyAddr(out) +0x10 u32 size(out)}(0x18); 0x09 {+0 dev +4 chn +8 u32RequiredLength +0x10 stStreamBuf(out) +0x38 s32MilliSec}(0x40) | pstRetStreamBuf: two segments {phy,len}; VA = mapVA + (phy - mapPhy) | per-channel mutex; first call: get buffer id + phys/size, MI_SYS_Mmap it once; then 0x09; convert phys->VA; on 0xA020200F (BUF_FULL) sleep 30 ms -> mi_jpd-mi_vdec.md | A020201F on ioctl/mmap failure | R/I |
| jpd | MI_JPD_PutStreamBuf | libmi_jpd | 0x00011b50 | /dev/mi_jpd | 0x4030690a | _IOW('i',0xa,0x30) | +0 DevId +4 ChnId +8 stStreamBuf(0x28) | - | - |  | R |
| jpd | MI_JPD_DropStreamBuf | libmi_jpd | 0x00011cec | /dev/mi_jpd | 0x4030690b | _IOW('i',0xb,0x30) | +0 DevId +4 ChnId +8 stStreamBuf(0x28) | - | - |  | R |
| ldc | MI_LDC_CreateDevice | libmi_ldc | 0x00010b14 | /dev/mi_ldc | 0x40046900 | _IOW('i',0x0,0x4) | +0 DevId (DevAttr arg not sent) | - | - |  | R |
| ldc | MI_LDC_CreateChannel | libmi_ldc | 0x00010cd4 | /dev/mi_ldc | 0x405c6902 | _IOW('i',0x2,0x5c) | +0 DevId +4 ChnId +8 stChnAttr(0x54) | - | - |  | R |
| ldc | MI_LDC_DestroyChannel | libmi_ldc | 0x00010d44 | /dev/mi_ldc | 0x40086903 | _IOW('i',0x3,0x8) | +0 DevId +4 ChnId | - | - |  | R |
| ldc | MI_LDC_StartChannel | libmi_ldc | 0x00010f88 | /dev/mi_ldc | 0x40086906 | _IOW('i',0x6,0x8) | +0 DevId +4 ChnId | - | - |  | R |
| ldc | MI_LDC_StopChannel | libmi_ldc | 0x00010fe8 | /dev/mi_ldc | 0x40086907 | _IOW('i',0x7,0x8) | +0 DevId +4 ChnId | - | - |  | R |
| scl | MI_SCL_CreateDevice | libmi_scl | 0x00010b34 | /dev/mi_scl | 0x40086900 | _IOW('i',0x0,0x8) | +0 DevId +4 stDevAttr:u32 u32NeedUseHWOutPortMask | - | - |  | R |
| scl | MI_SCL_DestroyDevice | libmi_scl | 0x00010ba8 | /dev/mi_scl | 0x40046901 | _IOW('i',0x1,0x4) | +0 DevId | - | - |  | R |
| scl | MI_SCL_CreateChannel | libmi_scl | 0x00010cf8 | /dev/mi_scl | 0x400c6902 | _IOW('i',0x2,0xc) | +0 DevId +4 ChnId +8 stChnAttr(4) | - | - |  | R |
| scl | MI_SCL_DestroyChannel | libmi_scl | 0x00010d5c | /dev/mi_scl | 0x40086903 | _IOW('i',0x3,0x8) | +0 DevId +4 ChnId | - | - |  | R |
| scl | MI_SCL_SetChnParam | libmi_scl | 0x00010dbc | /dev/mi_scl | 0x400c6906 | _IOW('i',0x6,0xc) | +0 DevId +4 ChnId +8 stChnParam(4) | - | - |  | R |
| scl | MI_SCL_StartChannel | libmi_scl | 0x00010f6c | /dev/mi_scl | 0x40086908 | _IOW('i',0x8,0x8) | +0 DevId +4 ChnId | - | - |  | R |
| scl | MI_SCL_StopChannel | libmi_scl | 0x00010fcc | /dev/mi_scl | 0x40086909 | _IOW('i',0x9,0x8) | +0 DevId +4 ChnId | - | - |  | R |
| scl | MI_SCL_SetOutputPortParam | libmi_scl | 0x0001102c | /dev/mi_scl | 0x4024690a | _IOW('i',0xa,0x24) | +0 DevId +4 ChnId +8 PortId +0xc stOutPortParam:MI_SCL_OutPortParam_t(0x18) | - | - |  | R |
| scl | MI_SCL_GetOutputPortParam | libmi_scl | 0x000110a8 | /dev/mi_scl | 0xc024690b | _IOWR('i',0xb,0x24) | +0 DevId +4 ChnId +8 PortId +0xc stOutPortParam(0x18, out) | copy +0xc (0x18) if ret==0 | - |  | R |
| scl | MI_SCL_EnableOutputPort | libmi_scl | 0x00011128 | /dev/mi_scl | 0x400c690c | _IOW('i',0xc,0xc) | +0 DevId +4 ChnId +8 PortId | - | - |  | R |
| scl | MI_SCL_DisableOutputPort | libmi_scl | 0x0001118c | /dev/mi_scl | 0x400c690d | _IOW('i',0xd,0xc) | +0 DevId +4 ChnId +8 PortId | - | - |  | R |
| sensor | MI_SNR_Enable | libmi_sensor | 0x00010c5c | /dev/mi_sensor | 0x40046900 | _IOW('i',0x0,0x4) | +0 ePADId | - | - |  | R |
| sensor | MI_SNR_Disable | libmi_sensor | 0x00010cbc | /dev/mi_sensor | 0x40046901 | _IOW('i',0x1,0x4) | +0 ePADId | - | - |  | R |
| sensor | MI_SNR_GetPadInfo | libmi_sensor | 0x00010d1c | /dev/mi_sensor | 0xc0386902 | _IOWR('i',0x2,0x38) | +0 ePADId +4 stPadInfo:MI_SNR_PADInfo_t(0x34, out) | copy +4..+0x37 to caller only if ret==0 | - |  | R |
| sensor | MI_SNR_GetPlaneInfo | libmi_sensor | 0x00010de0 | /dev/mi_sensor | 0xc0506903 | _IOWR('i',0x3,0x50) | +0 ePADId +4 u32PlaneID +8 stPlaneInfo:MI_SNR_PlaneInfo_t(0x48, out) | copy +8 (0x48) to caller if ret==0 | - |  | R |
| sensor | MI_SNR_QueryResCount | libmi_sensor | 0x00011038 | /dev/mi_sensor | 0xc0086908 | _IOWR('i',0x8,0x8) | +0 ePADId +4 u32ResCount (in/out) | *pu32 = +4 | - |  | R |
| sensor | MI_SNR_GetRes | libmi_sensor | 0x000110a4 | /dev/mi_sensor | 0xc03c6909 | _IOWR('i',0x9,0x3c) | +0 ePADId +4 u8ResIdx (+5..7 zero) +8 stRes:MI_SNR_Res_t(0x34, out) | copy +8 (0x34) if ret==0 | - |  | R |
| sensor | MI_SNR_SetRes | libmi_sensor | 0x00011238 | /dev/mi_sensor | 0x4008690b | _IOW('i',0xb,0x8) | +0 ePADId +4 u8ResIdx (+5..7 zero) | - | - |  | R |
| sensor | MI_SNR_SetPlaneMode | libmi_sensor | 0x00011394 | /dev/mi_sensor | 0x4008690e | _IOW('i',0xe,0x8) | +0 ePADId +4 bEnable:u8 (+5..7 zero) | - | - |  | R |
| sys | MI_SYS_Init | libmi_sys | 0x000114bc | /dev/mi_sys | 0x80046900 | _IOR('i',0x0,0x4) | +0 u32 0 (constant; u16SocId arg is NOT sent) | - | CamOsTsemInit(&g_mi_fd_semalock,1) before the ioctl |  | R |
| sys | MI_SYS_Exit | libmi_sys | 0x00011534 | /dev/mi_sys | 0x80046901 | _IOR('i',0x1,0x4) | +0 u32 0 | - | CamOsTsemDeinit(&g_mi_fd_semalock) after the ioctl (even on error) |  | R |
| sys | MI_SYS_BindChnPort2 | libmi_sys | 0x0001162c | /dev/mi_sys | 0x40306903 | _IOW('i',0x3,0x30) | +0 stSrc:MI_SYS_ChnPort_t(0x10) +0x10 stDst:ChnPort(0x10) +0x20 u32SrcFrmrate +0x24 u32DstFrmrate +0x28 eBindType +0x2c u32BindParam; socId not sent | - | - |  | R |
| sys | MI_SYS_UnBindChnPort | libmi_sys | 0x000116cc | /dev/mi_sys | 0x40306904 | _IOW('i',0x4,0x30) | +0 stSrc:ChnPort +0x10 stDst:ChnPort +0x20..0x2f unused (same 0x30 struct as Bind) | - | - |  | R |
| sys | MI_SYS_GetVersion | libmi_sys | 0x00011748 | /dev/mi_sys | 0x80806906 | _IOR('i',0x6,0x80) | payload = caller MI_SYS_Version_t (0x80, u8 au8Version[128]); wrapper points at the caller buffer directly | kernel writes the caller struct | - |  | R |
| sys | MI_SYS_GetCurPts | libmi_sys | 0x000117a4 | /dev/mi_sys | 0x80086907 | _IOR('i',0x7,0x8) | payload = caller u64 (wrapper points at it) | u64 pts | - |  | R |
| sys | MI_SYS_InitPtsBase | libmi_sys | 0x00011800 | /dev/mi_sys | 0x40086908 | _IOW('i',0x8,0x8) | +0 u64 u64PtsBase | - | - |  | R |
| sys | MI_SYS_SyncPts | libmi_sys | 0x000118bc | /dev/mi_sys | 0x40086909 | _IOW('i',0x9,0x8) | +0 u64 u64Pts | - | - |  | R |
| sys | MI_SYS_ChnOutputPortGetBuf | libmi_sys | 0x00011e60 | /dev/mi_sys | 0xc1286913 | _IOWR('i',0x13,0x128) | +0 stChnPort(0x10) +0x10 stBufInfo:MI_SYS_BufInfo_t(0x108, out) +0x118 hBuf (out) +0x120 u32 flags = 0xA0000000 written by the lib (bit 31 = map the buffer into the process); +0x11c and +0x124 not named | memcpy(pstBufInfo,+0x10,0x108) and *phBuf=+0x118 copied back EVEN when the ioctl fails | virtual addresses in BufInfo are filled by the kernel, which maps the buffer because flags bit 31 is set (no mmap in the lib) |  | R |
| sys | MI_SYS_ChnOutputPortGetBufPa | libmi_sys | 0x00011f60 | /dev/mi_sys | 0xc1286913 | _IOWR('i',0x13,0x128) | same as GetBuf (same ioctl nr 0x13) | same | same code path as GetBuf, but flags +0x120 = 0 (the kernel does not map the buffer into the process) |  | R |
| sys | MI_SYS_ChnOutputPortPutBuf | libmi_sys | 0x00012078 | /dev/mi_sys | 0x40046914 | _IOW('i',0x14,0x4) | +0 hBuf:u32 | - | - |  | R |
| sys | MI_SYS_ChnOutputPortPutBufPa | libmi_sys | 0x000120d8 | /dev/mi_sys | 0x40046914 | _IOW('i',0x14,0x4) | +0 hBuf:u32 (same ioctl as PutBuf) | - | - |  | R |
| sys | MI_SYS_SetChnOutputPortDepth | libmi_sys | 0x00012138 | /dev/mi_sys | 0x40186915 | _IOW('i',0x15,0x18) | +0 stChnPort(0x10) +0x10 u32UserFrameDepth +0x14 u32BufQueueDepth; socId not sent | - | - |  | R |
| vdec | MI_VDEC_CreateDev | libmi_vdec | 0x00011194 | /dev/mi_vdec | 0x40046900 | _IOW('i',0x0,0x4) | +0 stInitParam(4); VdecDev arg NOT sent | - | increments a lib-side device counter on success |  | R |
| vdec | MI_VDEC_CreateChn | libmi_vdec | 0x00011358 | /dev/mi_vdec | 0x40286902 | _IOW('i',0x2,0x28) | +0 VdecChn +4 stChnAttr(0x24); dev not sent | - | if attr word 5 (video mode [I]) == 0 (stream mode): malloc a 512 KiB staging buffer for this channel and remember codec type (attr word 0) |  | R/I |
| vdec | MI_VDEC_StartChn | libmi_vdec | 0x00011538 | /dev/mi_vdec | 0x40046905 | _IOW('i',0x5,0x4) | +0 VdecChn | - | marks the channel started in the lib state |  | R |
| vdec | MI_VDEC_GetChnStat | libmi_vdec | 0x00011624 | /dev/mi_vdec | 0xc0246907 | _IOWR('i',0x7,0x24) | +0 VdecChn +4 stChnStat(0x20, out) | copy +4 (0x20) if ret==0 | - |  | R |
| vdec | MI_VDEC_SendStream | libmi_vdec | 0x00011860 | /dev/mi_vdec |  |  | ioctl 0x0c (0x28): +0 VdecChn +4 0 +8 stVideoStream(0x18: pu8Addr, u32Len, u64PTS, flags) +0x20 s32MilliSec | - | stream mode: accumulate data in the 512 KiB buffer, split into frames with get_next_frame_start_position (H.264/H.265: 00 00 00 01 start code, slice NAL; JPEG: FF D8) and send frame by frame; frame mode: one ioctl -> mi_jpd-mi_vdec.md | A0082003 NULL addr; A008201F | R |
| vdec | MI_VDEC_SetOutputPortAttr | libmi_vdec | 0x00011eb8 | /dev/mi_vdec | 0x40086914 | _IOW('i',0x14,0x8) | +0 VdecChn +4 stOutputPortAttr(4: u16 w,h [I]) | - | - |  | R/I |
| vdec | MI_VDEC_GetOutputPortAttr | libmi_vdec | 0x00011f70 | /dev/mi_vdec | 0xc0086915 | _IOWR('i',0x15,0x8) | +0 VdecChn +4 stOutputPortAttr(4, out) | *attr = +4 | - |  | R |
| venc | MI_VENC_CreateChn | libmi_venc | 0x00011fcc | /dev/mi_venc | 0x40546902 | _IOW('i',0x2,0x54) | +0 VeDev +4 VeChn +8 stAttr:MI_VENC_ChnAttr_t(0x4c) | - | checks dev<10, chn<=32, attr!=NULL; CamOsMutexInit x3 in the per-channel state; on success clears the stream-map state; 0xA0022004 (EXIST) from the kernel is turned into 0 | A0022001 bad dev, A0022002 bad chn, A0022006 NULL | R |
| venc | MI_VENC_DestroyChn | libmi_venc | 0x00012358 | /dev/mi_venc | 0x40086903 | _IOW('i',0x3,0x8) | +0 VeDev +4 VeChn | - | MI_VENC_FreeCustomMap; lock; if a stream buffer is mapped: FlushInvCache+MI_SYS_Munmap; clear state; ioctl; unlock; destroy mutexes | A0022009 lock fail; -1 munmap fail | R |
| venc | MI_VENC_StartRecvPic | libmi_venc | 0x0001263c | /dev/mi_venc | 0x40086905 | _IOW('i',0x5,0x8) | +0 VeDev +4 VeChn | - | range checks | A0022001/2 | R |
| venc | MI_VENC_StopRecvPic | libmi_venc | 0x000129d0 | /dev/mi_venc | 0x40086907,0x40086907 | _IOW('i',0x7,0x8) _IOW('i',0x7,0x8) | +0 VeDev +4 VeChn | - | if the kernel answers 0xA0022012 (BUSY) retry every 10 ms (CamOsMsSleep), up to 100 times | A0022001/2 | R |
| venc | MI_VENC_Query | libmi_venc | 0x00012b64 | /dev/mi_venc | 0xc0306908 | _IOWR('i',0x8,0x30) | +0 VeDev +4 VeChn +8 stStat:MI_VENC_ChnStat_t(0x28, out) | copy +8 (0x28) if ret==0 | range/NULL checks | A0022001/2/6 | R |
| venc | MI_VENC_GetStream | libmi_venc | 0x000132d0 | /dev/mi_venc | 0xc00c6942,0xc0186943,0xc0506945,0xc054690b,0x400c6948,0x4050690c,0x400c6948,0x4050690c | _IOWR('i',0x42,0xc) _IOWR('i',0x43,0x18) _IOWR('i',0x45,0x50) _IOWR('i',0xb,0x54) _IOW('i',0x48,0xc) _IOW('i',0xc,0x50) _IOW('i',0x48,0xc) _IOW('i',0xc,0x50) | see mi_venc.md (ioctls 0x42 0x43 0x45 0x0b 0x48 0x0c) | fills pstStream->pstPack[] (0xb8 each), u32Seq, NALU info | per-channel mutex; maps the encoder output buffer once with MI_SYS_Mmap and converts physical pack addresses to VA; FlushInvCache per pack; _MI_VENC_ParseNalu (start-code scan, H.264/H.265 NAL types) in user space -> mi_venc.md | A0022001/2/6, A0022003 (timeout<-1, NULL pack, ioctl fail), A0022009, A002201F (NALU parse) | R |
| venc | MI_VENC_ReleaseStream | libmi_venc | 0x00013a50 | /dev/mi_venc | 0x4050690c,0x400c6948 | _IOW('i',0xc,0x50) _IOW('i',0x48,0xc) | per pack: 0x48 {+0 u32 handle(+ dev/chn)}(0xc); then 0x0c {dev,chn,stStream}(0x50) | - | lock; FlushInvCache each pack VA; ioctl 0x48 per pack; clear pstStream->pstPack VA; ioctl 0x0c; unlock -> mi_venc.md | A0022001/2/6/9 | R |
| venc | MI_VENC_RequestIdr | libmi_venc | 0x00014200 | /dev/mi_venc | 0x400c6910 | _IOW('i',0x10,0xc) | +0 VeDev +4 VeChn +8 bInstant:u8 (+9..0xb zero) | - | lock/unlock the per-channel mutex around the ioctl |  | R |
| venc | MI_VENC_GetFd | libmi_venc | 0x000147cc | /dev/mi_venc | 0xc00c6914 | _IOWR('i',0x14,0xc) | 0x14: +0 VeDev +4 VeChn +8 u32 pollId (out) | returns the fd (not a MI code) | cached per channel; else open("/dev/mi_poll",O_RDWR), ioctl(venc,0x14) -> pollId, ioctl(pollfd, 0, pollId) binds the poll fd to that wait queue; on any failure close and return -1 -> mi-user-abi.md §5 | -1 | R |
| vif | MI_VIF_CreateDevGroup | libmi_vif | 0x00010ba4 | /dev/mi_vif | 0x40206900 | _IOW('i',0x0,0x20) | +0 GroupId +4 stGroupAttr:MI_VIF_GroupAttr_t(0x1c) | - | - |  | R |
| vif | MI_VIF_DestroyDevGroup | libmi_vif | 0x00010cbc | /dev/mi_vif | 0x40046902 | _IOW('i',0x2,0x4) | +0 GroupId | - | - |  | R |
| vif | MI_VIF_SetDevAttr | libmi_vif | 0x00010e98 | /dev/mi_vif | 0x40186904 | _IOW('i',0x4,0x18) | +0 DevId +4 stDevAttr:MI_VIF_DevAttr_t(0x14) | - | - |  | R |
| vif | MI_VIF_EnableDev | libmi_vif | 0x00010f88 | /dev/mi_vif | 0x40046906 | _IOW('i',0x6,0x4) | +0 DevId | - | - |  | R |
| vif | MI_VIF_DisableDev | libmi_vif | 0x00010fe8 | /dev/mi_vif | 0x40046907 | _IOW('i',0x7,0x4) | +0 DevId | - | - |  | R |
| vif | MI_VIF_SetOutputPortAttr | libmi_vif | 0x000110e0 | /dev/mi_vif | 0x401c6909 | _IOW('i',0x9,0x1c) | +0 DevId +4 PortId +8 stAttr:MI_VIF_OutputPortAttr_t(0x14) | - | - |  | R |
| vif | MI_VIF_EnableOutputPort | libmi_vif | 0x000111d8 | /dev/mi_vif | 0x4008690b | _IOW('i',0xb,0x8) | +0 DevId +4 PortId | - | - |  | R |
| vif | MI_VIF_DisableOutputPort | libmi_vif | 0x00011238 | /dev/mi_vif | 0x4008690c | _IOW('i',0xc,0x8) | +0 DevId +4 PortId | - | - |  | R |
| vif | MI_VIF_Query | libmi_vif | 0x00011298 | /dev/mi_vif | 0xc024690d | _IOWR('i',0xd,0x24) | +0 DevId +4 PortId +8 stStat:MI_VIF_DevPortStat_t(0x1c, out) | copy +8 (0x1c) if ret==0 | - |  | R |
| camos | CamOsTsemInit | libcam_os_wrapper | - | - | - | - | int CamOsTsemInit(CamOsTsem_t*, u32 val) | - | {u32 magic 0x55AA5AA5; sem_t (16 B)}: under a global pthread mutex, sem_init(pshared=1, val), set magic; already-inited -> -1 + message; NULL -> -2 |  | R |
| camos | CamOsTsemDeinit | libcam_os_wrapper | - | - | - | - | int CamOsTsemDeinit(CamOsTsem_t*) | - | sem_destroy, clear magic; not inited -> message, -1 |  | R |
| camos | CamOsTsemDown | libcam_os_wrapper | - | - | - | - | void CamOsTsemDown(CamOsTsem_t*) | - | sem_wait if magic set, else only prints an error and returns (no locking!) — this is what happens to g_mi_fd_semalock before MI_SYS_Init |  | R |
| camos | CamOsTsemUp | libcam_os_wrapper | - | - | - | - | void CamOsTsemUp(CamOsTsem_t*) | - | sem_post if magic set, else message |  | R |
| camos | CamOsMutexInit | libcam_os_wrapper | - | - | - | - | int CamOsMutexInit(CamOsMutex_t*) | - | {u32 magic; pthread_mutex_t (24 B)} = 0x1c B; pthread_mutex_init(NULL attr) under a global mutex; already-inited -> -1 |  | R |
| camos | CamOsMutexLock | libcam_os_wrapper | - | - | - | - | int CamOsMutexLock(CamOsMutex_t*) | - | lazy init if magic missing, pthread_mutex_lock; error -> -1 |  | R |
| camos | CamOsMutexUnlock | libcam_os_wrapper | - | - | - | - | int CamOsMutexUnlock(CamOsMutex_t*) | - | pthread_mutex_unlock [I same pattern] |  | I |
| camos | CamOsMutexDestroy | libcam_os_wrapper | - | - | - | - | int CamOsMutexDestroy(CamOsMutex_t*) | - | pthread_mutex_destroy, clear magic [I same pattern] |  | I |
| camos | CamOsRwsemInit | libcam_os_wrapper | - | - | - | - | int CamOsRwsemInit(CamOsRwsem_t*) | - | {magic; pthread_rwlock_t}; pthread_rwlock_init under the global mutex |  | R |
| camos | CamOsRwsemDownRead/DownWrite/UpRead/UpWrite | libcam_os_wrapper | - | - | - | - | int f(CamOsRwsem_t*) | - | pthread_rwlock_rdlock / wrlock / unlock |  | R |
| camos | CamOsMsSleep | libcam_os_wrapper | - | - | - | - | void CamOsMsSleep(u32 ms) | - | usleep(((ms+9)/10)*10000) — rounds UP to 10 ms steps |  | R |
| camos | CamOsUsSleep | libcam_os_wrapper | - | - | - | - | void CamOsUsSleep(u32 us) | - | usleep(us) |  | R |
| camos | CamOsGetTimeOfDay | libcam_os_wrapper | - | - | - | - | void CamOsGetTimeOfDay(CamOsTimespec_t*) | - | gettimeofday -> {u32 sec; u32 nsec = usec*1000} |  | R |
| camos | CamOsMemAlloc / CamOsMemCalloc / CamOsMemRelease | libcam_os_wrapper | - | - | - | - | void* f(u32 size) / f(u32 n,u32 size) / void f(void*) | - | malloc / calloc / free (Release ignores NULL) |  | R |
| camos | CamOsPrintf / CamOsSnprintf | libcam_os_wrapper | - | - | - | - | printf-like | - | vfprintf(stdout) / vsnprintf |  | R |
| camos | CamOsMathDivU64 / CamOsMathDivS64 | libcam_os_wrapper | - | - | - | - | u64 f(u64 n, u64 d, u64 *rem) | - | 64-bit division, remainder through the pointer if not NULL |  | R |

Payload structs of the wrappers prog_uvc does not call are named from the SigmaStar headers where those exist; their field
meanings are the kernel modules' and are given in the module documents.
