# mi_vdisp-mi_shadow — mi_vdisp, mi_shadow and their user libraries
<!-- ev:mi_vdisp-mi_shadow -->

Two MI kernel modules of the SigmaStar media stack and their user libraries, both loaded on this camera and unused there
(§1):

- **mi_vdisp** ("virtual display", module id 0x0E) [R names, I purpose] — a compositor. A device owns one output port
  that produces frames of a fixed size and format at a fixed rate; up to 20 input channels each place a picture at a
  rectangle of that frame. Producers bound to a normal input channel allocate **directly into a sub-rectangle of the
  vdisp output buffer** (vdisp installs itself as the producer's allocator through
  `mi_sys_SetPeerOutputPortCusAllocator`), so composition costs no copy; overlay channels are copied on top; regions
  nobody filled are painted with a fill colour (`mi_sys_BufFillPa`) or copied from the previous frame
  (`mi_sys_BufBlitPa`). A private sub-buffer allocator (`sba_*`) and the kernel region-subtraction helpers (`InitClipRgn`
  … `SubtractRegion`) do the bookkeeping. `mi_vdisp.ko` with `/dev/mi_vdisp`; `libmi_vdisp.so` (14 wrappers + 2 entry
  points).
- **mi_shadow** (module id 0x14) [R names, I purpose] — a bridge that lets a **user-space process act as an MI module**: it
  registers a device with mi_sys on behalf of user space (any module id, port and channel counts), turns every mi_sys
  callback of that device (bind / unbind, dump, proc commands, task scheduling) into an event the process collects with a
  blocking ioctl and answers with a result, and maps the buffers of the device's ports into the process
  (`mi_sys_MmapBufToMultiUserVirAddr`). `mi_shadow.ko` with `/dev/mi_shadow`; `libmi_shadow.so` runs the event loop in a
  thread.

Other documents: the user-side ioctl wrapper, fd handling, return convention, MI error-code format and module ids —
`mi-user-abi.md` (§2 "The device fd", §3 "The ioctl call", §4 "Return values and error codes"); the mi_common device frame
that creates `/dev/mi_*` and the per-process hooks — `mi_sys-2-interfaces.md` §1 "mi_common.ko — the device frame of every
MI module"; the registration contract, pass-callback slots and their result codes, bind types and bind capability, frame
flow, back-reference queue — `mi_sys-1-engine.md` (§2 "Registration contract", §3 "Binding", §4 "Frame flow", §7.2 "Task
completion"); the mi_sys exports, allocators and BDMA helpers these modules call — `mi_sys-2-interfaces.md` (§3.2 "The
table", §6.1 "Imported exports", §7.6 "Generic, custom, meta and direct-buffer allocators", §7.8 "BDMA operations");
`libmi_vdf.so`, the only user of libmi_shadow — `sdk-orphan-libs.md` §7; the kernel built-in `region_substract.c` —
`kernel-platform.md` §2.2; the load lines in `demo.sh` and the inventory rows — `firmware-image.md` §2, §3.3, §3.5.

Tags: **[R]** read from the binary, a decompile, disassembly or a file; **[S]** stated by a source; **[I]** inference. A
heading tagged [R] means every statement under it is [R] unless marked otherwise. Struct and field names the binaries do
not carry (the `MI_VDISP_*_t` / `MI_SHADOW_*_t` type names, field names such as `u32BgColor`) follow the SigmaStar MI
headers' naming and count as [I]; offsets, sizes, constants and control flow are [R].

Binaries (dump of 2026-10-16) [R]: `/config/modules/4.9.227/mi_vdisp.ko` (79 300 bytes, 127 functions,
`.text` 0x6B60, about 25 KB of code), `…/modules/4.9.227/mi_shadow.ko` (63 296 bytes, 78 functions, `.text` 0x4C44, about
17 KB of code), `/config/lib/libmi_vdisp.so` (9 680 bytes), `…/lib/libmi_shadow.so` (13 812 bytes) [R:
stat for the byte sizes]. Build string of all four: `project_commit.7251436 sdk_commit.80327e1`, build times
`20220721223317` (mi_vdisp.ko), `20220721223321` (libmi_vdisp), `20220721223311` (mi_shadow.ko), `20220721223314`
(libmi_shadow).

## 1. The components on this camera [R]
<!-- ev:mi_vdisp-mi_shadow.1 -->

- Both modules are loaded by `customer/demo.sh`: `insmod mi_shadow.ko` (line 41) and `insmod mi_vdisp.ko
  vdisp_bufq_depth=4` (line 50).
- **No application opens either device.** prog_uvc has no `/dev/mi_vdisp` string, no libmi_vdisp import and no MI_VDISP
  call (`prog_uvc.md` §7.2); no binary in the dump other than the two libraries names `/dev/mi_vdisp` or `/dev/mi_shadow`
  [R: string search over the dump]. The prog_uvc menu commands 0x19 / 0x1e (freeze / compare view) do not reach
  mi_vdisp: the function at 0x362f4, named `MI_VDISP_DisableInputChannel` in the original function inventory, is `MI_ISP_StopChannel`,
  and prog_uvc's corrected function inventory contains no `MI_VDISP_*` name [R].
- Nothing imports libmi_vdisp. The only importer of libmi_shadow is `lib/libmi_vdf.so` (14 `MI_SHADOW_*` imports), itself a
  library with no user; it registers with mi_shadow as module VDF (module id 1) through its `g_stVDFRegDevInfo`
  descriptor [R] (`sdk-orphan-libs.md` §7.2). VDF fills `MI_SHADOW_RegisterDevParams_t` (§8.2) in
  mi_shadow's event order: 1 input port, 1 output port, 16 channels; no bind callbacks; event 5 (`cat`) →
  `_MI_VDF_OnDumpVdfAllAttr`, 6–8 none, 9 → `_MI_VDF_OnHelp`, 0x0B–0x11 → `_MI_VDF_Cmd{DebugLevel, CheckPoint,
  SelfChecking, WorkModeEnable, ChnEnable, DumpAllEnable, DumpImage}`; dumpUsr = a 0x8000-byte dump buffer
  [R]. This use also confirms the §8.2 field names +0x24…+0x50.
- Both modules and both libraries are therefore **unused** on this camera. mi_vdisp is the only importer of several
  mi_sys allocator exports, mi_shadow the only importer of the multi-user mapping exports (§9.1).

## 2. mi_vdisp — load, device file, ioctls, libmi_vdisp

### 2.1 Load and module state [R]
- `init_module`: proc dir `/proc/mi_modules/mi_vdisp/` with `debug_level`, `debug_file`, `debug_func`,
  `module_version_file` ("Sigmastar Module mi_vdisp version: project_commit.7251436 sdk_commit.80327e1
  build_time.20220721223317"), then `MI_DEVICE_Register` of its device object (`.data+0x58`) — the mi_common frame
  (`mi_sys-2-interfaces.md` §1.2 "Registered device object") creates `/dev/mi_vdisp`. `cleanup_module` removes the four
  proc files and unregisters.
- Parameters: `vdisp_bufq_depth` (short, 0644, `.data+0x8c`, default **3**; demo.sh loads with 4; parameter text "Depth of
  vdisp's bufq to neutralize jitter"), `debug_level` (uint, `.data+0x24`), `debug_file`, `debug_func` (strings) — the
  common MI debug filter.
- Module id **0x0E** (error codes `0xA00E….`, registration module id 14). Up to **4 devices** (dev id > 3 →
  `0xA00E2001`), each with **20 input channels** (0–15 normal, 16–19 overlay, §3.2) and **one output port** (port id ≠ 0
  → `0xA00E2003`).
- Global state `_vdisp_module` (.bss, 0x52A8 bytes; Ghidra address 0x1C850): +0x00 u32 module-initialised flag, +0x04
  unused, then 4 device structures of 0x14A8 bytes from +0x08 — device d at **P = `_vdisp_module` + 8 + d·0x14A8**; device
  fields are written **P+off** below (layout §3.1). Separately: a module semaphore (CamOsTsem, initialised by
  `MI_VDISP_IMPL_Init` with count 1) taken by every IMPL entry point, a per-device mutex at P+0x1478 taken inside it, 4
  per-device timers (`.bss`, 0x1C bytes each, §4.7) and `brecordDir` (.bss, 4 × 20 × 0x80 bytes: per-channel dump
  directory kept across `Init`, §5.3).

### 2.2 ioctls of `/dev/mi_vdisp` [R]
Wrapper and return convention as in `mi-user-abi.md` §3.1 "The argument is a 16-byte wrapper, not the struct" (`struct {u32
len; u32 pad; u64 ptr}`) and §4.1 "Return convention" (the handler's MI code is the ioctl result). Every handler takes the
module mutex (`.data+0`) and stores its address in the call context for mi_common to release [I: same idiom as the other MI
modules], then calls the IMPL function with the unpacked payload.

| nr | cmd | payload | handler → IMPL |
|---|---|---|---|
| 0x00 | `0x6900` (_IO, arg 0) | — | `MI_VDISP_IMPL_Init()` |
| 0x01 | `0x6901` (_IO, arg 0) | — | `MI_VDISP_IMPL_Exit()` |
| 0x02 | `0x40046902` | u32 dev | `OpenDevice(dev)` |
| 0x03 | `0x40046903` | u32 dev | `CloseDevice(dev)` |
| 0x04 | `0x40286904` | u32 dev, u32 port, `OutputPortAttr` (0x20) at +8 | `SetOutputPortAttr` |
| 0x05 | `0xC0286905` | same, attr returned | `GetOutputPortAttr` |
| 0x06 | `0x401C6906` | u32 dev, u32 chn, `InputChnAttr` (0x14) at +8 | `SetInputChannelAttr` |
| 0x07 | `0xC01C6907` | same, attr returned | `GetInputChannelAttr` |
| 0x08 | `0x40086908` | u32 dev, u32 chn | `EnableInputChannel` |
| 0x09 | `0x40086909` | u32 dev, u32 chn | `DisableInputChannel` |
| 0x0A | `0x4004690A` | u32 dev | `StartDev` |
| 0x0B | `0x4004690B` | u32 dev | `StopDev` |

Per-process hooks of the MI_DEVICE object: `mi_vdisp_process_init` only logs; **`mi_vdisp_process_exit` calls
`MI_VDISP_IMPL_Exit()`** — the exit (close) of any process that opened `/dev/mi_vdisp` tears down the whole module, all
devices included.

Structures (layout [R] from the IMPL accesses and the log formats; names [I]):
```c
typedef struct {            /* MI_VDISP_OutputPortAttr_t, 0x20 bytes */
    u32 u32BgColor;         /* +0x00 "cl": fill colour of uncovered areas, packed YUV (§4.4) */
    u32 ePixelFormat;       /* +0x04 only 0 (YUV422 YUYV) or 0x0B (YUV420 semi-planar NV12) accepted, else 0xA00E2008 */
    u64 u64pts;             /* +0x08 "pts": start PTS (§4.3) */
    u32 u32FrmRate;         /* +0x10 "fr": output frame rate; 0 → 30 fps */
    u32 u32Width;           /* +0x14 must be even */
    u32 u32Height;          /* +0x18 must be even for NV12 */
    u32 pad;                /* +0x1c copied, not used */
} MI_VDISP_OutputPortAttr_t;
typedef struct {            /* MI_VDISP_InputChnAttr_t, 0x14 bytes */
    u32 u32OutX, u32OutY;   /* +0x00/+0x04 position in the output frame; x even; y even for NV12 */
    u32 u32OutWidth;        /* +0x08 even */
    u32 u32OutHeight;       /* +0x0c even for NV12 */
    u32 s32IsFreeRun;       /* +0x10 "frn": 1 = do not wait for this channel when composing (§4.3, §5.1) */
} MI_VDISP_InputChnAttr_t;
```

### 2.3 libmi_vdisp.so [R]
14 exports, each a plain wrapper over §2.2 through the library opener of `mi-user-abi.md` §2.1 "Open: one fd per library,
opened lazily" (`/dev/mi_vdisp`, source file name `vdisp_api.c`): `MI_VDISP_Init / Exit` (nr 0/1), `OpenDevice /
CloseDevice (MI_VDISP_DEV)`, `SetOutputPortAttr / GetOutputPortAttr (dev, port, *attr)`, `SetInputChannelAttr /
GetInputChannelAttr (dev, chn, *attr)`, `EnableInputChannel / DisableInputChannel (dev, chn)`, `StartDev / StopDev (dev)`;
the Get calls copy the attribute back only when the ioctl returned 0.
Two more entry points: `MI_VDISP_InitDev()` = open the device (failure → returns **0x1F**, a bare number, not an MI code)
then ioctl 0; `MI_VDISP_DeInitDev()` = ioctl 1, then `close` of the library's fd and removal from the shared fd table
(`g_mi_fd_table[g_last_fd_index]` reset to −1 when it is the top entry), fd := −1; "fd is Invalid" when the fd was not
opened by this pid. Data export `sstar_mi_so_version_string` ("… build_time.20220721223321").

## 3. mi_vdisp — device, channels, control calls

### 3.1 Device structure (0x14A8 bytes at P, §2.1) [R offsets; names I]

| off | field |
|---|---|
| P+0x000 | mi_sys pass callback table (0x98 bytes, copied into the registration info; slots §4.1) |
| P+0x08C | (inside the table) pass slot 0x8C = concurrent input tasks = `vdisp_bufq_depth` × 20, set at open |
| P+0x098 + c·0xE8 | channel record C(c), c = 0…19 (below) |
| P+0x12B8 | u32 output attribute set (Set/GetOutputPortAttr, StartDev require it) |
| P+0x12C0 | `MI_VDISP_OutputPortAttr_t` copy (0x20): +0 BgColor, +4 format, **+8 u64 running output PTS** (starts at the attribute's pts, advanced by one frame interval per queued output buffer, §4.2), +0x10 frame-rate word — **always 0 in the copy**: SetOutputPortAttr stores the attribute and then writes 0 over this word, so GetOutputPortAttr and the proc dump report FrmRate 0 [R, disassembly at .text+0x36d6], +0x14 width, +0x18 height |
| P+0x12E0 | output ChnPort {0x0E, dev, 0, 0} |
| P+0x12F0 | u32 output port bound (OnBindOutputPort / OnUnBindOutputPort) |
| P+0x1308 | u64 frame interval µs = 1 000 000 / fr, or 33 333 when fr = 0 |
| P+0x1310 | u64 `SendOk`: output frames finished |
| P+0x1318 | dump-out record (§5.3): u8 on, +4 file, +8 u64 MMA cache, +0x10 its kernel VA, +0x14 dir (0x80), +0x94/+0x96 u16 w/h |
| P+0x13B0 | sub-buffer queue `bufq` (0x24 + mutex, §5.1); P+0x13CC = queued output buffers |
| P+0x13FC | previous finished output buffer head (kept for "copy from previous frame", §4.4) |
| P+0x1400 + o·0x10 | overlay record of overlay channel 16+o: +0 → its channel record, +4 list head of pending overlay buffers, +0xC atomic count |
| P+0x1440 | device state: 0 closed, 1 open, 2 started, 3 stopped, 4 closing |
| P+0x1444 | mi_sys device handle |
| P+0x1448 | atomic: live vdisp allocators of the device (must be 0 at close) |
| P+0x1450 | u64 next output deadline, µs of the monotonic clock (set by StartDev, advanced on each finished frame) |
| P+0x1478 | device mutex |
| P+0x14A0 | u32 bitmask of enabled normal channels (bit c) |

Channel record C(c) (0xE8): +0x00 x, +0x04 y, +0x08 w, +0x0C h, +0x10 IsFreeRun (the `InputChnAttr`); +0x14 state
(0 unset, 1 attribute set, 2 enabled, 3 disabled; proc names UnInit/Init/Enabled/Disabled); +0x18 own ChnPort {0x0E, dev,
c, 0}; +0x28 bound flag; +0x2C sub-buffer id in the bufq (−1 none; normal channels only); +0x30 bind record of the peer (5
words: peer ChnPort + bind attribute, from OnBindInputPort; the attribute's words 0–2 — the three capability values, always
0 — are only stored and cleared, never read [R offset search], `mi_sys-1-engine.md` §3.2 "Bind procedure"); +0x44 first output buffer id the channel takes part in (set from `sba_getnextbufid` when its attribute
changes, §4.4); +0x48 u64 `RecvOk` (input buffers accepted); +0x50 dump-in record (u8 on, +4 file, +8 u64 MMA, +0x10 VA,
+0x14 dir 0x80 bytes, +0x94/+0x96 u16 w/h).

### 3.2 Channel kinds [R]
- **Normal channels 0–15**: rectangles must lie inside the output (when the output attribute is set) and must not overlap
  another *enabled* normal channel (`vdisp_check_inputportattr`, logs every channel on a clash); x and w even, y and h even
  for NV12. Each gets a **sub-buffer** in every output buffer (§5.1): the producer writes straight into the output frame.
  Optional alignment check `_vdisp_check_input_alignment`: the producer's merged ext-conf (`mi_sys_GetInputPortBufExtConf`)
  low u16 = stride alignment in bytes; divided by bytes per pixel (NV12 1, YUYV 2) it must divide x and w — violations are
  only logged (at bind, at output bind and on every input task).
- **Overlay channels 16–19**: no overlap or output-bounds check, no sub-buffer; the producer allocates ordinary buffers and
  vdisp copies them into the output with `mi_sys_BufBlitPa` (§4.4, §4.5). They are drawn after the normal channels, in
  order 16→19, so they lie on top.

### 3.3 Control calls (IMPL) [R]
Common prologue: dev > 3 → `0xA00E2001`; module not initialised → `0xA00E2015`; then the module semaphore and the device
mutex. Error numbers used: 0x01 invalid dev, 0x03 invalid chn/port/attribute, 0x07 not configured, 0x08 unsupported format,
0x15 module not initialised, 0x16 already initialised, 0x18 input still enabled, 0x1B not stopped, 0x1F failure, 0x240
device already open, 0x241 device not open, 0x242 device not closed (Exit), 0x243 input still bound — all as `0xA00E2000 |
n` (n ≥ 0x100 lands in the "error" field as written: `0xA00E2240` etc.).
- **Init**: `CamOsTsemInit(sem, 1)`; already initialised → `0xA00E2016`. `sba_init` (two memory caches "subbufhead-cache"
  0x40 B and "bufhead-cache" 0xC0 B, an IDR for sub-buffer ids) — failure `0xA00E201F`; `vdisp_allcator_init` (cache
  "vdispallocation-cache" 0x188 B, 256-bucket hash cleared); the 4 × 20 dump-in flags and directories are saved, the whole
  `_vdisp_module` is zeroed, the flags/dirs are written back (so dump settings survive Exit/Init); 4 device mutexes;
  `region_substract_init`; flag := 1.
- **Exit**: CloseDevice(0…3); any device still not closed → `0xA00E2242`; `sba_deinit`, allocator cache destroyed, mutexes
  destroyed, `region_substract_deinit`, flag := 0, semaphore destroyed. Not initialised → 0.
- **OpenDevice(d)**: state ≠ 0 → `0xA00E2240`. Clears P+0x13FC, the deadline, the allocator count; channel records get
  ChnPort {0x0E, d, c, 0}, bound 0, sub-buffer id −1, RecvOk 0; output ChnPort {0x0E, d, 0, 0}; `sba_bufqinit`. Sets the
  pass callbacks (§4.1) and slot 0x8C, then `mi_sys_RegisterDev` with info {module 0x0E, dev d, 1 input port, 1 output
  port, 20 channels, 1 pass}, proc ops (`mi_vdisp_OnDump{Dev,Channel,InputPort,OutPort}Attr`, `OnHelp` empty) under
  `/proc/mi_modules/mi_vdisp`. Failure → `0xA00E201F`. Then `mi_sys_EnsureOutportBkRefFifoDepth(h, 0, 0, 0, 1)` (mi_sys
  keeps the last output buffer referenced — the back-reference queue of `mi_sys-1-engine.md` §4.5 "Finished output" and
  §4.7 "Depth settings"), five proc commands (§5.3), a CamOs timer (10 ms, §4.7), and for channels 0–15 a vdisp allocator
  each, installed as the input port's user allocator (`mi_sys_SetInputPortUsrAllocator(h, c, 0, 0, alloc)`: a user-space
  `MI_SYS_ChnInputPortGetBuf` on a vdisp input also gets a sub-buffer); on failure all 16 are reset to NULL and the device
  unregistered. State := 1.
- **SetOutputPortAttr(d, 0, a)**: device open; every enabled channel must fit inside the new w × h (`0xA00E2003`); format 0
  or 0x0B else `0xA00E2008`; w even; h even for NV12. Re-asserts back-reference depth 1, stores the attribute (rate word
  cleared, §3.1), interval, P+0x12B8 := 1; if dump-out is on and has no file yet, opens it.
- **GetOutputPortAttr**: needs P+0x12B8 (`0xA00E2007`).
- **SetInputChannelAttr(d, c, a)**: c ≤ 19; device open (`0xA00E2241`); with the output set: rectangle non-empty and inside
  the output; normal channels: no overlap, alignment (`0xA00E2003` "Attr illegal"); x, w even; y, h even for NV12. Stores
  the attribute; normal channels (re)register their sub-buffer (`sba_registersubbuf(bufq, x, y, w, h, oldId, &C)` — failure
  `0xA00E201F`) and set C+0x44 = `sba_getnextbufid`; state 0 → 1.
- **GetInputChannelAttr**: state 0 → `0xA00E2007`.
- **EnableInputChannel(d, c)**: state 0 → `0xA00E2007`; re-checks the rectangle against the output and the other channels;
  `sba_enable(id, 1)` for a normal channel; `mi_sys_EnableChannel(h, c)`, `mi_sys_EnableInputPort(h, c, 0)`; first enable:
  overlay → its overlay record gets the channel pointer, an empty list and count 0; normal → bit c in P+0x14A0; state := 2;
  opens the dump-in file if dump-in is armed for the channel.
- **DisableInputChannel(d, c)**: `mi_sys_DisableChannel`, `mi_sys_DisableInputPort`, `sba_enable(id, 0)`; if it was enabled:
  overlay → record pointer cleared (BUG if it pointed elsewhere), pending overlay nodes freed; state := 3 and **the whole
  enabled-normal mask P+0x14A0 is set to 0** (not only bit c); dump-in file closed.
- **StartDev(d)**: needs open + output attribute (`0xA00E2007`); first start → `sba_enable(-1, 1)` (bufq accepts
  allocations), state := 2; `mi_sys_EnableChannel` for every enabled channel and for channel 0, `mi_sys_EnableOutputPort(h,
  0, 0)`; deadline := now (µs).
- **StopDev(d)**: device open (`0xA00E2241`); unless already stopped: `mi_sys_DisableChannel` for enabled channels and
  channel 0; if started: `sba_enable(-1, 0)`, dump-out closed, state := 3; `mi_sys_DisableOutputPort(h, 0, 0)`;
  `CamOsTimerDeleteSync` of the device timer. A later StartDev does **not** re-arm the timer (only OpenDevice adds it).
- **CloseDevice(d)**: state 0 → returns −1 (0xFFFFFFFF) with "Device already stopped". Calls StopDev (outside the
  semaphore); then the state must be 1 or 3 (`0xA00E201B`); any channel enabled → `0xA00E2018`, bound → `0xA00E2243`;
  channel states := 0; state := 4; waits (1 ms polls) until every queued output buffer has no sub-buffer in flight and
  dequeues/frees them; resets the 16 input user allocators, unregisters the sub-buffers, `mi_sys_UnRegisterDev` (mutex
  released around it), `sba_bufqdestroy` (BUG if anything is left), BUG if allocators are still alive; state := 0.

## 4. mi_vdisp — frame flow

### 4.1 Pass callbacks (one pass) [R slots; slot names from the module's symbols]
`fn(cmdq, task, …, usrData = P)`. Result codes as `mi_sys-1-engine.md` §4.1 step 7 (enqueue: 0 queued, 1 done at once, 2
retry, 3 drop) and §7.2 "Task completion" (check-output: 0 still running, other → dequeue).

| slot | function | behaviour |
|---|---|---|
| 0x00 | `vdisp_OnPreProcessInputTask` | if fewer than `vdisp_bufq_depth` output buffers are queued → `mi_sys_NotifyWakeupISREvent(h, 0)` (lets the ASYNC thread queue another); normal channel → alignment re-check (log only); returns 0 |
| 0x08 | `vdisp_EnqueueInputTask` | §4.4 |
| 0x0C | `vdisp_BarrierInputTask` | returns 0 |
| 0x10 | `vdisp_OnPrepareAsyncOutputTaskMetaInfo` | returns 0 |
| 0x14 | `vdisp_EnqueueAsyncOutputTask` | §4.2 |
| 0x18 | `vdisp_BarrierAsyncOutputTask` | returns 0 |
| 0x1C | `vdisp_CheckInputTaskStatus` | overlay channel: 1 when its buffer has been copied (node removed, RecvOk +1), else 0; normal: 1 |
| 0x20 | `vdisp_DequeueInputTask` | returns 1 |
| 0x24 | `vdisp_CheckOutputTaskStatus` | §4.3 |
| 0x28 | `vdisp_DequeueOutputTask` | §4.4 |
| 0x2C | `vdisp_OnPollingAsyncOutputTaskConfig` | §4.2 (its presence gives the pass an ASYNC thread) |
| 0x30 | `vdisp_NeedIndividualWorkThread` | returns 1 |
| 0x40 / 0x44 | bind capability (input / output) | capability mask (+0x0C) := 1 → FRAME_BASE only; the three u16 left 0; no NULL check and no port check (fill code: `mi_sys-1-engine.md` §3.3) |
| 0x4C | `vdisp_OnBindInputPort` (after linking) | §4.6 |
| 0x50 | `vdisp_OnUnBindInputPort` (before) | §4.6 |
| 0x5C | `vdisp_OnBindOutputPort` (after) | alignment check of channels 0–15 (log only); fails (−1) when the device state is 0 or 2 (closed or **started**), else P+0x12F0 := 1 |
| 0x60 | `vdisp_OnUnBindOutputPort` (before) | same state test; P+0x12F0 := 0 |
| 0x68 | `vdisp_OnOutputPortBufRelease` | returns 0 |

The four bind callbacks (0x4C, 0x50, 0x5C, 0x60) check the port: module 0x0E, dev < 4, chn < 20, port 0, else −1.

### 4.2 Output buffers: the ASYNC thread [R]
1. **Poll** (slot 0x2C, config record `cfg`): closing → 1 (nothing). Under the device mutex: not started, or `bufq.count ≥
   vdisp_bufq_depth` → 1. Else `cfg+4` := 1 and `cfg+0x18` = `MI_SYS_BufConf_t` {eBufType 1 (FRAME), u16 width/height =
   output w/h, scan mode 0, eFormat = output format} → 0: mi_sys allocates one output frame from its pool.
2. **Enqueue** (slot 0x14, output task, its `MI_SYS_BufInfo_t *` at task+0x34): closing → 3. A bufhead (0xC0, §5.1) is built
   from the buffer: FRAME, format 0/0x0B, tile/compress/scan/field all 0 (else BUG); its PTS = the running output PTS
   (P+0x12C8), tolerance 500 000 µs; the running PTS advances by the frame interval. `sba_queuebuf` appends it (marks it
   FRESH, gives it the next buffer id, and hands it to every channel waiting for a target); its filled-region and masks are
   cleared → 0. No buffer / wrong type → 2.

At depth N (= `vdisp_bufq_depth`, 4 as loaded) up to N output frames are open at once, so producers can run up to N frames
ahead.

### 4.3 When an output frame is finished (slot 0x24) [R]
`vdisp_CheckOutputTaskStatus(…, outTask, …, P)`: closing → 3. Under the device mutex: the queue head must be this task's
buffer (same phys address) else 0. Then (the rate word of the stored attribute is always 0, §3.1, so only this branch
runs):
- if not every enabled normal channel has filled its rectangle (head filled-mask ≠ P+0x14A0): when the deadline P+0x1450 is
  more than 10 ms away and still in the future → 0 (wait); otherwise, if a sub-buffer of this frame is still being written
  → 0; else the frame is marked COMPLETE and SKIP (`sba_completebuf` + `sba_skipallocfromfirst`: channels that did not
  deliver in time move their target to the next frame).
- `sba_isbufcomplete`: no sub-buffer in flight **and not FRESH** → deadline := now + interval, `NotifyWakeupISREvent`, 1
  (finished; mi_sys calls DequeueOutputTask). Else 0.

FRESH is cleared only when a producer is handed a sub-buffer of that frame (`MI_VDISP_IMPL_InitBufInfoFromSubBuf`) [R: the
only two writes of the bit], so **a frame no normal-channel producer has allocated into is never finished** — the output
runs at the pace of its inputs, capped at the configured rate by the deadline [I from these paths].
Deadline arithmetic: µs from `CamOsGetMonotonicTime` (s·10⁹ + ns, divided by 1000).

### 4.4 Inputs and composition [R]
**Normal channels — zero copy.** The producer bound to channel c allocates its output buffers through vdisp's allocator
(installed at bind, §4.6): each allocation is a sub-buffer — a `MI_SYS_BufInfo_t` describing the channel's rectangle inside
the oldest open output frame (§5.2). When the producer releases it (the allocation's OnRelease), `sba_completeChn` sets bit
c in the frame's filled-mask and, unless the producer flagged the buffer `bDrop` (BufInfo +0x1C), adds the rectangle to the
frame's covered region and bit c to its drawn-mask; all enabled channels filled → COMPLETE.
`vdisp_EnqueueInputTask` (task: +0 chn, +0xC BufInfo*) for a normal channel: channel not enabled → 3; the buffer must be a
vdisp allocation (hash lookup by phys address, magic 0x56445000 "\0PDV") else warning "get buf not from our allocator" → 3;
its frame gets the rectangle and bit c, RecvOk +1; all enabled channels filled → COMPLETE + wake; → 1 (done).

**Overlay channels — copy.** EnqueueInputTask for 16–19: no overlay record → 3; else a 0x10-byte node {BufInfo*, copied=0}
is appended to the overlay's list, count +1 → 0 (the input stays in flight until CheckInputTaskStatus sees it copied).

**Finishing a frame** (`vdisp_DequeueOutputTask`, slot 0x28, Ghidra 0x124a4): a 0x980-byte scratch array of 16 copy
records (§4.5); dequeue the head (fails → "curbuf == NULL", 3); closing → free it, 3. Otherwise:
1. for each enabled normal channel that did not draw into this frame (bit clear in filled-mask, or set but not in
   drawn-mask): skip it while its C+0x44 ≥ this frame's id (the channel's rectangle changed after this frame was queued —
   C+0x44 is then cleared once passed); else, if a previous frame exists with the same format, queue a copy of the
   channel's rectangle **from the previous output frame** (`vdisp_addtocopy_prev`) and add it to the covered region;
2. run the copies (`vdisp_blit_copy`);
3. overlays (`vdisp_addtocopy`, 16→19): the first pending buffer not yet copied → copy min(its size, the channel's w/h) to
   the channel's x/y, wake the pass, mark copied; no new buffer → copy the channel's rectangle from the previous frame; in
   both cases the rectangle is added to the covered region;
4. `vdisp_fill_buf_not_ready`: covered region empty → fill the whole frame with BgColor; else fill every rectangle of
   (frame − covered region) (`SubtractRegion`) with BgColor;
5. `SendOk` +1, region emptied, dump-out record written (§5.3); the previous head is freed and this head becomes the
   "previous frame" (P+0x13FC) → 1.

Fills use `mi_sys_BufFillPa(frame, BgColor, rect)`; vdisp passes BgColor unchanged, the colour packing per format is that
of ioctl 0x23 BufFillPa (`mi_sys-2-interfaces.md` §3.2 "The table"). Copies: §4.5. `vdisp_blit_finish`, `vdisp_blit_open`
and `vdisp_blit_close` are empty.

### 4.5 Copy records and the `mi_sys_BufBlitPa` calls [R]
<!-- ev:mi_vdisp-mi_shadow.4.5 -->
Every copy — a channel's rectangle from the previous frame, or an overlay buffer — is one copy record ("copyinfo", 0x98
bytes, zeroed before it is built), and `vdisp_blit_copy(rec)` (Ghidra 0x16a48, file 0x6a3e…0x6aa8) turns it into one
`mi_sys_BufBlitPa` call per plane: a byte copy (pixel format 9, I8) of a rows × row-length block — YUYV one plane of
w·2 bytes × h; NV12 two planes (Y w × h, UV w × h/2).

| off | field |
|---|---|
| +0x00 | u32 `plane_num` (1 = YUYV, 2 = NV12; 3 planes fit) |
| +0x08 + 0x30·p | plane p: u32 source kernel VA (0 when the source is not mapped) — written, **not read** by the blitter |
| +0x10 + 0x30·p | u64 source physical address of the rectangle's first byte |
| +0x18 + 0x30·p | u32 source stride (bytes) |
| +0x1c + 0x30·p | u32 destination kernel VA + offset (0 when the destination plane has no VA) — written, not read |
| +0x20 + 0x30·p | u64 destination physical address of the rectangle's first byte |
| +0x28 + 0x30·p | u32 destination stride |
| +0x2c + 0x30·p | u32 row length in **bytes** (only the low 16 bits are used) |
| +0x30 + 0x30·p | u32 row count (low 16 bits used) |

Plane p's words at +0x30·p+0x00/+0x04 coincide with plane p−1's row count and an unused word; +0x04 of the record and
+0x94…+0x97 are never written.

**The calls.** For p = 0 … plane_num − 1, one call `mi_sys_BufBlitPa(&dstFrame, &dstRect, &srcFrame, &srcRect)` with two
`MI_SYS_FrameData_t` built on the stack (layout: ioctl 0x24 BufBlitPa, `mi_sys-2-interfaces.md` §3.2 "The table"):
- src frame (sp+0x18) and dst frame (sp+0x80): +0x04 pixel format **9** (I8, one byte per pixel), +0x18 u16 width = row
  length in bytes, +0x1a u16 height = row count, +0x28 u64 phys[0] = the plane's address, +0x40 stride[0] = the plane's
  stride. **No other field is written** — tile mode (+0x00), compress mode (+0x08), scan/field, phys[1–2], stride[1–2] are
  whatever the stack held [R: no memset, no other stores]. [I: harmless as long as mi_sys reads the compress mode only for
  the Bayer-16 group, ioctl 0x24 row.]
- src rect (sp+8) and dst rect (sp+0x10): `{u16 x 0, u16 y 0, u16 w = row length, u16 h = row count}` — the rectangle's
  position is already folded into the physical addresses.
- Return: the OR of all calls' results, truncated to u16. `vdisp_DequeueOutputTask` keeps only the last record's value and
  the overlay path passes it to `vdisp_blit_finish` (empty) — copy errors are ignored.

**Who builds records, with which values.** Plane descriptors of the output frames come from the bufhead (§5.1: plane p at
+0x28 + 0x18·p `{VA, u64 phys, stride, pixel-offset fn}`); `addr(x, y)` = plane phys + `fn(plane, x, y)` (YUYV `y·stride +
2x`; NV12 Y `y·stride + x`, UV `(y/2)·stride + x`).

| builder | plane_num | source | destination | row length × rows |
|---|---|---|---|---|
| previous-frame copy (`vdisp_addtocopy_prev` 0x10b04, assert text `copyinfo->plane_num`; one record per normal channel, up to 16 records in the 0x980-byte array of `DequeueOutputTask`) | from the previous head's +0x24; then BUG unless it matches the format (0 → 1, 0xB → 2), other formats BUG | previous output frame at the channel's (x, y) | current frame at the same (x, y) | YUYV `2w × h`; NV12 plane 0 `w × h`, plane 1 `w × h/2` (w, h = channel rectangle C+0x08/+0x0C) |
| overlay copy (`vdisp_addtocopy`, inlined in 0x10c40; one record per pending overlay buffer, on the stack) | from the **source** buffer's pixel format (BufInfo +0x24): 0 → 1, 0xB → 2, other → BUG | the overlay input buffer from its top-left: VA BufInfo +0x3c + 4p, phys +0x48 + 8p, stride +0x60 + 4p | current frame at the overlay channel's (x, y) | `min(src w, chn w)` (×2 for YUYV) × `min(src h, chn h)`; NV12 plane 1 `w × h/2` |

The overlay builder takes the plane count from the source and the plane descriptors from the output head without comparing
the two formats [R]; an NV12 overlay on a YUYV output would call plane 1's offset function of a head that has only one plane
(a slot the YUYV head does not set) [I: from the code path only].

### 4.6 Binding [R]
- **OnBindInputPort** (after linking; port checked): module initialised, device open; BUG if already bound. Normal
  channel: a new vdisp allocator for the channel, set as the **custom allocator of the producer's output port**
  (`mi_sys_SetPeerOutputPortCusAllocator(h, c, 0, 0, alloc)`; failure → release, −1). The bind record (peer ChnPort + bind
  attribute, 5 words) is stored at C+0x30, bound := 1, alignment check, and an already enabled channel's sub-buffer is
  enabled. Overlay channels keep the producer's own allocator.
- **OnUnBindInputPort** (before): not bound → −1; normal channel: `SetPeerOutputPortCusAllocator(…, NULL)` (failure → −1);
  bind record and bound flag cleared.
- Producers must therefore produce FRAME_BASE buffers through mi_sys allocation (the bind capability is FRAME_BASE only,
  §4.1).

### 4.7 Timer [R]
One CamOs timer per device, armed at OpenDevice with 10 ms; `timer_function` calls `mi_sys_NotifyWakeupISREvent(h, 0)` and
re-arms itself for 10 ms. It wakes the pass so CheckOutputTaskStatus re-evaluates deadlines even without input. Deleted by
StopDev and not re-armed by StartDev (§3.3).

## 5. mi_vdisp — sub-buffer allocator, vdisp allocator, debug

### 5.1 Sub-buffer allocator `sba_*` (per device, `bufq` at P+0x13B0) [R]
**bufq** (0x24 + mutex at +0x24): +0x00 list of output-buffer heads in frame order; +0x08 list of sub-buffer infos (one per
registered normal channel); +0x10 "pending" list (infos with no target frame); +0x18 u32 enabled (`sba_enable(-1, x)`);
+0x1C count of queued heads; +0x20 next buffer id.

**bufhead** (0xC0, cache "bufhead-cache"): +0x04 queue node; +0x0C list of sub-buffers handed out of this frame and not yet
released ("unfinished"); +0x14 flags bit0 SKIP (no more allocation), bit1 COMPLETE, bit2 FRESH (queued, nobody has
allocated yet); +0x18 pixel format; +0x1C/+0x20 width/height; +0x24 plane count (YUYV 1, NV12 2); +0x28 + 0x18·p plane p
{+0 kernel VA, +8 u64 phys (MIU), +0x10 stride, +0x14 pixel-offset function: YUYV `y·stride + 2x`, NV12 Y `y·stride + x`,
NV12 UV `(y/2)·stride + x`, each BUG on odd x (and odd y for NV12)}; +0x70 u64 frame PTS; +0x78 PTS tolerance (500 000);
+0x7C buffer id; +0x80 covered region (kernel `region_substract` clip region, 0x20 bytes); +0xA0 filled-mask; +0xA4
drawn-mask; +0xA8 allocated-mask ("tainted" in the proc dump); +0xAC → the output `MI_SYS_BufInfo_t`.

**sub-buffer info** (0x58, CamOsMemAlloc): +0x00 list node; +0x08 current target head (0 = pending); +0x0C list of its
allocations; +0x14 pending-list node; +0x1C id (IDR, 0…99); +0x20 enabled; +0x24 x, +0x28 y, +0x2C w, +0x30 h; +0x38 u64
PTS offset (initial 0x7FFFFFFF = unset); +0x40 zombie flag; +0x48 u64 allocation count; +0x50 early-frame counter; +0x54 →
channel record.

**sub-buffer** (0x40, cache "subbufhead-cache"): +0x00 type (3 = the channel's first allocation, 1 = later); +0x04 node in
the head's list; +0x0C → head; +0x10 node in the info's list; +0x18 → info.

Operations ("unused" = no call site in the module):
- `sba_registersubbuf(q, x, y, w, h, oldId, chn)`: w or h 0 → −1; new info with a fresh IDR id; with `oldId` ≥ 0 the old
  info's target, enable flag and PTS offset are inherited, the old id is removed and the old info freed (or, with
  allocations outstanding, marked zombie — freed by the last `sba_free`). A new info targets the first non-SKIP head, or
  waits on the pending list. Returns the id.
- `sba_unregistersubbuf(q, id)`, `sba_enable(q, id, on)` (id < 0: the whole queue; returns the previous value),
  `sba_getnextbufid(q, id)` (id of the info's target head, or the next id to be issued).
- `sba_alloc(q, id, pts, freerun)`: queue disabled, id < 0, unknown id, no target or info disabled → NULL. Unless freerun
  (the channel's IsFreeRun, bit 0): PTS pacing against the target head — offset unset → offset := headPts − pts; else t =
  pts + offset: t ahead of the head by more than the tolerance → resynchronise the offset; t behind by more than the
  tolerance: if behind by < 1 s and fewer than 31 consecutive times → refuse (NULL, counter +1: the producer's frame is too
  early for this output frame), else resynchronise; success clears the counter. Then a sub-buffer is linked into the head
  and the info, head allocated-mask |= bit chn, and the info's target moves to the next head without SKIP (none → pending
  list); allocation count +1.
- `sba_free(q, sub)`: unlink; frees a zombie info with no allocations left.
- `sba_queuebuf(q, head)`: FRESH, id := next id++, append, every pending info gets this head, count +1.
- `sba_dequeuebuf(q)`: first head, only if it has no sub-buffer in flight; infos still targeting it move on; count −1.
  `sba_dequeuebufnocheck` does the same without the in-flight test (unused).
- `sba_completebuf` (COMPLETE), `sba_isbufempty` (nothing in flight), `sba_isbufcomplete` (nothing in flight and not FRESH),
  `sba_completeChn(q, sub, chn, rect, dropped)` (§4.4; COMPLETE when the filled-mask equals the enabled-normal mask),
  `sba_skipallocfromfirst` (first head, if not FRESH: SKIP, infos move on), `sba_skipalloctaintedbuf` (SKIP on the queued
  heads from the first on, stopping at the last head if nothing is in flight in it; nothing when one head is queued;
  unused), `sba_firstbuf`, `sba_firstincompletebuf` (unused), `sba_findsubbuf` (unused), `sba_isEmptyBufAvailable` (unused),
  `sba_dumpbufQ` (proc dump), `sba_dumpsubbuf` (unused), `sba_dump_bufQ`, `sba_dump_bufQ1` (empty), `sba_newbufhead` /
  `sba_freebufhead`, `sba_subbufvalidwidth/height` (the part of the rectangle inside the frame: w if x+w ≤ frame width,
  else frame width − x; 0 if x ≥ width).
- `_MI_VDISP_IMPL_CalFrameSize` (unused; same size formula as §5.2 — inlined copies are used instead).

### 5.2 vdisp allocator [R]
Built on mi_sys's generic allocator and allocation objects (`mi_sys-2-interfaces.md` §7.6).
- Allocator object (0x58): +0x00 generic allocator (`'MALC'`; ops at `.data+0xB0`: generic on_ref / on_unref,
  `vdisp_allocator_OnRelease`, `vdisp_allocator_alloc`, `vdisp_allocator_suit_bufconfig` → 0x7FFFFFFF); +0x20 0; +0x24 →
  the channel's ChnPort (C+0x18; so +4 dev, +8 chn, −8 = IsFreeRun, +0x14 = sub-buffer id); +0x28 mutex; +0x50 released;
  +0x54 live allocations. Creation increments P+0x1448, final release decrements it.
- `alloc(allocator, ?, bufConf)`: only `eBufType` FRAME; channel without sub-buffer → NULL. Object (0x188, cache
  "vdispallocation-cache"): +0x00/+0x04 hash links, +0x08 magic **0x56445000**, +0x10 generic allocation (`'ALLC'`; ops at
  `.data+0x90`: generic on_ref / on_unref, `vdisp_bufallocation_OnRelease`, `map_user`, `unmap_user`, `vmap_kern`,
  `vunmap_kern`, hook 0) — this pointer is what mi_sys gets; +0x38 its `MI_SYS_BufInfo_t`; +0x150 → allocator; +0x154 →
  sub-buffer; +0x158 the three plane phys addresses (hash key; 256 buckets, index = (phys0 · 0x9E370001) >> 24); +0x170
  kernel VA cache. Then `sba_alloc(q, id, bufConf.u64TargetPts, freerun)`.
- `MI_VDISP_IMPL_InitBufInfoFromSubBuf`: clears the frame's FRESH bit; BufInfo = {pts = target pts, eBufType 1, FrameData:
  tile/format/compress/scan/field copied from the output frame, ePhylayoutType 2 (NORMAL_FRAME_DATA), width/height = the
  valid part of the rectangle (`sba_subbufvalidwidth/height`), u32BufSize = w·2·h (YUYV) or w·h·3/2 (NV12), per plane:
  stride = output stride, phys/VA = output plane + pixel-offset(x, y)}.
- `vmap_kern` (FRAME only): `mi_sys_Vmap` of the **whole output frame** (phys0, frame size, cached), kept per allocation;
  `map_user`: `vdisp_map_usr` maps the whole output frame into the calling process (page-aligned, shared per (page, mm),
  refcounted in a 256-bucket hash), returns the frame VA + (plane0 phys − frame phys); `unmap_user` drops the reference. Both
  BUG when the frame's BufInfo is missing.
- `vdisp_bufallocation_OnRelease` (last reference): `sba_completeChn` with BufInfo `bDrop` (+0x1C) as "dropped", unhash,
  `sba_free`, free the object; frees the allocator when it was released and this was its last allocation.

### 5.3 Proc file, commands, dumps [R]
- `/proc/mi_modules/mi_vdisp/mi_vdisp<d>` (mi_sys device proc file): device part "Private Vdisp%d Info" (DevStatus
  UnInit/Init/Start/Stop for states 0–3, "Unkown" for 4, AllocCnt = P+0x1448); input-port table (PortID, PortStatus
  UnInit/Init/Enabled/Disabled, ChnX/Y/W/H, IsFreeRun, RecvOk) for the 20 channels; output table (Inited, FrmInterval,
  BgColor, PixelFmt, FrmRate, Width, Height, SendOk) and a bufQ table (bufId, per-channel tainted/unfinished bits,
  SKIP/COMPLETE/FRESH flags). Channel dump and help are empty.
- Commands (`mi_sys_RegistCommand`, written into that proc file): `setchnstatus <chn> <0|1>` (Disable/EnableInputChannel),
  `setchnfreerun <chn> <0|1>` (Get+SetInputChannelAttr with IsFreeRun), `setdevpause <0|1>` (**1 → StartDev, 0 →
  StopDev**, inverted relative to the name), `dump_in <dev> <chn> <dir> <0|1>`, `dump_out <dev> <port> <dir> <0|1>`. Both
  dump commands always act on **device 0** whatever `<dev>` says (they pass device 0's structure).
- dump_out: opens `<dir>/dump_out_<port>_<µs>_<w>x<h>.vdisp` (`CamFsOpen` flags 0x101242, mode 0644); every finished frame
  (`vdisp_try_record`) is blitted into an MMA cache "vdisp_dump_cache" of the frame size, mapped, and its whole `u32BufSize`
  appended to the file. Off → close, unmap, free.
- dump_in: opens `<dir>/dump_in_<chn>_<µs>_<w>x<h>.vdisp` for an enabled/attributed channel and arms the record, but **no
  code writes input frames** (the record function has one caller, the output path).
- `brecordDir` keeps each channel's dump directory and on-flag across `MI_VDISP_IMPL_Init` (§3.3).

## 6. mi_shadow — load, device file, ioctls

### 6.1 Load and module state [R]
- `init_module`: `/proc/mi_modules/mi_shadow/` with `debug_level`, `debug_file`, `debug_func`, `module_version_file`
  ("Sigmastar Module mi_shadow version: project_commit.7251436 sdk_commit.80327e1 build_time.20220721223311"), then
  `MI_DEVICE_Register` → `/dev/mi_shadow`. Parameters: only the common `debug_level/file/func`. Module id **0x14** (20,
  `E_MI_MODULE_ID_SHADOW`; errors `0xA014….`). Author string "colin.hu".
- Per-process hooks: **open → `MI_SHADOW_IMPL_Init`** (once; later opens log "Already Initted"), **exit of any opener →
  `MI_SHADOW_IMPL_DeInit`**, which tears down every shadow device of every process.
- `Init`: global manager `_stDevMgr` (Ghidra 0x1787C): +0x00 device list, +0x08 IDR for buffer handles (`CamOsIdrInitEx`,
  0x800 ids), +0x30 Tsem guarding the IDR, +0x84 rwsem over the device list, +0xAC u8 initialised, +0xB0 mutex serialising
  all buffer-slot operations.
- `DeInit`: for each device: event := 0x13 (exit), wake both wait queues, deinit all slot queues (§7.3), disable enabled
  channels, free tables and the dump buffer, `mi_sys_UnRegisterDev`; then destroy the IDR, locks; flag 0.

### 6.2 ioctls of `/dev/mi_shadow` [R]
Wrapper as in `mi-user-abi.md` §3.1. Handlers marked "m" take the module mutex (`.data+0`, released by mi_common); the
blocking and per-buffer-status ones do not.

| nr | cmd | payload | handler → IMPL |
|---|---|---|---|
| 0x00 m | `0xC0186900` | +0 u32 module id, +4 dev id, +8 input ports, +0xC output ports, +0x10 channels; +0x14 out handle | `RegisterDev` (§7.1) |
| 0x01 m | `0x40046901` | u32 handle | `UnRegisterDev` |
| 0x02 m | `0xC1A06902` | +0 handle, +4 chn, +8 port, +0x10 out `MI_SYS_BufInfo_t` (0x108), +0x118 out buffer handle, +0x120 in `MI_SYS_BufConf_t` (0x78), +0x198 out u8 (never written by the kernel) | `GetOutputPortBuf` (§7.4) |
| 0x03 m | `0xC1206903` | +0 handle, +4 chn, +8 port, +0x10 out BufInfo, +0x118 out buffer handle | `GetInputPortBuf` |
| 0x04 m | `0x40086904` | handle, buffer handle | `FinishBuf` = DoneBuf(done) |
| 0x05 m | `0x40086905` | handle, buffer handle | `RewindBuf` = DoneBuf(retry) |
| 0x06 | `0x40086906` | handle, u32 timeout ms | `WaitOnInputTaskAvailable` |
| 0x07 | `0xC0846907` | +0 handle, +4 0, +8 out event type, +0xC out ChnPort A (4 × u32), +0x1C out ChnPort B, +0x2C out u8 argc, +0x30 eight user pointers for argv strings, +0x80 u32 timeout (ignored) | `WaitCallBack` (§7.2) |
| 0x08 | `0x40086908` | handle, s32 result | `SetCallbackResult` |
| 0x09 | `0x40106909` | +0 handle, +4 chn, +8 port, +0xC u8 enable, +0xD u8 whole channel, +0xE u8 input port (else output) | `SetChannelStatus` (§7.5) |
| 0x0A | `0xC014690A` | handle, chn, port, ext-conf (8 bytes) | `mi_sys_SetInputPortBufExtConf(h, chn, 0, port, &ext)` |
| 0x0B | `0xC014690B` | same | `mi_sys_SetOutputPortBufExtConf(h, chn, 0, port, &ext)` |
| 0x0C m | `0xC018690C` | +0 handle (0x18 bytes sent) | `ExitEventThread` |
| 0x0D m | `0xC008690D` | handle, user pointer | `CopyDumpBufFromUser`: copies **0x8000 bytes** from the pointer into the device's dump buffer (BUG on NULL) |

The "handle" is the mi_sys device handle returned by RegisterDev (a kernel pointer value) — user space addresses its device
by it. Buffer handles come from the IDR: input buffers 0x40–0x3FF, output buffers 0x400–0x7FF.

## 7. mi_shadow — kernel internals

### 7.1 Device record and registration [R]
`RegisterDev(info, &handle)` (module not initialised or NULL info → `0xA014201F`): under the list write lock, a 0xC8-byte
record R and a channel table of `channels × 0x6C`; every channel gets an input and an output slot queue (§7.3). mi_sys
registration info = {module id, dev id, input ports, output ports, channels} **as given by the caller** (any module id can
be claimed), 1 pass, pass slot 0x8C = **2** concurrent tasks, and callbacks:

| slot | callback | slot | callback |
|---|---|---|---|
| 0x00 | `_MI_SHADOW_OnPreProcessInputTask` | 0x40 / 0x44 | bind capability: mask := **0x15** (FRAME_BASE \| REALTIME \| HW_RING, bit values 1 / 4 / 0x10 of `mi_sys-1-engine.md` §3.1 "Bind types") |
| 0x08 / 0x0C | `EnqueueInputTask` / `BarrierInputTask` (→ 0) | 0x48 | `OnBindInputPort` (before) → event 0 |
| 0x14 / 0x18 | `EnqueueAsyncOutputTask` / `BarrierAsyncOutputTask` (→ 0) | 0x50 | `OnUnBindInputPort` (before) → event 2 |
| 0x1C / 0x20 | `CheckInputTaskStatus` / `DequeueInputTask` | 0x58 | `OnBindOutputPort` (before) → event 1 |
| 0x24 / 0x28 | `CheckOutputTaskStatus` / `DequeueOutputTask` | 0x60 | `OnUnBindOutputPort` (before) → event 3 |
| 0x2C | `OnPollingAsyncOutputTaskConfig` | 0x7C | `OnSupportOutputTaskReorder` → 1 ("may output tasks complete out of order"; the slot's name comes from this module) |

proc ops = the five dump callbacks (§7.2); a 32 KiB dump buffer (CamOsMemAlloc, BUG on failure); commands (with their
argument counts) `trace` (2), `log_level` (2), `sem_check` (2), `auto_check` (2), `en_workmode` (3), `en_chn` (3), `dump_all`
(2), `dump_image` (4). mi_sys returning NULL → queues and tables freed, `0xA014201F`. Else R is filled and linked, handle
returned.

R (0xC8): +0x00 u32 trace flag, +0x04 trace level; +0x08 list node; +0x10 module id, +0x14 dev id, +0x18 input ports, +0x1C
output ports, +0x20 channels; +0x24 mi_sys handle; wait queues +0x28 "stWaitCallback", +0x40 "stWaitCallbackResult", +0x58
"stWaitOnInputTask"; +0x70 callback result; +0x74 event pending; +0x78 result ready; +0x7C input-available flag; +0x80 dump
buffer; +0x84…+0x94 copy of module/dev/in/out/channels; +0x98 event ChnPort A, +0xA8 ChnPort B (4 words each); +0xB8 u8
argc, +0xBC argv; +0xC0 channel table; +0xC4 last event type.

Channel entry (0x6C): +0x00 u8 enabled, +0x04 input-port slot queue, +0x38 output-port slot queue (one port per direction
per channel is served — the queues ignore the port id; the port id only reaches mi_sys enable/disable and ext-conf).

`UnRegisterDev(h)`: unknown → `0xA014201F`; unlink, event := 0x13, wake both queues, deinit slot queues, disable enabled
channels, free, `mi_sys_UnRegisterDev` → 0. `ExitEventThread(h)`: event := 0x13, pending := 1, result-ready := 1, wake both
queues, result-ready := 0 → 0.

### 7.2 Events: mi_sys callback → user thread → result [R]
`_MI_SHADOW_IMPL_SetWakeUpDevCallback(R, type, portA, portB, argc, argv)`: R+0xC4 := type, result := `0xA014201F`; ports
copied as words 0, 1, 2, 4 of the kernel's 5-word port record (module, dev, chn, port; word 3 = pass is dropped); argc/argv
stored when argv ≠ NULL; pending := 1; wake "stWaitCallback". Type 0x13 → result 0, return. Otherwise wait for the result up
to **10 jiffies** (100 ms at HZ 100 [I: HZ]); not answered → result `0xA014201F` (log "timeout!!!"); result-ready := 0;
return the result to mi_sys.

Event types [R from the callers]:

| type | raised by |
|---|---|
| 0 / 1 / 2 / 3 | OnBindInputPort / OnBindOutputPort / OnUnBindInputPort / OnUnBindOutputPort (A = own port, B = peer) |
| 4 | not raised |
| 5 / 6 / 7 / 8 / 9 | OnDumpDevAttr / OnDumpChannelAttr / OnDumpInputPortAttr / OnDumpOutPortAttr / OnHelp |
| 10 | not raised (`trace` is handled in the kernel, §7.6) |
| 0x0B / 0x0C / 0x0D / 0x0E / 0x0F / 0x10 / 0x11 | proc commands `log_level` / `sem_check` / `auto_check` / `en_workmode` / `en_chn` / `dump_all` / `dump_image` (argc/argv of the command) |
| 0x13 | exit |

`WaitCallBack(h, …)`: not initialised → `0xA014201F`; unknown h → type 0x13, `0xA014201F`. If nothing pending, sleep on
"stWaitCallback" (interruptible) up to **100 jiffies** — the payload timeout is ignored; timeout → returns **0x14** without
writing the type. Event: if it is 0x13 → type 0x13, 0 (pending stays set, so every later wait returns 0x13 at once). Else
pending := 0; type written; types 0–3 → ports A/B; types 10–17 → argc and each argv string copied (`strlen` bytes, no
terminator) to the user pointers; → 0.

`SetCallbackResult(h, r)`: result := r, result-ready := 1, wake "stWaitCallbackResult" (no check that a callback is waiting).

Dump events: OnDumpDevAttr raises event 5 and then prints the dump buffer through the proc printer (BUG if it is NULL);
OnHelp raises 9 and prints the dump buffer line by line (`_MI_SHADOW_StrReadLine`, lines ≤ 255 chars, split at `\n` or 0xFF)
with `CamOsPrintString`; channel / input-port / output-port dumps only raise their event and print nothing. The user side
fills the dump buffer with ioctl 0x0D before answering (§8.2).

### 7.3 Slot queues ("Stl") [R]
Each port queue (0x34): +0x00 u8 initialised, +0x04 CamOsTsem, +0x24 list of slots, +0x2C slot array, +0x30 slot count —
**2 slots** per queue (double buffering). Slot (0x98): +0x00 status, +0x08 `MI_SYS_BufConf_t` (0x78), +0x80 kernel
`MI_SYS_BufInfo_t *`, +0x84 user buffer handle, +0x88 list node, +0x90 → queue.

Statuses (names from the module's table): 0 min, **1 unconfig**, **2 configed**, **3 prepared**, **4 using**, **5 done**,
**6 retry**, **7 polling**, 8 max. Allowed transitions (`_MI_SHADOW_BufQueue_SetStl`, else `0xA014201F` "status:%d
stl_status:%d"): →1 from 5/6 (conf and buffer cleared); →2 from 1 (conf copied if given); →3 from 2 or 7 (buffer attached;
without a buffer the conf is cleared); →4 from 3; →5 from 3/4; →6 from 4; →7 from 2; same status → no-op. A slot taken out
of the list by `GetStl` (first slot with a status, or the slot holding a given BufInfo) is put back at the tail by the next
transition. `Reset`: slots in "using" are unmapped and handle-freed; prepared/using/done/retry → done, the others →
unconfig; success only if every slot ended unconfig. `Deinit`: like the unmap part of Reset, then frees the array.

### 7.4 Data flow [R]
**Input port (the shadow device consumes frames from a bound producer):**
1. user `GetInputPortBuf(h, chn, port)`: channel must be enabled (else `0xA014201F`); if the queue has no slot that is
   configed/polling/prepared, an unconfig slot becomes **configed** (= "ready to receive") and the call returns
   **`0xA014201C`** after waking the pass; with a prepared slot: the buffer is mapped into the caller
   (`mi_sys_MmapBufToMultiUserVirAddr(h, buf, &token, 1, 1)`), the token gets an IDR handle (input range), BufInfo (0x108,
   with the user VAs) is copied out, slot → **using** → 0. Map or IDR failure → slot → done, `0xA014201F`.
2. mi_sys MAIN thread: `OnPreProcessInputTask` → 2 (drop) if the channel is disabled, 1 (not now) if no configed slot, else
   0 (and BufConf+4 := 0). `EnqueueInputTask`: no buffer → 3; a configed slot → **prepared** with the task's buffer,
   input-available := 1, wake "stWaitOnInputTask" → 0 (queued); none → 2 (retry).
3. user `WaitOnInputTaskAvailable(h, ms)`: returns at once if any enabled channel has a prepared input slot; else waits on
   "stWaitOnInputTask" up to `ms`; **always returns 0** (the timeout code −4 it computes is discarded).
4. user `FinishBuf(h, handle)` / `RewindBuf`: the handle's token is removed from the IDR and unmapped (unknown handle → 0,
   "Channel/Port Has been Disable"); the slot holding that buffer is searched in every channel (output queue first, then
   input) and set **done** (Finish) or **retry** (Rewind); pass woken.
5. `CheckInputTaskStatus`: done → 1, retry → 2, else 0. `DequeueInputTask`: done → 1, retry → 2, else 3; slot → unconfig.

**Output port (the shadow device produces frames):**
1. user `GetOutputPortBuf(h, chn, port, &bufConf)` (NULL conf → `0xA014201F`): no slot configed/polling/prepared → an
   unconfig slot becomes **configed** with the conf, wake, `0xA014201C`; configed or polling present but nothing prepared →
   wake, `0xA014201C`; prepared → mapped as above (output handle range), **using** → 0.
2. mi_sys ASYNC thread: `OnPollingAsyncOutputTaskConfig(cfg)`: chn < channels and enabled; a polling slot is offered again,
   else a configed slot; its conf → `cfg+0x18`, `cfg+4` := 1, slot → **polling** → 0; else `0xA014201C`.
   `EnqueueAsyncOutputTask(task)`: channel enabled, a polling slot, buffer at task+0x34 (missing → slot back to unconfig, 3).
   **Size check**: conf type RAW (0) → the buffer's size (RAW: BufInfo+0x30, FRAME: +0x6C) must equal conf+0x18
   (`stRawCfg.u32Size`); conf type **FRAME (1) → compared with 0, so every FRAME request is dropped** ("Drop Stl For
   Unsutiable BufSize", slot → unconfig, 3) [R, disassembly .text+0x1282–0x12FE]; other conf types: RAW/FRAME buffers
   dropped, other buffer types accepted unchecked. Accepted → **prepared** → 0; no polling slot → 2.
3. user fills the buffer and calls `FinishBuf` (→ done) or `RewindBuf` (→ retry).
4. `CheckOutputTaskStatus`: done → 1, retry → 3, else 0. `DequeueOutputTask`: done → 1, else 3; slot → unconfig.

### 7.5 Channel and port enable [R]
`SetChannelStatus(h, chn, port, enable, isChannel, isInput)`: unknown h → 0 for a disable, `0xA014201F` for an enable.
- channel: enable → `mi_sys_EnableChannel`; disable → both queues are Reset first — if a buffer is still in flight → wake
  pass, **`0xA0142012`** (busy) and the channel stays enabled; else `mi_sys_DisableChannel`; the enabled byte follows the
  request.
- input / output port: enable → `mi_sys_EnableInputPort(h, chn, 0, port)` / `EnableOutputPort`; disable → that queue is
  Reset; in flight → `0xA0142012`; else `mi_sys_Disable…Port`.

Ext-conf ioctls (0x0A / 0x0B) pass straight to mi_sys with pass 0.

### 7.6 Proc command `trace` [R]
<!-- ev:mi_vdisp-mi_shadow.7.6 -->
`trace <level> [<flag>]`: level < 8 → R+4, flag < 2 → R+0; level 7 dumps every enabled channel's slots ("Input"/"Output",
index, status name, BufInfo pointer, in list yes/no). The other commands are forwarded to user space as events 0x0B–0x11;
their meaning is the registering library's. libmi_vdf, the only registrant, implements all seven [R]; what each does is in `sdk-orphan-libs.md` §7.7.

## 8. libmi_shadow.so — user API and event thread [R]

### 8.1 Exports
Opener as in `mi-user-abi.md` §2.1 (`/dev/mi_shadow`, source file name `shadow_api.c`). One static registration per process
(Ghidra .bss 0x130FC: +0 handle, +4… callback table, +0x50/+0x54 user data, +0x58 thread, +0x5C running flag).

| export | ioctl / behaviour |
|---|---|
| `MI_SHADOW_RegisterDev(MI_SHADOW_RegisterDevParams_t *p, MI_SHADOW_HANDLE *ph)` | NULL → `0xA014201F`; ioctl 0 with p's first 5 words; failure → `*ph = 0x12345687`, returns the code; success → `*ph` = handle, callbacks and user data copied to the static table, and **only if at least one callback is non-NULL** a thread `MI_SHADOW_WaitCallbackThread` (CamOsThreadCreate) is started |
| `MI_SHADOW_UnRegisterDev(h)` | running := 0; ioctl 0x0C (wakes the thread with exit); join the thread if one was started; ioctl 1 |
| `MI_SHADOW_GetOutputPortBuf(h, chn, port, MI_SYS_BufConf_t *conf, MI_BOOL *pbBlocked, MI_SYS_BufInfo_t *info, MI_SYS_BUF_HANDLE *ph)` | conf or info NULL → `0xA014201F`; ioctl 2; 0 → info and *ph copied, `*pbBlocked` (if given) = payload byte +0x198 (always 0); non-zero → info filled with 0x1A bytes |
| `MI_SHADOW_GetInputPortBuf(h, chn, port, info, ph)` | info NULL → `0xA014201F`; ioctl 3; failure → info filled with 0xF1 bytes |
| `MI_SHADOW_FinishBuf(h, hBuf)`, `MI_SHADOW_RewindBuf(h, hBuf)` | ioctl 4 / 5 |
| `MI_SHADOW_WaitOnInputTaskAvailable(h, ms)` | ioctl 6 |
| `MI_SHADOW_EnableChannel(h, chn)` / `DisableChannel` | ioctl 9 {enable 1/0, whole channel 1}; Disable **retries every 5 ms while the result is `0xA0142012`** (busy), warning at the 100th retry |
| `MI_SHADOW_EnableOutputPort(h, chn, port)` / `DisableOutputPort` | ioctl 9 {1/0, 0, 0}; Disable on busy sleeps 5 ms once and returns the busy code (no retry) |
| `MI_SHADOW_EnableInputPort(h, chn, port)` / `DisableInputPort` | ioctl 9 {1/0, 0, 1}; Disable retries every 5 ms while busy, warning after 1000 tries |
| `MI_SHADOW_SetInputPortBufExtConf(h, chn, port, ext)` / `SetOutputPortBufExtConf` | ioctl 0x0A / 0x0B (NULL ext → `0xA014201F`) |
| `_MI_SHADOW_SetCallbackResult(h, result)` | ioctl 8 (exported with a leading underscore) |
| `MI_SHADOW_InitDev()` | opens the device; failure → 0x1F, else 0 (no ioctl) |
| `MI_SHADOW_DeInitDev()` | closes the library fd as `MI_VDISP_DeInitDev` does (§2.3), no ioctl |
| `sstar_mi_so_version_string` | "… build_time.20220721223314" |

### 8.2 Registration parameters and the event thread
<!-- ev:mi_vdisp-mi_shadow.8.2 -->
`MI_SHADOW_RegisterDevParams_t` (0x5C bytes; field names of +0x00–0x10 from the kernel log "eModuleId, u32DevId,
u32DevChnNum, u32InputPor…"; the rest [I] from the event numbers they serve, +0x24…+0x50 confirmed by libmi_vdf's use
[R]):

| off | field | event |
|---|---|---|
| +0x00 / +0x04 | eModuleId / u32DevId | — |
| +0x08 / +0x0C / +0x10 | u32InputPortNum / u32OutputPortNum / u32DevChnNum | — |
| +0x14 … +0x20 | OnBindInputPort, OnBindOutputPort, OnUnBindInputPort, OnUnBindOutputPort: `s32 fn(ChnPort *self, ChnPort *peer, void *bindUsr)` | 0–3 |
| +0x24 … +0x34 | OnDumpDevAttr, OnDumpChannelAttr, OnDumpInputPortAttr, OnDumpOutPortAttr, OnHelp: `s32 fn(h, 0, void *dumpUsr)` | 5–9 |
| +0x38 … +0x50 | commands log_level, sem_check, auto_check, en_workmode, en_chn, dump_all, dump_image: `s32 fn(h, 0, u8 argc, char **argv, 0)` | 0x0B–0x11 |
| +0x54 | bindUsr | |
| +0x58 | dumpUsr — must point to a ≥ 32 KiB text buffer: after a dump callback the library copies 0x8000 bytes from it into the kernel (ioctl 0x0D) | |

Events 4 and 10 map to unused table words (always NULL → result 0).

Thread loop: while running: payload {handle, argv → a static 0x100-byte buffer cut into 8 × 32-byte strings, timeout 500}
(the buffer and the payload words +4…+0x7F are zeroed each round); ioctl 7; type 0x13 → log "Shadow Callback Exit", end;
type 0x14 → repeat; type < 4 → bind callback; 4–9 → dump callback, then ioctl 0x0D; 10–0x11 → command callback; a NULL
callback answers 0; every answer goes back with `_MI_SHADOW_SetCallbackResult(handle, r)`.

The kernel signals a wait timeout only through the ioctl **return value** 0x14 and leaves the type field untouched (§7.2),
while the library tests the type field, which it zeroed — so every idle second the thread dispatches a bogus event 0
(`OnBindInputPort` with zeroed ports) and posts its result [R: both sides]; the posted result stays "ready" in the kernel
and would be taken as the answer to the next real event without waiting [I from §7.2].

Only one device with callbacks per process (one static table).

## 9. Dependencies

### 9.1 mi_sys exports used only by these two modules [R call sites]
Behaviour of each export: `mi_sys-2-interfaces.md` §6.1 "Imported exports" and §7.6; the back-reference queue:
`mi_sys-1-engine.md` §4.7.

| export | user | how it is called here |
|---|---|---|
| `generic_allocator_init / on_ref / on_unref` | vdisp | allocator object init with ops at `.data+0xB0`; on_ref/on_unref are the ops' first two members |
| `generic_allocation_init / on_ref / on_unref` | vdisp | allocation object (+0x10 of the 0x188 object) with ops at `.data+0x90` |
| `mi_sys_SetInputPortUsrAllocator(h, chn, 0, 0, alloc)` | vdisp | at OpenDevice for channels 0–15 (NULL at close / failure) |
| `mi_sys_SetPeerOutputPortCusAllocator(h, chn, 0, 0, alloc)` | vdisp | at OnBindInputPort for channels 0–15 (NULL at unbind) |
| `mi_sys_EnsureOutportBkRefFifoDepth(h, 0, 0, 0, 1)` | vdisp | at OpenDevice and every SetOutputPortAttr |
| `mi_sys_GetInputPortBufExtConf(h, chn, 0, 0, &ext)` | vdisp | alignment check: low u16 of ext word 0 = stride alignment |
| `mi_sys_MmapBufToMultiUserVirAddr(h, bufInfo, &token, 1, 1)` | shadow | the 4th argument (meaning not known) and the 5th (bFlush) are both 1 at every call |
| `mi_sys_UnmapBufToMultiUserVirAddr(h, token, &bufInfo, 1)` | shadow | 4th argument 1; returns the BufInfo; BUG in the module if it differs from the one mapped (GetPortBuf error path) |

Other mi_sys imports are shared with the modules the camera's chain uses: vdisp — registration, Enable/Disable
channel/input/output port, `NotifyWakeupISREvent`, `MMA_Alloc/Free`, `Vmap/UnVmap`, `BufBlitPa`, `BufFillPa`,
`RegistCommand`; shadow — registration, enable/disable, `Set{Input,Output}PortBufExtConf`, `NotifyWakeupISREvent`,
`RegistCommand`.

### 9.2 Kernel built-ins and mi_common [R]
<!-- ev:mi_vdisp-mi_shadow.9.2 -->
- `InitClipRgn`, `AddClipRect`, `SubtractRegion`, `EmptyClipRgn`, `IsEmptyClipRgn`, `region_substract_init/deinit` (vdisp
  only): built-in `drivers/sstar/mma_heap/region_substract.c` (`kernel-platform.md` §2.2) — a
  MiniGUI-style rectangle-list region (`CLIPRGN`, `RECT {left, top, right, bottom}` with exclusive right/bottom; vdisp passes
  `{x, y, x+w, y+h}`; the result list of `SubtractRegion` is walked through `CLIPRECT.next` at +0x10).
- CamOs (both), CamFs (`CamFsOpen/Write/Close`, vdisp dumps), `__wake_up` / `prepare_to_wait_event` / `schedule_timeout` /
  `__msecs_to_jiffies` (shadow waits), `proc_*`, `printk`, `sprintf`, `str*`.
- mi_common: `MI_DEVICE_Register/Unregister/GetProcModuleDir`, debug proc ops, `MI_COMMON_Print/RealGetSelfDir`.
