# isp-5-3a — the SigmaStar AE / AWB / AF library: in-kernel copy and libispalgo
<!-- ev:isp-5-3a -->

SigmaStar's 3A library — auto exposure (AE), auto white balance (AWB) and contrast auto focus (AF) — exists twice in this
firmware: as the 3A sub-part of `mhal.ko` (the **in-kernel copy**, all names kept) and as `libispalgo.so` (the **user copy**,
stripped, linked statically into prog_uvc and driven by libcus3a). Both are builds of the same code with the same object
sizes, method tables and API acceptance; on this camera only the user copy decides anything. This document specifies the
library: its two copies and how they differ, the entry points and the structs passed to them, the AE / AWB / AF objects field
by field, every per-frame computation, and what each AE / AWB / AF API id does to the objects.

Not here: the ISP API ids, their sizes, routing and the id → method mapping (`DrvAlgo_IF_ApiSet/ApiGet`) — isp-3-iq-api.md §4
(and the CUS3A ids 0x2Exx, §5); the IQ file format and its access functions — isp-4-iq-engine.md §13 (this document names the
items AE / AWB read); the MultiLS CCM chain from the AWB output into the IQ engine (`pAWB_MultiLS_CaliCcmCalc`,
`pAWB_Ccm_Inter`, ch+0x150 / ch+0x154) — isp-4-iq-engine.md §2.2; the AdaptiveGamma blend that uses the AE's Strategy-EX ratio
— isp-3-iq-api.md §3.5 "Gamma group"; libcus3a (the CUS3A framework, its built-in Sigma adaptor that fills the structs of
§2.2 / §3.1 / §4.3, `LoadIqFile`, id acceptance in user space), libsigma3a — isp-6-userspace.md §2, §3, §5; the statistics engines,
`/dev/isp_fe`, the ispmid channel record and `nCtrlFlag` — isp-2-ispmid-mload-ispscl.md (§2.2, §10, §11); the file inventory —
firmware-image.md §3.

Tags: **[R]** read from the binary, its disassembly or decompile, a data section, a file, or obtained by running the vendor's
own code in an emulator on the host; **[S]** stated by a source (vendor header, the camera's `isp_api.xml`, another
file quoted as such); **[I]** inference. A heading tagged [R] means every statement under it is [R] unless marked otherwise.

Binaries: `/config/modules/4.9.227/mhal.ko` (3A sub-part) and
`/config/lib/libispalgo.so` (`/config/lib` on the camera); prog_uvc carries a static copy of the
latter.

Addresses: kernel functions by their address in the rebased decompile (image at 0x40000000, e.g. `AE_InitParameters`
0x40057fa0); libispalgo by Ghidra address = file offset + 0x10000 where written "Ghidra", otherwise by file offset ("user
disassembly 0x…"). Object fields are written **h.0xNNN** = byte offset in the object the section is about: the 0x5E00-byte AE
object (§2), the 0x13FC-byte AWB object (§3), the 0x2CA0-byte AF object (§4). `M[0xNN]` = the method at byte 0xNN of the AE
method table.

## 1. The library and its two copies

### 1.1 The in-kernel copy (mhal.ko)
- [R] The 3A sub-part of mhal.ko's ISP block (the sub-parts of the block: isp-2-ispmid-mload-ispscl.md §1): 314 functions,
  62 KB. AE ≈130 functions (`pAE_*` API getters / setters, the APEX conversions `TV2USEC` / `GAIN2SVXBASE` / …, `pAE_ProcessAE`, flicker
  detection, de-banding, strategy / stabilizer / day-night, `AeInit` / `DoAe` / `AeRelease`); AF ≈80 (hardware filter set-up,
  motor, accumulators, search / wobbling state machine, `DoAf`); AWB ≈90 (light areas, gray-world / balance gains,
  multi-light-source CCM, calibration, `AwbInit` / `DoAwb`).
- [R] Callers (ispmid, isp-2-ispmid-mload-ispscl.md §4.1): `CameraIspAlgo_3A_IQ_Init` runs `AeInit` / `AwbInit` at camera open — also
  when the user 3A is active, so the handles (AE at ch+0x28, AWB at ch+0x120) exist and carry the 0x14xx / 0x18xx API state.
  `CameraIspAlgo_DoAe` calls `DoAe(…, out = ch+0x88)` + `CameraIsp_SensorShutterGainCompensation` only if the channel has AE
  enabled (`nCtrlFlag` ch+4 bit 4) **and** bit 1 of ch+0x196c4 is clear; `CameraIspAlgo_DoAwb` calls `DoAwb` (output ch+0x124)
  only if AWB is enabled (ch+4 bit 5) and bit 3 of ch+0x196c4 is clear. `DoAf` has no caller in mhal at all; the AF object is
  reached only through `AF_InitFuncPtr` from `AfInit`.
- [R] The switch: `CameraIsp_EnableCus3A(ch, {ae, awb, af})` sets per-channel bits 1 (AE), 3 (AWB), 4 (AF) of
  `g_tIspMidHandler[ch+0x5c]` (`nFlagCus3A`; 0x1A = all three) unless the global bypass mask `g_tIspMidHandler+0x33c`
  (`CameraIspSetCus3ABypass`) forbids it; `CameraIsp_EnableChannel3A` then copies them into ch+0x196c4 and restarts the 3A frame
  counters. Live log of every boot: `CUS3A_RegInterfaceEX Ch=0, Adap=0, Type=0/1/2` (libcus3a registers AE, AWB, AF),
  `[MI_ISP_CUS3A_Enable] AE = 1, AWB = 1, AF = 1` (prog_uvc), then from mhal `[0] AE=1, AWB=1, AF=1, nFlagCus3A = 0x1A`
  (`CUS3A ver : … mi_cus3a … build_time.20220721223311`). So on this camera **no in-kernel AE / AWB / AF decision runs**.
- [R] The in-kernel AF is dead twice over: `DoAf` has no caller, and every AF API id fails on the kernel path
  (isp-3-iq-api.md §4.4).

### 1.2 libispalgo.so — the user copy [R]
<!-- ev:isp-5-3a.1.2 -->
173 160 bytes, stripped, NEEDED libc only; imports `CamOsMemCalloc` / `MemRelease` / `Printf` / `MathDivU64` / `MathDivS64`
(libcam_os_wrapper), `log2`, `pow` (libm), `qsort`, `memcpy` / `memmove` / `memset`, `raise`, `sprintf` and the nine
`MI_ISP_CUS3A_SetAF*` calls (libmi_isp). Version string (`sstar_mi_so_version_string`, also returned by `IspAlgoVerInfo()`):
`Sigmastar Module mi_ispalgo version: project_commit.7251436 sdk_commit.80327e1 build_time.20220721223320` — the same commits as
libcus3a and the kernel's CUS3A banner. 361 dynamic symbols; Ghidra finds 421 functions once the method-table targets are
created (statics named from the emulated method tables).

| part | functions | kernel counterpart (sub-part of mhal.ko's ISP block) |
|---|---|---|
| AE / AWB / AF algorithms (`AeInit/DoAe/AeRelease`, `AwbInit/DoAwb/AwbRelease`, `AfInit/DoAf/AfRelease`, `pAE_*`, `AE_*`, `pAWB_*`, `Awb*`, `pAF_*`, `AF_*`, `hal_AF_InitParameters_*`) | 307 named + statics | 3A (314) — every kernel function has a copy except the 7 APEX helpers of §1.4 |
| `DrvAlgo_IF_ApiSet/ApiGet` + 67 `DrvAlgo_IF_{AE,AWB,CALIv30}_*` | 69 | IQ API — AE / AWB / AF / calibration part only (the `DrvAlgo_IF_IQ_*` handlers, the `MI_ISP_*` copies and the API-bin loader are **not** in libispalgo) |
| IQ-file access: `cfg_InitBinInfo`, `cfg_getcfgbyenum(_ptr)`, `cfg_getcfg_info_byenum`, `cfg_setcfgbyenum`, `cfg_ReleaseDBGInfo`, `Cfg_InitDbgBuf(Info)`, `Cfg_GetDbgBufSize`, `IQ_PARAMETER_TYPE`; interpolation `Interp*`, `IQ_InterpXBase`, `IQ_para_inter`; shading checks `Check_Shading_*`, `Get_Color_Ratio`, `Get_Uniform_Rows_average`, `Calc_STDEV_Power`, 2-D array helpers, two's-complement helpers | 33 | IQ engine (the generic parts only; no `IQ_Patch_*`, `IspRegSet*`, `hal_IQ_*`) |
| `IspLoadIqCfg(&cfg, buf, size)` / `IspFreeIqCfg` / `IspAlgoVerInfo` | 3 | none by these names; the kernel loads the IQ file through `CameraIspAlgo_LoadIQFile` → `IspAlgoReadIqBinH` (isp-4-iq-engine.md §13). `IspLoadIqCfg`: `Cfg_InitDbgBuf`, check `size ≥ Cfg_GetDbgBufSize`, `cfg_InitBinInfo(*cfg, buf)`; returns −1 unless the parser consumed exactly the header's length |
| libgcc division helpers, CRT | 11 unnamed | — |

### 1.3 Same code, same object layouts [R + I]
<!-- ev:isp-5-3a.1.3 -->
Both copies were decompiled and compared per function by (a) the set of direct callees and (b) the set of literal constants
(the same for the IQ-API / cfg part). For the 3A part: 137
functions identical in both sets; 59 differ only because the user build inlines the APEX helpers (`log2` / `pow` called
directly instead of `USEC2TVXBASE`, `GAIN2SVXBASE`, `ISO2SVXBASE`, `LumxBASE2Log`, `TV2USEC`, `SV2GAIN`, `AVXBASE2FNX10` — the 7
kernel functions without a user copy) or uses different division helpers; 78 differ only in constants that are
loop-unrolling / vectorisation artefacts (offset runs such as `0x9ec,0x9ee,…` for one loop; `>= 1024` written as `> 0x3ff`) [I
after reading a sample]; 30 differ in calls (§1.4). The objects have the **same sizes** — AE 0x5E00, AWB 0x13FC, AF 0x2CA0 bytes
(`CamOsMemCalloc` in `AeInit` / `AwbInit` / `AfInit`) — the method tables have the same slot count and order (96 / 41 / 35;
every exported target sits in the same slot as its kernel namesake), and the API switch accepts the same ids with the same sizes
(plus the user-only ids of §1.4). So the kernel decompile, which keeps all names, is the readable reference for the user build;
§2–§4 hold for both copies except where a difference is named. Every per-frame step of the AE is a method called through the
object's own table, so a customer could override steps — nothing in SigmaStar's code does.

### 1.4 Differences between the copies [R]
<!-- ev:isp-5-3a.1.4 -->
| function | kernel | libispalgo |
|---|---|---|
| `pAF_SetHWRoiMode/Win/FilterAttr/FilterSq/YParam/Source/PreFilter/YMap/Ldg` | store the struct in the AF object and return −1 — **no hardware effect** | store, then `MI_ISP_CUS3A_SetAF*` (0x2E11, 0x2E0E × 16, 0x2E0F, 0x2E10, 0x2E24–0x2E28) on dev / ch kept at AF h.0x29ac / h.0x29b0 — the AF statistics engine is really programmed |
| `hal_AF_InitParameters_HWRoiMode/Win/FilterAttr/FilterSq/YParam/Source/PreFilter/YMap/Ldg` (AF hardware defaults) | all **zero** | real defaults from.rodata (dumped by emulation): ROI mode {0 normal, 1}; 16 windows = a 4 × 4 grid over 0…1023 ({0,0,255,255}, {256,0,511,255}, …); 42-coefficient IIR filter set; FilterSq table; Y param {76, 75, 29}; source 2; pre-filter off; Y-map and LDG tables — pushed to the kernel's AF engine by `AF_InitParameters` through the CUS3A setters |
| `DrvAlgo_IF_ApiSet/ApiGet` | IQ, AE, AWB, AF, CALI ids (isp-3-iq-api.md §2) | AE / AWB / AF + 0x240F, plus 0x2C03 Set / Get and 0x2C04–0x2C06 Get; no IQ ids |
| APEX conversions (`USEC2TVXBASE`, `GAIN2SVXBASE`, `TV2USEC`, `SV2GAIN`, …), `pAE_GetBV`, `pAE_CheckLinearity`, `pAE_GetLV` | integer log2 table (2561 entries) | inlined `log2()` / `pow()` in double — results may differ by table rounding |
| `pAE_CalLumY` | own sort | `qsort` of the block luminances |
| `pAE_isStable` | calls `pAE_isReachBoundary` | inlined |
| `pAWB_CalcGain`, `pAWB_MixDaylightGain`, `pAWB_SetUserExtraArea` | helpers inlined | call `pAWB_GetDaylightGain/GetPrevGain`, `pAWB_GetBlendValue` (result discarded, §3.7), `pAWB_RBgainToXY` |
| `DoAwb` | reads the debug level with `pAWB_GetDebugLevel` | `pAWB_SetDebugParam` |
| `DoAf`, `AF_FSM_SearchStart`, `AF_CalcAccRatio`, `AF_UpdateUpDownStatus`, `AF_Acc_UpdateLocalOneValue` | inlined helpers | call `AF_UpdateInfo`, `AF_Acc_GetAccPtr`, `AF_Motor_ResetEverPos` |
| logging | `CamOsPrintf` in `pAE_SetConvergeSpeed`, `pAE_GetSceneTarget`, `pAE_CalPvExpoShort`, `AE_FlickerFrequencyDetect_RunTimeMode`, `pAF_SetMotor`, `pAF_SetSearchStart` | `CamOsPrintf` in `pAE_CalApexTV`, `pAE_SetPlainExpTbl/ShortExpTbl`, `AF_Acc_UpdateAllValue` |
| HDR short exposure | `pAE_CalPvExpoShort` inline | calls `AE_CalApexSVforHDRShort`, `AE_CalApexTVforHDRShort`, `AE_ConvertNextExpoToExpoVal` |
| `Interp_LUT_IQ(_P)` | `Interp` | `Interp_IQ` |

User-only ids (libispalgo, AWB handle): 0x2C03 Set / Get `AWB_Set/GetGoldenLightArea` (84 B), 0x2C04 Get `AWB_GetCTStats`
(23 044 B), 0x2C05 Get `AWB_GetHWStats` (34 560 B = 128 × 90 × 3, raw hardware statistics), 0x2C06 Get `AWB_GetCurLightArea`
(80 B); the kernel has the same four cases behind its 0x2400–0x2DFF range (isp-3-iq-api.md §4.3). Their field layouts are not
read. Nothing in the list changes what AE / AWB compute [I]; the AF difference decides whether AF ids and AF initialisation
reach the AF hardware.

### 1.5 Which copy runs on this camera
<!-- ev:isp-5-3a.1.5 -->
- [R] prog_uvc's `MI_ISP_CreateChannel` → libmi_isp `MI_ISP_EnableUserspace3A` → libcus3a `CUS3A_Init` +
  `CUS3A_EnableUserspaceAE/AWB/AF` registers libcus3a's built-in "Sigma" adaptor for AE, AWB **and AF** and tells the kernel
  {1, 1, 1}; a SCHED_RR thread `3A_Proc_0` waits on `/dev/isp_fe` and per frame runs AE (reads 0x2E01 / 02 / 03 / 1E / 05,
  writes 0x2E06), AWB (reads 0x2E07 / 09, writes 0x2E0A — the algorithm itself computes only on every third call of
  `SigmaAwbRun`; the other two frames send the previous gains with Change = 0, which the kernel ignores) and AF (isp_fe modes
  3 / 2, writes 0x2E21) (isp-6-userspace.md §1.1, §2.4–§2.7). The algorithms are libispalgo's `DoAe` / `DoAwb` / `DoAf`,
  initialised from `/config/iqfile/iqfile0.bin`; `iqfile0…3.bin` in `/config/iqfile` are symlinks to `imx307_iqfile.bin`, so AE
  and AWB start from the **IMX307** tuning (the same file as the kernel's built-in `iq_bin`, isp-4-iq-engine.md §13.5) before the
  API bin overrides them; the AF runs on its built-in defaults (§4).
- [R names; I conclusion] prog_uvc's function names (names recovered by library
  matching) contain 116 of the 314 in-kernel 3A names (`pAE_*`, `AE_*`, `pAWB_*`, `AF_*`) and 251 of the 503 IQ-API names
  (`DrvAlgo_IF_ApiSet/ApiGet`, `DrvAlgo_IF_AE_*`, `DrvAlgo_IF_AWB_GetHWStats`, `MI_ISP_*`), and none of the hal / driver,
  statistics, ispmid or IQ-engine names: libispalgo is a user-space build of the SigmaStar AE / AWB / AF library plus its API
  dispatcher, while the IQ engine and all hardware access exist only in mhal.
- [R] AE / AWB / AF API ids from the application are served by libispalgo while the user algorithm runs and by the kernel copy
  before that: libcus3a's `Cus3A_Proc{Ae,Awb,Af}Ctrl` return −1 (→ 0xA0078008, the id goes to the kernel) only while the algorithm
  is not running; once it runs every id of its range returns 0 to the application and libispalgo's own status (size mismatch,
  unsupported id) is discarded. 0x2400–0x27FF ids other than 0x240F, and 0x2C00–0x2DFF ids other than 0x2C03–0x2C06, always go to
  the kernel (isp-6-userspace.md §3).
- Division of work per frame [R call graph; I where marked]:

| step | where | functions |
|---|---|---|
| statistics engines configured, statistics DMA buffer per frame | kernel | `CameraIspAlgo_3A_IQ_Config` / `3A_StatisticInit`, `DrvIsp_3A_StatisticConfig`, `DrvIsp_HWBufferAssign`, `DrvIsp_CopyStatisBufInfo` at frame done (isp-2-ispmid-mload-ispscl.md §10) |
| "new frame" wake-up | kernel → user | `IspmidFrameEvent` → `/dev/isp_fe` poll; prog_uvc's `Cus3A_ProcRoutine` thread: `write(fd, CUS3A_CMD 20 B)`, `poll(1000 ms)`, `read(20 B)` (isp-2-ispmid-mload-ispscl.md §11) |
| read statistics | user ← kernel | 0x2E01 AE averages (46 088 B), 0x2E02 / 0x2E03 histograms (256 B), 0x2E07 / 0x2E19 AWB averages (34 568 B), 0x2E0D AF (7 168 B), 0x2E1E RGB-IR histogram; `isp_fe` read modes 2 / 3 also return AF statistics / status directly |
| AE / AWB / AF algorithm | **user** (libcus3a + libispalgo in prog_uvc) | §2–§4 |
| apply AE result | user → kernel | 0x2E06 `CameraSetAeParam`: shutter / sensor gain / ISP gain / ISO into the channel (ch+0x90…+0xb8), `CameraIsp_SensorShutterGainCompensation` (`DrvSensorTryShutter/TryGain`), the sensor work queued for the frame it must land on (`IspMidWqSetSensorShutter/Gain` → `DrvSensorSetUs/SetGain`), digital-gain work (`DrvIsp_ApplyDGain`), `CameraIspAlgo_RefreshIqGainData` (new ISO for the IQ engine), `CameraIsp_SensorAeNotify` |
| apply AWB result | user → kernel | 0x2E0A `CameraSetAwbParam`: four u16 channel gains into ch+0x146…0x14e + update flag; written to the WB-gain registers with the IQ shadow (`ISP_IQ_WBG_Cfg`) [I: isp-2-ispmid-mload-ispscl.md §1.1] |
| IQ (ISO / CT interpolation, all IP registers) | **kernel**, every frame | isp-2-ispmid-mload-ispscl.md §6, isp-4-iq-engine.md §3 |

The 0x2Exx payloads: isp-3-iq-api.md §5. The AF part of this chain on the camera: §4.9.

## 2. AE — `AeInit` / `DoAe` / `AeRelease`, `pAE_*`, `AE_*`
<!-- ev:isp-5-3a.2 -->

Written from the kernel decompile (names kept), checked against the user copy where the kernel decompile loses a return value or
a switch (the user decompile or disassembly is named where it was used). Object: 0x5E00 bytes; method table h.0x00…0x17c, 96
entries, kernel and user order identical.

### 2.1 Units [R]
All exposure arithmetic is in **APEX × 16384** (`XBASE` = 2^14): TV = log2(1 s / shutter), SV = log2(ISO / 3.125) with
ISO = gain·100/1024 (gain 1024 = 1× = ISO 100), AV = 2·log2(F-number), BV = AV + TV − SV at the reference brightness, LV = BV + 5.
An **APEX record** is {TV, SSV (sensor-gain SV), DSV (digital-gain SV), AV, BV}, 5 × s32, with BV = AV + TV − SSV − DSV.
Conversions (kernel: a 2561-entry table T[i] = round(log2(i)·16384) plus range splitting; user build: `log2()` / `pow()` in
double — so the two copies can differ by table rounding, ≤ 1 LSB of 1/16384 EV):

| function | formula |
|---|---|
| `USEC2TVXBASE(us)` | (6·log2 10 − log2 us)·16384 (0x4FB9F − T[us] for us < 1000; dividing 1e6 / 1e7 / 1e8 by us for larger values) |
| `TV2USEC(tv)` | 1e6 / 2^(tv/16384) |
| `GAIN2SVXBASE(g)` | (log2(g·100/1024) − log2 3.125)·16384 (`T[iso] − 0x6935`) |
| `ISO2SVXBASE(iso)` | (log2 iso − log2 3.125)·16384 |
| `SV2GAIN(sv)` | 1024 · 2^(sv/16384) · 3.125 / 100 |
| `AVXBASE2FNX10(av)` | 10 · 2^(av/32768) |
| `LumxBASE2Log(v)` | inverse table lookup: i with T[i] ≈ v, i.e. 2^(v/16384), 0…1000, nearest |
| `AE_Plain2Apex({FNx10, us, ISO, LVx10} → {TV, SV, 0, AV, BV})` | TV, SV as above, AV = 2·(T[FNx10] − log2 10·16384), BV = LVx10·16384/10 − 5·16384 |
| `AE_Apex2Plain` | the inverse: FNx10, us, gain, LVx10 = (BV·10 + 5·16384·10) / 16384 |

`pAE_GetLV` = LV×10 from `pAE_GetBV`, clamped to 0…200. `Interp(x0, x1, y0, y1, x)` = linear interpolation with x clamped to
[x0, x1]. `Interp_LUT(lut, x)` with **LUT** = `{u16 num; u16 pad; s32 Y[16]; s32 X[16]}` (0x84 bytes, the layout of every
"…_NUM / _Y / _X" triple of the IQ file and of the API structs) = piecewise-linear Y(X) over the first `num` points, X ascending
or descending, Y[0] / Y[num−1] outside the range. Luminances are Y×10 on the 0…255 scale (0…2550; 470 = 47.0).

### 2.2 Entry points and their structs [R]
```c
void *AeInit(void *cfg, const AeInitParam_t *p, const AeInput_t *in /* AvgBlkX/Y, debug */, AeOutput_t *out);
int   DoAe(void *h, const AeInput_t *in, AeOutput_t *out);   /* 0, or −1 = "unreliable statistics" (§2.5 step 6) */
int   AeRelease(void *h);
```
**`AeInitParam_t` (0x5c)**: +0x00 char sensor_id[32] (only byte 0 is ever filled), +0x20 u16 FNx10, +0x22 u16 shutter_min µs,
+0x24 u16 sensor_gain_min, +0x26 u16 HDR-short shutter_min, +0x28 u16 HDR-short gain_min, +0x2c Size = 0x5c, +0x30 fps,
+0x34 shutter, +0x38 shutter_step, +0x3c HDR-short shutter_step, +0x40 sensor_gain, +0x44 isp_gain, +0x48 isp_gain_max,
+0x4c shutter_max, +0x50 sensor_gain_max, +0x54 HDR-short shutter_max, +0x58 HDR-short gain_max. `AeInit` stores the min / max /
step values into the limit block h.0xa90…0xab0 / h.0xb00…0xb10; `shutter`, `sensor_gain`, `isp_gain`, `fps` are not read.

**`AeInput_t` (0x58)**: +0x00 Size, +0x04 hist1 (u16 × 128, the 128-bin luma histogram), +0x08 hist2 (unused by the AE), +0x0c
RGB-IR histogram (u16 × 256), +0x10 AvgBlkX, +0x14 AvgBlkY, +0x18 avgs (AvgBlkX·AvgBlkY × {u8 R, G, B, Y}), +0x1c short-frame
avgs (HDR; NULL when linear — the HDR window set is processed only if non-NULL), +0x20 u16 FNx10, +0x24 shutter µs, +0x28 sensor
gain, +0x2c ISP gain, +0x30 / +0x34 / +0x38 the same three for the HDR short frame, +0x3c previous CurY×10, +0x40 PreAvgY×10, +0x4c
u8 HDR mode (0 linear, 1 / 2 HDR; 2 = shared sensor gain, §2.13), +0x50 → `AEDEBUG_t {s32 enable, s32 debug_level}`, +0x54 fps
(values < 3000 are taken as fps and multiplied by 1000 in `DoAe` step 2; the run-time flicker detector reads it raw, §2.12).

**`AeOutput_t` (0x90)**: +0x00 Size, +0x04 Change, +0x08 shutter µs, +0x0c sensor gain, +0x10 ISP gain, +0x14 / +0x18 / +0x1c HDR
short shutter / sensor gain / ISP gain, +0x20 BV×16384, +0x24 CurY×10 (weighted, long set, h.0xba4), +0x28 AvgY×10 (h.0xbbc;
libcus3a reports it as AvgY), +0x2c weighted Y×10 of the HDR-short set (h.0xbb0; libcus3a reports it as WeightY — **0 in linear
mode**, the short set is never measured), +0x30 AvgY×10 of the short set (h.0xbc0), +0x34 smoothed target×10 (h.0x224), +0x38
IsStable (h.0xb90), +0x3c IsReachBoundary (h.0xb92), +0x40 u16 h.0xbc8 (point count of the HDR-ratio LUT; the M5 header calls
this slot HistMaxY), +0x44…+0x83 16 × u32 from h.0xc0c (`AeHdr_Num_BV`, 16 BV-bucket words for the IQ engine), +0x84 DebandFPS
×1000 (h.0x375c), +0x88 u16 FNx10, +0x8a u8 histogram max bin (h.0xc4c), +0x8c u16 = h.0x20c0 + 1 = **GMBlendRatio + 1** of
Strategy-EX (written by `DoAe`, user disassembly 0x12d1c–0x12d24; §2.11).

### 2.3 `AeInit` and the metering grids [R]
`AeInit(cfg, p, in, out)`: calloc 0x5E00; keep cfg / p / in / out at h.0x5df0 / 0x5df4 / 0x5df8 / 0x5dfc; `AE_InitFuncPtr`
(the method table); `AE_InitParameters` (built-in defaults §2.17, then `AE_GetParamFromCfg` = the 84 IQ-file items of §2.4, then
copies of the API-visible blocks into the pending-API shadow h.0x47c8…0x5de8, §2.16); limits from `p`; window grids: for set 0
(long) and set 1 (HDR short) allocate 4 arrays of nx·ny u32 (count, sum, avg, sorted avg) and `pAE_SetNthWin(h, 0, 1024, 1024, 0,
0, {nx, ny})` (whole frame) / `(h, 1, 409, 409, 0, 0, {nx', ny'})` (centre 40 %), nx·ny from `AE_WIN_X_NUM` / `AE_WIN_Y_NUM`
(default 16 × 16, clamp 1…32); first exposure **ISP gain = 1024, sensor gain = max(1024, gain_min) ≤ gain_max, shutter =
max(300 µs, shutter_min) ≤ shutter_max** (HDR short: 1024, max(1024, min) ≤ max, max(30 µs, min) ≤ max); output Change = 1 with
these values, BV = h.0xb9c, CurY / AvgY, target, stable flag, histogram info.
**`AeRelease`**: frees the 8 window arrays and the object.
**`pAE_SetNthWin(h, set, w, hgt, xoff, yoff, {nx, ny})`** (method 0x94): ROI = w/1024 × hgt/1024 of the AvgBlkX × AvgBlkY block
grid (w, hgt clamped 4…1024; at least nx × ny blocks), centred, shifted by xoff blocks and yoff·BlkY/90; stored as bytes {nx, ny,
x0, y0, x1, y1} at h.0x27f0 + 0x60·set, then the cell boundaries x[0…nx] at +6 and y[0…ny] at +0x27 (ROI divided evenly).
`DoAe` rebuilds both grids whenever the statistics are not 32 × 32 blocks (long 1024 × 1024; short 409 × 409 if the HDR flag byte
of the h.0x378c area is 0, else 1024 × 1024).

### 2.4 Configuration consumed from the IQ file (`AE_GetParamFromCfg`) [R]
<!-- ev:isp-5-3a.2.4 -->
84 items, each `cfg_getcfgbyenum(cfg, enum, h + off)` (names from; full list). Grouped:

| items (enum) | object | meaning |
|---|---|---|
| AE_HIST_TYPE 0x06; AE_CALI_AVOFFSET 0x07, BVOFFSET 0x08, ISO100GAIN 0x09, MINISO 0x0a | h.0x3744, 0x3784, 0x3780, 0x3778, 0x377c | calibration of the APEX scale (built-in defaults 0 / 0 / 13102 (0x332E) / 1782 (0x6F6) / 57 (0x39)) |
| AE_DEFAULT_TARGET 0x15 | h.0x220 | target Y×10 (default 470 = 47.0 on 0…255) |
| AE_TARGET_LUT (num 0x16, Y 0x17, X 0x18) | h.0x19c LUT | "Target Offset" vs BV — the API 0x1407 AETarget block |
| AE_INIT_SHUTTER 0x19, AE_INIT_GAIN 0x1a, AE_INIT_FNUM 0x1b | h.0x1858, 0x185c, 0x1860 | defaults 25 000 µs, 1024, 16 (used only by manual / initial paths) |
| AE_MIN_FPS 0x1c, AE_SHUTTER_COMP_GAIN 0x1e | h.0x3758, 0x3754 | de-banding / FPS floor (default 30 000 = 30 fps ×1000); shutter compensation into SG |
| AE_METERING_TYPE 0x21 | h.0x27ec | which of the three 32×32 weight tables (average / centre / spot) |
| AE_FLICKER_MODE 0x22, AE_FLICKER 0x23 | h.0x354 (then forced to 1), h.0x350 | flicker handling (§2.12) |
| AE_EVCOMP_GRAD 0x24, AE_EVCOMP 0x25 | h.0x2b8, h.0x2bc | EV compensation (= API 0x1403) |
| AE_CONV_RATIO (0x26…0x28), AE_CONV_INTO/OUT_STABLE_THD 0x29 / 0x2a | h.0x2cc LUT, h.0x2c4, h.0x2c6 | convergence speed vs error, stable thresholds (= API 0x1408) |
| AE_WIN_X_NUM 0x2b, AE_WIN_Y_NUM 0x2c | h.0x27f0, 0x27f1 | metering grid |
| AE_EXPO_TBL_NUM / TBL 0x2d / 0x2e, AE_HDR_SHORT_EXPO_TBL 0x2f / 0x30 | via `pAE_SetPlainExpTbl` / `ShortExpTbl` | the exposure program (§2.8) |
| AE_WEIGHTING_TBL 0x31 | h.0xc4e (3 × 1024 B) | average / centre / spot weight tables |
| AE_DR_BT/DT_* 0x34…0x3b | h.0x20dc…0x21f4 | dynamic-range estimation (§2.6) |
| AE_STRATEGY 0x43 … LOWER_X 0x4a | h.0x1908 block (= API 0x140f layout) | strategy mode, weight, upper / lower target limits vs BV |
| AE_ENABLE_STRETCH 0x4b | h.0x2208 | enables the tone-balance target in Auto strategy |
| AE_STGY_BT_* 0x4c…0x51, AE_STGY_DT_* 0x71…0x76 | h.0x1a3c…, h.0x1b6c… | bright-tone / dark-tone strategy LUTs |
| AE_MAIN_BT_* 0x55…0x61, AE_MAIN_MT_* 0x67…0x6c, AE_MAIN_DT_* 0x7a…0x80 | h.0x2218…0x2438, 0x2584…0x264c, 0x26ac…0x27c8 | tone-balance (Auto) target LUTs |

Not read by the AE: the status / output items of the same section (AE_ISO, AE_LVX10, AE_AV/TV/BV/ASV/DSV, AE_CUR_AVGY,
AE_IS_STABLE, AE_FGBG_*, AE_WDR_*, AE_HIST_STS, …) — debug-buffer slots the library could write back (`cfg_setcfgbyenum` is never
called by the 3A). Strategy-EX (§2.11) has no IQ item. Defaults when an item is absent: the values `AE_InitParameters` writes
first (the cfg read overwrites them). `cfg_getcfgbyenum` copies blindly — **a NULL cfg handle is not checked and would fault**
[R kernel decompile; user build not checked].

### 2.5 Per-frame flow `DoAe(h, in, out)` [R]
1. Keep in / out; if the statistics grid is not 32 × 32, rebuild the two metering grids (§2.3).
2. Copy the **applied** exposure from `in`: shutter → h.0xa50, sensor gain → h.0xa38, ISP gain → h.0xa40, FNx10 → h.0xa58
   (≥ 10); linear exposure product E = shutter·SG·DG / FN² (u64 h.0xa60); the same for the HDR short frame (h.0xac0 / aa8 / ab0,
   E h.0xad0). fps → h.0x3758 (×1000 if < 3000). Debug level → h.0x198.
3. `AE_ProcessAPI` (§2.16): if an API call is pending (h.0x47c8 = 1), copy the pending blocks into the live ones.
4. Flicker (§2.12): with the run-time detector off and a fixed mode set → `AE_FlickerFrequencyDetect` once per mode change;
   with FlickerEx enabled → `AE_FlickerFrequencyDetect_RunTimeMode` every frame.
5. `M[0x00] pAE_CalLumY` (§2.6) → return value r (≠ 0 = statistics unreliable); `M[0x04] pAE_GetBV` → BV (h.0xb9c);
   **target Y×10 = `Interp_LUT(AE_TARGET_LUT, BV)` → h.0x220** (the "default target" is re-derived from BV every frame);
   LV×10 = round(BV·10/16384) + 50 → h.0xba0; `M[0x88] pAE_isStable` → h.0xb90; `AE_MotionDetection`.
6. If r ≠ 0 and the state is RUN and fewer than 3 consecutive bad frames: count it, print `Unreliable stat`, **return −1** (output
   untouched → libcus3a keeps Change = 0).
7. State machine h.0x180 (`h.0x60·4`): 0 INIT → `AE_InitBV`, clear flags, `M[0x74] pAE_SetExposure(h, prev, applied)`, then →
   RUN; 2 (re-init) → RUN; 1 RUN → if `AEDEBUG.enable` = 1: `M[0x0c] pAE_ProcessAE` (§2.7); 3 paused (API 0x1406: no
   `pAE_ProcessAE`, the output repeats the last exposure); 4 (resume → 0). Frame counter h.0x18c++.
8. Without HDR the short-frame outputs mirror the long ones. Output: Change = (`M[0x80] pAE_isConverged` == 0) or a pending
   forced update (h.0x190); shutter / gains (long, short), FNx10, BV, CurY, AvgY, WeightY, target, IsStable, IsReachBoundary,
   histogram max bin, DebandFPS, frame-rate index + 1, the 16 BV-histogram words, and +0x8c (§2.2).

### 2.6 Measuring the scene [R]
**`pAE_CalLumY`** (method 0x00): copies hist1 (128 × u16) → h.0x29b0 and the RGB-IR histogram (256) → h.0x2bb0 (zeros if absent);
`AE_FindHistInfo`: total count h.0x2db4 and the highest non-empty bin ×2 + 1 → h.0xc4c (u8 "HistMaxY"); for each grid set (long;
short only with HDR statistics) and each cell (cx, cy): count = number of statistic blocks in the cell, sum = Σ block Y (byte 3 of
{R, G, B, Y}); cell Y×10 = 10·sum/count (rounded); **CurY×10** = Σ w·Y / Σ w with w = weight table[metering type][cy][cx] (32 × 32
u8 table indexed by cell, h.0xc4e + type·0x400 + cy·32 + cx; the short set uses w = 1); **AvgY×10** = plain mean of the cells; the
cell values are sorted ascending (a copy) for the percentile functions. h.0xba4 = CurY×10 (long), h.0xbbc = AvgY×10, h.0xbb0 /
h.0xbc0 the short-frame pair. With RGB-IR AE enabled it also derives the IR levels (§2.14).
**`pAE_GetBV`** (0x04): BV = log2(AvgY×10 / 470)·16384 + TV + AV − SSV − DSV of the **current** APEX record (h.0xb2c TV, h.0xb30
SSV, h.0xb34 DSV, h.0xb38 AV): the brightness the scene would have at the reference level 47.0. (Kernel: table with AvgY×10
clamped to 2560; user: `log2((double)max(AvgY, 1))`.) Side effect: day/night flag h.0x47c4 = (API day/night enable h.0x47b8 and
BV < threshold h.0x47bc).
**`AE_MotionDetection`**: counts blocks whose Y differs from the stored previous frame (h.0x37bb + 4i) by more than the threshold
u16 h.0x37ac; motion if count > h.0x37ae % of 1024 blocks; every h.0x37b6 frames, if the frame is outside the stable band, the state
h.0x37b0 = 2; result h.0x37b0 ∈ {0, 1 motion, 2}. Used with the AE stabilizer (§2.15).
**`pAE_CalSortYByPcent(h, set, fromDark, pcent‰)`** (0x34): mean Y×10 of the brightest (fromDark = 0) or darkest pcent‰ of the
sorted cell values (fractional cell weighted).
**`pAE_GetSceneDynamicRange`** (0x24): bright end Yb = (s·sortY(bright, p_b) + (1024 − s)·histY(bright, p_b)) / 1024 with p_b =
LUT `AE_DR_BT_PCENT`(BV), s = `AE_DR_BT_SORTY_WT`; dark end Yd likewise with `AE_DR_DT_*`; **DR = Yb·16384 / Yd** → h.0x2204
(×16384), components h.0x2164 / 0x2168 / 0x216c, h.0x21f8 / 0x2200 / 0x21fc.

**`pAE_CalHistogram(h, req)`** (method 0x30, the percentile engine) [R user disassembly 0x6ff0–0x753c; kernel decompile agrees]:
`req` = 17 u32 words (the AE keeps one at h.0x3700). Inputs: [5] mode, [6] direction, [7] exclusion enable (1 = on), [8] share ‰,
[9] exclusion share ‰, [10] / [11] Y bounds (mode 1), [0xd] Y range of the histogram (256), [0xe] bins, [0xf] total pixel count,
[0x10] → u16 histogram. Outputs: [0] Y×10, [1] 4·Y×10 (unrounded), [2] / [3] peak bin position / count, [4] share ‰ (mode 1),
[0xc] bin position where the share is reached (mode 0). Bin width w = [0xd] / [0xe]; bin i covers Y [w·i, w·(i+1)). Return 0.
At entry [2] = [3] = 0; any [5] other than 0 or 1 returns with nothing else written.
- **Mode 0 — mean of the brightest / darkest share**: N = ([8]·[0xf] + 500) / 1000 pixels; E = ([9]·[0xf] + 500) / 1000 when
  [7] = 1 (E = 0 if N < E, and always 0 when [7] ≠ 1).
  - [6] = 0 (from the bright end): bins i = [0xe]−1 … 0, pixel value v_i = w·(i+1) (top of the bin); accumulate counts until the
    running count reaches N (the crossing bin contributes only the missing part; [0xc] = w·i of that bin); the first E pixels of
    the scan (the very brightest) are accumulated separately and removed.
  - [6] ≠ 0 (from the dark end): bins i = 0 … [0xe]−1, v_i = w·i (bottom of the bin), the same with the darkest E pixels removed.
  - Result: [1] = (n/2 + 4·w·Σ 10·(i or i+1)·count) / n over the kept pixels (n = pixels counted − pixels excluded), [0] = ([1] +
    2)/4 = **mean Y×10 of the pixels ranked E … N from that end**; n = 0 → [0] = 470, [1] = 1880. If the histogram holds fewer than
    N pixels the whole histogram is used and [0xc] keeps the caller's value. [2] = position of the fullest bin (bright scan: w·(i+1)
    − 1, dark scan: w·i), [3] its count. Edge: a bin whose end coincides exactly with E is counted as kept, not excluded.
- **Mode 1 — share and mean above / below a level**: lo = ([10] + w/2) / w and hi = ([11] + w/2) / w (bin indices, rounded);
  [4] = 0.
  - [6] = 0: exclusion is disabled when lo ≥ hi; bins from the top down to lo + 1 are counted; with exclusion, bins above hi are
    left out of the mean (but not out of the share). If [0xe]−1 ≤ lo: [0] = 470, [1] = 1880, [4] = ([0xf]/2) / [0xf]. Result: [4] =
    (1000·count + [0xf]/2) / [0xf] = share ‰ of pixels brighter than [10]; [0] / [1] = mean Y×10 (v_i = w·(i+1)) of those pixels,
    minus the excluded ones.
  - [6] ≠ 0: exclusion is disabled when lo ≤ hi; bins 0 … lo − 1 are counted; with exclusion, bins below hi are left out of the
    mean. If lo = 0, the defaults above. [4] = share ‰ of pixels darker than [10]; [0] / [1] the mean (v_i = w·i).
  So in both directions [10] is the bound the scan counts up to and [11] the far-end cut-off (bright: ignore what is above
  [11]; dark: ignore what is below [11]).
- Debug bit 2 of h.0x198 with a non-zero frame counter prints bins in groups of six every tenth bin in the dark scan of mode 0.
- Callers and their request words [R store sites in the 3A code]: `pAE_StrategyEX_GetToneY` (0x40052ad0) and
  `pAE_GetSceneDynamicRange` (0x400528bc) write [5] = 0, [6] = 0 / 1, [7] = 0 as constants; `pAE_GetStrategyBTDT` takes {[5],
  [6], [7]} from the object words h.0x1a30…0x1a38 (bright tone) / h.0x1b60…0x1b68 (dark tone), and `pAE_GetToneBalanceTarget`
  from h.0x220c…0x2214 (dynamic-range part), h.0x2450…0x2458 (bright), h.0x2578…0x2580 (highlight limit), h.0x26a0…0x26a8 (mid
  tone, with [9] = (1000 − p_m)/2). No IQ item or API field writes these words; their values are the built-in defaults of §2.17.
  The RGB-IR levels (§2.14) use it too.

### 2.7 Exposure state and one RUN frame (`pAE_ProcessAE`, method 0x0c) [R]
Exposure state kept in APEX records: **prev** h.0xb18, **cur** h.0xb2c, **next** h.0xb40 (written N below; HDR short next
h.0xb54), manual h.0xb68 (short h.0xb7c), and one "plain" record **P** at h.0xa38 (HDR short **P_s** at h.0xaa8, same word layout):

| P word | offset | meaning |
|---|---|---|
| 0 / 1 | 0xa38 / 0xa3c | sensor gain SG (1024 = 1×) / previous; after `AE_InitBV` word 1 holds the SSV of the initial exposure |
| 2 / 3 | 0xa40 / 0xa44 | ISP digital gain DG / previous |
| 4 / 5 | 0xa48 / 0xa4c | total gain TG = SG·DG/1024 / previous |
| 6 / 7 | 0xa50 / 0xa54 | shutter µs / previous; after `AE_InitBV` word 7 holds the TV of the initial exposure |
| 8 (u16 ×2) | 0xa58 / 0xa5a | FN×10 / previous |
| 0xe | 0xa70 | shutter quantisation ratio ×1024 (`pAE_SetExposure`) |
| 0xf | 0xa74 | "a limit was hit" (`pAE_LimitMinMax`) |
| 0x10 (u16 ×2) | 0xa78 / 0xa7a | aperture limit words FN×10 (two writers with opposite order, §2.8) |
| 0x11 (u16 ×2) | 0xa7c / 0xa7e | user min ISP gain / min sensor gain |
| 0x12 (u16) | 0xa80 | user min shutter µs |
| 0x13 / 0x14 / 0x15 | 0xa84 / 0xa88 / 0xa8c | user max ISP gain / max sensor gain / max shutter µs |
| 0x16 (u16 ×2) | 0xa90 / 0xa92 | sensor min shutter / min gain (`AeInitParam_t`) |
| 0x17 / 0x18 / 0x19 / 0x1a | 0xa94 / 0xa98 / 0xa9c / 0xaa0 | sensor max shutter / max gain / shutter step / max ISP gain |

Linear exposure products E = shutter·SG·DG/FN² (u64) are kept for the applied (h.0xa60) and the next (h.0xa68) exposure.

One RUN frame:
1. **Linearity check** `pAE_CheckLinearity(prev APEX, cur APEX, CurY, previous CurY)` (0x8c) = |ΔEV commanded −
   log2(CurY/prevY)·16384| → h.0x1864 (how well the last change was obeyed by the sensor).
2. **Target**: if Strategy-EX is enabled (API 0x1416 ENABLE, h.0x1c94 = 1) → `pAE_StrategyEX_GetSceneTarget` (0x10, §2.11), which
   writes h.0x224 directly; else T_scene = `pAE_GetSceneTarget` (0x20, §2.10) and the smoothed target h.0x224 = (w·T_scene +
   (1024 − w)·h.0x224 + 512)/1024 with w = `Interp_LUT(h.0x1868 LUT, linearity error)` (default 4 points all 1024 → no
   smoothing), or T_scene directly when h.0x374c ≠ 0. Then, every RUN frame, `pAE_StrategyEX_GetGMBlendRatio` (0x1c, §2.11).
3. **EV compensation**: final target h.0x2b4 = T; if EV comp h.0x2bc ≠ 0: x = EVcomp·16384/grad (API 0x1403 {EVCOMP −10…10, GRAD
   1…10}, i.e. EVcomp/grad stops), ratio% = `Interp_LUT(h.0x230 LUT, x)` (13 points, defaults X = −6…+6 EV in 1 EV steps (×16384),
   Y = 1, 3, 6, 12, 25, 50, 100, 135, 160, 200, 232, 265, 300 %) → h.0x2c0; T = ratio·T/100.
4. **Time buffer** `pAE_CheckTimeBuffer` (0x90): after convergence, a change is allowed only after h.0x18fc / h.0x1904 frames
   (bright / dark side of the converged level h.0x18f0, thresholds h.0x18f8 / h.0x1900) unless a forced update is pending
   (h.0x190); otherwise count h.0x18f4 and keep the exposure.
5. If `pAE_isConverged` (0x80) = 0 or forced: `pAE_CalPvExpo` (0x38, §2.8) → next APEX; `pAE_SetExposure(next, P)` (0x74, §2.9); with
   HDR also `pAE_CalPvExpoShort` (0x3c, §2.13) and `pAE_SetExposure(short next, P_s)`; cur ← next. Else (converged): prev ← cur,
   converged level h.0x18f0 = CurY, counters reset; every h.0x37b6 frames keep a copy of the block averages for motion detection.

**Converged** (`pAE_isConverged`): CurY within [T − thd, T + thd] (thd = stable-in h.0x2c4 while moving, stable-out h.0x2c6 once
stable; lower bound ≥ 10, upper ≤ 2550), or the stabilizer motion state says hold (h.0x37a8 = 1 and h.0x37b0 = 0); also counted
converged after 30 frames at very high BV (BV ≥ 2·16384 and linearity error ≤ 30). RGB-IR adds an IR-level band test (§2.14).
**Reach boundary** (`pAE_isReachBoundary`): the next exposure equals the current one and the BV lies outside the exposure table
range by more than the table step. **Stable** (`pAE_isStable`) = converged or at boundary; output IsStable / IsReachBoundary.

**Convergence step** `pAE_CalSmoothExpo` (0x40): T' = min(T, 2550), Y = CurY×10; speed s = `Interp_LUT(AE_CONV_RATIO, Y)` (API
0x1408 CONV_SPEED_X/Y; ≥ 10) → h.0x2c8; max step m = s·max(stable-in, stable-out)/1024, at least 12 when the shutter is < 391 µs,
else 3; Δ = T' − Y; step = (s·Δ + 512)/1024, but if |step| < m then step = sign(Δ)·min(|Δ|, m); new Y = Y + step (or T' at once
when h.0x374c = 1) → h.0xbc4; exposure ratio h.0x1854 = (Y + step)·1024/max(Y, 10), ΔEV h.0x1850 = log2((Y + step)/Y)·16384.

### 2.8 Next exposure: the exposure table and the exposure modes [R]
Notation: N = the next record h.0xb40 being built; exposure table entry i = h.0x6fc + 0x14·i (an APEX record); T0 = entry 0 (the
bright end: highest BV, highest TV, lowest SV), TL = the last entry (count from `pAE_GetExpTblMaxIdx`); the table is ordered by
falling BV.

**Exposure tables as stored.**
- `pAE_SetPlainExpTbl(h, tbl)` (API 0x140a, and at init for the IQ-file table AE_EXPO_TBL_NUM + AE_EXPO_TBL): tbl = {u16 num; u16
  minDG; u16 minSG; u16 minShutter; u32 maxDG; u32 maxSG; u32 maxShutter; 16 × {u16 FNx10, u16 pad, u32 shutter µs, u32 total
  gain TG, u32 sensor gain SG}; then at +0x118 16 × u64 linear exposure E}; total 0x198 B. The setter fails (−1) if an entry has
  SG = 0, or if the table leaves the sensor limits (min SG < h.0xa92, max SG > h.0xa98, min shutter < h.0xa90, max shutter >
  h.0xa94); a NULL table dereferences address 0 (kernel). Otherwise it derives per table min / max of DG = TG·1024/SG (max ≤ ISP
  max h.0xaa0), SG, shutter, FNx10 and E_i = TG·shutter·1024 / FN², writes them into the table header, and **overwrites the user
  limits** with the table extents: h.0xa7c min DG, h.0xa84 max DG, h.0xa7e min SG, h.0xa88 max SG, h.0xa80 min shutter, h.0xa8c
  max shutter, **h.0xa78 = the largest FNx10, h.0xa7a = the smallest FNx10**; copies the table to h.0x3c8 (live; plain entries
  h.0x3cc + 0x10·i, linear E per point h.0x4e0 + 8·i) and h.0x4dd8 (API copy), converts it with `AE_ConversionExpoTbl` to h.0x6f8,
  sets the forced-update flag h.0x190. So loading a table (0x140a) resets any 0x1409 limits.
- `AE_ConversionExpoTbl(dst, tbl)`: dst = {u32 num; 18 × {TV, SSV, DSV, AV, BV}}: for each entry AV = 2·(T[min(FNx10, 1000)] −
  log2(10)·16384), TV = `USEC2TVXBASE`(shutter), SSV = `GAIN2SVXBASE`(SG), DSV = `GAIN2SVXBASE`(TG) − SSV, BV = AV + TV −
  `GAIN2SVXBASE`(TG); then slot 16 = a copy of entry 0 and slot 17 = a copy of the last entry. Long table dst = h.0x6f8 (count
  h.0x6f8, slots h.0x6fc…0x863), short table dst = h.0x864.
- `pAE_SetPlainShortExpTbl` (0x140b, AE_HDR_SHORT_EXPO_TBL) is the same for the HDR short frame: sensor limits h.0xb00…0xb08, user
  limits h.0xae8…0xafc (same order as h.0xa78…0xa8c), live copy h.0x560, API copy h.0x4f70, APEX table h.0x864 (entries h.0x868 +
  0x14·i, count u8 h.0x864), plain entries h.0x574 + 0x10·i, linear E at h.0x678 + 8·i.
- `pAE_GetExpTblMaxIdx` (0x78) = count, minus h.0x3774 entries unless the day/night mode word h.0x3770 is 4 or 7. Walking down
  the table (darker scene → lower BV) moves along the points in order, so the table defines which quantity grows first (typically
  shutter up to 1/fps, then sensor gain, then ISP gain) [I for "typically"].

**`pAE_CalPvExpo`** (0x38) [R; user disassembly 0xe220 for the mode switch]: ΔEV = 0 when frozen (h.0x358 = 1) or nothing to do;
prev ← cur; BV' = cur BV − ΔEV clamped to the exposure table's BV range; `pAE_CalIntpApex(BV')` (0x44): `pAE_CalApexTblIdx`
(0x4c) finds the brighter / darker entries B = i, D = i + 1 with BV_i > BV' ≥ BV_{i+1} (both 0 above the table, both last below
it; kept at h.0x9d0 = B, h.0x9d1 = D, BV' at h.0x9d4) and TV, SSV, DSV, AV are interpolated linearly in BV between the two entries
→ N; then by **exposure mode** h.0x184 (API 0x1404: 0 Auto, 1 AV priority, 2 SV priority, 3 TV priority, 4 Manual):

| mode | steps |
|---|---|
| 0 Auto | `pAE_CalApexAuto2` (0x54, below); N → plain values only when `pAE_FurtherLimitOnAperture` = 1 (otherwise P was filled by the interpolation step); then always `pAE_LimitMinMax` and `pAE_DoDeBanding` |
| 1 AV | `pAE_CalApexAV2` (0x58) → APEX→plain → `pAE_LimitMinMax` → `pAE_DoDeBanding` |
| 2 SV | `pAE_CalApexSV2` (0x5c) → plain → limit → de-band |
| 3 TV | `pAE_CalApexTV` (0x60) → plain → limit (no de-banding) |
| 4 Manual | N = the manual APEX record h.0xb68…0xb78 (set by API 0x1405, BV = AV + TV − SSV − DSV) → plain → limit |

APEX→plain = `AE_ConvertNextExpoToExpoVal`: shutter = TV2USEC(TV), SG = max(1024, SV2GAIN(SSV)), DG = max(1024, SV2GAIN(SSV +
DSV)·1024/SG), FN×10 = AVXBASE2FNX10(AV); previous values shifted into the "previous" words.

**The aperture limit words** [R + I]. Two writers use h.0xa78 / h.0xa7a with opposite meanings: the table setter stores (largest
FN, smallest FN) — FN of T0 and TL, since the bright end has the smallest opening — while `DrvAlgo_IF_AE_SetExposureLimit`
(0x1409) calls `pAE_SetMinMaxAperture(MIN_FNx10, MAX_FNx10)`, which clamps each to 10…220 and to the table's FN range and stores
**h.0xa78 = MIN_FN, h.0xa7a = MAX_FN** (`pAE_GetMinMaxAperture` returns them in the same order). Every reader follows the table
convention: `pAE_FurtherLimitOnAperture` (method 0x68) = 0 iff T0.AV = AV(h.0xa78) and TL.AV = AV(h.0xa7a), else 1;
`pAE_FindClosestAV` clamps its result to [AV(h.0xa7a), AV(h.0xa78)]. After a 0x1409 call with MIN_FN < MAX_FN the clamp window
is inverted, so any table aperture is replaced by one of the two user limits (above AV(MIN_FN) → AV(MIN_FN), otherwise
AV(MAX_FN)), and FurtherLimitOnAperture stays 1 [R arithmetic; I: an inversion in SigmaStar's code]. With a fixed-iris table (one
FN in every entry) both words are clamped to that FN and nothing changes; the FN values of this camera's IQ table were not read [I:
the camera has no iris].

**`pAE_FindClosestAV(h, AV, dir, n)`** (method 0x6c): index search over the table AV column (callers in the AE always pass n = 0):
dir 1 — scan from the last entry towards entry 0 and take the first entry with AV_i ≥ AV; dir 0 — scan from entry 0 and take the
first with AV_i ≤ AV; dir 2 — the entry with the smallest |AV_i − AV| (strictly smaller wins, so the first of equals; the running
best starts at the raw value T0.AV, not at a difference); any other dir → entry 0. Index clamped to [0, count − 1]; result AV_i
clamped to [AV(h.0xa7a), AV(h.0xa78)] (user build: double `log2`, kernel: table; FNx10 ≥ 1000 clamped to 1000 for the lookup).

**Priority modes** [R kernel decompile]. Ranges from the table: TV ∈ [TL.TV, T0.TV], SSV ∈ [T0.SSV, TL.SSV], DSV ∈ [T0.DSV,
TL.DSV + 16384] (the ISP gain may go one EV above the table's last entry). "Spill" = move what one quantity cannot take into the
next, keeping BV.
- **AV priority, `pAE_CalApexAV2`** (0x58, mode 1): manual AV h.0xb74 (set by 0x1405 from FNx10) clamped to [TL.AV, T0.AV] and
  written back; AV' = FindClosestAV(it, dir 2); TV' = TV + AV − AV' clamped to the TV range; the overflow r goes to SSV' = SSV − r
  (clamped), its overflow to DSV' = DSV + r' (clamped); what is left only prints a debug line; BV recomputed.
- **SV priority, `pAE_CalApexSV2`** (0x5c, mode 2): manual SSV h.0xb6c and DSV h.0xb70 clamped to their ranges (DSV only to
  TL.DSV here) and written back; e = new SV sum − old SV sum, limited so that TV + AV + e stays within [TL.(AV+TV), T0.(AV+TV)]; if
  TV + e would leave the TV range the aperture moves: AV' = FindClosestAV(AV + e, dir = (e ≥ 0 ? 1 : 0)), else AV' =
  FindClosestAV(AV, 2); TV' = TV + e − (AV' − AV) clamped; BV recomputed.
- **TV priority, `pAE_CalApexTV`** (0x60, mode 3): manual TV h.0xb68 clamped to the TV range and written back; δ = TVm − TV limited
  so that TV + AV + δ stays within [TL.(AV+TV), T0.(AV+TV)]; TV' = TV + δ; if SSV + DSV + δ would leave [T0.(SSV+DSV),
  TL.(SSV+DSV)], AV' = FindClosestAV(AV − δ, dir = (δ ≤ 0 ? 1 : 0)), else FindClosestAV(AV, 2); the SV sum moves by e = δ + AV' −
  AV: outside the sum range it is set to T0's or TL's SSV and DSV (overflow printed), inside SSV' = SSV + e clamped to its range and
  the rest added to DSV; BV recomputed.
- **Auto, `pAE_CalApexAuto2`** (0x54, mode 0), called as Auto2(h, D = h.0x9d1, B = h.0x9d0, N, P):
  - FurtherLimitOnAperture = 1 (user FN limits differ from the table's FN range): exactly the AV2 redistribution with AV' =
    FindClosestAV(AV_N, 2) — snap to the nearest permitted table aperture and spill the difference onto TV → SSV → DSV.
  - FurtherLimitOnAperture = 0 and AV_B = AV_D (the iris does not change inside this segment, always the case for a fixed iris):
    N unchanged (pure interpolation).
  - FurtherLimitOnAperture = 0 and AV_B ≠ AV_D (the interpolated aperture lies between two table stops): two snap attempts, both
    computed from the interpolated N:
    1. **close to AV_B** if (AV_B − AV_N) ≤ (TV_N − TV_D) + (SSV_D − SSV_N) + (DSV_D + 16384 − DSV_N) and the two gain rooms
       together are ≥ 0.5 EV (8192): δ = AV_N − AV_B (≤ 0); TV' = TV_N + δ if |δ| < TV room, else TV' = TV_D with carry c = δ + TV
       room; then SSV' = SSV_N + c if |c| < SSV room, else SSV' = SSV_D with carry c − SSV room; then DSV likewise against DSV_D +
       16384; AV' = AV_B.
    2. **open to AV_D** if (AV_N − AV_D) ≤ (TV_B − TV_N) + (SSV_N − SSV_B) + (DSV_N − DSV_B) and the two gain rooms are ≥ 1 EV
       (16384): δ = AV_D − AV_N (≤ 0); TV' = TV_N − δ if |δ| < TV room toward B, else TV' = TV_B with carry; the carry is then
       applied as SSV' = SSV_N − c / DSV' = DSV_N − c' but **tested against the rooms toward D** of step 1 (two locals,
       possibly already changed by step 1); AV' = AV_D. When both conditions hold, step 2 overwrites step 1.
    The carries in both steps keep the sign of δ where a BV-preserving split needs the opposite one (e.g. step 1 lowers SSV by |c|
    when the shutter cannot absorb the whole step, while its "room exhausted" branch raises SSV to SSV_D) [R arithmetic, same in
    the kernel and user builds; I: a defect — the recomputed BV then differs from the interpolated BV]. When |δ| fits into the TV
    room (the usual small step) only TV and AV change and BV is preserved.
  BV recomputed at the end. A fixed iris takes the table values unchanged.

**Debug-level bit 4: the linear-exposure interpolation variant.** The AE debug word h.0x198 (from `AEDEBUG_t.debug_level`;
libcus3a's default 0, settable through the customer control 0x3000, isp-6-userspace.md §2.7) is not only a print mask: with bit
4 (0x10) set, `pAE_CalPvExpo` first scales the applied linear exposure E (h.0xa60) by the exposure ratio h.0x1854/1024 into
h.0xa68, clamped to [E_0, E_last] of the table; `pAE_CalIntpApex` then (when the two bracketing plain entries have equal FN, or
equal shutter and SG, or equal DSV) interpolates FNx10, shutter and SG **linearly in E** between the plain entries (`InterpU64`)
and derives DG = E·FN²/(shutter·SG), clamped to the brighter entry's DG, writing P (h.0xa38 / 0xa40 / 0xa50 / 0xa58) directly;
`pAE_CalApexAuto2` also writes P's FN / SG / DG / shutter from the plain table in its snap steps. `pAE_CalIntpApexShort` does the
same for the short frame (plain entries h.0x574, E at h.0x678). The other bits tested in either copy (0, 1, 2, 5, 7, 9, 10, 11,
12, 14) only gate `CamOsPrintf` traces (e.g. bit 0 SetExposure, bit 1 per-frame flow, bit 2 histogram, bit 5 API setters /
getters, bit 7 de-banding, bit 9 RGB-IR, bit 11 Strategy-EX, bits 12 / 14 interpolation and start-up flicker detector) [R: every
test site].

### 2.9 Limits, de-banding and the final exposure [R]
**`pAE_LimitMinMax(next, P)`** (0x70) keeps E = shutter·SG·DG and clamps into the user limits (P words 0x10…0x15): modes 0 / 1 —
DG and SG clamped, shutter = E/(SG·DG) clamped, then SG = E/(DG·shutter) clamped, then DG = E/(shutter·SG) clamped (the residual
goes to shutter first, then sensor gain, then ISP gain); mode 2 — only the shutter moves; mode 3 — shutter fixed, SG then DG move;
mode 4 — untouched. P.0xf = 1 if any value sits on a limit; next APEX recomputed.
**`pAE_DoDeBanding(next, P)`** (0x64) [user decompile where the kernel switch is broken]: skipped when flicker handling is off
(h.0x354 = 1, the state after an IQ-file load, §2.12) or in modes 3 / 4. Period p = 8333 µs (60 Hz, h.0x350 = 1) or 10 000 µs
(50 Hz). Shutter < p: kept (or forced up to p when h.0x358 = 1, then gain reduced); else shutter = n·p with n = ⌊shutter/p⌋ (60 Hz
with h.0x358: n = 5 / 11 → n − 1, 6 / 12 → n + 1; 50 Hz: n = 5, 10, 15 adjusted), gain ratio r = shutter·1024/(n·p) → h.0x3768,
applied to SG (to DG in SV mode, or when SG would pass the sensor max), then SG ≤ sensor max and DG ≤ ISP max, lengthening the
shutter if the gain cannot absorb it.
**`pAE_SetExposure(APEX, P)`** (0x74, the last step, also used at INIT): rounds the shutter up to a multiple of the sensor shutter
step (P.0x19), ratio requested / quantised (×1024) → P.0xe, total gain TG' = TG·ratio; if AE_SHUTTER_COMP_GAIN (h.0x3754) = 1 and
not HDR mode 2 the correction goes into SG (SG = TG'/DG), else into DG (DG = TG'/SG); SG clamped to [sensor min, sensor max]
(unless frozen), DG = TG'/SG; if DG ends > ISP max or < 1024 the un-quantised shutter is kept and DG = TG/SG clamped to [1024, ISP
max]; RGB-IR AE re-derives DG (`AE_CalNewDGainWithIR`, §2.14); APEX recomputed from the final plain values.

### 2.10 Scene targets (`pAE_GetSceneTarget`, method 0x20) [R]
`pAE_GetSceneTarget`: `pAE_GetSceneDynamicRange` (§2.6); then by strategy mode h.0x1908 (API 0x140f MODE / IQ AE_STRATEGY: 0
bright-tone priority, 1 dark-tone priority, 2 auto): mode 2 → if AE_ENABLE_STRETCH (h.0x2208) = 1 the tone-balance target
`pAE_GetToneBalanceTarget` (h.0x27e0), else the BV target h.0x220; modes 0 / 1 → `pAE_GetStrategyBTDT` [R user decompile; the
kernel decompile is cluttered by debug prints here].
Every target is a luminance Y×10 (0…2550). Common pattern: a percentile p (‰, from a BV-indexed LUT) of the brightest or darkest
pixels, its luminance Y_p — the mean Y×10 of that share — from the 128-bin histogram (`pAE_CalHistogram` mode 0, §2.6) and / or
from the sorted cells (`pAE_CalSortYByPcent`), and a wanted level t; the target for the mean is then **t·CurY / Y_p** (scale the exposure so that the p-percentile lands on t).
API "strength" s and "sensitivity" σ (0…1024, API 0x140f fields BT / DT / A STRENGTH / SENSITIVITY, live copies h.0x1b58 / 0x1b5c
(BT), 0x1c88 / 0x1c8c (DT), 0x27d8 / 0x27dc (Auto)) blend LUT values toward neutral: `Interp(0, 1024, value, neutral, s)`.
- **Bright-tone priority** (`pAE_GetStrategyBTDT`, mode 0): p = LUT AE_STGY_BT_PCENT(BV) blended toward 10 ‰ by σ_BT → h.0x1b48;
  t = LUT AE_STGY_BT_TARGET(DR) blended toward the BV target h.0x220 by s_BT → h.0x1b4c; Y_p = histogram mean of share p from the bright
  end → h.0x1b50; T_bt = t·CurY/Y_p → h.0x1b54; result = default target if T_bt ≥ default, else (W·T_bt + (1024 − W)·default)/1024 with
  W = AE_STRATEGY_WEIGHTING (h.0x190c) — bright-tone can only lower the target.
- **Dark-tone priority** (mode 1): p = AE_STGY_DT_PCENT(BV) blended toward 900 ‰ by σ_DT → h.0x1c78; t = AE_STGY_DT_TARGET(BV)
  blended toward 40 by s_DT → h.0x1c7c; Y_p from the dark end → h.0x1c80; T_dt = 4·t·CurY/Y_p → h.0x1c84; result = default if T_dt ≤
  default, else the same W-blend — dark-tone can only raise it.
- Both then clamp into **[default − LOWER(BV), default + UPPER(BV)]** (AE_STRATEGY_LOWER / UPPER LUTs, ≥ 10, ≤ 2550; bounds kept in
  h.0x22c / h.0x228) → h.0x1c90 = the strategy target.
- **Auto, tone balance** (`pAE_GetToneBalanceTarget`, mode 2 with AE_ENABLE_STRETCH = 1), all LUTs indexed by BV unless noted:
  bright: p_b = MAIN_BT_PCENT blended toward 10 by σ_A (h.0x2324); Y_hist(p_b); t_b from MAIN_BT_TARGET (indexed by the dynamic
  range DR) scaled by MAIN_BT_TARGETBYBV, ≥ default, blended by s_A (h.0x2328); Y_sort(p_b) (h.0x243c); w = MAIN_BT_BLKYWGHT
  (h.0x2438); Y_b = (w·Y_sort + (1024 − w)·Y_hist)/1024 (h.0x2440); **T_b = t_b·CurY/Y_b** (h.0x244c).
  mid: p_m = MAIN_MT_PCENT blended toward 900 by σ_A (h.0x2690); t_m = MAIN_MT_TARGET blended toward 40 by s_A (h.0x2694); **T_m =
  t_m·CurY/Y_hist(p_m)** (h.0x269c). The mid-tone histogram request (h.0x26a0…0x26a8 = {mode 0, from the dark end, exclusion
  off}) passes [9] = (1000 − p_m)/2, but with exclusion off no band is excluded; nothing in the IQ file or API changes those words.
  dark: p_d = MAIN_DT_PCENT → 900 (h.0x27b8); t_d = MAIN_DT_TARGET → 40 (h.0x27bc); Y_d = (w_d·Y_sort(dark, p_d) + (1024 − w_d)·
  Y_hist)/1024, w_d = MAIN_DT_SORTY_WT; **T_d = 4·t_d·CurY/Y_d** ≤ 2550 (h.0x27c4), smoothed h.0x27d4 = (3·old + T_d + 2)/4 (direct
  when h.0x374c = 1).
  highlight limit: p_h = LUT h.0x245c (defaults 500 ‰ ×3), t_h = LUT h.0x24e0 (defaults 1440 / 1500 / 1550), **T_h =
  t_h·CurY/Y_hist(p_h)** clamped 100…2000 (h.0x2574).
  **T = max(min(max(T_d,smoothed, T_m, t_d), T_h), T_b)** → h.0x27e0, then clamped to [default − LOWER, default + UPPER] like the
  others. (As decoded; the bright term acts as a floor here.)
- **Auto without stretch**: the BV target h.0x220 (§2.5 step 5) unchanged.
- **Strategy-EX** (API 0x1416, off by default): replaces all of the above, §2.11.

### 2.11 Strategy-EX (`pAE_StrategyEX_*`, API 0x1416 / 0x1417) [R]
Methods 0x10 GetSceneTarget, 0x14 GetToneY, 0x18 GetCntPcntx10, 0x1c GetGMBlendRatio. Read from the kernel and user copies and the
user disassembly at 0x90e0 (base 0x10000) (GMBlendRatio), 0x95d0 (GetToneY), 0x96c8 (GetSceneTarget); field names from the camera's
`isp_api.xml`.

**API block 0x1416 `ExposureStrategyEx` (660 B) → pending h.0x4984 → live h.0x1c94 (0x42c B).** The API block stores each half
(bright tone BT, dark tone DT) as one shared X axis (BV) and four Y arrays; the object stores four full 0x84-byte LUTs per half,
each carrying its own copy of `num` and of the X array. `pAE_SetAEStrategyEX` expands the block into the pending area;
`AE_ProcessAPI` copies pending → live with one `memcpy(h+0x1c94, h+0x4984, 0x42c)` at the start of the next `DoAe`;
`pAE_GetAEStrategyEX` packs the **pending** area back.

| API off | field (XML) | type, range (XML) | pending | live | live LUT meaning |
|---|---|---|---|---|---|
| 0x000 | ENABLE | u32 0…1 | h.0x4984 | h.0x1c94 | 1 = Strategy-EX replaces the normal scene target |
| 0x004 | MODE | u32 0…2 "Count Mode, Target Mode" | h.0x4988 | h.0x1c98 | 1 = target mode, any other value = count mode |
| 0x008 | PRIORITY | u32 0…2 "BrightTone, DarkTone" | h.0x498c | h.0x1c9c | 1 = dark-tone priority, any other value = bright-tone |
| 0x00c | BT_NUM | u16 1…16 | num of the 4 BT LUTs: h.0x4990 / 0x4a14 / 0x4a98 / 0x4b1c | h.0x1ca0 / 0x1d24 / 0x1da8 / 0x1e2c | |
| 0x00e | PAD1 | u16 | — | — | not read |
| 0x010 | BT_BV[16] | s32 −81920…245760 (BV×16384, −5…+15 EV) | X of the 4 BT LUTs: h.0x49d4 / 0x4a58 / 0x4adc / 0x4b60 | h.0x1ce4 / 0x1d68 / 0x1dec / 0x1e70 | |
| 0x050 | BT_THDY[16] | u32 0…255 | Y h.0x4994 | **LUT h.0x1ca0** | BT luminance threshold (0…255) vs BV |
| 0x090 | BT_PERCENTx10[16] | u32 0…1000 ‰ | Y h.0x4a18 | **LUT h.0x1d24** | BT pixel share vs BV |
| 0x0d0 | BT_TARGETYx10[16] | u32 0…2550 | Y h.0x4a9c | **LUT h.0x1da8** | BT target Y×10 vs BV |
| 0x110 | BT_MaxOffsetDown[16] | u32 0…2550 | Y h.0x4b20 | **LUT h.0x1e2c** | how far below the BV target the result may go |
| 0x150 | DT_NUM | u16 1…16 | num of the 4 DT LUTs: h.0x4ba0 / 0x4c24 / 0x4ca8 / 0x4d2c | h.0x1eb0 / 0x1f34 / 0x1fb8 / 0x203c | |
| 0x152 | PAD2 | u16 | — | — | not read |
| 0x154 | DT_BV[16] | s32 | X h.0x4be4 / 0x4c68 / 0x4cec / 0x4d70 | h.0x1ef4 / 0x1f78 / 0x1ffc / 0x2080 | |
| 0x194 | DT_THDY[16] | u32 0…255 | Y h.0x4ba4 | **LUT h.0x1eb0** | DT luminance threshold vs BV |
| 0x1d4 | DT_PERCENTx10[16] | u32 ‰ | Y h.0x4c28 | **LUT h.0x1f34** | DT pixel share vs BV |
| 0x214 | DT_TARGETYx10[16] | u32 0…2550 | Y h.0x4cac | **LUT h.0x1fb8** | DT target Y×10 vs BV |
| 0x254 | DT_MaxOffsetUp[16] | u32 0…2550 | Y h.0x4d30 | **LUT h.0x203c** | how far above the BV target the result may go |

The live area ends at h.0x20c0 (= 0x1c94 + 0x42c), where the status block begins. Besides the pending copy, the setter writes
two live words **at once** (not through pending): h.0x20d0 = max(BT_MaxOffsetDown[0…BT_NUM−1]) and h.0x20d2 =
max(DT_MaxOffsetUp[0…DT_NUM−1]) (u16; 0 when the count is 0); then pending flag h.0x47c8 = 1 [R both copies]. No field is
range-checked: NUM > 16 makes the LUTs read past their 16 entries; the max loops count with an **8-bit** index compared against the
u16 NUM, so NUM ≥ 256 never terminates (kernel: a hung ISP thread; user: a hung 3A thread) [R loop code; consequence I]. Kernel
and user setters store the same fields (kernel word by word, user with 16-byte block copies). Strategy-EX has no IQ-file item;
the built-in defaults (§2.17: ENABLE = 0) keep the whole path idle until API 0x1416 enables it.

**Per frame.** `pAE_ProcessAE` calls `M[0x10]` instead of the normal target path when h.0x1c94 = 1, and calls `M[0x1c]` every RUN
frame regardless. BV = h.0xb9c, D = the BV target h.0x220, Y_cur = CurY×10 of metering set h.0xb93 (`h.0xba4 + 4·h.0xb93`;
h.0xb93 is only ever set to 0, in `AE_InitParameters`, so it is the long-frame CurY h.0xba4).

**`pAE_StrategyEX_GetSceneTarget`** (0x10):
1. LUT values at BV: thd_bt = BT_THDY(BV), thd_dt = DT_THDY(BV), tgt_bt = BT_TARGETYx10(BV), tgt_dt = DT_TARGETYx10(BV), up =
   DT_MaxOffsetUp(BV), down = BT_MaxOffsetDown(BV).
2. Bounds: upper = min(D + up, 2550) → h.0x20c2; lower = max(D − down, 10) → h.0x20c4 (both u16, the 0x1417 info fields).
3. `M[0x14] GetToneY`, then `M[0x18] GetCntPcntx10` (below).
4. Two candidate targets (unsigned integer division, rounded):
   - MODE = 1 (target mode): Ybt = max(h.0x20ca, 10), Ydt = max(h.0x20cc, 10) (mean Y×10 of the BT / DT pixel share); T_bt =
     (Y_cur·tgt_bt + Ybt/2) / Ybt, T_dt = (Y_cur·tgt_dt + Ydt/2) / Ydt — scale the exposure so that the mean of the brightest
     (darkest) share lands on its target.
   - otherwise (count mode): ybt = max(h.0x20ce, 1), ydt = max(h.0x20cf, 1) (luminance 0…255 at which the share is reached);
     T_bt = (Y_cur·thd_bt + ybt/2) / ybt, T_dt = (Y_cur·thd_dt + ydt/2) / ydt — scale so that the share boundary lands on the
     threshold. (Units: Y×10 · (0…255) / (0…255) = Y×10.)
5. Priority: PRIORITY = 1 (dark tone): T = T_dt if T_dt ≥ D or T_dt > T_bt; otherwise T = min(D, T_bt). Otherwise (bright tone):
   T = T_bt if T_bt ≤ D or T_bt < T_dt; otherwise T = max(D, T_dt).
6. T clamped to [lower, upper] (T ≤ lower → lower, T > upper → upper) and stored **directly** into the smoothed target h.0x224
   (no smoothing, unlike the normal path) [R user disassembly; kernel decompile agrees].

**`pAE_StrategyEX_GetToneY`** (0x14): two `pAE_CalHistogram` requests on the 128-bin luma histogram h.0x29b0 (request block
h.0x3700: [0xd] = 256 Y levels, [0xe] = 128 bins, [0xf] = total count h.0x2db4, [0x10] → h.0x29b0, [5] mode 0 = share → Y, [7] = 0
no exclusion):
- [6] = 0 (from the bright end), [8] = BT_PERCENTx10(BV) → mean Y×10 of the brightest share → **h.0x20ca** (BTYx10); the bin
  luminance where the share is reached ([0xc], 0…255) → u8 **h.0x20ce**;
- [6] = 1 (from the dark end), [8] = DT_PERCENTx10(BV) → mean Y×10 of the darkest share → **h.0x20cc** (DTYx10); boundary → u8
  **h.0x20cf** [R; literal pool 0x96b8 = {0x100, 0x80}, 0x96c0 = {0, 1}].

**`pAE_StrategyEX_GetCntPcntx10`** (0x18): shares of the 128-bin histogram H beyond the two thresholds, for information only
(nothing reads them back except the 0x1417 getter) [R: no other reader of h.0x20c6 / 0x20c8 in either copy]:
- BTCntPcntx10 h.0x20c6 = (Σ H[b] for b = thd_bt/2 … 127) · 1000 / 33909;
- DTCntPcntx10 h.0x20c8 = (Σ H[b] for b = 0 … thd_dt/2) · 1000 / 33909.
The sums are truncated to **16 bits** before the multiply, and the divisor is the fixed constant 33909 (0x8475; kernel: multiply by
0xF762C969 >> 47, the same division), not the histogram total. Why 33909 is not visible in the code [I: a nominal total of the
hardware histogram]. With thd_bt ≥ 256 the BT sum is empty → 0 (the user build tests it explicitly, the kernel by the empty loop).

**`pAE_StrategyEX_GetGMBlendRatio`** (0x1c), every RUN frame:
1. History shift h.0x20d8 → h.0x20da, h.0x20d6 → h.0x20d8, h.0x20d4 → h.0x20d6.
2. ENABLE ≠ 1: clear the info words h.0x20c2…0x20c9, h.0x20ca, h.0x20cc; ratio h.0x20d4 = 512.
   ENABLE = 1, with D = h.0x220 and Ts = h.0x224 (the Strategy-EX target of this frame): Ts > D → ratio = 512 + ((Ts − D)·512 +
   h.0x20d2/2) / h.0x20d2 (max DT_MaxOffsetUp); Ts < D → ratio = 512 − ((D − Ts)·512 + h.0x20d0/2) / h.0x20d0 (max
   BT_MaxOffsetDown); Ts = D → 512. Since Ts is clamped to the offsets of the current BV, the ratio stays in 0…1024 [I from the
   clamp]; a 0 maximum makes the user build divide by zero through its own division helper (0x20678) [R call; result I].
3. **h.0x20c0 = the old h.0x20da**, i.e. the ratio is published three frames late.
`DoAe` writes `out+0x8c = h.0x20c0 + 1` (u16).

**Status block h.0x20c0 = API 0x1417 `AE_StrategyExInfo`**: `DrvAlgo_IF_AE_GetStrategyEXInfo` copies 14 bytes from h.0x20c0 (XML
struct 16 B with PAD1):

| info off | h. | XML name | written by |
|---|---|---|---|
| 0 | 0x20c0 | GMBlendRatio (0…1024) | GetGMBlendRatio (3-frame delayed) |
| 2 | 0x20c2 | UpperLimitTargetx10 | GetSceneTarget |
| 4 | 0x20c4 | LowerLimitTargetx10 | GetSceneTarget |
| 6 | 0x20c6 | BTCntPcntx10 | GetCntPcntx10 |
| 8 | 0x20c8 | DTCntPcntx10 | GetCntPcntx10 |
| 10 | 0x20ca | BTYx10 | GetToneY |
| 12 | 0x20cc | DTYx10 | GetToneY |
| (14) | 0x20ce / 0x20cf | — (u8 BT / DT boundary luminance, not exported) | GetToneY |

Private words after it: h.0x20d0 / h.0x20d2 max offsets (setter), h.0x20d4…0x20da ratio history (4 × u16).

**Where the blend ratio goes.** In-kernel 3A: `CameraIspAlgo_DoAe` runs `DoAe(…, out = ch+0x88)`, so out+0x8c = **ch+0x114**;
`CameraIspAlgo_DoIq` and `CameraIspAlgo_DoIqPerFrame` copy ch+0x114 to IqInput+0x4e (`g_tIspMidHandler+0xe4e`), which the
AdaptiveGamma blend uses as w = value − 1 = GMBlendRatio (w < 0x200 blends toward BTGamma, w > 0x200 toward DTGamma;
isp-3-iq-api.md §3.5 "Gamma group", isp-4-iq-engine.md §2.2). So a Strategy-EX target below the BV target (bright-tone protection)
pulls the gamma toward the bright-tone curve, one above it toward the dark-tone curve, in proportion to how much of the allowed
offset is used. User 3A (this camera): libcus3a's `SigmaAeRun` copies out+0x88 (FNx10) and out+0x84 (DebandFPS) into the 0x2E06
result but nothing from out+0x8c, so the ratio computed by libispalgo is dropped [R libcus3a 0x14658]; ch+0x114 is then not fed
by the user AE [I: no 0x2E06 field maps to it]. Whether AdaptiveGamma is enabled in this camera's IQ file / API bin is not known.

### 2.12 Flicker [R unless marked]
**State.** h.0x350 frequency (1 = 60 Hz → period 8333 µs, 0 = 50 Hz → 10 000 µs); h.0x354 mode (1 = off / "disable", 2 =
de-banding on; built-in default 2, but the IQ item AE_FLICKER_MODE is read and then **overwritten with 1**, so after an IQ-file
load de-banding starts off); h.0x358 strict flag (shutter forced to ≥ one period). API 0x140e (`pAE_SetFlicker` → pending
h.0x4dbc, 1 = 60 Hz; `pAE_SetFlickerMode` → pending h.0x4dc0, 0 auto, 1 disable, ≥ 2 → 2) feeds h.0x350 / h.0x354 whenever
FlickerEx ENABLE ≠ 1. De-banding itself: `pAE_DoDeBanding` (§2.9).

**`AE_FlickerFrequencyDetect`** (start-up auto detection; runs when the run-time detector is off and h.0x34c / h.0x354 say
"detect"): a **global** state machine (one static block for all AE objects, `.bss` near `dev_attr_isp_reset+0x1618`: {settle
count, phase, score60, score50}): phase 1 → force 60 Hz de-banding (h.0x350 = 1, forced update) → wait → phase 5: measure; phase
3 / … → force 50 Hz → phase 7: measure. Measure = per block row the sum of block Y (h.0x2dc0[row]), difference to the previous
frame's row profile (h.0x2e40), correlated with ±1 square waves of every period rows/k (k = 1…rows/2), energy = Σ|correlation| →
keep the minimum per frequency. Decision at phase 7: the frequency with the smaller residual energy wins (60 Hz if score60 <
score50) → h.0x350, h.0x354 = 2, h.0x358 = 0, pending API copy updated, forced update; the detector restarts at phase 1 with the
scores reset to 99 999. Row profile and its scratch: h.0x2dc0…0x2ebf, h.0x2ec0…0x2eff.

**Run-time detector `AE_FlickerFrequencyDetect_RunTimeMode`** (API 0x1413 FlickerEx; called from `DoAe` while FlickerEx is
enabled) [R user disassembly 0xf898–0x100c0, the decompile hides branches; kernel decompile 0x40056dc0, same flow]. State:

| h. | size | meaning |
|---|---|---|
| 0x35c | u32 | FlickerEx ENABLE (live; pending 0x4dc4) |
| 0x360 | u32 | OP_TYPE 0 auto / 1 manual (pending 0x4dc8) |
| 0x364 | u8 | **100 − AMPSENSITIVITY** (pending 0x4dcc holds AMPSENSITIVITY itself; `AE_ProcessAPI` converts) |
| 0x365 / 0x366 / 0x367 | u8 | SCORETHD / REFRESHCYCLES / VALIDTIMESTHD (pending 0x4dcd / 0x4dce / 0x4dcf) |
| 0x368 | u32 | FLICKER_TYPE 0 = 60 Hz, 1 = 50 Hz (pending 0x4dd0) |
| 0x36c | u8 | detector debug-print switch (only gates `CamOsPrintf`) |
| 0x36d | u8 | armed (1) / settling (0) |
| 0x36e | u8 | "score valid" — exported as IS_EFFECTIVE |
| 0x370 | u16 | score 0…100 (100 = no banding found) — exported as SCORE |
| 0x374 | u32 | frame phase inside a cycle (settling: frames counted) |
| 0x37c | u32 | consecutive low-score results |
| 0x380 | u8 | k = cycle length in frames |
| 0x381 | u8 | cycles done in the current scoring round |
| 0x384…0x3a0 | u32 × 8 | per-strip maximum of the frame energy within a cycle |
| 0x3a4…0x3c0 | u32 × 8 | per-strip minimum (reset to 0x10000) |
| 0x3c4 | u16 | hits in the current round |
| 0x3c6 | u8 | fps the k was computed for |
| 0x2f00 + 0x80·s + 4·r | u32 [8][32] | current strip-row sums (strip s, block row r) |
| 0x3300 + 0x80·s + 4·r | u32 [8][32] | reference strip-row sums |

API 0x1414 `AE_FlickerInfo` (12 B, `DrvAlgo_IF_AE_GetFlickerInfo`) = {u32 IS_EFFECTIVE = h.0x36e, u32 FLICKER_BANDING_TYPE = h.0x368, u16 SCORE = h.0x370}.
`pAE_SetFlickerModeEX` (getter `pAE_GetFlickerModeEX`, reads the pending copy) clamps: ENABLE and OP_TYPE to ≤ 1, AMPSENSITIVITY and SCORETHD to 1…100, REFRESHCYCLES and VALIDTIMESTHD
to 1…10, FLICKER_TYPE → (value ≠ 0); sets the pending flag h.0x47c8. `AE_ProcessAPI`, when ENABLE = 1: de-banding mode h.0x354 =
2, frequency h.0x350 = (FLICKER_TYPE ≠ 1) (1 = 60 Hz), h.0x36d = 0 (re-settle); when ENABLE = 0 the plain 0x140e values
(h.0x4dbc / 0x4dc0) are used instead.

Per frame:
1. AE not stable (h.0xb90 ≠ 1): h.0x36d = 0, h.0x36e = 0, phase = 0, return. The detector only looks at frames of a converged AE.
2. fps = `AeInput_t`+0x54 (taken raw here, not ×1000-normalised). If fps ≠ h.0x3c6: k = the smallest k in 1…30 with (F·k) mod
   fps = 0, F = 100 when the current frequency is 60 Hz (h.0x350 = 1) and 120 when it is 50 Hz; 30 if none; store k (h.0x380) and
   fps (u8, h.0x3c6); disarm (h.0x36d = h.0x36e = 0, phase 0). F is the light-flicker rate of the **other** mains frequency (50 Hz
   mains → 100 Hz light, 60 Hz → 120 Hz), so k frames is the period after which that frequency's banding returns to the same rows
   [I from F and the switch in step 8].
3. k = 1: the other frequency's banding cannot move between frames → h.0x36e = 0, return.
4. Strip sums: for every block row r < AvgBlkY (in+0x14) and column c < AvgBlkX (in+0x10): cur[c/4][r] = Σ of the 4 block Y values
   of that strip (byte 3 of the {R, G, B, Y} block, in+0x18). Then the frame energy of each of the **8** strips: **E[s] = Σ_r
   max(|cur[s][r] − ref[s][r]| − 2, 0)** (only 8 strips are scored; AvgBlkX > 32 or AvgBlkY > 32 would write past the 32 × 8 arrays
   [R index math; I consequence]).
5. Settling (h.0x36d = 0): phase += 1; when it exceeds 2 (three stable frames) arm: h.0x36d = 1, h.0x36e = 0, phase = cycles =
   hits = low-score count = 0. Return.
6. Armed, phase = 0: the reference frame — ref ← cur (AvgBlkY words per strip), max[s] = 0, min[s] = 0x10000, phase = 1, return.
7. Armed, phase mod k ≠ 0 (an in-between frame): max[s] = max(max[s], E[s]), min[s] = min(min[s], E[s]) for all 8 strips —
   skipped at phase 1 when k ≥ 3; phase += 1, return.
8. Armed, phase mod k = 0 (k frames after the reference, where the other frequency's banding is back in place): cycles += 1; for
   each strip s: **hit** if E[s] < min[s] and max[s] − E[s] > (100 − AMPSENSITIVITY) — the strip is calmer than every in-between
   frame by more than the sensitivity margin. hits += hit count.
   - cycles < REFRESHCYCLES: phase = 0 (the next frame is a new reference), return.
   - cycles = REFRESHCYCLES: h.0x36e = 1; **score = 100 − ((100·hits) >> 3) / cycles** (signed division; hits per strip-test in %,
     subtracted from 100) → h.0x370. OP_TYPE = 0 (auto): score > SCORETHD → low-score count = 0; else low-score count += 1, and when
     it reaches VALIDTIMESTHD the frequency is switched: h.0x350 = 1 if FLICKER_TYPE was 1 (50 Hz) else 0, FLICKER_TYPE h.0x368 and
     its pending copy h.0x4dd0 = the other type, h.0x36d = h.0x36e = 0, low-score count = 0, h.0x3c6 = 0 (k recomputed next frame),
     h.0x370 = 100, forced update h.0x190 = 1 (the new de-banding applies on the next exposure). OP_TYPE = 1 (manual): the score is
     reported only. In both: hits = cycles = phase = 0, return.

So one scoring round lasts REFRESHCYCLES × (k + 1) frames; a switch needs VALIDTIMESTHD consecutive rounds whose share of hits is at
least 100 − SCORETHD % [I from the formula]. The detector never turns de-banding off; it only moves between 50 and 60 Hz, and only
in OP_TYPE auto (the built-in default OP_TYPE is manual) [R]. Kernel and user copies run the same algorithm on the same fields (same
early returns, k search, strip sums, energy with the −2 floor, settle / arm, hit test and switch; the user build unrolls the 8-strip
loops). Unit of the fps input: libcus3a passes the 0x2E05 "current sensor FPS" (`CusAEInfo_t`+0x3c), whose unit is not known; if it
reaches `AeInput_t`+0x54 as fps×1000 rather than fps, step 2 re-runs every frame (the u8 h.0x3c6 never equals it) and keeps the
detector disarmed [I]. FlickerEx is off by default.

### 2.13 HDR short frame [R]
Used only when `AeInput_t`+0x1c (short-frame statistics) is non-NULL, with the HDR mode in +0x4c; this camera's sensor runs
linear, so the whole path is idle on it [S]. It consists of a second metering grid (centre 40 %, §2.3), short APEX
records — next h.0xb54 {TV, SSV, DSV, AV, BV}, manual h.0xb7c…0xb88 (0x1412) — the plain record P_s h.0xaa8 (same word layout as P),
applied E_s h.0xad0, and the short exposure table (0x140b, §2.8). Output +0x44…+0x83 = the 16 BV-bucket words for the IQ engine.
- **`pAE_CalPvExpoShort`** (0x3c), called from `pAE_ProcessAE` after the long frame: r = `Interp_LUT`(AEHdrRatio LUT h.0xbc8,
  long total gain h.0xa48) = long / short exposure ratio ×1024 (API 0x1411; the LUT's X axis is used as total gain although the XML
  range is a BV range); BV_s = BV_long_next (h.0xb50) + log2(⌊r/1024⌋)·16384 (user: ratio below 2 → +0; kernel: table T with
  ⌊r/1024⌋ clamped to 0…2560); E_s = E_long_next (h.0xa68)·1024 / r; `M[0x48] pAE_CalIntpApexShort(h, BV_s, E_s)`. Then by exposure
  mode h.0x184 (user jump table 0xdfa2: 0 → 0xe090, 1 → 0xe090, 2 → 0xe0e0, 3 → 0xe128, 4 → 0xdfac):

  | mode | short-frame steps |
  |---|---|
  | 0 Auto, 1 AV | BV_s recomputed from the interpolated record; in HDR mode 2 (`AeInput_t`+0x4c = 2, shared sensor gain) `AE_CalApexSVforHDRShort`; then `pAE_LimitMinMax(N_s, P_s)` and, when the strict flag h.0x358 = 0, `pAE_DoDeBanding(N_s, P_s)` |
  | 2 SV | `AE_CalApexSVforHDRShort` → APEX→plain → `pAE_LimitMinMax` → de-banding when h.0x358 = 0 |
  | 3 TV | `AE_CalApexTVforHDRShort` → APEX→plain → `pAE_LimitMinMax` |
  | 4 Manual | N_s = manual short record (in HDR mode 2 SSV / DSV taken from the long next record), BV recomputed → APEX→plain → `pAE_LimitMinMax` |

  So the short frame is de-banded in modes 0–2 only while h.0x358 = 0, whereas the long frame's Auto / AV / SV path de-bands
  unconditionally (the routine itself checks the flicker mode). In HDR mode 2 (unless manual) the result is short SG / DG = the
  long ones and short shutter = long shutter·1024 / r (the E_s step plus `AE_CalApexSVforHDRShort`).
- **`pAE_CalApexTblIdxShort`** (0x50): as the long version on the short table (BV column h.0x878): pair h.0xa04 (brighter) /
  h.0xa05 (darker), BV' kept at h.0xa08. **`pAE_CalIntpApexShort`** (0x48): TV, SSV, DSV, AV interpolated linearly in BV between
  the two entries → h.0xb54…0xb60, BV h.0xb64; with debug bit 4 the linear-E variant (§2.8). **`pAE_GetExpTblMaxIdxShort`**
  (0x7c): u8 count h.0x864, minus h.0x3774 unless the day/night mode word h.0x3770 is 4 or 7 (as the long version).
- **`AE_CalApexSVforHDRShort(h, N_s, P_s)`**: HDR mode 2 — SSV_s, DSV_s = the long next SSV / DSV (h.0xb44 / 0xb48), P_s.SG = long
  P.SG (h.0xa38); other modes — manual short SSV h.0xb80 clamped to the short table's SSV range (written back), DSV_s = the **long**
  next DSV clamped to the short table's DSV range, but the SV change e is computed from the manual short DSV h.0xb84 (the two
  disagree when h.0xb84 ≠ that clamped value), P_s.SG = h.0x511c (the pending manual short SG). P_s.DG = long DG h.0xa40. TV: TV_s +
  e below the short table's last TV → TV_s = that, P_s.shutter = last entry's shutter; within → TV_s + e and P_s.shutter =
  shutter·SG·DG / max(SG'·DG', 1024) capped at the first entry's shutter (h.0x578); above → first entry's TV and shutter. BV
  recomputed.
- **`AE_CalApexTVforHDRShort(h, N_s)`**: manual short TV h.0xb7c clamped to the short table's TV range (written back); δ limited so
  that TV + AV stays within the short table's (AV + TV) range; TV_s += δ; the SV sum moves by δ: outside the short table's sum range
  SSV is set to the first / last entry's SSV (DSV left), inside SSV_s += δ clamped to the SSV range (no spill into DSV); in HDR mode
  2 SSV / DSV = the long next values clamped to the short ranges; BV recomputed. The last-entry index here is taken from the
  **long** table's count (`M[0x78]`, not `M[0x7c]`) [R], which reads the wrong row when the two tables differ in length [I].

### 2.14 RGB-IR AE [R]
Unused on this camera (the sensor is not RGB-IR and delivers no RGB-IR histogram [S]). State (`AE_InitParameters`,
`AE_ProcessAPI`): enable h.0x378c (u8, pending h.0x5d58; also copied to h.0x378d on API apply), compensation-frame flag h.0x378d,
phase counter h.0x378e, ratio R h.0x3790 (u16 ×10000 = 1.0, default 10000), latched ratio h.0x3792 (default 10000),
MinISPGainCompRatio h.0x3794 (API 0x1410 +6, default 2500), IR target h.0x3796 (default 1800, no API writes it), MaxYWithIR
h.0x3798 (API +4, default 2400), IR levels h.0x379a / 0x379c / 0x379e / 0x37a0, offset h.0x37a4 (default −208930). API 0x1410
`RGBIRAE` = {u32 enable 0…3 (only "≠ 0" is used), u16 MaxYWithIR 0…2550, u16 MinISPGainCompRatio 1…10000}.
- **`AE_FindIRHistInfo`**: Σ of the 256-bin IR histogram h.0x2bb0 → h.0x2dbc; highest non-empty bin → u8 h.0xc4d.
- **`pAE_CalLumY`** (IR part, enable ≠ 0): three `pAE_CalHistogram` requests: IR histogram, 256 bins of width 1, 1000 ‰ from the
  bright end → mean IR×10 → h.0x379c, and h.0x379a = |IR×10·10000 − h.0x37a4| / R; the same with 10 ‰ → mean of the brightest 1 %
  IR → h.0x379e; the visible 128-bin histogram, 10 ‰ → brightest 1 % Y×10, ·1024 / DG (h.0xa40) → h.0x37a0. Disabled: the three
  words are 0.
- **Frame cycle**: `pAE_SetExposure` (end) — when h.0x378d = 0, h.0x378e counts 0 → 1, and on the next call (odd) h.0x378d is set
  to 1; when h.0x378d = 1, h.0x3792 = R (latch), h.0x378d = h.0x378e = 0, and DSV is re-derived from the final gains.
  `pAE_CalPvExpo` with RGB-IR enabled: h.0x378d = 0 → ΔEV h.0x1850 = 0 and, without a forced update, next = current (no exposure
  change); h.0x378d = 1 → `AE_CalRGBIRTargetwithSmooth` replaces `pAE_CalSmoothExpo`. So the exposure moves on one frame in three
  [I from the counter sequence].
- **`AE_CalRGBIRTargetwithSmooth`** (IR-dependent target with smoothing): T = final target h.0x2b4 (≤ 2550), Y = CurY×10 (≥ 1); r_t
  = T·10000/Y; lim = max(MaxYWithIR·10000 / (h.0x379e + h.0x37a0), h.0x3796·10000 / h.0x379e) (divisors ≥ 1); L = min(lim, r_t·10000
  / h.0x3792); X = (L·10000 / r_t)·h.0x3792 / 10000; **R = clamp(X, MinISPGainCompRatio, 10000)** (X > 10000 or MinISPGainCompRatio
  > 10000 → 10000); IR-limited target T_IR = (R·r_t / h.0x3792)·Y / 10000 → h.0x314 and h.0x318; speed s = `Interp_LUT`(AE_CONV_RATIO
  h.0x2cc, Y) clamped 10…1024 → h.0x2c8; max step m = max(1, (s·max(stable-in, stable-out) + 512) >> 10); step = (s·(T_IR − Y) +
  512) >> 10, replaced by sign·min(|T_IR − Y|, m) when smaller than m; new Y = T_IR when h.0x374c = 1, else Y + step; R' =
  max(MinISPGainCompRatio, min(10000, newY·R / T_IR)) (1 when newY·R < T_IR) → h.0x3790; base = max(Y, 10) (0 → Y when direct); Y'
  = max(base + step, newY); ratio h.0x1854 = Y'·1024 / base, h.0xbc4 = newY, ΔEV h.0x1850 = T[Y'] − T[base] (indices clamped to
  2560).
- **`AE_CalNewDGainWithIR(h, DG)`** (from `pAE_SetExposure`, enable and h.0x378d set, mode ≠ Manual): DG' = DG·10000 / R; if DG' >
  ISP max h.0xaa0 or < user min DG h.0xa7c, DG' = that limit and R = DG·10000 / DG'. The target is lowered by R (less sensor
  exposure while IR inflates the luminance) and the ISP gain is raised by 1/R to restore the visible level [I from the two
  formulas].
- **`pAE_isConverged`** extra test: margin t = 100 (stable) or 50; IR ok = |h.0x379e − h.0x3796| ≤ t, or h.0x379e below the band
  with R ≥ 9500; visible+IR ok = |h.0x379e + h.0x37a0 − MaxYWithIR| ≤ t, or below with R ≥ 9500; if neither is ok and R differs from
  the latched ratio or from MinISPGainCompRatio, the AE is "not converged".

### 2.15 Day/night and stabilizer [R]
- **Day/night** (API 0x1418 DayNightDetection, 12 B, three words → pending h.0x4db0 / 0x4db4 / 0x4db8 → live h.0x47b8 enable /
  h.0x47bc BV threshold; the third word, default 600, is kept for the getter only, never applied or read): flag h.0x47c4 = (enable
  and BV < threshold), set in `pAE_GetBV`, readable via 0x1419 DayNightInfo word 0 (words 1–2 not written); no exposure effect
  inside the AE. No public header names the three words; the camera's XML gives the same-shaped struct {u32 Enable; s32
  D2N_BvThd; u32 N2D_VsbLtScoreThd} (§3.9) [S], and the AE's use of word 0 as enable and word 1 as a BV threshold fits Enable /
  D2N_BvThd [I]. The day/night mode word h.0x3770 and the shortening count h.0x3774 (`pAE_GetExpTblMaxIdx`) are written by no API id
  or IQ item (default 0).
- **Stabilizer** (API 0x1415 AEStabilizer {Enable, u16 DiffThd ≤ 20, u16 Percent ≤ 100} → h.0x37a8 / 0x37ac / 0x37ae): with Enable,
  `AE_MotionDetection` (§2.6) decides whether a luminance change is a real scene change (more than Percent % of the 1024 blocks
  changed by > DiffThd) and the AE holds while it is not (`pAE_isConverged` returns 1). Enabling it resets the motion state
  (h.0x37b0 = 2, h.0x37b4 = 0).

### 2.16 API ids → AE methods → fields [R setters, `AE_InitParameters` tail, `AE_ProcessAPI` (both copies)]
The id → method mapping and the payload sizes are isp-3-iq-api.md §4.2; this section gives what each method does to the object.
Most setters write a **pending-API shadow** h.0x47c8…0x5de8 and raise h.0x47c8; the next `DoAe` runs `AE_ProcessAPI`, which (only
when h.0x47c8 = 1) copies every block below to its live field, applies the listed side effects, clears h.0x47c8 and sets the
forced-update flag h.0x190 — so API changes take effect on the next frame, atomically. `AE_InitParameters` seeds the shadow from
the live values (after the IQ file), so an untouched shadow re-applies the same values.

| shadow | size | API id: field (setter) | → live | `AE_ProcessAPI` / notes |
|---|---|---|---|---|
| 0x47c8 | u32 | — | — | pending flag |
| 0x47cc | u32 | 0x1403 EVComp GRAD (`pAE_SetEVComp`: 0 → 1, clamped ≤ 20) | 0x2b8 | |
| 0x47d0 | s32 | 0x1403 EVCOMP | 0x2bc | |
| 0x47d4 / 0x47d6 | u16 | 0x1408 AEConverge CONV_THD_OUT / CONV_THD_IN (`pAE_SetConvergeThd`: 0 → 1, ≤ 255) | 0x2c6 / 0x2c4 | |
| 0x47d8 | LUT 0x84 | 0x1408 CONV_SPEED_X/Y (`pAE_SetConvergeSpeed`: num = 4, Y = CONV_SPEED_Y, X = CONV_SPEED_X; returns −1 without touching the shadow if X[3], Y[0] or Y[3] is 0 or any Y > 1024) | 0x2cc | |
| 0x485c | 0x128 | 0x140f ExposureStrategy (`pAE_SetAEStrategy`; rejected if UPPER_NUM or LOWER_NUM = 0) | 0x1908 | then BT strength / sensitivity h.0x1a18 → 0x1b58, DT h.0x1a20 → 0x1c88, A h.0x1a28 → 0x27d8 (u64 each) |
| 0x4984 | 0x42c | 0x1416 ExposureStrategyEx (§2.11) | 0x1c94 | the setter also writes h.0x20d0 / 0x20d2 immediately |
| 0x4db0 / 0x4db4 / 0x4db8 | u32 | 0x1418 DayNightDetection words 0 / 1 / 2 (`pAE_SetDayNightDetectionParam`, also sets h.0x190) | 0x47b8 / 0x47bc / — | word 2 (default 600) is kept for the getter only |
| 0x4dbc | u32 | 0x140e frequency (`pAE_SetFlicker`: 1 = 60 Hz) | 0x350 | used when FlickerEx ENABLE ≠ 1 |
| 0x4dc0 | u32 | 0x140e mode (`pAE_SetFlickerMode`: 0 auto, 1 disable, ≥ 2 → 2) | 0x354 | used when FlickerEx ENABLE ≠ 1; mode "auto" re-arms the start-up detector |
| 0x4dc4…0x4dd3 | 4 × u32 | 0x1413 FlickerEx (`pAE_SetFlickerModeEX`, clamps §2.12) | 0x35c…0x368 | h.0x364 byte = 100 − AMPSENSITIVITY; ENABLE = 1 → h.0x354 = 2, h.0x350 = (TYPE ≠ 1), h.0x36d = 0 |
| 0x4dd4…0x4dd7 | — | — | — | not used |
| 0x4dd8 | 0x198 | 0x140a plain exposure table (**immediate** `pAE_SetPlainExpTbl` keeps this copy for `pAE_GetPlainExpTbl`) | — | not applied by `AE_ProcessAPI` |
| 0x4f70 | 0x198 | 0x140b short table (immediate, getter copy) | — | same |
| 0x5108 / 0x510c / 0x5110 / 0x5114 | u16 / u32 ×3 | 0x1405 ManualExposure FNx10 / SENSOR_GAIN / ISP_GAIN / US (`pAE_SetManualExpo`; rejected with no change unless SG and US lie within the sensor limits [min, max]) | — | plain copy for the getter; the setter converts to APEX below |
| 0x5118 / 0x511c / 0x5120 / 0x5124 | u16 / u32 ×3 | 0x1412 ManualShortExposure (`pAE_SetManualExpoShort`), same layout | — | P_s.SG source of `AE_CalApexSVforHDRShort` (§2.13) |
| 0x5128…0x5138 | APEX ×5 | 0x1405 → {TV, SSV, DSV = SV(SG·DG) − SSV, AV, BV} | 0xb68…0xb78 | the manual record used in mode 4 |
| 0x513c…0x514c | APEX ×5 | 0x1412 → same | 0xb7c…0xb8c | |
| 0x5150 | u32 | 0x140c / 0x140d WEIGHTING_ID (`pAE_SetAEWinWgtType`: 0 average, 1 centre, values > 1 → 2 spot) | 0x27ec | |
| 0x5154 / 0x5554 / 0x5954 | 0x400 each | 0x140d AVERAGE / CENTER / SPOT tables (`pAE_SetAEWinWgtTbl`, 3 × 32×32 u8) | 0xc4e / 0x104e / 0x144e | |
| 0x5d54 | u32 | 0x1404 ExposureMode (`pAE_SetExpMode`: 0 Auto … 4 Manual, also sets h.0x190) | 0x184 | |
| 0x5d58 | u32 | 0x1410 RGBIRAE enable (`pAE_SetRGBIRAE`) | byte → 0x378c and 0x378d | enable ≠ 0 → h.0x378e = 0 |
| 0x5d5c / 0x5d5e | u16 | 0x1410 MaxYWithIR / MinISPGainCompRatio | 0x3798 / 0x3794 | |
| 0x5d60 | 0x84 | — | — | copy of the HDR-ratio LUT h.0xbc8 made by `AE_InitParameters`; never read (0x1411 is immediate on h.0xbc8 and its getter reads h.0xbc8) [R grep] |
| 0x5de4 | u32 | 0x1415 AEStabilizer Enable (`pAE_SetStabilizerParam`, → 0 / 1) | 0x37a8 | Enable = 1 → motion state h.0x37b0 = 2, h.0x37b4 = 0 |
| 0x5de8 / 0x5dea | u16 | 0x1415 DiffThd (≤ 20) / Percent (≤ 100) | 0x37ac / 0x37ae (one u32 copy) | |

Immediate setters (no shadow):
- 0x1406 AEState: `pAE_Pause` → state h.0x180 = 3 (frozen: no `pAE_ProcessAE`, the output repeats the last exposure);
  `pAE_Resume` → 2 (re-init), or 0 from state 4.
- 0x1407 AETarget: `pAE_SetTarget(lut)` copies the 0x84-byte target LUT (target vs BV) to h.0x19c, forced update.
- 0x1409 ExposureLimit: `pAE_SetMinMaxShutterUS`, `pAE_SetMinMaxAperture` (§2.8, the inverted aperture words), `pAE_SetMinGains`,
  `pAE_SetMaxGains` write the user limits P words 0x10…0x15 directly (min shutter and the two min gains stored as **u16** — values
  above 65 535 wrap; aperture clamped to the table's FN range; gains ≥ 1024 and ≤ the sensor / ISP maxima); forced update. The
  handler's own clamps before the calls: isp-3-iq-api.md §4.2.
- 0x140a / 0x140b: `pAE_SetPlainExpTbl` / `pAE_SetPlainShortExpTbl` rebuild the tables and overwrite the user limits (§2.8).
- 0x1411 AEHdrRatio: `pAE_SetAeHdrRatio` → LUT h.0xbc8.
- 0x1416: the h.0x20d0 / 0x20d2 part (§2.11).

Gets (`pAE_Get*`, `pAE_GetHist/LumY/AvgY`, `pAE_GetCurExpo/ShortExpo`, `pAE_GetCurBV/LV`, `pAE_isStable` / `isReachBoundary`) read
the shadow for the pending-style ids — so a Get right after a Set returns the new value before it is applied — and the live field
for the immediate ones [R getters sampled: 0x1416, 0x1413, 0x1418 shadow; 0x1407, 0x1411 live]. 0x1402 AEInfo assembles the stable
flag, boundary flag, current exposure, LV, BV and the histogram / weighted-Y block; 0x1414 and 0x1417 are read inline (§2.12,
§2.11); 0x1419 returns h.0x47c4 (§2.15).

### 2.17 Field dictionary and built-in defaults [R emulation; meaning R from the cited code unless marked]
<!-- ev:isp-5-3a.2.17 -->
How obtained: libispalgo's own `AE_InitFuncPtr` + `AE_InitParameters` run under unicorn on a zeroed 0x5E00-byte object with
`AE_GetParamFromCfg` skipped (so: the built-in defaults, before any IQ-file item), `log2` / `pow` computed on the host, `memcpy` /
`memset` performed, every store into the object recorded (1421 stores — offset, size, final value, program order; 1024 of them are the
motion-detection byte loop). The kernel `AE_InitParameters` (0x40057fa0) stores the same offsets and
constants (spot-checked: h.0x3778…0x37b4, h.0x47c0 = 600, the pending copies; it was not emulated itself). "IQ" names the
`AE_GetParamFromCfg` item that overwrites a field when the IQ file has it, "API" the id whose setter writes it (directly or through
the shadow, §2.16). Fields `AE_InitParameters` does not touch keep 0 from `AeInit`'s calloc; the exposure tables (h.0x3c8…0x9cf) and
the window grids come only from the IQ file / `AeInit` — with no IQ table the table count is 0 and `pAE_GetExpTblMaxIdx() − 1`
indexes row −1 [R index arithmetic; I consequence].

| h. | type | default | set by | meaning |
|---|---|---|---|---|
| 0x000–0x17c | 96 × fn* | method table | `AE_InitFuncPtr` | |
| 0x180 | u32 | 0 | state machine | 0 INIT, 1 RUN, 2 re-init, 3 paused, 4 (resume → 0) (§2.5) |
| 0x184 | u32 | 0 | API 0x1404 | exposure mode 0 Auto … 4 Manual |
| 0x188, 0x194 | u32 | 0 | — | written only by `AE_InitParameters` [R: no other direct access] |
| 0x18c | u32 | 0 | `DoAe` | frame counter |
| 0x190 | u32 | 0 | setters | forced-update flag |
| 0x198 | u16 | 0 | `AeInput_t` debug level | debug mask; bit 4 selects the linear-E interpolation (§2.8) |
| 0x19c | LUT 0x84 | 5 points: Y = 400 ×5, X = −98702, −85810, −58278, −20248, 8333 (BV) | IQ 0x016–0x018, API 0x1407 (immediate) | BV target vs BV (the built-in target is flat 40.0) |
| 0x220 / 0x224 / 0x228 / 0x22c | u32 | 470 / 470 / 470 / 470 | IQ 0x015 (0x220) | BV target / smoothed target / upper bound / lower bound (Y×10) |
| 0x230 | LUT 0x84 | 13 points: Y = 1, 3, 6, 12, 25, 50, 100, 135, 160, 200, 232, 265, 300 %; X = −6…+6 EV | — | EV-compensation ratio (§2.7 step 3) |
| 0x2b4 | u32 | 470 | per frame | final target after EV compensation |
| 0x2b8 / 0x2bc | u32 / s32 | 3 / 0 | IQ 0x024 / 0x025, API 0x1403 | EV grad / EV comp |
| 0x2c0 | u32 | 100 | per frame | EV ratio % |
| 0x2c4 / 0x2c6 | u16 | 10 / 20 | IQ 0x029 / 0x02a, API 0x1408 | stable-in / stable-out thresholds (Y×10) |
| 0x2c8 | u16 | 80 | per frame | current convergence speed |
| 0x2cc | LUT 0x84 | 4 points: Y = 200, 120, 120, 200; X = 100, 470, 470, 2000 (Y×10) | IQ 0x026–0x028, API 0x1408 | convergence speed vs current Y |
| 0x314 / 0x318 | u32 | 0 | RGB-IR | IR-limited target (§2.14) |
| 0x350 / 0x354 / 0x358 | u32 | 1 / 2 / 0 | IQ 0x023 / 0x022, API 0x140e / 0x1413 | flicker frequency (1 = 60 Hz) / mode (2 = de-band on; the cfg path then forces 1, §2.12) / strict flag |
| 0x35c…0x368 | FlickerEx | 0 / 1 / {30, 50, 3, 2} / 0 | API 0x1413 | enable 0, OP_TYPE manual, {100 − AMP = 30, SCORETHD 50, REFRESHCYCLES 3, VALIDTIMESTHD 2}, type 60 Hz (§2.12) |
| 0x36c…0x3c6 | detector state | 0; score h.0x370 = 50; k h.0x380 = 5 | — | §2.12 |
| 0x3c8…0x55f | plain table 0x198 | — (IQ) | IQ 0x02d / 0x02e, API 0x140a | long exposure table as loaded (§2.8) |
| 0x560…0x6f7 | plain table 0x198 | — (IQ) | IQ 0x02f / 0x030, API 0x140b | short exposure table |
| 0x6f8…0x863 | APEX table | — | `AE_ConversionExpoTbl` | long: count + 18 slots × 0x14 (§2.8) |
| 0x864…0x9cf | APEX table | — | same | short |
| 0x9d0 / 0x9d1, 0x9d4 | u8 ×2, s32 | 0 | per frame | long table segment (brighter, darker), BV' |
| 0xa04 / 0xa05, 0xa08 | u8 ×2, s32 | 0 | per frame | short table segment, BV'_s |
| 0xa38…0xaa0 | P record | SG 1024, DG 1024, shutter 250000 µs, FN 16; word 1 (0xa3c) = 81920 and word 7 (0xa54) = 32768 after `AE_InitBV` (it stores the SSV and TV of the initial exposure there); quantisation ratio 0xa70 = 1024; user limits FN 16 / 16, min DG 1024, min SG 1024, min shutter 1, max DG 0x3FFF, max SG 0x100000, max shutter 250000; sensor min shutter 1, min gain 1024, max shutter 1e9 µs, max gain 0x4E2000; shutter step 0; ISP max 0x3FFFF | `AeInit` limits, table setter, API 0x1409 | §2.7 table; FN words §2.8 |
| 0xa60 / 0xa68 | u64 | 0 | per frame | applied / next linear exposure E |
| 0xaa8…0xb10 | short P record | as the long one (shutter 250000, ISP max 0x3FFFF at 0xb10) | | HDR short frame |
| 0xb18 / 0xb2c / 0xb40 / 0xb54 | APEX ×5 | prev {TV 113163 (8333 µs), SSV 81920 (1×), DSV 0, AV 22219 (F1.6), BV 81920}; cur (from `AE_InitBV` on P) {TV 32768 (250 000 µs), SSV 81920, DSV 0, AV 22219, BV −26933}; next / short next 0 | `AE_Plain2Apex`, `AE_InitBV` | prev / cur / next / short next (§2.7) |
| 0xb68…0xb78 / 0xb7c…0xb8c | APEX ×5 | {TV 113163, SSV 81920, DSV 0, AV 22219, BV 81920} / short {TV 167598, SSV 81920, DSV 0, AV 22219, BV 81920} | API 0x1405 / 0x1412 via pending | manual long / short exposure (`AE_Plain2Apex` of FNx10 16, ISO 100, 8333 µs / 833 µs, LV 10) |
| 0xb90…0xb98 | u8 / u32 | 0 | per frame | IsStable, (0xb91 internal), IsReachBoundary 0xb92, set index 0xb93 (always 0), high-BV frame count 0xb94 |
| 0xb9c / 0xba0 | s32 | BV 81920 / LV 0 | per frame | BV, LV×10 |
| 0xba4…0xbc0 | u32 | 0 | `pAE_CalLumY` | CurY / AvgY of the long and short sets (§2.6) |
| 0xbc4 | u32 | 470 | convergence | smoothed next Y |
| 0xbc8 | LUT 0x84 | 5 points: Y = 10240 ×5 (10×), X = 1024, 2048, 4096, 8192, 16384 | API 0x1411 (immediate) | HDR long / short ratio vs total gain |
| 0xc0c | u32 ×16 | — | per frame | `AeHdr_Num_BV` (output +0x44) |
| 0xc4c / 0xc4d | u8 | 0 | per frame | highest luma bin ×2+1 / highest IR bin |
| 0xc4e…0x184d | u8 [3][32][32] | 0 | IQ 0x031, API 0x140d via pending | metering weight tables (average / centre / spot) |
| 0x1850 / 0x1854 | s32 / u32 | 0 / 1024 | per frame | ΔEV / exposure ratio |
| 0x1858 / 0x185c / 0x1860 | u32 | 25000 / 1024 / 16 | IQ 0x019–0x01b | initial shutter / gain / FN (manual and start-up paths) |
| 0x1864 | u32 | 0 | per frame | linearity error |
| 0x1868 | LUT 0x84 | 4 points: Y = 1024 ×4, X = 0, 1024, 2048, 4096 | — | target smoothing weight vs linearity error |
| 0x18ec…0x1904 | u32 ×7 | 0, 470, 9, 470, 6, 470, 8 | — | time buffer: 0x18f0 converged level, 0x18f4 frame count, 0x18f8 / 0x1900 bright / dark thresholds, 0x18fc / 0x1904 bright / dark frame counts (§2.7 step 4); 0x18ec has one more reader at 0x40050772, not opened |
| 0x1908…0x1a2f | strategy block 0x128 | all 0 (mode 0 = bright-tone priority, LUT counts 0) | IQ 0x043–0x04a, API 0x140f | the 0x140f layout: +0 MODE, +4 WEIGHT, +8 UPPER LUT, +0x8c LOWER LUT, +0x110…+0x124 BT / DT / A strength + sensitivity |
| 0x1a30…0x1a38 / 0x1b60…0x1b68 | u32 ×3 | {0,0,0} / {0,1,0} | — | histogram request {mode, direction, exclusion} of the bright-tone / dark-tone strategy |
| 0x1a3c…0x1b44, 0x1b48…0x1b54, 0x1b58 / 0x1b5c | LUTs, u32 | 0 | IQ 0x04c–0x051 | bright-tone percent / target LUTs, outputs, live strength / sensitivity |
| 0x1b6c…0x1c74, 0x1c78…0x1c84, 0x1c88 / 0x1c8c | LUTs, u32 | 0 | IQ 0x071–0x076 | dark-tone equivalents |
| 0x1c90 | u32 | 0 | per frame | strategy target |
| 0x1c94…0x20bf | Strategy-EX 0x42c | ENABLE 0, MODE 1, PRIORITY 0; every LUT 1 point at BV 0: BT ThdY 140, BT Percent 70 ‰, BT Target 1600, BT MaxOffsetDown 0, DT ThdY 5, DT Percent 50 ‰, DT Target 10, DT MaxOffsetUp 0 | API 0x1416 | §2.11 |
| 0x20c0…0x20da | u16 | ratio 512, others 0, history 512 ×4 | per frame | §2.11 |
| 0x20dc / 0x2120 | LUT | 2 points: Y = 100, 120 ‰; X = 0, 4 EV | IQ 0x034–0x036 | dynamic range: bright-end percent vs BV |
| 0x2160 | u32 | 384 | IQ 0x03a | DR bright sort-Y weight |
| 0x2164…0x216c, 0x21f8…0x2204 | u32 | 0 | per frame | DR components and DR×16384 |
| 0x2170 | LUT | 3 points: Y = 250, 200, 150 ‰; X = −5, −1, +4 EV | IQ 0x037–0x039 | dark-end percent vs BV |
| 0x21f4 | u32 | 512 | IQ 0x03b | DR dark sort-Y weight |
| 0x2208 | u32 | 0 | IQ 0x04b | AE_ENABLE_STRETCH |
| 0x220c…0x2214, 0x2450…0x2458, 0x2578…0x2580, 0x26a0…0x26a8 | u32 ×3 each | {0,0,0}, {0,0,0}, {0,0,1}, {0,1,0} | — | tone-balance histogram requests: first pass, bright, highlight limit (exclusion on), mid tone (from the dark end, exclusion off) |
| 0x2218…0x243c | LUTs | 0 | IQ 0x055–0x061 | tone-balance bright LUTs and weights |
| 0x245c | LUT | 3 points: Y = 500 ‰ ×3; X = −2, +4, +7 EV | — | highlight-limit percent vs BV |
| 0x24e0 | LUT | 3 points: Y = 1440, 1500, 1550; X = −2, +4, +8 EV | — | highlight-limit target vs BV |
| 0x2574 | u32 | 0 | per frame | highlight-limit target |
| 0x2584…0x269c, 0x26ac…0x27d4 | LUTs, u32 | 0 | IQ 0x067–0x06c, 0x07a–0x080 | mid-tone and dark tone-balance LUTs and outputs |
| 0x27d8 / 0x27dc / 0x27e0 | u32 | 0 | 0x140f via pending | auto strength / sensitivity / tone-balance target |
| 0x27ec | u32 | 0 | IQ 0x021, API 0x140c | metering type |
| 0x27f0 / 0x2850 | grid 0x60 each | {16, 16, …} / {1, 1, …} | IQ 0x02b / 0x02c, `AeInit` | metering grids, long / short (rebuilt by `AeInit` and `DoAe`) |
| 0x28b0…0x2baf | u16 / u32 | 0 | per frame | sorted cell values, histogram copy h.0x29b0 (128 × u16) |
| 0x2bb0…0x2daf | u16 ×256 | 0 | per frame | IR histogram |
| 0x2db0 / 0x2db4 / 0x2dbc | u32 | 0 | per frame | histogram bookkeeping / total count / IR total |
| 0x2dc0…0x2ebf, 0x2ec0…0x2eff | u32 | 0 | start-up flicker detector | row profile and its scratch (§2.12) |
| 0x2f00…0x36ff | u32 [8][32] ×2 | 0 | run-time flicker detector | §2.12 |
| 0x3700…0x3743 | request | — | per call | `pAE_CalHistogram` request block (§2.6) |
| 0x3744 | u32 | 0 | IQ 0x006 | AE_HIST_TYPE |
| 0x374c | u32 | 0 | — | "converge at once" flag (smoothing bypass in §2.7, §2.10, §2.14) |
| 0x3754 / 0x3758 / 0x375c | u32 | 0 / 30000 / 30000 | IQ 0x01e / 0x01c | shutter-compensation-into-SG flag / min fps ×1000 (overwritten per frame by the input fps) / DebandFPS ×1000 |
| 0x3764 / 0x3768 / 0x376c / 0x3770 | u32 | 0 / 1024 / 0 / 0 | — | de-banding state, gain ratio, (written only here), day/night mode word (table shortening unless 4 / 7) |
| 0x3774 | u32 | 0 | — | entries removed from the table end in day/night modes |
| 0x3778 / 0x377c / 0x3780 / 0x3784 | u32 | 1782 / 57 / 13102 / 0 | IQ 0x009 / 0x00a / 0x008 / 0x007 | ISO100 gain / min ISO / BV offset / AV offset (APEX calibration) |
| 0x378c…0x37a4 | RGB-IR | enable 0, R 10000, latched 10000, MinComp 2500, IR target 1800, MaxYWithIR 2400, IR words 0 / 2550 / 2550, offset −208930 | API 0x1410 | §2.14 |
| 0x37a8 / 0x37ac / 0x37ae | u32 / u16 / u16 | 0 / 2 / 25 | API 0x1415 | stabilizer enable / DiffThd / Percent |
| 0x37b0 / 0x37b4 / 0x37b6 | u32 / u16 / u16 | 2 / 1024 / 300 | — | motion state / block count / snapshot period in frames |
| 0x37b8…0x47b7 | u8 per block (stride 4) | 0 | motion detection | previous-frame block Y at +3 of each 4-byte slot (1024 blocks) |
| 0x47b8 / 0x47bc / 0x47c0 | u32 | 0 / −30000 / 600 | API 0x1418 via pending | day/night enable / BV threshold / third word (copied to the API shadow, never applied or read) |
| 0x47c4 | u32 | 0 | `pAE_GetBV` | day/night flag (API 0x1419 word 0) |
| 0x47c8…0x5de8 | shadow | seeded from the live fields | setters | §2.16 |
| 0x5df0…0x5dfc | ptr ×4 | — | `AeInit` | cfg, init param, in, out |

Fields with no direct reader found, listed above without a role: h.0x188, 0x194, 0x1b44, 0x1c74, 0x2320, 0x2564, 0x27b4, 0x27e4,
0x3750, 0x376c.

## 3. AWB — `AwbInit` / `DoAwb` / `AwbRelease`, `pAWB_*` [R unless marked]
<!-- ev:isp-5-3a.3 -->

Object 0x13FC bytes (`AwbInit` calloc); method table of 41 entries h.0x000…0x0a0 (all API setters / getters); the per-frame steps are direct calls. Gains are **R-gain = 1024·G/R, B-gain = 1024·G/B** (1024 =
1×), colour temperature in K. Kernel and user copies agree unless a difference is named (user disassembly addresses are file offsets).

### 3.1 Entry points and their structs
`void *AwbInit(void *cfg, const AwbInput_t *in, AwbOutput_t *out)`; `int DoAwb(void *h, const AwbInput_t *in, AwbOutput_t *out)`;
`int AwbRelease(void *h)` (free).

**`AwbInput_t` (0x48)**: +0x00 Size, +0x04 frame count, +0x08 BV×16384 (from the AE), +0x0c CurY / WeightY, +0x12 u8 HDR, +0x14 four
u16 (initial R / G / B gains: `AwbInit` takes +0x14 → h.0x578, +0x16 → h.0x57c **and** h.0x580 — the same input half-word read twice —
and +0x18 → h.0x584), +0x1c crop {x, y, w, h}, +0x2c u16 block count X / Y, +0x30 {u16, u16} sample size (normal light), +0x34 {u16,
u16} sample size (low light), +0x3c avgs (128 × 90 × {R, G, B}), +0x40 short-frame avgs, +0x44 → `AWBDEBUG_t {enable, mwb_enable,
mwb_r, mwb_b, level}` (field names after the M5 header [I]; `enable` gates the auto run, the debug level at +0x10 is copied each frame
to h.0x117c).

**`AwbOutput_t`**: +0x00…+0x13 crop and block settings proposed for the next frame (`pAWB_SetBlockValidSize`: the input crop /
blocks, the normal or low-light sample size depending on LV vs AWB_LOW_LUX_THD, a ratio byte 300/WeightY clamped 1…20) — libcus3a
ignores them; +0x22 u16 R gain, +0x24 u16 Gr, +0x26 u16 Gb, +0x28 u16 B gain, +0x2a u16 colour temperature, +0x2c u32 CCM-update
flag, +0x30…+0x41 9 × u16 CCM (row-major; default identity) — written only in MultiLS mode (§3.8); in the kernel path these are ispmid
ch+0x146…0x14e, ch+0x150 and ch+0x154 (the matrix the IQ engine's CCM is multiplied with), libcus3a does not forward the flag and
the CCM (isp-4-iq-engine.md §2.2).

**`AwbInit`**: keep cfg (h.0x13f8), in / out (h.0xa8 / 0xac), the initial gains (h.0x578…0x584), `pAWB_InitFuncPtr`,
`pAWB_InitParameters` (§3.12: `pAWB_GetParamFromCfg` first, then the built-in defaults, `pAWB_InitCaliParam(…, 1)` → light-area
polygons, and last `pAWB_InitGain`).

### 3.2 IQ-file items read (`pAWB_GetParamFromCfg`)
<!-- ev:isp-5-3a.3.2 -->
AWB_SPEED 0xe5 → h.0x59c (u8), AWB_CONV_THD 0xe6 → h.0x59d, AWB_CONV_OUT_THD 0xe7 → h.0x59e, AWB_COUNT_THD 0xe4 → h.0x1186 (min
blocks for a light area), 0xe0 / 0xe1 (unnamed) → h.0xb84 / 0xb86 (first / last light-area index used), AWB_LIGHT_WEIGHT 0xda →
h.0x5b2 (10 lights × 18 LV points, u16 %), AWB_PERFER_R_RATIO 0xdc → h.0x71a, AWB_PERFER_B_RATIO 0xdd → h.0x882 (same shape),
AWB_LUM_MAX 0xe2 → h.0x1182, AWB_LUM_MIN 0xe3 → h.0x1184, AWB_MIX_THIRD_THD 0xea → h.0x1189, AWB_DEFAULT_RATIO 0xdb → h.0x118a
(daylight-mix weight per light per LV), AWB_LOW_LUX_THD 0xeb → h.0x12f2, AWB_LOW_LUX_BLK_X / Y 0xec / 0xed → h.0x12f8 / 0x12fa;
AWB_LIGHT_CT 0xd8 (the golden light-area polygons → h.0xb88) and AWB_COLOR_TEMP 0xd9 (their colour temperatures) into temporaries
for the calibration. The same function then computes the high-precision areas
(§3.8), copies the golden areas into the active ones h.0xf28 and builds the light table h.0x1118 (§3.5).
Not read by the AWB: AWB_CALI_PREGAIN, AWB_LIGHT_EXTRA, AWB_PRESET_GAIN, AWB_PURECOLOR_CFG, AWB_STATISTICS_W/H, AWB_IMAGE_GAIN,
AWB_GAIN, AWB_SPC, the *_LIGHT status items, AWB_LVVALUE, AWB_MODE_CONTROL.

### 3.3 Coordinates, the weight window and the polygon test [R]
- A block's point is **x = (DivGTbl[G]·R) >> 8, y = (DivGTbl[G]·B) >> 8** with `DivGTbl[g]` = ⌊25600 / g⌋ (g = 0 → 25600; 256 u32
  entries, libispalgo `.data` 0x36fa4), i.e. x ≈ 100·R/G, y ≈ 100·B/G. A gain pair maps into the same space as x = 102 400 /
  R-gain, y = 102 400 / B-gain (`pAWB_RBgainToXY`; gains 1024 = 1×, so 102 400 / (1024·G/R) = 100·R/G).
- A block is used only if every channel average lies strictly between AWB_LUM_MIN (h.0x1184) and AWB_LUM_MAX (h.0x1182); the
  gray-world totals h.0xd4 / 0xd8 / 0xdc (ΣR, ΣG, ΣB) and the count h.0xc4 take every sampled block before that test.
- Block weight = WeightWin[9·MapY[row] + MapX[col]] (u32 at h.0xa00 + 4·idx, the 9 × 9-cell **weight window**, API WeightWin).
  `MapX_Tbl` (128 B, `.data` 0x373a4) maps the columns to cells 0:0–14, 1:15–28, 2:29–42, 3:43–56, 4:57–71, 5:72–85, 6:86–99,
  7:100–113, 8:114–127; `MapY_Tbl` (90 B, 0x37424) the rows to ten-row cells 0:0–9 … 8:80–89. In the kernel the two tables sit right
  after its own `DivGTbl` (+0x400 rows, +0x45a columns).
- **Polygon test** `pAWB_GetLightArea(h, areas, none, x, y, first, last)`: area i (8 s32 at areas + 32·i) is four points P0 = ([0],
  [1]), P1 = ([2], [3]), P2 = ([4], [5]), P3 = ([6], [7]); the left edge runs P0→P1, the right edge P2→P3, x on an edge at height y =
  Px + ((y − Py)·⌊(Qx − Px)·1024 / (Qy − Py)⌋) / 1024 (C division, toward zero). Inside ⇔ x_left ≤ x ≤ x_right and P1y ≤ y ≤ P0y; a
  vertical edge (Px = Qx) is the constant Px; an edge with Py = Qy is not tested at all. The first area of first…last that contains
  the point is returned, else `none` (user disassembly 0x177b4).

### 3.4 Per frame (`DoAwb`)
1. Frame counters h.0xb8 / 0xbc (the first `DoAwb` copies the frame count into h.0xb8 and clears the first-frame flag h.0xb4);
   **LV×10** = round(BV·10/16384) + 50 clamped 0…200 → h.0xb0; `pAWB_SetBlockValidSize` (output window).
2. State h.0x590 (API 0x1802 State / Mode): **0 auto** → if `AWBDEBUG.enable` = 1: `pAWB_GetStatisticsExt` + `pAWB_Process`; **1
   manual gains** (MWB attr R / B from h.0xb44…0xb4a, each < 0x2000) → applied once, state 4; **2 manual colour temperature**
   (`pAWB_ColorTempToGain(CT)`) → applied once, state 4; **4 hold**; **5** → back to 0.
3. **`pAWB_GetStatisticsExt`**: over the AWB block grid (128 × 90, sub-sampled by the steps h.0x117e / h.0x1180) accumulate the
   gray-world totals, apply the luminance test, compute the point (§3.3); then per usable block: if user extra areas are enabled
   (AdvMode h.0x5a4 = 1 and ExtraLtEnable h.0xb50 ≠ 0) the user areas are tested first (§3.6); otherwise, or with no user hit,
   `pAWB_GetLightArea` finds the first of the calibrated light areas (h.0x58c, indices first…last = h.0xb84…0xb86) containing (x, y);
   per area i accumulate count (+0xe0 + 0x1c·i), the sum of window weights W_i (+0xe4) and the weighted R, G, B sums; valid block count
   h.0x49c += 1; MultiLS high-precision counting when AlgMode = 3 (§3.8). The same is done for the HDR short frame (in+0x40) when
   in+0x12 = 1 (the gray-world count h.0xc4 is not incremented there). At the end the day/night score (§3.9). The per-frame
   accumulators h.0xc4…0x40b (incl. the per-area records h.0xe0 + 0x1c·i and the high-precision records h.0x1f8 + 0x1c·j) and
   h.0x40c…0x47b (user areas) are cleared every frame.
4. **`pAWB_Process`**: (AlgMode 3: `pAWB_CheckRefWhitePoint`, `pAWB_MultiLS_Check`, §3.8) → `pAWB_GetLightSource` →
   `pAWB_GetUserLight` (§3.6) → `pAWB_CalcGain` → `pAWB_GainStrength` → `pAWB_CheckGain` → `pAWB_ApplyGain`.
5. **`pAWB_GetLightSource`**: areas with count < AWB_COUNT_THD get weight 0; for the others c_i = ⌊w_i · W_i / 100⌋ with w_i =
   AWB_LIGHT_WEIGHT[i]\(LV) (`pAWB_GetLightWeight`, §3.7) and **W_i = the area's sum of window weights** (+0xe4), not its block count
   (with the default window W_i = 8 · count); w_i is also kept at h.0xec + 0x1c·i; c_i → h.0xe8 + 0x1c·i. In AlgMode 3 c_i is further
   multiplied by the dynamic CT weight (§3.8). The three largest non-zero c_i become L1, L2, L3 (h.0x4a0…0x4a2; ≥ 10 = none), the first
   index winning ties (strictly greater replaces) [R both copies].
6. **`pAWB_CalcGain`**: gray-world gains always computed (h.0x13dc / 0x13e0, rounding (sum + n/2)). With a calibrated light found (L1
   < 10): AlgMode **0** → gray world (user areas play no part); **1** (normal) → `pAWB_GetLightGain_Normal`: gains of L1 from its
   weighted sums; with L2 → count-weighted mean of L1 and L2; with L3 adjacent to L1 on the other side of L2 (L3 = L1 ± 1, L2 = L1 ∓ 1)
   and L3's share > AWB_MIX_THIRD_THD % → three-way mean; **2** → `pAWB_GetLightGain_Balance` (§3.5); then (AlgMode ≠ 0)
   `pAWB_MixDaylightGain` and `pAWB_GetPreferenceGain` (§3.7), then `pAWB_MixUserGain` if a user light was chosen (h.0x4a3 < 4), else
   h.0x54c = h.0x550 := 1024 (§3.6). **No calibrated light** (L1 ≥ 10) → the previous gains h.0x4ac / 0x4b0, replaced by
   `pAWB_GetUserGain` when a user light was chosen (§3.6). Colour temperature `pAWB_CalcColorTempByWeight` = Σ c_i·CT_i / Σ c_i over
   L1…L3 (CT_i from the light table h.0x1118 + 10·i; the previous CT if all counts are 0). Gains clamped to the CT limit box (API
   CtLimit → R in [h.0x5ac, h.0x5aa], B in [h.0x5b0, h.0x5ae]); 8-entry gain history (16-byte records at h.0x4a4) shifted. Then
   convergence: if not converged — `pAWB_isConverged`: |R − R_target| and |B − B_target| ≤ AWB_CONV_THD → converged; once converged it
   stays so while both stay ≤ AWB_CONV_OUT_THD; with the AWB stabilizer (0x1809, enable h.0x13c8) it also stays converged while the
   gray-world gains move ≤ h.0x13cc and the gains ≤ h.0x13d0, for at most h.0x13ce frames — or forced (h.0x588): jump to the target
   when the frame gap h.0xbc − h.0xb8 is below h.0x1188, else **smooth**: new = (old·(100 − speed) + target·speed)/100, at least one
   LSB toward the target (`pAWB_GetSmoothBySpeed`, speed = AWB_SPEED / API ConvergeSpeed 1…100), for the gains and the CT. If
   converged: three more 50 % steps, then hold (h.0x575 = 4); the gray-world gains are latched at h.0x13d4 / 0x13d8 when convergence
   completes (stabilizer reference). Result h.0x47c (R), h.0x488 (B), CT h.0xc0.
7. **`pAWB_GainStrength`**: R ×= RStrength/128, B ×= BStrength/128 (API GainStrength bytes h.0x5a8 / 0x5a9, 128 = neutral) → h.0x48c /
   h.0x498. **`pAWB_CheckGain`**: normalise so the smallest of R, G, B gains is 1024 (if R or B < 1024, G = 1024·1024/min and the others
   scaled), each clamped 1024…0x1FFF. **`pAWB_ApplyGain`**: write R, Gr = Gb = G, B and CT to the output and the last applied gains to
   h.0x13e8…0x13f4.
8. libcus3a copies `AwbOutput_t` into `CusAWBResult_t` (R, G, B, CT; isp-6-userspace.md §2.7) — the kernel writes R → R, G → Gr and
   Gb.

### 3.5 Light table, light areas and unit calibration
- **Light areas**: 10 light sources, each an area polygon of 8 s32 (§3.3, in x ≈ 100·R/G, y ≈ 100·B/G space). The IQ item
  AWB_LIGHT_CT gives the golden areas (h.0xb88, 10 × 32 B); the active areas are h.0xf28 (pointer h.0x58c).
- **Light table** [R]: record i at h.0x1118 + 10·i = {u16 CT, u16 R-gain (+2), u16 0, u16 0, u16 B-gain (+8)}, built by
  `pAWB_GetParamFromCfg` from the golden polygon i (h.0xb88 + 32·i) as R_i = 204 800 / (P0x + P3x), B_i = 204 800 / (P0y + P1y) (the
  mean x of the top edge and the mean y of the left edge, mapped back to gains with 102 400 / coordinate), CT_i from AWB_COLOR_TEMP;
  the two middle words are never written. `pAWB_ColorTempToGain(CT)` interpolates R / B gains between the two entries around CT
  (entries sorted by descending CT; out of range → −1 and gains 1). Entry 3 is the daylight gain (§3.7), entry 4 the start white
  point (§3.12).
- **`pAWB_GetLightGain_Balance`** (AlgMode 2): R / G / B means of every area in [h.0xb84, h.0xb86] weighted by the area's weight sum
  → one "balanced" gain pair.
- **Unit calibration** (`pAWB_InitCaliParam`; enable h.0x12fc, mode h.0x1300 = 1 one-point / 2 two-point, golden and unit
  white-point gains h.0x1304…0x1320, set through 0x240F `CALIv30_AWB_CaliApply` → `pAWB_ApplyUnitCalibration` (isp-3-iq-api.md
  §4.5) or 0x2C03 golden light area (`pAWB_SetGoldenLightArea`)): gains converted to xy (102 400 / gain), every area polygon
  shifted (one-point) or shifted and scaled (two-point, scale = unit span / golden span ×100) by `pAWB_CaliLightArea` →
  `pAWB_GetAreaOffset`; if any area degenerates (coordinate < 2) the golden areas are kept. Then the light-table gains are rebuilt
  and `pAWB_UpdateLightAeraHighPrecision` derives the high-precision areas (§3.8). At init `pAWB_InitParameters` calls
  `pAWB_InitCaliParam` with 1 as the second argument.

### 3.6 User extra light areas (API 0x1803 WBAttrEx, AdvMode = 1) [R]
**API block** (52 B, XML): +0x00 u32 ExtraLtEnable; four `sLtInfo[k]` of 12 B at +0x04 + 12·k: u16 WRgain (256…4095), u16 WBgain,
u8 AreaSize (1…32; the XML gives LT2 the range 1…255), 3 pad bytes, u32 bExclude. **Internal copy** h.0xb50…0xb83 has the same size
but keeps AreaSize at byte +10 of each 12-byte entry instead of +8: `DrvAlgo_IF_AWB_SetAttrEx` / `GetAttrEx` move the byte both
ways (both copies), so the API layout is the XML one [R: user 0x2e6f4, kernel 0x400634dc; libispalgo 0x17252 reads the size from the
struct's byte 10].
**`pAWB_SetUserExtraArea`** (method +0x70, the 52 bytes passed by value): copy into h.0xb50; all four polygons h.0x1068 + 32·k first
get `memset(…, 1, 0x80)` (every coordinate 0x01010101, overwritten below in every case); records h.0x10e8 + 12·k := 0; then per k: if
0x100 ≤ WRgain < 0x2000 and 0x100 ≤ WBgain < 0x2000 and 1 ≤ AreaSize ≤ 32 → centre (cx, cy) = (102 400 / WRgain, 102 400 / WBgain),
s = AreaSize, h = s >> 1, o = s & 1: x_lo = max(cx − h − o, 0), x_hi = cx + h, y_lo = max(cy − h, 0), y_hi = cy + h + o; polygon
{(x_lo, y_hi), (x_lo, y_lo), (x_hi, y_lo), (x_hi, y_hi)} — an axis-aligned square of side s (for odd s the extra unit goes to the
low-x and the high-y side); record {valid = 1, exclude = bExclude, —}. Otherwise the polygon is set to all 1 (every coordinate = 1)
and valid := 0; exclude is stored either way. The setter accepts gains up to 0x1FFF although the XML range ends at 4095. (The user
decompile seems to drop the third area; the disassembly continues to the fourth, 0x173ac `b 0x1717e`, as the kernel loop does.) The
third word of each record is never written (stays 0) [R: every `0x10e8` reference].
**`pAWB_GetUserExtraArea`** (+0x74) copies h.0xb50…0xb83 back.

**Statistics** (in `pAWB_GetStatisticsExt`, only when AdvMode h.0x5a4 = 1 and ExtraLtEnable h.0xb50 ≠ 0): each usable block first
goes through `pAWB_GetLightAreaUser` = `pAWB_GetLightArea(h.0x1068, none = 4, first 0, last 3)` — **it tests the polygons of invalid
entries too**, which match only the point (1, 1); exclude = the API copy's bExclude of the hit entry (h.0xb5c + 12·k).
- hit, exclude = 0 → user accumulators of entry k (h.0x40c + 0x1c·k, cleared every frame): +0x00 count += 1, +0x04 weight sum += w,
  +0x10 / +0x14 / +0x18 ΣR·w, ΣG·w, ΣB·w; the block is **not** tested against the calibrated areas, not counted in the valid count
  h.0x49c and not in the MultiLS high-precision counts.
- hit, exclude = 1 → the block is dropped from everything except the gray-world totals.
- no hit → the normal path (calibrated areas, then MultiLS counting, h.0x49c += 1).

**`pAWB_GetUserLight`** (after `pAWB_GetLightSource`): only when AdvMode = 1 and ExtraLtEnable ≠ 0, else h.0x4a3 := 4 (none),
return 0. For each valid entry: zero ΣR / ΣG / ΣB become 1; effective weight +0x08 := weight sum if count ≥ AWB_COUNT_THD (h.0x1186),
else 0. If no entry qualified → h.0x4a3 := 4; else h.0x4a3 := the entry with the largest non-zero effective weight (the first wins
a tie). Returns 1 whenever it ran (ignored by `pAWB_Process`).

**Use in `pAWB_CalcGain`**:
- **`pAWB_MixUserGain`** (a calibrated light found, AlgMode ≠ 0, a user light chosen): Ru = ⌊1024·ΣG / ΣR⌋, Bu = ⌊1024·ΣG / ΣB⌋ of the
  user entry (64-bit division, no rounding; skipped if any sum is 0); c = (u16)(c_L1 + c_L2) where c_i = the weighted count of
  calibrated area i (h.0xe8 + 0x1c·i); u = (u16) effective weight of the user entry; w_c = ⌊100·c/(c + u)⌋, w_u = ⌊100·u/(c + u)⌋; R
  := ⌊(R·w_c + Ru·w_u)/100⌋, B likewise; the result also in h.0x54c / 0x550. L2 is read even when it is "none" (index ≥ 10): c_L2 then
  comes from h.0xe8 + 0x1c·10 = h.0x200, a word of the record area just after the ten per-area records (cleared every frame by the
  0x1f8…0x40b memset) [R index arithmetic; I: it is normally 0].
- **`pAWB_GetUserGain`** (no calibrated light, a user light chosen): replaces the previous gains with the user entry alone: R =
  ⌊1024·(ΣG + n/2) / (ΣR + n/2)⌋, B = ⌊1024·(ΣG + n/2) / (ΣB + n/2)⌋, n = the entry's block count (a rounding term in count units added
  to weight-scaled sums); skipped when a sum is 0; h.0x54c / 0x550 := 1024 first, then the result.

### 3.7 Daylight mix, preference, LV-weight lookups, `pAWB_GetBlendValue` [R]
- **`pAWB_GetWeight(h, row, LV, &out)`** (rows of 18 u16: AWB_DEFAULT_RATIO, AWB_PERFER_R/B_RATIO) and **`pAWB_GetLightWeight(h, i,
  LV, &out)`** (AWB_LIGHT_WEIGHT row h.0x5b2 + 0x24·i): LV < 170 → k = LV/10, f = LV mod 10, v = T[k] + (f·(T[k+1] − T[k]))/10 (C
  division), then clamped to [0, max T[0…17]]; f = 0 → T[k] unclamped; LV ≥ 170 → T[17]. LV is the h.0xb0 value 0…200 (LV×10 + 50),
  so the 18 points cover LV×10 = 0…170 and 170…200 use the last one.
- **`pAWB_MixDaylightGain(h, Rd, Bd, &R, &B)`**: w = `GetWeight`(AWB_DEFAULT_RATIO row of **L1 only** = h.0x118a + 0x24·L1, LV); R :=
  ⌊(R·(100 − w) + Rd·w)/100⌋, B likewise; the result also in h.0x534 / 0x538. The daylight gains (Rd, Bd) are light-table entry **3**
  (`pAWB_GetDaylightGain`: u16 h.0x1138 / h.0x113e = entry 3's R / B). L2's weight is computed too (kernel: into a stack half-word that
  is never read; user: through `pAWB_GetBlendValue`, whose result is overwritten — libispalgo 0x19556), so both copies compute the
  same.
- **`pAWB_GetBlendValue(h, a, b, va, vb)`** returns ((⌊100a/(a+b)⌋·va + ⌊100b/(a+b)⌋·vb) / 100) as 16 bits (the code computes
  ⌊x·0x51eb851f / 2³²⌋, masks it to 21 bits and shifts by 5, so the result wraps once the weighted sum x reaches 2²¹/0.32 ≈ 6.55·10⁶)
  — a count-weighted mean of two weights; a + b = 0 divides by zero. Its only caller is the user copy of `pAWB_MixDaylightGain`, which
  discards the result; the kernel copy does not call it — unused in effect.
- `pAWB_GetPrevGain`: h.0x4ac / 0x4b0 → out and h.0x564 / 0x568. **`pAWB_GetPreferenceGain`**: r = `GetWeight`(PERFER_R_RATIO row
  of L1, LV), applied as R := ⌊R·r/100⌋ only when r ≠ 100; the same with PERFER_B_RATIO for B; results also in h.0x544 / 0x548. Like
  the daylight mix it also looks up L2's value when L2 < 10 and never reads it [R both copies].

### 3.8 High-precision light areas and MultiLS (AlgMode 3) [R]
<!-- ev:isp-5-3a.3.8 -->
MultiLS (API 0x1804 WBMultiLSAttr {enable, Sensitive 1…10, CaliStrength ≤ 100, CCM for reference 1 and 2 (9 × u16 ≤ 0x1FFF)},
h.0x1324…0x1354; the setter is inline in `DrvAlgo_IF_ApiSet`, isp-3-iq-api.md §4.3) estimates the share of two reference lights
from high-precision area counts and blends the AWB output CCM between two reference matrices. The CCM side (`pAWB_MultiLS_
RatioSlowChange`, `pAWB_MultiLS_CaliCcmCalc`, `pAWB_Ccm_Inter`, the defaults of h.0x1324…0x135f and the 16-bit / 13-bit defect) is
isp-4-iq-engine.md §2.2; this section gives the ratio side. first / last = AWB items 0xe0 / 0xe1 (h.0xb84 / h.0xb86).

**`pAWB_UpdateLightAeraHighPrecision`** (called from `pAWB_GetParamFromCfg` after the golden areas are loaded, from
`pAWB_InitCaliParam` after a calibration, and from `pAWB_SetGoldenLightArea`): 19 areas at h.0xcc8 + 32·j (ending at h.0xf28): j
even = a copy of area j/2 of the table h.0x58c points at; j odd = the coordinate-wise mean (C division by 2) of areas (j−1)/2 and
(j+1)/2. So high-precision index j = 2·light index, odd j = half-way between two neighbouring lights.

**`pAWB_GetLightAreaHighPrecisionCount(h, h+0xcc8, 19, x, y, block)`** (from `pAWB_GetStatisticsExt`, only when AlgMode = 3 and
MultiLS enable h.0x1324 = 1, for every block that went to the calibrated-area test, hit or not): for j = 2·first … 2·last, if the
point lies in the **bounding box** P0x ≤ x ≤ P2x, P1y ≤ y ≤ P0y of area j (the slanted edges are not used) → per-j record h.0x1f8 +
0x1c·j: +0x00 count += 1, +0x10 / +0x14 / +0x18 += R, G, B (unweighted). A block can count in several overlapping j (no early exit).
The sums are always read from the long-frame averages (in+0x3c), also when the call comes from the HDR short-frame loop, whose point
(x, y) is from the short frame [R; I: a slip — the R / G / B sums have no reader anyway]. The 19 records (0x214 B) are cleared every
frame; only +0x00 is read later (CountSmooth) [R: every 0x1f8-area reference].

**`pAWB_CheckRefWhitePoint`** (first in `pAWB_Process` when AlgMode = 3, whether or not MultiLS is enabled):
1. For i = first…last: s_i = count_i · WpWeight[i] (u16 h.0x9ea + 2i, default 100) when count_i ≥ AWB_COUNT_THD, else 0 → h.0x1388 +
   4i. ref = the i with the largest s_i (the first wins a tie); none → 10.
2. If ref ≠ 10 and ref ≠ WPInd (h.0x13b0): thr = 800 if ref < 3; else 100 if |ref − WPInd| = 1, else 800. If ⌊100·s_ref / (s_WPInd +
   1)⌋ > thr, the switch counter h.0x13b1 += 1; it is **not** reset when the test fails or when ref = WPInd, so it counts hits, not
   consecutive frames.
3. When the counter exceeds 10 (the 11th hit): WPInd := ref, counter := 0, and the target dynamic CT weights h.0x1374 + 2j (j 0…9) :=
   200 for j = WPInd, 100 for |j − WPInd| = 1, 20 for |j − WPInd| = 2, 10 otherwise and for j outside first…last. (The defaults of
   §3.12 are this pattern for WPInd = 3.)

**Dynamic CT weight** (in `pAWB_GetLightSource`, AlgMode 3 only): per area i with count ≥ COUNT_THD, cur_i (h.0x1360 + 2i) :=
⌊((100 − sp)·cur_i + sp·target_i)/100⌋ (16-bit result) with sp = AWB_SPEED h.0x59c, or target_i when sp ≥ 100; then c_i = ⌊⌊w_i ·
W_i / 100⌋ · cur_i / 100⌋ (§3.4 step 5).

**`pAWB_MultiLS_Check`** (after CheckRefWhitePoint; user disassembly 0x1a178, the kernel decompile loses the branch): if MultiLS
enable = 1: `CountSmooth` → `PeakCheck` → if at least two peaks `TargetRatioCalc`; otherwise (disabled, or < 2 peaks) the target
ratios h.0x135a / h.0x135b := 0. Then always `RatioSlowChange` and `CaliCcmCalc` (isp-4-iq-engine.md §2.2). Debug level 3 prints
the ratios.
- **`pAWB_MultiLS_CountSmooth`**: for j = 2·first … 2·last, n_j = high-precision count: S_j = ⌊(n_{j−1} + 18·n_j + n_{j+1})/20⌋; at j
  = 2·first S_j = ⌊2·(9·n_j + n_{j+1})/20⌋, at j = 2·last S_j = ⌊2·(9·n_j + n_{j−1})/20⌋ (19-entry stack array; first = last takes the
  first form).
- **`pAWB_MultiLS_PeakCheck`**: j is a peak if S_j ≥ AWB_COUNT_THD and S_j ≥ both neighbours (only the inner one at the two ends;
  ties count, so a plateau gives several peaks). Peak count n. If n < 2: detected h.0x1350 := 0. Else P1 = the peak with the largest S
  (first wins), V1 = (u16) S_P1; P2 = the largest of the others, V2 = (u16) S_P2. h.0x1358 := P1, h.0x1359 := P2 (19 = none; written
  also when n < 2). These are high-precision indices 0…18, which `AWB_QueryInfo` reports as FirstLSInd / SecondLSInd although the XML
  range says 0…9.
- **`pAWB_MultiLS_TargetRatioCalc(h, P1, P2, V1, V2)`** (user disassembly 0x19cf4–0x19f22, both decompiles are mis-structured; the kernel copy was compared with it only by its operands):
  1. If ⌊V1/V2⌋ (u16) > Sensitive (h.0x1328) → targets := 0, detected := 0, return (the second light is too weak). Else detected := 1.
  2. Spans: dR = |R_first − R_last|, dB = |B_first − B_last| of the light table. Reference point 2W (W = WPInd); d_k = (s8)(2W − P_k).
  3. Both peaks above 2W (d1, d2 < 0) → sign −1; both below (d1, d2 > 0) → sign +1; then tR = sign·100·|R_{P1/2} − R_{P2/2}| / dR, tB
     = sign·100·|B_{P1/2} − B_{P2/2}| / dB (signed C division).
  4. Otherwise (the peaks on different sides of 2W, or one on it): F = the peak farther from 2W (|d| larger); equal distances, or P1 =
     P2 → tR = tB = 0. sign = −1 if F has the higher index of the two, +1 if the lower; tR = sign·100·|R_W − R_{F/2}| / dR, tB likewise
     with B.
  5. Targets: h.0x135a := (s8) ⌊tR·CaliStrength/100⌋, h.0x135b := (s8) ⌊tB·CaliStrength/100⌋ (CaliStrength h.0x1329, 0…100; signed
     division toward zero). Light index of a high-precision peak = P >> 1 (an in-between peak counts as its lower neighbour).
  dR or dB = 0 (two lights with equal gains at the ends of the range) divides by zero (libgcc's `__aeabi_idiv`; the result is then
  whatever its divide-by-zero handler returns) [R division; I outcome]. V2 = 0 (possible only with AWB_COUNT_THD = 0) likewise.
- Meaning [I]: the light table runs from high CT (index 0) to low CT, so a negative ratio means "the second light is warmer" and
  `CaliCcmCalc` blends rows 0 and 2 of the CCM toward reference 1 (API `CaliCcm_LowCT`), a positive one toward reference 2
  (`CaliCcm_HighCT`), by |ratio| %.
- Names: the XML names of API 0x1804 +4 / +5 are **Sensitive** and **CaliStrength**; the code uses them as the peak-ratio limit and
  the strength above.
- With the built-in defaults AlgMode = 1 (§3.12), so CheckRefWhitePoint, the dynamic CT weight, the high-precision counting and the
  whole MultiLS chain stay unused unless API 0x1802 selects AlgMode 3 and 0x1804 enables MultiLS (whether the camera's API bin does:
  §5); likewise the user areas need AdvMode 1 + ExtraLtEnable, and day/night needs 0x1807.

### 3.9 Day/night detection (API 0x1807 DayNightDetection / 0x1808 DayNightInfo, 12 B each) [R]
Same struct shapes as the AE ids 0x1418 / 0x1419 and the IQ ids 0x103a–0x103b (XML: {u32 Enable; s32 D2N_BvThd; u32 N2D_VsbLtScoreThd 0…2000} and {u32 D2N; u32
N2D; u32 N2D_VsbLtScore 0…10000}).
- **Set** `pAWB_SetDayNightDetectionParam` (method +0x88): h.0x13b4 := Enable, h.0x13b8 := D2N_BvThd, h.0x13bc :=
  min(N2D_VsbLtScoreThd, 2000). **Get** (+0x84) returns the three words. Defaults: §3.12.
- **Per frame**, at the end of `pAWB_GetStatisticsExt`, when h.0x13b4 = 1: g = max(ΣG, 1); r = ⌊100·ΣR/g⌋, b = ⌊100·ΣB/g⌋ over the
  gray-world totals (every sampled block, long and — in HDR — short frame); **score** = (r − 100)² + (b − 100)² → h.0x13c4; **N2D**
  h.0x13c0 := (score > threshold) (unsigned compare). Enable ≠ 1 → h.0x13c0 := 0, h.0x13c4 keeps its last value (user disassembly
  0x17f48–0x18066; the kernel decompile computes the same score and loses the flag store).
- **Get info** `DrvAlgo_IF_AWB_GetDayNightInfo` writes only +0x04 (N2D = h.0x13c0) and +0x08 (score = h.0x13c4); +0x00 D2N is left as
  the caller had it. D2N_BvThd (h.0x13b8) has no reader in the AWB [R: every `0x13b8` reference].
- Meaning [I]: the score is the squared distance of the whole-frame colour from neutral grey in percent (100·R/G, 100·B/G); under IR
  illumination the image is almost monochrome (small score), visible light brings colour back, so a score above the threshold
  reports "night → day". The day → night half (BV threshold) belongs to the AE's own day/night id (§2.15).
- Nothing acts on the result inside the 3A: h.0x13c0 / 0x13c4 are read only by the 0x1808 getter [R: every reference in the kernel module and
  libispalgo]; an application has to poll 0x1808 and switch the IR-cut / IQ itself [I].

### 3.10 `AWB_QueryInfo` (API 0x1801 AWBInfo, 24 B) [R both copies]
`DrvAlgo_IF_AWB_QueryInfo(h, info)` (no method slot; reads the object directly; returns 6 when there is no handle) fills:

| off | field (XML) | source | written by |
|---|---|---|---|
| +0x00 | u32 IsStable | u8 h.0x574 | `pAWB_isConverged` (the converged flag, §3.4 step 6); 0 at init |
| +0x04 | u16 WB_Rgain | low half of h.0x13e8 | `pAWB_ApplyGain`: copy of AwbOutput +0x22 (final R gain) |
| +0x06 | u16 WB_GRgain | h.0x13ec | ApplyGain: out+0x24 (Gr) |
| +0x08 | u16 WB_GBgain | h.0x13f0 | ApplyGain: out+0x26 (Gb) |
| +0x0a | u16 WB_Bgain | h.0x13f4 | ApplyGain: out+0x28 (B) |
| +0x0c | u16 WB_CT | u16 h.0xc0 | smoothed colour temperature (§3.4 step 6) |
| +0x0e | u8 WPInd | u8 h.0x13b0 | `pAWB_CheckRefWhitePoint` (§3.8; MultiLS only), `pAWB_InitGain` := 3 |
| +0x0f | u8 pad | — | not written (the caller's buffer keeps its byte) |
| +0x10 | u32 MultiLS_Detected | h.0x1350 | `pAWB_MultiLS_TargetRatioCalc` / `PeakCheck` (§3.8); 0 at init |
| +0x14 | u8 FirstLSInd | h.0x1358 | `pAWB_MultiLS_PeakCheck` (§3.8) |
| +0x15 | u8 SecondLSInd | h.0x1359 | same |
| +0x16 | u8 pad[2] | — | not written |

The wrapper also allocates a 16-byte zeroed buffer and frees it again unused (user 0x1e81c / 0x1e86c; r0 is untouched in between).
Outside MultiLS (AlgMode ≠ 3) WPInd stays 3 and the three MultiLS fields keep their init values for the life of the handle [R: no
other writer].

### 3.11 API ids → AWB fields
The id → handler mapping and sizes: isp-3-iq-api.md §4.3. Effect on the object:

| id | effect |
|---|---|
| 0x1801 AWBInfo | `AWB_QueryInfo` (§3.10) |
| 0x1802 WBAttr (1464 B) | `pAWB_SetApiState` h.0x594 (1 = pause → state 4), `SetApiMode` h.0x598 (0 auto, 1 MWB, 2 CT-MWB → state h.0x590), `SetAwbAdvMode` h.0x5a4 (user areas), `SetMwbAttr` h.0xb44 / 0xb48, `SetGainStrength` h.0x5a8 / 0x5a9, `SetConvergeSpeed` h.0x59c (1…100), `SetCtLimit` (four u16 gain limits 0x100…0x1FFF, rejected if min > max) h.0x5aa / 0x5ae, `SetAwbAlgMode` h.0x5a0 (0…3, else −1), `SetConvergeThd` h.0x59d / 0x59e, per LV step `SetLvColorTempWeight` / `SetPreferRRatio` / `SetPreferBRatio` (the 18 × 10 tables), `SetWpWeight` (10 × u16 h.0x9ea…0x9fc), `SetWeightWin` (0x144 B → h.0xa00) |
| 0x1803 WBAttrEx | `pAWB_SetUserExtraArea` / `GetUserExtraArea` (§3.6) |
| 0x1804 WBMultiLSAttr | inline stores (isp-3-iq-api.md §4.3), fields h.0x1324…0x1354 (§3.8) |
| 0x1806 CTMWB | `pAWB_SetCTMwb`: h.0xb4c = CT; with mode 2 → state 2 |
| 0x1807 / 0x1808 | day/night detection parameters / info (§3.9) |
| 0x1809 AWBStabilizer | `pAWB_SetStabilizer`: h.0x13c8…0x13d0 {enable, GlbGainThd, CountThd, ForceTriGainThd} (§3.4 step 6) |
| user only: 0x2C03 / 0x2C04 / 0x2C05 / 0x2C06 | golden light area set / get (`pAWB_SetGoldenLightArea` re-derives the high-precision areas), CT statistics (23 044 B, `pAWB_GetCTStatistics`), raw hardware statistics (34 560 B = 128 × 90 × 3), current light area (`pAWB_GetCurLightArea`) — field layouts not read |
| 0x240F (calibration) | `CALIv30_AWB_CaliApply` → `pAWB_ApplyUnitCalibration` → unit calibration (§3.5) |

### 3.12 Built-in defaults (`pAWB_InitParameters`) and `pAWB_InitGain` [R emulation + both decompiles]
<!-- ev:isp-5-3a.3.12 -->
How obtained: libispalgo's own `pAWB_InitFuncPtr` + `pAWB_InitParameters` run under unicorn on a zeroed 0x13FC-byte object prepared
as `AwbInit` leaves it (h.0xa8 → a zeroed `AwbInput_t` with libcus3a's initial gains 0x400 at +0x14 / +0x16 / +0x18; h.0xac → a zeroed
`AwbOutput_t`; cfg NULL; h.0x578…0x584 = 0x400) with `pAWB_GetParamFromCfg` skipped (162 stores recorded). The kernel `pAWB_InitParameters` (0x4005f298) stores the same
offsets and constants [R: every constant store of its code matched against the list].
**Order**: `pAWB_GetParamFromCfg` runs **first** (IQ items, §3.2, including the golden areas, the high-precision areas, the active
areas and the light table), then the built-in defaults below. No default store overlaps an IQ-file field [R: offsets compared with
the destinations of `pAWB_GetParamFromCfg` and the 0xd8 / 0xd9 destinations], so the IQ file wins only for its own fields, and **fields of the
IQ list keep 0 when the file lacks the item** — notably AWB_SPEED h.0x59c (0 → `pAWB_GetSmoothBySpeed` moves one LSB per run),
AWB_CONV_THD / OUT_THD, the LV tables, LUM_MAX / MIN (0 / 0 → every block fails the luminance test) [R: no default store; I:
consequences].

| h. | type | default | meaning | also set by |
|---|---|---|---|---|
| 0x000…0x0a0 | ptr ×41 | method table | `pAWB_InitFuncPtr` | — |
| 0x0b0 | s32 | 100 | LV×10 + 50 (= LV 5.0) until the first `DoAwb` | DoAwb each frame |
| 0x0b4 | u32 | 1 | first-frame flag: the first `DoAwb` copies the frame count into h.0xb8 and clears it | DoAwb |
| 0x0c0 | u16 | light-table entry 4's CT (h.0x1140; 0 without IQ) | current colour temperature | CalcGain, ManualGain |
| 0x574 | u16 | 0 | converged flag (u8) + h.0x575 converge-step counter | isConverged, CalcGain |
| 0x588 | u32 | 1 | force: the next auto run jumps / smooths even if converged | CalcGain clears |
| 0x590 / 0x594 / 0x598 | u32 | 0 / 0 / 0 | DoAwb state (auto), API state (run), API mode (auto) | 0x1802 setters, 0x1806 |
| 0x5a0 / 0x5a4 | u32 | **1** / 0 | AlgMode = normal; AdvMode = off (no user areas) | 0x1802 |
| 0x5a8 / 0x5a9 | u8 | 128 / 128 | R / B gain strength (128 = 1×) | 0x1802 |
| 0x5aa / 0x5ac / 0x5ae / 0x5b0 | u16 | 0x1FFF / 0x100 / 0x1FFF / 0x100 | CtLimit: R max / R min / B max / B min | 0x1802 |
| 0x9ea | u16 ×10 | 100 each | white-point weights (WpWeight; used only by `CheckRefWhitePoint`, §3.8) | 0x1802 |
| 0xa00 | u32 ×81 | 8 each (`AwbWeightWin`) | 9 × 9 statistics weight window (§3.3) | 0x1802 |
| 0xb44…0xb4a | u16 ×4 | 1024 ×4 | MWB gains R, Gr, Gb, B | 0x1802 |
| 0xb4c | u32 | 5000 | CT for CT-MWB | 0x1806 |
| 0xb50 | u32 | 0 | ExtraLtEnable | 0x1803 |
| 0xb54 + 12k (k 0…3) | u16, u16, u8 @+6, u32 @+8 | 1024, 1024, size 1, exclude **1** | user extra areas (internal layout §3.6) | 0x1803 |
| 0x1068 | s32 ×32 | 1 each (`AwbLightUserArea`) | user-area polygons (match only (1, 1)) | 0x1803 |
| 0x10e8 | u32 ×12 | 0 (`AwbLightUserParam`) | user-area records {valid, exclude, —} | 0x1803 |
| 0x117c | u8 | 0 | debug level (copied each frame from `AWBDEBUG_t`+0x10 by `pAWB_SetDebugParam` / `GetDebugLevel`; 1 / 2 print logs, 3 prints MultiLS) | DoAwb |
| 0x117e / 0x1180 | u16 | 2 / 1 | statistics sampling steps: every 2nd column, every row | — (no setter) |
| 0x1188 | u8 | 1 | jump (no smoothing) while frames since start (h.0xbc − h.0xb8) < this — i.e. on the first run only | — |
| 0x12fc / 0x1300 | u32 / u16 | 0 / 0 | unit calibration enable / mode | 0x240F |
| 0x1304…0x1320 | u32 ×8 | 1024 each | golden and unit white-point gains | 0x240F |
| 0x1324…0x135f | | MultiLS off, Sensitive 5, CaliStrength 100, the two reference CCMs, ratios 0 (values: isp-4-iq-engine.md §2.2) | MultiLS | 0x1804 |
| 0x1350 / 0x1354 | u32 | 0 / 0 | MultiLS detected / CCM force | §3.8, 0x1804 |
| 0x1360 | u16 ×10 | 10, 20, 100, 200, 100, 20, 10, 10, 10, 10 (`AwbDynamicCTWeight`) | current dynamic CT weight per light (MultiLS) | GetLightSource |
| 0x1374 | u16 ×10 | same | its target | CheckRefWhitePoint |
| 0x1388 | u32 ×10 | 0 (memset 0x28) | per-light white-point score s_i (§3.8) | CheckRefWhitePoint |
| 0x13b0 / 0x13b1 | u8 | 3 (set by InitGain) / 0 | reference white point WPInd / switch counter | CheckRefWhitePoint |
| 0x13b4 / 0x13b8 / 0x13bc | u32 / s32 / u32 | 0 / −30000 / 600 | day/night enable / D2N_BvThd (unread) / N2D score threshold | 0x1807 |
| 0x13c0 / 0x13c4 | u32 | 0 / 0 | N2D flag / score | §3.9 |
| 0x13c8 / 0x13cc / 0x13ce / 0x13d0 | u32 / u16 / u16 / u16 | 0 / 64 / 2 / 150 | AWB stabilizer: enable / GlbGainThd / CountThd / ForceTriGainThd | 0x1809 |
| 0x13d4 / 0x13d8 | u32 | 0x1FFF / 0x1FFF | gray-world gains latched when convergence completes (stabilizer reference) | CalcGain |
| 0x13dc / 0x13e0 | u32 | 0 / 0 | gray-world gains of the frame | CalcGain |
| 0x13e8…0x13f4 | u32 ×4 | 1024 ×4 (after InitGain) | last applied R, Gr, Gb, B (QueryInfo) | ApplyGain |
| AwbOutput +0x2c / +0x30 | u32 / u16 ×9 | 0 / identity | CCM-update flag / CCM | `pAWB_MultiLS_CaliCcmCalc` |

Fields the AWB never initialises except through calloc (0): the per-frame accumulators h.0xc4…0x40b and h.0x40c…0x47b (cleared every
frame by `GetStatisticsExt`), h.0x49c…0x4a3, the AWB_* IQ fields of §3.2, h.0x1186 (COUNT_THD).

**`pAWB_InitGain`** [R both copies] — called last in `pAWB_InitParameters` and nowhere else. WPInd h.0x13b0 := 3. Start gains from
the `AwbInit` input (h.0x578 = in+0x14 R, h.0x57c = in+0x16 G, h.0x580 = in+0x16 again, h.0x584 = in+0x18 B):
- all four = 1024 (libcus3a's case) → **R, B := light-table entry 4's gains** (u16 h.0x1142 / h.0x1148; entry 4 is the record at
  h.0x1118 + 40), G := 1024;
- otherwise the given R, G (in+0x16), B unchanged (the nested tests on G and the second G copy all reduce to this).
Then: smoothed target h.0x47c (R) / h.0x480 = h.0x484 (G) / h.0x488 (B) and the strength output h.0x48c…0x498 := those gains; the
ten {R, B} pairs at h.0x524 + 8k (k 0…9) := (R, B); the eight 16-byte history records h.0x4a4 + 16k := {0, 0, R, B} (k 0…7);
`pAWB_CheckGain` and `pAWB_ApplyGain` (AwbOutput +0x22…+0x2a and h.0x13e8…0x13f4). So the first frame's output before any statistics
is entry 4's white point; the colour temperature h.0xc0 was set to entry 4's CT just before. Without an IQ file (cfg NULL) entry 4 is
all zero: R = B = 0 → `CheckGain` turns them into 1024 (emulated: output 1024 ×4). Which light entry 4 is depends on the IQ file's
AWB_COLOR_TEMP order [I: a mid-range CT, next to the daylight entry 3].

## 4. AF — `AfInit` / `DoAf` / `AfRelease`, `pAF_*`, `AF_*` [R unless marked]
<!-- ev:isp-5-3a.4 -->

Contrast-detection AF for a VCM lens. Object 0x2CA0 bytes, 35-method table (API setters / getters). Kernel copy: `DoAf` has no caller
and every AF API id fails on the kernel path (§1.1). User copy: runs every frame through libcus3a's built-in adaptor once the
kernel raises the AF event for the channel, initialised lazily at the first AF frame with cfg = NULL (isp-6-userspace.md §2.7); this
camera has no VCM, so its result (0x2E21) moves nothing (§4.9). Read from the 91 AF functions of libispalgo and the kernel copy (which inlines some helpers, §1.4). Kernel and user copies agree unless a difference is named.

### 4.1 `AfInit`, `AfRelease`, inputs and outputs
- **`AfInit(cfg (unused, NULL), initParam, in, out)`**: calloc; keep param / in / out (h.0x2c90 / 0x2c94 / 0x2c98); `AF_InitFuncPtr`;
  `AF_InitParameters`: init parameter = {MinPos, MaxPos, 1, MaxPos − MinPos, CurPos, dev, ch}; dev / ch (+0x14 / +0x18) → h.0x29ac /
  h.0x29b0; validates {MinPos, MaxPos, 1, MaxPos − MinPos, CurPos} (zero range → error print, values used anyway); built-in defaults
  (§4.2); then the nine `hal_AF_InitParameters_*` hardware defaults (kernel: zeros; user: the table of §1.4) and the nine
  `pAF_SetHW*` methods with them (user: → CUS3A 0x2E11, 16 × 0x2E0E, 0x2E0F, 0x2E10, 0x2E24, 0x2E25, 0x2E26, 0x2E27, 0x2E28);
  accumulators reset. **`AfRelease`** frees the object.
- **Inputs per frame** (`AF_UpdateInputParameters`, from `DoAf`'s in = {+0x08 frame-rate-like value, +0x0c → 16 × `AF_STATS_PARAM_t`
  (the AF statistics, isp-2-ispmid-mload-ispscl.md §10.3), +0x10 current motor position, +0x14 → debug {enable, level}}): h.0x9c :=
  debug level (u16; bit 0 input log, bit 1 output log, bit 2 per-window accumulator log, bit 3 one-value log, bit 4 scene-check
  histogram print, bit 5 one-shot log, bit 6 continuous log, bit 7 scene-check log, bit 8 search-ratio log), h.0xaa := current
  position, h.0x9e := in+0x08 × 10 (libcus3a passes 30 → 300), speed class h.0xa0 := clamp(3 − ⌊h.0x9e/100⌋, 0…2) (300 → 0); then
  `AF_Motor_UpdateStatus`, `UpdateHistPos`, `UpdateEverPos`, `AF_Acc_UpdateAllValue`, `AF_Acc_UpdateOneValue`.
- **Output** (`AF_UpdateOutputParameters`): out+0 = (h.0xac ≠ h.0xaa) "change", out+4 = h.0xac next position.
- **`DoAf(h, in, out)`**: only when debug.enable ≠ 0; the start delay h.0x8c (30 frames) counts down first; then the inputs,
  `AF_UpdateInfo`, h.0x2c8c := current position, and: State h.0x2b1c ≠ 0 → nothing (paused); Mode h.0x2b20 = 1 → h.0xac :=
  ManualMotorPos h.0x2b24; else Algo h.0x2b28 = 0 → `AF_ALGO_OneShot` (§4.6), 1 → `AF_ALGO_Continuous` (§4.5); the output is always
  written.

### 4.2 Object and built-in defaults [R emulation + decompiles]
<!-- ev:isp-5-3a.4.2 -->
How obtained: libispalgo's `AF_InitFuncPtr` + `AF_InitParameters` run under unicorn with libcus3a's init parameter, all positions 0
— the camera's case, where the 0x2E20 status read fails and leaves zeros — and the nine CUS3A setters returning 0
(226 stores recorded). With MinPos = MaxPos = 0 the range check
prints an error and the values are used anyway.

| h. | default | meaning |
|---|---|---|
| 0x000…0x088 | 35 methods | `AF_InitFuncPtr` (0 QueryInfo, 4 Trigger, 8 ForceTrigger, 0xc…0x50 HW set / get, 0x54…0x88 the API set / get pairs of §4.8) |
| 0x08c | 30 | start delay (frames) |
| 0x090 / 0x098 | 1 / 0 | "focused" gate of the dead `AF_StepSpeedCtrl` (§4.7), 0 after focus / previous FSM state |
| 0x094 | 3 | FSM state: 2 scene changing (wait), 3 search start, 4 search, 5 focused, 6 / 7 done (§4.5) |
| 0x09c | — | debug level (§4.1) |
| 0x09e | 300 | in+0x08 × 10 |
| 0x0a0 | 0 | speed class 0…2 |
| 0x0a2 / 0x0a4 / 0x0a6 / 0x0a8 | MinPos / MaxPos / 1 / MaxPos − MinPos | lens limits from the init parameter (check range of `pAF_SetMotor`) |
| 0x0aa / 0x0ac | CurPos / CurPos | current / next position |
| 0x0ae, 0x0b0, 0x0b2, 0x0b4 | 0 | position history: this frame, 1, 2, 3 frames back (statistics belong to an earlier position) |
| 0x0b6 / 0x0b8 | MaxPos / MinPos (search start resets them to motor max / min) | lowest / highest position visited ("ever" positions) |
| 0x0ba | MinPos | approach target after the peak (§4.5) |
| 0x0bc | 1 (init param +8) | current step size |
| 0x0be | 1 | s8 step direction (±1) |
| 0x0c0 / 0x0c4 | 0 / 0 | position inside the motor range / at one of its ends |
| 0x0c8 + 4k, 0x4c8, 0x8c8, 0xcc8, 0x10c8, 0x14c8 | — | per-window values (§4.3) |
| 0x18c8 / 0x20cc | −1 / −1 | selector of the "previous" / "focused" per-window snapshot (§4.3) |
| 0x28d0…0x28f7 | — | one-value results (§4.3) |
| 0x28f8 / 0x2908 / 0x2918 / 0x2928 | 0 | 4-deep histories of the one-values IIRH / IIRL / SBLV / SBLH |
| 0x2938…0x2963 | see §4.3 | local max / min tracker |
| 0x2964…0x2974 | 0 | one-shot phase flags (§4.6); 0x296c…0x2974 scene-check state (§4.5) |
| 0x2984…0x299f | 0 …, h.0x2988 = CurPos | continuous-search state: start value / position, ratios, up / down counters, reversed and peak flags (§4.5) |
| 0x29a0 / 0x29a4 | 2 / 1 | focus settle counter (§4.5) / wobble amplitude (dead, §4.7) |
| 0x29ac / 0x29b0 | dev / ch | CUS3A target of the HW setters |
| 0x29b4…0x2b1b | HW defaults | 0x1C01–0x1C09 copies (§1.4); h.0x29b4 HW ROI mode, h.0x29b8 VerticalBlockNumber |
| 0x2b1c…0x2b2b | 0, 0, MinPos, 1 | AFAttr (0x1C29): State, Mode, ManualMotorPos, Algo = continuous |
| 0x2b2c…0x2b33 | 320, 700, 1, 50 | AFMotor (0x1C2A): MinMotorPos, MaxMotorPos, MinMotorStep, MaxMotorStep |
| 0x2b34…0x2c43 | WinNumX 16, WinNumY 1, EqualWinWgtEn 1, WinWgt 255 ×256, IIRHBlendEn 0, blends 255 / 255 / 255 | AFAccWeight (0x1C2B) |
| 0x2c44 / 0x2c48 | 2 (SBLV) / 10 | AFOneShot (0x1C2C): AccSel / MotorStep |
| 0x2c4c…0x2c63 | PreAfAccSel 0 (IIRH); PreAe DiffThOft / Slp 20 / 5, CntThOft / Slp 20 / 0; PreAf DiffThOft / Slp 20 / 0, CntThOft / Slp 20 / 0; FocusAfAccSel 1 (IIRL); FocusAe DiffTh / CntTh 100 / 20, FocusAf DiffTh / CntTh 10 / 20; StableCntTh 15 [R] | SceneChange (0x1C2D) |
| 0x2c64…0x2c73 | u16 24, 12, 26, 16, 13, 1, 2, 26 | step constants of the dead step-speed control (§4.7; `.rodata` 0x26090 / …98) |
| 0x2c74 / 0x2c76 / 0x2c78 | 10 / 400 / 2 (SBLV) | SearchStart (0x1C2E) |
| 0x2c7c…0x2c81 | 1, 0, 20, 5, 20, 5 | Search (0x1C2F) |
| 0x2c82 | 0 (never written) | wobbling counter limit (dead, §4.7) |
| 0x2c84 | 5 | slope threshold of the dead wobbling (§4.7) |
| 0x2c88 / 0x2c8c | 0 / 320 | AF info: focus-done flag / current position (`pAF_QueryInfo`, 0x1C30) |
| 0x2c90 / 0x2c94 / 0x2c98 | — | init param / input / output pointers |

The defaults differ from the XML-documented "typical" values only where noted.

### 4.3 Accumulators (`AF_Acc_*`) [R]
- **Per-window values** `AF_Acc_UpdateAllValue` (every frame): for row r < WinNumY (h.0x2b35) and column c < WinNumX (h.0x2b34),
  record r of the statistics (in+0x0c + 0x1c0·r, `AF_STATS_PARAM_t`) and k = 16r + c: each 5-byte field (40-bit little-endian) is
  taken as **bytes 1…4 only, i.e. value >> 8**, into u32 arrays of 256: IIR1 (+0x00 + 5c) → h.0xc8 + 4k ("IIRH"), IIR2 (+0x50) →
  h.0x4c8 ("IIRL"), FIR-V (+0xf0) → h.0x8c8 ("SBLV"), FIR-H (+0x140) → h.0xcc8 ("SBLH"), luma (+0xa0) → h.0x10c8; the 3-byte
  Y-saturation count (bytes +0x190 + 4c, +1, +2) → h.0x14c8 (full 24 bits). Debug bit 2 prints them.
- **Selector** `AF_Acc_GetAccPtr(h, kind, sel)`: kind 0 = per-window arrays: sel 0 IIRH h.0xc8, 1 IIRL h.0x4c8, 2 SBLV h.0x8c8, 3 SBLH
  h.0xcc8, 4 luma h.0x10c8, any other → h.0xc8; kind 1 = one-value histories: 0 IIRH h.0x28f8, 1 IIRL h.0x2908, 2 SBLV h.0x2918, 3 SBLH
  h.0x2928, other → h.0x28f8; kind ≥ 2 → h.0xc8. The API selectors (AccSel 0…3 = IIRH, IIRL, SBLV, SBLH) index this.
- **One value per kind** `AF_Acc_UpdateOneValue` (every frame): over the same windows, EqualWinWgtEn (h.0x2b38) = 1 → plain 64-bit
  sums, n = number of windows; else each window × WinWgt[16r + c] (u8 h.0x2b3c + k) and n = Σ WinWgt (u16); n = 0 → 1. Results (u64,
  = sum / n) IIRH h.0x28d0, IIRL h.0x28d8, SBLV h.0x28e0, SBLH h.0x28e8, luma h.0x28f0. If IIRHBlendEn (h.0x2c3c) ≠ 0, IIRH is replaced
  by a three-step blend with a = FirstBlendIIRL (h.0x2c40), b = SecondBlendSBLV (h.0x2c41), c = ThirdBlendSBLH (h.0x2c42): v1 =
  (a·IIRH + (256 − a)·IIRL + 128) >> 8, v2 = (b·v1 + (256 − b)·SBLV + 128) >> 8, IIRH := (c·v2 + (256 − c)·SBLH + 128) >> 8 (64-bit).
  Debug bit 3 prints them.
- **Histories** `AF_Acc_UpdateHistOneValue` (called by the algorithms, not per frame): each of the four 4-entry u32 histories
  h.0x28f8 / 0x2908 / 0x2918 / 0x2928 shifts by one (memmove 12 B) and takes the low word of the current IIRH / IIRL / SBLV / SBLH.
  `ResetHistOneValue` zeroes all 16 words.
- **Per-window snapshots** `AF_Acc_UpdatePreAllValue(h, sel)` → h.0x18c8 := sel, h.0x18cc + 4k := luma[k], h.0x1ccc + 4k :=
  value[sel][k]; `AF_Acc_UpdateFocusAllValue(h, sel)` the same into h.0x20cc / h.0x20d0 / h.0x24d0 (the snapshot taken when focus is
  reached); `Reset…` set the selector to −1 and the windows to 0 (only the WinNumX × WinNumY used ones).
- **Local extremes** `AF_Acc_UpdateLocalOneValue(h, sel)` with v = history[sel][0], v_prev = history[sel][1]: if the "new max" flag
  h.0x2954 = 1 → h.0x294c := v, h.0x2950 := position 1 frame back (h.0xb0), flag := 0 (the value one step after the maximum); if v >
  max h.0x293c → max := v, h.0x2940 := h.0xb0 (its position), h.0x2944 := v_prev, h.0x2948 := h.0xb2, flag := 1; min h.0x2958 :=
  min(v, ·); h.0x295c := max(v, ·) (a second running max). `ResetLocalOneValue`: h.0x2938 := −1, max := 0, positions h.0x2940 /
  0x2948 / 0x2950 := motor min (h.0x2b2c), h.0x2944 = h.0x294c := 0, and the two 8-byte constants 0x247d0 / …d8 into
  h.0x2954…0x2963 (flag 0, min 0xFFFFFFFF, max 0 [R emulation: h.0x2954 = 0, h.0x2958 = 0xFFFFFFFF, h.0x295c…0x2963 = 0]).
- `AF_Acc_CheckUnstable` and `AF_CalcAccRatio`: §4.5.

### 4.4 Motor (`AF_Motor_*`) [R]
- `UpdateStatus`: h.0xc0 := (motor min h.0x2b2c ≤ pos ≤ motor max h.0x2b2e); h.0xc4 := (pos = min or pos = max).
- `UpdateHistPos`: h.0xb4 ← h.0xb2 ← h.0xb0 ← h.0xae ← pos (h.0xaa).
- `UpdateEverPos`: h.0xb6 := min(h.0xb6, pos), h.0xb8 := max(h.0xb8, pos); `ResetEverPos`: h.0xb6 := motor max, h.0xb8 := motor min.
- `MoveByDirAndStep`: next h.0xac := clamp(pos + dir h.0xbe · min(step h.0xbc, MaxMotorStep h.0x2b32), motor min, motor max).
- `MoveToTargetPos(target, maxStep)`: next := clamp(pos ± min(|target − pos|, maxStep), motor min, motor max) (direction + when
  target > pos, − otherwise).
- The algorithm only proposes h.0xac; libcus3a sends it as `CusAFResult_t` NextPos (0x2E21) every frame.

### 4.5 Continuous AF (`AF_ALGO_Continuous`, Algo = 1, the default) [R; branches checked in the user disassembly 0x15a9c and the kernel one]
Per frame, in order:
1. Position outside the motor range (h.0xc0 = 0) → `MoveToTargetPos(clamp(pos, min, max), MaxMotorStep)`, done.
2. Motor still travelling (pos ≠ next h.0xac) → `MoveToTargetPos(next, MaxMotorStep)`, done (the FSM waits for arrival).
3. `AF_Acc_UpdateHistOneValue`; **`AF_CheckScene`** (below); if it returns 1: focus-done h.0x2c88 := 0, state := 3 (previous in
   h.0x98) and `AF_FSM_SearchStart` runs at once.
4. Else by state h.0x94: **2** (scene changing) step := 0, wait; **3** `AF_FSM_SearchStart`; **4** `AF_FSM_Search`; **5**
   `AF_FSM_Focused`; **6 / 7** focus-done := 1, step := 0 (hold). Debug bit 6 logs each branch.
The start state after init is 3, so the first frames after the start delay run a search.

**`AF_CheckScene`** (returns 1 = start a new search):
- sel_pre = PreAfAccSel (h.0x2c4c), sel_f = FocusAfAccSel (h.0x2c58); Δ = |h.0xb0 − h.0xb2| (motor movement between the two
  positions the current statistics belong to).
- If a focus snapshot exists for sel_f (h.0x20cc = sel_f): unstable = `CheckUnstable`(luma, focus luma snapshot h.0x20d0, diff
  FocusAeAccDiffTh h.0x2c5c, count FocusAeAccCntTh h.0x2c5d) **or** `CheckUnstable`(values[sel_f], focus snapshot h.0x24d0,
  FocusAfAccDiffTh h.0x2c5e, FocusAfAccCntTh h.0x2c5f) — each threshold capped at 99.
- Else (no focus yet) the thresholds grow with the movement: AE diff = Δ·PreAeAccDiffThSlp (h.0x2c51) + Oft (h.0x2c50), AE count =
  Δ·h.0x2c53 + h.0x2c52, AF diff = Δ·h.0x2c55 + h.0x2c54, AF count = Δ·h.0x2c57 + h.0x2c56 (each capped at 99); compared with the
  previous-frame snapshots (luma h.0x18cc, values[sel_pre] h.0x1ccc).
- Then the previous snapshot := current (`UpdatePreAllValue(sel_pre)`). h.0x296c := unstable. Unstable → "changing" h.0x2970 := 1,
  stable counter h.0x2974 := 0, focus snapshot reset, state := 2 (unless already 2). Stable → h.0x2974 += 1 (u8, wraps).
- Returns 1 once, when h.0x2970 = 1 and the stable counter **equals** StableCntTh (h.0x2c60, default 15); h.0x2970 := 0 then. So a
  search starts StableCntTh frames after the scene stopped changing (StableCntTh = 0 → at once, on the frame that saw the change,
  since the counter was just set to 0). Debug bit 7 logs.
- **`AF_Acc_CheckUnstable(h, cur, ref, diffPct, cntPct)`**: over the used windows, d = |cur − ref|, m = max(cur, ref); a window counts
  as changed when d > ⌊m·diffPct/100⌋ (64-bit); unstable = (changed > ⌊cntPct·N/100⌋), N = WinNumX·WinNumY. It also builds an 11-bin
  histogram of ⌊⌊100·d/max(m, 1)⌋/5⌋ (capped at 10), each bin as % of N capped at 99 — used only for the debug-bit-4 print.

**`AF_FSM_SearchStart`** (state 3): sel_s = SearchAccSel (h.0x2c78); `ResetEverPos`, `ResetLocalOneValue`; ratios h.0x298a…0x298e,
up / down status h.0x2990, counters h.0x2994 / 0x2995, reversed flag h.0x2998 and peak flag h.0x299c := 0; direction h.0xbe := +1 if
pos < SearchMotorDirByPosTh (h.0x2c76, default 400) else −1; step h.0xbc := SearchMotorStep (h.0x2c74, default 10);
`UpdateLocalOneValue(sel_s)`; start value h.0x2984 := history[sel_s][0], start position h.0x2988 := h.0xb0; state := 4. The motor
does not move in this frame.

**`AF_FSM_Search`** (state 4), per frame:
1. `UpdateLocalOneValue(sel_s)`; **`AF_CalcAccRatio`**: with max = h.0x293c, v = history[sel_s][0]: if max ≠ 0: MinMax ratio h.0x298a
   = ⌊100·(max − min)/max⌋ (min = h.0x2958), Start ratio h.0x298b = ⌊100·(max − start)/max⌋, Now ratio h.0x298c = ⌊100·(max −
   v)/max⌋; if the second max h.0x295c ≠ 0: NowFake ratio h.0x298d = ⌊100·(h.0x295c − v)/h.0x295c⌋ (all stored as u8 low bytes of
   64-bit quotients); peak threshold h.0x298e = clamp(⌊(R − (everMax − everMin))·MinMaxAccRatioPeakThSlp (h.0x2c7d)/R⌋ +
   MinMaxAccRatioPeakThOft (h.0x2c7c) + speed class, 0, 99), R = motor max − min — the less of the range has been swept, the higher
   the threshold. Debug bit 8 logs.
2. **`AF_UpdateUpDownStatus`**: v < previous → status h.0x2990 := 1 (falling), fall counter h.0x2995 += 1, rise counter h.0x2994 :=
   0; v > previous → fall counter := 0 and, if v ≥ max, status := 2, rise counter += 1; equal → nothing.
3. **`AF_CheckPeakCandidate`** = MinMax ratio ≥ threshold **and** (Start ratio ≠ 0 **or** reversed) **and** (fall counter ≥
   AccDecCntPeakTh (h.0x2c7e) + speed class **or** NowFake ratio ≥ NowFakeMaxAccRatioPeakTh (h.0x2c7f)) → peak flag h.0x299c := 1.
   Else **`AF_CheckSearchAllPos`** = reversed and at a range end (h.0xc4) — the whole range has been swept. Either → state := 5;
   target h.0xba := peak position h.0x2940 + h (pos < peak; clamped to max) or − h (otherwise; clamped to min), h = step/2 if step > 3
   else 1; `ResetLocalOneValue` + `UpdateLocalOneValue(sel_f)`; `MoveToTargetPos(h.0xba, step > 3 ? step/2 : 1)`.
4. Else **`AF_CheckWrongDir`** (only while not yet reversed): wrong = fall counter ≥ AccDecCntWrongDirTh (h.0x2c80) + speed class,
   or NowFake ratio ≥ NowFakeMaxAccRatioWrongDirTh (h.0x2c81), or the motor stands at the range end it is moving toward (user
   disassembly 0x1520c–0x15312; both decompiles garble this). Wrong → direction reversed, reversed := 1, second max h.0x295c := 0.
   Then `MoveByDirAndStep` (step capped at MaxMotorStep).

**`AF_FSM_Focused`** (state 5): `UpdateLocalOneValue(sel_f)` (the maximum is re-tracked with the focus selector while the lens
approaches); when pos = h.0xba: if pos = the re-tracked peak position h.0x2940 → settle counter h.0x29a0: non-zero → decrement and
wait; zero → counter := 2, focus snapshot `UpdateFocusAllValue(sel_f)`, h.0x90 := 0, state := 6 (8-byte constant {0, 6} at h.0x90,
0x254a8); else h.0xba := that peak position. Moves with `MoveToTargetPos(h.0xba, step > 3 ? step/2 : 1)`.
Meaning [I]: the lens overshoots the peak by half a step on the far side and comes back, then waits two frames before the result
counts — backlash and statistics-delay handling; the step is not reduced any further (no fine search in the live path, §4.7).

### 4.6 One-shot AF (`AF_ALGO_OneShot`, Algo = 0) [R user disassembly 0x15bf8]
- Phase A (h.0x2964 = 0): pos ≠ motor min → `MoveToTargetPos(min, MaxMotorStep)`; at min → h.0x2964 := 1, histories and local
  extremes reset, continue in the same frame.
- Phase B (h.0x2968 = 0): pos ≠ motor max → `UpdateHistOneValue`, `UpdateLocalOneValue(OneShot AccSel h.0x2c44)`, direction +1,
  step := OneShot MotorStep (h.0x2c48), `MoveByDirAndStep`; at max → focus-done h.0x2c88 := 1, h.0x2968 := 1, continue.
- Phase C: pos = the maximum's position h.0x2940 → nothing; else `MoveToTargetPos(h.0x2940, MaxMotorStep)`.
- The sweep runs once; `pAF_Trigger` (method +4, called by every AF setter) clears h.0x2964…0x2974 and restarts it. Done is reported
  when the sweep ends, before the lens is back at the peak. Debug bit 5 logs.

### 4.7 Unused branch: step-speed control, 4-point slope and wobbling [R: no caller]
`AF_StepSpeedCtrl`, `AF_FSM_Wobbling` and `AF_FSM_Check_AFaccState` are exported but **nothing calls them** in either copy: no
`bl` / PLT call and no PLT slot or other relocation names them in libispalgo (the helpers below do have PLT slots), and no call in the
kernel decompile [R: disassembly + `readelf -r`]. `AF_Calc4ptSlope`, `AF_LIB_BigRatio`, `AF_SlowDownStepBy1`, `AF_Step1Check`,
`AF_WobbingReturnPeak`, `AF_WobbingMoveBySlope`, `AF_WobbingMoveByMaxAcc` are reached only from those three, so the whole branch is
dead code; the fields it alone uses: h.0x90 (written by init / Focused, read only here), h.0x2c64…0x2c73, h.0x2c84, h.0x29a4.
Specified below as found. Defaults of its fields (§4.2): h.0x2c64…0x2c72 = u16 {24, 12, 26, 16, 13, 1, 2, 26}; h.0x2c82 = 0;
h.0x2c84 = 5; h.0x29a4 = 1; h.0x90 = 1.
- **`AF_Calc4ptSlope`** → h.0x29a2 (s16): over the last four positions p = h.0xae, 0xb0, 0xb2, 0xb4 and the SBLV history v =
  h.0x2918…0x2924 (fixed selector, whatever the API selects): p̄ = (Σp >> 2) & 0x3fff, v̄ = Σv >> 2, d_i = p_i − p̄, S = Σ d_i² (0 →
  1); slope = Σ ±`BigRatio`(|v_i − v̄|, d_i, S) (+ when v_i > v̄), clamped to ±3000 [R NEON disassembly 0x15508 + decompile]. I.e. the
  least-squares slope of focus value against position ×1 (no scaling). d_i is passed as an unsigned word, so a negative offset enters
  as a large number [R parameter type; I: the result for d_i < 0 is not a true regression term; not evaluated numerically].
- **`AF_LIB_BigRatio(a, b, c)`** = ⌊a·b/c⌋ without a 64-bit multiply: while a·b overflows 32 bits, halve c and the larger of a, b; a or
  b = 0 → 0; c = 0 → 0xFFFFFFFF.
- **`AF_StepSpeedCtrl`**: h.0x90 = 1 → nothing (so with the init value it is inert even if called); SBLV history not full (h.0x2924 =
  0) → step := h.0x2c70 (2) and the slow-down state h.0x297f / h.0x2980 := 0. Otherwise: slope s := `Calc4ptSlope`, slope history
  h.0x2976…0x297c shifted; trend t = Σ sign(p_k − p_{k+1}) over the three position pairs (−3…3); consistency bit f = 1 when (t > 2 and
  s ≥ 0) or (t < −2 and s ≤ 0), shifted into h.0x297e; growth base = 24 (h.0x2c64) while the fall counter h.0x2995 < 2, else 12;
  candidate = (((step + 1)·base) & 0xFFFFF) >> 4. |t| ≤ 2 → step := 2 (h.0x2c70); f = 0 or not two consistent frames in a row →
  `AF_Step1Check`; else a slope-band choice capped by the candidate: both |s| and |s_prev| < 31 → min(12 (h.0x2c66), cand); otherwise
  bands |s| < 61 → 26 (h.0x2c68), 61…100 → 16 (h.0x2c6a), falling slope (|s| < |s_prev|) → 13 (h.0x2c6c), with a slow-down mode
  h.0x297f entered when the step exceeds 11 and left after four `SlowDownStepBy1` calls (counter h.0x2980). Finally step := min(step,
  26 (h.0x2c72)). [R for the bounds and constants; I: the exact band order — both decompiles garble the if-chain and the result of the
  band choice appears overwritten by the slow-down call; the disassembly (libispalgo 0x15648–0x157f0) was not read for it.]
- **`AF_Step1Check`**: step := 1, counter h.0x2981 += 1; counter 13…16 → 2, 17…20 → 3, 21…24 → 4, > 24 → 5 and counter := 49.
- **`AF_SlowDownStepBy1(h, x)`**: x > 100 → step := min(step + 1, 4); else step < 5 → max(step − 1, 1), else 2 (x is the caller's
  second register, not set explicitly in the decompiles [I: |slope|]).
- **`AF_FSM_Check_AFaccState`** (would run in a "wobbling" state): slope := `Calc4ptSlope`; h.0x29a8 := (sum of the new and the three
  previous slopes) / 4 (toward zero); slope history shifted; counter h.0x29a5 += 1; h.0x2c82 = 0 → `WobbingMoveByMaxAcc`; else counter
  < h.0x2c82 → `WobbingMoveBySlope`, else counter := 0 and `WobbingMoveByMaxAcc`; then phase h.0x29a1 := 1, state := 6, h.0x29a6
  cycles 0 → 1 → 2 → 0xFF…, amplitude h.0x29a4 := (old h.0x29a6 < 2).
- **`AF_FSM_Wobbling`**: phase h.0x29a1 0 → 1 and 1 → 2 each with step := amplitude h.0x29a4; 2 → 3; 3 → 0 and state := 7; then
  `MoveByDirAndStep`.
- **`AF_WobbingReturnPeak`**: step := 2·amplitude, direction reversed, `MoveByDirAndStep`.
- **`AF_WobbingMoveBySlope`**: |slope h.0x29a2| < h.0x2c84 (5) → reverse direction, step := 2·amplitude; a steep slope (> 10)
  against the direction flips it; the function then **adds / subtracts 3·amplitude to h.0xaa, the current-position input field**,
  instead of setting the next position h.0xac; otherwise step := 0 and `MoveByDirAndStep` [R stores `strh [r0, #170]` at 0x15918,
  0x1595e; I: the branch order — the two decompiles disagree, the disassembly 0x158c0–0x15962 was not read for it].
- **`AF_WobbingMoveByMaxAcc`**: the position (h.0xae…0xb4) of the largest of the four **IIRH** history values h.0x28f8…0x2904 (later
  entries win only when strictly larger); phase := 0, state := 6; direction toward it; distance < 9 → **h.0xaa** := that position
  (again the current-position field, `strh` at 0x159d4), else `WobbingReturnPeak`.
- Writing h.0xaa would be overwritten by the next frame's input anyway (`AF_UpdateInputParameters`), so even if this branch were
  wired in, those moves would be lost [R overwrite; I consequence].

### 4.8 API ids 0x1C29…0x1C30 → AF fields [R: `DrvAlgo_IF_ApiSet/ApiGet` size checks and method slots, the setters]
<!-- ev:isp-5-3a.4.8 -->
Every Set checks the exact size (else status 0xE "Structure mismatch") and needs the AF handle (no handle: status 6, and the
caller's buffer is zeroed in part or whole). "Trigger" = method +4 `pAF_Trigger`: one-shot phases, histories, local extremes, both
per-window snapshots reset, focus-done := 0, state := 3 (search start). The hardware ids 0x1C01–0x1C09 (`pAF_SetHW*`) store their
struct (h.0x29b4…0x2b1b) and, in the user copy only, send it to the kernel (§1.4); the method slots of all AF ids: isp-3-iq-api.md
§4.4.

| id (size) | XML fields → object | checks | after the store |
|---|---|---|---|
| 0x1C29 AFAttr (16), `pAF_SetAttr` | +0 State → h.0x2b1c (≠ 0 pauses the algorithm), +4 Mode → h.0x2b20 (1 = manual position), +8 u16 ManualMotorPos → h.0x2b24 (+10 pad copied), +12 Algo → h.0x2b28 (0 one-shot, 1 continuous) | Mode = 1 needs motor min ≤ ManualMotorPos ≤ motor max, else −1 | Trigger |
| 0x1C2A AFMotor (8), `pAF_SetMotor` | u16 MinMotorPos, MaxMotorPos, MinMotorStep, MaxMotorStep → h.0x2b2c, 0x2b2e, 0x2b30, 0x2b32 | lens MinPos h.0xa2 ≤ Min ≤ Max ≤ lens MaxPos h.0xa4; 1 (h.0xa6) ≤ MinStep ≤ MaxStep ≤ lens range h.0xa8; else −1 | Trigger |
| 0x1C2B AFAccWeight (272), `pAF_SetAccWeight` | memcpy to h.0x2b34: +0 u8 WinNumX, +1 u8 WinNumY, +4 EqualWinWgtEn h.0x2b38, +8 WinWgt[16][16] h.0x2b3c, +264 IIRHBlendEn h.0x2c3c, +268 / +269 / +270 the three blend weights h.0x2c40…0x2c42 | WinNumY > 1 needs HW ROI mode (h.0x29b4) ≠ 0 (matrix) and WinNumY ≤ VerticalBlockNumber (h.0x29b8), else −1; WinNumX is not checked | Trigger |
| 0x1C2C AFOneShot (8), `pAF_SetOneShot` | +0 AccSel → h.0x2c44, +4 u16 MotorStep (+6 pad) → h.0x2c48 (u32) | MinMotorStep ≤ MotorStep ≤ MaxMotorStep, else −1 | Trigger only when Algo = one-shot |
| 0x1C2D SceneChange (24), `pAF_SetSceneChange` | six words to h.0x2c4c…0x2c63: +0 PreAfAccSel; +4…+11 PreAe DiffThOft / Slp, CntThOft / Slp, PreAf DiffThOft / Slp, CntThOft / Slp (bytes); +12 FocusAfAccSel; +16…+19 FocusAe DiffTh / CntTh, FocusAf DiffTh / CntTh; +20 StableCntTh (+21 pad) | none (the 0…100 XML ranges are not enforced; `CheckScene` caps the derived thresholds at 99) | Trigger only when continuous |
| 0x1C2E SearchStart (8), `pAF_SetSearchStart` | +0 u16 SearchMotorStep → h.0x2c74, +2 u16 SearchMotorDirByPosTh → h.0x2c76, +4 SearchAccSel → h.0x2c78 | MinMotorStep ≤ step ≤ MaxMotorStep; motor min ≤ DirByPosTh ≤ motor max; else −1 | Trigger only when continuous |
| 0x1C2F Search (6), `pAF_SetSearch` | bytes → h.0x2c7c…0x2c81: MinMaxAccRatioPeakThOft, …Slp, AccDecCntPeakTh, NowFakeMaxAccRatioPeakTh, AccDecCntWrongDirTh, NowFakeMaxAccRatioWrongDirTh | none | Trigger only when continuous |
| 0x1C30 (8, Get only) | `pAF_QueryInfo`: +0 u32 focus-done h.0x2c88, +4 u32 h.0x2c8c (u16 current position + u16 h.0x2c8e, never written = 0) | — | — |

- AccSel values 0…3 select IIRH, IIRL, SBLV, SBLH (§4.3); values 4 (luma) and above are accepted by every setter and map to luma (4)
  or IIRH (≥ 5) in `AF_Acc_GetAccPtr` kind 0, to IIRH in kind 1.
- Method +8 `pAF_ForceTrigger(h, &flag)` (flag ≠ 0 → Trigger, flag := 0) has no API id: neither switch routes to slot +8.
- In libispalgo's id switches Set 0x1C2B calls method +0x64, Get 0x1C2B
  method +0x68 and Get 0x1C30 method +0; the other AF Get rows marked "inline" there were not rechecked [I: also method calls].

### 4.9 What the AF does on this camera [R code chain; I: runtime inputs]
<!-- ev:isp-5-3a.4.9 -->
- The AF event [R chain; I: that the simple-Bayer path is not taken on this pipeline and the CUS3A bypass mask stays 0]: prog_uvc
  `ST_VideoModuleInit` fills the 0x50-byte `MI_ISP_CreateChannel` attribute with only word 0 = 1 << sensor pad (pads 0…3 → 1 / 2 / 4 /
  8) and zeros (`memset` of 0x50 bytes), so the application sets no control
  bits. ispmid `CameraIsp_InitChannelFmt` (called from `MhalCameraOpen`) sets the channel's `nCtrlFlag` (ch+4): if the per-channel
  override word `g_tIspMidHandler[ch + 0x7c]` has bit 31 set, its low 31 bits (bit 31 is then cleared); else on the channel's first
  open (ch+0x19778 = 0) **0x7f** (IQ, menuload / WDR, AE, AWB, AF all on); else it keeps the old value; then YUV input modes
  (attribute word 5 = 4 / 5) → 10, ch+0x19730 = 0 → AE bit 4 cleared, no bound sensor → bits 4–6 cleared
 . `CameraIsp_InitIspAttribute` clears bits 4–6 (and the CUS3A bits) on its
  "nIspSimpleBayerMode" path. The only writer of the override word is `CameraDebugSetControlFlag(ch, bit, on)`, reached
  from `ispmid_fwrite` for the text command "ctlflag <ch> <bit> <on>" on the ispmid debug file; the string "ctlflag"
  occurs in no file of the firmware except mhal.ko, so no shipped program or script issues it [R: string search].
  `CameraIsp_EnableChannel3A(ch, nFlagCus3A)` sets ch+0x196c4 bit 4 (user AF event) only if nFlagCus3A bit 4 is set, the CUS3A bypass
  mask `g_tIspMidHandler+0x33c` bit 2 is clear, and `nCtrlFlag` bit 6 is set; libcus3a's `CUS3A_EnableUserspaceAF` sends 0x2E08
  `Cus3AEnable_t` with AF = 1 → nFlagCus3A bit 4 [S isp-6-userspace.md §1.1]. The bypass mask is changed only by `CameraIspSetCus3ABypass`,
  whose callers were not traced [I: default 0]. So for prog_uvc's realtime Bayer channel with a bound sensor `nCtrlFlag` = 0x7f, the
  AF bit is set, and the user AF runs every frame. Not confirmed on the camera: ispmid's log line "nCtrlFlag = 0x%x" (printed by
  `MhalCameraOpen` when the ispmid log mask enables it) would show it (docs/firmware/camera-checks.md K26).
- The inputs: init MinPos = MaxPos = CurPos = 0 (the 0x2E20 status read fails and leaves zeros), the current position in every `DoAf`
  input = 0 (no VCM reports a position [I]), the default motor range 320…700. The position 0 lies outside the motor range, so
  `AF_ALGO_Continuous` takes step 1 on every frame: `MoveToTargetPos(clamp(0, 320, 700) = 320, 50)` → 0 + 50 = 50 is below the motor
  minimum and is clamped to 320. The output is Change = 1, NextPos = 320 on every frame after the 30-frame start delay; the FSM (scene
  check, search, focus) never runs, while the per-frame accumulator and history updates still do. `pAF_SetMotor` cannot repair this:
  with lens limits 0…0 its step check 1 ≤ MinMotorStep ≤ MaxMotorStep ≤ lens range (0) can never pass, so every 0x1C2A Set returns −1.
  0x2E21 is written every frame and nothing moves (no VCM).
- `MI_ISP_GetIspRoot` [R + S]: API id 0x66 is routed to the kernel; the mhal ISP API switch has no case for it (isp-3-iq-api.md §2);
  the one way it could still be served, a command filter registered through `MS_CAM_RegIspCmdFilter`, is never set up on this
  firmware: the function only stores a pointer pair, nothing inside mhal.ko calls it, and no other file of the firmware
  (modules, libraries, applications) contains the symbol name [R: string search; the compressed kernel image is not searchable this
  way, I: built-ins do not call module symbols]. So `MI_ISP_GetIspRoot` fails on every call and libcus3a loads the 3A IQ file from its
  fallback directory `/config/iqfile` (isp-6-userspace.md §2.8).

## 5. What is not known
Where an open point belongs to one section it is stated there; this list gathers them.
- **Configuration on this camera**: whether the camera's API bin / IQ file selects AWB AlgMode 3, AdvMode 1, MultiLS, AWB or AE
  day/night detection, Strategy-EX, FlickerEx or a different AF Algo was not read (the built-in defaults: §2.17, §3.12, §4.2);
  whether AdaptiveGamma is enabled (§2.11).
- **AE**: the unit of the fps input of the run-time flicker detector (§2.12); the field names of the AE's 0x1418 struct (§2.15, from the XML only by shape); the
  roles of the fields without a reader and of h.0x18ec, and the kernel `AE_InitParameters` not emulated itself (§2.17); the divisor
  33909 (§2.11); a NULL cfg handle in the user build (§2.4). The debug-bit-4 (linear-E) plain-value branches of
  `pAE_CalApexAuto2` and `pAE_CalIntpApex(Short)` are known by what they write (§2.8), not formula by formula; the kernel and user
  copies of `pAE_CalApexAuto2` were compared on the APEX path only (the user decompile merges the FurtherLimitOnAperture branches;
  the kernel one was used).
- **AWB**, known only at the level of §3.4–§3.5: `pAWB_isConverged` with the stabilizer, `pAWB_GetLightGain_Normal` / `_Balance`,
  `pAWB_InitCaliParam` / `pAWB_CaliLightArea` / `pAWB_GetAreaOffset` / the verify and `pAWB_ApplyUnitCalibration` functions,
  `pAWB_SetBlockValidSize`, and the debug prints `pAWB_ShowDebugLog` / `pAWB_ShowApiParameters`; the field layouts of the user-only
  ids 0x2C03–0x2C06 (§1.4); the kernel copy of `pAWB_MultiLS_TargetRatioCalc` (§3.8); the `AWBDEBUG_t` field names (§3.1).
- **AF**, dead code only: the step-speed control, `AF_SlowDownStepBy1`'s argument, `AF_WobbingMoveBySlope`'s branch order and
  `AF_Calc4ptSlope` with negative offsets (§4.7).
- **Copies**: the class of the 78 functions that differ only in constants rests on a sample (§1.3).
- **Camera confirmation** of the AF chain: `nCtrlFlag` = 0x7f, the CUS3A bypass mask staying 0, and the callers of
  `CameraIspSetCus3ABypass` (§4.9).
