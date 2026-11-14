# mi_venc — mi_venc.ko and libmi_venc.so
<!-- ev:mi_venc -->

The video encoder of the MI layer. `mi_venc.ko` is an **mi_sys device driver**: its ioctls validate and store the channel
configuration, and the encoding is driven from the mi_sys task callbacks (enqueue, check, dequeue) run by the device's work
thread, with the engines reached through mhal (`MHAL_MFE_*` H.264, `MHAL_MHE_*` H.265 — both the Wave5 codec —, and
`MHAL_JPE_*` JPEG). The encoded stream does not go through mi_sys output buffers: it is written into an mi_sys **ring pool**
per channel, which the user library reads through its own ioctls and a mapping of the ring. `libmi_venc.so` is the user
side.

Shared facts owned elsewhere: the user-side ioctl wrapper, payload packing, return codes, the MI error-code format and its
low-byte numbers, module ids, the fd create counts and `/dev/mi_poll` — `mi-user-abi.md`; kernel-side dispatch
(`MI_DEVICE_Register` / `MI_DEVICE_Ioctl`), per-module procfs and debug parameters, mi_sys exports, private heaps, MIU
protect — `mi_sys-2-interfaces.md`; the registration contract (info record, pass callback table), bind types and bind
capability, task results, the sideband message, SRAM and the ring pools — `mi_sys-1-engine.md`; the mhal side of every
`MHAL_VENC` SetParam / GetParam / SetDevConfig / GetDevConfig id, the Wave5 host library, codec clocks and throughput
limits — `mhal-vcodec.md` §7, §5, §2.2, §6; the JPE engine — `mhal-scl-jpe-cmdq.md` §4; `chagall.bin` — `chagall-fw.md`; the app's calls and
values — `prog_uvc.md` §3.2, §7.4.

Tags: **[R]** read from the binary (decompile, disassembly, `.data` / `.rodata`) or from a cited file; **[S]** stated by a
source (SDK header, vendor text); **[I]** inference.

## 0. Notation and working files
<!-- ev:mi_venc.0 -->

- Module: `/config/modules/4.9.227/mi_venc.ko` — 432 KB file, `.text` 0x2ccdc bytes (184 KB), 308
  functions, full ELF symbols, no debug info [R]. Library: `libmi_venc.so`, 78 exports (`mi-user-abi.md` §1.1) [R].
- The 37 symbol-less static functions are named here `S<own __func__ string>`, e.g. `S_MI_VENC_CreateMhalDevice` [R].
- **Addresses** are `.text` offset + 0x10028 unless written "section offset"; in the same numbering `.data` starts at
  0x4da64 and `.bss` at 0x4f0d0. Module parameters sit at `.LANCHOR0` +0x6d0 `use_ring_ref`, +0x6e4 `discard_dqp`, +0x6e8
  `frm_lost_win`, +0x6ec `discard_max_cnt`, +0x6f0 `reencode_max_cnt`, +0x6f4 `reencode_dqp` [R].
- `SMR` = `_stModRes` (`.bss`+0x2d8, 0x3cb0 bytes); `CHIPIDX` = the chip index word at `.data`+0x30c; "dev" = the device
  record (§2.1), "chn" = the channel state (§2.2), "task" = a task slot (§7.1), "buf" = the mi_sys input buffer
  `MI_SYS_BufInfo_t`.
- Error codes are written `0xA0022xxx` (module 2 = VENC; the low byte is the MI common error number, `mi-user-abi.md` §4.3).
- MI struct names and layouts: the M6 header `mi_venc_datatype.h`
  and, for the library, the public skyipc_tiramisu `mi_venc_datatype.h` [S]; `MI_BOOL` = u8, `MI_PHY` = u64 (`mi_sys_datatype.h`,
  `mi_common_datatype.h`) [S].
- mhal structs passed by SetParam / GetParam are `{u32 size, u32 version, fields…}`; "[n]" = 32-bit word n of such a struct
  (word 0 size, word 1 version); "MI+x" = byte offset in the caller's MI struct.

## 1. The module

### 1.1 Load, init, exit [R]
- `init_module`: creates `/proc/mi_modules/mi_venc/` with `debug_level` (`.data`+0x178), `debug_file`, `debug_func`,
  `module_version_file` ("Sigmastar Module mi_venc version: project_commit.7251436 sdk_commit.80327e1
  build_time.20220721223317"), then `MI_DEVICE_Register` of the device struct at `.data`+0x17c (name "venc", ioctl table
  `.data`+0x24 with 0x55 entries, `mi_venc_process_init` / `mi_venc_process_exit`) — the character device `/dev/mi_venc`.
  `cleanup_module` reverses it. `depends=mi_sys,mi_common`.
- `process_init` = `MI_VENC_IMPL_Init`: if `_stModRes[0]` is set → "Already Initialized" and return; else zero the whole
  `_stModRes` (0x3cb0 B), compare `Chip_Get_Platform_Name()` case-insensitively against the `.rodata` table at 0xe2c
  `{0 "I5", 1 "I6", 2 "I2M", 3 "I6E", 4 "I6b0", 5 "M6", 6 "P3"}` and store the index in `CHIPIDX` (no match → 7); set
  `_stModRes[0]` = 1. No hardware access. **This camera is M6 = 5.**
- `process_exit` = `MI_VENC_IMPL_DeInit`: `_stModRes[1]` = 1 (exiting), `MI_VENC_IMPL_DeInitDev(0..9)`, clear the inited
  flag. It is **not refcounted**: closing any fd of `/dev/mi_venc` stops and destroys every channel and device. libmi_venc
  closes its fd only when its device create count returns to 0 (`mi-user-abi.md` §2.2).

### 1.2 Imports and exports [R]
- mhal: `MHAL_MFE_*` (15), `MHAL_MHE_*` (15), `MHAL_JPE_*` (16), reached through the dispatch layer of §1.5.
- mi_sys: device registration, channel / port enable, `mi_sys_RingPool_*` (13 functions), `mi_sys_PrivateDevChnHeapAlloc/Free`,
  `mi_sys_Vmap/UnVmap/VFlushInvCache`, `mi_sys_GetChnBufInfo`, `mi_sys_GetChnMMAConf`, `mi_sys_GetModuleMMaHeapName`,
  `mi_sys_IsMmuMma`, `mi_sys_setMiuProtectMMa`, `mi_sys_GetCurPts`, `mi_sys_NotifyWakeupISREvent`, `mi_sys_RecodeTimeStamp`,
  `mi_sys_SetInputPortBufExtConf`, `mi_sys_SetInputPortSidebandMsg`, `MI_SYSCFG_GetMmapInfo`; mi_common:
  `MI_COMMON_CreatePoll`, `MI_COMMON_PollRelease`, `MI_COMMON_WakeUpClient`.
- CamOs: Tsem, atomics, timers, memory, time; kernel: `request_threaded_irq`, `call_usermodehelper`,
  `filp_open` / `vfs_read` / `vfs_write` / `vfs_fsync` (dumps, §12.4).
- Exports: none.

### 1.3 Devices and engines per chip [R, `.rodata` 0x18b4, read by `InitDev` / `DestroyDevice` as `0x3e780 + CHIPIDX·0x7c`]
Six records of 0x7c bytes: `u32 chip`, then 10 slots `{u8 enabled (u32 slot), u32 engine type, u32 dev id}`. Engine type =
index into `astMHalDevConfig` (§1.5): 0 "mfe" (H.264), 1 "mhe" (H.265), 2 "jpeg" (JPE).

| chip (CHIPIDX) | slot 0 | slot 1 | slot 2 | slots 3–9 |
|---|---|---|---|---|
| I5 (0), I6 (1), I6E (3), I6b0 (4), **M6 (5)** | mfe → dev 0 | mhe → dev 0 | jpeg → dev 8 | empty |
| I2M (2) | jpeg → dev 8 | — | — | empty |
| P3 (6) | — | — | — | — |

- On M6, **device 0 holds MFE and MHE (H.264 and H.265), device 8 holds JPE** — the app uses VENC dev 0 for H.264 / H.265
  and dev 8 for MJPEG (`prog_uvc.md` §3.2).
- **P3 has no record**: index 6 reads 0x7c bytes past the end of the table, i.e. the following `__func__` strings
  ("_MI_VENC_StopMiDevice", …) [R]. The "enabled" bytes are then non-zero letters and the dev-id words are ASCII, so no
  dev id 0–9 matches and InitDev on P3 creates no engine, but still registers the proc commands and returns 0 [I: a missing
  table row, harmless only because no dev id matches]. An unknown chip (7) is refused by InitDev (`CHIPIDX > 6` →
  0xA0022008).
- Dev ids 1–7 and 9 are never enabled on any chip; a CreateChn on them fails in InitDev with 0xA0022007 [R].

### 1.4 Per-device callback table [R, `.rodata` 0x45c, 10 × 0x30 B, indexed by dev id; filled for devs 0, 1, 8, 9]

| dev | +0 | +4 capacity | +8 IRQ handler (chips < 5) | +0xc event callback (SetDevConfig 0x8003001c) | +0x10 reset callback (0x80030021) | +0x14…+0x2c MMA callbacks (SetDevConfig 0x80030014) |
|---|---|---|---|---|---|---|
| 0 | 0 | 0xa21fe | NULL | `_VENC_ISR_CB_H26x_0` | `_MI_VENC_ResetCB` | `_MI_VENC_IMPL_MiSys{Alloc, Free, Map, UnMap, VFlushInvCache, GetModuleMMaHeapName, SetMiuProtectMMa}` |
| 1 | 1 | 0xa21fe | NULL | `_VENC_ISR_CB_H26x_1` | `_MI_VENC_ResetCB_H26x_1` | `…Alloc_H26x_1`, `…Free_H26x_1`, Map, UnMap, Flush, `…HeapName_H26x_1`, MiuProtect |
| 8 | 0 | 0x77880 | NULL | `VENC_ISR_JPEG_0` | NULL | NULL ×4, Map, UnMap, Flush |
| 9 | 1 | 0x77880 | NULL | `VENC_ISR_JPEG_1` | NULL | NULL ×4, Map, UnMap, Flush |

Readers [R]: `S_MI_VENC_CreateMhalDevice` passes `+0x14…+0x2c` with SetDevConfig 0x80030014 (H.26x, dev < 2, chip > 2),
`+0xc` / `+0x10` with 0x8003001c / 0x80030021 (chip > 3), and `+8` to `MI_VENC_RequestIrq` = `request_threaded_irq(irq,
handler = +8, thread_fn NULL, IRQF_ONESHOT, "VENC-ISR", dev+(type+3)·4)` on chips < 5 — the slot is NULL in this build, so
the old-chip IRQ path would register no handler [R; I: only meaningful in the builds for those chips]. **On M6 mi_venc
requests no IRQ for any engine** (§7.5). The `+4` word (664 062 for the H.26x devices, 489 600 for JPEG) is read only by
`_MI_VENC_OnDumpDevAttr`, which prints `dev+0x18 · 100 / word` as the device load in percent (§12.2) — a
macroblock-per-second capacity [R reader at 0x3b44…0x3b62; I name]. Dev 1 and 9 (second H.26x / JPEG instance) are never
enabled by §1.3.

### 1.5 The local `MHAL_VENC_*` dispatch layer and `astMHalDevConfig` [R]
`astMHalDevConfig` (`.rodata` 0xed8) = 3 × `{ops table, name}`: `{.data+0x904, "mfe"}`, `{.data+0x948, "mhe"}`,
`{.data+0x8c0, "jpeg"}`. Each ops table is 17 words:

| slot | 0 | 1 | 2 | 3 | 4 | 5 | 6 | 7 | 8 | 9 | 10 | 11 | 12 | 13 | 14 | 15 | 16 |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| op | CreateDevice | DestroyDevice | SetDevConfig | GetDevConfig | IsrProc | CreateInstance | DestroyInstance | SetParam | GetParam | EncodeOneFrame | EncodeFrameDone | EncodeCancel | EncodeAddOutBuff | QueryBufSize | GetEngStatus | CheckInstanceStatus | BarrierOneFrame |
| mfe / mhe | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | NULL | ✓ | ✓ | ✓ | ✓ | NULL |
| jpeg | ✓ | ✓ | NULL | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |

The mi_venc functions `MHAL_VENC_<op>` (static, same names as the I2 mhal API — `MHAL_VENC_SetParam`,
`MHAL_VENC_GetParam`, …; EncodeFrameDone is called as `MHAL_VENC_EncDone`) take the device record (device ops) or the
channel (instance ops), pick `ops = dev+0x1a8+type·4` (device ops) or `chn->dev+(chn->type+0x6a)·4` (instance ops; `chn+0xf8` = engine type) and call the slot with the mhal
handle (`dev+0x18c+type·4`, or `dev+4` before the device exists) or the instance (`chn+0x238`). A NULL slot returns 0
(success, no action) — so EncodeCancel and BarrierOneFrame are no-ops for H.26x and SetDevConfig is a no-op for JPEG.
Argument errors: NULL → 0xA0022006, type > 2 → 0xA0022003, no ops / no handle → 0xA0022006; CreateInstance with a non-zero
out handle → 0xA0022002.

## 2. Module state objects

### 2.1 `_stModRes` and the device record [R]
`_stModRes[0]` inited, `[1]` exiting, `[2+dev]` device inited (10 bytes), then 10 device records of 0x610 bytes at
`SMR+0x10+dev·0x610`. `CHIPIDX` is not in it (`.data`+0x30c). Device record ("dev+"):

| off | content |
|---|---|
| +0 / +1 | u8 mi device created / running |
| +4 | dev id |
| +8 | mi_sys device handle |
| +0xc+type·4 | engine type of the slot (0 mfe, 1 mhe, 2 jpeg) |
| +0x18 | sum of the channels' MB rates (§5.2) |
| +0x1c | channel slots = 0x11 |
| +0x21 | RejectCmdq decision |
| +0x24…+0xbc | mi_sys task ops / callbacks (§3.2); +0xb0 tasks per device; +0xb4 u16 1000 (H.26x) / 100 (JPEG) [I: work-thread timeout ms]; +0xbc self |
| +0xc0…+0x100 | 17 channel-slot pointers (engine instances in use) |
| +0x104+chn·4 | `pastChnRes[33]` channel pointers |
| +0x18c+type·4 | mhal device handle; +0x198 IRQ count; +0x19c+type·4 IRQ number (+0x19c / +0x1a0 / +0x1a4) |
| +0x1a8+type·4 | mhal ops table (§1.5) |
| +0x1b4+type·100 | GetDevConfig 0x80030003 alignment table: crop x / y / w / h alignment at +0x10…+0x1c, input-stride alignment +0x20 (`dev+0x1d4`), height alignments +0x2c (FRAME_BASE), +0x38 (HW_RING YUYV), +0x50 (HW_RING 420), SCL prefer-align entries +0x40 / +0x44 / +0x58 / +0x5c (for type 0 these are `dev+0x1e0…+0x210`) |
| +0x2e0+type·0x18 | 0x8003000e: min W `+0x2e8`, min H `+0x2ec`, max W `+0x2f0`, max pixels `+0x2f4` |
| +0x328+type·0x18 | 0x8003001a capability: +8 HW_RING / "SupportRing" (`dev+0x330`), +9 "NeedPadding" (`dev+0x331`), +0xa REALTIME / "SupportImi" (`dev+0x332`), +0xc SRAM width cap (`dev+0x334`), +0x14 task cap (`dev+0x33c`), +0x15 flag |
| +0x370+type·0x18 | 0x8003001e QP limits (`dev+0x378…` read on chips < 3) |
| +0x3b8+type·0xc | 0x80030024 "type supported" (+8) |
| +0x3dc | the cmdq interface pointer of the last PreProcess (§7.2) |
| +0x3e0 | 0x80030002 answer (CMDQ size, old chips) |
| +0x3e8 | printed as "CmdqBufSize" (§12.2) |
| +0x3f0 / +0x4a0 | utilisation / fps stopwatches |
| +0x550 / +0x552 | FPS integer / hundredths (u16) |
| +0x554 / +0x556 / +0x55a / +0x55c | UtilHw / UtilMi / PeakHw / PeakMi (u16) |
| +0x560…+0x580 | engine event counters (§7.5, §12.2) |
| +0x584 | REALTIME binds (atomic) |
| +0x588 | bitmap of channels using a ring input |
| +0x590 | last event PTS (u64) |
| +0x598 | u8 task count (device cap); +0x59c tasks in flight (atomic) |
| +0x5a0, +0x5a1, +0x5d0…+0x5d5 | timeout / reset flags (§10) |
| +0x5a4 | printed as "ResetTime" (§12.2) |
| +0x5b8 | wait queue "MITimeoutWaitqueue"; +0x5d8 device semaphore |
| +0x5f8…+0x5fc | SRAM present / busy / preemption bytes (+0x5f8, +0x5f9, +0x5fa), SRAM size (+0x5fc) |
| +0x600 | list of in-flight tasks of the device |
| +0x608 / +0x60c | device max W / H |

What writes `dev+0x331`, `dev+0x3e8` and `dev+0x5a4` is not known; only the dump labels name them [R labels].

### 2.2 Channel state (0x16c0 bytes, `CamOsMemAlloc` at CreateChn, kept across Destroy / Create) [R]

| off | content |
|---|---|
| +0 / +1 / +2 | u8 created / started (StartRecvPic) / instance streaming (mhal STREAM_ON) |
| +4 | chn id; +8 channel semaphore; +0x28 device record |
| +0x30 / +0x3c | reference memory PA (u64) / size ("RefMem") |
| +0x40 / +0x48 / +0x4c | "AlMem" PA (u64) / VA / size (named by the dump label only; the writer is not known) |
| +0xc0 / +0xc4 | current / replacement bitstream ring pool; +0xc8 swapped flag, +0xc9 tasks on the old pool |
| +0xcc / +0xd0 / +0xd4 | closed records / unread records / max stream count (default 3) |
| +0xd8 | GetStream "get" count — streams held by the user (atomic) |
| +0xdc / +0xdd / +0xde / +0xdf | forced IDR / EnableIdr value / EnableIdr pending / drop current output |
| +0xe0 / +0xe4 | user-data buffer / pending length; +0xe8 / +0xec anti-tamper buffer / pending length |
| +0xf0 | alignment word sent to SCL as PREFER_ALIGN (§11) |
| +0xf4 | input pixel format code (0xb NV12 default, 0xc NV21) |
| +0xf8 | engine type (eType − 2) |
| +0xfc…+0x147 | `MI_VENC_ChnAttr_t` (0x4c): Profile +0x10c, RefNum +0x120, RC mode +0x124, RC union from +0x128 |
| +0x148…+0x17f | pending `MI_VENC_RcParam_t` |
| +0x180…+0x188 | RefParam: base, enhance, u8 bEnablePred |
| +0x214…+0x21c | pending flags: reconfig, VeAttr, RC attr, RcParam, JPEG param, VUI, Ref, Crop, custom map |
| +0x220 | ring size; +0x228 total bytes; +0x22c last frame size; +0x230 super-frame QP step |
| +0x234 | instance not yet configured; +0x236 ROI used |
| +0x238 | mhal instance handle; +0x240 STREAM_ON buffer |
| +0x318…+0x597 | 16 ROI entries of 0x28 (§6.5); +0x598 ROI bitmap |
| +0x59c…+0x60b | H.264 VUI struct (0x70) |
| +0x60c…+0x72b | H.265 VUI struct (0x120) |
| +0x72c…+0x73c | crop |
| +0x740…+0x74c | super-frame cfg: mode, IThr, PThr, BThr |
| +0x750…+0x803 | frame-lost cfg, statistics and window (§8.3) |
| +0x778 | oversize counter (shared by super-frame and frame-lost) |
| +0x804 | bByFrame |
| +0x808…+0x814 | gop, bitrate, fps num, fps den (§6.2) |
| +0x818…+0x824 | pointers to picture W, H, max W, max H inside the attr (offsets differ for JPEG) |
| +0x828 | RcPriority |
| +0x830 | u64 frame sequence |
| +0x838 / +0x83c / +0x840 | frames enqueued / dropped ("rejected": result 2 of PreProcess, Enqueue drops) / blocked (result 1 of PreProcess) |
| +0x844 / +0x848 / +0x84c | output frames / dropped / re-encoded |
| +0x850, +0x854 / +0x858 / +0x85c | ISR counters: buffer full (two words) / ring full / re-encode |
| +0x860 | MB rate |
| +0x864 | kbps over 1 s ("kbps1s") |
| +0x868 / +0x86a | fps over 1 s, integer / hundredths (u16) |
| +0x86c | kbps over 10 s; +0x870 / +0x872 fps over 10 s |
| +0x878 / +0x928 | fps stopwatches |
| +0x9d8 | attr update count |
| +0x9dc…+0x9ec | Query / GetStream / ReleaseStream / poll-ready / poll-fail counts |
| +0x9f0 | SCL line count; +0x9f2 input bound; +0x9f4 bind type (1 FRAME_BASE, 4 REALTIME = `E_MI_SYS_BIND_TYPE_REALTIME`, 8 HW_AUTOSYNC, 0x10 HW_RING); +0x9f8 ring height |
| +0x9fc | pool semaphore; +0xa1c poll handle |
| +0xa20…+0xbd8 | proc dump / drop / debug state (§12.1): +0xa20 after-bind drop counter, +0xa28 input-source mode, +0xbc9 drop-input flag |
| +0xc68 | GetStream latency record (0x48; min +0xc88, max +0xc90, avg +0xca0, last +0xc80) |
| +0xcb0, +0xcb8…+0x15ff | flow-stat enable and 33 stat records (§12.3) |
| +0x1600…+0x1608 | force-reset test |
| +0x1610…+0x1613 | AdvCustRcAttr: bEnableHistoStaticInfo, bEnableQPMap, bAbsQP, bEnableModeMap |
| +0x1618 | done count; +0x161c…+0x1624 last EncDone words |
| +0x1628…+0x1650 | the two custom maps; +0x1658 custom-map wait queue |
| +0x1670 / +0x1674 / +0x1678 | u8 task count / tasks in flight / task array |
| +0x167c | in-flight task list; +0x1684 task timeout flag |
| +0x1685 | record-minimum recompute flag: set by SetSliceSplit and `S_MI_VENC_SetRestartInterval`, cleared by `S_MI_VENC_UpdateRecordMinAllocSize` after it has recomputed the minimum |
| +0x1686 | H.26x slice-split enable (Set / GetSliceSplit) |
| +0x1688 | JPEG restart count |
| +0x168c / +0x1694 | ring minimum record size; +0x1690 GOP position |
| +0x1698…+0x16a3 | FRC tracking (§11) |
| +0x16a4…+0x16bc | last input geometry: +0x16a4 frame slot, +0x16a6 W, +0x16a8 H, +0x16aa stride, +0x16ac / +0x16ae crop W / H, +0x16b0 ring start line, +0x16b2 ring total height, +0x16b4 Y size, +0x16b8 u32BufSize, +0x16bc gradient |

The task object (0xd8) is in §7.1.

## 3. Devices

### 3.1 InitDev (ioctl 0x53, and implicit from CreateChn) [R]
`MI_VENC_IMPL_InitDev(dev, {u32MaxWidth, u32MaxHeight})`: NULL → 0xA0022006; `CHIPIDX > 6` → 0xA0022008; device already
inited (`_stModRes[2+dev]`) → 0xA0022009. Zero the 0x610-byte device record (§2.1); default max size from the module
parameters (`max_h26x_*` for dev < 2, `max_jpe_*` otherwise); the caller's size is used only when W·H ≥ 0xE1000
(921 600 = 1280×720) and W·H ≤ default W·H, else a warning ("too small" / "too big") and the default stays. CreateChn calls
InitDev implicitly with the module-parameter size when the device is not inited; the app relies on that (`prog_uvc.md` §3.2).
Then, for every enabled slot of §1.3 whose dev id matches: `S_MI_VENC_CreateMiDevice(dev, type)` for all slots first, then
`S_MI_VENC_CreateMhalDevice(dev, type)` for all slots; any failure → `S_MI_VENC_DestroyDevice(dev)` and 0xA0022007.
Finally the proc commands of §12.1 are registered (`mi_sys_RegistCommand`; table at `.data`+0x1b8, 21 entries of 0x10 B
`{name, argc, handler, .bss slot}`, the slot receives a pointer to the entry) and `_stModRes[2+dev]` = 1.

### 3.2 `S_MI_VENC_CreateMiDevice(dev, type)` — the mi_sys side, once per device [R]
 0x1b27c, own `__func__` string `_MI_VENC_CreateMiDevice`.
1. Record dev id (`+4`), engine type → `+0xc+type·4`, mhal ops table → `+0x1a8+type·4`; GetDevConfig **0x80030024**
   `{0xc, 1}` into `+0x3b8+type·0xc`; byte `+8` of the answer = 0 → "dev type %d unsupported", return 0 (slot skipped).
2. If the mi device already exists (`+0` set) return; else fill the mi_sys task ops in the record (`+0x24`…`+0xbc`):
   `OnPreProcessInputTask`, `EnqueueInputTask`, `BarrierInputTask`, `CheckInputTaskStatus`, `DequeueInputTask`,
   `CheckOutputTaskStatus`, `DequeueOutputTask`, `OnGetInputPortBindCapability`, `OnGetOutputPortBindCapability`,
   `NeedIndividualWorkThread`, `OnBindInputPort`, `OnUnBindInputPortPost`, `OnBindOutputPort`, `OnUnBindOutputPort`,
   `OnInputPortFrcChanged`, `RejectCmdq` (§7, §11), plus `OnConfigSramBuf` / `AllowPreemptionSram` and the dump callbacks
   `OnDumpDevAttr`, `OnDumpChannelAttr`, `OnDumpInputPortAttr`, `OnDumpOutPortAttr`, `OnHelp` (§12). (Slot positions in the
   pass callback table: `mi_sys-1-engine.md` §2.3.)
3. **Tasks in flight per device** `+0xb0`: `max_h26x_task` (dev < 2) or `max_jpe_task`, forced to 1 when outside 1…4; then
   GetDevConfig **0x8003001a** `{0x18, 2}` into `+0x328+type·0x18`: if its byte `+0x15` = 0 → min(param, byte `+0x14`);
   else if dev = 8 → min(param, `+0x14`)·17 capped at 4; else the param unchanged. MFE / MHE answer `+0x14` = 1, `+0x15` = 1
   (`mhal-vcodec.md` §7.9) → **H.26x: `max_h26x_task` (2)**. `+0x598` = the same count as a byte.
4. Register with mi_sys (`mi_sys_RegisterDev`): info 0x298 B `{module 2, dev, 1 device, 0x21 channels, 1 input port, …,
   thread priority = param `thread_priority` (99), MMU flag (set only for dev ≥ 2 when `mi_sys_IsMmuMma`)}`, the task ops
   above, the dump callbacks, the proc dir. Handle → `+8`; failure → 0xA0022007.
5. `+0x1c` = 0x11 (**17 channel slots per device**), atomics `+0x59c`, `+0x584` = 0, two stopwatches (`mi_venc_InitSw`):
   `+0x4a0` device fps (30-frame window, `_MI_VENC_ReportDevFps`), `+0x3f0` device utilisation (50, `_MI_VENC_ReportDevUtil`);
   semaphore `+0x5d8` (device lock), wait queue `+0x5b8` "MITimeoutWaitqueue", list head `+0x600`; `+0` / `+1` = 1 (created,
   running); `+0xb4` = 1000 for H.26x / 100 for JPEG (u16).

### 3.3 `S_MI_VENC_CreateMhalDevice(dev, type)` — the engine side [R]
 0x1b748, own `__func__` string `_MI_VENC_CreateMhalDevice`; skipped when 0x80030024 said unsupported.
1. GetDevConfig **0x80030003** `{100, 2}` → `+0x1b4+type·100` (alignment table); **0x8003000e** `{0x18, 1}` →
   `+0x2e0+type·0x18`. Device max size below the minimum or above max W / max pixels → 0xA0022003.
2. H.26x (dev < 2) on chip > 2: SetDevConfig **0x80030014** (MMA callbacks, §1.4), **0x80030016** `{0x14, 1, W, H, 0}` (max
   resolution = the device size), **0x80030019** (`fw_path`, §4).
3. `MHAL_VENC_CreateDevice` → handle `+0x18c+type·4` (failure 0xA0022001). For MFE / MHE mhal loads `chagall.bin` here
   (`mhal-vcodec.md` §7.1, §2.7).
4. Chip < 5 (old chips): GetDevConfig 0x8003001a, for H.26x also 0x8003001e, then GetDevConfig **0x80030001** (IRQ number →
   `+0x19c+type·4`; 10000 = none) and `MI_VENC_RequestIrq` (§1.4), `+0x198` = IRQ count.
   Chip ≥ 5 (**M6**): for JPE (type 2) `+0x5f8` = 0x100 (u16), `+0x5fa` = 0, `+0x5fc` = 0; GetDevConfig **0x80030023** (codec
   SRAM size) → `+0x5fc`, `+0x5f8` = 1 (u16: SRAM "present" byte 1, "busy" byte 0) [I names]; GetDevConfig 0x8003001a again
   (`+0x328…`), **0x8003001e** `{0x18, 1}` → `+0x370+type·0x18` (QP limits 48 / 12 / 31 / −32, `mhal-vcodec.md` §7.9); chip > 3:
   SetDevConfig **0x8003001c** (event callback) and **0x80030021** (reset callback) from §1.4. Any failure:
   `MHAL_VENC_DestroyDevice` and 0xA0022007.

### 3.4 `S_MI_VENC_DestroyDevice(dev)` and DeInitDev (ioctl 0x54) [R]
Called by DeInitDev, by an InitDev failure and by module DeInit.
1. Stop: for each enabled slot of the device, if running (`+1`): up to 4 rounds over the 17 channel slots (`+0xc0…`), each
   round checks that no channel is still created-and-started; `+1` = 0; a channel still active after 4 rounds → "Unable to
   stop mi dev" and 0xA0022007.
2. Per slot: free the IRQ if one was requested (`+0x19c` ≠ 0, ≠ 10000), `MHAL_VENC_DestroyDevice(+0x18c)` (failure →
   0xA0022007).
3. Per slot: if the mi device exists: deinit `+0x5d8`, `mi_sys_UnRegisterDev(+8)` (failure → 0xA0022007), then for
   `pastChnRes[0 … +0x1c−1]` (**only channels 0–16**): `MI_VENC_IMPL_PutPollData` if a poll was created, deinit the two
   channel semaphores, `CamOsMemRelease` the channel; `+0` = 0. Channel ids 17–32 (accepted by CreateChn, §5.2) are not freed
   here — their 0x16c0-byte state leaks on DeInitDev [R loop bound; I consequence].

`MI_VENC_IMPL_DeInitDev(dev)`: not inited → 0xA0022009; else DestroyDevice and clear `_stModRes[2+dev]`.

## 4. Module parameters [R; defaults read from `.data` / `.bss`]
None is passed on this camera (demo.sh, `rootfs-customer.md` §3.2), so the defaults hold.

| param (type, default) | read by | effect |
|---|---|---|
| `max_h26x_width` / `max_h26x_height` (int, 4096 / 2176), `max_jpe_width` / `max_jpe_height` (8192 / 8192) | InitDev, CreateChn | default device max size (§3.1); for H.26x also sent to mhal as SetDevConfig 0x80030016 |
| `max_h26x_task`, `max_jpe_task` (int, 2 / 2) | CreateMiDevice, CreateChn | tasks in flight per device / channel, clamped 1…4 and by the engine cap (§3.2, §7.1) |
| `use_ring_ref` (int, **1**) | StartRecvPicEx, Dequeue, SetSuperFrameCfg, proc `use_ring_ref` | sent to mhal as SetParam **0x8003000f** {0xc, 1, u8} at the first start; on M6 the id carries this flag into mhal inst+0x4c8 (the I2 enum names 0x8003000f ENABLE_IDR, which does not match this use [I: the name; R: the use]); the proc help calls it "mhal inplace mode", i.e. the reconstructed picture overwrites the reference in place [I]. With it set, oversized frames are discarded instead of re-encoded (§8.2) |
| `max_ring_pool_size` (int, 0) | `S_MI_VENC_CalChnBufsize` | when > 0 it **replaces** the bitstream ring size of every channel (§7.7) |
| `h264_height_16x` (bool, 0) | StartRecvPicEx | H.264 only: SetParam **0x80030012** {0x10, 1, 0, 1} at the first start; mhal accepts 0x80030012 without action — no effect on M6 [R both sides] |
| `frm_lost_win` (int, 6) | SetFrameLostStrategy, Dequeue | window length of §8.3 (≤ 0 → 1, no upper clamp) |
| `discard_max_cnt` (4), `discard_dqp` (4), `reencode_max_cnt` (4), `reencode_dqp` (4) | Dequeue | §8.2 |
| `discard_dqf`, `reencode_dqf` (int, 8 / 8) | — | **no reader** (no reference in `.rel.text`) |
| `mi_timeout_reset` (bool, 1), `jpeg_timeout_reset` (bool, 1) | CheckInputTaskStatus | §10.1 |
| `use_anti_tamper` (bool, 0) | Dequeue | §9 |
| `fw_path` (string, empty) | CreateMhalDevice | sent with SetDevConfig 0x80030019 for H.26x (dev < 2, chip > 2); mhal uses it as the `chagall.bin` path when non-empty; mhal tries this path first, then its own module parameter `drv_codec40_common.fw_load_path`, then `/config/venc_fw/chagall.bin` (`mhal-vcodec.md` §2.7) |
| `mock_in` (string, empty) | OnDumpDevAttr | only printed in the device dump; **no I/O code uses it** |
| `mock_out` (string, empty) | — | **no reader** |
| `thread_priority` (int, 99, `.data`+0x894) | CreateMiDevice | priority of the device work thread, passed in the mi_sys register info |
| `debug_level` (uint), `debug_file`, `debug_func` (string) | everywhere | logging only (`mi_sys-2-interfaces.md` §1.6) |

## 5. ioctls

### 5.1 Table and common checks [R]
<!-- ev:mi_venc.5.1 -->
85 entries (`.data`+0x24), nr 0x15 empty. Every handler takes the module mutex;
`MI_VENC_IOCTL_x` unpacks `{dev, chn, payload}` (user-side packing: §13.2) and calls `MI_VENC_IMPL_x`, which checks in this
order NULL → 0xA0022006, dev > 9 → 0xA0022001, chn > 32 → 0xA0022002, channel not created → 0xA0022005, then takes the
channel semaphore `chn+8`. "Direct" = mhal SetParam / GetParam is called at once (also on a running channel); "pending" =
stored in the channel and applied by the next Enqueue (§7.3 step 8). mhal failure → 0xA002201F unless noted. **Bold** = used
by prog_uvc (`prog_uvc.md` §3.2, §7.4); 9 / 0xa and 0x36 / 0x37 are used by libmi_iqserver's remote tuning (`-q`).

| nr | name | § | nr | name | § |
|---|---|---|---|---|---|
| 0 / 1 | Set / GetModParam | 5.5 | 0x2a–0x33 | Set / GetH265InterPred, IntraPred, Trans, Dblk, Vui | 6.4 |
| **2** | CreateChn | 5.2 | **0x34** / **0x35** | Set / GetJpegParam | 6.6 |
| **3** | DestroyChn | 5.3 | 0x36 / 0x37 | Set / GetRcParam | 6.3 |
| 4 | ResetChn | 5.3 | 0x38 / 0x39 | Set / GetRefParam | 6.6 |
| **5** / 6 | StartRecvPic / StartRecvPicEx | 5.3 | 0x3a / 0x3b | **Get** / **Set**Crop (order reversed) | 6.6 |
| **7** | StopRecvPic | 5.3 | 0x3c / 0x3d | Set / GetFrameLostStrategy | 8.3 |
| **8** | Query | 5.3 | 0x3e / 0x3f | Set / GetSuperFrameCfg | 8.1 |
| 0x9 / 0xa | Set / GetChnAttr | 5.2 | 0x40 / 0x41 | Set / GetRcPriority | 6.6 |
| **0xb** / **0xc** | GetStream / ReleaseStream | 5.4 | **0x42** | GetRingPoolHandle | 5.4 |
| 0xd | InsertUserData | 6.8 | **0x43** | RingPool_GetRingPoolInfo | 5.4 |
| **0xe** / 0xf | Set / GetMaxStreamCnt | 5.5 | 0x44 | RingPool_GetFreeSpace (stub) | 5.4 |
| **0x10** | RequestIdr | 5.5 | **0x45** | RingPool_GetHeadRecord | 5.4 |
| 0x11 | EnableIdr | 5.5 | 0x46 / 0x47 | RingPool_AppendTailRecord / UnFinishTailRecord (stubs) | 5.4 |
| 0x12 / 0x13 | Set / GetH264IdrPicId | 5.5 | **0x48** | RingPool_ReleaseHeadRecord | 5.4 |
| **0x14** | GetPollData | 5.5 | **0x49** | SetInputSourceConfig | 5.5 |
| 0x15 | — (NULL) | | 0x4a–0x4c | AllocCustomMaps, GetAsyncCustomMap, ApplyCustomMap | 6.9 |
| 0x16–0x19 | Set / GetRoiCfg, Set / GetRoiBgFrameRate | 6.5 | 0x4d–0x4f | SetAdvCustRcAttr, GetLastHistoStaticInfo, FreeCustMap | 6.9 |
| **0x1a** / **0x1b** | Set / GetH264SliceSplit | 6.4 | 0x50 | SetSmartDetInfo | 6.4 |
| 0x1c–0x27 | Set / GetH264InterPred, IntraPred, Trans, Entropy, Dblk, Vui | 6.4 | 0x51 / 0x52 | Set / GetIntraRefresh | 6.4 |
| **0x28** / 0x29 | Set / GetH265SliceSplit | 6.4 | 0x53 / 0x54 | InitDev / DeInitDev | 3.1, 3.4 |

### 5.2 CreateChn (2) and Set / GetChnAttr (9 / 0xa) [R]
- **CreateChn** `{dev, chn, attr 0x4c}`: type must be H264E 2 / H265E 3 / JPEGE 4 (else 0xA0022003); dev < 10, chn ≤ 32;
  implicit InitDev (§3.1); channel exists → 0xA0022004 (EXIST; libmi_venc maps it to 0). Then `S_MI_VENC_VerifyFps(attr)`
  (below); MB rate `chn+0x860` = MBs (16×16 of the picture) × fps num / den, added to `dev+0x18` (device load, shown by
  proc; no limit enforced); wrong RC mode for the codec ("eType and eRcMode dismatched") → refused;
  `S_MI_VENC_ValidateResolution` (picture ≤ max picture ≤ device max, minimum and alignment from the 0x80030003 /
  0x8003000e answers). Channel slot: first free entry of the device's 17-entry table `dev+0xc0…+0x100` (all used →
  0xA0022005); `MHAL_VENC_CreateInstance(dev, type, chn+0x238)`; attr copied to `chn+0xfc` (0x4c); pointers
  `chn+0x818…+0x824` to picture W / H and max W / H inside it; `chn+0x804` = bByFrame; `S_MI_VENC_ConfigVeAttr` (§6.1);
  for H.26x with bByFrame = 0 the default slice split (`_MI_VENC_EnableSliceIrq`: SetParam 0x80000009 with rows (H+15)/32
  or 0x80010008 with (H+31)/64, ll = 1); defaults: max stream count `chn+0xd4` = 3, IDR state cleared, `chn+0x234` = 1
  (instance configuration still to do at the first start), fps / util stopwatches, task slots (§7.1), smoothing windows
  (§7.7), 33 stat records (§12.3). Error after the instance exists → `MHAL_VENC_DestroyInstance`, 0xA0022010.
- Channel ids 0–32 are accepted, but **at most 17 channels per device exist at once** (slot table above), and
  DestroyDevice frees only ids 0–16 (§3.4).
- `S_MI_VENC_VerifyFps(attr)`: the source frame rate (num, den) of the RC union — CBR / VBR / ABR / AVBR (modes 1, 2, 3, 5,
  8, 9, 11) at attr words 0xd / 0xe, FIXQP and MJPEG-CBR (4, 6, 10) at 0xc / 0xd, MJPEG-FIXQP (7) at 0xb / 0xc; den 0 → 1;
  num 0 → 30; num/den > 60 → 30/1 (written back into the attr). Other modes → 0xA0022003.
- **SetChnAttr (0x9)**: the new attr must equal the stored one except, for H.26x, `u32Profile`, `u32PicWidth`,
  `u32PicHeight` and, for JPEG, the picture W / H (any other difference → 0xA0022003; eType other than 2–4 → 0xA0022008).
  A changed VeAttr part → pending `+0x214` / `+0x215` (STREAM_OFF, ConfigVeAttr, STREAM_ON at the next frame); a changed RC
  part (attr words 10–18) → pending `+0x216` (RC SetParam). VerifyFps, copy into `chn+0xfc`; if the picture size changed and
  ROI was used: the 16 ROI entries are reset and the custom map re-armed (`+0x21c`). `chn+0x9d8` (update count) += 1.
  **GetChnAttr (0xa)** returns the stored attr.

### 5.3 Start, stop, destroy, reset, query [R]
- **StartRecvPic (5) / StartRecvPicEx (6)**: the same operation — StartRecvPicEx's `s32RecvPicNum` argument is not read
  (StartRecvPic calls the same function with no third argument). While a codec reset is pending (`dev+0x5d0`) it waits on
  `MITimeoutWaitqueue` (§10). At the first start: `S_MI_VENC_ConfigInstance` (§6.1), SetParam 0x8003000f (`use_ring_ref`),
  0x80030012 (`h264_height_16x`, H.264), the input-source mode as SetParam 0x80030015 (§5.5), `S_MI_VENC_CalChnBufsize`
  (§7.7); then `MHAL_VENC_QueryBufSize` → private heap buffers (`mi_sys_PrivateDevChnHeapAlloc` + `Vmap`), **the bitstream
  ring pool** (`mi_sys_RingPool_Create`, §7.7), `mi_sys_SetInputPortBufExtConf`, `mi_sys_EnableChannel`,
  `mi_sys_EnableInputPort`, STREAM_ON (0x80030004) with the 16 ROI entries; `chn+1` = 1.
- **StopRecvPic (7)**: `mi_sys_DisableChannel` + `DisableInputPort`; if input buffers or engine tasks are still in flight it
  returns **0xA0022012 (BUSY)** at once (the library retries every 10 ms, ×100) — only during module DeInit does it wait
  itself (100 ms steps, ≤ 1 s).
- **DestroyChn (3)**: StopRecvPic if needed, `MHAL_VENC_DestroyInstance`, free buffers and ring pool,
  `MI_VENC_IMPL_PutPollData`; 0xA0022014 (CHN_NOT_STOPED) when it cannot stop.
- **ResetChn (4)**: channel started → 0xA002201F; else release every record of the current ring (`venc_release_ring_records` (0x14c8c)), and if a
  second (replacement) pool exists: drain it, `mi_sys_RingPool_Destroy` it (BUG_ON on failure), clear `chn+0xc4` / `+0xc8`.
  It does not create a pool.
- **Query (8)** `{dev, chn, ChnStat out}` (0x30): input frames left (`mi_sys_GetChnBufInfo`), stream frames / bytes from the
  ring (`mi_sys_Ringpool_GetRecordUnreadNum` / `GetHeadRecordTotalNum`).

### 5.4 The stream ioctls [R]
The stream is read by the library from the channel's ring pool (`mi_sys-1-engine.md` §10) — one or more ring records per
frame (one per slice with slice split). The library side is §13.3.
- **GetStream (0xb)** `{dev, chn, Stream 0x48, s32MilliSec}`: **bookkeeping only** — validates, increments the channel's
  "get" count (`chn+0xd8`) and timing statistics; type check → 0xA0022007. The data comes through 0x42–0x48.
- **ReleaseStream (0xc)** `{dev, chn, Stream}`: decrements the "get" count; if a replaced ring pool is empty →
  `mi_sys_RingPool_Destroy` (§7.6 step 10).
- **GetRingPoolHandle (0x42)** `{dev, chn, out handle}`: the channel's ring-pool handle (normal or replacement).
- **RingPool_GetRingPoolInfo (0x43)** `{handle, +8 u64 phys, +0x10 size}`: `mi_sys_RingPool_GetRingPoolTotalSpace`.
- **RingPool_GetHeadRecord (0x45)** (0x50): zero a 0x50-byte ring content record (`mi_sys-1-engine.md` §10.1),
  `mi_sys_RingPool_GetHeadRecordTimeout(pool = +0, chn = +0x3c, &rec, markRead 1, ms = +0x34)`, copy into the payload (field
  map §13.4), then update the per-channel latency statistics (`MI_VENC_IMPL_UpdateStatTimeInfo`, record `chn+0xc68`) with
  `+0x38` (dev) and the record's timestamp. **It waits up to the caller's timeout (40 ms in the app) while holding the
  module mutex**, so a waiting GetStream blocks every other VENC ioctl for that time.
- **RingPool_ReleaseHeadRecord (0x48)** `{handle, dev, chn}`: `mi_sys_RingPool_ReleaseHeadRecord(handle, chn, 0)`.
- **0x44 RingPool_GetFreeSpace, 0x46 RingPool_AppendTailRecord, 0x47 RingPool_UnFinishTailRecord**: stubs returning 0x1F
  (`movs r0, #31; bx lr`) — the library calls always fail.

### 5.5 Channel options, IDR, poll, input source [R]

| nr | handler | behaviour |
|---|---|---|
| 0 / 1 | Set / GetModParam | return 0xA0022008 (not supported) — no other code |
| 0xe / 0xf | Set / GetMaxStreamCnt | Set stores `chn+0xd4` unchecked (default 3; the app sets it); Get returns it. It is the back-pressure limit of §7.3 step 3 |
| 0x10 | RequestIdr `{dev, chn, bInstant}` | needs a started channel (else 0xA0022013); H.26x only (JPEG → 0xA0022008); sets the IDR request; `bInstant` → drop every stream record still in the ring |
| 0x11 | EnableIdr `{bool}` | H.26x only (else 0xA0022008); pending `chn+0xde` = 1, `chn+0xdd` = value → SetParam **0x80030007** {0xc, 1, value} at the next frame. mhal accepts 0x80030007 without action, so on M6 **EnableIdr has no effect** [R both sides] |
| 0x12 / 0x13 | Set / GetH264IdrPicId | return 0xA0022008 |
| 0x14 | GetPollData `{dev, chn, out id}` | creates once `MI_COMMON_CreatePoll(chn+0xa1c, _MI_VENC_IMPL_PollFdState, _MI_VENC_IMPL_ReleaseFd)` and returns its id (bound to `/dev/mi_poll` by the library, `mi-user-abi.md` §5); readable when `mi_sys_Ringpool_GetRecordUnreadNum` > 0; the ISR side wakes it with `MI_COMMON_WakeUpClient`. `MI_VENC_IMPL_PutPollData` (no ioctl; DestroyChn / DestroyDevice) releases it. The app opens it but waits with GetStream's timeout instead |
| 0x49 | SetInputSourceConfig `{dev, chn, eInputSrcBufferMode}` | channel started → refused; mode > 3 → 0xA0022003; modes 1 / 2 (ring one-frame / half-frame) allowed for **one channel per device** (bitmap `dev+0x588`, bit = chn), only for H.26x by-slice channels (`chn+0x804` = 0) and not with slice split enabled (`chn+0x1686`); stored in `chn+0xa28`; applied at StartRecvPicEx as SetParam 0x80030015 {0x10, 1, 2, sub-mode 1 / 2} (mode 3 HW auto sync → sub-mode 3 on M6). The app sends NORMAL_FRMBASE (0) with `-H` |

## 6. Encoder parameters → mhal

### 6.1 Instance configuration [R]
`S_MI_VENC_ConfigInstance(chn, attr)` (first StartRecvPicEx and after a reference change): RESOLUTION {0x14, 1, W, H, pixel
format: 0 NV12 / 1 NV21 by `chn+0xf4` (0xb → NV12 default, 0xc → NV21, other → warning "use NV12")} with the id of the codec
(`.rodata` 0x10c0: H.264 0x80000000, H.265 0x80010000, JPEG 0x80020001) → RC struct (§6.2) → `S_MI_VENC_SetProfile`
(H.264: u32Profile 0 → 66 Baseline, 1 → 77 Main, 2 → 100 High, others → warning; SetParam PROFILE 0x8000000a {0xc, 1, idc};
H.265: none) → `S_MI_VENC_SetRestartInterval` (JPEG, §6.7). `S_MI_VENC_ConfigVeAttr` is the same without the RC step.
`venc_apply_rc(chn, attr)` (0x1f1dc) = ConvertRc + RC SetParam only (pending `+0x216`). H.264 / H.265 channels receive only NV12 input
on this path (PreProcess drops NV21 / YUYV for them, §7.2), although ConfigInstance could program NV21.

**GOP structure and B-frames**: mi_venc never sends `u32BFrameNum` / `u32RefNum` — the B-frame count the app sets in
`MI_VENC_ChnAttr_t` has no effect, and mhal's default GOP is IPPP with one reference, so the stream has no B-frames
(`mhal-vcodec.md` §7.12); the GOP structure changes only through SetRefParam (§6.6: REF id → TSVC / LTR in mhal) [R].

### 6.2 MI RC attr → mhal RC struct (`S_MI_VENC_ConvertRc`, `S_MI_VENC_GetRcIdx`, `venc_apply_rc`) [R]
mhal RC struct = 0xb0 bytes, version 2: `[2]` mode, `[3…]` fields (ids 0x8000000c H.264 / 0x8001000a H.265 / 0x80020003
JPEG). RC id by RC mode: 1–5 → 0x8000000c, 6–7 → 0x80020003, 8–11 → 0x8001000a; it must match the codec's id (table at
`.rodata` 0x113c: H.264 0x8000000c, H.265 0x8001000a, JPEG 0x80020003), else 0xA0022003.

| MI eRcMode (hdr) | mhal mode | mhal words `[3…]` ← MI union words | channel copies `+0x808 gop, +0x80c bitrate, +0x810 / +0x814 fps num / den` |
|---|---|---|---|
| 1 H264CBR | 1 | [3] Gop, [4] StatTime, [5] FpsNum, [6] FpsDen, [7] BitRate, [8] FluctuateLevel | gop, BitRate |
| 2 H264VBR | 2 | [3] Gop, [5] FpsNum, [6] FpsDen, [7] MaxBitRate, [8] MaxQp, [9] MinQp — **StatTime not passed** ([4] = 0) | gop, MaxBitRate |
| 3 H264ABR | — | refused 0xA0022003 ("unsupported RC mode") | |
| 4 H264FIXQP | 5 | [3] Gop, [4] FpsNum, [5] FpsDen, [6] IQp, [7] PQp | gop, bitrate 0 |
| 5 H264AVBR | 3 | [3] Gop, [4] StatTime, [5] FpsNum, [6] FpsDen, [7] MaxBitRate, [8] MaxQp, [9] MinQp | gop, MaxBitRate |
| 6 MJPEGCBR | 0xc | GetParam 0x80020003 first (keeps the current Q tables in [4…0x23]); [3] BitRate, [0x24] FpsNum, [0x25] FpsDen, [0x26] max Qfactor 90, [0x27] min Qfactor 20 | bitrate, fps |
| 7 MJPEGFIXQP | 0xb | same table copy; [3] Qfactor, [0x24] FpsNum, [0x25] FpsDen | fps |
| 8 H265CBR / 9 VBR / 10 FIXQP / 11 AVBR | 6 / 7 / 10 / 8 | as the H.264 rows | |

So mhal's H.264 codes are 1 CBR, 2 VBR, 3 AVBR, 5 FIXQP (4 = UBR never sent) and H.265 6 CBR, 7 VBR, 8 AVBR, 10 FIXQP (9 =
UBR never sent). mhal does not read StatTime and FluctuateLevel (`mhal-vcodec.md` §7.4). The app uses H264CBR / H265CBR /
MJPEGFIXQP (`prog_uvc.md` §3.2). The channel copies feed the frame-lost threshold (§8.3), the RcPriority test (§8.2), the
watchdog's 8-frame-interval budget (§10.1) and the proc output.

### 6.3 RcParam (ioctl 0x36 / 0x37) → mhal RC-advanced struct [R]
**Set** (`MI_VENC_IMPL_SetRcParam`): H.26x / JPEG (type 2–4, else 0xA0022008); copies the 0x38-byte `MI_VENC_RcParam_t` to
`chn+0x148`, pending `+0x217`. At the next frame `S_MI_VENC_SetRcParam`: H.26x → GetParam then SetParam of **0x8000000d /
0x8001000b** {0x34, 1, `[2]` sub-mode, `[3…0xc]`}; `au32ThrdI`, `au32ThrdP`, `u32RowQpDelta` (MI words 0–2) and `pRcParam` are
**not passed** — the union starts at MI word 3 (MI+0xc):

| RC mode | `[2]` | mhal words ← `MI_VENC_RcParam_t` union field |
|---|---|---|
| H264CBR 1 / H265CBR 8 | 1 / 4 | [3] MaxQp, [4] MinQp, [5] IPQPDelta, [6] MaxIQp, [7] MinIQp, [8] MaxIPProp, [9] MaxISize, [10] MaxPSize |
| H264VBR 2 / H265VBR 9 | 2 / 5 | [3] IPQPDelta, [4] ChangePos, [5] MinIQp, [6] MaxIPProp, [7] MaxIQp, [8] MaxISize, [9] MaxPSize |
| H264AVBR 5 / H265AVBR 11 | 3 / 6 | [3] IPQPDelta, [4] ChangePos, [5] MinIQp, [6] MaxIPProp, [7] MaxIQp, [8] MaxISize, [9] MaxPSize, [10] MinStillPercent, [11] MaxStillQp, [12] MotionSensitivity |
| MJPEGCBR 6 | — | GetParam 0x80020003, words [0x26] ← MaxQfactor, [0x27] ← MinQfactor, SetParam 0x80020003 |
| MJPEGFIXQP 7 | — | 0xA002201F |
| FIXQP 4 / 10, ABR 3 | — | 0xA0022008 "Rc Mode is not support SetRcParam" |

In mhal's VBR / AVBR order MinIQp comes before MaxIPProp before MaxIQp, unlike the MI struct for VBR. mhal does not read the
AVBR `MaxStillQp` (`mhal-vcodec.md` §7.5).

**Get** (`MI_VENC_IMPL_GetRcParam`, 0x2689c): checks as every handler, then by codec (`chn+0xfc`):
- **H.264 / H.265** (eType 2 / 3): id 0x8000000d / 0x8001000b, struct {0x34, 1} zeroed; word [2] (sub-mode) is set from the
  RC mode **before** the GetParam: H264CBR 1 → 1, H264VBR 2 → 2, H264AVBR 5 → 3, H265CBR 8 → 4, H265VBR 9 → 5, H265AVBR 11 → 6;
  any other RC mode (FIXQP 4 / 10, ABR 3) → 0xA0022008 "RcMode:%d is not support GetRcParam" without a GetParam. GetParam
  failure → 0xA002201F. Copy-back (MI union from MI+0xc):

| RC mode | MI fields ← mhal words | not returned (caller's bytes kept) |
|---|---|---|
| CBR (1, 8) | MaxQp ← [3], MinQp ← [4], IPQPDelta ← [5], MaxIQp ← [6], MinIQp ← [7], MaxIPProp ← [8] | MaxISize, MaxPSize (Set sends them as [9], [10]) |
| VBR (2, 9) | IPQPDelta ← [3], ChangePos ← [4], MinIQp ← [5], MaxIPProp ← [6], MaxIQp ← [7] | MaxISize, MaxPSize ([8], [9] on Set) |
| AVBR (5, 11) | IPQPDelta ← [3], ChangePos ← [4], MinIQp ← [5], MaxIPProp ← [6], MaxIQp ← [7], MaxISize ← [8], MaxPSize ← [9], MinStillPercent ← [10], MaxStillQp ← [11], MotionSensitivity ← [12] | — |

  Get is the exact inverse of the Set for AVBR, and for CBR / VBR except the two frame-size limits. `au32ThrdI`,
  `au32ThrdP`, `u32RowQpDelta` and `pRcParam` are neither sent nor returned.
- **JPEG** (eType 4): RC mode must be MJPEGCBR (6), else 0xA002201F (MJPEGFIXQP included); GetParam **0x80020003** {0xb0, 2}
  (zeroed): MaxQfactor (MI+0xc) ← [0x26], MinQfactor (MI+0x10) ← [0x27].
- other eType → 0xA0022008 "ModType:%d is not support GetRcParam".

### 6.4 Per-codec parameters — direct, no state change [R]
Each Set copies the MI struct (hdr) into the mhal struct and calls SetParam at once; each Get zeroes the mhal struct, uses
the same {size, version} header and id, calls `MHAL_VENC_GetParam(chn, id, &s)` and copies back (failure → 0xA002201F).
The Gets do **no codec check** (only the argument / channel checks): a Get on a channel of another codec sends a foreign id
to that instance's mhal ops, and the outcome is mhal's (JPE answers unsupported ids with error 8 → 0xA002201F,
`mhal-scl-jpe-cmdq.md` §4.5) [R mi_venc side; I outcome].

| nr (Set / Get) | MI struct (handlers) | mhal id {size, ver} | Set: words passed | Get: copy-back (MI ← mhal) |
|---|---|---|---|---|
| 0x1a / 0x1b | H264SliceSplit (`SetH264SliceSplit` / `GetH264SliceSplit`; app: `{1, 15}`) | 0x80000009 {0x14, **2**} | [2] u8 bSplitEnable, [3] u32SliceRowCount, [4] u8 = `!chn+0x804` (by-slice); non-H.264 → 0xA0022008; on success `chn+0x1685` = 1, `chn+0x1686` = enable | bSplitEnable (u8) ← byte [2]; u32SliceRowCount ← [3]; side effect `chn+0x1686` ← bSplitEnable |
| 0x28 / 0x29 | H265SliceSplit (`SetH265SliceSplit` / `GetH265SliceSplit`; app: `{1, (H+31)/32}`) | 0x80010008 {0x14, 2} | same | same, same side effect |
| 0x1c / 0x1d | H264InterPred (`SetH264InterPred` / `GetH264InterPred`) | 0x80000008 {0x18, 1} | MI+0…+0xf copied whole into [2]…[5]: HWSize, VWSize, the 8 bools (b16x16, b16x8, …, bExtedge) as two words — all fields passed | the same 16 bytes ← [2]…[5] |
| 0x1e / 0x1f | H264IntraPred (`SetH264IntraPred` / `GetH264IntraPred`) | 0x80000007 {0x18, 1} | [2] the 4 bools (b16x16, bNxN, bConstrained, bIpcm) as one word, [3] Intra16x16Penalty, [4] Intra4x4Penalty, byte [5] bIntraPlanarPenalty — penalties passed | same layout back |
| 0x20 / 0x21 | H264Trans (`SetH264Trans` / `GetH264Trans`) | 0x80000006 {0x14, 1} | [2] IntraTransMode, [3] InterTransMode, [4] ChromaQpIndexOffset (mhal checks −12…12) | same order |
| 0x22 / 0x23 | H264Entropy (`SetH264Entropy` / `GetH264Entropy`) | 0x80000005 {0x10, 1} | [2] EntropyEncModeI, [3] EntropyEncModeP (mhal: must be equal, no CABAC in Baseline) | same |
| 0x24 / 0x25 | H264Dblk (`SetH264Dblk` / `GetH264Dblk`) | 0x80000004 {0x14, 1} | [2] idc, [3] alpha offset, [4] beta offset | same |
| 0x26 / 0x27 | H264Vui (`SetH264Vui` / `GetH264Vui`) | 0x80000003 {0x70, 2} | `MI_VENC_IMPL_SetH264Vui` builds the struct in the channel at **`chn+0x59c`** (0x70 bytes; version word written from `chn+0xfc`, = 2), the MI fields at the same body offsets, the rest 0; sets pending `+0x219` **and** calls `S_MI_VENC_SetVui` at once (SetParam 0x80000003 with `chn+0x59c`) | MI+0…+0x1a ← body bytes +0…+0x1a at the same offsets (aspect bytes and SAR, timing flags, NumUnitsInTick [5], TimeScale [6], the seven video-signal bytes); the rest of mhal's 0x68-byte body is not returned |
| 0x2a / 0x2b | H265InterPred (`SetH265InterPred` / `GetH265InterPred`) | 0x80010007 {0x28, 1} | [2] HWSize, [3] VWSize, [4] 4 bools (MI+8), [5] 3 bools (MI+0xc u16 + MI+0xe u8), [6] Inter8x8Penalty, [7] Inter16x16Penalty, [8] Inter32x32Penalty, byte [9] bExtedgeEn | the same mapping |
| 0x2c / 0x2d | H265IntraPred (`SetH265IntraPred` / `GetH265IntraPred`) | 0x80010006 {0x18, 1} | [2] the 4 bools as one word (constrained flag included), [3]–[5] penalties 32×32, 16×16, 8×8 | bIntra32x32 + bIntra16x16 (u16) ← [2] low half, bIntra8x8 ← byte 2 of [2]; penalties ← [3], [4], [5]; **`bConstrainedIntraPredFlag` (MI+3) is not returned** |
| 0x2e / 0x2f | H265Trans (`SetH265Trans` / `GetH265Trans`) | 0x80010005 {0x1c, 1} | [2] IntraTransMode, [3] InterTransMode, the chroma offset into [4], [5], [6] (Cb, Cr, common) | IntraTransMode ← [2], InterTransMode ← [3], ChromaQpIndexOffset ← **[5]** |
| 0x30 / 0x31 | H265Dblk (`SetH265Dblk` / `GetH265Dblk`) | 0x80010004 {0x14, 1} | [2] idc, [3] tc offset, [4] beta offset | same |
| 0x32 / 0x33 | H265Vui (`SetH265Vui` / `GetH265Vui`) | 0x80010003 {0x120, 1} | `MI_VENC_IMPL_SetH265Vui` builds {0x120, 1} at **`chn+0x60c`** with the placement of the Get, the rest 0, pending `+0x219` and an immediate `S_MI_VENC_SetVui` (SetParam 0x80010003) | aspect bytes and SAR ← body +0…+7; TimingInfoPresentFlag ← byte [4]; NumUnitsInTick ← [5]; TimeScale ← [6]; the seven video-signal bytes ← body +0x20…+0x26 ([10], [11]); body +0x14…+0x1f ([7]–[9]) and the rest of the 0x118-byte body are not returned |
| 0x51 / 0x52 | Set / GetIntraRefresh | 0x80030010 {0x14, 1} | byte [2] bEnable, [3] RefreshLineNum, [4] ReqIQp (mhal: rows·period < MB rows, not with TSVC) | same |
| 0x50 | SetSmartDetInfo | 0x8003001b {0x28, 2} | the SAD-ratio words of `MI_VENC_SmartDetInfo_t` (mhal sums up to 16 u16 into an AVBR still-percent override for 15 frames) | — |

- mhal accepts INTRA_PRED / INTER_PRED (0x80000007 / 0x80000008, 0x80010006 / 0x80010007) without action, and its GetParam
  mirrors the SetParam table, so these four Gets return the zeroed struct — all-zero MI fields whatever was set [I: from
  the reading of mhal in `mhal-vcodec.md` §7.3, not re-read here; mi_venc side R].
- VUI: mhal regenerates the headers from it; both codecs' blocks carry HRD and bitstream-restriction fields
  (`mhal-vcodec.md` §7.6).

### 6.5 ROI and background frame rate [R]
- **0x16 SetRoiCfg** `{u32Index < 16, bEnable, bAbsQp, s32Qp, rect}`: device must exist (0xA0022010); H.26x only; rect inside
  the picture (else 0xA0022003). Alignment check as coded: H.264 refuses the rect only when **none** of x, y, w, h is a
  multiple of 16, H.265 only when none is a multiple of 32 — the message says "all members must be" aligned, the test is
  inverted (a rect with one aligned member passes). On chips < 3 the QP is range-checked against the QP limits
  (`dev+0x378…`); on M6 that check is skipped (mhal checks: abs-QP refused, −32…31). Stored as entry `chn+0x318 + idx·0x28`
  `{0x28, 2, mode, idx, u8 enable (+0x10), u8 absQp, qp, x (+0x18), y (+0x1c), w (+0x20), h (+0x24)}`, `chn+0x598` |= 1 << idx,
  `chn+0x236` = 1. If no QP-map buffer exists yet, one is allocated ("venc-map-<chn>-0", one byte per MB of the max
  picture, filled with 0x20) and the custom-map path (§6.9) is armed; on a started channel `S_MI_VENC_SetRoiCfg(chn, idx)` →
  SetParam ROI 0x8000000b / 0x80010009 mode 0 directly. All 16 entries are re-sent after every STREAM_ON (§7.3, §10.2). A
  resolution change (SetChnAttr §5.2, PreProcess §7.2) clears the table.
- **0x17 GetRoiCfg** `{idx}`: GetParam ROI with the index → MI struct.
- **0x18 / 0x19 Set / GetRoiBgFrameRate** `{src, dst}`: ROI mode 2 SetParam (mhal: dst < src, not with a QP map → here
  0xA0022009 when the channel uses a QP map, 0xA0022013 when not started); Get reads it back through `S_MI_VENC_GetRoiCfg`.

### 6.6 Reference, crop, RC priority, JPEG parameters — stored, applied at the next frame [R]

| nr | handler | behaviour |
|---|---|---|
| 0x38 / 0x39 | Set / GetRefParam `{base, enhance, bEnablePred}` | H.26x only (else 0xA0022008); stored at `chn+0x180…0x188`, pending `+0x214` / `+0x21a`: REF SetParam + buffer re-allocation at the next frame (§7.3 step 8). Get returns the stored struct |
| 0x3b / 0x3a | SetCrop / GetCrop `{bEnable, x, y, w, h}` | device must exist (0xA0022010); disabled → full picture; alignment from the device's 0x80030003 table (`dev+0x1b4+type·100`: x `+0x10`, y `+0x14`, w `+0x18`, h `+0x1c` — on M6 x, y multiple of 16, w of 8, h of 2), a zero alignment entry → 0xA0022008, misaligned → 0xA0022003; stored `chn+0x72c…0x73c`, pending `+0x214` / `+0x21b`. Get: GetParam of the CROP id. The nr order is reversed against the usual pairing (0x3a Get, 0x3b Set), in the kernel table and the library alike |
| 0x40 / 0x41 | Set / GetRcPriority `{1 bitrate first, 2 frame bits first}` | other values → 0xA0022003; stored `chn+0x828` (read by §8.2) |
| 0x34 / 0x35 | Set / GetJpegParam (app: `Qfactor` 80) | Set: JPEG only (else 0xA0022008), stores `MI_VENC_ParamJpeg_t` (Qfactor, Y and CbCr Q tables, McuPerEcs), pending `+0x218`: at the next frame GetParam 0x80020003, replace Qfactor / tables, SetParam. Get: GetParam 0x80020003 → MI struct |

### 6.7 JPEG restart interval [R]
`S_MI_VENC_SetRestartInterval` (from ConfigVeAttr / ConfigInstance, JPEG only): `u32RestartMakerPerRowCnt & 0xffff`; enabled
(flag byte 1) only when ≠ 0, < MB columns ((W+15)/16) and the channel is not by-slice; SetParam **0x80020004** {0xc, 2, u16
count, u8 enable}; sets `chn+0x1685` = 1, `chn+0x1688` = count.

### 6.8 User data (0xd InsertUserData) [R]
`{ptr, len}`: len > 0x400 → 0xA0022003; H.26x only (else 0xA0022008); one 0x400-byte buffer per channel (`chn+0xe0`,
allocated on first use, failure 0xA002200D); `CamOsCopyFromUpperLayer` copies the user bytes from the user pointer the
payload carries; `chn+0xe4` = len; sent once with the next frame as USER_DATA SetParam (0x8000000e / 0x8001000c {0x10, 1,
buf, len}, §7.3) and then cleared. A second call before the next frame overwrites the first (only one pending entry). mhal
queues ≤ 4 user-data entries and writes them as a user-data-unregistered SEI (`mhal-vcodec.md` §7.3, §7.1).

### 6.9 Custom QP / mode maps and advanced RC (0x4a–0x4f) [R]
Two map buffers per channel: size = one byte per 16×16 MB of the max picture for H.264, `(W/64)·(H/64)·4` (one byte per
32×32) for H.265, other types → 0xA0022003. Map n: `chn+0x1628 / 0x1640` PA, `+0x1630 / 0x1648` VA, `+0x1634 / 0x164c`
applied flag, `+0x1638 / 0x1650` busy count (atomic). Buffers come from the channel private heap ("venc-map-<chn>-<n>"),
filled with 0x20.

| nr | handler | behaviour |
|---|---|---|
| 0x4a | AllocCustomMaps | allocate both maps if absent; returns both PAs and the size; allocation failure → 0xA0022007 |
| 0x4b | GetAsyncCustomMap | returns the PA of a map whose busy count is 0; if both are busy: set `+0x21c`, sleep on `chn+0x1658` (10-jiffy steps) until the encoder releases one; −1 means none (library → 0xA0022012) |
| 0x4c | ApplyCustomMap `{PA}` | mark that map applied and the other not, pending `+0x21c` → SetParam **0x80030018** {0x20, 1, map PA…} at the next frame (mhal: "plz set customer map buf without encoding"), the busy count taken until the frame is done |
| 0x4d | SetAdvCustRcAttr `{bEnableQPMap, bAbsQP, bEnableModeMap, bEnableHistoStaticInfo}` | H.26x only (JPEG → 0xA0022009); unchanged QP-map flag → 0; else SetParam ROI mode 1 {0x28, 2, 1, enableQPMap, absQP, enableModeMap}; stores `+0x1610` bEnableHistoStaticInfo, `+0x1611` bEnableQPMap, `+0x1612` bAbsQP, `+0x1613` bEnableModeMap |
| 0x4e | GetLastHistoStaticInfo | fills `MI_VENC_FrameHistoStaticInfo_t` (0x30 B) with zeros except `+0xc`, `+0x10`, `+0x14` = `chn+0x161c`, `+0x1620`, `+0x1624` = the last frame's EncDone result words `+0x3c` (intra blocks), `+0x48` (merge), `+0x44` (skip) (firmware registers 0x138 / 0x13c / 0x144, `mhal-vcodec.md` §7.10) |
| 0x4f | FreeCustMap | clear both applied flags; on a started channel set `+0x21c` and wait on `chn+0x1658` until the encoder has dropped the map, then `S_MI_VENC_FreeCustMap` (unmap, free the heap blocks) |

## 7. Encode cycle

All mi_sys callbacks run in the device's work thread (`mi_sys-1-engine.md` §6). Their first argument is mi_sys's **CMDQ
interface** pointer ("cmdq"; NULL when mi_sys runs the task without a CMDQ). PreProcess, Enqueue, Check and Dequeue take
the **device semaphore `dev+0x5d8`, then the channel semaphore `chn+8`** — one device, i.e. MFE and MHE together, works on one
callback at a time [R]. Task results (0 / 1 / 2 / 3) are mi_sys's (`mi_sys-1-engine.md` §4.1).

### 7.1 Task slots and the task object [R]
- Per channel an array of `chn+0x1670` (byte) task objects of 0xd8 bytes at `chn+0x1678`, allocated at CreateChn: count =
  min(`max_h26x_task` | `max_jpe_task` clamped to 1…4, engine cap byte `dev+0x33c+type·0x18`) — **2 for H.26x, ≤ 2 for
  JPEG** with the defaults. A slot is free when its byte `+0x10` is 0.
- [S] (SigmaStar Tiramisu MI_VENC document `venc_zh` §1.4.16 and chip notes): «Pudding/Tiramisu/Ikayaki Chip下jpe
  内部会开两个output pool» — JPE runs two output pools so a REALTIME input reaches the full frame rate; with one
  (`max_jpe_task=1`) it reaches half of the SCL output rate. Binds from SCL on Tiramisu: H.264 / H.265 FRAME_BASE or HW_RING (SCL
  and VENC read and write one frame buffer at once), not REALTIME; JPEG FRAME_BASE or REALTIME, not HW_RING; a JPEG REALTIME
  input in NV12 must be ≤ 1792 wide; a FRAME_BASE input needs at least 3 frame buffers. H.264 / H.265 up to 4096 × 2176, JPEG
  up to 8192 × 8192.
- In-flight tasks are linked twice: into the channel list `chn+0x167c` (append at the tail, links `+0` / `+4`) and the device
  list `dev+0x600` (links `+8` / `+0xc`). Dequeue always takes the **head of the channel list** — frames finish in submission
  order; the device list is used to restart the timers of the tasks behind a finished one (60 ms each) and by the
  force-reset test (§10.4).
- Counters: `chn+0x1674` and `dev+0x59c` = tasks in flight (atomic +1 after a successful EncodeOneFrame, −1 in Dequeue).
- Fields: `+0x10` u8 in use; `+0x14` status (1 encoding, 2 done (Check), 3 failed, 4 out of ring space / to be re-encoded, 5
  cancelled by the hang hook, 6 frame-done event seen in the ISR); `+0x18…+0x67` the 0x50-byte ring content record being
  built (`mi_sys-1-engine.md` §10.1): `+0x18` extFlags, `+0x20` PTS, `+0x28` seq, `+0x30` EOS byte, `+0x38` u64 output
  (bitstream) PA — the ring region reserved by `EnsureTailSpaceTimeout`, `+0x40` output kernel VA, `+0x44` reserved output
  length (then the record length), `+0x48` QP byte, `+0x4c` POC, `+0x50` gradient (= descriptor +0x9c), `+0x54` chn,
  `+0x58…+0x60` input-buffer words; `+0x68` ring pool the task writes to; `+0x6c` bytes already committed as slice records;
  `+0x70` u64 input Y PA (= descriptor +0x28), `+0x78` input `u32BufSize`; `+0x7c` CamOs timer (60 ms,
  `mi_venc_worker_thread_timeout_monitor` → `mi_sys_NotifyWakeupISREvent` = wake the work thread); `+0x94` start time;
  `+0x9c…+0xcc` encode-time window (`_MI_VENC_EncodeTimeRecordStart` / `End`, `_AppendSmoothWindowValue`; `+0xbc` smoothed
  encode time in ms used by §10.1).

### 7.2 `_MI_VENC_OnPreProcessInputTask` (section offset 0x86ac) [R]
Arguments (cmdq, input-task info, meta block, device record); return **0** go on, **1** not now (input stays queued),
**2** drop the input. Inlined helpers named by its strings: `_MI_VENC_ValidBufferSize`, `_MI_VENC_UpdateResolution`,
`_MI_VENC_ClearRoiAttrIfResChange`; called: `S_MI_VENC_GetResolution` (Ghidra 0x130b4), `S_MI_VENC_SetResolution` (0x13604),
`S_MI_VENC_ValidateResolution` (0x15f70). Decision order:
1. Channel lookup (dev from the device record, chn = input-task word 0): channel absent or not created → **2**; channel not
   started (`chn+1` = 0) → **2** (`chn+0x83c` += 1 for every 2 below).
2. Under `dev+0x5d8` and the channel semaphore: BUG_ON when "reject CMDQ" (`dev+0x21`) disagrees with a NULL cmdq;
   `dev+0x3dc` = cmdq.
3. **Buffer size** (`_MI_VENC_ValidBufferSize`), skipped for a REALTIME bind (`chn+0x9f4` = 4): Y size (phyAddr[1] −
   phyAddr[0]) must be ≥ stride × aligned height, else **2** ("InputYUVBufSize %d is smaller than min buf size"). Stride =
   u32Stride[0], or for a RINGBUF buffer W aligned to the table entry +0x20 (×2 for YUYV); height = crop H when crop W and crop
   H are both non-zero, else u16Height; a RINGBUF buffer uses the ring height `chn+0x9f8`. Height alignment from the
   device's 0x80030003 table: FRAME_BASE bind entry +0x2c, HW_RING bind +0x38 (YUYV) / +0x50 (420), other binds 0 (no
   check, `align(x, 0)` = 0). mhal H.26x answers +0x2c = 32, +0x38 = 2, +0x50 = 0 (`MHAL_VENC_Wrapper_GetDevConfig`
   0x80030003 fills words [2]–[0xf] = 8, 2, 16, 16, 8, 2, 32, 32, 32, 32, 0, 32, 2, 2).
4. M6 JPEG channel (engine type 2, chip > 4): SRAM flags by buf ePhylayoutType — NORMAL: `dev+0x5f8` = 0 (u16), `+0x5fa` =
   0; REALTIME: `dev+0x5f8` = 0x100, `+0x5fa` = 0.
5. buf bDrop (+0x1c) set → **2** (and the after-bind drop counter `chn+0xa20` −1 when > 0); proc drop flag `chn+0xbc9` = 1
   (`drop_in`) → **2** ("drop").
6. BUG_ON on inconsistent task counters. **Busy** — channel tasks = channel cap (`chn+0x1670`) or device tasks = device cap
   (`dev+0x598`), any reset flag (`dev+0x5a0`, `0x5d0`, `0x5d3`, `0x5d5`, `0x5d4`, `0x5d1`, `0x5d2`), or a JPEG channel with
   tasks in flight on a device without a CMDQ (`dev+0x3dc` = 0) → REALTIME buffer: **2**; other buffers: **1**
   (`chn+0x840` += 1).
7. buf eBufType ≠ FRAME (1) → **2** ("Unsupported in Buf Type"). Pixel format not YUYV (0), NV12 (0xb) or NV21 (0xc) → **2**
   ("Unsupported in Pix Type"). **NV21 or YUYV on an H.26x device (dev id 0 / 1) → 2** ("only jpeg support in Pix Type").
8. **Resolution update** (`_MI_VENC_UpdateResolution`, every engine, every accepted frame):
   - new W × H = crop W × crop H (buf +0x80 / +0x82) when both non-zero, else u16Width × u16Height; a RINGBUF buffer takes H =
     u16RingBufRealTotalHeight (+0x72). The last input geometry is stored first (`chn+0x16a6…+0x16b8`, §2.2).
   - current size from the channel attr (`S_MI_VENC_GetResolution`: H.26x attr words 6 / 7 = u32PicWidth / Height, JPEG words
     5 / 6; other eType → 0xA0022008); failure → **2** ("unable to get resolution err").
   - equal → no change. Different → alignment test with the channel alignment word `chn+0xf0` (§11, `OnBindInputPort`: bytes
     [31:24] = table +0x58, [23:16] = +0x5c, [15:8] = +0x40, [7:0] = +0x44; YUYV uses the two high bytes for W / H, 420 the two
     low bytes): "W = align(curW)" or "H = align(curH)".
     - **NORMAL / REALTIME buffer**: either equality → no change ("NORMAL_FRAME_DATA don't need change resolution", debug
       level > 6); neither → change.
     - **RINGBUF buffer**: either equality → change ("RINGBUF_FRAME_DATA change resolution in dynamic, res[..]->[..]",
       warning); neither → **2** ("RINGBUF_FRAME_DATA do not support change resolution in dynamic").
     On M6 H.26x the table entries +0x40 / +0x44 / +0x58 / +0x5c are 0, so `chn+0xf0` = 0, `align(x, 0)` = 0 and no
     equality can hold: **any** size difference changes the resolution for a NORMAL / REALTIME buffer and is dropped for a
     RINGBUF one [R table values + code; I: holds as long as the peer is SCL and mhal answers as read].
   - change: save the old size; `S_MI_VENC_SetResolution(attr, W, H)` (writes only the attr's picture W / H, not the max size)
     — failure → **2** ("unable to set resolution"); `S_MI_VENC_ValidateResolution(attr, dev)` (§5.2) — failure → the old size
     is written back and **2**; re-read the size (failure only logged); if it changed and ROI was used (`chn+0x236`): the 16
     ROI entries are reset to `{0x28, 2, 0…}`, `chn+0x21c` = 1, `chn+0x236` = 0 ("…has been changed to…, clear the ROI");
     "auto change resolution from [%dx%d] to [%dx%d]" (debug level > 1); reconfig flags `chn+0x214` = `chn+0x215` = 1.
9. HW_RING bind with the after-bind drop counter `chn+0xa20` > 0 → counter −1, **2** ("drop frame %d after bind").
10. Accept: input-task info word +4 = 1, meta byte +4 (port 0 wants an output buffer) = 0, return **0**.

A size change is therefore **not** deferred: the same input is accepted, and the following Enqueue of that input sees
`+0x214` / `+0x215` and does STREAM_OFF → `S_MI_VENC_ConfigVeAttr` (new RESOLUTION) → STREAM_ON before encoding it (§7.3
step 8). No 0xA0022xxx code leaves this callback.

### 7.3 `_MI_VENC_EnqueueInputTask(cmdq, inTask, …, dev)` (section offset 0x190a0) — 0 queued, 2 retry later, 3 input dropped [R]
1. Locks; channel must exist, be started (`chn+1`) and have its input bound (`chn+0x9f2` = 1), else 3. Any device reset /
   recovery flag (`dev+0x5d0…+0x5d5`, §10) → 3 (frame dropped while the codec is being reset).
2. Input sanity (`inTask+0xc` = buf, must be a FRAME buffer): with `chn+0x9f4` = 4 (REALTIME bind) the buffer must be large
   enough for the aligned height × stride; else 3.
3. **Back-pressure**: `mi_sys_Ringpool_GetRecordUnreadNum` → `chn+0xd0` (wake the poll client if > 0);
   `mi_sys_Ringpool_GetRecordTotalNum` → `chn+0xcc`; **closed records ≥ `chn+0xd4` (MaxStreamCnt, default 3) → 3: the input
   frame is dropped** until the user releases streams.
4. No free task slot → 3 (`chn+0x83c` += 1). Else take the slot, init its lists, `+0x10` = 1.
5. Ring reservation: `S_MI_VENC_UpdateRecordMinAllocSize` sets the pool's minimum record size from the smoothed I / P frame
   sizes (§7.7) — the I-frame size when an IDR is due (GOP position `chn+0x1690 % gop`, forced IDR `chn+0xdc`, pending
   reconfig, JPEG); then `mi_sys_RingPool_EnsureTailSpaceTimeout(pool = chn+0xc0, 0 ms, &task+0x18, no hint, 1)`; failure → 3
   (no stream space: frame dropped).
6. Build the mhal input descriptor (0xd0 bytes, version 6, on the stack at `sp+0x168`, pointer kept in `sp+0xa4`) from buf,
   the task and the cmdq — field map §7.4. Forced IDR = `chn+0xdc` (then `chn+0xdc` and `chn+0xdf` are cleared). A
   pixel-format change on a running channel sets the reconfig flags `+0x214` / `+0x215`.
7. Debug: `dump_in` (§12.1) writes the input YUV to a file.
8. **Pending settings** (written by the ioctls, applied here in this order, each failure → 3):
   - user data (`chn+0xe4` len, `+0xe0` buf, §6.8): SetParam USER_DATA {0x10, 1, buf, len};
   - anti-tamper user data (§9);
   - `+0x214` reconfigure (SetChnAttr / SetRefParam / SetCrop / pixel format / resolution): if the instance is streaming
     (`chn+2`): SetParam **STREAM_OFF 0x80030005**; then `+0x215` → `S_MI_VENC_ConfigVeAttr` (resolution, profile, JPEG
     restart interval); `+0x21a` → REF {0x18, 1, base `+0x180`, enhance `+0x184`, bEnablePred `+0x188`} (0x80000002 /
     0x80010002), then max resolution (`venc_set_max_res` (0x1e2c0), RESOLUTION with the max size), free and re-allocate the private
     buffers (`S_MI_VENC_FreeChnMemory`, `S_MI_VENC_AllocBuf`; failure → 0xA002200C and **the channel is destroyed**),
     `S_MI_VENC_ConfigInstance`; `+0x21b` → CROP {0x1c, 1, en `+0x72c`, x, y, w, h `+0x730…`} with the id from the table at
     `.rodata` 0x1678 (H.264 0x80000001, H.265 0x80010001, JPEG 0x80020002); `+0x219` → `S_MI_VENC_SetVui`; then STREAM_ON
     (as StartRecvPicEx) and the 16 ROI entries (`S_MI_VENC_SetRoiCfg` 0…15); `chn+2` = 1;
   - `+0xde` EnableIdr → SetParam 0x80030007 {0xc, 1, `chn+0xdd`} (no effect on M6, §5.5);
   - `+0x216` RC attr changed → `venc_apply_rc` = ConvertRc + RC SetParam (§6.2);
   - `+0x217` → `S_MI_VENC_SetRcParam(chn, chn+0x148)` (§6.3);
   - `+0x218` → JPEG: GetParam 0x80020003, replace Q tables / Qfactor from SetJpegParam, SetParam (§6.6);
   - `+0x21c` custom map → SetParam 0x80030018 (§6.9), wake the custom-map wait queue `chn+0x1658`.
9. Fill the task: PTS from the frame, seq = `chn+0x830` (u64), EOS byte = buf +0x14, output buffer from the descriptor,
   status 1, timestamps; link into the two lists; arm the 60 ms timer. Task bookkeeping from the descriptor (0x19f62…0x19fd2):
   task+0x50 = desc+0x9c, task+0x70 = desc+0x28, task+0x78 = buf u32BufSize; `chn+0x16a4` = desc+0x1e, `chn+0x16b0` =
   desc+0x24, `chn+0x16bc` = desc+0x9c. If the instance is not streaming (`chn+2` = 0) → 3.
10. `MHAL_VENC_EncodeOneFrame(chn, desc)`: 0 → in-flight counters +1, `chn+0x838` += 1, return 0; **0x12 (mhal queue full) →
    unlink, clear the slot, return 2 (mi_sys retries the same input later)**; other errors → 3.

Old chips only (chips < 3): when the input is dropped for back-pressure or ring space, mi_venc writes the frame slot
(descriptor +0x1e value) to register 0x132bfc with mask 0xf through the CMDQ (slot +0x2c = register write with mask) and
kicks it (slot +0x40) — re-arming the source's ring handshake [R call; I purpose]. Not reached on M6.

### 7.4 The mhal input descriptor (0xd0 bytes, version 6) [R unless tagged]
<!-- ev:mi_venc.7.4 -->
Writer: `_MI_VENC_EnqueueInputTask` step 6 (fill at 0x19c28…0x1a198 plus the mode branches 0x1a132, 0x1a19a, 0x1a49c…0x1a4d8, 0x1a684, 0x1a820, 0x1a902). Readers: mhal
`MHAL_VENC_Wrapper_EncodeOneFrame` (0x400c6858), `_UpdateVencDynamicSramInfo` (0x400c11ec), `SetEncPicParam` (0x400c09a8) for
H.264 / H.265, and `MHAL_JPE_EncodeOneFrame` (0x400b4c10) for JPEG. The meaning of the mhal EncParam words it feeds ([5], [6], [9]–[0xb], [0xe], [0x41],
[0x42], +0xfc / +0xfe / +0x102) is `mhal-vcodec.md` §7.11's.

Sources of the values:
- buf = `MI_SYS_BufInfo_t` (`inTask+0xc`), M6 header layout: +0x00 PTS, +0x14 bEndOfStream, +0x18 u32SequenceNumber, +0x1c
  bDrop, +0x24 ePixelFormat, +0x34 ePhylayoutType (0 REALTIME_FRAME_DATA, 1 RINGBUF_FRAME_DATA, 2 NORMAL_FRAME_DATA), +0x38 /
  +0x3a u16Width / u16Height, +0x48 / +0x50 / +0x58 phyAddr[0..2] (u64), +0x60 / +0x64 / +0x68 u32Stride[0..2], +0x6c
  u32BufSize, +0x70 u16RingBufStartLine, +0x72 u16RingBufRealTotalHeight, +0x74 stFrameIspInfo.eType, +0x78
  u32GlobalGradient, +0x7c…+0x82 stContentCropWindow. Every offset mi_venc reads matches this layout [R: loads in the fill
  code; I: that the header is the one this build used].
- inTask = the mi_sys input-task info handed to the callback; its +0x58 (u64) / +0x60 are the SRAM block address / size
  mi_sys put into the task (`mi_sys-1-engine.md` §9: input task +0x90…+0xa0 = info +0x58…+0x68) [R offsets; I mapping].
- task = the task slot (§7.1); cmdq = the callback's first argument (mi_venc calls its slots +0x2c register write with mask
  and +0x40 kick; mhal JPE uses desc+0x10 as the CMDQ handle, "IMI/EMI mode need with CMDQ!").

The whole struct is zeroed first (`memset(+8, 0, 200)`), so every offset not listed below is 0.

| off | size | written by mi_venc | read by mhal |
|---|---|---|---|
| +0x00 | u32 | 0xd0 (size) | version header |
| +0x04 | u32 | 6 (version) | version header |
| +0x08…+0x0f | | 0 | — |
| +0x10 | ptr | cmdq interface (callback arg 1; NULL when mi_sys runs the task without a CMDQ) | JPE: CMDQ handle, needed for input modes 1–4; VENC: not read |
| +0x14 | u8 | forced IDR = `chn+0xdc` (then `chn+0xdc` and `chn+0xdf` are cleared) | VENC `SetEncPicParam`: non-zero or first frame → IDR (EncParam `forceIdr` words [0xc]=1, [0xd]=3), then mhal writes 0 back into the descriptor; JPE: not read |
| +0x18 | u32 | input mode: 0 frame (NORMAL buffer), 2 ring (RINGBUF buffer), 3 realtime/IMI (REALTIME buffer, NV12/NV21 within the SRAM width cap, or other formats), 4 EMI (REALTIME NV12/NV21 wider than the cap) | JPE: 0→frame, 1→ring (rejected in this build), 2→ring-2, 3→IMI, 4→EMI (`mhal-scl-jpe-cmdq.md` §4.7); VENC: not read (ring mode comes from SetParam 0x80030015, inst+0x428/0x42c) |
| +0x1c | u16 | ring height = buf u16Height (RINGBUF only) | VENC `u16RingBufHeight`: must be ≥ align2(picture H) for subFrameSyncMode 0/3, = align2(H/2) for mode 1, else error 3; replaces the aligned picture height in the buffer-size check; copied to EncParam +0xfc. JPE: ring height (16-aligned, warning) |
| +0x1e | u16 | frame slot = `1 << (buf seq % 3)` (1, 2, 4) on chips ≥ 3 (M6); `(seq & 3) + 1` on chips < 3 | VENC: EncParam word [0xe]; JPE: frame index, clipped 0–15 → reg 00 bits 4–7 (`mhal-scl-jpe-cmdq.md` §4.8) |
| +0x20 | u32 | buf u32SequenceNumber — only for a HW_AUTOSYNC bind (`chn+0x9f4` = 8) on chips > 4 | VENC: EncParam word [0x42] |
| +0x24 | u16 | ring start line = buf u16RingBufStartLine (RINGBUF; 0 with input-source mode 1 = one-frame ring) | VENC: added to the start row of Y and C (`Y = +0x28 + stride·(start + cropY) + cropX`, `C = +0x40 + cropX + stride·cropY/2 + start·stride/2`), copied to EncParam +0xfe; JPE: ring start line |
| +0x26 | u16 | `chn+0x9f0` = the SCL peer's line count (set by `OnBindInputPort`) | not read |
| +0x28 | u64 | Y plane PA = buf phyAddr[0]; REALTIME on M6 = SRAM address (inTask+0x58); EMI = the `E_MMAP_ID_EMI` base; REALTIME on chips ≤ 4 = 0 | VENC: source Y address; JPE: Y address (must be non-zero in frame mode), MIU-range check |
| +0x30 | u64 | RINGBUF only: Y end address = Y PA − 1 + stride·height | not read by VENC or JPE |
| +0x38 | u32 | Y plane size = phyAddr[1] − phyAddr[0]; REALTIME on M6: SRAM size·2/3 (NV12/NV21, whole size for YUYV); EMI: align256(W·32); REALTIME on chips ≤ 4: 0 | VENC `InputYUVBufSize`: must be ≥ aligned height × stride (error 3); JPE: Y size (IMI check `align8(W)·32 ≤`) |
| +0x3c | u32 | Y stride = buf u32Stride[0]; REALTIME: picture W rounded up to the engine's input alignment (`dev+0x1d4+type·100`, the 0x80030003 table entry +0x20), ×2 for packed YUYV (format 0) | VENC `InputYUVBuf1Stride`: must be a multiple of 32 and ≥ align32(W), else error 3; JPE: pitch (16-aligned, warning) |
| +0x40 | u64 | C plane PA = buf phyAddr[1]; REALTIME on M6: SRAM address + Y size (0 for YUYV); EMI: base + Y size | VENC / JPE: C address |
| +0x48 | u64 | RINGBUF only: C end address = C PA − 1 + stride·height/2 | not read |
| +0x50 | u32 | written only in the ring/realtime branches: C size = Y size / 2 (one-frame ring), SRAM size / 3 (REALTIME NV12/NV21 on M6), align256(Y size / 2) (EMI), else 0 | JPE: C size (IMI check `align8(W)·16 ≤`); VENC: only copied for the proc dump |
| +0x54 | u32 | C stride = buf u32Stride[1] (0 in the REALTIME branch) | not read |
| +0x58 | u64 | third plane PA = buf phyAddr[2] | JPE: MIU-range check only |
| +0x64 | u32 | third stride = buf u32Stride[2] (0 in REALTIME) | not read |
| +0x68 | u64 | output (bitstream) PA = task+0x38 (the ring region reserved by `EnsureTailSpaceTimeout`) | VENC: EncParam bitstream address [5]; JPE: output address (16-aligned) |
| +0x70 | u32 | output kernel VA = task+0x40 (+0x74 = 0) | JPE: output config; VENC: only the dump list |
| +0x78 | u32 | output length = task+0x44 | VENC: EncParam bitstream size [6]; JPE: output size (non-zero, 16-aligned) |
| +0x94 | fn | `mi_sys_VFlushInvCache` (cache-flush callback) | JPE: conf type 11 (flush callback); VENC: not read |
| +0x9c | u32 | ISP global gradient = buf u32GlobalGradient when buf eType = 1, else 0 (chips < 3) or 0x40000 (chips ≥ 3) | VENC: 0 → 0xffff, > 0xffff → 0x7fff, else the value → EncParam u16 +0x102 (so on M6 the default 0x40000 becomes 0x7fff) |
| +0xa4 | u8 | 0 | VENC: picture QP, clipped into the instance's I / P / B QP ranges → EncParam [9]–[0xb] |
| +0xa8 | u32 | 0 | VENC: EncParam [0x41] |
| +0xb0 | u64 | SRAM address = inTask+0x58 — when the device has SRAM (`dev+0x5f8` ≠ 0, H.26x on chips > 4) | VENC `_UpdateVencDynamicSramInfo`: SRAM address for the VPU (inst+0x154c) |
| +0xbc | u32 | SRAM size = inTask+0x60 (same condition; also in the REALTIME branch on M6) | VENC: must be ≥ `_CalculateMaxVencSramSize(W, …)` else error 0xc; → inst+0x1548 |
| +0xc0 | u32 | 0x100 (same condition) | not read by VENC or JPE |
| +0xc8 | u32 | `eImiMode` = 1 when SRAM is present or in the REALTIME branch on M6; else 0 | VENC: **must be 1** — `_UpdateVencDynamicSramInfo` runs for every H.26x frame and returns 8 "not support eImiMode" otherwise; JPE: must be 1 in IMI mode (3) |

How the branches fill it:
- Order: common part (+0x00…+0x9c, from buf / task) → HW_AUTOSYNC sequence word (+0x20) → SRAM window (+0xb0…+0xc8) →
  branch by buf ePhylayoutType.
- **NORMAL (2)**: nothing more — mode 0, the three planes as given by mi_sys. This is the app's path (FRAME_BASE bind from
  SCL) [I: prog_uvc binds SCL→VENC frame-mode, `mi_sys-1-engine.md` §3.4].
- **RINGBUF (1)**: mode 2, +0x1c = height, +0x26 line count, +0x24 start line, +0x30 / +0x48 end addresses; with input-source
  mode 1 (one-frame ring, `chn+0xa28` = 1) +0x24 = 0 and +0x50 = Y size / 2.
- **REALTIME (0)**: stride from the engine alignment. NV12 / NV21 wider than the device cap (`dev+0x334+type·0x18`, the
  0x8003001a answer word +0xc) → **EMI** (mode 4): area `E_MMAP_ID_EMI` from `MI_SYSCFG_GetMmapInfo`, BUG_ON when absent or
  when Y+C (32 lines) exceed it. Otherwise mode 3: on chips ≤ 4 all addresses 0 and +0xc8 = 0; on M6 the SRAM block is the
  input (Y = first 2/3, C = last 1/3 for NV12 / NV21; whole block for YUYV), +0xc8 = 1. Since mhal refuses every H.264 /
  H.265 frame without `eImiMode` = 1 and a large enough SRAM window (rows +0xbc, +0xc8), M6 H.26x encoding depends on the
  SRAM block mi_sys puts into each task.

### 7.5 Events from the engine [R]
- **H.264 / H.265 on M6**: mi_venc requests no IRQ; mhal's hard-IRQ handler `codec_isr` calls the event callback
  `_VENC_ISR_CB_H26x_0` (registered with SetDevConfig 0x8003001c, §3.3) → `_VENC_VCodec_ISR(dev 0, msg[12 words])`. Event
  counters `dev+0x560…+0x580` (labels §12.2: total, frame done, buffer full (+0x568, +0x570), slice done, ring full,
  re-encode, timeout, other). By `msg[0]`: 0 count only; **1 frame done** and **3 slice / buffer full** →
  `_VENC_Codec_IsrHandle`; 4, 8 count only; **7 re-encode request** (from mhal's super-frame logic): find the channel by
  instance handle `msg[10]` (or by the in / out addresses `msg[2…5]`), its head task: if status 3 or 6 →
  `mi_sys_RingPool_CancelUnFinishedTailRecord` (drop the slice records already committed), status = 1, `+0x6c` = 0,
  `chn+0x85c` += 1. Always: `dev+0x590` = current PTS (used as the PTS of frames whose input had none, §7.6).
- `_VENC_Codec_IsrHandle(dev, 0, msg)`: find the channel (instance handle, else addresses) and the in-flight task whose
  input Y PA (`task+0x70`) and output PA (`task+0x38`) match `msg[2…5]`; restart its timer; `msg[6]` = bytes produced by this
  piece. For a slice (3): commit it as a record — extFlags |= codec << 1 (no frame-end bit),
  `AppendTailContentTailRecord(pool, chn, isr 1, close 0, &task+0x18)`, wake the poll client, `EnsureTailSpaceInISR` with
  the record as hint for the next slice (failure → status 4: out of ring space); `task+0x6c` += bytes. A full slice area is
  thus handled by reserving the next ring region in the ISR, not by EncodeAddOutBuff. For frame done (1): status 6 and the
  last piece is committed the same way. Size sanity (`msg[6]` > reserved length) only counts (`chn+0x854` / `0x858`).
- **JPEG (and old chips)**: mhal JPE's ISR calls `VENC_ISR_JPEG_x` → `VENC_IP_ISR`: `MHAL_VENC_GetEngStatus`,
  `MHAL_VENC_IsrProc`, `_MI_VENC_HandleIsr` (the same slice / frame-done handling for the JPE engine, including
  `MHAL_VENC_EncodeAddOutBuff` when the engine reports a full buffer: a new tail region is reserved in ISR and handed to the
  engine), cancel on error, `mi_sys_NotifyWakeupISREvent`.

### 7.6 `_MI_VENC_DequeueInputTask` (section offset 0x142e8) — finishing a frame [R]
1. Head task of the channel list; failure if: status 5, device HW timeout `dev+0x5a0`, task timeout `chn+0x1684`, reset flags
   `dev+0x5d0` / `5d1`. Restart the timers of the tasks queued behind it.
2. `MHAL_VENC_EncDone(chn, &res 0x58)` (`EncodeFrameDone`): `res+8` flags (0 normal, 2 nothing output, 0x800000 the
   firmware's input-ring-overflow warning, `warnInfo` — treated like "nothing output"), `res+0x18` frame bytes, `res+0x4c`
   reference type (0–5, 0 = I; 6 = dropped), `res+0x50` POC, `res+0x54` average QP; `res+0x3c` / `0x44` / `0x48` (intra / skip / merge
   counts) → `chn+0x161c…` (layout of the result: `mhal-vcodec.md` §7.10). Status 4 (out of space) → super-frame QP step (§8.2) and
   failure.
3. Failure (EncDone error, status 3–5, nothing output, ring overflow, HW / task timeout): set **forced IDR `chn+0xdc`** for the
   next frame, cancel the unfinished records, and run the reset handshake if this was the last task in flight (§10.2).
4. `chn+0xdf` (drop request from `drop_out`, §12.1): with a FRAME_BASE bind (`chn+0x9f4` = 1) the frame takes the re-encode
   path (records cancelled, input returned with status 2 = re-queued); other binds take the failure path.
5. Size bookkeeping: I or P (reference type 0 = I) → smoothing window + ring minimum size update (§7.7); GOP counter
   `chn+0x1690` += 1.
6. Oversized frame and frame-lost strategy (§8) may now discard or re-encode the frame.
7. Anti-tamper (§9).
8. **Final record** (`_MI_VENC_ProcessTaskDone` inlined): remaining length = frame bytes − `task+0x6c` (pieces already
   committed by the ISR); `task+0x18` extFlags |= reference type << 4 | codec (eType) << 1 | 1 (frame end); PTS = input PTS
   (−1 → `dev+0x590`); seq = `chn+0x830`; `+0x30` = input EOS byte; `+0x44` remaining length; `+0x48` QP, `+0x4c` POC,
   `+0x54` chn; `AppendTailContentTailRecord(task+0x68, chn, isr 0, close 1, &task+0x18)` (length 0 just closes the frame,
   `mi_sys-1-engine.md` §10.4) — BUG_ON on failure; wake the poll client; `chn+0x830` += 1, `chn+0x228` += bytes, first frame
   → `mi_sys_RecodeTimeStamp("VENC_FirstFrameDone")`. Field meanings on the user side: §13.4.
9. Release: `+0x230` (super-frame step) = 0, in-flight counters −1, timer deleted, encode time recorded, slot cleared; the
   input buffer goes back to mi_sys with the status (1 done, 2 re-encode — mi_sys re-queues the same input, 3 failed). PTS
   via `mi_sys_GetCurPts`.
10. **Ring-pool replacement**: when a second pool is pending (`chn+0xc4`, made by ResetChn / a size change) and the old one has
    delivered `chn+0xd4` frames, the pools are swapped under `chn+0x9fc` (`chn+0xc8` = 1, `+0xc9` counts tasks still writing to
    the old one); ReleaseStream destroys the old pool once it is empty (§5.4).

### 7.7 Ring size and minimum record size [R]
`S_MI_VENC_CalChnBufsize` (first StartRecvPicEx): ring size `chn+0x220` = the attr's `u32BufSize`; 0 → max W·H/2 (W·H rounded
to even when that is ≤ 0x25800); `max_ring_pool_size` > 0 replaces it. Minimum record size = min(size / the max stream count,
size / 9) → `chn+0x1694`, `+0x168c` [I: the divisor as decompiled is implicit; the function BUG_ONs when `chn+0xd4` = 0].
`mi_sys_RingPool_Create(dev handle, chn, MMA heap from mi_sys_GetChnMMAConf(module 2, dev, chn) or default,
"venc-ring-mem<chn>", {size, min, alignment 0x20, …, ring count = byte `chn+0x1670`, flag = byte `chn+0x804`}, &chn+0xc0)`
(config layout `mi_sys-1-engine.md` §10.1). `chn+0x1670` is the task count of §7.1 (2 for H.26x), while
`mi_sys-1-engine.md` §10.1 / §10.5 reads that word as the ring count and finds one ring per pool in use; which value the byte
holds at Create time is not settled [I].
`_MI_VENC_RingPoolAllocMinBufferInit` / `_MI_VENC_RingPoolAllocIFrameMinBufferInit` (CreateChn) allocate two smoothing
windows per channel; Dequeue appends every P / I frame size; `S_MI_VENC_UpdateRecordMinAllocSize(chn, task, isI)` sets
`mi_sys_RingPool_SetRecordMinAllocSize(pool, size)` from the window (I-frame size before an expected I frame), so the ring
reserves at least a typical frame in one contiguous region; failure → 0xA002201F. It clears `chn+0x1685` after recomputing.

### 7.8 Multi-task scheduling [R; I where marked]
Up to 2 H.26x frames per channel are handed to mhal before the first completes; mhal's own queue may refuse (0x12 → retry).
The engine finishes them in order (one Wave5 core, `mhal-vcodec.md` §1.2) and Dequeue consumes strictly from the channel list
head, so record order in the ring = submission order. All channels of a device share the device semaphore, so submission
and completion of different channels interleave only between callbacks. There is no priority between channels other than
mi_sys's work-thread order [I]. How the encode time of a frame splits between firmware and host is not known from the
binaries; mhal's `/proc/mhe/ctx_info` shows it on a running camera (`mhal-vcodec.md` §2.8).

## 8. Oversized frames and the frame-lost strategy
<!-- ev:mi_venc.8 -->

Code: `_MI_VENC_DequeueInputTask` — frame-lost block 0x147ac…0x1484e, trip 0x1570c…0x157a8, threshold 0x15cb0…0x15de6 and
0x15fdc / 0x15ff0, oversize handling 0x15080 (ring-ref), 0x157ac…0x15852 (counter logic), 0x168a6 (M6 H.26x);
`MI_VENC_IMPL_SetFrameLostStrategy` (0x27d24) [R: disassembly].

### 8.1 Configuration: SuperFrameCfg — SetSuperFrameCfg (0x3e) / GetSuperFrameCfg (0x3f) [R]
`MI_VENC_IMPL_SetSuperFrameCfg` stores `MI_VENC_SuperFrameCfg_t` {mode 0 none / 1 discard / 2 re-encode (> 2 →
0xA0022003), IThr, PThr, BThr bits} at `chn+0x740…0x74c`. On chips > 2: if the channel is not started and `use_ring_ref` = 0
it first sends SetParam **0x8003000f** {0xc, 1, 0} (the ring-reference switch, §4); mode 2 → SetParam **0x8003000c** {0x14,
1, u8 QP step 0, IThr·9/10, PThr·9/10} (mhal's super-frame re-encode thresholds, `mhal-vcodec.md` §7.8); modes 0 / 1 →
0x8003000c with zeros (mhal re-encode off). Get returns the stored struct. Default: mode 0 (CreateChn zeroes the channel).
Unused by the app.

### 8.2 Oversize handling (shared by super-frame and frame-lost trips) [R]
Run after EncDone, for frames accepted so far. **Entry**: a frame oversized by the super-frame test — not when mode = 0, or
when RcPriority = bitrate-first (`chn+0x828` = 1) and `chn+0x864`·1000 < the bitrate `chn+0x80c` (`chn+0x864` is the
measured kbps over 1 s, so the test compares the measured rate with the configured one [I from the dump label]); else a JPEG
or I frame is oversized when IThr (`+0x744`) ≠ 0 and `size·8 ≥ IThr`, a P frame when PThr (`+0x748`) ≠ 0 and `size·8 ≥ PThr`
— or a frame-lost trip (§8.3; any super-frame mode, including 0). `m` = super-frame mode (`chn+0x740`), `c` = the oversize
counter `chn+0x778`. "Discard" = the failure path at 0x14b4e: unfinished records cancelled, task fails (status 3), reset
handshake when it was the last task in flight and a reset is pending (§10.2). "Re-encode" = 0x156d2: records cancelled,
`chn+0x84c` += 1, the input buffer returns to mi_sys with status 2 (re-queued).

| condition (tested in this order) | action |
|---|---|
| `use_ring_ref` ≠ 0 (default 1), any codec — the reference picture was overwritten in place, so the frame cannot be redone | `c = min(c + 1, discard_max_cnt)`; SetParam **0x8003000c** {0x14, 1, QP step = `discard_dqp·c` (u8) on chips ≤ 2, 0 on chips > 2; IThr·9/10; PThr·9/10}; forced IDR (`chn+0xdc`); **discard** |
| H.26x on chips > 2 (**M6**) | forced IDR; m = 1 → `c = min(c + 1, discard_max_cnt)` and 0x8003000c {0, 0, 0}; any m → **discard**; with m = 2 the re-encode is done inside mhal itself (it raises event 7 and the task restarts, §7.5) [I: from the event-7 handler; Dequeue only discards here] |
| H.26x on chips ≤ 2 | SetParam **0x8003000b** {0xc, 1, `use_ring_ref` (= 0 here)} [I: re-encode enable], then the counter rule below |
| JPEG (any chip, with `use_ring_ref` = 0) | the counter rule below (no 0x8003000b) |

Counter rule (chips ≤ 2 H.26x and JPEG):

| c | m ≠ 2 (mode 1, or 0 for a frame-lost trip) | m = 2 (re-encode) |
|---|---|---|
| c = `reencode_max_cnt` and c ≠ `discard_max_cnt` | c += 1; QP step `discard_dqp·c`; 0x8003000c; **discard** | give up: c −= 1, forced IDR, **discard** |
| c = `reencode_max_cnt` = `discard_max_cnt` (defaults 4 / 4) | give up: c −= 1, forced IDR, **discard** | give up: c −= 1, forced IDR, **discard** |
| c = `discard_max_cnt` ≠ `reencode_max_cnt` | give up: c −= 1, forced IDR, **discard** | c += 1; QP step `reencode_dqp·c`; 0x8003000c; **re-encode** |
| otherwise | c += 1; QP step `discard_dqp·c`; 0x8003000c; **discard** | c += 1; QP step `reencode_dqp·c`; 0x8003000c; **re-encode** |

0x8003000c always carries IThr·9/10 and PThr·9/10 from `chn+0x744` / `0x748`. With the default parameters, mode 2 re-encodes
an oversized frame up to four times with QP steps 4, 8, 12, 16 and discards it at the fifth (c then 3); giving up also
discards. The counter returns to 0 on the next frame that gets past the frame-lost block (§8.3 step 5). None of the chips ≤ 2
arms run on M6 except the JPEG row, and that one only with `use_ring_ref` = 0. Status-4 tasks (out of ring space) apply
`+0x230`·`discard_dqp` as QP step and count `+0x230` up to 6. `discard_dqf` and `reencode_dqf` are not read (§4).

### 8.3 Frame-lost strategy (ioctl 0x3c / 0x3d; parameter `frm_lost_win`, default 6) [R]
State (channel offsets):

| off | content |
|---|---|
| +0x750 / +0x754 / +0x758 / +0x75c | stored `MI_VENC_ParamFrameLost_t`: u8 bFrmLostOpen, u32FrmLostBpsThr (bits/s [I from the name]), eFrmLostMode, u32EncFrmGaps |
| +0x760 | I-frame figure `(BpsThr/8)·Isize / R` — written at every I frame, **only printed** (debug level of `dbg_frmlost`), no decision uses it |
| +0x764 | P-frame threshold T (bytes per frame) |
| +0x768 | N: number of P frames seen since SetFrameLostStrategy (accepted and tripped) |
| +0x76c | S: sum of the sizes of the accepted P frames since SetFrameLostStrategy (never reset by Dequeue) |
| +0x770 | never written after the Set (always 0) — so +0x774 always equals S |
| +0x774 | last non-zero S (fallback when S = 0) |
| +0x778 | u8 oversize counter shared with §8.2 |
| +0x77c + 4·i | window slots, i < `frm_lost_win` |
| +0x7fc / +0x800 / +0x802 | window sum / u16 slot index / s16 skip counter (< 0 = testing) |

**Set** (0x3c): only `eFrmLostMode` = NORMAL (0) is accepted (PSKIP → error), and not for MJPEG-FIXQP; copies the 16-byte
struct to +0x750, zeroes +0x768…+0x803 (so the skip counter +0x802 starts at **0**), sets +0x760 and **+0x764 = 0xffffffff**,
and fills `frm_lost_win` slots with `0xffffffff / (2·frm_lost_win)` (sum ≈ 0x7fffffff); `frm_lost_win` ≤ 0 is handled as 1;
there is no upper clamp (the window has room for 32 slots; a larger module parameter writes past +0x7f8 into the sum / index
words) [R: no compare against 32 in Set or Dequeue; I consequence]. **Get** (0x3d) returns the stored struct.

**Per frame**, H.264 / H.265 only (eType ≠ JPEG), when bFrmLostOpen ≠ 0 — run after EncDone and the super-frame test, for
frames not already discarded; `size` = EncDone frame bytes, I = reference type 0:
1. **Skip counter ≥ 0**: counter −= 1, then *no test*: a P frame is put into the window and accounted (step 4), an I frame
   recomputes T (step 3). Frames inside the gap are therefore **kept, not dropped**. The first frame after a Set takes this
   branch (counter 0 → −1).
2. **Counter < 0, P frame**: `M = (sum + size − slot[idx]) / frm_lost_win` (u32 divide). **M > T → trip**: counter =
   `u32EncFrmGaps − 1`; the slot is emptied (`sum −= slot[idx]`, `slot[idx] = 0`, idx += 1, wrap at `frm_lost_win`); N += 1 (S
   unchanged); the frame enters the oversize handling of §8.2 exactly as an oversized frame would. M ≤ T → step 4.
3. **I frame (either branch)**: `A = S ≠ 0 ? S : +0x774` (and +0x774 = S when S ≠ 0);
   - `R` (estimated stream rate, bytes/s) = `(A / (N + 1)) · FpsNum / FpsDen` (`chn+0x810` / `0x814`); when A = 0: `R = BitRate
     / 8` (`chn+0x80c`), and when that is 0 too `R = BpsThr / 16` (the 0x161da arm, read in the decompile only); R = 0 → 1.
   - `+0x760 = (BpsThr >> 3) · size / R` (u64 arithmetic; debug only).
   - if A ≠ 0 and N ≠ 0: **`T = ((BpsThr >> 3) · S · 12 / 10) / R / N`** (u64, truncating at each divide), i.e. the mean
     P-frame size rescaled from the measured rate to the threshold rate, plus 20 %. Otherwise T keeps its value (the initial
     0xffffffff until the first I frame that follows at least one P frame).
   - I frames never enter the window and are not counted in N or S.
4. **Accept a P frame**: `slot[idx] = size`, sum updated, idx += 1 (wrap), S += size, N += 1.
5. After the block, for every frame that reached it, the oversize counter +0x778 = 0.

So a P frame is dropped when the mean of the last `frm_lost_win` P-frame sizes (this one included) exceeds 1.2 × (mean P
size since the Set) × (BpsThr / 8) / R, with R re-estimated at every I frame; after a drop the next `u32EncFrmGaps − 1`
frames pass untested [R formula; I wording]. Because R is computed from the same S and N, the expression is algebraically
`T = 1.2 · (BpsThr / 8) · (FpsDen / FpsNum) · (N + 1) / N` up to the integer truncations — the per-frame byte budget of the
threshold bitrate plus 20 % (plus 1/N); the measured sizes cancel out [I: algebra on the R formula]. When the BitRate or
BpsThr fall-backs are used (S = 0) T is not recomputed, because N = 0 or A = 0 skip it.

## 9. Anti-tamper ("VAT", parameter `use_anti_tamper`, default 0 — off on this camera) [R]

With `use_anti_tamper` = 1, every finished H.26x frame produces a 44-byte record that is inserted into the **next** frame as
SEI user data. It carries an obfuscated length of the frame's slice data, a time slot and a counter — the bytes of the
picture are not hashed (`MI_VENC_VAT_CheckSum` is an empty function). Nothing on the camera turns it on (no module
parameters are passed, §4).

1. `_MI_VENC_DequeueInputTask` (§7.6), after a frame is accepted and if the task's `+0x6c` word is 0: allocate the channel's
   0x2c-byte buffer once (`chn+0xe8`, `CamOsMemAlloc`), `off = MI_VENC_VAT_GetCFDOffset(VA, len)`; unless it returns
   0xA0022005, `MI_VENC_VAT_GetAntiTamperBytes(PA + off, len − off, chn+0xe8)` and `chn+0xec` = 0x2c (pending).
2. `_MI_VENC_EnqueueInputTask` of the next frame calls `_MI_VENC_InsertAntiTamperData(chn)`: H.264 / H.265 only; if `chn+0xec`
   ≠ 0: clear it and `SetParam(USER_DATA 0x8000000e | 0x8001000c, {0x10, 1, buf = chn+0xe8, len = 0x2c})` (mhal writes it as a
   user-data-unregistered SEI, §6.8). Failure → 0xA002201F and the frame is not encoded. (The InsertUserData path of §6.8
   uses the same ids separately.)

`MI_VENC_VAT_GetCFDOffset(p, n)` — offset of the first VCL NAL: scans `min(n − 4, 300)` bytes for a 4-byte start code `00 00
00 01`; at a match looks at the NAL header byte `h`: returns the start-code offset when `(h & 0x1b) == 1` (H.264
nal_unit_type 1 or 5 = non-IDR / IDR slice) or when `(h & 0x7e) >> 1` ∈ {1, 2, 19} (H.265 TRAIL_R, TSA_N, IDR_W_RADL); none
within the window → not found.

`MI_VENC_VAT_GetAntiTamperBytes(addr, L, out[0x2c])` — record layout:

| bytes | content |
|---|---|
| 0 | 0 |
| 1–3 | big-endian `L ^ (mask & 0x45c409)` with `mask = (ID[3] << 8) | (ID[2] << 16) | IDbyte`, for `L < 0x1000000`; else `ff ff ff`. A flag is set when `L ≥ 0xffffff`, `L = 0x1f` or `L < 0x1f` [R; I: "short / oversized frame"] |
| 4–5 | big-endian time slot = seconds / 900 (15-minute slot) [I: seconds from the kernel clock read just before; the call is stripped from the decompile] |
| 6–7 | big-endian counter within the slot (`.bss`+0x216, reset to 0 when the slot `.bss`+0x214 changes) |
| 8–0x25 | 0 (two zero-fill variants depending on the flag — same result) |
| 0x26–0x2a | custom ID: the word at `.data`+0x1b0 (bytes in memory order) and the byte at `.data`+0x1b4 |
| 0x2b | 0x73 ('s') |

- Custom ID default: `.data`+0x1b0 = 0xffffffff, `.data`+0x1b4 = 0xff. `MI_VENC_VAT_SetCustomID(word, byte)` writes them but
  has **no caller** (global symbol, not exported), so the ID is always the default; with ID = all ones the mask is
  `0xffffff & 0x45c409` = 0x45c409.
- Present and never called (unused): `MI_VENC_VAT_RBSPtoEBSP(dst, src, n)` (inserts emulation-prevention 0x03 after two zero
  bytes when the next byte ≤ 3), `MI_VENC_VAT_CheckSum` (empty), `CalcSum(p, n)` (byte sum), `CheckSum` (hex dump / print
  helper, reached only from the empty `VAT_CheckSum` per Ghidra's call graph).
- A viewer would need the vendor's checker to use the record; no such checker is on the camera [I].

## 10. Timeout recovery, codec reset handshake, `/bin/sh` hook [R]

Device flags (byte at `dev+…`, names from use): `0x5a0` HW timeout seen, `0x5a1` recovered, `0x5d0` **reset pending** (new
frames refused, StartRecvPicEx waits), `0x5d1` pre-reset requested, `0x5d2` reset requested by the peer while idle, `0x5d3`
peer reset in progress, `0x5d4` post-reset must be retried, `0x5d5` peer (VDEC) busy. Per-device hang settings at
`.data`+0x310 + dev·0x8c: `+0` run_sh enable (default 0), `+4` script path (0x80 chars, default empty), `+0x84` en_recover
(`.data`+0x394 + dev·0x8c; default **1** for devs 0, 1, 8, 9). Encoder hangs are handled as an expected event.

### 10.1 Watchdog: `_MI_VENC_CheckInputTaskStatus` (run by the work thread when a task timer fires)
1. Channel gone or not started → status 3 (fail). `dev+0x5d1` → `_MI_VENC_PreResetCodec`. `dev+0x5d5` → re-arm the timer 10 ms
   and wait. Reset pending (`0x5d0`) → fail.
2. Budget: base 1000 ms (H.26x) / 100 ms (JPEG); 2 × the task's smoothed encode time if known; elapsed = time since the task
   started.
3. `MHAL_VENC_CheckInstanceStatus(chn)` (MFE / MHE: mhal's encode-phase word, `mhal-vcodec.md` §7.1):
   - 0: running → keep waiting.
   - 1 (fired, not finished): still within max(base, 2·smooth) and within 8 frame intervals (`FpsDen·8000/FpsNum` ms, not below
     base) → wait (unless the force-reset test says otherwise, §10.4). Otherwise **timeout**: H.26x — when `mi_timeout_reset`
     (default 1) and no reset pending: `_MI_VENC_PreResetCodec`, `dev+0x5d0` = 1, then the hook; JPEG — when
     `jpeg_timeout_reset` (default 1): `MHAL_VENC_EncodeCancel` (JPE aborts; for MFE / MHE the slot is NULL).
   - 2: finished → status 2, return 1 (mi_sys calls Dequeue).
   - 3: engine error → JPEG: cancel as above; H.26x: the hook.
   - 4: `EncodeCancel`, status 3.
   - **hook** (H.26x): if `en_recover[dev]` and `run_sh[dev]`: `call_usermodehelper("/bin/sh", {"/bin/sh", "-c", path},
     {"HOME=/", "PATH=/sbin:/usr/sbin:/bin:/usr/bin", "LD_LIBRARY_PATH=/lib"}, UMH_WAIT_PROC)` — the kernel thread blocks until
     the script exits; task status 5. The proc help names the intended script "/mnt/dump_mhal_venc_status_hook.sh" (a register
     dump) [R string]. Off by default.
4. A failed task: cancel its unfinished records (`CancelUnFinishedTailRecord`), free pending dump entries.

`mi_venc_worker_thread_timeout_monitor` (the 60 ms task timer) only wakes the work thread (`mi_sys_NotifyWakeupISREvent`).

### 10.2 Reset sequence (the last task to fail does it, in Dequeue §7.6 step 3)
1. Pre-reset `_MI_VENC_PreResetCodec(dev)`: SetDevConfig **0x8003001f** {0xc, 1, out}: out 2 → `dev+0x5d5` = 1 (VDEC busy, try
   later), out 1 → ok; clears `0x5d1`. (mhal side: handshake with VDEC, `mhal-vcodec.md` §9.)
2. If no task timed out and a reset is pending: `S_MI_VENC_GetChnResHandle(dev)` = SetParam STREAM_OFF (0x80030005) on every
   streaming channel.
3. Post-reset: SetDevConfig **0x8003001d** {0xc, 1, out} (mhal `codec_dev_reset`): out 1 → if a reset was pending:
   `S_MI_VENC_StreamOnAllChannel(dev)` (STREAM_ON 0x80030004 + the 16 ROI entries for every created channel), clear `0x5d0`,
   wake `MITimeoutWaitqueue` (StartRecvPicEx waiters); out 2 → `0x5d4` = 1 (retry with the next failure); then clear `0x5a0`
   and `chn+0x1684`, `0x5a1` = 1.
4. The failed frame is lost (status 3); the next frame of every channel starts with a forced IDR (`chn+0xdc`).

### 10.3 Reset requested by the codec / VDEC: `_MI_VENC_ResetCB` (dev 0) and `_MI_VENC_ResetCB_H26x_1` (dev 1)
Registered with SetDevConfig 0x80030021; mhal calls it with a message whose `+8` word is the phase:

| phase | action |
|---|---|
| 0 / 1 | `dev+0x5d3` = 0 / 1 |
| 2 | find the channel of instance `msg+0xc`; `dev+0x5d3` = 1, that channel's `+0x1684` = 1 (task timeout) |
| 3 | lock `dev+0x5d8`; no task in flight → answer `msg+8` = 4 (go ahead) and `dev+0x5d2` = 1; else `dev+0x5d0` = `0x5d1` = 1 (reset pending, done by the last task, §10.2) |
| 4 | `dev+0x5d5` = 0; if `0x5d2`: STREAM_OFF all channels |
| 5 | STREAM_ON all channels; if it was the pending kind: clear `0x5d0`, `0x5d4`, wake the queue; else clear `0x5d2` |
| other | log |

After phases 3–5 the device lock is released; every phase ends with `mi_sys_NotifyWakeupISREvent(dev handle)`.

### 10.4 Force-reset test (proc `force_reset`)
`force_reset <chn> <0|1> [period]` sets `chn+0x1600` (enable), `+0x1604` (period in frames, < 1 → 30); `+0x1608` counts frames.
`_MI_VENC_NeedForceReset(chn)`: the channel's head task is the device's oldest and `count % period == 0` — then
CheckInputTaskStatus treats a running task as timed out (§10.1), exercising the whole reset path.

## 11. The other mi_sys callbacks [R]

What mi_venc fills into the bind capability record is also listed in `mi_sys-1-engine.md` §3.3; the bind-type bits are that
document's (§3.1).

| callback | behaviour |
|---|---|
| `_MI_VENC_OnGetInputPortBindCapability` | capability word `+0xc` = 1 (FRAME_BASE); from SCL (peer module 0x22) also HW_RING (0x10) when the engine's 0x8003001a answer byte `+8` (`dev+0x330`) is set, plus HW_AUTOSYNC (0x8) on chips > 4; REALTIME (0x4) when byte `+0xa` (`dev+0x332`) is set. Channel not created → warning, 0xA0022005. Only `+0xc` is filled |
| `_MI_VENC_OnGetOutputPortBindCapability` | FRAME_BASE (1) only |
| `_MI_VENC_OnBindInputPort` | from SCL: `chn+0x9f0` = the peer's line count, `chn+0xf0` = the PREFER_ALIGN word for SCL (bytes 422 W, 422 H, 420 W, 420 H alignment, high to low, from the 0x80030003 table entries +0x58, +0x5c, +0x40, +0x44); bind type → `chn+0x9f4` (REALTIME → `dev+0x584` += 1). HW_RING needs `SetInputSourceConfig` mode 1 / 2 first ("Please call MI_VENC_SetInputSourceConfig…"), a non-zero ring height (`chn+0x9f8`) and a picture height divisible by it (`venc_check_ring_height` (0x11a4c)); REALTIME / HW_RING on chips < 3 send `mi_sys_SetInputPortSidebandMsg` with the alignments (`chn+0xf0`; format `mi_sys-1-engine.md` §5); then `S_MI_VENC_SetInputPortBufExtConf` (input buffer alignment from the 0x80030003 table, `mi_sys-1-engine.md` §4.2), `chn+0x9f2` = 1 (input bound) |
| `_MI_VENC_OnUnBindInputPortPost` | `chn+0x9f2` = 0; REALTIME → `dev+0x584` −= 1 |
| `_MI_VENC_OnBindOutputPort`, `_MI_VENC_OnUnBindOutputPort` | nothing (return 0) |
| `_MI_VENC_OnPreProcessInputTask` | §7.2 |
| `_MI_VENC_BarrierInputTask` | started channel: `MHAL_VENC_BarrierOneFrame` (NULL slot for H.26x → no-op, JPE has one); stats |
| `_MI_VENC_CheckOutputTaskStatus`, `_MI_VENC_DequeueOutputTask` | empty (4-byte functions): VENC has no mi_sys output buffers, the stream goes through the ring pool |
| `_MI_VENC_NeedIndividualWorkThread` | reads `dev+0x59c`; out = 0; true for H.26x devices (dev < 2) or when no REALTIME bind is active (`dev+0x584` = 0) |
| `_MI_VENC_RejectCmdq` | chips ≥ 5 (M6): reject (return 1) unless a REALTIME bind is active; else / old chips: GetDevConfig 0x80030002 (JPE) × task count → CMDQ size out. `dev+0x21` = the decision |
| `_MI_VENC_OnInputPortFrcChanged` | when no dump is running: compares the new input frame rate with the channel fps (`chn+0x810` / `0x814`); a stable change (same rate twice, ratio ≥ 2 either way) → SetParam **0x80030025** {0xc, 1, realtime-fps mode} |
| `_MI_VENC_OnConfigSramBuf`, `_MI_VENC_AllowPreemptionSram` | report `dev+0x5f8` (SRAM present), `+0x5fc` (size), `+0x5f9`, and `+0x5fa` (preemption allowed) to mi_sys (`mi_sys-1-engine.md` §9) |

The SRAM size actually requested on the running pipeline is not known statically. `E_MMAP_ID_EMI` is reached only for a
REALTIME NV12 / NV21 input wider than the SRAM width cap (§7.4).

## 12. Proc interface, dumps, statistics

"print" = the mi_sys proc print callback (output of `cat` / `echo` on `/proc/mi_modules/mi_venc/mi_venc<dev>`, mechanics
`mi_sys-2-interfaces.md` §10.3); `CamOsPrintf` goes to the kernel log. Format strings are quoted exactly; `\n` = newline.
Code: `_MI_VENC_ProcShowAll` (0x8124), `_MI_VENC_ProcFrameCnt` (0x8270), `_MI_VENC_ProcShowOutInfo` (0x6f68), `_MI_VENC_OnHelp`
(0x196c), `_MI_VENC_OnDumpDevAttr` (0x3840), `_MI_VENC_OnDumpChannelAttr` (0x2360), `_MI_VENC_OnDumpInputPortAttr` (0x1d58),
`_MI_VENC_OnDumpOutPortAttr` (0xf440) [R].

### 12.1 Commands of `/proc/mi_modules/mi_venc/mi_venc<dev>` [R, table `.data`+0x1b8, 21 × {name, argc, fn, slot}]
Registered per created device (§3.1). All are diagnostics. Common entry checks: the handler's table slot (`.bss` pointer to
its 0x10-byte entry) must be set (else 0xA0022006); when fewer tokens than the entry's argc are given, or argv[1] is `-h` /
`help`, the help text is printed (`"\n%s %s"`, name, text) and 0xA002201F returned (show_out: help printed, 0 returned).

| command (argc) | effect |
|---|---|
| `dump_in chn n path` (3) | write the next n input frames to `path/input_res<W>x<H>_dev<d>chn<cc>_seq<nnn>_pixelFormat<f>_pts<pts>.yuv` (`chn+0xa38` count, `+0xa3c` path) |
| `dump_in_big chn n path [bytes]` (4) | same, but only for frames whose encoded size ≥ the threshold or ≥ 2 × the previous frame (`chn+0xb40` count, `+0xb44` path, `+0xbc4` threshold; done in Dequeue) |
| `dump_out chn n path` (3) | write the next n output frames (entries queued on `chn+0xa2c` by the ISR, written in Dequeue; count `chn+0xabc`) |
| `show_all [0|1]` (1) | below |
| `frame_cnt <chn>` (1) | below |
| `drop_out chn d|r` / `drop_out a d|r` (2) | drop (`chn+0xdf`, handled in Dequeue §7.6 step 4) or resume one / all channels' output |
| `drop_in chn d|r` / `drop_in a …` (2) | drop or resume input frames (`chn+0xbc9`, `chn+0xa20`; PreProcess §7.2) |
| `dbg_supfrm chn lvl` (2), `dbg_frmlost chn lvl` (2), `dbg_getout chn lvl` (2) | log levels 0–2 for §8.2, §8.3, GetStream (`chn+0xbcc`, `+0xbd0`, …) |
| `dump_dram chn path` (2) | write the channel's private buffers (reference / work memory from QueryBufSize) to files |
| `dump_map chn times path` (3) | write the applied custom QP / mode map (`chn+0xbd8` count) |
| `run_sh en path` (2) | §10.1 hook (`.data`+0x310 + dev·0x8c) |
| `en_recover en` (2) | §10.1 (`.data`+0x394 + dev·0x8c, default 1) |
| `show_out <chn>` (1) | below |
| `search_window chn hw vw` (3) | "only used for SSC32x series with h26x format": stores a search window for the inter-pred params [R help; I: no effect on M6, where mhal ignores INTER_PRED] |
| `debug_sed chn type en [r]` (3) | print smart-detect (libmi_sed / IVE) info per frame |
| `debug_sram chn en [r]` (2) | print SRAM use per frame |
| `flow_stat chn en` (2) | enable the timing statistics of §12.3 (`chn+0xcb0`) |
| `force_reset chn en [period]` (3) | §10.4 |
| `use_ring_ref on|off` (1) | set the module parameter `use_ring_ref` (§4) at run time ("mhal inplace mode") |

- **`show_all [0|1]`** — prints **no state**: it sets the module-wide flag `.data+0x308` ("show all") used by every dump
  callback below. Argument parsed `%d`, clamped 0…1, reply `"dev:%d set %s to %d\n"`; `-g`/`get` → `"dev:%d %s=%d\n"`
  with the flag; unparsable → `"[ERROR] dev:%d argc:%d, argument error\n"`. Help: `"param:show_all [0-1]\n\t 0:Show only
  requested device info.\n\t 1:Show all used device info. (with created channels)\nFor example:\necho show_all 1 >
  /proc/mi_modules/mi_venc/mi_venc0\n"`. With the flag 0 a dump shows only the requested device's channels; with 1 all
  devices that have a created channel.
- **`frame_cnt <chn>`** — `%d` clamped 0…32, reply `"dev:%d set %s to %d\n"`; then the output frame count of that channel,
  `chn+0x844`, is written with **`CamOsPrintf("%d\n")` to the kernel log**, not to the proc reply; a channel not created →
  `"[ERROR] chn:%d not created error\n"` and 0xA0022005. `get` replies `"dev:%d %s=%d\n"` with a constant **0**; `get`,
  help and argument errors then also log the count of channel 0. Help text: `"show output frame countparam:frame_cnt
  $channel_num\nFor example:\necho frame_cnt 0 > /proc/mi_modules/mi_venc/mi_venc0\n"` (no separator after "count").
- **`show_out <chn>`** — chn ≤ 32 (else `"[ERROR] chn:%d error\n"`, 0xA0022003), created (else `"[ERROR] chn:%d not
  created error\n"`, 0xA0022005), unparsable → `"[ERROR] dev:%d chn:%d argc:%d, argument error\n"`; logs (debug level > 1)
  `"show chn%u output stream info"` and calls `mi_sys_RingPool_DumpRecordInfo(chn+0xc0)` — the text is mi_sys's printk
  (`mi_sys-1-engine.md` §10: per ring `PA VA Size MinSize Align bBufLock`, per record `index bContentClosed bContentRead PA
  VA length extFlags pts seqNum bEOS`). Help: `"\nshow output stream info\nparam:show_out $channel_num\nFor example:\necho show_out 0 >
  /proc/mi_modules/mi_venc/mi_venc0\n"`.
- **`OnHelp`** (`cat` of the help node): `"\n====================== MI_VENC Help =====================\n"`, then for each of the 21
  command-table entries that has a handler: `"\n\e[1;33m%s\e[0m usage: "` with the name, and the handler called with
  argc 4 and argv[1] = `"-h"` so it prints its own help (frame_cnt therefore also logs channel 0's count).

### 12.2 Dumps (`cat` of the device file, mi_sys dump callbacks) [R]
The device dump also prints the `mock_in` path (§4).

#### `_MI_VENC_OnDumpDevAttr` [R]
Devices shown: the requested one, or with show_all every device 0–9 that has a channel bound to it (banner
`"\n------------------------------ All VENC Dev info ------------------------------"`). Then
`"\n------------------------------- VENC%2d Dev info -------------------------------\n"` and three tables (one row per shown
device):
1. header `"%7s%8s%12s%15s%15s%16s%15s%12s%15s%12s\n"`: DevId, IrqNum, IsrTotalCnt, IsrFrmDoneCnt, IsrBufFullCnt,
   IsrSliceDoneCnt, IsrRingFullCnt, IsrReEncCnt, IsrTimeoutCnt, IsrOtherCnt. Row: `"%7d"` dev+4; IrqNum `"%4d/%3d"`
   dev+0x19c / dev+0x1a0 for device 0, `"%8d"` dev+0x1a4 otherwise; `"%12d"` dev+0x560; `"%15d"` dev+0x564; `"%9d,"`
   dev+0x568 followed by `"%4d"` dev+0x570 (two numbers in the BufFull column); `"%16d"` dev+0x56c; `"%15d"` dev+0x574;
   `"%12d"` dev+0x578; `"%15d"` dev+0x57c; `"%12d\n"` dev+0x580.
2. header `"%7s%12s%18s%8s%8s%8s%8s%8s%12s%4s\n"`: DevId, MaxTaskCnt, WorkingTaskCnt, UtilHw, UtilMi, PeakHw, PeakMi, FPS,
   MbRate, " %". Row `"%7d%12d%18d%8d%8d%8d%8d%5d.%02d%12d%4d\n"`: dev+4, u8 dev+0x598, atomic dev+0x59c, u16 dev+0x554,
   +0x556, +0x55a, +0x55c, FPS u16 dev+0x550 "." u16 dev+0x552, MbRate dev+0x18 (sum of the channels' MB rates, §5.2),
   and **`dev+0x18 · 100 / C`** where C = word +4 of the per-device `.rodata` 0x45c record (§1.4: 664 062 for
   devices 0/1, 489 600 for 8/9) — the load in percent of a fixed macroblock-per-second capacity [R reader at 0x3b44…0x3b62;
   I "capacity"].
3. header `"%7s%12s%12s%12s%12s%12s%11s%5s%5s%10s%10s\n"`: DevId, SupportRing, SupportImi, NeedPadding, CmdqHandle,
   CmdqBufSize, ResetTime, MaxW, MaxH, AlignMaxW, AlignMaxH. Row `"%7d%12d%12d%12d%12p%12x%11d%5d%5d%10d%10d\n"`: dev+4;
   the 0x8003001a capability bytes of engine slot 0 (devices 0/1) or 2 (others): +8 (`dev+0x330+slot·0x18`), +0xa
   (`+0x332`), +9 (`+0x331`); `dev+0x3dc` (the cmdq pointer stored by PreProcess); `dev+0x3e8`; `dev+0x5a4`; `dev+0x608`,
   `dev+0x60c`, and the same two again as AlignMaxW/H.

#### `_MI_VENC_OnDumpChannelAttr` [R]
`"\n------------------------------- VENC%2d CHN info -------------------------------\n"`, then tables over the shown
channels (same device/show_all filter), "Chn" = channel index:

| header | row format | values |
|---|---|---|
| `"%5s%15s%15s%15s\n"` ChnId, IsrBufFullCnt, IsrRingFullCnt, IsrReEncCnt | `"%5d%9d,%4d%15d%15d\n"` | chn+0x850 "," chn+0x854, chn+0x858, chn+0x85c |
| `"%5s%7s%8s%10s%12s%9s%9s%11s\n"` ChnId, DevId, bStart, RefMemPA, RefMemSize, AlMemPA, AlMemVA, AlMemSize | `"%5d%7d%8d%10llX%12d%9llX%9p%11d\n"` | dev+4, u8 chn+1, u64 chn+0x30, chn+0x3c, u64 chn+0x40, chn+0x48, chn+0x4c |
| `"%5s%7s%8s%6s%9s%13s%8s%10s%8s%11s\n"` ChnId, State, EnPred, base, enhance, MaxStreamCnt, Fps_1s, kbps1s, Fps10s, kbps10s | `"%5d%7d%8d%6d%9d%13d%5d.%02d%10d%5d.%02d%11d\n"` | State = constant 0; u8 chn+0x188; chn+0x180; chn+0x184; chn+0xd4; u16 chn+0x868 "." u16 chn+0x86a; chn+0x864; u16 chn+0x870 "." u16 chn+0x872; chn+0x86c |
| `"%5s%9s%15s%15s%15s%15s\n"` ChnId, QueryCnt, GetStreamCnt, RlsStreamCnt, PollReadyCnt, PollFailCnt | `"%5d%9d%15d%15d%15d%15d\n"` | chn+0x9dc, +0x9e0, +0x9e4, +0x9e8, +0x9ec |
| `"%5s%7s%7s%7s%7s%10s%10s%15s%20s%10s%10s%10s\n"` ChnId, FrmIdx, BufW, BufH, Stride, ContentW, ContentH, RingStartLine, RingRealTotalHeight, YBufSize, BufSize, Gradient | `"%5d%7d%7d%7d%7d%10d%10d%15d%20d%10d%10d%10d\n"` | u16 chn+0x16a4 … +0x16b2 (last input geometry, §7.2, §7.4), chn+0x16b4, +0x16b8, +0x16bc |
| `"%5s%17s%18s%18s%18s\n"` ChnId, SuperFrmCfgMode, SuperIFrmBitsThr, SuperPFrmBitsThr, SuperBFrmBitsThr | `"%5d%17d%18d%18d%18d\n"` | chn+0x740, +0x744, +0x748, +0x74c |
| `"%5s%10s%10s%10s%10s%10s\n"` ChnId, RoiIdx, RoiX, RoiY, RoiW, RoiH (header once) | `"%5d%10d%10d%10d%10d%10d\n"` | per ROI entry i whose enable byte (entry+0x10) = 1 and W ≠ 0: i, entry+0x18, +0x1c, +0x20, +0x24 (entry = chn+0x318+i·0x28) |
| per channel `"----------------------------chn%u pipeline delay in us----------------------------\n"`, `"%20s%15s%15s%15s%15s\n"` Flow, Min, Max, Avg, Diff | `"%20s%15lld%15lld%15lld%15lld\n"` | "GetStream", u64 chn+0xc88, +0xc90, +0xca0, +0xc80 |
| per channel with `flow_stat` on (`chn+0xcb0`): `"------------------chn%d flow stat time interval in us---------------------\n"`, `"%50s%13s%13s%13s%13s%13s\n"` Flow, Min, Max, Avg, Cur, StatCnt | `"%50s%13lld%13lld%13lld%13lld%13lld\n"` | for the 33 records r = chn+0xcb8+i·0x48: name i, u64 r+0x20, r+0x28, r+0x38, r+0x18, r+0 |

Flow names (`.rodata` 0x198, 33 × {u32 id, char *name}): 0 MI_VENC_IMPL_StartRecvPic, 1 MI_VENC_IMPL_StopRecvPic,
2 MI_VENC_IMPL_Query, 3 MI_VENC_IMPL_GetStream, 4 MI_VENC_IMPL_ReleaseStream, 5 START_TO_FIRST_FRAME_IN,
6 START_TO_FIRST_FRAME_OUT, 7 START_TO_FIRST_FRAME_QUERY, 8 START_TO_FIRST_FRAME_GET_STREAM,
9 START_TO_FIRST_FRAME_RELEASE_STREAM, 10 START_TO_STOP, 11 STOP_TO_START, 12 START_TO_START, 13 STOP_TO_STOP,
14 QUERY_TO_QUERY, 15 GET_STREAM_TO_GET_STREAM, 16 RELEASE_STREAM_TO_RELEASE_STREAM, 17 PREPARE, 18 ENQUEUE, 19 BARRIER,
20 CHECK, 21 DEQUEUE, 22 PREPARE_TO_ENQUEUE, 23 ENQUEUE_TO_BARRIER, 24 BARRIER_TO_CHECK, 25 CHECK_TO_DEQUEUE,
26 DEQUEUE_TO_PREPARE, 27 PREPARE_TO_DEQUEUE, 28 PREPARE_TO_PREPARE, 29 ENQUEUE_TO_ENQUEUE, 30 BARRIER_TO_BARRIER,
31 CHECK_TO_CHECK, 32 DEQUEUE_TO_DEQUEUE [R]. Statistic record layout (0x48, also the GetStream record at chn+0xc68) from
the update code in PreProcess and the print order: +0 u64 count, +8 u64 previous timestamp, +0x10 u64 current timestamp,
+0x18 u64 last interval (Cur/Diff), +0x20 u64 min, +0x28 u64 max, +0x30 u64 sum, +0x38 u64 average (sum / count), +0x40
u8 armed [R offsets; I field names from the headers].

#### `_MI_VENC_OnDumpInputPortAttr` [R]
`"----------------------------- InputPort of dev:%2d -----------------------------\n"`; header (once)
`"%5s%7s%8s%12s%6s%6s%10s%9s%10s\n"` ChnId, Width, Height, SrcFrmRate, MaxW, MaxH, FrameCnt, DropCnt, BlockCnt; row
`"%5d%7d%8d%6d/%-5d%6d%6d%10d%9d%10d\n"`: picture W/H and max W/H through the attr pointers `chn+0x818/0x81c/0x820/0x824`,
frame rate `chn+0x810` "/" `chn+0x814`, `chn+0x838` (enqueued), `chn+0x83c` (dropped: return 2 of PreProcess / Enqueue
drops), `chn+0x840` (blocked: return 1 of PreProcess). A channel with eType > 4 aborts the dump with 0xA0022003.

#### `_MI_VENC_OnDumpOutPortAttr` [R]
`"---------------------------- OutputPort of dev:%2d -----------------------------\n"`, then:
1. header (once) `"%5s%7s%9s %9s%13s%8s%10s%10s%9s%10s%15s%14s%15s\n"` ChnId, CODEC, Profile, BufSize, MinAllocSize, RefNum,
   bByFrame, FrameCnt, DropCnt, ReEncCnt, RingUnreadCnt, RingTotalCnt, UsrLockedCnt; row
   `"%5d%7s%9d %9d%13d%8d%10d%10d%9d%10d%15d%14d%15d\n"`: codec name "H264" / "H265" / "JPGE" / "????" by eType; Profile =
   attr `chn+0x10c` (H.26x, 0 for JPEG); `chn+0x220`; `chn+0x168c`; RefNum = attr `chn+0x120` (H.26x, else 0); u8
   `chn+0x804`; `chn+0x844` (output frames), `chn+0x848` (dropped), `chn+0x84c` (re-encoded); `chn+0xd0` and `chn+0xcc`
   refreshed from the ring pool just before; atomic `chn+0xd8` (streams held by the user).
2. RC tables, printed group by group (group of each RC mode from `.rodata` 0x1160: FIXQP 4/10 → 0, CBR 1/8 → 1, VBR 2/9 →
   2, MJPEG-FIXQP 7 → 3, MJPEG-CBR 6 → 4, AVBR 5/11 → 5, **ABR 3 → 100: never printed**); a group's header is printed
   before its first channel. Values: GOP = attr `chn+0x128`; attr bitrate `chn+0x138` (H.26x CBR BitRate, VBR/AVBR
   MaxBitRate), MaxQp / MinQp = attr `chn+0x13c` / `+0x140` (VBR, AVBR); "[n]" = word n of a fresh GetParam of the RC-advanced
   id (0x8000000d H.264 / 0x8001000b H.265, sub-mode 1–6 as in §6.3); QpMap / AbsQp / ModeMap = bytes `chn+0x1611` /
   `+0x1612` / `+0x1613`. A GetParam failure is logged `"chn%u get param err:0x%x"` and the row printed anyway.

| group | header (all start ChnId, RateCtl) | row: name, values |
|---|---|---|
| 0 | `"%5s%8s%5s%6s%6s%7s%7s%9s\n"` GOP, QP.I, QP.P, QpMap, AbsQp, ModeMap | `"%5d%8s%5d%6d%6d%7d%7d%9d\n"`: "FixQP", GOP, attr `chn+0x134` (IQp), `chn+0x138` (PQp), maps |
| 1 | `"%5s%8s%5s%11s%11s%7s%7s%8s%8s%10s%10s%7s%7s%9s\n"` GOP, MaxBitrate, IPQPDelta, MaxQp, MinQp, MaxIQp, MinIQp, MaxISize, MaxPSize, QpMap, AbsQp, ModeMap | `"%5d%8s%5d%11d%11d%7d%7d%8d%8d%10d%10d%7d%7d%9d\n"`: "CBR", GOP, bitrate, [5], [3], [4], [6], [7], [9], [10], maps |
| 2 | `"%5s%8s%5s%11s%11s%7s%7s%8s%8s%10s%10s%11s%7s%7s%9s\n"` GOP, MaxBitrate, IPQPDelta, MaxQp, MinQp, MaxIQp, MinIQp, MaxISize, MaxPSize, ChangePos, QpMap, AbsQp, ModeMap | `"%5d%8s%5d%11d%11d%7d%7d%8d%8d%10d%10d%11d%7d%7d%9d\n"`: "VBR", GOP, bitrate, [3], attr MaxQp, attr MinQp, [7], [5], [8], [9], [4], maps |
| 3 | `"%5s%9s%9s\n"` Qfactor | `"%5d%9s%9d\n"`: "FixQP", word [3] of GetParam 0x80020003 {0xb0, 2} |
| 4 | `"%5s%9s%9s\n"` Bitrate | `"%5d%9s%9d\n"`: "CBR", attr `chn+0x128` (MJPEG CBR bitrate) |
| 5 | `"%5s%8s%5s%11s%11s%7s%7s%8s%8s%11s%10s%12s%9s%9s%10s%7s%7s%9s\n"` GOP, MaxBitrate, IPQPDelta, MaxQp, MinQp, MaxIQp, MinIQp, ChangePos, MinStPct, MotionSens, MaxISize, MaxPSize, MaxIPProp, QpMap, AbsQp, ModeMap | `"%5d%8s%5d%11d%11d%7d%7d%8d%8d%11d%10d%12d%9d%9d%10d%7d%7d%9d\n"`: "AVBR", GOP, bitrate, [3], attr MaxQp, attr MinQp, [7], [5], [4], [10], [12], [8], [9], [6], maps |
| (ABR) | `"%5s\n"` ChnId | `"%5d\n"` — code present, unreachable through the group table |

Not shown anywhere: CBR MaxIPProp ([8]), VBR MaxIPProp ([6]), AVBR MaxStillQp ([11]) [R].

### 12.3 Flow statistics and stopwatches [R]
With `flow_stat` on, the 33 records of 0x48 bytes at `chn+0xcb8…0x1600` (layout and names above) are updated at the entry of
every callback and ioctl of the channel and printed by the channel dump (§12.2). `_MI_VENC_ReportChnFps` /
`_MI_VENC_ReportDevFps` / `_MI_VENC_ReportDevUtil` are the stopwatch callbacks behind the fps and utilisation figures;
`MI_VENC_IMPL_UpdateStatTimeInfo` keeps the GetStream latency record `chn+0xc68`, fed by ioctl 0x45.

### 12.4 File helpers [R]
`InitKernelEnv`, `OpenFile`, `ReadFile`, `WriteFile`, `CloseFile`, `GetFileAttr` (`filp_open` / `vfs_read` / `vfs_write` /
`vfs_fsync` wrappers with `set_fs(KERNEL_DS)`, old fs in `.bss`+0x210), `_MI_VENC_MapMemToFile(name, va, len)` (used by the
dumps), `PrintHex` / `mi_venc_PrintHex` (hex dump). No file I/O reads `mock_in` / `mock_out` (§4).

## 13. libmi_venc.so — the user side [R]

### 13.1 Library state
Per-channel state of 0x70 bytes for 33 channels × 10 devices: stream-buffer (ring) id, poll fd, mapped phys / VA / size
(the first 0x18 bytes), frame counter, three CamOs mutexes at +0x1c / +0x38 / +0x54 (created in `CreateChn`). Stream calls
run under the channel mutex. Per (dev, chn) a further 0x28-byte custom-map state (§13.5). The fd of `/dev/mi_venc` is opened
lazily and closed by `MI_VENC_DestroyDev` when the create count returns to 0 (`mi-user-abi.md` §2.1–§2.2).

### 13.2 Uniform wrappers
Every per-channel wrapper checks, in this order: dev < 10 else **0xA0022001**, chn ≤ 32 else **0xA0022002**, pointer argument
≠ NULL else **0xA0022006** (error prints only when `/proc/mi_modules/mi_venc/debug_level` ≠ 0), then sends `{u32 dev, u32 chn,
struct by value}` (`mi-user-abi.md` §3); Get variants copy the struct back **only when the ioctl returned 0**.
The functions prog_uvc calls are in `mi-user-abi.md` §3.3 (`MI_VENC_CreateChn`, `MI_VENC_DestroyChn`,
`MI_VENC_StartRecvPic`, `MI_VENC_StopRecvPic`, `MI_VENC_Query`, `MI_VENC_GetStream`, `MI_VENC_ReleaseStream`,
`MI_VENC_RequestIdr`, `MI_VENC_GetFd`, with their payload sizes), plus seven outside that list: `UVC_StartCapture` calls
`SetH264SliceSplit`, `SetH265SliceSplit`, `SetInputSourceConfig`, `SetMaxStreamCnt`; `UVC_StartCapture_photo` calls
`SetJpegParam` / `GetJpegParam`; `St_VencModuleInit` calls `GetH264SliceSplit` / `SetH264SliceSplit` and `SetMaxStreamCnt`
(`prog_uvc.md` §3.2, §7.4). The others:

| function(s) | nr (payload) | note |
|---|---|---|
| `MI_VENC_CreateDev(dev, MI_VENC_InitParam_t *p)` | 0x53 InitDev (0xc: dev, 2 words of p) | open the fd first (fail → 0x1F); count +1 on 0 |
| `MI_VENC_DestroyDev(dev)` | 0x54 DeInitDev (4) | count −1; at 0 close the fd; then for each of the 33 channel states of `dev`: if a cached poll fd > 0 → `MI_SYS_CloseFd(fd)`; zero the first 0x18 bytes (buffer id, poll fd, mapping) |
| `MI_VENC_ResetChn` / `MI_VENC_StartRecvPic` | 4 / 5 (8: dev, chn) | |
| `MI_VENC_StartRecvPicEx(dev, chn, MI_VENC_RecvPicParam_t *p)` | 6 (0xc: dev, chn, `s32RecvPicNum`) | |
| `MI_VENC_SetChnAttr` / `MI_VENC_GetChnAttr` | 9 / 0xa (0x54: dev, chn, attr 0x4c) | Get zero-fills, `memcpy` 0x4c back on 0 |
| `MI_VENC_InsertUserData(dev, chn, u8 *data, u32 len)` | 0xd (0x10: dev, chn, data **pointer**, len) | the kernel reads `len` bytes at the user pointer |
| `MI_VENC_SetMaxStreamCnt` / `MI_VENC_GetMaxStreamCnt` | 0xe / 0xf (0xc) | |
| `MI_VENC_EnableIdr(dev, chn, bool)` | 0x11 (0xc) | |
| `MI_VENC_SetH264IdrPicId` / `MI_VENC_GetH264IdrPicId` | 0x12 / 0x13 (0x10) | |
| `MI_VENC_SetRoiCfg(dev, chn, MI_VENC_RoiCfg_t *c)` | 0x16 (0x24: dev, chn, 7 words) | |
| `MI_VENC_GetRoiCfg(dev, chn, u32 idx, MI_VENC_RoiCfg_t *c)` | 0x17 (0x28: dev, chn, idx, 7 words out) | the index is an extra argument |
| `MI_VENC_SetRoiBgFrameRate` / `MI_VENC_GetRoiBgFrameRate` | 0x18 / 0x19 (0x10) | |
| `MI_VENC_Set/GetH264{InterPred 0x1c/0x1d (0x18), IntraPred 0x1e/0x1f (0x18), Trans 0x20/0x21 (0x14), Entropy 0x22/0x23 (0x10), Dblk 0x24/0x25 (0x14), Vui 0x26/0x27 (0x24)}` | | |
| `MI_VENC_GetH265SliceSplit` 0x29 (0x10); `MI_VENC_Set/GetH265{InterPred 0x2a/0x2b (0x28), IntraPred 0x2c/0x2d (0x18), Trans 0x2e/0x2f (0x14), Dblk 0x30/0x31 (0x14), Vui 0x32/0x33 (0x24)}` | | **`MI_VENC_GetH265InterPred` never copies the result back** — the caller's struct is left unchanged although the ioctl ran (disassembly at 0x6a4c: after `ioctl` only the return path) |
| `MI_VENC_SetRcParam` / `MI_VENC_GetRcParam` | 0x36 / 0x37 (0x40) | |
| `MI_VENC_SetRefParam` / `MI_VENC_GetRefParam` | 0x38 / 0x39 (0x14) | |
| `MI_VENC_SetCrop` / `MI_VENC_GetCrop` | **0x3b / 0x3a** (0x1c: dev, chn, 5 words) | nr order reversed against the usual pairing; the kernel table agrees |
| `MI_VENC_SetFrameLostStrategy` / `MI_VENC_GetFrameLostStrategy` | 0x3c / 0x3d (0x18) | |
| `MI_VENC_SetSuperFrameCfg` / `MI_VENC_GetSuperFrameCfg` | 0x3e / 0x3f (0x18) | |
| `MI_VENC_SetRcPriority` / `MI_VENC_GetRcPriority` | 0x40 / 0x41 (0xc) | |
| `MI_VENC_SetAdvCustRcAttr(dev, chn, MI_VENC_AdvCustRcAttr_t *a)` | 0x4d (0xc: dev, chn, the 4 bools as one word) | on 0 the library keeps `a->bEnabelHistoStaticInfo` in its per-channel custom-map state (gates `GetLastHistoStaticInfo`) |
| `MI_VENC_SetSmartDetInfo` | 0x50 (0x30) | imported by libmi_sed (unused) |
| `MI_VENC_SetIntraRefresh` / `MI_VENC_GetIntraRefresh` | 0x51 / 0x52 (0x14) | |

Importers of these on the camera: libmi_iqserver Set / GetChnAttr, Set / GetRcParam (besides the app-used ones); libmi_sed
Set / GetChnAttr, SetRoiCfg, SetSmartDetInfo; prog_uvc none of them [R imports].

### 13.3 GetStream and ReleaseStream
`MI_VENC_GetStream(dev, chn, pstStream, s32MilliSec)`; checks dev < 10, chn ≤ 32, `pstStream`, `pstPack`, `u32PackCount` ≠ 0,
timeout ≥ −1:
1. venc **0x42** `{dev, chn, u32 bufId (out)}` → id of the output (bitstream) ring. If it differs from the cached one, the old
   mapping is `MI_SYS_Munmap`ped.
2. If not mapped: venc **0x43** `{u32 bufId, +8 u64 phyAddr (out), +0x10 u32 size (out)}`, then `MI_SYS_Mmap(phy, size, &va)`
   (the sys ioctl 0x0a maps it into the process; no `mmap()` in the library) and `MI_SYS_FlushInvCache(va, size)`.
3. Loop per pack, venc **0x45** (0x50, fields §13.4). Pack i (0xb8, `MI_VENC_Pack_t`): `phyAddr`, `pu8Addr = va + (phy −
   mapPhy)` (cache invalidated for `len`), `u32Len`, `u64PTS`, `bFrameEnd`, `u32Offset = 0`; `pstStream->u32Seq` = seq; the
   frame counter is kept by the library.
4. On the first pack: venc **0x0b** GetStream `{dev, chn, Stream 0x48, s32MilliSec}` (the kernel's bookkeeping, §5.4); then
   **`_MI_VENC_ParseNalu`** scans the first ≤ 0x708 bytes for start codes and fills `stDataType` and up to 8 `asackInfo[]`
   entries (NAL type, offset, length): H.264 types 1 / 5 (slice), 6 / 7 / 8 / 14 (SEI / SPS / PPS / prefix); H.265 types 0–2 → 1,
   19 (IDR), 32 / 33 / 34 / 39 (VPS / SPS / PPS / SEI). Unknown type → 0xA002201F.
5. Stop at frame end or when `u32PackCount` is reached. Error paths release with venc **0x48** `{bufId, dev, chn}` and **0x0c**
   `{dev, chn, zeroed Stream_t (0x48)}`.

`MI_VENC_ReleaseStream(dev, chn, pstStream)`: per pack `FlushInvCache(pu8Addr, u32Len)` and venc **0x48** `{bufId, dev, chn}`;
clear `pstStream` VA; venc **0x0c** with a zero-filled `Stream_t` (the caller's packs are not sent). The mapping stays until
the id changes or `DestroyChn` (which unmaps). `MI_VENC_StopRecvPic` retries every 10 ms up to 100 times while the kernel
answers BUSY.

### 13.4 ioctl 0x45 `RingPool_GetHeadRecord` — every field
<!-- ev:mi_venc.13.4 -->
Kernel side §5.4 (`MI_VENC_IOCTL_RingPool_GetHeadRecord` in); "record" = the 0x50-byte
ring content record the kernel task built (§7.1, §7.6 step 8).

| payload | ← record | meaning | what libmi_venc does with it (`MI_VENC_GetStream`) |
|---|---|---|---|
| +0x00 u32 | — | ring handle (from 0x42) | in |
| +0x08 u64 | +0x00 extFlags | bit 0 frame end, bits 1–3 codec (2 H.264, 3 H.265), bits 4–6 reference type | frame end → pack `bFrameEnd = 1`; codec selects `_MI_VENC_ParseNalu`; at frame end the ref type goes to `Stream.stH264Info.eRefType` (+0x3c) for H.264 or the H.265 info word at Stream +0x34 |
| +0x10 u64 | +0x08 pts | | pack `u64PTS` |
| +0x18 u64 | +0x10 seqNum | | `Stream.u32Seq` |
| +0x20 u64 | +0x18 u8 **bEOS** (rest 0) | end-of-stream record | non-zero → pack `bFrameEnd` is overwritten with **2** (not a frame end) or **3** (frame end) and no further pack is read after it |
| +0x28 u64 | +0x20 PA | | pack `phyAddr`; `pu8Addr = mapVA + (PA − mapPA)`, cache invalidated for `len` |
| +0x30 u32 | +0x2c u32Length | | pack `u32Len` |
| +0x34 s32 | — | timeout ms | in |
| +0x38 u32 | — | dev | in (statistics) |
| +0x3c u32 | — | chn | in |
| +0x40 u8 | +0x30 | **frame QP** | at frame end → pack `u8FrameQP` (+0x28) |
| +0x44 s32 | +0x34 | **POC** | at frame end → pack `s32PocNum` (+0x2c) |
| +0x48 u32 | +0x38 | **gradient** | at frame end → pack `u32Gradient` (+0x30) |

mi_sys does not interpret the record fields +0x30…+0x3b; the names come from where the library stores them in
`MI_VENC_Pack_t` (header) and match what the kernel task writes there (QP, POC, gradient, §7.1) [R store targets; I meaning =
header names]. The library also numbers the packs of one frame: `asackInfo[0].u32SliceId` (pack +0x40) = 1, 2, … (counter
reset at frame end), `u32Offset` / `u32DataNum` = 0, `Stream.hMiSys` = the VA of the last pack.

### 13.5 Functions with their own logic
- **`MI_VENC_DupChn(dev, chn)`** (no ioctl of its own): (re)initialises the library's per-channel state — the three mutexes,
  zero buffer id / mapping / counter — then tail-calls `MI_VENC_StartRecvPic(dev, chn)`. Meant for a second process attaching
  to a channel another process created [I].
- **`MI_VENC_CloseFd(dev, chn)`**: argument check only, returns 0 — the poll fd from `MI_VENC_GetFd` (`mi-user-abi.md` §5) is
  **not** closed (only `DestroyDev` closes cached poll fds).
- **Custom QP / mode maps** — per (dev, chn) a 0x28-byte state: `+0 u8 histoEnabled, +4 u32 size, +8 u64 phys0, +0x10 u64
  phys1, +0x18 va0, +0x1c va1, +0x20 u8 free0, +0x21 u8 free1` (under the channel mutex at state +0x1c).
  `MI_VENC_AllocCustomMap(dev, chn, u64 *phy, void **va)`: first call → ioctl **0x4a AllocCustomMaps** (0x20: dev, chn, out +8
  size, +0x10 u64 phys0, +0x18 u64 phys1), `MI_SYS_Mmap(phys, size, &va, uncached)` for both, both marked free; every call
  returns the first free map (phys + va), or **0xA002200F** (BUF_FULL, prints "MI_ERR_VENC_BUF_FULL") when both are in use.
  `MI_VENC_ApplyCustomMap(dev, chn, u64 phy)`: payload `{+0 = 0 (the dev argument is not sent), +4 chn, +8 phy low word; +0xc
  uninitialised}`; ioctl **0x4b GetAsyncCustomMap** (−1 → **0xA0022012**), on 0 ioctl **0x4c ApplyCustomMap** with the same
  payload; on 0 the applied map becomes busy and the other one free (double buffering); a phy that is neither map →
  **0xA0022011**. `MI_VENC_FreeCustomMap(dev, chn)`: unmaps both (no map → **0xA002201F**), clears the state and **destroys
  the channel mutex** (later calls on the channel use a destroyed mutex until `CreateChn` / `DupChn` re-inits it).
- **`MI_VENC_GetLastHistoStaticInfo(dev, chn, void **pp)`**: `*pp` = a library-static 0x30-byte buffer for (dev, chn); if
  `histoEnabled` is 0 → **0xA0022007** ("not config"); else ioctl **0x4e** (0x38: dev, chn, out 0x30) and on 0 copies the 0x30
  bytes into the static buffer under the channel mutex. `MI_VENC_ReleaseHistoStaticInfo` = argument check, 0.
- `MI_VENC_DestroyChn` also calls `MI_VENC_FreeCustomMap`; `MI_VENC_CreateChn` maps the kernel's 0xA0022004 (EXIST) to 0.
