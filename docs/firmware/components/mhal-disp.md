# mhal-disp — the DISP (display engine) block of mhal.ko
<!-- ev:mhal-disp -->

DISP is the display-engine block of `mhal.ko`: the driver of the SoC's two display devices (device 0 and device 1). Each
device has a top block (DISP_TOP: resets, interrupts, CMDQ handshake, DAC frame-lock PLL), an OP2 block (timing generator,
frame and window colour, two colour engines), a MOP (the video layers that fetch frames from DRAM: one MOPG with 16
windows, one MOPS with one window, one rotation engine) and a gamma block; one MACE picture engine serves device 0 [I:
one MACE bank, 1152; device 1's sharpness goes through bank 116F, §12.5].
Around them sit the output muxes and clocks of the interfaces (HDMI, VGA DAC, CVBS TV encoder, LCD / BT656 / BT601 /
BT1120 / SRGB / MCU panels, MIPI-DSI), one write-back DMA that captures a device into DRAM or IMI, the PQ ("picture
quality") bin loader with its SRAM writers, and the compiled-in per-timing pixel-clock synthesizer, DAC and HDMI-TX analog
tables. Its callers are `mi_disp.ko` (50 imports) and `fbdev.ko` (5 imports) through 73 exported `MHAL_DISP_*` functions.
On this camera device 0 drives HDMI only, with the timings 1080P60 or 3840x2160_30; VGA, CVBS, LCD, MCU, DSI, the DMA, the
rotation and PQ are present and unused. This document covers the source structure and revision of the block, the
exported API and its structures, the internal objects, the query layer, the CPU / CMDQ register path, device life cycle,
interfaces and output timing, interrupts, clocks, module glue, MOP, the picture path (colour engines, MACE, gamma, DAC
trim), PQ, the write-back DMA, and what is not known.

Shared facts owned elsewhere: mhal's block map and register banks per block, the consumers' import lists, the
`camdriver_init` order (step 16 `_DrvDispModuleInit`), the device-node quirk shared by GE / JPD / HDMITX / PNL / DISP /
MIPITX, the DT properties mhal reads per node and the IMI heap — `mhal-core.md` §2.1, §2.4, §2.5, §3.2, §3.5, §4.2, §7;
RIU addressing (bank → physical address), the clock tree with the parents of the four DISP clocks, the interrupt
controller, the DTB as a whole and the MIU arbiter — `kernel-platform.md` §3.1, §4.1, §5.1, §12, §14; the CMDQ interface
table, engine ids and trigger-bus events — `mhal-scl-jpe-cmdq.md` §2.3, §2.1, §2.8; mi_sys's CMDQ objects — `mi_sys-1-engine.md` §7.1; the
boot-time `riu_w` list whose rows 6–7 are the MOP thresholds of §11.4 — `rootfs-customer.md` §5.2, §5.3; the `PQ.bin` file
format, U-Boot's PQ path and this camera's `/misc/PQ.bin` — `uboot.md` §6; U-Boot's copy of the DISP HAL and its
`bootlogo` calls — `uboot.md` §5, §7; the 0x170-byte panel record and how mi_disp fills the panel configs —
`config_tool.md` §5.2, §5.5; the caller side (MI timing enum ↔ mhal timing ids, mi_disp's ioctls, IRQ requests, PQ table
call, rotation path) — `mi_disp-mi_hdmi.md` §4.6.4, §4.7, §4.5, §4.3, §4.11; the HDMI-TX block (DTOP timing tables, ATOP field names, drive-current
interface) — `mhal-hdmitx.md` §4.3, §4.4, §4.2, §4.6; fbdev's calls — `rgn-gop.md` §5.4.

Tags: **[R]** read from the binary (Ghidra decompile, Thumb disassembly, `.data` / `.rodata`, symbol table, relocations),
the DT or a cited file; **[S]** stated by a source (the reference source text, a
member name taken from a source header, a debug string used as a name); **[I]** inference. A heading tagged [R] means
every statement under it is [R] unless marked otherwise.

Binary: `/config/modules/4.9.227/mhal.ko`; the DISP block is objdump `.text` 0xff80–0x2a244 (106.8 KB, 1082 symbols,
1055 decompiled functions — the other 27 symbols are 4-byte thunks Ghidra folds into their callers [R count; I]).
"disassembly 0x…" are `.text` offsets.

Register notation: `bank:reg` = one 16-bit RIU register (bank and register index in hex); `[h:l]` a bit field; `regL` /
`regH` the low / high byte of a register (byte tables). A byte address `A` in the code is `bank<<8 | 2·reg`. "Direct" =
an immediate RIU write; "buffered" = a write through the device's CMDQ context (§5), which goes to the CPU or the CMDQ
according to the access mode. A "unit" of delay is the argument of the HAL delay command.

Address forms: objdump `.text` offset + 0x40000028 = the 0x400xxxxx address used here (image rebased to 0x40000000).

## 1. The block [R]

### 1.1 Source files and address ranges
The binary keeps the source file order. Link order: `drv_disp_ctx`, `drv_disp_if`, `drv_disp_irq`, the linux module /
sysfs / OS glue, `mhal_disp`, then the HAL files. Inside the HAL the functions of `hal_disp_mop.c` / `hal_disp_mop_if.c`
and of `hal_disp_dma.c` / `hal_disp_dma_if.c` interleave, i.e. this build has the static helpers of each pair in one
translation unit [I from the interleaving].

| range (objdump) | source file | SAME / DIFF / NOSRC |
|---|---|---|
| 00ff80–0109a4 | `drv/disp/src/drv_disp_ctx.c` (contexts: device / video layer / input port / DMA) | 25 / 2 / 0 |
| 010a50–013a7e | `drv/disp/src/drv_disp_if.c` (MHAL → HAL translation, query dispatcher) | 76 / 29 / 2 |
| 013aac–0143a8 | `drv/disp/src/drv_disp_irq.c` | 10 / 8 / 0 |
| 0143ac–014cf0 | `drv/disp/src/linux/disp_module.c`, `disp_sysfs.c` (sysfs attributes, renamed) | 3 / 0 / 18 |
| 014d24–015684 | `drv/disp/src/linux/drv_disp_module.c`, `drv_disp_os.c` | 21 / 9 / 0 |
| 0156ac–0165ec | `drv/midisp/src/mhal_disp.c` — the exported API | 73 / 1 / 2 |
| 016614–018984 | small HAL helpers (op2 DAC / VGA, colour, mop, utility incl. the CMDQ / RIU write layer, DAC trim) | 57 / 32 / 15 |
| 018988–019dd0 | `hal/mercury6/src/disp_hpq/*` — PQ bin loader | 44 / 22 / 17 |
| 019ddc–01a908 | `hal_disp_clk.c` | 32 / 23 / 1 |
| 01a914–01b0a0 | `hal_disp_dma_if`, `hal_disp_picture_if.c`, `hal_disp.c` (CMDQ context lookup) | 4 / 7 / 6 |
| 01b0f0–01c2dc | `hal_disp_op2.c` (timing generator, frame colour, top-level resets) | 58 / 12 / 9 |
| 01c318–01da00 | `hal_disp_mace.c` (MACE: peaking, DLC / DCR, gamma, …) | 95 / 10 / 0 |
| 01db3c–01e630 | `hal_disp_color.c` (CSC, colour adjust) | 2 / 16 / 1 |
| 01e6e4–021b74 | `hal_disp_mop.c` + `hal_disp_mop_if.c` (MOP layers / ports / rotation) | 68 / 14 / 9 |
| 021c1c–0230a0 | `hal_disp_picture_if.c` tail, `hal_disp_dma.c` + `hal_disp_dma_if.c` (write-back DMA) | 54 / 7 / 5 |
| 0230f0–0282b4 | `hal/mercury6/src/hal_disp_if.c` (HAL query handlers: device, layer, port, DMA, PQ, clocks) | 62 / 53 / 15 |
| 0282e8–02a224 | `hal_disp_irq.c` | 9 / 13 / 4 |

### 1.2 Comparison with the reference source
Every DISP function of the decompile was compared with the same-named function of the source by four features: callees
that are themselves DISP functions (a leading `_` ignored: the binary renamed a few statics `_X` → `__X`), RIU registers
referenced (source macros evaluated through every header; binary literal-pool constants including `[lit] ± off`
arithmetic), format strings (normalised: KERN level digit, ANSI colours and `%` conversions dropped), and the value / mask
pairs of every `HalDispUtility*W*BYTE*` register write with constant arguments. Source callees the binary never calls
anywhere (inlined accessors such as `HalDispUtilityR2BYTEDirect`) are not counted as differences. "SAME" = all four agree;
it does not prove identical code — struct offsets, local arithmetic and non-constant write values are not compared [R
method; I on what it can miss].

| class | functions | meaning |
|---|---|---|
| SAME | 693 | feature-equal with the source function of the same name |
| DIFF | 258 | same name, a feature differs |
| NOSRC | 104 | no function of that name in the source (new or renamed in this build) |
| SRCONLY | 313 | source functions absent from the binary: whole files not linked into mhal (`drv_disp_vip*.c` — an SCL VIP copy, `disp_sysfs.c`, `drv_disp_debug.c`, most of `hal_disp_hpq_main.c` / `hal_disp_hpq_if.c` = the TV-PQ API, never called) or features this build lacks (capability queries, CVBS param get, VGA HPD ISR) |

Beyond the feature compare, these were read side by side with the source: every DIFF function whose callees, registers
and constant write values equal the source (119 functions, "strings differ"); every function on the CVBS / VGA-DAC / LCD /
MCU / DSI paths (names containing Cvbs, Vga, Dac, Lcd, Mcu, Dsi, Mipi, Tve, Idac, Fpll, Synth, Aff, Hpd: 86 SAME — 70 HAL
bodies read, 16 MHAL / `DrvDispIf` wrappers checked for query id and size — 36 DIFF, 24 NOSRC). Because Ghidra drops the
mask argument of many `HalDispUtilityW2BYTEMSK` calls, the mask and constant value of every write call were taken from the
disassembly, over every SAME / DIFF function of the block. Every behavioural difference found is given in this document at
the place it belongs; all other functions are equal to the source in logic, registers, masks and constant values. The
SAME functions off those paths (HDMI timing, MOP, colour, PQ) stand on the feature compare plus the mask check; their
computed (non-constant) values were not re-read.

### 1.3 Revision
Neither side is a superset [R for the listed facts; I for the conclusion]. The binary is **older than the source in the
common layers** (`drv/midisp`, `drv/disp`: the public MHAL header of this build has an older timing enum, a 16-byte
`MHAL_DISP_DeviceConfig_t`, a 1-byte CVBS param, no capability / priority / compress / buffer-fire queries in the exported
set; its internal objects are the older ssd_lcd generation, §3) and **newer than the source in the mercury6 HAL**
(per-device CMDQ contexts, the DAC FPLL lock sequence and frame-lock monitor, the InterCfg reset controls the source stubs
out, MOP DMA FIFO thresholds, LCD / MCU IRQ, a 27-entry VGA / HDMI timing table with an HDMI-TX analog table, PQ load on
every timing change, live PQ SRAM writers). The OpenIPC tree evidently carries a newer common layer (shared with
infinity7) on top of an older M6 HAL snapshot [I]. U-Boot's DISP HAL is the OpenIPC mercury6 HAL revision with mhal's
coloured debug strings and an older 0x6c-byte panel struct — older than this mhal HAL (`uboot.md` §7).

## 2. The exported API [R]

### 2.1 Exports
80 exports: 73 `MHAL_DISP_*` and 7 OS helpers (`DrvDispOsCreateTask`, `DrvDispOsDestroyTask`, `DrvDispOsGetSystemTime`,
`DrvDispOsGetSystemTimeStamp`, `DrvDispOsMemAlloc`, `DrvDispOsMemRelease`, `DrvDispOsMsSleep` — thin CamOs wrappers that
no module imports). Three more `MHAL_DISP_*` exist as local symbols only (`MHAL_DISP_VideoLayerBufferFire`,
`MHAL_DISP_VideoLayerSetCompress`, `MHAL_DISP_VideoLayerSetPriority`) and cannot be called from other modules. Every
`MHAL_DISP_*` returns `u8` 1 = OK / 0 = fail and is a two-line wrapper around a `DrvDispIf*` function, which builds a
query (§4); in the `mhal_disp.c` region 73 functions are feature-equal with the source, `MHAL_DISP_InputPortFlip` differs
in debug text only, and the two `GetHwCount` of layer and port are new; below them `DrvDispIfDeviceCreateInstance` /
`DrvDispIfDeviceDestroyInstance` differ from the source in prints only. Instance handles are pointers to the 12-byte
context of §3.1; the functions that take a device id take the MI device id (0 / 1) directly. Users are the importing
modules (`mhal-core.md` §2.4); "—" = imported by nobody. The table drops the `MHAL_DISP_` prefix (`InitMmapConfig` =
`MHAL_DISP_InitMmapConfig`, `DeviceSetConfig` = `MHAL_DISP_DeviceSetConfig`, …).

| export | user | arguments (binary) | does | query id(s) |
|---|---|---|---|---|
| `InitPanelConfig` | mi_disp | `MHAL_DISP_PanelConfig_t[ ]`, `u8 n` (≤ 0x27) | stores the timing table (§7.1) | — |
| `InitMmapConfig` | mi_disp | type, `MmapInfo*` | **empty: returns 1** — the `E_MMAP_ID_XC_MAIN_FB` / `XC_MLOAD` areas mi_disp passes are ignored | — |
| `DeviceCreateInstance` | mi_disp | `{alloc, free}`, device id, `void **ctx` | allocate the device context, clocks on (§10.1), DEVICE_INIT (§6.1), make it the current device context, create the internal (time-zone) IRQ (§9.4) | 0x00 |
| `DeviceDestroyInstance` | mi_disp | ctx | destroy the internal IRQ, DEVICE_DEINIT, clocks off, free | 0x01 |
| `DeviceEnable` | mi_disp | ctx, `u8` | DEVICE_ENABLE (§6.2) | 0x02 |
| `DeviceSetBackGroundColor` | mi_disp | ctx, `u32` 0x00BBGGRR | OP2 frame colour (§6.2) | 0x05 |
| `DeviceAddOutInterface` | mi_disp | ctx, `u32` interface mask | DEVICE_INTERFACE (§7.2) | 0x06 |
| `DeviceSetOutputTiming` | mi_disp | ctx, `u32` interface, `DeviceTimingInfo*` | timing translation (§7.1), DEVICE_OUTPUTTIMING (§7.3) | 0x07 |
| `DeviceSetVgaParam` / `DeviceGetVgaParam` | mi_disp / — | ctx, `VgaParam*` | CSC, gain, sharpness of the VGA path; Set also applies the DAC trim (§12.6); Get = new query | 0x08 / 0x3b |
| `DeviceSetColorTempeture` | mi_disp | ctx, `ColorTempeture*` | colour offsets / gains, stored at device container +0xa8 | 0x09 |
| `DeviceSetCvbsParam` | mi_disp | ctx, `CvbsParam*` (**1 byte** `bEnable` in this build) | GetInfo is an empty `return 0`: nothing is stored (§3.3) | 0x0a |
| `DeviceSetHdmiParam` / `DeviceGetHdmiParam` | mi_disp / — | ctx, `HdmiParam*` | CSC + sharpness of the HDMI path | 0x0b / 0x3c |
| `DeviceSetLcdParam` / `DeviceGetLcdParam` | mi_disp / — | ctx, `LcdParam*` | CSC + sharpness of the LCD path (§4.2) | 0x0c / 0x3d |
| `DeviceSetGammaParam` | mi_disp | ctx, `GammaParam*` | gamma tables (MACE gamma, §12.3) | 0x0d |
| `DeviceSetTimeZoneConfig` / `DeviceGetTimeZone` | — | ctx, `TimeZoneConfig*` / `TimeZone*` | invalid-area config: GetInfo returns 2 NOT_SUPPORT; which zone (sync / back porch / active / front porch) the scan is in (§9.4) | 0x10 / 0x0e |
| `DeviceGetDisplayInfo` | fbdev | ctx, `DisplayInfo*` | Htotal / Vtotal / DE start / size from the stored timing config, not read from hardware (§4.3) | 0x0f |
| `DeviceAttach` / `DeviceDetach` | — | src ctx, dst ctx | device 0 → device 1 through the DISP DMA: GetInfo stores the source container in the destination (+0xd0); the Set half is empty | 0x03 / 0x04 |
| `DeviceGetInstance` | fbdev | device id, `void **` | current device context of that id | — |
| `DeviceSetPqConfig` | mi_disp | ctx, `PqConfig*` | PQ bin / source id / picture (§13) | 0x28 |
| `DeviceGetHwCount` | fbdev | `u32*` | 2 | 0x36 |
| `DeviceSetConfig` / `DeviceGetConfig` | fbdev / — | **device id**, `DeviceConfig*` | internal config (§6.3): boot-logo flag, colour id, test pattern, block resets | 0x39 / 0x3a |
| `VideoLayerCreateInstance` / `VideoLayerDestoryInstance` | mi_disp | `{alloc, free}`, layer id, `void **` / ctx | allocate the context, VIDEOLAYER_INIT (empty HAL handlers) | 0x11 |
| `VideoLayerEnable` | mi_disp | ctx, `u8` | stored only (layer container +0x08) — both HAL handlers are empty, no register changes | 0x12 |
| `VideoLayerBind` / `VideoLayerUnBind` | mi_disp | layer ctx, device ctx | layer ↔ device: layers 0 / 1 → MOP of device 0, 2 / 3 → device 1; layer 2 / 3 cannot bind to device 0 | 0x13 / 0x14 |
| `VideoLayerSetAttr` | mi_disp | ctx, `VideoLayerAttr*` | layer window, size, pixel format → `HalDispMopIfSetInfoStretchWinSize` + `HalDispMopIfSetInfoHextSize` (MOP stretch window, horizontal extension) | 0x15 |
| `VideoLayerCheckBufferFired` | — | ctx | double-buffer latch done? | 0x19 |
| `VideoLayerGetHwCount` | — | `u32*` | 4 | 0x36 |
| `InputPortCreateInstance` / `InputPortDestroyInstance` | mi_disp | `{alloc, free}`, layer ctx, port id, `void **` / ctx | allocate the port context bound to the layer, INPUTPORT_INIT (MOP init, §11.2) | 0x1a |
| `InputPortEnable` | mi_disp | ctx, `u8` | window on / off | 0x1b |
| `InputPortSetAttr` | mi_disp | ctx, `InputPortAttr*` | display window + source size | 0x1c |
| `InputPortShow` / `InputPortHide` | — | ctx | window on / off without changing the enable state | 0x1d / 0x1e |
| `InputPortAttrBegin` / `InputPortAttrEnd` | — | layer ctx | batch attribute updates | 0x1f / 0x20 |
| `InputPortFlip` | mi_disp | ctx, `VideoFrameData*` | new frame-buffer addresses / pitch (§11.3) | 0x21 |
| `InputPortRotate` | mi_disp | ctx, `RotateConfig*` | MOP rotation (MOPG: one window; sets the DMA FIFO thresholds, §11.2) | 0x22 |
| `InputPortSetCropAttr` | mi_disp | ctx, `VidWinRect*` | source crop | 0x23 |
| `InputPortSetRingBuffAttr` | — | ctx, `RingBufferAttr*` | ring input from SCL (handshake) | 0x24 |
| `InputPortSetImiAddr` | mi_disp | ctx, `u32` | IMI start address of the rotation buffer | 0x25 |
| `InputPortGetHwCount` | — | layer id, `u32*` | 16 for layers 0 / 2, 1 for layers 1 / 3 | 0x36 |
| `DmaCreateInstance` / `DmaDestoryInstance` / `DmaBind` / `DmaUnBind` / `DmaSetAttr` / `DmaSetBufferAttr` | mi_disp | | write-back DMA (bank 1158, §14) | 0x30–0x35 |
| `GetDevIrq` / `EnableDevIrq` / `GetDevIrqFlag` / `ClearDevInterrupt` | mi_disp | ctx, … | device vsync IRQ (§9) | — |
| `GetDevIrqById` | fbdev | device id, `u32*` | IRQ number by device id | — |
| `GetLcdIrq` / `EnableLcdIrq` / `GetLcdIrqFlag` / `ClearLcdInterrupt` | mi_disp | ctx, … | LCD / MCU IRQ (slot 5) | — |
| `GetDmaIrq` / `EnableDmaIrq` / `GetDmaIrqFlag` / `ClearDmaInterrupt` | mi_disp | DMA ctx, … | DMA active on / off IRQ (slots 2 / 3) | — |
| `SetRegAccessConfig` | mi_disp | ctx, `RegAccessConfig*` | CPU vs CMDQ register path + CMDQ interface (§5.4) | 0x2d |
| `SetRegFlipConfig` | mi_disp | ctx, `RegFlipConfig*` | kick the batched register writes (§5.4) | 0x2e |
| `SetRegWaitDoneConfig` | mi_disp | ctx, `RegWaitDoneConfig*` | append "wait for vsync / frame done" to the CMDQ (§5.4) | 0x2f |
| `SetCmdqInterface` | — | device id, `CmdqInfConfig*` | store a CMDQ interface for the device (GetInfo: no check) | 0x38 |
| `DbgLevel` | — | `u32` | debug mask | — |

The CMDQ interface (`MHAL_CMDQ_CmdqInterface_t`) comes from mi_sys through the `pCmdqInf` field of the three `SetReg*`
configs.

### 2.2 Structures as this build uses them [R offsets from the copy / translate code; names S from the source header where they agree]
Agreeing with the binary (offsets verified in
`_DrvDispIfTrans*`): `MHAL_DISP_SyncInfo_t` (0x30), `DeviceTimingInfo_t` {eTimeType, pstSyncInfo}, `VidWinRect_t`
(4 × u16), `VideoLayerAttr_t` (0x14: win, u32 width, u32 height, ePixFormat), `InputPortAttr_t` (0xc: win, u16 source
width, source height), `VideoFrameData_t` (0x38: ePixelFormat, eCompressMode, eTileMode, u64 aPhyAddr[3] at +0x10, u32
au32Stride[3] at +0x28), `RingBufferAttr_t`, `Csc_t`, `VgaParam_t`, `HdmiParam_t`, `LcdParam_t`, `GammaParam_t`,
`ColorTempeture_t`, `DisplayInfo_t` (0xe), `RegAccessConfig_t` {enType, pCmdqInf}, `RegFlipConfig_t` {u8 bEnable,
pCmdqInf}, `RegWaitDoneConfig_t` {u32 u32WaitType, pCmdqInf}, `PqConfig_t` {u32 flags, u32 size, void *data},
`DmaAttrConfig_t` (0x24), `PanelConfig_t` (0x74: the packed `MHAL_DISP_PanelType_t`, eTiming @0x68, u32OutputDev @0x6c,
bValid @0x70), `RotateConfig_t`, `MmapInfo_t`.

| struct / enum | this build | source header |
|---|---|---|
| `MHAL_DISP_DeviceTiming_e` | **0 PAL, 1 NTSC, 2 960H_PAL, 3 960H_NTSC, 4 1080P24, 5 1080P25, 6 1080P30, 7 720P50, 8 720P60, 9 1080I50, 0xa 1080I60, 0xb 1080P50, 0xc 1080P60, 0xd 576P50, 0xe 480P60, 0xf 640x480_60, 0x10 800x600_60, 0x11 1024x768_60, 0x12 1280x1024_60, 0x13 1366x768_60, 0x14 1440x900_60, 0x15 1280x800_60, 0x16 1680x1050_60, 0x17 1920x2160_30, 0x18 1600x1200_60, 0x19 1920x1200_60, 0x1a 2560x1440_30, 0x1b 2560x1600_60, 0x1c 3840x2160_30, 0x1d 3840x2160_60, 0x1e 1080P59.94, 0x1f 1080P29.97, 0x20 720P59.94, 0x21 720P29.97, 0x22 2560x1440_50, 0x23 2560x1440_60, 0x24 3840x2160_25, 0x25 3840x2160_29.97, 0x26 USER, 0x27 MAX** — identical to the HAL enum (`__DrvDispIfTransDevTimingTypeToHal` passes 0…0x26 through, anything else → 0x27) | a reordered, longer enum (720P24 … 2560x1600) — not compatible |
| `MHAL_DISP_DeviceConfig_t` | 0x10 bytes: +0 `u32 eType` (bit 0 BOOTLOGO, 1 COLORID, 2 GOPBLENDID, 3 DISPPAT, 4 RSTMOP, 5 RSTDISP, 6 RSTDAC, 7 RSTHDMITX, 8 RSTLCD, 9 RSTDACAFF, 10 RSTDACSYN, 11 RSTFPLL), then u8 +4 bBootlogoEn, +5 u8ColorId, +6 u8GopBlendId, +7 bDispPat, +8 bRstMop, +9 bRstDisp, +0xa bRstDac, +0xb bRstHdmitx, +0xc bRstLcd, +0xd bRstDacAff, +0xe bRstDacSyn, +0xf bRstFpll | 0x18 bytes; adds u8PatMd, bCscEn, u8CscMd, bCtx, u8MopYThrd, u8MopCThrd |
| `MHAL_DISP_CvbsParam_t` | 1 byte `bEnable` (only byte 0 is copied) | 0x1c (bEnable + Csc + sharpness) |
| `MHAL_DISP_DmaBufferAttrConfig_t` | copies bEn, aPhyAddr[3] (+8), u32Stride[3] (+0x20), u16 RingBuffHeight (+0x2c), RingStartLine (+0x2e), FrameIdx (+0x30); no CRC field | + `enCrcType` at +0x34 |
| capability queries (`DeviceGetCapabilityConfig`, `VideoLayerGetCapabilityConfig`, `DeviceGetIntefaceCapabilityConfig`, `DmaGetCapabilityConfig`, `DmaGetHwCount`, `DeviceGetCvbsParam`) | absent | present |
| `VideoLayerGetHwCount`, `InputPortGetHwCount` | present | absent |

Pixel formats (`MHalPixelFormat_e` of `mhal_common.h` → HAL, `__DrvDispIfTransPixelFormatToHal`): 0–6 (YUYV, ARGB8888,
ABGR8888, BGRA8888, RGB565, ARGB1555, ARGB4444) unchanged, 10 SP422 → 7, **11 SP420 / NV12 → 8**, 13 MST420 → 9, others →
10 (unsupported). Compress mode 0–3 pass, else 4; tile mode 1–4 pass, else 0; CSC matrix 0–6 pass, else 7. The other 21
`_DrvDispIfTrans*` copy the same fields as the source; `__DrvDispIfTrans*` are the source's `_DrvDispIfTrans*` helpers,
renamed. The `DeviceConfig` translation is `_DrvDispIfTransDeviceConfigToHal` / `…ToMHal` (16-byte config),
`_DrvDispIfTransDeviceCvbsParamToHal` copies one byte, `_DrvDispIfTransDmaBufferAttrToHal` has no CRC type.

## 3. Contexts and internal objects [R]

Object sizes come from the allocators (`DrvDispOsMemAlloc(n)` + `memset(n)`) and the symbol table; every offset the DISP
code uses on each object was collected and read in the using functions; member names, and the members no code touches,
are [S] from the source headers (DWARF layouts of an ARM test compile of both trees). "unused" = no access anywhere in the
DISP block. The common-layer objects of this build are the **older** (ssd_lcd) generation — 12-byte context, old DMA
container, old device container plus two additions; the port and layer containers are the same in both trees.

### 3.1 Context handle and allocation request [R `_DrvDispCtxAlloc*`]
`DrvDispCtxConfig_t` (0xc, allocated per instance): +0 `enCtxType` (0 device, 1 video layer, 2 input port, 3 DMA),
+4 `u32Idx` (index into the container's arrays), +8 `pstCtxContain` (always `&gstDispCtxContainTbl`). The newer source's
16-byte context (`u32ContainIdx`, `u32CtxIdx`) does not apply. `DrvDispCtxAllocConfig_t` (0x14, the caller's): +0
`enType`, +4 `u32Id`, +8 `pstBindCtx`, +0xc `stMemAllcCfg {alloc, free}` (copied into the container +0xd0 / +0xd4 at every
device allocation). Port allocation takes the first free slot (the source: the last).

### 3.2 Global container `gstDispCtxContainTbl` (.bss, 0xdc)
One global container (`HAL_DISP_CTX_MAX_INST` = 1), set up by `DrvDispCtxInit`. Differences from the source: no "current
context" arrays per object type inside the container and no `DrvDispIrqInit` call in `DrvDispCtxInit`.

| off | member | notes |
|---|---|---|
| +0x00 | `u8 bDevContainUsed[2]` | |
| +0x02 | `u8 bVidLayerContainUsed[4]` | |
| +0x06 | `u8 bInputPortContainUsed[34]` | 2 × (16 MOPG + 1 MOPS) |
| +0x28 | `u8 bDmaContainUsed[1]` | `_DrvDispCtxDeInitDmaContain` clears **+6** (port 0's flag) instead of +0x28 — vendor bug, effect only at module deinit |
| +0x2c | `DrvDispCtxDeviceContain_t *pstDevContain[2]` | both allocated once at `DrvDispCtxInit` (0xd4 each) |
| +0x34 | `DrvDispCtxVideoLayerContain_t *pstVidLayerContain[4]` | 0x6c each |
| +0x44 | `DrvDispCtxInputPortContain_t *pstInputPortContain[34]` | 0x78 each |
| +0xcc | `DrvDispCtxDmaContain_t *pstDmaContain[1]` | 0x68 |
| +0xd0 | `stMemAllcCfg {alloc, free}` | mi_disp's callbacks |
| +0xd8 | `HalDispHwContain_t *pstHalHwContain` | 0x18 (§3.7) |

Followed in .bss by `gbDispCtxInit` (+0xdc, the init flag), `pgstCurDispDevCtx[2]`, `pgstCurVidLayerCtx[4]`,
`pgstCurInputPortCtx[34]`, `pgstCurDmaCtx` — the "current context" pointers, kept by `DrvDispCtxSetCurCtx`.

### 3.3 Device container `DrvDispCtxDeviceContain_t` (0xd4) [R offsets; names S]
The ssd_lcd layout with a 4-byte MCU-counter pair inserted after `u32Interface`, four layer pointers and no DMA pointer.

| off | size | member | used by |
|---|---|---|---|
| +0x00 | 4 | `u32DevId` | everywhere (MI device id 0 / 1) |
| +0x04 | 1 | `bEnable` | DEVICE_ENABLE GetInfo |
| +0x08 | 4 | `u32BgColor` | BACKGROUND_COLOR GetInfo |
| +0x0c | 4 | `u32BindVideoLayerNum` | unused |
| +0x10 | 4 | `u32Interface` (mask, §7.2) | interface / timing / IRQ / PQ code |
| +0x14 | 2 | `u16McuCnt` | unused |
| +0x16 | 2 | `u16McuFenceId` | REG_WAITDONE on MCU panels: copy of the MCU frame counter polled at 1171:1F |
| +0x18 | 4 | `eTimeType` (HAL timing id) | OUTPUTTIMING |
| +0x1c | 0x2c | `HalDispDeviceTimingConfig_t`: u16 HsyncWidth +0x1c, HsyncBackPorch +0x1e, Hstart +0x20, Hactive +0x22, Htotal +0x24, VsyncWidth +0x26, VsyncBackPorch +0x28, Vstart +0x2a, Vactive +0x2c, Vtotal +0x2e, Fps +0x30, SscStep +0x32, SscSpan +0x34; u32 VSyncPeriod +0x38, VBackPorchPeriod +0x3c, VActivePeriod +0x40, VFrontPorchPeriod +0x44 (µs) | TGEN, DisplayInfo, time zone |
| +0x48 | 4 | `stCvbsParam` (u8 bEnable) | never written (CVBS-param GetInfo is an empty `return 0`) |
| +0x4c | 0x18 | `stHdmiParam` {Csc {matrix, luma, contrast, hue, saturation}, u32 sharpness} | HDMI param set / get |
| +0x64 | 0x1c | `stVgaParam` {Csc, u32 gain, u32 sharpness} | VGA param set / get |
| +0x80 | 0x18 | `stLcdParam` {Csc, u32 sharpness} | LCD param set / get |
| +0x98 | 0x10 | `stGammaParam` {u8 bEn +0x98, u16 u16EntryNum +0x9a, u8 *R +0x9c, *G +0xa0, *B +0xa4} — tables heap-copied, re-allocated when the entry count grows; the entry count must be 0x21 when enabled | GAMMA GetInfo |
| +0xa8 | 0xc | `stColorTemp` {u16 R / G / B offset, u16 R / G / B colour} | COLORTEMP GetInfo |
| +0xb4 | 0xa | `stTimeZoeCfg` (u16 u16InvalidArea[5]) | never written (TIMEZONE_CONFIG GetInfo returns 2 NOT_SUPPORT) |
| +0xc0 | 0x10 | `pstVidLayeCtx[4]` | layer bind; more than 2 bound layers → MOP window merge + merged ODCLK |
| +0xd0 | 4 | `pstDevAttachSrc` | DEVICE_ATTACH GetInfo (destination device ← source container); the Set half is empty |

### 3.4 Video-layer container `DrvDispCtxVideoLayerContain_t` (0x6c)
+0x00 `u32VidLayerId`; +0x04 `pstDevCtx` (the device context it is bound to; used for the CMDQ lookup of a layer
context); +0x08 `bEnable` (stored only, §2.1); +0x0c `stAttr` {win u16 X / Y / W / H +0x0c…+0x12, u32 width +0x14, u32
height +0x18, ePixFormat +0x1c}; +0x20 `stCompress` (u8 bEnbale; the compress query is local-only, §2.1); +0x24
`u32Priority` (local-only query); +0x28 `pstInputPortContain[17]` (port id < 0x11; bind writes the slot and the port's
back-pointer). Same as both source trees' M6 layout. Binding a port to its layer (`__DrvDispCtxBindInputPortWidthVidLayer`,
`__DrvDispCtxUnBindInputPortWidthVidLayer`) is as the source.

### 3.5 Input-port container `DrvDispCtxInputPortContain_t` (0x78)
+0x00 `bUsed`, +0x04 `u32PortId`, +0x08 `bEnable`, +0x09 `bDisplay`, +0x0a `bBeginEnd`; +0x10 `stFrameData` (0x38:
ePixelFormat +0x10, eCompressMode +0x14, eTileMode +0x18, u64 au64PhyAddr[3] +0x20 / +0x28 / +0x30, u32 au32Stride[3]
+0x38 / +0x3c / +0x40); +0x48 `stAttr` {display window X / Y / W / H +0x48…+0x4e, u16 SrcWidth +0x50, SrcHeight +0x52};
+0x54 `stCrop` {X / Y / W / H +0x54…+0x5a}; +0x5c `stRot.enRotate`; +0x60 `stRingBuffAttr` {u8 bEn +0x60, u16 BuffHeight
+0x62, u16 BuffStartLine +0x64, eAccessType +0x68}; +0x6c `pstVidLayerContain` (back-pointer); +0x70
`u16FlipFrontPorchCnt` (cleared at port init / enable). Identical in both source trees.

### 3.6 DMA container `DrvDispCtxDmaContain_t` (0x68) — write-back DMA, unused on this camera
+0x00 `u32DmaId`; +0x04 `stInputCfg` {eType +4, ePixelFormat +8, eCompressMode +0xc, u16 Width +0x10, Height +0x12};
+0x14 `stOutputCfg` {eAccessType +0x14, eMode +0x18, ePixelFormat +0x1c, eCompressMode +0x20, u16 Width +0x24, Height
+0x26}; +0x28 `stBufferAttrCfg` {u8 bEn +0x28, u64 u64PhyAddr[3] +0x30, u32 u32Stride[3] +0x48, u16 RingBuffHeight +0x54,
RingStartLine +0x56, FrameIdx +0x58}; +0x60 `pSrcDevContain`, +0x64 `pDestDevContain`. The ssd_lcd layout; the newer
source adds `u8RingModeEn` / `u16RingMdChange` (0x70) — not in this build.

### 3.7 HAL hardware container `HalDispHwContain_t` (0x18) [R `HalDispClk{Init,DeInit}GpCtrlCfg`, `HalDispClkSet{GpCtrlCfg,MergeOdClk}`, `_HalDispIfGetInfoDevice{Init,DeInit}`, `HalDispGetCmdqByCtx`]
+0x00 `u8 bClkGpCtrl[2]` (per device: GP-control config attached); +0x04 `void *pvCmdqCtx[3]` (CMDQ id 0 / 1 / 2 →
`&gstDispUtilityCmdqCtx[id]`, set at DEVICE_INIT / DMA bind, cleared at deinit); +0x10 `void *pvClkGpCtrl[2]` (per
device → `&stDispClkGpCtrlCfg[dev]`, an 8-byte `{u8 bEn; u32 type}` with type 1 HDMI, 2 LCD, 4 DAC / VGA, 8 DSI, 0x10
CVBS as set by DEVICE_INTERFACE). Neither source tree has this shape (old: 0xc without the clock part; new: 0x14 with two
CMDQ slots and `bTimingUpdate`).

CMDQ context lookup (`hal_disp.c`): `HalDispGetCmdqIdByDevId` (device 0 → 0, device 1 → 1), `HalDispGetCmdqIdByDmaId`
(DMA 0 → 2), `__HalDispGetCmdqCtxByDevId`, `__HalDispGetCmdqCtxByDmaId`, `_HalDispGetCmdqByDmaCtx` (new);
`HalDispGetCmdqByCtx` dispatches by context type 0–3 (a layer context through its `pstDevCtx`).

### 3.8 Every other DISP object (complete list from the symbol table) [R sizes; roles from the code]
<!-- ev:mhal-disp.3.8 -->
The CMDQ contexts and utility globals are in §5.2, the MACE context in §12.3, the colour and picture state in §12.2.

| object | section, size | role |
|---|---|---|
| `_tDispDevice`, `DispClassName`, `stDrvDispPlatformDevice`, `u64Disp_DmaMask`, `stDrvDispPlatformDriver`, `_DispMatchTable` | .data 0xc0 / 4 / 0x1d8 / 8 / 0x5c / 0x188 | Linux device / driver glue (the platform driver is never registered, §10.3) |
| `_PQ_Mutex` | .data 4 | PQ mutex handle |
| `u32FpllHdmiSynthSet[39]` | .data+0xa68, 0x9c | DMA HDMI → CVBS FPLL synthesizer value per timing (§14) |
| CVBS / HDMI-TX ATOP / VGA table descriptors and tables (`stHAL_DISP_*_TBL`, `ST_*_TBL`) | .data | §7.4 |
| `gstMonitorPllConfig[2]` | .data+0x10ac, 0x16 = 2 × 11 bytes | per-device state of the DAC-FPLL frame-lock monitor (§9.5); not in the source tree |
| `gbDispCtxInit`, `pgstCur*` | .bss | §3.2 |
| `gstDispDevTimingCfg` | .bss 8 | `{u32 n; entry *tbl}` (0x34-byte entries, §7.1) |
| `_gu32DispDbgLevel`, `u32DispDbgmgLevel`, `u8PtGenMode` | .bss | debug mask, sysfs `dbgmg`, test-pattern mode |
| `gstDispIrqCfg[6]`, `gbDispIrqInit`, `gstTimeZoneStatus[2]` (+ the VGA-HPD flag word written right after it, +0x50) | .bss 0x30 / 1 / 0x50 | IRQ slots `{u32 virq; u8 valid}`; time-zone status `{u32 flags; u64 ts[4]}` per device (§9) |
| `_tDispClass`, `gstMemAlloc`, `gpDispDeviceNode` | .bss | sysfs class, mi_disp's alloc callbacks, DT node |
| `u32Rot0ImiYaddr`, `u32Rot0ImiCaddr`, `u32Rot1ImiYaddr`, `u32Rot1ImiCaddr`, `u32RotImiIniaddr`, `bImiInit`, `bImiSetAddrInit`, `_gstMopgRotRec` (0x18), `u32MopRotImiTotalAddr` | .bss | MOP rotation IMI buffer bookkeeping (`_HalDispMopIfSetMopRotImiAddrManage`, rotation path; unused on this camera) |
| `PQ_RIU_BASE`, `PQ_SRAM1_IHC` … `PQ_SRAM4_IHC` (4 × 0x248 = 0x121 u16 + padding) | .bss | PQ register base; IHC SRAM staging (§13.5) |
| `gstPqSrcTypeCfg` (8), `_u16PQSrcType_DBK_Detect`, `pHpqCtx`, `gePqLoadSettingType`, `g_u8PkSramEn` | .bss | PQ source-type LUT {count, heap array of 6-byte entries}, HPQ context, load type (manual / auto), peaking-SRAM flag (§7.3) |
| `stPQBinHeaderInfo[10]` | .bss 0x820 = 10 × 0xd0 (`MAX_PQ_BIN_NUM` × `MS_PQBin_Header_Info`) | parsed PQ bin headers |
| unnamed, 0x8866–0x10066 | .bss 0x7800 | the PQ table-index cache `u16 [2 windows][30 panels][256 IPs]` (`MDrv_PQBin_GetTable`) |
| `stDispClkGpCtrlCfg[2]` | .bss 0x10 | clock GP-control config per device (§3.7) |
| `gpMopgLbOdTbl` (0x180), `gbMopgInitNum`, `_bOSD_On` | .bss | MOPG line-buffer order table [R use in `_HalDispMopIf*LineBuf*`]; the other two known by name only [I] |
| `u16MopsSwShadowReg`, `u16MopsSwShadowVal`, `u16Mopg01SwShadowReg`, `u16Mopg01SwShadowVal`, `u16Mopg02SwShadowReg`, `u16Mopg02SwShadowVal` | .bss | software shadows of MOP registers that are written in pieces |
| unnamed, 0x100d4 | .bss 8 | the two u32 MCU frame counters (cleared at DEVICE_DEINIT of an MCU device) |
| `gstVgaDacTrimCfg` (0xc), `gau16VgaDacTrimDefault[3]` | .bss | DAC trim state (§12.6) |
| `gpDispCbTbl[62]` (0x2e8 = 62 × 12), `gbDispHwIfInit` | .bss | HAL query table and its init flag (§4.1) |

## 4. The query layer [R]

### 4.1 Dispatch [R `_DrvDispIfExecuteQuery`, `HalDispIfQuery`, `_HalDispIfGetCallBack`, `HalDispIfInit`]
Every `DrvDispIf*` builds `{u32 query id, void *cfg, u32 cfg size}` and calls `HalDispIfQuery(ctx, q)`. `HalDispIfInit`
(idempotent, flag `gbDispHwIfInit`) fills `gpDispCbTbl`, a 62-entry table of 12-byte rows `{GetInfo fn, Set fn, u16 cfg
size}`. The dispatcher checks `size == row.size` (else "Size %ld != %d" and CFGERR), calls `GetInfo(ctx, cfg)` (validates
and stores into the context; returns 0 OK / 1 CFGERR / 2 NOT_SUPPORT) and, on OK, `Set(ctx, cfg)` (programs the
registers). Ids 0x00–0x3a carry the source's
names, **0x3b–0x3d** (`DEVICE_{VGA,HDMI,LCD}_PARAM_GET`) are new — they copy the stored param struct back
(`_HalDispIfGetInfoDevice*ParamGet`; the Set halves are empty). Query ids per export: §2.1; the others named in this
document: 0x26 CLK_SET, 0x27 CLK_GET, 0x37 CLK_INIT (§10.1), 0x2a DRVTURNING_SET, 0x2b DRVTURNING_GET (§12.6), 0x36
HW_INFO (type 1 layer, 2 port; `DrvDispIfVideoLayerGetHwCount`, `DrvDispIfInputPortGetHwCount`, new).

### 4.2 Validation rules of the GetInfo handlers
- VGA param (0x08, `_HalDispIfGetInfoDeviceVgaParam`) and LCD param (0x0c, `_HalDispIfGetInfoDeviceLcdParam`): the CSC
  matrix must be 0 (bypass), 3 (BT601 → RGB PC) or 4 (BT709 → RGB PC); LCD copies the 0x18-byte `HalDispLcdParam_t` to
  device container +0x80 and returns 0; any other matrix → "Not Support %s", 1 (CFGERR) — as the source.
- `_HalDispIfGetInfoDeviceHdmiParam`, `_HalDispIfGetInfoDeviceColorTemp`, `_HalDispIfGetInfoCmdqInf` (no check),
  `_HalDispIfGetInfoDeviceAttach`, `_HalDispIfGetInfoInterCfgGet`: as the source.
- `_HalDispIfGetInfoClkSet`: exactly 4 clocks. `_HalDispIfGetInfoClkGet`, `_HalDispIfGetInfoClkInit`: as the source.
- `_HalDispIfGetInfoDmaBind`: source ≠ destination device; `_HalDispIfGetInfoDmaUnBind` with NULL contexts only prints and
  returns OK; `_HalDispIfGetInfoDmaAttr`: as the source; `_HalDispIfGetInfoDmaBufferAttr` (DMA buffer attr,
  `DmaBufferAttr`): needs a bound source and destination.
- `_HalDispIfGetInfoVidLayerBind`: layer 2 / 3 cannot bind to device 0; `_HalDispIfGetInfoVidLayerUnBind` as the source.
- Input port: enable, attr, flip, crop, ring-buffer attr — §11.3.

### 4.3 Getters
`_HalDispIfGetInfoDeviceEnable` (u8 at container +4), `_HalDispIfGetInfoDeviceBackGroundColor` (u32 at +8) and
`_HalDispIfGetInfoDevicDisplayInfo`: as the source. DisplayInfo = {Htotal +0x24, Vtotal +0x2e, Hstart +0x20, Vstart +0x2a,
Hactive +0x22, Vactive +0x2c} of the stored timing config at container +0x1c; the interlace and YUV-out bytes are 0.

## 5. Register access: CPU, CMDQ, flip and wait-done [R `hal_disp_utility.c` region, `_HalDispIfSetReg{Access,Flip,WaitDone}`]

### 5.1 Addressing and the register shadow
A register is addressed as the 24-bit `A = bank<<8 | byte`; CPU access is at VA `0xFD000000 + 2·A`
(`gu32DispUtilityRiuBaseVir` = 0xFD000000, fixed). One shared 0x1900-byte shadow `RegBuffer` (`gpvRegBuffer`) = 25
banks × 0x100 bytes is filled at the first init by reading the banks, by type: 0 145B DAC, 1 1150 TOP0, 2 1151 OP2_0,
3 1152 MACE, 4 1153 GAMMA0, 5 1160 TOP1, 6 1161 OP2_1, 7 1163 GAMMA1, 8–0x11 1170…1179 MOP, 0x12 1158 DMA, 0x13 1038
CLKGEN, 0x14 1156 LCD_MISC, 0x15 1157 TVENC, 0x16 **145A** (the source's 1037 LPLL slot; `_HalDispUtilityTransBankToType
(0x1037)` still returns 0x16), 0x17 145A HDMITX ATOP, 0x18 116F (new; type → bank table at .rodata+0x3c08).
`_HalDispUtilityReadBankCpyToBuffer` / `_HalDispUtilityReadBufferCpyToBank` copy a bank to / from its 0x100-byte slot under
`gstDispUtilityMutexCfg`; `_HalDispUtilityWrite1ByteRegBuffer` / `_HalDispUtilityWrite2ByteRegBuffer` write the shadow
(full or masked) — as the source. The source's `_HalDispUtilityGet*` getters are inlined.

### 5.2 CMDQ contexts `gstDispUtilityCmdqCtx[3]` (.bss, 3 × 0x50) [R `HalDispUtilityInit`]
There is one context per CMDQ id 0…2 (`HalDispUtilityInit(id)` rejects id > 2): device 0 uses id 0, device 1 id 1, the DMA
id 2 (§3.7). Per context:

| off | member | |
|---|---|---|
| +0x00 | `s32CmdqId` | |
| +0x04 | `pvRegBuffer` | the shared 0x1900-byte shadow |
| +0x08 | `pvDirectCmdqBuffer` | 0x1c00 B = 0x380 entries `{u32 addr, u16 data, u16 mask}` |
| +0x0c | `u32DirectCmdqCnt` | |
| +0x10 | `pvCmdqCmdBuffer` | 0x280 B = 0x20 entries of 0x14 B `{u32 index in the direct buffer, u32 addr, u16 val, u16 mask, u32 time, u8 pollEq}` (delay / wait / poll records) |
| +0x14 | `u32CmdqCmdCnt` | |
| +0x18 | `pvCmdqInf` | the mi_sys CMDQ interface (`MHAL_CMDQ_CmdqInterface_t *`) |
| +0x1c | u16 `u16RegFlipCnt` | |
| +0x1e | u16 `u16WaitDoneCnt` | |
| +0x20 | u16 `u16SyncCnt` | |
| +0x22 | u8 `bEnSyncCnt` | |
| +0x24 | mutex (`CamOsMutex`, 0x2c bytes, to +0x50) | an addition to the source |

The first 0x24 bytes are the newer source's `HalDispUtilityCmdqContext_t`. Neighbouring globals:
`u32DispUtilityCmdqInitFlag` (a bit per CMDQ id; non-zero = shadow already filled), `gpvRegBuffer`,
`gu32DispUtilityRiuBaseVir` (the unnamed word after `gpvRegBuffer`), `gstDispUtilityMutexCfg` (0x2c, guards the shadow),
and in .data `geDispRegAccessMode[3]` — the access mode **per CMDQ id** (u32: 0 CPU, 1 CMDQ; the source has one global
mode).

### 5.3 Write primitives
- `HalDispUtilityW2BYTEMSKDirect` — immediate RIU read-modify-write; `…DirectAndNoshadow` — the same, for registers
  outside the shadowed banks; `WBYTEMSK` / `W2BYTE` — byte / full-word variants.
- `HalDispUtilityW2BYTEMSK(ctx, reg, val, mask)` — access mode CMDQ → `W2BYTEMSKInquireBuf` (record into the shadow /
  buffer); CPU → Direct.
- `W2BYTEMSKDirectCmdq` — CMDQ mode: append `{addr, data, mask}` to the direct buffer **under the context mutex** (at most
  0x380, an error print when full); CPU mode: immediate write.
- `W2BYTEMSKDirectCmdqWrite(ctx)` ("kick"), under the mutex: if the mode is CMDQ and a CMDQ interface with
  `WriteRegCmdqMaskMulti` exists → send the buffer, interleaving the recorded CmdqCmd entries (delay `0xffff02`,
  **wait `0xffff04` (new)**, poll otherwise) through `_HalDispUtilityDirectCmdqWriteWithCmdqCmd`; otherwise replay the
  buffer with CPU writes and execute the CmdqCmd entries inline (delay → `AddDelayCmd`, wait → `AddWaitCmd`, poll →
  `PollWait`). The counts are reset to 0.
- `_HalDispUtilityAddWriteRegCmdqMaskMulti`: waits for `CheckBufAvailable(n + 10)`, resets the CMDQ engine when its
  status & 0x1700, then `WriteRegCmdqMaskMulti` — as the source.
- `AddW2BYTEMSK(reg, val, mask, ctx)` (new): one immediate CMDQ masked write through `WriteRegCmdqMaskMulti` if the
  context has an interface, else a CPU write.
- `WriteWaitDoneCmd(ctx)` (new), `WriteDelayCmd(time, ctx)`, `WritePollCmd`: record a CmdqCmd entry at the current
  direct-buffer position (CMDQ mode only).
- `HalDispUtilityAddDelayCmd`: the delay goes to the CMDQ only when the access mode of that CMDQ id is CMDQ (1);
  otherwise it returns 0 without a delay (the source: in any mode).
- `AddWaitCmd(ctx)`: `pCmdqInf->CmdqAddWaitEventCmd(event)` with **event 0x1f for CMDQ context 0, 0x20 for context 1**
  (CMDQ mode only). `PollWait(inf, reg, val, mask, time_ns, eq)`: with an interface → `CmdqPollRegBits` (time 0) or
  `CmdqPollRegBits_ByTime`; without → a CPU poll of `reg & val & mask` up to 200 × `max(time / 10000, 1)` µs.
- `__HalDispIfSetDelayWait(ctx, n)` (new): CMDQ mode → n × {WaitDone command; write 0xffff to 1150:40 (context 0) /
  1160:40}; CPU mode → sleep 16·n ms.
- `HalDispUtilityResetCmdqCnt` — the counts := 0.

`HalDispSetCmdqMux` writes nothing: the source compiles its write to DISP_TOP 00 bit 0 only when `CMDQ_POLL_EQ_MODE` is
0, and `CMDQ_POLL_EQ_MODE` = 1 in this build — shown by `HalDispMcuLcdCmdqInit`, whose compiled body is the source's
`#if (CMDQ_POLL_EQ_MODE)` branch (§8.1).

### 5.4 Flip and wait-done handshakes (device context)
Software flags kept in MOP dummy registers: **"CMDQ in process"** = 1172:0F bit 8 (device 0) / 1177:0F bit 8 (device 1);
**wait count** = 1172:1F [15:8] / 1177:1F; the DISP → CMDQ event clear / mask = 1150:40 / 1150:41 (device 1: 1160:40 / 41).
- `REG_ACCESS` (0x2d): GetInfo stores `pCmdqInf` into the context (`HalDispUtilitySetCmdqInf`); Set: mode := CMDQ if
  `enType == 1` else CPU, for this CMDQ id; `HalDispMopIfSetForceWr(ctx, 0)`.
- `REG_FLIP` (0x2e), `bEnable = 1`: pre-act — if the mode is CMDQ with an interface and the in-process flag is 0: set the
  flag (buffered write 0x100) and, unless the interface is MCU (0x200000 / 0x800000), **CMDQ-poll 1150:42 bit 3 (device 1:
  1160:42) until set, timeout 2 s** (the source: an `AddWaitCmd` + clear); RegFlipCnt++ (wraps to 1). Then kick; post-act
  — `HalDispMopIfSetDdbfWr` (MOP double-buffer write strobe, §11.1) and, in CMDQ mode, kick again. The source's post-act
  DAC-AFF reset sequence after a timing change (HDMI + VGA) is **gone**. `bEnable = 0` → `HalDispUtilityResetCmdqCnt`.
- A DMA context (type 3) flips through `__HalDispIfDmaRegFlip` (new): if 1158:01 bit 15 is set, CMDQ-poll 1150:02 bit 2
  (device 1: 1160:02) and 1158:4D == the DMA ring frame index, then kick.
- `REG_WAITDONE` (0x2f), non-MCU interface: pre-act (if the in-process flag == 0 → bEnSyncCnt := 1; WaitDoneCnt++);
  post-act (write the WaitDoneCnt low byte to 1172:1F [15:8]; immediate CMDQ writes 1150:40 bit 1 := 1, 1150:41 :=
  0xfffd, wait event, 1150:40 bit 1 := 1; buffered 1172:0F bit 8 := 0; kick); then, for TTL / BT / MIPI / SRGB /
  MCU-class interfaces (mask 0xfd0000), an immediate write 1156:23 bit 4 := 0. MCU interfaces: pre-act, CMDQ-poll LCD
  1154:0B bit 9 (FLM) for MCU-with-FLM, `…PostActForMcuFlm`, increment a frame counter and CMDQ-poll 1171:1F == counter
  (kept in `u16McuFenceId`), `…PostActForMcuFrameDone` — unused on this camera (no MCU panel).

## 6. Device life cycle and internal config [R `_HalDispIf*Device{Init,DeInit,Enable,BackGroundColor}`, InterCfg handlers]

### 6.1 Init and deinit
- **DEVICE_INIT GetInfo**: the CMDQ id of the device = `HalDispGetCmdqIdByDevId(dev)` (new; on failure it prints
  "UnSupport Cmdq Id" and returns 0 = OK — a vendor bug); `HalDispUtilityInit(cmdq id)` (§5.2); the CMDQ context goes into
  the hw-contain slot of that id; device BgColor := 0; luma / contrast / hue / saturation of the stored VGA, HDMI and LCD
  CSC := 50; `HalDispClkInitGpCtrlCfg`. (The source and U-Boot's copy call `HalDispUtilityInit(dev id)`.)
- **DEVICE_INIT Set** (needs the CMDQ context): `__HalDispIfSetInitWithoutCmdq` = with this CMDQ id temporarily in CPU
  mode: `HalDispSetCmdqIntMask(0xfffd)` (1150:41; the source 0xfffb) → `HalDispSetMaceSrcSel(1)` (MACE source = MOP) →
  `HalDispSetDacInit` (new: 145B:00, 145B:1F, 145B:36 := 0 as full words [I: the DAC stays powered down until VGA is used])
  → `HalDispSetSwReste(0)` → kick → frame colour and display-window colour := (R 0x80, G 0, B 0x80) →
  `HalDispSetDacRgbHpdInit` (empty in this build, §8.1) → `HalDispColorInitVar` → colour-correction matrix of both colour
  engines := `tVideoColorCorrectionMatrix` (§12.1) → `__HalDispIfSetCscToDefualt(colorId == 0)` (new:
  YUV → RGB matrix index 0 for that engine, video RGB gains 0x80, HCS (hue 0x32, saturation 0x80, contrast 0x80),
  brightness 0x80 × 3) → if the PQ "updated" flag of the device is 0: YUV → RGB matrix of the configured colour id :=
  HDTV (1) and hue / saturation / contrast / brightness from the HDMI CSC through `HalDispPictureIfTransNonLinear`; else
  `HalDispPictureIfApplyPqConfig` → read the factory DAC trim once (§12.6). The source: always HDTV for both engines, no CSC default, no DAC init or trim read.
- **DEVICE_DEINIT GetInfo**: `HalDispClkDeInitGpCtrlCfg`, `HalDispUtilityDeInit(cmdq id)`, drop the CMDQ context; if the
  interface was MCU, clear the MCU frame counters. **Set** (new; the source's is empty): DISP_TOP software reset 1150:00
  bit 8 := 1 (device 1: 1160:00), LCD_MISC 1156:00 bit 0 pulse 1 → 0 — all direct writes.

### 6.2 Enable and background colour
- **DEVICE_ENABLE**: `HalDispSetDispWinColorForce(!enable)` only (OP2 1151:22): a disabled device shows the forced window
  colour.
- **BACKGROUND_COLOR**: `HalDispSetFrameColor(R, G, B)` from 0x00BBGGRR (1151:1B / 1C).

### 6.3 Internal config (`DeviceSetConfig` / `DeviceGetConfig`, INTERCFG 0x39 / 0x3a)
State: `gstInterCfg[2]` (.data+0x1048, 2 × 0x10 bytes per device in the layout of `MHAL_DISP_DeviceConfig_t`, §2.2;
initial values: **colour id = 1 for both devices**, everything else 0) and two saved frame-lock words (initial 0xff).
`DeviceSetConfig(dev id, cfg)` runs with a context that carries only the device id (`DrvDispIfSetDeviceConfig`: no `bCtx`).
GetInfo (`_HalDispIfGetInfoInterCfgSet`) stores per set bit of `eType` into `gstInterCfg[dev]`: BOOTLOGO → +4
bBootlogoEn; COLORID → +5 u8ColorId (values > 1 → 0); DISPPAT, RSTMOP, RSTDISP, RSTDAC, RSTHDMITX, RSTLCD, RSTDACAFF,
RSTDACSYN, RSTFPLL → bytes +7…+0xf; then `eType |= cfg.eType`. GOPBLENDID prints "Not Support GOP Blend ID": the other
flagged bytes are still stored, byte +6 (GOP blend id) never is, and the `eType` merge and the Set half are skipped
(returns 1 = CFGERR). Set (new; the source's Set is empty), each a direct no-shadow write:

| bit | action |
|---|---|
| DISPPAT | DISP_TOP0 / 1 reg 07 [3:0] := 0xe (on) / 1 (off) — internal test pattern (1150:07 / 1160:07) |
| RSTMOP | MOPG 1170:00 bit 0 := flag (device 1: 1175:00) |
| RSTDISP | DISP_TOP 1150:00 bit 8 := flag (device 1: 1160:00) |
| RSTDAC | 1150:30 bit 0 (`HalDispSetInterCfgRstDac`) |
| RSTHDMITX | HDMI-TX DTOP 1459:00 bit 1 |
| RSTLCD | LCD 1154:00 bit 0 (`HalDispSetInterCfgRstLcd`) |
| RSTDACAFF | frame-lock monitor (below), then 1150:30 bit 1 (`HalDispSetInterCfgRstDacAff`) |
| RSTDACSYN | monitor, then DAC 145B:01 bit 0 (`HalDispSetInterCfgRstDacSyn`) |
| RSTFPLL | monitor, then 1150:15 bit 7 (`HalDispSetInterCfgRstFpll`) |

Reset-control monitor (`__HalDispIfInterCfgRstContrlMonitor`): while any of the DAC-AFF / DAC-SYN / FPLL resets is
requested, the device's frame-lock bit (1172:3F bit 0 for device 0, bit 1 for device 1) is saved once and cleared; when
all three are released, the saved bit is restored.

fbdev uses only COLORID: `sstar_FB_SetDst(layer, en)` → `DeviceSetConfig(dev, {eType 2, u8ColorId = en != 0})` for the
device of the layer — it selects which of the two OP2 colour engines (§12.1) gets the device's CSC settings; nothing is
routed (`rgn-gop.md` §5.4). U-Boot's `bootlogo` calls it with the MISC config's colour-matrix / GOP ids (`uboot.md` §5.3).

## 7. Output timing and interface [R]

### 7.1 Timing table hand-over [R `DrvDispIfInitPanelConfig`, `_DrvDispIfTransDeviceOutpuTimingInfoToHal`, `__DrvDispIfGetTimingFromTbl`]
mi_disp passes 0x27 `MHAL_DISP_PanelConfig_t` built from the SYSCFG panel records (`config_tool.md` §5.5).
`InitPanelConfig(arr, n)` rejects n > 0x27, frees a previous table and allocates n × 0x34-byte entries in
`gstDispDevTimingCfg = {u32 n; entry *tbl}` (.bss, 8 bytes in the symbol table; the first reading of the code had a
third word `u32 dbg` after `tbl` — not reconciled with the 8-byte symbol). Entry (this build; the source's entry has a
DCLK field this one lacks):

| off | from PanelConfig | meaning |
|---|---|---|
| +0x00 u8 | `bValid` (+0x70) | slot valid |
| +0x04 u32 | `eTiming` (+0x68) through the timing translation | HAL timing id (§2.2 enum) |
| +0x08…+0x1a u16 × 10 | HSyncWidth +0x18, HSyncBackPorch +0x1a, HStart +0x20, Width +0x24, HTotal +0x2a, VSyncWidth +0x1c, VBackPorch +0x1e, VStart +0x22, Height +0x26, VTotal +0x30 | the `HalDispDeviceTimingConfig_t` (HsyncW, HsyncBP, Hstart, Hactive, Htotal, VsyncW, VsyncBP, Vstart, Vactive, Vtotal) |
| +0x1c u16 | `round(DCLK(+0x38) / (VTotal·HTotal))` (rounds up when the remainder > half) | fps |
| +0x1e, +0x20 u16 | SpreadSpectrumStep +0x42, Span +0x44 | SSC (stored, not programmed by the HAL) |
| +0x24…+0x30 u32 × 4 | 10⁶ / fps × {VsyncW, VBP, Vactive, Vtotal − VsyncW − VBP − Vactive} / Vtotal | µs of each vertical zone (time-zone bookkeeping, §9.4) |

Of the 0x74-byte panel config mhal reads only bValid, eTiming, HSyncWidth, HSyncBackPorch, HStart, Width, HTotal,
VSyncWidth, VBackPorch, VStart, Height, VTotal, DCLK (only to compute the frame rate, so DCLK is taken in Hz) and the SSC
step / span; Max / Min totals, Max / Min DCLK, polarities, dither, bit mode and link type are ignored. The porch timings
are handed in; the per-timing pixel-clock synthesizer, DAC and HDMI-TX analog tables are compiled into mhal (§7.4). (U-Boot's
copy of this function, 0x23E219C8, with the 0x6c-byte struct and a limit of 0x1f, divides 1000000 by a u8 fps field:
`uboot.md` §5.4.)

`DeviceSetOutputTiming(ctx, intf, {eTimeType, pstSyncInfo})`: **USER (0x26)** → the HAL timing is built from the
SyncInfo (HsyncW = Hpw, HsyncBP = Hbb, Hactive = Hact, Hstart = HStart, Htotal = Hpw + Hbb + Hfb + Hact, the same for V;
fps = u32FrameRate; the four µs zones as above; a NULL SyncInfo → fail); **any other id** → the first valid table entry
with that id (none → "Can't find Timing(%s) in Table", fail). Then query 0x07 with `{u32 timing, 0x2c-byte timing config}`
(size 0x30). For HDMI the pixel clock is **not** derived from DCLK: it comes from the per-timing synthesizer table
(§7.4); a USER timing gets table id 0x1b (invalid) [R `__HalDispIfGetTblId`; I: so a USER timing cannot drive HDMI
correctly].

### 7.2 Interface (`DEVICE_INTERFACE`, 0x06)
GetInfo rejects YPbPr (8) and LCD (0x10); more than one bit set is accepted only as HDMI | VGA (5); the mask is stored at
device container +0x10. Set for HDMI (1, this camera's case): `HalDispSetLcdFpllRefSel(3)`; `HalDispSetHdmiSrcMux(dev ==
1)`; `HalDispSetDsiClkGateEn(the other device exists and drives MIPI-DSI)`; `HalDispSetTgenExtHsEn(0)`; clock GP-control
`{en 1, type 1 HDMI}` → `HalDispClkSetGpCtrlCfg` (§10.2). The other interfaces (unused on this camera; register bodies in
§8.2):
- VGA (4): adds `HalDispSetDacSrcMux(dev)`, `HalDispSetDacFpllDly` (device 1 0xb8 / device 0 0x50),
  `HalDispSetDacFpllRefSel(3)`, `HalDispSetDacExtFrmRstEn(1)`, `HalDispSetDacSynthSetSel(intf == VGA)`, the VGA H / V-sync
  pads (`HalDispSetVgaHVsyncPad`); GP-control type 4.
- CVBS: `HalDispSetCvbsSrcMux`; GP-control type 0x10.
- DSI: `HalDispSetDsiSrcMux`, `HalDispSetDsiClkGateEn(1)`, `HalDispSetTgenExtHsEn(1)`, `HalDispSetDsiAff(0x6140)`; GP-control type 8.
- TTL / BT656 / BT601 / BT1120 / SRGB: `HalDispSetLcdFpllDly` (device 1 0xb8, SRGB 0xb4; device 0 0x60, SRGB 0x33),
  `HalDispSetLcdSrcMux(dev)`; BT656 / BT601 → `Hdmi2OdClkRate(1)`, SRGB → `(2)`; GP-control type 2.
- MCU: `HalDispMcuLcdCmdqInit`, `HalDispKeepMcuReset(1)`, `HalDispSetCmdqMux(1)` (writes nothing, §5.3),
  `HalDispSetLcdSrcMux(dev)`; GP-control type 2.
Differences from the source: the DSI clock-gate logic is new; the VGA path no longer sets DacFpllGain 0x99 / Thd 8 here
(moved to §7.3 step 4).

### 7.3 Output timing (`DEVICE_OUTPUTTIMING`, 0x07) [R `_HalDispIfSetDeviceOutputTiming`, `__HalDispIfSetTgenConfig`, `__HalDispIfSetTgenRest`]
GetInfo stores the timing id (device container +0x18) and the 0x2c-byte config (+0x1c). Set, in order:
1. `HalDispSetFrameLockDummy(0, dev1 ? 2 : 1)` — 1172:3F, mask 1 / 2: a dummy register used as the frame-lock flag,
   cleared at the start of every OUTPUTTIMING.
2. Interface-specific pre-steps: HDMI | VGA → `HalDispSetDacFpllEn(0)`, `HalDispSetSwReste(1)`,
   `HalDispSetDacPllPowerDonw(1)`; CVBS → SwReset(1), `HalDispSetCvbsSwRst(0)`; DSI → SwReset(1),
   `HalDispSetMipidsiAffReset(1)`. HDMI alone: none.
3. **Timing generator** (`__HalDispIfSetTgenConfig`) from the stored config: `HsyncSt = (Hstart ? Hstart : Htotal) −
   HsyncW − HsyncBP` (the same for V); VsyncEnd = VsyncSt + VsyncW − 1; Vfde / Vde start = Vstart, end = Vstart + Vactive −
   1; Vtt = Vtotal − 1; the same pattern for H; Htt = Htotal − 1. DSI: Hfde / Hde start −58 when > 58, Vfde / Vde start +1.
   **New:** when the peaking SRAM is enabled (`g_u8PkSramEn`) the DE (not FDE) window of device 1 starts 2 pixels / lines
   earlier. VGA also sets the DAC copy of the TGEN (`HalDispSetTgenDac{HsyncSt,HsyncEnd,HdeSt,HdeEnd,VdeSt,VdeEnd}`, §8.2).
   Registers (`HalDispSetTgen*`, feature-equal to the source; the getters `HalDispGetTgen*` are inlined reads): OP2 bank
   1151 (device 1: 1161), regs 0x11 Htt, 0x12 Vtt, 0x13 / 0x14 HsyncSt / End, 0x15 / 0x16 VsyncSt / End, 0x17 / 0x18
   HfdeSt / End, 0x19 / 0x1a VfdeSt / End, 0x1d / 0x1e HdeSt / End, 0x1f / 0x20 VdeSt / End (13 bit, mask 0x1fff).
4. Table id = `__HalDispIfGetTblId(timing)` for HDMI / CVBS / VGA (byte table .rodata+0x4939, 0x26 entries, default 0x1b;
   mapping in §7.4). HDMI or VGA: **HDMI → `__HalDispIfDumpHdmitxAtopTbl(id)`** (new: the HDMI-TX analog table, §7.4);
   `__HalDispIfDumpSynthSetTbl(id)` (synthesizer for the HDMI pixel clock; the VGA synthesizer table only when the
   interface is exactly VGA; no `HalDispSetHdmiSynthSet` call — the source and U-Boot call `HalDispSetHdmiSynthSet(0)` for
   HDMI + VGA and write the VGA table when intf & 4); `HalDispSetDacFpllGain(0xff)`, `HalDispSetDacFpllThd(t)` with t = 8
   for 576P50 / 480P60, 3 for 720P50, 720P60, 1024x768, 720P59.94, 720P29.97, 2 for 1280x1024, 1366x768, 1440x900,
   1280x800, 1 otherwise. VGA: IDAC tables (common + per timing), `HalDispSetDacPllPowerDonw(0)`.
5. `__HalDispIfSetTgenRest`: TTL → LCD AFF reset pulse (10-unit delay); VGA bit set → (device 1 and `CamOsChipRevision()
   == 1` → `ClearDisp0SwReset`); HDMI | VGA → SwReset(0), `HalDispSetDacFpllRst(1)`, `HaldispSetDacFpllSynthSwRst(1)`,
   wait 5 frames, release both, `HalDispSetDacFpllEn(1)`, `HalDispPollDacFpllLock` (CMDQ poll 1150:17 bit 15, 2 s); VGA
   alone → 200-unit delay; `HalDispSetDacAffReset` pulse — 1150:30 [3:1] := 0xE then 0 (wait 2); HDMI | VGA →
   `HalDispClearDacFpllUnLock` (1150:17 := 0x4000) and, for interface value 5 (HDMI + VGA together) only,
   `HalDispSetFrameLockDummy` sets the frame-lock flag (device 0 bit 0 / device 1 bit 1, same mask) that arms the
   frame-lock monitor (§9.5). HDMI alone: nothing. (The source's version of this step is the plain reset logic without the
   lock sequence.)
6. CVBS: PAL / NTSC timing-generator constants (PAL as the source; **NTSC differs**: HsyncEnd 0x3d, HfdeSt 0x7a, HfdeEnd
   0x349, HdeSt 0x7a, HdeEnd 0x349, Htt 0x359; V: 0 / 0x2c / 0x2d / 0x20c), CVBS tables, SwReset(0), CvbsSwRst(1). DSI:
   `HalDispSetMipidsiAffReset(0)` (1150:24 bit 12, device 1 1160:24), SwReset(0).
7. **PQ reload (new):** if `HalDispHpqGetSourceIdx` finds a PQ source for the device: (load type 0 →
   `HalDispHpqSetSrcId`), `__HalDispIfGetPictureDataFromPq` → `HalDispPictureIfSetPqConfig` →
   `HalDispPictureIfApplyPqConfig` → `HalDispHqpLoadSetting` (§13). With no PQ bin loaded (this camera: no PQConfig.ini)
   the step does nothing [I].
This camera's two timings (1080P60 = 0xc → table id 8; 3840x2160_30 = 0x1c → id 0x11) therefore program: the TGEN, the
HDMI-TX ATOP common rows + column, the synthesizer column, DAC FPLL gain 0xff / threshold 1.

### 7.4 Compiled-in register tables [R: dumped from `.data`]
<!-- ev:mhal-disp.7.4 -->
Row format `{addr[3] big-endian byte address, mask, data…}`; `HalDispDumpRegTab` writes each row with `WBYTEMSK` (a byte
write; an odd byte address = the high byte of the 16-bit register). Tables: VGA — IDAC per timing (1 row), IDAC
common (9 rows), HDMI synthesizer (3 rows), VGA synthesizer (3 rows); HDMI-TX ATOP — per timing (14
rows) + common (23 rows); CVBS — IDAC common (8), PLL common (19), TVENC per standard (24), TVENC common
(26). The source's tables have 18 timing columns and the IDAC per-timing table 2 rows (145B:33, 145B:37); this build
moved 145B:33 into the common table.

Timing id → column (27 columns): 0 480p60, 1 576p50, 2 720p50, 3 720p60, 4 1080p24, 5 1080p25, 6 1080p30, 7 1080p50,
**8 1080p60**, 9 720p29.97, 10 720p59.94, 11 1080p29.97, 12 1080p59.94, 13 1440p30, 14 1440p50, 15 1440p60, 16
2160p25, **17 2160p30**, 18 2160p29.97, 19 1024x768, 20 1280x800, 21 1280x1024, 22 1366x768, 23 1440x900, 24 1680x1050,
25 1600x1200, 26 1920x1200. HAL timing → id: PAL → 1 (CVBS table), NTSC → 0, 1080P24 / 25 / 30 → 4 / 5 / 6, 720P50 / 60
→ 2 / 3, 1080P50 / 60 → 7 / 8, 576P50 → 1, 480P60 → 0, 1024x768 → 19, 1280x1024 → 21, 1366x768 → 22, 1440x900 → 23,
1280x800 → 20, 1680x1050 → 24, 1600x1200 → 25, 1920x1200 → 26, 2560x1440_30 / 50 / 60 → 13 / 14 / 15, 3840x2160_30 → 17,
1080P59.94 / 29.97 → 12 / 11, 720P59.94 / 29.97 → 10 / 9, 3840x2160_25 / 29.97 → 16 / 18; 960H, 1080I, 640x480, 800x600,
1920x2160, 2560x1600, 3840x2160_60 → 0x1b (no table: the table writers print an error and still write column 0x1b, i.e.
past the row [I: a vendor bug]).

HDMI synthesizer (DISP_TOP0 1150:10 L / H, 1150:11 L — one 24-bit value per timing [I: the HDMI pixel-clock synthesizer
ratio]): 0x2e8ba2 for all 74.25 / 148.5 / 297 MHz-family timings (720p, 1080p, 1440p, 2160p25 / 30), 0x2e9790 for the
1000 / 1001 rates, 0x3fefa1 480p60, 0x400000 576p50, the VGA modes individual.

HDMI-TX ATOP per timing (bank 145A): the 1440p50 / 60 and 2160p columns differ from all others (145A:16H 0x07, 22 / 23 /
24 / 25 0x2c, 30H / 31H / 32L 0x08, 39H / 3BL 0x33, 3BH 0x23, 1DL 0x00); 1DL is otherwise 0x13 (480p, 576p, 720p29.97),
0x12 (720p, 1080p24–30, the 1024x768 class) or 0x11 (1080p50 / 60, 1440p30, 1600x1200, 1920x1200) [R values; I: drive
strength / pre-emphasis stepped by the TMDS rate]. The rows 145A:32L (0x0F / 0x08) are the PdRtermCh / PdLdoPreDrvCh
fields of the HDMI-TX drive-current interface; field names of the ATOP registers — `mhal-hdmitx.md` §4.2. DISP thus writes
bank 145A itself at every HDMI timing change.

The HDMI-TX DTOP timing tables `ST_INIT_HDMITX_DTOP_TIMING_TBL` (.data+0x2488, 13 rows × 27) and its common table
(.data+0x261b, 7 rows) sit next to the DISP data but belong to the HDMI-TX block (descriptor at .data+0x2470, used from
HDMITX code at `.text` 0x417c4…0x41ae4 [R relocations]); they are dumped and explained in `mhal-hdmitx.md` §4.3, §4.4.

The CVBS tables: `__HalDispIfDumpCvbsTbl` writes the 4 descriptors, the common ones with id 1 / column 0, the
per-standard ones with the table id (2 = invalid, printed and still written). `__HalDispIfDumpIdacTbl` writes the IDAC
tables.

## 8. The other output paths: VGA DAC, CVBS, LCD, MCU, DSI — unused on this camera [R]

### 8.1 Differences from the source
1. **`HalDispSetDacAffReset`** (VGA path, OUTPUTTIMING step 5): writes 1150:30 **[3:1] := 0xE** (on) / 0 (off), mask 0xE
   [R disassembly 0x1b85c: `mov.w r2,#14`]; the source writes bit 0 (mask 1). Bit 0 of 1150:30 is the INTERCFG RSTDAC bit
   and the DMA path's `HalDispDmaSetDacAffRst` (a pulse, §14); INTERCFG RSTDACAFF uses bit 1 only;
   the frame-lock monitor writes AFF << 2 (§9.5). So this build has several writers of 1150:30 with different bit sets.
2. **`_HalDispClkSetMipiTxDsiApb`** (DSI): disable = 1137:37 **bit 0** (value `(rate & 3) << 2 | !en`, mask 0xF); the
   source computes `bEn ? 0 : 0x10` with mask 0xF, so its disable bit falls outside the mask and is never written — fixed
   in the binary.
3. **`HalDispMcuLcdCmdqInit`** (MCU): the source's `#if (CMDQ_POLL_EQ_MODE)` branch — direct writes 1154:08 bit 0 := 0 and
   bit 9 := (interface == MCU) ? 0 : 1.
4. **`HalDispSetDacRgbHpdInit` is empty** (the source body is under `DISP_VGA_HPD_ISR_SUPPORT`, 0 here). The DAC hot-plug
   set-up is written by **`HalDispSetVgaHpdInit`** through the buffered path: 1150:34 bit 0 := 1 (DAC HPD enable),
   1150:37 [11:0] := 0x110 (length), 1150:38 [7:0] := 0xFF (delay), 1150:39 [7:0] := 0x50 (data code), 145B:1E [1:0] := 0,
   145B:14 [3:0] := 7 — equal to the source's `HalDispSetVgaHpdInit` (the source's `RgbHpdInit` copy would write 0x120 /
   mask 0x3FF to 1150:39).
5. `HalDispSetTgenExtHsEn` sets bit 2 of 1151:10, for device 1 of 1161:10 (the source: 1151:10 for both devices — fixed
   in the binary).
6. `HalDispSetCvbsSwRst`: a 20-unit delay before the release (§8.2).
7. New helpers: `HalDispSetMipidsiAffReset`, the DAC FPLL helpers (`HalDispSetDacFpllRst`, `HaldispSetDacFpllSynthSwRst`,
   `HalDispClearDacFpllUnLock`, `HalDispPollDacFpllLock`, `HalDispSetDacPllPowerDonw`, `HalDispSetDacFpllStepGain`),
   `HalDispSetDacInit` (§6.1), the frame-lock dummy set / get (1172:3F), the InterCfg handlers (§6.3),
   `HalDispDumpRegTab` (§7.4).
8. The DMA path's `HalDispDmaSetHdmiSynthSet` — §14.
Everything else on these paths is equal to the source in logic, registers, masks and values.

### 8.2 Register bodies of the setters (this build)
All buffered unless marked "direct". Set-ups given elsewhere: `HalDispSetDacInit` §6.1, the `HalDispSetInterCfgRst*`
writers §6.3, `HalDispSetDacAffReset`, `HalDispMcuLcdCmdqInit` and `HalDispSetVgaHpdInit` §8.1, the DAC trim §12.6,
`HalDispDmaWaitFpllLock` and `HalDispDmaSetDacAffRst` §14.

| function | register / bits | value |
|---|---|---|
| `_HalDispClkSetMipiPixelG` / `_HalDispClkSetMipiPixel` | 1137:23 [3:0] / [7:4] (direct) | bit 0 / bit 4 = gate (1 when disabled), [3:2] / [7:6] = rate select |
| `_HalDispClkSetLcdSrc` / `_HalDispClkSetLcd` | 1137:24 [3:0] / [11:8] (direct) | gate bit 0 / bit 8, rate [3:2] / [11:10] |
| `HalDispClkSetFpllOdclk0` / `HalDispClkSetFpllOdclk1` | 1137:25 [3:0] / [7:4] (direct) | gate bit 0 / bit 4, rate [3:2] / [7:6] |
| `_HalDispClkSetTve` | 1137:35 [7:4] (direct) | gate bit 4, rate [7:6] |
| `_HalDispClkSetDac` / `_HalDispClkSetDacWaff` | 1137:36 [3:0] / [7:4] (direct) | gate bit 0 / bit 4, rate [3:2] / [7:6] |
| `_HalDispClkSetMipiTxDsiApb` | 1137:37 [3:0] (direct) | gate bit 0, rate [3:2] (§8.1 item 2) |
| `HalDispClkSetMipiTxDsi` | CLKGEN 1038:5E [12:8] (direct) | gate bit 8, rate [12:10] |
| `HalDispSetTgenDacHsyncSt`, `…HsyncEnd`, `…HdeSt`, `…HdeEnd`, `…VdeSt`, `…VdeEnd` | 1151:23, 24, 27, 28, 29, 2A [12:0] | the DAC copy of the timing generator |
| `HalDispSetDacFpllDly` / `HalDispSetDacFpllRefSel` / `HalDispSetDacExtFrmRstEn` | 1151:10 [15:8] / [7:6] / bit 4 | delay byte, reference select, external frame reset |
| `HalDispSetLcdFpllDly` / `HalDispSetLcdFpllRefSel` | 1161:10 [15:8] / [7:6] | (OP2 of device 1, as the source) |
| `HalDispSetDacFpllEn` / `HalDispSetDacFpllRst` | 1150:15 bit 0 / bit 7 | |
| `HalDispSetDacFpllStepGain` / `HalDispSetDacFpllGain` / `HalDispSetDacFpllThd` / `HalDispSetDacFpllAbsLimit` | 1150:1A [11:0] / 1150:1B [7:0] / 1150:1B [15:8] / 1150:1C–1D (32 bit) | |
| `HalDispClearDacFpllUnLock` / `HalDispPollDacFpllLock` | 1150:17 := 0x4000 / CMDQ poll 1150:17 bit 15, a 200-unit delay first, timeout 2 s | |
| `HalDispSetDacSrcMux`, `HalDispSetLcdSrcMux`, `HalDispSetDsiSrcMux`, `HalDispSetCvbsSrcMux` | 1150:21 bit 0, bit 2, bit 3, bit 4 | source device select (1 = device 1) |
| `HalDispSetDsiAff` / `HalDispSetMipidsiAffReset` | 1150:24 [15:0] / 1150:24 (device 1: 1160:24) bit 12 | |
| `HalDispSetClkDsiGateEn`, `HalDispSetDsiClkGateEn` | 1150:25 bit 4 (two functions, the same bit) | |
| `HalDispSetDacSynthSetSel` | 145B:05 bit 4 | |
| `HaldispSetDacFpllSynthSwRst` / `HalDispSetDacPllPowerDonw` | 145B:01 [1:0] / 145B:36 [11:0] + a 20-unit delay | |
| `HalDispIrqSetDacEn` | 145B:15 [2:0] (direct) | 7 = R / G / B DAC outputs on, 0 = off |
| `HalDispSetLcdAffReset` / `HalDispLcdAffGateEn` | 1156:30 bit 1 / 1156:32 bit 0 | |
| `HalDispMcuLcdRefresh` / `HalDispMcuLcdReset` | 1154:00 bits 6 + 1 (0x42) / 1154:00 bit 0 | |
| `HalDispKeepMcuReset` | direct: 1150:00 (device 1: 1160:00) high byte \|= 0x11 (bits 8, 12); 1156:00 bit 0 := v, then := 0 | |
| `HalDispSetCvbsSwRst` | TVENC 1157:40 bit 0; the release preceded by a 20-unit delay | |
| `HalDispSetVgaHVsyncPad` | PADTOP 103C:77 [9:8] := 3 (direct), skipped when `DrvDispOsPadMuxActive()` ≠ 0 | |
| `HalDispDmaSetCvbsRingEn`, `…MopMux`, `…Dest`, `…OffsetEn` | 1158:6F bit 0, bit 1, bit 2, bit 3 | |
| `HalDispDmaSetCvbsImiStartAddr` / `HalDispDmaSetCvbsImiOffset` | 1158:70 / 71 (Y, 16 + 12 bit), 72 / 73 (C) / 1158:74–77 | |
| `HalDispDmaSetCvbsSize` / `HalDispDmaSetCvbsBufSize` | 1158:78, 7A, 7C, 7E / 1158:79, 7B, 7D, 7F (H0, V0, H1, V1) | |
| `HalDispDmaSetVideoPathFpllDly` / `HalDispDmaSetVideoPathFpllRefSel` | 1151:2C [12:0] / 1151:2B [1:0] | |
| `HalDispDmaSetFpll1SrcMux` | 1150:21 bit 9 (enable), bit 8 (source = device 1) | |
| `HalDispDmaSetHdmiSynthSet`, `HalDispDmaSetTvencSynthSet` | 1150:10–11 / 1160:10–11 (24 bit) | |
| `HalDispDmaSetFpllEn`, `…SwRst`, `…Gain`, `…Thd`, `…AbsLimit` | 1160:15 [1:0], 1160:15 bit 7, 1160:1B [7:0], [15:8], 1160:1C–1D | |
| `HalDispDmaSetTvencSynthSel` | LPLL 1037:4C bit 0 | |
| `__HalDispIfDumpSynthSetTbl`, `__HalDispIfDumpIdacTbl`, `__HalDispIfDumpCvbsTbl` | the tables of §7.4 | |

## 9. Interrupts [R: DT node `disp`, `_DrvDispModuleInit`, `drv_disp_irq.c` / `hal_disp_irq.c` regions]
<!-- ev:mhal-disp.9 -->

### 9.1 Slots
DT `disp { compatible = "sstar,disp"; interrupts = <SPI 0x63, 0x67, 0x64, 0x68, 0x6a, 0x6c (level-high)>; clocks = <4
handles> }`. Module init walks `DrvDispIrqGetIrqCount()` = **6** (the source: 5) entries with `CamOfIrqToResource(node, i)`
and stores the virq in slot i (`gstDispIrqCfg[6] = {u32 virq, u8 valid}`; `DrvDispIrqSetIsrNum` bounds the index at 5 and
has no lazy init). Slot → source (`HalDispIrqIoCtl` GET_ID variants):

| slot | SPI | source | mask / flag / clear registers, bits |
|---|---|---|---|
| 0 | 0x63 | device 0 "vsync" (MOPG frame) | mask 1170:0C bit 0 (1 = masked), flag 1170:02 bit 0, clear = 1170:0C bit 0 written 1 then 0 |
| 1 | 0x67 | device 1 | the same in 1175:0C / 1175:02 |
| 2 | 0x64 | device 0 internal: time-zone edges + DMA active | mask 1150:03, flag / clear 1150:02 (clear = write 1); bits 7 vsync↑, 6 vsync↓, 5 VDE↑, 4 VDE↓ (time zone), 3 DMA active on, 2 DMA active off |
| 3 | 0x68 | device 1 internal | 1160:03 / 1160:02 |
| 4 | 0x6a | VGA hot-plug (DAC) | mask 1150:3B, flag 1150:3E, clear 1150:3D; bit 2 on, bit 3 off. Support compiled out for Linux (`HAL_DISP_VGA_HPD_ISR_SUPPORT_LINUX` 0 in the source); the binary's `_DrvDispIrqCreateVgaHpdIsr` / `_DrvDispIrqDestroyVgaHpdIsr` are empty |
| 5 | 0x6c | **LCD / MCU (new)** | mask LCD 1154:08, flag 1154:0B, clear 1154:0D; bits 0 FRM_END, 1 IDX_RDY, 2 CMD_RDY, 9 FLM; clearing FRM_END also writes the caller's u16 to 1171:1F (MCU frame counter) |

`MHAL_DISP_GetDevIrq` / `EnableDevIrq` / `GetDevIrqFlag` / `ClearDevInterrupt` act on slot 0 / 1 of the device, the DMA
variants on slot 2 / 3, the LCD variants on slot 5 (`_HalDispIrqGetLcdId` = 5). mi_disp requests slot 0 / 1 as
"mi_disp%d_isr" and slot 5 as "mi_disp%d_isr_lcd" (`mi_disp-mi_hdmi.md` §4.5). `DrvDispIrqGetIsrNumByDevId`: a fake device
context + GET_ID, as the source; `DrvDispIrqGetIsrNum*` and `DrvDispIrqGetLcdIsrNum` differ in prints only. Enabling a
device or internal source writes 0 to its mask bit, disabling writes 1. The HAL functions `_HalDispIrqEnableIsr`,
`_HalDispIrqGetFlag`, `_HalDispIrqClear` and the DMA, time-zone and VGA-HPD variants use the same bits and registers as
the source macros (direct RIU access; the decompile shows the VA form).

### 9.2 IRQ type values and ioctls
The HAL IRQ type values of this build (the source tree's shared `hal/pub/hal_disp_irq.h` is a different revision and does
not define the time-zone names its mercury6 HAL uses; its values do not apply) [R `_HalDispIrq{Enable,GetFlag,Clear}*` bit
tests] — these are the values mi_disp passes through `MHAL_DISP_EnableDevIrq` and the DMA / LCD variants:

| type | value | register bit |
|---|---|---|
| device (MOP) vsync | 0x0001 | 1170:02 / 1175:02 bit 0 |
| time zone VDE↓ / VDE↑ / vsync↓ / vsync↑ | 0x0100 / 0x0200 / 0x0400 / 0x0800 | bits 4 / 5 / 6 / 7 of 1150:02 / 03 (device 1: 1160) |
| VGA HPD on / off | 0x1000 / 0x2000 | 1150:3E bits 2 / 3 |
| DMA active-on / active-off | 0x4000 / 0x8000 | 1150:02 bit 3 / bit 2 |
| LCD FRM_END / IDX_RDY / CMD_RDY / FLM (own ioctls 4–7) | 0x0001 / 0x0002 / 0x0004 / 0x0200 | 1154 bits 0 / 1 / 2 / 9 |

`HalDispIrqIoCtl` ioctls: ENABLE 0, GET_FLAG 1, CLEAR 2, GET_ID 3, **LCD_ENABLE 4, LCD_GET_FLAG 5, LCD_CLEAR 6, LCD_GET_ID 7
(new)**, DMA_ENABLE 8 … DMA_GET_ID 0xb, TIMEZONE_SUPPORTED 0xc … TIMEZONE_CLEAR 0x10 (TIMEZONE_GET_STATUS 0x11),
VGA_HPD_SUPPORTED 0x12 … VGA_HPD_CLEAR 0x16.

### 9.3 LCD interrupt quirks
- `_HalDispIrqEnableLcdIsr` writes the LCD mask 1154:08 **only when disabling** (`cbnz` over the store when `bEnable` ≠
  0): enabling never clears a mask bit [R disassembly 0x2849a]. `DrvDispIrqEnableLcd` asks for FRM_END | FLM = 0x201.
- `_HalDispIrqGetLcdFlag` reports only bits 0–2 (never FLM).
- `_HalDispIrqClearLcd` writes 1 to 1154:0D for the given bits, writes the caller's u16 to 1171:1F for FRM_END, and **also
  sets the mask bits** 1154:08 for IDX_RDY / CMD_RDY.
These are NOSRC functions; LCD is not used on this camera (no panel).

### 9.4 The internal time-zone interrupt
At device create `DrvDispIrqCreateInternalIsr` → `_DrvDispCreateTimeZoneIsr` installs DISP's own handler on slot 2 / 3
when the HAL reports time-zone support (ioctl 0xc; `HAL_DISP_TIMEZONE_ISR_SUPPORT_LINUX` = 1 [S]): `request_threaded_irq
(virq, _DrvDispIrqTimeZoneCb, 0, flags 0x2080, "mdisp_interisr0" / "mdisp_interisr1")`, then disable / enable and
TIMEZONE_ENABLE for type **0x100 only** (VDE↓). `_DrvDispIrqDestroyTimeZoneIsr` disables the same type. The source uses
all three edges (`E_HAL_DISP_IRQ_TYPE_TIMEZONE`) and checks the slot id against MAX, which the binary does not.

The handler (`_HalDispIrqGetTimeZoneFlag`, a copy inlined in `HalDispIrqIoCtl`) records the edge flags and timestamps in
`gstTimeZoneStatus[dev]` `{u32 flags; u64 ts[4]}` (and `u16SyncCnt`) as the source, and in its VDE↓ branch runs the
frame-lock monitor (§9.5). `DeviceGetTimeZone` (`__HalDispIfGetTimeZoneType`) issues TIMEZONE_GET_STATUS (0x11) with type
0x100 into that status (timestamps at +8, +0x10, +0x18, +0x20 = vsync↑, vsync↓, VDE↑, VDE↓): flags 0x800 → zone 2
BACKPORCH; flags 0x200 or 0xA00 → 3 ACTIVE if now − ts[VDE↑] ≤ device container +0x40 (the V-active µs of the stored
timing, §7.1) else 4 FRONTPORCH; 0x100 → 4 FRONTPORCH; else 0 NONE — as the source.

### 9.5 DAC-FPLL frame-lock monitor (VDE↓ branch of `_HalDispIrqGetTimeZoneFlag`) [R: counter names S from its debug strings]
<!-- ev:mhal-disp.9.5 -->
New in this build: a per-device state machine that watches the VGA DAC frame-lock PLL and mutes / resets the DAC. State
`gstMonitorPllConfig[dev]` (.data+0x10ac, 2 × 11 bytes, initial `00 00 00 00 00 05 0F 01 00 00 00`):

| byte | meaning (debug name) |
|---|---|
| +0 | Aff3Cnt — frames with AFF status 3 |
| +1 | Aff21Cnt — frames with AFF status 1 or 2 |
| +2 | UnLock4Cnt — frames with the sticky unlock bit (1150:17 bit 14) |
| +3 | UnLock0Cnt — frames with neither the lock nor the unlock bit |
| +4 | stable counter — locked frames with AFF status 0 |
| +5 | AFF threshold (5; 0x78 after an AFF mute) |
| +6 | unlock threshold (0x0F; 0x1E after an unlock mute / reset) |
| +7 | DAC un-muted flag (1 ⇔ 145B:15 = 7) |
| +8 | AffRst count ("Aff Rst %d") |
| +9 | UnLockRst count ("UnLock Rst %d") |
| +10 | reset pending ("Clear Rst" at the next IRQ) |

Per VDE↓ interrupt of device d (m = 1 for device 0, 2 for device 1); all writes are direct full-word RIU stores:
1. **Frame lock not active** (1172:3F & m == 0): print "~~ P-END (d) ~~" if any counter is non-zero; if +7 == 0 → 145B:15
   := 7 and +7 := 1; reset +0…+4 := 0, +5 := 5, +6 := 0x0F, +8 := +9 := 0. Done.
2. **Frame lock active**: read AFF = 1150:31 [1:0], LOCK = 1150:17 (bit 15 lock, bit 14 sticky unlock), ERR = 1150:18 /
   1150:19 (printed only). If +10: 145B:01 := 0, 1150:15 := 1, +10 := 0 ("Clear Rst"). Then 1150:17 := 0x4000 (clear the
   sticky unlock), 1150:30 := AFF << 2, then 1150:30 := 0.
3. **AFF ≠ 0 and LOCK & 0xC000 == 0x8000** (locked, AFF error): AFF == 3 → +0++, else +1++; +2 = +3 = +4 := 0. If +7 and
   (+0 > +5 or +1 > +5): mute — +7 := 0, 145B:15 := 0, +5 := 0x78 ("Aff Mute"). Then, with the current +5, if (+0 > +5 or
   +1 > +5): 145B:01 := 1 (synthesizer reset), +10 := 1, +8++, +0 = +1 := 0, +5 := 0x78, +6 := 0x1E ("Aff Rst"). Done.
4. **Otherwise**: if LOCK bit 14 is clear and bit 15 set (locked, AFF 0): +4++; when +4 > 0x78 (≈ 2 s of frames at 60 Hz
   [I]): +0…+4 := 0, +5 := 5, +6 := 0x0F, and if +7 == 0 → 145B:15 := 7, +7 := 1 ("Stable UnMute"); done. If neither bit:
   +3++; if bit 14 is set: +2++. Then: if +7 and (+2 > +6 or +3 > +6): +7 := 0, 145B:15 := 0, +6 := 0x1E, 145B:01 := 1
   then 0 ("UNLOCK Mute"). If +3 > +6: 1150:15 := 0x81 (FPLL enable + FPLL reset), 145B:01 := 1, +10 := 1, +2 = +3 := 0,
   +6 := 0x1E, +9++ ("UnLock Rst"). Finally +0 = +1 = +4 := 0, +5 := 0x78.

The monitor acts only while the frame-lock flag 1172:3F bit m is set, i.e. after an OUTPUTTIMING with interface value 5
(HDMI + VGA together, §7.3 steps 1 / 5). On this camera (HDMI only) step 1 runs at every frame; since +7 starts
at 1 it writes no register [R initial data; I runtime].

## 10. Clocks and module glue [R]

### 10.1 Clocks [R `_DrvDispIfSetClkOn`, `DrvDispOsSetClkOn`, `hal_disp_clk.c` region]
Four DT clocks in this order: mop0, mop1, disp_432, disp_216 (parents: `kernel-platform.md` §5.1), default rates **384,
384, 432, 216 MHz** (.rodata literals 0x16e36000, 0x19bfcc00, 0xcdfe600). At device create: read the current enable and
rate of each through the HAL (`DrvDispIfGetClk` → CLK_GET 0x27), then `DrvDispOsSetClkOn`: requires the DT node to have
exactly 4 parents; for each: `of_clk_get(node, i)`, `clk_round_rate(rate)`, `clk_set_rate(rounded + 1 MHz)`, prepare +
enable if it was off, `clk_put` (the non-`CONFIG_CAM_CLK` branch of the source). If that fails, the HAL path
`DrvDispIfSetClk` (CLK_SET 0x26) programs the clock-gen registers directly. `DrvDispOsSetClkOff`: the same branch (exactly
4 parents, disable + unprepare the enabled ones), as the source. CLK_INIT (0x37, from `DrvDispIfClkOn` at module init)
enables the HAL clock bookkeeping; `HalDispClkInit` first reads Odclk0 / 1 and re-applies the current ODCLK0 / 1
selection (the source forces the device pixel clocks). The 23 HAL clock getters (`HalDispClkGet*`, CLKGEN 1038 and
DISP_GP_CTRL 1137 fields) are feature-equal with the source (they read through an inlined `R2BYTEDirect`). Register
bodies of the 1137 / 1038 setters: §8.2.

### 10.2 Clock GP-control per interface type (`HalDispClkSetGpCtrlCfg`)
Called after every DEVICE_INTERFACE with the `{en, type}` of both devices (`stDispClkGpCtrlCfg`, §3.7); the individual
setters are the source's CLKGEN 1038 / GP_CTRL 1137 functions; the function differs from the source in prints only.
- ODCLK0 select by the device-0 type: HDMI / HDMI_DAC 0, DAC 1, LCD / DSI / none 2, CVBS 4; ODCLK1 by the device-1 type:
  HDMI / HDMI_DAC 0, DAC 1, LCD / DSI / none 3, CVBS 4.
- MIPI pixel / MIPI-TX DSI / DSI APB clocks on if a device is DSI (select 1 for device 1, 0 for device 0).
- LCD source + LCD clocks and FPLL-ODCLK1 on if a device is LCD.
- HDMI clock on if a device is HDMI or HDMI_DAC; TVE clock on for CVBS; DAC + DAC-WAFF clocks and FPLL-ODCLK0 on if a
  device has the VGA bit.
- Display-pixel clock of a device on only for LCD (select 9, or 10 when the other device is DSI) or DSI (select 1).
- More than two layers bound to a device → merged ODCLK (`HalDispClkSetMergeOdClk`).
This camera's HDMI-only case: ODCLK0 0, HDMI clock on, everything else off.

### 10.3 Module init, device, sysfs, DT [R; init path R from `camdriver_init`]
The platform driver `stDrvDispPlatformDriver` (`DrvDispModuleInit` with the probe `DrvDispModuleProbe`) is never
registered: `camdriver_init` calls `_DrvDispModuleInit` directly (step 16, `mhal-core.md` §3.2). `_DrvDispModuleInit`
(the new name of the source's `DrvDispModuleDeviceInit`): char-device region and device "mdisp" (`CamDeviceCreate(class,
…, "mdisp")`) — because `_tDispClass` is 0 at that call the class is taken from `msys_get_sysfs_class()` and the cdev
branch is skipped, so the device "mdisp" and its sysfs exist and no cdev does (`mhal-core.md` §3.2, §4.2) —
`DrvDispSysfsInit`, `of_find_compatible_node("sstar,disp")`, the IRQ slots (§9.1), `DrvDispOsSetDeviceNode`,
`DrvDispIfClkOn`. DT properties read: `interrupts` (6) and `clocks` (4), nothing else (no `of_property_read*` in the
block). The deinit path is `_DrvDispModuleDeInit` (the platform remove `DrvDispModuleRemove` → `_DrvDispModuleDeInit` +
`CamDeviceUnregister` is never registered either); `DrvDispModuleExit` and `DrvDispModuleResume` are as the source (resume
only prints).

sysfs attributes on the "mdisp" device, mode 0644 (`DrvDispSysfsInit`, `dev_attr` array at .data+0x708; handlers
`check_disp*_show` / `check_disp*_store` → `Disp*Show` / `Disp*Store`, the source's `DrvDispSysfsCheck*`, renamed):
**`dbgmg`** (debug-message mask, `u32DispDbgmgLevel`), **`pq`** (PQ bin / PQ debug), **`clk`** (show / set the four
clocks: name, enable, rate), **`turndrv`** (drive tuning: DAC trim, §12.6). No procfs.

OS layer (`drv_disp_os.c`): `DrvDispOsOpenFile` uses `filp_open` under `set_fs(KERNEL_DS)` (not CamFs);
`DrvDispOsParsingTxt` allocates with `CamOsMemAlloc` directly; `DrvDispOsImiHeapAlloc` / `DrvDispOsImiHeapFree` have no
caller (`mhal-core.md` §3.5); `DrvDispOsPadMuxActive` gates the VGA pad write (§8.2).

## 11. MOP — the video layers (banks 1170–1179) [R]

### 11.1 Layout [R `hal_disp_mop.c` functions, feature-equal with the source unless listed in §11.2; field names S from the source comments]
Two MOPs (MOP 0 → device 0, MOP 1 → device 1); each has a MOPG (16 windows, video layer 0 / 2), a MOPS (1 window, layer
1 / 3) and a rotation engine. Banks: MOPG globals 1170 / 1175; MOPG windows 0–7 in 1171 / 1176, 8–15 in 1172 / 1177 (16
registers per window, window offset = (id % 8) × 0x10 registers, byte table .rodata+0x3ad2); MOPS 1173 / 1178; rotation
1174 / 1179. Some registers of 1171 / 1172 / 1177 also serve as software flags (the CMDQ handshake of §5.4, the
frame-lock flag 1172:3F of §7.3, the MCU frame counter 1171:1F of §9.1). The MOPG / MOPS / rotation register setters the
compare lists as DIFF differ only in register-base arithmetic.

| MOPG / MOPS global reg | field |
|---|---|
| 00 bit 0 | software reset (pulsed 1 → 0 by `HalDispMopgSwReset` / `HalDispMopsSwReset`; INTERCFG RSTMOP holds it, §6.3) |
| 01 bits 8 / 9 / 10 / 11 | gated MIU clock / gated GOP clock / auto stretch window (from blanking) / auto blanking |
| 02 bit 0 | frame IRQ flag (MOPG only, §9.1) |
| 03 [4:0] | pipe delay (init 0xA) |
| **04 [7:0]** | **Y (luma) DMA request threshold `ymd_thd`** |
| **05 [7:0]** | **C (chroma) DMA request threshold `cdma_thd`** (source comment: "default=0xf8, use 0xd0 for patch") |
| 06 | luma DMA high priority: [7] enable, [6:0] threshold (init 0xF0) |
| 07, 08 [12:0] | stretch window width, height |
| 0B | chroma DMA high priority (init 0xF0) |
| 0C bit 0 | frame IRQ mask / clear (MOPG) |
| 15 [12:0] | `gw_hext` horizontal extension (init 0x80; `HalDispMopIfSetInfoHextSize`) |
| 40 [10:0] | scaler taps (init 0x689 = "set 4tap") |
| **52** (MOPG) / **53** (MOPS) | **"FF0" DMA FIFO threshold, 16 bit (new in this build; init 0x4040, rotation 0x2020)** |
| 7F bit 8 / bit 9 | double-buffer write strobe (`HalDispMopDbBfWr`: 1 then 0) / force write |

| MOPG window reg (+0x10·w) / MOPS reg | field |
|---|---|
| 00 / 20 bit 0 | window enable |
| 01 / 21 | [12:9] start-address sub-16-byte offset (crop x & 0xf); [8:0] line-buffer start = hstart / 8 + 2·priority (MOPG) |
| 02–03 / 22–23 | Y address >> 4 (16 + 12 bits) |
| 04–05 / 24–25 | C address >> 4 |
| 06, 07, 08, 09 / 26–29 | H start, H end (= start + width − 1), V start, V end (13 bit) |
| 0A / 2A | pitch >> 4 (13 bit) |
| 0B, 0C / 2B, 2C | source width − 1, source height − 1 |
| 0D, 0E / 2D, 2E | H, V scale factor |
| — / 54, 55 | MOPS ring input: [15] enable, [14] source, [12:0] ring-buffer height; start line |

Rotation bank (rot0 / rot1 at +0x00 / +0x20 registers): 01 bit 0 / 1 double-buffer enable; 0A / 51 rotate enables and
modes (MOPS regs); 10 / 11 source width / height; 14–15 read Y, 24–25 read C, 17–18 write Y, 27–28 write C; 16 / 26 pixel
dummy; 12 / 13 / 16 RDMA length / threshold / high priority / stop / burst (init 0x0820, 0xA01F, 0x0800); 29 block number;
2A–2F crop.

Line-buffer order: MOPG ports are kept in an order list (`gpMopgLbOdTbl`; helpers `_HalDispMopIfSetAllMopgLineBufVal`,
`…MopgWinIsUsedForLbstart`, `…MopgLineBufOrder` — as the source except prints). Some MOP registers written in pieces
have software shadows (`u16Mopg01SwShadow*`, `u16Mopg02SwShadow*`, `u16MopsSwShadow*`, §3.8).

### 11.2 Initialisation and the DMA thresholds [R `HalDispMopIfSetInputPortInit`, `HalDispMop{g,s}Init`, `HalDispMop{g,s}CheckSwReset`, `HalDispMopIfGetInfoInputPortRotate`]
At every input-port INIT (first enable of a port): the rotation engine is initialised if 1174:12 reads 0 (never
initialised; `HalDispMopRotCheckInit` = 1174:12 ≠ 0); for a MOPG port the MOPG is initialised **only if its
software-reset bit 1170:00[0] reads 1** (still in reset: `CheckSwReset` returns `~bit0 & 1` and `MopgInit` runs when that
is 0); a MOPS port initialises MOPS and, under the same condition, MOPG. Then the window is disabled, source 1×1, window
0, 0, 1, 1, and the port is inserted into the line-buffer order list. The source (and U-Boot's copy) use static "done
once" flags instead of the hardware check.

`HalDispMopgInit` (0x23e2fe6c) per MOP (direct writes): SwReset pulse; 01 bits 8–11 := 1; 03 := 0xA; **04 := 0x30, 05 := 0x30, 52 :=
0x4040** (`HalDispMopgSetDmaYFifoThrd`, `HalDispMopgSetDmaCFifoThrd`, `HalDispMopgSetFf0DmaFifoThrd`); 06 := 0xF0; 0B :=
0xF0; 15 := 0x80; 40 := 0x689; 7F bit 8 pulse. `HalDispMopsInit` the same on 1173 / 1178 (FF0 at reg 53). The source and
U-Boot write 04 := **0xF8**, 05 := **0xD0** and have no FF0 register.

`InputPortRotate` (query 0x22, GetInfo) rewrites the thresholds whenever it is called: rotation none → 04 / 05 := 0x30,
FF0 := 0x4040; 90 / 270° → 04 := 0xF8, 05 := 0xD0, FF0 := 0x2020 (MOPG: one rotated window per MOP, else an error; MOPS
supports none / 90 / 270). `HalDispMopIfSetInputPortRotate` also clears the rotation crop X / Y on "none" / unsupported.
mi_disp calls `InputPortRotate` only when a rotation mode is set (the per-frame path with the M6 rotate flag,
`SetVideoLayerRotateMode`, `DisableVideoLayer` of a rotated layer) [R mi_disp.c 0x18cb4 callers] — never on this camera's
path.

### 11.3 Frame flip and port attributes [R `HalDispMopIfGetInfoInputPortFlip`, `HalDispMopIfSetInputPortFlip`]
Validation (GetInfo, as the source; `HalDispMopIfGetInfoInputPort{Enable,Attr,Flip,Crop,RingBuffAttr}` read and equal in
logic): the 0x38-byte frame data is stored in the port; every plane address and stride must be 16-byte aligned (else
CFGERR, "PHY_ADDR should be 16 align" / "Stride should be 16 align"); for YUYV and SP422 the strides are halved and plane
1 := plane 0 + 16; port id > 16 → NOT_SUPPORT. SetAttr stores {display window, source w / h}; SetCropAttr requires even
x / y / w / h (else CFGERR; the values are stored anyway) and makes crop w / h the source size; RingBuffAttr with enable and
height 0 → CFGERR.

No rotation: Y address = frame Y + crop.x + stride·crop.y; C address = frame C + crop.x + stride·(crop.y / 2) (NV12);
pitch := stride; the sub-16-byte offset := crop.x & 0xf; when that offset is non-zero the source width is re-set to crop.w
+ offset. Window position / size and source size come from SetAttr / SetCropAttr; the registers are buffered and latched
by the double-buffer strobe in the flip post-act (§5.4). Rotation 90 / 270 routes the frame through rot0 / rot1 into the
IMI buffer (`InputPortSetImiAddr`; warning "Imi address is not set!" if missing; bookkeeping
`_HalDispMopIfSetMopRotImiAddrManage`) — unused.

### 11.4 The boot-time `riu_w` writes to 1170:04 / 05 [R registers; I effect]
The boot-time list (`rootfs-customer.md` §5.2, rows 6–7: `riu_w 1170 04 0056`, `riu_w 1170 05 0056`; bank 0x1170) sets
**the Y and C DMA request thresholds of the MOPG of display device 0 to 0x56**. What the registers hold before: U-Boot's `bootlogo`
initialises the MOPG once at its first input port with the source values **0xF8 / 0xD0** [R U-Boot `HalDispMopgInit` =
MopgInit, called from 0x23e33d1c under a once-flag]; the Linux mhal leaves them alone because U-Boot released the MOP
reset (§11.2), and nothing on this camera's path calls the rotate query — so the thresholds stay at U-Boot's values until
`riu_w` lowers them to 0x56 [I: assumes U-Boot reached its input-port init, i.e. that the boot logo ran on HDMI, which is
unconfirmed; if it did not, mhal's MopgInit writes 0x30 / 0x30 and `riu_w` raises them]. Bank 1175 (device 1) is not
touched; the FAQ variant that also writes 1175:04 / 05 writes the same two registers of device 1 (`rootfs-customer.md`
§5.3). The SigmaStar FAQ ships this sequence against MOP / GOP underflow at 4K30 while VENC loads the DRAM [I].

## 12. Picture path: colour engines, MACE, gamma, DAC trim [R]

### 12.1 Colour engines (OP2) [R `hal_disp_color.c` region]
Each device has two colour engines selected by the **colour id** (0 / 1; `gstInterCfg.u8ColorId`, default 1, set by fbdev,
§6.3): engine 0 at OP2 1151:30 (device 1: 1161:30), engine 1 at 1151:3C. Control reg [3:0] = 0xB (matrix on) / 1
(bypass); nine coefficients at +1…+9 (1151:31–39 / 3D–45), sign-magnitude s13 (bit 12 sign, magnitude clamped to
0xfff); brightness at +0xA (R [7:0], G [15:8]) and +0xB (B) (1151:3A / 3B, 46 / 47), with an enable bit in the control
reg. The effective matrix = colour-correction × contrast-RGB × (YUV → RGB × hue / saturation)
(`_HalDispColorArrayMultiply`, 3×3 s16, 1.0 = 0x400); it is written only when the colour-correction matrix was updated.
**Difference from the source:** the control value is 1 (bypass) when the matrix type is bypass **or** the YUV → RGB matrix
equals identity (`_HalDispColorIsByPassMatrix`, new); the source tests the type only. Of the 18 non-SAME functions of the
region, 13 differ only in register-base arithmetic or debug strings.

YUV → RGB matrices in .data: `tByPassYuv2rgb`, `tHDTVYuv2rgb`, `tSDTVYuv2rgb` (0x12 B each = 9 × s16) and type 3 =
custom (from PQ, §13.3); `HalDispColorSeletYuvToRgbMatrix` selects type 0 bypass, 1 HDTV, 2 SDTV, 3 user. The
colour-correction default is `tVideoColorCorrectionMatrix` (.data+0x106c, 0x40 = 3×3 s16 + padding as declared in the
source); `HalDispColorSetColorCorrectMatrix` copies 9 × s16 to config +0x4e and sets +4 := 1.

The adjusters `HalDispColorAdjustBrightness`, `…VideoContrast`, `…VideoSaturation`, `…VideoCbCr`, `…VideoHue`,
`…VideoRGB`, `…PCContrast`, `…PCRGB`, `HalDispColorAdjustHCS`, `_HalDispColorAdjustPCContrastRGB` and
`_HalDispColorSetVideoContrastMatrix` are equal to the source; each public one returns 1 for a device context and 0 (with
"CtxType is not Device") otherwise. `ENABLE_CBCR` is 1 in this build (the `VideoCbCr` body is present, as the source's
`#define ENABLE_CBCR 1`).

### 12.2 Colour and picture state
- `g_tColorCfg[2][2]` (.bss, 4 × 0x78): per device (0 → +0, 1 → +0xf0) and colour engine (id 0 → +0, id 1 → +0x78) one
  `HalDispColorConfig` (0x76 bytes + 2 padding; `_HalDispColorGetColorConfig` returns NULL for a device id > 1). Layout
  (the source's, offsets confirmed): `tSrgbMatrix` pointer +0, `bColorCorrectMatrixUpdate` +4, `u8ACEConfig` +5,
  `enMatrixType` +8, video R / G / B contrast +0xc / +0xd / +0xe, video contrast +0xf, saturation +0x10, brightness R / G /
  B +0x11…+0x13, Cb +0x14, Cr +0x15, hue +0x16, `sYuvToRGBMatrix[3][3]` +0x18, `sVideoSatHueMatrix` +0x2a,
  `sVideoContrastMatrix` +0x3c (diagonal = RCon·Contrast >> 4 etc.), `sColorCorrrectMatrix` +0x4e, PC R / G / B contrast
  +0x60…+0x62, PC contrast +0x63, `sPCConRGBMatrix` +0x64.
- `gstPicturePqCfg[2]` (.data, 2 × 0x2a, §13.3).
- `stPictureNonLinearSetting` (.data+0x974, 0xa0 = 2 devices × 8 curves × `{u16 OSD_0, OSD_25, OSD_50, OSD_75,
  OSD_100}`: contrast, R / G / B brightness, saturation, hue, sharpness, sharpness 1): the curves that map 0–100 user
  values (`HalDispPictureIfTransNonLinear`, as the source except debug strings). `_HalDispPictureIfUpdateNoneLinearCurveOsd50`
  sets OSD_25 = |OSD50 − OSD_0| / 2, OSD_75 = OSD50 ± |OSD_100 − OSD50| / 2 — equal to the source.

### 12.3 MACE (bank 1152) and gamma (1153 / 1163)
All 98 `HalDispMace*` functions are feature-equal with the source except debug strings and register-base arithmetic (3×3
matrix range / coefficients, DLC / DCR, gamma, BW extension, LPF, H-coring, peaking, FCC). MACE input select = MOP
(`HalDispSetMaceSrcSel(1)` at device init, §6.1).

The eight setters `HalDispMaceSetBwExtensionConfig`, `…LpfConfig`, `…DlcDcrConfig`, `…HCoringConfig`, `…PeakingConfig`,
`…FccConfig`, `…GammaConfig`, `…3x3MatrixConfig` have the source's call order and field offsets
(`hal/mercury6/inc/hal_disp_mace.h` structs `HalDispMace*Config_t`), the same flag OR into the global
`gstHwMaceCtx.enFlag` (BW 0x01, LPF 0x02, HCORING 0x04, PEAKING 0x08, FCC 0x10, DLC_DCR 0x20, GAMMA 0x40, 3X3MATRIX 0x80)
and the same `bUpdate := 0`; `HalDispMaceSetHwCtxFlag(flag, pCtx)` ignores `pCtx` in the source too (one global MACE
context for both devices) [S]. **Only `HalDispMaceSetGammaConfig` has a caller** (`_HalDispIfSetDeviceGammaParam`, with
a stack copy; reached from `DeviceSetGammaParam`, mi_disp's ioctl 0x2e, which the camera's app does not use); the other
seven have no call and no relocation anywhere in mhal.ko — dead code in this build [R `objdump -dr` relocations and
decompile references]. Field offsets confirmed in the binary:
- DlcDcr (u16 each): DcrEn +2, DcrOffset +4, Hactive +6, Vactive +8, Hblank +0xa, PwmDuty +0xc, PwmPeriod +0xe, DlcEn
  +0x10, DlcOffset +0x12, DlcGain +0x14, YGain +0x16, YGainOffset +0x18.
- Gamma: En +1, R[33] +2, G[33] +0x23, B[33] +0x44.
- 3x3: Id +1, bEn +2, RangeR / G / B +3…+5, Coef11…33 (u16) +6…+0x16.
- Peaking (u8): LtiEn 1, LtiCoef 2, LtiStep 3, LtiMedian 4, CtiEn 5, CtiCoef 6, CtiStep 7, CtiMedian 8, DiffAdapEn 9,
  PosLimitTh 0xa, NegLimitTh 0xb, PeakingEn 0xc, Band1 Coef / Step / DiffAdap / Pos / Neg 0xd–0x11, Band2 0x12–0x16,
  CoringTh1 / 2 0x17 / 0x18, Term1–4Sel 0x19–0x1c.
- Fcc: Cb / Cr_T1…T8 1–0x10, CbCr_D1…D8 0x11–0x18, D9 0x19, K_T2T1…K_T8T7 0x1a–0x1d, En 0x1e.
- HCoring: bytes 1–0xe in source order; BwExt: bytes 1–7; Lpf: Y 1, C 2.

MACE context `gstHwMaceCtx` (.bss, 0xf4) [R size and field use; S layout]: the source's `HalDispMaceHwContext_t`, the same
size: +0x00 `enFlag` (the only member any code touches — the setters OR their flag into it, nothing reads it), +0x04
`stBwExtCfg` (8), +0x0c `stLpfCfg` (3), +0x10 `stDlcDcrCfg` (0x1a), +0x2a `stHCoringCfg` (0xf), +0x39 `stPeakingCfg`
(0x1d), +0x56 `stFccCfg` (0x1f), +0x75 `stGammaCfg` (0x65), +0xda `st3x3MatrixCfg` (0x18). The sub-structs are never
filled: the setters take the caller's struct, not the context's.

### 12.4 Frame colour, window colour, test pattern [R `hal_disp_op2.c`]
`HalDispSetFrameColor(R, G, B)` → 1151:1B / 1C; `HalDispSetFrameColorForce` 1151:1C; `HalDispSetDispWinColor` 1151:21 /
22; `HalDispSetDispWinColorForce` 1151:22 (device enable, §6.2); pattern-generator mode 1150:07 (`HalDispSetPatGenMd`,
mode kept in `u8PtGenMode`, and INTERCFG DISPPAT, §6.3).

### 12.5 Picture config and peaking
`HalDispPictureIfSetConfig`: device 1's sharpness goes through the 116F peaking block, device 0's through MACE (the
source: by colour id, device 1 "ToDo"). `HalDispHpqSetPeakingCoef(ctx, band1, band2)` (new) writes 116F:18–1B ([5:0] band
1, [13:8] band 2) — bank 116F is the display PQ peaking block [I]. The PQ side of the picture config: §13.3.

### 12.6 DAC trim and "drive tuning" (VGA DAC, unused on this camera) [R `HalDispGetDacTriming`, `HalDispSetDacTrimming`, `__HalDispIfGetDacTrim`, `__HalDispIfSetDacTrim`, `_HalDispIfSetDrvTurningSet`, `_HalDispIfSetDeviceVgaParam`]
The factory trim of the three DAC channels is read once at device init into `gau16VgaDacTrimDefault[3]` (from the DAC
trim registers 145B:1A (B), 1B (G), 1C (R), bits [6:0], through `HalDispGetDacTriming`). The VGA param set (0x08) and
DRVTURNING_SET (0x2a, sysfs `turndrv`) apply `__HalDispIfSetDacTrim(trim[3], delta)`: each 7-bit sign-magnitude trim (bit
6 sign) plus delta, clamped to [−0x40, +0x3f], re-encoded, then `HalDispSetDacTrimming(r, g, b)` as direct no-shadow
writes (the source: buffered writes). DRVTURNING_GET (0x2b) returns the current trim; state `gstVgaDacTrimCfg` (0xc).

## 13. PQ (picture-quality bin) [R `hal_disp_hpq*` + `disp_hpq/*` regions, `_HalDispIf{GetInfo,Set}PqSet`]
The `PQ.bin` file format (header, IP / table index, group / rule tables, text bin) is the OpenIPC `drv_disp_hpq_bin.c`
format; it and this camera's `/misc/PQ.bin` are `uboot.md` §6.2–§6.3; this section is mhal's loader. The PQ region
(018988–019dd0) has 44 functions feature-equal with the source, 22 that differ and 17 new.

### 13.1 Hand-over (`DeviceSetPqConfig`, query 0x28)
`DeviceSetPqConfig(ctx, {u32 flags, u32 size, void *data})`; with ctx = NULL it runs without a context for flags 1, 2, 4
(`DrvDispIfDeviceSetPqConfig`). Flags in this build:
- **1 LOAD_BIN** (data = PQ bin image in memory, size > 0) → `HalDispHpqLoadBin`: `MDrv_PQ_Init({1 bin, buffer, size})` →
  bin header parse (`MDrv_PQBin_Parsing`, moved into `pq_bin_parse` (0x400195b0); equal in logic to the source) + common-table load
  (`MDrv_PQBin_LoadCommTable`), then `MDrv_PQ_InitSourceTypeTable` (new) = read the bin's source LUT (6-byte entries)
  into a heap array (`gstPqSrcTypeCfg` {count, array}). `MDrv_PQ_Init` has no display-type / panel-width set-up.
- **2 SET_SRC_ID** (data → u32 = source id | panel id << 16) → `HalDispHpqSetSrcId`, `HalDispHpqSetPnlId` (if < the
  panel count), picture data from the bin → `HalDispPictureIfSetPqConfig(panel id, …)`.
- **3 PROCESS** (data → u32 ≠ 0) → `MDrv_PQ_LoadSettings(0)` (source id + panel index) + `HalDispPictureIfApplyPqConfig`.
- **4 LOAD_SETTING type** (data → u32 ≠ 0 = manual; `gePqLoadSettingType`).
The source's flags differ (FREE_BIN etc.; its Set always loads the bin and updates both devices). mi_disp calls it once
at init with flag 1 and the table `MI_SYSCFG_GetPqTbl` returns (`mi_disp-mi_hdmi.md` §4.3) — on this camera there is no
PQConfig.ini / PQ table (`config_tool.md` §6), so nothing is loaded, and the PQ step of OUTPUTTIMING (§7.3 step 7) finds no
source LUT (`MDrv_PQ_GetMatchSoruceType` fails) [R code; I runtime].

### 13.2 Automatic source selection at each timing change (new)
Key and LUT entry: two readings that no check has settled:
- [R mhal, field roles I] the entry is `{u8 interface code, u8, u16, u16}` [I: interface + output size]; the key =
  {interface code (`_HalDispHpqTransInterface`: HDMI 1, VGA 2, CVBS 3, BT1120 4, BT656 5, BT601 6, TTL 7, DSI 8, MCU 9,
  SRGB 10), …} is matched against the LUT — exact match first, then by interface for codes 7–10, or by interface + size
  with flag 0.
- [R U-Boot's copy of the same code] entry and key are **{u8 device flag (device == 1), u8 interface code, u16 width,
  u16 height}**, and the "codes 7–10" rule tests byte 0, so it never fires; mhal's byte order is the same code [I]
  (`MDrv_PQ_GetMatchSoruceType` and its key builder in mhal not re-read against this).
A hit → source id, picture data, apply, load settings — unless the load type is manual (flag 4 set). New functions of this
path: `MDrv_PQ_GetMatchSoruceType`, `MDrv_PQ_SetSrcType`, `MDrv_PQ_GetPnlNum`, `MDrv_PQ_SetPnlId`,
`MDrv_PQ_InitSourceTypeTable`, `MDrv_PQBin_GetSourceLut`, `MDrv_PQBin_GetSourceLutPnlNum`,
`_MDrv_PQ_Clear_SRAM_Table_Index`, `HalDispHpqGetSourceIdx`.

### 13.3 Picture data (`__HalDispIfGetPictureDataFromPq`, new)
A 0x20-byte PQ "picture" item = 11 bytes {video gain R, G, B; brightness R, G, B; hue, saturation, contrast; peaking band
1, band 2} + 9 × u16 sign-magnitude (bit 12) YUV → RGB coefficients → `gstPicturePqCfg[dev]` (0x2a B: u8 bUpdate, 11 ×
u16, 9 × s16). `HalDispPictureIfApplyPqConfig`: YUV → RGB matrix type 3 with those coefficients, video RGB gains, HCS,
brightness; device 0 also the peaking band 1 / 2 coefficients (MACE). `HalDispPictureIfSetPqConfig` also rebuilds the
0–100 non-linear curves (OSD-50 points, §12.2) from the PQ values. `HalDispPictureIfIsPqUpdate` also differs from the source;
it reads the "updated" flag that DEVICE_INIT tests (§6.1) [I].

### 13.4 PQ-bin readers
`MDrv_PQBin_TransToPQWin`, `MDrv_PQBin_GetTable`, `MDrv_PQBin_LoadTableBySrcType`, `MDrv_PQBin_GetGRule_LevelIndex`,
`MDrv_PQBin_GetGRule_TableIndex`, `MDrv_PQBin_DumpFilterTable`, `MDrv_PQTextBin_GetSrcType`, `MDrv_PQTextBin_GetIPName`,
`MDrv_PQTextBin_GetTableName`, `MDrv_PQ_LoadTable`: equal to the source with `PQ_ENABLE_PIP`
and `ENABLE_MULTI_SCALER` off and `SCL_ASSERT` compiled out (out-of-range indices return 0xFFFF or clamp to 0 exactly as
the source, without the assert). `MDrv_PQBin_CheckCommTable` and `MDrv_PQBin_CheckTableBySrcType` are empty.
`MDrv_PQTextBin_Parsing` differs from the source by the feature compare; what differs is not recorded. Offsets used, all
as the source defines:
- `MS_PQBin_Header_Info` (0xd0): `u8BinID` +0, `u32BinStartAddress` +4, `u16GRule_RuleNum` +0x50, `u16GRule_PnlNum`
  +0x52, `u32GRule_Offset` +0x54, `u32GRule_Pitch` +0x58, `u32GRule_Lvl_Offset` +0x5c, `u32GRule_Lvl_Pitch` +0x60,
  `u16IPNum` +0xaa, `u32IP_Comm_Offset` +0xac, `u32IP_Comm_Pitch` +0xb0. `MS_PQTextBin_Header_Info`: `u16SrcNum` +0x50,
  `u32SrcType_Offset` +0x54, pitch +0x58, `u16IPNum` +0x6c, `u32IPName_Offset` +0x70, pitch +0x74, `u16GroupIPNum`
  +0x88, `u32IP_Group_Offset` +0x8c, pitch +0x90.
- `MS_PQBin_IP_Table_Info` (filled by `GetTable`): `u32TabOffset` +0, `u16RegNum` +4, `u16TableType` +6, `u16GroupNum` +8,
  `u16GroupIdx` +0xa, `ePQWin` +0xc. IP-common record: table type +0, common reg num +2, IP reg num +4, group num +6,
  common offset +8, IP offset +0xc; table offsets are relative to bin start + 16 (`PQ_BIN_HEADER_LEN`). Table index
  0xFFFF = NULL (offset 0xFFFFFFFF, all counts 0), 0xFFFE = COMM (group num 1, idx 0). The table-index cache is
  `u16[win][30][256]` (`PQ_BIN_MAX_PNL` 30, `PQ_BIN_MAX_IP` 256; the unnamed .bss block of §3.8).
- G-rule record (pitch × rule): IP num +0, source num +2, group num +4, rule offset +0xa; entry = rule data + 16 +
  2·(IPNum·(GroupNum·Src + NR) + IP). Level record: level num +0, level offset +2; level idx at level data + 16 + 2·lvl.
- `MDrv_PQ_LoadTable`: main window only — `MDrv_PQBin_LoadTable(panel idx + display type, …)` when the PQ bin is enabled;
  window 1 (sub) returns; any other window returns silently (the source: a DSPY window print, else assert).
- `TransToPQWin` always returns 0 (main window) — with PIP and multi-scaler off the source does the same.
- `DumpFilterTable`: the source's switch; the SRAM1–4, C_SRAM1–4, colour-index and colour-gain SNR / DNR handlers (table
  types 53–63) compile to nothing because their bodies are empty / `#if 0` in this build too; type 64 (`VIP_IHC_CRD_SRAM`)
  → the IHC writer inlined; type 65 (`VIP_ICC_CRD_SRAM`) → `Hal_PQ_set_sram_icc_crd_table(win, 0xb, data)`.
- `MDrv_PQBin_DumpGeneralRegTable` / `MDrv_PQBin_DumpScalerRegTable`: rows `{bank, addr, mask, value[group]}` → a byte
  write to RIU `0x11<bank>:<addr>` directly (base `PQ_RIU_BASE`).

### 13.5 SRAM writers (live in this build; `#if 0` in the source tree)
- **ICC** CRD SRAM (`Hal_PQ_set_sram_icc_crd_table`; 32 × 32 = 0x400 entries; type id must be 0xb; window 0 only):
  1169:78 bit 0 := 1 (port I/O disable); per entry 1169:79 [9:0] := index, 1169:7A [8:0] := data (byte 0 + bit 0 of byte
  1 · 0x100), 1169:7A bit 15 := 1 (write strobe); then 1169:78 bit 0 := 0.
- **IHC** CRD SRAM (`Hal_PQ_set_sram_ihc_crd_table`; type id must be 0xc, else "Type(%d) is not correct" — the source
  returns silently): the 0x441 (33 × 33) u16 entries are split into `PQ_SRAM1_IHC` (0x121), `PQ_SRAM2_IHC` (0x110),
  `PQ_SRAM3_IHC` (0x110), `PQ_SRAM4_IHC` (0x100), each written by `_Hal_PQ_set_sram_ihc_crd_table(win, buf, sram 0–3,
  n)`: 1169:7C bit 0 := 1, 1169:7C [2:1] := SRAM index (> 3 → 0); per entry 1169:7D [8:0] := index, 1169:7E [8:0] :=
  value, 1169:7E bit 15 := 1; then 1169:7C bit 0 := 0; window ≠ 0 → "PQWin:%d is not correct".
The source's dead copy names the registers `REG_VIP_ACE2_7C` / `7D` / `7E` in SCL bank "BK18" [S]; on M6 the bank is 1169.

## 14. Write-back DMA (bank 1158) — unused on this camera [R `hal_disp_dma*.c` regions; mi_disp ioctls 4 / 5 attach / detach]
One DMA engine (`HAL_DISP_DMA_MAX` 1) captures device 0 or 1 — the source device, at the "front" (before) or "back" (after
the picture path) tap — into DRAM or IMI, in frame or ring mode, to mirror one device onto the other (e.g. HDMI → CVBS).
It runs on **CMDQ context 2** (`HalDispGetCmdqIdByDmaId`: DMA 0 → 2) or, through `_HalDispDmaGetCmdqCtx`, on the CMDQ
context of its source or destination device. Of the `hal_disp_dma*.c` functions 56 are feature-equal with the source, 6
differ and 11 are new (including the DMA CMDQ lookups). Differences:
- Bind (0x32): after the clock / FPLL mux set-up also `HalDispDmaSetFrmWDbEn(1)` and `HalDispDmaClearIrqWriteDone`
  (1150:02 := 4 / 1160:02, write-1-clear) — moved here from SetBufferAttr.
- Frame-write config: new `HalDispDmaSetR2Y(input type == BACK)` (1158:5F bit 0, RGB → YUV on the back tap); MIU mux, VEN
  mode, CVBS destination, MOP mux and ring enable as the source.
- FPLL for HDMI → CVBS ring mirroring: as the source — HDMI synthesizer from `u32FpllHdmiSynthSet[source timing]`
  (.data+0xa68, 39 u32 indexed by HAL timing: 720P50 and 1080P50 0x2e8ba2, 720P60 and 1080P60 0x2e978d, 576P50 / 480P60
  0x400000, the VGA modes individual (0x3539ce 1024x768, 0x200ae3 1280x1024, 0x2851cd 1366x768, 0x206c16 1440x900,
  0x294cb5 1280x800, 0x17a2e8 1680x1050, 0x2ab596 1600x1200), all others (1080P24–30, the 1000 / 1001 rates, 1440p, 2160p,
  …) 0 = unsupported); TVENC synthesizer 0x400000, gain 0x88, threshold 0x90, absolute limit 0x10000, enable. New: wait for
  lock (`HalDispDmaWaitFpllLock`: CMDQ poll 1160:17 bit 15 + wait-done + 1150:40 / 1160:40 := 0xffff) and, when the source
  is HDMI + VGA, a DAC-AFF reset pulse (`HalDispDmaSetDacAffRst`: 1150:30 bit 0, 10-unit delay). `HalDispDmaSetHdmiSynthSet`
  queues its 1150:10 [15:0] / 1150:11 [7:0] writes on the **destination** device's CMDQ context (`_HalDispDmaGetCmdqCtx(…,
  1 = DEST)`); the source uses the source device's.
- New helpers: `HalDispDmaCheckFrmWEn` (1158:01 bit 15), `HalDispDmaPollWriteDone` (CMDQ poll 1150:02 bit 2, 2 s),
  `HalDispDmaPollVWriteCnt` (CMDQ poll 1158:4D == n), the DMA register flip (§5.4); `__HalDispIfDmaWaitDone` is empty.
- DMA IRQ: slots 2 / 3, bits 3 (active on) / 2 (active off) of 1150:02 / 03 (§9.1).
Register bodies of the CVBS-destination and FPLL setters: §8.2. Container: §3.6.

## 15. What is not known

- **MOP thresholds before `riu_w`.** Whether 1170:00 bit 0 is clear and 1170:04 / 05 hold U-Boot's 0xF8 / 0xD0 when the
  boot-time list runs — i.e. whether U-Boot's `bootlogo` reached its input-port init on HDMI (§11.4) — is not known; only
  the running camera, stopped before `demo.sh`, shows it (CAMERA-CHECKS P11). Whether the HDMI boot logo is visible and
  what MOP / GOP state U-Boot leaves behind is likewise open (CAMERA-CHECKS K37).
- **Direction of the thresholds.** Whether a lower or a higher value of 1170:04 / 05 makes the MOP fetch earlier is stated
  nowhere at hand; the source names the fields only `ymd_thd` / `cdma_thd`.
- **CMDQ handshake in use.** Whether the flags 1172:0F bit 8 and 1172:1F [15:8] (§5.4) change while streaming is not known
  (CAMERA-CHECKS K30).
- **PQ at run time.** That no PQ bin is loaded follows from the code and the missing PQConfig.ini [I]; what sysfs `pq` on
  "mdisp" (`/sys/…/mdisp/pq`) reports is not known (CAMERA-CHECKS K31). The byte order of the PQ source-type entry and key in mhal (§13.2) is
  not settled: mhal's `MDrv_PQ_GetMatchSoruceType` and its key builder were not re-read against U-Boot's reading.
- **Meaning of compiled-in values.** That the 24-bit value in 1150:10–11 is the HDMI pixel-clock synthesizer ratio, that
  the HDMI-TX ATOP per-timing columns step drive strength / pre-emphasis by TMDS rate, that bank 116F is the display PQ
  peaking block, and that `HalDispSetDacInit` keeps the DAC powered down are inferences (§7.4, §12.5, §6.1). Whether
  DISP's ATOP table is what drives the HDMI PHY on the running camera (live 145A:22–25 / 32 under the vendor stack) is not
  known (CAMERA-CHECKS K32).
- **Scaling of the zoomed picture.** Whether DISP (input port fixed at 3840×2160) scales a zoomed 4K picture on HDMI or
  shows the window smaller is not known (CAMERA-CHECKS K36).
- **Coverage limits.** The SAME functions off the CVBS / VGA / LCD / MCU / DSI paths (HDMI timing, MOP, colour, PQ) were
  compared by features and by the write-mask check only; their computed (non-constant) values were not re-read (§1.2).
- **Objects known by name only:** `gbMopgInitNum`, `_bOSD_On` (§3.8); the members of `gstHwMaceCtx` other than `enFlag`
  have the source's layout but no code touches them (§12.3).
- **USER timings on HDMI.** A USER timing gets the invalid table id 0x1b, so the synthesizer and ATOP writers write
  column 0x1b past the row; what the hardware then outputs is not known [I] (§7.1, §7.4).
