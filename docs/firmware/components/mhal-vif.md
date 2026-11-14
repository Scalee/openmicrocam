# mhal-vif — the VIF (video input) block of mhal.ko
<!-- ev:mhal-vif -->

VIF is the video-input block of `mhal.ko`. Per sensor channel it takes the output of the MIPI CSI-2 receiver (or of a
parallel, BT656 or BT1120 bus), crops it, sets pixel format and precision, routes the channel to one of eight ISP pipes
and either streams it straight into the ISP (realtime) or writes it to DRAM through its write DMA (WDMA, frame mode). It
also owns the sensor pins (pad mux, reset, power-down, MCLK) the SNR block drives. This document covers the hardware
layer (`HalVif*`, `HalVifIsp*`, `VifIsp*`), the driver layer for every bus and work mode (`DrvVif*`, `DrvVifIsp*`), the
`MHal_VIF_*` API with its configuration structures, the interrupts, the frame-mode buffer ring, the OTP checks and the
debug surface. Sensor interface (SNR) and CSI receiver: mhal-csi-snr.md. Module-level facts (block map, ISRCB registry,
DT properties, character devices): mhal-core.md §2.1, §3.4, §7, §4. The caller `mi_vif`: mi-capture.md §3.

Tags: **[R]** read from the binary, a decompile, the disassembly, the DT or a header; **[S]** stated by a source (the
Infinity2 source tree); **[I]** inference. A heading tagged [R] means every statement under it is [R] unless marked
otherwise.

Binary: `/config/modules/4.9.227/mhal.ko`. Function addresses 0x400xxxxx are `.text` offset + 0x40000000; "disassembly 0x…" are `.text` offsets.

Image base [R]: at Ghidra's default base the module occupies 0x10000–0x2xxxxx, the same range as RIU register constants,
so such constants can show up as pointers. Rebased to 0x40000000 and diffed, no register address in VIF changed: VIF
addresses registers as ioremap base + offset, and literal-pool words are resolved from the module's relocations.

Register notation: `BANK.RR[h:l]` = RIU bank (hex), 16-bit register index `RR` (hex), bit field. A bank is 0x200 bytes at
`0x1F000000 + BANK·0x200`; each 16-bit register takes 4 bytes (low byte at +0, high byte at +1), so a driver byte offset
`o` inside an ioremapped bank is register `o>>2`, bits `8·(o&1)…` (RIU addressing itself: kernel-platform.md §3.1). `VIFn.RR`
= register RR of the channel bank of channel n (§3); `w.RR` = register RR of a WDMA engine block (§4.2). All VIF writes are
byte read-modify-writes of one field unless stated.

## 1. The block [R]
<!-- ev:mhal-vif.1 -->

44 KB of code, 465 functions, 65 exports. Three layers:
- **HAL** — `HalVif*` (channel banks, bank 1303, pads, pixel clock, MCLK, OTP), `HalVifIsp*` / `VifIsp_*` (WDMA and the
  ISP-side banks 1302 / 1342), `HalVifIspscl_*` (forwarders to the ISPSCL block).
- **Driver** — `DrvVif_*` (per-bus configuration, context, interrupts, input mask, frame-mode ring, MCU, debug),
  `DrvVifIsp_*` (WDMA programming).
- **API** — `MHal_VIF_*`, called by `mi_vif` (22 imports) and `mi_isp` (`MHal_VIF_InputMask`); the per-module import lists
  are in mhal-core.md §2.4. The ISP block of mhal calls `DrvVif_ChnID2DevID`, `DrvVif_GetDev2SNRPadID`,
  `DrvVif_Dev2ChnID`, `DrvVif_SNRPad2GroupID`, `DrvVif_Group2DevID`, `DrvVif_GetVifChnByPipe`,
  `DrvVif_GetSensorInputFmt`, `DrvVif_GetWdmaInputFmt` and `DrvVif_GetSensorFrmCnt64`.

Inputs: MIPI CSI-2 (from the CSI block), parallel ("digital camera"), BT656 (1-, 2- or 4-channel time-division mux, ITU
TDM) and BT1120. Work modes: **realtime** (channel → pipe → ISP directly) and **frame mode** (channel → WDMA → DRAM ring of
buffers, handed to `mi_vif`). HDR: two-exposure pairs on two channels (virtual channel / DOL) and line-interleave; also
YUV input, optionally converted to Bayer by ISPSCL (YUV→Bayer, "YUV2B").

This camera uses one configuration: MIPI sensor on pad 0, group 0, dev 0, channel 0 (bank 1304, sub-channel 0), pipe 0,
port 0, realtime, Bayer RG 10-bit 3840×2160 (§10.7). Frame mode, BT656 / BT1120 / parallel, HDR, YUV→Bayer, the pattern
generator, strobe and the 8051 MCU are present in the binary and not used on this camera.

Public sources [R: compared with the binary]:
- **Infinity2** `jockyw2001/sdk` `mhal/i2/vif` (2018): `drv_vif.c`,
  `hal_vif.c`, `mhal_vif_datatype.h`, and `mhal.o` with DWARF. Same HAL function family (`HalVif_Crop`,
  `HalVif_SensorMask`, `HalVif_SensorFormat…`) and the same frame-mode design, but an **older MHal API** (dev / chn /
  sub-chn instead of the M6 group / dev / pipe / port) and a register layout without the M6 pipe / port routing and
  indirect window; CSI and sensorif were outside mhal on I2.
- **Mercury5** (`longyanjun2020/SDK_pulbic`): `hal_vif.o` with DWARF, register
  headers `kernel_vif.h`, `mercury5_reg_vif.h` (names of the per-channel control bits) and `kernel_clkgen.h`.
- **mercury6 camclk**: names and parent
  lists of the clock-generator fields of bank 1038, which confirm the MCLK registers of §6.3.

## 2. Platform driver, resources, numbering [R]

### 2.1 Probe and remove

The platform driver "mstar-vif" is never registered with a working probe: `mstar_vif_probe` / `mstar_vif_remove` are
2-byte stubs, and the real work runs from `DrvVif_Init` / `DrvVif_Deinit`, called by `camdriver_init` (order:
mhal-core.md §3.2). `DrvVif_Init` finds the DT node `"sstar,vif"` and runs the probe body `vif_probe` (0x400dda26):
- `ioremap` of every non-empty entry of `g_tVifHandle` (§2.2);
- DT reads (§2.3), IRQ request, misc device `vif` + class device `vif0` with sysfs `vif_ints`, `vif_info` (§15);
- frame-handler and shared-memory ring init (`initialSHMRing`, `DrvVifFrameHandlerInit`, §12.2);
- `DrvVif_CLK` (empty);
- default interrupt masks: all 4 bits of all 16 channels masked (§11.2);
- `DrvVif_Dev2SNRPadCtxInit` (dev → pad map = 0xff), `DrvVif_CtxInit` (interface words 5, dev work mode 2 for devs 0–3,
  pipe table 0xff, BT656 minimum heights 0x870, …);
- `DrvVifIsp_Open` — the WDMA register handle (§4.1), stored at .bss+0x3115c; used by frame mode only;
- `DrvVif_SharedInputMaskInit`, `DrvVifSetChnBaseAddr` (= `HalVifSetChnBaseAddr` for channels 0..15);
- input mask on for all 16 channels.

Remove (`vif_remove` (0x400ddf20)): `iounmap` of the handles, `DrvVif_CtxDeInit` (empty), IRQ affinity hint cleared, `free_irq`, the
two sysfs files, `misc_deregister`, CamDevice destroy, the char region, `DrvVifIsp_Close`,
`DrvVif_SharedInputMaskRelease`. `camdriver_exit` does not undo the VIF probe work (mhal-core.md §3.3).

### 2.2 Register handles and banks

`g_tVifHandle[16] = {va, pa, size}` (12 bytes each) is compiled in, not taken from the DT `reg` property.
`HalVifSetHandle` / `HalVifGetHandle` / `HalVifGetHandleSize` / `HalVifGetHandleBasePA(i)` read and write entry i.

| entry | bank (PA = 0x1F000000 + bank·0x200) | use |
|---|---|---|
| 0 | 1304, size 0x800 = banks **1304–1307** | the four channel banks (§3) |
| 1 | — (all zero) | — |
| 2 | — (all zero) | BDMA in the MCU loader (§14.2) |
| 3 | — (all zero) | MCU8051 in the MCU loader |
| 4 | — (all zero) | PM_SLEEP in the MCU loader |
| 5 | 103e | pad input enables (`HalVifDataIeConfig`, §6.1) |
| 6 | 103c padtop | pad modes (§6.1) |
| 7 | 113a | pixel-clock source (§6.2) |
| 8 | 1038 clkgen | MCLK, ISP clock (§6.3, §6.4) |
| 9 | whole RIU 0x1F000000, 4 MiB | WDMA engine blocks (§4.1) |
| 10 | 101e chiptop | all-pad-in bit (§6.1) |
| 11 | 1303 | FIFO mux and pipe routing (§5) |
| 12 | 1110 | not known |
| 13 | 1302 | ISP-side WDMA / FIFO resets (§4.5) |
| 14 | 1342 | pipe-channel reset (§4.5) |
| 15 | — (all zero) | MCU mailbox (§14.2) |

Per-channel pointer `g_VIFReg[16]` (`HalVifSetChnBaseAddr(ch)`): channels 0–3 → handle[0].va (bank 1304), 4–7 → +0x200
(1305), 8–11 → +0x400 (1306), 12–15 → +0x600 (1307); channel ≥ 16 → −2. Inside a bank the fields of sub-channel `ch & 3`
sit at different offsets (nibble / byte selection inside the HAL functions). The global `g_VifMclkSupport_37125` u8[4]
sits just before `g_VIFReg`; the code addresses `g_VIFReg` and the throughput-cap state (§7.2) relative to it.

`HalVifReadRegWithBankAddr(a)` reads the u16 at RIU byte `(a << 1) & 0x1fffffe`, i.e. `a` = bank·0x100 + reg·2
(0x101f5c = 101f.2e); it is how VIF reads the eFuse words.

**Main channel only**: almost every per-channel HAL writer starts with `if ((ch & 3) == 0)` and silently ignores channels
1–3, 5–7, …; the sub-channels exist only as virtual channels of the TDM / VC demultiplexer. The exceptions are marked
"(sub)" in §3.1.

### 2.3 Device tree, interrupt, clocks

The properties of node `vif` and the values in the camera's DT are in mhal-core.md §7: the 34 `vif_srN_*_mode` pad modes
→ `g_VifSnrPadMode` (used by `HalVif_SetIOPad`, §6.1), `vif_srN_mclk_37p125` → the 37.125 MHz support flags (§6.3),
`vif_sr_gpio_puse_0`, `mcu` (read, unused), `interrupts[0]`. Interrupt: **GIC SPI 0x59** →
`request_threaded_irq(irq, vif_isr, …, "vif interrupt")`, affinity hint **CPU1**. The DT `clocks` (sr00, sr01, sr1 MCLK)
are not requested; MCLK is written directly to the clock generator (§6.3).

### 2.4 Numbering

- group g = sensor-pad group 0–3; dev = 4g + i; **channel = dev** (`HalVif_DevID2ChnID` is the identity); pipe 0–7;
  port 0–1 per pipe.
- `HalVif_ChnID2PhyChn` / `PhyChn2ChnID`: identity, ch ≤ 16 accepted. `ChnID2DevID` / `DevID2ChnID`: identity, < 16.
  `DevID2GroupID` = dev >> 2, `GroupID2DevID` = g << 2.
- Sensor pad → device / channel (`GetDevIDBySNRPadID`, `GetChnBySNRPadID`): pad 0 → 0, 1 → 8, 2 → 4, 3 → 12. Group →
  pad (`GetSNRPadIDByGroupID`): g 0 → 0, 1 → 2, 2 → 1, 3 → 3. `GetDefaultCHIDByPIPEID` → 0.
- The dev → (pad, plane) map (`DrvVif_SetDev2SNRPadMap`, ctx+0xab9, 5 bytes per group) is filled by
  `MHal_SNR_SetPlaneMode` (mhal-csi-snr.md §3.2); `mi_sensor`'s SetPlaneMode(0) sets group 0 → pad 0.
  `DrvVif_SNRPad2GroupID` / `DrvVif_GetDevIDBySNRPadID` search it (0xff if not found).
- `HalVif_GetPipePort2ISRCBID(pipe, port, *id)` — the ISRCB ids of the WDMA-done events (registry: mhal-core.md §3.4):
  port 0 → pipe 0–3: 0–3, pipe 4–7: 0x28–0x2b; port 1 → 0x2c + pipe; another port → returns 0 without writing; pipe ≥ 8
  → −1.
- WDMA engine of a port: port 0 → dma id = pipe (0–7), port 1 → dma id = pipe + 8 (`vif_port_dma_id` (0x400d8290), identity map of
  pipe → dma id). Ring id of a port = pipe + 8·port (§12.2).

## 3. Channel banks 1304–1307 [R]

### 3.1 Register map

Field = what the HAL function of that name writes; "(sub)" = per sub-channel (ch & 3) variant, all others act on the
main channel only (§2.2). M5 names are given where they match. "Used by" = the driver path
that calls the writer (§10, §11).

| reg | bits | HAL function / meaning | used by |
|---|---|---|---|
| .00 | [0] | `SensorSWReset` — `sensor_sw_rstz` (active low) | Dev enable / disable (all buses); `HalVif_EnableParallelIF` (§6.4) |
| .00 | [1] | `IFStatusReset` | BT656 / BT1120 GroupSetConfig |
| .00 | [2] / [3] | `SensorReset` / `SensorPowerDown` — sensor reset / power-down pins (M5 `vif_ch0_sensor_rst`, `vif_ch0_sensor_pwrdn`); only channels 0 / 4 / 8 / 12 have the pins | SNR (§6.5) |
| .00 | [4] / [5] | `LiModeEn` / `LiCh` — line-interleave HDR mode / channel | MIPI HDR mode 4 (§10.2) |
| .00 | [12:8] | `SelectSource(ch, src)` — `src_sel`: src 5 → 4 plus the `.3c[1]` and `.34` writes below; src 6 → 4 and `.3c[1] = 1`; else src, and `.3c[1] = ch & 1` | DevSetConfig of all buses (MIPI src 0) |
| .00 | [15] | `SensorChannelEnable` — `vif_chN_en` (`HalVif_GlobalInit` clears it on all 16) | PortSetConfig / PortDisable |
| .01 | [1] | `IfDeMode` — DE-only mode for parallel input | Parallel |
| .02 | [0] [1] [4] | `EnSWStrobe`, `SWStrobe` (trigger), `StrobePolarity` | none (§14.1) |
| .03 | [14:0] [15] | `StrobeStart` (lines), `StrobeRef` | none |
| .04 | [14:0] [15] | `StrobeEnd`, `EnHWStrobe` | none |
| .05 | [3:0] [14] [15] | `HWStrobeCNT`, `HWLongStrobeEndRef`, `HWStrobeMode` | none |
| .06 | [7] | `SensorBitSwap` | Parallel, BT656 |
| .06 | [8] / [9] | hsync / vsync "from sync code" select: cleared by `SensorHsyncPolarity` / `SensorVsyncPolarity`, set to 1 by the `…Bt656` variants | Parallel / BT656 |
| .06 | [12] | `SensorRgbIn` — `sensor_rgb_in` (Bayer input) | DevSetConfig |
| .07 | [0] / [1] | vsync / hsync invert. `SensorVsyncPolarity(ch, pol)`: pol 0 → `.07[9:8] = 3`, `.07[0] = 0`; pol ≠ 0 → `.07[9:8] = 0`, `.07[0] = 1`. `SensorHsyncPolarity`: pol 0 → `.07[12] = 1`, `.07[1] = 0`; else `.07[12] = 0`, `.07[1] = 1`. `…Bt656` variants: hs `.07[12] = 1`, `.07[1] = ch & 1`; vs `.07[9:8] = 3`, `.07[0] = 0` | Parallel / BT656 |
| .07 | [6] | `SensorYc16Bit` | BT1120 |
| .07 | [9:8] / [12] | `SensorVsDly` / `SensorHsDly` (also written by the polarity functions above) | BT656 (`SensorHsDly(1)`) |
| .08 … .0b | [0] | **(sub)** `SensorMask(ch, m)` — `vif_chN_sensor_mask` of sub-channel 0..3 (§3.3) | input mask |
| .20 | [0] [1] [2] [3] [5:4] [10:8] | pattern generator: `PatTgenEn`, `PatDgenEn`, `PatHsgenEn`, `PatDgenRst`, `PatSensorArray` (Bayer order), `PatDeRate` | none (§14.1) |
| .21 / .22 | [12:0] | `PatImgWidthM1` / `PatImgHeightM1` | none |
| .23 | [7:0] / [15:8] | `PatVsLine` / `PatHsPxl` | none |
| .24 | [7:0] / [15:8] | `PatHblank` (×2) / `PatVblank` | none |
| .25 / .26 | [12:0] | `PatBlkWidthM1` / `PatBlkHeightM1` | none |
| .27 | [2:0] / [5:4] | `PatColorInitIdx` / `PatColorPercent` | none |
| .28 | [7:0] / [11:8] / [15:12] | `PatFrmChgRate` / `PatFrmChgX` / `PatFrmChgY` | none |
| .29 / .2a | [12:0] | `InputHsCnt` / `InputDeCnt` (written; M5 calls them counts) | none |
| .30 | [0] [1] [2] [3] | `BT656InputSelect`, `BT656ChannelDetectEnable`, `BT656ChannelDetectSelect`, `BT656BitSwap` | BT656 |
| .30 | [4] / [5] | `Bt656Yc16bitMode` / `Bt1120Yc16bitMode` | BT656 / BT1120 |
| .30 | [9:8] / [10] | `Bt656Chanel` (write) / `Bt656Field` (read) | BT656 |
| .31 | [1:0] [2] [3] [7] | `BT656VSDelay`, `BT656HsyncInvert`, `BT656VsyncInvert`, `BT656ClampEnable` | BT656 |
| .31 | [14] / [15] | `Bt656DetStatus0` / `Bt656DetStatus1` (read) | BT656 debug |
| .32 / .33 | [15:0] | `BT656VerticalCropSize` / `BT656HorizontalCropSize` | BT656 |
| .34 | [1], [7:3], [14:8], [15] | written only by `SelectSource` src 5: `[1] = ch & 1`, `[7:3] = 0` (`= 1`, i.e. `[3]`, for ch 4 / 8), `[14:8] = 0`, `[15] = ch & 1`; meaning not known — no header on hand names it [I: TDM source routing] | BT656 TDM |
| .3c | [0] | `ITU_TDMEnable(bank, en)` (indexed by bank 0..3, `g_VIFReg[4·bank]`) | BT656 |
| .3c | [1] | per `SelectSource` | DevSetConfig |
| .3c | [2] [6] [9:8] | `ITU_TDMConfig(bank, mode, fmt)`: mode 0 → `[9:8] = 0`, `[6] = 0`; 1 → `[9:8] = 1`; other → `[9:8] = 3`, `[6] = 1`; then `[2] = (fmt == 2)` | BT656 |
| .3c | [4] / [5] | `ITU_TDM_YCOutSwap` (also read by `ITU_TDM_GetYCOutSwap`) / `ITU_TDM_UVSwap` | BT656, pipe config (§9.4) |
| .3e | [15:1] | window settle: written 0 three times, keeping bit 0, after every indirect access (§3.2) | — |
| .40–.49 | — | indirect window (§3.2) | — |
| .4a | [15:0] | cleared by `FrameMask` | — |
| .4b | [3:0] | **(sub)** `CropEnable(ch, en)`: `.4b \|= en << (ch & 3)` (set only, never cleared) | DevSetConfig |
| .50 / .51 | [12:0] | read by `HalVif_GetInfo`: received pixel / line counts (§15) | debug, throughput cap |
| .54 / .55 | [12:0] | read: `TotalPixelCount` (.54); `GetVifInfo` returns (.54, .55) | debug |
| .57 | [12:0] | read: `TotalLineCount` | debug |
| .58 | [3:0] [7:4] [11:8] [15:12] | `Crop_Cnt0DbgSelc`, `Crop_Cnt1DbgSelc`, `total_Cnt0DbgSelc`, `total_Cnt1DbgSelc` (bank 1304 only) | none |
| .59 | [5:0] | `HalVIFLogDbgSel` (bank 1304 only) | none |
| .59 | [8] | `CaptureEn` | none |
| .5b … .5f | [8:0] | second interrupt group, 9 bits: `IrqMask` (`.5b \|= bits`, returns the old low byte), `IrqUnMask` (`&= ~bits`), `IrqMaskGet`, `IrqForce` (`.5c = bits`), `IrqClr` (`.5d = bits`), `IrqClr1` (`.5d \|= bits`, then the same register `&= ~bits` — a pulse), `IrqFinalStatus` (.5e), `IrqRawStatus` (.5f) | only the unused `DrvVif_ISR` path (§11.3) |
| .60 | 4 bits per sub-channel | **(sub)** `ITU_TDM_IrqMask(ch, bits)`: nibble ch&3 of `.60` (low byte = sub 0 / 1, high byte = sub 2 / 3) `\|= bits` **and** `.64[3:0] \|= 0xf`; `ITU_TDM_IrqUnMask`: nibble `&= ~bits`; `ITU_TDM_IrqMaskGet`: the nibble | VIF interrupt (§11) |
| .64 | [3:0] | set to 0xf by every `ITU_TDM_IrqMask` call, never cleared; meaning not known — no header on hand names it [I: interrupt enable or "mask latch" of the four sub-channels] | — |
| .65 | [15:0] | `ITU_TDM_IrqForce(ch, bits)` = `bits << 4·(ch&3)` | none |
| .6a | [15:0] | `ITU_TDM_IrqClr` = `bits << 4·(ch&3)` (`ITU_TDM_IrqClr1` is a 4-byte stub) | VIF interrupt |
| .70 | [15:0] | `ITU_TDM_IrqFinalStatus` = nibble ch&3 | VIF interrupt |
| (.5f) | | `ITU_TDM_IrqRawStatus(ch)` reads `.5f` (the second group's raw status) shifted by 4·(ch&3) [I: the wrong register] | none |

### 3.2 Indirect window and its tables

Crop, format, frame mask and line / pixel counts are not plain registers: they go through a window in the channel bank.
- **Write**: `VIFn.40[12:9] = table`, `VIFn.40[3:0] = ch & 3`, data words in `VIFn.42 … .45`, then `VIFn.40[8] = 1`
  (trigger), then three writes `VIFn.3e[15:1] = 0` keeping bit 0 (settle; nothing is polled).
- **Read**: the same select with `VIFn.40[15] = 1`, the same settle writes, data in `VIFn.46 … .49`.

Tables:
- **table 2 — format / frame mask**, word 0: `[1:0]` a, `[3:2]` b, `[5:4]` c (`SensorFormatGroup(ch, a, b, c)`);
  `[11:8]` frame mask. Per mhal precision index 0..4 (8 / 10 / 12 / 14 / 16-bit) the MIPI path writes (a, b, c) =
  (0, 0, 0), (0, 0, 1), (0, 0, 3), (1, 0, 2), (0, 0, 2), keeping `[11:8]` from the read-back — 10-bit → **word 0 = 0x10**.
  Field c is the precision code also used for WDMA (§12.1: c = 0 for 8-bit, 1 for 10-bit, 3 for 12-bit, 2 for 14 / 16-bit).
  `FrameMask(ch, code)` writes `[11:8]`: code 0 → 0 (no skip), 1 → 0x800, 3 → 0x200, 4 → 0xc00; code 2 and anything
  else → "unsupport frame rate", field 0. `FrameMask` reads table 2 first, writes the pattern back and clears `VIFn.4a`.
- **table 4 — crop**, words 0–3: x0, x1, y0, y1 (`HalVif_Crop`; DevSetConfig writes x, x+w−1, y, y+h−1).
- **table 6 — counters**: word 0 = line count 0 (`Vif2IspLineCnt0`), word 1 = line count 1 (`Vif2IspLineCnt1`), words 2 / 3
  = pixel count low 16 / high 8 bits (`Vif2IspPixCnt(ch, ?, n)`). Each writer reads the table first and writes the other
  words back unchanged. Line count 0 is the line at which interrupt bit 1 fires (§11.1).
- `HalVif_GetInfo` / `HalVif_GetDataPrecInfo` / `HalVif_ShowInfo` read tables 2, 4 and 6 back (§15).

### 3.3 `HalVif_SensorMask(ch, m)` — input mask with FIFO / pipe recovery

Works on any sub-channel (no main-channel check). State: a 16-bit "masked" bitmap at `g_tVifHandle+0x524`.
- m = 1: set bit ch in the bitmap; log the pipe (`DrvVif_GetPipeByPhyChn`); write `.08+(ch&3) [0] = 1`.
- m = 0: if bit ch is set: if **all four** sub-channel masks of the bank are 1 → `HalVif_ResetAffCh(FindVifChToAffCh(ch))`
  (the async FIFOs of this group); if this sub-channel's mask is 1 → `HalVif_ResetPipeCh(FindVifChToPipeCh(ch))` (the
  pipes it feeds); clear bit ch. Then write `.08+(ch&3) [0] = 0`.
Returns the previous mask bit. Every unmask after a mask therefore resets the FIFO / pipe datapath first — the recovery
for an input that was masked and restarted. `HalVif_GetSensorMask(ch)` reads the bit. The FIFO / pipe search and reset
functions are in §4.5. The driver-level mask with its user bits is §10.6.

## 4. WDMA — the frame-mode write DMA [R]

### 4.1 Handle and engines

`HalVifIsp_Init` allocates a 0x5c-byte handle of 23 pointers into the whole-RIU mapping (handle 9): [0] 1302, [1] 1303,
[2] 1342, [3..6] 1308 + 0x000 / 0x080 / 0x100 / 0x180, [7..10] 130b + the same, [11..14] 1348 + the same, [15..18] 1349 +
the same, [19..22] 1308 / 130b / 1348 / 1349 + 0x1e0. There are therefore **16 WDMA engines** (dma id 0..15), four per
bank in 1308, 130b, 1348 and 1349, each with a 32-register block; the per-bank register-0x78 block (+0x1e0) holds fields
shared by the four. Helper `vif_reg_block(h, id)` (0x400de200) returns block [3 + id], `vif_reg_shared_block(h, id)` (0x400de25c) the shared block
[19 + id/4] (disassembly 0xde1d8 / 0xde234). `VifIspSysGetRegBase` = handle 9 (cached at .bss+0x310dc);
`VifIspSysMalloc` / `VifIspSysCalloc` / `VifIspSysFree` = CamOs allocation with a live-allocation counter.

### 4.2 Engine block registers

Registers relative to a WDMA engine block (`w.RR` = block register RR):

| reg | bits | function | meaning |
|---|---|---|---|
| w.00 | [0] / [4] / [7] | `WdmaEnable` / `WDMAAuto` / `WDMASetHighPri` | enable, auto (hardware-triggered per frame), high priority |
| w.00 | [11:8] | `WDMADataPrecision(prec)` | 0..7 as is; 10 → 6; others unchanged |
| w.00 | [13:12] | `WDMAMiuSel(m)` (m > 2 → 3) | MIU select |
| w.01 | [12:0] | `WDMASetPitch(…, fmt, width, height)` / `RawStorePipeUpdatePitch` | pitch (§4.3) |
| w.03 | [0] / [5:4] | `WDMASetCurrentFrameIndex` / `WDMASetFrameIndexMode` | frame index / mode |
| w.03 | [9:8] / [11] | `WDMAStartLineCntThreshold` / `DmaFieldModeConfig` sets [11] | start line-count threshold / field mode |
| w.03 | [12] / [13] | read: `DmaGetPushRdy` / `DmaGetPopRdy` | FIFO ready |
| w.04 | [15:0] | `WDMASetPitch`: width in units − 1 | |
| w.05 | [12:0] [15] | `WDMASetPitch`: height − 1, [15] = 1 | |
| w.06 / w.07 | [15:0] / [11:0], [15] | `RawStorePipeUpdateLimit(limit)`: limit = (addr >> 4) − 2 (28 bits), `[15]` limit enable; 0 → enable off | write limit (MIU protect) |
| w.08 / w.09 | [15:0] / [11:0] | `WDMABaseAddr(addr)`: addr >> 4 split 16 + 12 bits; addr < 16 → w.08 = 0, w.09 = 0x8000 | buffer 0 |
| w.0a | [15:0] | `WDMASetVSyncLine2RDMA` | |
| w.0c | [7:0] / [13:8] | `WDMAReqThd` / `WDMAReqHighPriThd` (> 30 → 31) | request thresholds |
| w.0d | [7:0] | `WDMAReqBustNum` | burst number |
| w.0e / w.0f | as w.08 / w.09 | `WDMABase1Addr` (addr < 16 → w.0e = 0x00ff, w.0f = 0x8000) | buffer 1 |
| w.10 | [0], [1] | `WDMASwTrig`: `[1] = 1`, then `[0] = 1` | software trigger |
| w.12 | [15] | `WDMAStatusClear`: w.16[15] = 1 (byte +0x59), w.12[15] = 0 (byte +0x49) | status clear |
| w.14 / w.15 | | read: `DmaGetDoneBase` = w.14 \| (w.15[11:0] << 16) | done address >> 4 |
| w.18 / w.1a / w.38 / w.3a | [15:0] | `DmaSetFenceId(id, slot, v)`: slot 0 → byte +0x60 (w.18), 1 → +0xe0 (w.38), 2 → +0x68 (w.1a), 3 → +0xe8 (w.3a); `DmaGetFenceId` reads +0x64 / +0xe4 / +0x6c / +0xec | fence ids |
| w.5e / w.5f | | read: `DmaGetDoneBase_1` = byte +0x178 \| (+0x17c[11:0] << 16) | second done address |

### 4.3 Pitch and formats

`VifIsp_WDMASetPitch(h, id, fmt, w, h)` writes to w.01: fmt 0 → ((w+15)·8)/128, fmt 1 → w·10/128, fmt 2 → ((w+7)·16)/128,
fmt 3 → w·12/128, fmt 4 → (w+15)/16, fmt 5 → ((w+63)/64)·5, fmt 6 → (w+7)/8, fmt 7 → ((w+31)/32)·3, fmt 10 → (w+15)/16
with a unit of 8; capped at 0xfff; another fmt → −1. The pitch is in 16-byte units [I: each formula is the line
size in bits / 128 for its storage, and the ring slots carry the pitch as stride >> 4, §12.2]. Then
`WDMADataPrecision(fmt)`, w.04 = ceil(w / unit) − 1 (unit 1 for fmt 0–3, 4 for 4–7, 8 for 10), w.05 = (h − 1) | 0x8000.

Formats by storage (from this arithmetic) [I]: 4 = 8 bit/pixel, 5 = 10-bit packed (64 pixels in 80 bytes), 6 = 16
bit/pixel, 7 = 12-bit packed (32 pixels in 48 bytes), 10 = FBC-compressed; fmt 0–3 [I: 0 YUV422 8-bit, 1 RAW10 packed,
2 16-bit, 3 RAW12 packed]. The driver (§12.1) uses only formats 4, 5, 6, 7 and 10.

### 4.4 Shared register-0x78 block (one per WDMA bank, sub = id & 3)

- `DmaFbcEnable(id, en)` → `0x78[4 + sub]`.
- `SenDataPrecision(id, p)` → `0x79`, 2-bit field at [1:0] / [5:4] / [9:8] / [13:12].
- `4Pixel_Data_Pak_AtMsb_To_Wdma(id, en)` → `0x79` [2] / [6] / [10] / [14].
- `DmaFbcWidth(id, w)` → w − 1 into register 0x58 + sub of the bank's first block (13 bits).

`VifIsp_SenDataType`, `VifIsp_SenYCOrder`, `VifIsp_DmaCompressEnable` and `VifIsp_SWReset` return 0 without effect;
`HalVifIsp_GetSubDmaInfo` returns zeros; `HalVifIsp_GetDmaInfo(id)` = (w.04, w.05[12:0]).

### 4.5 Banks 1302, 1303, 1342 (handle [0] / [1] / [2])

- `VifIsp_EnableISP(h, path, en)`: `1302.05` bit `path` (0..2).
- `VifIsp_WDMAReset(h, id)`: by bank id/4: 0 → `1302.09[3]` 0→1 pulse, and **on chip revision 1 only** also
  `1302.09[6:4] = 0` then 1 (all four banks reset together); 1 → `[4]` pulse; 2 → `[5]`; 3 → `[6]`. Then for id 8..15:
  `1342.46[id−8]` low-then-restore pulse. The chip revision of this camera (`CamOsChipRevision`) is kernel-platform.md §3.2's.
- `VifIsp_AffReset(h, g)`: `1302.09[8+g]` 0→1, then `1302.18[g] = 1` [I: async FIFO reset of group g].
- `VifIsp_RAW_Store_Mode(h, sel, en_mode, v)` (acts only when en_mode == 0): sel 0 / 1 → `1302.10[0]` / `[4]`, 2 / 3 →
  `1302.10[8]` / `[12]`, 4 / 5 → `1302.12[0]` / `[4]`, 6 → `1302.12[8]`, 7 → `1302.12[12]` and `1303.01[0] = 0`, 8..15 →
  `1302.13[sel−8]`.
- `HalVif_ResetPipeCh(mask)`: for each set bit k = 0..3 whose `1342.41[k]` is set: pulse `1342.46[k]` (clear, restore).
- `HalVif_ResetAffCh(mask)`: for each set bit k: `1302.09[8+k]` 0 → 1.
- `HalVif_FindVifChToAffCh(ch)`: bit k set if `1303.08+k [3:0] == ch >> 2` (k = 0..3; the async FIFO fed by that group).
- `HalVif_FindVifChToPipeCh(ch)`: bit p set if pipe p is routed (`1303.10+p [2] == 0`), its FIFO bit is in the AFF result
  and `1303.10+p [9:8] == ch & 3`.
- `VifIsp_RegW(h, base, reg, v)` / `VifIsp_RegWM(h, base, reg, v, mask)` (u16 write at base + reg·2) and
  `_VifIspRegBitShift(m)` (index of the lowest set bit, 16 if none) — generic helpers, no caller in the block.

## 5. Bank 1303 — FIFO mux and pipe routing [R]

Four async FIFOs (AFF) take the channels of a group; eight pipes take a FIFO's virtual channel to the ISP.

| function | registers |
|---|---|
| `FIFOMerge(sel, v)` | `1303.00`: sel 0 → `[0] = !v`; 1 → `[0] = 1`; 2 → `[4] = !v`; 3 → `[4] = 1`; other → error print |
| `FIFOMUXSelect(ch, fifo, vc_en)` | `1303.08+fifo`: `[3:0] = ch >> 2` (group), `[12:8] = (ch & 3) \| (vc_en ? 0x10 : 0)` |
| `FIFOMUXDuplicate(fifo, grp, v)` | `1303.08+fifo`: `[3:0] = grp`, `[12:8] = v` |
| `FIFOVCEnable(fifo, en)` | `1303.08+fifo [12:8] = en ? 0x10 : 0` (virtual-channel enable of the FIFO) |
| `VCMux2PipeSelectMask(ch, pipe, en)` | `1303.10+pipe`: `[1:0] = ch >> 2`, `[2] = !en`, `[9:8] = ch & 3` |

A pipe is "unrouted" when `1303.10+p [2:0] = 4, [9:8] = 3` (the form `MHal_VIF_Init` and a pipe release write). Group
configuration switches the VC enable of FIFO g for group g (`FIFOVCEnable(g, …)`, §10.1); dev 12 is additionally
hard-wired to FIFO 3 with VC enabled (`FIFOMUXSelect(12, 3, 1)` in `MHal_VIF_DevSetConfig`); MIPI HDR re-maps FIFOs
(§10.4).

## 6. Pads, pixel clock, MCLK, ISP clock [R]

### 6.1 Sensor pads

Pad-mode registers live in padtop **103c** (handle 6). Per sensor pad (case 0..3 of the function):

| function | pad 0 | pad 1 | pad 2 | pad 3 |
|---|---|---|---|---|
| `HalVifSetMIPIPad(pad, m)` | `.69[2:0]` | `.6b[10:8]` | `.69[10]` | `.6b[12]` |
| `HalVifSetParallelPad(pad, m)` | `.6a[5:4]` | `.6b[6:4]` | — | — |
| `HalVifSetBT656Pad(pad, m)` | pads 0 / 2: `.69[5:4] = 0`, `.6a[9:8] = 0`, `.6a[3:0] = m` | pads 1 / 3: `.75[1:0] = 0`, `.77[1:0] = 0`, `.6b[3:0] = m` | | |
| `HalVifSetBT1120Pad(pad, m)` | pads 0 / 2: `.69[5:4] = 0`, `.6a[9:8] = m`, `.6a[3:0] = 0` | pads 1 / 3: `.75[1:0] = 0`, `.77[1:0] = m`, `.6b[3:0] = 0` | | |
| `HalVifSetVifSnrCtrlMode(pad, m)` | `.69[5:4]` | `.75[1:0]` | `.7a[1:0]` | `.76[1:0]` |
| `HalVifSetVifSnrMclkMode(pad, m)` | `.69[6]` | `.75[4]` | `.7a[5:4]` | `.76[5:4]` |
| `HalVifSetVifSnrPdnMode(pad, m)` | `.69[9:8]` | `.75[13:12]` | `.69[15]` | `.76[12]` |
| `HalVifSetVifSnrRstMode(pad, m)` | `.69[7]` | `.75[8]` | `.69[14:13]` | `.76[9:8]` |
| `HalVif_ITU_SetPadEnable(grp, en)` | grp 0 / 1: `.58[0]` | | grp 2 / 3: `.59[0]` | |

`HalVif_SetIOPad(pad, bus)` (driver `DrvVif_SetIOPad`) takes the values from `g_VifSnrPadMode` (.data+0x1ca08, 34 entries, stride 0x21,
filled from the DT `vif_srN_*_mode` properties — list in mhal-core.md §7; `HalVifSetVifSnrPadMode` /
`HalVifGetVifSnrPadMode(i)` = byte i·0x21):
- bus 0 (parallel) → ParallelPad, Pdn, Rst, Mclk;
- bus 1 (MIPI) → MIPIPad, Pdn, Rst, Ctrl, Mclk;
- bus 3 (BT656) → BT656Pad + Mclk; bus 4 (BT1120) → BT1120Pad;
- pads 2 and 3 accept only MIPI (MIPIPad, Ctrl, Pdn, Rst, Mclk) and BT656 (BT656Pad with the mode byte of pad 0 resp.
  pad 1); pad 3 first sets `HalVif_FIFOMerge(3, 1)`;
- every MIPI arm ends with `HalVifDataIeConfig(pad, 0)`; another bus → "VIF bus type %d not support", −2.

Before the pads, `HalVifSetAllPadIn(grp 0..3, v)` → chiptop `101e.50[15] = v` (all groups share the bit); SetIOPad
writes 0 (all-pad-in off). `HalVifDataIeConfig(0, 0)`: while `103c.69[2:0] == 1`, bits 4 and 6 of `103e.1d … 103e.26`
= 0 [I: input enable of the parallel-sensor pads that share the balls].

On this camera (MIPI, pad 0, DT values of mhal-core.md §7): `101e.50[15] = 0`; `103c.69[2:0]` = `vif_sr0_mipi_mode` (1),
`103c.69[9:8]` = `…_pdn_mode` (0), `103c.69[7]` = `…_rst_mode` (0), `103c.69[5:4]` = `…_ctrl_mode` (1), `103c.69[6]` =
`…_mclk_mode` (0); then `HalVifDataIeConfig(0, 0)`. The lane count argument of the SNR SetIOPad slot is not used.

`HalVif_Ccir8BitDataPadConfig`, `Ccir16BitDataPadConfig`, `CcirCtrlPadConfig`, `CcirPclkPadConfig`, `ChannelAlignment`,
`HDRen`, `HDRSelect`, `LiSt`, `LiEnd`, `FrameStartIrqFinalStatus` / `FrameStartIrqMask` / `FrameStartIrqClr` print
"mercury6 TBD" (log bit 3) and do nothing; `HalVifSetBT656_SPI_MCLK` and `HalVif_SensorPclkPolarity` are empty.

### 6.2 Pixel-clock source, bank 113a (handle 7)

- `HalVif_SetPclkSource(ch, src, inv, gate)` (ch 0 / 4 / 8 / 12 only): value `src'<<2 | inv<<1 | gate` (7 bits) with
  src 1 ↔ 2 swapped, into ch 0 `.28[6:0]`, ch 4 `.28[14:8]`, ch 8 `.29[6:0]`, ch 12 `.29[14:8]`. MIPI passes the pad index
  as src: `113a.28[6:0] = pad'<<2` (0 on this camera).
- `HalVif_ParallelPclkSrc(grp, ch)`: grp 0 → `.27[3:0] = 0xc`, value 0x10; grp 1 → `.27[11:8] = 0xc`, value 0x18; then
  `.28[6:0] = value` and, for ch/4 = 1 → `.28[14:8]`, ch/4 = 2 → `.29[6:0]`; another grp → error.
- `HalVif_BT656SE1MX` / `DE1MX` / `SE2MX` / `DE2MX` / `SE4MX` / `DE4MX(n)` (single / dual edge × 1 / 2 / 4-channel mux;
  all six are the same code): `.27[3:0] = 0xc`, `.28[6:0] = 0x40`, and n 1 → `.28[14:8] = 0x60`, 2 → `.29[6:0] = 0x60`,
  3 → `.29[14:8] = 0x40`.

### 6.3 MCLK, bank 1038 (handle 8)

`HalVif_SetMCLK` (SNR slot MCLK → `DrvSensorSetMCLK` → `DrvVif_SetMclk` → `HalVif_SetMCLK`): writes
`1038.62[3:0] = 0`, then the pad's 6-bit field = `sel<<2 | !on` (field bit 1 = invert, forced 0 — bit 9 for pad 0):
pad 0 `1038.62[13:8]`, pad 1 `1038.65[13:8]`, pad 2 `1038.65[5:0]`, pad 3 `1038.66[13:8]`. The source index sel comes from `HalVif_ConvertMCLK(kHz,
*idx)`:

| kHz | 27000 | 72000 | 61700 | 54000 | 48000 | 43200 | 36000 | 24000 | 21600 | 12000 | 5400 | 37125 | other |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| sel | 0 | 1 | 2 | 3 | 4 | 5 | 6 | 7 | 8 | 9 | 10 | 11 | −1 |

The sensor ABI's `CUS_MCLK_FREQ` codes map to kHz in the SNR layer (0 27000, 1 21600, 2 12000, 3 5400, 4 36000, 5 54000,
6 43200, 7 61700, 8 72000, 9 48000, **10 24000**, 11 / 12 37125). This camera's 24 MHz: `1038.62[13:8] = 0x1c` (camclk
`sr00_mclk` parent 7 = `xtali_24m`), off `0x1d`.

- `HalVif_QueryMCLK` (SNR slot QueryMCLK): writes nothing; answers from the 37.125 MHz support flags.
- `HalVifSetMCLKSupport(pad, kHz, v)` / `HalVifGetMCLKSupport`: only 37125 kHz is a valid key
  (`g_VifMclkSupport_37125[pad]`, set from DT `vif_srN_mclk_37p125`); Get returns 0 if supported, −1 if not.
- `HalVif_SetMCLK_Ext(?, ch)` (`MHal_VIF_SetMclk` → `DrvVif_SetMclk_Ext` lands here): ch 2 → nothing; ch < 6 →
  "un-implement!"; else "[VIF] Invalid MCLK Setting for channel for BT656".

### 6.4 ISP clock, parallel enable, global init

- `HalVif_IspClkCtrl(sel, on)`: prints "%s ISP_CLK:%d"; `1038.61[12:8] = (!on) | (sel << 2) & 0x1d` (ISP clock gate +
  source). No caller in the VIF block; the caller is the ISP block.
- `HalVif_EnableParallelIF(grp, en)`: grp 0 only → `1304.00[0] = (en ≠ 0)` (the SW-reset bit); another grp → error print.
- `HalVif_GlobalInit` = `SensorChannelEnable(ch, 0)` for ch 0..15.

### 6.5 Sensor reset and power-down pins

`DrvVif_SensorPdwn(pad, v)` (SNR slot PowerOff via `DrvSensorPdwn`) → `VIFn.00[3] = (v == 0)`; `DrvVif_SensorReset(pad, v)`
(SNR slot Reset via `DrvSensorReset`, and the `/dev/vif` ioctl, §15) → `VIFn.00[2] = (v == 0)`; the channel is
`HalVif_GetChnBySNRPadID(pad)`, so only channels 0 / 4 / 8 / 12 (banks 1304–1307) are reached. For pad 0: `1304.00[3]` and
`1304.00[2]`. The SNR side skips both while sensor early init is on (mhal-csi-snr.md §3.14).

## 7. OTP checks [R]

The eFuse words and the class rule are ipl.md §7 (`101f.2e`, `.2f`, `.26`, `.2c`); their values on this chip are not known.

### 7.1 Pipe and port capability

`HalVif_CheckOTPPipeCapability(pipe)` / `HalVif_CheckOTPPortCapability(port)` read eFuse words `101f.2e`, `.26`, `.2f`,
`.2c`: s = `.2e & 3`, a = `.26 & 0x800`, t = `.2f & 3`, b = `.2c & 0x800`. **Full chip** if s == 0, or s == 3 and (t == 0
or t == 3 or b == 0), or s ∈ {1, 2} and a == 0. Full → every pipe and port allowed (1). Cut-down → pipes 0–5 allowed,
6–7 refused; port 0 allowed, port 1 refused (0). `MHal_VIF_PipeSetConfig` runs the pipe check, every `PortSetConfig` the
port check.

### 7.2 Input throughput cap (`HalVif_OTP_VENC_Fps_Capability`)

Despite its name this is a VIF-side cap on the input pixel rate, applied to frame-mode ports. Called with (pipe, 1) by the
frame-start handler from the 3rd to the 8th frame of a frame-mode port (§12.4), and with (pipe, 0) by every PortDisable.
- It reads `101f.26` (`HalVifReadRegWithBankAddr(0x101f4c)`) and uses **bit 2 only** — no class s / t test as in §7.1
  [R]. Bit 2 = 0 → return 1 (no cap).
- (pipe, 0) clears the device's state and returns 1.
- If the device's "over" flag is set → mask TDM bits 0–1, clear all four, channel off (the HDR pair too), clear the
  state, return 0. The second channel of an HDR pair (`ch & 3 ≠ 0` with HDR on) returns 1.
- Else for 5 frames accumulate fps × w × h (fps = 10⁹ / frame interval ns from `DrvVif_IntGet`, w / h from
  `HalVif_GetInfo`; ×2 for HDR) and store the average; when the device average or the sum over all 16 devices exceeds
  **0x9FC9000 = 167 546 880 pixels/s** → `HalVif_SensorMask` on the channel (and its pair), set "over", print
  "Over Spec!!!", return 0.
- State right after `g_VifMclkSupport_37125`: +0x44 count[16], +0x54 over[16], +0x64 sum[16], +0xa4 average[16].

The encoders' cap, derived from the same words by another rule, is 167 116 800 pixels/s (ipl.md §7.3). The camera's
3840×2160 at 30 fps is 248.8 Mpx/s, so on a chip with bit 2 set a frame-mode port at that rate would be cut [I]; the
realtime path never calls the check, so 4K30 reaching the ISP does not tell whether the bit is set [I].

## 8. Driver state [R]

**Context** at `.bss+0x31628` ("ctx" below; all offsets relative to it). The symbol `vif_drv_ctx` starts at ctx+0x10.
The per-group and per-device mode words occupy ctx+0x210…+0x27c.

| offset | content | accessors |
|---|---|---|
| +0x10 / +0x14 / +0x18 / +0x1c + ch·0x10 | event callbacks of channel ch: frame start / line count / frame end / FIFO full | `DrvVif_Register{FrameStart,LineCnt,FrameEnd,FifoFull}Cb` (§9.6) |
| +0x110 / +0x114 / +0x118 / +0x11c + ch·0x10 | private data of the four callbacks | same |
| +0x210 + g·4 | interface mode of group g (§9.2; initial 5 = none) | `DrvVif_SetIntfMode` / `GetIntfMode` |
| +0x220 + dev·4 | work mode of dev: 0 realtime, 1 frame (initial 2 for devs 0–3) | `DrvVif_SetWorkMode` / `GetWorkMode` |
| +0x260 + g·4 | multiplex mode of group g (§9.2) | `DrvVif_SetMultiplexMode` / `GetMultiplexMode` |
| +0x270 + g·4 | HDR mode of group g | `DrvVif_SetHdrMode` / `GetHdrMode` |
| +0x280 + ch·0x58 | the channel's current mhal ring element (passed with the frame-end event, §12.2) | `DrvVif_SetMetaDataCnt` |
| +0x814 / +0x818 + dev·0x18 | parallel vsync / hsync polarity words — never written anywhere in mhal, so both are always 0 | read by parallel DevSetConfig |
| +0x994 | runtime-HDR buffer handle | `DrvVifGetRtHdrBufHandle` |
| +0xa18 + pipe | context byte (= pipe) of the WDMA-done ISRCB | PortSetConfig |
| +0xa78 | FIFO-full ISRCB handles | GroupEnable / GroupDisable |
| +0xab8 | "use default pad map" flag; cleared by `MHal_VIF_SetVIFDev2SensorPadMux` | `DrvVif_SetUseDefaultPadMap` / `IsUseDefaultPadMap` |
| +0xab9 | dev → (pad, plane) map, 5 bytes per group (initial 0xff) | `DrvVif_SetDev2SNRPadMap`, `DrvVif_GetDev2SNRPadID` |
| +0xad0 + ch·4 | channel active ("PhyChnStatus"; the ISR loops over these) | `DrvVif_SetPhyChnStatus` / `GetPhyChnStatus` |
| +0xb10 + pipe·4 | pipe → channel (0xff = free; = `vif_drv_ctx`+0xb00) | `DrvVif_SetPhyChn2Pipe`, `GetPhyChnByPipe`, `GetVifChnByPipe`, `GetPipeByPhyChn` (search, 0xff if none) |
| +0xb30 | pipe × port status | `DrvVif_SetPipe2PortStatus` / `GetPipe2PortStatus` |
| +0xb70 + g·4 | minimum non-zero height of the group's channels, BT656 (initial 0x870 = 2160) | BT656 DevSetConfig |
| +0xb80 | skip-start-frame flags (cleared after one skip) | `DrvVif_SetSkipStartFrame` / `GetSkipStartFrame` |
| +0xbc0 | stream mask | `DrvVif_SetStreamMask` / `GetStreamMask` |
| +0xc00 + ch·4 | MIPI HDR pair flag | `DrvVif_SetISR_MIPI_HDR` / `GetISR_MIPI_HDR` |
| +0xc48 | per-channel image info (w, h) | `DrvVif_SetChnImageInfo` / `GetChnImageInfo` |
| +0xd40 | port work mode per pipe × port | `DrvVif_SetPortWorkMode` / `GetPortWorkMode` |
| +0xd80 | total frame count | `DrvVif_ResetTotalFrameCount` / `GetTotalFrameCount` |
| +0xdc0 | shared input-mask user bits, one byte per channel; mutex at +0xf30 | §10.6 |
| +0xe00 | pointer to the ring area (§12.2) | `initialSHMRing` |
| +0xe08 + i·0x10 | 64-bit timer accumulators | `GetTimerCnt64` (§12.7) |
| +0xf08 | mutex destroyed by `unInitialSHMRing` | |
| +0xf58 | pending input-mask change (HDR frame-sync) | `DrvVif_TriggerMaskChange` |

Mutexes: one each for DevSetConfig, Dev enable / disable and PortSetConfig.

Other state, each described where it is used: the "masked" bitmap of `HalVif_SensorMask` (§3.3); the throughput-cap
state after `g_VifMclkSupport_37125` (§7.2); `gtCurVifIrqTs` and `gVifIntStats` (§11); `ulg_itu_tdm_vif_def_mask`
(§11.2); `pFuncGetPts` (§9.1); `g_VifSnrPadMode` (§6.1); the ring area (§12.2); the WDMA handle (§2.1) and the whole-RIU
base cache (§4.1); the log mask (§15); the buffer-allocator ops (§9.2).

## 9. The MHal_VIF API [R]

### 9.1 Entry points

Registers in the right column are those of channel 0 (bank 1304) where a channel is meant. Per-bus detail: §10.

| export (caller) | what it does | registers |
|---|---|---|
| `MHal_VIF_Init` (mi_vif, first open) | `DrvVif_InputMask(ch, 1, 0)` for ch 0..15 (input mask on), `DrvVif_SetDefaultIntsMask` (all VIF interrupts masked), `DrvVif_SetDefaultPipeConfig` (`VCMux2PipeSelectMask(3, p, 0)` and pipe table 0xff for p = 0..7) | `VIFn.08+(ch&3) [0] = 1` for every channel; masks §11.2; `1303.10+p [2:0] = 4, [9:8] = 3` (pipe unrouted) |
| `MHal_VIF_Deinit`, `MHal_VIF_Reset`, `MHal_VIF_PipeQuery` | return 0 | — |
| `MHal_VIF_GroupSetConfig(g, cfg, ops)` | dispatch by interface (§10.1); MIPI → `DrvVif_GroupSetConfig_MIPI` | MIPI: `1303.08+g [12:8] = 0x10` (FIFO VC enable) |
| `MHal_VIF_GroupEnable(g)` | `ISRCB_RegisterCallback(0x38+g, DrvVif_DoVifFifoFullEventCb, g)` (BT656 / BT1120 also release the bank, §10.1) | MIPI: none |
| `MHal_VIF_GroupDisable(g)` | unregister the FIFO-full callback, FIFO VC off | `1303.08+g [12:8] = 0` |
| `MHal_VIF_GroupReset(g, 0)` | `VifIsp_AffReset` | `1302.09[8+g]` 0→1 pulse, `1302.18[g] = 1` |
| `MHal_VIF_DevSetConfig(dev, cfg)` | under a mutex; generic part (§9.3), then by bus — MIPI → `DrvVif_DevSetConfig_MIPI` (§10.2) | MIPI: `113a.28[6:0]`, table 2, `VIFn.06[12]`, table 4, `VIFn.4b`, `VIFn.00[12:8]`, `VIFn.3c[1]`, table 6 |
| `MHal_VIF_DevEnable(dev)` | mark the channel active; MIPI / parallel: sensor SW reset released (§10.3) | `VIFn.00[0] = 1` |
| `MHal_VIF_DevDisable(dev)` | mark inactive; channel off, SW reset asserted | `VIFn.00[15] = 0`, `VIFn.00[0] = 0` |
| `MHal_VIF_PipeSetConfig(dev, pipe, cfg)` | OTP pipe check; route / release (§9.4) | `1303.10+pipe` |
| `MHal_VIF_PortSetConfig(pipe, port, cfg)` | OTP port check; needs the pipe routed; `cfg+0 = 0` → `PortDisable_<bus>`; else configure (§10.4) | MIPI realtime Bayer port 0: `VIFn.6a = 0xf << 4·(ch&3)` (clear), `VIFn.60` nibble `&= ~3` (unmask bits 0–1), `VIFn.00[15] = 1` |
| `MHal_VIF_InputMask(dev, m)` (mi_vif, mi_isp) | user 0 of the shared mask (§10.6); HDR modes 1 / 2, and mode 4 in realtime: frame-synced on the pair; then polls the hardware bit until it matches (≤ 2000 × 500 µs; both channels for HDR) | `VIFn.08+(ch&3) [0]` |
| `MHal_VIF_GetInputMask(dev, *m)` | the hardware bit (HDR: AND of the pair) | read `VIFn.08+(ch&3) [0]` |
| `MHal_VIF_DevReset(dev, t)` | t 0 → `DrvVifIsp_SWReset` = `VifIsp_SWReset` twice, an empty HAL function — **a no-op**; 1 → WDMA reset; **2 → AFIFO + WDMA reset** (`DrvVifIsp_AFIFO_WDMAReset`, mi_vif's FIFO-full recovery, §12.1) | WDMA banks + `1302.09` / `.18` |
| `MHal_VIF_RegisterEventCallback(dev, type, fn, priv?, data)` | type 0 frame start, 1 frame end, 2 line count, 4 FIFO full; 3 and others → −1 (§9.6) | — |
| `MHal_VIF_RegisterGetPtsCallback(fn)` | `pFuncGetPts = fn` (used by the frame-mode handlers) | — |
| `MHal_VIF_PortStatQuery(pipe, port, *st)` | ring and WDMA status (§12.5) | reads only |
| `MHal_VIF_QueueFrameBuffer`, `MHal_VIF_DequeueFrameBuffer`, `MHal_VIF_QueryFrames` | frame-mode buffer ring (§12.3) | — |
| `MHal_VIF_SetVIFDev2SensorPadMux(g, {u8 pad; u32 plane[4]})` / `MHal_VIF_GetVIFDev2SensorPadMux` | write / read the dev → (pad, plane) map of the group's 4 devs; Set clears the "use default pad map" flag | — |
| `MHal_VIF_SetMclk(g, ?)` | → `DrvVif_SetMclk_Ext` → `HalVif_SetMCLK_Ext`: prints "un-implement!" (§6.3) | — |
| `MHal_Vif_GetSensorFrmCnt64` | empty stub, no return value set (the ISP calls it; `DrvVif_GetSensorFrmCnt64` is empty too) | — |

### 9.2 Group configuration (`MHal_VIF_GroupSetConfig(group, cfg, ops)`, 0x30 bytes)
<!-- ev:mhal-vif.9.2 -->

Field use read from the mhal consumers and cross-checked against the mi_vif writer
(pipe cfg at `g_stVifModRes+0x4b4c+pipe·100` = `.bss`
0x252cc, port cfg at `+0x4b58+port·0x28` = 0x252d8; mi-capture.md §3.2, §3.5) and the `DrvVifUt` text commands `groupcfg`, `devcfg`,
`pipecfg`, `portcfg` (§15), which build the same structures. Enumeration names marked [S] are from the I2 header
`mhal/i2/vif/drv/pub/mhal_vif_datatype.h` of the public `jockyw2001/sdk` tree; their M6 values are confirmed by the dispatch.

| off | field | mhal use |
|---|---|---|
| +0x00 | interface: 0 BT656, 1 parallel ("digital camera"), 2 BT1120 standard, 3 BT1120 interleaved, 4 MIPI (`MHal_VIF_IntfMode_e` [S]) | dispatch; stored ctx+0x210+g·4 |
| +0x04 | work mode: 0 / 1 / 2 = 1 / 2 / 4-channel multiplex (BT656 / BT1120), 3 RAW realtime, 4 RAW frame mode (`MHal_VIF_WorkMode_e` [S]) | BT656 / BT1120: TDM mux; stored as "multiplex" ctx+0x260+g·4 (MIPI and parallel store 3) |
| +0x08 | HDR mode: 0 none; 1, 2 two exposures on a channel pair (VC / DOL); 3 accepted, no pair; 4 line-interleave | MIPI only: ctx+0x270+g·4; modes 1 / 2 / 4 on group 0 store `ops` as the HDR buffer allocator |
| +0x0c | clock edge: 0 single up, 1 single down, 2 double (`MHal_VIF_ClkEdge_e` [S]) | BT656 / BT1120: SE (0) / DE (2) pixel-clock mux; 1 → "clk edge not support" |
| +0x10 / +0x14 / +0x18 | `DrvVifUt` fills bit 1, bit 0, bit 2 of its "PinPol" argument; mi_vif writes 0 | **not read** by any mhal path |
| +0x1c … +0x24 | zero | not read |
| +0x28 / +0x2c | mi_vif: mclk, scan mode | not read |

`ops` (5 function pointers: alloc, free, map, unmap, flush — mi_sys private-heap wrappers) is stored by
`DrvVif_SetMemAllocator` (`DrvVif_GetMemAllocator` reads it back) and used only by the HDR / frame-mode buffer code
(`DrvVifGetSysMemAlloc`, `_DrvVifSysMalloc` / `_DrvVifSysMfree` / `_DrvVifSysMemMap` / `_DrvVifSysMemUnMap` /
`_DrvVifSysFlashCache`, §15).

### 9.3 Device configuration (`MHal_VIF_DevSetConfig(dev, cfg)`, 0x28 bytes)

| off | field | use |
|---|---|---|
| +0x00 / +0x02 / +0x04 / +0x06 | u16 crop x, y, w, h | crop table, image info |
| +0x08 | pixel format (mhal code: 0 YUV422 YUYV, 0xe / 0xf / 0x10 the other YUV422 orders, 0x2a… Bayer = 0x2a + Bayer id) | RGB-in, YUV order |
| +0x0c | precision index 0..4 (8 / 10 / 12 / 14 / 16-bit; `DrvVifUt devcfg` passes (bits − 8)/2) | format table (§3.2) |
| +0x10 | frame mask code | BT656: `HalVif_FrameMask(ch, cfg+0x10)` at the end; MIPI / parallel ignore it; mi_vif writes "interlaced" here |
| +0x14 | line-count-1 value | see +0x1c |
| +0x1c | line-count callback | if ≠ 0, `MHal_VIF_DevSetConfig` programs `Vif2IspLineCnt1(ch, cfg+0x14)` and registers +0x1c as the channel's line-count callback before the bus-specific part; mi_vif leaves +0x14 and +0x1c 0 |
| +0x18, +0x20, +0x24 | — | not read |

Dev 12 additionally gets `HalVif_FIFOMUXSelect(12, 3, 1)` (the fourth async FIFO hard-wired to channel 12, VC enabled).

### 9.4 Pipe configuration (`MHal_VIF_PipeSetConfig(dev, pipe, cfg)`, 8 bytes)

+0 enable (1 = route dev's channel to the pipe, 0 = release; any other value → return 0 doing nothing); +4 "HDN_EN" =
YUV horizontal ½ downscale request (mi_vif copies a byte of its dev attribute, the H2T1P flag).

mhal: OTP pipe check (§7.1; pipe ≥ 6 refused on a cut-down chip); remember the channel for the pipe (ctx+0xb10+pipe·4,
0xff free); `VCMux2PipeSelectMask(ch, pipe, en)` (§5);
`HalVifIspscl_Yuv2B_H_DN(pipe, en_hdn, yc_swap)` with en_hdn = HDN_EN and yc_swap = (`ITU_TDM_GetYCOutSwap(ch)` ≠ 0 &&
HDN_EN) — on release both 0 and the routing set to "channel 3, disabled". The forwarder calls
`DrvIspScl_Yuv2B_H_DN(0, pipe, en_hdn, yc_swap)` in the ISPSCL block (§13). On this camera: `1303.10 = 0x0000`, YUV
H-downscale off.

### 9.5 Port configuration (`MHal_VIF_PortSetConfig(pipe, port, cfg)`, 0x28 bytes)

| off | field (mi_vif source) | mhal use |
|---|---|---|
| +0x00 | enable (1) | 0 → `PortDisable_<bus>` |
| +0x04 | FBC / compression (dev attr +0xe8) | frame mode: compressed WDMA (format 10) + FBC width |
| +0x08 / +0x0a | u16 crop x / y (port attr) | port 1 (YUV→Bayer crop) |
| +0x0c / +0x0e | u16 crop w / h (h halved for both-field) | port 1 |
| +0x10 | output height (dev attr +0xc6, halved for both-field) | port 1 WDMA height / YUV2B out h |
| +0x14 | output width (dev attr +0xc4) | port 1 WDMA width / YUV2B out w |
| +0x18 | work mode 0 realtime / 1 frame (mi_vif: bind type ≠ 4) | `DrvVif_SetWorkMode(dev)`, `SetPortWorkMode` (−1 = unsupported), `YUV2B_SetRunningMode(pipe, ==1)` |
| +0x1c | pixel format (mhal code, dev attr +0x10c) | YUV-in / YUV→Bayer decision |
| +0x20 | frame-rate code (0 full, 1 ½, 2 ¼, 3 ⅛, 4 ¾ — `MHal_VIF_FrameRate_e` [S]) | `DrvVif_setChnFPSBitMask(ch, port, code)` (frame mode only, §12.6) |
| +0x24 | both-field flag (field == 3) | BT656 frame mode: `WDMASetFrameIndexMode(pipe, 3)` (field combine); YUV→Bayer: "Unsupport Field combine mode" |

Port work mode (`DrvVif_SetPortWorkMode`): **realtime is accepted only on pipes 0–2, port 0** — `(pipe < 3 && port == 0)
|| mode ≠ 0`, else −1 "unsupport workmode".

### 9.6 Event callbacks

`MHal_VIF_RegisterEventCallback(dev, type, fn, priv?, data)` stores per channel through
`DrvVif_Register{FrameStart,FrameEnd,LineCnt,FifoFull}Cb(ch, fn, ?, priv)` (storage: §8). Callback signature `fn(chn, type, ring_elem*, priv)`.
`DrvVif_DoVif*Cb(ch)` call `fn(ch, type, elem, priv)` with type 0 (frame start), 1 (frame end), 3 (line count) and 4 (FIFO
full): **the line-count event is delivered with type 3 by `DrvVif_DoVifLineCntCb` although mi_vif registered it as type 2**.
`elem` is NULL except for frame end, which passes the channel's current 0x58-byte ring element (§8, §12.2).
The FIFO-full one also prints "DrvVif_DoVifFifoFullEventCb Fifo=%d"; it is raised through ISRCB ids 0x38–0x3b by the ISP
block's interrupt (mhal-core.md §3.4).

## 10. Configuration per bus [R]

### 10.1 Group: SetConfig, Enable, Disable

`GroupSetConfig` by interface:
- **MIPI** (4): `FIFOVCEnable(g, 1)`; store interface, multiplex 3 and HDR mode; HDR modes 1 / 2 / 4 on group 0 keep
  `ops` as the HDR buffer allocator.
- **BT656** (0): `ITU_SetPadEnable(g, 1)`; `ITU_TDMConfig(g, work, edge)`; `ITU_TDMEnable(g, 1)`; pad =
  `GetSNRPadIDByGroupID(g)`; `DrvVif_SetIOPad(pad, 3 (BT656), padmode[9])`; by work mode: 1 (2-mux) → `FIFOVCEnable(g, ?)`
  + `BT656SE2MX` / `DE2MX(g)`; 2 (4-mux) → `FIFOVCEnable(g, 1)` + `SE4MX` / `DE4MX`; 0 → `SE1MX` / `DE1MX`; edge must be
  0 or 2. Then on channel 4g: `IFStatusReset(0)`, `SelectSource(5)`, `BT656InputSelect(0)`, `SensorBitSwap(0)`,
  `SensorHsyncPolarityBt656` / `SensorVsyncPolarityBt656`, `SensorRgbIn(0)`, `SensorHsDly(1)`,
  `BT656ChannelDetectEnable(1)`, `BT656VSDelay(3)`, `BT656HorizontalCropSize(0xffff)`; store interface and multiplex =
  work mode.
- **BT1120** (2 / 3): the same with `SetIOPad(pad, 4, padmode[11])`, then only `IFStatusReset(0)`, `SelectSource(6)`,
  `SensorRgbIn(0)`.
- **Parallel** (1): `FIFOVCEnable(g, 0)`, return 0; store interface, multiplex 3.
- Another interface → "Invaild device interface mode", returns 0.

`GroupEnable(g)`: BT656 / BT1120: `SensorSWReset(4g, 1)` (release) + the FIFO-full callback on ISRCB 0x38+g; parallel /
MIPI: the callback only. `GroupDisable(g)`: BT656 / BT1120: channel 4g off and in reset, image info of the 4 channels
cleared, the group's BT656 minimum height reset to 0x870 (2160), callback unregistered; parallel / MIPI: callback
unregistered; all: `FIFOVCEnable(g, 0)`.

### 10.2 Device: SetConfig

- **MIPI** (`DrvVif_DevSetConfig_MIPI`): store w / h; pixel-clock source = the pad (`113a.28[6:0] = pad'<<2`, pad 1 ↔ 2
  swapped); `FrameMask(ch, 0)` (frame mask off); format table (§3.2); `SensorRgbIn` = (format is Bayer); crop table
  (x, x+w−1, y, y+h−1) + `CropEnable`; `SelectSource(0)` (`VIFn.00[12:8] = 0`, `VIFn.3c[1] = ch & 1`); line count 0 at
  h − ⌊0.3·h⌋ (2160 → **1512**). HDR branches: mode 1 / 2 → the paired channel (next dev) gets the same format, crop and
  line count, and `HDRen` / `HDRSelect` (both "TBD" stubs); mode 4 (line interleave) → both channels get crop height 2h,
  the pair's source / pixel clock = the pad, `LiCh` 0 / 1, `LiModeEn` 1, `LiSt` / `LiEnd` — stubs, so the interleave
  window is never set [I: line-interleave HDR does not work in this build]. For the paired channel the code passes the
  precision's c-value as the `RgbIn` argument [R; I: a slip].
- **BT656 / BT1120** (`DevSetConfig_BT656`, used for both): store w / h as channel image info; `FrameMask(ch, 0)`; the
  group's minimum non-zero height over its 4 channels is kept (ctx+0xb70+g·4, initial 0x870); for all 4 channels of the
  bank: `SensorFormatGroup(ch, 0, 1, 2)` and line count 0 = hmin − hmin·k/1000 (k = 300 / 400 / 500 for hmin < 480 /
  < 720 / ≥ 720); YUV order by format (disassembly 0xdab68): 0 → UVSwap 0, YCOutSwap 1; 0xe → UVSwap 0, YCOutSwap 0; 0xf →
  UVSwap 1, YCOutSwap 1; 0x10 → UVSwap 1, YCOutSwap 0; another format → neither is written; finally
  `FrameMask(ch, cfg+0x10)`.
- **Parallel**: precision index p (cfg+0x0c) 0 / 1 → p, 2 → 3, 4 → 2, other → −1; `ParallelPclkSrc(pad, ch)`; HDR stubs;
  `SensorRgbIn(1)`; `FrameMask(0)`; `SensorFormatGroup(ch, 0, 0, p')`; `SensorHsyncPolarity(ch, ctx+dev·0x18+0x818)`,
  `SensorVsyncPolarity(ch, …+0x814)` — both words are never written, so both polarities are always 0; crop + enable;
  `SelectSource(3)`; line count 0 = h − h/10.

### 10.3 Device: Enable, Disable

`DevEnable(dev)`: mark the channel active (ctx+0xad0+ch·4 = 1 — the ISR's "PhyChnStatus"); MIPI / parallel:
`SensorSWReset(ch, 1)`, for HDR modes 1 / 2 / 4 also the next channel; BT656 / BT1120: nothing else (the bank was released
at GroupEnable). `DevDisable(dev)`: mark inactive; MIPI / parallel: channel off + reset (the HDR pair too when the
device's work mode (ctx+0x220+dev·4) is 1 = frame and the HDR mode is 1 / 2, or the HDR mode is 4).

### 10.4 Port: SetConfig

Every bus starts with the OTP port check (§7.1), requires the pipe to be routed, sets the port work mode (§9.5) and
calls `resetSHMRingByChnID(pipe + 8·port)` (§12.2).
- **MIPI**, Bayer, port 0, realtime (the camera's case): read the channel format back; **no DMA, no YUV2B**; clear and
  unmask interrupt bits 0–1, enable the channel; `YUV2B_SetRunningMode(pipe, 0)`.
- **MIPI** otherwise: HDR pair (mode ≠ 0): both channels flagged (ctx+0xc00+ch·4 = 1); realtime HDR → FIFO 0 = paired
  channel, FIFO 1 = main channel, `VCMux2PipeSelectMask(4, 1)` + routing of the pair to pipe+1; frame-mode HDR → pipe 7
  refused, widths ≥ 2560 (0xa00) only on channel 0 ("unsupport over 8M resolution"), FIFO / pipe re-mapping as in the
  decompile; port 0 YUV input and YUV→Bayer exactly as BT656 below; port 0 Bayer frame mode → WDMA `in`
  = precision 0..3, or 10 with FBC; port 1: YUV input only ("Port 1 only support YUV422 Input"), frame mode only,
  YUV→Bayer through ISPSCL instance pipe+8 with the port's crop and output size, WDMA on dma pipe+8, ISRCB
  `GetPipePort2ISRCBID(pipe, 1)`. Frame mode always: WDMA-done ISRCB + `WDMAAuto`; then bits 0–1 unmasked and the
  channel enabled (HDR: both).
- **BT656 / BT1120**: port 0 with YUV input: YUV out (format 0 / 0xe–0x10): YUV2B config with in = (w, or w/2 with
  H_DN), h, YUV2B disabled; frame mode: WDMA `in` 2 (YUV422 16-bit) or 10 (FBC); field combine → frame index mode 3;
  realtime: `PreviewRAWStoreModeDisable(pipe)`, pipe 2 → `YUV_RealTimePipe_Settle(w, h, 1)`. YUV→Bayer (format
  0x2a–0x2d): Bayer id 0–3, YUV2B config (+ YC swap flag when the bank's YCOutSwap is set and H_DN off) and enable; frame
  mode WDMA `in` 3 (or 10). RGB input on BT656 → "Unsupport RGB sensor format in". Then (if not already) clear / unmask
  bits 0–1, channel enable; frame mode registers the WDMA-done ISRCB.
- **Parallel**: port 1 refused ("Parallel sensor Unsupport VifPort1"); frame mode: WDMA `in` = precision 0..3 (or 10
  with FBC), WDMA-done ISRCB (`GetPipePort2ISRCBID(pipe, 0)`, context byte ctx+0xa18+pipe), `WDMAAuto`, FPS mask; then
  clear / unmask bits 0–1 and channel enable.

WDMA `in` is the input code of `DrvVifIsp_WDMAConfig` (§12.1), which maps it to a storage format.

### 10.5 Port: Disable

`PortDisable_<bus>` (MIPI 640 bytes, BT656 300 bytes, parallel 184 bytes of code; same shape): YUV2B config zeroed and
disabled, WDMA auto off, ISRCB unregistered, TDM interrupt bits 0–1 masked, `HalVif_OTP_VENC_Fps_Capability(pipe, 0)`
(throughput-cap state reset, §7.2), port status cleared.

### 10.6 Input mask (driver)

`DrvVif_InputMask(ch, m, user)` and `DrvVif_InputMaskFrameSync` keep a per-channel byte of user bits (ctx+0xdc0; bit
`user`; users: 0 = `MHal_VIF_InputMask` / `MHal_VIF_Init`, 1 = the SNR skip-frame timer, mhal-csi-snr.md §3.10) and mask while
any bit is set. `DrvVif_InputMask` writes the hardware at once (`HalVif_SensorMask`, §3.3); the frame-sync form only
records the change, and `DrvVif_TriggerMaskChange(ch)` makes the next line-count-0 interrupt apply it to ch and ch+1 (HDR
pairs, §11.1). `DrvVif_CheckInputMask(ch, m)` polls the hardware bit ≤ 2000 × 500 µs; `DrvVif_GetInputMask` returns the
combined value; `DrvVif_CheckMaskStatus` is a thunk. `DrvVif_SharedInputMaskInit` / `…Release` create and free the state
(mutex ctx+0xf30).

### 10.7 Sequence on this camera (mi_vif and mi_isp calls)

1. `MHal_VIF_Init` — all channel input masks on, all VIF interrupts masked, pipes 0–7 unrouted.
2. `GroupSetConfig(0, {MIPI, …, HDR 0})` → `1303.08[12] = 1`; `GroupEnable(0)` → FIFO-full callback on ISRCB 0x38.
3. `DevSetConfig(0, {0, 0, 3840, 2160, Bayer RG, 10-bit})` → `113a.28[6:0] = 0`; frame mask off; format table = 0x10;
   `1304.06[12] = 1`; crop table (0, 3839, 0, 2159); `1304.4b[0] = 1`; `1304.00[12:8] = 0`, `1304.3c[1] = 0`; line-count
   table 1512.
4. `DevEnable(0)` → `1304.00[0] = 1`; mi_vif registers its frame-start / frame-end / line-count / FIFO-full callbacks.
5. (bind to the ISP, realtime) `PipeSetConfig(0, 0, {1, 0})` → `1303.10 = 0`; YUV H-downscale off in ISPSCL.
6. `InputMask(0, 1)`; `PortSetConfig(0, 0, {en, realtime, Bayer RG, …})` → `1304.6a = 0xf`, `1304.60[1:0] = 0`,
   `1304.00[15] = 1`; `InputMask(0, 0)` → `1304.08[0] = 0` once the hardware bit follows.

Shutdown reverses it: `PortSetConfig(en 0)` → `PortDisable_MIPI` (§10.5), `PipeSetConfig(en 0)`, `DevDisable`,
`GroupReset(g, 0)`, `GroupDisable`. The pins and MCLK of pad 0 are driven by the SNR block through §6.1, §6.3 and §6.5
(its sequence: mhal-csi-snr.md §3.4).

## 11. Interrupts [R]

### 11.1 The live handler

One line, GIC SPI 0x59 → `vif_isr` → `DrvVif_ITU_TDM_ISR(…, pts*)`, which loops over the 16 channels marked active by
DevEnable. Per channel 4 status bits: status = `VIFn.70` nibble (`>> 4·(ch&3)`) & ~`VIFn.60` nibble; clear by writing
`bits << 4·(ch&3)` to `VIFn.6a`. On entry `gtCurVifIrqTs[ch]` = now (CamOs time of day, s·1e9 + ns, 32-bit). Per bit:

- **bit 0 = VREF falling (frame start)**: pipe = `GetPipeByPhyChn(ch)`; if the channel is not the second channel of an
  HDR pair: the frame-start handler for each enabled port of that pipe (`DrvVifFrameStartHandler(pipe or pipe+8)`,
  §12.4), then — unless a start-frame skip is set (`DrvVif_SetSkipStartFrame` / `GetSkipStartFrame`, cleared after one
  skip) — mi_vif's frame-start callback (type 0) and `ISRCB_Proc(ch + 8)`. For sub-channel 1 of an HDR pair the same is done
  on behalf of ch − 1. ISRCB ids 8–0x17 have no listener (mhal-core.md §3.4). Interval: `gVifIntStats[ch]+0x24` = now −
  previous frame start (ns). Counter [0]++.
- **bit 1 = line count 0 reached** — the line programmed in table 6 word 0: h − ⌊0.3·h⌋ for MIPI, h − ⌊0.1·h⌋ for
  parallel, h − ⌊0.3 / 0.4 / 0.5·h⌋ for BT656 by height class < 480 / < 720 / ≥ 720. A pending HDR mask change is applied
  (`HalVif_SensorMask` on ch and ch+1 with the stored user mask, only for HDR channels 0 / 4 / 8 / 12); the line-count
  handler for each enabled frame-mode port (§12.4); `ISRCB_Proc(ch + 0x18)` = the SNR `_FrameEndISR` → AE notify
  (mhal-csi-snr.md §3.6); counter [1]++. The event mhal names "frame end" (ISRCB ids 0x18–0x27) is therefore this one: on the
  MIPI path it fires at ~70 % of the active frame, not at its end.
- **bit 2 = line count 1 reached**: mi_vif's line-count callback (delivered as type 3, §9.6); counter [2]++. Line count 1
  is programmed only if a DevSetConfig caller passes one (cfg+0x1c); mi_vif does not, so it stays at its reset value.
- **bit 3 = total pixel count**: counter [3]++ only.
- Then `ITU_TDM_IrqClr(ch, handled bits)`.

mi_vif's **frame-end callback (type 1) is never called in realtime mode**: it runs only from `DrvVifWdmaDoneCb` (frame
mode, §12.4) and from the unused `DrvVif_ISR` (§11.3). mi_vif completes its realtime work on the frame-start count
(mi-capture.md §3.8). On this camera bits 2–3 stay masked, so only frame start and line count 0 fire.

### 11.2 Counters and masks

`gVifIntStats[16]` (0x28 bytes each): +0 VREF_FALLING, +4 LINE_CNT_0, +8 LINE_CNT_1, +0xc TOTAL_PIX_CNT, +0x24 frame
interval ns (names from `DrvVif_IntShow`).
`DrvVif_IntGet(ch, out)` = {interval, VREF, —, LINE_CNT_0, LINE_CNT_1}; the SNR stall monitor reads it and therefore
compares the frame-start and line-count-0 counts. `DrvVif_ResetIntStats` zeroes all.

Default masks (`DrvVif_SetDefaultIntsMask`, values `ulg_itu_tdm_vif_def_mask`, .data): 0xf for all 16 channels. Masking
through `ITU_TDM_IrqMask` also sets `VIFn.64[3:0] = 0xf` (§3.1). `PortSetConfig` of any bus clears and unmasks bits 0–1
only; `PortDisable_<bus>` masks them again.

### 11.3 The older per-channel handler (unused)

`DrvVif_ISR(ch, pts*)` has no caller. It uses the 9-bit second group (`VIFn.5b–.5f`): bit 0 frame start (handler, callback,
ISRCB ch+8, counter +0x80), bit 1 counter +0x84, bit 3 +0x8c, bit 4 +0x90, bit 5 = frame done (line-count handler;
realtime: mi_vif frame-end callback; ISRCB ch+0x18; counter +0x94; mask-change apply), bit 6 line count (callback,
+0x98), bit 8 (+0xa0); clear with `HalVif_IrqClr`. `DrvVif_EnableInterrupt(ch, en)` (no caller on the realtime path)
masks all 8 bits of that group and, for en, clears and unmasks them. `DrvVif_SetVifFrameStartInt` calls the "TBD"
frame-start stubs (§6.1).

## 12. Frame mode [R]

Frame mode follows the I2 design in which an 8051 MCU ran the buffer ring ("SHM ring", "mcu_ch"; I2 `drv_vif.c` has the
same functions with source [S]). In this build the ARM does the MCU's job in the interrupt handlers and the MCU is never
started (§14.2).

### 12.1 WDMA programming

- `DrvVifIsp_WDMAConfig(h, dma, w, h, in, addr)`: `in` 0 / 1 / 2 / 3 / 10 → WDMA format 4 / 5 / 6 / 7 / 10 (other → −1);
  raw-store mode on; `WDMASetPitch(dma, w, h, fmt)`; `WDMADataPrecision(fmt)`; `SenDataPrecision(dma, in)`; 4-pixel MSB
  packing on for formats 4, 5, 7, 10 (bit mask 0x4b0), off for 6; base and base1 = addr; normal priority; request
  threshold and burst by package (`Chip_Get_Package_Type` 2 or 0x30 → 0x11 / 0x3e; width 0x500 with format 10 → 0x14 /
  0x30; else 0x1e / 0x40); high-priority threshold 15; status clear; addr == 0 → software trigger. `in` comes from the
  format word's c-field (§3.2 — so 14-bit raw is stored in 16-bit containers), from YUV input (2) or from YUV→Bayer (3).
- `DrvVifIsp_PreviewRAWStoreMode(h, dma, w, h, type ∈ {0, 1}, ?, addr)` (no caller): raw store on, no 4-pixel packing,
  status clear, WDMA enable + auto, pitch / precision with format 6, base = addr, software trigger.
  `DrvVifIsp_PreviewRAWStoreModeDisable(h, dma)`: WDMA off, raw store off, both bases 0 (= the 0x8000 disabled form),
  software trigger.
- `DrvVifIsp_AFIFO_WDMAReset(h, ch)` (= `MHal_VIF_DevReset(dev, 2)`): the pipe of the channel; WDMA reset of dma pipe (if
  port 0 is enabled) and pipe+8 (if port 1 is enabled); `AffReset(ch >> 2)`. `DevReset(dev, 1)` → WDMA reset only.
- `DrvVifIsp_DMA_FbcConfig(h, dma, w)` → `DmaFbcWidth`; `DrvVifIsp_DMA_CompressEnable` → the empty
  `VifIsp_DmaCompressEnable`.
- The other `DrvVifIsp_*` (MiuSel, FrameIndexMode, WDMAAuto / AutoDisable / Enable, UpdateAddr / FieldAddr / Limit / Pitch,
  Trigger, GetDmaInfo / SubDmaInfo, FIFO push / pop ready, done base, fence id) are one-line wrappers over the §4.2 HAL
  with the pipe → dma id map.

### 12.2 Ring storage

`initialSHMRing(&SHM_Ring)` (probe) points ctx+0xe00 at a static 0x25c0-byte area (.bss+0x325c0) = 16 rings × 0x25c,
ring id = pipe + 8·port. `unInitialSHMRing` zeroes the ring area and the PTS table and destroys the ctx+0xf08 mutex.

Ring header:

| off | content |
|---|---|
| +0x00 | u8 active |
| +0x01 | u8 fill index (next slot the WDMA writes) |
| +0x02 | u8 queue index |
| +0x03 | u8 dequeue index |
| +0x04 | u32 FPS bitmask (§12.6) |
| +0x08 | frame-start count |
| +0x0c | frames completed |
| +0x14 | frames dropped (no buffer at line count) |
| +0x18 | BT656 field repeat count |
| +0x1c / +0x20 | 64-bit frame number |
| +0x24 | started flag |
| +0x28 | u8[8] field-mode retrigger flags |
| +0x30 | eight 0x44-byte slots |
| +0x250 | u8[8] fence-error latch |
| +0x258 | u8 frames since start (throughput-cap warm-up, §7.2) |

`resetSHMRingByChnID(ring)` (each PortSetConfig) zeroes indices and counters, sets the FPS mask to 0xffffffff and every
slot to status 3.

**Slot** (0x44 bytes, "MCU element"): +0x00 address high word, +0x04 address (bit 31 = MIU select, low 31 bits = MIU
address), +0x08 / +0x0c second address (field 2, or the short exposure of an HDR pair), +0x10 u16 pitch (16-byte units),
+0x14 u16 crop x, +0x16 y, +0x18 w, +0x1a h, +0x1c u16 h (copy), +0x1e u16, +0x20 / +0x24 pixel format / precision
(copied through), +0x2c / +0x30 64-bit PTS, +0x34 MiPriv, +0x38 / +0x3c 64-bit frame number, +0x40 u8 status (0 EMPTY,
1 FILLING, 2 READY, 3 FREE / INVALID, 4 DROPPED), +0x41 u8 BT656 field id, +0x42 u8 flags (bit 6 field-combine, bit 7 HDR
pair), +0x43 u8 error (fence mismatch).

**mhal ring element** (what Queue / Dequeue and the frame-end callback exchange; 0x58 bytes; layout from
`convertMhalRgbBuffElmtoMCU` / `convertMCURgbBuffElmtoMhal` and the `DrvVifUt qframe` builder): +0x00 u64 MIU address
(buffer 0), +0x08 u64 buffer 1, +0x10 / +0x18 (I2 HDR addresses; not converted), +0x20 u32 stride 0, +0x24 stride 1,
+0x28 u16 crop x, y, w, h, +0x30 u64 PTS, +0x38 status, +0x3c field type (input 3 = both fields → combine), +0x40 MiPriv,
+0x44 pixel format (mhal code), +0x48 precision, +0x4c frame number low, +0x50 frame number high | ring id << 24, +0x54
error flag. Its ancestor is `MHal_VIF_RingBufElm_t` of the I2 header [S] (I2 ends before +0x44).

### 12.3 Queue, QueryFrames, Dequeue

- **Queue** `MHal_VIF_QueueFrameBuffer(dev, port, elem)` (tail call to `DrvVif_QueueFrameBuffer`): ring = pipe(dev) +
  8·port; mark active; slot = queue index; the slot must be status 3, else "error" and −1; convert: address / stride →
  slot (pitch = stride >> 4); field type 3 → pitch doubled, second address = base + stride, flag 0x40; group HDR mode
  1 / 2 → second address = base + stride·h, flag 0x80; status 0 (EMPTY); queue index = (index + 1) mod 8. Then reclaim:
  from the dequeue index, every DROPPED (4) slot → FREE (3), the dequeue index advances.
- **QueryFrames** `(chn, port, *n)`: the same reclaim, then n = number of consecutive READY slots from the dequeue index.
- **Dequeue** `(chn, port, elem)`: from the dequeue index, skip / reclaim DROPPED slots; the first READY slot is converted
  to `elem`, set FREE, the index advances; **if no READY slot exists in all 8, the slot at the dequeue index is converted
  anyway and freed** — the caller must QueryFrames first (mi_vif does).

### 12.4 Handlers

- **Line count 0** (interrupt bit 1) → `DrvVifFrameLineCountHandler(ring)` for each frame-mode port of the channel's pipe:
  record the BT656 field id in the fill slot; if fill index == queue index → dropped++ and return (no buffer); if a
  field-combine retrigger is pending → `RawStorePipeTrigger`, skip the next frame start, return; slot EMPTY → (first
  frame: wait until the device is not stream-masked) status FILLING, frame number++, program the WDMA:
  `WDMAMiuSel(addr bit 31)`, base = address, field mode: second-field base + retrigger flag, else fence id = slot, pitch,
  WDMA enable, software trigger; HDR pair: the same on dma+1 with the second address. Slot already FILLING → prepare the
  next slot the same way if it is EMPTY and on the same MIU (double buffering). A buffer is thus armed ~0.3 frame before
  the frame it will hold.
- **Frame start** (interrupt bit 0) → `DrvVifFrameStartHandler(ring)`: frame-start count++; meta-data counters; if fill ==
  queue → return; throughput-cap check (§7.2) from the 3rd to the 8th frame; field mode: read the done base, flag top /
  bottom; read the WDMA fence id; the fill slot must be FILLING and so must the next (else return — not finished); PTS =
  `pFuncGetPts()` (mi_vif's `RegisterGetPtsCallback`) or 0; address 0 → status DROPPED; fence id ≠ slot → error latch
  ("FS Check 1'st rd Error"), slot left; else status READY, error flag from the HDR fence check, frames done++, frame
  number copied, fill index advances.
- **WDMA done** (ISRCB id from `GetPipePort2ISRCBID`, context byte = pipe; raised by the ISPSCL block's interrupt,
  mhal-core.md §3.4) → `DrvVifWdmaDoneCb`: if the device is in frame mode, mi_vif's frame-end callback (type 1) with the
  channel's ring element (§8) and frame-end count++.
- `DrvVifFramdDoneHandler` returns 0. `DrvVifFrameHandlerInit` zeroes `dma_info` and `uUpdateAddr`.
  `DrvVif_SetMetaDataCnt(ring, …)` writes the frame number into the channel's ring element.

### 12.5 `MHal_VIF_PortStatQuery(pipe, port, st)` → `DrvVif_PortStatQuery`

st +0x00 = 0, +0x04 u8 fill index, +0x05 queue index, +0x06 dequeue index, +0x08 frame-start count, +0x0c frames done,
+0x10 dropped, +0x14 received width (from `HalVif_GetInfo`; halved for 2-mux, quartered for 4-mux BT656), +0x18 received
height, +0x1c / +0x20 WDMA output size (w.04, w.05), +0x24 / +0x28 sub-DMA size (always 0), +0x2c u8[8] slot statuses.
Its ancestor is I2 `MHal_VIF_ChnStat_t` [S].

### 12.6 Frame-rate decimation mask

`DrvVif_setChnFPSBitMask(ch, port, code, *manual)` writes the ring's +0x04 word: code 0 → 0xffffffff, 1 → 0x55555555,
2 → 0x11111111, 3 → 0x01010101, 4 → 0x77777777 (table `.rodata+0x121f4`), 5 → the user mask byte-swapped
(`convertFPSMaskToMCU`), NULL → 0xffffffff; other codes → "MHal_VIF_FrameRate out of range!!", 0xffffffff. **No ARM-side
handler reads the word** (it was the MCU's input), so frame-mode decimation does not happen in this build [R for the
missing reader, I for the consequence]. The hardware frame mask (`HalVif_FrameMask`, table 2, §3.2) is a separate
mechanism that also works in realtime; it is used only with code 0, except by BT656 DevSetConfig (cfg+0x10).

### 12.7 Unused frame-mode helpers

- `convertMhalBuffElmtoMCU` / `convertMCUBuffElmtoMhal` — the YUV-variant converters (addresses and strides in 32-byte
  units, both strides used) — and `DrvVif_GetFillingRingBufElm` (the "current filling slot" getter): no caller; Queue /
  Dequeue use the RGB variants.
- `BufPtrInit(a, b, release_fn)` / `BufPtrAddRef` / `BufPtrRelease` — a 0x18-byte reference-counted buffer handle {a, b,
  refcount, release fn, release, addref}; release at count 0 calls the function and frees; no caller in mhal.
- `GetTimerCnt64(t, i)` / `GetTimerCnt32` — a wrap-safe 64-bit accumulator per index (ctx+0xe08+i·0x10) and ÷12 [I: 12 MHz
  timer ticks → µs]; no caller.
- `DrvVif_dumpInfo` (no caller) prints the 16 rings as "R/W" index maps and slot states (E/F/R/I).

## 13. YUV→Bayer and YUV downscale — forwarders to ISPSCL [R]

`HalVifIspscl_Yuv2B_H_DN(pipe, en, yc_swap)`, `HalVifIspscl_Yuv2B_H_DN_Status(pipe)`, `YUV2B_Enable`,
`YUV2B_Config(pipe, {in w, in h, crop x, y, w, h, out w, out h, Bayer id, u16 flags})`, `YUV2B_SetRunningMode(pipe,
frame)` and `YUV_RealTimePipe_Settle(w, h, 1)` forward to the ISPSCL driver (`DrvIspScl_*`, instance 0; specified in
isp-2-ispmid-mload-ispscl.md §13), with a log line each. VIF uses them for the pipe's horizontal ½ downscale (§9.4), for
YUV input on BT656 / BT1120 / MIPI port 0 and for YUV→Bayer conversion (§10.4).

## 14. Present, never driven [R]

### 14.1 Pattern generator, strobe, line-interleave, debug selects

All register writers are in §3.1 (`.20–.28` pattern generator, `.02–.05` strobe, `.00[5:4]` line interleave, `.58` /
`.59` debug selects). No driver function drives the pattern generator or the strobe: `DrvVif_PatGenCfg` returns 0 and the
strobe functions have no caller; `LiSt` / `LiEnd` are stubs; the debug-select writers have no caller. M5 names
describe the pattern generator as timing generator, data generator, Bayer order, sizes,
blanking and colour bars. `DrvVif_LineCountCfg(ch, l0, l1)` → `Vif2IspLineCnt0` / `Vif2IspLineCnt1` (no caller);
`DrvVif_LineCnt0` / `DrvVif_LineCnt1` are thin wrappers (LineCnt1 used by DevSetConfig cfg+0x14); `DrvVif_ChannelEnable`
→ `SensorChannelEnable`.

### 14.2 8051 MCU

`DrvVif_MCULoadBin()` (no caller):
1. BDMA (handle 2) copies the firmware from `IPCRamPhys` to the MCU SRAM: reg 2 = 0x0630 (source / destination selector),
   reg 4 / 5 = source address low / high (13 bits), reg 6 / 7 = 0 (destination), reg 8 = 0x8000 (size 32 KiB), reg 9 = 0,
   reg 3 = 0; 20 ms; reg 0 = 1 (trigger); a fixed 500 ms wait; reg 1 = 8 (clear done).
2. MCU8051 (handle 3): reg 0xc = 1 (boot from SRAM), regs 0, 1, 2 = 0, reg 3 = 0xfbff; 500 ms.
3. PM_SLEEP (handle 4): reg 0x29 = 0x9003.

The bank roles come from the I2 source of the same function (`g_BDMA`, `g_MCU8051`, `g_PMSLEEP`) [S]; I2 polls the BDMA
done bit where M6 sleeps. Handles 2, 3, 4 and 15 are all zero in the compiled-in table, so a call would dereference
NULL.

Mailbox (handle 15): `DrvVif_setARMControlStatus(s)` writes reg 0x6e and sleeps 20 ms; `DrvVif_getMCUStatus` reads reg
0x6f and sleeps 1 s; `DrvVif_changeMCUStatus(s)` (s < 3: `MCU_state_tlb` = "STOP", "START", "POLLING" [S]) writes and
polls up to 11 times; `stopMCU` / `startMCU` / `pollingMCU` wrap it under `mculock` (polling refused from STOP);
`DrvVif_pauseMCU` is a 4-byte thunk. None of them has a caller. The DT property `mcu` is read and not used.

## 15. Debug surface [R]

- **misc `vif`** (`/dev/vif`; `vif_open`, `vif_fread`, `vif_release` return 0; registration: mhal-core.md §4): **ioctl**
  type `'V'` (else −25), only `0x80605642` (`_IOR('V', 0x42, 96)`): copies 96 bytes in and calls
  `DrvVif_SensorReset(buf+0, buf+4)` (the sensor reset pin, §6.5); other 'V' numbers print "Invalid ioctl command" (log
  bit 3) and return 0. **write**: text to `DrvVifUt` (below).
- **sysfs** on `vif0`: `vif_ints` — for every active channel "== VIF CH-%d ==" + `DrvVif_IntShow`: Interval(ns),
  VREF_FALLING, LINE_CNT_0, LINE_CNT_1, TOTAL_PIX_CNT; `vif_info` — `DrvVif_ShowInfo` "INTF_MODE = %d" (reads the interface
  word of group `ch & 1`, an indexing slip) plus `HalVif_ShowInfo`: CH_EN, SRC, INPUT_FMT, PIX_FMT bits, CROP_EN, crop X /
  Y range, PIXEL_CNT, LINE_CNT, TOTAL_PIXEL_CNT, TOTAL_LINE_CNT.
- `HalVif_GetInfo(ch, info)` (also used by the driver): info +0 u8 channel enable (`.00[15]`), +1 u8 source
  (`.00[12:8]`), +2 u8 RGB-in (`.06[12]`), +3 u8 precision field c (format word [5:4]), +5 u8 source again, +8 width =
  x1 + 1 − x0, +0xc height = y1 + 1 − y0 (crop table), +0x10 `.50[12:0]`, +0x14 `.51[12:0]` (received pixel / line
  counts), +0x18 x0, +0x1c y0. `DrvVif_GetSensorInputFmt(ch)` = info+2; `DrvVif_GetWdmaInputFmt(pipe)` (pipe ≥ 8 → −8) =
  info+2 of the pipe's channel. `HalVif_GetDataPrecInfo(ch, info)` fills info+3 only. `DrvVif_GetInfo` is a 4-byte thunk.
- Log mask `VIFLogGetEn` = `.data+0x1c844`, initial **3** (bit 0 trace, bit 1 errors; bit 3 verbose, off); `VIFLogSetEn` has
  no caller.

**`DrvVifUt` text commands** (written to `/dev/vif`):

| command | effect |
|---|---|
| `groupcfg g intf mux hdr edge pinpol` | group config (§9.2) |
| `mclk_ext g` | `MHal_VIF_SetMclk` |
| `groupen g en` | group enable / disable |
| `devcfg dev pixfmt bits x y w h` | device config |
| `deven dev en` | device enable / disable |
| `pipecfg dev pipe en hdn` | pipe config |
| `portcfg pipe port en fmt fbc x y w h mode outW outH` | port config (fmt 1–4 → 0x2a–0x2d; port 0 leaves mode / out size 0) |
| `qframe dev fmt bits w h` | allocates w·h·2 of DMA memory "WDMA%d" and queues it |
| `dqframe dev file` | query + dequeue + write the frame to a file through `VifFileOpen` / `VifFileWrite` / `VifFileClose` (= `filp_open` / `vfs_write` with the address limit lifted) |
| `qframe_cus_buf dev fmt bits w h phys` / `dqframe_cus_buf dev` | the same with a caller-given physical buffer |
| `alloc_buf id size` / `free_buf id` | 8 ids, via `DrvVifGetSysMemAlloc` = `_DrvVifSysMalloc` (CamOs direct memory + 16-byte header) / `_DrvVifSysMfree` |
| `qframe_buf dev fmt bits w h id` | queue an allocated buffer |
| `mask dev m` | input mask |
| `autotest ch file fps×1000` | checks that the frame-start count moves within 1 s, that the interval is within ±300 µs of 1/fps and that the crop fits the received size; writes PASS / FAIL lines to the file |
| `pdbg pipe en` | per-pipe ISR trace bit (`VIFPipeDbgSetEn`) |

`_DrvVifSysMemMap`, `_DrvVifSysMemUnMap` and `_DrvVifSysFlashCache` are empty.

## 16. Remaining small driver functions [R]

- `_DrvVif_BppBayerIDtoFormat(id)`: id 0..3 → table `.rodata+0x120b0` = 0x16, 0x1b, 0x28, 0x21; else 0x37; no caller.
- `DrvVif_ChnID2PhyChn`, `DrvVif_Dev2GroupID`, `DrvVif_Dev2ChnID`, `DrvVif_Group2DevID`, `DrvVif_GetDevIDBySNRPadID`,
  `DrvVif_SNRPad2GroupID` (search of the dev → pad map, 0xff if not found) — the exported wrappers over §2.4 that the ISP
  block uses.
- `DrvVif_Reset` (4-byte thunk), `DrvVif_CLK` (empty), `DrvVifFreeRtHdrBuf` (empty), `DrvVif_CtxDeInit` (empty).
