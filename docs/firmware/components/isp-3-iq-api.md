# isp-3-iq-api — the ISP API ids of mhal.ko: routing, payloads, converters; the API bin
<!-- ev:isp-3-iq-api -->

The ISP API is the set of numbered controls ("API ids", also "Ctrl ID") through which user space reads and changes the ISP of
`mhal.ko`: the IQ ids 0x10xx (one struct per IQ function, turned into
ISP-register parameters by a converter per API slot), the AE / AWB / AF ids 0x14xx / 0x18xx / 0x1Cxx (state of the in-kernel
3A objects), the calibration ids 0x24xx, the bypass and AWB-calibration ids 0x28xx / 0x2Cxx, the CUS3A ids 0x2Exx (statistics
and results exchanged with a user-space 3A) and the 0x3000–0x3003 control words. This document holds every id with its
payload, the route it takes inside mhal, where its data lands and what reaches the registers, and the API bin — the file of
recorded Set calls (`/customer/iq_file_gc8613.bin`) the app loads at start.

Not here: the IQ engine that runs the converters' output (IQ context, API-slot and IP-slot machinery, the per-IP runner and
`IQ_Patch_*`, `IspRegSet*`, `Regmap` and the official IP-parameter names, the IQ file `iqfile<ch>.bin`, the built-in `iq_bin`)
— isp-4-iq-engine.md; the methods of the in-kernel AE / AWB / AF objects the 0x14xx / 0x18xx / 0x1Cxx ids feed — isp-5-3a.md;
libcus3a's id routing, libispalgo's acceptance of each id, the user-space 3A loop and `isp_api.xml` as a file — isp-6-userspace.md;
the ispmid channel data, the statistics engines and their payload formats, `/dev/isp_fe`, the CMDQ batches — isp-2-ispmid-mload-ispscl.md;
the ISP registers field by field and the bank map — isp-1-hal.md; libmi_isp's wrapper, its ioctls 0x10 / 0x11 and its user-space
API-bin and calibration loaders — mi-capture.md §5; prog_uvc's menu calls and its `-i` option — prog_uvc.md §6.2, §2.1.

**Conventions.** Origin tags: [R] read from the binary, its disassembly / decompile, emulation of its code, a data
section or a cited file; [S] stated by a source; [I] inference. A tag on a heading covers the section unless a line says
otherwise. Addresses are `.text` / `.data` / `.rodata` offsets of the relocatable module unless written otherwise. "ctx"
= the per-channel IQ context (the "IQ handle" the handlers receive, ≈0x58040 bytes; its regions: isp-4-iq-engine.md
§2.1); "ch" = the ispmid channel data (isp-2-ispmid-mload-ispscl.md §2.2); "h" = an in-kernel 3A object (AE or AWB);
IqInput = `g_tIspMidHandler`+0xe00, the IQ engine's input block. "API slot k" = one of the 53 API slots of the IQ
engine, dirty flag at ctx+0x535b8+4k, bypass word at ctx+0x5368c+4k. "IP n" = IP slot n of the per-IP runner
`hal_IQ_Set_ALL`; Register notation `BANK.rr[bits]` = RIU bank, 16-bit register rr (isp-1-hal.md, conventions); "1325.00
b4" = bit 4 of 1325.00. "auto[i].X" / "manual.X" = fields of an API struct; auto entry i is used at ISO index i, manual
in all 16 rows. "Our bin" = the camera's API bin `/customer/iq_file_gc8613.bin` (§6). Field maps tagged [R emulation]
were obtained by running the converter alone in an emulator of mhal.ko with every struct element at the bin's value
(else the middle of its XML range) and nudging each element by ±1: "copy" = destination bytes equal the element,
"derived" = computed; an element whose ±1 changes nothing (clamped, rounded away) was read from the code; the method
reproduces the hand-read maps of NR3D_EX, PFC_EX, HDR_EX, COLORTRANS, Saturation, CrossTalk, AntiFalseColor, OBC and
NRDeSpike exactly.

"row i" = the API-table row for ISO index i (16 rows) of that IP; its offsets are the IP-parameter offsets of the
register sets (`isp-4-iq-engine.md`), so a row offset names a cfg enum (0x178…0x40c) and through it a register.

## 1. The API in mhal.ko

### 1.1 Code [R]
- **Dispatch:** `MHalIspApiSet/Get(dev, chn, id, size, data)` (global mutex) → `MS_CAM_IspApiSet/Get` (§1.2) →
  `DrvAlgo_IF_ApiSet/ApiGet(id, AE handle, AWB handle, skip flag, IQ handle, size, data)` for the IQ, 3A and calibration ids,
  the `MS_CAM_*` / `Camera*` functions for the CUS3A ids.
- **Handlers:** ≈190 `DrvAlgo_IF_{IQ,AE,AWB,CALIv30}_Set/Get*`, called after `DrvAlgo_IF_ApiSet/ApiGet` has checked the size: each copies the payload into the
  algorithm's parameter block (or unpacks it into method calls of a 3A object) and marks the API for the next trigger — e.g.
  `DrvAlgo_IF_IQ_SetContrast`: `memcpy(iq + 0x3a62c, data, 0x4c); *(iq + 0x535ec) = 1`.
- **Converters:** `IQ_API20_SET_<X>`, one per API slot, run by the IQ engine (`DoIq` → `IQ_API20_Trigger_CHK` →
  `hal_IQ_API20_SET_ALL` → `IQ_API20_SET_<X>`), §3.
- **A kernel copy of the user library:** `MI_ISP_*` / `call_MI_ISP_*` (≈310 functions, 24 KB) with `MI_ISP_API_EXCUTE_CMD`,
  `MI_ISP_ALGO_API_CmdLoadBinFile` and `MI_ISP_API_CRC32` — byte-identical to the API-bin loader of libmi_isp (§6.2) — and
  `DrvAlgoLoadFlashApiBin`. Neither the loader nor `DrvAlgoLoadFlashApiBin` has a direct caller in mhal; whether either is
  reached indirectly is not known (likely dead [I]). prog_uvc loads its bin in user space.

### 1.2 Route of an id [R]
User space: `libmi_isp` → ioctl `mi_isp` 0x10 (Set) / 0x11 (Get) (mi-capture.md §4.6, §5.2) → `MHalIspApiSet/Get` →
`MS_CAM_IspApiSet/Get`:
1. an optional filter hook (`MS_CAM_RegIspCmdFilter` sets it) may consume the call (any result ≠ 0xF ends it); nothing on
   this firmware registers one (mi-capture.md §5.3);
2. id 0x2E08 (CUS3A enable, size 3) → `MS_CAM_ISP_SetCus3AEnable` → `CameraIsp_EnableCus3A` — handled before the channel check;
3. the channel must exist (`g_tIspMidHandler` slot `chn + 0x3c`, else "ISP channel [%d] have NOT been created"); ids
   0x1000–0x1FFF and 0x2400–0x2DFF → `DrvAlgo_IF_ApiSet/Get` with the AE handle (ch+0x28), the AWB handle (ch+0x120), the skip
   argument `(g_nIspSkipFuncFlag & 0x3f) >> 5` and the IQ handle; the call is made only when `g_nIspSkipFuncFlag` bit 5 is clear,
   so the skip argument is always 0 (§4.4). Both 3A handles exist even with a user-space 3A (`AeInit` / `AwbInit` run at open);
4. 0x2E01–0x2E28 → the `MS_CAM_*` / `Camera*` statistics and 3A-I/O functions (switch, §5), 0x3000–0x3003 → inline control
   words (§5.4);
5. anything else → "[ERROR - ISP API SET] Not support - Ctrl ID = %d" and 0xF.

### 1.3 Return codes and Get semantics [R]
<!-- ev:isp-3-iq-api.1.3 -->
| status | when |
|---|---|
| 0 | done |
| 5 | 0x1023 LinearityLUT Get (not supported) — handler level, see below |
| 6 | the algorithm object the id needs is null (0x14xx AE, 0x18xx / 0x240F / 0x2Cxx AWB, the other 0x24xx IQ; every AF id, §4.4) |
| 7 | 0x2E0E AF window with x1 ≤ x0 or y1 ≤ y0 |
| 0xD | 0x240F with a calibration version other than 3 or 4 |
| 0xE | payload size ≠ the size the kernel checks for that id ("Structure mismatch - Ctrl ID = %d") |
| 0xF | id not accepted ("Not support - Ctrl ID") |

Sizes are checked exactly, Set and Get alike (Set and Get sizes agree for every id that has both). **On the Get path
`MS_CAM_IspApiGet` discards the status of `DrvAlgo_IF_ApiGet`**, so every Get of an id in 0x1000–0x1FFF or 0x2400–0x2DFF returns
0 to the caller whatever the handler reported (5, 6 and 0xE included); the statuses of the CUS3A switch (§5) are returned as
they are [R: decompile of `MS_CAM_IspApiGet`].

### 1.4 What reaches the kernel on this camera [R routing; I where marked]
The app calls `MI_ISP_*` in libmi_isp, which offers every id first to libcus3a's agent; the agent's rule (by id range, and
whether the user-space algorithm is running) is isp-6-userspace.md §3. With it:
- **IQ ids 0x10xx:** all reach the kernel — the agent returns "not mine" and libmi_isp issues the ioctl. At boot the API bin
  (§6) sets 42 of them (every 0x10xx record of the bin except 0x103A), then prog_uvc's menu sets ColorToGray, Contrast,
  Brightness, Lightness, Saturation, Defog, Sharpness, WDR and HSV at run time (prog_uvc.md §6.2).
- **AE 0x14xx, AWB 0x18xx, AF 0x1Cxx, 0x3000 / 0x3001 / 0x3003, 0x2C03–0x2C06:** taken by the user-space 3A (libcus3a /
  libispalgo) once its algorithm runs; while it is not running, `Cus3A_Proc{Ae,Awb,Af}Ctrl` return −1 (→ 0xA0078008) and the call
  goes to the kernel. 0x2400–0x27FF other than 0x240F and 0x2C00–0x2DFF other than 0x2C03–0x2C06 always go to the kernel.
  The kernel's 3A objects exist, so such a Set is accepted and stored, but nothing consumes it: `nFlagCus3A` = 0x1A keeps
  `DoAe` / `DoAwb` from running and `DoAf` has no caller (§4.1).
- **CUS3A 0x2Exx:** sent by libcus3a / libispalgo every frame (the list: isp-6-userspace.md §4). prog_uvc contains
  `MI_ISP_CUS3A_*` wrappers for 0x2E04–0x2E06, 0x2E08–0x2E0A, 0x2E0E, 0x2E0F, 0x2E13–0x2E16, 0x2E20, 0x2E21 and
  `MI_ISP_AE/AWB_Get*HwStats` for 0x2E01–0x2E03, 0x2E07, 0x2E19, 0x2E1E. The user AF runs on this camera:
  `MI_ISP_EnableUserspace3A` calls `CUS3A_EnableUserspaceAF`, which registers libcus3a's built-in Sigma AF adaptor; on the
  kernel's AF event libispalgo's `AfInit` / `DoAf` run every frame — `AfInit` pushes AF window / filter defaults through
  0x2E0E–0x2E11 and 0x2E24–0x2E28, 0x2E21 is written every frame (no VCM, no effect), and the failing 0x2E20 read is ignored.
  The AWB ids are read and written every frame, but the built-in AWB algorithm (`SigmaAwbRun`) computes only every third call; the other two
  frames send the previous gains with Change = 0, which the kernel ignores (§5.1).

## 2. Every accepted id [R]
<!-- ev:isp-3-iq-api.2 -->

Decoded from the four dispatch switches and the decompile of `DrvAlgo_IF_ApiSet` /
`DrvAlgo_IF_ApiGet`. "size" = what the kernel checks (Set and Get agree); handler names
without the `DrvAlgo_IF_` prefix; "inline" = handled inside the switch; empty = not accepted in that direction. "used": `bin` =
a record of our bin (record length = size + 8 for a one-pointer record, §6.1), `app` = prog_uvc calls it directly. Names,
payloads and behaviour per class: §3 (0x10xx, 0x2802, 0x2803), §4 (0x14xx, 0x18xx, 0x1Cxx, 0x24xx, 0x2Cxx), §5 (0x2Exx,
0x30xx).

| id | size | Set → | Get → | used |
|---|---|---|---|---|
| 0x1001 | 12 | IQ_SetVersionInfo | IQ_GetVersionInfo | bin |
| 0x1002 | 4 |  | IQ_GetParaInitStatus |  |
| 0x1003 | 4 | IQ_SetFastMdoe | IQ_GetFastMdoe |  |
| 0x1004 | 4 | inline | inline | bin app |
| 0x1005 | 76 | IQ_SetContrast | IQ_GetContrast | bin app |
| 0x1006 | 76 | IQ_SetBrightness | IQ_GetBrightness | bin app |
| 0x1007 | 76 | IQ_SetLightness | IQ_GetLightness | bin app |
| 0x1008 | 26120 | IQ_SetGammaLUT | IQ_GetGammaLUT | bin |
| 0x1009 | 17416 | IQ_SetYUVGammaLUT | IQ_GetYUVGammaLUT | bin |
| 0x100a | 416 | IQ_SetSaturation | IQ_GetSaturation | bin app |
| 0x100b | 28 | IQ_SetDefog | IQ_GetDefog | bin app |
| 0x100c | 488 | IQ_SetRGBmatrix | IQ_GetRGBmatrix | bin |
| 0x100d | 144 | IQ_SetFalseColor | IQ_GetFalseColor | bin |
| 0x100e | 2116 | IQ_SetNR3D | IQ_GetNR3D | bin |
| 0x100f | 588 | IQ_SetNRDeSpike | IQ_GetNRDeSpike | bin |
| 0x1010 | 588 | IQ_SetNRLuma | IQ_GetNRLuma | bin |
| 0x1011 | 280 | IQ_SetNRChroma | IQ_GetNRChroma | bin |
| 0x1012 | 4904 | IQ_SetSharpness | IQ_GetSharpness | bin app |
| 0x1013 | 348 | IQ_SetCrossTalk | IQ_GetCrossTalk | bin |
| 0x1014 | 144 | IQ_SetOBC | IQ_GetOBC | bin |
| 0x1015 | 144 | IQ_SetOBC_P1 | IQ_GetOBC_P1 | bin |
| 0x1016 | 1640 | IQ_SetWDR | IQ_GetWDR | bin app |
| 0x1017 | 756 | IQ_SetDynamicDP | IQ_GetDynamicDP | bin |
| 0x1018 | 4940 | IQ_SetHSV | IQ_GetHSV | bin app |
| 0x1019 | 756 | IQ_SetRGBIR | IQ_GetRGBIR | bin |
| 0x101b | 400 | IQ_SetPFC | IQ_GetPFC | bin |
| 0x101c | 416 | IQ_SetDEMOSAIC | IQ_GetDEMOSAIC | bin |
| 0x101d | 28 | IQ_SetCOLORTRANS | IQ_GetCOLORTRANS | bin |
| 0x101e | 960 | IQ_SetHDR | IQ_GetHDR | bin |
| 0x101f | 12 | IQ_SetEffect | IQ_GetEffect |  |
| 0x1021 | 592 | IQ_SetLSC | IQ_GetLSC | bin |
| 0x1022 | 4 | IQ_SetMode | IQ_GetMode |  |
| 0x1023 | 1536 |  | inline |  |
| 0x1024 | 8 |  | IQ_GetOBCCALIB |  |
| 0x1027 | 24 | IQ_SetR2Y | IQ_GetR2Y |  |
| 0x1028 | 8336 | IQ_SetALSC | IQ_GetALSC | bin |
| 0x1029 | 148 | IQ_SetLSC_CTRL | IQ_GetLSC_CTRL | bin |
| 0x102a | 148 | IQ_SetALSC_CTRL | IQ_GetALSC_CTRL | bin |
| 0x102b | 1164 | IQ_SetDynamicDP_CLUSTER | IQ_GetDynamicDP_CLUSTER | bin |
| 0x102c | 26 |  | IQ_GetCCMInfo |  |
| 0x102d | 2492 | IQ_SetNRLuma_Adv | IQ_GetNRLuma_Adv | bin |
| 0x102e | 776 | IQ_SetNRChroma_Adv | IQ_GetNRChroma_Adv | bin |
| 0x102f | 32 | IQ_SetPFC_EX | IQ_GetPFC_EX | bin |
| 0x1030 | 28 | IQ_SetHDR_EX | IQ_GetHDR_EX | bin |
| 0x1031 | 1572 | IQ_SetSHP_EX | IQ_GetSHP_EX | bin |
| 0x1032 | 36 | IQ_SetNR_3D_EX | IQ_GetNR_3D_EX | bin |
| 0x1033 | 26192 | IQ_SetDUMMY | IQ_GetDUMMY |  |
| 0x1034 | 1548 | IQ_SetDUMMY_EX | IQ_GetDUMMY_EX |  |
| 0x1035 | 3900 | IQ_SetWDRCurveFull | IQ_GetWDRCurveFull | bin |
| 0x1036 | 1028 | IQ_SetAdaptiveGamma | IQ_GetAdaptiveGamma | bin |
| 0x1037 | 416 | IQ_SetNRChroma_Pre | IQ_GetNRChroma_Pre | bin |
| 0x1038 | 264 | IQ_SetTEMP | IQ_GetTEMP |  |
| 0x1039 | 1 |  | inline |  |
| 0x103c | 152 | IQ_SetALSC_ADJ | IQ_GetALSC_ADJ | bin |
| 0x1401 | 520 |  | AE_GetHistWghtY |  |
| 0x1402 | 572 |  | AE_QueryExposureInfo | app |
| 0x1403 | 8 | AE_SetEVComp | AE_GetEVComp | bin |
| 0x1404 | 4 | AE_SetExpMode | AE_GetExpMode | bin app |
| 0x1405 | 16 | AE_SetManualExpo | AE_GetManualExpo | bin app |
| 0x1406 | 4 | AE_SetState | AE_GetState | bin |
| 0x1407 | 132 | AE_SetTarget | AE_GetTarget | bin app |
| 0x1408 | 40 | AE_SetConverge | AE_GetConverge | bin |
| 0x1409 | 32 | AE_SetExposureLimit | AE_GetExposureLimit | bin app |
| 0x140a | 260 | AE_SetPlainExpTbl | AE_GetPlainExpTbl | bin |
| 0x140b | 260 | AE_SetPlainShortExpTbl | AE_GetPlainShortExpTbl | bin |
| 0x140c | 4 | AE_SetWinWgtType | AE_GetWinWgtType |  |
| 0x140d | 3076 | AE_SetWinWgt | AE_GetWinWgt | bin |
| 0x140e | 4 | AE_SetFlicker | AE_GetFlicker | bin app |
| 0x140f | 296 | AE_SetStrategy | AE_GetStrategy | bin |
| 0x1410 | 8 | AE_SetRGBIRAE | AE_GetRGBIRAE | bin |
| 0x1411 | 132 | AE_SetAEHDR | AE_GetAEHDR | bin |
| 0x1412 | 16 | AE_SetManualExpoShort | AE_GetManualExpoShort | bin |
| 0x1413 | 16 | AE_SetFlickerEX | AE_GetFlickerEX | bin |
| 0x1414 | 12 |  | AE_GetFlickerInfo |  |
| 0x1415 | 8 | AE_SetStabilizer | AE_GetStabilizer | bin |
| 0x1416 | 660 | AE_SetStrategyEX | AE_GetStrategyEX | bin |
| 0x1417 | 14 |  | AE_GetStrategyEXInfo |  |
| 0x1418 | 12 | AE_SetDayNightDetection | AE_GetDayNightDetection |  |
| 0x1419 | 12 |  | inline |  |
| 0x1801 | 24 |  | AWB_QueryInfo | app |
| 0x1802 | 1464 | AWB_SetAttr | AWB_GetAttr | bin app |
| 0x1803 | 52 | AWB_SetAttrEx | AWB_GetAttrEx | bin |
| 0x1804 | 44 | AWB_SetMultiLSAttr | AWB_GetMultiLSAttr | bin |
| 0x1806 | 4 | AWB_SetCTMwb | AWB_GetCTMwb | bin |
| 0x1807 | 12 | AWB_SetDayNightDetection | AWB_GetDayNightDetection |  |
| 0x1808 | 12 |  | inline |  |
| 0x1809 | 12 | AWB_SetStabilizer | AWB_GetStabilizer | bin |
| 0x1c01 | 8 | inline | inline | bin |
| 0x1c02 | 128 | inline | inline | bin |
| 0x1c03 | 84 | inline | inline | bin |
| 0x1c04 | 52 | inline | inline | bin |
| 0x1c05 | 3 | inline | inline | bin |
| 0x1c06 | 4 | inline | inline | bin |
| 0x1c07 | 12 | inline | inline | bin |
| 0x1c08 | 28 | inline | inline | bin |
| 0x1c09 | 40 | inline | inline | bin |
| 0x1c29 | 16 | inline | inline | bin |
| 0x1c2a | 8 | inline | inline | bin |
| 0x1c2b | 272 | inline | inline | bin |
| 0x1c2c | 8 | inline | inline | bin |
| 0x1c2d | 24 | inline | inline | bin |
| 0x1c2e | 8 | inline | inline | bin |
| 0x1c2f | 6 | inline | inline | bin |
| 0x1c30 | 8 |  | inline |  |
| 0x240f |  | CALIv30_AWB_CaliApply |  |  |
| 0x2410 |  | CALIv30_OB_CaliApply |  |  |
| 0x2414 |  | CALIv30_SDC_CaliApply |  |  |
| 0x2415 |  | CALIv30_ALSC_CaliApply |  |  |
| 0x2416 |  | inline ("not support" print) |  |  |
| 0x2417 |  | CALIv30_LSC_CaliApply |  |  |
| 0x2802 | 4 |  | IQ_Getind |  |
| 0x2803 | 8 | IQ_SetAPIBypassMode | IQ_GetAPIBypassMode | bin (record skipped by the loader, §6.3) |
| 0x2c03 | 84 | AWB_SetGoldenLightArea | AWB_GetGoldenLightArea | bin |
| 0x2c04 | 23044 |  | AWB_GetCTStats |  |
| 0x2c05 | 34560 |  | AWB_GetHWStats |  |
| 0x2c06 | 80 |  | AWB_GetCurLightArea |  |
| 0x2e01 | 46088 |  | MS_CAM_ISP_GetAeHwAvgStats |  |
| 0x2e02 | 256 |  | MS_CAM_ISP_GetHistoHwStats |  |
| 0x2e03 | 256 |  | MS_CAM_ISP_GetHistoHwStats |  |
| 0x2e04 | 108 |  | MS_CAM_ISP_GetAeInitStatus |  |
| 0x2e05 | 65 |  | MS_CAM_ISP_GetAeStatus |  |
| 0x2e06 | 56 | MS_CAM_ISP_SetAeParam |  |  |
| 0x2e07 | 34568 |  | MS_CAM_ISP_GetAwbHwAvgStats |  |
| 0x2e08 | 3 | MS_CAM_ISP_SetCus3AEnable (before the switch) |  |  |
| 0x2e09 | 41 |  | MS_CAM_ISP_GetAwbStatus |  |
| 0x2e0a | 24 | MS_CAM_ISP_SetAwbParam |  |  |
| 0x2e0b | 4 | MS_CAM_ISP_SetAEWindowBlockNumber | inline |  |
| 0x2e0c | 12 | MS_CAM_ISP_SetAWBSampling | MS_CAM_ISP_GetAWBSampling |  |
| 0x2e0d | 7168 |  | MS_CAM_ISP_GetAfHwStats |  |
| 0x2e0e | 20 | MS_CAM_SetAFWindow | CameraGetAFWindow |  |
| 0x2e0f | 140 | CameraSetAFFilterAttr | CameraGetAFFilterAttr |  |
| 0x2e10 | 52 | CameraSetAFFilterSq | CameraGetAFFilterSq |  |
| 0x2e11 | 8 | CameraSetAF_ROI_Mode | CameraGetAF_ROI_Mode |  |
| 0x2e12 | 10 | CameraSetHistoWins | CameraGetHistoWins |  |
| 0x2e13 | 16 |  | MS_CAM_GetImageResolution |  |
| 0x2e14 | 20 |  | MS_CAM_EnableISPOutImage |  |
| 0x2e15 | 4 |  | CameraISP_ImageStoreGetCounter |  |
| 0x2e16 | 24 | MS_CAM_CaptureHdrRawImage |  |  |
| 0x2e17 | 24 |  | MS_CAM_ISP_GetFrameMetaInfo |  |
| 0x2e18 | 24 | MS_CAM_ISP_WriteSensorData | MS_CAM_ISP_ReadSensorData |  |
| 0x2e19 | 34568 |  | MS_CAM_ISP_GetAwbHwAvgStatsShort |  |
| 0x2e1a | 8 |  | MS_CAM_CaptureRawImage |  |
| 0x2e1e | 512 |  | MS_CAM_ISP_GetRgbIrHistoHwStats |  |
| 0x2e20 | 16 |  | MS_CAM_ISP_GetAfStatus |  |
| 0x2e21 | 12 | MS_CAM_ISP_SetAfParam |  |  |
| 0x2e22 | 8 | MS_CAM_ISP_SetAEWindowBlockRange | inline |  |
| 0x2e23 | 8 | MS_CAM_ISP_SetAWBWindowBlockRange | inline |  |
| 0x2e24 | 3 | CameraSetAF_YParam | CameraGetAF_YParam |  |
| 0x2e25 | 4 | CameraSetAF_Source | CameraGetAF_Source |  |
| 0x2e26 | 12 | CameraSetAFPreFilter | CameraGetAFPreFilter |  |
| 0x2e27 | 28 | CameraSetAFYMap | CameraGetAFYMap |  |
| 0x2e28 | 40 | CameraSetAFLdg | CameraGetAFLdg |  |
| 0x3000 | (4) | inline | inline | bin (record skipped by the loader) |
| 0x3001 | (4) | inline | inline | bin (record skipped by the loader) |
| 0x3002 | 100 | inline | inline |  |
| 0x3003 | (4) | inline | inline | bin (record skipped by the loader) |

Not accepted (→ 0xF): every id outside these rows, among them 0x101A FPN, 0x1020, 0x1025 NR3D_P1, 0x1026 WDRCurveAdv, 0x103A
DayNightDetection (a record of our bin), 0x1805, 0x1C0A–0x1C28, 0x2411–0x2413, 0x2E1B–0x2E1D, 0x2E1F. The sizes of 0x3000,
0x3001 and 0x3003 in brackets are the XML's (one s32); the switch stores and returns word 0 (§5.4); whether it checks their size
is not recorded.

## 3. IQ ids 0x10xx, 0x2802 and 0x2803

### 3.1 Payload layout [R]
<!-- ev:isp-3-iq-api.3.1 -->
- `isp_api.xml` (the IQ tuning tool's description, "Tiramisu" v1.2, 99 groups / 994 items; isp-6-userspace.md §8) gives every
  field of every IQ / AE / AWB / AF struct. Its group sizes equal the payload sizes mhal
  checks for all 84 ids both know; three read-only "info" structs differ by 2–3 tail bytes, because the XML counts padding the
  kernel does not (0x1417: XML 16, kernel 14). The few kernel ids the XML lacks are described by the Tiramisu headers
  (`skyipc_tiramisu`, qiming `isp/tiramisu`).
- Layout rule (from the XML's `AutoMode="a~b"` attribute, checked against the kernel and the C headers): items a…b are the
  members of **one per-ISO struct stored as an array of 16 entries** (`stAuto.stParaAPI[16]`), i.e. interleaved; the XML lists
  them field by field with N = 16, which is not the memory order. Everything else is laid out in item order with no implicit
  padding (the XML carries explicit PAD items). The relaid offsets agree with the Tiramisu headers for all 82 ids both describe
  — struct size, offset of the auto block and entry size; the one apparent difference (CCM auto block at 12 vs
  `offsetof(stAuto)` = 8) is only that the header puts `bISOActEn` inside `stAuto`. Kernel reads confirm it (the CCM CCT
  thresholds are read at struct+12 with stride 28).
- Common head of almost every IQ struct: `u32 bEnable; u32 enOpType (0 auto, 1 manual); auto[16]; manual`.
- Ids the XML lacks, from the headers: 0x101a FPN {u32 enable} (4 B), 0x1025 NR3D_P1 {u32} (4), 0x1026 WDRCurveAdv {u32} (4),
  0x1027 R2Y (24: u32 enable + 9 × u16 matrix + u8 + pad), 0x101f Effect (12: 3 × u32), 0x1022 IQMode {u32 0 day / 1 night}.

### 3.2 From a Set to the registers [R]
1. The Set handler copies the struct into the IQ context and raises the dirty flag of its API slot k.
2. At the next `DoIq` the slot's converter `IQ_API20_SET_<X>` runs: it fills the per-ISO **API tables** of the IP slots that
   slot k feeds (`hal_IQ_API20_IP2API`) — same layout as the IQ-file tables — and switches those IP slots to API mode.
3. `hal_IQ_Set_ALL(ip)` interpolates two ISO rows (the CCM, HSV and Temperature by other axes, §3.5, §3.11), applies the effect
   (`IQ_DoEffect`, §3.5), `IQ_Patch_<IP>` and `IspRegSet<IP>`, which writes cfg enums through `Regmap` into the shadow image and
   sets the shadow id's dirty byte.
4. At the frame's batch `DrvIsp_ApplyShadow(id)` → `HalIsp_Apply<IP>` copies the shadow fields into the bank (through CMDQ);
   large tables go by menuload (isp-2-ispmid-mload-ispscl.md §12).

The IQ file (`iqfile0.bin` = `imx307_iqfile.bin`) is the default of every IP parameter, per ISO step; the API only overrides.
On this camera the kernel loads the **imx307** file for the GC8613 sensor (the user 3A loads the same file: `/config/iqfile/iqfile0…3.bin`
are symlinks to `imx307_iqfile.bin`), and our bin then overrides 42 API groups — every IP whose API is not in the
bin, or is in it with Enable 0, runs on imx307 tuning. Steps 2–4: isp-4-iq-engine.md §1.2, §5.2, §10; the API slots and API
mode: §6 there; the IQ file: §13 there.

### 3.3 Chain per id [R]
<!-- ev:isp-3-iq-api.3.3 -->
"ctx" = copy target of the Set handler, "k" = API slot, converter = `IQ_API20_SET_<X>`, then the IP slots that slot k feeds with
their cfg-enum ranges and banks. Get handlers copy the same bytes back (no conversion) unless noted in §3.4.

| id | XML group | size | ctx | k | converter | IP slots fed (cfg enums) → banks |
|---|---|---|---|---|---|---|
| 0x1001 | APIVer | 12 | — | — | — | — |
| 0x1002 | — | 4 | — | — | — | — |
| 0x1003 | — | 4 | — | — | — | — |
| 0x1004 | ColorToGray | 4 | +0x4548c (inline) | 15 | CTG | 40 YCUVM10 (via `hal_DoIQ`/`DoIqPerFrame`) [0x3ff-0x403] 133e |
| 0x1005 | Contrast | 76 | +0x3a62c | 13 | GAMA_C2A_ISP | 26 GAMA_C2A_ISP [0x30a-0x30f] 1339 |
| 0x1006 | Brightness | 76 | +0x3a594 | 11 | GAMA_C2A_ISP | 26 GAMA_C2A_ISP [0x30a-0x30f] 1339 |
| 0x1007 | Lightness | 76 | +0x3a5e0 | 12 | GAMA_C2A_ISP | 26 GAMA_C2A_ISP [0x30a-0x30f] 1339 |
| 0x1008 | Gamma | 26120 | +0x3a678 | 14 | GAMA_C2A_ISP | 26 GAMA_C2A_ISP [0x30a-0x30f] 1339 |
| 0x1009 | YUVGamma | 17416 | +0x40c80 | 23 | YUVGAMA | 39 YUVGAMA [0x3f4-0x3fc] 133e |
| 0x100a | Saturation | 416 | +0x3879c | 3 | YCUVM10 ADJUV | 37 ADJUV [0x3d8-0x3e8] 133e; 40 YCUVM10 [0x3ff-0x403] 133e |
| 0x100b | Defog | 28 | +0x499ac | 17 | DEFOG | 20 WDRLOC [0x294-0x2bd] 1321 1322; 39 YUVGAMA [0x3f4-0x3fc] 133e |
| 0x100c | CCM | 488 | +0x385b4 | 2 | CCM | 24 CCM [0x2fd-0x302] 1338 |
| 0x100d | AntiFalseColor | 144 | +0x38a5c | 6 | FCOLOR | 23 DM [0x2da-0x2fa] 1325 |
| 0x100e | NR3D | 2116 | +0x37d48 | 10 | NR3D | 2 NR3D [0x184-0x1cb] 1328 132a 132b; 28 NLM (no IP code); 32 ACLUT (no IP code) |
| 0x100f | NRDeSpike | 588 | +0x36ef4 | 8 | SpikeNR | 21 SpikeNR [0x2c0-0x2ce] 1317 |
| 0x1010 | NRLuma | 588 | +0x37140 | 16 | NLM | 1 BNR [0x178-0x181] 1326; 28 NLM (no IP code) |
| 0x1011 | NRChroma | 280 | +0x499c8 | 33 | MXNR | 38 MXNR [0x3eb-0x3f1] 133e |
| 0x1012 | Sharpness | 4904 | +0x38c48 | 9 | YEE | 30 YEE [0x317-0x36f] 1333 1334 1335 133a; 32 ACLUT (no IP code) |
| 0x1013 | Crosstalk | 348 | +0x38aec | 7 | GE | 17 GE (no IP code); 18 ANTICT [0x27c-0x283] 1317 |
| 0x1014 | BlackLevel | 144 | +0x3893c | 4 | OBC | 6 OBC [0x1ce-0x1d7] 1312 |
| 0x1015 | BlackLevel_P1 | 144 | +0x389cc | 5 | OBC_P1 | 7 OBC_P1 [0x1da-0x1e3] 1313 |
| 0x1016 | WDR | 1640 | +0x46f5c | 21 | WDR_LOC | 20 WDRLOC [0x294-0x2bd] 1321 1322 |
| 0x1017 | DynamicDP | 756 | +0x45490 | 19 | DPC | 16 DPC [0x252-0x279] 1316 |
| 0x1018 | HSV | 4940 | +0x45c10 | 20 | HSV | 25 HSV [0x305-0x307] 1338 |
| 0x1019 | RGBIR | 756 | +0x49504 | 22 | RGBIR | 19 RGBIR [0x286-0x291] 1318 |
| 0x101b | PFC | 400 | +0x497fc | 28 | PFC | 33 PFC [0x39a-0x3b8] 1327 |
| 0x101c | DeMosaic | 416 | +0x36d54 | 30 | DEMOSAIC | 23 DM [0x2da-0x2fa] 1325 |
| 0x101d | Colortrans | 28 | +0x49fa0 | 25 | COLORTRANS | 41 COLORTRANS [0x406-0x40c] 133e |
| 0x101e | HDR | 960 | +0x49fbc | 26 | HDR | 14 HDR [0x205-0x248] 131e |
| 0x101f | — | 12 | +0x4c7a0 | 27 | EFFECT | 48 EFFECT (no IP code) |
| 0x1021 | LSC | 592 | +0x4a398 | 35 | LSC | 10 LSC [0x1e6-0x1f0] 1319 |
| 0x1022 | IQMode | 4 | — | — | — | — |
| 0x1023 | (LinearityLUT) | 1536 Get | — | — | inline: returns 5 (not supported) | — |
| 0x1024 | — | 8 | — | — | — | — |
| 0x1027 | — | 24 | +0x49f88 | 34 | R2Y | 27 R2Y [0x312-0x314] 1339 |
| 0x1028 | ALSC | 8336 | +0x4a67c | 37 | ALSC | 12 ALSC [0x1f3-0x202] 131d |
| 0x1029 | LSC_CTRL | 148 | +0x4a5e8 | 36 | — | 10 LSC [0x1e6-0x1f0] 1319 |
| 0x102a | ALSC_CTRL | 148 | +0x4c70c | 38 | — | 12 ALSC [0x1f3-0x202] 131d |
| 0x102b | DynamicDP_Cluster | 1164 | +0x45784 | 39 | DPC_CLUSTER | 16 DPC [0x252-0x279] 1316 |
| 0x102c | CCMInfo | 26 | — | — | — | — |
| 0x102d | NRLuma_Adv | 2492 | +0x3738c | 44 | GNR | 34 GNR [0x3bb-0x3d0] 1330 |
| 0x102e | NRChroma_Adv | 776 | +0x49ae0 | 29 | SIIR | 31 SIIR [0x386-0x397] 133a; 35 UVM [0x3d3-0x3d5] 133d |
| 0x102f | PFC_EX | 32 | +0x4998c | 40 | PFC_EX | 33 PFC [0x39a-0x3b8] 1327 |
| 0x1030 | HDR_EX | 28 | +0x4a37c | 41 | HDR_EX | 14 HDR [0x205-0x248] 131e |
| 0x1031 | Sharpness_EX | 1572 | +0x39f70 | 42 | YEE_EX | 30 YEE [0x317-0x36f] 1333 1334 1335 133a |
| 0x1032 | NR3D_EX | 36 | +0x3858c | 43 | NR3D_EX | 2 NR3D [0x184-0x1cb] 1328 132a 132b |
| 0x1033 | Dummy | 26192 | +0x4c7b8 | 45 | DUMMY | 49 DUMMY (no IP code) |
| 0x1034 | Dummy_EX | 1548 | +0x52e08 | 46 | DUMMY_EX | 49 DUMMY (no IP code) |
| 0x1035 | WDRCurveFull | 3900 | +0x475c8 | 47 | WDRCurveFull | 20 WDRLOC [0x294-0x2bd] 1321 1322 |
| 0x1036 | AdaptiveGamma | 1028 | +0x45088 | 14 | GAMA_C2A_ISP | 26 GAMA_C2A_ISP [0x30a-0x30f] 1339 |
| 0x1037 | NRChroma_Pre | 416 | +0x49de8 | 49 | XNR | 29 XNR [0x372-0x383] 133d |
| 0x1038 | Temperature | 264 | +0x53414 | 50 | TEMP | — |
| 0x1039 | TemperatureInfo | 1 Get | inline: byte ctx+0x5351c | — | — | — |
| 0x103c | ALSC_ADJ | 152 | +0x53520 | 52 | — | — |
| 0x2803 | BypassItem | 8 | +0x5368c+4k | k | `SetAPIBypassMode` {u32 bypass; u32 k}: bypass value of API slot k, slot k marked dirty | the IPs of slot k (mode 3, converter writes "off" rows) |

The 53 slots: 0 = the global bypass word; 18, 24, 48 and 51 have no handler; 31, 32, 36 (LSC_CTRL), 38 (ALSC_CTRL) and 52
(ALSC_ADJ) have no converter — LSC_CTRL, ALSC_CTRL and ALSC_ADJ are read by the LSC / ALSC IP code directly (§3.10). Converter
functions with no ioctl path: `IQ_API20_SET_FPN`, `_NR3D_P1`, `_WDR_GBL`, `_LSC_CTRL`, `_ALSC_CTRL` are 4-byte `return 0` stubs and
`_WDRCurveAdv` is empty; none is in the slot switch (slots 1, 31, 32, 36, 38 fall to its default case).

The cfg enums carry official names in mhal `.data` (isp-4-iq-engine.md §12.2); the field
names used in §3.5–§3.11 for the API structs are the XML's. What each register field does in the hardware beyond its name is not known — no register reference for this chip exists.

### 3.4 Ids without a converter, bypass, and the rules every converter follows [R]
**Ids without a converter.**
- 0x1001 VersionInfo {vendor, major, minor}: Set stores word 0 (ctx+0x5803c); Get returns {that word, 1, 3} (§6.2).
- 0x1002 ParaInitStatus (Get): byte ctx+0x55c08 (IQ init done).
- 0x1003 FastMode {u8}: ctx+0x538d0; 1 = recompute every IP at every ISO / CT change (`IQ_ISOIndexGet` treats it as a forced
  trigger).
- 0x1022 IQMode {0 day, 1 night} (`pIQ_SetParaMode`, values > 1 → 0): ctx+0x538dc; selects the ISO gain table of the IQ file
  (isp-4-iq-engine.md §4.1) and, with CCM `ISO_ACT_EN`, the night CCM (identity, §3.5).
- 0x1023 LinearityLUT (Get 1536): not supported — the inline case returns 5, which the Get path discards (§1.3).
- 0x1024 OBCCALIB (Get 8): the four manual OBC values (ctx+0x389c4…).
- 0x102c CCMInfo (Get 26): the current CCM — 9 coefficients from the CCM output block, the row sums recomputed into every 4th
  u16 as in the API struct — + u16 current colour temperature (ctx+0x537a0).
- 0x1039 TemperatureInfo (Get 1): byte ctx+0x5351c (§3.11).
- 0x2802 IQind (Get 4): `DrvAlgo_IF_IQ_Getind`, a 16-byte handler; what it returns is not read.
- 0x101f Effect → ctx+0x4c7a0, slot 27: its converter writes no table; `IQ_DoEffect(ip)` reads the struct directly in every IP
  run (§3.5).

**Handlers with no ioctl id** (dead, or reached only from inside mhal): `IQ_SetFPN` (ctx+0x497f8, slot 1), `IQ_SetNR3D_P1`
(ctx+0x385b0, slot 31), `IQ_SetWDRCurveAdv` (ctx+0x475c4, slot 32), `IQ_SetMirrorFlip` (→ `pIQ_SetMirrorFlip`: stores the flip
mode and re-runs IPs 12 ALSC, 2 NR3D and 33 (0x21) PFC), `IQ_SetEnSysMcnrMemory` (empty), `IQ_SetColorToGray` (same as the 0x1004 inline
code), `IQ_GetLinearityLUT`, `IQ_ForceTrigIQ` (called by mhal itself at open); the AWB ones are in §4.3.

**Bypass — 0x2803 BypassItem** (Set / Get 8 B: {u32 bypass; u32 k < 53}; `DrvAlgo_IF_IQ_SetAPIBypassMode` stores the value at
ctx+0x5368c+4k and marks slot k dirty; Get `DrvAlgo_IF_IQ_GetAPIBypassMode`). Any non-zero bypass value puts the slot's IPs into
API mode (3), and every converter starts with "if bypass[k] = 1: write enable = 0 into all 16 API rows" — the IP is switched
off (exceptions: YUVGamma, whose bypass writes identity curves, §3.9; the gamma levels, §3.5). The XML's BypassItem group (128
B) is a 32-word GUI form, one u32 per item, its `Param` naming the slot; the tool turns each word into one 0x2803 call [I]:

| word (byte) | XML item | slot | word (byte) | XML item | slot |
|---|---|---|---|---|---|
| 0 (0) | API_BypassItem_DPC (DynamicDP) | 19 | 16 (64) | API_BypassItem_CCM | 2 |
| 1 (4) | API_BypassItem_CR (Crosstalk) | 7 | 17 (68) | API_BypassItem_HSV | 20 |
| 2 (8) | API_BypassItem_FCOLOR (AntiFalseColor) | 6 | 18 (72) | API_BypassItem_GMA (Gamma) | 14 |
| 3 (12) | API_BypassItem_DM (Demosaic) | 30 | 19 (76) | API_BypassItem_BRI (Brightness) | 11 |
| 4 (16) | API_BypassItem_OBC | 4 | 20 (80) | API_BypassItem_LIG (Lightness) | 12 |
| 5 (20) | API_BypassItem_OBC_P1 | 5 | 21 (84) | API_BypassItem_CTS (Contrast) | 13 |
| 6 (24) | API_BypassItem_NR_DESPIKE | 8 | 22 (88) | API_BypassItem_CTG (ColorToGray) | 15 |
| 7 (28) | API_BypassItem_NR_Numa (NRLuma) | 16 | 23 (92) | API_BypassItem_SHP (Sharpness) | 9 |
| 8 (32) | API_BypassItem_NR_Numa_Adv | 44 | 24 (96) | API_BypassItem_SAT (Saturation) | 3 |
| 9 (36) | API_BypassItem_NR_Chroma | 33 | 25 (100) | API_BypassItem_YUVGMA | 23 |
| 10 (40) | API_BypassItem_NR_Chroma_Adv | 29 | 26 (104) | API_BypassItem_WDR | 21 |
| 11 (44) | API_BypassItem_NR_Chroma_Pre | 49 | 27 (108) | API_BypassItem_DEFOG | 17 |
| 12 (48) | API_BypassItem_NR_3D | 10 | 28 (112) | API_BypassItem_RGBIR | 22 |
| 13 (52) | API_BypassItem_HDR | 26 | 29 (116) | API_BypassItem_PFC | 28 |
| 14 (56) | API_BypassItem_R2Y | 34 | 30 (120) | API_BypassItem_LSC | 35 |
| 15 (60) | API_BypassItem_COLORTRANS | 25 | 31 (124) | API_BypassItem_ALSC | 37 |

Our bin's BypassItem record (all 32 words 0) is skipped by the loader (§6.3), so nothing on this camera bypasses an IP.

**Rules of every converter.**
- It returns −1 without doing anything when ctx+0x53bd4 = 4.
- It starts with the bypass rule above for its own slot (the `_EX` converters, WDRCurveFull and TEMP do not check their own
  slot: §3.6–§3.11).
- Argument 0xff = called from inside another converter: no cross-calls are made then. Pairs of converters that share an IP
  (Sharpness / Sharpness_EX, NR3D / NR3D_EX, PFC / PFC_EX, HDR / HDR_EX, DynamicDP / DynamicDP_Cluster, DeMosaic /
  AntiFalseColor) run each other first when the partner is enabled or bypassed, otherwise reset the IP's API table to the
  IQ-file table — so the shared table always holds both APIs.
- "Disabled" (Enable = 0, not bypassed) writes nothing unless stated: the API table keeps what it held, and the IP stays in API
  mode only while some API feeding it is enabled (isp-4-iq-engine.md §6).
- The exported `IQ_API20_SET_NR3D` and `IQ_API20_SET_SIIR` only do the order / bypass part and tail-jump to unnamed bodies
  (NR3D's at 0x6635c, inside `hal_AF_InitParameters_HWLdg`'s range).

### 3.5 Gamma, colour and effects [R unless tagged]

#### Gamma group — 0x1005 Contrast, 0x1006 Brightness, 0x1007 Lightness, 0x1008 Gamma, 0x1036 AdaptiveGamma
Slots 11–14 → `IQ_API20_SET_GAMA_C2A_ISP` → IP 26 GAMA_C2A_ISP (row 0x608 B: u8 enable; u16 R[256] at +2, G[256] at +0x202,
B[256] at +0x402; registers 1339.00 / .02 / .03 + the 256 × {R, G, B} LUT by menuload).
1. Gamma enabled: row i = auto[i].{R, G, B} (OpType auto) or manual.{R, G, B}; LUT values are 12-bit (0…4095 per the header, XML
   max 1023 — the kernel does not clip). Gamma disabled: the API table is a copy of the IQ-file table (imx307 gamma).
2. AdaptiveGamma enabled: each LUT entry is blended with BTGamma (bright) or DTGamma (dark) by w = (u16 at IqInput+0x4e) − 1
   (0…0x3ff, 0x200 = neutral): w < 0x200: (w·LUT + (0x200−w)·BT) >> 9; w > 0x200: ((0x400−w)·LUT + (w−0x200)·DT) >> 9.
   `DoIqPerFrame` re-runs this every frame while both Gamma and AdaptiveGamma are enabled. IqInput+0x4e is filled by
   `CameraIspAlgo_DoIq` / `CameraIspAlgo_DoIqPerFrame` from ch+0x114 = in-kernel AE output +0x8c = Strategy-EX GMBlendRatio + 1
   (isp-5-3a.md §2.11, §2.2), so a Strategy-EX target below the BV target pulls the gamma toward the bright-tone curve and one above it toward
   the dark-tone curve. With the user 3A (this camera) libcus3a's `SigmaAeRun` forwards nothing from AE output +0x8c into the
   0x2E06 result, so the ratio computed by libispalgo is dropped and ch+0x114 is not fed by the user AE [I: no 0x2E06 field maps
   to it].
3. Levels: brightness b, lightness l, contrast c per row = auto[i].LEVEL or manual.LEVEL of each enabled struct, else 50. If any
   of the three is enabled, `LutGen_GAMA_EFFECT` builds per row an effect curve E[256] (10-bit) and composes it with the row's
   **G** LUT into R:
   - contrast: c' = clamp(c+14, 14, 114); c < 50 → E = (Lo·(64−c') + 4x·c')/64, else E = (Hi·(c'−64) + 4x·(128−c'))/64, kept
     strictly increasing;
   - brightness: b' = clamp(b+14, 14, 114); gain g = −10·(64−b') for b' ≤ 64, +30·(b'−64) above (per mille; computed as
     `(64−b')·500/50` and `(b'−64)·1500/50`, `umull` by 0x51eb851f, `>>36`); E = E·(1000+g)/1000 — brightness 0 halves E,
     brightness 100 multiplies it by 2.5;
   - lightness: l' likewise, E += 6·(l'−64);
   - clip 0…1023; R[x] = G[E[x]·255/1023], kept increasing.
   Lo / Hi are fixed S-curves built at IqInit (`IQ_InitParameters`, `_exp(x,n)` = integer xⁿ): for 0 < x < 128, Hi = (200·x(128−x)
   + 512x²) >> 14, Lo = (800·x(128−x) + 512x²) >> 14; for 128 ≤ x < 255, with u = 256−x, v = x−128: Hi = (512u² + 1840uv + 1023v²)
   >> 14, Lo = (512u² + 1280uv + 1023v²) >> 14; Hi/Lo(0) = 0, (255) = 1023. Hi steepens the mid-tones (more contrast), Lo flattens
   them.
4. **Then G and B are overwritten with R in every row, unconditionally** — in API mode the three channels always share one
   curve (R's, or G∘E when a level struct is enabled); per-channel API gamma is not possible on this build.
5. Bypass: slot 14 bypass zeroes the row enables; slots 11 / 12 / 13 bypass keep that level at 50.

#### 0x100c CCM (RGBmatrix)
Slot 2 → `IQ_API20_SET_CCM` → IP 24 CCM (own code `IQ_SET_CCM`, which falls through into IP 25 HSV; registers 1338.00–10). The
API table is at ctx+0x277f8, row 0x3c B: u8 enable (+0), 9 × u16 coefficients at +6…+0x16 (row-major 3×3; the SUM column of the
API matrix dropped), u16 0x100 at +0x18 (gain 1.0 in 1/256), u16 CCT at +0x1a; one saturation byte per row at ctx+0x1caf0+i.
- auto: row i = auto[i].CCM, sat = auto[i].CCM_SAT; with ISO_ACT_EN = 1 **and** IQ mode night (0x1022) the matrix is replaced by
  the identity (1024 = 1.0 on the diagonal; the API uses the same scale, rows sum to 1024). manual: manual.CCM in all rows,
  manual.CCM_SAT converted ("derived") into the saturation bytes.
- auto[i].CCT_THR is written at row i **+0x1a+2i** (`*(u16 *)(ctx + 0x27812 + i·0x3e)`, stride 0x3e instead of 0x3c), so only
  row 0 gets its CCT at +0x1a; rows 1–15 get it 2i bytes further, in bytes that hold nothing. The CT axis the CCM is
  interpolated on is built by `IQ_ISOIndexGet` from the struct itself (struct+12, stride 28 → ctx+0x537ac, when CCM is enabled),
  not from the rows, so the stray writes are harmless [I].
- Unlike other IPs the CCM is interpolated **by colour temperature** between the two auto entries around the current CT
  (`IQ_INTERPA_CCM` / `IQ_DOINTER_CCM`), not by ISO.
- The converter rewrites every SUM column (CCM[3], [7], [11]) of the **stored API struct** to the row sum, even when CCM is
  disabled; the SUM words are never read. Coefficients are 13-bit two's complement (≥ 0x1000 negative).
- `IQ_SET_CCM` then applies the effects, saturation = row sat × u16 at ctx+0x1babc / 256 × a temperature factor per channel
  (`IQ_DOINTER_TEMP` of Temperature ColorToneRatio[k] for channel k = 0, 1, 2, §3.11), and `IQ_CCM_SaturctionCalc`: S = s·I +
  (1−s)·L with L the luma matrix rows (316, 624, 84)/1024 (0.3086, 0.6094, 0.0820), M' = S·M, each row renormalised to sum 0x400,
  diagonal clipped to 0xfff, stored 13-bit; "CCM primary coef overflow (4095)" if a diagonal would exceed 0x800000/max(diag)/8;
  → `IspRegSetCCM`. The matrix the result is further multiplied with, and the CCM output block: isp-4-iq-engine.md §7.3.

#### 0x1018 HSV
Slot 20 → `IQ_API20_SET_HSV` → IP 25 HSV (own code `IQ_SET_HSV`, registers 1338.20–38). Entry (290 B): HUELUT s16[24], SATLUT
u16[24], HUELUT_ByIso s16[24], SATLUT_ByIso u16[24], HUELUT_FORUNITCCM s16[24], SATLUT_FORUNITCCM u16[24], GLOBAL_SAT u8, pad
(24 hue sectors).
- The HSV API table is at ctx+0x27bb8 (16 rows × 0x34 B: +0 enable, +2 hue[24], +0x1a sat[24], one **byte** per entry;
  `hal_IQ_API20_InitParameters` fills it from the IQ-file copy at ctx+0xb03a); a second "unit CCM" table at ctx+0x1cb00 has the
  same layout. bypass[20] → +0 = 0 in every row of the first table.
- Enable = 1: row i (auto[i] or manual) of the first table gets HUELUT[k] → +2+k and SATLUT[k] → +0x1a+k, of the second table
  HUELUT_FORUNITCCM → +2+k and SATLUT_FORUNITCCM → +0x1a+k — **only the low byte** of each s16 / u16 is stored (values are meant
  to stay in −64…64 / 0…255). HUELUT_ByIso, SATLUT_ByIso and GLOBAL_SAT are not touched by the converter.
- Run time: `IQ_SET_HSV` → `IQ_HSV_Update_Iqsub_ByIso` reads the **struct** directly in every IP run: per sector, HUELUT and
  HUELUT_FORUNITCCM of the two colour-temperature entries around the current CT and HUELUT_ByIso of the two ISO entries around the
  current gain (each clamped to ±64, interpolated with `IQ_para_inter`), the SAT tables likewise (clamped to 255), GLOBAL_SAT
  interpolated by ISO (manual: the manual set), and the CCM saturation byte (ctx+0x1caf0) interpolated by ISO; the result goes
  to the HSV output block at ctx+0x1bae2 (the combination arithmetic: isp-4-iq-engine.md §7.4). So the API tables built by the
  converter are a copy the HSV code itself does not need when HSV is enabled [I].
- Our bin: Enable 1, OpType 1 → used.

#### 0x100a Saturation
Slot 3 → `IQ_API20_SET_YCUVM10` + `IQ_API20_SET_ADJUV` → IP 40 YCUVM10 and IP 37 ADJUV (bank 133E). Entry (24 B): AllStr u8,
ByYsft u8[5], ByYLut u8[6], BySsft u8[5], BySLut u8[6], Coring u8.
- YCUVM10 row (0xe B): +0 = 1 (enum 0x3ff, 133E.60 b0); +8…+0xb = {AllStr, 0, 0, AllStr} → enum 0x403, the 2×2 UV matrix
  133E.63–.64: saturation is a plain U / V gain.
- ADJUV row (0x50 B): first checks that the five ByYsft (and BySsft) steps, as powers of two, add up to ≤ 0xff after four and
  ≥ 0x100 after five steps (the curve must span 0…256), else "ERR_SAT_X_FAIL" and nothing is written. Then +0 = 1 (0x3d8,
  133E.20 b0), +4 = 1 (0x3da, 133E.20 b2–3), +0xc ByYsft (0x3de, 133E.21), +0x12 BySsft (0x3df, 133E.22), +0x24 and +0x2a ByYLut
  (0x3e2 / 0x3e3: U-by-Y and V-by-Y tables, 133E.26 / .29…), +0x30 and +0x36 BySLut (0x3e4 / 0x3e5: U / V by saturation,
  133E.2c / .2f…), +0x48 Coring × 6 (0x3e8, 133E.38…). The same LUT drives U and V.
- Bypass of slot 3 clears both IPs' enables.

#### 0x1004 ColorToGray
<!-- ev:isp-3-iq-api.0x1004-colortogray -->
Slot 15 → `IQ_API20_SET_CTG`. The Set handler (inline in the switch) stores the u32 at ctx+0x4548c. Converter: flag ctx+0x53762
= (value == 1); bypass[15] → flag 0. Consumers: `hal_DoIQ` re-runs IP 40 YCUVM10 in every DoIq while the flag (or ctx+0x53761,
which nothing writes — dead) is set, and `IQ_Patch_YCUVM10` then zeroes the UV matrix (0x403 = 0, 0, 0, 0) and forces the block
on → grey picture. Called by prog_uvc's menu (§1.4); our bin's record sets it to 0 (off) [R].

#### 0x101d Colortrans
<!-- ev:isp-3-iq-api.0x101d-colortrans -->
Slot 25 → `IQ_API20_SET_COLORTRANS` → IP 41 COLORTRANS (a single row 0x20 B at ctx+0x36d31, bank 133E.68–.74; not
interpolated). Struct (28 B): Enable u32, manual.Y_OFST u16, U_OFST u16, V_OFST u16 (0…2047), Matrix u16[9] (10-bit two's
complement, ×1/256). Row, with the official field names:

| row offset | enum → register | name | written by the converter |
|---|---|---|---|
| +0 | 0x406 → 133E.68 b0 | `IQ_COLORTRANS_MIX_EN` | Enable (the bit Enable and bypass drive) |
| +2 | 0x407 → 133E.68 b1 | `IQ_COLORTRANS_PREOFFSET` | no (IQ-file value 1) |
| +4 | 0x408 → 133E.68 b2 | `IQ_COLORTRANS_DITHERING_MODE` | no (IQ-file value 1) |
| +6…+0x16 | 0x409 → 133E.69–.71 [9:0] | `IQ_COLORTRANS_M` (10-bit two's complement) | H (below) |
| +0x18 / +0x1a / +0x1c | 0x40a–0x40c → 133E.72 [10:0] / 133E.73 [10:0] / 133E.74 [10:0] | `IQ_COLORTRANS_Y_OFFSET` / `IQ_COLORTRANS_U_OFFSET` / `IQ_COLORTRANS_V_OFFSET` | the three offsets |

There is no `IQ_COLORTRANS_EN`.
- bypass[25] = 1: row+0 = 0 (block off), return.
- Enable = 1: M = Matrix as signed 10-bit (`TwoComp2Int(v, 0x1ff)`), H = M · C with C the constant **YUV→RGB** matrix at
  `.rodata+0xaf20` = [[256, 0, 359], [256, −88, −183], [256, 453, 0]] / 256 (BT.601 full range), each product term rounded `>>8`
  (`roundRSl`, symmetric); H stored as 13-bit two's complement (`interto_twos_complement`: negative → (v & 0xfff) + 0x1000; the
  register keeps 10 bits) into +6…+0x16; the three offsets (u16, unconverted) into +0x18…+0x1c; row+0 = 1. The nine words after
  C in `.rodata` (77, 150, 29, −43, −85, 128, 128, −107, −21 = RGB→YUV) only initialise the stack buffer that M overwrites —
  unused.
- So the API matrix is expressed **RGB→YUV** (the hardware block sits on YUV, the kernel prepends YUV→RGB) [R formula; I
  meaning].
- Disabled: nothing written; the API row keeps its init copy of the IQ-file row.
- **Offset format: 11-bit two's complement in 10-bit pixel units** [I, from three [R] facts]: (1) the converter builds the
  matrix as M · C with C a pure linear YUV→RGB matrix (no bias term), so the block computes on zero-centred chroma; (2) the imx307
  row converts full to limited range with Y offset 64 (= 16 in 10-bit units) and U / V offsets 0 while scaling chroma by 224/256 —
  only correct if chroma is zero-centred at the offset stage; (3) with zero-centred chroma Sepia's U = 0x762 = −158 and V = +100
  (≈ −40 / +25 in 8-bit units) give Cb < 0, Cr > 0, the brown of a sepia tone, whereas an unsigned or offset-binary reading
  would push U to +1890 or +866. The Emboss / Sketch Y offset 400 (≈ mid-grey added to an edge signal) fits the same unit.
  That PREOFFSET removes the chroma bias before the matrix is [I]; it is 1 in the imx307 row and in every effect [R]. The reading
  is not confirmed on the hardware (it would show as the colour cast of U offset 0x7f0 — −16 if signed — against 0x010 with the
  COLORTRANS API enabled on an identity-like matrix).
- Our bin: Enable 0, Matrix 66 129 25 / −38 −74 112 / 112 −94 −18, offsets 64 / 0 / 0 (BT.601 limited range). The imx307 row:
  MIX_EN 0, matrix 219, 0, 1023 (−1) / 1023 (−1), 224, 1 / 1022 (−2), 1023 (−1), 225, Y offset 64 (full → limited range). So on
  this camera the block is off; the Sepia, Emboss and Sketch effects switch it on (Effect, below).

#### 0x1027 R2Y
Slot 34 → `IQ_API20_SET_R2Y` → IP 27 R2Y (a single row 0x18 B at ctx+0x2df78, bank 1339, not interpolated). Struct (24 B, not in
the XML; `MI_ISP_IQ_R2Y_TYPE_t` = {bEnable; R2Y_PARAM_t}): u32 enable, u16 matrix[9] (+4), u8 mode (+0x16), pad. bypass[34] →
row+0 = 0. Enable = 1 → row+0 = 1 (0x312, 1339.10 b0), +2…+0x12 = matrix[9] (0x313, 1339.15…, 13-bit two's complement; imx307
file: 511 −428 −83 / 306 601 117 / −173 −339 511 = BT.601 RGB→Cr/Y/Cb × 1024), +0x14 = the mode byte (0x314, 1339.12 b0). Enable
= 0 (not bypassed) → nothing written. The kernel accepts 0x1027, but the XML has no group for it and our bin has no record →
unused on this camera.

#### 0x101f Effect
Slot 27 → `IQ_API20_SET_EFFECT`; applied by `IQ_DoEffect(ip)` inside every IP run. Struct (12 B) = `MI_ISP_IQ_EFFECT_TYPE_t`
{u32 bEnable; u32 eStyle; u8 u8Level; pad} (Tiramisu `mi_isp_iq_datatype.h`); the kernel reads the level as a byte at +8.
- Converter: bypass[27] = 1 → bEnable := 0 in the stored struct. Then always `pIQ_IQTrigger(ctx, 1, 50)` (mode 1: mark every IP
  of the ISO-index list as "force run" in ctx+0x54800+4·ip, set ctx+0x538d3 = 1) — the next `DoIq` re-runs those IPs and their
  `IQ_DoEffect` call applies or removes the effect. No API table is written.
- `IQ_DoEffect(ctx, ip)` (called by `hal_IQ_Set_ALL` after interpolation, before `IQ_Patch` / `IspRegSet`, on the IP's **output
  block**) returns at once unless bEnable = 1; then by eStyle, only for the listed IPs (s = min(level, 100)):

  | eStyle | IP (slot) | writes in the output block |
  |---|---|---|
  | 0 Vivid | 37 ADJUV | U-by-saturation LUT 0x3e4[0..5] (+0x30): entries 0–2 v += s·(255−v)/100, entry 4 v += s·((v+255)/2 − v)/100, 3 and 5 kept; the same 6 bytes copied to the V LUT 0x3e5 (+0x36); enable 0x3d8 = 1 |
  | 1 Pale | 37 ADJUV | same LUT: entry 3 v += s·(v/2 − v)/100, entries 4–5 v −= v·s/100, 0–2 kept; copy to V; enable = 1 |
  | 2 Sepia | 40 YCUVM10 | UV matrix 0x403 `IQ_UVM10_M` (+8…+0xb) = 0, 0, 0, 0 (grey); 0x3ff `IQ_YC10_EN` = 1 |
  |  | 41 COLORTRANS | 0x406 MIX_EN = 1, 0x407 PREOFFSET = 1, U offset 0x40b = 0x762 (−158), V offset 0x40c = 100 — the output chroma is exactly the two offsets, a fixed brown tint |
  | 3 BlackWhite | 40 YCUVM10 | UV matrix = 0; enable = 1 |
  | 4 Emboss | 30 YEE | 0x324 (+0x5a, 133A.72 b2) = 1; 0x351[0..8] = 0x100; 0x360[0..8] and 0x361[0..8] = 0xa0; 0x34b[0..2] = 0; 0x349[0..15] = 0; 0x332 (1333.54) = 0x3c; 0x335 (1333.56) = 0; 0x365, 0x366 (1335.2e / .2f) = 200 |
  |  | 41 COLORTRANS | MIX_EN = 1, Y offset 0x40a = 400 |
  | 5 Negative | 26 GAMA_C2A_ISP | R, G, B LUTs: v = max(0, 1023 − v) |
  | 6 Sketch | 30 YEE | 0x324 = 1; 0x351[0..8] = 0; 0x360 / 0x361[0..8] = 0xa0; 0x34c[0..2] = 0x100; 0x36f[0..15] = 0x100; 0x329 (1333.4e hi) = 0x50; 0x32f (1333.52 hi) = 0; 0x365, 0x366 = 200 |
  |  | 41 COLORTRANS | MIX_EN = 1, Y offset = 400 |
  | 7 Oil | 21 SpikeNR | 0x2c2 (1317.21) = 15; 0x2cb[0..4] (1317.35…) = 0x7ff |
  |  | 34 GNR | 0x3c5[0..1] (1330.12…) = 0x100; 0x3c8[0..14] (1330.20…) = 0 |
  |  | 30 YEE | 0x361[0..8] = 0x80; 0x351[0..8] = 0x100; 0x34b[0..2] = 0; 0x349[0..15] = 0; 0x332 = 0x7f; 0x335 = 0; 0x366 = 500 |
  |  | 40 YCUVM10 | UV matrix = 0x40, 0, 0, 0x40 |

  The level is used only by Vivid and Pale; eStyle 8 (`eEFFECT_MAX`) and above have no case (no effect). The COLORTRANS output
  block is at ctx+0x1ca0f, right after YCUVM10's [R by layout: the written offsets match the COLORTRANS row]. Effects never touch
  the API structs of the IPs they change, so a Get returns the un-effected values.
- 0x101f is not in our bin. Besides its Set handler and the bypass path, only the Dummy_EX converter writes the struct (§3.8);
  Dummy_EX is not in our bin either → no effect on this camera.

### 3.6 Bayer-domain blocks [R; maps of 0x1017, 0x102b, 0x1019, 0x101b, 0x101c by emulation]

#### 0x1014 BlackLevel (OBC), 0x1015 BlackLevel_P1
0x1014 → slot 4 → `IQ_API20_SET_OBC` → IP 6 OBC (row 0x16 B, 1312); 0x1015 → slot 5 → IP 7 OBC_P1 (same code, 1313). Entry (8 B):
R, Gr, Gb, B (u16, 16-bit pixel scale; our bin has 3840 = 0xf00). Row: +0 = 1 (0x1ce `IQ_OBC_EN`, .00 b0); +4…+0xa = the four
offsets (0x1d0–0x1d3 → .01–.04); +0xc…+0x12 = gains ⌈1024·65535 / (65535 − v)⌉ (0x1d4–0x1d7 → .05–.08, 14 bit, 1.0 = 1024) — the
gain that stretches the range back to full scale after the subtraction. (0x1cf is `IQ_OBC_DITHER_EN`, not written.)
`IspRegSetOBC` then, for a linear (non-HDR) pipe, rescales the offsets with the sensor's digital gain (IqInput+0xc) and writes
the same values into the second OBC copy at 1312.50–.58 (`HalIsp_ApplyOBC_b`). Get 0x1024 OBCCALIB returns the manual values;
calibration 0x2410 overwrites this struct (§4.5).

#### 0x1013 CrossTalk
Slot 7 → `IQ_API20_SET_GE` → IP 18 ANTICT (row 0x20 B, bank 1317); IP 17 GE is also fed but has no code. Entry (20 B): Strength
u8, StrengthByY u8[15], Threshold u8, pad, Offset u16. Row: +0 = 1 (0x27c, 1317.00 b0); +8 = 31 − Strength (0x280, 1317.05); +6
= Threshold (0x27f, 1317.04); +0xa = Offset (0x281, 1317.06, 12 bit); +0xe…+0x1c = StrengthByY (0x283, 1317.08–…).

#### 0x1017 DynamicDP
Slot 19 → `IQ_API20_SET_DPC` → IP 16 DPC (row 0xb0 B, bank 1316). Entry (44 B): HOTPIX_EN u32, HOTPIX_COMPSLPOE u16, pad,
DARKPIX_EN u32, DARKPIX_COMPSLPOE u16, DPCTH u16, BLEND_EN u32, DIFF_LUT u16[6], Y_LUT u16[6].
- Order: bypass[39] or DynamicDP_Cluster enabled → the Cluster converter first (unless 0xff); otherwise reset the DPC table to
  the IQ file. bypass[19] → row+0 = 0, return. (Cluster mirrors this with bypass[19] / DynamicDP.)
- Enable = 1, per row: +0 = 1 (0x252, 1316.00 b0); +6 HOTPIX_EN (0x255, .00 b5); +8 DARKPIX_EN (0x256, .00 b6); +0xa = (BLEND_EN
  ≠ 0) (0x257, .00 b8); +0xc…+0x12 = min(0xfff, 16·DPCTH) four times (0x258–0x25b, .01–.04); +0x24…+0x2c = min(0xfff,
  16·HOTPIX_COMPSLPOE) five times (0x260–0x264, .10–.14); +0x2e…+0x36 = min(0xfff, 16·DARKPIX_COMPSLPOE) five times
  (0x265–0x269, .15–.19).
- The two LUTs are piecewise-linear curves over **fixed x knots that stay from the IQ file**: X_d = 0x26a (row +0x38, 5 × u16,
  imx307 80 255 1100 2100 3100) and X_y = 0x26d (+0x58, same values). The converter writes DIFF_LUT[0..4] → +0x42 (0x26b,
  1316.1f…, 11 bit) and Y_LUT[0..4] → +0x62 (0x26e, .2f…), and the six segment slopes: DIFF → +0x4c (0x26c, .24…): s0 = D0·1024/X0,
  s_k = (D_k − D_{k−1})·1024/(X_k − X_{k−1}) for k = 1…4, s5 = (D5 − D4)·1024/(4095 − X4); Y → +0x6c (0x26f, .34…, signed): the
  same with the curve starting at 1024 (s0 = (Y0 − 1024)·1024/X0). So DIFF_LUT[5] / Y_LUT[5] are the curve values at x = 4095
  and reach the hardware only through the last slope (check: the imx307 rows give 20/80·1024 = 256 = 0x26c[0] and
  (700 − 1024)·1024/80 = −4147 = 0x26f[0]).
- Our bin: Enable 1, OpType 1, HOTPIX / DARKPIX / BLEND on → used.

#### 0x102b DynamicDP_Cluster
Slot 39 → `IQ_API20_SET_DPC_CLUSTER` → IP 16 DPC. Entry (68 B): EDGE_MODE u32, NEI_DELTA_TH u16, NEI_SMOOTH u8, pad, SORT_EN u32,
SORT_RGGB_EN u32[4], SORT_1x3_MODE_EN u32, SORT_LUMTBL_L / D / S u16[6] each. Enable = 1, per row: +0 = 1 (0x252); +4 EDGE_MODE
(0x254, 1316.00 b4); +0x14 NEI_SMOOTH (0x25c, .08); +0x16 NEI_DELTA_TH (0x25d, .09, 12 bit); +0x78 SORT_EN (0x270, .50 b0);
+0x7a / +0x7c / +0x7e / +0x80 SORT_RGGB_EN[0..3] (0x271–0x274, .50 b1–b4); +0x82 SORT_1x3_MODE_EN (0x275, .50 b5); +0x8a / +0x96
/ +0xa2 SORT_LUMTBL_L / D / S (0x277 / 0x278 / 0x279, .53… / .59… / .5f…, 12 bit) — all copies. Our bin: Enable 0 → unused.

#### 0x1019 RGBIR
Slot 22 → `IQ_API20_SET_RGBIR` → IP 19 RGBIR (row 0x3c B, bank 1318). Entry (44 B): IRPOS_TYPE u8, pad[3], REMOVEL_EN u32,
RATIO_R / G / B u16[6] each. bypass[22] → row+0 = 0. Enable = 1: +0 = 1 (0x286, 1318.00 b0); +2 IRPOS_TYPE (0x287, .00 b1); +6
REMOVEL_EN (0x289, .00 b3); +0x12 RATIO_R[6] (0x28d, .06…), +0x1e RATIO_G[6] (0x28e, .0c…), +0x2a RATIO_B[6] (0x28f, .12…), 12
bit, copies. Disabled → nothing. Meaningful only for an RGB-IR sensor; the GC8613 is plain Bayer and our bin has Enable 0 →
unused.

#### 0x101c DeMosaic and 0x100d AntiFalseColor (IP 23 DM, row 0x6c B, bank 1325)
**0x101c DeMosaic** → slot 30 → `IQ_API20_SET_DEMOSAIC`. Entry (24 B): DbgMode, IsoTh, GradientTh u8, pad, SmoEn u32, SmoRange,
SmoStrength u8, pad, DtlEn u32, DtlBypassColorCheck u32, DtlColorRatio, DtlColorOft, DtlHScale, DtlMScale u8.
- Order: bypass[6] or AntiFalseColor enabled → the FCOLOR converter first; else reset to the IQ file. bypass[30] → row+0 = 0.
- Enable = 1, per row: +0 = 1 (0x2da); +4 and +6 = DtlEn (0x2dc / 0x2dd, 1325.00 b8 / b9); +8 SmoEn (0x2de, b10); +0xa DbgMode
  (0x2df, b12–13); +0x12 IsoTh (0x2e3, .03); +0x14 GradientTh (0x2e4, .04); +0x16 DtlBypassColorCheck (0x2e5, .05 b0); +0x18
  DtlColorOft (0x2e6, .05 b8–13); +0x1a DtlColorRatio (0x2e7, .06); +0x4c SmoRange (0x2ee, .1b); +0x4e SmoStrength (0x2ef, .1c);
  detail scales: +0x24 and +0x2c (0x2e9 .09…, 0x2ea .0d…) = min(63, H[k]·DtlHScale >> 2), +0x3c and +0x44 (0x2ec .13…, 0x2ed .17…)
  = min(63, M[k]·DtlMScale >> 2), k = 0…7, with fixed shapes H = 0 2 4 8 4 2 0 0 and M = 0 12 16 16 12 8 4 0 (stack constants;
  scale 4 = 1.0 reproduces the imx307 file values) [R emulation with both scales = 4].
- Our bin: Enable 0 → unused (DM runs on the IQ file + AntiFalseColor).

**0x100d AntiFalseColor** → slot 6 → `IQ_API20_SET_FCOLOR`. If DeMosaic is enabled or bypassed, the DEMOSAIC converter runs
first (it owns the rest of the row); otherwise the DM API table is first reset to the IQ-file table. Entry (8 B): DbgEn u32,
ColorSpaceSel u8, Preserve u8, Strength u8, pad. Row: +2 = 1 (0x2db, 1325.00 b4 = false-colour stage on), +0xa = DbgEn ? 3 : 0
(0x2df, 1325.00 b12–13), +0x50 = ColorSpaceSel (0x2f0, 1325.1d b0), +0x56 = Preserve (0x2f3, 1325.1d b4–6), +0x5a = Strength
(0x2f5, 1325.1f).

#### 0x101b PFC and 0x102f PFC_EX (IP 33 PFC, row 0x66 B, bank 1327)
<!-- ev:isp-3-iq-api.0x101b-pfc-and-0x102f-pfc-ex-ip-33-pfc-r -->
**0x101b PFC** → slot 28 → `IQ_API20_SET_PFC`. Entry (23 B): Strength, UStrength, VStrength u8; StrengthByY u8[8]; FlatProtect
u8; SatByY u8[2]; SatSrcSel u8; StrengthByContrast u8[8].
- Order: bypass[40] or PFC_EX enabled → the PFC_EX converter first; else reset to the IQ file. bypass[28] → row+0 = 0.
- Enable = 1, per row: +0 = 1 (0x39a); +0x3e StrengthByY[8] (0x3af, 1327.2a…); +0x46 SatSrcSel (0x3b0, .30 b0); +0x50
  StrengthByContrast[8] (0x3b2, .33…); +0x5a FlatProtect (0x3b4, .3b); +0x5c Strength (0x3b5, .3c); +0x5e UStrength (0x3b6, .3d);
  +0x60 VStrength (0x3b7, .3e) — copies; SatByY expanded into two 3-step tables: +0x22…+0x24 = min(31, s1+6), min(31, s1+3),
  min(31, s1) (0x3ab, .12…) and +0x26…+0x28 = min(31, s0+4), min(31, s0+2), min(31, s0) (0x3ac, .16…), s0 / s1 = SatByY[0] / [1].
- Our bin: Enable 1, OpType 1 → used.

**0x102f PFC_EX** → slot 40 → `IQ_API20_SET_PFC_EX`. Struct (32 B): Enable, DbgEn, GSkipEn u32; SatMaskSel u8, pad[3]; SatAmpEn
u32; StrengthByHue u8[12].
- Order: if bypass[28] (PFC's slot) or PFC enabled → the PFC converter first (unless 0xff); bypassed → row+0 = 0 in all rows,
  return. Else reset the PFC API table to the IQ-file table. Its own bypass[40] is not checked.
- Enable = 1, every row: +0 = 1 (0x39a, 1327.00 b0); +4 SatAmpEn (0x39c, .01 b0); +6 DbgEn (0x39d, .01 b4); +0x20 SatMaskSel
  (0x3aa, .10 b0); +0x2a StrengthByHue[12] (0x3ad, .20…, u8 each); +0x62 GSkipEn (0x3b8, .3f b0); +0x48…+0x4e = a 7-entry 4-bit
  table (0x3b1, .31…) chosen by (SatMaskSel, SatAmpEn) from four constants in `.rodata.str1.1` [R disassembly — the decompile
  shows the four loads in the wrong order]:

  | SatMaskSel | SatAmpEn | table (0x3b1) | address |
  |---|---|---|---|
  | 0 | 0 | 3 3 4 5 6 7 7 | 0x15a12 |
  | 0 | 1 | 4 4 5 6 7 8 8 | 0x15a1a |
  | 1 | 0 | 2 2 3 4 5 6 7 | 0x15a2a |
  | 1 | 1 | 3 3 4 5 6 7 8 | 0x15a22 |
  | other | any / 0, other | 3 3 4 5 6 7 7 | 0x15a12 |

  (imx307 file value of 0x3b1: 4 4 5 6 6 7 8.) Name tables that call 0x39b–0x39d "FalseColor_*_RATIO" are misaligned; the register is the authority.
- Our bin: Enable 0 → unused.

### 3.7 Noise reduction [R; maps of 0x1010, 0x102d, 0x1011, 0x102e, 0x1037, 0x1033 by emulation]

#### 0x100f NRDeSpike
Slot 8 → `IQ_API20_SET_SpikeNR` → IP 21 SpikeNR (row 0x48 B, bank 1317). Entry (34 B): BlendRatio, Str.CenterNeighbor,
Str.MeanStd, Str.CornerCross, DiffGain.MeanStd, DiffGain.CenterNeighbor, DiffThd.CornerCross, pad, BlendLut u16[5], StrengthByY
u8[15], pad. Row: +0 = 1 (0x2c0, 1317.20 b0); +4 BlendRatio (0x2c2, .21); +6 DiffThd.CornerCross (0x2c3, .22, 10 bit); +8
DiffGain.MeanStd (0x2c4, .23); +0xa and +0x12 DiffGain.CenterNeighbor (0x2c5 .24 and 0x2c9 .28); +0xc Str.CenterNeighbor (0x2c6,
.25); +0xe Str.CornerCross (0x2c7, .26); +0x10 Str.MeanStd (0x2c8, .27); +0x1e BlendLut (0x2cb, .35–.39); +0x36 StrengthByY (0x2ce,
.41–.4f). Row bytes the converter does not write (0x2c1, 0x2ca, 0x2cc, 0x2cd) keep the IQ-file values.

#### 0x1010 NRLuma
Slot 16 → `IQ_API20_SET_NLM` → IP 1 BNR (row 0x38 B, bank 1326); IP 28 NLM is also fed (no code). Entry (34 B): Strength u8,
KernelStr u8, KernelStrByDiff u8[32]. Bypass[16] → row+0 = 0. Enable = 1 → row+0 = 1 (0x178, 1326.00 b0), +8 Strength (0x17c,
1326.11, 6 bit), +0xa KernelStr (0x17d, 1326.13, 3 bit), +0x16 KernelStrByDiff[32] (0x181, 1326.20…, u8) — all copies, nothing
else. Disabled → nothing written (no reset to the IQ file). BNR row +0xe / +0x10 (weight count) belong to Dummy (below). Our bin:
Enable 1, OpType 0 (auto, 16 entries) → used.

#### 0x102d NRLuma_Adv
Slot 44 → `IQ_API20_SET_GNR` → IP 34 GNR (row 0xc0 B, banks 1330 / 1331). Entry (146 B): DbgMode u8, Strength u8[2],
StrengthByMot u8[16], SmoDeltaSft u8, SmoDeltaLut u8[16], EdgeTh u16[16], SmoDeltaTh u16[16], StrengthByHue u8[24],
StrengthBySat u8[2], SmoByMot u8[16] (XML item order; the memory order can differ).
- Not bypassed — **whether enabled or not** — every row gets +2, +6, +0xa = 1 (0x3bc / 0x3be / 0x3c0 = 1330.00 b1 / b3 / b6).
  bypass[44] = 1 → those three = 0 and +0x30 / +0x32 (Strength, 0x3c5) = 0, return.
- Enable = 1: row+0 = 1 (0x3bb, 1330.00 b0) and

| field | n | row offset | enum → register | value |
|---|---|---|---|---|
| DbgMode | 1 | +0x8 | 0x3bf → 1330.00 b4–5 | copy |
| StrengthByMot | 16 | +0x10 | 0x3c3 → 1330.02… | copy |
| Strength | 2 | +0x30 | 0x3c5 → 1330.12… (9 bit) | copy |
| SmoDeltaSft | 1 | +0x34 | 0x3c6 → 1330.14 (4 bit) | copy |
| SmoDeltaLut | 16 | +0x36 | 0x3c7 → 1330.18… | copy |
| EdgeTh | 16 | +0x46 | 0x3c8 → 1330.20… (13 bit) | copy |
| SmoDeltaTh | 16 | +0x66 | 0x3c9 → 1330.30… (10 bit) | copy |
| StrengthByHue | 24 | +0x86 | 0x3ca → 1330.40… | copy |
| StrengthBySat | 2 | +0x9e | 0x3cb → 1330.4c… | copy |
| SmoByMot | 16 | +0xae | 0x3d0 → 1330.58… | copy |

  The Oil effect later zeroes EdgeTh (0x3c8) and sets Strength to 0x100 in the output block (§3.5 Effect).
- Our bin: Enable 1, OpType 1 (manual) → used.

#### 0x1011 NRChroma
Slot 33 → `IQ_API20_SET_MXNR` → IP 38 MXNR (row 0x1a B, bank 133E). Entry (16 B): MatchRatio u8, pad, UvTh u16, StrengthByCEdge
u16[6]. Bypass[33] → row+0 = 0. Enable = 1: +0 = 1 (0x3eb, 133E.00 b0); +4 = 31 − MatchRatio (0x3ed, .01, 7 bit); +8 **and** +0xa
= UvTh (0x3ef .02, 0x3f0 .03, 10 bit); +0xc…+0x16 = 0x1ff − StrengthByCEdge[k] (0x3f1, .04–.09, 10 bit) — the API's "strength" is
stored inverted as a threshold. Disabled → nothing. Our bin: Enable 0 → unused.

#### 0x102e NRChroma_Adv
Slot 29 → `IQ_API20_SET_SIIR` → IP 31 SIIR (row 0x60 B, bank 133A) and IP 35 UVM (row 0x16 B, bank 133D). Entry (45 B):
StrengthByY u8[8], StrengthByYEdge u8[8], StrengthByCEdge u8[8], MaxStrength u8, StrengthByMot u8[16], MotionClip u8,
MotionColorReduce u8, MotionColorRecover u8, PreStrength u8.
- bypass[29] → SIIR row+0 = 0 and UVM row+2 = 0. Enable = 1, per row:

| field | row offset (SIIR unless noted) | enum → register | value |
|---|---|---|---|
| (enable) | +0; UVM +2 | 0x386 → 133A.00 b0; 0x3d4 → 133D.1d b0 | 1, 1 |
| MaxStrength | +0x8 and +0xe | 0x38a → 133A.03; 0x38d → .05 | copy, twice |
| MotionClip | +0x12; +0x14 | 0x38f → 133A.06 (11 bit); 0x390 → .07 | clip·3/2 (u16); copy |
| StrengthByYEdge[8] | +0x16 | 0x391 → 133A.08… | copy |
| StrengthByYEdge[8] | +0x36 | 0x395 → 133A.28… | entry 0: min(255, 4·v); entries 1–7: min(255, 32·v/10) |
| StrengthByY[8] | +0x1e | 0x392 → 133A.10… | copy |
| StrengthByCEdge[8] | +0x26 and +0x2e | 0x393 → 133A.18…; 0x394 → .20… | copy, twice |
| MotionColorReduce | UVM +4 (16 B) | 0x3d5 → 133D.20… | U[k] = 128 − Reduce·(128 − C[k])/255, C = 0 0 1 2 3 5 6 9 12 16 21 27 42 64 95 128 (constant in `.rodata`) |
| MotionColorRecover | +0x3e (16 B) | 0x396 → 133A.30… | S[j] = min(255, 128 + Recover·(16384/max(U[15−j], 1) − 128)/255) — reversed index, inverse of U |
| StrengthByMot[16] | +0x4e…+0x5d **reversed** ([0] → +0x5d, [15] → +0x4e) | 0x397 → 133A.38… | copy |
| PreStrength | UVM +0 | 0x3d3 → 133D.1b | copy |

  So Reduce = 0 gives a flat 128 UVM curve (no motion-dependent chroma reduction) and Recover = 0 a flat 128 SIIR curve; the C
  table is the fixed reduction shape [R emulation with Reduce / Recover set to 0, 128, 255].
- Our bin: Enable 0 → unused.

#### 0x1037 NRChroma_Pre
Slot 49 → `IQ_API20_SET_XNR` → IP 29 XNR (row 0x26 B, bank 133D). Entry (24 B): DbgEn u32, Strength u16, MotionEnhance u8[2],
MaskGenTh u16, MeanFilterTh u16[2], MedianFilterTh u16[2], BlendTh u16, BlendGap u8, pad. bypass[49] → row+0 = 0. Enable = 1, per
row: +0 = 1 (0x372, 133D.00 b0); MaskGenTh → +2, +4, +6, +8 (0x373–0x376, .04–.07, the same value four times); MeanFilterTh[0] →
+0xa (0x377, .08), [1] → +0xc and +0xe (0x378 .09, 0x379 .0a); MedianFilterTh[0] → +0x10 (0x37a, .0b), [1] → +0x12 and +0x14
(0x37b .0c, 0x37c .0d); +0x16 = DbgEn ? 3 : 2 (0x37d, .10 b0–1); BlendTh → +0x18 (0x37e, .12); BlendGap → +0x1a (0x37f, .13, 4
bit); MotionEnhance[0] → +0x1c (0x380, .16), [1] → +0x1e and +0x20 (0x381 .17, 0x382 .18); +0x22 = 256 − Strength (0x383, .1c, 9
bit). Disabled → nothing. Our bin: Enable 0 → unused.

#### 0x100e NR3D
<!-- ev:isp-3-iq-api.0x100e-nr3d -->
Slot 10 → `IQ_API20_SET_NR3D` → IP 2 NR3D (row 0x2fa B, banks 1328 / 132A) + two side tables; slots 28 NLM and 32 ACLUT are also
fed (no IP code). The exported function only does the order / bypass part and tail-jumps to the unnamed body at 0x6635c
(Ghidra's listing of `IQ_API20_SET_NR3D` already shows it inlined). Entry (124 B): MdThd u8, pad, MdGain u16, TfStr u8, TfStrEx
u8, MdThdByY u8[16], MdGainByY u8[16], M2SLut u8[16], TfLut u16[16], YSfBlendLut u8[16], YSfBlendOffset u8, MotHistDelayByDiff
u8[16], pad, DeDotDiffOffset u16, DeDotDiffShift u8, pad.
- Order: bypass[43] or NR3D_EX enabled → the NR3D_EX converter first (unless 0xff); otherwise the API table is reset to the
  IQ-file table. Then bypass[10] = 1 → row+0xa and row+0xc = 0 in all rows (TfStr / TfStrEx → no temporal filtering) and the
  "first enable" latch ctx+0x54706 = 0; return.
- Enable = 1, row i from auto[i] (OpType 0) or manual:

  | row offset | enum → register | value |
  |---|---|---|
  | +0 | 0x184 → 1328.03 b0 | 1 |
  | +6 / +8 | 0x187 → .06 (12 bit) / 0x188 → .06 b12–14 | DeDotDiffOffset / DeDotDiffShift |
  | +0xa / +0xc | 0x189 → .20 b0–6 / 0x18a → .20 b8–14 | TfStr / TfStrEx |
  | +0x14 | 0x18e → .31… (5 bit × 16) | M2SLut[16] |
  | +0x28 | 0x191 → .3c… (3 bit × 16) | MotHistDelayByDiff[16] |
  | +0x42 | 0x197 → .5a… (× 16) | max(0, 16 − YSfBlendLut[j]) |
  | +0x52 | 0x198 → .62… (× 16) | clamp(YSfBlendOffset + 16 − YSfBlendLut[j], 0, 16) |
  | +0x192 / +0x194 | 0x1ac / 0x1ad (no register) | MdThd / MdGain (u16) |
  | +0x226 | 0x1b4 → 132A.50… (12 bit × 16) | TfLut[16] |

  The two by-Y arrays go **outside the API table**: MdGainByY → ctx+0x53ccc + 12·i, MdThdByY → ctx+0x53d8c + 12·i (16 bytes copied
  per row at a 12-byte stride, so each row's last 4 bytes are overwritten by the next row; both regions are memset to 0x40 by
  `hal_IQ_InitParameters`) [R disassembly 0x6649a: `add r8, r5, #192`, `adds r5, #12`].
- Consumer of MdThd / MdGain and the by-Y side tables: `IQ_Patch_NR3D_calc_motion_th_gain` interpolates the side-table rows of the
  two ISO steps around the current gain, multiplies MdThdByY[j]·MdThd with the noise-mean array (thresholds) and MdGainByY[j]·MdGain
  with the noise-std array (gains), `>>11`, clip 0x3fff (thresholds) or 0x1000/x (gains), for **12** luma bins — entries 12–15 of
  MdThdByY / MdGainByY are never used [R loop: output arrays 12 u16 apart]. A debug override word (u16 0xdb01: take k1 / k2 from
  +0x1fa instead of MdThd / MdGain; 0xdb02: print them) sits in the NE block at IqInput+0x78 +0x1f8 and is unreachable,
  because `ISP_DNRReadIQNoiseEst` zeroes it every frame.
- **First-frame pulse.** `IspRegSetNR3D` uses the latch: the first run with TfStr (output +0xa) ≠ 0 and the latch clear sets shadow
  byte +0x2410 bit 0 for that run, sets the latch and calls `pIQ_IQTrigger(ctx, 4, 2)` (re-run IP 2 next frame); otherwise it clears
  the bit. Shadow +0x2410 is bank code 0x28 register 0x04 = **1328.04[0]**, which `HalIsp_Apply3DNR` copies to the hardware: the
  3DNR first-frame trigger, the bit `ISP_DNRTrigger1stFrame` sets (isp-1-hal.md §8.1). So whenever the temporal filter is switched
  on again after being off (TfStr 0, bypass) the 3DNR restarts its temporal history on that frame [R mechanism; I purpose]. The
  software clears the shadow bit on the next run; whether the hardware bit clears itself is not known (the HAL never writes 0 to
  it).
- Our bin: Enable 1, OpType 1 (manual) → the manual set runs in all 16 rows on this camera.
- [S] (SigmaStar «ISP API Tuning SOP» v1.3 for Tiramisu, §3.3.1, as shown in a SigmaStar
  training video and in the open `iq_zh` page): MdThd 0–255, the motion threshold — motion below it counts as static, larger =
  stronger 3DNR, «建议不要设超过10» (do not set above 10); MdGain 0–10230, motion scale, larger = less motion = stronger 3DNR;
  MdThdByY / MdGainByY 0–255, the same per luma bin; TfStr / TfStrEx 0–64, strength and extra strength, normally left at 64 / 0
  (the open page suggests TfStr 63); M2SLut 0–31, strength of the moving → static transition, larger = weaker 3DNR and stronger
  2D NR where motion ends (page example `{24,18,11,8,7,7,6,6,6,5,5,5,5,4,4,4}`); TfLut 0–4095, temporal-filter strength over
  difference and motion, falling, left = static, right = moving; YSfBlendLut 0–16, NRLuma strength by motion (left moving →
  right static); YSfBlendOffset 0–16, how much 3DNR is blended back into the reference in moving parts; MotHistDelayByDiff 0–7,
  extra frames the motion information is held, only with NR3D_EX MotHistMapSel = 1; DeDotDiffOffset / Shift keep single
  blinking pixels from turning the motion decision. Too strong a 3DNR leaves trails behind moving objects (运动物体容易出现残影).
  With a noise calibration applied (`ne_cali.data`, `MI_ISP_API_CmdLoadCaliData`, «Cali Item = NE») the SOP's defaults are
  MdThd / MdGain 64 / 400 and MdThdByY / MdGainByY 64.

#### 0x1032 NR3D_EX
Slot 43 → `IQ_API20_SET_NR3D_EX` → IP 2 NR3D (row 0x2fa B, bank 1328). Struct (36 B): Enable, DbgEn u32; WinSizeSel u8,
MotHistMapSel u8, pad[2]; AREn u32; ARLumaTh u8[2], ARMotTh u8[2]; PREn u32; PRDiffTh u16; PRRatioConf u8[4]; PRContrastTh u8[2];
PRMotTh u8[2]; pad[2].
- Order: if bypass[10] (NR3D's slot) = 1 → run the NR3D converter (unless 0xff) and, if still bypassed, write 0 into row+0xa and
  row+0xc of all rows (0x189 = 1328.20 b0–6, IQ file 63 per row; 0x18a = 1328.20 b8–14, IQ file 0 — the two strengths the NR3D
  converter's own bypass also zeroes), return. Else if NR3D is enabled → the NR3D converter first; if NR3D is disabled → reset the
  NR3D API table to the IQ-file table (0x2fa0 B). Its own bypass[43] is not checked.
- Enable = 1, every row: +0 = 1 (0x184, 1328.03 b0); +2 WinSizeSel (0x185,.05 b8–9); +4 DbgEn (0x186,.05 b15); +0x26
  MotHistMapSel (0x190,.3c b2); +0x38 AREn (0x192,.44 b0); +0x3a ARMotTh[2] (0x193,.45); +0x3c ARLumaTh[2] (0x194,.46); +0x62 PREn
  (0x199,.70 b0); +0x64 PRDiffTh (0x19a,.71, 12 bit); +0x66 PRMotTh[2] (0x19b,.72); +0x6a PRContrastTh[2] (0x19d,.74, 4 bit);
  +0x6e PRRatioConf[4] (0x19f,.76, 5 bit). All values copied unchanged into all 16 rows (no per-ISO form).
- Names [I from the XML]: AR = artifact reduction (luma / motion thresholds), PR = a second motion-protection stage.
- [S] (the Tiramisu tuning SOP as shown in a SigmaStar training video): DbgEn shows the motion map (darker =
  judged moving; AE paused first); WinSizeSel 0–2, the motion window (larger = better noise estimate, moving edges more often
  called static); MotHistMapSel 1 = motion information falls by at most 1 per frame from moving to static; AREn with ARLumaTh /
  ARMotTh = «Alpha Blending Refine» — below Th[0] NR3D unchanged, above Th[1] NR3D strength 0, by luma and by motion; PR* = a
  purple-fringe helper used only when the picture is rotated. This replaces the reading of the line above for PR.
- Our bin: Enable 0 → unused.

#### 0x1033 Dummy — BNR weight count and an AE-driven 3DNR override
Slot 45 → `IQ_API20_SET_DUMMY`. Struct (26192 B): Enable, OpType, LogEn u32; auto[16] {Dummy0..2 s16[256], Dummy3, Dummy4 s16}
(1540 B each); manual (same).
- Converter: Enable = 1 → for row i, v = Dummy3 of auto[i] (or manual): v = 255 → 256; v = −1 → BNR row +0xe / +0x10 copied from the
  IQ-file row; v < 0 → 0; else min(v, 256); written to BNR row +0xe and +0x10 (0x17f[0..1], 1326.15…, 9 bit; LogEn text: "Dummy3 =
  IQ_BNR_WEICNT_Y[0~1]"). Disabled → both restored from the IQ file. Then `pIQ_IQTrigger(ctx, 4, 1)` (re-run IP 1 BNR).
- Dummy0[0..3] = AEDiff_Th, MdTh_Ratio, MdGain_Ratio, LogEn (LogEn text), read at run time, not by the converter: `hal_DoIQ` →
  `IQ_Patch_NR3D_bAEAccControlMdThGain` re-runs IP 2 (NR3D) in every DoIq while Dummy is enabled and the four values of both ISO
  entries around the current index (auto) or of manual are ≠ −1; inside the NR3D patch, `IQ_Patch_NR3D_AEAccControlMdThGain`
  compares the AE block statistics (IqInput+0x54 → 1024 × 4 B, byte 3 of each) with the copy kept from the previous frame (static
  buffer in mhal `.data`), takes the largest absolute difference D, and if D < AEDiff_Th (static scene) scales the NR3D output
  MdThd = min(255, MdTh_Ratio·MdThd >> 6) and MdGain = min(0x27f6, MdGain_Ratio·MdGain >> 6); LogEn ≠ 0 prints "[AE Acc]
  AEDiff_Max …" each time. Auto values are interpolated between the two ISO entries.
- Unused parts: Dummy0[4..255], Dummy1, Dummy2, Dummy4.
- Not in our bin → unused.

### 3.8 Sharpness [R emulation, order and bypass from the disassembly]

#### 0x1012 Sharpness
<!-- ev:isp-3-iq-api.0x1012-sharpness -->
Slot 9 → `IQ_API20_SET_YEE` → IP 30 YEE (row 0x290 B, banks 1333 / 1334 / 1335 / 133A); slot 32 ACLUT also fed (no code).
- Order: bypass[42] (Sharpness_EX's slot) or Sharpness_EX enabled → the YEE_EX converter first (unless 0xff); otherwise reset the
  YEE API table to the IQ-file table **and always run the DUMMY_EX converter (slot 46) with 0xff** [R disassembly 0x6c49c: `movs
  r1, #46` before `hal_IQ_API20_SET_ALL`] — DUMMY_EX owns three YEE fields and the Effect switch, so a table reset must replay it.
  Then bypass[9] = 1 → row+0x56 = 0 in all rows (0x322, 133A.72 b0 = YEE off), return.
- Enable = 1, every row: +0x56 = 1 (0x322, 133A.72 b0), and:

| field | n | row offset | enum → register | value |
|---|---|---|---|---|
| SharpnessD [0], [1], [2] | 3 | +0x6a, +0x6e, +0x7e | 0x328 → 1333.4e lo; 0x32a → .4f lo; 0x332 → .54 lo | copy |
| SharpnessUD [0], [1], [2] | 3 | +0x6c, +0x70, +0x80 | 0x329 → 1333.4e hi; 0x32b → .4f hi; 0x333 → .54 hi | copy |
| PreCorD [0], [1], [2] | 3 | +0x76, +0x7a, +0x84 | 0x32e → 1333.52 lo; 0x330 → .53 lo; 0x335 → .56 lo | copy |
| PreCorUD [0], [1], [2] | 3 | +0x78, +0x7c, +0x86 | 0x32f → 1333.52 hi; 0x331 → .53 hi; 0x336 → .56 hi | copy |
| StdAdjByMot | 16 | +0xbe | 0x33d → 1334.01… (u8) | copy |
| MidRatioDByMot | 16 | +0xf8 | 0x349 → 1334.17… (9 bit) | copy |
| MidRatioUDByMot | 16 | +0x118 | 0x34a → 1334.27… (9 bit) | copy |
| HighRatioDByState [1], [0], [2] | 3 | +0x138, +0x13a, +0x13c | 0x34b `IQ_YEE_HIGH_DIR`[0..2] → 1334.37 / .38 / .39 (9 bit) — entries 0 and 1 **swapped** | copy |
| HighRatioUDByState [1], [0], [2] | 3 | +0x13e, +0x140, +0x142 | 0x34c `IQ_YEE_HIGH_ALL`[0..2] → 1334.3a / .3b / .3c — entries 0 and 1 swapped | copy |
| DetailGainUDByMot | 16 | +0x14a | 0x350 → 1334.3e… (9 bit) | copy |
| DirRatioByState | 9 | +0x16a | 0x351 `IQ_YEE_LABELE_WEISTR` → 1334.4e… (9 bit) — not permuted | copy |
| StdAdjByY | 9 | +0x190 | 0x354 → 1334.65… (u8) | copy |
| GainByStd | 7 | +0x1a0 | 0x356 → 1334.6c… (u8) | copy |
| CorByMot | 16 | +0x1b4 | 0x35d → 1335.03… (u8) | copy |
| CorByY | 9 | +0x1c4 **and** +0x1ce | 0x35e → 1335.0b…; 0x35f → 1335.0f… hi — the same table twice | copy |
| OverShootGain × SclByY | 9 | +0x1d8 | 0x360 → 1335.14… | min(255, SclByY[k]·OverShootGain >> 6) (product ≥ 0x4000 → 255) |
| UnderShootGain × SclByY | 9 | +0x1e2 | 0x361 → 1335.18… hi | min(255, SclByY[k]·UnderShootGain >> 6) |
| EdgeKillLut | 7 | +0x1f2 **and** +0x200 | 0x363 → 1335.20…; 0x364 → 1335.27… (10 bit) — twice | copy |
| OverShootLimit / UnderShootLimit | 1 / 1 | +0x20e / +0x210 | 0x365 → 1335.2e; 0x366 → 1335.2f (10 bit) | copy |
| GainByMot | 16 | +0x212 | 0x367 → 1335.30… (u8) | copy |

  Every struct element is read; none is dropped. ("ByMot" = 16 motion bins, "ByState" = the 3 edge states, "ByY" = 9 luma bins
  [names from the XML].)
- **HighRatio*ByState order.** Both directions swap: the converter writes API [1], [0], [2] to elements 0, 1, 2, and
  `hal_IQ_API20_InitParameters` fills API [0] from element 1, [1] from element 0, [2] from element 2, for every auto row and for
  manual (auto[0].AUTO_HighRatioDByState[0] ← IQ file +0x117bc = `IQ_YEE_HIGH_DIR[1]`). It
  is a fixed, consistent permutation between the API's state order and the register order: a Get after a Set returns what was
  set; API HighRatioDByState[0] → 1334.38, [1] → 1334.37, [2] → 1334.39; HighRatioUDByState[0] → 1334.3b, [1] → 1334.3a, [2] →
  1334.3c (one element per register, 9-bit mask) [R]. A one-sided bug would break the round trip; both functions carry the swap,
  so it is how the vendor's API orders the three edge states relative to the hardware [I]. imx307: `IQ_YEE_HIGH_ALL` = 200, 86,
  86 → a Get returns HighRatioUDByState = 86, 200, 86; `IQ_YEE_HIGH_DIR` = 256 × 3.
- Our bin: Enable 1, OpType 1 (manual) → used.

#### 0x1031 Sharpness_EX
Slot 42 → `IQ_API20_SET_YEE_EX` → IP 30 YEE. Entry (92 B): DbgEn u32, SmoNRFilterSel u8, pad, SmoDiffThByY u16[8], SmoBlendByStd
u8[7], SmoBlendByMot u8[16], SblLut u8[6], EdgeLabelSel u8, GainByStdSft u8[6], EdgeKillLutSft u8[6], StrengthByHue u8[24],
StrengthBySat u8[2], pad[2].
- Order: bypass[9] or Sharpness enabled → the Sharpness converter first (unless 0xff); otherwise reset the YEE table to the IQ
  file and run DUMMY_EX (as above). bypass[9] → row+0x56 = 0, return. **Its own bypass[42] is not checked** (only used by the
  Sharpness converter to decide the order) — the same as NR3D_EX, PFC_EX and HDR_EX, which ignore bypass[43] / [40] / [41].
- Enable = 1:

| field | n | row offset | enum → register | value |
|---|---|---|---|---|
| StrengthByHue | 24 | +0x12 | 0x31b → 133A.50… (u8) | copy |
| StrengthBySat | 2 | +0x2a | 0x31c → 133A.5c… | copy |
| DbgEn | 1 | +0x5a | 0x324 → 133A.72 b2 | copy |
| SblLut | 6 | +0x64 | 0x327 → 1333.4a… hi | copy |
| SmoDiffThByY | 8 | +0x9c | 0x33a → 1333.69… (10 bit) | copy |
| SmoBlendByMot | 16 | +0xac | 0x33b → 1333.71… | copy |
| SmoBlendByStd | 7 | +0xd4 **and** +0xe2 | 0x33f → 1334.0a… hi; 0x341 → 1334.0f… hi | copy |
| SmoNRFilterSel | 1 | +0xea, +0xec, +0xee | 0x342 / 0x343 / 0x344 → 1334.13 b0 / b1 / b2 (the same value into three 1-bit fields) | copy |
| EdgeLabelSel | 1 | +0xf0 | 0x345 → 1334.15 b0 | copy |
| GainByStdSft | 6 | +0x19a | 0x355 → 1334.6a… (4 bit) | copy |
| EdgeKillLutSft | 6 | +0x1ec | 0x362 → 1335.1d… (4 bit) | copy |

  The converter writes no enable byte of its own (row+0x56 stays as Sharpness or the IQ file left it).
- Our bin: Enable 1, OpType 1 → used.

#### 0x1034 Dummy_EX — a side channel into YEE and Effect
Slot 46 → `IQ_API20_SET_DUMMY_EX`. Struct (1548 B): Enable, LogEn u32; Dummy0..2 s16[256]; Dummy3, Dummy4 s16. Only Dummy0[0..2]
and Dummy1[0..2] are read (LogEn prints their meaning: "Dummy0[0~2] = IQ_YEE_STATE_COR, IQ_YEE_STATE_GAIN, IQ_YEE_STATE_SFT",
"Dummy1[0~2] = EFFECT.bEnable, EFFECT.eStyle, EFFECT.u8Level"); Dummy0[3..255], Dummy1[3..255], Dummy2, Dummy3 and Dummy4 are
unused.
- Enable = 1: in every YEE API row, +0xf2 (0x346, 1334.16 lo) = clamp(Dummy0[0], 0, 255), +0xf4 = clamp(Dummy0[1], 0, 7), +0xf6 =
  clamp(Dummy0[2], 0, 3) — or, if Dummy0[0] = −1, those three bytes copied from the IQ-file row. Then if Dummy1[0] ≠ −1:
  Effect.bEnable = (Dummy1[0] > 0), eStyle = clamp(Dummy1[1], 0, 8), u8Level = clamp(Dummy1[2], 0, 100), written straight into
  the 0x101f struct; if Dummy1[0] = −1, Effect.bEnable = 0.
- Disabled: the three YEE state bytes are restored from the IQ file and **Effect.bEnable is set to 0**.
- Always at the end: the Effect converter (re-trigger) and `pIQ_IQTrigger(ctx, 4, 30)` (re-run IP 30 YEE).
- So every Sharpness / Sharpness_EX Set made while Sharpness_EX is disabled runs DUMMY_EX disabled and switches any Effect off
  [R chain]. On this camera Sharpness_EX is enabled, so this path is not taken; Dummy_EX is not in our bin.

### 3.9 Tone: WDR, YUV gamma, defog, HDR [R; maps of 0x1016, 0x1035, 0x1009, 0x101e by emulation]

#### 0x1016 WDR
Slot 21 → `IQ_API20_SET_WDR_LOC` → IP 20 WDRLOC (row 0x202 B, banks 1321 / 1322 + menuload WDR tables). Entry (96 B): BoxNum u8,
pad[3], AutoDetailEnhance u32, ManualDetailEnhance u8, GlobalDarkToneEnhance u8, WDRStrByY u8[33], Strength u8, DarkLimit u8,
BrightLimit u8, StrengthByOpsCnt u16[8], DeSatSrcSel u8, DeSatLut u8[33].
- bypass[21] → row+0 = 0. Disabled → nothing written. Enable = 1, row i (auto[i] or manual):

| row offset | enum → register | value |
|---|---|---|
| +0 | 0x294 → 1322.00 b0 | 1 |
| +0x2 | 0x295 (no register) | BoxNum |
| +0xc | 0x29a → 1322.00 b1 | ~AutoDetailEnhance (byte NOT: 1 → bit 0, 0 → bit 1) |
| +0x1a | 0x2a1 → 1322.00 b14–15 | DeSatSrcSel |
| +0x20 | 0x2a4 → 1322.05 | Strength through a fixed 3-knot curve (0,0)–(40,128)–(255,255); **255 if IqInput+0x50 = 1** (HDR input) |
| +0x38 | 0x2b0 → 1322.34 | Strength through (0,0)–(40,138)–(255,255) |
| +0x3a | 0x2b1 → 1322.35 (12 bit) | min(0xfff, (255 − BrightLimit)·16 + 16) |
| +0x3c | 0x2b2 → 1322.36 | min(0xfff, (255 − DarkLimit)·16 + 16) |
| +0x3e | 0x2b3 → 1322.37 | ManualDetailEnhance |
| +0x48 | 0x2b5 → 1322.76… (12 bit) | StrengthByOpsCnt[8] |
| +0x118 | 0x2ba (menuload table) | WDRStrByY[k] << 4, 33 knots |
| +0x1de | 0x2bd (menuload table) | DeSatLut[33] |
| +0x58 / +0x78 / +0xb8 / +0xd8 | 0x2b6–0x2b9 (menuload global-tone shift / LUT, two copies) | only when WDRCurveFull GlobalTone is **not** enabled: copied from **IQ-file row number GlobalDarkToneEnhance** (+0x58 → +0x58 and +0xb8, 31 B; +0x78 → +0x78, +0xd8 → +0xd8, 64 B) — the 16 ISO rows of the IQ file serve as 16 preset tone curves and GlobalDarkToneEnhance picks one |

  The two strength curves are `Interp_LUT_IQ` over stack constants {3 knots: x 0, 40, 255; y 0, 128 / 138, 255} [R emulation:
  Strength swept 0…255, outputs linear between those knots]. The imx307 file's own global-tone rows are identity (0, 128, …).
- Our bin: Enable 1, OpType 1 → used.

#### 0x1035 WDRCurveFull
Slot 47 → `IQ_API20_SET_WDRCurveFull` → IP 20 WDRLOC. Struct (3900 B): ENABLE_GlobalTone, ENABLE_Curve1, ENABLE_Curve2,
OP_TYPE_GlobalTone, OP_TYPE_Curve1, OP_TYPE_Curve2 u32; auto[16] {GlobalToneSft u8[31], pad, GlobalToneLut u16[32], Curve1
u16[33], Curve2 u16[33]} (228 B); manual (same).
- Always first: reset the whole WDRLOC API table to the IQ file and run the WDR converter (slot 21, with 0xff), so the WDR API is
  re-applied underneath. No bypass check of its own slot 47.
- GlobalTone enabled: per row, GlobalToneSft[31] → +0x58 and +0xb8 (0x2b6, 0x2b8), min(0xfff, GlobalToneLut[32]) → +0x78 and
  +0xd8 (0x2b7, 0x2b9) — both copies of the tone curve get the same data.
- Curve1 enabled: per row, for k = 0…32 (x_k = 128·k): +0x19c+2k (0x2bc) = clamp(F1[k]·Curve1[k] / max(x_k, 1), 0, 0xfff), with
  F1 = 0x2bc of **IQ-file row 0** (imx307 0, 584, 1064, 1387, …) — the API value is a per-knot gain relative to the identity
  (Curve1[k] = x_k leaves the file curve unchanged) [R formula; I reading].
- Curve2 enabled: the same over 0x2bb (+0x15a, file row 0: 0, 471, 942, 1420, …), clamp 0xffff.
- Each part has its own OpType (auto rows from auto[i], else manual). Our bin: all three enables 0 → unused.
- `IQ_API20_SET_WDRCurveAdv` (slot 32, struct 0x1026 {u32}) is an empty function and not in the switch — dead.

#### 0x1009 YUVGamma
Slot 23 → `IQ_API20_SET_YUVGAMA` → IP 39 YUVGAMA (row 0x40e B, bank 133E + Y / U / V LUTs by menuload). Entry (1024 B): Y u16[256],
U u16[128], V u16[128].
- Enable = 1: per row, Y → +4 (0x3f6, 10 bit), U → +0x204 (0x3f7, 9 bit), V → +0x304 (0x3f8, 9 bit) — plain copies — and +0 = 1
  (0x3f4, 133E.40 b0). Bytes +2 (0x3f5) and +0x404…+0x40a (0x3f9–0x3fc, 133E.4b–.4e) stay at the IQ-file values.
- Disabled: the three LUTs of every row are copied back from the IQ-file table; the enable byte is left alone.
- bypass[23] = 1: **not an off switch** — every row gets +0 = 1 and identity curves: Y[k] = 4k (0…1020), U[k] = V[k] = 4k (0…508).
- Defog (0x100b) runs this converter first and then rewrites the Y LUT (below).
- Our bin: Enable 0 → the IQ-file (imx307) YUV gamma is used.

#### 0x100b Defog
Slot 17 → `IQ_API20_SET_DEFOG` → IP 39 YUVGAMA (row 0x40e B, bank 133E + Y-gamma LUT by menuload); IP 20 WDRLOC is also fed but not
written. Struct (28 B): Enable u32, OpType u32, auto[16].Strength u8, manual.Strength u8, pad[3].
1. Unless called from inside another converter (argument 0xff), it first runs the YUVGamma converter (slot 23), so Defog always
   works on a freshly built YUVGAMA API table.
2. If bypass[17] ≠ 1 and Enable = 1, for each row i: s = OpType ? manual.Strength : auto[i].Strength; the row's Y LUT (enum 0x3f6,
   u16[256] at row+4, 10-bit) is copied to scratch ctx+0x57c38 and rebuilt by `LutGen_GAMA_EFFECT(ctx, dst = row+4, src =
   scratch, work = ctx+0x57e38, b = s, l = 50, c = 50)`; row+0 (0x3f4, 133E.40 b0) = 1. With l = c = 50 the effect curve reduces
   to the brightness term of §3.5: b' = clamp(s+14, 14, 114), g = −10·(64−b') for b' ≤ 64 else +30·(b'−64) (per mille), E[x] =
   min(1023, 4x·(1000+g)/1000), Y'[x] = Y[E[x]·255/1023] (rounded, kept strictly increasing, clip 1023). So s = 50 is neutral,
   s = 100 reads the Y curve at 2.5× the input (brighter, shadows lifted), s = 0 at 0.5× (darker) — "defog" is a pure input-axis
   stretch of the YUV Y gamma, not a local-contrast operation [R formula; I reading].
3. Defog disabled: nothing is written by this converter; the YUVGAMA rows stay as the YUVGamma converter left them. WDRLOC is put
   into API mode by slot 17 being enabled and then runs from its API table (IQ-file copy unless 0x1016 / 0x1035 wrote it).
- Our bin: Enable 0 (OpType 1, manual 48, auto 50 × 16) → unused; prog_uvc's menu sets it at run time (§1.4).

#### 0x101e HDR
Slot 26 → `IQ_API20_SET_HDR` → IP 14 HDR (row 0xee B, bank 131E). Entry (56 B): NrEn u32, YwtTh1 u16, YwtTh2 u16, NoiseLevel
u16[16], MotionTh u16, MotionAdjLut u16[7]. Order with HDR_EX as for PFC / PFC_EX; bypass[26] → row+0 = 0. Enable = 1, per row:
+0 = 1 (0x205); +0x12 NrEn (0x20e, 131E.02 b0); +0x58 YwtTh1 (0x222, .2c, 10 bit); +0x5a YwtTh2 (0x223, .2d); +0x5e MotionTh (0x225,
.2f, 9 bit); +0x60 NoiseLevel[16] (0x226, .30…, 10 bit); MotionAdjLut[7] → +0x8a **and** +0x9e (0x22a .44…, 0x22c .4d…, 11 bit) —
copies.

#### 0x1030 HDR_EX
Slot 41 → `IQ_API20_SET_HDR_EX` → IP 14 HDR (row 0xee B, bank 131E). Struct (28 B): Enable, DbgEn u32; SensorExpRatio u16, pad[2];
DynRatioEn u32; DynRatioSrcSel, YSrcSel, NoiseLevelSrcSel, MotionMaxVal u8; MotionLumaTh u16; MotionAdjSft u8[6].
- Order as PFC_EX: bypass[26] or HDR enabled → the HDR converter first; bypassed → row+0 = 0, return; else reset to the IQ-file
  table (0xee0 B). Its own bypass[41] is not checked.
- Enable = 1, every row: +0 = 1 (0x205, 131E.00 b0); +0xc DbgEn (0x20b, .00 b8); +0xe SensorExpRatio (0x20c — no register, read by
  the HDR IP code, file value 16384); +0x20 DynRatioSrcSel (0x215, .06 b14); +0x22 DynRatioEn (0x216, .06 b15); +0x4e YSrcSel (0x21d,
  .2b b0–1); +0x52 MotionMaxVal (0x21f, .2b b4–5); +0x54 NoiseLevelSrcSel (0x220, .2b b6–7); +0x82 MotionLumaTh (0x228, .41, 10 bit);
  MotionAdjSft[6] **twice**: +0x84 (0x229, .42…) and +0x98 (0x22b, .4b…) — both 4-bit shift tables get the same six values.
- HDR runs only in an HDR pipe (`hal_DoIQ` runs IP 14 in HDR mode); this camera's pipe is linear (HDR mode 0,
  isp-2-ispmid-mload-ispscl.md §1.1) and our bin has Enable 0 for both HDR and HDR_EX → unused.

### 3.10 Shading: LSC and ALSC [R; maps by emulation]

#### 0x1021 LSC
Slot 35 → `IQ_API20_SET_LSC` → IP 10 LSC (3 colour-temperature rows × 0xd2 B, bank 1319). Struct (592 B): EN u32, CENTER_X u16,
CENTER_Y u16, RATE_X u16, RATE_Y u16, SHIFT u8, pad, R / G / B_GAIN_Table u16[32×3] (32 radial knots for each of the 3 CT rows),
pad[2]. No OpType (one set). bypass[35] or EN = 0 → row+0 = 0 in the 3 rows. EN = 1, every row r: +0 = 1 (0x1e6, 1319.00 b0); +4
CENTER_X (0x1e8, .01, 13 bit); +6 CENTER_Y (0x1e9, .02); +8 SHIFT (0x1ea, .03, 5 bit); +0xc RATE_X (0x1ec, .05, 11 bit); +0xe RATE_Y
(0x1ed, .06) — the struct order RATE_X, RATE_Y, SHIFT differs from the row order; then R / G / B table entries 32r…32r+31 → row r
+0x10 / +0x50 / +0x90 (0x1ee / 0x1ef / 0x1f0, 1319.10…, 10 bit). Calibration 0x2417 overwrites this struct (§4.5). Our bin: EN 0 →
unused.

#### 0x1029 LSC_CTRL (slot 36, no converter)
Read by `IQ_Patch_LSC` in every LSC run. Struct (148 B): EN u32, R / G / B_RATIO_BY_CCT u8[3] each, pad[3], POINTS_NUM u16, pad,
RATIO s32[16], BV s32[16]. EN = 1: w = `Interp_LUT_IQ` over the POINTS_NUM knots (x = BV[], y = RATIO[]) at the current scene
brightness (IqInput+8, the AE's BV); then every output gain of the 32 knots × 3 colours: g' = (((w·ratio_c[ct]) >> 7)·g + 64) >> 7,
ct = the LSC / ALSC CT index (ctx+0x53798) — shading strength by brightness and colour temperature, 128 = 1.0. Our bin: EN 0.

#### 0x1028 ALSC
Slot 37 → `IQ_API20_SET_ALSC` → IP 12 ALSC (3 CT rows × 0xb0e B, bank 131D + gain grid by menuload). Struct (8336 B): EN u32,
GridX u8, GridY u8, CCTThr u16[3], DeltaMode u8, pad[3], DeltaModeAdvEn u32, DeltaLutX u8[26], DeltaLutY u8[26], R / G /
B_GAIN_PAGELE u16[459×3] (a 27 × 17 grid per CT row), pad.
- bypass[37] or EN = 0 → row+0 = 0 in the 3 rows, return.
- **Preset grid spacing:** if DeltaModeAdvEn = 0 and (GridX, GridY) = (27, 17), the converter overwrites the stored struct's
  DeltaLutX[26] / DeltaLutY[16] with a preset chosen by DeltaMode (cell widths; the second half mirrors the first) [R emulation of
  every mode]:

  | DeltaMode | DeltaLutX (26) | DeltaLutY (16) |
  |---|---|---|
  | 0, ≥ 16 | 4 × 26 | 4 × 16 |
  | 1 | 2 2 4 4 4 4 4 4 4 4 4 4 8 8 4 4 4 4 4 4 4 4 4 4 2 2 | 4 × 16 |
  | 2 | 2 2 2 2 4 4 4 4 4 4 4 8 8 8 8 4 4 4 4 4 4 4 2 2 2 2 | 2 2 4 4 4 4 4 8 8 4 4 4 4 4 2 2 |
  | 3 | 2 2 2 2 2 2 4 4 4 4 8 8 8 8 8 8 4 4 4 4 2 2 2 2 2 2 | as 2 |
  | 4 | 2 2 2 2 2 2 2 2 4 8 8 8 8 8 8 8 8 4 2 2 2 2 2 2 2 2 | 2 2 2 2 4 4 8 8 8 8 4 4 2 2 2 2 |
  | 5 | 1 1 2 4 4 4 4 4 4 4 4 8 8 8 8 4 4 4 4 4 4 4 4 2 1 1 | as 4 |
  | 6 | 1 1 2 2 2 4 4 4 4 4 8 8 8 8 8 8 4 4 4 4 4 2 2 2 1 1 | 1 1 2 4 4 4 8 8 8 8 4 4 4 2 1 1 |
  | 7 | 1 1 2 2 2 2 2 4 4 8 8 8 8 8 8 8 8 4 4 2 2 2 2 2 1 1 | as 6 |
  | 8 | 1 1 1 1 2 2 4 4 4 8 8 8 8 8 8 8 8 4 4 4 2 2 1 1 1 1 | 1 1 2 2 2 8 8 8 8 8 8 2 2 2 1 1 |
  | 9 | 1 1 1 1 4 4 4 4 4 4 8 8 8 8 8 8 4 4 4 4 4 4 1 1 1 1 | as 8 |
  | 10 | 1 1 1 1 1 1 2 4 8 8 8 8 8 8 8 8 8 8 4 2 1 1 1 1 1 1 | 1 1 1 1 4 8 8 8 8 8 8 4 1 1 1 1 |
  | 11 | 1 1 1 1 1 1 2 4 4 4 8 8 16 16 8 8 4 4 4 2 1 1 1 1 1 1 | as 10 |
  | 12 | 1 1 1 1 1 1 1 1 4 8 8 8 16 16 8 8 8 4 1 1 1 1 1 1 1 1 | 1 1 1 1 4 4 4 16 16 4 4 4 1 1 1 1 |
  | 13 | 1 1 1 1 1 1 1 1 4 4 4 16 16 16 16 4 4 4 1 1 1 1 1 1 1 1 | as 12 |
  | 14 | 1 1 1 1 1 1 1 1 2 2 8 16 16 16 16 8 2 2 1 1 1 1 1 1 1 1 | 1 1 1 1 2 2 8 16 16 8 2 2 1 1 1 1 |
  | 15 | 1 1 1 1 1 1 1 1 1 2 8 16 16 16 16 8 2 1 1 1 1 1 1 1 1 1 | as 14 |

  DeltaMode picks how much denser the grid is at the image edges; with DeltaModeAdvEn = 1 (or another grid size) the user's
  DeltaLut arrays are used as given.
- Then per CT row r: +0 = 1 (0x1f3, 131D.40 b0); +0xa = GridX − 1 (0x1f8, .43 lo); +0xc = GridY − 1 (0x1f9, .43 hi); +0x14
  DeltaLutX[26] (0x1fd, .50…); +0x2e DeltaLutY[26] (0x1fe); +0x48 CCTThr[r] (0x1ff, no register: the CT axis of the 3 rows); gain
  grid: for each grid line y < GridY, GridX u16 of R / G / B table r (struct line stride 27) → row +0x4a / +0x3e0 / +0x776 +
  2·GridX·y (0x200 / 0x201 / 0x202, menuload), i.e. repacked to the actual grid width. ctx+0x55c14 / 0x55c15 = GridX / GridY.
- Calibration 0x2415 overwrites this struct, 0x2417 its CCTThr (§4.5). Our bin: EN 0 → unused (ALSC runs on the IQ file,
  whose ALSC rows are what the hardware gets).

#### 0x102a ALSC_CTRL (slot 38, no converter)
Read by `IQ_Patch_ALSC`. Same layout as LSC_CTRL. EN = 1: w_c = (Interp(BV)·ratio_c[ct]) >> 7 per colour; every grid point is
first normalised by its smallest channel (n_c = 1024·g_c / min(R, G, B)); then w < 28: g' = 1024 + w·(n_c − 1024)/28 (from flat to
colour-only correction), w ≥ 28: g' = n_c + (w − 28)·(g_c − n_c)/100 (towards the full table at w = 128) [R arithmetic; I
reading]. Our bin: EN 0.

#### 0x103c ALSC_ADJ (slot 52, no converter)
Read by `IQ_Patch_ALSC` and `hal_DoIQPerFrame`, which re-runs IP 12 every frame while EN = 1. Struct (152 B): EN u32, CPX_TH u16,
MAX_R, MIN_R, MAX_B, MIN_B u16, CHK_RANGE u8, UNIFORM_TH u8, DBG u8, pad[3], POINTS_NUM u16, pad, RATIO s32[16], BV s32[16]. EN = 1
runs an adaptive shading tuner: a state machine in ctx+0x55c18…+0x55c2d (R / B correction factors at +0x55c18 / +0x55c1a, 256 =
1.0) that on each frame calls `Check_Shading_Performance` on the AE block statistics (IqInput+0x54) with CPX_TH, CHK_RANGE
(clamped 8…16), UNIFORM_TH and the BV-interpolated RATIO, steps the R / B factors by ±1 towards uniformity, decays them (·9/10)
when the scene is unsuitable, and re-inits after repeated failures; DBG = 1 forces a measurement every frame. EN = 0 resets the
state byte ctx+0x55c2c. The per-state arithmetic, the use of MAX / MIN_R / B and the exact addressing of
`Check_Shading_Performance`: isp-4-iq-engine.md §9.3, §9.4. Our bin: EN 0.

### 3.11 0x1038 Temperature [R]
Slot 50 → `IQ_API20_SET_TEMP`: no table; consumed by the IP patches. Struct (264 B): ENABLE u32, TemperatureLut u8[16], OP_TYPE
u32, auto[16] {ObcOffset s16, DynamicDPRatio, CrosstalkRatio, NRDeSpikeRatio u8, NR3DRatio u8[2], NRLumaRatio u8, SharpnessRatio
u8[2], SaturationRatio u8, ColorToneRatio u8[3]} (14 B), manual (same), pad[2].
- Converter: only `pIQ_IQTrigger(ctx, 1, 50)` (force re-run of the ISO-list IPs); no bypass handling.
- Temperature axis: `IQ_Temp_Index_Check` (in each DoIq) takes the sensor temperature from IqInput+0x1c, stores it as the byte
  0x1039 TemperatureInfo returns (ctx+0x5351c), and, if Temperature is enabled, finds the first k with temperature ≤
  TemperatureLut[k] → index hi = k, lo = max(k−1, 0) (ctx+0x57c35 / 0x57c36); a change of more than 1 degree forces an IQ re-run.
  Above the last threshold the indices keep their previous values.
- `IQ_DOINTER_TEMP(ctx, entry[lo].X, entry[hi].X, manual.X, default)`: disabled → default; enabled + manual → manual.X; enabled +
  auto → linear interpolation between the two entries by temperature. Users, all ratios default 50 = neutral: ObcOffset →
  `IQ_Patch_OBC` / `_OBC_P1` (default 0, an offset added to the black level), DynamicDPRatio → `IQ_Patch_DPC`, CrosstalkRatio →
  `IQ_Patch_ANTICT`, NRDeSpikeRatio → `IQ_Patch_SpikeNR`, NR3DRatio[0], [1] → `IQ_Patch_NR3D`, NRLumaRatio → `IQ_Patch_BNR`,
  SharpnessRatio[0], [1] → `IQ_Patch_YEE`, SaturationRatio → `IQ_Patch_YCUVM10` (UV gain ·t/50, clip 127), ColorToneRatio[k] →
  `IQ_SET_CCM`, the saturation factor of channel k = 0, 1, 2. How each patch applies its ratio: isp-4-iq-engine.md §7.5, §9.
- **The temperature is a constant:** `CameraIspAlgo_DoIq`, the caller of `DoIqPerFrame` and the IQ init all write IqInput+0x1c =
  0x24 (and +0x20 = 0x24) before every run — 36 [R: `*(g_tIspMidHandler + 0xe1c) = 0x24` at three sites]. So on this build the
  temperature index never moves, 0x1039 TemperatureInfo always returns 36, and an enabled Temperature API acts as a fixed
  interpolation at 36 (or its manual set). Not confirmed on the running camera (a Get of 0x1039 is expected to return 36;
  CAMERA-CHECKS K19).
- Not in our bin → Temperature disabled → all defaults.

## 4. AE (0x14xx), AWB (0x18xx, 0x2C03–0x2C06), AF (0x1Cxx) and calibration (0x24xx) ids

### 4.1 Route and where they land [R]
<!-- ev:isp-3-iq-api.4.1 -->
`DrvAlgo_IF_ApiSet/Get` checks the size per id and calls the handler with the **algorithm object**: 0x14xx → AE handle (ch+0x28),
0x18xx, 0x240F and 0x2C03–0x2C06 → AWB handle (ch+0x120), the other 0x24xx → IQ handle; a null handle → 6. The AF cases use the 4th
argument as their object, which is always 0 (§1.2), so every 0x1Cxx id fails on the kernel path (§4.4).

The in-kernel AE / AWB / AF objects start with a method table that `AE_InitFuncPtr` (96 entries), `pAWB_InitFuncPtr` (41) and
`AF_InitFuncPtr` (35) fill at `AeInit` / `AwbInit` / `AfInit` (object, offset, method). The
handlers are thin: they unpack the API struct into method arguments and call `pAE_*` / `pAWB_*` / `pAF_*` through that table
. So every 3A id lands in the state of the in-kernel 3A object, and the only consumer is that
object's own frame loop (`DoAe` / `DoAwb`; `DoAf` has no caller) — the methods and the fields they touch: isp-5-3a.md §2.16, §3.11, §4.8. On this
camera nothing consumes them: the user-space 3A takes these ids (§1.4) and `nFlagCus3A` = 0x1A keeps `DoAe` / `DoAwb` from running
(isp-2-ispmid-mload-ispscl.md §1.1); the objects still exist (`AeInit` / `AwbInit` run at open), so a kernel Set is accepted and stored.
Sizes: §2.

### 4.2 AE ids [R]
| id | size | Set → methods (argument mapping) | Get → methods |
|---|---|---|---|
| 0x1401 HistWghtY | 520 | — | `GetHistWghtY`: `pAE_GetLumY`, `pAE_GetAvgY`, `pAE_GetHist` |
| 0x1402 AEInfo | 572 (XML 572) | — | `QueryExposureInfo`: `pAE_isStable`, `isReachBoundary`, `GetCurExpo`, `GetCurShortExpo`, `GetCurLV`, `GetCurBV` + the HistWghtY block |
| 0x1403 EVComp | 8 | `pAE_SetEVComp(h, d[0], d[1])` | `pAE_GetEVComp` |
| 0x1404 ExposureMode | 4 | `pAE_SetExpMode` | `pAE_GetExpMode` |
| 0x1405 ManualExposure | 16 | `pAE_SetManualExpo(h, d[0], d[1], d[2], d[3], d)` | `pAE_GetManualExpo` |
| 0x1406 AEState | 4 | d = 1 → `pAE_Pause`, else `pAE_Resume` | `pAE_GetSMState` |
| 0x1407 AETarget | 132 | `pAE_SetTarget` | `pAE_GetTarget` |
| 0x1408 AEConverge | 40 | `pAE_SetConvergeThd(h, d[0], d[1])`, `pAE_SetConvergeSpeed(h, &d[2], &d[6])` | `GetConvergeThd/Speed` |
| 0x1409 ExposureLimit | 32 | clamps first, **writing the clamped values back into the caller's struct**: min / max shutter (d[0], d[1]) to 1…`h+0xa94` (sensor max), apertures d[2], d[3] to 10…220, gains d[4…7] ≥ 1024; then `pAE_SetMinMaxShutterUS(h, d[0])`, `SetMinMaxAperture(h, d[2], d[3])`, `SetMinGains(h, d[5], d[4])`, `SetMaxGains(h, d[7], d[6])` | `GetMinMaxShutterUS`, `GetMinMaxAperture`, `GetMinGains`, `GetMaxGains` |
| 0x140a AEPlainLongTbl | 260 | `pAE_SetPlainExpTbl(h, {u16 count = d[0]; entries = d[1..64]})` | `pAE_GetPlainExpTbl` |
| 0x140b AEPlainShortTbl | 260 | `pAE_SetPlainShortExpTbl` (same packing) | `pAE_GetPlainShortExpTbl` |
| 0x140c WinWgtType | 4 | `pAE_SetAEWinWgtType` | `pAE_GetAEWinWgtType` |
| 0x140d AEWinWeight | 3076 | `pAE_SetAEWinWgtType(h, d[0])`, `pAE_SetAEWinWgtTbl(h, average, center, spot)` (3 × 32×32 u8) | `GetAEWinWgtType/Tbl` |
| 0x140e Flicker | 4 | d = 1 / 2 → `pAE_SetFlickerMode(h, 2)` then `pAE_SetFlicker(h, 1 for 1 = 60 Hz, 0 for 2 = 50 Hz)`; d = 3 → `SetFlicker(h, 0)` only; 0 or > 3 → `SetFlicker(h, 1)` only — the 50 / 60 Hz naming per the XML [I] | `pAE_GetFlickerMode`, `pAE_GetFlicker` |
| 0x140f ExposureStrategy | 296 | `pAE_SetAEStrategy` | `pAE_GetAEStrategy` |
| 0x1410 RGBIRAE | 8 | `pAE_SetRGBIRAE` | `pAE_GetRGBIRAE` |
| 0x1411 AEHdrRatio | 132 | `pAE_SetAeHdrRatio` | `pAE_GetAeHdrRatio` |
| 0x1412 ManualShortExposure | 16 | `pAE_SetManualExpoShort` | `pAE_GetManualExpoShort` |
| 0x1413 FlickerEx | 16 | `pAE_SetFlickerModeEX` | `pAE_GetFlickerModeEX` |
| 0x1414 AE_FlickerInfo | 12 | — | inline from the AE object: {u32 = byte h+0x36e, u32 = h+0x368, u16 = h+0x370} |
| 0x1415 AEStabilizer | 8 | `pAE_SetStabilizerParam` | `pAE_GetStabilizerParam` |
| 0x1416 ExposureStrategyEx | 660 | `pAE_SetAEStrategyEX` | `pAE_GetAEStrategyEX` |
| 0x1417 AE_StrategyExInfo | 14 (XML 16) | — | inline: 7 × u16 from h+0x20c0…+0x20cc |
| 0x1418 DayNightDetection | 12 | `pAE_SetDayNightDetectionParam` | `pAE_GetDayNightDetectionParam` |
| 0x1419 DayNightInfo | 12 | — | inline (`GetDayNightInfo`): word 0 = AE h+0x47c4; words 1–2 are not written |

Inside the AE object, 0x1409 stores (MIN_FN, MAX_FN) in the opposite order of every reader of those words, and 0x140a stores the
table's largest and smallest FN in h.0xa78 / 0xa7a and overwrites all user limits with the table extents (isp-5-3a.md §2.8).

### 4.3 AWB ids [R]
<!-- ev:isp-3-iq-api.4.3 -->
| id | size | Set → | Get → |
|---|---|---|---|
| 0x1801 AWBInfo | 24 | — | `AWB_QueryInfo` (calloc'd temp, AWB object queries) |
| 0x1802 WBAttr | 1464 | `AWB_SetAttr`: `pAWB_SetApiState(d[0])`, `SetApiMode(d[1])`, `SetAwbAdvMode(byte d+0x10)`, `SetMwbAttr({d[2], d[3]})`, `SetGainStrength(u16 d+0x18)`, `SetConvergeSpeed(d+0x11, d+0x12)`, `SetCtLimit(d+0x1c, d+0x1d)`, `SetAwbAlgMode(d[5])`, `SetConvergeThd(d+0x1e, d+0x22)`, then for each of the 18 LV steps `SetLvColorTempWeight(k, …)`, `SetPreferRRatio(k, …)`, `SetPreferBRatio(k, …)`, `SetWpWeight(d+0x45e)`, `SetWeightWin(d+0x474)` | `AWB_GetAttr`: the matching getters |
| 0x1803 WBAttrEx | 52 | `pAWB_SetUserExtraArea` | `pAWB_GetUserExtraArea` |
| 0x1804 WBMultiLSAttr | 44 | inline stores into the AWB object: h+0x1324 = (d[0] ≠ 0), h+0x1328 = clamp(byte d+4, 1, 10) (Sensitive: limits the peak ratio V1/V2), h+0x1329 = min(byte d+5, 100) (CaliStrength: scales the target ratios), two 9-entry u16 CCM tables (d+6, d+0x18) clamped to 0x1fff → h+0x132a / +0x133c, h+0x1354 = 1 (dirty) | `GetMultiLSAttr` |
| 0x1805 | — | no case (neither direction) | — |
| 0x1806 CTMWB | 4 | `pAWB_SetCTMwb` | `pAWB_GetCTMwb` |
| 0x1807 DayNightDetection | 12 | `pAWB_SetDayNightDetectionParam` | `pAWB_GetDayNightDetectionParam` |
| 0x1808 DayNightInfo | 12 | — | inline: word 1 = AWB h+0x13c0, word 2 = h+0x13c4; word 0 not written |
| 0x1809 AWBStabilizer | 12 | `pAWB_SetStabilizer` | `pAWB_GetStabilizer` |
| 0x2C03 AWBCTCali | 84 | `AWB_SetGoldenLightArea` (`pAWB_SetGoldenLightArea`) | `AWB_GetGoldenLightArea` |
| 0x2C04 | 23044 | — | `AWB_GetCTStats` (`pAWB_GetCTStatistics`) |
| 0x2C05 | 34560 | — | `AWB_GetHWStats`: memcpy 0x8700 B from the AWB statistics buffer (h+0xa8 → +0x3c) |
| 0x2C06 AWBCurCTCali | 80 | — | `AWB_GetCurLightArea` (`pAWB_GetCurLightArea`) |

The 0x2Cxx cases sit in `DrvAlgo_IF_ApiSet` / `ApiGet` behind the 0x2400–0x2DFF range of `MS_CAM_IspApiSet/Get` [R]; libispalgo implements the same four ids with the same sizes in user space and takes them on this
camera while its AWB runs (§1.4). Our bin's 0x2C03 record goes to `MI_ISP_AWB_SetCTCaliAttr` (§6.3). Handlers with no id:
`AWB_SetCTWeight` / `AWB_GetCTWeight` (`pAWB_Set/GetLvColorTempWeight`; 0x1805 has no case) and `AE_GetDayNightInfo` (the inline
code of 0x1419). The meaning of the AWB fields (user areas, MultiLS, day / night score, `AWB_QueryInfo` layout): isp-5-3a.md §3.6, §3.8, §3.9, §3.10.

### 4.4 AF ids [R]
Handled inline in the switch: size check, then if the AF object (4th argument) is non-null a tail call of its method with (h,
data); if it is null **the caller's buffer is zeroed** (Set and Get alike; for most ids by `memset`, for 0x1C01 / 05 / 06 / 07 /
0x1C2A / 2C / 2E / 2F by word / halfword stores) and 6 returned. Since `MS_CAM_IspApiSet/Get` always passes 0 there
(`DrvAlgo_IF_ApiSet(id, ch+0x28, ch+0x120, skipbit5 = 0, iq, size, data)`, the AF cases test r6 = argument 4), **every AF id that
reaches the kernel zeroes the caller's data and returns 6**. On this camera the AF ids are normally served in user space by
libispalgo (accepted when its AF handle exists) and reach the kernel only before the user AF runs (§1.4). The method table the
kernel code would use (offset in the AF object):

| id | size | Set → | Get → |
|---|---|---|---|
| 0x1C01 AFHWRoiMode | 8 | `pAF_SetHWRoiMode` (+0xc) | `pAF_GetHWRoiMode` (+0x10) |
| 0x1C02 AFHWWin | 128 | `SetHWWin` (+0x14) | `GetHWWin` (+0x18) |
| 0x1C03 AFHWFilterAttr | 84 | `SetHWFilterAttr` (+0x1c) | `GetHWFilterAttr` (+0x20) |
| 0x1C04 AFHWFilterSq | 52 | `SetHWFilterSq` (+0x24) | `GetHWFilterSq` (+0x28) |
| 0x1C05 AFHWYParam | 3 | `SetHWYParam` (+0x2c) | `GetHWYParam` (+0x30) |
| 0x1C06 AFHWSource | 4 | `SetHWSource` (+0x34) | `GetHWSource` (+0x38) |
| 0x1C07 AFHWPreFilter | 12 | `SetHWPreFilter` (+0x3c) | `GetHWPreFilter` (+0x40) |
| 0x1C08 AFHWYMap | 28 | `SetHWYMap` (+0x44) | `GetHWYMap` (+0x48) |
| 0x1C09 AFHWLdg | 40 | `SetHWLdg` (+0x4c) | `GetHWLdg` (+0x50) |
| 0x1C29 AFAttr | 16 | `SetAttr` (+0x54) | `GetAttr` (+0x58) |
| 0x1C2A AFMotor | 8 | `SetMotor` (+0x5c) | `GetMotor` (+0x60) |
| 0x1C2B AFAccWeight | 272 | `SetAccWeight` (+0x64) | `GetAccWeight` (+0x68) |
| 0x1C2C AFOneShot | 8 | `SetOneShot` (+0x6c) | `GetOneShot` (+0x70) |
| 0x1C2D AFContinous_SceneChange | 24 | `SetSceneChange` (+0x74) | `GetSceneChange` (+0x78) |
| 0x1C2E AFContinous_SearchStart | 8 | `SetSearchStart` (+0x7c) | `GetSearchStart` (+0x80) |
| 0x1C2F AFContinous_Search | 6 | `SetSearch` (+0x84) | `GetSearch` (+0x88) |
| 0x1C30 (AF info) | 8 | — | `pAF_QueryInfo` (+0) |
| 0x1C0A–0x1C28 | — | no case | no case |

The AF setters call `pAF_Trigger`; `pAF_ForceTrigger` has no API id. The kernel AF object (`AfInit` via `AF_InitFuncPtr`) exists
but is unreachable through the API, and its loop `DoAf` has no caller in mhal — the in-kernel AF is dead twice over. The hardware
AF windows and filters the user-space AF needs are set through the CUS3A ids 0x2E0E–0x2E11 and 0x2E24–0x2E28 (§5.3). The AF fields
each id maps to: isp-5-3a.md §4.8.

### 4.5 Calibration ids 0x24xx (Set only) [R]
| id | handler | needs | effect |
|---|---|---|---|
| 0x240F | `CALIv30_AWB_CaliApply` (AWB handle) | data[0] = version 3 or 4 (else "Wrong Awb unit calibratoin version", 0xD) | packs {d+0x20, d+0x24, byte d+0x1c, four words (v3: d+0x38…0x44; v4: d+0x30…0x3c)} → `pAWB_ApplyUnitCalibration` → copies the possibly adjusted values back into the caller's data; −1 → "Wrong Awb unit calibratoin data" |
| 0x2410 | `CALIv30_OB_CaliApply` → `pIQ_Cali_OBC_Apply` (IQ ctx method +0x53bc4) | version 3 | **overwrites the 0x1014 BlackLevel API struct**: Enable 1, OpType 0 (auto), auto[i] = {R, Gr, Gb, B} from d+0x34+8i (16 entries), manual = entry 0; runs the OBC converter at once and sets dirty[4] |
| 0x2414 | `CALIv30_SDC_CaliApply` → `pIQ_Cali_SDC_Apply` (+0x53bd0) | version 4, item type d+0x18 = 2 ("mixer mode"), table size d+0x1e ≤ 256 | writes **the IQ-file SDC table** directly (no SDC API exists): ctx+0xa1ac = 1 (enable), 256 words at ctx+0xa1b8 = z·2^26 + y·2^13 + x from the u16 triples at d+0x40+16·size+8k (defect-pixel coordinates [I]); `pIQ_IQTrigger(4, 22)` re-runs IP 22 SDC |
| 0x2415 | `CALIv30_ALSC_CaliApply` → `pIQ_Cali_ALSC_Apply` (+0x53bc8) | version byte; CCT count ≤ 3, GridX ≤ 27, GridY ≤ 17, table size > 0 | **overwrites the 0x1028 ALSC struct**: EN 1, DeltaModeAdvEn 1, DeltaMode 0, GridX / Y, DeltaLutX / Y from the data, the R / G / B grids and CCTThr; dirty[37] |
| 0x2416 | inline | — | prints "[DrvAlgo_IF_ApiSet] - [ERROR] FPN_CaliApply API not support." |
| 0x2417 | `CALIv30_LSC_CaliApply` → `pIQ_Cali_LSC_Apply` (+0x53bcc) | version 5; CCT count ≤ 3; table size 1…32 | **overwrites the 0x1021 LSC struct**: EN 1, CENTER_X / Y, SHIFT, rates and the R / G / B tables per CCT; also writes the ALSC CCTThr and sets dirty[35] **and** dirty[37] |
| 0x2411–0x2413, other | — | — | no case |

Calibration is applied by rewriting the corresponding IQ API structs (OB, LSC, ALSC) or the IQ-file SDC table, after which they
behave as if the app had Set them. The `pIQ_Cali_*` side: isp-4-iq-engine.md §8. The user-space calibration loader that sends these ids from a calibration file
(`MI_ISP_API_CmdLoadCaliData`, item → id) is mi-capture.md §5.8. None of these ids is in our bin; nothing in the firmware is known
to send them (prog_uvc imports no `MI_ISP_*Cali*` call [I: not re-checked]). Whether a static defect-pixel calibration was ever
applied on this camera (bank 1316.6a–6f and the SDC menuload table against the imx307 default list) is not known (CAMERA-CHECKS
K22).

## 5. CUS3A ids 0x2Exx and the control words 0x3000–0x3003

### 5.1 Payloads and kernel behaviour [R]
For every 0x2Exx id the channel must exist and its IQ handle be set; sizes are checked exactly (0xE), unknown ids → 0xF. Names:
SigmaStar `mi_isp_api_id.h` (ids 1–30) / handler names. The statistics payload formats (marked "stats") and the channel fields
named here: isp-2-ispmid-mload-ispscl.md §10.3, §2.2.

| id | name | Set | Get | struct (size) — kernel behaviour |
|---|---|---|---|---|
| 0x2E01 | AE_HW_AVG_STATS | | ✓ | stats |
| 0x2E02 / 0x2E03 | AE_HW_HISTO_0 / _1 | | ✓ | stats |
| 0x2E04 | AE_GET_INIT_STATUS | | ✓ | `CusAEInitParam_t` (108): Size 0x6c, sensor_id zero, shutter 300, shutter step / min / max (µs, from the sensor driver), sensor gain = max, gain min / max, ISP gain 0x400, ISP gain max 0x3FFF, FN×10 = 18, fps = 30, HDR-short limits, AvgBlkX/Y; returns 0 untouched if AE is off |
| 0x2E05 | AE_GET_CUR_STATUS | | ✓ | `CusAEInfo_t` (65, packed): hist / avgs pointers 0, AvgBlkX/Y, current shutter / sensor gain / ISP gain (+ HDR short), PreAvgY, HDRCtlMode, FN×10, current sensor FPS, PreWeightY |
| 0x2E06 | AE_SET_PARAM | ✓ | | `CusAEResult_t` (56). Taken only if AE is on and user AE is enabled; with Change ≠ 0: store (ispmid AE state; HdrRatio ignored; DebandFPS → target FPS), `CameraIsp_SensorShutterGainCompensation` (`DrvSensorTryShutter/TryGain`, the rounding error moved into the ISP gain, min 1×), then realtime: queue shutter and gain works, refresh the IQ ISO input, queue the digital-gain work, run sensor works due now, `CameraIsp_SensorAeNotify`; frame mode: `CameraIspAlgo_IQProcFrame`. Shutter unit is µs (passed to `DrvSensorSetUs`; the header comment says ns) |
| 0x2E07 | AWB_HW_AVG_STATS | | ✓ | stats |
| 0x2E08 | 3A_ENABLE | ✓ | | `Cus3AEnable_t` (3 bools): `CameraIsp_EnableCus3A` → per-channel `nFlagCus3A` bits 1 / 3 / 4 (refused per bit by the bypass mask) → channel +0x196c4, 3A counters reset. Handled before the channel check |
| 0x2E09 | AWB_GET_CUR_STATUS | | ✓ | `CusAWBInfo_t` (41, packed): AvgBlkX/Y, current R / G / B gains, HDR flag, BV, WeightY; pointers left as passed |
| 0x2E0A | AWB_SET_PARAM | ✓ | | `CusAWBResult_t` (24), `MS_CAM_ISP_SetAwbParam` → `CameraSetAwbParam`: taken if AWB on, user AWB on and Change ≠ 0: R → gain R, G → Gr and Gb, B → gain B, ColorTmp; consumed by the next `DoIq` / `DoIqPerFrame` (WB-gain IQ input and shadow), i.e. registers at the next frame's batch |
| 0x2E0B | AE_WINDOW_BLOCK_NUMBER | ✓ | ✓ (no-op) | u32 `MS_CUST_AE_WIN_BLOCK_NUM_TYPE_e` (0…6); only the 32 × 32 entry is accepted. Get checks the size and returns 0 without writing |
| 0x2E0C | AWB_SAMPLING | ✓ | ✓ | `CusAWBSample_t` (12): blocks X, Y, ratio |
| 0x2E0D | AF_STATS | | ✓ | stats |
| 0x2E0E | AF_WINDOW | ✓ | ✓ | `CusAFWin_t` (20): index + {x0, y0, x1, y1} 0…1023; Set clamps to 1023, needs x1 > x0, y1 > y0 (else 7) |
| 0x2E0F | AF_FILTER | ✓ | ✓ | size checked as 140 (= `CusAFFilter_Geo_t`) but only the first 84 bytes (= `CusAFFilter_t`, 42 × u16) are stored and used |
| 0x2E10 | AF_FILTER_SQUARE | ✓ | ✓ | `CusAFFilterSq_t` (52) |
| 0x2E11 | AF_MODE | ✓ | ✓ | `CusAFRoiMode_t` (8) |
| 0x2E12 | AE_HISTOGRAM_WINDOW | ✓ | ✓ | `CusAEHistWin_t` (10) |
| 0x2E13 | GET_IMAGE_RESOLUTION | | ✓ | `CusImageResolution_t` (16): by node — sensor / ISP input size or ISP output size + pixel depth |
| 0x2E14 | ENABLE_ISP_OUT_IMAGE | | ✓ | `CusISPOutImage_t` (20): image-store debug capture (`CameraISP_ImageStoreEnable`, node 0 / 2 → raw capture) — a Set-like action on the Get path |
| 0x2E15 | GET_ISP_OUT_IMAGE_COUNT | | ✓ | u32 counter of stored images |
| 0x2E16 | CAPTURE_HDR_RAW_IMAGE | ✓ | | `CusHdrRawImage_t` (24) |
| 0x2E17 | FRAME_META_INFO | | ✓ | `IspFrameMetaInfo_t` (24): input u64 PTS → shutter, sensor gain, colour temperature of that frame, from the PTS table and the metadata pool; shutter and gain are taken 3 frames back in a 6-slot ring (the exposure that lit that frame [I]), colour temperature is the current one |
| 0x2E18 | I2C_ACCESS | ✓ write | ✓ read | `CusSensorI2cParam_t` (24): `DrvSensorI2cWrite/Read` on the channel's sensor |
| 0x2E19 | AWB_HW_AVG_STATS_SHORT | | ✓ | stats |
| 0x2E1A | CAPTURE_RAW_IMAGE | | ✓ | 8 B {node, MIU address}: `CameraIsp_CaptureRawImage` — an action on the Get path |
| 0x2E1E | AE_HW_RGBIR_HISTO_STATS | | ✓ | stats |
| 0x2E20 | AF status | | ✓ | `CusAFInfo_t` (16): VCM min / max / current position (`DrvVcmGetMinMaxPos/GetCurPos`; fails without a VCM driver) |
| 0x2E21 | AF param | ✓ | | `CusAfResult_t` (12): user AF on and Change ≠ 0 → `DrvVcmSetPos(NextPos)` |
| 0x2E22 / 0x2E23 | AE / AWB window range | ✓ | ✓ (no-op) | `CusAEAWBCropSize_t` (8): crop x, y, w, h in 0…1023 of the input, applied to the AE / AWB window (restored if rejected) |
| 0x2E24 | AF Y param | ✓ | ✓ | `CusAFYParam_t` (3) |
| 0x2E25 | AF source | ✓ | ✓ | `CusAFSource_e` (4) |
| 0x2E26 | AF pre-filter | ✓ | ✓ | `CusAFPreFilter_t` (12) |
| 0x2E27 | AF Y map | ✓ | ✓ | `CusAFYMap_t` (28) |
| 0x2E28 | AF LDG | ✓ | ✓ | `CusAFLdg_t` (40) |

The 0x2E0B enum → block-size table contents are not known (only the 32 × 32 entry is accepted anyway). The sensor's shutter /
gain delays that set when the 0x2E06 works take effect are read from the sensor driver at open (`ae_shutter_delay` /
`ae_gain_delay`, 2 / 2 for the GC8613 driver: sensor-gc8613.md §4.1; the queueing: isp-2-ispmid-mload-ispscl.md §6.2).

### 5.2 Accepted ids (from the `MS_CAM_IspApiSet/Get` switches) [R]
- Set: 0x2E06, 0x2E08 (before the switch), 0x2E0A, 0x2E0B, 0x2E0C, 0x2E0E, 0x2E0F, 0x2E10, 0x2E11, 0x2E12, 0x2E16, 0x2E18,
  0x2E21, 0x2E22, 0x2E23, 0x2E24–0x2E28, 0x3000–0x3003. The other slots of 0x2E06…0x2E28 (0x2E07, 0x2E09, 0x2E0D, 0x2E13–0x2E15,
  0x2E17, 0x2E19–0x2E20) only unlock the mutex and return "not support".
- Get: 0x2E01–0x2E05, 0x2E07, 0x2E09, 0x2E0B (no-op), 0x2E0C–0x2E15, 0x2E17–0x2E1A, 0x2E1E, 0x2E20, 0x2E22 / 0x2E23 (no-op),
  0x2E24–0x2E28, 0x3000–0x3003. Refused: 0x2E06, 0x2E08, 0x2E0A, 0x2E16, 0x2E1B–0x2E1D, 0x2E1F, 0x2E21.
- 0x2E1B…0x2E1D and 0x2E1F lie inside the switch range but return "not support" (0xF) in both directions.

### 5.3 Window and filter setters: storage and application [R; I where marked]
Each setter first checks the global CUS3A-bypass mask (`g_tIspMidHandler`+0x33c: bit 3 AE / histogram, bit 4 AWB, bit 5 AF — a set
bit makes the call a silent no-op returning 0), stores the value in the channel's 3A configuration block (ch+0x1977c…, the block
`DrvIsp_3A_StatisticConfig` programs at open and on every frame-mode context switch), and **in realtime mode** immediately calls
the `DrvIsp_Set*` register writer, whose writes go through the current CMDQ interface into the batch the next frame start executes
at ISP VSYNC (isp-2-ispmid-mload-ispscl.md §7) [I: application at the next batch from the interface mode, not traced per call].
In frame mode only the stored copy changes and the next context switch applies it. `CameraIspAlgo_3A_ConfigBackup/Restore` keep
the block across a re-open.

| id | setter → writer | stored at (ch +) |
|---|---|---|
| 0x2E0B AE block number | `CameraSetAeWinBlock` → `DrvIsp_SetAeWin` | +0x197f0 (only the 32 × 32 enum accepted) |
| 0x2E0C AWB sampling | `CameraSetAwbWinBlock` → `DrvIsp_SetAwbWin` | +0x197f8 / +0x197fa block counts, +0x19804 ratio |
| 0x2E0E AF window | `CameraSetAFWindow` → `DrvIsp_SetAfWin` | +0x19808 (index), +0x1980c… window |
| 0x2E0F AF filter | `CameraSetAFFilterAttr` → `DrvIsp_SetAfFilterAttr` | +0x19914, 84 B (of the 140 checked) |
| 0x2E10 AF filter square | `CameraSetAFFilterSq` → `DrvIsp_SetAfFilterSq` | +0x19968 |
| 0x2E11 AF ROI mode | `CameraSetAF_ROI_Mode` → `DrvIsp_SetAfROIMode` | +0x19810 / +0x19812 |
| 0x2E12 histogram window | `CameraSetHistoWins` → `DrvIsp_SetHistoWins` | in the driver's window state |
| 0x2E22 AE range | `CameraSetAeWinBlockRange` → `DrvIsp_SetAeWin` | +0x1979c…+0x197ac crop, recomputed against the input size (+0x19784 / +0x19786) |
| 0x2E23 AWB range | `CameraSetAwbWinBlockRange` → `DrvIsp_SetAwbWin` | (the same scheme for AWB) |
| 0x2E24 AF Y param | `CameraSetAF_YParam` → `DrvIsp_SetAfYParam` | +0x1999c…+0x1999e |
| 0x2E25 AF source | `CameraSetAF_Source` → `DrvIsp_SetAfSource` | +0x1999f (= `cfg+0x223`, isp-2-ispmid-mload-ispscl.md §10.2) |
| 0x2E26 AF pre-filter | `CameraSetAFPreFilter` → `DrvIsp_SetAfPreFilter` | +0x199a0…+0x199a8 (+ +0x19800) |
| 0x2E27 AF Y map | `CameraSetAFYMap` → `DrvIsp_SetAfYMap` | +0x199ac |
| 0x2E28 AF LDG | `CameraSetAFLdg` → `DrvIsp_SetAfLdg` | +0x199c8 |
| 0x2E16 HDR raw capture | `MS_CAM_CaptureHdrRawImage` → `MS_CAM_CaptureRawImage` | — (action) |
| 0x2E21 AF result | `MS_CAM_ISP_SetAfParam` → `CameraSetAfParam` → `DrvVcmSetPos` | — (needs a VCM driver; none on this camera) |

The `DrvIsp_Set*` writers and the statistics windows they program (banks 131A / 131B / 131C): isp-2-ispmid-mload-ispscl.md §10.2.
On this camera libispalgo's `AfInit` sets the AF window and filters through 0x2E0E–0x2E11 and 0x2E24–0x2E28 (§1.4); whether anything
calls the AE / AWB / histogram window ids (0x2E0B, 0x2E0C, 0x2E12, 0x2E22, 0x2E23) is not traced, so those windows are probably the
open-time defaults [I]; a read of bank 131B on the running camera would show it (CAMERA-CHECKS K21).

### 5.4 0x3000–0x3003 [R]
| id | XML | Set | Get | consumer |
|---|---|---|---|---|
| 0x3000 | DebugLev AE (s32 0…65536) | ch+0x11c = d[0] | returns ch+0x11c | the word lies inside the AE output block `DoAe` receives (ch+0x88…) — the in-kernel AE's debug-print level; zeroed by `CameraIspAlgo_3A_IQ_Init` |
| 0x3001 | DebugLev AWB (0…7) | ch+0x178 = d[0] | returns ch+0x178 | inside the AWB block `DoAwb` receives (ch+0x124…) — the in-kernel AWB's debug level; zeroed at init |
| 0x3002 | — (payload 100 B, only word 0 read) | ch+0x196d8 = min(d[0], 100) | returns ch+0x196d8 | `CameraIspAlgo_UpdateIQ`: the in-kernel 3A runs on a 3-frame schedule (period ch+0x196cc = 3, AE phase mask +0x196d0 = 7, AWB mask +0x196d4 = 7 from `CameraIsp_3A_FrameCntInit`); once the frame count reaches ch+0x196d8 (default 3) the AWB mask drops to 4, i.e. AWB every 3rd frame. So 0x3002 = the number of start-up frames with AWB on every frame |
| 0x3003 | DebugLev AF (0…65536) | ch+0x18c = d[0] | returns ch+0x18c | no reader in mhal besides Set / Get (the in-kernel AF never runs) — unused |

The strings of this part of the switch name AE / AWB / AF "info" blocks (`AeInfo_t`…); the code stores and returns only the words
above (customer AE / AWB / AF control words). All four matter only for the in-kernel 3A, which does not run on this camera;
our bin's DebugLev records are not even loaded (§6.3).

## 6. The API bin (`/customer/iq_file_gc8613.bin`, loaded by `prog_uvc -i`)

### 6.1 Format [R]
A file of recorded Set calls: a header and records back to back. Our bin is 93 961 B.

```
header 32 B (8 × u32)
  +0x00 chip id          0x6d650323 = ID_CHIP_M6 (mi_isp_api_id.h); bytes 23 03 65 6d
  +0x04 version          major<<16 | minor  (ours 0x00010002 = 1.2)
  +0x08 data length      bytes after the header (ours 93 929 = file − 32)
  +0x0c checksum         MI_ISP_API_CRC32(data, len) = ~(0xffffffff − Σ bytes) = the plain byte sum of the data from
                         offset 0x20 (ours 0x45f426, verified)
  +0x10 key              0x4d2; the caller's key must be 0 or equal; every record's key must equal it
  +0x14 0
  +0x18 checksum enable  low byte (1 = verify)
  +0x1c 0
records, back to back, no alignment:
  u32 key (0x4d2) | u32 type (5) | u32 len | data[len]
  data = u16 api_id | u16 n | u32 off[n] | payload        pointer i = data + 4 + 4n + off[i]
```

A record with one pointer is therefore `key | 5 | len | u32 (1 << 16 | api_id) | u32 0 | payload (len − 8)`. Group ids and item
order are those of `isp_api.xml`. Records with several pointers: ManualExposure 0x1405 (n = 4, offsets 0 / 4 / 8 / 12), EVComp
0x1403 (2), APIVer 0x1001 (2) — the payload is still the plain struct; the `call_` wrapper reads it through pointer 0.

### 6.2 Loader [R]
<!-- ev:isp-3-iq-api.6.2 -->
`MI_ISP_ALGO_API_CmdLoadBinFile(dev, ch, buf, key)` in libmi_isp (the user-space entry `MI_ISP_API_CmdLoadBinFile`, which reads
the file: mi-capture.md §5.7); mhal carries a byte-identical kernel copy (`MI_ISP_ALGO_API_CmdLoadBinFile`, `MI_ISP_API_EXCUTE_CMD`,
`MI_ISP_API_CRC32`, `call_MI_ISP_*`) that nothing in mhal calls (§1.1).
- Version: major must equal `MI_ISP_IQ_GetVersionInfo().major` (else −1); a minor mismatch is only a warning ("minor version is not
  match", printed if the ISP log level allows). `MI_ISP_IQ_GetVersionInfo` is a Get of 0x1001, which ends in mhal's
  `pIQ_GetIQVersion`: {vendor = the word last set by 0x1001 Set, major = 1, minor = 3}, major / minor parsed from the constant string
  "0.1.3". Our bin is 1.2: the major matches, the minor differs → warning only.
- Key: the `key` argument must be 0 or equal to the header key, else −1. A record whose key differs from the header key is reported
  ("Failed to Compare magic key") and the loop **re-reads the same record forever** — the data pointer advances only after a
  matching key (a loader defect).
- `MI_ISP_API_EXCUTE_CMD(dev, ch, type, data)` builds the n pointers and looks up `(type, api_id)` in a 90-entry table (mhal `.data`
  at `VIF_PATTERN_GEN_DEINIT`+0x40, `{u32 type; u16 id; fn}`), calling every match: a
  `call_MI_ISP_<group>_Set<X>` copies the payload into a heap struct and calls the ordinary `MI_ISP_<group>_Set<X>`. All 90
  entries are type 5 and Set calls, so loading the bin is a fixed sequence of ordinary Set calls; nothing bin-specific reaches the
  kernel. The table's ids:
  - AE: 0x1403–0x140b, 0x140d, 0x140e, 0x140f, 0x1410, 0x1411, 0x1412, 0x1413, 0x1415, 0x1416 (18);
  - AWB: 0x1802, 0x1803, 0x1804, 0x1806, 0x1809, 0x2c03 (`call_MI_ISP_AWB_SetCTCaliAttr`) (6);
  - AF: 0x1c01–0x1c09, 0x1c29–0x1c2f (16);
  - IQ: 0x1001, 0x1004–0x101e, 0x1021, 0x1025–0x102b, 0x102d–0x1038, 0x103a, 0x103c (50) — among them 0x101a FPN, 0x1025 NR3D_P1,
    0x1026 WDRCurveAdv and 0x103a DayNightDetection, which the kernel does not accept, and 0x1027 R2Y, 0x1033 Dummy, 0x1034 Dummy_EX,
    0x1038 TEMP, which it does.
  Not in the table: 0x2803 and 0x3000–0x3003.

### 6.3 The camera's bin [R]
<!-- ev:isp-3-iq-api.6.3 -->
87 records, all type 5, key 0x4d2 (record list; every field with its value). In file order: 0x2803 BypassItem (at 0x20, 32 × u32, all 0); AE 0x1406, 0x1404, 0x1405,
0x1407, 0x1408, 0x1415, 0x1403, 0x140a, 0x1409, 0x140d, 0x140e, 0x1413, 0x140f, 0x1416, (IQ 0x1036), 0x1410, 0x3000, 0x1411, 0x140b,
0x1412; AWB 0x1802, 0x1803, 0x1804, 0x1806, 0x1809, 0x3001, 0x2c03; AF 0x1c29–0x1c2f, 0x3003, 0x1c01–0x1c09; IQ 0x1017, 0x102b,
0x1013, 0x100d, 0x101c, 0x1014, 0x1015, 0x100f, 0x1010, 0x102d, 0x1011, 0x102e, 0x1037, 0x100e (NR3D at 0x462d, len 2124, payload
starting ENABLE = 1, OP_TYPE = 1), 0x1032 (NR3D_EX at 0x4e85), 0x100c, 0x1018, 0x1008, 0x1006, 0x1007, 0x1005, 0x1004, 0x1012, 0x1031,
0x100a, 0x1009, 0x1016, 0x100b, 0x1035, 0x101e, 0x1030, 0x101d, 0x1019, 0x101b, 0x102f, 0x1028, 0x102a, 0x103c, 0x1021, 0x1029,
0x103a, 0x1001.
- Records that do nothing: **0x2803 BypassItem** (not in the loader table → skipped; its 128-byte payload is the XML's 32-word
  form, not the kernel's 8-byte {bypass, k}), **0x3000 / 0x3001 / 0x3003 DebugLev** (not in the table), **0x103A DayNightDetection**
  (in the table, but the kernel has no case → "Not support", 0xF).
- 0x2C03 AWBCTCali goes to `MI_ISP_AWB_SetCTCaliAttr` → libispalgo while the user AWB runs, else the kernel's
  `AWB_SetGoldenLightArea` (§4.3).
- The remaining 42 IQ records and the AE / AWB / AF records are ordinary Sets. Enables of the IQ records (so which of them change
  the picture against the imx307 IQ file; details per id in §3.5–§3.11) [R, the API bin read record by record]:
  - enabled: Gamma, Contrast, Brightness, Lightness (all manual), CCM (manual, ISO_ACT_EN 0), Saturation (manual), HSV, CrossTalk
    (manual), BlackLevel (manual, offsets 3840), NRDeSpike (manual), NRLuma (auto), NRLuma_Adv, NR3D (manual), DynamicDP, PFC,
    Sharpness, Sharpness_EX, WDR;
  - present with Enable 0: AdaptiveGamma, ColorToGray, Defog, COLORTRANS, AntiFalseColor, BlackLevel_P1, DeMosaic, DynamicDP_Cluster,
    RGBIR, PFC_EX, NRChroma, NRChroma_Adv, NRChroma_Pre, NR3D_EX, YUVGamma, WDRCurveFull, HDR, HDR_EX, LSC, LSC_CTRL, ALSC, ALSC_CTRL,
    ALSC_ADJ;
  - VersionInfo 0x1001 (no enable).
- The 3DNR *level* of the ISP channel (2 on this camera) is a channel attribute the app sets (`SetChnParam`, mi-capture.md §4.4), not part of this file.
- Whether the bin is applied as specified and nothing later (prog_uvc's menu, Dummy_EX) overwrote it — a read-back of banks
  1333 / 1334 / 1335 / 133A (YEE), 1328 (NR3D), 1316 (DPC), 1322 (WDR) and 1330 (GNR) after boot against the bin's manual sets — is
  not confirmed on the running camera (CAMERA-CHECKS K20).

