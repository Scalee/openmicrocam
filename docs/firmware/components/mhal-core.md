# mhal-core — mhal.ko as a module
<!-- ev:mhal-core -->

`mhal.ko` is SigmaStar's media HAL for the Mercury6 (M6) chip: about 25 vendor HAL libraries linked into one kernel
module. This document covers the module as a whole — its blocks and their address ranges, which kernel module calls which
block, the calls between blocks, the register banks each block touches, the public sources that match it, the module entry
and init order, the ISR-callback registry (ISRCB), the IMI heap, the character devices, the module parameters, the files
it loads, the device-tree properties it reads — and the one block no other document owns, DIVP. Each block's own
interface and register programming is in the block's document (named per block in §2.1).

Tags: **[R]** read from the binary, a decompile, the DT or a source file; **[S]** stated by a source; **[I]** inference.
A heading tagged [R] means every statement under it is [R] unless marked otherwise.

Binary: `/config/modules/4.9.227/mhal.ko` (`/config/modules/4.9.227/mhal.ko` on the camera). Ghidra addresses in this
document are `.text` offset + 0x40000028 (image rebased to 0x40000000).

## 1. The module [R]

- File: 2.5 MB, `.text` 992 840 B, 7746 functions, full ELF symbol table, no debug info. Modinfo `description=MHal
  author=SigmaStar license=GPL`, `depends=` empty; vermagic `4.9.227 SMP preempt mod_unload ARMv7 thumb2 p2v8`; build
  string `build.202207212233` (inside `mhal_module_version`, §5).
- Exports: 988 (from the `__ksymtab` relocations). Two of them are data, not functions: `g_tIspMidHandler`,
  `gbDbgMessageLv`.
- Undefined symbols: 236, all resolved by kernel built-ins (kernel-platform.md §16.2, kernel-drivers.md) — the CamOs shim (74
  `CamOs*` functions, 7842 call sites spread over every block), the `Cam*` wrappers (clock, OF, I2C, file, sysfs),
  `msys_get_sysfs_class`, padmux (`mdrv_padmux_*`), `camdriver_gpio_*`, `Chip_MIU_to_Phys`, `set/get_core_voltage`, and
  stock kernel API (cdev / misc devices, IRQs, procfs, `filp_open` / `vfs_read`, `ioremap`, `i2c_transfer`,
  `kernel_neon_begin`).
- Layout: the libraries sit in `.text` one after another, so every block is one or a few contiguous address ranges. Block
  boundaries are where the function-name family changes. Code = sum of symbol sizes, 955 358 B; the other ≈37 KB
  of `.text` is padding and literal pools not covered by symbols.
- Other builds: 20 `mhal.ko` files of other M6 products (SIYI A8 mini, RunCam Split-H,
  Anjoy MC-H12S and others) — 10 distinct builds, none with debug info.

Three clusters of blocks call each other heavily and are independent of each other [I from §2.3]: **capture** (SNR, CSI,
VIF, ISP, with ISPSCL and MLOAD), **codec** (one Chips&Media Wave5 VPU library, VCODEC, under both VENC — H.264 "MFE",
H.265 "MHE" — and VDEC; firmware `chagall.bin`; JPE is a separate MJPEG engine) and **display** (DISP, HDMITX, RGN — the
GOP part for fbdev).

## 2. Blocks

### 2.1 Block table [R]

"Used" = called by a kernel module or device the product runs (the module set is in firmware-image.md §3.3). Code = sum of the
block's symbol sizes. "Document" = where the block's interface and programming are specified.

| block | what | code KB | funcs | exports | used | document |
|---|---|---|---|---|---|---|
| ISP | ISP hal/driver, in-kernel 3A, IQ engine, IQ API, ispmid | 301.5 | 2199 | 234 | yes (VIF→ISP realtime) | isp-1-hal.md, isp-2-ispmid-mload-ispscl.md, isp-3-iq-api.md, isp-4-iq-engine.md, isp-5-3a.md |
| DISP | display engine: timing, MOP layers, PQ/MACE, DMA, clocks, DAC | 106.8 | 1082 | 80 | yes (HDMI out) | mhal-disp.md |
| VCODEC | Chips&Media Wave5 library + vcodec device, clocks, IRQ, SRAM configuration, MIU protect, firmware load and version check | 73.9 | 330 | 0 | yes (VENC, VDEC) | mhal-vcodec.md |
| SCL | VPE scaler (+ `HalUtility` register / CMDQ access) | 56.1 | 340 | 50 | yes (SCL→DISP, →VENC) | mhal-scl-jpe-cmdq.md |
| HDMITX | MStar Utopia HDMITx api/drv + hal, HDCP, EDID | 46.3 | 653 | 102 | yes | mhal-hdmitx.md |
| VIF | video input (sensor pads, MCLK, VIF→ISP, VIF WDMA) | 44.2 | 465 | 65 | yes | mhal-vif.md |
| GE | Utopia GFX api + GE 2D driver | 35.4 | 403 | 80 | no (mi_gfx, rotation only) | gfx-ge.md |
| VENC | `MHAL_VENC` wrapper + `MHAL_MFE` / `MHAL_MHE` over the VPU | 33.5 | 57 | 33 | yes (H.264 / H.265 UVC) | mhal-vcodec.md |
| JPD | NJPD hardware driver + software JPEG helpers | 32.6 | 273 | 15 | photo viewer only | mhal-jpd.md |
| JPE | JPEG encoder: device / context, hardware ops, rate control | 30.5 | 143 | 16 | yes (MJPEG UVC) | mhal-scl-jpe-cmdq.md |
| CMDQ | 8 command-queue engines + a hash-table helper | 25.2 | 262 | 21 | yes (every block user) | mhal-scl-jpe-cmdq.md |
| PNL | LCD / MIPI-DSI panel timing; the DSI controller (1A38 / 1A39) is here | 24.1 | 263 | 23 | no (LCD only) | panel-mipitx.md |
| AUDIO | "bach" audio codec, I2S, DMA | 21.8 | 178 | 34 | no | audio.md |
| SNR | sensorif (sensor-driver ABI, I2C, VCM, skip-frame) + sensor early-init stub (0.6 KB) | 20.0 | 175 | 66 | yes | mhal-csi-snr.md |
| RGN | GOP / OSD / cover | 17.1 | 181 | 42 | yes (fbdev GOP) | rgn-gop.md |
| CSI | MIPI CSI-2 receiver (mac / dig / ana) | 17.1 | 220 | 0 | yes (via SNR) | mhal-csi-snr.md |
| VDEC | `MHAL_VDEC` wrapper over the VPU | 15.8 | 30 | 28 | MP4 player only | mhal-vcodec.md |
| LDC | lens distortion correction | 15.0 | 89 | 14 | no | ldc-1-stack.md |
| DIVP | DIP engine under the Infinity2 `MHAL_DIVP` API (no `mi_divp` on the camera) | 11.3 | 51 | 14 | no | §8 here |
| ISPSCL | ISP→SCL / YUV2Bayer path | 8.4 | 84 | 8 | internal (ISP, VIF) | isp-2-ispmid-mload-ispscl.md |
| MIPITX | MIPI CSI-2 transmitter only | 7.9 | 101 | 12 | no | panel-mipitx.md |
| MLOAD | menuload: DMA of register tables into the ISP; the SCL half is stubbed in this build (every `SCL_*` function returns −1, the SCL table range is empty) | 7.0 | 98 | 19 | internal (ISP) | isp-2-ispmid-mload-ispscl.md |
| IPU | IPU / DLA power, clock, firmware boot (address windows, reset vector, core start), shutdown, IRQ clearing | 1.6 | 28 | 23 | no | ipu-host.md |
| INIT | `module_camdriver.c`: init order, IMI heap, ISR-callback registry | 1.0 | 12 | 3 | yes | §3 here |
| (module) | `.init` / `.exit` / `.text.unlikely` | 1.4 | 29 | 4 | — | §3 here |

Notes to the table:
- LDC: Ghidra finds 90 functions in the block range — the 89 symbols plus one unnamed sysfs helper. `MHAL_LDC_GetDevConfig`
  id 1 returns the CMDQ size 0x38400.
- RGN: 181 symbols + 9 static functions = 190 decompiled functions.
- JPE: the MJPEG engine at bank 1620; its hardware ops include `_EncFire` and `_IsrFunc`.
- AUDIO: one of the 34 exports, `MHAL_AUDIO_DmaRdSwap`, is not in the published `mhal_audio.h`.
- IPU: the IPU firmware is loaded by libmi_ipu on the first `MI_IPU_CreateDevice`, not by a kernel module or boot script
  (ipu-host.md §1); on this camera nothing loads it.
- The INIT address range also holds the IPU helpers `MDRV_IPU_*` (0x424b0–0x42ae0), which belong to block IPU.
- Blocks with no consumer in the product: AUDIO, LDC, MIPITX, IPU, DIVP, PNL (LCD only), GE (mi_gfx rotation only), the
  sensor early-init stub.

### 2.2 Sub-parts [R]

- ISP: hal/driver (registers, DMA, statistics) 71.4 KB / 788 functions; **in-kernel 3A** (`DoAe`, `DoAwb`, `DoAf`,
  `pAE_*` …) 55.2 / 299; IQ engine (`DrvAlgo_IF_*`, `IQ_API20_SET_*`, `IQ_Patch_*`, `IspRegSet*`) 88.5 / 400; IQ user API
  (`MI_ISP_*`, `call_MI_ISP_*`, `isp_api_iq.c`) 23.8 / 313; ispmid (frame flow, CMDQ, the `isp_fe` / `ispmid` devices)
  62.6 / 399.
- Codec: Wave5 library (`Product*`, `VPU_*`, `Wave5*`) 42.5 KB / 166 symbols (164 functions in the decompile);
  vcodec device / proc / ramlog / MMA 31.5 / 164.

The picture's 3A is the user-space one (libcus3a + libispalgo, `nFlagCus3A = 0x1A`; isp-6-userspace.md §1.1, isp-5-3a.md §1.5),
reached through `/dev/isp_fe`; the in-kernel 3A sub-part is specified in isp-5-3a.md. YUV2Bayer (ISPSCL) is fully off for
this camera's sensor (isp-2-ispmid-mload-ispscl.md §13.9).

### 2.3 Calls between blocks [R]

Direct calls across block boundaries (count = call sites):

| from → to | sites | what |
|---|---|---|
| SNR → VIF | 50 | pad / channel / group mapping, input mask, MCLK |
| ISP → SNR | 40 | sensor AE notify, shutter / gain limits, FPS, I2C read / write |
| VDEC → VCODEC, VENC → VCODEC | 109, 104 | the Wave5 `VPU_*` API, device handles, timing |
| ISP → INIT, VIF → INIT, ISPSCL → INIT, SNR → INIT | 33, 18, 16, 7 | ISR-callback registry (`ISRCB_*`, §3.4) |
| ISP → MLOAD, ISP → CMDQ, ISP → VIF, ISP → ISPSCL | 19, 17, 16, 7 | register tables, command queues, VIF channel info, YUV pipe |
| VIF → ISPSCL, SNR → CSI | 7, 3 | YUV2Bayer config; CSI clock / lane count / HDR mode |
| VENC → JPE, SCL → LDC / MIPITX, JPE → VCODEC, DISP → INIT | 35, 11, 6, 2 | shared helpers only (clip math, string / clock utilities, IMI heap) |

DISP, SCL and RGN make no direct CMDQ calls: they use the CMDQ function table handed in from the MI side (§2.4). DISP and
SCL also have a direct register path (`HalUtilityW2BYTEMSKDirect`).

### 2.4 Consumers — what each module imports, by block [R]

Undefined symbols of each `.ko` in the dump that mhal exports . The product's
modules first.

- `mi_sys` — **CMDQ** (8): MHAL_CMDQ_{DeinitCmdqMmapInfo, GetCmdqHWNumber, GetFreeId, GetSpecificId, GetSysCmdqService, InitCmdqMmapInfo, ReleaseId, ReleaseSysCmdqService}
- `mi_sensor` — **SNR** (16): MHal_SNR_{CustFunction, GetAnadecSrcAttr, GetCurRes, GetFPS, GetInfo, GetOrien, GetPlaneInfo, GetRes, QueryResCount, SensorDisable, SensorEnable, SetAnadecSrcAttr, SetFPS, SetOrien, SetPlaneMode, SetRes}
- `mi_vif` — **VIF** (22): MHal_VIF_{Deinit, DequeueFrameBuffer, DevDisable, DevEnable, DevReset, DevSetConfig, GetInputMask, GetVIFDev2SensorPadMux, GroupDisable, GroupEnable, GroupReset, GroupSetConfig, Init, InputMask, PipeSetConfig, PortSetConfig, PortStatQuery, QueryFrames, QueueFrameBuffer, RegisterEventCallback, RegisterGetPtsCallback, SetVIFDev2SensorPadMux}
- `mi_isp` — **ISP** (17): MHalIspApiGet, MHalIspApiSet, MHalVpeCreateIspInstance, MHalVpeDestroyIspInstance, MHalVpeIspCheckIrq, MHalVpeIspClearIrq, MHalVpeIspConfigAckTie1, MHalVpeIspDeInit, MHalVpeIspEnableIrq, MHalVpeIspGetIrqNum, MHalVpeIspInit, MHalVpeIspInputClose, MHalVpeIspInputConfig, MHalVpeIspNextFramePTS, MHalVpeIspProcess, MHalVpeIspRotationConfig, MHalVpeIspSnrEarlyInitParam; **SNR** (2): MHal_SNR_{GetSwitchSensorId, SwitchSensor}; **VIF** (1): MHal_VIF_InputMask
- `mi_scl` — **SCL** (21): MHalVpeCreateSclInstance, MHalVpeDestroySclInstance, MHalVpeSclCheckIrq, MHalVpeSclClearIrq, MHalVpeSclDeInit, MHalVpeSclDmaPortConfig, MHalVpeSclEnableIrq, MHalVpeSclGetActiveBuf, MHalVpeSclGetIrqNum, MHalVpeSclGetQpHistogram, MHalVpeSclInit, MHalVpeSclInstanceAttach, MHalVpeSclInstanceDetach, MHalVpeSclPortMirrorFlipConfig, MHalVpeSclPortSizeConfig, MHalVpeSclPreCropConfig, MHalVpeSclProcess, MHalVpeSclReset, MHalVpeSclRotationConfig, MHalVpeSclSendSwMessage, MHalVpeSclSetWaitDone
- `mi_rgn` — **RGN** (24): MHAL_RGN_{CoverDeinit, CoverDisable, CoverEnable, CoverInit, CoverSetColor, CoverSetWindow, GetChipCapability, GetGopBindOsd, GopDeinitByType, GopGwinDisable, GopGwinEnable, GopGwinSetBuffer, GopGwinSetBufferXoffset, GopGwinSetPixelFormat, GopGwinSetWindow, GopInitByType, GopSetAlphaType, GopSetAlphaZeroOpaque, GopSetArgb1555AlphaVal, GopSetBaseWindow, GopSetColorkey, GopSetPalette, SetGopBindOsd, SetupCmdQ} — none is reached in the product (mi-capture.md §6.12)
- `mi_gfx` — **GE** (33 = 26 `MApi_GFX_*` + 7 GE calls): MAPI_GE_GetIrqNum, MApi_GE_{ClearInterrupt, DisableInterrupt, EnableInterrupt, Exit, SupportInterrupt, TagIsFinish}, MApi_GFX_{BeginDraw, BitBlt, DrawLine, EnableAlphaBlending, EnableDFBBlending, EnableVCmdQueue, EndDraw, Init, RectFill, SetAlphaThr_ARGB1555, SetAlpha_ARGB1555, SetClip, SetDFBBldConstColor, SetDFBBldFlags, SetDFBBldOP, SetDstBufferInfo, SetDstColorKey, SetMirror, SetNextTAGID, SetPaletteOpt, SetRotate, SetScallingCoeff, SetSrcBufferInfo, SetSrcColorKey, SetVCmdBuffer, WaitForTAGID} — rotation only, unused in the product
- `mi_venc` — **VENC** (30): MHAL_MFE_{CheckInstanceStatus, CreateDevice, CreateInstance, DestroyDevice, DestroyInstance, EncodeAddOutBuff, EncodeFrameDone, EncodeOneFrame, GetDevConfig, GetEngStatus, GetParam, IsrProc, QueryBufSize, SetDevConfig, SetParam} and the same 15 for MHAL_MHE_; **JPE** (16): MHAL_JPE_{BarrierOneFrame, CheckInstanceStatus, CreateDevice, CreateInstance, DestroyDevice, DestroyInstance, EncodeAddOutBuff, EncodeCancel, EncodeFrameDone, EncodeOneFrame, GetDevConfig, GetEngStatus, GetParam, IsrProc, QueryBufSize, SetParam}
- `mi_disp` — **DISP** (50): MHAL_DISP_{ClearDevInterrupt, ClearDmaInterrupt, ClearLcdInterrupt, DeviceAddOutInterface, DeviceCreateInstance, DeviceDestroyInstance, DeviceEnable, DeviceSetBackGroundColor, DeviceSetColorTempeture, DeviceSetCvbsParam, DeviceSetGammaParam, DeviceSetHdmiParam, DeviceSetLcdParam, DeviceSetOutputTiming, DeviceSetPqConfig, DeviceSetVgaParam, DmaBind, DmaCreateInstance, DmaDestoryInstance, DmaSetAttr, DmaSetBufferAttr, DmaUnBind, EnableDevIrq, EnableDmaIrq, EnableLcdIrq, GetDevIrq, GetDevIrqFlag, GetDmaIrq, GetDmaIrqFlag, GetLcdIrq, GetLcdIrqFlag, InitMmapConfig, InitPanelConfig, InputPortCreateInstance, InputPortDestroyInstance, InputPortEnable, InputPortFlip, InputPortRotate, InputPortSetAttr, InputPortSetCropAttr, InputPortSetImiAddr, SetRegAccessConfig, SetRegFlipConfig, SetRegWaitDoneConfig, VideoLayerBind, VideoLayerCreateInstance, VideoLayerDestoryInstance, VideoLayerEnable, VideoLayerSetAttr, VideoLayerUnBind}; **PNL** (3): MhalPnlCreateInstanceEx, MhalPnlDestroyInstance, MhalPnlSetUnifiedParamConfig
- `mi_hdmi` — **HDMITX** (11): MhalHdmitx{CreateInstance, DestroyInstance, GetSinkInfo, SetAnalogDrvCur, SetAttr, SetAttrBegin, SetAttrEnd, SetDebugLevel, SetInfoFrame, SetMute, SetSignal}
- `mi_jpd` — **JPD** (12): MHAL_JPD_{CreateDevice, CreateInstance, DestroyDevice, DestroyInstance, ExtractJpgInfo, GetParam, Init, RegisterIsrCallback, ResetDev, SetParam, StartDecodeOneFrame, UnRegisterIsrCallback}
- `mi_vdec` — **VDEC** (28): MHAL_VDEC_{AddVlcBuffer, CheckInstanceStatus, ClearDispFlag, CompletSeqInit, CreateDevice, CreateInstance, DecodeAddInBuff, DecodeCancel, DecodeFrameDone, DecodeOneFrame, DestroyDevice, DestroyInstance, FlushBSBuffer, GetBSBuffer, GetDevConfig, GetEngStatus, GetParam, QueryBufSize, SeqChange, SeqDeinit, SeqInit, SetBSRdPtr, SetDevConfig, SetParam, StreamOff, StreamOn, UpdateBSBuffer, UpdateSeqChange}
- `fbdev` — **DISP** (5): MHAL_DISP_{DeviceGetDisplayInfo, DeviceGetHwCount, DeviceGetInstance, DeviceSetConfig, GetDevIrqById}; **RGN** (15): MHAL_RGN_{GetChipCapability, GopDeinitByType, GopGwinDisable, GopGwinEnable, GopGwinSetBuffer, GopGwinSetPixelFormat, GopGwinSetWindow, GopInitByType, GopSetAlphaType, GopSetAlphaZeroOpaque, GopSetArgb1555AlphaVal, GopSetBaseWindow, GopSetColorkey, GopSetOutputFormat, GopSetPalette}
- `gc8613_MIPI` (the sensor driver) — **SNR** (6): DrvRegisterSensorDriverEx, DrvRegisterSensorI2CSlaveID, DrvSensorHandleVer, DrvSensorI2CVer, DrvSensorIFVer, DrvSensorRelease (the sensor ABI: mhal-csi-snr.md §2)

Modules the product does not use (`consumers-unused.md`): mi_ai 14 + mi_ao 17 (AUDIO), mi_ldc 12 (LDC), mi_ipu 22
(IPU), mi_mipitx 9 (MIPITX), mi_panel 17 (PNL), imx307_MIPI 9 (SNR). mi_common, mi_shadow and mi_vdisp import nothing
from mhal.

Indirect use: `MHAL_CMDQ_GetSysCmdqService` fills a function table with 23 more CMDQ functions (WriteRegCmdq*,
PollRegBits*, AddWaitEventCmd, KickOffCmdq*, Read/WriteDummyRegCmdq, ResetEngine, IsCmdqEmptyIdle, …); mi_sys hands such
tables to the HAL blocks (`MHAL_DISP_SetCmdqInterface`, `DrvIsp_ChangeCmdqIF`, `MHAL_RGN_SetupCmdQ`), so the CMDQ engine is
used through that table beyond the 8 imports (table layout: mhal-scl-jpe-cmdq.md §2.3; mi_sys's side: mi_sys-1-engine.md §7.1).

User space reaches mhal directly only through `/dev/isp_fe` (opened by prog_uvc's 3A thread `Cus3A_ProcRoutine`, §4.3).
The IQ API runs `libmi_isp` → `mi_isp` → `MHalIspApiGet` / `MHalIspApiSet` → `call_MI_ISP_*` (isp-3-iq-api.md §1.2).

### 2.5 Registers per block

How mhal reaches the RIU [R] (the RIU address map itself is kernel-platform.md §3.1's): a register is a 24-bit
`A = bank<<8 | byte-offset`, accessed at VA `0xFD000000 + 2·A` (`HalUtilityW2BYTEMSKDirect`, `HalDispUtilityR2BYTEDirect`);
PA = `0x1F000000 + bank·0x200`. ISP, ISPSCL, MLOAD, VIF and the sensor early-init stub `ioremap(0x1F000000, 4 MiB)` and
add `bank·0x200`; CSI `ioremap`s each 0x200-byte bank from the DT `banks` property; JPE, JPD and LDC take their bank from
DT `reg`. Most media DT nodes have no `reg`: the bank numbers are compiled into mhal.

Banks per block [R] (hex bank numbers). Sources of each entry: DT = `board.dts`; const = code constants;
data = data tables; init = the init-time base tracker; src = mercury6 names in the
OpenIPC u-boot-sigmastar headers; the rest from the blocks' decompiles. Register positions written `bank.RR` are
16-bit register indexes (PA = `0x1F000000 + bank·0x200 + RR·4`); RGN's are given as its source writes them ("reg").

| block | banks | IRQ (GIC SPI) / clocks (DT) |
|---|---|---|
| CMDQ | 1120, 1123–1129 (data `gHalRegCmdCtlBase`, 8 engines); trigger-event table `gstCmd0TrigEvt` | 0x31 (cmdq0–3), 0x77 (cmdq4–7) / CLK_mcu |
| CSI | 1538–1546, 1038 (DT `banks`; probe labels: ClkGen, MIPI0, MIPI1, 2-lane group, 4-lane group) | 0x41 / csi_mac_lptx, csi_mac, ns × 2 ports |
| VIF | 1303, 1304–1307, 103C padtop, 113A, 1038 clkgen, 101E chiptop (DT `reg`, one entry mis-typed `0x1f207c`); also 103E, 1110, 1302, 1342 (data `g_tVifHandle`) | 0x59 / sr00, sr01, sr1 MCLK |
| ISP | 1302–134F, 54 banks set up by `HalIsp_Init` (init); DT `isp banks = <0x1302>`; clkgen | 0x39 / CLK_isp |
| ISPSCL | 19 of the ISP banks (1302…134F, init) | 0x71 / — |
| SCL | 1200, 1210–1226 (const; 1214–1219 = SCL-to-GOP / cover per src) | VPE 0x34, 0x5a, 0x5b, 0x5f / CLK_fclk1 |
| DIVP | SCL banks 1210, 1211, 1213, 1219, 121f, 1225, 1226; clock gate 1038 byte c8 (§8.1) | DIP 0x5f / CLK_fclk1 |
| DISP | 1150 TOP0, 1151 OP2, 1152 MACE, 1154 LCD, 1156 LCD_MISC, 1157 TVENC, 1158 DMA, 1160 TOP1, 1161 OP2_1, 1163 GAMMA1, 1170–1179 MOP, 1137 GP_CTRL, 145B DAC, 1037 PLL, 1038 (const + src names); also 116F (PQ peaking coefficients 116F.18–1B), 1169 (PQ ICC/IHC SRAM ports 1169.78–7A), 145A (the HDMI-TX ATOP per-timing table, written by DISP at every HDMI timing change). The MOP registers 1170 written by the boot-time `riu_w` list are DISP's (mhal-disp.md §11.4) | 0x63 dev 0, 0x67 dev 1, 0x64 dev 0 internal, 0x68 dev 1 internal, 0x6a VGA HPD, 0x6c LCD / mop0, mop1, disp_432, disp_216 |
| HDMITX | 1459 DTOP, 145A ATOP, 1038 (const; src also names 1458 LTP) | no IRQ in DT (hot-plug: §7) / disp_432 |
| RGN | GOP0–5 1281, 1284, 1287, 1291, 1294, 1297; display GOP 11A1, 11A4, 11B1, 11B4; 1038 (const + src); also 1214–1219 (per-SCL-port OSD blender enable reg 0x50, cover windows regs 0x20–0x26 + 8·window), 12F0 reg 0x71 (LDC OSD blender), 1210 reg 0x08 bit 12 (shared-OSD bind), CLKGEN 1038 regs 0x53 / 0x64 (GOP clock gates), 1137 reg 0x21 (zeroed at every GOP init) | — / fclk1, ldcfeye |
| GE | 1409 (`GE_Init_RegImage`: RIU + 0x281200) | 0x76 / CLK_ge |
| VCODEC / VENC / VDEC | 1A29–1A2C "vpu-bit", 1629 "venc-brige" / "venc-gp" (DT `reg`; two more entries named hw-uart point at 101E and 103C) | 0x35 "mhe-irq" / vhe, mfe, ven_axi |
| JPE | 1620 (DT) | 0x3d / CLK_jpe |
| JPD | 1622 (DT); MIU protect-mask bits 1012.43[7] / 1006.43[7] toggled around every NJPD reset, plus 1006.7a[7] in `SetMIU` | 0x4c / CLK_njpd |
| PNL | 1151, 1154, 1161 (shared with DISP), 1033 LPLL, 1A38–1A39 DSI, 1535 DPHY, 103C, 101E; also writes 1037.48/49 (DISP PLL set when context 0 is TTL and context 1 is DSI), 1150.25/32 (DSI gate, disp-to-DSI), 1156.23/32, 1038.5E (MIPI-TX-DSI clock mux / gate, written directly), 116D.71 and 101E.50 (all pads in); reads 001E.67 (chip revision) | — / mipi_tx_dsi (not obtained from the DT, §7) |
| LDC | 12F0, 12F1 (DT `sstar,ldc`); clock register CLKGEN 1038.53 (the same register as RGN's 1038 reg 0x53, GOP 5 LDC-mode clock) | 0x4b / ldcfeye |
| MIPITX | 1535, 1536; also the LPLL 1033.40–4B (shared with PNL), clock-gen 1038.5C, pads 103C.64 and 103E.43–4C, 1132.01 | 0x56 / mipi_tx_csi |
| AUDIO | 1502–1504, 1034 (src `BACH_REG_BANK_*`) | 0x4a / CLK_bach |
| IPU | 101E (power 0x6C, 0x1F), 1035 (IPU PLL 0x11, 0x60–0x62), 1038.50 (clock gate), 1530 (0x00, 0x13, 0x17, 0x21, 0x28), and the IPU's own space at CPU phys 0x1800_0000 (CCIF mailbox, boot windows, reset vector); 1520 and 1020 do not occur | DLA 0x55 / — |

Limits [I]: blocks that index from a base pointer at run time (VIF, CSI, SNR, VENC, AUDIO) can touch banks not visible as
constants; the base tracker sees only `add rd, rn, #imm` chains in the named init functions; hits on 1000 / 1001 / 1003
were dropped as non-register constants (1 MiB sizes and the like). The full bank sets of VENC and AUDIO are therefore not
known from this table; for VIF, CSI and SNR the banks of the camera's capture path are in mhal-vif.md §2.2 and mhal-csi-snr.md §4.1.
The ISP bank names are isp-1-hal.md §9's.

### 2.6 Public sources and headers [R unless tagged]
<!-- ev:mhal-core.2.6 -->

Coverage = how many of the block's function names (≥ 6 characters) occur as identifiers in the tree,
unless the row gives a count from a function-by-function comparison of the block. Local copies: OpenIPC `u-boot-sigmastar`
(OpenIPC u-boot-sigmastar), the jockyw2001/sdk tree (sparse clone of GitHub `jockyw2001/sdk`, commit 5b06f9a,
2018-06-23; it also holds MI sources under `impl/`), StarFive's Wave521 driver, OpenIPC's sensor drivers and the OpenIPC SigmaStar kernels.

| block | best public source | coverage | relation to this build |
|---|---|---|---|
| DISP | OpenIPC `u-boot-sigmastar` `drivers/mstar/disp/hal/mercury6` + `disp/drv/midisp/pub/mhal_disp.h` | 1000 / 1081 | same chip, register tables included; not the same revision — the source's common layer is newer and its mercury6 HAL older than this build (ABI and HAL differences: mhal-disp.md §1.2) |
| PNL | same tree, `panel/hal/mercury6` + `mhal_pnl.h` | 258 / 263 | same chip |
| AUDIO | same tree, `aio/hal/mercury6` + `mhal_audio.h` | 172 / 178 | same chip, older revision: 6 AI / 5 AO devices where this mhal has 9 / 6, and 6 functions of this mhal are not in it (audio.md §5.1) |
| RGN | same tree, `rgn/hal/mercury6` (`hal_gop.c`, `hal_osd.c`, `hal_cover.c`) + `mhal_rgn.h` | 160 of the 190 decompiled functions match by name | same chip; this build is a later revision — fbdev's `GopInitByType(1)` and `GopSetOutputFormat` do not exist in the source (rgn-gop.md §5.5) |
| HDMITX | the ssd_lcd boot tree `drivers/mstar/hdmitx` (i2m HAL); also MStar Utopia `hdmi/{api,drv}/hdmitx` in `jockyw2001/sdk` `mhal/i2/utpa` (2018, Infinity2) and u-boot `hdmitx/hal/mercury6p` + `mhal_hdmitx.h` | 622 / 653 (ssd_lcd); 416 / 653 (Utopia); 150 (u-boot) | the M6 HDMI-TX HAL is the i2m HAL ported (mhal-hdmitx.md §1.3) |
| GE | Utopia `graphic/{api/gfx,drv/ge,hal}` (`jockyw2001/sdk`) | 335 / 403 | the HAL is the I2 `hal/i2/ge` code at bank 0x1409 with M6 changes: register writes go straight out, `WaitForTAGID` waits for nothing; `MApi_GE_SupportInterrupt` is hard-wired to 1, so mi_gfx uses IRQ 0x76 (gfx-ge.md §4.2, §4.7) |
| JPD | `jockyw2001/sdk` `mhal/i2/njpd`; u-boot `jpd/jpd_hw` (mercury6p hal) + `mhal_jpd.h` | 193 / 273 | sibling chips |
| CMDQ | `jockyw2001/sdk` `mhal/i2/cmdq_service`; u-boot `cmdq_service/drv/pub/mhal_cmdq.h` | 145 / 262 | earlier generation; M6 banks and trigger events differ [I] |
| VIF | `jockyw2001/sdk` `mhal/i2/vif` | 177 / 465 | earlier generation |
| SCL | `jockyw2001/sdk` `mhal/i2/vpe` (+ `mhal_vpe.h`) | 124 / 340 | earlier generation |
| MLOAD, JPE, DIVP | `jockyw2001/sdk` `mhal/i2/{mload,jpe,divp}` | 38 / 98, 56 / 120, 12 / 51 | earlier generation; for DIVP only the API names and structs carry over (§8) |
| VCODEC | Chips&Media VPU API in StarFive `freelight-u-sdk` `soft_3rdpart/wave521/code/vpuapi`; mainline Linux `drivers/media/platform/chips-media/wave5` | 134 of the 164 library functions have a StarFive namesake | same vendor library; the product code is faked as 0x5210 in the register accessor, and the firmware is matched by its date tag only (§6; mhal-vcodec.md §4.6, §2.7) |
| VENC | `jockyw2001` `mhal/i2/{mfe,mhe}` | 22 / 57 | API names only — on Infinity2 MFE / MHE are MStar engines, on M6 they wrap the Wave5 (§2.3) |
| CSI | GitHub `longyanjun2020/SDK_pulbic` Mercury5 `hal/mercury/csi/inc/hal_csi.h` (header only) | — | older chip |
| ISP | no HAL source; register headers `mercury5_reg_isp0..12.h` (same Mercury5 repository) and u-boot `isp/infinity_reg_isp0.h`; user-side IQ / 3A headers (skyipc_tiramisu, Xsyt, `isp_3a_if.h`) match 190–211 `MI_ISP_*` names (API structs only) | — | older chips / API only |
| SNR | `drv_ms_cus_sensor.h` (sstar) — the sensor-driver ABI | 13 names | ABI only |
| LDC, IPU, MIPITX | none known; Infinity2 `warp` is LDC's predecessor [I] | — | — |
| INIT / CamOs / ISRCB | OpenIPC kernels `drivers/sstar/{cam_os_wrapper,isrcb,camclk}` | ISRCB 4 / 4 | kernel side; ISRCB: §3.4 |

Whether public code exists under the names `MHAL_LDC_*`, `JpeIos*`, `MDRV_IPU_*`, `IspMLoad*` or `DrvSensorTryShutter` is
not known.

## 3. INIT block — module entry, init and exit order, ISRCB, IMI heap [R]

### 3.1 Module entry and the "built-in" objects
<!-- ev:mhal-core.3.1 -->

- `.gnu.linkonce.this_module` points init at `init_module` (.init.text+0x1a8) and exit at `cleanup_module`
  (.exit.text+0x100). `init_module`: `printk("<6>mhal driver init\n")`, `camdriver_init()`, **return 0** — the result of
  `camdriver_init` and of every step inside it is ignored. `cleanup_module`: `printk("<6>mhal exit\n")`,
  `camdriver_exit()`.
- Most sub-libraries were compiled **without `-DMODULE`**: their `module_init` / `subsys_initcall` became entries in
  `.initcall6.init` (13: `mstar_csi_init`, `DrvDispModuleInit`, `DrvGeModuleInit`, `DrvHdmitxModuleInit`, `Isp_Driver_Init`,
  `IspScl_Driver_Init`, `DrvMipiTxCsiModuleInit`, `mload_init`, `DrvPnlModuleInit`, `rgn_init`, `mstar_sensorif_init_driver`,
  `drv_venc_init`, `_DrvSclModuleInit`) and `.initcall4.init` (3: `ss_sensor_earlyinit_init_driver`, `IspMid_Driver_Init`,
  `mstar_vif_init_driver`); their `module_exit` became 16 `.exitcall.exit` entries; their `module_param` names carry the
  object name as prefix (§5). Kernel 4.9.227 `kernel/module.c` runs only `mod->init` (`do_one_initcall(mod->init)`, line
  3413), and `include/linux/module.h` maps `module_init` to `__initcall` when `MODULE` is undefined (source tree). **So none of the initcall / exitcall entries runs by itself**; only what
  `camdriver_init` calls happens. The Infinity2 source of this file
  has the same style (`subsys_initcall(mhal_driver_init)`).
- Consequence [R code + I loader]: these platform drivers are never registered and their probes never run —
  `ss_sensor_earlyinit_*` (no `/dev/sensor_earlyinit`), `mstar_vif_init_driver` (without effect: its probe `mstar_vif_probe` is a stub, and
  `DrvVif_Init` does the probe work itself), and the platform-driver wrappers `DrvDispModuleInit` (with `DrvDispModuleProbe`),
  `DrvHdmitxModuleInit`, `DrvPnlModuleInit`, `DrvMipiTxCsiModuleInit`, `DrvGeModuleInit` (with `DrvGeModuleProbe`; the
  GE platform driver is named "mge"), `_DrvSclModuleInit` — their device set-up is called directly instead (§3.2).
  `rgn_init` / `rgn_exit` are empty.

### 3.2 `camdriver_init` order

| # | call | effect | document |
|---|---|---|---|
| 1 | `ISRCB_EarlyInit()` | clear the 60×8 callback table (§3.4) | here |
| 2 | `mstar_csi_init()` | `CamPlatformDriverRegister` "sstar,csi" (probe `csi_probe` → misc `csi`); on failure prints, then `alloc_chrdev_region(1, "csi")` and, if that **succeeds**, unregisters the driver and returns (inverted test — dead in practice) | §4, mhal-csi-snr.md §1.3 |
| 3 | `DrvVif_Init()` | `of_find_compatible_node("sstar,vif")` + `of_find_device_by_node`; none → −19; else the probe body (`ioremap` banks, IRQ, misc `vif`) | §4, mhal-vif.md §2.1 |
| 4 | `mstar_sensorif_init_driver()` | platform driver "sstar,sensorif" (probe → misc `sensorif`) | §4, mhal-csi-snr.md §1.3 |
| 5 | `find_exec_symbol2("drv_venc_init")` | `mhal_mode = 1`; platform driver "venc" matching `sstar,cedric-jpe` and `sstar,venc` (probe `codec_probe` → `jpe_probe` / `venc_probe`) | §4, mhal-vcodec.md §2.1, mhal-scl-jpe-cmdq.md §4.3 |
| 6 | `find_exec_symbol("DRV_LDC_Init_KernelModule")` | ref-counted; char region "ldc", cdev, device "ldc" under the msys class, `DRV_LDC_Init_Sysfs` | ldc-1-stack.md §8.2 |
| 7 | `_DrvSclVpeModuleInit()` | ref-counted; char region + cdev + device "mscl", `DrvSclSysfsInit`, DT node `sigmastar,vpe`, `DrvSclOsSetSclIrqIDFormSys` for IRQ slots 0–3, `genScinit = 1` | mhal-scl-jpe-cmdq.md §3.16 |
| 8 | `Isp_Driver_Init()` | `IspDriverEarlyInit`, platform driver "isp" (compatible `isp`) | isp-1-hal.md §3.1 |
| 9 | `IspScl_Driver_Init()` | platform driver "sstar,ispscl"; then `DrvIspScl_Open` if not open, `IspScl_RegisterIsr` if the IRQ is known | isp-2-ispmid-mload-ispscl.md §13.8 |
| 10 | `IspMid_Driver_Init()` | `IspMidEarlyInit`, IQ directory string := `/config/iqfile`, platform driver "sstar,ispmid" | isp-2-ispmid-mload-ispscl.md §9 |
| 11 | `mload_init()` | `cdev_alloc` + region "chr_arr_dev" + class **"chardrv"** (own class) + device "mload" + 3 attributes | isp-2-ispmid-mload-ispscl.md §12.7 |
| 12 | `find_exec_symbol("_DrvGeModuleInit")` | device "mge" (static MKDEV(250,240)), DT `sstar,ge0`, GE IRQ | gfx-ge.md §4.2 |
| 13 | `DrvJPDModuleInit()` | device "jpd" (static MKDEV(250,251)), DT `sstar,njpd` | mhal-jpd.md §2.3 |
| 14 | `_DrvHdmitxModuleInit()` | ref-flag; region + device "mhdmitx", `DrvHdmitxSysfsInit`, DT `sstar,hdmitx`, I2C id, HPD GPIO, HPD IRQ, SW-DDC GPIO from DT | mhal-hdmitx.md §9.1 |
| 15 | `_DrvPnlModuleInit()` | region + device "mpnl", `DrvPnlSysfsInit`, DT `sstar,pnl` | panel-mipitx.md §4.2 |
| 16 | `_DrvDispModuleInit()` | region + device "mdisp", `DrvDispSysfsInit`, DT `sstar,disp`, IRQ slots, `DrvDispIfClkOn` | mhal-disp.md §10.3 |
| 17 | `find_exec_symbol("_DrvMipiTxCsiModuleInit")` | region + device "mmipitx", sysfs, DT `sstar,mipitx`, IRQ slot 0 | panel-mipitx.md §7.1 |
| 18 | `find_exec_symbol("RgnSysfsInit")` | once: `/proc/mrgn/{dbglv,gop,osd,cover}` (mode 0444), `DrvGopInitCscConfig(0..9)` | rgn-gop.md §4.10, §4.8 |
| 19 | `find_exec_symbol("DRV_DIVP_PROC_Init")` | `/proc/divp/*` (§8.7) | §8 |

`find_exec_symbol(name)` and `find_exec_symbol2(name)` are identical except for the source line in their error print:
`find_symbol(name, NULL, NULL, false, false)` (the kernel's exported-symbol lookup), on success
`printk("find symbol:%s.\n", sym->name)` and a tail call of `sym->value` with no arguments; on failure
`printk("not find symbol, %s:%d\n", __FILE__, 0x60 / 0x70)`. Every looked-up name is one of mhal's own exports
(`__ksymtab`), and `find_symbol` sees a module in COMING state [I: kernel source, `each_symbol_section` skips only
UNFORMED modules], so every lookup succeeds: the indirection only lets optional libraries be left out of a build.

**Device-node quirk**: steps 12–17 (`_DrvGeModuleInit`, `DrvJPDModuleInit`, `_DrvHdmitxModuleInit`, `_DrvPnlModuleInit`,
`_DrvDispModuleInit`, `_DrvMipiTxCsiModuleInit`) share one pattern — "if the class pointer is 0, take
`msys_get_sysfs_class()` (or create an own class); **else** `cdev_init` + `cdev_add`". The class pointer (for DISP
`_tDispClass`) is 0 on the only call, so **no cdev is ever added**: the device and its sysfs attributes exist, and the
`/dev` node opens with −ENXIO [I: `chrdev_open` without a cdev]. `mscl` and `ldc` use the working order (class first, then
cdev). GE and JPD use a hard-coded major 250 that is never reserved (no `register_chrdev_region`); dynamic majors count
down from 254 in kernel 4.9 [I], so 250 may belong to another driver [I].

### 3.3 `camdriver_exit` order

`mload_cleanup` (cdev_del, device + class destroy, region release), `IspMid_Driver_Exit` (`CameraFinalRelease`,
unregister), `IspScl_Driver_Exit` (`DrvIspScl_Close` if open, unregister), `Isp_Driver_Exit` (unregister,
`IspDriverFinalRelease`), `mstar_sensorif_exit_driver`, `_DrvHdmitxModuleDeInit`, `_DrvPnlModuleDeInit`,
`_DrvDispModuleDeInit`, `_DrvSclVpeModuleDeInit`, then by lookup `DRV_LDC_DeInit_KernelModule`, `drv_venc_exit`
(unregister "venc", `mhal_mode = 0`), `DrvGeModuleExit` (prints, unregisters the never-registered GE platform driver),
then `DrvJPDModuleExit` (print only), `mstar_csi_exit` (unregister), by lookup `RgnSysfsDeInit`, `DRV_DIVP_PROC_DeInit`.
Not undone: the VIF probe work of `DrvVif_Init` (no `DrvVif_Deinit` call here; mi_vif calls `MHal_VIF_Deinit`), the
"mmipitx" device, the jpd and mge devices.

### 3.4 ISRCB — the mhal-internal interrupt-callback registry

Same code as OpenIPC i6c `drivers/sstar/isrcb/drv/src/common/drv_isrcb.c` (the same behaviour), with the id range widened
from 16 to **60**. Data: `CBInfo_t g_CBInfo[60][8]` (.bss+0x16d08, 3840 B), `CBInfo_t = {void *pData; void
(*fp)(void *pData);}`.
- `ISRCB_EarlyInit()` — zero all 480 slots, return 0.
- `ISRCB_RegisterCallback(id, fp, pData)` — first slot of row `id` whose `fp` is NULL: store `pData`, then `fp`, return
  the slot address as the handle; row full → return NULL. No lock, no bounds check on `id`.
- `ISRCB_UnRegisterCallback(handle)` — `fp = pData = NULL` (the handle must be non-NULL; callers check).
- `ISRCB_Proc(id)` — for slot 0..7 of row `id`: if `fp`, call `fp(pData)`. Runs in the caller's context, which is the
  hard-IRQ handler for every caller below.
- `fake_kernel_init()` returns 0 and has no caller.

These functions are not exported, and no other module imports them: the registry is internal to mhal.

Id map (raised by = who calls `ISRCB_Proc`; listener = who registers):

| id | event | raised by | listener (argument) |
|---|---|---|---|
| 0–3 | VIF WDMA pipe 0–3 port 0 done (`eISRCB_ID_ISP_WDMA0..3_DONE`) | `DrvIspScl_Isr_Entry` bits 0–3; ids 2, 3 also `DrvIspISR` ev 11, 12 | `DrvVifWdmaDoneCb` (pipe byte), registered by `DrvVif_PortSetConfig_{MIPI,BT656,Parallel}` in frame mode via `HalVif_GetPipePort2ISRCBID(pipe, 0)` |
| 4–7 | ISP WDMA0–3 line-hit (`…_HIT_LINE_CNT`) | `DrvIspISR` ev 8, 9, 10, 13 | `SensorFrameDoneISR` (pData 0–3), registered by `CameraIsp_RegisterCallbackAPI` (from `MhalCameraOpen`) |
| 8–0x17 | VIF channel 0–15 frame start | `DrvVif_ISR` bit 0 (`ch + 8`), `DrvVif_ITU_TDM_ISR` bit 0 (HDR sub-channel 1 raises for ch − 1) | **none** |
| 0x18–0x27 | VIF channel 0–15 frame end | `DrvVif_ISR` bit 5, `DrvVif_ITU_TDM_ISR` bit 1 (`ch + 0x18`) | `_FrameEndISR` (pData = channel), registered by `DrvSensorInit(pad)`: parallel → one id (pad 0: 0x18, pad 1: 0x20, pads 2 / 3: none); MIPI → four ids: pad 0 0x18–0x1b, pad 1 0x20–0x23, pad 2 0x1c–0x1f, pad 3 0x24–0x27 |
| 0x28–0x2b | VIF WDMA pipe 4–7 port 0 done | `DrvIspScl_Isr_Entry` bits 4–7, `DrvIspISR` ev 16–19 | `DrvVifWdmaDoneCb` (frame mode) |
| 0x2c–0x33 | VIF WDMA pipe 0–7 port 1 done | `DrvIspScl_Isr_Entry` bits 8–15 | `DrvVifWdmaDoneCb` (frame mode) |
| 0x34–0x37 | ISP WDMA4–7 line-hit [I name, from the pattern of 4–7] | `DrvIspISR` ev 20–23 | `SensorFrameDoneISR` (pData 4–7) |
| 0x38–0x3a | ISP pipe 0–2 FIFO full in frame mode | `DrvIspISR` ev 24–26 | `DrvVif_DoVifFifoFullEventCb` (group), registered by `DrvVif_GroupEnable(g)` on `0x38 + g` for every interface type |
| 0x3b | (group 3 FIFO full) | **never raised** (ev 27 has no `ISRCB_Proc`) | `DrvVif_DoVifFifoFullEventCb` (3) |

Listener bodies:
- `_FrameEndISR(ch)` → `SensorIfThreadWqAdd(gSensorIfWorkPool, _DrvSensorifFrameEndProc, ch)` — the sensor AE-notify path
  (mhal-csi-snr.md §3.6).
- `SensorFrameDoneISR(n)` (n < 16): VIF channel of pipe `n & 7` → device → group → sensor pad / plane; if pad < 4 and
  channel < 16, `IspMidThreadWqAdd(handler+0x88, SensorFrameDoneProc, handler, pad, n)`.
- `DrvVifWdmaDoneCb(&pipe)`: physical channel of the pipe → device; only in work mode 1 (frame mode)
  `DrvVif_DoVifFrameEndCb(ch)`.
- `DrvVif_DoVifFifoFullEventCb(g)`: if mi_vif registered an event callback for group `g`, print
  `DrvVif_DoVifFifoFullEventCb Fifo=%d` and call it with `(g, 4, 0, user)` (event type 4 = FIFO full, mhal-vif.md §9.6).

On the camera's realtime path (MIPI sensor on pad 0, channel 0) [I from the table and the capture documents]: only 0x18
(frame end → SNR AE notify) and, once `MhalCameraOpen` has run, 4–7 / 0x34–0x37 are registered; 0x18 is the one that
fires.

Unregistration: `CameraIsp_UnRegisterCallbackAPI` (from `MhalCameraClose`), `DrvSensorDeInit`, `DrvVif_GroupDisable`,
`DrvVif_PortDisable_*` — each clears its stored handles.

### 3.5 IMI heap (`ImiHeapMalloc`, `ImiHeapFree`, `ImiHeapQueryMaxFree`; exported) — unused

A first-fit allocator over **offsets** 0…0x20000 (128 KiB of IMI SRAM; it hands out offsets, never addresses). State at
`gImiBlkBanklist` (.bss+0x16b04) for heap index `h` (only h = 0 is ever passed): `+h·8` spare-node list head;
`+8+h·8` block list head (address order); `+0x10+h·0x154` 16 spare nodes of 20 B and the initial node; `+0x164` CamOs
mutex `gImiHeapMutex`; `+0x18c+h·4` "initialised" flag. Node = `{next, prev, start, size, used}`.
- Lazy init (first call of any of the three, under the mutex): empty both lists, one block `{start 0, size 0x20000,
  free}`, put the 16 spare nodes on the spare list.
- `ImiHeapMalloc(h, size, *out)`: size 0 → `*out = 0`, return −1. Walk the block list for the first free block with
  `size ≥ req`; equal → mark used; larger → take a spare node (none: print, unlock, **return −2 without writing `*out`**),
  make it the remainder `{start+req, size−req, free}` linked right after, shrink the found block to `req`, mark used.
  `*out = start`, return 0. No fit → `*out = 0`, return −1. No alignment.
- `ImiHeapFree(h, start)`: every used block with that start → free (return 0, else −4); then coalesce: a free block
  followed by a free block is merged into the follower (follower.start = block.start, follower.size += block.size) and
  its node returns to the spare list.
- `ImiHeapQueryMaxFree(h)`: largest free block size.

Users: only `DrvDispOsImiHeapAlloc/Free` (heap 0), which have no caller; no module in the dump imports the three exports
(relocations of every `.ko`). DISP's rotate buffer gets its IMI address from mi_sys instead
(`MHAL_DISP_InputPortSetImiAddr`, mhal-disp.md §11.3).

## 4. Character devices [R unless tagged]

### 4.1 Two shapes

(a) **Misc device + sysfs-only twin**: the probe `kmalloc`s a `struct miscdevice` {minor 0xff = dynamic, name, fops,
parent = pdev}, `misc_register`s it (→ `/dev/<name>`, major 10), then `alloc_chrdev_region(1, "<name>0")` and
`CamDeviceCreate(msys_get_sysfs_class(), …, "<name>0")` with **no cdev** — the `<name>0` device only carries sysfs
attributes (class `mstar` [I: msys source], `/sys/class/mstar/<name>0/…`). (b) **cdev device**: `alloc_chrdev_region` +
`cdev_init` / `cdev_add` + `CamDeviceCreate`. Every `open` below takes no lock and keeps no per-file state unless stated.

### 4.2 The devices

| node | created in | fops (open / read / write / ioctl / poll / release) | exists? | operations specified in |
|---|---|---|---|---|
| `/dev/isp` (+ `isp0`) | `isp_probe` (platform "isp", compatible `isp`), misc | `isp_open` 0 / `isp_fread` / `isp_fwrite` / `isp_ioctl` (only 0x4902 = power-off) / `isp_poll` / `isp_release`; fops table at .data 0x26b0 | yes | isp-1-hal.md §3.2 |
| `/dev/ispscl` (+ `ispscl0`, attributes `ispscl_func` 0600, `ispscl_ints` 0444) | `ispscl_probe`, misc | `ispscl_open` / `fread` / `fwrite` / `ioctl` / `release` | yes | isp-2-ispmid-mload-ispscl.md §13.8 |
| `/dev/csi` (+ `csi0`) | `csi_probe` (platform "sstar,csi"), misc, fops `csi_fops` .data+0x1c0 | open, read, write, release → 0; ioctl type `'C'` (else −25) nr 1–6 `_IOW('C', n, 16)` | yes; `csi0` is created with dev_t **0** (the region variable is filled only on the never-taken failure path of `mstar_csi_init`), so it has no `/dev` node — sysfs only (`csi_create_bin_file`: `ints`, `dbg_mask`) | mhal-csi-snr.md §1.3 |
| `/dev/sensorif` (+ `sensorif0`, attribute `info`) | `mstar_sensorif_probe` (platform "sstar,sensorif"), misc, fops .data+0x1c4b8 | open / read / release → 0; write = text commands; ioctl `'S'` `_IOW('S', nr, 84)` | yes | mhal-csi-snr.md §3.15 |
| `/dev/vif` (+ `vif0`, attributes `vif_ints`, `vif_info`) | VIF probe body run by `DrvVif_Init` (0x400dda26; the platform driver "mstar-vif" itself is never registered), misc, fops .data+0x1c854 | open / read / release → 0; write = `DrvVifUt` text; ioctl `'V'` only `0x80605642` | yes | mhal-vif.md §15 |
| `/dev/mload` | `mload_init` (cdev, region "chr_arr_dev", own class "chardrv"), fops .data+0x1bcf4 | read → 0, write = UT commands, ioctl → 0, open, release | yes | isp-2-ispmid-mload-ispscl.md §12.7 |
| `/dev/sensor_earlyinit` | `ss_sensor_earlyinit_probe` (platform "sstar,sensor_earlyinit"), misc, fops .data+0x1644 | open / read / write / ioctl / release → 0 | **no** — the platform driver is registered only from `.initcall4.init`, which never runs (§3.1) [R + I loader]; the DT has no `sensor_earlyinit` node either | mhal-csi-snr.md §1.3, §3.14 |
| `/dev/venc` | `venc_probe` (via `codec_probe`, platform "venc", compatible `sstar,venc`): `cdev_init(venc_ops)`, region "venc" **65 minors**, one cdev at the first minor, device "venc" under the msys class | `venc_open` / `venc_ioctl` / `venc_release`: print `[ven-e][%s] venc_xxx not support please enable MACRO MHAL_VENC_UT_ENABLE in sdk/mhal/chip_common/venc/config.mk …` (only if the VENC log filter lets errors through) and return **−14** | yes (stub) | mhal-vcodec.md §2.1 |
| `/dev/jpe` | `jpe_probe` (via `codec_probe`, compatible `sstar,cedric-jpe`): `cdev_init(jpe_ops)` (.rodata+0x109fc), region "jpe" **16 minors**, one cdev, device "jpe" | `jpe_open` / `ioctl` / `release`: the same message pattern, return −14 | yes (stub) | mhal-scl-jpe-cmdq.md §4.3 |
| `vcodec` | — | `vcodec_ops` (.rodata+0x107dc: `vcodec_open` / `ioctl` / `release`, the same −14 stubs) | **never created**: no code or data refers to `vcodec_ops` (no relocation to .rodata+0x107dc) | — |
| `/dev/ispmid`, `/dev/isp_fe`, `/dev/ispmid_cusapi` | `mstar_ispmid_probe`, `AddFeNode`, `AddCusApiNode`, misc | `isp_fe`: poll + read | yes | isp-2-ispmid-mload-ispscl.md §9, §11 |
| `/dev/mscl`, `/dev/ldc` | §3.2 steps 7, 6 (cdev) | — | yes | mhal-scl-jpe-cmdq.md §3.16, ldc-1-stack.md §8.2 |
| `mge`, `jpd`, `mhdmitx`, `mpnl`, `mdisp`, `mmipitx` | §3.2 steps 12–17 | no cdev (§3.2 quirk): device + sysfs only; for `mpnl` and `mmipitx` the creation path also does no register access | device yes, `/dev` open fails [I] | gfx-ge.md §4.2, mhal-jpd.md §2.3, mhal-hdmitx.md §9.1, panel-mipitx.md §4.2, §7.1, mhal-disp.md §10.3 |

### 4.3 Who opens them

Only `/dev/isp_fe` is opened by user space in the product (prog_uvc's `Cus3A_ProcRoutine`). Nothing in the dump opens
the other nodes — [R] for `mload` and `mge`, [I] for the rest (no string hits outside mhal.ko). The sysfs attributes
(`/sys/class/mstar/{isp0,ispscl0,csi0,sensorif0,vif0,venc,jpe,…}`) are debug knobs, except `venc`'s `ven_clock*`
(mhal-vcodec.md §2.2), which a product may write before the first H.26x stream.

What the running camera shows has not been read: the derivation above predicts no `sensor_earlyinit` and no `vcodec` in
`ls -l /dev` / `cat /proc/misc`, the nodes `mge`, `jpd`, `mdisp`, `mhdmitx`, `mpnl`, `mmipitx` present but failing to
open, and `/proc/divp/` present (§8.7).

## 5. Module parameters (all 33 in `__param`) [R unless tagged]
<!-- ev:mhal-core.5 -->

Decoded from `__param`; readers found by relocation search plus base+offset matching in
the decompiles. Every entry has level −1, flags 0. Names with a prefix come from objects built
without `-DMODULE` (§3.1); on `insmod` they are passed with the prefix (`insmod mhal.ko drv_isp_module.g_u32PreSetClkRate=…`)
[I: kernel `parse_args` matches the full name]. The boot script passes none (rootfs-customer.md §3.2), so every default below
is what runs.

| parameter | type, perm | default | reader → effect |
|---|---|---|---|
| `drv_divp_os.gDivpClk` | ulong 0644 | 432 000 000 | `DRV_DIVP_OS_ClkOn`: DIVP clock rate = `round_rate(gDivpClk + 1 MHz)`, only when the "rate enable" byte is set (`/proc/divp/clkRateEn`, §8.7); DIVP unused |
| `drv_isp_module.g_s32PreSetClkIdx` | int 0660 | −1 | `DrvIspModuleClkInit`: ≠ −1 → clock by index. **Overwritten to 1 by `isp_probe`** before the clock init (isp-1-hal.md §3.1), so the parameter has no effect |
| `drv_isp_module.g_u32PreSetClkRate` | int 0660 | 0 | `DrvIspModuleClkInit` / `GetClkRate` / `GetClkIndex`: ≠ 0 → ISP clock set to this rate, **taking precedence over the index**; also written by the sysfs `isp_clk` store (`DrvIspModulePreSetClkRate`) |
| `libcamera_module.ispsclttl` | int 0660 | 0 | `mstar_ispmid_probe` → `CameraSetSCL2TTL(v)` = flag `g_nIspMid_Scl2Ttl_1920x1080_800x480_En`; sysfs `ispmid_sclen` show / store (store normalises to 0 / 1). **No effect**: the flag's only reader is dead code (§5.1) |
| `mdrv_jpd.progressive_max_width`, `progressive_max_height`, `progressive_support` | uint 0644 | 1920 / 1080 / 0 | `MHAL_JPD_CreateDevice`, `JPEG_locate_sof_marker`: software progressive-JPEG decode limits / enable (mhal-jpd.md §2.4) |
| `drv_ldc_module.m_s32PreSetLdcClkIdx`, `m_u32PreSetLdcClkRate` | int 0644 | −1 / 0 | `DRV_LDC_Init_ClockSetting`, `…ModulePreSetClk{Idx,Rate}` (the rate also from the sysfs `check_clk` store): the same index / rate / default precedence as ISP [I: the same 4-read pattern]; LDC unused |
| `drv_ldc_module.m_u32LdcMaskTblEn` | int 0644 | 0 | `DRV_LDC_GetMaskTblEn` → `MHAL_LDC_SetBlkAttr`, `DRV_LDC_InstSetBlkAttr` (mask-table enable passed to `HAL_LDC_SetBlockAttrEx`; ldc-1-stack.md §8.2) |
| `drv_ldc_sysfs.m_u32LdcIdleClockDiv`, `m_u32LdcWorkClockDiv` | int 0644 | 0 / 0 | sysfs `check_idle_clk_div`, `check_wrok_clk_div` show / store only |
| `rgn_module.geGopOsdFlag` | ulong 0644 | 0xF5F | GOP↔OSD sharing mask (`MHAL_RGN_GopOsdFlag_e` bits), 16 readers in RGN; only 0xF5F, 0xF6F and 0xFCF are accepted — they decide whether DIVP, LDC or VPE_PORT5 owns HAL GOP 5 (rgn-gop.md §4.2) |
| `drv_rgn_os.gGopDmaThd` | ulong 0644 | 0 | `HalGopUpdateParam`: GOP DMA threshold, GOP register 0x19 [7:0] (byte offset 0x32); 0 means 0x40 (rgn-gop.md §4.4) |
| `VdeMiuPChn`, `VenMiuPChn` | ushort 0644 | 8 / 8 | MIU-protect channel counts of VDEC / VENC (`codec_attr_parameters_init`, `venc_clock_init`, `ven_clock_set`, `venc_dev_init`; mhal-vcodec.md §2.6). With `mhal_module_version` the only three parameters with a `parm:` modinfo line (normal `MODULE` objects) |
| `mhal_jpe.g_perctx_fire_num` | int 0644 | 2 | `MHAL_JPE_GetDevConfig` (mhal-scl-jpe-cmdq.md §4.4) |
| `mhal_jpe.g_parallel_chn_inq` | ushort 0644 | 1 | `JpeDevLockAndFire`, `JpeDevUnlockAfterEncDone` (mhal-scl-jpe-cmdq.md §4.13) |
| `drv_codec40_common.fw_load_path` | string(128) 0644 | "" (buffer .bss+0x30138) | `codec_load_firmware`: second choice of the firmware path (§5.1, §6) |
| `drv_venc_wrapper.CHANGE_RC_MODEL_FORCE_I` | uint 0644 | 1 | `MHAL_VENC_Wrapper_SetParam(0x8000000c)`: ≠ 0 → RC value changes reset `ctx+0x184` (§5.1) |
| `drv_venc_wrapper.RC_BITRATE_TOLERANCE` | uint 0644 | 6 | `SetParam(0x80030004)` STREAM_ON: clipped 0…15 into the instance's firmware parameter area (§5.1) |
| `drv_venc_wrapper.RC_SCENE_RECOVER_PATCH` | uint 0644 | 0 | `SetParam(0x80030004)` STREAM_ON: into the instance's firmware parameter area (§5.1) |
| `drv_venc_wrapper.GRADIENT_MODEL_LEVEL` | uint 0644 | 0 | `open+0x490 = clip(0..3)` for CBR / VBR / AVBR (§5.1) |
| `drv_venc_wrapper.AVBR_STILL_BITRATE_COMPENSATE`, `AVBR_MD_DOWN_SPEED_RATIO`, `AVBR_MOTION_RESPONSE_LEVEL` | uint 0644 | 0 / 0 / 5 | AVBR only: `open+0x486 = clip(0..4)`, `+0x485 = clip(0..7)`, `+0x484 = clip(1..5)` (§5.1) |
| `drv_venc_wrapper.AVBR_SAD_THD` | uint 0644 | 2 | **never read** (§5.1) |
| `drv_scl_module.m_s32PreSetClkIdx`, `m_u32PreSetClkRate` | int / uint 0644 | −1 / 0 | `DrvSclModuleClkInit`: rate ≠ 0 → fclk1 = rate (also the sysfs `check_clk` store); else index ≠ −1 → `DrvSclOsSetFclk1ByIndex`; else the default |
| `drv_scl_module.m_u32IsODLevel` | uint 0644 | 0 | `DrvSclDebugGetDefaultFclk1Clock(od)`: default SCL clock = **432 MHz when 0**, 480 MHz otherwise (OD = over-drive voltage level [I]) |
| `drv_scl_module.gbHistints` | ushort 0644 | 1 | `DrvSclIfProcess`, `HalSclHwWaitDone` (mhal-scl-jpe-cmdq.md §3.10) |
| `drv_scl_module.gbHvspLpfSetting` | ushort 0644 | 0 | `HalSclLpfFilterInit` (from `HalSclInit`): 1–3 → fill the 0x140-byte HVSP SRAM filter area at `MST_SRAM1_Main+0x140` with the 5-byte pattern at row `lvl·5 + 0x3c0` of the same table, repeated; 0 or > 3 → the default table |
| `mhal_module_version` | charp 0444 | 3-line build string (`…[MHAL_camdriver]…build.202207212233`, `[MHAL_sdk]`, `[MHAL_project]`) | no reader; visible in `/sys/module/mhal/parameters/mhal_module_version` [I] |

The LDC over-drive level `m_u32LdcOdLevel` is not a module parameter.

### 5.1 Effects that need more than one reader

Read in `MHAL_VENC_Wrapper_SetParam`, `…EncodeOneFrame`, `…EncodeFrameDone`, `codec_load_firmware`,
`MHAL_VENC_Wrapper_SetDevConfig` and the TTL helpers. The VENC wrapper reads its
knobs through two base pointers: `RC_BITRATE_TOLERANCE` (.data+0x1c7ac) +4 = `CHANGE_RC_MODEL_FORCE_I`, +8 =
`AVBR_MOTION_RESPONSE_LEVEL`, +0xc = `AVBR_SAD_THD`; `RC_SCENE_RECOVER_PATCH` (.bss+0x30ff0) +4 = `GRADIENT_MODEL_LEVEL`,
+8 = `AVBR_MD_DOWN_SPEED_RATIO`, +0xc = `AVBR_STILL_BITRATE_COMPENSATE`. All uses are in the rate-control set
`SetParam(0x8000000c)` (payload 0xb0 bytes, version 2; the RC payload is mhal-vcodec.md §7.4's).
- **`CHANGE_RC_MODEL_FORCE_I`** (1): when a running channel's RC values change (GOP-relative bitrate, max bitrate, source
  frame rate, derived VBV / CPB value at open+0x3c), the field `ctx+0x184` is zeroed only if this knob is ≠ 0 (else
  kept). A change of the RC *mode* (CBR↔VBR↔AVBR, `open+0x9c` / `+0x488`) zeroes `ctx+0x184` regardless. [I: `ctx+0x184`
  counts frames since the last I-frame / gates the next IDR, so 1 = "force an I-frame on every RC change", as the name
  says.]
- **`GRADIENT_MODEL_LEVEL`** (0): CBR, VBR and AVBR: warn "invalid parameters:GRADIENT_MODEL_LEVEL" if > 3, then
  `open+0x490 = clip(0..3)` ("GradientLevel"); a change sets bit 0x10 of the pending-change mask `open+0x458` (applied
  before the next frame, mhal-vcodec.md §7.2).
- **`AVBR_MOTION_RESPONSE_LEVEL`** (5) → `open+0x484 = clip(1..5)`; **`AVBR_MD_DOWN_SPEED_RATIO`** (0) → `open+0x485 =
  clip(0..7)`; **`AVBR_STILL_BITRATE_COMPENSATE`** (0) → `open+0x486 = clip(0..4)` — AVBR only (`eRcMode` 3 or 8), each
  out-of-range value warns; any change sets mask bit 0x400 (debug print "MotionResponseLevel=… MdDownSpeedRatio=…
  StillBitrateCompensate=…").
- **`RC_BITRATE_TOLERANCE`** (6), **`RC_SCENE_RECOVER_PATCH`** (0): read at `STREAM_ON` (SetParam 0x80030004) into the
  instance's 0x3c-byte firmware parameter area — +0x36 u16 `RC_SCENE_RECOVER_PATCH`, +0x38 u16 `RC_BITRATE_TOLERANCE`
  clipped to 0…15 (with +0x34 the min-QP scaling and +0x3a `CHANGE_RC_MODEL_FORCE_I`; mhal-vcodec.md §7.8) [R].
  **`AVBR_SAD_THD`** (2) is **never read** — its only reference is the base-pointer load above. The words after
  `RC_SCENE_RECOVER_PATCH` (+0x10…+0x30) are run-time statistics written by `EncodeOneFrame` / `EncodeFrameDone`, not
  parameters.
- **`drv_codec40_common.fw_load_path`** (.bss+0x30138 = `g_codec_fw_path + 0x80`; the "+0x124" test in
  `codec_load_firmware` is the first byte of this buffer, i.e. "parameter non-empty"): the firmware path order is §6.
- **`libcamera_module.ispsclttl`**: its only consumer `CameraSCL2TTL_Init` (if the flag equals **0x77**, start the VIF
  pattern generator via `CameraConfig_VIF_Pattern_Gen_Interface(1, w, h, 0x77)`) and the getter `CameraGetSCL2TTL` have
  **no caller and no reference** — the parameter (and sysfs `ispmid_sclen`, which can only store 0 / 1) has no effect.

## 6. Files and built-in tables [R unless tagged]

mhal uses no `request_firmware`; its files go through `filp_open` / `vfs_read` (CamFs).

| file | loaded by | notes |
|---|---|---|
| `/config/venc_fw/chagall.bin` (385 KB, header string `CODEC40.22031116.I5535c99.03`) | `codec_load_firmware` (VCODEC), at mhal device creation | Wave5 firmware for encode and decode: VCPU firmware plus four 8 KiB VCORE engine-microcode segments (container: chagall-fw.md §2). Path order, each tried only if non-empty and `CamFsOpen` succeeds: (1) `g_codec_fw_path`, set by `SetDevConfig(0x80030019, string < 0x80)` — mi_venc's `fw_path` (mi_venc.md §4); (2) the parameter `drv_codec40_common.fw_load_path` (§5); (3) `/config/venc_fw/chagall.bin`; (4) `/data/ut/chagall.bin`. Then a size check 1…0x80000 B, `VPU_InitWithBitcode(file, (size+1)/2)`, and the version words at file offset 0x1008: only the build date "22031116" is enforced ("fw ver … mismatch … chagall.h"); the interface version "03" only warns |
| `/config/iqfile/iqfile%d.bin` | `IspMid_Driver_Init` (sets the directory `/config/iqfile`), `CameraReadIqData` | `iqfile0.bin` is a symlink to `imx307_iqfile.bin` (firmware-image.md §3.6); IQ file format: isp-4-iq-engine.md §13 |
| built-in `iq_bin` (39 KB in `.data`) | ISP IQ engine | default IQ table compiled in; its content is not known |
| `isp_wdma_raw.bin`, `mload_ut_*`, `/mnt/sram_dump` | ispmid / mload / sysfs debug writes | debug and unit-test only |

Not loaded by mhal:
- DISP: the porch timings, PQ and panel tables are handed in by mi_sys from `config.ini` (config_tool.md §5,
  mi_disp-mi_hdmi.md §4.3). The per-timing pixel-clock synthesizer, DAC and HDMI-TX analog tables are compiled into mhal (27
  timing columns; mhal-disp.md §7.4).
- The ISP API bin `/customer/iq_file_gc8613.bin` is loaded by prog_uvc (isp-3-iq-api.md §6).
- IPU firmware: loaded by libmi_ipu (§2.1).
- `DrvVif_MCULoadBin` exists but has no caller.

## 7. Device-tree properties mhal reads, per node [R]
<!-- ev:mhal-core.7 -->

Every call site of the OF imports (`of_find_compatible_node`, `of_find_device_by_node`, `of_device_is_available`,
`CamofPropertyReadU32`, `CamOfPropertyReadU32Index/Array`, `CamIrqOfParseAndMap`, `CamOfIrqToResource`,
`CamOfAddressToResource`, `of_clk_get`, `of_clk_get_by_name`, `CamOfClkGetParentCount/Name`, `CamPlatformGetIrqByname`)
— matched against the board's DT (the DT as a whole is kernel-platform.md §14's). "probe" = the node is reached through a platform driver's `of_match_table` (only the
drivers `camdriver_init` registers, §3.1); "lookup" = `of_find_compatible_node` by string. Properties not listed for a
node are not read by mhal.

| node (compatible) | how found | properties read → use | DT value / remark |
|---|---|---|---|
| `isp` | probe (`isp_probe`) | `clock-frequency-index` (read, ignored; isp-1-hal.md §3.1); `interrupts[0]` (stored for mi_isp); `clocks[0]` (`DrvIspModuleInitClk`, parents = the rate table) | 5 / SPI 0x39 / CLK_isp. `io_phy_addr`, `banks` present but **not read** (ISP banks compiled in) |
| `csi` (`sstar,csi`) | probe | `io_phy_addr`, `banks[0..14]`, `sr0_mipi_mode`, `csi_srN_lane_num`, `csi_srN_lane_select[]`, `csi_srN_lane_pn_swap[]` (N 0–3), `interrupts[0..3]` | values in mhal-csi-snr.md §4.1; `clocks` (6) **not read** |
| `vif` (`sstar,vif`) | lookup + `of_find_device_by_node` (`DrvVif_Init` / `Deinit`) | the 34 `vif_srN_*_mode` (below), `vif_srN_mclk_37p125` (N 0–3 → `HalVifSetMCLKSupport(N, 0x9105, v)`), `vif_sr_gpio_puse_0` → `g_snr_gpio_puse`, `mcu` (read into a local, unused), `interrupts[0]` | all 34 modes present; mclk_37p125 for sr0–2; no `vif_sr_gpio_puse_0`, no `mcu`. `reg` (11 windows) and `clocks` (3) **not read** — VIF bases come from `HalVifGetHandleBasePA(0..15)`, MCLK through the clock-gen registers |
| `sensorif` (`sstar,sensorif`) | probe | `sensorif_grpN_i2c` (N 0–3) → `gSensorI2CMaster[N]` | 1, 2, 1, 2 |
| `ispscl` (`sstar,ispscl`) | probe | `interrupts[0]` | SPI 0x71 |
| `ispmid` (`sstar,ispmid`) | probe | nothing | — |
| `venc` (`sstar,venc`) | probe (`codec_probe` → `venc_probe`) | `status` (`of_device_is_available`), `clocks[i]` for i < 16 until failure, IRQ by name `mhe-irq`, every `IORESOURCE_MEM` resource (mhal-vcodec.md §2.1) | 3 clocks; `reg-names` and `clock-names` not read (resources taken by index) |
| `jpe@…` (`sstar,cedric-jpe`) | probe (`jpe_probe`) + lookups (`JpeDevClkInit`, `JpeDevGetResourceIrq/Mem`) | `status`, `reg[0]` (bank), `interrupts[0]`, clock `CKG_jpe` by name, else `clocks[i]` | `clk-select`, `interrupt-names` not read |
| `jpd@…` (`sstar,njpd`, or `sstar,njpd%d` for device ≥ 1) | lookup (`DrvJPDModuleInit`, `JPD_OS_*`, `MApi_JPD_RequestIrq`) | `reg[0]`, `interrupts[0]`, `EOI`, clock `CKG_jpd` by name, and `clocks[1]` as well (absent in our DT) [R] | EOI 0 (mhal-jpd.md §2.1) |
| `ge0` (`sstar,ge0`) | lookup (`_DrvGeModuleInit`) | `interrupts[0]` (`CamOfIrqToResource`), `clocks[*]` (`DrvGeOsSetClkOn/Off`) | SPI 0x76, CLK_ge |
| `cmdq0`…`cmdq7` (`sstar,cmdq%d`) | lookup + device (`DrvCmdqGetClk` / `GetIrqNum`) | `clocks[0]`, `interrupts[0]` | mhal-scl-jpe-cmdq.md §2.5 |
| `ldc` (`sstar,ldc`, `sstar,ldc%d`) | lookup (`DRV_LDC_Dev*`) | `reg[0]` (`CamOfAddressToResource`), `interrupts[0]`, `clocks[0]` | ldc-1-stack.md §8.1; LDC unused |
| `vpe` (`sigmastar,vpe`) | lookup (`_DrvSclVpeModuleInit`) | `interrupts[0..3]` (`DrvSclOsSetSclIrqIDFormSys`), `clocks[0]` (`DrvSclOsInitFclk1`) | SPI 0x34, 0x5a, 0x5b, 0x5f; CLK_fclk1 |
| `dip` (`sstar,dip`) | lookup + device (DIVP, §8.1) | `interrupts[0]`, `clocks[*]` | SPI 0x5f, fclk1 |
| `disp` (`sstar,disp`) | lookup (`_DrvDispModuleInit`) | `interrupts[0..5]`, `clocks[*]` (on / off by parent name) | mhal-disp.md §9.1, §10.1 |
| `hdmitx` (`sstar,hdmitx`) | lookup (`_DrvHdmitxModuleInit`) | `i2c_id`, `hpd_gpio`, `hpd_irq`, `i2c_sw`, `i2c_sda_gpio`, `i2c_scl_gpio`, `interrupts[n]` (`_DrvHdmitxGetIrqNum`), `clocks[0]` | `clocks = <&CLK_disp_432>`, `i2c_id 0`, `hpd_gpio 131`, `i2c_sw 1` (software I2C on GPIO 132 / 133: `i2c_sda_gpio 132`, `i2c_scl_gpio 133`); `hpd_irq` absent, so hot-plug is polled by a 100 ms timer (GPIO131 + 145A.18) — a GPIO IRQ is used only with `hpd_irq`. The HAL also has a `_HDMITx_Task` thread and an `_HPD_Isr` (mhal-hdmitx.md §5.2, §5.3) |
| `pnl` (`sstar,pnl`) | lookup (`_DrvPnlModuleInit`) | `ttl-16bit-mode`, `ttl-24bit-mode` (`DrvPnlOsGetMode`), `clocks[*]` | both modes absent; the clock sits under a property literally named `#clocks = <0x5f>`, which `of_clk_get` ignores, so PNL gets no clock from the DT [I] (panel-mipitx.md §4.2) |
| `mipitx_csi` (`sstar,mipitx`) | lookup (`_DrvMipiTxCsiModuleInit`) | `interrupts[0]`, `clocks[*]` | SPI 0x56 (panel-mipitx.md §7.1) |
| `gop` (`sigmastar,gop`) | lookup (`DrvRgnOsSetGopClkEnable/Disable`) | `clocks[i]` (by index, names printed) | 3 clocks (rgn-gop.md §4.5) |
| `sound` (`sstar,audio`) | lookup (`DrvAudApiDtsInit`, AUDIO — not run in the product) | `interrupts[0]`, `clocks[0]`, `amp-gpio`, the padmux / I2S / TDM / DMIC properties | audio.md §5.6 |

Nodes in the DT that mhal never reads: `ispalgo` (`sstar,ispalgo`), `scl` (`sstar,scl`), `rgn` (`sstar,rgn`), `gop1`,
`gop2`, `dla` (`sstar,dla`), `ipu` (`sstar,ipu`) — none of these strings is in mhal.ko. A node `sensor_earlyinit` does not
exist in the DT, so its platform driver could not bind even if it were registered.

The 34 VIF pad-mode properties (`g_VifSnrPadMode`, .data+0x1ca08, 34 × 0x21 B = name + value byte, default 0 each): for
sr0 and sr1 `par_mode`, `par_rst_mode`, `par_pdn_mode`, `par_mclk_mode`, `mipi_mode`, `mipi_ctrl_mode`, `mipi_rst_mode`,
`mipi_pdn_mode`, `mipi_mclk_mode`, `bt656_mode`, `bt656_mclk_mode`, `bt1120_mode`; for sr2 and sr3 the five `mipi_*`
ones; each found value → `HalVifSetVifSnrPadMode(index, value)` (used as pad-mux modes, mhal-vif.md §6.1). The DT's values
for sensor pad 0: `mipi_mode 1`, `mipi_ctrl_mode 1`, `mipi_rst/pdn/mclk_mode 0`.

## 8. DIVP block (51 functions, 11.3 KB) — unused [R unless tagged]
<!-- ev:mhal-core.8 -->

No module on the camera imports any `MHAL_DIVP_*`: there is no
`mi_divp.ko`. `libmi_divp.so` is on the camera (`/config/lib/libmi_divp.so`) but
unused, because nothing provides `/dev/mi_divp` (sdk-orphan-libs.md §1.2). So nothing below runs except the proc files
(§8.7), which `camdriver_init` creates. The API keeps the Infinity2 names and structs
; the HAL is new — the M6 "DIP" path of
the SCL engine, on banks shared with SCL. The names of the static HAL steps come from their debug labels (`// %s`
prints).

### 8.1 Resources and DT

- DT node `compatible = "sstar,dip"` (`DRV_DIVP_OS_Init`, once): `of_find_compatible_node` + `of_find_device_by_node`;
  IRQ = `CamIrqOfParseAndMap(node, 0)` (`MHAL_DIVP_GetIrqNum` returns it; nothing in mhal requests it); clocks = the
  node's `clocks` list: `DRV_DIVP_OS_ClkOn` takes clock 0, and if `gbClkRateEn` ≠ 0 sets the rate
  `round_rate(gDivpClk + 1 MHz)` (module parameter, default 432 MHz), then prepares and enables it; `…ClkOff` disables
  and unprepares every clock of the node. The DT gives IRQ GIC SPI 0x5f and clock `CLK_fclk1` (shared with SCL).
- Registers, written here as `bank:byte` (byte offset within the bank's 24-bit RIU space; VA `0xFD000000 + 2·A`, §2.5):
  1210 (SCL top: reset, input select, IRQ), 1211 (Y→R CSC), 1213 (DIP read DMA), 1219 (crop / write select), 121f (HVSP
  scaler + its filter SRAM port), 1225 (DIP write DMA), 1226 (rotate DMA), 1038:c8 bit 0 (clock gate; `HAL_DIVP_ClockOn`
  writes 0, `…Off` 1 — both unreferenced). The bank roles are [I] from the step labels below.
- Register writes go through `_DivpWrite(cmdq, A, val, mask)` (0x4002b66c, called with 2 × the RIU byte address, e.g.
  `0x243e2c` = `121f:16`): cmdq = NULL → direct read-modify-write of the RIU word (debug print `wriu -w 0x%04x 0x%04x`);
  else `cmdq->WriteRegCmdqMask(cmdq, A, val, mask)` (vtable slot +0x18 of the CMDQ service table, §2.4). Where the
  decompile drops the mask argument of a single-bit write, the mask is that bit [I: the call passes value and mask in
  r2 / r3 and Ghidra merged them].

### 8.2 API (14 exports)

| export | behaviour |
|---|---|
| `MHAL_DIVP_Init(dev)` | dev must be < 2 (`HAL_DIVP_CheckDevId`), else print, −1. Sets `gstDivpRes[dev].init = 1`, `.isr_clean = 0`; `DRV_DIVP_OS_Init`, `…ClkOn`, `HAL_DIVP_Init` (§8.5) and `HAL_DIVP_HvspInit` (filter SRAM, §8.5); 0 |
| `MHAL_DIVP_DeInit(dev)` | `HAL_DIVP_DeInit`, `ClkOff`, `OS_DeInit`, `.init = 0`; 0 |
| `MHAL_DIVP_GetChipCapbility(dev, *caps)` | `*caps = 7`; NULL → −1 |
| `MHAL_DIVP_CreateInstance(dev, maxW, maxH, pfAlloc, pfFree, chn, **ctx)` | `CamOsMemAlloc(0x88)` instance (§8.3), `.dev`, `.maxW/H`, alloc / free pointers if non-NULL, open count `gstDivpRes[dev].open++`, name `"%s%d"` (prefix string next to `gDivpClk`, count); first free of **34** slots of the device row → slot byte `.slot`, `*ctx`; `DRV_DIVP_CreateInstance(slot)` (HAL record `.used = 1`, frame count 0); no slot → print "Open too many channels", free, −1. The `chn` argument is ignored |
| `MHAL_DIVP_DestroyInstance(ctx)` | clear the slot, `open--`; when it reaches 0 `HAL_DIVP_DeInit`; clear the HAL record (0xf0 B); free; 0 (0xFF if the record free failed) |
| `MHAL_DIVP_CaptureTiming(…)` | print "Not support!", −1 (display capture not implemented) |
| `MHAL_DIVP_ProcessDramData(ctx, in, out, cmdq)` | copy `MHAL_DIVP_InputInfo_t` / `OutPutInfo_t` (Infinity2 layouts) into the instance (§8.3; pitch in pixels = `Stride[0] / Bpp`, Bpp 2 for formats 0, 4, 14–16, 4 for 1–2, else 1; formats via table B, §8.4), store cmdq, `DRV_DIVP_Process` (§8.6); 0 / 0xFF |
| `MHAL_DIVP_SetAttr(ctx, type, *param)` | 0 TNR: store, value ≠ 0 → "not support", −1; 1 DI: the same; 2 ROTATE: store the enum (+0x38), `rot90 = (enum & ~2) == 1`, mirror / flip bits re-XORed so that 180° = H+V mirror; 3 CROP: no-op (crop comes with each frame); 4 MIRROR {u8 H, u8 V}: stored XOR (rotate == 180°); 5: store u32 at +0x50 = input source (not in the Infinity2 enum) [I name, from §8.5]; other → −1 |
| `MHAL_DIVP_EnableFrameDoneIsr(b)` | global byte `irq_en = b` (copied into each HAL record at process time) |
| `MHAL_DIVP_CleanFrameDoneIsr()` | `gstDivpRes[0].isr_clean++`; `HAL_DIVP_CleanFrameDoneIsr(NULL)` (§8.6) |
| `MHAL_DIVP_WaitCmdQFrameDone(cmdq, ev)` | ev must be 0x11, else −1; `cmdq->slot+0x30()` (wait-event entry of the CMDQ table), then `HAL_DIVP_CleanFrameDoneIsr(cmdq)` |
| `MHAL_DIVP_GetIrqNum(*irq)` | `DRV_DIVP_OS_GetIrqNum` writes the virtual IRQ to `*irq` but always returns 0, which the wrapper turns into **−1** (`-(ret == 0)`): the call reports failure even when it worked |
| `DRV_DIVP_PROC_Init` / `DeInit` | §8.7 |

### 8.3 Data structures [R offsets; field names are descriptive, not from the binary]

- `gstDivpRes[2]` (.bss+0x12440, 0x90 B each): +0 u8 open count, +1 u8 last processed slot, +2 u8 init, +4 u16
  clean-ISR count, +8 `void *inst[34]`.
- Instance (0x88 B, `CamOsMemAlloc`): +0 dev, +4 u8 slot, +6 u16 maxW, +8 u16 maxH, +0xc pfAlloc, +0x10 pfFree,
  +0x14 char name[32], +0x34 u8 rot90, +0x38 rotate enum (0 / 90 / 180 / 270 = 0–3), +0x3c u16 crop x, +0x3e y, +0x40 w,
  +0x42 h, +0x44 u8 H-mirror, +0x45 u8 V-flip, +0x48 TNR, +0x4c DI, +0x50 input source, +0x54 u16 inW, +0x56 inH,
  +0x58 u16 in pitch (px), +0x5c / +0x60 / +0x64 input address Y / C / third (low 32 bits of `u64BufAddr[0..2]`), +0x68
  in format (table B), +0x6c u16 outW, +0x6e outH, +0x70 u16 out pitch (px), +0x74 / +0x78 / +0x7c output addresses,
  +0x80 out format, +0x84 CMDQ interface (NULL = direct register writes).
- HAL record `gstHalDivpInst[slot]` (.bss+0x10440, 0xf0 B; filled by `DRV_DIVP_Process` 0x4002a2d0 from the instance):
  +0 used, +1 IRQ-enable copy, +4 rot90, +8 rotate enum, +0xc crop-on (if `(cropW == inW || cropW == 0) && (cropH ==
  inH || cropH == 0)` then crop-on = (x, y) ≠ (0, 0), else 1), +0xe…+0x14 crop x / y / w / h, +0x18 / +0x28 H / V "DDA
  reduces" flags, +0x20 / +0x30 H / V DDA steps (s64), +0x38 / +0x3a DDA input W / H, +0x3c / +0x3e DDA output ("mid")
  W / H, +0x60 / +0x70 H / V HVSP scaling on, +0x68 / +0x78 H / V HVSP steps (u64), +0x80 / +0x82 scaler input W / H
  (first the crop size if crop-on, else the input size; after the DDA, mid W / H), +0x84 / +0x86 output W / H (swapped
  when rot90), +0x88 step threshold (u64, default **0x17fda8**), +0x90 "wide" (mid W > 2048), +0x94 H filter type, +0x98
  H filter on, +0x9c V filter type, +0xa0 V filter on, +0xa8 H-mirror, +0xa9 V-flip, +0xac TNR, +0xb0 DI, +0xb4 input
  source, +0xb8 / +0xba inW / inH, +0xbc in pitch, +0xc0 / +0xc4 / +0xc8 input addresses, +0xcc HAL in format (table A),
  +0xd0 / +0xd2 / +0xd4 outW / outH / out pitch, +0xd8 / +0xdc / +0xe0 output addresses, +0xe4 HAL out format, +0xe8
  CMDQ interface, +0xec u16 frame counter.

### 8.4 Formats [R tables; I names]

Two table look-ups turn the caller's pixel format into the HAL code. Table B (`ProcessDramData`, .rodata, 20 entries,
other → 10): `0→0, 1→5, 2→6, 3→10, 4→4, 5–10→10, 11→7, 12→8, 13→10, 14→2, 15→1, 16→3, 17→10, 18→10, 19→9`. Table A
(`DRV_DIVP_Process`, 10 entries, ≥ 10 → 0xe): identity for 0–8, `9→0xd`. The input numbering matches
`MI_SYS_PixelFormat_e` (YUYV, ARGB8888, ABGR8888, BGRA8888, RGB565, ARGB1555, …, NV12 = 11, NV21 = 12, UYVY = 14,
YVYU = 15, VYUY = 16, …), and the Bpp rule of §8.2 fits it [I]. Resulting HAL codes:

| HAL | from | meaning [I] | Bpp in `_SetRead` / `_SetWrite` [R] |
|---|---|---|---|
| 0 | YUYV | YUV422 packed | 2 |
| 1, 2, 3 | YVYU, UYVY, VYUY | YUV422 packed, other orders | 2 |
| 4 | RGB565 | RGB565 | 2 |
| 5, 6 | ARGB8888, ABGR8888 | 32-bit RGB | 4 |
| 7, 8 | NV12, NV21 | YUV420 semi-planar | 1 (+ C plane) |
| 0xd | format 19 | tiled / compressed 420 (pitch 128-aligned) | 1 (+ C plane) |
| 0xe | everything else | unsupported (rejected by §8.5) | — |

### 8.5 Checks and initialisation

`HAL_DIVP_CheckLimitation(rec)` (called by `DRV_DIVP_Process`; failure → print, no processing, `ProcessDramData` returns
0xFF). Sub-checks by label:
- `_CheckInputSrc`: input source ≤ 3 ("Error Type").
- `_CheckAddr`: source 0 (DRAM) → input Y address, and the C address for formats 7, 8, 0xd, 16-byte aligned; output Y
  (and C for 7, 8, 0xd) 16-byte aligned.
- `_CheckSize` without rotation: max input / crop width 0x1080 (4224) for 7 / 8 / 0xd, else 0xffff; max output width
  0x1000 for 7 / 8 / 0xd, else 0xffff (heights are not checked against a maximum); min input 16×2 and output 16×2; out
  pitch ≥ 16, and for source 0 also in pitch ≥ 16. With rotation (rot90): input W, H and pitch each 16…0x2000.
- `_CheckCrop`: crop x, y, w, h even (`x & 1`, `(y | w) & 0x10001`, `h & 1`).
- `_CheckFmt` without rotation: input HAL code 0–3 or 5–8; output code in {0–3, 5–8, 0xd} (mask 0x21ef). With rotation:
  input code in mask 0x1ab1 (0, 4, 5, 7, 9, 11, 12 — of which only 0, 4, 5, 7 can occur after table A).
- `_CheckAlign` without rotation: input width 16-aligned, output width even, out pitch 16-aligned (128 for 0xd), source 0:
  in pitch 16-aligned. With rotation: in W / H even, in and out pitch aligned to 4 (code 5), 8 (0, 2, 4), 16 (7, 9) or 32
  (11, 12) pixels.
- `_CheckScaling`: if H or V scaling: in / out ≤ 4096 (fixed-point limit, "ratio can't larger than 4096").
- `_CheckRotate`: rotation excludes crop, scaling, mirror (`+0xa8` u16, i.e. H or V), format conversion (in ≠ out code),
  any source but DRAM, and rotate type ≥ 4.

`HAL_DIVP_Init` (direct writes): `1225:02 = 0x000e` (mask 0x801e), `1219:04[0] = 1`, `1226:00[8] = 1`, `1213:04[15] = 1`,
`1213:64[0] = 1`, `1225:64[0] = 1`; `_SetInputSrc` with source 0 (`1210:0c[11:10] = 3`, `1213:02`); `_SetY2RParams`:
`1213:aa…ba` = 0x01c2, 0x1e87, 0x1fb7, 0x0107, 0x0204, 0x0064, 0x1f68, 0x1ed6, 0x01c2 (R→Y CSC [I]), `1211:04…18` =
0x1080, 0x8080, 0x0cc4, 0x0950, 0x3ffc, 0x397e, 0x0950, 0x3cde, 0x3ffe, 0x0950, 0x1024 (Y→R CSC [I: BT.601 limited
range, 0x950 = 1.164 in Q11]).

`HAL_DIVP_DeInit`: `1225:02 = 0x000e` (mask 0x801e), `1219:04[0] = 1`.

`_SetInputSrc(src)` (skipped while `/proc/divp/forceDipr` ≠ 0): `1210:0c[11:10]` = src 0 → 3, 1 → 0, 2 → 2, 3 → 1; then a
`1213:02` write. `HAL_DIVP_ForceDipr`: `1210:0c[11:10] = 3`, `1213:02[15] = 1` (called by the `forceDipr` proc write,
§8.7).

`HAL_DIVP_HvspInit`: `1219:04[0] = 0`, then `_SetHvspSram` twice — the V table (.data+0x1368) on indices 0–63, the H table
(.data+0x14a8) on 0x40–0x7f: per entry `121f:82[0] = 1`, `121f:84[7:0] = index`, u16 → `121f:86`, u16 → `121f:87`, u8 →
`121f:88` (5 bytes per entry), `121f:82 = 0x301`, `121f:82[0] = 0`; finally `1219:04[0]` is restored to its earlier value.
The odd address `121f:87` is as in the binary. Which
table entries realise filter type 1 and which type 2 (§8.6 step 8) is not encoded anywhere in DIVP — no other register
selects a table — so the meaning of the two types stays [I].

### 8.6 `HAL_DIVP_Process(rec)` — one frame, DRAM → DRAM

All writes go through `_DivpWrite(rec.cmdq, …)` (direct or queued); the steps are inlined in
0x4002bf18–0x4002cfac, their labels come from the binary. "Bpp" = 2 for HAL codes 0–4, 4 for 5–6, 1 otherwise (the
same rule in read and write). Per-register writes in program order (register (mask) → value):

| step | register (mask) | value |
|---|---|---|
| 1 `_PreSetting` | `1210:02` (0x10) | 1, then later 0 (engine reset pulse [I]) |
| | `121f:c4` (ffff), `121f:c8` (ffff) | 0, 0 |
| | `1225:02` (801e) | 0x800e |
| | `1219:04` (1) | 0 |
| 2 `_EnableFrameDoneIsr` | `1210:06` (ff00) | IRQ off, or IRQ on with no rotation and source ≥ 4: 0; IRQ on + rotation: 0x2000; IRQ on, no rotation, source 1–3: 0x2000; source 0: 0x6000 |
| | `1226:3c` (0030) | 0xffef with rotation (bit 4 unmasked), else 0xffff |
| | `1210:e0` (ffff) | 0xefff when the IRQ is on without rotation (source 0–3), else 0xffff |
| | `1210:e2` (ffff) | 0xffff always |
| | `1210:e4` (ffff) | 0x7fff with rotation, else 0xffff |
| | `1210:58` (ffff) | 0x7fff with rotation; 0xf7ff IRQ on, no rotation, source 0–3; else 0xffff (the bits unmasked in `1210:58` / `1210:e0`, 11 / 12, = DIP write done [I]) |
| | then `_SetInputSrc` | §8.5 |
| 3 `_SetRotate` | `1210:0c` (8000) | 0 without rotation; 0x8000 with rotation, followed by: |
| | `1226:02` (003f) | format code (in HAL 5→0, 4→1, 0/2→4, 9→5, 0xb→6, 7→8, 0xc→9, other→0) \| 0x10 if rotate enum 1 (90°), 0x30 if 3 (270°) |
| | `1226:04`, `1226:06`, `1226:08` (3fff) | in W, in H, in pitch |
| | `1226:0a` (3fff) | in pitch if in code 7, else 0 (code 8 → 0) |
| | `1226:0c` (3fff), `1226:0e` (3fff) | out pitch; out pitch if out code 7, else 0 |
| | `1226:10/12, 14/16, 18/1a` | in Y, C, third address (bytes): low word & 0xfff0 (mask fff0), high word (ffff) |
| | `1226:20/22, 24/26, 28/2a` | out Y, C, third address, the same split |
| 4 `_SetRead` (source 0 only) | `1213:08`, `1213:0a` | in W, in H |
| | `1213:10` / `1213:12` (0fff) | in Y >> 4 (low 16 bits) / in Y >> 20 |
| | `1213:20` / `1213:22` (0fff) | in C >> 4 / in C >> 20 |
| | `1213:42/44/46`, `1213:48` (00ff) | 0x3e02, 0x3e02, 0x3e3e, 0x7c (read-DMA thresholds) |
| | `1213:04` (1f84) | code 0, 1: 0x0e80; 2, 3: 0x1e80; 4: 0x0c00; 5, 6: 0x0c00; 7, 8, 0xd: 0x0204 |
| | `1213:06` (bf00) | code 5: 0xa100; 6: 0x8900; else 0 |
| | `1213:a0` (1), `1213:a4` (1) | 1 for codes 5, 6, else 0 |
| | `1213:a6` (0011) | 0x11 for codes 5, 6, else 0 (RGB input [I]) |
| | `1213:50/52` (52: 0fff) | Y window start = in Y >> 4 (split 16 / 12 bits) |
| | `1213:54/56` | Y window end = (in Y >> 4) − 1 + (H · pitch · Bpp) / 16 (signed, rounded toward 0) |
| | `1213:58/5a` | C window start = in C >> 4 |
| | `1213:5c/5e` | C window end = (in C >> 4) − 1 + (H · pitch) / 32 for codes 7, 8, 0xd; else 0 |
| | `1213:60`, `1213:62` | in pitch; C pitch = in pitch for codes 7, 8, 0xd, else 0 |
| 5 `_SetCrop` | `1219:20` (1) | crop-on (§8.3 rule) |
| | `1219:22`, `1219:24`, `1219:26`, `1219:28`, `1219:2a`, `1219:2c` | in W, in H, crop x, crop w, crop y, crop h |
| 6 `_SetScaling` + `_SetDda` | `121f:aa` (8) | 8 for in codes 1, 3, 8, else 0 |
| | `121f:c4/c6`, `121f:c8/ca` | scaler input W / H (crop size if crop-on), DDA output ("mid") W / H — rule below |
| | `121f:c0` (9fff) | H reduced: `(stepH & 0x7fff) | 0x8000` (the mask keeps bits 15 and 12:0), else 0 |
| | `121f:c2` (9fff) | V likewise |
| 7 `_SetHvsp` | `121f:40`, `121f:42`, `121f:44`, `121f:46` | mid W, mid H, out W, out H (out W / H swapped when rot90) |
| | `121f:18` | 0xfff0 |
| | `121f:a2` (8) | 8 |
| | `121f:0e`, `121f:10` (01ff) | H scaling: hStep[15:0], hStep[23:16] \| 0x100 (enable); else 0, 0 |
| | `121f:12`, `121f:14` (01ff) | V likewise with vStep |
| 8 `_SetHvspCfg` | `121f:16` (efef) | rule below |
| 9 `_SetWrite` | `1219:06` (1) | 1 if out code 0xd |
| | `1225:08/0a` | out W, out H |
| | `1225:10/12` (12: 0fff) | Y' >> 4 / Y' >> 20 (Y' = out Y after mirror / flip, below) |
| | `1225:20/22` (22: 0fff) | C' >> 4 / C' >> 20 |
| | `1225:42/44/46`, `1225:48` (00ff) | 0x3030, 0x1818, 0x3030, 0x60 (write-DMA thresholds) |
| | `1225:04` (738f) | base by out code: 0: 0x384, 1: 0x385, 2: 0x386, 3: 0x387, 4: 0x304, 5: 0x4304, 6: 0x4304, 7: 0x00c, 8: 0x00d, 0xd: 0x00c; \| 0x2000 with H mirror, \| 0x1000 with V flip |
| | `1225:06` (bfff) | code 5: 0xa100; 6: 0x8900; else 0 |
| | `1211:02` (0020) | 0x20 for out codes 5, 6, else 0 (Y→R CSC on [I]) |
| | `1225:60`, `1225:62` | out pitch; C pitch = out pitch for codes 7, 8, 0xd, else 0. A frame time stamp is taken |
| 10 `_Trigger` | `1226:00` (1) | 1 with rotation (rotate-DMA start); nothing else is written |
| | `1225:02` (1) | without rotation, source 1–3: 1 |
| | `1225:02` (1), then `1210:26` (1) | without rotation, source 0: 1, 1 (start of the read DMA [I]) |
| | — | without rotation, source ≥ 4: no trigger, return 1 |

Mirror / flip address arithmetic (step 9): H mirror → Y' = Y + Bpp · (align16(outW) − 1), C' = C + (align16(outW) − 1)
(C only for codes 7, 8, 0xd); V flip → Y' += Bpp · pitch · (outH − 1), C' += pitch · (outH/2 − 1) (codes 7, 8, 0xd).

**DDA intermediate size (step 6).** Let inW / inH = scaler input, oW / oH = output (after the rot90 swap), o4W = 4·oW,
o4H = 4·oH.
1. If inW ≤ 4096 and inW ≤ o4W and inH ≤ o4H: mid = in (no reduction).
2. Else if o4H < inH (vertical reduction needed): midH = o4H and the width cap is **1024**; midW = min(o4W, inW, 1024).
3. Else (horizontal only): midH = inH; cap = 2048 if oW ≤ 2048, else 4096; midW = min(o4W, inW, cap).
4. Refinement, per direction (n = in size, c starting at mid): while `(n + 4096·c − 1)/n == (n + 4096·c − 2)/(n − 1)`
   (signed integer division) decrement c; mid = the first c where the two quotients differ.
5. `/proc/divp/ddaOutSize`: a non-zero stored W (resp. H) replaces midW (midH).
6. Record: +0x38 / +0x3a = in W / H, +0x3c / +0x3e = mid W / H, +0x18 / +0x28 = (in ≠ mid) per direction; DDA steps
   stepH = (inW − 1 + 4096·midW)/inW, stepV likewise (+0x20 / +0x30, sign-extended to 64 bits). Afterwards the scaler
   input fields +0x80 / +0x82 become mid W / H, and +0x60 / +0x70 = (mid ≠ out) per direction.

HVSP steps (step 7): hStep = (midW << 20) / oW, vStep = (midH << 20) / oH, unsigned 64-bit (+0x68 / +0x78).

**HVSP filter selection (step 8).** Threshold T = rec+0x88 (u64); when zero it is first set to **0x17fda8** (≈ 1.4994 in
the 2^20 step scale [I: 1.5·2^20 = 0x180000]). Wide = midW > 2048 (rec+0x90). With hOn = (midW ≠ oW), vOn = (midH ≠ oH):

| field | value |
|---|---|
| +0x94 H filter type | hOn ? (hStep ≤ T ? 1 : 2) : 0 |
| +0x98 H on | hOn |
| +0x9c V filter type | vOn ? (vStep ≤ T ? 1 : (midW ≤ 2048 ? 2 : 1)) : 0 |
| +0xa0 V on | vOn |

`121f:16` (mask 0xefef) = bit 0 (H type ≠ 0) | bit 1 (hOn) | bit 6 (H type ∈ {2, 3}) | bit 7 (H type = 3) | bit 8 (V type
≠ 0) | bit 9 (vOn) | bit 11 (wide) | bit 14 (V type ∈ {2, 3}) | bit 15 (V type = 3). Type 3 is never produced, so bits 7
and 15 stay 0. In words: a direction that scales uses filter type 1 (bits 7:6 = 00) up to a reduction of ≈ 1.5× and type
2 (bits 7:6 = 01) beyond it; vertically, type 2 is used only when the intermediate line is at most 2048 pixels wide [R:
the code; I: that type 2 is the stronger low-pass and the 2048 limit is a line-buffer limit].

`HAL_DIVP_CleanFrameDoneIsr(cmdq)`: time stamp; no cmdq → `1226:3c = 0x300`, `1210:92 = 0x3000`, `1210:94 = 0`,
`1210:96 = 0x8000` (clear the done status [I]); with cmdq → queued `1210:5e = 0x8880`.

Nothing in DIVP waits for completion or handles the IRQ itself: the caller (the absent `mi_divp`) would wait on CMDQ
event 0x11 (`WaitCmdQFrameDone`) or on the IRQ.

### 8.7 `/proc/divp` (created at mhal load by `DRV_DIVP_PROC_Init`, removed by `…DeInit`)

Directory `divp`, five entries, mode 0444 as passed (0x124) yet with write handlers; every write copies ≤ 7 bytes (≤ 14
for `ddaOutSize`) and parses with `sscanf("%li")` (decimal or 0x-hex; else prints "%s is not in hex or decimal form.").

| entry | read (seq_file) | write |
|---|---|---|
| `dbglvl` | current level | `gDivpDbgLvl` = value (bit 0: info prints such as clock rate and IRQ; bit 1: trace every register write as `wriu -w` / `CMDQ => reg …`) |
| `info` | three tables over both devices and all 34 slots: device `FinalChnId Init UpdateHvsp ClearIsrCnt`; channel `ChnId Cmdq IrqEn RotEn RotType InputSrc Mirror Flip FrameCnt`; buffers `Input/Output Width Height Stride Format YAddr UAddr VAddr` | — |
| `forceDipr` | "0: normal, 1: Force Dipr", value | byte `gDivpForceDipr`; non-zero → `HAL_DIVP_ForceDipr()` at once, and `_SetInputSrc` is skipped from then on |
| `ddaOutSize` | "Set DDA output size (default=0)", current W × H | `"%li %li"` → `gDdaOutW`, `gDdaOutH` (override of the DDA intermediate size, §8.6) |
| `clkRateEn` | "0: disable, 1: enable", `gbClkRateEn` | byte `gbClkRateEn` (lets `ClkOn` set `gDivpClk`) |
