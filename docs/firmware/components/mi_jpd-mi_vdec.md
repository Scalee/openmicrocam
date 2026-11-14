# mi_jpd, mi_vdec — the MI decoders (mi_jpd.ko, mi_vdec.ko) and their user libraries
<!-- ev:mi_jpd-mi_vdec -->

The two decoders of the MI layer. Both are **mi_sys devices with no input port**: they are pure sources whose work is
driven by mi_sys's `_ASYNC` thread through the pass callbacks (poll → enqueue → interrupt → check → dequeue), and their
input — the compressed data — does not come from a bound producer but from an mi_sys **ring pool** per channel that
the user writes into.

- **mi_jpd** (module 0x20) decodes one JPEG picture per ring record through `MHAL_JPD_*` (the NJPD engine) into a YUYV
  frame at the picture's own (MCU-aligned, possibly scaled-down) size; its one output port binds FRAME_BASE or REALTIME.
  On the camera it serves the photo viewer (menu ids 0x1a / 0x1d 1).
- **mi_vdec** (module 8) feeds an elementary-stream (ES) ring to the Wave5 VPU through a 28-entry table of
  `MHAL_VDEC_*` and delivers NV12 frames on two output ports (port 0 scaled / cropped, port 1 full size); a second
  output is made by mi_scl device 3. It runs a per-channel state machine (stream on → sequence init → decode) woken
  by the VPU interrupt and an mi_sys HW timer. On the camera it serves the MP4 player (menu ids 0x1b / 0x1d).
- `libmi_jpd.so` (14 exports) and `libmi_vdec.so` (33 exports) are the user sides; both carry logic of their own
  (JPD: argument checks and the stream-buffer mapping; VDEC: stream splitting into frames).

Shared facts owned elsewhere: the user-side ioctl wrapper, payload packing, return codes, the MI error-code format,
module ids, the fd open / close rules and create counts — `mi-user-abi.md` (§2, §3, §4); kernel-side dispatch
(`MI_DEVICE_Register` / `MI_DEVICE_Ioctl`, the module mutex), the per-device procfs file, mi_sys exports, private heaps,
MMA, MIU protect, the HW timer — `mi_sys-2-interfaces.md` (§1, §6, §7, §8, §10); the registration contract (info record,
pass callback table), bind types and bind capability, the ASYNC thread, task results, the sideband message, SRAM and
the ring pools — `mi_sys-1-engine.md` (§2, §3, §5, §6.5, §7.2, §9, §10); mi_scl device 3 and `mi_scl_StretchBufByVdec`
— `mi-capture.md` §6.8; the `MHAL_JPD_*` side (NJPD driver, parameter ids, decode descriptor, interrupt) —
`mhal-jpd.md`; the `MHAL_VDEC_*` side (parameter and dev-config ids, StreamOn / SeqInit / DecodeOneFrame /
DecodeFrameDone descriptors, frame-buffer counts, eStatus mapper) — `mhal-vcodec.md` §8; the app's calls and values —
`prog_uvc.md` §7.5 (MP4 player), §7.6 (photo viewer); the inventory rows — `firmware-image.md` §3.3, §3.5.

Tags: **[R]** read from the binary (decompile, disassembly, `.data` / `.rodata`, relocations) or from a cited file;
**[S]** stated by a source (SDK header, vendor source, log string used as a name); **[I]** inference.

## 0. Notation and working files
<!-- ev:mi_jpd-mi_vdec.0 -->

- Binaries: `/config/modules/4.9.227/mi_jpd.ko` (100.8 KB file, `.text` 0x9808 bytes, 81 function
  symbols) and `mi_vdec.ko` (240.6 KB file, `.text` 0x17408 bytes, 119 function symbols); MI build
  `project_commit.7251436 sdk_commit.80327e1 build_time.20220721223310`; full ELF symbols, no debug info, no exports [R].
  The `.text` sizes and symbol counts are those of `readelf` on the dumped files and of `firmware-image.md` §3.3
  (an earlier count of ours, "54 KB code, 92 functions" for mi_jpd and "143 KB code" for mi_vdec, is a measure of another
  kind).
- **Addresses** are `.text` offset + 0x10028 unless written
  "objdump" (`.text` offset).
- **Error codes** are `0xA0202xxx` for JPD (module 0x20) and `0xA0082xxx` for VDEC (module 8); `…xx` writes only the
  low byte. Low bytes these modules return [R: values; names from 
 `MI_ErrCode_e` [S]]: 1 INVALID_DEVID, 2 INVALID_CHNID, 3 ILLEGAL_PARAM, 4 EXIST, 5 UNEXIST, 6 NULL_PTR, 7 NOT_CONFIG,
  8 NOT_SUPPORT, 9 NOT_PERM, 0xc NOMEM, 0xe BUF_EMPTY, 0xf BUF_FULL, 0x11 (GetBSBuffer wrapper, ES phys 0; name not
  given), 0x12 BUSY, 0x13 CHN_NOT_STARTED, 0x14 CHN_NOT_STOPED, 0x15 NOT_INIT, 0x16 INITED, 0x18 NOT_DISABLE,
  0x1a DEV_NOT_STARTED, 0x1c CHN_NO_CONTENT, 0x1f FAILED. Codes of mi_sys's own module (9) pass through where said
  (`0xA009200C` ring full, `0xA0092008` bind refused).
- HZ = 100 on this kernel, so 1 jiffy = 10 ms (`kernel-platform.md` §15.1) [R].
- mi_jpd: **dev** = the device record, **chn** = the channel record (§1.2), **dbg** = the channel's debug record
  (§1.9). mi_vdec: **D** = the device record (`.bss+0x37800`), **D'** = D + 0x208 (the pointer mi_sys and the channels
  hold), **P** = the channel record, **S** = the channel's part of `_stVdecDbgInfo` (§2.2), **r** = the 0xb0-byte
  DecodeFrameDone result (§2.11.4), **meta** = the 0x50-byte meta record (§2.13).
- "→ 0" at the end of a step list = the call returns 0.

**Common to both modules** [R]: `init_module` creates `/proc/mi_modules/mi_<mod>/{debug_level, debug_file, debug_func,
module_version_file}` and calls `MI_DEVICE_Register` (→ `/dev/mi_jpd`, `/dev/mi_vdec`); `cleanup_module` reverses it;
`depends=mi_common` only (the mi_sys and mhal imports resolve through load order). Every ioctl handler takes the
module-wide mutex and leaves the unlock to mi_common after the copy-back — so a JPD or VDEC call that waits for ring
space (GetStreamBuf, SendStream) blocks every other call of the same module for that time.

## 1. mi_jpd.ko

### 1.1 Load, open, close [R]
- Module parameters: only `debug_level`, `debug_file`, `debug_func`. No device node besides `/dev/mi_jpd`.
- `process_init` = `MI_JPD_IMPL_Init`: on the first open, zero the 300-byte module state (§1.2) and set "module inited";
  no hardware is touched.
- `process_exit` = `MI_JPD_IMPL_DeInit` — **not refcounted**: closing *any* `/dev/mi_jpd` fd runs the DeInitDev sequence
  of §1.4 for dev 0 when it is inited (stop and destroy every channel, unregister the ISR, reset and destroy the mhal
  device, unregister the mi_sys device), then module inited = 0; it always returns 0. libmi_jpd never closes its fd
  (`mi-user-abi.md` §2.2).

### 1.2 Module state, device record, channel record [R offsets from every accessor; names from the BUG_ON / log strings]
Module state (300 B, `.bss`): `+0` u8 module inited, `+1` u8 dev 0 inited ("mhal device created"), `+4` the device
record (0x128 B; only dev 0 exists):

| dev off | field | written by |
|---|---|---|
| +0x00 | u8 registered with mi_sys | registration `jpd_register_dev` (0x105f0) (idempotent: returns 0 at once when set) |
| +0x04 | dev id (0) | registration |
| +0x08 | mi_sys device handle | `mi_sys_RegisterDev` |
| +0x0c | channel count = 16 | registration |
| +0x14…+0xab | the 0x98-byte pass table handed to mi_sys (slot = offset − 0x14) | registration |
| +0xb0…+0xef | `pastChnRes[16]` channel pointers | CreateChn |
| +0xf4 | u8 `u8MaxTaskCnt` = pass slot 0x8c value (1) | registration |
| +0xf8 | atomic **working task count** (0..1; shared by the async path and the direct ioctls 0xc / 0xd) | enqueue / dequeue / 0xc / 0xd |
| +0xfc | wait queue `stTcondWaitDecode` — one queue for both waits: the task slot (ioctls 0xc / 0xd; woken by the async poll and dequeue) and the end of a direct decode (woken by the ISR) [R: both waits and the ISR use `.bss+0x310` = dev +0xfc] | registration |
| +0x114 | mhal device handle (`MHAL_JPD_CreateDevice`) | InitDev |
| +0x118 | channel id of the decode in flight (the ISR finds its channel through it) | enqueue / DirectBufDecode |
| +0x11c | u8: 1 = the decode in flight is an async (ring) task, 0 = a direct one — the ISR wakes the direct wait queue only when 0 | enqueue sets 1, DirectBufDecode sets 0 |
| +0x120 | picture width of the last REALTIME-bound task (SRAM sizing, §1.8) | async poll |
| +0x124 | atomic count of REALTIME-bound channels | bind / unbind output |

Channel record (0x110 B, `CamOsMemAlloc`, ≤ 16):

| chn off | field |
|---|---|
| +0x00 / +0x01 | u8 created / u8 started |
| +0x04 | chn id |
| +0x08 | device pointer |
| +0x0c | CamOs semaphore guarding the channel |
| +0x2c…+0x38 | create attr {u32 stream-buffer (ring) size, u32 attr word 1, u32 max width, u32 max height} |
| +0x3c | copy of the ring size |
| +0x40 / +0x44 | **original picture size** (the SOF size, `MHAL_JPD_GetParam 3`, mhal ctx +0x14c / +0x14e) |
| +0x48 | output pixel format (always 0) |
| +0x4c / +0x50 | **buffer size** — the MCU-aligned (and scaled-down) output size from `MHAL_JPD_ExtractJpgInfo` (mhal ctx +0x148 / +0x14a) |
| +0x54 | ring-pool handle |
| +0x58 / +0x60 / +0x64 | ring phys / ring VA / ring size |
| +0x68… | reserved tail record of the current GetStreamBuf (phys at +0x88 / +0x8c) |
| +0x94 | committed content length |
| +0xb8 | "DecState" — only CreateChn writes it (= 0); the poll branch for state 1 ("header already parsed") is unreachable in this build [R: no other writer] |
| +0xbc | u8 channel max task count (1) |
| +0xc0 | atomic channel task count |
| +0xc4 | mhal instance |
| +0xc8 | decode status (3 decoding, 1 done, other = error; written by the ISR) |
| +0xd0 | u64 decoded-frame counter of the direct path |
| +0xd8 | output bind type (4 = REALTIME) |
| +0xdc | u8 "realtime ready" |
| +0xf0 | decode-time smoothing window (§1.9) |
| +0x100 / +0x104 | decode start / end time (ms) |
| +0x108 | record min-alloc size (shown as `MinAllocSize`) |

Attr word 1 is only stored; it never reaches mhal [R].

### 1.3 mi_sys registration (`jpd_register_dev`, first InitDev) [R]
`mi_sys_RegisterDev` info: module **0x20**, dev 0, **0 input ports, 1 output port**, 16 channels, 1 pass, info +0x20 = 0
(device-wide wake-up, `mi_sys_NotifyWakeupISREvent`), info +0x2c `OnConfigSramBuf` (§1.8), +0x30 `AllowPreemptionSram`
(returns 1); procfs ops: the dump callbacks of §1.10. Right after the registration the three proc commands are registered
(§1.10). Pass table (slot = offset inside the 0x98-byte table, `mi_sys-1-engine.md` §2.3):

| pass slot | callback | what it does |
|---|---|---|
| 0x10 | `OnPrepareAsyncOutputTaskMetaInfo` | write the output buffer's u16 width / height (+0x80 / +0x82) = the original picture size (chn +0x40 / +0x44) [I: the frame's visible size]; REALTIME bind → buffer +0x72 = 0x10 |
| 0x14 / 0x18 | `EnqueueAsyncOutputTask` / `BarrierAsyncOutputTask` (no-op) | start one decode (§1.7) |
| 0x24 / 0x28 | `CheckOutputTaskStatus` / `DequeueOutputTask` | completion, timeout, release of the bitstream record (§1.7) |
| 0x2c | `OnPollingAsyncOutputTaskConfig` | decide whether a task can start and with what output buffer (§1.7) — its presence makes mi_sys run the `_ASYNC` thread for the device |
| 0x40 / 0x44 | input capability (stub, returns 0, writes nothing → any bind into JPD is refused, `0xA0092008`) / **output capability = mask 5 (FRAME_BASE \| REALTIME)** | |
| 0x48 / 0x50 | bind / unbind input (no-op) | |
| 0x58 / 0x60 | `OnBindOutputPort` / `OnUnBindOutputPort` | store the bind type (chn +0xd8); REALTIME → device counter +0x124 ±1 and the channel's "realtime ready" flag |
| 0x68 | `OnOutputPortBufRelease` | `mi_sys_NotifyWakeupISREvent` (a consumer released a frame → the next decode may start) |
| 0x8c | concurrent-task limit = 1 | |
| 0x92 | u16 = 200 | output-task timeout in ms (`mi_sys-1-engine.md` §2.3) — the same value is the floor of the module's own timeout in `CheckOutputTaskStatus` |

### 1.4 Device bring-up and teardown — `MI_JPD_IMPL_InitDev` / `DeInitDev` (ioctls 0 / 1) [R]
- **InitDev(dev)** — ioctl 0 (unused by the app) or implicitly from the first CreateChn (`InitDev(0)` when dev 0 is not
  inited, with a zero init word — the app's only path). The init parameter (`MI_JPD_InitParam_t`, one word) is **ignored**:
  `MI_JPD_IMPL_InitDev(dev)` has no use of it.
  1. dev 0 already inited → **`0xA0202009` NOT_PERM**.
  2. Register with mi_sys (§1.3; already registered → skipped); failure → `0xA0202007`. Log "create mi dev%u".
  3. `MHAL_JPD_Init(0)` → `MHAL_JPD_SetParam(0, 0xb, &memOps)` with memOps = `{alloc → mi_sys_PrivateDevChnHeapAlloc(h,
     −1, …), free → mi_sys_PrivateDevChnHeapFree, map → mi_sys_Vmap, unmap → mi_sys_UnVmap, flush →
     mi_sys_VFlushInvCache}` (mhal takes its working memory from mi_sys) → `MHAL_JPD_CreateDevice(0, &hdl)` (→ dev +0x114)
     → `MHAL_JPD_RegisterIsrCallback(hdl, _MI_JPD_IsrFuncCBDev0)`. Log "create mhal dev%u"; dev inited = 1; return 0.
  4. Any mhal failure → **`0xA0202007` NOT_CONFIG** (an ISR-registration failure destroys the mhal device first). The mi_sys
     registration made before the failure is **kept**; the next InitDev reuses it.
- **DeInitDev(dev)** — ioctl 1 (unused by the app) and `process_exit` (DeInit):
  1. Not inited → **`0xA0202009`**.
  2. StopChn (§1.5) for every created channel that is started; then DestroyChn for every created channel. Log "stop mi
     dev%u".
  3. `jpd_mhal_teardown` (0x12a08): `MHAL_JPD_UnRegisterIsrCallback`, `MHAL_JPD_ResetDev(hdl, 0)`, `MHAL_JPD_DestroyDevice` — its result
     is returned: non-zero → `0xA020201F`, the device stays half-down, inited stays 1. Log "destroy mhal dev%u".
  4. `jpd_sys_unregister` (0x13374): `mi_sys_UnRegisterDev` (failure → `0xA020201F`, but the teardown continues), free every channel
     record (semaphore deinit + `CamOsMemRelease`), registered = 0. Log "destroy mi dev%u"; inited = 0; return 0.

### 1.5 ioctls (`/dev/mi_jpd`, 16 entries) [R]
Common checks: dev ≠ 0 → `0xA0202001`; chn ≥ 16 → `0xA0202002`; a NULL out pointer → `…06`; a channel that is not
created → `…05`. Payload sizes are the library's (§1.12); **bold** = used by the app.

| nr | name | payload | kernel action |
|---|---|---|---|
| 0 / 1 | InitDev / DeInitDev | `{u32 dev, MI_JPD_InitParam_t (1 word)}` (8) / `{u32 dev}` (4) | §1.4 |
| **2** | CreateChn | `{dev, chn, attr 0x10}` (0x18) | implicit InitDev (§1.4); channel exists → `0xA0202004` (the library maps it to 0); store the attr; **create the ring pool `"jpd-out-D<dev>C<chn>"`** of attr[0] bytes (min record 0x400, alignment 0x20, 1 ring — `mi_sys-1-engine.md` §10.1) and read back its phys / VA / size; `MHAL_JPD_CreateInstance(dev, &inst)`; `MHAL_JPD_SetParam(inst, 1, {u16 attr[2], u16 attr[3]})` and `SetParam(inst, 0, same)` = the **progressive / baseline maximum decode resolution** (with the app's 0x2000 the SOF limit is 65528 px and pictures above 8192 px are scaled down by 2 / 4 / 8 — `mhal-jpd.md` §6.5); task limits; the smoothing window (8 samples) and the debug record (§1.9); any failure → `0xA020201F` and the channel is freed |
| **3** | DestroyChn | `{dev, chn}` (8) | not created → `0xA0202005`; still started → `0xA0202014`; `MHAL_JPD_DestroyInstance`, free the smoothing window (`_MI_JPD_FreeSmoothWindowInfo`), `mi_sys_RingPool_Destroy` |
| 4 | GetChnAttr | `{dev, chn}` in, 0x18 back (0x20) | unused; below |
| **5** | StartChn | `{dev, chn}` | `mi_sys_EnableOutputPort(h, chn, 0)` + `mi_sys_EnableChannel(h, chn)`; started = 1 |
| **6** | StopChn | `{dev, chn}` | `mi_sys_DisableChannel` + `mi_sys_DisableOutputPort`; then wait for the channel's task count to drop to 0 (10-ms steps, ≤ 1 s; still busy → `0xA0202013`); a task still in flight → `MHAL_JPD_ResetDev(dev, 0)`; started = 0 |
| 7 | GetChnStatus | `{dev, chn}` in, 0x1c back (0x24) | unused; below |
| 8 | ResetChn | `{dev, chn}` (8) | unused; below |
| **9** | GetStreamBuf | `{dev, chn, u32ReqLen, +0x10 MI_JPD_StreamBuf_t (out), +0x38 s32MilliSec}` (0x40) | not started → `0xA0202013`; no ring → `0xA0202007`; `mi_sys_RingPool_SetRecordMinAllocSize(ring, ReqLen)`; release the channel semaphore and **`mi_sys_RingPool_EnsureTailSpaceTimeout(ring, s32MilliSec, &tailRec, 0, 1)` — waits up to the caller's timeout (the app passes −1) while the module mutex stays held**; success with space ≥ ReqLen → StreamBuf +0x08 u64 phys, +0x10 length = ReqLen (one contiguous segment; the second segment is never filled), the record is kept at chn +0x68…; else wake mi_sys and return **`0xA020200F` BUF_FULL** |
| **0xa** | PutStreamBuf | `{dev, chn, StreamBuf 0x28}` (0x30) | phys must equal the reserved record's phys (else `0xA0202003`); content length = StreamBuf +0x24, or +0x10 when that is 0; **`mi_sys_RingPool_AppendTailContentTailRecord`** commits the record (one closed record per picture); input statistics (§1.9) and the `dumpbs` append (§1.10); if no direct decode is running, `mi_sys_NotifyWakeupISREvent(h, 0)` |
| **0xb** | DropStreamBuf | `{dev, chn, StreamBuf}` (0x30) | **no-op** after the argument checks, returns 0 (the reserved tail is simply not committed) |
| 0xc | QueryStreamInfo | (0x38) | unused; §1.6 |
| 0xd | DirectBufDecode | (0x48) | unused; §1.6 |
| **0xe** | RingPool_GetRingPoolInfo | `{handle, +8 u64 phys (out), +0x10 size (out)}` (0x18) | `mi_sys_RingPool_GetRingPoolTotalSpace(handle)` — the library maps this range once with `MI_SYS_Mmap` |
| **0xf** | GetRingPoolHandle | `{dev, chn, out handle}` (0xc) | not started → `0xA0202013`; returns the channel's ring-pool handle — a kernel pointer in this build |

**GetChnAttr (4)**: under the channel semaphore copies six words `{+0x3c ring size, +0x40 picture width, +0x44 picture
height, +0x48 pixel format (0), +0x4c buffer width, +0x50 buffer height}`; before the first picture the four sizes are 0
[I: nothing else writes them]. Field names [I]: `{u32StreamBufSize, u32PicWidth, u32PicHeight, ePixelFormat,
u32BufWidth, u32BufHeight}` — the names the QueryStreamInfo debug print uses (`u32CurPicWidth … u32BufHeight`).

**GetChnStatus (7)**: out NULL → `…06` ("pstChnStatus"). Under the semaphore:

| out | value |
|---|---|
| +0x00 | `LeftStreamBytes` (dbg +0x90: bytes committed by PutStreamBuf and not yet decoded) |
| +0x04 | records in the ring (`mi_sys_Ringpool_GetRecordTotalNum`); without a ring: (LeftStreamBytes ≠ 0) |
| +0x08 | u16 from `mi_sys_GetChnBufInfo(h, chn, port 0)` at info +0xac — pictures queued at the output port [I meaning] |
| +0x0c | `SendCnt` (dbg +0x88, pictures committed) |
| +0x10 | `DecCnt` (dbg +0xf0, pictures decoded) |
| +0x14 | u8 started |

A failing ring or buf-info query → `0xA020201F`. Names [I]: `{u32LeftStreamBytes, u32LeftStreamFrames, u32LeftPics,
u32RecvStreamFrames, u32DecodeStreamFrames, bChnStart}`.

**ResetChn (8)**: started → **`0xA0202008` NOT_SUPPORT** (not CHN_NOT_STOPED). Stopped: with a ring,
`mi_sys_RingPool_GetHeadRecordTimeout(ring, chn, &rec, mark 1, 0 ms)` + `ReleaseHeadRecord` in a loop until the ring is
empty → 0. Counters are not reset.

### 1.6 The direct ioctls — 0xc `QueryStreamInfo`, 0xd `DirectBufDecode` (unused) [R]
Input buffer `MI_JPD_DirectInputBuf_t` (0x18) = `{+0 user VA, +8 u64 phys, +0x10 u32 length}`.

**QueryStreamInfo** — payload `{dev, chn, inputBuf = in (0x18) at +8, MI_JPD_StreamInfo_t out (0x14) at +0x20}`:
1. Checks dev, chn, `in` / `out` NULL (`…06`), created (`…05`), **started** (else `0xA0202013`), VA ≠ 0 and length ≠ 0
   (else `0xA0202003`).
2. **Takes the device's single task slot**: `CompareAndSwap(dev+0xf8, 0 → 1)`; while busy: release the channel semaphore,
   sleep on dev +0xfc in 20-jiffy (200 ms) slices until the count is 0, retake the semaphore, retry. While the slot is
   held the async poll sees the device full and starts nothing.
3. Input copy: phys = 0 → `mi_sys_MMA_Alloc("mma_heap_name0", "jpd-direct", length)`; else the caller's phys is used as
   is. `mi_sys_Vmap(phys, length, uncached 0)` (failure → `…1F`), then **`CamOsCopyFromUpperLayer(kva, user VA, length)`**
   — the data always come from the user VA, also when a phys was given (they are then copied onto that buffer).
4. `_MI_JPD_ParserHeader(chn, {VA1 = kva, phys1, size1 = length, VA2 = kva + length, phys2 = phys + length, size2 = 0},
   &wh)` = `MHAL_JPD_SetParam(inst, 6, buf)` + `MHAL_JPD_ExtractJpgInfo` on the channel's own instance. Success:
   out +0x0c / +0x10 = buffer (aligned, scaled) width / height; `MHAL_JPD_GetParam(inst, 3)` → out +0x00 / +0x04 =
   original width / height. **out +0x08 (pixel format) is not written** — whatever the library left in the payload is
   returned.
5. `mi_sys_UnVmap`; the temporary block → `mi_sys_MMA_Free` (failure → `…1F`); release the task slot (atomic −1);
   semaphore up.
- Errors from the header parse or GetParam are returned **raw** (mhal codes such as 6, `mhal-jpd.md` §4.1, §3.2), not as
  `0xA020xxxx`. The channel's decode statistics are not touched; its async decodes are only delayed.

**DirectBufDecode** — payload `{dev, chn, inputBuf = in (0x18) at +8, outputBuf = MI_JPD_DirectOutputBuf_t out (0x28) at +0x20}`; output
buffer = `{+0 u32 (pixel format [I]), +4 u32 width, +8 u32 height, +0xc u32 stride[0], +0x10 u32 stride[1], +0x18 u64
phys[0], +0x20 u64 phys[1]}` — five u32 and two u64, matching the debug print `pstOutputBuf(%u %u %u %u %u 0x%llx 0x%llx)`.
1. Checks as QueryStreamInfo plus `out->phys[0] ≠ 0` (else `…03`); takes the task slot the same way.
2. Poll timestamp (dbg +0x150); input as QueryStreamInfo step 3 (temporary `"jpd-direct"` block when phys = 0).
3. `_MI_JPD_ParserHeader` → chn +0x4c / +0x50 = buffer size. Then **`out->height × out->stride[0] < bufW × bufH × 2`**
   (YUYV, 2 bytes per pixel) → `…03` with the log "wrong output info … right:w:%u h:%u". `GetParam 3` → chn +0x40 / +0x44.
4. `mi_sys_Vmap(out->phys[0], height × stride[0])` (failure → `…1F`); decode descriptor (0x80 B, this build's
   `MHAL_JPD_JpgFrame_t`, `mhal-jpd.md` §3.3): input VA1 / phys1 / size1 = the input block, VA2 / phys2 = its end, size2 0;
   output VA[0] = the kernel mapping, phys[0] = `out->phys[0]`, +0x74 = height × stride[0]; all else 0. **`SetParam 8`
   (output kind) is not sent** — the instance keeps the kind of its last async decode (0 = frame on a fresh instance;
   0xd / 0xe when the channel is REALTIME-bound, so the direct decode would then run as an IMI / EMI ring decode)
   [R no call; I consequence].
5. Start time chn +0x100, dev +0x118 = chn, status (chn +0xc8) = 3, dev +0x11c = 0, `MHAL_JPD_StartDecodeOneFrame`
   (always returns 0). Statistics: SendStreamSize / LeftStreamBytes += length, input rate step (§1.9), input DirectCnt
   (dbg +0xa0) +1, EnqueueCnt (dbg +0x98) +1.
6. Semaphore up, **wait on dev +0xfc while status = 3, at most 20 jiffies (200 ms) in total** (a spurious wake-up
   continues with the remaining time). Semaphore down, check timestamp (dbg +0x15c), LeftStreamBytes −= length.
7. Status 1 → done: dequeue timestamp (dbg +0x160), end time, decode time into the smoothing window, output rate step
   (DecCnt, ofps, interval), output DirectCnt (dbg +0xf8) +1, chn +0xd0 (u64) +1 → 0. Any other status (timeout or
   error) → **`0xA020201F`**, output DropCnt (dbg +0xf4) +1, and **no `MHAL_JPD_ResetDev`** — a hung decode is left
   running (the async path resets on its own timeout).
8. UnVmap output and input, free the temporary block, release the task slot, `mi_sys_NotifyWakeupISREvent(h, 0)` (lets
   the async path continue), semaphore up.
- The frame is written straight into the caller's buffer; nothing is copied back in the payload.

### 1.7 The decode path (mi_sys `_ASYNC` thread of the JPD device) [R calls and values; I where marked]
One picture per ring record, at most one decode in flight per device and per channel.
1. **`OnPollingAsyncOutputTaskConfig`** (poll timestamp dbg +0x150): channel created and started, device and channel task
   counts below their maximum (1 each), the ring holds a record. Peek the head record (no wait) and parse the JPEG
   header: `MHAL_JPD_SetParam(inst, 6, &bitstreamDesc)` + `MHAL_JPD_ExtractJpgInfo(inst, &desc, &u32 wh)` → the
   **buffer size** (MCU-aligned, scaled down above the maximum resolution) at chn +0x4c / +0x50; zero width / height or a
   failure → **drop the record** (`mi_sys_RingPool_ReleaseHeadRecord`) and report "nothing to do" (`0xA020201C`). Else
   `MHAL_JPD_GetParam(inst, 3, &u32 wh)` → the original SOF size at chn +0x40 / +0x44. Fill mi_sys's output-buffer
   config: frame type 1, **pixel format 0** [I: YUV422 YUYV], u16 width / height = the buffer size, PTS −1; REALTIME bind
   → dev +0x120 = the picture width (§1.8). So **the output buffer has the JPEG's own size** (rounded to MCUs); the bound
   SCL scales it to the display layer.
2. mi_sys allocates the output buffer → `OnPrepareAsyncOutputTaskMetaInfo` (§1.3).
3. **`EnqueueAsyncOutputTask`**: take the device task slot — taken (e.g. by ioctl 0xc / 0xd) → RewindCnt (dbg +0x94) +1,
   return 2 (retry later); skip the task (return 3, input DropCnt dbg +0x9c +1) when the channel was stopped or a
   REALTIME bind is not ready. Take the head record again (bitstream phys / VA / length → the 0x80-byte decode
   descriptor). Output target, by the output buffer's type field (`buf+0x34`):
   - type 2 → one plane at `buf+0x48`, size `buf+0x6c`; output kind **0 = frame** into the mi_sys buffer;
   - type 0 [I: the buffer of a REALTIME bind] → `MHAL_JPD_SetParam(inst, 9, {0x10})` = output ring line count (16
     lines) and output kind **0xd = IMI ring** or **0xe = EMI ring** (row handshake with SCL), chosen by task word 0x1b
     [I: mi_sys's SRAM result, §1.8]; address / size from task words 0x1c–0x1e.

   Then `MHAL_JPD_SetParam(inst, 8, &outFmt)` (the output kind), status (chn +0xc8) = 3, start time chn +0x100, dev +0x118 = chn,
   dev +0x11c = 1, **`MHAL_JPD_StartDecodeOneFrame(inst, &desc)`**, channel task count +1, EnqueueCnt +1, enqueue
   timestamp. Failure → the task fails (3).
4. **ISR** `_MI_JPD_IsrFuncCBDev0(inst, status)` (registered with `MHAL_JPD_RegisterIsrCallback`; mhal owns the IRQ).
   mhal calls it only when the "decode done" event is set (status 1 = clean, 2 = done with error bits); an interrupt
   with error bits alone produces no callback, so for those mi_jpd's timeout is the only error path. The handler writes
   the status into the channel of dev +0x118, counts IsrDoneCnt (status 1) or IsrOtherCnt, stamps the time, calls
   `mi_sys_NotifyWakeupISREvent(h, 0)` and, when dev +0x11c = 0, wakes the direct decode waiting on dev +0xfc.
5. **`CheckOutputTaskStatus(…, elapsed, dev)`** returns 0 pending, 1 done, 3 failed: channel gone or stopped → fail;
   status 1 → done (output rate step of §1.9, frame dump when `dumpfb` is on); status 3 → pending while elapsed <
   **max(200, 2 × smoothed decode time)** ms, else fail; any other status → fail. Every failure calls
   `MHAL_JPD_ResetDev(dev, 0)` and counts output DropCnt (dbg +0xf4).
6. **`DequeueOutputTask`**: decode time into the smoothing window, frame counter, **release the head bitstream record**
   (`mi_sys_RingPool_GetHeadRecordTimeout` + `ReleaseHeadRecord`), LeftStreamBytes −= length, task counts −1, wake the
   device wait queue. mi_sys then delivers the output buffer to the bound input.

### 1.8 SRAM for REALTIME binds [R; I where marked]
`_MI_JPD_OnConfigSramBuf` (registration +0x2c, active on M6 — `mi_sys-1-engine.md` §9): while at least one channel is
REALTIME-bound (dev +0x124 > 0) and a REALTIME task has set dev +0x120: need = 1, size = **width × 32 bytes** [R size;
I: 16 YUYV lines — the ring line count `SetParam 9 = 0x10` of §1.7], keep = 0. `AllowPreemptionSram` returns 1. So a
REALTIME bind to SCL decodes into IMI SRAM (output kind 0xd) when mi_sys grants the block [I].

### 1.9 Statistics and the decode-time smoothing window [R]
**Per-channel debug record** `_astChnDbgInfo[dev][chn]` (`.bss+0x340`, 0x168 B per channel, 0x1680 per device; zeroed by
CreateChn, which also sets both "Min" interval fields to all-ones and both window starts to the current jiffies):

| off | field | updated by |
|---|---|---|
| +0x02 / +0x03 | u8 dump-bitstream / dump-frame enable | `dumpbs` / `dumpfb` |
| +0x04 | char[0x80] dump file name | the three commands |
| +0x88 | SendCnt (input pictures counted by the input rate step) | input step |
| +0x8c / +0x90 | SendStreamSize (bytes, total) / LeftStreamBytes (committed − decoded) | PutStreamBuf, DirectBufDecode, dequeue (−) |
| +0x94 | RewindCnt — enqueue found the task slot taken (returns 2 = retry later) | enqueue |
| +0x98 | EnqueueCnt | enqueue, DirectBufDecode |
| +0x9c | DropCnt (input) — enqueue failed (returns 3) | enqueue |
| +0xa0 | DirectCnt (input) | DirectBufDecode |
| +0xa4 / +0xa8 / +0xac | input 1-s window start (jiffies) / pictures in this window / **ifps** (last full window) | input step |
| +0xb0 (u64) | input interval count | input step |
| +0xb8 / +0xc0 | previous / current input time (u64 ms) | input step |
| +0xc8, +0xd0, +0xd8, +0xe0, +0xe8 | input interval Diff, Min, Max, Sum, Avg = Sum / count (u64 ms) | input step |
| +0xf0 / +0xf4 / +0xf8 | DecCnt / output DropCnt (failed or timed-out decodes, async and direct) / output DirectCnt | output step, check, DirectBufDecode |
| +0xfc / +0x100 / +0x104 | output 1-s window start / pictures in window / **ofps** | output step |
| +0x108 (u64) | output interval count | output step |
| +0x110 / +0x118 | previous / current output time | output step |
| +0x120, +0x128, +0x130, +0x138, +0x140 | output interval Diff, Min, Max, Sum, Avg | output step |
| +0x148 / +0x14c | IsrDoneCnt (status 1) / IsrOtherCnt (any other status) | ISR |
| +0x150 … +0x160 | Poll, Enqueue, Isr, Check, Dequeue timestamps (u32 ms) | each step |

- **Rate step** (`jpd_rate_step_in` (0x13440) input, called by PutStreamBuf and DirectBufDecode; `jpd_rate_step_out` (0x12004) output, called on DONE
  by CheckOutputTaskStatus and by DirectBufDecode): count +1 (+0x88 / +0xf0), window count +1; when more than `HZ`
  jiffies passed since the window start, fps = window count, window count = 0, window start = now. Interval: now (ms)
  against the previous time; the first call only records the time; later calls set Diff, Min, Max, Sum += Diff,
  count +1, Avg = Sum / count.
- **Smoothing window** (chn +0xf0, `_MI_JPD_AllocSmoothWindowInfo(…, 8)` at CreateChn): `+0` u32 smoothed value, `+4`
  pointer to a u32[max] array, `+8` running sum, `+0xc` u8 write index, `+0xd` u8 active count, `+0xe` u8 max count = 8.
  Append(v): sum −= slot[idx]; slot[idx] = v; sum += v; idx = (idx + 1) mod max; active = min(active + 1, max);
  value = sum / active. Values are decode times in ms (start = chn +0x100 at enqueue / DirectBufDecode, end = chn
  +0x104; a non-increasing end appends nothing). Its only user is the async timeout max(200, 2 × smoothed) ms of §1.7
  step 5; it is 0 before the first decode, so the timeout is then 200 ms. Freed by DestroyChn.

### 1.10 procfs: commands and views of `/proc/mi_modules/mi_jpd/mi_jpd0` [R]
Commands are registered with `mi_sys_RegistCommand(name, argc incl. the name, fn)` right after the mi_sys registration;
`echo "<cmd> <args>" > /proc/mi_modules/mi_jpd/mi_jpd0` runs them. Wrong argc → usage text, return 0; chn ≥ 16 or not
created → `0xA020201F`. All three write with `filp_open(path, O_WRONLY|O_CREAT|O_APPEND|O_SYNC (0x101441), 0644)` +
`vfs_write` under `set_fs(KERNEL_DS)` (0x10c40); a failed open returns raw 0x1F.

| command | args | does |
|---|---|---|
| `dumpsb` | `chn path` | **at once**: file `<path>/chn<chn>_stream_buffer_pts<now>.jpg` ← the **whole ring buffer** (VA chn +0x60, size chn +0x64), whatever is in it — a raw ring image, not a JPEG |
| `dumpbs` | `chn enable path` | set dbg +2 = enable and build the name `<path>/chn<chn>_bitstream_pts<now>.jpg` once; then every **PutStreamBuf** appends the committed record to that one file: VA = ring VA + (record phys − ring phys), **length = the reserved length (StreamBuf +0x10), not the content length** (with the app, GetStreamBuf len = file size, the two are equal); a write failure clears the enable |
| `dumpfb` | `chn enable path` | set dbg +3 and the name `<path>/output_res_<W>x<H>_format<fmt>_chn<chn>_pts<now>.yuv` (W / H = chn +0x40 / +0x44 at command time, fmt = chn +0x48); then every async decode that ends DONE in `CheckOutputTaskStatus` appends the output frame (`mi_sys_Vmap(buf+0x48 phys, buf+0x6c size rounded up to 4 KiB)`, length `buf+0x6c`); direct decodes are not dumped |

The file name lives in dbg +4 (0x80 bytes); "enable" with a new path rebuilds it, enable 0 stops. `_MI_JPD_OnHelp`
prints nothing (returns 0) — the commands are discoverable only through their usage text.

**Read view** (`cat` the same node; mi_sys calls the module's dump callbacks `OnDumpDevAttr`, `OnDumpChannelAttr`,
`OnDumpOutPortAttr` — the last is empty and returns 0):
- "Private Jpd0 Info / DEV Info": `DevId`, `WorkingTaskCnt` (dev +0xf8).
- "CHN Info", one row per created channel: `ChnId Start DecState Width Height BufW BufH StreamBufSize MinAllocSize
  WorkingTaskCnt IsrDoneCnt IsrOtherCnt` = chn +4, +1, +0xb8, +0x40, +0x44, +0x4c, +0x50, +0x2c, +0x108, atomic +0xc0,
  dbg +0x148, dbg +0x14c.
- `ChnId Poll Enque Isr Check Dequeue`: the five timestamps of the last task, dbg +0x150 … +0x160 (ms,
  `CamOsGetMonotonicTime` / 10⁶); then `P->E E->I I->C C->D P->D` (their differences) and `SmoothDecodeTime` (chn +0xf0).
- "CHN INPUT Info": `SendCnt LeftCnt SendStreamSize LeftStreamBytes EnqueueCnt DirectCnt DropCnt RewindCnt ifps` = dbg
  +0x88, ring record count, +0x8c, +0x90, +0x98, +0xa0, +0x9c, +0x94, +0xac; then `MinIntvl MaxIntvl SumIntvl AvgIntvl
  Diff` (u64, ms) = dbg +0xd0, +0xd8, +0xe0, +0xe8, +0xc8.
- "CHN OUTPUT Info": `DecCnt DirectCnt DropCnt ofps` = dbg +0xf0, +0xf8, +0xf4, +0x104; then the same five interval
  columns = dbg +0x128, +0x130, +0x138, +0x140, +0x120.

A failed ring query in the input table makes the read return `0xA020201F` (the table is still printed).

### 1.11 Imports and exports [R]
- mhal: `MHAL_JPD_{Init, CreateDevice, DestroyDevice, CreateInstance, DestroyInstance, SetParam, GetParam,
  ExtractJpgInfo, StartDecodeOneFrame, ResetDev, RegisterIsrCallback, UnRegisterIsrCallback}`. Parameter ids used:
  SetParam 0 / 1 (baseline / progressive maximum decode resolution), 6 (bitstream descriptor), 8 (output kind 0 / 0xd /
  0xe), 9 (output ring line count), 0xb (memory ops); GetParam 3 (original SOF size) — their mhal side is `mhal-jpd.md` §3.4, §3.5.
- mi_sys: `mi_sys_RegisterDev` / `UnRegisterDev` / `RegistCommand`, `Enable/DisableChannel`, `Enable/DisableOutputPort`,
  `mi_sys_RingPool_{Create, Destroy, SetRecordMinAllocSize, EnsureTailSpaceTimeout, AppendTailContentTailRecord,
  GetHeadRecordTimeout, ReleaseHeadRecord, GetRingPoolTotalSpace}`, `mi_sys_Ringpool_GetRecordTotalNum`,
  `mi_sys_NotifyWakeupISREvent`, `mi_sys_GetChnBufInfo`, `mi_sys_PrivateDevChnHeapAlloc` / `Free`, `mi_sys_Vmap` /
  `UnVmap` / `VFlushInvCache`, `mi_sys_MMA_Alloc` / `Free` + `mi_sys_Miu2Cpu_BusAddr` (ioctls 0xc / 0xd and debug),
  `mi_sys_GetCurPts`.
- CamOs (Tsem, atomics, memory, time), kernel wait queues, `filp_open` / `vfs_write` (dump commands). Exports: none.

### 1.12 libmi_jpd.so (14 exports) [R]
The only MI library with **argument checks in user space**: dev ≠ 0 → `0xA0202001`, chn ≥ 16 → `0xA0202002`, NULL →
`0xA0202006`. It keeps a per-channel state of 0x38 B (16 channels) with a CamOs mutex, and never closes its fd.

- **`MI_JPD_GetStreamBuf(dev, chn, u32ReqLen, pstRet, s32MilliSec)`**: on the channel's first use → ioctl 0x0f `{0, chn,
  u32 bufId (out)}` (the ring-pool handle), ioctl 0x0e `{bufId, +8 u64 phys (out), +0x10 size (out)}` and one `MI_SYS_Mmap` of that range;
  then ioctl 0x09 (0x40) `{0, chn, u32ReqLen, +0x10 MI_JPD_StreamBuf_t (out), +0x38 s32MilliSec}`; the two returned
  segments get `VA = mapVA + (phys − mapPhy)`. Kernel `BUF_FULL` (`0xA020200F`) → sleep 30 ms and return it.
- `MI_JPD_PutStreamBuf` / `MI_JPD_DropStreamBuf` send `{dev, chn, StreamBuf (0x28)}` (ioctls 0xa / 0xb). `MI_JPD_CreateChn`
  maps the kernel's EXIST to 0; `MI_JPD_DestroyChn` unmaps the ring.
- The other seven:

| function | nr (size) | payload / copy-back |
|---|---|---|
| `MI_JPD_CreateDev(dev, MI_JPD_InitParam_t *p)` / `DestroyDev(dev)` | 0 / 1 (8 / 4) | {dev, 1 word of p} / {dev}; the fd is never closed |
| `MI_JPD_GetChnAttr(dev, chn, *a)` | 4 (0x20) | {dev, chn}; 0x18 back on 0 |
| `MI_JPD_GetChnStatus(dev, chn, *s)` | 7 (0x24) | {dev, chn}; 0x1c back on 0 |
| `MI_JPD_ResetChn(dev, chn)` | 8 (8) | |
| `MI_JPD_QueryStreamInfo(dev, chn, MI_JPD_DirectInputBuf_t *in, MI_JPD_StreamInfo_t *out)` | 0xc (0x38) | {dev, chn, in (0x18)}; `*out` = payload +0x20 (0x14) on 0 |
| `MI_JPD_DirectBufDecode(dev, chn, *in (0x18), MI_JPD_DirectOutputBuf_t *out (0x28))` | 0xd (0x48) | {dev, chn, in, out} — the output buffers are given by address, nothing is copied back |

### 1.13 Use on the camera [R app code (`prog_uvc.md` §7.6); I where marked]
`St_JpdModuleInit` (0x161d4): CreateChn(0, 0, attr `{0xA00000 (10 MiB ring), 0, 0x2000, 0x2000}`) → StartChn → bind
**JPD(0, 0, 0) → SCL(dev 1, chn 0, port 0) REALTIME (type 4), 30:30**, SCL1 → DISP FRAME_BASE. Per photo (`photo_view_decode` (0x15f64)):
read the whole file, GetStreamBuf(len, timeout −1), memcpy into the mapped ring, PutStreamBuf (content length = file
size). `St_JpdModuleDeinit` (0x16124): StopChn + DestroyChn.
- One picture per ring record; a JPEG file larger than the 10 MiB ring cannot be shown [I: EnsureTailSpace cannot
  satisfy the min-alloc size]. The ring limits the file size, not the picture size: pictures up to 8192 px per side
  decode at full size, larger ones (SOF limit 65528 px) are scaled down by 2 / 4 / 8.
- A 4K photo (3840×2160 JPEG) decodes at full size into a YUYV frame (≈ 16 MiB) that SCL dev 1 scales [I from the sizes].
- The used ioctls are 2, 3, 5, 6, 9, 0xa, 0xb, 0xe, 0xf; 0, 1, 4, 7, 8, 0xc, 0xd and the proc commands are unused.

## 2. mi_vdec.ko

### 2.1 Load, parameters, open, close [R]
<!-- ev:mi_jpd-mi_vdec.2.1 -->
- Module parameters: **`thread_priority`** (int, default 99, `.data+0x140`, description "vdec work thread priority") and
  **`disable_low_latency_flag`** (ushort, default 0, `.bss+0x49b40`), besides the three debug parameters. **No code reads
  either of the two**: no MOVW / MOVT pair or literal relocation in any code section resolves to them
 ; they exist only in `/sys/module/mi_vdec/parameters/`. demo.sh
  passes none.
- `init_module` also runs `MI_VDEC_IMPL_Insmod` (two semaphores, module refcount 0).
- `process_init` = `MI_VDEC_IMPL_Init`: **refcounted**; the first open zeroes the module state (0x37a00 B) and initialises
  the 32 channel slots. `process_exit` = `MI_VDEC_IMPL_DeInit`: the last close of `/dev/mi_vdec` (refcount at D +0) runs
  DeInitDev (§2.5). libmi_vdec closes its fd when its create count returns to 0 (`mi-user-abi.md` §2.2).

### 2.2 Device record and debug state [R offsets; names from log strings, I where marked]
The module state `_stModRes` (`.bss`) holds the channel records at `_stModRes + 8 + chn·0x1bd0` and the device record
**D** at `.bss+0x37800`; mi_sys and the channels reach it as **D' = D + 0x208** (= `_stModRes + 0x37a08 + dev·0x228`,
the pointer kept in every channel at P+0x1c and passed to the pass callbacks as `usr`). Offsets below are from D.

| D off | field |
|---|---|
| +0x000 | open refcount |
| +0x208 | u8 inited |
| +0x20c | mi_sys device handle |
| +0x338 | mhal device handle |
| +0x33c | **ops table pointer** (= D'+0x134) → the 28-entry `MHAL_VDEC_*` table in `.data` (below) |
| +0x344 | bitmask: sequence-init done (interrupt event 9) |
| +0x348 | bitmask: VLC buffer request (event 10) |
| +0x34c | bitmask: frame done (event 0) |
| +0x350, +0x354, +0x358, +0x35c, +0x360, +0x370, +0x374 | interrupt counters shown as `IsrCnt InitSeq VlcInsuff FrmDone UnderRun HwTimeout Other` (D'+0x148 … +0x16c); +0x364…+0x36c are not printed |
| +0x392 | u8 pre-reset issued |
| +0x393 | u8 "wait pre-reset" |
| +0x394 | u8 reset requested by mi_vdec |
| +0x395 | u8 reset pending (VPU still busy) |
| +0x396 | u8 reset running |
| +0x397 | u8 late finish |
| +0x398 | u8 "need reset" |
| +0x39c | need-reset channel bitmap |
| +0x3a0 | time of the last frame done |
| +0x3a8 | started-channel count (`.bss+0x37ba8`) |
| +0x3ac | atomic tasks queued in the VPU (max 3, "DRV_COMMAND_QUEUE_DEPTH") |
| +0x3b0 | output layout mode (ioctl 0x16) |
| +0x3c8 | check semaphore (`CheckOutputTaskStatus`) |
| +0x3e8 | reset semaphore |
| +0x408 / +0x410 / +0x420 / +0x424 | scaling job: result / start time / the task being scaled / u8 busy (the stretch callback reaches them as `.bss+0x37c00` +8 / +0x18 / +0x24) — one job for all channels (§2.12) |
| +0x428 / +0x42c | HW timer slot / running flag |

The HW-reset wait queue (StartChn / StopChn wait on it) is part of the record; its offset is not given.

**Ops table** (`.data`, 28 entries, in this order; mi_vdec calls mhal only through it, entry *i* at +4·*i*): CreateDevice,
DestroyDevice, SetDevConfig, GetDevConfig, CreateInstance, DestroyInstance, StreamOn, StreamOff, SeqInit,
CompletSeqInit, SeqDeinit, SeqChange, UpdateSeqChange, SetBSRdPtr (+0x34), GetBSBuffer (+0x38), UpdateBSBuffer (+0x3c),
FlushBSBuffer, ClearDispFlag, SetParam, GetParam, DecodeOneFrame, DecodeFrameDone, DecodeCancel, DecodeAddInBuff,
AddVlcBuffer, QueryBufSize, GetEngStatus, CheckInstanceStatus. In mhal, `QueryBufSize`, `GetEngStatus`,
`CheckInstanceStatus`, `DecodeAddInBuff` and `DecodeCancel` are stubs returning 8 (`mhal-vcodec.md` §8.1).

**Debug state** `_stVdecDbgInfo` (`.bss+0x6d0`, 0x11840 B, zeroed at InitDev):
- bytes [0] / [1] / [2] = the global switches of `flowdbg` / `dropoutbuf` / `setperf` (§2.16); `_4_4_` / `_8_4_` = print
  mode / level (`setprint`);
- +0x0c / +0x10 / +0x14 / +0x18 = `CP1…CP4`, the source line number last stored by poll / enqueue / check / dequeue
  (Ghidra names the words by decimal offset: `_stVdecDbgInfo._16_4_` = +0x10, `_20_4_` = +0x14, `_44_4_` = +0x2c);
  +0x1c / +0x24 / +0x2c further line marks of poll / enqueue / check (not printed); +0x20 / +0x28 / +0x30 / +0x38 =
  `RUNTIME1…4`, call counters of the poll / enqueue / check / dequeue callbacks (each does += 1 at entry);
- per channel from `[0x241 + 0x90·chn]`: +0x241 `dumpbs` enable, +0x242 `dumpfb` enable, +0x244 `dumpfb` frame count,
  +0x248 `bDetile`, +0x249 `bDumpAll`, +0x24a dump path (0x80 bytes, shared by `dumpbs` and `dumpfb`), +0x2ca
  `force_reset` on, +0x2cc `force_reset` interval;
- per channel **S** = `_stVdecDbgInfo + 0x820·chn` from +0x1440 (= `.bss+0x1b10 + 0x820·chn`): the statistics of the
  read view (§2.18) and the `flowstat` switch S+0x1508.

Other `.bss` arrays (Ghidra addresses): per-channel `dumpfb` frame counters `0x337c0 + 4·chn` (frame path) and
`0x335c0 + 4·chn` (meta path), both starting at 0; per-channel decoded-frame counter at `0x339c0` (`force_reset`).

### 2.3 Channel record and create attr [R offsets; names from log strings and the proc view, I where marked]
Channel record **P**: 0x1bd0 B, ≤ 32 channels (the `.bss` `.LANCHOR1` max-channel word).

| P off | field |
|---|---|
| +0x00 / +0x01 | u8 created / u8 started (the read view tests the u16 at +0 = 0x101 for "created and started") |
| +0x02 | u8 output port 0 enabled ("port flag") |
| +0x03 | u8 only ever written 0 (StartChn, StopChn) — the branches that test it ("skip the stream-off", "started or +3") reduce to the started flag |
| +0x04 | chn id |
| +0x0c + 8p / +0x0e + 8p / +0x10 + 8p | per output port p (0, 1): u8 bound / u16 user-side depth copied from `mi_sys_GetChnBufInfo` (info +0x7c / +0x7e) [I: so a port read only with `MI_SYS_ChnOutputPortGetBuf` also counts] / bind count; +0x14 (port 1 bound) is the "port 1 enabled" flag ResetChn tests |
| +0x1c | D' |
| +0x20 | CamOs semaphore |
| +0x40…+0x63 | the create attr (0x24 B, below) |
| +0x68…+0x8f | statistics block of GetChnStat: +0x68 u64 bytes sent, +0x70 bytes left, +0x74 records left, +0x7c SendStream count, +0x88 error code (9 = idle), +0x8c dropped tasks |
| +0x9c / +0x9d / +0x9e | u8 error-recovery wait flags (§2.11.5) |
| +0xa0 | never written (0) |
| +0xa4 | display mode (ioctl 0x12) |
| +0xa8 | mhal instance |
| +0xb0 / +0xb4 | ES ring physical address, u64 (lo / hi) |
| +0xb8 / +0xbc | ES ring VA / size |
| +0xd5 | u8 "watchdog error → bitstream flush pending" (§2.7) |
| +0xd6 | u8 sequence initialised |
| +0xd8 | **state**: 1 created, 2 stream on, 3 sequence init queued, 5 decoding, 6 stopped |
| +0xe9 | u8 "this decode serves a refresh / step" |
| +0xec | u8 flag cleared after the index checks (§2.11.5) and by FlushChn |
| +0x17c…+0x188 | decoder crop window |
| +0x1bc8 | frame-buffer generation (meta mode) |
| +0x1bcc | u8 scaling retry flag (§2.12) |
| +0x1d8 (0x68 B) | the sequence info from `CompletSeqInit` |
| +0x1e0 / +0x1e4 | coded picture size |
| +0x1e8…+0x1f4 | display window {x, y, w, h} of the sequence [I from the crop code] |
| +0x22c | frame-buffer count (display frame buffers registered by mhal, `RefRecFrmCnt`) |
| +0x230 | `LinearFrmCnt` |
| +0x240 | u8 "unsupported resolution" (meta mode) |
| +0x244 / +0x248 / +0x24c | picture info words of the sequence-init-done event (sequence size) |
| +0x250 | u8 sequence change pending |
| +0x254 | sequence-change sub-state in meta mode ("SeqChange must wait for displayed frames", §2.11.6) |
| +0x258 / +0x25c | the meta record of the black bridging frame |
| +0x260 / +0x264 | the last meta record released by a consumer |
| +0x2b8…+0x2c8 | dest crop as set {u8 enable, x, y, w, h} (ioctl 0x18) |
| +0x2b9 | u8 latched crop enable |
| +0x2ba | u8 latched crop lies outside the picture (ignored) |
| +0x2cc…+0x2d8 | latched crop x, y, w, h |
| +0x2dc / +0x2e0 | ES read / write pointer seen by the VPU |
| +0x2e4 / +0x2e8 / +0x2ec | resume state {flag, record PA, read pointer inside it} (§2.7) |
| +0x2f0 | u8 tiled output |
| +0x2f4 | layout mode applied |
| +0x2f8 | tile format (proc view: 4 "32x32", 5 "128x32", 6 "128x64") |
| +0x2fc | ES ring-pool handle |
| +0x300 | SendStream calls without a ring |
| +0x304 | checks of a task on a channel that is not started |
| +0x30c | error-code table, 32 × {count, code, jiffies} (§2.11.5) |
| +0x48e / +0x490 / +0x492 | output-port attr {set, w, h} (ioctl 0x14) |
| +0x496 / +0x498 / +0x49a / +0x49c | latched port-0 scaling: u8 scale / target w / target h / u8 "no scaling" (§2.8); an earlier reading took +0x496…+0x4a2 as the scaling state, which overlaps +0x4a0 below — the known fields are these four |
| +0x4a0 | atomic output tasks in flight (max 1) |
| +0x4a4 | atomic queued sequence inits |
| +0x4a8 | play mode: 0 normal, 1 paused, 2 refresh, 3 step |
| +0x4ac | u8 paused |
| +0x4b0 / +0x4b4 | atomic pending refresh / step requests |
| +0x4b8 (u64) | PTS of the ES record that held the start of the last decoded frame |
| +0x4c0 | atomic display frames held by consumers (meta mode) |
| +0x4c8 | frame table, 32 × 0xb8 (meta mode, §2.13) |

**Create attr** `MI_VDEC_ChnAttr_t` (0x24 B, at P+0x40) [R offsets and checks; I names except where the proc view
prints them]:

| attr off | P off | field | checks / values |
|---|---|---|---|
| +0x00 | +0x40 | `eCodecType` | 0 H.264, 1 H.265 (2 JPEG appears in dump and release code, rejected by CreateChn) |
| +0x04 | +0x44 | `u32BufSize` | ES ring size = max(this, 128 KiB) |
| +0x08 | +0x48 | `u32Priority` | stored |
| +0x0c / +0x10 | +0x4c / +0x50 | `u32PicWidth` / `u32PicHeight` | 176..4096 / 128..4096, both even; only range-checked and shown in proc |
| +0x14 | +0x54 | `eVideoMode` | proc: 0 "STREAM", 1 "FRAME" |
| +0x18 | +0x58 | `eDpbBufMode` | ≤ 2; proc: 0 "NORMAL", 1 "INPLACE1", 2 "INPLACE2" |
| +0x1c | +0x5c | `u32RefFrameNum` | 1..16 when the DPB mode is 0; sent to mhal as the StreamOn maximum reference frames |
| +0x20 | +0x60 | u8 `bUseCusPolicy` (proc `bCusPolicy`) | |
| +0x21 | +0x61 | u8 `u8ErrMBPercentThreshold` (proc `ErrMBDropThrd`) | ≤ 100 when the policy byte is set; 30 by default (policy byte 0) |
| +0x22 | +0x62 | u8 **`bDisableLowLatency`** (proc label) | selects the decoder-owned display-frame mode (§2.13) |

### 2.4 mi_sys registration and the output ports [R]
`vdec_sys_register` (0x158cc), inside InitDev. Info: module **8**, dev 0, **0 input ports, 2 output ports**, 32 channels, 1 pass, info
+0x20 = 1 (per-channel wake-up: the module wakes with `mi_sys_NotifyWakeupISREventByChn`), +0x2c `OnConfigSramBuf`,
+0x30 `AllowPreemptionSram`. Pass table:

| pass slot | callback | what it does |
|---|---|---|
| 0x14 / 0x18 | `EnqueueAsyncOutputTask` / `BarrierAsyncOutputTask` | §2.9 |
| 0x24 / 0x28 | `CheckOutputTaskStatus` / `DequeueOutputTask` | §2.11, §2.14 |
| 0x2c | `OnPollingAsyncOutputTaskConfig` | the channel state machine, §2.8 |
| 0x30 | `NeedIndividualWorkThread` | returns 0 |
| 0x40 / 0x44 | input capability (stub, returns 0, writes nothing → a bind into VDEC is refused) / output capability = **1 (FRAME_BASE)** | |
| 0x48 / 0x50 | input bind / unbind (no-op) | |
| 0x58 / 0x60 | `OnBindOutputPort` / `OnUnBindOutputPort` | per port: bound flag and bind count (P+0xc+8p, +0x10+8p); the first bind of **port 1** also calls `mi_sys_EnableOutputPort(…, 1)`, its last unbind disables port 1 while the channel is started |
| 0x68 | `ReleaseFrameCallBack` | §2.14 |
| 0x7c | `OnSupportOutputTaskReorder` | returns 1 (mi_sys calls it in `DequeueInputOutputTasks`) |
| 0x8c | concurrent-task limit = 3 | |
| 0x92 | u16 = 10000 | output-task timeout in ms |

**Ports**: **port 0** is the **scaled / cropped** output (the SetOutputPortAttr target and the dest crop apply to it);
**port 1** is the **full-size** decoded picture. A port gets an output buffer in the poll only when it is bound or has a
user-side depth (P+0xe+8p), so port 1 runs only when a consumer binds it (or reads it with a depth) — the app never does.
With port 1 in use, the VPU decodes into port 1's buffer and SCL device 3 makes port 0 from it (§2.12).

### 2.5 Device bring-up and teardown — `MI_VDEC_IMPL_InitDev` / `DeInitDev` (ioctls 0 / 1) [R]
- **InitDev** — ioctl 0 (used by the app as `MI_VDEC_CreateDev`), implicitly from CreateChn and from
  SetOutputPortLayoutMode. The init parameter pointer must be non-NULL (`…06`) and is **not read**; already inited →
  **`0xA0082016` INITED**. Sequence: zero the device state, `_stVdecDbgInfo` and the reset wait queue → mi_sys registration
  (§2.4) + the proc commands (§2.16) → the ops table → `MHAL_VDEC_CreateDevice(0, &hdl)` → `MHAL_VDEC_GetDevConfig(hdl, 0x8002000d,
  …)` → `MHAL_VDEC_SetDevConfig(0x80020003, memOps {alloc → mi_sys_PrivateDevChnHeapAlloc, free → mi_sys_PrivateDevChnHeapFree, map
  → mi_sys_Vmap, unmap → mi_sys_UnVmap, flush → mi_sys_VFlushInvCache, heap name → mi_sys_GetModuleMMaHeapName, MIU
  protect → mi_sys_setMiuProtectMMa})` → `SetDevConfig(0x80020008, _MI_VDEC_Interrupt_Handler)` →
  `SetDevConfig(0x8002000c, _MI_VDEC_Reset_CB)` → **`mi_scl_Init()` + `mi_scl_CreateDevice(3, {0x20})`** → mi_sys HW timer
  `RegisterHwTimer(h, _MI_VDEC_HwTimerHandler, …, 0x21, …)` + `EnableTimer` (period 0x21 = **33 ms**, the mi_sys timer
  counts ms) → replay `/customer/magic_book.cfg` (`vdec_magic_book_replay` (0x1e390), §2.16). Any failure → full teardown (`vdec_teardown` (0x2293c)).
  mi_vdec never sends `SetDevConfig 0x80020005` (the VPU's maximum resolution), so the VPU keeps its default limit.
- **DeInitDev** — ioctl 1 (no payload; unused by the app) and the last close: not inited → **`0xA0082015` NOT_INIT**.
  HW timer: `mi_sys_DisableTimer` + `UnRegisterHwTimer` (D +0x42c, +0x428); `mi_scl_DestroyDevice(3)`; `mi_scl_DeInit()`
  (failure → `0xA008201F`, nothing else torn down); then `vdec_teardown`: StopChn for every started channel, DestroyChn for
  every created one, `MHAL_VDEC_DestroyDevice` (failure → `0xA0082007`), `mi_sys_UnRegisterDev` (failure →
  `0xA0082007`), layout mode (D +0x3b0) = 0; inited = 0; return 0.
- InitDev creates SCL device 3 whether or not port 1 is ever used, so the `mi_scl_*` exports are needed whenever
  mi_vdec is initialised [I].

### 2.6 ioctls (`/dev/mi_vdec`, 27 entries) [R]
The device id is never sent (§2.20). "Checks" = chn < 32 (else `0xA0082002`) and channel created (else `…05`). Payload
sizes are the library's; **bold** = used by the app.

| nr | name | payload | kernel action |
|---|---|---|---|
| **0** | InitDev (`MI_VDEC_CreateDev`) | `{InitParam 4}` (4) | §2.5 |
| 1 | DeInitDev | u16 0 (2) | §2.5 |
| **2** | CreateChn | `{chn, attr 0x24}` (0x28) | implicit InitDev; attr checks (§2.3): codec 0 or 1, else **`0xA0082003`** ("UnSupport Codec Type"; H.265 has the same limits as H.264, only the log text says "h265"); width 176..4096, height 128..4096, both even; DPB mode ≤ 2, and if 0 the reference-frame count in 1..16; policy byte set → threshold ≤ 100 (else 30). Channel exists → `0xA0082004`. Store the attr; ES size = max(attr +4, 128 KiB); **create the ES ring pool `"vdec-esBuf<chn>"`** (min record 0x1000, alignment 1, 1 ring — `mi_sys-1-engine.md` §10.1) and read back phys / VA / size; `MHAL_VDEC_CreateInstance(dev, codec, &inst)` (codec 1 opens a HEVC instance); store the DPB mode; created, state 1 |
| **3** | DestroyChn | `{chn}` (4) | started → `0xA0082014`; `MHAL_VDEC_DestroyInstance`; cache flush of the ring; `mi_sys_RingPool_Destroy`; clear the slot |
| 4 | GetChnAttr | `{chn}`, 0x24 back (0x28) | checks; out NULL → `…06`; copy the attr (P+0x40) |
| **5** | StartChn | `{chn}` (4) | wait ≤ 1000 jiffies (10 s) for a running VPU reset to finish; `vdec_stream_on` (0x17490): `MHAL_VDEC_StreamOn(inst, {0x40, 2})` (below), `MHAL_VDEC_SetParam(inst, 0x80020001, …)` and `(0x80020004, {1})`; state 2; `mi_sys_EnableOutputPort(h, chn, 0)` + `mi_sys_EnableChannel`; started = 1; counters reset; S+0x1454 written |
| 6 | StopChn | `{chn}` (4) | not started → **`0xA0082013`**. `mi_sys_DisableChannel` + `DisableOutputPort(port 0)` (a failure is a BUG), port flag 0; wait until no output task is in flight (P+0x4a0 = 0; 1-ms sleeps; after 20000 sleeps it re-checks under the semaphores and loops again — not a hard limit); if a VPU reset is running (D +0x394 / +0x395 / +0x396), release the semaphores and wait ≤ 1000 jiffies (10 s) on the HW-reset wait queue; `vdec_complete_queued_seqinit` (0x12eec) = complete a queued SeqInit (`CompletSeqInit`), **`MHAL_VDEC_StreamOff`**, `MHAL_VDEC_SeqDeinit` if a sequence was initialised (meta mode with display frames still held → **`0xA0082012` BUSY**, the stop is abandoned), clear the channel's bit in D +0x344, zero the sequence info; drop every ES record (`vdec_drop_es` (0x11300)); started = 0, **state 6**; reset the statistics, the error table, the play mode, the tiled flag (= 1), P+0x2f4 = 0, P+0x88 = 9 (status "idle"); started-channel count −1; meta mode: held-frame count 0, generation 0 |
| **7** | GetChnStat | `{chn, out 0x20}` (0x24) | `{codec, ES bytes left, ES frames left, pictures waiting in the output queue (mi_sys_GetChnBufInfo), frames received, frames decoded, u8 started, error code (9 when idle)}` — the field order of `MI_VDEC_ChnStat_t` [I] |
| 8 | FlushChn | `{chn}` (4) | [R camera 2026-11-11: on a paused channel with `RefreshChn`s going it never returns — the caller in D state in `msleep`, the decoder dead until a reboot, 2 of 2] not started → **`0xA008201A` DEV_NOT_STARTED**; sequence change pending (P+0x250) → `…12`. Port flag 0; wait (1-ms steps, ≤ 20000) until no task is in flight, no SeqInit is queued and no frame is left in the output queue (`mi_sys_GetChnBufInfo`); meta mode: `DisableOutputPort(0)`, wait ≤ 20 ms for held frames to return, `EnableOutputPort(0)`; `ClearDispFlag` for every frame buffer (P+0x22c of them); **`MHAL_VDEC_FlushBSBuffer({0x28, 1, ES phys, size})`** (failure → port flag 1, `…1F`); drop every ES record; **`SetBSRdPtr(ES phys)`**; read / write pointers = ES start; last PTS, P+0xec and the 32-entry frame table cleared; port flag 1 → 0. The channel stays started; decoding resumes with the next SendStream |
| 9 | ResetChn | `{chn}` (4) | not started → **0** (nothing done). `DisableChannel`, `DisableOutputPort(0)` and, when port 1 is enabled, `(1)`; wait until no task is in flight; stream off as in StopChn (`vdec_complete_queued_seqinit`); **StreamOn again** (`vdec_stream_on`, state 2); success → started 1, `EnableOutputPort(0)` (+ port 1 if enabled), `EnableChannel`, port flag 1; failure → started 0 and the error is returned. ES records are **kept** (decoding restarts from the sequence header found in what is left) |
| 0xa / 0xb | Set / GetChnParam | `{chn, p (0xc)}` (0x10) | **`0xA0082008` NOT_SUPPORT** |
| **0xc** | SendStream | `{chn, 0, VideoStream (MI_VDEC_VideoStream_t) 0x18 {user VA, u32Len, u64 PTS, u8 bEndOfFrame, u8 bEndOfStream}, +0x20 s32MilliSec}` (0x28) | not started → `0xA0082013`; then §2.7 |
| 0xd | PauseChn | `{chn}` (4) | checks; not started → `…1A`. Play mode = 1 unless a refresh / step is set up (mode 2 / 3 kept); paused = 1 → 0. The poll then starts no new decode while paused and no request is pending |
| 0xe | RefreshChn | `{chn}` (4) | not started → `…1A`; **not paused → `0xA0082018` NOT_DISABLE**; no ES record → `0xA008200E` BUF_EMPTY; no unread record → `0xA008201C`; a refresh or step pending → `…12`. Mode 2, refresh count +1, wake the channel → one decode while paused (it re-decodes the head, oldest unreleased, ES record, so the current picture is produced again) |
| 0xf | ResumeChn | `{chn}` (4) | not started → `…1A`; refresh or step pending → `…12`. P+0xe9 = 0, mode 0, paused 0; wakes the channel if the mode was 2 |
| 0x10 | StepChn | `{chn}` (4) | not started → `…1A`; ring empty / no unread record / request pending as RefreshChn. Mode 2 (a refresh is set up): mode 3, paused 1, step count +1, wake, then **wait ≤ 50 ms** (1-ms sleeps) for the step to finish, then mode 2 and refresh count +1; other modes: mode 3, step count +1. Then paused 1, wake → 0. One frame is decoded and output; CheckOutputTaskStatus decrements the step count when that frame is done (mode 3) |
| 0x11 | GetUserData | (0x80c) | **`0xA0082008`** — the library's user-data buffer (§2.20) is therefore always returned empty |
| 0x12 | SetDisplayMode | `{chn, mode}` (8) | checks; mode < 2 → P+0xa4, else `…03`. **Stored only** — nothing reads P+0xa4 |
| 0x13 | GetDisplayMode | `{chn, mode}` (8) | checks; returns P+0xa4 |
| **0x14** | SetOutputPortAttr | `{chn, u16 w, u16 h}` (8) | w, h in 8..4096, even; stored at P+0x48e…; latched at the next sequence init / output config (§2.8) |
| **0x15** | GetOutputPortAttr | `{chn, out}` (8) | returns the stored size |
| 0x16 | SetOutputPortLayoutMode | `{mode}` (4) | mode < 3, else `…03`; device not inited → implicit InitDev (failure `…1F`); **any channel started → `…12` BUSY** (started-channel count `.bss+0x37ba8`); store D +0x3b0. Applied per channel by `vdec_apply_out_mode` (0x10b9c) at sequence init / output config: **0 = follow the consumer** — the channel switches to tiled output (P+0x2f0 = 1) while its output buffers carry a type-2 sideband request ("deliver tiled frames") and ACKs it, the first such frame after linear logs "Linear to tile, realloc frame buffer"; back to linear when they do not (`mi_sys-1-engine.md` §5.3); **1 = linear** (tiled 0); **2 = always tiled** (tiled 1, every buffer ACKed; buffers 128×64-aligned) |
| 0x17 | GetOutputPortLayoutMode | `{mode}` (4) | not inited → `…15`; returns D +0x3b0 |
| 0x18 | SetDestCrop | `{chn, u8 bEnable, u16 x, y, w, h}` (0x10) | stored at P+0x2b8…+0x2c8. Disabled → stored as is. Enabled: x ≤ 4080, y ≤ 4094, w 16..4096, h 2..4096, **x and w multiples of 16, y and h even** (else `…03`, log "crop x/w 16 aligned, y/h 2 aligned"). Latched into P+0x2b9 / +0x2cc… at the next sequence init / output config and applied as in §2.8 and §2.11.4 |
| 0x19 | GetDestCrop | (0x10) | returns the stored (not the latched) crop |
| 0x1a | SetChnErrHandlePolicy | `{chn, u8 bUseCusPolicy, u8 threshold}` (8) | → P+0x60 / +0x61 — **no range check** here (CreateChn checks ≤ 100); used by the error-MB check of §2.11.5 |

**StreamOn `{0x40, 2}`** as mi_vdec fills it (`vdec_stream_on`; layout and mhal meaning in `mhal-vcodec.md` §8.2): [4] / [5] = ES
ring physical address lo / hi (P+0xb0 / +0xb4), [6] = ES size (P+0xbc), [10] (ES VA) = **0** (not filled), byte [0xc] =
P+0x62 `bDisableLowLatency` (the output mode: 0 = caller-buffer, ≠ 0 = mhal-buffer), [0xd] = P+0x5c (maximum reference
frames), [0xe] = 3 (extra display buffers); every other word 0 [R].

### 2.7 The ES path: SendStream, the VPU window, release [R unless marked]
<!-- ev:mi_jpd-mi_vdec.2.7 -->
mi_vdec does not parse the stream: no NAL scanning exists in the module (except the `setperf` input count, §2.18); the ES
ring is handed to the VPU as a byte stream, and the header parse is the VPU's, in SeqInit [I]. Ring record = mi_sys ring
content record (`mi_sys-1-engine.md` §10.1: +0x08 u64 pts, +0x20 PA, +0x2c length). Wrappers: `vdec_set_bs_rdptr(P, addr)` (0x16fc4) =
`MHAL_VDEC_SetBSRdPtr(inst, addr, 1)` (ops +0x34); `_MI_VDEC_Wrapper_UpdateBSBuffer` (0x00010634) →
`MHAL_VDEC_UpdateBSBuffer(inst, len)` (ops +0x3c; r1 passed through, objdump 0x60c–0x628); `vdec_get_bs_buffer(P, rec)` (0x15cf8) =
GetBSBuffer (ops +0x38; fills the ES phys into rec +8 / +0xc first; ES phys 0 → `0xA0082011`); `vdec_flush_bs_buffer(P, rec)` (0x15bc8) =
FlushBSBuffer. In all four: no instance (P+0xa8 = 0) → `0xA0082005`, NULL → `…06`, mhal failure → `…1F`.

The mhal side of the four (1831–2060 [R]; Wave5 meaning from
 [S]; the rest of mhal's VDEC block is `mhal-vcodec.md` §8's):
- `SetBSRdPtr(inst, addr, 1)` → `VPU_DecSetRdPtr(h, addr, updateWrPtr = 1)`: read **and write pointer** := addr ("used to
  operate the bitstream buffer in PicEnd mode") [S].
- `UpdateBSBuffer(inst, n)` → `VPU_DecUpdateBitstreamBuffer(h, n)`: write pointer += n (wrap handled by the API); 0 = end of
  stream, −1 = resume after end, −2 = "decode until the write pointer and force the end" [S]. mhal refuses (8) after a
  stream end was sent on an instance flagged so (`ctx+4 → +0x68`, `+0x20 → +0x2d0`) and, while its ES dump count
  (`ctx+0x2928`) is non-zero, first writes `<ctx+0x292c>/es_dev%02d_chn%02d.es` (mhal debug, not reached from mi_vdec's
  commands).
- `GetBSBuffer(inst, rec)`, rec = `{0x28, 1, u64 ES phys (must equal ctx+0x2900 / +0x2904, else "mismatch BSBufPhyAddr" →
  3), +0x10 —, +0x14 rdPtr, +0x18 wrPtr, +0x1c room}` (`VPU_DecGetBitstreamBuffer(h, &rd, &wr, &room)`; PhysicalAddress
  is 32-bit here, the three words are adjacent) [R; I: `room` = the Wave5 `size` out-parameter].
- `FlushBSBuffer(inst, rec)` (same 0x28 header and phys check): `UpdateBitstreamBuffer(−2)`, then up to 10001 rounds of
  `VPU_DecFrameBufferFlush` while it answers 0x1a (frame not complete): wait ≤ 10 ms for an interrupt (clear it, read the
  output info, release the frame it names), 1-ms sleep, `SetRdPtr(current rd, 1)`; done → 0, exhausted → "NO RESPONSE
  FROM VPU_DecFrameBufferFlush()", 9.

**SendStream** (`MI_VDEC_CopyEsbufferFromUser`, Ghidra 0x207c0), after the started check:
1. No ring (P+0x2fc = 0) → `0xA0082007`, P+0x300 += 1. Chunk length > ES size P+0xbc → `0xA0082003` ("input data larger
   than bitstream buffer").
2. `_MI_VDEC_UpdateRecordMinAllocSize(P)` (min record = Len); loop: release the channel semaphore,
   **`mi_sys_RingPool_EnsureTailSpaceTimeout(pool, ms = the caller's timeout, &rec, 0, resetWhenEmpty = 1)`** while the
   module mutex stays held, take the semaphore again.
   - Failure `0xA009200C` (no room) in **state 2** (stream on, no sequence yet): `vdec_try_recover_room(P)` (0x170f4) (below); if it returns 1
     and this is the first try, loop once more. Any other failure, or a second one → **`0xA008200F` BUF_FULL** ("failed to
     get buffer from ring pool, s32MilliSec, ret").
   - Success in state 2: `vdec_try_recover_room(P)` as well; returned 1 on the first try → reserve again (the ring was reset).
3. **`CamOsCopyFromUpperLayer` straight from the user VA** into the reserved VA (failure `…1F`),
   `mi_sys_VFlushInvCache(VA, len)`, content length = len, pts = the user's u64; `AppendTailContentTailRecord(pool, chn,
   isr 0, bClose 1, &rec)` (a failure other than `0xA009200C` → BUG). PTS {lo −2, hi −1} is the "end marker" PTS (logged;
   such a frame is not delivered, §2.11.4).
4. Counters: P+0x70 += len, P+0x7c += 1, P+0x68 (u64) += len, P+0x74 += 1, mirrored into S+0x1440, S+0x1448 / +0x144c
   (u64), S+0x1450; with `setperf` on, the input-picture count (§2.18); `dumpbs` on (`[0x241 + 0x90·chn]`) →
   `_MI_VDEC_DebugDumpBitStream` appends the chunk (§2.16); `mi_sys_NotifyWakeupISREventByChn` when no task is queued.

Each SendStream chunk is therefore **one closed ring record**. Records are never marked read (mi_vdec imports no
`GetHeadRecordTimeout` / `…InISR`; objdump relocation count), so for mi_vdec "unread" ≡ "published" ≡ total [R +
`mi_sys-1-engine.md` §10].

**Feeder for the sequence header** `_MI_VDEC_UpdateBitStrmBufForInitHead(P)` (0x1e7ec), called by the poll in state 2
before `MHAL_VDEC_SeqInit`: queued sequence inits (atomic P+0x4a4) ≥ 1 → `0xA0082012`; unread records 0 → `0xA008201C`;
head unreleased record, length 0 → `…1F` ("get ring pool data len is 0"); **`SetBSRdPtr(PA)` then `UpdateBSBuffer(length)`**
→ 0. The record is not released here.

**Feeder for a picture** `_MI_VDEC_UpdateBitStrmBufForDecoding(P)` (0x1e978–0x1ec0b, ~100 lines), called by the poll in state 5
when the channel is started, port 0 enabled and the ring exists, after the play-mode gate (paused with no refresh / step
request → no decode; refresh request P+0x4b0 ≥ 1 → P+0xe9 = 1, step request → 0, else 0):
1. Output tasks in flight (P+0x4a0) ≥ 1 → **`0xA0082012`** (one decode at a time per channel).
2. Total records 0 → **`0xA008200E`** BUF_EMPTY. Not a refresh (P+0xe9 = 0) and unread records 0 → `0xA008201C` —
   unreachable for mi_vdec (unread ≡ total) [I].
3. Head unreleased record (`GetHeadRecordUnrelease(pool, chn, &rec, 0, 0)`; failure → `…1F` "failed to get ring pool
   data"); length 0 → `…1F`.
4. Start address / length: normally the record's PA / length. **Resume**: if the resume flag P+0x2e4 is set, its record
   address P+0x2e8 equals this PA and its read pointer P+0x2ec lies strictly inside (PA < rp < PA + len), start = rp and
   length = PA + len − rp ("set the cur record, RdPtr, Len").
5. **`SetBSRdPtr(start)`** (failure → "set read ptr failed", `…1F`), **`UpdateBSBuffer(length)`** (failure → "update
   bitstream buffer failed", `…1F`). The VPU window is now exactly the (rest of the) head record.
6. Resume used (same condition as in 4) → clear P+0x2e4 / +0x2e8 / +0x2ec. → 0.

Poll reaction: any non-zero result → the poll returns `0xA008201C` (no task this round). On 0 the poll goes on to ask
mi_sys for an output buffer (§2.8); in meta mode with ≥ 4 held frames and no sequence change pending it logs "no frame
buffer, stop decoding and wait" and also returns `…1C` — the window just set is set again next round [R; I harmless]. A
refresh re-feeds the head record, which is still unreleased (`GetHeadRecordUnrelease` ignores the read flag).
GetBSBuffer and FlushBSBuffer are **not** used per frame: GetBSBuffer only in the proc view (§2.18) and the watchdog flush
below; FlushBSBuffer in FlushChn and the watchdog flush [R: callers of the wrappers].

**Release after a decode** `vdec_release_after_decode(P, r, &pts)` (0x11b10; called 4× from CheckOutputTaskStatus). Inputs: r+0x40 = the
VPU's ES read pointer after the decode, r+0xa0 = a second ES address from DecodeFrameDone [I: the frame's start position,
Wave5 `bytePosFrameStart`]. No records → "no es data left" (debug log), the output PTS = P+0x4b8. Otherwise, repeatedly on
the head unreleased record (lookup failure → BUG), with end = PA + len, rd = r+0x40:
- end < rd and PA ≠ r+0xa0 → release it (P+0x70 −= len, P+0x74 −= 1, S+0x1450 = P+0x70), continue;
- end = rd, **or PA = r+0xa0** → take its pts as the frame's PTS (out and P+0x4b8 / +0x4bc), release it, **stop** — records
  after it stay even if they lie below rd (released on a later call);
- end > rd → keep it and stop; if PA < rd (the VPU stopped inside it) set the resume state P+0x2e4 = 1, P+0x2e8 = PA,
  P+0x2ec = rd, used by the next feed (step 4 above).

Comparisons are plain unsigned (records never wrap the ring end, `mi_sys-1-engine.md` §10, and the window never spans two
records) [R; I why no wrap handling is needed]. Release failure → BUG.

**Watchdog flush in the poll (P+0xd5) — unreachable.** P+0xd5 = 1 is set only in CheckOutputTaskStatus for eStatus
**0x20004** ("syserr watchdog timeout"), a branch that cannot run (§2.11.5); the other writes clear it (post-reset, and
here). So this poll branch never runs [I from those two]: with P+0xd5 set and no sequence change pending (P+0x250 = 0) —
in meta mode `vdec_clear_disp_flag(P, i)` (0x10894) for each of the P+0x22c frame buffers [I: ClearDispFlag], then `FlushBSBuffer({0x28, 1,
ES phys, size P+0xbc})` (failure → skip the rest), `GetBSBuffer` → wr (+0x18); release head records while their end < wr
(counters as above); a record ending exactly at wr is released last; one ending past it is kept; then P+0xd5 = 0.

**`vdec_try_recover_room(P)` — drop when everything was read (SendStream, state 2) — never effective.** Total records 0 → BUG
("u32TotalNum == 0"); unread > total → BUG. **Unread = 0**: release every record (counters as above), total must now be 0
(else BUG), `EnsureTailSpaceTimeout(pool, 0, &rec, 0, 1)` (resets the empty ring to its start; failure → BUG),
`SetBSRdPtr(rec.PA)` → 1 on success; otherwise 0. Since mi_vdec never marks records read, unread = total ≥ 1 at this point
and the drop **never happens**; the function always returns 0 and SendStream's retry never fires [I from the import list
and the ring-pool rules]. Intended meaning [I]: in state 2 (no sequence header found yet) throw away input the sequence
search has already looked at, to make room.

### 2.8 The poll — channel state machine and output buffers (`OnPollingAsyncOutputTaskConfig`, pass slot 0x2c) [R calls and states; I flow details where marked]
Per channel (the device wakes per channel), counting `Issue` (S+0x14a8) and stamping the poll time (S+0x14f4):
- **Device in HW reset** → finish / report the reset and return "not now".
- **State 2** (stream on) with ES records and fewer than 3 tasks queued in the VPU: the sequence-header feeder (§2.7),
  **`MHAL_VDEC_SeqInit(inst, {0x68, 2, …})`** (mhal reads no input word and only returns eStatus at +0x5c; for 0x20000
  mi_vdec logs "not found sps, retry init seq"), state 3.
- **State 3**: `vdec_complete_seqinit` (0x13598) completes the sequence init (`MHAL_VDEC_CompletSeqInit` → picture size, display window, ES
  pointers after the header parse, the number of frame buffers mhal registered → P+0x22c, sequence info at P+0x1d8;
  counted as `Complete` S+0x14ac and `ChkInitSeq` S+0x14f0) → state 5. In meta mode the output config
  (`SetParam 0x8002000a`, `vdec_set_out_config` (0x121a4)) is sent here once per sequence, after checking the sequence size is 176..4096 ×
  128..4096 (else P+0x240 = 1, "unsupported resolution", nothing is decoded). `vdec_apply_out_mode` applies the layout mode
  (§2.6 ioctl 0x16) at this point and at each output config.
- **State 5** (decoding), channel started and port 0 enabled, no output task in flight: the play-mode gate and the picture
  feeder (§2.7); in meta mode no new decode while **4 or more** display frames are held (P+0x4c0 ≥ 4) unless a sequence
  change is pending. Then, per port p = 0 then 1 (`mi_sys_SetOutputPortBufExtConf(h, chn, 0, port …)` with port = 0, then 1), a port that is
  bound or has a user depth gets an output buffer:
  - **normal mode** (`bDisableLowLatency` = 0): frame type 1, **pixel format 0xB (NV12)**, PTS −1, size = the coded
    picture size (P+0x1e0 / +0x1e4), ext conf `{0x20010, 2}`; tiled layout → ext conf `{0x400080, 0x40}` (128×64
    alignment);
  - **port 0 only** latches, at this moment, the output-port attr (P+0x48e… → P+0x496…) and the dest crop (P+0x2b8… →
    P+0x2b9, P+0x2cc…) and computes its size:
    1. crop enabled and inside the picture → P+0x2ba = 0, size = crop w × h, decoder crop window (P+0x17c…+0x188) = the
       crop; crop outside the picture → P+0x2ba = 1 (crop ignored, window 0);
    2. scaling (P+0x496 set): target (P+0x498 / +0x49a) larger than the source (crop or picture) → log "not support
       scaling up", P+0x49c = 1 (no scaling, full size); smaller than **1/8** of the source in either axis → clamped to
       `(src/8 + 1) & ~1` with the log "scaling out of range"; else size = target, ext conf `{0x40010, 2}`; with a crop
       the window is scaled too (`x·W'/W & ~15`, `y·H'/H & ~1`, `w & ~15`);
  - **meta mode** (`bDisableLowLatency` ≠ 0): a **meta buffer** instead of a frame — buffer type 2, +0x1c = 0x80000000,
    +0x30 = 0x50 (the size of the meta record) — or, while a sequence change waits for displayed frames (P+0x254 = 1),
    a dummy 32×32 NV12 frame (§2.11.6).
- **Rate window** (Ghidra ~0x1f660): when ≥ 1000 ms have passed since S+0x1464 the poll copies the input-picture count of the window
  (u16 S+0x1458) to `ifps` (S+0x145a) and the frame count (u16 S+0x1468) to `fps` (S+0x146a) and zeroes both; the window
  closes only when the poll runs, so both are "per last window of ≥ 1 s" [I].

### 2.9 Enqueue (`EnqueueAsyncOutputTask`, pass slot 0x14) [R]
Stamps S+0x14f8 and counts `Start` (S+0x14b0). Checks the output buffer geometry against the sequence (linear: 16-byte
stride; tiled: 128-byte stride, Y = stride × ⌈H⌉64, C = stride × ⌈H/2⌉64). Per output buffer the layout mode is applied
to the sideband (mode 2 → ACK every buffer; mode 0 with a type-2 request → tiled and ACK; `mi_sys-1-engine.md` §5.3).
The VPU decode target is **port 1's buffer when the task has one, else port 0's** (`buf[port1] ?: buf[port0]`); with only port 0 the VPU's own scaler /
crop produce port 0 directly — the output config `SetParam(0x8002000a, {0x2c, 1})` built by `vdec_set_out_config` carries the
tiled byte, scale {enable, W, H} and crop {enable, x, y, w, h}, bounded by the sequence size P+0x244 / +0x248 with the
same 1/8 rule (`UpdateSeqChange` when needed). Then **`MHAL_VDEC_DecodeOneFrame`** (descriptor `{0xd0, 2}`,
`mhal-vcodec.md` §8.2), task word 2 = the enqueue jiffies, in-flight counters +1.

### 2.10 VPU events: interrupt, HW timer, reset callback [R]
- **`_MI_VDEC_Interrupt_Handler(dev, ev)`** (registered with `SetDevConfig(0x80020008)`; mhal owns the IRQ), ev = {event,
  …, [10] instance}: event 0 = frame done → the channel's bit in D +0x34c; event 9 = sequence-init done → bit in D +0x344
  and the picture info words copied to P+0x244 / +0x248 / +0x24c; event 10 = **VLC buffer request** → bit in D +0x348;
  events 1, 8 and others are only counted (D +0x360, +0x370, +0x374 — the `UnderRun`, `HwTimeout`, `Other` columns of the
  read view). Every handled event wakes the channel (`mi_sys_NotifyWakeupISREventByChn(h, 0, chn)`); counts `InitSeq`,
  `FrmDone`, `VlcInsuff` (S+0x14d0 / +0x14d4 / +0x14d8) and stamps S+0x14fc.
- **HW timer** `_MI_VDEC_HwTimerHandler` (33 ms, §2.5) wakes the device while tasks are queued — the decoder is also polled.
- **`_MI_VDEC_Reset_CB(dev, ev)`** (mhal's reset callback, `SetDevConfig 0x8002000c`), by ev [2]; counted as `RcvRstCB`
  (S+0x14e0):
  - **0 / 1** → "need reset" (D +0x398) = 0 / 1;
  - **2** = one instance (ev [3]) needs a reset → D +0x398 = 1, its channel's bit in D +0x39c, wake that channel;
  - **3** = may the reset start? → no task queued in the VPU (D +0x3ac = 0): answer ev [2] = 4 and D +0x396 = 1
    (running); else D +0x395 = 1 (pending);
  - **4** = hardware reset done → D +0x393 = 0; if running: stream off every started channel and drop its ES
    (`vdec_stream_off_drop` (0x15ac4)), zero the frame / queue counters;
  - **5** = reset finished → if running: take and release every channel semaphore (barrier, `vdec_chn_barrier` (0x17738)), D +0x396 = 0,
    wake the HW-reset wait queue (StartChn / StopChn wait on it); else D +0x397 = 1.

  Every event also wakes the device. A reset therefore aborts all decoding: the channels are stream-off'ed, their ES
  data dropped, and decoding restarts with the next SendStream through a new sequence init from state 2 [I].

### 2.11 `_MI_VDEC_CheckOutputTaskStatus` (pass slot 0x24) [R unless tagged]
`(cmdq, task, msSinceEnqueue, D')` (the largest function of the module). Locks D +0x3c8 (the
device check semaphore), then the channel semaphore. Returns **0 pending** (mi_sys asks again later; its own timeout is
the pass u16 0x92 = 10000 ms), **1 done** (buffers delivered), **3 failed** (buffers released; P+0x8c "dropped tasks"
+1). Task words used: [0] chn, [2] scratch (enqueue jiffies, or the magic **`'SCLT'` (0x53434c54)** once the decode is
finished and a scaling step is due), [0xd] port-0 buffer, [0xe] port-1 buffer. Helpers: `vdec_pre_reset` (0x17f8c) =
`SetDevConfig(0x8002000b)` **pre-reset**, `vdec_post_reset` (0x1797c) = `SetDevConfig(0x80020009)` **post-reset** (VPU software
reset, "MHAL_VDEC_SWReset"), `MHAL_VDEC_SeqChange` (0x000172c4), `vdec_release_after_decode` = release of the consumed ES records
(§2.7), `_MI_VDEC_EnqueueScalingTask` (0x00013cc4) (§2.12). Stamps S+0x1500 and keeps the counters `Get`, `GetOK`,
`Done`, `DropNoVCL`, `Run`, `SeqChange`, `ChkRstCB`, `ChkFrmDone`, `ChkVlcInsuff` (§2.18).

#### 2.11.1 Order of checks
1. Channel **not started** → P+0x304 +1, fail (3).
2. State < 5 (no sequence yet) → log "eDecState", fail.
3. **VPU reset requested or pending** (D +0x394 or +0x395 ≠ 0) → §2.11.2.
4. Task already marked `'SCLT'` → the scaling phase of §2.12.
5. Otherwise the decode phase (§2.11.3 → §2.11.6).

#### 2.11.2 VPU reset handling
In the check: "wait pre-reset" (D +0x393) → pending. Pre-reset not issued (D +0x392 = 0) → release the semaphores,
`SetDevConfig(0x8002000b)` under the reset semaphore; success → D +0x392 = 1 → pending. Issued: no task of this channel in
flight (P+0x4a0 = 0) → pending; else "drop task" → fail (the VPU lost it). The rest of the protocol is the reset callback
of §2.10.

#### 2.11.3 Waiting for the VPU (decode phase, before DecodeFrameDone)
With "need reset" (D +0x398) clear:
1. **VLC request** bit (D +0x348) → clear it, **`MHAL_VDEC_AddVlcBuffer(inst)`**; success → pending; failure → "add vlc
   buffer failed", fail.
2. **`force_reset`** (§2.16) enabled for the channel and its decoded-frame counter (`.bss` array at Ghidra 0x339c0) a
   non-zero multiple of the interval → reset (step 5).
3. **Frame-done** bit (D +0x34c) → clear it, task word 2 = now, go to §2.11.4.
4. No bit: elapsed = now − max(task enqueue time, D +0x3a0); **< 10 s → pending**; ≥ 10 s → "interrupt timeout" → step 5.
5. Reset: unless one is pending (D +0x395), release the semaphores, `SetDevConfig(0x8002000b)` under the reset semaphore;
   success → D +0x392 = 1, D +0x394 = 1 → pending; failure → pending.

With "need reset" set: this channel's bit not in D +0x39c → pending; set → clear the bitmap, count, and go to §2.11.4
with **"drop after collecting"** (the result is read and the ES consumed, then `SetDevConfig(0x80020009)` post-reset,
P+0xd5 = 0, D +0x3a0 = now, and the task fails).

#### 2.11.4 DecodeFrameDone and the result record [R: offsets: mi_vdec reads, mhal writes; Wave5 names from `DecOutputInfo` of [S], I where the order differs]
<!-- ev:mi_jpd-mi_vdec.2.11.4 -->
`MHAL_VDEC_DecodeFrameDone(inst, r)` with r = `{u32 size 0xb0, u32 version 1, …}`, zeroed by mi_vdec first (words mhal
does not write keep 0); failure → "decode done error", fail:

| r off | value (`DecOutputInfo` field) | mi_vdec use |
|---|---|---|
| +0x08 | eStatus (mhal `_ErrorWarnDrv2Api` of errorReason / warnInfo, §2.11.5) | §2.11.5 |
| +0x0c | sequenceChanged | §2.11.6 |
| +0x10…+0x1c | decoded rect x, y, w, h (rcDecoded) | statistics |
| +0x20…+0x2c | display rect x, y, w, h (rcDisplay) | dumps |
| +0x30 / +0x34 | decPicWidth / Height | dumps, statistics (S+0x146c / +0x1470) |
| +0x38 / +0x3c | dispPicWidth / Height (or the configured crop + 1) | meta w / h; "no display" when 0; S+0x1474 / +0x1478 |
| +0x40 / +0x44 | ES read / write pointer | P+0x2dc / +0x2e0 (kept inside [ES phys, phys + size], wrap handled); the ES records up to the read pointer are released (§2.7) |
| +0x48 (u64) | display frame Y address | must equal the task buffer's Y (normal mode) |
| +0x50 | not written by mhal | — |
| +0x54 | Cb − Y (luma plane size, "Ysize") | meta +0x30 |
| +0x58 | stride | meta stride |
| +0x60 (u64) | Cb address | meta +0x20 |
| +0x6c / +0x70 | chroma size / half stride | meta +0x34 |
| +0x78 (u64) | Cr address | — |
| +0x90 | nalType | §2.11.5 |
| +0x94 | u8 (`DecOutputInfo` +0x2e0) | "LongtermRef" in logs, §2.11.5 |
| +0x98 / +0x9c | numOfErrMBs / numOfTotMBs | error-MB threshold |
| +0xa0 / +0xa4 | byte positions of the picture in the ES [I: `bytePosFrameStart` / `bytePosFrameEnd`] (`DecOutputInfo` +0x210 / +0x214) | +0xa0: the release loop takes the PTS of the record whose PA equals it (§2.7) |
| +0xa8 | indexFrameDecoded (−1 none, −2 skipped) | §2.11.5 |
| +0xac | indexFrameDisplay (< 0 = nothing to display yet) | meta mode |

Then, by mode:
- **Normal mode** (`bDisableLowLatency` = 0): the buffer is port 1's if present, else port 0's; its Y phys (`buf+0x48`)
  must equal r+0x48 (else "check Y buffer err", fail). Frame crop in the buffer (`+0x7c…+0x82`, u16 x, y, w, h) = the
  display window (P+0x1e8…) — or, with the dest crop latched (P+0x2b9 and valid), the intersection of the two — and, for
  a VPU-scaled port 0, every value × (target / picture) with x & ~15, y & ~1. PTS (`buf+0`) from the ES record that
  started the frame.
- **Meta mode**: the frame-table entry of indexFrameDecoded ← r; the 0x50 meta record of §2.13 for indexFrameDisplay
  (none → meta +0x40 = −1 and the task fails: nothing to show yet).
- **Refresh / step accounting**: a decode serving a refresh (P+0xe9) clears the flag and the refresh count −1 (and does
  not release ES — the same record is decoded again); else the consumed ES is released. Play mode 3 → step count −1.
- **Frame dump** (`dumpfb`, §2.17) runs here, after the frame is accepted and before the delivery decision.
- **Delivery**: `dropoutbuf` on → fail (frame thrown away); PTS = {−2, −1} (the SendStream end marker) → fail; else task
  word 2 = `'SCLT'`; with two buffers → the scaling phase (§2.12); with one → **done (1)**: decoded-frame counter +1,
  D +0x3a0 = now (via the meta path also P+0x258 = the meta record, §2.11.6); S+0x1460 `DecFrmCnt` +1, the frame count of
  the rate window +1, S+0x147c / +0x147e / +0x1480 = the delivered frame's width / height / stride.

#### 2.11.5 Status, errors and skipping (normal and meta mode)
<!-- ev:mi_jpd-mi_vdec.2.11.5 -->
Evaluated after DecodeFrameDone, before delivery; "drop" = consume the ES of the frame, step accounting, fail (3); a
refresh decode that drops logs "current nalu is invalid vlc data" and clears the refresh. The eStatus tests run in the
order 0x20005 → sequenceChanged → 0 → 0x20001 → bit 5 → bit 6 → `& 0x1e` → == 0x20004 → else:
1. eStatus **0x20005** → log "sw reset, eStatus", VPU software reset (`SetDevConfig(0x80020009)`), P+0xd5 = 0; the other
   status tests are skipped (go to 3).
2. sequenceChanged ≠ 0 → §2.11.6. eStatus 0 → go to 3. **0x20001** → "drop frame, eStatus" → drop. Bit 5 (0x20) → "over
   profile, drop frame"; bit 6 (0x40) → "over level, drop frame" → drop. `& 0x1e` ≠ 0 → go to 3 (a picture decoded with
   errors). **== 0x20004** → "syserr watchdog timeout", P+0xd5 = 1 — **unreachable**, because 0x20004 has bit 2 set and
   `& 0x1e` catches it first. Any other value → log "over max multi-slice num" and the **error table** P+0x30c (32 ×
   {count, code, jiffies}): the code's count +1, logged at most once per second with its count → drop.
3. indexFrameDecoded < 0: a sequence change pending → continue; **−1** → "no available frame buffer" → fail; **−2** →
   "enable skipframeMode to drop frame" → drop; other → "error value" → fail. P+0xec = 1 → cleared.
4. **Error-MB threshold** (`ErrMBDropThrd` P+0x61, 30 by default): numOfErrMBs ≠ 0 and errMBs × 100 > totMBs × threshold
   → the picture counts as broken: a random-access picture (H.264 nal 5 IDR, H.265 **nal 19 IDR_W_RADL**) → P+0x9d = 1
   ("wait for IDR"), else P+0x9c = 1; the frame is dropped.
5. **Wait flags**: P+0x9d set → frames that are non-reference / droppable (H.264 nal 1, H.265 **nal < 2** TRAIL_N /
   TRAIL_R) are dropped ("lost frame") until another kind arrives, then the flag is cleared; P+0x9c set → the same, except
   frames with "LongtermRef" (r+0x94) = 1 pass; eStatus with `& 0x1e` (≠ 8) and P+0xa0 = 1 → P+0x9e = 1 → the same
   skipping until the next reference picture [I: P+0xa0 is never written in this module (0), so the P+0x9e rule is dead].
   [I] Net effect for the camera's own IPPP H.264: one broken P frame makes the decoder drop every frame until the next
   IDR instead of showing smeared pictures.

**eStatus values** — produced by mhal's `_ErrorWarnDrv2Api(code, *out, codStd, isError, seqChange)` (0x400bad60, 7152) for DecodeFrameDone [2] and SeqInit / CompletSeqInit / SeqChange [0x17];
two encodings share the word [R mapping; the [I] names follow the Chips&Media error / warning numbering — the appendix
itself is not in the tree, `vpuapi.h` 2946 only points to it].

Error path (`isError` = 1; `code` = the firmware `errorReason` when DecodeFrameDone has `decodingSuccess` = 0 and
`errorReason` ≠ 0, CompletSeqInit's `seqInitErrReason` (`DecInitialInfo` +0xc4, printed "SEQERR"), or for SeqInit mhal's
own return code); one value:

| input `code` | eStatus | meaning |
|---|---|---|
| (only with `seqChange` ≠ 0) 0x1002, 0x1003, 0x1008, 0x1009, and 0x100b (HEVC) / 0x100d (H.264) | **0x20007** | sequence change with an SPS error ("SeqChage with err %d" logged); SeqChange also returns 0x20007 when re-registering the frame buffers fails [R]. [I: by the Chips&Media numbering: picture width / height, luma / chroma bit depth, max-dec-pic-buffering — the fields that force new frame buffers] |
| 0x5000 | **0x20000** | SPS not found ("not found sps, retry init seq" at sequence init) [R] |
| 0x5001 | **0x20001** | VCL (picture data) not found [I: CompletSeqInit's warning for this group reads "DEC_PIC_HDR not found sps or not found vcl … SEQERR(%08x)" [R string], 0x5000 is the SPS case]; DecodeFrameDone does not log it ("decode error" suppressed for 0x20001) and mi_vdec drops the frame quietly [R] |
| 0x5002 | **0x20002** | [I: no valid slice in the access unit] |
| 0x500f | **0x20003** | [I: another decoder error of the 0x5000 group] |
| 0x4004, 0x4005 | **0x20006** | [I: spec-violation group 0x4000 — buffer overflow / wrong buffer access] |
| 0x20000 | **0x20004** | firmware watchdog timeout — mhal's `Wave5VpuDecGetSeqInfo` maps the same fail reason to `RETCODE_VPU_RESPONSE_TIMEOUT` (0x10); mi_vdec's message is "syserr watchdog timeout" [R] |
| 0x80000 | **0x20005** | [I: VCPU timeout]; mi_vdec answers with the VPU software reset [R] |
| anything else (incl. SeqInit's own 2 / 3 / 4) | **0xfffff** | unknown |

Warning path (`isError` = 0; DecodeFrameDone with `decodingSuccess` ≠ 0; `code` = firmware `warnInfo`); a bit set, 0 when
none of the listed bits is set; other `warnInfo` bits are dropped [R]:

| `warnInfo` bit | eStatus bit | name |
|---|---|---|
| 16 | 0x001 | [I: init-seq / VCL not found] |
| 17 | 0x002 | [I: missing reference picture] |
| 18 | **0x004 for H.265, 0x008 for H.264** | [I: H.265 wrong temporal id / H.264 error picture referenced — the only codec-dependent bit] |
| 19 | 0x010 | [I: error picture referenced] |
| 20 | **0x020** | **over profile** [R: mi_vdec's message] |
| 21 | **0x040** | **over level** [R] |
| 22 | 0x080 | [I] |
| 25 | 0x100 | [I] |
| 26 | 0x200 | [I: pre-scan / parse error] |

So `& 0x1e` = "any of warning bits 17–19" (missing / wrong / erroneous reference): a picture decoded with reference
problems [R mapping; I wording]. What mi_vdec does with each value, given that the error codes all carry low bits [R]:

| eStatus | effective branch |
|---|---|
| 0 | normal |
| 0x20000 | "over max multi-slice num" message, error table, drop |
| 0x20001 | "drop frame, eStatus", drop (counted as `DropNoVCL` [I]) |
| 0x20002, 0x20003, 0x20006, 0x20007 (`& 0x1e` ≠ 0) | treated as "decoded with errors", on to the index / error-MB checks |
| **0x20004** | also `& 0x1e` ≠ 0 (bit 2) → as the row above; the watchdog branch is unreachable |
| 0x20005 | "sw reset", post-reset `SetDevConfig(0x80020009)`, P+0xd5 = 0 |
| **0xfffff** | bit 5 set → logged as "over profile, drop frame" |
| warning sets | bit 5 → over-profile drop; bit 6 → over-level drop; bits 1–4 → continue; only 0x001 / 0x080 / 0x100 / 0x200 → "over max multi-slice num", error table, drop |

(The H.264 warning bit 0x008 and the H.265 bit 0x004 both fall in `& 0x1e`.)

#### 2.11.6 Sequence change
- **Normal mode**: sequenceChanged → P+0x250 = 1, counted, **`MHAL_VDEC_SeqChange(inst, {0x68, 2})`** (`MHAL_VDEC_SeqChange`) at
  once, the task fails; the poll later re-runs the sequence init (state 3 → 5) with new frame buffers.
- **Meta mode** (decoder-owned frames still on screen): P+0x250 = 1 and, if the frame had a display index, it is shown
  first; then the sub-state P+0x254: **1** → the next task gets the dummy buffer (§2.8), painted **black** (Y 0x10,
  C 0x80, the rest 0 over stride × height), delivered (done), sub-state 2; **2** → wait until the last released meta record
  (P+0x260 / +0x264, from ReleaseFrameCallBack) is that black frame's (P+0x258 / +0x25c) → sub-state 3; **3** → held count
  0, generation = 1, `SeqChange`, sub-state 4; **4** → wait (the poll's sequence init takes over). So the old DPB is reused
  only after the display has let go of every old frame, with a black frame bridging the gap.

#### 2.11.7 Ordering and concurrency
- `OnSupportOutputTaskReorder` (slot 0x7c) returns 1 and the pass accepts 3 concurrent tasks (slot 0x8c) with at most one
  output task per channel (P+0x4a0) — so mi_sys may finish tasks of different channels out of order; a channel's own
  frames stay in order [I from the limits].
- Display reordering (B-frames) is the VPU's: in normal mode a frame comes out in **decode order** (the VPU writes the
  task's own buffer and must report it as the display frame, else the Y check fails — streams that need reordering are
  not handled in this mode [I]); in meta mode the VPU's display index selects the frame, i.e. display order [I: the VPU's
  display order is the reorder].

### 2.12 Port 1 and the SCL device 3 stretch path [R; I where marked]
Runs only when a task carries buffers for both ports, i.e. when port 1 has a consumer (§2.4).
- After DecodeFrameDone the check marks the task with `'SCLT'` in task word 2 and runs **`_MI_VDEC_EnqueueScalingTask`
  (0x00013cc4)**: source = port 1's frame descriptor (0x68 B from buf+0x20), destination = port 0's buffer;
  destination size = the port-0 size of §2.8 (the P+0x498 pair, or the source size when no scaling is set: a 1:1 copy),
  destination crop = source crop × (target / picture); `mi_scl_StretchBufByVdec(src {type 1}, &zero, dst {type 1},
  _MI_VDEC_StretchBuffCB)` (the VDEC variant of SCL's StretchBuf) on **SCL device 3** (created at InitDev with `mi_scl_CreateDevice(3, {0x20})`, `mi-capture.md`
  §6.8). The device-wide job state in D (+0x408 result, +0x410 start time, +0x420 the task being scaled, +0x424 u8 busy)
  allows **one scaling job at a time for all channels**. Return of `StretchBufByVdec`: 0 queued (busy = 1, result
  "pending"), 2 SCL busy → retry later (P+0x1bcc = 1), 3 failed → the task is dropped.
- **`_MI_VDEC_StretchBuffCB(ctx, result)`** (SCL completion, any context): when no job is marked busy only the result is
  stored; for results 1 or 3 it first waits up to 100 ms (1-ms sleeps) for the busy flag [R; I: guards the race with the
  enqueue], then stores the result and the time, calls `mi_sys_NotifyWakeupISREventByChn(h, 0, …)`, busy = 0.
- **The check for an `'SCLT'` task** (§2.11.1 step 4): another task's job in flight → "scaling busy", P+0x1bcc = 1, return
  0 (pending); this task's job (or a retry flag) → ask `_MI_VDEC_EnqueueScalingTask` again: 0 running → pending; 2 → pending ("retry
  later"); 1 **done** → D +0x420 = 0, frame counted, return 1 (both buffers delivered); 3 → drop (task failed).
- Consequence [I]: binding port 1 costs one SCL dev-3 pass per frame and serialises all channels on it.

### 2.13 Decoder-owned display frames — `bDisableLowLatency` (attr +0x22 → P+0x62) [R; names from the proc view]
The app sends 0. The module parameter `disable_low_latency_flag` plays no part (§2.1).
- **`bDisableLowLatency` = 0 ("low latency", mhal caller-buffer mode)**: the VPU writes each picture into a buffer mi_sys
  allocated for this task (port 0 or 1); the frame is delivered as soon as the task completes. The decoded-frame address
  returned by DecodeFrameDone must equal the task's buffer (§2.11.4); the output config is (re)sent per task.
- **`bDisableLowLatency` ≠ 0 (mhal-buffer mode, "meta mode")**:
  - StreamOn passes the byte (`{0x40, 2}` byte [0xc] → mhal `open+0x68`); `CompletSeqInit` then makes mhal allocate and
    register its **own DPB frame buffers**, and mi_vdec sends the output config once per sequence (§2.8).
  - The poll asks mi_sys for a meta buffer (§2.8). At frame done the check fills the **meta record** (`buf+0x20`, 0x50 B):

    | meta off | field |
    |---|---|
    | +0x00 | 0 linear / 6 tiled |
    | +0x04 | pixel format 0xB |
    | +0x18 (u64) | Y phys |
    | +0x20 (u64) | C phys |
    | +0x28 / +0x2c | Y stride / C stride |
    | +0x30 / +0x34 | Y size / C size |
    | +0x38 / +0x3a | u16 display w / h |
    | +0x3c | **display frame index** |
    | +0x40 | **generation** (P+0x1bc8); −1 when there is nothing to display |
    | +0x44…+0x4a | u16 crop {x, y, w, h} (dest crop and port scaling applied as in §2.8) |

    Buffer +0x38 = 8, +0x30 = 0x50, PTS from the ES record that started the frame. The display index is entered in the
    32-entry frame table (P+0x4c8 + idx·0xb8 ← the 0xb0 decode result) and the held-frame count (P+0x4c0) +1.
  - Flow control: the poll starts no new decode while 4 or more display frames are held unless a sequence change is
    pending; StopChn / SeqDeinit refuse (BUSY) while frames are held; FlushChn waits ≤ 20 ms for them.
  - A consumer must understand the 0x50-byte meta record to show such frames [I: the vendor DISP does].

### 2.14 Dequeue and frame release [R]
- **`DequeueOutputTask`** (slot 0x28): counters, in-flight −1, stamps S+0x1504; mi_sys delivers the frame to the bound
  input (the DISP input in the app).
- **`ReleaseFrameCallBack`** (slot 0x68, when a consumer releases an output buffer): wakes the channel. If the channel is
  in meta mode, the codec ≠ 2 and the released buffer is a meta buffer (type 2): the port id must be {module 8, dev 0,
  chn < 32, port 0}; the record's generation must equal P+0x1bc8 (stale records from before a sequence change are
  ignored) and its index ≥ 0 → held count −1, **`MHAL_VDEC_ClearDispFlag(inst, index)`** — the VPU may reuse that DPB
  buffer; P+0x260 = the record. Consequence [I]: a display that holds a VDEC frame in meta mode keeps a DPB slot busy.

### 2.15 H.265 [R]
Nothing in mi_vdec parses the stream, so H.265 differs from H.264 only in these places:
- **CreateChn**: codec 1 accepted with the **same limits** as H.264; `MHAL_VDEC_CreateInstance(dev, codec = 1, &inst)` →
  mhal opens a HEVC instance.
- **Error / skip policy** (§2.11.5): random-access point = nal 5 (IDR) for H.264, nal 19 (IDR_W_RADL) for H.265;
  non-reference / droppable picture = nal 1 (H.264 non-IDR slice) vs nal < 2 (TRAIL_N / TRAIL_R) for H.265.
- The warning bit 18 of the eStatus mapper (0x004 for H.265, 0x008 for H.264) and the `setperf` input count (§2.18).
- Names only: dump files (`…_h265_…`, `.h265`), the proc view codec column ("H265"). Codec 2 ("jpeg") appears in the dump
  and release code, but CreateChn rejects it.
- ES ring, sequence init (`SeqInit {0x68, 2}`), frame buffers, output ports, crop and scaling are shared. The H.265 limits
  that matter (CTU 64, a 10-bit profile → "over profile" drop) live in the VPU / mhal.

### 2.16 procfs commands of `/proc/mi_modules/mi_vdec/mi_vdec0` and `/customer/magic_book.cfg` [R]
Registered at device init (`mi_sys_RegistCommand(name, argc, fn, handle)`; the argc is the minimum the handler wants,
including the command word). `cat` of the node prints the help below (`_MI_VDEC_OnHelp`) and the views of §2.18; a bad
argc prints a usage line and returns `0xA008201F`, as does a chn ≥ max ("Debug Chn(%d) Error"). Offsets in `[…]` are into
`_stVdecDbgInfo` (§2.2).

| command (help text) | args | effect |
|---|---|---|
| `vpulog [chn] [on/off] [level]` | ≥ 5 | **no-op**: `_MI_VDEC_EnableVpuLog` returns 0 (VPU firmware logging is not wired in this build) |
| `setprint mode level` | ≥ 3 | mode < 2 → print mode (0 = `printk`, 1 = MI log), level < 3 → print level of the module's `[WRN]` / `[INFO]` flow messages (0 none, 1 warnings, 2 also info); other values → `…1F` |
| `dumpbs [chn] [path]` | ≥ 3 | path "off" → stop; else store the path (`[0x24a + 0x90·chn]`, the field `dumpfb` shares) and enable `[0x241]`: every **SendStream** then appends the copied ES chunk to `<path>/chn_<chn>_<h264\|h265\|jpeg>_dump_vdec.es` (open `O_WRONLY\|O_CREAT\|O_APPEND\|O_SYNC`, 0644); a write error disables it |
| `dumpfb [chn] [path] [frmcnt] [bDumpAll] [bDetile]` | ≥ 6 | §2.17 |
| `dumpbsb [chn] [path]` | ≥ 3 | **at once**: write the whole ES ring (VA P+0xb8, size P+0xbc) to `<path>/chn_<chn>_dump_vdec_bitstream_buf.<h264\|h265\|jpeg>` if the channel is started |
| `flowdbg [on/off]` | ≥ 1 | `[0]` — enables the "FuncFlow Debug" lines of the view (§2.18) |
| `dropoutbuf [on/off]` | ≥ 1 | `[1]` — CheckOutputTaskStatus then **does not deliver** decoded frames (they are released at once) [I: a decode-only throughput test] |
| `setperf [on/off]` | ≥ 1 | `[2]` — record the per-stage timestamps (ms) for the performance tables of the view and count input pictures for `ifps` |
| `flowstat [chn] [on/off]` | ≥ 2 | S+0x1508 — per-channel flow statistics (µs; the timing helpers `vdec_flow_time_start` (0x154e0) / `vdec_flow_time_end` (0x156c8) run only when set); its messages say "Start/Stop drop output buffer" (copied text) |
| `force_reset [chn] [on/off] [frmintvl]` | ≥ 3 | `[0x2ca + 0x90·chn]` = on (number, 0 / 1), `[0x2cc]` = interval: CheckOutputTaskStatus forces a **VPU reset every *frmintvl* decoded frames** of that channel (§2.11.3) |

**`/customer/magic_book.cfg`** (absent in the dump): read once at the end of every InitDev (`vdec_magic_book_replay`), `filp_open
(O_RDONLY|O_SYNC)`, **at most 1023 bytes**. Format: shell-like lines `echo <command> <args> > …mi_vdec0`. Parsing: find
each `echo`; the command text runs up to the next `>`; the target text after `>` must contain `mi_vdec0` (dev 0) or
`mi_vdec1` (dev 1 — the handlers ignore the device), else the line is skipped; split on space / CR / LF into ≤ 20 tokens;
dispatch on the first token to the same handlers as the proc node: `setprint`, `dumpbs`, `dumpfb`, `dumpbsb`, `flowdbg`,
`dropoutbuf`, `setperf`. **`vpulog` is skipped on purpose; `flowstat` and `force_reset` are not recognised.** Handler
output goes to `_MI_VDEC_OnPrintOut` (discarded). So the file only pre-sets debug switches from boot; nothing in it
affects decoding except `dropoutbuf`.

### 2.17 `dumpfb`: the frame dump files [R; addresses Ghidra = objdump `.text` + 0x10028]
**Command** (`_MI_VDEC_DumpFrameBuffer` 0x14f70) `dumpfb chn path num bDumpAll bDetile`: argc < 6 → usage text,
`0xA008201F`; chn ≥ max channels → "Debug Chn(%d) Error", `0xA008201F`. Path "off" (case-insensitive, first 3 chars) →
enable `[0x242 + 0x90·chn]` = 0. Else enable = 1, path copied to `[0x24a + 0x90·chn]` with `strncpy(dst, path,
strlen(path)+1)` — **no length limit** although the field is 0x80 bytes (the next field, `[0x2ca]`, is `force_reset`'s
on-flag); the path is the one `dumpbs` uses too. `num` empty → frame count `[0x244]` = 3, else `strtoul(num)`. In both
cases (also with "off") `bDumpAll` → `[0x249]`, `bDetile` → `[0x248]` (u8 of `strtoul`). Frame counters: §2.2.

**When** (CheckOutputTaskStatus 0x1bac0…, objdump 0xbafa–0xbb0a, 0xc014–0xc01e): after DecodeFrameDone has accepted a
frame (so also for frames `dropoutbuf` throws away), if `[0x242]` is set: meta mode (P+0x62 ≠ 0) →
`_MI_VDEC_DebugDumpMetaFrmBuf(P, &r, buf, tiled)`, else `_MI_VDEC_DebugDumpFrameBuffer(P, &r, buf, tiled)`; `tiled` = byte
P+0x2f0, passed in r3 in both calls. Then in each routine:
- counter ≤ `[0x244]` − 1 (unsigned) → dump, counter +1; otherwise counter = 0 and `[0x242]`, `[0x248]`, `[0x249]` are
  cleared (the dump stops by itself). **`num` = 0 makes `[0x244]` − 1 = 0xFFFFFFFF, i.e. every frame is dumped until
  `dumpfb chn off`** [R arithmetic, I consequence]. With `[0x242]` = 0 at entry the routine clears the counter, `[0x248]`,
  `[0x249]`.
- codec P+0x40: 0 → "h264", 1 → "h265", anything else (JPEG) → return at once, no file, counter unchanged.
- r+0x48 (u64 Y) = 0 or r+0x60 (u64 C) = 0 → "yuv address error" (logged only at debug level ≥ 2), no file.
- `mi_sys_Vmap` of the buffer fails → no file, counter unchanged.

**Frame path** `_MI_VDEC_DebugDumpFrameBuffer` (0x1a518, normal mode). From the mi_sys output buffer: Y phys `buf+0x48`
(u64), C phys `buf+0x50`, stride `buf+0x60`, total size `buf+0x6c`; vmapped from Y phys, size rounded up to 4 KiB.
- Luma size L = C phys − Y phys; chroma size C = `tiled` ? (Y phys + size − C phys) : L / 2.
- Main file `<path>/dump_chn<NN>_<h264|h265>_src_<r+0x30>_<r+0x34>_dst_<r+0x38>_<r+0x3c>_<stride>_<n>.yuv` (decoded W / H,
  display W / H, `buf+0x60`, frame counter n; `%02d` / `%d`), **always written**, with **`size` bytes** (the whole mi_sys
  buffer, padding included):
  - `bDetile` = 0: the buffer bytes as they are — linear NV12 (Y plane L bytes, then interleaved CbCr) when the channel
    is linear, the raw tiled image when it is tiled [R copy; I format];
  - `bDetile` ≠ 0: `vmalloc(size)`, `_MI_VDEC_DebugDetileYUV(dst, Y, Y+L, dispW = r+0x38, dispH = r+0x3c, stride, P+0x2f8,
    L, C)`, write `size` bytes from dst, `vfree`. vmalloc failure → no main file (silently), counter still +1.
- `bDumpAll` ≠ 0: **in addition** `<path>/dump_chn<NN>_<r+0x38>_<r+0x3c>_<stride>_<n>_luma.yuv` (L bytes from Y) and
  `…_chroma.yuv` (C bytes from Y + L), raw (never de-tiled), written before the main file; these names carry neither codec
  nor decoded size.
- Before writing, a `[MI WRN]` line "Chn %d dump dec[…], disp[…], stride: %u, bufY…" is printed at debug level ≥ 2.

**Meta path** `_MI_VDEC_DebugDumpMetaFrmBuf` (0x19f64). From the meta record m = buf+0x20: Y phys m+0x18 (u64), Y stride
m+0x28, C stride m+0x2c, Y size m+0x30, C size m+0x34, display height m+0x3a (u16).
- `tiled` = 0: L = m+0x30, C = m+0x34. `tiled` ≠ 0: H64 = (m+0x3a + 63) & ~63, **L = m+0x28 · H64**, **C = m+0x2c ·
  ((H64/2 + 63) >> 6) · 64**; L ≠ m+0x30 → warning "Y buffer size error, u32YSize: L, get YSize: m+0x30" (debug level ≥ 2)
  and the computed L is used anyway (objdump 0xa248–0xa29a).
- Vmapped from Y phys, (L + C) rounded up to 4 KiB — the chroma plane is assumed to follow the luma plane at Y + L (the
  C phys m+0x20 is not used).
- Main file `<path>/dump_chn<NN>_<h264|h265>_<r+0x38>_<r+0x3c>_<m+0x28>_<n>.yuv` (no `src_` / `dst_` parts), **L + C bytes**:
  raw, or de-tiled into a `vmalloc(L + C)` copy (`DetileYUV(dst, Y, Y+L, r+0x38, r+0x3c, m+0x28, P+0x2f8, L, C)`).
  `bDumpAll`: `_luma.yuv` (L bytes) and `_chroma.yuv` (C bytes) with the same names as the frame path.

**De-tiling** `_MI_VDEC_DebugDetileYUV(dst, srcY, srcC, W, H, stride, unused, L, C)` (0x19df8). The output is linear with
the **same stride** as the source: `dst[0 … L)` luma, `dst[L … L+C)` interleaved CbCr (NV12). The 7th argument (P+0x2f8)
is not read. For every output byte i (luma: i < L; chroma: i < C), x = i mod stride, y = i / stride:
- luma: copied only if x < W and y < H; chroma: only if x < W and y < H/2 and the source offset < C (no bound check for
  luma). **Bytes outside the display rectangle are not written** — they keep whatever `vmalloc` returned (uninitialised
  kernel memory ends up in the file) [R no memset; I consequence].
- Source offset (both planes, `srcY` / `srcC` as base):
  `off = (x & ~0x7f)·64 + (y >> 6)·stride·64 + ((y & 0x3e) >> 1)·256 + ((x & 0x70) >> 4)·32 + (y & 1)·16 + (x & 0xf)`
  [R; `(y & 0x3e)·0x80` and `(x & 0x70)·2` rewritten]. That is a **128 × 64 tile** of 8192 bytes, tiles stored left to right
  (`(x / 128)·8192`) in tile rows of `stride·64` bytes; inside a tile, 32 pairs of rows of 256 bytes; each pair is 8 units of
  16 × 2 pixels (32 bytes: 16 bytes of the even row, then 16 of the odd row) [I from the arithmetic]. The chroma plane uses
  the same formula on interleaved CbCr bytes, so one chroma tile covers 64 CbCr pairs × 64 chroma rows (128 × 128 picture
  pixels) [I] — the 128 × 64 buffer alignment of the tiled layout (§2.8).

**File writer** `vdec_file_write(name, buf, len)` (0x10fd4): `set_fs(KERNEL_DS)`; `filp_open(name, O_RDWR|O_CREAT (0x42),
0777)`; open failure → nothing (no log); `f_op->llseek(f, 0, SEEK_SET)`; `vfs_write(f, buf, len, &f->f_pos)`; written ≠
len → "vfs_write buffer failed, pu8buf, u32Len" at debug level ≥ 2 (objdump 0xffc `cmp r0, r7`); restore fs; `filp_close`.
**No `O_TRUNC`**: dumping again to an existing longer file (same name — the counter restarts at 0 for every new `dumpfb`
session) leaves its old tail [R flags, I consequence].

### 2.18 Read view `_MI_VDEC_OnDumpChannelAttr` [R]
<!-- ev:mi_jpd-mi_vdec.2.18 -->
`cat /proc/mi_modules/mi_vdec/mi_vdec0` runs `_MI_VDEC_OnDumpChannelAttr(print, ctx, dev)` (0x15e9c, `.text`
0x5e74) after the device view. Tables in print order; row filters: "created" = P+0 = 1, "created and started" = the u16 at
P+0 = 0x101. Fields located from literal-pool relocations, writers looked up by their stores; a label's meaning beyond the writer is [I].

| table (header line) | rows | column = field |
|---|---|---|
| "CHN ATTR Info" | created | `ChnID` index; `CodecType` P+0x40 (0 "H264", 1 "H265", 2 "JPEG", else the number); `Width` P+0x4c; `Height` P+0x50; `BufSize` P+0x44; `VideoMode` P+0x54 (0 "STREAM", 1 "FRAME", else number); `DpbBufMode` P+0x58 (0 "NORMAL", 1 "INPLACE1", 2 "INPLACE2", else "INVALID"); `RefFrmNum` P+0x5c, printed "NA" when the codec is JPEG — the create attr |
| "CHN PARAM Info" | created | `bCusPolicy` P+0x60 ("Y" / "N" / number); `ErrMBDropThrd` P+0x61; `DisplayMode` P+0xa4 (0 "PREVIEW", 1 "PLAYBACK", else number); `bDisableLowLatency` P+0x62 |
| "Crop & Scale Info" | created | `bCrop` P+0x2b9 (latched crop enable); `CropX/Y/W/H` P+0x2cc / +0x2d0 / +0x2d4 / +0x2d8 (the latched copy, not the SetDestCrop value); `bScale` P+0x496 (Y / N); `ScaleW` / `ScaleH` u16 P+0x498 / +0x49a (the port-0 scaling latch) |
| "Decode Frame Info" | created | `DecW` S+0x146c, `DecH` S+0x1470, `DispW` S+0x1474, `DispH` S+0x1478 (written by CheckOutputTaskStatus at frame done; [I] the sizes of the last frame); `RefRecFrmCnt` P+0x22c (frame-buffer count); `LinearFrmCnt` P+0x230 |
| "Output Frame Info" | created | `bTileMode` / `TileFormat`: computed **once, before the row loop**, from a channel pointer left over from the previous loop, so every row shows the same value [R decompile + objdump 0x64ba–0x64c6 (`ldr r2,[sp,#0x2c]`); I: the P of the last slot visited]: P+0x2f0 = 0 → "N", "NA"; else "Y" and P+0x2f8: 4 "32x32", 5 "128x32", 6 "128x64", other "Invalid". `Width` u16 S+0x147c, `Height` u16 S+0x147e, `Stride` S+0x1480 (written by CheckOutputTaskStatus from the delivered frame); `PixelFmt` the constant "YUV420SP" |
| "CHN STATE" | created | `bStart` P+1 (Y / N); `DecState` P+0xd8; `SendCnt` S+0x1440 (SendStream calls); `SendStrmSize` u64 S+0x1448 (bytes sent); `LeftStrmBytes` S+0x1450 (written by SendStream and the release); `LeftCnt` P+0x74; `DecFrmCnt` S+0x1460 (+1 per accepted frame); `fps` u16 S+0x146a [R; 64-bit argument alignment checked] |
| `Start Get GetOK Done Drop DropNoVCL Run SeqChange` | created and started | S+0x14b0, +0x14b4, +0x14b8, +0x14bc, +0x14c0, +0x14c4, +0x14c8, +0x14cc. `Start` is written by EnqueueAsyncOutputTask, `Get` / `GetOK` / `Done` / `DropNoVCL` / `Run` / `SeqChange` by CheckOutputTaskStatus; [I] = tasks enqueued, result fetches tried / succeeded, frames done, dropped, dropped as "no VCL" (eStatus 0x20001), still running, sequence changes |
| `Issue Complete InitSeq VlcInsuff FrmDone ChkInitSeq ChkVlcInsuff ChkFrmDone RcvRstCB ChkRstCB` | created and started | `Issue` S+0x14a8 (+1 in the poll); `Complete` S+0x14ac (`vdec_complete_queued_seqinit` / `vdec_complete_seqinit`, the sequence-init completion); `InitSeq` S+0x14d0, `VlcInsuff` S+0x14d8, `FrmDone` S+0x14d4 (all three counted in `_MI_VDEC_Interrupt_Handler`; storage order InitSeq, FrmDone, VlcInsuff, print order as the header); `ChkInitSeq` S+0x14f0 (`vdec_complete_seqinit`); `ChkVlcInsuff` S+0x14ec, `ChkFrmDone` S+0x14e8, `ChkRstCB` S+0x14e4 (CheckOutputTaskStatus); `RcvRstCB` S+0x14e0 |
| "ISR STATE" `IsrCnt InitSeq VlcInsuff FrmDone UnderRun HwTimeout Other` | one row, device `dev` | D'+0x148, +0x14c, +0x150, +0x154, +0x158, +0x168, +0x16c (= D+0x350 … D+0x374); D'+0x15c…+0x164 are not printed |
| "PERF STATE" `ifps bs_addr bs_size u64RdPtr u64WrPtr EnqueCnt` | created; **only when `setperf on`** | `ifps` u16 S+0x145a; `bs_addr` u64 P+0xb0 (ES ring phys); `bs_size` P+0xbc; `u64RdPtr` / `u64WrPtr` = words +0x14 / +0x18 of a fresh `MHAL_VDEC_GetBSBuffer` record, printed `%8x` (32 bits); `EnqueCnt` atomic P+0x4a0 (output tasks in flight) [R objdump 0x6a12–0x6a66]. The query takes the channel semaphore P+0x20; a failure prints "get bitstream buffer failed" and **ends the whole view** with `0xA008201F` (the later tables are not printed) |
| `poll enque isr check dequeue` | created and started; `setperf` | S+0x14f4 (poll), +0x14f8 (EnqueueAsyncOutputTask), +0x14fc (Interrupt_Handler), +0x1500 (CheckOutputTaskStatus), +0x1504 (DequeueOutputTask) — timestamps of the last pass of each stage, **ms** (monotonic ns / 10⁶, low 32 bits) [R for poll, enqueue, dequeue; I for isr and check] |
| `p->e e->i i->c c->d p->d` | same | differences of the five: enque − poll, isr − enque, check − isr, dequeue − check, dequeue − poll (signed 32-bit) |
| "FuncFlow Debug" `RUNTIME1…4 CP1…CP4` | **only when `flowdbg on`** | `RUNTIME1…4` = `_stVdecDbgInfo` +0x20 / +0x28 / +0x30 / +0x38; `CP1…CP4` = +0x0c / +0x10 / +0x14 / +0x18 — the source line numbers last stored (statements like `CP3 = 0x18b4` at each step), i.e. where each callback last was (§2.2) |
| "Chn%02d Flow Stat" `FLOW MinTimeIntvl MaxTimeIntvl AvgTimeIntvl CurTimeIntvl StatCnt` | created and `flowstat chn on` (S+0x1508) | 26 rows, names from the `.rodata+0x668` table (8-byte entries {name, id 1…}): MI_VDEC_IMPL_StartChn, MI_VDEC_IMPL_StopChn, MI_VDEC_IMPL_SendStream, START_TO_FIRST_STREAM_IN, START_TO_FIRST_FRAME_OUT, START_TO_STOP, STOP_TO_START, START_TO_START, STOP_TO_STOP, SEND_STREAM_TO_SEND_STREAM, SEND_STREAM_TO_PREPARE, PREPARE, ENQUEUE, CHECK, DEQUEUE, PREPARE_TO_ENQUEUE, ENQUEUE_TO_ISRCB, ISRCB_TO_CHECK, CHECK_TO_DEQUEUE, DEQUEUE_TO_PREPARE, PREPARE_TO_DEQUEUE, PREPARE_TO_PREPARE, ENQUEUE_TO_ENQUEUE, ISRCB_TO_ISRCB, CHECK_TO_CHECK, DEQUEUE_TO_DEQUEUE. Row k = 0x48 bytes at S+0x1510 + k·0x48: `StatCnt` u64 +0x00, `CurTimeIntvl` u64 +0x18, `Min` u64 +0x20, `Max` u64 +0x28, `Avg` u64 +0x38 [R objdump 0x6e08–0x6e40] |

**Flow-stat row** in full (`vdec_flow_time_start` / `vdec_flow_time_end`): +0x00 u64 count, +0x08 u64 start / previous time, +0x10 u64
now, +0x18 u64 last interval, +0x20 min, +0x28 max, +0x30 u64 sum, +0x38 avg = sum / count, +0x40 u8 "span open". Time
unit **µs** (`CamOsGetMonotonicTime` ns / 1000). `vdec_flow_time_start` = an "X_TO_X" tick: interval since the previous call, then
start := now; `vdec_flow_time_end` = the end of an "A_TO_B" span (interval since +0x08, set elsewhere) and clears +0x40. Neither
runs a first interval while +0x08 is 0. **Min is never initialised** (no store to +0x20 outside these two; it starts at 0
from the zeroing at InitDev) and is only lowered, so `MinTimeIntvl` reads 0 [R no other writer; I consequence]. With print
mode / level set, each update also logs "pts diff min max sum avg cnt".

**Rate counters**: `ifps` / `fps` come from the poll's rate window (§2.8). The input-picture count S+0x1458 is counted in
SendStream **only while `setperf` is on** and only for a chunk of ≥ 5 bytes that starts with `00 00 00 01` and a NAL header
byte b with b < 3, b ∈ {0x41, 0x65, 0x26} or (b & 0xbf) = 0x21 [R; I: H.264 slice / IDR, H.265 TRAIL_N / TRAIL_R /
IDR_W_RADL headers] — so `ifps` reads 0 unless setperf is on; `fps` is always counted.

SendStream keeps P+0x7c, P+0x68, P+0x70, P+0x74 and mirrors the first three into S+0x1440, S+0x1448 / +0x144c, S+0x1450.
Written but shown in no table: S+0x1454 (StartChn), S+0x1484 (CheckOutputTaskStatus) and the per-flow +0x08 / +0x10 /
+0x30 / +0x40 words.

### 2.19 Imports and exports [R]
- mhal: the 28 `MHAL_VDEC_*` of §2.2, only through the ops table. Ids used: `SetDevConfig` 0x80020003 (memory ops),
  0x80020008 (interrupt handler), 0x8002000b (pre-reset), 0x80020009 (post-reset / software reset), 0x8002000c (reset
  callback); `GetDevConfig` 0x8002000d; `SetParam` 0x80020001, 0x80020004 `{1}` (StreamOn), 0x8002000a (output config);
  never 0x80020005. Their mhal meaning: `mhal-vcodec.md` §8.3.
- **mi_scl** (exports of mi_scl.ko, `mi-capture.md` §6.8): `mi_scl_Init`, `mi_scl_DeInit`, `mi_scl_CreateDevice(3, …)`,
  `mi_scl_DestroyDevice(3)`, `mi_scl_StretchBufByVdec`.
- mi_sys: registration + `RegistCommand`, `Enable/DisableChannel`, `Enable/DisableOutputPort`, `SetOutputPortBufExtConf`,
  `GetChnBufInfo`, `NotifyWakeupISREventByChn`, `RegisterHwTimer` / `UnRegisterHwTimer` / `EnableTimer` / `DisableTimer`,
  `mi_sys_RingPool_{Create, Destroy, SetRecordMinAllocSize, EnsureTailSpaceTimeout, AppendTailContentTailRecord,
  GetHeadRecordUnrelease, ReleaseHeadRecord, GetRingPoolTotalSpace}`, `Ringpool_GetRecordTotalNum` /
  `GetRecordUnreadNum`, `PrivateDevChnHeapAlloc` / `Free`, `Vmap` / `UnVmap` / `VFlushInvCache`,
  `GetModuleMMaHeapName`, `setMiuProtectMMa`. Not imported: `GetHeadRecordTimeout` and the `…InISR` ring calls.
- CamOs, `vmalloc` / `vfree`, `filp_open` / `vfs_read` / `vfs_write` / `vfs_fsync` (magic_book.cfg, dumps). Exports: none.

### 2.20 libmi_vdec.so (33 exports) [R]
The device id is never sent; the wrappers do no argument checks in user space; Get variants copy back only on 0 unless
stated. The library has two pieces of logic of its own:
- **Stream splitting**: `MI_VDEC_CreateChn(dev, chn, attr)` sends `{chn, attr (0x24)}`; if attr word 5 (`eVideoMode`) is
  0 = stream mode, the library mallocs a **512 KiB staging buffer** for the channel and remembers the codec (attr word 0:
  0 H.264, 1 H.265, 2 JPEG). `MI_VDEC_SendStream(dev, chn, pstStream, s32MilliSec)`: frame mode → one ioctl 0x0c `{chn, 0,
  MI_VDEC_VideoStream_t (0x18), s32MilliSec}`; stream mode → append to the staging buffer, cut frames with the exported
  helper `get_next_frame_start_position` (H.264 / H.265: `00 00 00 01` followed by a slice / first-NAL type; JPEG `FF D8`)
  and send each complete frame with 0x0c (end-of-frame flags `0x101`); the tail stays buffered for the next call.
  `MI_VDEC_StopChn` returning 0 also clears the library's per-channel "stream mode" byte.
- **User data**: `MI_VDEC_GetUserData` uses one library-static 0x80c buffer for all channels (below); the kernel answers
  NOT_SUPPORT, so the result is always empty.

| function | nr (size) | payload / copy-back |
|---|---|---|
| `MI_VDEC_CreateDev(dev, *p)` | 0 (4) | {InitParam}; create count +1 (`mi-user-abi.md` §2.2) |
| `MI_VDEC_DestroyDev(dev)` | 1 (**2**) | u16 0; create count −1, close of the fd at 0 |
| `MI_VDEC_CreateChn(dev, chn, *attr)` | 2 (0x28) | {chn, attr}; stream-mode staging above |
| `MI_VDEC_DestroyChn`, `StopChn`, `FlushChn`, `ResetChn`, `PauseChn`, `RefreshChn`, `ResumeChn`, `StepChn` (dev, chn) | 3, 6, 8, 9, 0xd, 0xe, 0xf, 0x10 (4) | {chn} |
| `MI_VDEC_StartChn(dev, chn)` | 5 (4) | {chn} |
| `MI_VDEC_GetChnAttr(dev, chn, *a)` | 4 (0x28) | {chn}; 0x24 back on 0 |
| `MI_VDEC_GetChnStat(dev, chn, *s)` | 7 (0x24) | {chn}; 0x20 back on 0 |
| `MI_VDEC_Set/GetChnParam(dev, chn, *p)` | 0xa / 0xb (0x10) | {chn, p (0xc)} |
| `MI_VDEC_SendStream(dev, chn, *stream, ms)` | 0xc (0x28) | above |
| `MI_VDEC_GetUserData(dev, chn, MI_VDEC_UserData_t *u, u32 ms)` | 0x11 (0x80c) | the static buffer {+0 chn, +4 u8 valid (out), +6 u16 len (in 0x800 = capacity, out), +8 ms, +0xc data[0x800]}; on 0 with len ≠ 0: `u = {pu8Addr = static data, u32Len = len, bValid = +4}`; else `u` zeroed. Not thread-safe; the data are valid until the next call |
| `MI_VDEC_ReleaseUserData(dev, chn, *u)` | — | returns 0 |
| `MI_VDEC_Set/GetDisplayMode(dev, chn, mode)` | 0x12 / 0x13 (8) | {chn, mode} |
| `MI_VDEC_Set/GetOutputPortAttr(dev, chn, *a)` | 0x14 / 0x15 (8) | {chn, u16 w, u16 h} |
| `MI_VDEC_Set/GetOutputPortLayoutMode(dev, mode)` | 0x16 / 0x17 (4) | {mode} |
| `MI_VDEC_Set/GetDestCrop(dev, chn, MI_VDEC_CropCfg_t *c)` | 0x18 / 0x19 (0x10) | {chn, u32 x, u32 y, u16 w, u16 h = 0 …} (in the binary the struct is 2 words + 1 halfword: +4, +8, +0xc u16) |
| `MI_VDEC_SetChnErrHandlePolicy(dev, chn, p*)` | 0x1a (8) | {chn, +4 u8, +5 u8, +6 = 0} |
| `MI_VDEC_Set/GetOutputPortAttrEx`, `Set/GetDestCropEx` | none | print "Not Support", return **0xA0082008** |
| `get_next_frame_start_position` | — | exported helper of the stream splitting |

Payload structs of the unused wrappers are named from the SigmaStar headers where those exist; their field meanings are
the kernel's (this document).

### 2.21 Use on the camera [R app code (`prog_uvc.md` §7.5); I where marked]
- `StartVdec` (0x14c98), on the first play: CreateDev → CreateChn(0, `{codec 0 H.264, ES 0x100000 (1 MiB), priority 0,
  1920×1080, video mode 1 = frame, DPB mode 1, reference frames 1, bytes +0x20 / +0x22 = 0}`) → StartChn →
  SetOutputPortAttr(1920×1080), read back with GetOutputPortAttr in `sstar_module_bind` (only printed / kept in globals).
- Each play: unbind SCL → DISP, **bind VDEC(0, 0, 0) → DISP FRAME_BASE 30:30** (`sstar_module_bind` 0x14e14), then the
  player thread (0x15114) sends one SendStream per MP4 sample (frame mode, bEndOfFrame 1, timeout 20 ms) + GetChnStat,
  paced by `usleep(33000)`; pause = not sending. Stop: unbind VDEC → DISP, rebind SCL → DISP (`sstar_module_unbind`
  0x14fa0). The channel is never stopped or destroyed.
- Used ioctls: 0, 2, 5, 7, 0xc, 0x14, 0x15; output port 0 only; meta mode, port 1, SCL dev 3's stretch path, H.265 and
  every other ioctl, the proc commands and `magic_book.cfg` are unused.
- Player input: the recorder writes the VENC packets into the MP4 unchanged (`KJD_video_mp4_pth` copies each pack from
  its data offset, then `MP4WriteSample`), so samples carry Annex-B start codes and each IDR sample carries SPS / PPS; the
  player sends samples as they are, so mi_vdec gets a valid Annex-B stream for the camera's own 1080p recordings. **Camera reading 2026-11-10 [R]: microcamd's own recordings (V00086/V00087, fMP4) hold AVCC samples (4-byte lengths, SPS/PPS only in `avcC`), not Annex-B — see `docs/measurements/night-vdec.md`; the vendor app's recordings were not looked at.** A
  foreign MP4 (length-prefixed AVCC samples, SPS / PPS only in `avcC`) would not decode — the app parses SPS / PPS into
  globals but never sends them [I from the code].
- **A 4K recording** (3840×2160, menu id 0x17) in the 1920×1080 channel [R for mi_vdec; I for the VPU]: the create
  attr's picture size is only range-checked and shown in proc — nothing passes it to mhal (no `SetDevConfig 0x80020005`).
  Frame buffers follow the **sequence** size from `CompletSeqInit`, and `SetOutputPortAttr(1920, 1080)` makes port 0 a
  VPU-scaled output: target 1920×1080 = 1/2 of the source, inside the 1/8…1 window, so mi_vdec would deliver 1080p
  frames. Whether the VPU accepts 4K under its default maximum resolution is mhal's and is not known (§3).

## 3. What is not known
<!-- ev:mi_jpd-mi_vdec.3 -->

- **4K MP4 playback in the vendor app's 1080p VDEC channel** was not run in the app itself; the VPU decodes 3840×2160
  under its default maximum (no `SetDevConfig 0x80020005`) — read on the camera in channels made at 3840×2160 and
  1920×1080 picture attributes, port 0 full or scaled to 1920×1080 [R camera, 2026-11-10/11] (CAMERA-CHECKS K39).
- **NJPD power registers while the photo viewer decodes**: `HAL_NJPD_PowerOn` (which would set `101e:42[7]` = 0 and the
  J:76 P0 / P1 power bits on, P2 off) is never executed in this build [R]; J:76 and `101e:42` are expected to stay at their
  reset values [I] — not read on the camera (CAMERA-CHECKS K38; the registers are `mhal-jpd.md` §5.1, §5.2's).
- **eStatus names** behind 0x20002, 0x20003, 0x20006, 0x20005 (input 0x80000) and the warning bits 16–19, 22, 25, 26
  (§2.11.5): the Chips&Media error appendix is not in the tree, and the firmware's own error-raising sites are not read.
- `DecOutputInfo` between +0x78 and +0x208 is mapped only where mhal reads it (`mhal-vcodec.md` §8.2).
- The `isr` and `check` timestamps of the read view (S+0x14fc, S+0x1500) are taken to be ms like the other three.
- How many ES records the state-2 sequence-init path itself releases (it bears on `vdec_try_recover_room` never making room,
  §2.7) was not re-read.
- Which of interrupt events 1 and 8 feeds which of the `UnderRun` / `HwTimeout` counters.
- The offset of mi_vdec's HW-reset wait queue in the device record.
- Names of the GetChnAttr / GetChnStatus (JPD) and GetChnStat (VDEC) fields: inferred from field order and debug prints
  (§1.5, §2.6), no header for the M6 `MI_JPD_*` / `MI_VDEC_*` structs was checked.
- The SRAM size mi_jpd requests on the running pipeline (`mi_sys-1-engine.md` §9).
