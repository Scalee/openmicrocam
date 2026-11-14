# mhal-hdmitx — the HDMITX block of mhal.ko
<!-- ev:mhal-hdmitx -->

HDMITX is the HDMI-transmitter block of `mhal.ko`: the driver of the SoC's HDMI-TX (a digital part, DTOP, bank 1459: mode,
mute, colour conversion, packets, its own timing generator, audio; an analog PHY part, ATOP, bank 145A: TMDS enable,
Rx-sense, drive current, power). It is the SigmaStar `hdmitx` driver of the ssd_lcd boot tree — the MStar Utopia
`apiHDMITx` / `drvHDMITx` core with its HDCP 1.4 logic, an instance / query layer, a Linux OS layer — on top of a chip HAL
that is the Infinity2M (i2m) HAL ported to this chip (M6): bank bases moved, a new DTOP timing-generator table, an extended
timing list, a wider drive-current interface, one clock. Its only caller is `mi_hdmi.ko` through 11 of the 13 exported
`MhalHdmitx*` functions. Hot-plug is polled every 100 ms from a timer (no HDMI interrupt); a kernel thread runs a 5-state
output state machine that reads the EDID over a software I²C bus on two GPIOs and then programs timing, AVI infoframe and
TMDS. HDCP is present as code and dead. On this camera it outputs 1920x1080p60 or 3840x2160p30, HDMI mode, YUV444 out,
24-bit colour.
This document covers the block's source structure and revision, the exported API and its enums, the query and instance
layers, the M6 HAL (register access, register map, timing tables, sequences, drive current, clock), hot-plug and the
state machine, DDC and EDID, HDCP, packets and infoframes, module glue, sysfs and the OS layer, and what is not known.

Shared facts owned elsewhere: mhal's block map, the consumers' import lists (`mi_hdmi` imports 11 `MhalHdmitx*`), the
`camdriver_init` order (step 14 `_DrvHdmitxModuleInit`), the device-node quirk shared by GE / JPD / HDMITX / PNL / DISP /
MIPITX and the DT properties mhal reads per node — `mhal-core.md` §2.1, §2.4, §2.5, §3.2, §4.2, §7; RIU addressing
(bank → physical address), the DT clock tree, GPIO and padmux, the DTB as a whole — `kernel-platform.md` §3.1, §5.1, §6;
the HDMI-TX ATOP per-timing and common tables (`ST_INIT_HDMITX_ATOP_TBL` / `_COMMON_TBL`), which on Linux are written by
DISP at every HDMI timing change, DISP's timing generator and the `disp_432` clock — `mhal-disp.md` §7.3, §7.4; the
caller side (mi_hdmi's MI → mhal enum tables, its ioctls, the Start sequence it issues, the EDID parser in libmi_hdmi) —
`mi_disp-mi_hdmi.md` §2.4, §2.3, §3.3; U-Boot's copy of this driver and how `bootlogo` drives it — `uboot.md` §8, §5.3.

Tags: **[R]** read from the binary (Ghidra decompile, Thumb disassembly, `.data` / `.rodata`, symbol table), the DT or a
cited file; **[S]** stated by a source (the reference source text, a name taken from a
source header, a debug string used as a name); **[I]** inference. A heading tagged [R] means every statement under it is
[R] unless marked otherwise.

Binary: `/config/modules/4.9.227/mhal.ko`; the HDMITX block is objdump `.text` 0x36acc–0x420bc (653 symbols, the HDMITX
rows of). "disassembly 0x…" are `.text` offsets.

This tree is the same code generation as the binary and its i2m HAL is the direct ancestor of the M6 HAL; it covers 622
of the 653 names. The 2018 Infinity2 Utopia copy (`jockyw2001-sdk`, `mhal/i2/utpa` `hdmi/{api,drv}/hdmitx`) matches only
the Utopia api / drv layer (416 / 653 names); the OpenIPC `u-boot-sigmastar` `hdmitx/hal/mercury6p` HAL is a newer
driver generation (149 / 653 names) and is not the base of this build [R name counts].

Register notation: `bank:reg` = one 16-bit RIU register (bank and 16-bit register index in hex); `[h:l]` a bit field;
`regL` / `regH` the low / high byte of a register (byte tables). The HAL's access functions take (bank byte address,
16-bit register index, …): `MHal_HDMITx_Mask_Write(0x145a00, 0x16, 0xff, v)` = 145A:16 low byte. A byte address `A` in a
table row is `bank<<8 | byte index`, byte index = 2·reg (+1 for the high byte).

Address forms: objdump `.text` offset + 0x40000028 = the 0x400xxxxx address used here (image rebased to 0x40000000).
Reference source ("src"): the SigmaStar `hdmitx` driver of the ssd_lcd boot tree (`fifteenhex/ssd_lcd` on GitHub,
`boot/drivers/mstar/hdmitx/`) — `drv/mihdmitx` (MHAL API), `drv/hdmitx/src/drv_hdmitx_{if,ctx}.c` (instance / query
layer), `drv/hdmitx/src/linux/` (kernel OS layer, module, sysfs), `drv/hdmitx/src/api` + `src/drv` (MStar Utopia
`apiHDMITx` / `drvHDMITx`), `hal/infinity2m` (the Infinity2M chip HAL: `halHDMITx.c`, `halHDMIUtilTx.c`,
`src/hal_hdmitx_if.c`, tables `hal_hdmitx_top_tbl.c`).

## 1. The block [R]

### 1.1 Layers
```
mi_hdmi ──► MhalHdmitx* (mhal_hdmitx.c, 13 fns)          NULL check, else forward; NULL → 1 (CFGERR)
             └► DrvHdmitxIf* (drv_hdmitx_if.c)            ctx (DrvHdmitxCtx*), MI→HAL struct translation, query
                 └► HalHdmitxIfQuery → HalHdmitxIf table   17 entries {GetInfo, Set, cfg size}  (§3.1)
                     └► MApi_HDMITx_* (apiHDMITx.c)        Utopia API: instance via _HDMITXOpen, mutex, *_U2
                         └► MDrv_HDMITx_* (drvHDMITx.c)    state (gstHdmitxResourcePirvate), FSM task, EDID, HDCP 1.4
                             └► MHal_HDMITx_* (M6 HAL)      registers DTOP 1459 / ATOP 145A / CLKGEN 1038:48 (§4)
OS layer DrvHdmitxOs* (linux/drv_hdmitx_os.c): CamOs threads/timers/events, GPIO, I2C, clocks, MMIO base.
```
[R call graph of the decompile; layer names S from src.] Every layer except the M6 HAL behaves as src unless a difference
is listed in this document (§1.3).

### 1.2 Source files and address ranges [R]
| src file | `.text` range | bytes | SAME / DIFF / NOSRC / tail jumps |
|---|---|---|---|
| `api/hdmitx/apiHDMITx.c` (Utopia API, `MApi_HDMITx_*`) | 0x36acc–0x38fe0 | 9492 | 79 / 86 / 4 / 20 |
| (no src: `hdmitx_nosrc_helper` (0x40039008), `MDrv_EEPROM_Read`) | 0x38fe0–0x39090 | 176 | NOSRC 2 |
| `drv/hdmitx/drvHDMITx.c` (Utopia driver, `MDrv_HDMITx_*`, HDCP 1.4 state machine) | 0x39090–0x3c234 | 12708 | 93 / 50 / 2 / 15 |
| `drv_hdmitx_ctx.c` | 0x3c234–0x3c430 | 508 | 6 / 0 |
| `drv_hdmitx_if.c` (`DrvHdmitxIf*`, query executor) | 0x3c430–0x3d5fc | 4556 | 22 / 9 / 2 |
| `linux/drv_hdmitx_module.c` (platform driver) | 0x3d5fc–0x3d6f6 | 250 | 3 / 4 / 1 / 2 |
| `linux/drv_hdmitx_os.c` (OS layer: threads, timers, events, GPIO, I2C, clocks) | 0x3d6f6–0x3e4bc | 3526 | 44 / 6 / 2 / 4 |
| `linux/hdmitx_module.c` (char device, DT) | 0x3e4bc–0x3e728 | 620 | 3 / 2 / 1 |
| `linux/hdmitx_sysfs.c` | 0x3e728–0x3f050 | 2224 | 12 / 3 / 8 |
| `mihdmitx/mhal_hdmitx.c` (the 13 `Mhal*` exports) | 0x3f050–0x3f20a | 442 | 12 / 0 / 1 / 1 |
| `hal/…/halHDMITx.c` (M6 HAL proper, `MHal_HDMITx_*`) | 0x3f20a–0x406c8 | 5310 | 45 / 18 / 3 |
| `hal/…/halHDMIUtilTx.c` (register access, SW I2C/DDC, EDID read) | 0x406c8–0x40e54 | 1932 | 26 / 10 / 0 / 4 |
| `hal/…/src/hal_hdmitx_if.c` (`HalHdmitxIf*`, HAL query table) | 0x40e54–0x420bc | 4712 | 36 / 5 / 6 / 1 |

The M6-specific chip HAL is ≈ 12 KB of the 46 KB (the last three rows); the rest is chip-independent driver code that
matches src [R sizes; I "chip-independent" from the src layout].

Data the block owns: the DTOP per-timing table `ST_INIT_HDMITX_DTOP_TIMING_TBL` (.data+0x2488, 403 B) and the DTOP common
table (.data+0x261b, 35 B) with their descriptor `stHAL_HDMITX_TOP_TBL` (.data+0x2470) (§4.4); `HDMITxVideoModeTbl`
(.data+0x20b4, 736 B), `TxAudioFreqTbl` (.data+0x1fec) and the other tables of §4.3; the packet register maps
`HDMITX_{Spd,Avi,Vs,Ado}PktReg` and `NULL_PACKET` (§8); `_HdmitxMatchTable` (392 B, §9.1); the sysfs attribute table
(.data+0x1f18, §9.2); the HAL's DDC block (.data+0x2464, §6.1); `g_bHpdDetect` (.data+0x1f68, §5.1) [R `notes.md`];
`gstHdmitxResourcePirvate` (.bss, 11 444 B, §1.3); `gpHdmitxCbTbl` (§3.1), `gstForceAttrCfg` (§3.4),
`gstHdmitxCtxTbl` (§3.2) in .bss. The HDMI-TX ATOP tables `ST_INIT_HDMITX_ATOP_TBL` / `_COMMON_TBL` (.data+0xce8 /
+0xe9a) belong to the DISP block (`mhal-disp.md` §7.4) [R symbol addresses].

### 1.3 Comparison with the reference source [R]
<!-- ev:mhal-hdmitx.1.3 -->
Each function was reduced to its callees, its register accesses through the five HAL access functions (bank normalised
to a logical name: DTOP = i2m 0x1125 / M6 0x1459, ATOP = 0x1126 / 0x145A, CLKGEN 0x1038, GPCTRL i2m 0x1133), its fully
known write values and its strings; src was reduced the same way with every macro resolved and `#if 0` blocks dropped.
| status | functions | meaning |
|---|---|---|
| SAME | 381 | features equal to the same-named src function |
| DIFF | 193 | named in src, some feature differs: 129 are artefacts (semaphore / mutex macros that become calls, folded tail calls, inlined `*_U2`), 64 are real differences, each given in the section of its function |
| NOSRC | 33 | no function of that name in any source at hand (each given in the section of its function) |
| four-byte tail jumps | 46 | `b.w <target>` only, folded into the target |
| src functions absent from the binary | 163 | the i2m HDCP HAL files (`halHDCPTx.c`, `hdcp/halHDCP.c`), `drv/hdcp/drvHDCP.c`, CEC, and a few api/drv functions |

The 46 tail jumps: `DrvHdmitxOs{MemAlloc,MemRelease,MsDelay,MsSleep,UsDelay,UsSleep}` → `CamOs*`; 19 `MApi_*_U2` /
`MApi_*` → their `MDrv_*`; 15 `MDrv_HDMITx_*` → their `MHal_HDMITx_*`; `MHal_HDMITxPM_{Read,Write,Mask_Write}` → the normal
`MHal_HDMITx_{Read,Write,Mask_Write}` (no separate PM bank on M6); `MHal_HDMITx_HDCPKeyReadByte` → `MDrv_EEPROM_Read` (§7);
`MhalHdmitxSetDebugLevel` → `DrvHdmitxIfSetDebugLevel`; `_HalHdmitxIfSetDeInitConfig` → `MApi_HDMITx_Exit`.

The 381 SAME functions were also compared beyond features. src's `apiHDMITx.c`, `drvHDMITx.c`, `drv_hdmitx_if.c`,
`drv_hdmitx_ctx.c`, `mhal_hdmitx.c`, `halHDMITx.c`, `halHDMIUtilTx.c`, `hal_hdmitx_if.c` were compiled for ARM Thumb-2
(`arm-linux-gnueabihf-gcc -O2 -fno-pic -mthumb`, U-Boot OS header with stub includes) and each SAME function compared with
the binary by the multiset of its data constants (immediates of compare / logic / move / shift instructions and
un-relocated literal words); the 62 SAME functions of the Linux layer (`drv_hdmitx_os.c` 44, `hdmitx_sysfs.c` 12, the two
module files 6) need kernel headers and were read side by side:
| outcome | functions |
|---|---|
| constants equal | 194 |
| differ only by a uniform offset (−5 in the driver state, ±2 / +0x2b… `__LINE__` arguments of debug prints, −4 one struct) | 28 |
| enum translators (`_DrvHdmitxIfTrans*`, `__HalHdmitxIfTrans*`: switch tables vs compare chains) — their binary mapping is §2.2, §3.3, §4.3 | 17 |
| M6 vs i2m bank base in the HAL (0x1459 / 0x145A vs 0x1125 / 0x1126), same registers | 7 |
| other: M6 HAL files `halHDMITx.c` / `halHDMIUtilTx.c` / `hal_hdmitx_if.c` — read function by function (§4) | 35 |
| other: api/drv/if/ctx (`apiHDMITx.c` 10, `drvHDMITx.c` 16, `drv_hdmitx_if.c` 8, `drv_hdmitx_ctx.c` 4) | 38 |
| Linux layer, read side by side | 62 |

Of the 38 api/drv/if/ctx functions, 16 were read side by side (`MApi_HDMITx_{Exit,GetFsmStatus,GetEDIDData}`,
`_MDrv_HDCPTx_HdcpSyncSetting`, `MDrv_HDMITx_{InitVariable,CheckOver3G,CheckEDIDQuantizationRange,GetRxStatus,GetStatus,
SetDeepColorMode,GetEDIDPhyAdr,GetRxIDManufacturerName}`, `_MDrv_HDMITx_ParseHFVSDB`, `_MDrv_HDMITX_InitEdidField`,
`DrvHdmitxCtx{Init,Allocate}`); all are equal in logic. The other 22 (six `MApi_*` wrappers of the same shape as the three
read, five HDCP-1.4 getters — dead code, §7 —, `DrvHdmitxCtx{DeInit,Get}`, the 8 `DrvHdmitxIf*` setters, `_HDMITXOpen`)
differ only by constants of the same kinds (`__LINE__` shifted uniformly by +0x2B…+0x31 in `drv_hdmitx_if.c` and ±2 in
the ctx file, debug-bit shift amounts, the resource size 0x2CB4 vs 0x2CB9) [R constants; I equality]. The constant
differences are `__LINE__` values, debug-mask bit tests (the binary gates prints on its own bits, §2.1 `SetDebugLevel` —
e.g. src's unconditional HF-VSDB `printf`s are gated on drv debug bit 0), inlining of `*_U2` bodies by the reference
compiler, and these layout and enum facts of the binary [R]:
- Utopia timing enum: `E_HDMITX_RES_MAX` = **0x2E** (src 0x25) — `MDrv_HDMITx_InitVariable` sets
  `output_video_prevtiming` to it; ids 37–45 are the extended list of §4.3.
- The analog-setting block that `InitVariable` clears is 0x15 bytes (src 0x12) [R]; consistent with the 11-byte drive
  current (§2.2) [I].
- Driver state: the EDID area starts at +0x9C (src +0xA1); `gstHdmitxResourcePirvate` is 0x2CB4 bytes (src 0x2CB9).
- The context table and its HAL containers have their own layout (§3.2).
The Linux-layer differences are in §9.3.

## 2. The exported API [R decompile 0x3f050–0x3f20a; names S from src `mhal_hdmitx.c`]

### 2.1 Exports
All take the instance handle first (the `DrvHdmitxCtxConfig_t *` returned by `CreateInstance`); a NULL handle or NULL
config → print "Param NULL", return **1**. Return values `MhalHdmitxRet_e`: 0 SUCCESS, 1 CFGERR, 2 NOTSUPPORT, 3 NONEED,
4 HPDERR, 5 ERR [S src enum; R the binary returns 1 and 5 as listed]. The query ids are those of §3.1.
| export | used by mi_hdmi | config | effect (query id) |
|---|---|---|---|
| `CreateInstance(void **h, u32 id)` | yes | — | ctx init + alloc; **clock on** (`_DrvHdmitxIfSetClkOn`, §4.7) **before** query INIT (src: after); INIT (0) = `MApi_HDMITx_Init` (§4.5) + HDCP off, video / audio / TMDS / AV-mute from ctx (`_HalHdmitxIfSetInitConfig`, SAME as src). Failure → 5 |
| `GetInstance(void **h, u32 id)` | no (sysfs only) | — | returns the ctx allocated for `id` |
| `DestroyInstance(h)` | yes | — | DEINIT (1) = `MApi_HDMITx_Exit` (§4.5); clock off (`_DrvHdmitxIfSetClkOff`); ctx free |
| `SetAttrBegin(h)` | yes | — | ATTR_BEGIN (2) = `MApi_HDMITx_TurnOnOff(0)` (TMDS off, FSM reset, stops the Rx timer) |
| `SetAttr(h, MhalHdmitxAttrConfig_t*)` | yes | 0x2c B (§2.2) | ATTR (3): §3.3 |
| `SetAttrEnd(h)` | yes | — | ATTR_END (4) = `TurnOnOff(1)`, `MApi_HDMITx_Exhibit()` (FSM kick, §5.4), then poll `GetFsmStatus` every 10 ms until DONE or 500 ms |
| `SetSignal(h, {bool bEn})` | yes | 1 B | SIGNAL (5): on → `DisableTMDSCtrl(0)`, `SetTMDSOnOff(1)`; off → `SetTMDSOnOff(0)`, `DisableTMDSCtrl(1)` |
| `SetMute(h, {bool bMute; u32 enType})` | yes | 8 B | MUTE (6): enType bits 1 VIDEO, 2 AUDIO, 4 AVMUTE (the GCP AV-mute); mute: AVMUTE → `SetAVMUTE(1)` + `DisableAvMuteCtrl(1)`, VIDEO → `SetVideoOnOff(0)`, AUDIO → `SetAudioOnOff(0)`; unmute in reverse order; enType 3 = video + audio |
| `GetSinkInfo(h, {u32 enType; union})` | yes | 0x88 B | SINK_INFO (8); GetInfo does the work (Set is empty): type 0 EDID block (`u8 block` +4, 128 B data +5) via `MApi_HDMITx_GetEDIDData`, on failure `__HalHdmitxIfCheckEdidRdy` (EDID check every 10 ms for ≤ 200 ms) and one retry, else 4; type 1 HDMI supported (+4 bool); type 2 colour formats for a timing (+4 mhal timing in, +8 out bit mask 1 RGB444 / 2 YUV444 / 4 YUV422 / 8 YUV420 from the EDID); type 3 **hot-plug** (+4 bool) = `MApi_HDMITx_GetRxStatus()` = "Rx status == DVI-clock-high & HPD-high" (§5.1) |
| `SetInfoFrame(h, MhalHdmitxInfoFrameConfig_t*)` | yes (unused path) | 0x8c B | INFO_FRAME (9): the MI struct is translated into a byte image (`__DrvHdmitxIfTransAviInfoFrame` / `__DrvHdmixIfTransAudioInfoFrame` / SPD copy), then enabled → `PKT_Content_Define(type, data, len)` + `PKT_User_Define(type, 1, CYCLIC, 1)`; disabled → `PKT_User_Define(type, 0, CYCLIC, 1)` |
| `SetAnalogDrvCur(h, {u8 x11})` | yes (unused path) | 0xb B | ANALOG_DRV_CUR (7): §4.6 |
| `SetDebugLevel(h, u32)` | yes (level 3) | 4 B | stores `_gu32HdmitxDbgLevel` (bit 16 = DrvHdmitxIf prints, bit 17 = HalHdmitxIf prints); DEBUG_LEVEL (10) → `MApi_HDMITx_SetDbgLevel(u16)` → drv debug word (bit 0 = MDrv prints, bit 2 = HAL util / I2C prints, bit 4 = HAL prints, bit 5 = OS prints). mi_hdmi's level 3 sets bit 0, so the MDrv debug prints (FSM state changes, "Hpd Detect", AnalogDrvCur) are enabled in the product [R bit tests; I: that they reach the kernel log] |
| `SetHpdConfig(h, {u8 u8GpioNum})` | no | 1 B | HPD (11): `MApi_HDMITx_SetHPDGpioPin(n)` → `MHal_HDMITx_SetHPDGpioPin` (stores the pin, requests it as input) |

### 2.2 Structures and enums as this build uses them [R offsets from the HalHdmitxIf translators and printers; names S from src]
`MhalHdmitxAttrConfig_t` (0x2c): +0x00 `bool bVideoEn`, +0x04 `enInColor`, +0x08 `enOutColor`, +0x0c `enColorDepth`,
+0x10 `enOutputMode`, +0x14 `enTiming`, +0x18 `bool bAudioEn`, +0x1c `enAudioFreq`, +0x20 `enAudioCh`, +0x24 `enAudioCode`,
+0x28 `enAudioFmt` — the src layout. The enums [R `_HalHdmitxIfGetInfoAttrConfig` name switch + the `__HalHdmitxIfTrans*`
functions]:
| enum | values |
|---|---|
| colour (in / out) | 1 RGB444, 2 YUV444, 4 YUV422, 8 YUV420, 0x10 AUTO (out only) |
| output mode | 0 DVI, 1 DVI_HDCP, 2 HDMI, 3 HDMI_HDCP, **4 AUTO** (new) |
| colour depth | 0 NO_ID, 1 24 bit, 2 30, 3 36, 4 48 |
| timing (`MhaHdmitxTimingResType_e`) | 0 720x480p60, 1 720x576p50, 2 1280x720p60, 3 1280x720p50, 4/5/6/7/8 1920x1080p24/25/30/50/60, 9/0xa/0xb/0xc/0xd 3840x2160p24/25/30/50/60, 0xe 1024x768p60, 0xf 1366x768p60, 0x10 1440x900p60, 0x11 1280x800p60, 0x12 1280x1024p60, 0x13 1680x1050p60, 0x14 1600x1200p60; **new in this build:** 0x15 2560x1440p30, 0x16 1920x1200p60, 0x17 1280x720p29.97, 0x18 1280x720p59.94, 0x19 1920x1080p29.97, 0x1a 1920x1080p59.94, 0x1b 2560x1440p50, 0x1c 2560x1440p60, 0x1d 3840x2160p29.97; 0x1e = MAX (mi_hdmi's "30 = unsupported") |
| audio freq | 0 NO_SIG, 1 32K, 2 44K, 3 48K, 4 88K, 5 96K, 6 176K, 7 192K |
| audio ch / coding / source format | 0 CH_2, 1 CH_8 / 0 PCM, 1 NONPCM / 0 PCM, 1 DSD, 2 HBR, 3 NA |
| sink info type | 0 EDID_DATA, 1 HDMI_SUPPORT, 2 COLOR_FORMAT, 3 HPD_STATUS |
| infoframe type | 0 AVI, 1 SPD, 2 AUDIO |

The timing names `2560x1440p30` (attr-name table) and `2560x1440_30P` (M6-HAL mode-name table) are strings of this binary;
neither is in src [R].
mi_hdmi's MI → mhal maps (`mi_disp-mi_hdmi.md` §2.4) resolve against these enums to: 1080p60 (MI 9) → mhal 8; 4K30 (MI 0xd) →
mhal 0xb; out colour MI 2 (YCbCr444) → 2 YUV444; deep colour MI 1 → mhal 2 (30 bit) per mi_hdmi's `{1,2,3,4,0}` table,
which mi_hdmi's SetAttr forces to MI 0 (24 bit) → mhal 1 before sending [R both tables; I: the composition].

`MhalHdmitxAnaloDrvCurConfig_t` in this build is **11 bytes**, not src's 8: `u8 Tap1Ch0..3, Tap2Ch0..3, PdRtermCh,
PdLdoPreDrvCh, PdLdoMuxCh` (names S from the sysfs usage string, §9.2).
`MhalHdmitxInfoFrameConfig_t` (0x8c), `MhalHdmitxSinkInfoConfig_t` (0x88), `MhalHdmitxMuteConfig_t` (8), `…SignalConfig_t`
(1), `…HpdConfig_t` (1): src layouts, sizes confirmed by the query table (§3.1).

## 3. Query and instance layers [R]

### 3.1 HAL query table (`HalHdmitxIfInit`, `gpHdmitxCbTbl` .bss, 17 × 12 B + init flag at +0xcc)
Entry `{GetInfo(ctx, cfg) → ret, Set(ctx, cfg), u16 cfg size}`. `_HalHdmitxIfGetCallBack` rejects a size mismatch (ret 1)
and returns ret 2 when the entry has no Set function; `_DrvHdmitxIfExecuteQuery` runs Set only when GetInfo returned
0 (OK) or 3 (NONEED), otherwise prints the query and result names and fails. Query struct (0x14): `{u32 id, void *cfg, u32
size, u32 ret, fn *set}`. src has ids 0–13; **14–16 are new** [R; names I from the function names].
| id | name | size | GetInfo / Set (`_HalHdmitxIf…`) |
|---|---|---|---|
| 0 | INIT | 0 | GetInfoInitConfig / SetInitConfig |
| 1 | DEINIT | 0 | GetInfoDeInitConfig / SetDeInitConfig = tail jump to `MApi_HDMITx_Exit` |
| 2 | ATTR_BEGIN | 0 | GetInfoAttrBeginConfig / SetAttrBeingConfig |
| 3 | ATTR | 0x2c | GetInfoAttrConfig (copies the 11 words to the HAL ctx +0x18, debug print) / SetAttrConfig (§3.3) |
| 4 | ATTR_END | 0 | GetInfoAttrEndConfig / SetAttrEndConfig |
| 5 | SIGNAL | 1 | GetInfoSignalConfig / SetSignalConfig |
| 6 | MUTE | 8 | GetInfoMuteConfig / SetMuteConfig |
| 7 | ANALOG_DRV_CUR | 0xb | GetInfoAnalogDrvCurConfig (copies 11 B to HAL ctx +0xd) / SetAnalogDrvCurConfig (§4.6) |
| 8 | SINK_INFO | 0x88 | GetInfoSinkInfoConfig (does the read, §2.1) / SetSinkInfoConfig (empty) |
| 9 | INFO_FRAME | 0x8c | GetInfoInfoFrameConfig / SetInfoFrameConfig |
| 10 | DEBUG_LEVEL | 4 | GetInfoDebugLevelConfig / SetDebugLevelConfig |
| 11 | HPD | 1 | GetHpdConfig / SetHpdConfig |
| 12 | CLK_SET | 0xc | GetInfoClkSet / SetClkSet → `MHal_HDMITx_SetClk` |
| 13 | CLK_GET | 0xc | GetInfoClkGet → `MHal_HDMITx_GetClk` / SetClkGet (empty) |
| 14 | FORCE_ATTR_SET (new) | 0x10 | GetInfoForceAttrConfigSet / SetForceAttrConfigSet (§3.4) |
| 15 | FORCE_ATTR_GET (new) | 0x10 | GetInfoForceAttrConfigGet (copies `gstForceAttrCfg`) / SetForceAttrConfigGet (empty) |
| 16 | ANALOG_DRV_CUR_GET (new) | 0xb | GetInfoAnalogDrvCurConfigGet (reads back via `MApi_HDMITX_GetAnalogDrvCur`) / SetAnalogDrvCurConfigGet (empty) |

Only the sysfs nodes (§9.2) issue 12–16 directly; the MHAL exports issue 0–11 [R callers].

### 3.2 Instance and context [R `drv_hdmitx_ctx.c` region, SAME ×6]
`DrvHdmitxCtxInit` / `Allocate` / `Free` / `IsAllFree` / `GetCurCtx` / `SetCurCtx` as src. One static table
`gstHdmitxCtxTbl` of 2 × 12 bytes `{u8 bUsed; s32 s32CtxId; DrvHdmitxCtxHalContain_t *pstHalCtx}`, each with a 0xD0-byte
HAL container `DrvHdmitxCtxHalContain_t` (bVideoOnOff, bAudioOnOff, bSignal, bAvMute, the drive-current copy at +0xd, the
attr copy at +0x18). One instance in practice (mi_hdmi id 0).

### 3.3 Attribute programming (`_HalHdmitxIfSetAttrConfig`) [R; differs from src]
MHAL → HAL translation (`_DrvHdmitxIfTrans*ToHal`): identity with clamps — colour not in {1,2,4,8,0x10} → 1 (RGB444),
output mode > 3 → 4 (AUTO), timing outside 0–0x1d → 0x1e, audio ch / coding → bool. Then, **video first** (src: audio
first):
1. Output mode: AUTO (4) → mode HDMI (2) with force off; any other → `__HalHdmitxIfTransOutputMode` (0/1/3 kept, else 2)
   with force on. Out colour: AUTO (0x10) → YUV444 (2) with force off; other → `__HalHdmitxIfTransColorFmt` with force on.
   Translation to Utopia: colour 1→0 RGB444, 2→2 YUV444, 4→1 YUV422, 8→3 YUV420, else 0; depth 1/2/3/4 → 4/5/6/7, else 0;
   timing → Utopia `MsHDMITX_VIDEO_TIMING` via `__HalHdmitxIfTransTimingRes` (§4.3).
2. Calls, in order: `MApi_HDMITx_SetHDMITxMode(mode)`, **`MApi_HDMITx_ForceHDMIOutputMode(force, mode)` (new)**,
   `SetColorFormat(in, out)`, `ForceHDMIOutputColorFormat(force, out)`, `SetHDMITxMode_CD(mode, depth)`,
   `SetVideoOutputTiming(timing)`.
3. If `bAudioEn`: `SetAudioConfiguration(freq, ch 2|8, NONPCM flag)`, `SetAudioSourceFormat(fmt)`, `SetAudioFrequency(freq)`.

"Force" means the EDID check (§6.3) keeps the requested mode / colour even when the sink does not advertise it; with AUTO
the driver may change them from the EDID [R `MDrv_HDMITx_EdidChecking` "ForceOututMode/Color" prints; I: the semantics].
The API layer of this build has no range checks on mode, depth, colour or audio frequency (an older revision of
`apiHDMITx.c` prints "Error output mode / deep color / video color / audio frequency" — `uboot.md` §8) [R].

### 3.4 Force-attribute config (new, `gstForceAttrCfg` .bss, 0x10 B)
`{u32 flags; u8 bOutputModeEn (+4); u32 enOutputMode (+8, MHAL enum); u8 bHpdDetect (+0xc)}`. FORCE_ATTR_SET: flag bit 0
→ store +4 / +8 and `MApi_HDMITx_ForceHDMIOutputMode(en, TransOutputMode(mode))`; flag bit 1 → store +0xc and
`MApi_HDMITx_SetHpdDetect(b)` → `MHal_HDMITx_SetHpdDetect` → `g_bHpdDetect` (§5.1). Reached only from sysfs `forcecfg`
(§9.2).

## 4. The M6 HDMI-TX HAL (`MHal_HDMITx_*`, 0x3f20a–0x420bc) [R]

The M6 HAL is src's i2m HAL with the bank bases moved (DTOP 1125 → 1459, ATOP 1126 → 145A, GPCTRL 1133 → none, CLKGEN 1038
unchanged), a new DTOP timing table, an extended timing list, a wider drive-current interface, a single clock and a few
removals. Everything not named below is as src [R comparison + function-by-function reading].

### 4.1 Register access [R `MHal_HDMITx_{Read,ReadByte,Write,Mask_Write,Mask_WriteByte,RegsTbl_Write,SetIOMapBase}`]
- `Read/Write/Mask_Write(bank, reg, [mask,] val)`: 16-bit access at `RIU_base + bank·2 + reg·4` (bank = byte address such
  as 0x145900, `reg` = 16-bit register index); `RIU_base` = 0xFD000000 (`DrvHdmitxOsGetMmioBase` type 0; type 1 =
  0xFD200000, size 0x200000 each, §9.3), the kernel's static mapping of physical 0x1F000000 [R base values; I: the mapping].
  `ReadByte/Mask_WriteByte(bank, byte_index, mask, val)`: byte access, byte index = `reg·2 + (high byte ? 1 : 0)`, address
  `RIU_base + bank·2 + (idx & ~1)·2 + (idx & 1)`.
- All writes are suppressed while `g_bDisableRegWrite` (.bss) is 1 (`MApi_HDMITx_DisableRegWrite`, unused).
- `MHal_HDMITxPM_*` are tail jumps to the same functions: there is no separate PM bank (src i2m had none either).
- Banks used [R literal pools]: **DTOP 0x1459** (digital: mode, mute, CSC, packets, timing generator, audio), **ATOP
  0x145A** (analog PHY: TMDS enable, Rx-sense, drive current, power), **CLKGEN 0x1038 reg 0x48** (the 432 MHz clock), and
  two debug-only reads of **0x1126:16** in `HDMITx_Handler`, inherited literally from src (`MHal_HDMITx_Read(0x112600,
  0x16)`, the i2m ATOP address): on M6 they read an unrelated block and only feed a debug print. src's GPCTRL bank 0x1133 is
  gone (`SetClk` / `GetClk`). src also names a bank 1458 (LTP, `mhal-core.md` §2.5); the code does not use it.

### 4.2 Register map (DTOP 1459 / ATOP 145A) [R offsets from the M6 code; names S from src `regHDMITx.h`; I that they keep their meaning on M6]
<!-- ev:mhal-hdmitx.4.2 -->
| reg | bits | name (src) | written by |
|---|---|---|---|
| 1459:00 | 0 | `PD` (1 = block on) | `Power_OnOff` |
| 1459:00 | 1 | `SW_RST` | `SwReset` (also DISP's RSTHDMITX, `mhal-disp.md` §6.3) |
| 1459:01 | 0 | `MODE` 1 HDMI / 0 DVI | `SetHDMImode` |
| 1459:01 | 1 | `AUDIO_LAYOUT` 0 = 2 ch, 1 = 8 ch | `SetAudioFrequency` |
| 1459:01 | 11:8 | `AUDIO_SP` sub-packets (0 / 0xF) | `SetAudioFrequency` |
| 1459:01 | 12 | `AUDIO_MUTE` | `SetAudioOnOff` |
| 1459:01 | 13 | `VIDEO_MUTE` | `SetVideoOnOff` |
| 1459:01 | 14 | `AVMUTE` | `SendPacket` (GCP) |
| 1459:05 | 3/4/5 | `CH0/1/2_MUTE` | `SetVideoOnOff` |
| 1459:06 | 3:0 | `CSC_SEL` 0 bypass, 2 Y→R, 4 R→Y, (1 422 10→8, 8 444→422) | `SetColorFormat`, `ColorandRange_Transform` |
| 1459:08 | 7:0 | `DATA_PERIOD_ISLAND` (0 or 0x21) | `EnablePacketGen` |
| 1459:0F | 2 / 3 | `VSYNC_OUT_POL` / `HSYNC_OUT_POL` (1 = invert, set when the mode table says "low") | `SetVideoOutputMode` |
| 1459:10–19 | | timing generator (§4.4) | `DumpAtopTable` |
| 1459:1B | 0…8 | `PKT_EN` per packet (0 SPD/PKT0, 1 VS/PKT1, 2 audio sample, 3 ACR, 4 audio IF, 5 AVI, 6 GC, 7 NULL, 8 PKT2) | `SendPacket`, `Set_AVI_InfoFrame` |
| 1459:1D | 0–2 | `PKT_RDY_SEL` | `SendPacket` |
| 1459:1F | 0, 1 | (unnamed; **new**, set by `InitSeq`) | `InitSeq` |
| 1459:20 | 3:0 | `AUDIO_SF` (0) | `SetAudioFrequency` |
| 1459:21–2C | bytes | `PKT_ASP_CS_1..24` audio channel status | `SetAudioFrequency`, `PKT_Content_Define(0x84)` |
| 1459:30 | 0 | `ACR_MOD` | — |
| 1459:31 / 32 / 33 | | `ACR_CTS` [15:0] + 33[3:0] = CTS[19:16]; `ACR_N` [15:0] + 33[7:4] = N[19:16] | `SetAudioFrequency` |
| 1459:34–3B | bytes | AVI infoframe PB0 (checksum, byte 0x68) … PB13 (`HDMITX_AviPktReg`) | `SendPacket(0x82)`, `Set_AVI_InfoFrame` |
| 1459:3C–3F | bytes | audio infoframe (`HDMITX_AdoPktReg`) | `SendPacket(0x84)` |
| 1459:40–50 | bytes | SPD: HB0–2 at 0x80–0x82 (0x83/0x01/0x19), checksum 0x84, PB1–25 (`HDMITX_SpdPktReg`) | `SendPacket(0x83)` |
| 1459:52–63 | bytes | VS: HB0–2 at 0xa4–0xa6 (0x81/0x01/0x1b), PB… (`HDMITX_VsPktReg`, 27 B) | `SendPacket(0x81)` |
| 145A:16 | 7:0 | `en_data_out` (0xF0 = TMDS on, 0 = off); read back = TMDS status | `SetTMDSOnOff`, `GetTMDSStatus` |
| 145A:18 | 15:0 | Rx sense / DVI clock detect (`& 0xCCCC == 0` ⇒ "DVI clock high") | `GetRXStatus` (read) |
| 145A:19 / 1A | | defaults 0x1B1A / 0x1A19 if they read 0 | `InitSeq` |
| 145A:22–25 | 5:0, 13:8 | drive current Tap1 Ch0/1, Ch2/3, Tap2 Ch0/1, Ch2/3 | `SetAnalogDrvCur` (§4.6) |
| 145A:32 | 3:0 / 7:4 / 11:8 | **new**: PdRtermCh / PdLdoPreDrvCh / PdLdoMuxCh | `SetAnalogDrvCur` |
| 145A:33 / 34 | 0 | power-down bits (0 = on); 34 also `PLLOnOff` (1 = off) | `Power_OnOff`, `PLLOnOff` |
| 145A:35 | 4:0 | power-down bits (0 = on, 0x1F = off) | `Power_OnOff` |
| 145A:38 | 7:0 | default 0x1A if it reads 0 | `InitSeq` |

The per-timing ATOP rows written by **DISP** (`__HalDispIfDumpHdmitxAtopTbl`, `mhal-disp.md` §7.3 step 4, §7.4: 145A:16H,
1D, 22–25, 30–32, 39, 3B) overlap this map: DISP sets 145A:22–25 per timing on every output-timing change, and the HDMI-TX
HAL overwrites them only when a non-zero drive-current config exists (§4.6) [R both writers]. The app starts HDMI
first: `St_HdmiModuleInit` (which calls `MI_HDMI_Start`) runs before `St_DispModuleInit` at all four call sites
(`prog_uvc.md` §2.2, §5.1) [R]; so at start-up DISP's per-timing rows are
written after the HAL's [I: from the call order, the writes inside the modules not traced]. DISP's ATOP
rows 145A:32L (0x0F, or 0x08 for the 1440p50 / 60 and 2160p columns) fall on the bits this HAL names PdRtermCh (3:0) and
PdLdoPreDrvCh (7:4), so the "Pd" fields are normally 0xF / 0x0 and, at the 297 MHz class, 0x8 / 0x0 [R values; I: the
field overlap].

### 4.3 Timings and the per-timing data tables [R: `__HalHdmitxIfTransTimingRes`, `.data` tables; dump]
<!-- ev:mhal-hdmitx.4.3 -->
Utopia `MsHDMITX_VIDEO_TIMING` = src 0–36 plus **37 2560x1440p30, 38 1920x1200p60, 39 1920x1080p59.94, 40
1920x1080p29.97, 41 1280x720p59.94, 42 1280x720p29.97, 43 2560x1440p50, 44 2560x1440p60, 45 3840x2160p29.97**, 46 = MAX
(`E_HDMITX_RES_MAX` = 0x2E, §1.3) [R: names of 37–45 from the mhal → Utopia map and the pixel clocks].
mhal → Utopia (`__HalHdmitxIfTransTimingRes`): 0→3, 1→4, 2→6, 3→5, 4–8→9–13, 9–0xd→17–21, 0xe→30, 0xf→34, 0x10→28,
0x11→35, 0x12→29, 0x13→36, 0x14→27, 0x15→37, 0x16→38, 0x17→42, 0x18→41, 0x19→40, 0x1a→39, 0x1b→43, 0x1c→44, 0x1d→45,
else 46.

`HDMITxVideoModeTbl` (.data+0x20b4, 46 × 16 B `{u32 interlace 0 / progressive 1, u32 hpol, u32 vpol (1 = low), u32 pixel
clock Hz}`), with `HDMITX_AviCmrTbl` and `HDMITX_AviVicTbl` (46 B each, same index). Entries 0–36 are byte-equal to src,
including src's 94 000 000 Hz for 4096x2160p60 (594 000 000 in the 4096x2160p50 row) [R; I: a typo of src]; 37–45 are new.
Names of 0–36 S from src; 37–45 as above:
| id | name | i/p | hpol | vpol | pixel clock Hz | AVI C/M/R byte | VIC |
|---|---|---|---|---|---|---|---|
| 0 | 640x480p | P | low | low | 25175000 | 0x48 | 1 |
| 1 | 720x480i | I | low | low | 27000000 | 0x48 | 6 |
| 2 | 720x576i | I | low | low | 27000000 | 0x48 | 21 |
| 3 | 720x480p | P | low | low | 27000000 | 0x48 | 2 |
| 4 | 720x576p | P | low | low | 27000000 | 0x48 | 17 |
| 5 | 1280x720p50 | P | high | high | 74250000 | 0xa8 | 19 |
| 6 | 1280x720p60 | P | high | high | 74250000 | 0xa8 | 4 |
| 7 | 1920x1080i50 | I | high | high | 74250000 | 0xa8 | 20 |
| 8 | 1920x1080i60 | I | high | high | 74250000 | 0xa8 | 5 |
| 9 | 1920x1080p24 | P | high | high | 74250000 | 0xa8 | 32 |
| 10 | 1920x1080p25 | P | high | high | 74250000 | 0xa8 | 33 |
| 11 | 1920x1080p30 | P | high | high | 74250000 | 0xa8 | 34 |
| 12 | 1920x1080p50 | P | high | high | 148500000 | 0xa8 | 31 |
| 13 | 1920x1080p60 | P | high | high | 148500000 | 0xa8 | 16 |
| 14 | 1920x2205p24 | P | high | high | 148500000 | 0xa8 | 32 |
| 15 | 1280x1470p50 | P | high | high | 148500000 | 0xa8 | 19 |
| 16 | 1280x1470p60 | P | high | high | 148500000 | 0xa8 | 4 |
| 17 | 3840x2160p24 | P | high | high | 297000000 | 0xa8 | 0 |
| 18 | 3840x2160p25 | P | high | high | 297000000 | 0xa8 | 0 |
| 19 | 3840x2160p30 | P | high | high | 297000000 | 0xa8 | 0 |
| 20 | 3840x2160p50 | P | high | high | 594000000 | 0xa8 | 96 |
| 21 | 3840x2160p60 | P | high | high | 594000000 | 0xa8 | 97 |
| 22 | 4096x2160p24 | P | high | high | 297000000 | 0xa8 | 0 |
| 23 | 4096x2160p25 | P | high | high | 297000000 | 0xa8 | 99 |
| 24 | 4096x2160p30 | P | high | high | 297000000 | 0xa8 | 100 |
| 25 | 4096x2160p50 | P | high | high | 594000000 | 0xa8 | 101 |
| 26 | 4096x2160p60 | P | high | high | 94000000 | 0xa8 | 102 |
| 27 | 1600x1200p60 | P | high | high | 162000000 | 0x48 | 0 |
| 28 | 1440x900p60 | P | high | high | 106500000 | 0xa8 | 0 |
| 29 | 1280x1024p60 | P | high | high | 108000000 | 0x48 | 0 |
| 30 | 1024x768p60 | P | high | high | 65000000 | 0x48 | 0 |
| 31 | 1280x720p24 | P | high | high | 59400000 | 0xa8 | 60 |
| 32 | 1280x720p25 | P | high | high | 74250000 | 0xa8 | 61 |
| 33 | 1280x720p30 | P | high | high | 74250000 | 0xa8 | 62 |
| 34 | 1366x768p60 | P | high | high | 85800000 | 0xa8 | 0 |
| 35 | 1280x800p60 | P | high | high | 83700000 | 0xa8 | 0 |
| 36 | 1680x1050p60 | P | high | high | 146300000 | 0xa8 | 0 |
| 37 | 2560x1440p30 | P | high | high | 148500000 | 0xa8 | 0 |
| 38 | 1920x1200p60 | P | high | high | 193622000 | 0xa8 | 0 |
| 39 | 1920x1080p59.94 | P | high | high | 148500000 | 0xa8 | 16 |
| 40 | 1920x1080p29.97 | P | high | high | 74250000 | 0xa8 | 34 |
| 41 | 1280x720p59.94 | P | high | high | 74250000 | 0xa8 | 4 |
| 42 | 1280x720p29.97 | P | high | high | 37125000 | 0xa8 | 62 |
| 43 | 2560x1440p50 | P | high | high | 297000000 | 0xa8 | 0 |
| 44 | 2560x1440p60 | P | high | high | 297000000 | 0xa8 | 0 |
| 45 | 3840x2160p29.97 | P | high | high | 296700000 | 0x00 | 0 |

`MHal_HDMITx_GetPixelClk_ByTiming(t, colour, depth)` = pixel kHz × {8; or 10 / 12 / 16 for depth 5 / 6 / 7} / 8 (/16 for
YUV420) = TMDS clock in kHz; `GetMaxPixelClk` = 597 000 kHz.

`TxAudioFreqTbl` (.data+0x1fec, 8 × 12 B `{u8 CH_Status3; u32 N (+4); u32 CTS (+8)}`, index = Utopia audio frequency):
| idx | CH_Status3 | N | CTS |
|---|---|---|---|
| 0 | 0x02 | 6144 (0x1800) | 27000 (0x6978) |
| 1 | 0x03 | 4096 (0x1000) | 27000 (0x6978) |
| 2 | 0x00 | 6272 (0x1880) | 30000 (0x7530) |
| 3 | 0x02 | 6144 (0x1800) | 27000 (0x6978) |
| 4 | 0x08 | 12544 (0x3100) | 30000 (0x7530) |
| 5 | 0x0a | 12288 (0x3000) | 27000 (0x6978) |
| 6 | 0x0c | 25088 (0x6200) | 30000 (0x7530) |
| 7 | 0x0e | 24576 (0x6000) | 27000 (0x6978) |

N / CTS are fixed for a 27 MHz-based clock regardless of the video clock, as src.

Smaller tables [R: `.data`]:
- `_gu8AviCmrTbl` (30 B): 48 48 a8 a8 a8 a8 a8 a8 a8 a8 a8 a8 a8 a8 a8 a8 a8 a8 a8 a8 a8 a8 00 00 00 00 00 00 00 00
- `_gu8AviVicTbl` (30 B): 02 11 04 13 20 21 22 1f 10 00 00 00 60 61 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00
  — 30 entries, the count of mhal timing ids 0–0x1d, and the values fit the mhal timings in order (0 → VIC 2, 1 → 17,
  2 → 4, 3 → 19, 4–8 → 32 / 33 / 34 / 31 / 16, 0xc / 0xd → 96 / 97); the new ids 0x16–0x1d have 0 in both tables [R values;
  I: the index; the function that reads the two tables is not named in the sources].
- `_gu8AudChStatus3` (8 B): 02 03 00 02 08 0a 0c 0e
- `HDMITX_IEEEHdmiIdTbl` (3 B) and `VSDB_HDMI_IEEE_ID` (3 B): 03 0c 00; `HFVSDB_HDMI_IEEE_ID` (3 B): d8 5d c4
- `HDMITX_EdidHeaderTbl` (8 B): 00 ff ff ff ff ff ff 00
- `HDMITX_SPDData` (25 B): 53 49 47 4d 41 20 20 20 48 44 4d 49 54 58 20 54 58 20 44 45 4d 4f 20 20 20
  (`"SIGMA   HDMITX TX DEMO   "`)
- `NULL_PACKET` (12 B): 00 00 00 00 80 00 00 00 00 00 00 00

### 4.4 The DTOP timing generator (new on M6) [R: `MHal_HDMITx_DumpAtopTable`, `_MHal_HDMITx_DumpRegTab`, `.data`; dump]
<!-- ev:mhal-hdmitx.4.4 -->
`MHal_HDMITx_DumpAtopTable(timing, depth, b420, bCommon)` — despite the name it writes the DTOP table of
`stHAL_HDMITX_TOP_TBL` (.data+0x2470: two descriptors `{u8 *rows, u16 nrows, u32 type}`): [0] →
`ST_INIT_HDMITX_DTOP_TIMING_TBL` (13 rows, per timing), [1] → `ST_INIT_HDMITX_DTOP_TIMING_COMMON_TBL` (7 rows, common).
`bCommon` = 1 → only the common table; 0 → only column `c` of the per-timing table. Rows `{addr[3] big-endian byte
address, mask, value per column}` are written by `_MHal_HDMITx_DumpRegTab` as byte writes. Column map (Utopia timing →
c): 3→0, 4→1, 5→2, 6→3, 9–13→4–8, 18→16, 19→17, 27→25, 28→23, 29→21, 30→19, 34→22, 35→20, 36→24, 37→13, 38→26, 39→12,
40→11, 41→10, 42→9, 43→14, 44→15, 45→18; any other (interlaced, 640x480, 2160p24 / 50 / 60, 4096x2160, 1280x720p24–30,
3D) → "Atop id fail", **nothing written**. Depth and `b420` are ignored. The 27 columns are in the order of DISP's ATOP
table (`mhal-disp.md` §7.4).

Per-timing table (all masks 0xff):
| reg | 480p60 | 576p50 | 720p50 | 720p60 | 1080p24 | 1080p25 | 1080p30 | 1080p50 | 1080p60 | 720p29.97 | 720p59.94 | 1080p29.97 | 1080p59.94 | 1440p30 | 1440p50 | 1440p60 | 2160p25 | 2160p30 | 2160p29.97 | 1024x768 | 1280x800 | 1280x1024 | 1366x768 | 1440x900 | 1680x1050 | 1600x1200 | 1920x1200 |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 1459:10L | 59 | 5f | bb | 71 | bd | 4f | 97 | 4f | 97 | 71 | 71 | 97 | 97 | e3 | 77 | e3 | 9f | 2f | 2f | 3f | 8f | 97 | ff | 6f | bf | 6f | 1f |
| 1459:10H | 03 | 03 | 07 | 06 | 0a | 0a | 08 | 0a | 08 | 06 | 06 | 08 | 08 | 0c | 0f | 0c | 14 | 11 | 11 | 05 | 06 | 06 | 06 | 07 | 08 | 08 | 0a |
| 1459:11L | 0c | 70 | ed | ed | 64 | 64 | 64 | 64 | 64 | ed | ed | 64 | 64 | db | db | db | c9 | c9 | c9 | 25 | 3e | 29 | 1d | a5 | 40 | e1 | dc |
| 1459:11H | 02 | 02 | 02 | 02 | 04 | 04 | 04 | 04 | 04 | 02 | 02 | 04 | 04 | 05 | 05 | 05 | 08 | 08 | 08 | 03 | 03 | 04 | 03 | 03 | 04 | 04 | 04 |
| 1459:13L | 3d | 3f | 27 | 27 | 2b | 2b | 2b | 2b | 2b | 27 | 27 | 2b | 2b | 4f | 4f | 4f | 57 | 57 | 57 | 87 | 7f | 6f | 8e | 97 | af | bf | c7 |
| 1459:15L | 05 | 04 | 04 | 04 | 04 | 04 | 04 | 04 | 04 | 04 | 04 | 04 | 04 | 09 | 09 | 09 | 09 | 09 | 09 | 05 | 05 | 02 | 02 | 05 | 05 | 02 | 05 |
| 1459:16L | 7a | 84 | 04 | 04 | c0 | c0 | c0 | c0 | c0 | 04 | 04 | c0 | c0 | 08 | 08 | 08 | 80 | 80 | 80 | 28 | 48 | 68 | 66 | 80 | c8 | f0 | 18 |
| 1459:16H | 00 | 00 | 01 | 01 | 00 | 00 | 00 | 00 | 00 | 01 | 01 | 00 | 00 | 02 | 02 | 02 | 01 | 01 | 01 | 01 | 01 | 01 | 01 | 01 | 01 | 01 | 02 |
| 1459:17L | 49 | 53 | 03 | 03 | 3f | 3f | 3f | 3f | 3f | 03 | 03 | 3f | 3f | 07 | 07 | 07 | 7f | 7f | 7f | 27 | 47 | 67 | bb | 1f | 57 | 2f | 97 |
| 1459:17H | 03 | 03 | 06 | 06 | 08 | 08 | 08 | 08 | 08 | 06 | 06 | 08 | 08 | 0c | 0c | 0c | 10 | 10 | 10 | 05 | 06 | 06 | 06 | 07 | 08 | 08 | 09 |
| 1459:18L | 24 | 2c | 19 | 19 | 29 | 29 | 29 | 29 | 29 | 19 | 19 | 29 | 29 | 32 | 32 | 32 | 52 | 52 | 52 | 23 | 1c | 29 | 1b | 1f | 24 | 31 | 2a |
| 1459:19L | 03 | 6b | e8 | e8 | 60 | 60 | 60 | 60 | 60 | e8 | e8 | 60 | 60 | d1 | d1 | d1 | c1 | c1 | c1 | 22 | 3b | 28 | 1a | a2 | 3d | e0 | d9 |
| 1459:19H | 02 | 02 | 02 | 02 | 04 | 04 | 04 | 04 | 04 | 02 | 02 | 04 | 04 | 05 | 05 | 05 | 08 | 08 | 08 | 03 | 03 | 04 | 03 | 03 | 04 | 04 | 04 |

Common table (mask 0xff, value 0): 1459:12L, 12H, 13H, 14L, 14H, 15H, 18H.

Register meaning [R values; I meaning, checked on every column against the CEA / VESA totals]:
| reg | value (1080p60 / 2160p30) | meaning |
|---|---|---|
| 1459:10 | 0x0897 / 0x112F | Htotal − 1 |
| 1459:11 | 0x0464 / 0x08C9 | Vtotal − 1 |
| 1459:12 | 0 (common) | Hsync start |
| 1459:13L | 0x2B / 0x57 | Hsync end (width − 1) |
| 1459:14 | 0 (common) | Vsync start |
| 1459:15L | 0x04 / 0x09 | Vsync end (width − 1) |
| 1459:16 | 0x00C0 / 0x0180 | H active start (sync + back porch) |
| 1459:17 | 0x083F / 0x107F | H active end |
| 1459:18L | 0x29 / 0x52 | V active start |
| 1459:19 | 0x0460 / 0x08C1 | V active end |

So the HDMI-TX block has its own timing generator, which must match DISP's TGEN (`mhal-disp.md` §7.3); a USER timing
from DISP has no DTOP column and leaves these registers at the last table value [I].

### 4.5 Sequences
- **Init** (`MDrv_HDMITx_Init`, from `MApi_HDMITx_Init` on query INIT): MMIO bases → `SetIOMapBase`; `Power_OnOff(1)`
  (1459:00 bit 0 := 1; 145A:33 bit 0, 145A:34 bit 0 := 0; 145A:35[4:0] := 0); once: `MDrv_HDMITx_InitSeq` =
  `MHal_HDMITx_InitSeq` — `DumpAtopTable(3, 4, 0, 1)` (Utopia 720x480p, 24 bit, common table only; src passes the same
  arguments), 145A:19 := 0x1B1A and 145A:1A := 0x1A19 if zero, 145A:38[7:0] := 0x1A if zero, **1459:1F |= 1, then |= 2**
  (new) — then `MDrv_HDMITx_InitVariable` (src, §1.3) and `Init_PadTop` (empty); event group "HDMITx_Event"; task
  "HDMITx_Task" (§5.3) and a wait in 10 ms steps until the task has started (**new handshake**); DDC: `gbSwI2cEn` = 1 → the
  two DDC GPIOs as outputs driven high (§6.1). **No interrupt is attached**: src's `HDMITX_ISR_ENABLE` path is not built;
  `MHal_HDMITx_Int_*` are empty stubs, `Int_Status` returns 0.
- **Video mode** (`MDrv_HDMITx_SetVideoOutputMode(t)`, from `SetVideoOutputTiming` and the FSM): store t; AVI packet
  parameters; HDCP Ri-check period `gHDCPCheckRiTimer` = 5400 (24 / 25 Hz timings), 4300 (1080p30, 2160p30,
  **2160p29.97**), else 500 [R; src's "else" value is `HDCP_RiCheck_Timer`]; then `MHal_HDMITx_SetVideoOutputMode(t, csc,
  depth, &analog, ssc)`:
  1. HDMI 2.0 test (in the HAL function): Utopia 0x14, 0x15, 0x19, 0x1A (2160p50 / 60, 4096x2160p50 / 60) always,
     except with YUV420 output at 24 bit; with a colour depth other than NoID / 24 bit (0 / 4) also 0x10 (1280x1470p60),
     0x11–0x13 (2160p24–30), 0x16–0x18 (4096x2160p24–30) and **0x2D (2160p29.97, new)** [R decompile, mask 0x200001cf
     over t − 0x10]. The driver's `MDrv_HDMITx_CheckOver3G` (> 3.4 Gbps) uses almost the same set: 0x14, 0x15, 0x19, 0x1A
     always unless the output is YUV420 (then never), and with a depth other than 0 / 4 also 0x11–0x13, 0x16–0x18 and 0x2D —
     without 0x10 [R decompile, mask 0x100000e7 over t − 0x11].
  2. TMDS clock (§4.3), flag "148–165 MHz"; sync polarities from `HDMITxVideoModeTbl` (1459:0F).
  3. **No SSC call** (src calls `EnableSSC`, which is empty here anyway).
  4. The DTOP timing column (`DumpAtopTable(t, depth, b420, 0)`, §4.4) unless HDMI 2.0, which prints "Hdmi20 not support"
     and writes no column — so 2160p50 / 60 cannot be output.
  5. `SetAnalogDrvCur(&analog)` (§4.6); then `SendPacket(AVI, cyclic)` (§8).
  On the camera, 4K60 gave no signal on a monitor that takes 4K60 over HDMI 2.0, and 2160p29.97 gave no signal either [R
  camera run, 2026-10-19; I there: a TX limit of 4K30]. For 2160p29.97 the static code writes DTOP column 18 and treats it
  as HDMI 2.0 only above 24 bit; why it shows no picture is not known (its mode-table row has AVI C/M/R 0x00 and VIC 0,
  §4.3) [R table values].
- **TMDS / video / audio / colour / mode**: `SetTMDSOnOff` 145A:16[7:0] := 0xF0 / 0; `SetVideoOnOff` (1459:05 channel-mute
  bits and 1459:01 bit 13); `SetAudioOnOff` (1459:01 bit 12); `SetColorFormat(csc)` (1459:06 := 4 R→Y or 0); `SetHDMImode`
  (1459:01 bit 0); `SetAudioFrequency(f, ch, coding)` (layout, sub-packets, channel-status bytes 0x42–0x47, CTS / N from
  `TxAudioFreqTbl`, §4.3) — all equal to src, as are `ColorandRange_Transform`, `Power_OnOff` and `PKT_Content_Define`.
- **Packets**: §8.
- **Exit** (`MApi_HDMITx_Exit_U2`, new body): `MDrv_HDMITx_DeInit` (NOSRC: stop the task, delete the event group, clear the
  created flags), `Power_OnOff(0)` (1459:00 bit 0 := 0, 145A:33 / 34 bit 0 := 1, 145A:35 := 0x1F), bInit := 0, destroy the
  instance mutex.

### 4.6 Analog drive current [R `MHal_HDMITx_{Set,Get}AnalogDrvCur`, `MDrv_HDMITx_AnalogDrvCur`, sysfs `drvcurcfg`]
Config (11 B): Tap1Ch0..3, Tap2Ch0..3 (6 bit each), PdRtermCh, PdLdoPreDrvCh, PdLdoMuxCh (4 bit each).
`MDrv_HDMITx_AnalogDrvCur` copies them to the driver's analog struct (+10…+0x14 of the 0x15-byte struct at driver state
+0x6d; ctx+0x77…0x81) and calls `SetAnalogDrvCur`, which — **only if at least one of the 11 bytes is non-zero** — writes
145A:22 = Tap1Ch1<<8 | Tap1Ch0, 145A:23 = Tap1Ch3<<8 | Tap1Ch2, 145A:24 = Tap2Ch1<<8 | Tap2Ch0, 145A:25 = Tap2Ch3<<8 |
Tap2Ch2 (mask 0x3F3F), 145A:32[11:0] = PdLdoMuxCh<<8 | PdLdoPreDrvCh<<4 | PdRtermCh. `GetAnalogDrvCur` reads them back.
The same struct is re-applied on every `SetVideoOutputMode`. The driver state starts zeroed and nothing on the product sets
it (mi_hdmi's ioctl 0xc, the only other caller besides sysfs, is unused), so on this camera the drive current is whatever
DISP's per-timing ATOP table wrote (0x2C… for the 1440p / 2160p columns, `mhal-disp.md` §7.4) [R callers; I: "nothing sets
it" = no caller found besides ioctl 0xc and sysfs]. "Pd" = power-down masks per TMDS channel (bit n = channel n) [I from
the names]. An all-zero config cannot be written.

### 4.7 Clock [R `MHal_HDMITx_{SetClk,GetClk,GetDefault*}`, `_DrvHdmitxIfSetClkOn/Off`, `DrvHdmitxOsSetClkOn/Off`, DT]
One clock (src: three): `CLK_disp_432`, CLKGEN 1038:48 high byte — bit 8 gate (1 = off), bits 11:10 mux (0 = 432 MHz);
the DT `hdmitx` node's `clocks = <&CLK_disp_432>` (reg 0x1f207120, mux-shift 10, gate-shift 8). `SetClk` / `GetClk` handle
one clock only (size 1); `GetClk` reports 432 000 000 for mux 0, else 0.
`_DrvHdmitxIfSetClkOn` reads the current state through the HAL, then calls `DrvHdmitxOsSetClkOn(u8 *pbEn, u32 *pRate,
n)`: exactly one DT clock (n must be 1) — DT clock 0 (`of_clk_get`), `clk_round_rate(432 MHz)`, `clk_set_rate` with the
rounded rate + 1 MHz, prepare + enable only when the caller's "currently enabled" byte is 0, `clk_put` (src: a clock-mux
branch and an unconditional enable). When that succeeds (returns 1) it **also** writes the register through the HAL
(`SetClk`: bit 8 := 0, mux := 0); when the OS path fails (no DT node / clock) it returns success without touching the
register (an inverted test in the vendor code). `_DrvHdmitxIfSetClkOff` / `DrvHdmitxOsSetClkOff` mirror it with en = 0
(disable + unprepare only when the byte is 1; src unconditional). The same clock is DISP's `disp_432` (`mhal-disp.md` §10.1), so
it is already running when DISP is up [R DT; I: the sharing consequence].

### 4.8 Empty or constant HAL functions on M6
Empty: `HDCP2TxInit/EnableEncryptEnable/FillCipherKey`, `SetChipVersion`, `Int_Disable/Enable/Clear`, `SetHDCPConfig`,
`VideoInit`, `AudioInit`, `SetAudioSourceFormat`, `MuteAudioFIFO`, `SetHDCPOnOff`, `EnableSSC`, `Init_PadTop`. Constant:
`Int_Status` 0, `GetM02Bytes` 0, `GetAudioCTS` 0, `RxBypass_Mode` / `Disable_RxBypass` 0, `IsSupportDVIMode` 1,
**`IsSupportHDCP` 0**, `CSC_Support_R2Y` 1, `GetGC_ColorDepth` 4, `GetClkNum` 1, `GetDefaultClkRate` 432 MHz,
`GetDefaultClkMuxAttr` 0. Flags: `SetSCDCCapability(b)` → `g_bSupportSCDC`; `DebugEnable(b)` (new) → HAL debug flag
(.bss 0x16a24, the `DBG_HDMITX` enable); `SetHpdDetect(b)` (new) → `g_bHpdDetect`.

## 5. Hot-plug detection, the HDMITx task and the output state machine [R]

### 5.1 Rx status [R `MHal_HDMITx_GetRXStatus`, `MDrv_HDMITx_GetRxStatus`, DT]
`GetRXStatus()` returns `MsHDMITX_RX_STATUS`: 0 DVIClock_L_HPD_L, 1 DVIClock_L_HPD_H, 2 DVIClock_H_HPD_L, 3
DVIClock_H_HPD_H. "DVI clock high" = `(145A:18 & 0xCCCC) == 0` (Rx-sense of the TMDS lines); HPD = the GPIO value of the
HPD pin (DT `hpd_gpio` = 131 = `PAD_HDMITX_HPD`, GPIO register 103E:6A: bit 0 = in, bit 1 = out, bit 2 = OEN [S OpenIPC
mercury6 `mhal_gpio.c`]; requested as input at probe). **New on M6:** when `g_bHpdDetect` = 0 the function returns 3
without reading anything ("always connected"); the default is 1 (.data), changed only by sysfs `forcecfg hpddetect`
(§3.4, §9.2). mi_hdmi's "connected" (`GetSinkInfo` type 3) = `GetRXStatus() == 3`, read live on every call.

### 5.2 Where the events come from [R `MDrv_HDMITx_Exhibit`, `_MDrv_HDMITX_CheckRx_TimerCallback`, `_HPD_Isr`, `_DrvHdmitxOsTimerNotify`, `_DrvHdmiTxUpdateHpdIrq`]
<!-- ev:mhal-hdmitx.5.2 -->
No hardware HDMI interrupt exists in this build: the DT node has no `interrupts`, `HDMITX_ISR_ENABLE` is off, and
`_DrvHdmitxGetIrqNum` is never reached on the product path [I]. Two sources feed the event group "HDMITx_Event":
- **Rx-check timer** (created by the first `Exhibit`, i.e. the first `MhalHdmitxSetAttrEnd`): CamOs timer, period 100 ms,
  re-armed by its own notify while running; the callback reads `GetRXStatus()` into a cache and posts **0x04 (RXTIMER)** —
  every tick when `hpd_irq` is off, only on "HPD low" when it is on. `TurnOnOff(0)` (ATTR_BEGIN) stops it; the next
  `Exhibit` restarts it.
- **HPD GPIO interrupt** (`_HPD_Isr`, posts **0x80 (HPD_ISR)**): requested by `Exhibit` through `DrvHdmitxOsGpioIrq`
  (rising edge, name "HPD_ISR") only when the DT has `hpd_irq` ≠ 0. The camera's DT has no `hpd_irq`, so this path is off
  and hot-plug is polled every 100 ms [R: DT node `hdmitx`].

Other event bits: 0x01 RUN (posted by the handler itself and by `Exhibit`), 0x02 IRQ (never posted on M6), 0x08–0x40
CEC / HDCP events (never posted: their senders are absent or unreached) [R; I "never posted" = no poster found].

### 5.3 Task and handler [R `_HDMITx_Task`, `HDMITx_Handler` (switch arms read in the disassembly), `MDrv_HDMITx_Exhibit`]
`HDMITx_Task` (kernel thread via `DrvHdmitxOsCreateTask`, name "HDMITx_Task"): sets the "started" flag that `Init` waits
for; loops until `kthread_should_stop` (**new**, `DrvHdmitxOsTaskShouldStop`): wait for any of the 8 event bits,
OR-and-clear, **timeout 500 ms** (src 5000 ms + a sleep); take the driver semaphore; `HDMITx_Handler(events)`; release.
**`HDCPTx_Handler` is not called** (src calls it on HDCP events) — it has no caller at all in this build.

`HDMITx_Handler(events)` (nothing if the Rx-bypass flag is set):
1. events & (0x04 | 0x80): status := live read (0x80) or the timer's cached value (0x04). If it differs from the stored
   previous status: status ∈ {1, 3} (HPD high) → `bCheckEDID` = 1, FSM := **VALIDATE_EDID (1)**, edid_ready := 0, TMDS flag
   := (status == 3), clear the auth / EDID bytes, `MDrv_HDMITx_SetTMDSOnOff(1)`; else → FSM := PENDING (0), edid_ready := 0,
   clear the HDCP flags, `SetTMDSOnOff(0)`. Store the status, post RUN. Equal status and RUN set → re-post RUN. (src also
   stops HDCP here — `SetRxValid(0)`, `StopRiTimer`, `HdcpSetEncrypt(0)`; removed in this build.)
2. else events & 0x01 (RUN), only while the stored status is 1 or 3, by FSM state:
   - PENDING → stay.
   - **VALIDATE_EDID** → clear 8 EDID-result bytes (new), `MDrv_HDMITx_EdidChecking()` (§6.3); OK →
     `SetSCDCCapability(sink SCDC)`, edid_ready := 1, `MDrv_HDMITx_SetVideoOutputMode(current timing)` (§4.5), FSM :=
     TRANSMIT; if HDCP is off → `SetHDCPOnOff(0)`; failed → `SetSCDCCapability(0)`, FSM := PENDING; post RUN.
   - **TRANSMIT** → `SetHDMITxMode(mode)`, `SetColorFormat(in, out)`, `SetAudioFrequency(f)`, Rx-valid := 1; if the HDCP
     flag (output-mode bit 0) is set and the un-HDCP control is not NORMAL (INIT sets BLUE_SCREEN) → AV-mute on, video and
     audio off, auth start := 1, post 0x10 (HDCP run) — which nothing handles, so **an HDCP output mode stays muted for
     good**; otherwise video and audio on, AV-mute off if it was on; FSM := DONE, post RUN [R disassembly 0x3b5f6–0x3b686].
   - **DONE** → post RUN; once, a debug print of the phase times (`HpdTIme=%d, EdidTime:…TransmitTime…DoneTime`, and
     `TMDS=%x`; lines newer than src).
   - PRE_TRANSMIT → nothing.
3. else events & 0x02 (IRQ): `Int_Status()` (always 0) — dead.

`MApi_HDMITx_GetFsmStatus`: 0 PENDING, 1 VALIDATE_EDID, 3 DONE, else 2 TRANSMIT (FSM 4 = DONE).

### 5.4 `Exhibit` and the start sequence
`MDrv_HDMITx_Exhibit` (from ATTR_END): HDCP flag := output-mode bit 0; 4:2:0 / 4:2:2 deep-colour clamp for the 1920x2205,
1280x1470 and 4K timings (as src); `SetDeepColorMode`, `SetColorFormat`, `SetHDMITxMode`, `SetVideoOutputMode`,
`SetAudioFrequency`; if not authenticated: unless the sink is already validated (edid_ready and status 3), FSM :=
VALIDATE_EDID; the first call creates the Rx timer (+ the HPD IRQ if enabled), later calls restart it (and re-arm the IRQ,
posting 0x80 if HPD is high); then video / audio on when authenticated, AV-mute release [R].

The resulting sequence for mi_hdmi's Start (`SetAttrBegin`, `SetAttr`, `SetAttrEnd`, `SetMute` AV on, `SetSignal` on,
200 ms, audio unmute — `mi_disp-mi_hdmi.md` §2.3): ATTR_BEGIN stops the timer and turns TMDS off; ATTR programs mode / colour
/ timing; ATTR_END restarts the timer and polls the FSM ≤ 500 ms for DONE; on the next timer tick (≤ 100 ms) the handler
sees "connected", reads the EDID over DDC (§6), programs the timing column and drive current, sends AVI, turns video /
audio on [I: the composed order; each step R].

What is live on this build: polling of 145A:18 and GPIO131 every 100 ms (the GPIO interrupt only with `hpd_irq`); on
HPD high the EDID read, DTOP timing column, AVI and TMDS on; on HPD low TMDS off. HDCP, CEC, SCDC and the IRQ path do
nothing on this build [I from §5.2, §5.3, §7].

## 6. DDC and EDID [R]

### 6.1 DDC bus: software I²C on two GPIOs [R `_DrvHdmixUpdateSWDdcGpio`, `i2c_*`, `DrvHdmitxOs{Set,Get}GpioValue`, DT]
DT `hdmitx`: `i2c_sw = 1`, `i2c_sda_gpio = 132` (`PAD_HDMITX_SDA`, GPIO register 103E:6B), `i2c_scl_gpio = 133`
(`PAD_HDMITX_SCL`, 103E:6C), `i2c_id = 0` (adapter for the unused hardware-I²C path) [R DT; S pad / register names from
OpenIPC mercury6 `gpio.h` / `mhal_gpio.c`]. The HAL keeps the pins in a small `.data` block (.data+0x2464: u32 delay
count, u8 SCL pin, u8 SDA pin, u8 sw-enable = 1, RIU base); its compiled-in defaults 87 / 88 are the **i2m** pad numbers
and are replaced at probe [R values; I: i2m origin, from the i2m GPIO table where `PAD_HDMITX_SCL/SDA` = GPIO87/88].
Bit-bang: `SetGpioValue(pin, v)` = `camdriver_gpio_direction_output(pin, v)` (driven push-pull, high included — no
open-drain emulation); `GetGpioValue(pin)` = `direction_input` + `gpio_get`. Each half-bit = `i2c_Delay`, a busy loop of
`u32DDCDelayCount` iterations. `MDrv_HDMITx_InitVariable` calls `MHal_HDMITx_AdjustDDCFreq(100 kHz)`; with
`DrvHdmitxOsGetCpuSpeedMhz()` hard-wired to 1000 the src formula `cpu·100/S − ((ParamBase1(cpu≥1000: 150) + 250 (S ≥ 100)
+ adj) − S/adj) + (1 << ((440 − S)/40))`, adj = 10000/cpu = 10, gives **856 iterations** (initial value 1200) [R formula
and constants; I: the arithmetic]. `i2c_Start/Stop/SendByte/ReceiveByte/AccessStart/BurstReadBytes/BurstWriteBytes` are
src (interrupts disabled around each byte; ACK = SDA read after the 9th clock; "No ACK" prints). `i2c_SendByte` releases
SDA (input) instead of driving 1 for a trailing 1-bit (compiler-merged, same bus effect).
Hardware-I²C path (`i2c_sw` = 0): `DrvHdmitxOsSetI2cAdapter(i2c_id)` + `i2c_transfer` reads / writes (src) — unused here.

### 6.2 EDID read [R `MHal_HDMITx_EdidReadBlock`, `_MHal_HDMITx_EdidSetSegment`, `_MHal_HDMITx_EdidReadBytes`]
Block n: for n ≥ 2 first write the segment pointer n>>1 to I²C address 0x60 (E-DDC); then burst-read 128 bytes from 0xA0
at offset (n & 1)·128. Returns 1 on success.

### 6.3 EDID check (`MDrv_HDMITx_EdidChecking`) [R; src plus the listed changes]
Runs only while `bCheckEDID` = 1 (set by the HPD-high transition, §5.3): `_MDrv_HDMITX_InitEdidField` clears the parsed
EDID fields (§6.4); output-mode bit 1 is cleared (HDMI → DVI until a VSDB is found); block 0 is read with 3 tries 100 ms
apart (failure → 128 zero bytes, "Check EDID fail"); header `00 FF FF FF FF FF FF 00` (`HDMITX_EdidHeaderTbl`) else
"invalid EDID header", return 0 (FSM → PENDING); block 0 and the manufacturer id are stored; extension count = byte 0x7E
capped at 4 ("> 4" printed); each extension block is read the same way, block 1 stored, each parsed by
`_MDrv_HDMITx_ParseCEAExtEdid` (§6.5). **New:** if the output colour is forced (`ForceHDMIOutputColorFormat`, §3.3) it
overrides the EDID result ("ForceOututColor"); if the output mode is forced it replaces the EDID-derived mode
("ForceOututMode"), and a DVI mode clears the colour / packet fields. Then edid_ready := 1, `bCheckEDID` := 0,
`SetSCDCCapability(sink SCDC)`. mi_hdmi always passes HDMI + YUV444 (forced), so on the product the EDID only feeds
`GetSinkInfo` and the debug prints [I].
`MDrv_HDMITx_CheckEDIDTiming / ColorFormat / ColorDepth / CheckLegalTiming` (API `CheckLegalTiming`): src logic, extra
debug strings only; not on the mi_hdmi path. `_MDrv_HDMIRx_MapVideoTimingToVIC` exists unnamed (`hdmitx_nosrc_helper`, NOSRC
range of §1.2) with the new timings: 39→16, 40→34, 41→4, 42→62, 45→95; its error print reads "nonstandard video timing"
(src: "Error video timing").

### 6.4 EDID fields in the driver state [R decompile; field names S from src's struct `RxEdidInfo`]
State: the driver instance's private block (`*(inst + 8)`, part of `gstHdmitxResourcePirvate`); offsets are this build's.
`g_u8SupTimingNum` (count of SVDs collected so far) is a global at resource+0x2CB4+0x2B0, the debug word at +0x30.
| off | field | written by |
|---|---|---|
| +0x1C | `output_mode` (bit 1 = HDMI; cleared to DVI before parsing, §6.3) | CEA parser (HDMI VSDB found) |
| +0x40 / +0x44 | `input_color` / `output_color` (0 RGB444, 1 YUV422, 2 YUV444, 3 YUV420) | CEA parser (Y/C support bits) |
| +0x9C / +0x11C | `EdidBlk0[128]` / `EdidBlk1[128]` | `EdidChecking` |
| +0x19C | `ManufacturerID[3]` | `EdidChecking` |
| +0x19F | `CEADataBlkLen[8]` (index = CEA tag 0–7) | CEA parser |
| +0x1A7 | `PhyAddr[2]` | HDMI VSDB |
| +0x1A9 | `SupportHdmi` | CEA parser |
| +0x1AA / +0x1AB | `HF_VSDBVerInfo` / `MaxTmdsCharRate` (kept as the maximum seen) | HF-VSDB, HDMI VSDB |
| +0x1AC…+0x1B1 | `SupportIndependView`, `SupportDualView`, `Support3DOsdDisparity`, `SupportLTEScramble`, `SupportSCDC` (+0x1B0), `SupportRR` (+0x1B1) | HF-VSDB byte 5 bits 2, 1, 0, 3, 7, 6 |
| +0x1B2 | `YUV420DeepColorInfo` (HF-VSDB byte 6 & 7) | HF-VSDB |
| +0x1B3 | Supports_AI (HDMI VSDB byte 5 bit 7) | HDMI VSDB |
| +0x1B4 / +0x1B5 / +0x1B6 | 3D_present / 3D_Multi_present (2 bits) / HDMI_3D_LEN (5 bits) | HDMI VSDB |
| +0x1B7 / +0x1B8 | `Support2D_50Hz` / `Support2D_60Hz` | CEA video block |
| +0x1B9 / +0x1BA | `Support3D_50Hz` / `Support3D_60Hz` (2D flag && 3D_present) | HDMI VSDB |
| +0x1BB (+0x1BC…+0x1BE cleared) | sink's highest colour depth: 4 = 24 bit, 5 = 30, 6 = 36, 7 = 48 | `_MDrv_HDMITx_ParseDeepColorInfo` (VSDB byte 5 bits 4/5/6) |
| +0x1BF / +0x1DF / +0x1FF / +0x21F / +0x23F | `AudioDataBlk[32]` / `VideoDataBlk[32]` / `VendorDataBlk[32]` / `SpeakerAllocDataBlk[32]` / `VESA_DTCDataBlk[32]` (only audio and video are filled by this parser) | CEA parser |
| +0x25F / +0x266 | `HdmiVICList[7]` / HDMI_VIC_LEN | HDMI VSDB |
| +0x267 | `SupVidTiming[32]`, 12 bytes each: +0 VIC (SVD & 0x7F), +4 3D-structure mask | CEA video block, HDMI VSDB |
| +0x3E7 / +0x407 | `HDMI_VSDB[32]` / its length | CEA parser |
| +0x408 / +0x428 | `HF_VSDB[32]` / its length | CEA parser |
| +0x42A / +0x44A | `YCbCr420VidDataBlk[32]` / `YCbCr420CapMapDataBlk[32]` | extended tags 14 / 15 |
| +0x46A / +0x46B / +0x46C | `ExtColorimetry` / `bSupportHDR` / `HDRStaticDataBlk[32]` | extended tags 5 / 6 |
| +0x48C / +0x48D | `VideoCapDataBlk` (QY bit 7, QS bit 6) / `VSVideoBlk[32]` | extended tags 0 / 1 |

`_MDrv_HDMITX_InitEdidField` zeroes exactly these fields (blocks 0 / 1, every array, lengths and flags) before each check.
The query helpers `MDrv_HDMITx_Get{RxDCInfo,RxVideoFormat,VICList,DataBlockLength,RxAudioFormat,Rx3DStructure,ColorFormat,
EDIDPhyAdr,RxIDManufacturerName}FromEDID` and `EdidGetHDMISupportFlag` / `GetEdidRdyFlag` copy these fields out unchanged.

### 6.5 The EDID parsers [R decompile of the binary]
**`_MDrv_HDMITx_ParseCEAExtEdid(inst, block)`** (each extension block; returns 0 = invalid): block[0] must be 0x02 and
block[1] (revision) < 4; d = block[2] must not be 0–4 or 0xFF. From index 4 while index < d, header byte h: tag = h >> 5,
len = h & 0x1F; **len 0 → skip one byte** (new); else by tag —
- 1 audio: copy len bytes to `AudioDataBlk`, `CEADataBlkLen[1]` := len;
- 2 video: append to `VideoDataBlk` at `g_u8SupTimingNum`, each SVD's VIC = byte & 0x7F into `SupVidTiming[n].VIC`; VIC ≤
  16 or 35 / 36 → `Support2D_60Hz`; else VIC ≤ 31 or 37–39 → `Support2D_50Hz`; `g_u8SupTimingNum` += len;
  `CEADataBlkLen[2]` := len;
- 3 vendor: IEEE `03 0C 00` → `SupportHdmi` := 1, HDMI VSDB parser; on success store the VSDB (≤ 32 bytes) and its length,
  `output_mode` |= 2, then by block[3] & 0x30 (sink YCbCr support): 0x00 → if input is RGB or YUV444, output := RGB; 0x10 →
  if input YUV444, output := YUV422; 0x20 / 0x30 → if input RGB or YUV444, output := YUV444. Any other IEEE → HF-VSDB parser
  (which checks `D8 5D C4`); on success store HF-VSDB + length, `SupportHdmi` := 1. `CEADataBlkLen[3]` := len;
- 7 extended: code = next byte — 0 video capability (len must be 2, else return 0) → `VideoCapDataBlk`; 1 VS video →
  `VSVideoBlk`; 5 colorimetry (len must be 3, else return 0) → `ExtColorimetry` (the second payload byte, metadata, is
  skipped); 6 HDR static metadata → `HDRStaticDataBlk`, `bSupportHDR` := 1; 14 YCbCr420 video → `YCbCr420VidDataBlk`; 15
  YCbCr420 capability map → `YCbCr420CapMapDataBlk`; others skipped. **Every extended-tag copy is capped at 0x20 bytes**
  (src has no cap). Then `CEADataBlkLen[7]` := len;
- 0, 4, 5, 6 (reserved, speaker allocation, VESA DTC, reserved): skipped, `CEADataBlkLen[tag]` := len.

Detailed timing descriptors after d are not parsed. Returns 1.

**`_MDrv_HDMITx_ParseHdmiVSDB(inst, p, len, nSVD)`** (p at the IEEE id, len = block length): PhyAddr := p[3], p[4]. If
len > 5: Supports_AI := p[5] bit 7; deep colour from p[5]. If len > 7 and p[7] bit 5 (HDMI_Video_present): L = latency
bytes (p[7] bits 7:6 = 11 → 4, 01 → 2, else 0); `MaxTmdsCharRate` := max(itself, p[6]) (only on this path); if L + 9 < len:
f = p[L+8], v = p[L+9]; if f bit 7 or v >> 5: 3D_present := f >> 7, 3D_Multi_present := f bits 6:5, HDMI_3D_LEN := v &
0x1F, HDMI_VIC_LEN := v >> 5; 3D_present with `Support2D_60Hz` / `_50Hz` sets `Support3D_60Hz` / `_50Hz`; the VIC_LEN HDMI
VICs are copied to `HdmiVICList` (when L + 9 + VIC_LEN < len) and L += VIC_LEN. 3D_Multi_present 1 (len > L + 11): the
16-bit 3D_Structure_ALL (p[L+10] << 8 | p[L+11]) goes into the 3D mask of the first min(nSVD, 16) SVD entries, 3D_LEN −= 2;
3D_Multi_present 2 (len > L + 13): ALL as above and 3D_MASK = p[L+12] << 8 | p[L+13]; entry i gets ALL if MASK bit i else
0, 3D_LEN −= 4. Then, while 3D_LEN ≥ 2 remain: entries from p[L+12] (multi 1), p[L+14] (multi 2) or p[L+10] (multi 0),
each byte = 2D_VIC_order (bits 7:4) | 3D_Structure (bits 3:0): structure 0–6 ORs bit (1 << s) into that SVD's mask, 8
(side-by-side half) ORs 0x100 and skips the following 3D_Detail byte, 7 and 9–15 are ignored. Returns 1.

**`_MDrv_HDMITx_ParseDeepColorInfo(inst, b5)`**: highest of DC_48bit (bit 6) → 7, DC_36bit (bit 5) → 6, DC_30bit (bit 4)
→ 5, none → 4 (24 bit), into +0x1BB; DC_Y444 (bit 3) is not looked at.

**`_MDrv_HDMITx_ParseHFVSDB(inst, p, len, nSVD)`**: p[0..2] must be `D8 5D C4` (else 0); version := p[3];
`MaxTmdsCharRate` := max(itself, p[4]) (× 5 MHz); flags from p[5] and p[6] as in the §6.4 table; returns 1 (prints only
with drv debug bit 0).

## 7. HDCP — present, unused and not functional [R]

What exists: the HDCP 1.4 transmitter logic of src `drvHDMITx.c` (`MDrv_HDCPTx_*`: init, the A0–A9 state machine in
`HDCPTx_Handler`, the Ri check timer `_MDrv_HDMITX_HDCP_Ri_TimerCallback` with `gHDCPCheckRiTimer` 500 / 4300 / 5400 ms
(§4.5), the repeater KSV-list read into `g_u8KSVList` (635 B), the SRM / revocation list check, `GetBksv` / `GetAksv` /
`GetHdcpKey`), and the API wrappers (`MApi_HDMITx_HDCP_*`, `HDCP2*` stubs). What makes it dead on M6:
- `HDCPTx_Handler` has **no caller** (src calls it from the task loop); HDCP events 0x08–0x40 are never handled (§5.2).
- The HAL HDCP functions are empty (`SetHDCPOnOff`, `SetHDCPConfig`, `HDCP2Tx*`), `IsSupportHDCP` returns 0, src's HDCP
  register bank (i2m 0x172B) has no M6 counterpart in the code, the i2m HDCP HAL files are not in the binary (§1.3), and
  the key read `MHal_HDMITx_HDCPKeyReadByte` is a tail jump to `MDrv_EEPROM_Read`, which returns 1 without reading anything
  (NOSRC, 4 bytes).
- `_HalHdmitxIfSetInitConfig` switches HDCP off and sets the un-HDCP behaviour to BLUE_SCREEN, so selecting an `_HDCP`
  output mode only mutes the output (§5.3).

## 8. Packets and infoframes [R; src code unless noted]

Packet types (`MsHDMITX_PACKET_TYPE`): 0x81 VS, 0x82 AVI, 0x83 SPD, 0x84 audio infoframe, 0x80+ others "not implemented";
general packets 0–10 (NULL, ACR, audio sample, GC, ACP, ISRC1/2, DSD, HBR, GM). `SendPacket(type, process)`: process 0x80
= stop (clear the type's `PKT_EN` bit, 1459:1B), else write header + payload + checksum and set the enable bit (cyclic) or
the ready bit in 1459:1D (single). Headers: VS `81 01 1B`, SPD `83 01 19`.

Byte placement (DTOP byte index = reg·2 + high byte) [R: `.data`]:
- `HDMITX_AviPktReg` (13 byte addresses): 34:H 35:L 35:H 36:L 36:H 37:L 38:L 38:H 39:L 39:H 3a:L 3a:H 3b:L — AVI checksum
  (PB0) at byte 0x68, PB1–13 at 0x69–0x76.
- `HDMITX_AdoPktReg` (6): 3c:H 3d:L 3d:H 3e:L 3e:H 3f:L — audio infoframe at bytes 0x78–0x7D.
- `HDMITX_SpdPktReg` (25): 42:H 43:L 43:H 44:L 44:H 45:L 46:L 46:H 47:L 47:H 48:L 48:H 49:L 4a:L 4a:H 4b:L 4b:H 4c:L 4c:H
  4d:L 4e:L 4e:H 4f:L 4f:H 50:L.
- `HDMITX_VsPktReg` (27): 54:H 55:L 55:H 56:L 56:H 57:L 58:L 58:H 59:L 59:H 5a:L 5a:H 5b:L 5c:L 5c:H 5d:L 5d:H 5e:L 5e:H
  5f:L 60:L 60:H 61:L 61:H 62:L 62:H 63:L.

AVI bytes (`SendPacket(0x82)`): 0x69 PB1 = Y (colour) << 5 | A (0x10, AFD valid) | S (scan); 0x6a PB2 = the C/M/R byte from
`HDMITX_AviCmrTbl[timing]`, aspect M overridden for 4:3 / 16:9 codes, R = AFD when the AFD override is on, C replaced by
the colorimetry field when it is not "no data"; 0x6b PB3 [6:4] extended colorimetry (≤ 6), [3:2] RGB quantisation (RGB
only); 0x6c PB4 = VIC from `HDMITX_AviVicTbl` (aspect 16:9 and timing 1–4 → VIC + 1; aspect 21:9 → VIC 93–97 + 10, timings
17–19 → VIC + timing + 0x56, timing 45 → VIC + 0x69 (new), else a 14-entry pair table, "Invalid VIC Code for 21:9"); 0x6d
PB5 [7:6] YCC quantisation, [3:0] pixel repetition (1 for interlaced timings below 3); checksum at 0x68.
`Set_AVI_InfoFrame(COLORIMETRY)` keeps a src bug: it uses the 16-bit `Mask_Write` with the byte indices 0x6a / 0x6b, so it
hits registers 0x6a / 0x6b, not AVI bytes PB2 / PB3 [R both].
`PKT_User_Define(_Clear)`, `PKT_Content_Define`, `InfoFrameCheckSum`, `SetGCPParameter`, `SetPktAttribute`,
`Set{AVI,VS,Audio}InfoParameter`, `Set_AVI_InfoFrame`: src code.
SPD default data `HDMITX_SPDData` (§4.3). NULL packet / ACR / audio sample / GC are enabled through `EnablePacketGen`
(1459:08 := 0x21), and the GC packet carries AV-mute and colour depth. mhal `SetInfoFrame` reaches only AVI, SPD and audio
IF; VS (HDMI VIC / 3D) is driven by the drv layer for the 4K VIC timings.

## 9. Module glue, device tree, sysfs, OS layer [R]

### 9.1 Platform driver and DT [R: `DrvHdmitxModule*`, `_DrvHdmitxModuleInit`, `_DrvHdmitxUpdate*`]
<!-- ev:mhal-hdmitx.9.1 -->
`DrvHdmitxModuleInit` registers `stDrvHdmitxPlatformDriver` (match `"sstar,hdmitx"`, `_HdmitxMatchTable`); probe takes the
msys sysfs class, stores the DT node, then `_DrvHdmitxModuleInit` (once): char device region "mhdmitx" (1 minor; the cdev
is never added — the device-node quirk of `mhal-core.md` §3.2), device "mhdmitx" in the msys class, the 5 sysfs files,
`of_find_compatible_node("sstar,hdmitx")`, and the DT reads below. Remove → `_DrvHdmitxModuleDeInit` (NOSRC:
reference-counted cdev / region removal, clears the class pointer) + `CamDeviceUnregister`. Suspend / resume: empty.
The node and its values on this camera are listed in `mhal-core.md` §7; what each property does here:
| DT property (node `hdmitx`) | read by | effect |
|---|---|---|
| `compatible` "sstar,hdmitx" | match | |
| `clocks` (`<&CLK_disp_432>`) | `DrvHdmitxOsSetClkOn/Off` (clk framework, `num_parents` / rate prints) | §4.7 |
| `i2c_id` | `_DrvHdmitxUpdateI2cId` | hardware-I²C adapter number (unused, §6.1) |
| `hpd_gpio` | `_DrvHdmitxUpdateHpdGpio` | HPD pin, requested as input; missing → "No hpd_gpio property" (§5.1) |
| `hpd_irq` | `_DrvHdmiTxUpdateHpdIrq` | 1 = GPIO interrupt on HPD in addition to the 100 ms timer; absent on this camera → 0 (§5.2) |
| `i2c_sw` | `_DrvHdmixUpdateSWDdcGpio` | 1 = software DDC on the two GPIOs below (§6.1) |
| `i2c_sda_gpio` / `i2c_scl_gpio` | same | DDC pins; a missing `i2c_scl_gpio` prints the SDA message and zeroes SDA (vendor bug) |

`interrupts` is not read (no HDMI IRQ); `_DrvHdmitxGetIrqNum` ("Can't Get SCL_IRQ") exists but is unreached [I].

### 9.2 sysfs (`/sys/class/<msys class>/mhdmitx/`, mode 0644) [R attribute table .data+0x1f18, handlers]
`DrvHdmitxSysfsInit` creates 5 attributes (src 3):
| file | write | read |
|---|---|---|
| `dbgmg` | `<hex level>` → `MhalHdmitxSetDebugLevel` | "dbglevle: %08x" |
| `ptgen` | `<en> <width>` → **raw write of `0x0003 | width<<8` (or 0) to 0xFD224A14 = RIU 1125:05**, the i2m DTOP pattern-generator register, copied from src's literal `HDMITXREG(0x112500, 0x05)`; on M6 bank 1125 is not HDMI-TX [R; I: the M6 equivalent would be 1459:05] | en, width |
| `clk` | `clktree <en> <rate>` → CLK_SET query (1 clock); usage text otherwise | CLK_GET: name, en, rate |
| `forcecfg` (new) | `outputmode <0/1> <hdmi/dvi>` or `hpddetect <0/1>` → FORCE_ATTR_SET (§3.4) | "Flag:%x, ForceOutputMode(%d, %s), HpdDetect:%x" |
| `drvcurcfg` (new) | 11 hex values `Tpa1Ch0..3 Tpa2Ch0..3 PdRtermCh PdLdoPreDrvCh PdLdoMuxCh` → ANALOG_DRV_CUR (the decompile shows all parsed into one local — a Ghidra artefact or a vendor bug, not settled) | ANALOG_DRV_CUR_GET: "Tap1(..) Tap2(..), RterM:, LdoPreDrvCh:, LdoMuxCh:" |

The new handlers are `_HdmitxForceCfg{Show,Store}`, `_HdmitxDrvCurCfg{Show,Store}` with their checkers
`check_hdmitxforcecfg_{show,store}`, `check_hdmitxdrvcurcfg_show`, `check_hdmitxdrvcucfgr_store` (NOSRC) [R].

### 9.3 OS layer [R `linux/drv_hdmitx_os.c` region, 44 SAME, read side by side]
<!-- ev:mhal-hdmitx.9.3 -->
CamOs wrappers as src: semaphores / mutexes (CamOs); event groups (spinlock + wait queue, `_HdmitxOs_EventGroup_Info`,
4 × 0x48 = 288 B, ids `0x7654xxxx`); the symbol `irqContext` (0x3d6bc) is a function — the deferred "set event" (OR the
bits into the group, `__wake_up`) used when an event is set from interrupt context; timers (`CamOsTimerInit/Add`,
re-armed in `_DrvHdmitxOsTimerNotify`); tasks (`CamOsThreadCreate`; new `DrvHdmitxOsTaskShouldStop` =
`CamOsThreadShouldStop`); GPIO (`camdriver_gpio_*`); GPIO IRQ (`DrvHdmitxOsGpioIrq`: `request_threaded_irq`, flags 1 =
rising edge, "HPD_ISR"); I²C adapter; clocks (§4.7); `GetCpuSpeedMhz` = 1000 (constant). Differences from src:
`DrvHdmitxOsDeleteTimer` uses `CamOsTimerDeleteSync` (src `CamOsTimerDelete`); `DrvHdmitxOsGetMmioBase` gives 0xFD000000
(type 0, the PM window) / 0xFD200000 (type 1), size 0x200000 for both (src 0x400000 for the PM window), and returns 1
without writing for an unknown type; `DrvHdmitxOsSetClkOn/Off` as in §4.7. Exported to other modules:
`DrvHdmitxOsMemAlloc/MemRelease/GetSystemTime`, which none of the camera's modules imports [R].

## 10. Other copy

The vendor U-Boot carries an older revision of this driver (0x23E34080–0x23E3E7A0, ≈ 42 KB): the same M6 HAL with fewer
timings, the ATOP tables still in the HDMI-TX HAL, an 8-byte drive current, 14 queries, the full HDCP 1.4 path and the
i2m HDCP HAL, and `*_if.c` wrapper layers of the same revision as this binary's. `bootlogo` drives it blind (no EDID, no
state machine). Described in `uboot.md` §8 and §5.3 [R].

## 11. What is not known

- **Debug prints in the product.** Whether mi_hdmi's debug level 3 (§2.1) really puts the MDrv HDMI prints (FSM state
  changes, "Hpd Detect", AnalogDrvCur) into the kernel log is not known (CAMERA-CHECKS K34).
- **Who drives the PHY.** That DISP's per-timing ATOP table is what sets the drive current on the running camera (§4.2,
  §4.6) follows from the code [I]; the live values of 145A:22–25 / 32 under the vendor stack are not known (CAMERA-CHECKS
  K32). The "Pd" fields as per-channel power-down masks are an inference from the names.
- **1459:1F.** The meaning of the two bits `InitSeq` sets (§4.2, §4.5) is not known (CAMERA-CHECKS K33).
- **HPD override.** Whether `forcecfg hpddetect 0` (§3.4, §5.1) makes the output come up without a sink is not known; it
  is a state-changing check (CAMERA-CHECKS P5).
- **2160p29.97.** Why this timing gives no signal on the camera (§4.5) is not known.
- **Register meanings.** The DTOP timing-generator meanings of 1459:10–19 (§4.4) are inferred from the values; the src
  field names of §4.2 are assumed to keep their meaning on M6 [I].
- **`drvcurcfg` parsing.** Whether the 11 values written to sysfs `drvcurcfg` reach 11 fields or one local (§9.2) is not
  settled.
- **`_gu8AviCmrTbl` / `_gu8AviVicTbl`.** Their index (the mhal timing id) is inferred from the values; their reader is not
  named (§4.3).
- **Coverage limits.** 22 of the 38 api / drv / if / ctx SAME functions (§1.3) were checked by their compiled constants only
  — every difference is of a kind seen in the 16 read ones — and not re-read line by line; a changed computed value inside
  them would not show. The other SAME functions were compared by constants or read (§1.3); the DIFF and NOSRC functions
  were read.
