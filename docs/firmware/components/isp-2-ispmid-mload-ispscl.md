# isp-2-ispmid-mload-ispscl — ispmid, ISP statistics, MLOAD and ISPSCL of mhal.ko
<!-- ev:isp-2-ispmid-mload-ispscl -->

Four parts of the ISP code in `mhal.ko` that sit between `mi_isp` and the ISP hardware. **ispmid** is the interface
`mi_isp` calls (`MHalVpeIsp*`, `MHalIspApi*`): camera open and close, the per-frame flow driven from the ISP's own
interrupts, the per-frame register batch and the ISP's own CMDQ engine, work queues that land sensor and ISP settings on the
right frame, and the device nodes `/dev/ispmid`, `/dev/isp_fe`, `/dev/ispmid_cusapi`. **statistics** is the AE / AWB /
histogram / AF / RGB-IR / NE statistics engines with their DMA area, and the services that hand the statistics to user 3A.
**MLOAD** (menuload) is the driver of the ISP's table-DMA engine (bank 130D) that copies lookup tables into ISP SRAMs.
**ISPSCL** is a small HAL over the YUV→Bayer converters, the IspScl scaler/CSC units, the ISP→SCL realtime YUV pipe and the
VIF-WDMA interrupt group in bank 1342. Not here: the ISP hal/driver layer (interrupt controller bit map, banks 1302/1303
field by field, DMA engines, 3DNR, bank map) — isp-1-hal.md; the ISP API ids and their payloads, 0x2Exx included —
isp-3-iq-api.md; the IQ engine, `Regmap` and the IQ file — isp-4-iq-engine.md; the in-kernel AE / AWB / AF — isp-5-3a.md;
libcus3a and the user 3A frame — isp-6-userspace.md §2; mhal's block map, ISRCB registry, character devices and module
parameters — mhal-core.md; the caller `mi_isp` — mi-capture.md §4; the VIF side of ISPSCL — mhal-vif.md §13; the CMDQ interface
table and trigger-bus events — mhal-scl-jpe-cmdq.md §2.3, §2.8.

Tags: **[R]** read from the binary, its disassembly or decompile, a data section, the boot log, a live log or a cited source
file; **[S]** stated by a source; **[I]** inference. A heading tagged [R] means every statement under it is [R] unless marked
otherwise.

Binary: `/config/modules/4.9.227/mhal.ko`. Callers read from `mi_isp.ko` (mi-capture.md) and
prog_uvc (libcus3a statically linked).

Every `switch` quoted here was decoded from the disassembly (the jump tables in the rebased decompiles are not
usable as printed); straight-line decompiled code is read as is.

Addresses: `.text` / `.data` / `.rodata` / `.bss` offsets of the relocatable module.

Register notation: `BANK.rr` = RIU bank (hex), 16-bit register `rr` (hex), physical address
`0x1F000000 + BANK·0x200 + rr·4`; `[h:l]` a bit field (RIU addressing itself: kernel-platform.md §3.1). The ISP HAL helpers
`ISP_RegW/RegWM(bank, r, …)` take `r` as twice the RIU register number (RIU register = `r/2`); ISPSCL's `ISPSCL_RegWM`
takes the same doubled offset (§13.2). ISP, ISPSCL and MLOAD each map the RIU as `ioremap(0x1F000000, 4 MiB)`.

Function addresses written 0x400xxxxx are these offsets + 0x40000028 (image rebased to 0x40000000; 0x4007f5dc =
`.text 0x7f5b4`).

## 1. The parts [R sizes and names; grouping I]

The object files of the ISP library are linked one after another, so the sub-parts are address ranges, refined by name for
statistics. MLOAD and ISPSCL are blocks of their own in mhal's block table (mhal-core.md §2.1): MLOAD `.text`
0x9ede8–0xa0934, ISPSCL `.text` 0x8de98–0x8ff6c. The rest of the ISP code (hal/driver, in-kernel 3A, IQ engine, IQ API) and
the whole-ISP size table: isp-1-hal.md §1.1.

| sub-part | KB | funcs | exports | what |
|---|---|---|---|---|
| statistics | 18.7 | 190 | 41 | AE / AWB / histogram / AF / RGB-IR statistics engines: windows, sources, buffers, HDR switching; ispmid's statistics services |
| ispmid | 56.2 | 360 | 35 | `MHalVpeIsp*` / `MHalIspApi*` (mi_isp's entry points), `MS_CAM_*`, `Camera*` frame flow, CMDQ batch, work queues, `/dev/ispmid`, `/dev/isp_fe` |
| menuload | 6.8 | 98 | 19 | MLOAD block: `IspMLoad*`, `DrvIsp_MLoad*`, `Mload*`, `/dev/mload` — DMA of lookup tables into ISP SRAM |
| ispscl | 8.2 | 84 | 8 | ISPSCL block: `DrvIspScl_*`, `HalIspScl_*` — YUV→Bayer front, IspScl units, YUV-pipe settle, VIF-WDMA interrupt group, `/dev/ispscl` |

**statistics** — `DrvIsp_3A_StatisticConfig/Disable`, `DrvIsp_SetAeWin/SetAwbWin/SetHistoAttr/SetHistoWins/SetAf*` (AF:
window, ROI mode, filters, pre-filter, Y map, LDG), `DrvIsp_AWB_*` / `DrvIsp_Histo_*` (enable, source, HDR pipe switching),
`DrvIsp_GetStatisBufInfo/CopyStatisBufInfo`; HAL `ISP_AE_*`, `ISP_AWB_*`, `ISP_AF_*` (AF filter set-up is the largest, ~5 KB),
`ISP_HISTO_*`, `ISP_RGBIR_*`, `ISP_MOT_*`, `HalIsp_HwStatistics*` (one DMA area per frame: sizes, offsets and buffer info for
AE, AWB, histogram, AF, RGB-IR, NE). ispmid side: `CameraIsp_StatisticsAllocate/ReleaseBuffer`,
`CameraIspAlgo_3A_StatisticInit/DeInit`, `CameraGetAe/Awb/Histo/Af/RgbIrHistoStats`, `CameraCapture3AStatistic`,
`CameraSet/Get(AF*|AeWin*|AwbWin*|HistoWins)`.

**ispmid** —
- mi_isp's entry points `MHalVpeIsp*`, `MHalIspApiSet/Get` (§3) and the older `MS_CAM_*` API layer they call
  (`MS_CAM_IspApiSet/Get`, statistics / AE / AWB / AF accessors);
- frame flow: `MhalCameraOpen/Close`, `CameraContextSwitch*`, `FrameStartProc`, `FrameDoneProc[FrameMode]`,
  `SensorFrameDoneProc`, `CameraIspAlgo_*` (3A / IQ glue: `UpdateIQ`, `IQProcFrame`, `DoAe/DoAwb/DoIq/DoIqPerFrame`,
  `LoadIQFile`), HDR-linear AE conversion, 3DNR buffers, raw capture / fetch (debug and frame mode);
- CMDQ: `CameraIspCmdqBuffer_*` (the per-frame register batch, "index mode" master/slave), `CameraCmdq*` (register writes as
  CMDQ commands), `CameraChangeMhalCmdqIF`;
- work queues: `IspMidThreadWq*` / `IspMidWork*` (sensor shutter / gain, digital gain, IQ shadow, AE output applied at the
  right frame), `IspFrameCtxtRingBuf*`, `BufPool*`;
- devices: `/dev/ispmid` (+ `ispmid0`, sysfs `isproot`, `sclen`, `info`; `ispmid_ioctl` 2.2 KB, `ispmid_fwrite` 4.5 KB debug
  console), `/dev/isp_fe`, `/dev/ispmid_cusapi` (`AddCusApiNode`, `IspMid_CusApi_*`: poll / read / write); IQ file I/O
  `CameraReadIqData` / `OpenFile` / `ReadFile`.

**menuload** and **ispscl**: §12, §13.

### 1.1 How it runs on this camera [R unless tagged]

The camera runs one ISP channel in **realtime** (VIF → ISP directly, open mode 0, §3.2):
- **The ISP programs itself from its frame-start interrupt.** `FrameStartProc` runs in the "IspDriverThreadWq" kernel
  thread, collects every register write of the frame (IQ, digital gain, sensor works) into a RAM command batch ("buffer
  mode") and pushes the batch into **the ISP's own CMDQ engine** (taken with `MHAL_CMDQ_GetFreeId` at `MHalVpeIspInit`)
  behind a *wait for ISP_VSYNC* command; the writes land at the next VSYNC (§6, §7).
- **`MHalVpeIspProcess` does nothing in realtime** unless output port 1 (YUV DMA) or port 2 (IR DMA) of the process
  descriptor is enabled; this pipeline enables neither, so the call returns 1 at once. The mi_sys CMDQ handed to the ISP
  carries no ISP register writes. `CameraContextSwitch` is frame mode only (§8).
- **What mi_isp learns per frame** from `MHalVpeIspCheckIrq(mode 1)`: code 4 at every frame start and code 1 at every ISP
  idle (frame end) (§5).
- **3A runs in user space.** prog_uvc's libcus3a sends `MI_ISP_CUS3A_Enable{AE 1, AWB 1, AF 1}` (live log), mhal sets
  `nFlagCus3A = 0x1A`, and `CameraIspAlgo_DoAe/DoAwb` skip the in-kernel `DoAe/DoAwb` when those bits are set. The kernel
  keeps doing statistics (§10), wakes user space through `/dev/isp_fe` (§11), serves statistics on request and applies the
  exposure / WB results. The IQ engine stays in the kernel and runs every frame (isp-4-iq-engine.md §3.5).
- **Per frame prog_uvc's user 3A reads** 0x2E01, 0x2E02, 0x2E03, 0x2E1E, 0x2E05 (AE), 0x2E07, 0x2E09 [, 0x2E19] (AWB) and
  writes 0x2E06 (AE result) and 0x2E0A (AWB result); the AWB ids are read and written every frame but the built-in AWB
  algorithm computes on every third call (`SigmaAwbRun`) and the other frames send Change = 0, which the kernel ignores. The
  user AF runs too: libmi_isp's `MI_ISP_EnableUserspace3A` calls `CUS3A_EnableUserspaceAF`, which registers libcus3a's
  built-in Sigma AF adaptor; on the kernel's AF event libispalgo `AfInit` / `DoAf` run every frame, `AfInit` pushes window /
  filter defaults through 0x2E0E–0x2E11 / 0x2E24–0x2E28, 0x2E21 is written every frame (no VCM — no effect) and a failing
  0x2E20 read is ignored. The ids and their payloads: isp-3-iq-api.md §5; the user side: isp-6-userspace.md §4.
- **AE results land on the sensor `max(shutter delay, gain delay)` frames later** through the per-channel work queue
  (§6.2); AWB gains (stored by `CameraSetAwbParam` at ch+0x146…) go into the next IQ run (`DoIq` / `DoIqPerFrame` →
  WB-gain shadow) [I: written to the WB-gain registers through `ISP_IQ_WBG_Cfg`, isp-1-hal.md §10.2].
- `/dev/ispmid` (ioctl + text console) and `/dev/ispmid_cusapi` are vendor test interfaces; the cusapi node's only
  producer (`TestIspApiCmd`) has no caller (§9).
- MLOAD loads the dirty IQ tables in the same per-frame batch (§12.6); ISPSCL is initialised all-off and stays off for
  this Bayer sensor (§13.9).

## 2. ispmid objects [R]

### 2.1 Global objects

| object | what |
|---|---|
| `gpVpeIoBase` (0x38 B) | +0 RIU base (defaults to 0xFD000000 at the first `CreateIspInstance`), +4 instance reference count (mutex created at 0→1, destroyed at 1→0), +8 **`g_MhalIspMutexLock`** (every `MHalVpeIsp*` entry point that touches the camera holds it), +0x30 debug level byte (`MHalVpeIspDbgLevel`), +0x34 "initialised" flag (`MHalVpeIspInit/DeInit` are idempotent through it) |
| `g_tIspMidHandler` | the ispmid state. Word [0] = channel being processed; +4 mutex (taken by `FrameStartProc`, `FrameDoneProc`, open / close, statistics getters); words [ch+0x3c] = **channel data pointer** for 32 channels; +0x48 copy of mi_sys's 5 memory ops (alloc, free, map, unmap, flush — +0x58 = flush, called on statistics buffers); +0x5c current CMDQ interface (the "target" of register writes), +0x60 the ISP's own CMDQ interface, +0x64…+0x74 the 5-slot write table handed to the HAL, +0x78 buffer mode on/off, +0x7c largest batch seen, +0x80 commands written; +0x84 frame-context ring (frame mode); +0x88 the sensor work-queue thread "IspMidThreadWq"; +0xe4…+0xec frame counters; +0xe98 PTS table (16 × {u64 pts, slot}); +0xf98 metadata pool (16 × {shutter, sensor gain, colour temperature}); words [ch+0x5c] = per-channel **`nFlagCus3A`**; +0x270…+0x2ac command-batch buffers (§7); +0x2b0…+0x2cc VIF-event callback handles; +0x338 debug control flags (bit 0 = never write the sensor, bit 1 = skip `FrameStartProc` / `FrameDoneProc`, bit 2 = force ack-tie, bit 7 = run IQ in frame mode, bit 8 = 3DNR "PV" enable, bit 9 = apply bypass every frame, bit 10 = HDR buffers); +0x33c CUS3A bypass mask (§10.2); +0x3cc per-channel 0x288-byte backup of the 3A window set-up; +0xd60…+0xe80 `g_IspAlgoInput` (the input blocks of the in-kernel AE / AWB / IQ) |
| channel data (0x1C320 B, one per opened channel) | §2.2 |
| `IspDriverThreadWq` (hal/driver, `_g_tIspDriverThreadWqPool`) | kernel thread "IspDriverThreadWq", priority 99; ring of 256 {fn, a0, a1, valid}; 5 s idle timeout; fed from the ISR (§5). Also holds the IRQ number parsed from DT (`DrvIsp_GetIrqNum`) |
| `IspMidThreadWq` (`g_tIspMidHandler+0x88`) | kernel thread "IspMidThreadWq", priority 99; ring of 256 {fn, a0, a1, a2, valid}; fed by the VIF "sensor frame done" callbacks (frame / HDR modes only) |

### 2.2 Channel data [R offsets and use; names I where no string names them]

The fields the interface and the per-frame path use (`ch` below = a channel data pointer):

| off | field |
|---|---|
| +0x00 | dev id |
| +0x04 | `nCtrlFlag` (printed under that name): bits 0–1 IQ engine on, bits 2–3 menuload / WDR tables, bit 4 AE, bit 5 AWB, bit 6 AF; set by `CameraIsp_InitChannelFmt` — 0x7f on the channel's first open unless the override word `g_tIspMidHandler[ch + 0x7c]` (bit 31 = pending, written only by the "ctlflag" debug command) is set, YUV modes → 10, no sensor → bits 4–6 cleared, ch+0x19730 = 0 → bit 4 cleared (isp-6-userspace.md §1.1) |
| +0x08 | u64 frame counter (incremented at frame start in realtime) |
| +0x10 | open mode (§3.2: 0/1 realtime, 2/3 Bayer frame mode, 5 YUV frame mode) |
| +0x18 / +0x20 | u64 VIF sensor frame count (now / last seen) |
| +0x28 | AE handle (`AeInit`) — exists even with user AE |
| +0x88…+0x114 | AE state: +0x8c "result valid", +0x90 shutter (µs), +0x94 sensor gain, +0x98 ISP gain, +0x9c / +0xa0 / +0xa4 the same for the HDR short frame (gain 1× = 0x400), +0xa8 BV, +0xac…+0xb8 brightness, +0x10c target FPS, +0x110 u16 F-number ×10 |
| +0x120 | AWB handle (`AwbInit`) |
| +0x146 / +0x148 / +0x14a / +0x14c / +0x14e | u16 WB gains R, Gr, Gb, B (1× = 0x400) and colour temperature |
| +0x190 + k·0x8710 (k = 0…2) | IQ handle of IQ slot k; the slot's register shadow starts at +0x1a0 + k·0x8710, its 46 IP dirty flags at shadow+0x4d20 |
| +0x196c4 | CUS3A bits copied from `nFlagCus3A` (bit 1 user AE, bit 3 user AWB, bit 4 user AF) |
| +0x196cc…+0x196fc | 3A pacing: AE period 3, per-period masks (AE 7, AWB 7, IQ 3, …), work offsets (shutter, gain, digital gain at +0x196f8, IQ shadow — §6.2) |
| +0x19770 | sensor id (pad); +0x19778 u8 opened; +0x1977c the in/out-mode and statistics configuration block (0x288 B; AE window config at +0x70, AE blocks X / Y at +0x74 / +0x76 (= ch+0x197f0 / 0x197f2), AWB blocks X / Y at +0x7c / +0x7e; the statistics enables and sources of §10.2) |
| +0x199f8 | ack-tie flag (`CameraIspConfigAckTie1`) |
| +0x19a10…+0x19a4c | statistics: DMA area (virt, phys, MIU, size), pool, CPU copy, the 10 per-statistic pointers at +0x19a28 (§10.1) |
| +0x19b58… | "ISP_WDR" DMA buffer VA / PA (§12.6) |
| +0x19b6c / +0x19b70 | menuload handle / current menuload frame (§12.6) |
| +0x19b74 | per-channel work queue (`IspMidWork`, §6.2) |
| +0x19c40…+0x19c7c | frame-mode bookkeeping, FPS measurement (+0x19c6c fps×1000) |
| +0x19e7c | copy of 1321.7E taken at every frame start: the IQ engine's debug command word; value 0xDB = ispmid "freeze" code (isp-4-iq-engine.md §11.1) |
| +0x19eb0 / +0x19eb4 | current IQ slot (0…2) / shadow history slot (0…8, history at +0x19eb8 + n·0x400) |
| +0x1c2c8…+0x1c318 | IQ-shadow / menuload apply counters, index-mode flags (+0x1c2d0 menuload result, +0x1c2e4 menuload success counter, +0x1c2f4 index mode on — §7) |

## 3. The interface mi_isp calls [R]

### 3.1 Exports

All return MHAL "bool": 1 = success, 0 = failure (`MHalIspApiSet/Get` return the API status instead). `inst` is the
0xA4-byte instance of §3.2. The order in which mi_isp calls them, and its side of each call: mi-capture.md §4.4, §4.7. Per frame
mi_isp's input task (`EnqueueInputTask`, run by the mi_sys worker) calls `MHalVpeIspNextFramePTS` and `MHalVpeIspProcess`.

| export | args | what it does |
|---|---|---|
| `MHalVpeIspInit()` | — | once (flag in `gpVpeIoBase+0x34`): `CameraInit` = `DrvIsp_Resume` (power, clocks) + `CameraCmdqGetCmdqService` (the ISP's own CMDQ engine: `MHAL_CMDQ_GetFreeId`, `MHAL_CMDQ_GetSysCmdqService(id, {size 0x20000, align 0x20}, forceRIU 0)`; failure = kernel panic) |
| `MHalVpeIspDeInit()` | — | `CameraDeInit`: release the CMDQ service and id, `DrvIsp_Suspend` |
| `MHalVpeIspGetIrqNum(dev, u32 *irq)` | | `CameraIsp_GetIrqNum`: the DT interrupt of `isp` (GIC SPI 0x39) as parsed by `isp_probe` |
| `MHalVpeIspEnableIrq(ctx, mode, en)` | mode 1 | `CameraIspEnableIrq` → `HalIspEnableIrq` → `HalIspEnableIrqMapTable`: only mode 1 does anything — unmask / mask **1302.28 bit 10** (interrupt group 1, bit 10, ISP_IDLE). All other ISP interrupt bits are set by mhal itself (§5) |
| `MHalVpeIspCheckIrq(ctx, mode)` | mode 1 | returns the status code of §5 **and runs the whole ISP interrupt decode (`DrvIspISR`)** — mhal's ISP interrupt handler is this call, made from mi_isp's `_MI_ISP_IMPL_Isr` |
| `MHalVpeIspClearIrq(ctx, mode)` | | no-op, returns 1 (clearing happens inside the decode) |
| `MHalVpeCreateIspInstance(ops, {u32 dev, u32 chn}, &inst)` | ops = 5 mi_sys memory functions | allocate and zero the instance, copy dev and chn, copy the ops (alloc / map / flush and the rest); create the global mutex on the first instance; always 1 |
| `MHalVpeDestroyIspInstance(inst)` | | `MhalCameraClose(inst.chn)`, free, drop the reference |
| `MHalVpeIspSnrEarlyInitParam(inst, p)` | 0x48-byte block | copied to inst+0x44 if `p+4` (size) ≠ 0, else cleared and 0 returned. The copy reaches `MhalCameraOpen` (+0x64 of the open attribute) but `CameraIspAlgo_SnrEarlyInitParam` is an empty stub in this build — **no effect**. mi_isp calls it only when it has IQ-version data |
| `MHalVpeIspRotationConfig(inst, u32 *rot)` | | stores `*rot` at inst+0x40; used at the next open and by the DMA ports (rotation codes 1, 3, 6, 7 swap width and height; for the IQ engine 1/3 → 4, 2 → 3, 4 → 1, 5 → 2, 6/7 → 5 in `DrvAlgo_IF_IQ_SetMirrorFlip`). How mi_isp derives the code from mirror / flip / rotate (`mi_isp` `isp_rotation_code` (0x10ed8)) is not read |
| `MHalVpeIspInputConfig(inst, cfg)` | §3.2 | fails (0) if `cfg+0x18` (input type) is 0; copies the fields; for input type 1…3 → `_MHalIspInit` (pixel format → Bayer id / bits / YUV order) → **`MhalCameraOpen`** (§4) under the global mutex; returns open == 0 |
| `MHalVpeIspConfigAckTie1(inst, notSclRealtime)` | | inst+0x94 = flag; `CameraIspConfigAckTie1`: channel +0x199f8 = flag, `DrvIsp_ConfigAckTie1` (ISP → SCL handshake select, used when port 0 is not realtime to SCL) |
| `MHalVpeIspNextFramePTS(inst, cmdq, u64 *pts)` | | `CameraIsp_SetNextFramePTS`: store the PTS in the 16-entry table keyed by `frame & 0xF` (a repeat of the same PTS only updates the slot) — this is what lets 0x2E17 return the exposure of the frame with a given PTS (isp-3-iq-api.md §5.1) |
| `MHalVpeIspProcess(inst, cmdq, desc)` | §3.2 | realtime (input type 3): **if both DMA-port blocks are disabled return 1 and do nothing**; else, with `cmdq` as the write target in buffer mode, configure the IR / YUV DMA ports (`_MHalVpeIspProcessBayer`) and call `CameraContextSwitch` (a no-op in realtime). Frame mode: §8. The "< 0x975 free CMDQ entries" refusal is mi_isp's, not mhal's |
| `MHalVpeIspInputClose(inst)` | | `MhalCameraClose(inst.chn)` under the mutex (mi_isp's ResetDevPass) |
| `MHalIspApiSet/Get(dev, chn, id, size, data)` | | `MS_CAM_IspApiSet/Get` under the mutex (route and ids: isp-3-iq-api.md §1.2, §2) |
| `MHalVpeIspGetCaps(inst, u32 *caps)` | | caps = 0x1B00 [meaning not known]; no MI caller |
| `MHalVpeIspAssignMetadataBuffer(inst, buf)` | | fills `buf` with the AF ROI mode, the 16 AF windows and a copy of the AF statistics (0x1B00 B at +0x10c); no MI caller [unused] |
| `MHalVpeIspDbgLevel(u8 *)` | | stores the byte; no MI caller |
| `MHalVpeIspExtDmaPortConfig`, `MHalVpeIspIrDmaPortConfig` | inst, port block | exported, called from `MHalVpeIspProcess` (§8) |

### 3.2 Structures [R layouts; field names from mi_isp's log strings where quoted]

**Instance** (0xA4): +00 dev | +04 / +08 crop x / y (always 0) | +0c width | +10 height | +14 input type | +18 pixel format
(mhal code) | +1c precision | +20…+33 the 5 memory ops | +34 HDR mode | +38 channel | +3c 3DNR level | +40 rotation code |
+44…+8b early-init block | +8c sensor index − 1 (input 0 → −1, 1…8 → n−1, > 8 → 0) | +90 VIF / WDMA bind − 1 (1…16, else −1)
| +94 u8 not-SCL-realtime | +98 FBC | +a0 IMI address (HDR).

**Input config** (`cfg`, mi_isp channel +0xFC; mi_isp prints it as "MHalVpeIspInputConfig res:%u * %u, ePixFmt, dataPre,
eInput, eHDRMode, u323DNRLevel, FBC, intype, bindwdma, sensorbind"):

| off | field | meaning |
|---|---|---|
| +00 | intype | mi_isp's bind type of the input [not read by mhal] |
| +04 | ePixFmt | mhal pixel code: 0 YUYV, 0xb NV12, 0xe…0x10 other YUV422 orders, 0x2a + Bayer order (0x2a…0x35) |
| +08 | dataPre | precision (Bayer bits code) |
| +0c | — | 0 |
| +10 / +14 | width / height | |
| +18 | eInput | 1 = YUV frame input, 2 = Bayer frame input, 3 = realtime from VIF |
| +1c | eHDRMode | 0 none, 1/2/4 HDR variants, 3 [kept linear by mhal] |
| +20 | 3DNR level | 0…7 |
| +24 | FBC | compressed frame input |
| +28 | bindwdma | VIF channel + 1 for realtime, else 0 |
| +2c | sensorbind | sensor index + 1 |
| +30 | u8 | port 0 not realtime to SCL |
| +34, +38 | pass counter, mi_isp channel field | not read by mhal |
| +40 | IMI address | read by mhal (inst+0xa0) |

**Open attribute** (built by `_MHalIspInit`, 0xAC bytes, passed to `MhalCameraOpen`; names from its log lines): [0] dev,
[3] / [4] width / height, [5] bytes-per-pixel code (4 YUV422, 5 NV12, Bayer: table by precision; 6 for FBC frame input of
1–2-byte Bayer), [6] Bayer id (format − 0x2a), [7] **mode**, [8] HDR type, [9] YUV order, [10] ops (byte +0x28), [11]
channel, [12]…[17] crop / out sizes (= width / height), [18] IMI address (0), [19] "Open IMI", [20] sensor id, [21] WDMA /
VIF id, [22] 3DNR, [23] rotation, [24] u8 ack-tie, +0x64 early-init block. **Mode**: eInput 1 → 5 (YUV frame); eInput 2 →
2, or 3 when HDR ∉ {0, 3} (Bayer frame); eInput 3 → 0 (HDR 0/3) or 1 (HDR 1/2/4: +0x48 = 0, +0x4c = IMI).
`IspMid_IsFrameMode(mode)` = mode ∈ {2, 3, 5}. This camera: eInput 3, HDR 0 → **mode 0**.

**Process descriptor** (0xF8, a mi_isp stack buffer) [R offsets from both sides]:

| off | field |
|---|---|
| +00 / +08 / +10 | u64 input plane 0 / 1 / 2 address (frame mode) |
| +18 / +1c / +20 | input strides |
| +28 | u64 frame id / meta tag (frame mode: compared with the last processed id; older → the work queue is flushed) |
| +78 | **port 1 block** (mi_isp output port 1 → mhal "external WDMA", node 4 = ISP out): +0 u8 enable, +4 pixel (0 YUYV, 0xa YUV422 SP, 0xb NV12, 0xe…0x10), +8 / +10 / +18 u64 plane addresses, +20 / +24 / +28 pitches (YUV422 8-aligned, SP / NV12 16-aligned), +30 u64 meta, +38 u8 crop on, +3a / +3c / +3e / +40 u16 crop x / y / w / h |
| +c0 | **port 2 block** (mi_isp output port 2 → mhal "IR DMA", node 7 = RGB-IR IR-only): same layout, crop unused |

## 4. Open and close [R]

### 4.1 `MhalCameraOpen(attr)`

4 KB of code; the global ispmid mutex is held throughout.
1. `DrvIsp_CheckChipCapability(w, h)` and `CameraIsp_CheckOpenAttrParamValid`; allocate the 0x1C320-byte channel data on
   first open (a re-open must have the same sensor id).
2. Unless frame mode with other channels open: `CameraChangeMhalCmdqIF(0)` — register writes go direct (CPU) during set-up.
3. Sensor facts through SNR: `DrvSensorGetPlaneInfoByChn`, `DrvSensorGetShutterInfo`, `DrvSensorGetAEMinMaxGain` (no sensor
   bound → zeroed); log `DevId, Ch, SNR ID, VifCh, WdmaID`, `3DNR, Rotation, AckTie`, `(W, H)`, `PixelFmt, BayerID,
   YuvOrder`, `Mode, HdrType`, `CMDQ FRAME=2`.
4. `CameraIspCmdqBuffer_IndexMode_GetService` (stitch only); first open: `CameraIsp_WorkQueueInit` (per-channel work queue
   of 32), memory ops copied; debug fence 7 |= 1 << ch (open-channel mask, §7.6).
5. `CameraIsp_EnableIspRegShadow(mode, 0)`; YUV input (bpp code 4/5): ISP bypass set-up and done. Bayer:
   `CameraIsp_InitChannelFmt`, `CameraIsp_InitIspAttribute`, `CameraIsp_EnableChannel3A(ch, nFlagCus3A)`.
6. First open: statistics DMA area (§10.1, `CameraIsp_StatisticsAllocateBuffer`) if AE / AWB / AF / IQ is on,
   `CameraMload_Init` (§12.6), **IQ file** (`CameraIspAlgo_LoadIQFile` → `CameraReadIqData` reads
   `/config/iqfile/iqfile0.bin`, `IspAlgoReadIqBinH` + `cfg_InitBinInfo` parse it — format: isp-4-iq-engine.md §13.2; on failure
   the channel runs with all gains 1× and IQ off), `CameraIspAlgo_3A_IQ_Init` (`AeInit` / `AwbInit` / `IqInit` — run even
   with user 3A), then `CameraIsp_TriggerAlgoEvent(ch, AE/AWB/AF, 1)` for each enabled algorithm (wakes `isp_fe`, §11).
7. WDR local buffer ("ISP_WDR"), `Camera3DNR_AllocateBuffer`, `CameraIspAlgo_3A_ConfigCheck` / `IQ_Config` /
   `ConfigRestore` (statistics windows, §10.2), `DrvIsp_InOutModeConfig`, HDR pipe buffers + init,
   **`CameraIsp_RegisterCallbackAPI`** (§4.3), `DrvIsp_PowerOn`.
8. `DrvAlgo_IF_IQ_ForceTrigIQ` + `CameraIspAlgo_DoIq` (first full IQ calculation and write), copy it into all three IQ
   slots; realtime: apply both shadow halves at once (`isp_apply_shadow(ch, 0/1, …)`), clear CMDQ-irq 0x40, queue + execute the
   first digital-gain work and (with AE on and user AE off, or early-init data) the first shutter / gain work; 3DNR engine
   reset and enable.
9. Realtime: unmask the ISP input; **`CameraIspCmdqBuffer_Init`** (§7.3) — from here on register writes go to the command
   batch.

Errors release everything in reverse and return −1 (`MHalVpeIspInputConfig` → 0). Bank 1302 values that ISPSCL's init
left in debug fences 6 and 7 (§13.7) are overwritten here: fence 6 := 0 for realtime, fence 7 |= 1 << ch.

### 4.2 Close

`MhalCameraClose(ch)`: `TriggerAlgoEvent(…, 0)` for each algorithm, `CameraIsp_EnableChannel3A(ch, 0)`, opened = 0; back to
direct writes; debug fence 7 &= ~(1 << ch); last channel: unregister the IRQ callbacks, mask the ISP input, `CleanCmdq`,
`CameraIspCmdqBuffer_DeInit` (back to IF none), power off, 3DNR / statistics engines / WDR off; flush and destroy the work
queues; release WDR, HDR, 3DNR, IQ (`AeRelease` / `AwbRelease` / `IqRelease`), IQ file, menuload and statistics buffers;
free the channel data. `_MhalCameraInsideClose` (re-open with a different YUV / Bayer type) does the same without powering
off. `MHalVpeIspDeInit` → `CameraDeInit` closes the rest (§3.1).

### 4.3 `CameraIsp_RegisterCallbackAPI`

`DrvIsp_RegisterCallback(slot 0 = FrameDoneProc, slot 1 = FrameStartProc)` (slot 1 not in frame mode), and once globally
`ISRCB_RegisterCallback(4…7, 0x34…0x37, SensorFrameDoneISR)` (VIF events; `SensorFrameDoneISR` queues
`SensorFrameDoneProc` on "IspMidThreadWq", and that handler only acts in frame / HDR mode). The same function also sets the
statistics MIU select, the WDR menuload buffer and `IspMidWqSetSensorGain`-related state. ISRCB registry: mhal-core.md §3.4.

### 4.4 The open sequence on this camera (live log) [R]
<!-- ev:isp-2-ispmid-mload-ispscl.4.4 -->

`change isproot : /config/iqfile`, `[CameraReadIqData] isp root is /config/iqfile, nCh = 0`, `Load iq file
/config/iqfile/iqfile0.bin`, `[isp_fe_open]`, `[0] AE=1, AWB=1, AF=1, nFlagCus3A = 0x1A`. `/config/iqfile/iqfile0…3.bin` are
symlinks to `imx307_iqfile.bin` (isp-4-iq-engine.md §13.4).

## 5. Interrupts [R]

Chain: GIC SPI 0x39 → mi_isp `_MI_ISP_IMPL_Isr` → `MHalVpeIspCheckIrq(ctx, 1)` → `CameraIspCheckIrq` →
`DrvIsp_CheckIrq`: `st = HalIspCheckIrqMapTable(bank1302, 1)`, then **`DrvIspISR`** (the real decode), return `st`. mhal
parses the IRQ number in `isp_probe` but does not request it; mi_isp does. The interrupt controller (bank 1302.28–.43),
its bit names and the full event map: isp-1-hal.md §4.

**What mi_isp sees** — `HalIspCheckIrqMapTable(h, 1)` (disassembly 0x4ed0c), on group-1 status (`1302.2b & ~1302.28`):

| condition | st |
|---|---|
| group-1 bit 10 (ISP_IDLE) | 1 |
| group-1 bit 12 (VSYNC, frame start) | 4 (wins over 1) |
| group-1 bits 6 / 7 (P0 / P1 FIFO full) or group-2 bit 11 (ROT error) while the word at `.bss`+0x700 is 0 | 2 |
| debug fence 11 == 0x5A5A (set by the frame-mode context switch, §8) | 2, fence cleared |

Codes 1 and 2 are guarded by an "external YUV420 WDMA active" test, `ISP_IsExtWdam420()`, which is the constant 1 in this
build, so they occur in realtime too. The `.bss`+0x700 word reads both as a frame-interval guard and as
"not in frame mode" in different readings of the code [R; not settled]. mi_isp: bit 2 → its frame-start callback list; bit
1 → read the CMDQ tag; bits 0|1 → frame done, `mi_sys_NotifyWakeupISREvent`. **So in realtime mi_isp gets 4 at every frame
start and 1 at every ISP idle (frame end).**

**The decode** — `DrvIspISR` → `HalIspIsr` reads and clears the seven status groups and returns an event word; each event is
queued to the "IspDriverThreadWq" thread with the time stamp (`IspDriverThreadWqAddInISR`), never run in interrupt
context:

| event bit | source | goes to |
|---|---|---|
| 0 | group 1 bit 12 ("FS") | callback slot 1 = **`FrameStartProc`** |
| 1 | group 2 bit 14 ("FE") | callback slot 0 = **`FrameDoneProc`** |
| 2, 3, 5, 6, 7 | groups 3–5 (WDMA / RDMA done, line-count hits) | callback slots 5, 6, 3, 2, 4 (raw store / HDR / image-store handlers, frame mode only) |
| 8…27 | VIF-WDMA and pipe events, FIFO-full P0…P3 | `ISRCB_Proc(4…7, 0x34…0x37, 2, 3, 0x28…0x2b, 0x38…0x3a)` (VIF callbacks; `SensorFrameDoneISR` registered on 4…7 and 0x34…0x37, §4.3; VIF's `DrvVif_GroupEnable(g)` registers `DrvVif_DoVifFifoFullEventCb` on 0x38 + g; 0x3b is registered but never raised) |

Interrupt registers (bank 1302): group n mask / clear / status at registers 0x28 + 4(n−1) / +2 / +3 (n = 1…7; groups 3, 4,
6 are 11–12 bits wide). `HalIspEnableInt` loads the masks at power-up (`DrvIsp_Resume`) from a `.data` table — mask values
(1 = masked) g1 0x2A3F, g2 0xA2FF, g3 0xFF06, g4 0xF074, g5 0xFB64, g6 0xFB6D, g7 0xFF87. Frame start (g1 bit 12), frame
end (g2 bit 14) and ISP_IDLE (g1 bit 10) are unmasked by this table; mi_isp's `EnableIrq(ctx, 1, 1)` sets the same bit 10
unmasked again.

Callbacks registered at open: §4.3. The ISR-callback ids and their other listeners: mhal-core.md §3.4.

## 6. One frame in realtime [R]

### 6.1 `FrameStartProc` and `FrameDoneProc`

`FrameStartProc(g_tIspMidHandler)` — ISP driver thread, ispmid mutex held:
1. Skip when the channel is closed, in frame mode, during the first 4 frames, or with debug flag bit 1. Frame counter ++.
2. **Previous batch check**: if debug fence 2 (written by the last batch, §7.4) is behind the batch counter the IQ part of
   this frame is skipped ("[Skip IQ]"); after 300 consecutive skips the ISP CMDQ is reset
   (`CameraIspCmdqBuffer_CleanCmdq`, §7.5).
3. `CameraIspCmdqBuffer_GetNextWrBufPtr` (switch batch buffer), mask the ISP input (`CameraIsp_MaskIspInput(…, 1)`).
4. Run due works: type 1 (sensor) and type 2 (ISP) for this frame; AWB source switch (HDR), HDR alignment read, 3DNR
   noise-estimate read; copy 1321.7E to ch+0x19e7c.
5. If any 3A is enabled and a CUS3A bit is set: **`IspmidFrameEvent(ch)`** → wakes `isp_fe` readers (§11).
6. **`CameraIspAlgo_UpdateIQ`** (phase = frame mod 3): in-kernel `DoAe` (every phase) / `DoAwb` (every phase for the first
   3 frames, then phase 2) only for algorithms not taken by user space; `RefreshIqGainData` (ISO input from the current
   gains); `CameraIspAlgo_DoIq` at phase 0 (the IQ engine itself recalculates only on its triggers — API change, a total-gain
   step > 194/1024 while any IP interpolates (else an ISO-index change), a CT change; isp-4-iq-engine.md §4.1), `CameraIspAlgo_DoIqPerFrame` every frame; keep a 0x400-byte shadow history; record {shutter, sensor gain,
   WB gains, CT} for the metadata pool. IQ-slot rotation over the three slots happens only in frame mode. Inside:
   `DoIq` → `hal_DoIQ`, `DoIqPerFrame` → `hal_DoIQPerFrame`, both → `hal_IQ_Set_ALL` / `hal_IQ_API20_SET_ALL` →
   `IQ_Patch_<IP>` → `IspRegSet<IP>` → `WriteRegs` (shadow) and `IspMLoadSet<IP>` (tables) — isp-4-iq-engine.md §1.2.
7. Kernel AE only: queue shutter / gain works. With IQ on: queue the digital-gain work (offset ch+0x196f8) and the
   IQ-shadow work for this frame (no user AE / AWB) or the frame before (user AE or AWB on).
8. Execute the type-1 works due now (+ `CameraIsp_SensorAeNotify`), then type 2; **apply the IQ shadow**
   (`isp_apply_shadow(ch, 1)` → per dirty IP `DrvIsp_ApplyShadow` → `HalIsp_Apply<IP>` → `ISP_RegWM`… into the batch; menuload
   tables through `IspMloadCombieSet`, §12.6).
9. Unmask the ISP input, **`CameraIspCmdqBuffer_FinishBufferWrite`** (§7.4).

`FrameDoneProc`: realtime only `DrvIsp_CopyStatisBufInfo` (hardware statistics → CPU copy, §10.1), flush the DMA area
through mi_sys's op, frame-done counter. Frame mode: `FrameDoneProcFrameMode` (§8).

### 6.2 Per-channel work queue (`IspMidWork`, channel +0x19b74)

32 entries of 0x80 B {valid, u64 due frame, callback, type mask (1 sensor, 2 ISP, 8 AE-output), 0x44-byte argument copy,
name[32]}. `Add` replaces an entry with the same due frame, type and name; `Execute(frame, type)` runs entries due exactly
then and drops older ones.

Offsets (`CameraIsp_3A_FrameCntInit`, from the sensor driver's delays): D = max(shutter delay, gain delay); shutter work at
frame + (D − shutter delay), gain at frame + (D − gain delay), digital gain at frame + D − 1, IQ shadow at frame + 1 (D = 0
→ all 0, digital gain 1). The shutter and gain delays of this camera's GC8613 driver, which set D, are read from the
driver at open; their values are the handle fields `ae_shutter_delay` = 2 and `ae_gain_delay` = 2 (sensor-gc8613.md §4.1)
and were not cross-checked against this path.

Work callbacks: `IspMidWqSetSensorShutter` → `DrvSensorSetUs`, `IspMidWqSetSensorGain` → `DrvSensorSetGain` (both suppressed
by debug flag bit 0), `IspMidWqApplyDGain` → `DrvIsp_ApplyDGain`, `IspMidWqApplyIqShadow`, `IspWqIspApplyAeOutputParam`
(frame mode).

So an AE result written by user space within a frame reaches the sensor D frames later and the ISP gains at the next
frame [I from the offsets].

### 6.3 How a register write reaches the hardware

- **IQ values go through a shadow image.** `IspRegSet<IP>` → `WriteRegs(cfg, shadow, …, cfg-enum)` looks up `Regmap[enum]`
  (bank code, register, bit mask) and stores into a per-channel RAM image of the ISP banks (0x8710 bytes per IQ slot, bank
  codes 0x11…0x3e mapped to +0x200 steps) and marks the IP dirty (46 IP ids). Large LUTs go to menuload instead
  (`IspMLoadSet<IP>`, §12.3). `Regmap` itself: isp-4-iq-engine.md §12.1.
- **Shadow → registers**: the IQ-shadow work (`IspMidWqApplyIqShadow` → `isp_apply_shadow`) runs in `FrameStartProc` for the
  frame it is due on and calls, per dirty IP, `DrvIsp_ApplyShadow(id)` → `HalIsp_Apply<IP>` →
  `ISP_RegIndexTableWM(bank_va, shadow+off, field table, n)` → `ISP_RegWM` per field. Simple-Bayer mode uses
  `DrvIsp_ApplySimpleBayerShadow`.
- **`ISP_RegW/RegWM`**: if the HAL handle has a CMDQ interface (`handle+0x148`, set by `DrvIsp_ChangeCmdqIF`) the write
  goes to it with RIU address `IspSysRegV2R(bank_va)/2 + reg`; otherwise a direct 16-bit store to the ioremapped bank. The
  interface is one of: none (direct), "CMDQ" (`CameraCmdqRegW/RegWM/RegWMaskMulti/PollWait/CmdDelay` → a CMDQ object),
  "buffer mode" (`CameraCmdqBufModeRegW*` → append to the current RAM batch, later copied into the CMDQ by
  `FinishBufferWrite`) — §7.2; in realtime the batch runs on the ISP's own CMDQ engine at ISP_VSYNC (§7.4).

## 7. CMDQ use [R]

### 7.1 Engines

The ISP owns one CMDQ engine for its whole life (free id at `MHalVpeIspInit`, 0x20000-byte ring, `CameraCmdqGetCmdqService`).
mi_sys's CMDQ object (the `cmdq` argument of `MHalVpeIspProcess`) is used only in frame mode or when a DMA port is
configured from `MHalVpeIspProcess`. A second "VPE" CMDQ service (`CameraCmdqGetVpeCmdqService`, 0x20000) exists only for
the `/dev/ispmid` test ioctl (§9). The CMDQ interface itself, engine ids and trigger-bus events: mhal-scl-jpe-cmdq.md §2.3, §2.1, §2.8;
mi_sys's CMDQ objects: mi_sys-1-engine.md §7.1.

### 7.2 Write targets

`CameraChangeMhalCmdqIF(cmdq, bufMode)` → `CameraIspChangeCmdqIfCallBack` → `DrvIsp_ChangeCmdqIF` selects one of:
- **none** — direct 16-bit RIU stores;
- **"CMDQ"** — the slots of the CMDQ object (`RegW` +0x24, `RegWM` +0x18, `RegWMaskMulti` +0x1c, `PollWait` +0x2c,
  `CmdDelay` +0x54; the object: mhal-scl-jpe-cmdq.md §2.3);
- **"buffer"** — append {u32 RIU address (bank << 8 | byte offset), u16 value, u16 mask (0xFFFF for a plain write)} to the
  current RAM batch (writes to bank 130D bypass index mode).

The table `CameraIspChangeCmdqIfCallBack` builds for the HAL (`g_IspCmdqIf`, `.bss+0x1acdc`) is `{+0 RegW, +4 RegWM(if,
addr, val, mask), +8 RegWMaskMulti, +0xC PollWait(if, addr, val, mask, timeout), +0x10 CmdDelay(if, ns)}` with `addr =
bank·0x100 + reg·2` (8-bit RIU address); ispmid fills it with `CameraCmdqBufModeRegW*` (buffer mode, PollWait = NULL) or
`CameraCmdqRegW*` (direct CMDQ). MLOAD uses this table (§12.4).

### 7.3 Batches

`CameraIspCmdqBuffer_Init` allocates "ISP_CMDQ" 2 × 0x8000 B (2 × 4096 commands) as a pool of two; realtime switches the
target to buffer mode for good. `GetNextWrBufPtr` toggles buffers and copies over a batch that was never sent; overflow
drops writes ("CmdQ buffer write overflow").

### 7.4 `CameraIspCmdqBuffer_FinishBufferWrite` (realtime, own engine present)

Target := own CMDQ; clear CMDQ-irq 1; `CameraIspAddCmdqWaitEventCmd(own, 1)` = queued commands {force-irq 1 off, unmask 1,
**wait trigger-bus event `ISP_GetIpCmdsqTrigBusId(1)` = 0x1E (ISP_VSYNC)**, force off, mask 1}; clear irq 1; unmask irq 4;
one `WriteRegCmdqMaskMulti(batch, n)`; batch counter ++ and **debug fence 2 := counter as a queued write**; `KickOff`;
target := buffer. So every frame's ISP writes execute at the next ISP VSYNC, and fence 2 tells the next `FrameStartProc`
whether they did (§6.1 step 2).

### 7.5 CMDQ-irq ids and recovery

`DrvIsp_Force/Mask/Clr/PollCmdqIrq` map an id bit to a register bit:

| id | bank / registers (mask / force / clear / status) | bit |
|---|---|---|
| 1 | 1303.50 / .51 / .52 / .53 | 1 |
| 2 | 1303.50…53 | 7 |
| 0x40 | 1303.50…53 | 15; `PollCmdqIrq(0x40)` is degenerate (register 0, mask 0) |
| 4 | 1302.54 / .55 / .56 / .57 | 15 |
| 8 | 1302.54…57 | 13 |
| 0x80 / 0x100 / 0x200 | 1302.50… | 5 / 15 / 14 |
| 0x10 | poll only: 1303.60 | 1 |
| 0x20 | poll only: 130D.0A (menuload busy, wait for 0 — §12.2) | 1 |

Trigger bus (`DrvIsp_GetIspCmdqTrigBusId`, HAL `ISP_GetIpCmdsqTrigBusId`): ids with bits 0x73 → 0x1E (ISP_VSYNC), 0x0C or
0x380 → 0x19 (ISP_TRIG).

**Recovery** (`CameraIspCmdqBuffer_CleanCmdq`): `DrvIsp_ResetIp(1)`, clear trigger, abort buffer, check idle, reset engine
(CMDQ slots 0x44, 0x34, 0x48, 0x38).

### 7.6 Debug fences and index mode

**Debug fence registers** 1302.70 + k (k = 0…11, `DrvIsp_Set/GetDebugFenceID`): 2 = batch counter, 6 = frame-mode flag, 7 =
open-channel mask, 11 = 0x5A5A frame-mode marker. RDMA fence (`ISP_SetRdmaFenceID`, `DrvIsp_Set/GetRdmaFenceID`) = channel |
0x8000 in frame mode.

**Index mode** (`CameraIspCmdqBuffer_IndexMode_*`, registers 1302.7c / .7e / .7f; master / slave kick-off, wait) — a CMDQ
scheme for multi-sensor stitching, active only when channel +0x1c2f4 is set [unused here: one channel].

## 8. Frame mode and DMA ports [R] — unused on this camera

- `MHalVpeIspProcess` for eInput 1/2: target := mi_sys `cmdq` in buffer mode, `CameraContextSwitch(ch, desc.meta)`, then
  the input DMA: `CameraRawFetch(ch, 0, {addr, stride, w, h, bayer, bpp…})` + `CameraRawFetchTrigger` (HDR 1/2: a second
  fetch at addr + h·stride), target := none. Bayer inputs take `_MHalVpeIspProcessBayer`, YUV422 inputs
  `_MHalVpeIspProcessYUV422`; NV12 input only logs; pixel 4 fails.
- `CameraContextSwitch` (frame mode only) → `CameraContextSwitchIspPipeActive`: finish pending frames if needed
  (`FrameDoneProcFrameMode`); statistics buffer from the pool (8 in frame mode; none left → finish pending frames, else
  panic) → hardware (`DrvIsp_HWBufferAssign` / `GetStatisBufInfo`); menuload next frame (`IspMLoadNextFrame`); stale works
  flushed; `CameraIspCmdqBuffer_GetNextWrBufPtr`; 3DNR set-up (`Camera3DNR_Init/Enable`); WDR clock (`DrvIsp_SetWdrClk`);
  3A window config (`CameraIspAlgo_3A_IQ_Config`); `DrvIsp_InOutModeConfig` (format, crop, line-count interrupts); HDR
  statistics switching; RDMA fence; works due (`IspMidWorkExecute(frame id)`); IQ shadow apply;
  `DrvIsp_ApplyIspBypassMode` (if requested); debug fence 11; the batch into the mi_sys CMDQ (`FinishBufferWrite`);
  `CameraIspAddCmdqWaitEventCmd(cmdq, 0x40)` (ISP_VSYNC); frame context pushed to the ring (`IspFrameCtxtRingBufPush`).
  YUV bypass channels use `CameraContextSwitchIspPipeBypass`.
- `FrameDoneProcFrameMode`: pop contexts up to the RDMA fence, free menuload frames (`IspMLoadFreeFrame`), copy HDR / NE
  reads, run late works, AE-output works, `IspmidFrameEvent`, `CameraIspAlgo_IQProcFrame`, return statistics buffers.
- **Port 1** `MHalVpeIspExtDmaPortConfig`: checks crop ≤ output size and pitch alignment, then `CameraRawStore(ch, node 4,
  {pitches, addresses, format 4/5, crop})`. **Port 2** `MHalVpeIspIrDmaPortConfig`: `CameraRawStore(ch, node 7, …)`.
  `_MHalIspCheckExtDmaRequestFromTool` lets the debug console force port 1. The DMA engines behind them: isp-1-hal.md §7.
- Other frame-mode / debug helpers: `CameraRawFetch/RawFetchTrigger/RawStore`, image store (`CameraISP_ImageStore*`,
  0x2E14 / 0x2E15), raw capture (`CameraIsp_CaptureRawImage`, `…CaptureHdrPipeRawImage`, DMA-done handlers), HDR pipe
  buffers, SCL-to-TTL and VIF pattern-generator register tables (`CameraConfig_*`, FPGA bring-up), `CameraDebug*` (proc
  info, forced AE / AWB / IQ values).

## 9. `/dev/ispmid` and `/dev/ispmid_cusapi` [R] — test interfaces
<!-- ev:isp-2-ispmid-mload-ispscl.9 -->

Both are created by `mstar_ispmid_probe` (platform driver "sstar,ispmid", registered by `IspMid_Driver_Init`, mhal-core.md
§3.2).
- **`/dev/ispmid`** (misc, + char dev `ispmid0` and sysfs `isproot` (IQ file directory), `sclen` (SCL-to-TTL test), `info`
  (`CameraDebugShowInfo`)). ioctl type 'I' or 'q': 0x400C4901 CameraOpen(sensor), 0x400C4902 CameraClose, 0x400C4905 control
  sub-commands (0x80 own CMDQ pool test, 0…3 VPE-CMDQ on / off, 0x81 / 0x8F CameraInit / DeInit, 0x88 wait for image store),
  0x400C4907 manual context switch, 0x40104908 copy AE / AWB statistics (`CameraCapture3AStatistic`), 0x40384906 open-ex
  (`IOCTL_ISPMID_CAMERA_OPEN_EX`, YUV→Bayer test, §13.9). `write()` = a text console (`open`, `close`, `openex`, `cmdq*`, `isp_raw`, `save_dma`, `fpga_*`, `vif_pat_*`, `log_en`, `skip_func`, `bypass_cus3a`, `ae_gain`, `dbgae/awb/af/iq`, `isproot`, `isp_buf_read` (§12.5), `yuv_pipe2`, …; strings in). Test / debug only:
  no program, library or module of the dump contains the string `/dev/ispmid` [R: grep over every `*.so`, `prog_*`, `*.ko`].
- **`/dev/ispmid_cusapi`** (misc node `ispmid_cusapi`, `AddCusApiNode`, `IspMid_CusApi_*`): kernel → user request channel
  (per-open buffer {u32 head size, u32 data size, payload}; `poll` reports a pending request, `read` takes it, `write`
  returns the answer that `ISPMID_AE_Set/GetAeWinWgtMode` read back). The only sender `TestIspApiCmd` has no caller — dead
  code; the string is absent from prog_uvc and the libraries, so nothing on the camera opens the node.
- **`/dev/isp_fe`**: §11.

The IQ directory set at module init and the module parameter behind `sclen` (`libcamera_module.ispsclttl`): mhal-core.md
§3.2, §5.

## 10. Statistics [R]

### 10.1 The statistics DMA area

One area of **0x14480 bytes** (`HalIsp_HwStatisticsMemSize`), base programmed into every statistics engine
(`HalIsp_HwStatisticsBuffAddrAssign`, MIU address >> 4 split low 16 / high 12 bits):

| offset | size | content | engine buffer registers |
|---|---|---|---|
| 0x00000 | 0x1000 | AE block averages, 4 B per block {R, G, B, Y} — 32 × 32 blocks fill it exactly | 131B.08 / .09 |
| 0x01000 | 0x8700 | AWB block averages, 3 B per block {R, G, B}, up to 128 × 90 | 131B.0a / .0b |
| 0x09700 | 0x200 | histogram 0 (u16 × 128 used, 256 B) | 131B.0c / .0d (one base for both windows) |
| 0x09900 | 0x200 | histogram 1 | |
| 0x09b00 | 0x1c00 | AF: 16 records × 0x1c0 (`AF_STATS_PARAM_t`, §10.3) | `ISP_AF_SetStatsBuf` (131A / 131C) |
| 0x0b700 | 0x8700 | AWB of the HDR short frame | (HDR only) |
| 0x13e00 | 0x200 | histogram 2 (HDR short) | |
| 0x14000 | 0x200 | RGB-IR histogram (u16 × 256) | `ISP_RGBIR_SetStatsBuf` |
| 0x14200 | 0x240 | NE (3DNR noise estimate) | `ISP_NE_SetStatsBuf` |

The "AE 1" slot (second AE buffer) is never present in this build (offset query returns 0).

Life cycle (`CameraIsp_StatisticsAllocateBuffer`, at open when AE / AWB / AF / IQ is on; `CameraIsp_StatisticsReleaseBuffer`
at close): one "ISP_STATIS" DMA allocation through mi_sys's alloc op, cut into a pool of **2 buffers in realtime** (8 in
frame mode) of 0x14480 rounded to 16. Realtime: buffer 0 is given to the hardware once (`DrvIsp_HWBufferAssign`), buffer 1
becomes the **CPU copy** (`DrvIsp_BufferPtrAssign` → the 10 pointers at channel +0x19a28: AE, AE1, AWB, histo 0, histo 1,
AF, AWB short, histo 2, RGB-IR, NE). At every frame done `FrameDoneProc` copies each region hardware → copy
(`DrvIsp_CopyStatisBufInfo`, sizes as above) and flushes the hardware area through mi_sys's flush op. Every CUS3A
statistics read (0x2E01…) is a `memcpy` from the copy — so a read returns **the last completed frame**, and a read racing a
frame-done copy can mix two frames [I: no lock between the ISP driver thread and the API path other than the global mutex
in `MHalIspApiGet`, which `FrameDoneProc` does not take]. Frame mode instead takes a pool buffer per frame in
`CameraContextSwitchIspPipeActive` and returns it at frame done.

### 10.2 Statistics engines and windows

Configured at open and on every frame-mode context switch by `DrvIsp_3A_StatisticConfig(handle, cfg)` (`cfg` = channel
+0x1977c, §2.2); `CameraIspAlgo_3A_StatisticInit/DeInit` around it:
- **Sources** (`HalIsp_3A_StatisticConfig`): linear: AE and histogram source 5, AWB source 4, WBG / LSC / ALSC source
  select 1; HDR / alternative: 7, 6 and 0; AF source from `cfg+0x223` (CUS3A 0x2E25, `CusAFSource_e`). AE "interrupt block
  row" = 0x1d. Gamma 16→12 enable from `cfg+0x27d`.
- **AE** (`DrvIsp_SetAeWin`, enabled if `cfg+0x27e`): image ≥ 256 × 320; default block size = width/32 × height/32 (32 × 32
  blocks); window offset centres the remainder. Registers 131B: 02 bit 0 enable, 03 source, 16 row number, 30 / 31 block
  w−1 / h−1, 34 / 35 offset x / y, 38…3d per-Bayer-phase normalisation (≈ 100 / sample count, rounded), 3e cleared. **Only
  32 × 32 blocks are accepted** by 0x2E0B (`CameraSetAeWinBlock` rejects anything else with "AE block number must be
  32x32"); the 0x2E0B enum → block-size table contents are not read.
- **AWB** (`DrvIsp_SetAwbWin`, enable `cfg+0x27f`): image ≥ 40 × 60; blocks default 128 × 90 (clamped to width/10,
  height/8), sampling sizes default 4 × 2, ratio parameters 10, 8, 1. Registers 131B: 02 bit 4 enable, 04 source, 40 / 41
  block size−1, 42 / 43 block counts−1, 44 / 45 offsets, 48 / 49 sample sizes, 50…55 normalisation. 0x2E0C sets blocks
  (clamped 6…128 × 5…90) and the ratio byte.
- **Histograms** (`DrvIsp_SetHistoAttr`, `DrvIsp_SetHistoWins`; two windows, enabled together with AE): image ≥ 120 × 5;
  block grid 128 × 90 (fewer for small images); registers 131B: 02 bits 8 / 12 enable, 05 source, 61 / 62 block size−1,
  63 / 64 grid−1, 65 / 66 offset, 69…6c window 0 (x0, x1−1, y0, y1−1 in blocks), 6d…70 window 1, 60 bits 0 / 4 window update
  strobes, 78…7d normalisation, 7e mode. Default both windows = whole grid; 0x2E12 changes one window (`CusAEHistWin_t`,
  index in `u2WinIdx`).
- **AF** (16 windows, 0…1023 normalised coordinates scaled by the AF source size): filter set-up
  `HalIsp_AF_FilterAttrConfig` (`CusAFFilter_t`), `…FilterSqConfig`, `…YParamConfig`, `…PreFilterConfig`, `…YMapConfig`,
  `…LdgConfig`, windows `DrvIsp_SetAfWin`, ROI mode (`CusAFRoiMode_t`: normal / matrix with N vertical blocks). Enabled when
  `cfg+0x280` is set. Banks 131A / 131C; their register-level meaning is not read.
- **RGB-IR** histogram: enabled only for an RGB-IR sensor (`cfg+0x64` ≠ 8); window 128 × 90 grid; register-level set-up not
  read. **NE** comes with 3DNR (isp-1-hal.md §8.3); its statistics set-up at register level is not read.
- Overrides: `CameraIspAlgo_3A_ConfigBackup/Restore` keep the per-channel window set across re-open (backup at
  `g_tIspMidHandler+0x3cc`); global CUS3A-bypass bits (`g_tIspMidHandler+0x33c`) make the setters ignore user calls (bit 3
  AE / histogram windows, bit 4 AWB, bit 5 AF).

Whether prog_uvc's user 3A changes the 131B windows at run time or they stay at the open-time defaults is not known
statically.

### 10.3 Payload formats [R; struct names from the SigmaStar Tiramisu header `mi_isp_hw_dep_datatype.h`]

All 0x2Exx statistics payloads match that public header in size and in every field the kernel reads. The getters are
`CameraGetAe/Awb/Histo/Af/RgbIrHistoStats`.

| statistic | layout |
|---|---|
| AE (0x2E01, `MI_ISP_AE_HW_STATISTICS_t`, 46 088 B) | u32 nBlkX, u32 nBlkY, then nBlkX·nBlkY × {u8 R, G, B, Y}; kernel copies min(nBlkX·nBlkY·4, 0xB400) from the copy buffer (at most 0x1000 meaningful = 32 × 32) |
| AWB (0x2E07 long / 0x2E19 short, `MI_ISP_AWB_HW_STATISTICS_t`, 34 568 B) | u32 nBlkX, u32 nBlkY, then blocks × {u8 R, G, B} |
| histogram (0x2E02 window 0, 0x2E03 window 1, 256 B) | u16 × 128 |
| RGB-IR (0x2E1E, 512 B) | u16 × 256; 0 bytes (status 5) unless an RGB-IR sensor |
| AF (0x2E0D, `CusAFStats_t`, 7 168 B) | 16 × `AF_STATS_PARAM_t` (448 B each): per 16 windows: IIR1 5 B, IIR2 5 B, luma 5 B (34-bit), FIR-V 5 B, FIR-H 5 B (37-bit values), Y-sat 3 B — each field as its own 80 / 48-byte array |
| NE | 0x240 B, kernel-internal (3DNR), not exported |

A getter returns status **5** when it copied 0 bytes (channel not open or statistic absent), else 0. The semantics of the
AF records in matrix ROI mode (16 records × 16 windows per the header comments) are not checked against the registers.
The other 0x2Exx ids (results, windows, enables): isp-3-iq-api.md §5.

## 11. `/dev/isp_fe` [R]

Misc device created by `mstar_ispmid_probe` (`AddFeNode`); opened by prog_uvc's `Cus3A_ProcRoutine` thread. A 20-byte
command / 20-byte event protocol:
- **open**: per-file record (0x4C B) linked into a global list; prints the build date ("Jul 21 2022") — the `[isp_fe_open]`
  line of §4.4.
- **write(buf, ≥ 20)**: copies a 20-byte command `{u32 dev; u32 mode; u32 channel; u32 —; u32 —}` into the record (+0x38).
  Shorter writes are refused with a log line, but `write` always returns 0.
- **poll**: waits on the record's wait queue; returns POLLIN|POLLRDNORM (0x41) and clears the pending flag when an event
  arrived.
- **read** by the last command's `mode`:
  - 0 = **event**: returns up to 20 bytes `{u32 frameMask; u32 aeMask; u32 awbMask; u32 afMask; u32 reopenMask}` — frame
    and re-open masks are per-file and cleared by the read, the three algorithm masks are the global "algorithm enabled"
    channel masks at read time; bit n = channel n;
  - 2 = **AF statistics** of `channel`: 0x1C00 bytes (as 0x2E0D);
  - 3 = **AF / VCM status** of `channel`: 16 bytes `CusAFInfo_t` (as 0x2E20);
  - other modes return 0 bytes.
- **events**: `IspmidFrameEvent(ch)` — frame start of a channel with any 3A enabled and a CUS3A bit set (realtime), frame
  done in frame mode; `IspmidAlgoEvent(ch, AE/AWB/AF, on/off)` — channel open / close per enabled algorithm
  (`CameraIsp_TriggerAlgoEvent`; sets / clears the global mask bit, wakes everyone); `IspmidReopenEvent(ch)` — re-open of an
  open channel with 3A.

User side (prog_uvc `Cus3A_ProcRoutine`, one thread per device, 16 channels): `write(mode 0)` — the command always carries
device word 0 —, `poll(1000 ms)`, `read(20)`; for each channel: AE / AWB / AF mask edges → init (AE: 0x2E04 init status →
algorithm init) / release callbacks; re-open bit → callbacks with code 0x69; frame bit → `Cus3A_ProcAE`, AWB and AF
processing (isp-6-userspace.md §2.4, §2.5).

## 12. MLOAD — menuload [R unless tagged]

MLOAD is the driver of the ISP's **menuload** DMA engine (bank **130D**): it copies whole lookup tables from a DRAM buffer
into ISP-internal SRAMs. The IQ engine prepares the tables in a per-channel staging image (0x38C0 bytes: 18 table
slots + 14 "dirty" bytes); once per frame ispmid calls `IspMloadCombieSet`, which copies the dirty
tables into a non-cached DMA buffer of 0x38B0 bytes and appends the load commands (bank 130D register writes) to the
ISP's per-frame CMDQ batch. Only tables the IQ engine marked dirty are loaded. Tables loaded on this camera: ALSC grid
(SRAM 2), YUV gamma (3), SDC defect table (6), six WDR-local tables (9, 10, 12–15) and the RGB gamma (17) — which of them
the IQ file enables is the IQ engine's business (isp-4-iq-engine.md §10.4). SRAMs 0 and 1 are never loaded (the apply loop skips
ids ≤ 1), SRAMs 4 and 11 only in frame mode (their dirty bytes are never set). The radial LSC is not a menuload table. The
SCL half of the API is stubbed (always −1) in this build: every `SCL_*` function returns −1 and the SCL table range is empty.

### 12.1 Origin and structure
<!-- ev:isp-2-ispmid-mload-ispscl.12.1 -->

The block is the Infinity2 `mload` driver (`drv_mload.c`, `hal_mload.c`, `mdrv_mload.h` of) carried forward and changed for this chip: same function names, same
register names, new table layout, an added "shadow → table" layer (`ShadId`, `Combie`) and an allocator hook. Differences
from the I2 source: §12.9. Object order in `.text`: driver (`_DrvIsp_MLoad*`, `DrvIsp_*`, `DrvScl_*`, `DrvMload*`), public
API (`IspMLoad*`, `SclMLoadApply`, `IspMloadCombieSet`), memory helpers (`Mload*`), `/dev/mload` glue (`mload_*`), HAL
(`ISP_MLoad*`, `SCL_*`, `MLoadRead`, `_IspMload*`).

### 12.2 Hardware: the menuload engine, bank 130D [R register use; names from I2 `hal_mload.h`, which uses the same register numbers]

| reg | bits | name (I2) | use in this build |
|---|---|---|---|
| 130D.00 | [4:0] | `load_sram_id` | target SRAM id of a load |
| 130D.00 | [12:8] | `sram_read_id` | SRAM id for a CPU read-back |
| 130D.01 | [15:0] | `load_amount` | number of SRAM entries − 1 (I2 wrote the count itself as "HW bug workaround"; this build writes count − 1) |
| 130D.02 | [15:0] | `sram_st_addr` | first SRAM entry (always 0) |
| 130D.03 | [15:0] | `load_addr` | MIU address bits [19:4] of the table (16-byte units) |
| 130D.04 | [15:0] | `load_addr_1` | MIU address bits [35:20] (written as `addr >> 20`) |
| 130D.05 | [15:0] | `sram_rd_addr` | entry index for read-back |
| 130D.06 / 07 / 08 | [15:0] | `sram_read_data_0..2` | read-back data, 3 × 16 bit (a 64-bit entry loses its top word) |
| 130D.09 | [0] | `load_register_w1r` | write 1: latch the parameters of one table load |
| 130D.09 | [1] | `load_st_w1r` | write 1: start the queued loads |
| 130D.09 | [2] | `sram_read_w1r` | write 1: read entry `sram_rd_addr` |
| 130D.09 | [3] | `load_start_error_clear` | not used |
| 130D.0A | [0] / [1] | `load_start_error` / `load_busy` | status; [1] polled by the unused `ISP_WaitMloadIdle` and by the ISP's CMDQ-irq id 0x20 (§7.5) |
| 130D.0B | [1:0] | `sram_width` | entry width: 0 = 16 bit, 1 = 32 bit, 2 = 64 bit |
| 130D.0B | [8] | `sram_rw` | 1 = write (load), 0 = read-back |
| 130D.0B | [9] | `miu2sram_en` | engine enable |
| 130D.0B | [10] / [11] | `load_register_non_full` / `load_done` | not used |
| 130D.0B | [12] | `wait_hw_rdy_en` | written as `!cfg.bNoWait` (§12.4) — always 1 on every caller |
| 130D.0B | [14:13] | `load_water_level` | written 0 |
| 130D.0B | [15] | `sw_rst` | active-low reset; written 0, 0, 0, 1 at the start of every apply |
| 1302.34 | [0] | — | menuload interrupt mask (`ISP_EnableMloadIntr`: 1 = masked) — function unreferenced |
| 1302.36 | [0] | — | menuload interrupt clear (pulse 1 → 0) — unreferenced |
| 1302.37 | [0] | — | menuload interrupt status (polled == 1 by unreferenced `ISP_WaitMloadIntrDone`) |

MIU address = output of `MloadHalUtilPHY2MIUAddr(pa)`: unchanged when the allocator hook is installed (mi_sys gives MIU
addresses), else `CamOsDirectMemPhysToMiu(pa)`. Clock: I2 had `ISP_MLoadSwitchClk` (clkgen 1432.26); this build has no
clock code in the block [R: no such function; I: the ISP clock path covers it].

**Load sequence** (`ISP_MLoadApply(cmdq, cfg)`, disassembly 0xa04d6; all writes through `MloadRegW`, i.e. into the CMDQ
batch when `cmdq` ≠ 0):
1. `130D.0B[9] = bit 9 of the cfg pointer` (an uninitialised value: the register argument still holds the cfg address —
   harmless, step 3 sets it); `0B[15] = 0` three times, then `0B[15] = 1`; `0B[12] = cfg.bNoWait ? 0 : 1`;
   `0B[14:13] = 0`.
2. For SRAM id i = 0…17 with `cfg.dirty[i]` set **and i > 1**: clear dirty; `0B[1:0]` = width code from `cfg.unit[i]`
   (2 → 0, 4 → 1, 8 → 2; other units: field left as is); `0B[8] = 1`; `00[4:0] = i`; `03 = (miu >> 4) & 0xFFFF`;
   `04 = miu >> 20` with `miu = PHY2MIU(cfg.pa + cfg.offset[i])`; if `cfg.unit[i] == 0` → return −1;
   `01 = size[i]/unit[i] − 1`; `02 = 0`; `09[0] = 1`, then `09[0] = 0` written 32 times (I2 comment: "write 0, HW is not
   working, same as dummy command" — spacing in the command stream).
3. If at least one table was queued: `0B[9] = 1`, `09[1] = 1` (start), return 0; else return −1.

There is no wait for completion in the per-frame path: the batch moves on and the hardware finishes on its own [R; I: the
next frame's load cannot start before, the ISP's own frame timing is the fence].

**Read-back** (`MLoadRead(bank, width, id, idx, u16 out[3])`, CPU only, disassembly 0xa0796): id must be a known SRAM id;
`0B[9] = 0`, `0B[8] = 0`, `00[12:8] = id`, `0B[1:0]` = width code (2 → 0, 4 → 1, 6 or 8 → 2; others leave it), `05 = idx`,
`0B[9] = 1`, `09[2] = 1` twice, read `06`, `07`, `08` into `out[0..2]`, `0B[8] = 1`. `DrvScl_MLoadRead` calls it with bank
**0xFFFF** (no SCL menuload bank on this chip): the direct-RIU path would access `gpIoBase + 0xFFFF·0x200`, far outside the
4 MiB mapping [R address arithmetic; I: an oops if reached — reachable only through the `/dev/mload` test commands].

### 12.3 SRAM table map [R: offsets / sizes / units from `ISP_MLoadRefreshCfgContent`, `.rodata+0xeb54` (size table, 18 × s16), `_IspMloadLayoutConvertBuffAddr` (tbb table checked in the disassembly), `g_tShadIdToSramIdTable` (`.data+0x1bd9c`, 14 pairs); writers from the IQ engine (`IspMLoadSet*`, dirty bytes searched over the whole ISP decompile)]
<!-- ev:isp-2-ispmid-mload-ispscl.12.3 -->

The DMA buffer (0x38B0 bytes) and the IQ staging image (0x38C0 bytes) use **the same offsets**; the staging image adds the
14 dirty bytes at +0x38B0 (index = shadow id). Shadow ids are the API's numbering; SRAM ids are the hardware's. `ctx` = the
context pointer the writer `IspMLoadSet*` receives [I: the IQ context, isp-4-iq-engine.md §2.1].

| SRAM id | shadow id | offset | bytes | entry | entries | content (writer) | loaded on this camera |
|---|---|---|---|---|---|---|---|
| 0 | 0 | +0x0000 | 0x204 | 32 bit | 129 | unknown — no writer; `IspMLoadSetFPN` is an empty stub [I: FPN] | never (apply skips ids ≤ 1) |
| 1 | 1 | +0x0210 | 0x204 | 32 bit | 129 | unknown — no writer; `IspMLoadSetFPN_P1` empty [I: FPN of path 1] | never |
| 2 | 2 | +0x0420 | 0xE58 | 64 bit | 459 | **ALSC gain grid**: per grid point {a, b, c, 0} u16 from three planes ctx+0x1a680 / +0x1aa16 / +0x1adac [I: R, G, B], GridX×GridY points (ctx+0x55c14 / 15) (`IspMLoadSetALSC`) | when ALSC is set |
| 3 | 3 | +0x1280 | 0x800 | 64 bit | 256 | **YUV gamma**: entry k = {A[k mod 128], B[k mod 128], Y[k], 0} — Y 256 points from ctx+0x1c5f7, A / B 128 points from ctx+0x1c7f7 / +0x1c8f7 [I: U and V], written twice (entries 0–127 and 128–255) (`IspMLoadSetYUVGAMA`) | when YUV gamma is set |
| 4 | 4 | +0x1A90 | 0x800 | 64 bit | 256 | unknown — no writer (`IspMLoadSetGAMA_16to16_ISP` / `_C2C_P1_ISP` empty) [I: a 256-point RGB gamma of the 16-bit path] | frame mode only (forced) |
| 5 | — | — | 0 | — | — | not present in this layout | — |
| 6 | 5 | +0x22A0 | 0x400 | 32 bit | 256 | **SDC static-defect table**: 512 u16 from ctx+0x1b636 (`IspMLoadSetSDC`) | when SDC is set |
| 7, 8 | — | — | 0 | — | — | not present | — |
| 9 | 6 | +0x26B0 | 0x44 | 32 bit | 17 | **WDR local** table, 0x42 bytes from ctx+0x1b57c (`IspMLoadSetWDRLOC`) | when WDR-local is set |
| 10 | 7 | +0x2700 | 0x44 | 32 bit | 17 | WDR local, 0x42 bytes from ctx+0x1b53a | idem |
| 11 | 8 | +0x2750 | 0x800 | 32 bit | 512 | **WDR hardware table**: per frame ispmid copies 0x800 bytes (`IspMLoadGetWdrSize`) from the "ISP_WDR" DMA buffer (ch+0x19b58), the buffer the WDR engine writes through 1322.7E / 7F (`ISP_WDRSetWdrMloadBuffer`), then calls a memory-op callback on it | **never in realtime**: its dirty byte is never set, so only frame mode (forced) loads it [R; I: a vendor slip or intentional] |
| 12 | 9 | +0x2F60 | 0x40 | 16 bit | 32 | WDR curve: 32 × (s16 + nibble·0x1000) from ctx+0x1b4b8 / byte array ctx+0x1b498 (alternative set +0x1b458 / +0x1b438 chosen by a flag at `*(ctx+0x53b88)+0x50 == 1`) | when WDR-local is set |
| 13 | 10 | +0x2FB0 | 0x40 | 16 bit | 32 | previous content of SRAM 12 (copied before 12 is rewritten) | idem |
| 14 | 11 | +0x3000 | 0x44 | 32 bit | 17 | WDR local, 0x42 bytes from ctx+0x1b4f8 | idem |
| 15 | 12 | +0x3050 | 0x44 | 32 bit | 17 | WDR local: 33 u16, even entries = two bytes of ctx+0x1b5be packed little-endian, odd entries 0 | idem |
| 16 | — | — | 0 | — | — | not present | — |
| 17 | 13 | +0x30A0 | 0x800 | 64 bit | 256 | **RGB gamma (C2A)**: entry k = {a[k], b[k], c[k], 0} from ctx+0x1bb16 / +0x1bd16 / +0x1bf16 [I: R, G, B] (`IspMLoadSetGAMA_C2A_ISP`) | when RGB gamma is set |

The contents of SRAMs 0, 1 and 4 (no writer in this build), and whether the hardware uses the never-loaded WDR table (SRAM
11) in realtime, are not known; an SRAM read-back on the running ISP would show them.

Which IQ cfg ids feed these tables (e.g. 0x2b6–0x2bd for the WDR tables; WDRLOC, YUVGAMA, ALSC, SDC, GAMA_C2A):
isp-4-iq-engine.md §10.4, §5.3 [R]. `IQ_InitParameters` clears the 14 dirty bytes. In frame mode
`CameraIspAlgo_UpdateIQ` ORs the dirty bytes of the IQ slots and copies each dirty table between slot staging images using
`IspMLoadConvertIDs`, `IspMLoadConvertBuffAddr`, `IspMLoadGetSize`, `IspMLoadGetMaxShdId` (= 13) [R.text 0x867c0–0x869e0].

### 12.4 Software objects

**Handle** (`DrvIsp_MLoadInit`, `CamOsMemAlloc(0x70)`; up to 32 recorded in `g_tBakMloadHandle[32]`, `.bss+0x1efd8`):

| off | field |
|---|---|
| +0x00 | owner (argument of `IspMLoadInit`; passed to the allocator as its first argument) — ispmid passes `ctx[0]`, `/dev/mload` passes 0 |
| +0x04 | current frame config (`cfg`, below) or NULL |
| +0x08 | CMDQ interface used by `ISP_MLoadApply` (NULL = CPU writes) |
| +0x0C…+0x28 | pool: 8 cfg pointers |
| +0x2C | index of the current ("working") cfg |
| +0x30 | number of freed cfgs kept for reuse ("Available", at most 2) |
| +0x34 | number of allocated cfgs ("Allocated") |
| +0x38 | CamOs mutex |
| +0x64 | maximum ever allocated (debug) |
| +0x68 | "busy" flag, 1 while a table set or apply runs |
| +0x6C | error flags: bit 0 allocation failed with < 8 cfgs, bit 1 all 8 in use, bit 2 free-all while busy, bit 3 free of the working cfg while busy |

**Frame config** `cfg` (0x130 bytes, `ISP_MLoadRefreshCfgContent` fills it on every `NextFrame`):

| off | field |
|---|---|
| +0x000 | `bNoWait`: set to 1 by a table set with flag 0x40, else 0 → `130D.0B[12] = !bNoWait` |
| +0x004 | 0x38B0 = buffer size; doubles as "in use" (0 = free for reuse) |
| +0x008 / +0x00C | buffer VA (non-cached) / PA |
| +0x010 + 4i | dirty flag of SRAM i (i = 0…17) |
| +0x058 + 4i | offset of SRAM i in the buffer (§12.3) |
| +0x0A0 + 4i | size in bytes |
| +0x0E8 + 4i | entry width in bytes (2 / 4 / 8; 0 = absent) |

**Memory** (`MloadMallocNonCache(owner, size, align, &pa)`): if all five allocator callbacks are installed
(`IspMLoadSetMemAllocator(ops)` copies `{alloc, free, map, unmap, flush}` to `.bss+0x1f058`; refused
when one is NULL or a set is already installed) → `alloc(owner, "ISP_MLOAD", size, &u64 pa)`, `map(pa_lo, pa_hi, size, 0)`
→ VA (on map failure `free` and print "MapMemFail"); without the hook → `CamOsDirectMemAlloc("MloadMem%.6d" (µs of the
clock), size, &va, &pa, &miu)`. A counter at `.bss+0x1f06c` tracks live buffers. `MloadFreeNonCache` uses `unmap` +
`free`, or `CamOsDirectMemRelease`. `MloadFlushNonCache` is empty (the buffer is uncached) and `_MloadFlushMemAllocator`
(calls `flush`) is unreferenced. ispmid installs the ops from its open attribute (`CameraMload_Init`:
`IspMLoadSetMemAllocator(*(attr+0x28))`, the mi_sys memory ops of §3.2) [R; I: mi_sys MMA callbacks].

**Frame pool**: `IspMLoadNextFrame(h)` → `_DrvIsp_MLoadNewCfg`: walk the 8 slots; if nothing is kept for reuse, allocate a
new cfg (0x130) + buffer (0x38B0, align 0x10) in the first empty slot; else take the first kept free cfg; make it current,
refresh its content (zeroes the buffer) and return the slot index; −1 and a "MLOAD_FAIL … Max=8, Used, Available" print
when none. `IspMLoadFreeFrame(h, idx)`: idx 8 = free all; else mark free and keep (while fewer than 2 kept) or release;
clears "current" when idx is the working slot.

**Domain switch** `g_iIoctl_Mload_Domain` (`.data+0x1bcec`, initial **3**): bit 0 SCL domain on, bit 1 ISP domain on, bit 7
unit-test mode (set by the `/dev/mload` UT commands, cleared on release). `IspMLoadApply` needs bit 1, `SclMLoadApply` bit
0; `IspMloadCombieSet` runs the SCL part when `bits & 0x81`, the ISP part when `bits & 0x82`, and refuses any handle other
than the UT one while bit 7 is set and a UT handle exists.

**CMDQ interface** expected at `h+8` [R: slots used +4, +0xC, +0x10]: the table `g_IspCmdqIf` of §7.2. MLOAD's register
writes use `RegWM` only, with the value already in field position (no shift — unlike ISPSCL, §13.2); `MloadPollRegBit` uses
`PollWait`, `MloadDelayNs` uses `CmdDelay`.

### 12.5 API — every function

| function | does | callers |
|---|---|---|
| `IspMLoadInit(owner)` | first free slot of `g_tBakMloadHandle` → map RIU (`MloadWarperInit`: `gpIoBase = ioremap(0x1F000000, 4 MiB)` once), `DrvIsp_MLoadInit` (alloc handle, mutex, clear) → handle or 0 | `CameraMload_Init`, `mload_open` |
| `IspMLoadDeInit(h)` | remove from table, `_DrvIsp_MLoadFreeCfg(h, 8)`, destroy mutex, free | `CameraMload_DeInit`, `IspMLoadReleaseModule`, `mload_release` |
| `IspMLoadReleaseModule()` | DeInit all 32, clear allocator ops, `iounmap(gpIoBase)` | `mload_cleanup` (module exit) |
| `IspMLoadNextFrame(h)` / `IspMLoadFreeFrame(h, idx)` | frame pool, §12.4 | `CameraMload_Init` (realtime: one cfg kept for the whole stream, index at ch+0x19b70), `CameraContextSwitchIspPipeActive` (frame mode), `FrameDoneProcFrameMode`, `CameraMload_DeInit`; `mload_open` |
| `IspMLoadSetMemAllocator(ops)` | §12.4 | `CameraMload_Init` |
| `IspMLoadChangeCmdqIF(h, if)` | swap `h+8` under the mutex, return the old one | `IspMloadCombieSet` |
| `IspMLoadTableSet(h, sramId, src)` | (NextFrame if no cfg) memcpy `size[id]` bytes into the buffer, dirty = 1 | **none** |
| `IspMLoadTableGet(h, sramId)` | VA of the table in the current cfg | `ispmid_fwrite` "isp_buf_read" debug command (via `CameraDebugGetMloadHandle` = channel data (channel of `g_tIspMidHandler+0xf0`)+0x19b6c) |
| `IspMLoadApply(h)` | if domain bit 1: lock, busy, `ISP_MLoadApply(h->cmdq, cfg)`, `ISP_MLoadPatchDomain` (empty) | **none** |
| `SclMLoadApply(h)` | if domain bit 0: `SCL_MLoadApply` (stub → −1) | **none** |
| `IspMloadCombieSet(h, shadow, cmdq, flags)` | the per-frame entry: (combie log, §12.7) SCL part (stub, fails); ISP part `DrvIsp_MLoadTableShadSet(h, shadow, flags)` → `ISP_MLoadTableShadSet`, then swap CMDQ to `cmdq`, `DrvIsp_MLoadApply`, restore; returns 0 when the ISP part applied | ispmid IQ-shadow apply (`isp_apply_shadow`, `.text 0x7f5b4`), `mload_fwrite` UT |
| `IspMLoadGetWdrSize()` | 0x800 | ispmid (WDR buffer size at open, per-frame copy) |
| `IspMLoadGetMaxShdId()` | 13 | `CameraIspAlgo_UpdateIQ` |
| `IspMLoadGetSize(sramId)` | `.rodata+0xeb54[id]` (0 for id ≥ 18) | idem |
| `IspMLoadConvertIDs(shadowId)` | shadow → SRAM id (−1 if unknown) | idem |
| `IspMLoadConvertBuffAddr(sramId, base)` | `base + offset[id]` (0 if absent or base 0) | idem, ispmid WDR copy |

`ISP_MLoadTableShadSet(cfg, shadow, flags)` (`.text 0xa040a`): flag bit 0 → walk shadow ids 0…13; flag bit 1 → walk an
**empty** range (the SCL tables of I2 are gone); bit 7 (0x80) → take every id regardless of its dirty byte; for each taken
id: clear its dirty byte, `ISP_MLoadTableSet(sramOf(id), shadow + offset, cfg)`. If any table was copied: `cfg.bNoWait =
(flags & 0x40) != 0` and return 0, else −1.

Internal helpers without other users: `DrvIsp_MLoadTableGet`, `DrvMloadGet*` / `DrvMloadConvert*` /
`DrvMloadUT_InitShdContent` (tail-call wrappers of the `_IspMload*` / `ISP_MLoad*` HAL functions), `DrvIsp_MLoadGetMaxShdNum`
(14), `ISP_MLoadGetIdTableAmount` (`size/unit − 1`), `_CheckSramIdValid` (id is one of the 14 mapped SRAM ids), `MloadVA2PA`
(returns 0, unused), `MloadRegR` (direct read; 0 through CMDQ), `MloadPollRegBit` (CMDQ `PollWait(addr, val, mask, 1000)`, −1
without CMDQ), `MloadDelayNs` (CMDQ `CmdDelay(ns)` or `udelay(ns/1000)`).

### 12.6 Per-frame flow on this camera [R call sites]

1. `MhalCameraOpen` → `CameraMload_Init(attr, ch)`: allocator ops from the open attribute, `IspMLoadInit(ch[0])` →
   ch+0x19b6c; realtime: one `IspMLoadNextFrame` → ch+0x19b70 (failure → `CamOsPanic`). Also the 0x800-byte "ISP_WDR"
   buffer (`CameraAllocDmaBuffer`, VA / PA at ch+0x19b58…) whose address goes to 1322.7E / 7F.
2. The IQ engine (`CameraIspAlgo_DoIq` / `DoIqPerFrame` → `IspRegSet<IP>` → `IspMLoadSet<IP>`) fills the staging image
   (+0x1a0 + slot·0x8710 + 0x4d80 — inside the IQ slot's register shadow of §2.2 —, pointer at `ctx`+0x53b94 of the
   writer's context) and sets dirty bytes (§12.3).
3. Every frame in `FrameStartProc` the IQ-shadow apply (`.text 0x7f5b4`), when IQ menuload / WDR is on (`ch+4 & 0xC`,
   `nCtrlFlag` bits 2–3) and ch+0x19e7c ≠ 0xDB: if the ISP_WDR buffer exists, copy it into staging SRAM 11 and call the
   function at +0x58 of the ispmid handler object (mi_sys's flush op, §2.1) with (buffer, 0x800); then
   `IspMloadCombieSet(ch+0x19b6c, staging, cmdq = (handler+0x68 ≠ 0) ? &g_IspCmdqIf : NULL [I: +0x68 = CMDQ in use],
   flags = frame mode ? 0x81 : 0x01)`. Result to ch+0x1c2d0; success counter ch+0x1c2e4.
4. The 130D writes land in the frame's CMDQ batch with the IQ register writes; `FinishBufferWrite` hands the batch to the
   ISP CMDQ engine, which runs it at the next ISP VSYNC (§7.4) — so the tables change together with the registers of the
   same frame [I].
5. Close: `CameraMload_DeInit` frees the kept frame (realtime) and the handle.

In realtime the same DMA buffer is reused every frame and rewritten by the CPU while the previous frame's load may still be
running [R single cfg; I: a race the vendor accepts — the CPU writes at frame start, the load runs at the VSYNC after it].

### 12.7 `/dev/mload`, sysfs and the unit test

Created by `mload_init` (called from `camdriver_init`, mhal-core.md §3.2): `cdev_alloc` with `fops` (`.data+0x1bcf4`: read
`mload_fread` → 0, write `mload_fwrite`, unlocked_ioctl `mload_ioctl` → 0, open `mload_open`, release `mload_release`), char
region "chr_arr_dev", class "chardrv", device **`mload`** with attributes `sram`, `info`, `combie` (mode 0644). The boot
print "Mload Inside init module" is KERN_INFO and does not show on the console log [R boot log; I level]. No program,
library or module in the dump contains the strings `mload` / `/dev/mload` except mhal.ko itself [R: grep over the whole
dump], so nothing opens the device on the camera. `mload_cleanup` undoes it at module exit.

| interface | behaviour |
|---|---|
| `open` | first open: UT handle `IspMLoadInit(0)` + `NextFrame` (→ `g_MloadUT_hnd`), UT shadow 0x38C0 (`g_pMloadUT_Shd`) filled by `DrvMloadUT_InitShdContent(shadow, 0)` = every table byte = its byte index, all dirty |
| `release` | clear domain bit 7, free UT shadow, DeInit UT handle, free the 4 KiB read buffer |
| `write "isp_read W N ID"` | for i < N: `DrvIsp_MLoadRead(W, ID, i)` (CPU read-back, §12.2), print `i:a,b,c`, append 6 bytes to `g_ptSysfsMloadBuff` (4 KiB, `.bss+0x1f07c`, fill `.bss+0x1f084`) |
| `write "scl_read W N ID"` | same with bank 0xFFFF — unsafe (§12.2) |
| `write "mload_isp_domain 0/1"`, `"mload_scl_domain 0/1"` | clear / set domain bit 1 / bit 0 |
| `write "mload_ut_set 0/x"` | set bit 7; `IspMloadCombieSet(UT hnd, UT shadow, NULL, 0 → flags 1, else 2)` (CPU writes) |
| `write "mload_ut_init P"` | fill UT shadow with byte P (P = 0: byte index) |
| `write "mload_ut_auto"` | fill with 1, set bit 7, CombieSet flags 1 then 2, create `mload_ut_data.txt` and `unittest.report` in the writer's cwd, for ids 0–19 read entry 0 (PASS if 0x0101) and dump every entry, then ids 20–60 through the unsafe SCL read |
| `write "write"` / other | nothing / "Invalid command" |
| `sram` (show) | the read buffer, ≤ 4096 bytes; store: no-op |
| `info` (show) | per live handle: index, handle, Working, Available, Allocated, iDbgMaxAllocated, iDbgErrorFlag; store: no-op |
| `combie` (show) | the combie log, **and clears `g_iSysfsCombieFlag`** (`.data+0x1bcf0`, initial 1) so logging stops after the first read; store: no-op |

**Combie log**: while `g_iSysfsCombieFlag` is set, every `IspMloadCombieSet` writes a record into the 60-entry ring
`g_tSysfsCombieStatus` (`.bss+0x1f09c`, 0x48 bytes each: handle, shadow, flags, up to 14 dirty shadow ids + −1 terminator;
index `g_iSysfsCombieStatusIndex`) — so on this camera every frame writes one record from boot until someone reads
`combie`. The show function prints all 60 records into the one-page sysfs buffer without a length check (≈60 × up to ~110
characters) [R; I: can overflow the 4 KiB page]. The live handle and its error flags are readable at
`/sys/class/chardrv/mload/info`; their values on the running camera are not known.

### 12.8 Function inventory (98) [R; "unreferenced" = no code or data reference in the module]

- Used on this camera: `IspMLoadInit/DeInit/ReleaseModule/NextFrame/FreeFrame/SetMemAllocator/ChangeCmdqIF/GetWdrSize`,
  `IspMloadCombieSet`, `DrvIsp_MLoadInit/FreeHandle/NextFrame/FreeFrame/ChangeCmdQ/TableShadSet/Apply/GetMaxShdNum`,
  `DrvScl_MLoadTableShadSet` (stub path), `_DrvIsp_MLoadInitCfg/NewCfg/FreeCfg`, `ISP_MLoadTableShadSet/TableSet/Apply/
  RefreshCfgContent/PatchDomain/GetMaxShdNum`, `SCL_MLoadTableShadSet` (stub), `_IspMloadConvertIDs/LayoutConvertBuffAddr`,
  `_CheckSramIdValid`, `Mload{Malloc,Free,MallocNonCache,FreeNonCache,FlushNonCache,HalUtilPHY2MIUAddr,RegW,SetMemAllocator,
  WarperInit,WarperDeInit}`, `_Mload{Clear,Alloc,Free,MiuAddr}MemAllocator`, `mload_init`.
- Frame mode only: `IspMLoadGetMaxShdId/GetSize/ConvertIDs/ConvertBuffAddr` and their `DrvMload*` / `_IspMload*` targets.
- Debug / `/dev/mload` only: `mload_*` (fops, sysfs), `IspMLoadTableGet`, `DrvIsp_MLoadTableGet`, `ISP_MLoadTableGet`,
  `DrvIsp_MLoadRead`, `DrvScl_MLoadRead`, `MLoadRead`, `MloadRegR`, `DrvMloadGetIdTableAmount`, `ISP_MLoadGetIdTableAmount`,
  `DrvMloadUT_InitShdContent`, `ISP_MLoadUT_InitShdContent`, `DrvScl_MLoadApply` + `SCL_MLoadApply` / `SCL_WaitMloadIdle`
  (stubs, reached from CombieSet only after the SCL table set — which always fails, so in fact never).
- **Unreferenced** (unused, specified above): `IspMLoadTableSet`, `IspMLoadApply`, `SclMLoadApply`, `DrvIsp_MLoadTableSet`,
  `DrvIsp_MLoadDump`, `DrvScl_MLoadDump` (read every entry of a table into a u16 array, 1–4 words per entry by width),
  `_MloadFlushMemAllocator`, `MloadVA2PA`, `ISP_WaitMloadIntrDone` (→ `ISP_WaitMloadIdle` → `MloadPollRegBit`,
  `MloadDelayNs`), `SCL_WaitMloadIntrDone`, `ISP_EnableMloadIntr`, `SCL_EnableMloadIntr`.

### 12.9 Differences from the Infinity2 source [R both sides]

| I2 (`hal_mload.c`, `mdrv_mload.h`) | this build |
|---|---|
| bank 0x1519, clock 1432.26, interrupt in ISP0 0x4A / 0x4C / 0x4D bit 3 | bank 130D, no clock code, interrupt 1302.34 / 36 / 37 bit 0 |
| `MLoadLayout_t`: FPN, gamma16to16, ALSC 61×69, FPN / gamma / ALSC of pipe 1, DPC 1024, gamma 10→10, + 23 SCL tables (ids 33–60) | 14 ISP tables, SRAM ids 0–17 as §12.3; SCL tables gone (stubs) |
| `eTarget[i]` per table (front / image domain), `ISP_MLoadApplyIspDomain(…, eTarget)` | no domain split; `ISP_MLoadPatchDomain` empty |
| sw_rst 0 → 1 once; `wait_hw_rdy_en = 1` always; `load_amount = count` ("HW bug workaround") | sw_rst 0, 0, 0 → 1; `wait_hw_rdy_en = !bNoWait`; `load_amount = count − 1`; SRAM ids 0 / 1 skipped |
| ring buffer / double buffer (`u32RingBufWp`, `uBufferIdx`) | pool of up to 8 per-frame cfgs, 2 kept for reuse |
| `IspMLoadTableSet(handle, id, table)` as the IQ path | shadow image + dirty bytes + `IspMloadCombieSet`; `IspMLoadTableSet` left unreferenced |

## 13. ISPSCL — YUV→Bayer converters, IspScl units, ISP→SCL YUV pipe, VIF-WDMA interrupt group [R unless tagged]

### 13.1 What the block is [R names and register use; I for the hardware roles where marked]

Four groups of hardware, one small HAL each, one driver layer (`DrvIspScl_*`) and a platform driver (`ispscl_*`):

| group | units / ids | banks | role |
|---|---|---|---|
| YUV→Bayer (`HalIspScl_Yuv2Bayer*`) | 0–7 = VIF pipe 0–7, port 0 | 1309 (16 registers per unit: 1309.00 + 0x10·n), per-unit bits in 1342.40 / .45, H-down-sample bits in 1303.41 | turns a YUV422 sensor stream into a Bayer-pattern stream so the Bayer ISP can take it ("YUV in Bayer out", VIF pixel formats 0x2A–0x2D) [I from the VIF log strings] |
| IspScl (`HalIspScl_IspScl*`) | 8–15 = VIF pipe 0–7, port 1 | 134C, 134C+0x40, 134D, 134D+0x40, 134E, 134E+0x40, 134F, 134F+0x40 (64 registers per unit), per-unit bits in 1342.41 / .46, 1303.42 | YUV→RGB (BT.601 matrix) + crop + down-scale + Bayer re-mosaic for VIF port 1 (BT656 / BT1120 sub-stream, frame mode only) [I from the code path and the coefficients] |
| YUV pipe (`HalIspScl_YuvPipe_*`) | one | 1302.03 / .05 / .09 / .16 / .1E / .1F, 1303.24 / .25 | ISP→SCL realtime YUV pipe (VIF pipe 2 YUV realtime): interface clock, input size, line-count interrupt, ISP→SCL enable, async FIFO [I names from the function names] |
| interrupt group | 6 groups × 16 bits | 1342.00–.17 | "WDMA0–15 done", "FIFO0–3 full", "WDMA0–15 start" — the VIF→DRAM write-DMA events of VIF frame mode, forwarded to ISR callbacks |

Banks 134A / 134B are mapped but not used by ISPSCL (nor accessed by the ISP HAL, isp-1-hal.md §9). The VIF side of these
calls (`HalVifIspscl_*` forwarders): mhal-vif.md §13.

### 13.2 HAL handle and register access
<!-- ev:isp-2-ispmid-mload-ispscl.13.2 -->

`HalIspScl_Init(&out)` allocates (once) a 0x14C-byte handle (`_s_pIspSclHalHandler`, `.bss+0x1e0d4`) holding RIU virtual
addresses, with the **same slot layout as the ISP HAL handle** of `HalIsp_Init` (isp-1-hal.md §2.1) for the slots it fills: +0x00 1302, +0x04 1303, +0x74 1342, +0x90 133E, +0x94…+0xA4 1308
(+.00 /.20 /.40 /.60 /.78), +0xA8 / +0xAC 130A (+.00 /.20), +0xB0…+0xC0 130B (+.00 /.20 /.40 /.60 /.78), +0xC4…+0xD4
1348 (same five), +0xD8…+0xE8 1349 (same five), +0xEC 130C, +0xF0 130E, +0xF4 130F, +0xF8 134A, +0xFC 134B,
**+0x108…+0x124 IspScl units 8–15**, **+0x128…+0x144 YUV→Bayer units 0–7** (1309 + 0x40·n bytes), +0x148 CMDQ interface =
0. The RIU window comes from `IspSclSysGetRegBase` (ioremap 0x1F000000, 4 MiB, cached in the device struct). Only 1302,
1303, 1342, the 1309 and 134C–134F sub-blocks are ever written by ISPSCL code; the other slots are filled and unused.
`HalIspScl_Deinit` frees the handle.

`ISPSCL_RegWM(h, bankVA, off, value, mask)`: **`off` is twice the register number** and **`value` is right-aligned**: it is
shifted left by the position of the mask's lowest set bit (`_ISPSCL_RegBitShift`), then `reg = (reg & ~mask) | (v & mask)`
at `bankVA + off·2`. If `h+0x148` (a CMDQ interface `{RegW, RegWM, …}`) were set, the write would go to `RegWM(if,
(bankVA − base)/2 + off, v, mask)` (`IspSclSysRegV2R`) — never the case here (nothing sets +0x148), so **every ISPSCL write
is a direct CPU write**. `ISPSCL_RegW` (no mask) is unreferenced. The interrupt registers are accessed with plain 16-bit
loads / stores.

### 13.3 YUV→Bayer unit n (0–7), base `Y = 1309.00 + 0x10·n` [R; field names descriptive]

| function | register effect |
|---|---|
| `Yuv2BayerEnable(n, en)` | `1342.40[n] = 1`; `1342.45[n] = 0` then `= 1` (pulse); `Y.00[0] = en` |
| `Yuv2BayerSetYuvSwap(n, a, b)` | `Y.02[0] = a`, `Y.02[1] = b` |
| `Yuv2BayerSetBayerFmt(n, fmt)` (fmt 0–3, else −1) | `Y.02[3:2] = {0, 1, 3, 2}[fmt]` (`.rodata+0xd1ae`); `Y.03[0] = 1` |
| `Yuv2BayerSetWidth(n, w)` | odd w → w − 1 (warning); `Y.01[11:0] = (w + 3)/4 − 1` (width in 4-pixel units) |
| `Yuv2BayerSetPipeGuardCycle(n, cfg, frameMode)` | `Y.02[10:4] = frameMode == 1 ? 0 : 6` |
| `Yuv2Bayer_DownSampleSwapYC(n, s)` | shadow u16 `.bss+0x1e0d0` bit n = s; `1303.41[15:8] = shadow` |
| `Yuv2BayerDownSampleEnable(n, e)` | shadow u16 `.bss+0x1e0d2` bit n = e; `1303.41[7:0] = shadow` |
| `Yuv2BayerDownSampleStatus(n)` | returns `1303.41[n]` (read from the hardware) |
| `Yuv2BayerBase(n)` | VA of unit n (slot +0x128 + 4n); n ≥ 8 → 0 and "does not exist" |

The shadows make `1303.41` a whole-byte write: the two H-down-sample bytes cover all 8 pipes at once.

### 13.4 IspScl unit n (8–15), base `S` = slot +0x108 + 4(n−8)

| function | register effect |
|---|---|
| `IspSclEnable(n, en)` | `1342.41[n−8] = 1`; `1342.46[n−8] = 0`, and `= 1` again only when `en` (so a disabled unit is left with this bit 0); `S.01[15] = !en` |
| `IspSclSetYuvSwap(n, a)` | `S.01[2] = a` (the driver passes a second swap flag which this function ignores) |
| `IspSclSetBayerFmt(n, fmt)` (0–3) | `S.02[1:0] = {0, 1, 3, 2}[fmt]` (the same 4-byte table at `.rodata+0xd1ae`); `S.01[1:0] = 2` |
| `IspSclSetInOutSize(n, W, H, X, Y, CW, CH, OW, OH)` | `S.10[2:1] = 0`; `S.11 = W`, `S.12 = H`, `S.17 = W`, `S.18 = H`; `S.13 = X`, `S.14 = CW`, `S.15 = Y`, `S.16 = CH`; `S.10[0] = 1`; `S.32 = CW`, `S.33 = CH`, `S.34 = OW`, `S.35 = OH`; horizontal: `S.30[15] = (OW ≠ CW)`, and if so `S.30[12:0] = ceil(OW·4096 / CW)`; vertical: same in `S.31` with OH / CH. Warnings (log bit 1): crop that would need the last 4 pixels of the last line, odd crop size, OW > CW or OH > CH (no up-scaling), and OW > 960 (checked only when vertical scaling is on) |
| `IspSclSetCSC(n)` | `S.20[0] = 1`, `S.20[8] = 0`, `S.20[15] = 1`; `S.21[7:0] = 0x80`; `S.22[15:8] = 0x10`; `S.23[7:0] = 0x80`, `S.23[15:8] = 0x80`; 14-bit coefficients `S.24…S.2C` = 0x0CC4, 0x0950, 0x3FFC, 0x397E, 0x0950, 0x3CDE, 0x3FFE, 0x0950, 0x1024; `1303.42[7:0] = 0x10` |
| `IspSclBase(n)` | VA of unit n; n ∉ 8…15 → 0 |

The CSC coefficients are BT.601 limited-range YCbCr→RGB in s3.11 two's complement (0x950 = 1.164, 0xCC4 = 1.596, 0x397E =
−0.813, 0x3CDE = −0.392, 0x1024 = 2.018; rows ordered {Cr, Y, Cb}) with offsets 16 (Y) and 128 (Cb, Cr) [I from the
values]. The meaning of the control bits `S.10`, `S.20`, `S.21–23` and of `1342.40 / .41 / .45 / .46` beyond "per-unit bit,
pulsed" is not known (no register header for this chip); the ISP HAL also pulses 1342.45 / .46 in `IrqResetIp`
(isp-1-hal.md §5.3).

### 13.5 YUV pipe (ISP→SCL realtime YUV)

`DrvIspScl_YuvPipe_Settle(h, W, H, en)`:
- `en`: `1302.05[2] = 1` (interface clock), `1303.24[12:0] = W − 1`, `1303.25[12:0] = H − 1`,
  `1302.1E[12:0] = 1302.1F[12:0] = min(H, 0x1FFF) − 1` (ISP output line-count interrupt), `1302.03[1] = 1` (ISP→SCL enable),
  `1302.16[4] = 0` (async off).
- `!en`: `1302.16[4] = 1`, `1302.03[1] = 0`, `1302.05[2] = 0`.

The HAL functions behind these writes: `HalIspScl_YuvPipe_InputImageResolution`, `…SetIspOutputLineCountInt`,
`…ICPASync`, `…Isp2SclEnable`, `…InterfaceClkEnable` [I: which field each writes, from the names].

`HalIspScl_YuvPipe_IrqResetSwIf(h, en)` (`1302.09[2] = !en`) is unreferenced. The same 1302 fields are also written by the
ISP HAL: 1302.03 = 3 at `PowerOn`, 1302.05[2] = 1 at probe, 1302.16[4] = 1 at init (isp-1-hal.md §5.1).

### 13.6 Interrupt group in bank 1342, ISR and callbacks

Six groups g = 1…6 at `1342.(4(g−1))` … +3: `+0` mask (1 = masked), `+1` [I: force], `+2` clear, `+3` status.

| function | effect |
|---|---|
| `DisableInt` | all six masks = 0xFFFF |
| `EnableInt` | masks = `{0x0000, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFF0, 0xFFFF}` (`.data+0x1b39c`), clears = `~mask` — i.e. group 1 (all 16 bits) and group 5 bits 0–3 unmasked |
| `GetIntStatusN` | `status & ~mask` of group N |
| `MaskClearN(bits)` | `clear |= bits; force &= ~bits` |
| `MaskIntN(bits)`, `MaskIntAll`, `ClearInt` | unreferenced (`MaskIntN`: `mask |= bits`; `MaskIntAll` = DisableInt + clear = ~table; `ClearInt` sets a bit in a caller's word) |
| `HalIspScl_Isr` | read the six statuses; count group-1 bits into `gIspSclIntCnts[0…15]` ("WDMA0…15_DONE"), group-5 bits 0–3 into `[0x40…0x43]` ("FIFO0…3_FULL"), group-6 bits into `[0x50…0x5F]` ("WDMA0…15_ST" — masked, so always 0); groups 2–4 read and ignored; acknowledge groups 1, 5, 6 with `MaskClear`; return the group-1 bits |
| `ShowInts` (sysfs `ispscl_ints`) | the 36 counters as `name : value` lines |

The 16 group-1 bits go to ISR-callback ids 0–3 and 0x28–0x33: `DrvIspScl_Isr_Entry` maps the returned bit k to
`ISRCB_Proc(k)` for k = 0–3 and `ISRCB_Proc(0x28 + k − 4)` for k = 4–15. VIF's `HalVif_GetPipePort2ISRCBID` gives the same
ids: pipe p port 0 → {0, 1, 2, 3, 0x28, 0x29, 0x2A, 0x2B}[p], port 1 → 0x2C + p; VIF registers `DrvVifWdmaDoneCb` on them in
frame mode (`DrvVif_PortSetConfig_*`) — so **group-1 bit k = "VIF WDMA of pipe k mod 8, port k / 8, done"** [R both
mappings; the I6E `drv_isrcb.h` names ids 0–3 `eISRCB_ID_ISP_WDMA0..3_DONE`]. The ISP's own decode raises ids 2, 3 and
0x28…0x2B too (§5); the ISRCB registry: mhal-core.md §3.4.

The Linux handler `IspScl_ISR` (registered by `IspScl_RegisterIsr` with `request_threaded_irq(irq, IspScl_ISR, NULL, 0,
"IspScl Interrupt")` + affinity hint to the online CPUs) runs the entry when the driver handle exists and returns
IRQ_HANDLED, else IRQ_NONE. DT: `ispscl { compatible = "sstar,ispscl"; interrupts = <0 0x71 4>; }` (GIC SPI 0x71, level)
[R board.dts]; boot log: `[ISPSCL] Request IRQ: 31, 113` and `IspScl Interrupt SUCCESS with IsrNum: 31` (Linux IRQ 31, hw
113).

### 13.7 Driver layer (`DrvIspScl_*`)

Driver handle (`.bss+0x1dc30`, 0x284 bytes): `+0` HAL handle, `+4 + 0x28·id` config of id 0–15:

| cfg word | meaning |
|---|---|
| [0], [1] | input W, H |
| [2], [3] | crop X, Y |
| [4], [5] | crop W, H |
| [6], [7] | output W, H |
| [8] | Bayer order 0–3 |
| [9] byte 0 / byte 1 | YUV swap flags (swap0 to `SetYuvSwap` a, swap1 to b; IspScl uses only swap0) |

| function | does |
|---|---|
| `DrvIspScl_Open()` | clear the handle, `HalIspScl_Init`, `DrvIspScl_Init`; returns the handle |
| `DrvIspScl_Init()` | `HalIspScl_InitReg` + `HalIspScl_EnableInt` (also called by `ispscl_resume`) |
| `HalIspScl_InitReg` | for n = 0–7: SwapYC 0, down-sample 0, `Yuv2BayerEnable(n, 0)`; for n = 8–15: `SetCSC(n)`, `IspSclEnable(n, 0)`; then `1302.76 = 0x5757`, `1302.77 = 0x7A7A` — ISP debug fences 6 and 7 (frame-mode flag, open-channel mask, §7.6), which `MhalCameraOpen` overwrites (fence 6 := 0 for realtime, fence 7 |= 1 << ch), so these values live only until the first camera open |
| `DrvIspScl_Close(h)` | `HalIspScl_Deinit` (free the HAL handle) |
| `DrvIspScl_Yuv2B_Config(h, id < 16, cfg)` | store the 10 words |
| `DrvIspScl_Yuv2B_Enable(h, id, en)` | only if stored W and H are both ≠ 0: `Yuv2B_Settle(h, id, stored cfg, en)`; else −1 (no register write) |
| `DrvIspScl_Yuv2B_Settle(h, id, cfg, en)` | id 0–7: if en {Enable(1), SetYuvSwap, SetBayerFmt, SetWidth(W)}; Enable(en). id 8–15: if en {IspSclEnable(1), SetYuvSwap, SetBayerFmt, SetInOutSize}; IspSclEnable(en) |
| `DrvIspScl_Yuv2B_H_DN(h, pipe, en, swapYC)` | SwapYC(pipe, swapYC), DownSampleEnable(pipe, en) (log reminder: input width must be a multiple of 4) |
| `DrvIspScl_Yuv2B_H_DN_Status(h, pipe)` | 1303.41 bit |
| `DrvIspScl_Yuv2B_SetRunningMode(h, pipe, frameMode)` | guard cycle (§13.3) |
| `DrvIspScl_YuvPipe_Settle` | §13.5 |
| `DrvIspScl_Isr(h)` | `HalIspScl_Isr` |
| `DrvIspScl_GetHnd()` | returns the handle — unreferenced |

Every function accepts NULL for "the global handle". Log level `IspSclLogGetEn()` = `.data+0x1b11c`, initial **0x0B**, never
changed (`IspSclLogSetEn` unreferenced): bit 0 banners / failures, bit 1 warnings / IRQ result, bit 3 trace of Open / Init /
Settle / Config / H_DN / YuvPipe; bits 2 and 6 select `IspSclLogPrintTimeStamp` output (printk, or bit 6: a 512-byte record
buffer at `.bss+0x1deb4` flushed by `IspSclLogPrintTimeRec`) — both functions unreferenced.

### 13.8 Platform driver and device nodes

`IspScl_Driver_Init` (`.text.unlikely`, from `camdriver_init`, mhal-core.md §3.2): `CamPlatformDriverRegister(&driver)`
(`.data+0x1b1b8`: probe, remove, suspend, resume, name "sstar-ispscl", of_match "sstar,ispscl"); then, once,
`DrvIspScl_Open()` → device struct +0x10 (`gpIspSclDrvHnd`), and if an IRQ was mapped `IspScl_RegisterIsr(irq)`.
`IspScl_Driver_Exit` (`.exit.text`): unregister, `DrvIspScl_Close`.

Device struct (`.bss+0x1e0b8`): +0x00 platform device, +0x04 dev_t (major), +0x08 dev_t region, +0x0C Linux IRQ, +0x10
driver handle, +0x14 RIU VA.

| item | behaviour |
|---|---|
| `ispscl_probe` | log banner; kmalloc a misc device {minor = dynamic, name **"ispscl"**, fops = `ispscl_fops`, parent = pdev}; `misc_register` (→ `/dev/ispscl`); `alloc_chrdev_region("ispscl0")` (no cdev is added); `CamDeviceCreate(msys class, devt, "ispscl0")` + link from the platform device + attributes `ispscl_func` (0600) and `ispscl_ints` (0444); `CamIrqOfParseAndMap(of_node, 0)` → +0x0C |
| `ispscl_remove` | remove `ispscl_func` (not `ispscl_ints`), `misc_deregister`, destroy the class device, free the region, kfree |
| `ispscl_suspend` / `resume` | 0 (−EINVAL for a NULL device) / `DrvIspScl_Init` |
| `open` / `release` | set / clear the file's private pointer to the platform device's drvdata |
| `read` | returns 0 |
| `write` | copy from user, `sscanf("%s %d %d %d %d")`, log `CMD:… PARAM:…` (bit 0); no command is acted on; returns the count |
| `unlocked_ioctl` | 0 when the ioctl type byte is 'L' (0x4C), else −ENOTTY; no action |
| `ispscl_func` show / store | "IspScl Func\n" / parse a decimal number and discard it |
| `ispscl_ints` show | `HalIspScl_ShowInts` (§13.6) |

No file in the dump other than mhal.ko contains `ispscl` [R grep], so nothing opens these nodes. Whether the WDMA-done /
FIFO-full counters of `ispscl_ints` (class device `ispscl0`, msys sysfs class) stay at 0 on the running camera is not known
[I: they should — no VIF WDMA runs in realtime].

### 13.9 Callers, and the state for this sensor [R: call graph, branch conditions]
<!-- ev:isp-2-ispmid-mload-ispscl.13.9 -->

| caller | ISPSCL calls | when |
|---|---|---|
| `IspScl_Driver_Init` (module load) | `DrvIspScl_Open` → InitReg + EnableInt; IRQ registration | always |
| VIF `DrvVif_PipeSetConfig` → `HalVifIspscl_Yuv2B_H_DN` | `DrvIspScl_Yuv2B_H_DN(pipe, en, swapYC)` | every pipe set-up; on this camera `(0, 0, 0)` [S mhal-vif.md §9.4] |
| VIF `MHal_VIF_PortSetConfig` → `HalVifIspscl_YUV2B_SetRunningMode` | guard cycle of `pipe` | after every successful port enable / disable |
| VIF `DrvVif_PortSetConfig_MIPI`, port 0, YUV input (channel type 0) | fmt 0 / 0xE–0x10 "YUV in YUV out": Config + `Yuv2B_Enable(pipe, 0)` (+ `YuvPipe_Settle(W, H, 1)` for pipe 2 realtime); fmt 0x2A–0x2D "YUV in Bayer out": Config (Bayer = fmt − 0x2A) + `Yuv2B_Enable(pipe, 1)` | YUV sensors only |
| VIF `DrvVif_PortSetConfig_MIPI`, port 0, **Bayer input (type 1)** | **none** in realtime; frame mode only WDMA set-up | this camera: none |
| VIF `DrvVif_PortSetConfig_BT656` | the same YUV paths, plus port 1 YUV422 frame mode → IspScl unit `pipe + 8` with crop / output size from the port config and swap0 = YC-out swap | BT656 / BT1120 sensors |
| VIF `DrvVif_PortDisable_{MIPI,BT656}` | Config(pipe + 8·port, zeros) + `Yuv2B_Enable(…, 0)` → returns −1 without a register write (W = 0); pipe-2 YUV realtime: `YuvPipe_Settle(0, 0)` → disable | port shutdown |
| ispmid `CameraIspScl_Yuv2B_Settle` | H_DN + Settle | only from `CameraIspOpenEx` (**no reference anywhere** — dead) and the `/dev/ispmid` test ioctl `IOCTL_ISPMID_CAMERA_OPEN_EX` / text commands (`yuv_pipe2`, …), which nothing opens (§9) |

**Register state left by ISPSCL on this camera** (gc8613, MIPI, Bayer RAW10, VIF pipe 0 port 0, realtime) [R writes; I that
nothing outside ISPSCL and the ISP HAL rewrites them]:
- `1309.00[0] … 1309.70[0] = 0`: all eight YUV→Bayer converters off; `1342.40[7:0] = 0xFF`, `1342.45[7:0] = 0xFF` (pulsed).
- `1309.02[10:4] = 6` (pipe 0 guard cycle, realtime) — the only field touched at stream start; the unit stays off.
- `1303.41 = 0x0000`: no horizontal down-sample, no YC swap on any pipe.
- IspScl units 8–15: CSC loaded, `S.01[15] = 1` (off), `1342.41[7:0] = 0xFF`, `1342.46[7:0] = 0x00`; `1303.42[7:0] = 0x10`.
- `1302.76` / `1302.77`: overwritten by `MhalCameraOpen` (§13.7).
- YUV pipe: never touched by ISPSCL (pipe 2 only).
- Interrupts: 1342 masks `{0x0000, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFF0, 0xFFFF}`; the ISR is installed. With no VIF WDMA in
  realtime the WDMA-done bits should stay quiet [I]; FIFO-full events (group 5 bits 0–3) are only counted, no callback.

So **YUV→Bayer is fully off for this sensor**: the only YUV2B write at stream start is the guard-cycle field of a disabled
unit. Whether the reset defaults of these registers already equal this boot state is not known statically.

### 13.10 Function inventory (84) [R]

- Run on this camera: `IspScl_Driver_Init`, `ispscl_probe`, `IspScl_RegisterIsr`, `IspScl_ISR`, `DrvIspScl_Open/Init/
  Isr/Isr_Entry/Yuv2B_H_DN/Yuv2B_SetRunningMode/Yuv2B_Config + Yuv2B_Enable` (shutdown, no write), `HalIspScl_Init/InitReg/
  EnableInt/Isr/GetIntStatus1–6/MaskClear1–6/Yuv2Bayer_DownSampleSwapYC/Yuv2BayerDownSampleEnable/Yuv2BayerEnable/
  Yuv2BayerBase/Yuv2BayerSetPipeGuardCycle/IspSclSetCSC/IspSclEnable/IspSclBase`, `ISPSCL_RegWM`, `_ISPSCL_RegBitShift`,
  `IspSclSysGetRegBase`, `IspSclLogGetEn`; on demand: `ispscl_ints_show` + `HalIspScl_ShowInts`, the other fops / sysfs
  functions, `ispscl_suspend/resume`.
- Reached only for YUV sensors / port 1 (unused here, specified in §13.3–§13.5): `DrvIspScl_Yuv2B_Settle`,
  `DrvIspScl_Yuv2B_H_DN_Status`, `DrvIspScl_YuvPipe_Settle`, `HalIspScl_Yuv2BayerSetYuvSwap/SetBayerFmt/SetWidth/
  DownSampleStatus`, `HalIspScl_IspSclSetYuvSwap/SetBayerFmt/SetInOutSize`, `HalIspScl_YuvPipe_InputImageResolution/
  SetIspOutputLineCountInt/ICPASync/Isp2SclEnable/InterfaceClkEnable`.
- Module exit: `IspScl_Driver_Exit`, `DrvIspScl_Close`, `HalIspScl_Deinit`, `ispscl_remove`.
- **Unreferenced**: `DrvIspScl_GetHnd`, `IspSclLogSetEn`, `IspSclLogPrintTimeStamp`, `IspSclLogPrintTimeRec`,
  `IspSclSysRegV2R` (only from `ISPSCL_RegW/RegWM`'s CMDQ branch, never taken), `HalIspScl_YuvPipe_IrqResetSwIf`,
  `HalIspScl_DisableInt` (only from `MaskIntAll`), `HalIspScl_MaskIntAll`, `HalIspScl_MaskInt1–6`, `HalIspScl_ClearInt`,
  `ISPSCL_RegW`.
