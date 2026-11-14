# isp-4-iq-engine — the ISP IQ engine of mhal.ko
<!-- ev:isp-4-iq-engine -->

The IQ engine is the part of the ISP code in `mhal.ko` that turns the
IQ file's tuning tables and the API structs set through the ISP API into register values. It keeps a per-channel IQ context;
on each IQ run it picks a pair of ISO rows by the total gain and a pair of colour-temperature rows, blends every parameter of
the IPs (ISP sub-blocks) whose trigger fired, applies special effects and per-IP patches, and writes the result through the
table `Regmap` into a shadow image of the ISP banks that ispmid copies to the hardware; lookup tables go to menuload staging.
This document owns: the IQ context and the per-frame input record `IqInput`, the IQ debug block, the run order and the trigger
machinery, the per-IP runner `hal_IQ_Set_ALL`, the API-slot layer (`hal_IQ_API20_*`) and the initial API state, CCM / HSV,
calibration apply, the `IQ_Patch_*` patches, the register writers (`WriteRegs`, `IspRegSet*`, `IspMLoadSet*`), the shadow id →
`HalIsp_Apply*` map, the IQ debug command word (register 1321.7E) and the YUV pipe modes of the engine, `Regmap` with the
official parameter names, the IQ file format and the built-in `iq_bin`. Not here: the ISP API ids, their payloads and each
`IQ_API20_SET_*` converter that has an ioctl id, the Effect table, the API bin — isp-3-iq-api.md §2, §3, §6; the in-kernel AE / AWB / AF
(including the AWB internals behind ispmid ch+0x124) — isp-5-3a.md; ispmid (channel data, per-frame flow, CMDQ), the
statistics engines and their payloads, MLOAD (staging formats, SRAM ids) — isp-2-ispmid-mload-ispscl.md §2.2, §6, §7, §10, §12; the ISP register bank
list, the `HalIsp_Apply*` functions themselves, `ISP_IQ_Init_Cfg` and the 131D.45–.47 kernel table, the NE engine —
isp-1-hal.md §9, §10.4, §10.3, §8.3; libcus3a, the user 3A and `isp_api.xml` — isp-6-userspace.md §2, §1.1, §8; the file inventory rows — firmware-image.md §3.

**Conventions.** Origin tags: [R] read from the binary, its disassembly / decompile / emulation of its code, a data section or
a cited file; [S] stated by a source; [I] inference. A tag on a heading covers the section unless a line says otherwise.
Addresses are `.text` / `.data` / `.rodata` / `.bss` offsets of the relocatable module (Ghidra address = `.text` + 0x40000028).
"ctx" = the per-channel IQ context (§2.1); "ip" = an IP slot number 0…49 (§5.3); "slot" (API) = an API slot number 0…52 (§6.1);
"enum" = a cfg enum 0x000…0x42f, the index shared by `Dbg_Items`, `Regmap` and the IQ file (§12, §13). Register notation
`BANK.rr[bits]` = RIU bank, 16-bit register rr (the RIU addressing: kernel-platform.md §3.1); in the shadow image a register r of a
bank lies at bank base + 4r (§10.1). "ch" = an ispmid channel data pointer (isp-2-ispmid-mload-ispscl.md §2.2). "This camera"
= GC8613 Bayer RAW over MIPI, ISP pipe mode 0, user AE / AWB (`nFlagCus3A` = 0x1A), IQ file = the imx307 file (§13.4).

## 1. The IQ engine in mhal.ko

### 1.1 Place and inventory [R sizes, counts; grouping I]
The IQ engine is one of the eight sub-parts of the ISP block of mhal (the block map: isp-1-hal.md §1.1): 273 functions, 80 536 B
of code, `.text` 0x5f8e0–0x61c84 (core: `pIQ_*`, `IQ_*`, `cfg_*`, interpolation helpers, `DoIq` / `DoIqPerFrame` / `IqInit` /
`IqRelease`) and 0x66d44–0x78488 (API layer, per-IP code, register writers). "Stub" = a 4-byte `movs r0,#0; bx lr` (or a 2-byte
`bx lr`); "unref" = no code caller, no function-pointer use and no data reference anywhere in mhal.ko (the compiler dropped
the calls to empty functions, so every stub is also unreferenced).

| group | functions | bytes | stubs | unref | section |
|---|---|---|---|---|---|
| entry points (`DoIq`, `DoIqPerFrame`, `IqInit`, `IqRelease`, `IspAlgoReadIqBinH`) | 5 | 416 | 0 | 0 | §3.1 |
| method table (`pIQ_*` except calibration) | 10 | 534 | 0 | 0 | §3.2 |
| core state / triggers (`IQ_*` state machine, ISO / CT index, debug hooks) | 28 | 3 880 | 0 | 8 | §4 |
| IQ-file access (`Cfg_*`, `cfg_*`, `IQ_Get/Set_cfg*`) | 14 | 1 070 | 2 | 2 | §13.3 |
| math helpers | 23 | 3 540 | 1 | 6 | §14 |
| hal_IQ core (`hal_IQ_Init*`, `hal_IQ_Set_INIT/ALL`, `hal_DoIQ*`) | 7 | 15 000 | 0 | 0 | §3, §5 |
| API tables (`hal_IQ_API20_*`) | 6 | 9 968 | 2 | 1 | §6 |
| API converters (`IQ_API20_SET_*`, `LutGen_GAMA_EFFECT`) | 48 | 14 884 | 7 | 7 | §6.1 (the ones with an id: isp-3-iq-api.md §3.3) |
| CCM / HSV / effects / init / ITP | 18 | 6 816 | 0 | 1 | §7 |
| calibration apply (`pIQ_Cali_*`) | 4 | 2 312 | 0 | 0 | §8 |
| per-IP patch (`IQ_Patch_*`) | 44 | 6 044 | 27 | 27 | §9 |
| register writers (`WriteRegs`, `IspRegSet*`, `IspMLoadSet*`) | 57 | 10 728 | 16 | 16 | §10 |
| debug forcing (`IQ_IP_*_ALL`, `IQ_Debug_Log`) | 5 | 5 092 | 0 | 0 | §11.1 |
| YUV pipe modes (`IQ_Yuv*`) | 4 | 252 | 0 | 0 | §11.2 |
| **total** | **273** | **80 536** | **58** | **68** | |

Unreferenced functions with a body (not stubs): `IQ_SetIQParaInitStatus`, `IQ_Property_Set/Get`, `IQ_CCM_smooth`,
`IQ_Debug_FixISO`, `IQ_Set_LookUp`, `IQ_Get_LookUp` (returns −1), `IQ_ParamGetSwitch` / `IQ_SetParamToCfg` (return −1),
`Interp_LUT_IQ_P`, `Int2TwoComp`, `twos_complement_to_inter`, `IQ_InterpXBase`, `MinMaxNormalization`, `IQ_Set_cfgdata` (and
`hal_IQ_API20_API2IP`, a stub). Each is specified with its group.

Exports [R `__ksymtab`]: `DoIq`, `DoIqPerFrame`, `IqInit`, `IqRelease`, `IspAlgoReadIqBinH`, `Cfg_InitDbgBuf`, `Cfg_GetDbgBufSize`,
`cfg_InitBinInfo`, `cfg_ReleaseDBGInfo`, `cfg_setcfgbyenum` — all called only from ispmid (`CameraIspAlgo_*`) inside mhal; no
other module imports them.

### 1.2 Path of a parameter [R]
1. A Set of an IQ id (isp-3-iq-api.md §3.2) copies the struct into the API-struct region of ctx and sets one of 53 API dirty
   flags.
2. The next `DoIq` runs the slot's converter `IQ_API20_SET_<X>`, which fills per-ISO **API tables** of the same layout as the
   IQ-file tables, and switches the IPs fed by that API to API mode (§4.3).
3. `hal_IQ_Set_ALL(ip)` blends two rows of the IQ-file table or of the API table into the IP's **output block**, then
   `IQ_DoEffect`, `IQ_Patch_<IP>` and `IspRegSet<IP>` (§5).
4. `IspRegSet<IP>` writes each cfg enum through `Regmap` into the **shadow image** (0x8710 B per IQ slot of a channel) and
   sets the shadow id's dirty byte `shadow+0x4d20+id`; tables go to menuload staging through `IspMLoadSet<IP>` (§10).
5. ispmid copies the dirty shadow parts to the banks with `DrvIsp_ApplyShadow(id)` → `HalIsp_Apply<IP>` (via the ISP's own
   CMDQ at frame start) and loads the dirty menuload tables (isp-2-ispmid-mload-ispscl.md §6, §12).

The IQ file is the default for every IP parameter, per ISO step; the API only overrides it.

### 1.3 On this camera [R unless tagged]
- The kernel loads the **imx307** IQ file (`/config/iqfile/iqfile0.bin` → `imx307_iqfile.bin`) for the GC8613 sensor (§13.4);
  the app's API bin (isp-3-iq-api.md §6) then overrides 42 API groups, so every IP whose API is not in the bin runs on imx307
  tuning [R files; consequence I].
- 20 IPs have interpolation on in the imx307 file, so an IP re-run happens on a total-gain change of more than 194/1024
  (≈ 19 %) — not on an ISO-index change — or on a colour-temperature change; LSC / ALSC switch tables with a hysteresis; IP 20
  WDRLOC runs every frame and IPs 6 / 7 OBC on every gain change (§4.1, §4.3, §3.5).
- The in-kernel AWB does not run (user AWB), so the matrix the CCM is multiplied with stays the identity (§2.2, §7.3).
- Pipe mode 0 (raw Bayer) [I: GC8613 raw over MIPI, ch+0x199fc = 0]: neither YUV pipe mode of §11.2 applies.

## 2. The IQ context and its inputs

### 2.1 IQ context regions [R offsets; names I unless official]
ctx = `CamOsMemCalloc(1, 0x58040)` per channel (`IqInit`, §3.1); it is the "IQ handle" `DrvAlgo_IF_ApiSet` receives. Fields
used by one function only are given with that function.

| ctx offset | what |
|---|---|
| +0 / +2 / +0x104 / +0x144 / +0x184 / +0x185 | IQ_INFO copies from the IQ file (§3.3 `IQ_GetParamFromCfg`): `CHIP_VER`, IR-ISO remap table (16 × 16 u8, +2…+0x101), `GAIN_TBL0`, `GAIN_TBL1` (16 × u32 each), `EXTEND_ISO_IND`, `ISO_IND` (+0x185 is rewritten with the index in use, §4.1) |
| +0x186 + off[e] | the 88 INIT enums (§5.1) |
| +0x2c2 … +0x1a0d2 | **IQ-file tables**: per IP, 16 rows (one per ISO step; LSC / ALSC: 3 rows, one per colour-temperature step; CCM / HSV: rows by colour temperature, §7.1; DECOMP, GAMA_16to12, SDC, R2Y, COLORTRANS: 1 row) of that IP's parameters, loaded from the IQ file (§5.3 "file table") |
| +0x1a1d8 | gain-table value of the current ISO row (§3.3) |
| +0x1a1dc … +0x1ca1x | **IP output blocks**: the blended parameter set of each IP for the current run, handed to `IspRegSet<IP>` (§5.3 "out block") |
| +0x1ce40 … +0x36d50 | **API tables**: the same layout as the IQ-file tables, filled by the converters from the API structs (§5.3 "API table"); the API tables are not in IP order |
| +0x36d54 … +0x5359c | **API structs**: the last Set of every IQ id, byte-identical to the user struct (isp-3-iq-api.md §3.2) |
| +0x535b8 + 4k (k = 0…52) | API dirty flags (Set handler writes 1; cleared by `IQ_Trigger_API`) |
| +0x5368c + 4k | per-API "bypass" value (written by id 0x2803; feeds the IP mode, §4.3) |
| +0x53770 / +0x53774 | pointers to the IR-ISO remap table (ctx+2) and to the gain table in use (§4.1) |
| +0x53778 / +0x5377c / +0x53780 / +0x53788 / +0x5378c | ISO index used / gain used / previous index / plain ISO index 0…15 / remap block (§4.1) |
| +0x53790 / +0x53794 / +0x53798 | CT index / previous and target ALSC table index (§4.2) |
| +0x5379c / +0x537a0 / +0x537a4 / +0x537a8 | colour temperatures (§4.1 step 8) |
| +0x537ac … +0x537e8 | 16 colour-temperature thresholds (§4.1 step 7) |
| +0x537ec / +0x537f0 / +0x537f4 | the three ALSC table CCTs (§4.2) |
| +0x537f8 | 16-entry identity map [I: ISO / CT index → table row] |
| +0x5383c | CCM-smooth history (§4.2) |
| +0x538bc / +0x538bd | CT-threshold copy flags |
| +0x538c0 / +0x538c4 / +0x538c8 | gain at the last trigger / current total gain / u16 IR-block input |
| +0x538cc | first-run flag |
| +0x538d0 / +0x538d1 / +0x538d2 / +0x538d3 / +0x538d4 | fast mode / force-ISO on / forced ISO index / force pending / debug IP byte (0x32) |
| +0x538dc | IQ mode (0 day, 1 night) |
| +0x538e4 + off[e] | the 31 ITP records (§5.1); CCM's record (0x41f) at +0x53a76, HSV's (0x420) at +0x53a7e |
| +0x53b80 / +0x53b84 | number of IPs with interpolation on / 0xc2 (the gain-change threshold, §4.1 step 5) |
| +0x53b88 | pointer to `IqInput` (§2.2) |
| +0x53b8c / +0x53b90 / +0x53b94 | shadow image (0x8710 B) / shadow+0x4d7c / menuload staging (shadow+0x4d80) |
| +0x53b98 | handle of the parsed IQ file (§13.3) |
| +0x53b9c … +0x53bd0 | method table (§3.2) |
| +0x53bd8 … +0x54000 | NR3D noise-profile / motion-threshold state (§3.3, §9.2) |
| +0x540c0 / +0x542c0 | contrast S-curves Hi / Lo (256 × u16, §3.3) |
| +0x544c4 / +0x544c8 | a 1-byte buffer nobody reads [R: no other reference in the sub-part] / 0 |
| +0x544cc … +0x544d7 | debug bytes copied from the IQ debug block (§2.3, §4.4) |
| +0x544d8 + 4·ip | per-IP trigger class (§4.3) |
| +0x545a0 / +0x545d2 / +0x54604; +0x54636 / +0x54637 / +0x54638 | IP lists 1 / 2 / 3 and their counts (§4.3) |
| +0x5463c + 4·ip | IP present (0 → `hal_IQ_Set_ALL` returns at once) |
| +0x54704 | 50 bytes [I: per-IP "ran this frame" flags], cleared at init |
| +0x54738 + 4·ip | IP mode: 1 = load from the IQ file (only until the first run), 0 = run from the IQ-file tables, 3 = run from the API tables; no writer stores any other value (§4.3) |
| +0x54800 + 4·ip | force word per IP (`pIQ_IQTrigger`, §3.2) |
| +0x548c8 + 4·ip | trigger bits: 1 = list 1 (mode 1, ISO and CT), 2 = list 2 (mode 2, ISO), 4 = list 3 (mode 3, CT), 8 = single IP, 0x10 = API, 0x20 = init, 0x40 = forced |
| +0x54990 + 4·ip | frame count of the IP's last run (an IP runs at most once per frame) |
| +0x54fec… | per-parameter interpolation flags of the IPs (u32 arrays, §5.2); CCM's at +0x5564c (6 words), HSV's at +0x5566c (3 words) |
| +0x55b18 + 4k | result of `hal_IQ_API20_EnGet(k)` (§6.1) |
| +0x55bec / +0x55bf4 | mirror / flip mode; vertical flip (§3.2) |
| +0x55bf8 / +0x55bfa | stored input width / height (§4.2) |
| +0x55bfc | frame count (IqInput+4) |
| +0x55c00 / +0x55c04 | gain / shutter at the last WDRLOC exposure-ratio computation (initial 0x400 / 300, §10.2) |
| +0x55c08 | IQ parameter-init status |
| +0x55c0c / +0x55c10 | shadow+0x8650 / shadow+0x8651 |
| +0x55c14 / +0x55c15 | ALSC grid width / height (§8, §9.3) |
| +0x55c18 … +0x55c2d | ALSC_ADJ state (§9.3) |
| +0x55c30 / +0x56430 / +0x56c30 / +0x57430 | ALSC_ADJ line buffers VR / VB / HR / HB (§9.3) |
| +0x57c30 / +0x57c32 | debug print mode / last debug code (§11.1) |
| +0x57c34 / +0x57c35 / +0x57c36 | temperature state (§4.2) |
| +0x58038 | 1 = pipe mode is not 4 (§3.1) |
| +0x5803c | IQ version "vendor" word (§3.2) |

### 2.2 `IqInput` — the per-run input [R]
<!-- ev:isp-4-iq-engine.2.2 -->
ispmid's `g_IspAlgoInput+0xa0`, 0x84 bytes, filled by `CameraIspAlgo_DoIq` before every `DoIq` from the channel data;
`DoIq` is only called when ch+4 (`nCtrlFlag`) has bit 0 or 1 set.

| offset | field |
|---|---|
| +0x00 | size 0x84 |
| +0x04 | frame count |
| +0x08 | BV (ch+0xa8) |
| +0x0c | ISP digital gain (ch+0x98) |
| +0x10 | ch+0xa4 |
| +0x14 | total gain = (sensor gain ch+0x94 × ISP gain ch+0x98 + 0x200) >> 10 (1024 = 1×) |
| +0x18 | shutter (ch+0x90) |
| +0x1c, +0x20 | temperature, constant 36 |
| +0x24 | colour temperature (ch+0x14e) |
| +0x28 / +0x2c / +0x30 | WB gains R / G / B (ch+0x146 / +0x148 / +0x14c) |
| +0x34 | 0x400 |
| +0x38 | u16 0 (the IR-block value, §4.1 step 4) |
| +0x3c / +0x40 | width / height |
| +0x44, +0x48 | 16 |
| +0x4e | u16 adaptive-gamma weight = ch+0x114 = the in-kernel AE's output +0x8c = GMBlendRatio + 1 (isp-5-3a.md §2.2); on the user-3A path the AE does not feed it |
| +0x50 | pipe mode: 4 = simple-Bayer / YUV→Bayer front, 5 = YUV sensor, 3 = RGB-IR, 1 = HDR pipe, 2 = ch+0x19780 in 1…4, else 0 |
| +0x54 | pointer to the AE block statistics (1024 blocks × 4 B {R, G, B, Y}, row-major, 32 blocks per row) |
| +0x58 | pointer to the IQ debug block (shadow+0x8660 of the active shadow slot, §2.3) |
| +0x60 | CCM re-run request = ch+0x150 |
| +0x64…+0x75 | nine u16 copied from ch+0x154…+0x165: the matrix the CCM is multiplied with (§7.3) |
| +0x78 / +0x7c / +0x80 | pointers to ch+0x19c80 (the NE block, §9.2), ch+0x19e80, ch+0x19e90 (HDR alignment values) |

**ch+0x150 / ch+0x154 — the in-kernel AWB's output** [R]:
ispmid passes ch+0x124 as the third argument of `AwbInit` (at `CameraIspAlgo_3A_IQ_Init`) and of every `DoAwb`; the AWB keeps
it at handle+0xac and writes through it. With `AwbOutput_t` (isp-5-3a.md §3.1): ch+0x146…+0x14e = out+0x22…+0x2a (gains, CT),
ch+0x150 = out+0x2c (u32 CCM-update flag), ch+0x154…+0x165 = out+0x30…+0x41 (9 × u16 CCM, row-major, 13-bit two's complement,
1024 = 1.0). ispmid only reads them; `CameraSetAwbParam` (0x2E0A, user AWB) writes ch+0x146…+0x14e only. The only two writers:
1. `pAWB_InitParameters` (from `AwbInit`, once per channel open): out+0x2c := 0; out+0x30… := identity {1024, 0, 0, 0, 1024,
   0, 0, 0, 1024} (`.rodata`+0xabe4); MultiLS state: enable h.0x1324 := 0, h.0x1328 (API name Sensitive; limits the peak ratio
   V1/V2) := 5, h.0x1329 (API name CaliStrength; scales the target ratios) := 100 (u16 0x6405 over both bytes), reference CCM 1
   h.0x132a := {512, 384, 128, 0, 1024, 0, 52, 102, 870} (`.rodata`+0xac38), reference CCM 2 h.0x133c := {870, 102, 52, 0, 1024,
   0, 128, 384, 512} (`.rodata`+0xac5c), force h.0x1354 := 0, target and current ratios h.0x135a…h.0x135f := 0. Reference 1
   mixes red strongly (row 0 = 0.5 R + 0.375 G + 0.125 B) and blue mildly, reference 2 the reverse (rows 0 and 2 swapped
   mirror-wise); every coefficient ≥ 0, row sums 1024.
2. `pAWB_MultiLS_CaliCcmCalc`, called at the end of `pAWB_MultiLS_Check`, which `pAWB_Process` calls only when AlgMode
   (h.0x5a0) = 3; `pAWB_Process` runs from `DoAwb` only in API state 0 (auto) with valid statistics. Per run:
   `pAWB_MultiLS_RatioSlowChange` first — previous (h.0x135e/f) := current (h.0x135c/d); each current s8 ratio (R, B) moves
   towards its target (h.0x135a/b, from `pAWB_MultiLS_TargetRatioCalc`; 0 when MultiLS is disabled or fewer than two peaks)
   by 3 if they differ by more than 2, else takes the target. Then out+0x2c := 0. Row 0 (out+0x30…+0x34) is recomputed when
   force = 1 or current R ≠ previous R: v = current R; v = 0 → {1024, 0, 0}; v < 0 → c_i = `pAWB_Ccm_Inter`(|v|, 0…100,
   identity[0][i], ref1[0][i]); v > 0 → the same with ref2[0][i]; out+0x2c := 1. Row 2 (out+0x3c…+0x40) likewise from current
   B (h.0x135d) with identity row 2 {0, 0, 1024} and row 2 of ref1 (h.0x1336) / ref2 (h.0x1348); out+0x2c := 1. Row 1 is never
   written after init (stays {0, 1024, 0}). Finally force := 0. So the flag is 1 exactly on the AWB runs where a row changed.
   `pAWB_Ccm_Inter(dbg, flag, x, x0, x1, y0, y1)` (`.text` 0x5cd28) decodes y0 / y1 as 13-bit (> 0x1000 → −0x2000); returns raw
   y1 when flag ≠ 1, y0 = y1 or x > x1, raw y0 when x < x0, else `_InterpXBase` = round(y0 + (x − x0)·(y1 − y0)/(x1 − x0)) in
   signed arithmetic (scale 1024, +512) [R: disassembly + emulation, 1024 → 512 at x 50 =
 768, at 33 = 856]. **Defect** [R]: the interpolated value is a signed int stored as u16, i.e. 16-bit two's complement (−64 →
   0xffc0), while `IQ_CCM_multiplier` decodes the matrix as 13-bit, so a negative blended coefficient is read as a large positive
   one (0xffc0 → 57280 ≈ 56×); unreachable with the default references (no negative coefficient), reachable after API 0x1804
   sets a reference with negative entries. API 0x1804 WBMultiLSAttr Set (in the kernel module): enable := (word ≠ 0); Sensitive :=
   clamp(1…10); CaliStrength := min(100); ref1 / ref2 9 × u16 each clamped to ≤ 0x1fff; force := 1. How the target ratios
   come from the light-area peaks (`pAWB_MultiLS_TargetRatioCalc`) is AWB-internal (isp-5-3a.md §3.8).

On this camera `nFlagCus3A` = 0x1A (user AWB bit 3 set), so ispmid never calls `DoAwb` (gate in `CameraIspAlgo_DoAwb`:
`nCtrlFlag` bit 5 set and CUS3A bit 3 clear); ch+0x150 stays 0 and ch+0x154 stays the identity written by `AwbInit` for the life
of the channel; the user AWB's result (`CusAWBResult_t`, 24 B) carries no matrix [R].

### 2.3 The IQ debug block [R]
0x98 B = 38 words per shadow slot at shadow+0x8660, initialised by `CameraIspAlgo_3A_IQ_Init` from `.rodata+0xc808`. Any word
can be set through the `/dev/ispmid` text command `dbgiq <ch> <word> <value>` → `CameraDebugIQ`, which accepts word < 38; the
parser is `sscanf("%s %d %d")`, so `<value>` is never parsed and the word gets a stale stack value [R decompile of
`ispmid_fwrite`].

| word (offset) | default | read by | meaning |
|---|---|---|---|
| 0 (+0x00) | 1 | `DoIq` (byte) | IQ engine on; 0 = `DoIq` does nothing |
| 1 (+0x04) | 0 | `IQ_Debug_Check` | 1 = debug prints on (ctx+0x544cc / +0x544d0 / +0x544ce) |
| 2 (+0x08) | 0 | `IQ_Debug_Check` | IP number for the debug trigger (ctx+0x544d1 / +0x544d2) |
| 3 (+0x0c) | 0 | `IQ_Debug_Check` | ctx+0x544d4 / +0x544d5 (read by `IQ_Debug_Log`, §11.1) |
| 5 (+0x14) | 16 | `IQ_Debug_Check` | ctx+0x544d7 (§11.1) |
| 6 (+0x18) | 0 | `IQ_Debug_Check` | 1 = run the debug trigger and API reset in this `DoIq` |
| 7 (+0x1c), 8 (+0x20) | 0, 16 | `IQ_Debug_FixISO` (unreferenced) | force-ISO on, forced index (> 15 = keep current) |
| 9 (+0x24), 10 (+0x28) | 0, 0 | `IQ_Debug_TriggerIQ` | trigger on (1) / off (0); trigger mode for `pIQ_IQTrigger` |
| 13 (+0x34) | 0 | `IQ_Debug_Check` | ctx+0x544cd (§11.1) |
| 14 (+0x38) | 0 | `IQ_Debug_Check`, `IQ_Debug_ApiReset` | API slot to reset (< 53), ctx+0x544d6 (§11.1) |
| 30 (+0x78) | 10 | — | not read by the IQ engine |
| 31…35 (+0x7c…+0x8c) | 45, 63, 71, 150, 200 | `IQ_ISOIndexGet` (u16 each) | thresholds of the IR "block" index (§4.1 step 4) |
| others | 0 | — | not read by the IQ engine |

## 3. Entry points, method table, initialisation, run order

### 3.1 Entry points [R]
- **`IqInit(cfg, input, shadow)`** → ctx = `CamOsMemCalloc(1, 0x58040)`; ctx+0x53b88 = input, +0x53b8c = shadow, +0x53b90 =
  shadow+0x4d7c, +0x53b94 = shadow+0x4d80 (menuload staging), +0x53b98 = cfg (the parsed IQ file, §13.3); ctx+0x544c4 =
  `CamOsMemCalloc(1, 1)`, ctx+0x544c8 = 0; ctx+0x55c08 (IQ init status) = 0 → `IQ_InitFuncPtr` (§3.2) → `IQ_InitParameters`
  (§3.3) → +0x55c08 = 1. Returns ctx.
- **`IqRelease(ctx)`**: frees the 1-byte buffer and ctx.
- **`DoIq(ctx, input, shadow)`** (per IQ run, from `CameraIspAlgo_DoIq`): stores input / shadow pointers as in `IqInit` and also
  ctx+0x55c0c = shadow+0x8650, +0x55c10 = shadow+0x8651; ctx+0x55bfc = input+4 (frame count). Runs only if the byte at
  `*(input+0x58)` is non-zero (word 0 of the IQ debug block, default 1). Then:
  1. `f = IQ_Debug_Check` (§4.4); if 1 → `IQ_Trigger_Froce`.
  2. `a = IQ_API20_Trigger_CHK`; if 1 → `IQ_Trigger_API` (§4.3).
  3. `m = IQ_ISOIndexGet` (§4.1); `IQ_ResolutionCheck` = 1 → m = 1. If f ≠ 1, a ≠ 1 and m = 0 → m = 8 (nothing to do).
  4. ctx+0x58038 = (input+0x50 ≠ 4) (pipe mode not YUV→Bayer); `IQ_ParamSetSwitch(m, 0x32)`; `hal_DoIQ` (§3.4).
  5. u16 shadow+0x8652 = u16 ctx+0x1b153 = `IQ_HDR_SENSOR_EXP_RATIO` of the HDR output block, handed back to ispmid.
- **`DoIqPerFrame(ctx, input, shadow)`**: stores the pointers and the frame count, then `hal_DoIQPerFrame` (§3.5).
- **`IspAlgoReadIqBinH(dst, n)`**: `memcpy(dst, iq_bin+0x28, n)`, returns 0x98df (the size of the embedded file image including
  its 8-byte header) — the built-in imx307 fallback (§13.5).

ispmid calls `CameraIspAlgo_DoIq` at phase 0 of its 3-frame cycle and `CameraIspAlgo_DoIqPerFrame` every frame
(isp-2-ispmid-mload-ispscl.md §6.1).

### 3.2 Method table (`IQ_InitFuncPtr`, ctx+0x53b9c…+0x53bd0) [R]
| slot | function | what it does |
|---|---|---|
| +0x53b9c | `pIQ_GetIQVersion(ctx, &vendor, &major, &minor)` | vendor = ctx+0x5803c (the word last set by id 0x1001 Set); major / minor parsed with `strsep` / `simple_strtol` from the constant "0.1.3" → 1, 3 |
| +0x53ba0 | `pIQ_SetIQVersion(ctx, &v)` | ctx+0x5803c = v[0] |
| +0x53ba4 | `pIQ_GetIQParaInitStatus(ctx, &b)` | b = (u8) ctx+0x55c08 |
| +0x53ba8 | `pIQ_GetIsoInd(ctx, &i)` | i = ctx+0x53778 (ISO index in use) |
| +0x53bac | `pIQ_IQTrigger(ctx, mode, ip)` | the external trigger, below |
| +0x53bb0 | `pIQ_SetParaMode(ctx, &m)` | ctx+0x538dc = m (> 1 → 0); `IQ_ParaModeGet` (§4.1) |
| +0x53bb4 | `pIQ_GetParaMode` | returns ctx+0x538dc |
| +0x53bb8 / +0x53bbc | `pIQ_SetFastMode` / `pIQ_GetFastMode` | u8 ctx+0x538d0 |
| +0x53bc0 | `pIQ_SetMirrorFlip(ctx, m)` | ctx+0x55bec = m; ctx+0x55bf4 = (m > 1) (vertical flip [I: modes 2 / 3 = flip, 1 = mirror]); then `pIQ_IQTrigger(4, 0x0c)`, `(4, 2)`, `(4, 0x21)` — re-runs IP 12 ALSC, IP 2 NR3D and IP 33 PFC |
| +0x53bc4 / +0x53bc8 / +0x53bcc / +0x53bd0 | `pIQ_Cali_OBC_Apply` / `_ALSC_Apply` / `_LSC_Apply` / `_SDC_Apply` | §8 |

`pIQ_IQTrigger(ctx, mode, ip)`: mode ≤ 8 sets ctx+0x538d3 = 1 (force pending) and then: mode 1 / 2 / 3 → for every IP in list
1 / 2 / 3 (§4.3) ctx+0x54800+4·ip = 1; mode 4 → that one IP (ip < 0x32, else "ip_ind (%d) >= ENU_BLKMAX (%d)"); modes 0, 5–8
only set the pending byte. Mode 9 → ctx+0x538d3 = 0 and the whole force array +0x54800…+0x548c4 (50 words) cleared; > 9 →
"mod (%d) > IQ_PA_MAX (%d)". The pending byte is what `IQ_Debug_Check` returns (§4.4), so on the next `DoIq`
`IQ_Trigger_Froce` turns every forced IP into trigger bit 0x40 and clears the request with mode 9; the IPs run in the same
`DoIq` through `IQ_ParamSetSwitch` → `IQ_Trigger_RUNTIME`. Callers: `pIQ_SetMirrorFlip`, `IQ_Debug_TriggerIQ`,
`IQ_Trigger_Froce` (mode 9), `IQ_InitParameters` (mode 9), `IQ_IP_Reset_ALL` (§11.1), `IQ_Yuv2Bayer_UpdateIPStatus` (§11.2), the
converters `IQ_API20_SET_{EFFECT,TEMP,DUMMY,DUMMY_EX}`, `IspRegSetNR3D`, `IspRegSetWDRLOC` (§10.2), `pIQ_Cali_SDC_Apply` (§8),
and in the IQ API sub-part `DrvAlgo_IF_IQ_ForceTrigIQ` (mhal's own call at open; isp-3-iq-api.md §3.4).

### 3.3 Initialisation (`IQ_InitParameters`, from `IqInit`) [R]
1. Clears the 46 shadow dirty bytes (shadow+0x4d20, 0x2e B), 14 bytes of the menuload staging area (staging+0x38b0), the 50
   bytes at ctx+0x54704; `hal_IQ_InitTriggerMode` (§4.3).
2. State defaults: gain 0x400 (ctx+0x538c4, +0x538c0), CT index 1, ALSC previous / target index 1 (+0x53794 / +0x53798), colour
   temperatures 5000 K (+0x5379c…+0x537a8), CT thresholds {2500, 3500, 6000, 0 ×13} (+0x537ac…), ISO indexes 0, +0x537f8[k] = k,
   the 32-word CCM-smooth history 0, +0x538cc = 1 (first run), fast mode 0, IQ mode 0 (day), +0x538e0 = 1, CT-copy flags {1, 0},
   +0x538d4 = 0x32, debug bytes +0x544d0… = {0, 0x32, 0x32, 0x32} and +0x544cc = 0, +0x544d6 = 0x1000 (u16: API-reset slot 0 and
   byte 0x10), frame count 0, stored size = input size, +0x55c00 = 0x400, +0x55c04 = 300, ctx+0x55b18…+0x55beb zeroed (0xd4 B).
3. `hal_IQ_API20_InitFuncPtr` (empty), `hal_IQ_InitIPstatus` (all 50 IP-present words +0x5463c = 1), `IQ_GetParamFromCfg`
   (below), `hal_IQ_API20_InitParameters` (§6.2), `IQ_API20_IPMode_Update` (§4.3).
4. ctx+0x1a1d8 = gain-table[ISO index used] [I: the gain value of the current row, read by the IP code];
   `hal_IQ_Set_ALL(ip 0x2a = 42, iso 0, mode 1)` — IP 42 has no case, so this only performs the common part of `hal_IQ_Set_ALL`
   for a non-existent IP [R; effect I: none]; `pIQ_IQTrigger(9)` (clear forces).
5. Builds the two 256-entry contrast S-curves used by the gamma converter (isp-3-iq-api.md §3.5): ctx+0x540c0 = Hi, ctx+0x542c0 = Lo
   (u16), entry 0 = 0, entry 255 = 0x3ff; for 0 < x < 128: Hi = (200·x(128−x) + 512·x²) >> 14, Lo = (800·x(128−x) + 512·x²) >> 14;
   for 128 ≤ x < 255, with u = 256 − x, v = x − 128: Hi = (512u² + 1840uv + 1023v²) >> 14, Lo = (512u² + 1280uv + 1023v²) >> 14
   (the products are `_exp(x, 2)` etc.).
6. u16 ctx+0x55c14 = 0x111b (ALSC grid 27 × 17); vendor word ctx+0x5803c = 0; `hal_IQ_InitParameters` (below).

**`IQ_GetParamFromCfg`** copies from the parsed IQ file (`cfg_getcfgbyenum`): enum 0x101 `CHIP_VER` → ctx+0, 0x103 `GAIN_TBL0` →
ctx+0x104, 0x105 `GAIN_TBL1` → ctx+0x144, 0x107 `EXTEND_ISO_IND` → ctx+0x184, 0x108 `ISO_IND` → ctx+0x185, 0x102 `IRISO_TBL` →
ctx+2; then `IQ_ParaModeGet`, `IQ_ISOIndexGet`, ctx+0x1a1d8 = gain-table[index], `IQ_ParamSetSwitch(6)` (= trigger bit 0x20 on
all 50 IPs and `IQ_Trigger_INIT`, §4.3 — **the first full IP load**), ctx+0x538cc = 0. The block tables 0x104 / 0x106 (`BLK_TBL0/1`)
are not read by the engine [R: no reference] — the IR block thresholds come from the debug block (§2.3).

**`hal_IQ_InitParameters`**: ctx+0x53bd8 = 0x00200010, +0x53bdc = 0x00010020; ctx+0x53ccc and +0x53d8c: 0xc0 bytes of 0x40;
ctx+0x53bf4, +0x53c3c, +0x53c84: 0x48 bytes of 0. The same pattern again at ctx+0x53e4c / +0x53e50, +0x53f40 / +0x54000 (0x40),
+0x53e68 / +0x53eb0 / +0x53ef8 (0). The first set is the NR3D noise-profile / motion-threshold state read by
`IQ_Patch_NR3D_calc_noise_profile` and `…_calc_motion_th_gain` and written by `IQ_API20_SET_NR3D` (§9.2); the second set has no
reader by literal offset [R] [I: an NR3D_P1 copy this build never uses]. Then pipe mode (IqInput+0x50) 4 → `IQ_Yuv2Bayer_Init`,
5 → `IQ_Yuv_Init` (§11.2).

### 3.4 `hal_DoIQ(ctx)` (end of every `DoIq`) [R]
1. IqInput+0x60 = 1 (the CCM request, ch+0x150) → `hal_IQ_Set_ALL(24 CCM, iso, CCM's mode)` (→ HSV too, §5.2).
2. ctx+0x53761 = 1 (no writer anywhere) or ctx+0x53762 = 1 (ColorToGray on, written by `IQ_API20_SET_CTG`) → IP 40 YCUVM10 —
   on every `DoIq` while the flag is set.
3. Pipe mode 1 (HDR) → IP 14 HDR.
4. `IQ_Patch_NR3D_bAEAccControlMdThGain` (§9.2).
The IPs of steps 1–3 are subject to the once-per-frame rule (§5.2), so they do nothing if the trigger pass already ran them.

### 3.5 `hal_DoIQPerFrame(ctx)` (every frame, from `DoIqPerFrame`) [R]
1. If total gain, ISP gain or IqInput+0x10 differ from the stored copy — kept in **one static triple in `.bss`** (label
   AwbDefaultGainPercent+0x568…+0x570), shared by all channels — store them and run IP 6 OBC and IP 7 OBC_P1 (each in its mode).
2. IP 20 WDRLOC, every frame.
3. `IQ_Patch_NR3D_bAEAccControlMdThGain`.
4. ColorToGray dirty (API slot 15 flag, ctx+0x535f4 = 1): converter slot 15 (`IQ_API20_SET_CTG`), then an **inline copy of the
   IP 40 YCUVM10 run** (mode-1 load, gain blend, `IQ_DoEffect(40)`, `IQ_Patch_YCUVM10`, `IspRegSetYCUVM10`) that bypasses the
   present / once-per-frame checks; flag cleared. (In mode 4 the function would return −1 here and skip steps 5–7; dead, §4.3.)
5. AdaptiveGamma enabled (ctx+0x45088 = 1, struct of id 0x1036) and GammaLUT enabled (ctx+0x3a678 = 1, struct of id 0x1008):
   converter slot 14 (`IQ_API20_SET_GAMA_C2A_ISP`, isp-3-iq-api.md §3.3) and IP 26 GAMA_C2A.
6. ALSC_ADJ enabled (ctx+0x53520 = 1) → IP 12 ALSC every frame; else IP 12 once if the one-shot byte ctx+0x55c2c is 1 (set by
   the ALSC patch, §9.3), byte cleared.
7. `IQ_Debug_Log` (§11.1).

## 4. ISO index, colour-temperature index, triggers

### 4.1 `IQ_ParaModeGet` and `IQ_ISOIndexGet` [R]
**`IQ_ParaModeGet`**: gain table = ctx+0x144 (enum 0x105 `GAIN_TBL1`) if IQ mode (ctx+0x538dc) = 1 (night), else ctx+0x104
(enum 0x103 `GAIN_TBL0`) → ctx+0x53774; IR-ISO remap table = ctx+2 (enum 0x102, 16 × 16 u8) → ctx+0x53770.

**`IQ_ISOIndexGet(ctx)`** returns the trigger mode (0 none, 1 ISO and CT / fast, 2 ISO, 3 CT, 7 error):
1. g = input+0x14 (total gain) → ctx+0x538c4; u16 input+0x38 → ctx+0x538c8; previous index ctx+0x53780 = ctx+0x53778.
2. If thr[9] ≤ thr[0] (the gain table is not increasing) → "CFG IQ Parsing Error, please check CFG", return 7.
3. ISO index i = number of thresholds thr[0…15] that are < g (0…16), 16 → 15; forced by ctx+0x538d1 = 1 (index ctx+0x538d2,
   `IQ_SetIsoInd`). → ctx+0x53788. ctx+0x5377c = g.
4. IR remap — only if RGBIR row 0 enable (ctx+0x794c) = 1 and day mode: block b = number of debug-block thresholds (words
   31…35) below the u16 input+0x38 (0…5; b = 1 when only the first is below); index used = remap[b·16 + i] (ctx+0x5378c,
   ctx+0x53778) and the gain used = g · 2^|remapped − i| (ctx+0x5377c). Otherwise index used = i, ctx+0x5378c = 0. On this
   camera input+0x38 is always 0, so b = 0, and the imx307 remap row 0 is the identity [R values] — no effect.
5. ISO trigger. ctx+0x53b80 = the number of IPs whose ITP "interpolate" byte is 1 (set by `IQ_SET_ITP`, §5.1; ctx+0x53b84 =
   0xc2). If it is 0 → mode 2 whenever the index used changed. Otherwise → mode 2 when |g − g_last|·1024 / min(g, g_last) > 0xc2
   (a gain change of more than 194/1024 ≈ 19 % since the last trigger; g_last = ctx+0x538c0, then updated); an index change
   alone does not trigger. The imx307 file turns interpolation on for 20 of the 31 ITP enums [R, the imx307 file decoded], so on this
   camera the IPs are recomputed on ≥ 19 % gain steps.
6. ctx+0x185 (enum 0x108 `IQ_INFO_ISO_IND`, a run-time value in the IQ-file table area) = index used.
7. CT thresholds ctx+0x537ac[16]: if the CCM API is enabled (ctx+0x385b4 = 1) the 16 `CCT_THR` of its auto entries (u16,
   stride 0x1c from ctx+0x385c0), ctx+0x53838 = 0; else, once (flags ctx+0x538bc = 1 / +0x538bd = 0), the 16 u16 from
   ctx+0xac94 (= CCM row 0 + 0x1a, the `IQ_CCM_CCT_THR`, enum 0x302, of the IQ file, §7.1); after a copy the flags are swapped.
8. Colour temperature t = input+0x24 → ctx+0x537a4, +0x537a8. Fast mode (ctx+0x538d0 = 1): ctx+0x5379c = ctx+0x537a0 = t,
   mode 1. Else a change of t sets ctx+0x537a0 = t and mode 3 (or 1 if the ISO also changed); ctx+0x5379c = t.
9. After a CT-threshold copy (flag +0x538bd = 1): CT index = k + 1 for the largest k in 0…14 with thr[k] < CT and
   thr[k] < thr[k+1] (so trailing equal / zero entries are skipped), else 0 → ctx+0x53790; flags reset. Without a copy the CT
   index is set to 2 [R; it is copied every call while the CCM API is enabled].
10. `IQ_ALSC_Index_Check` and `IQ_Temp_Index_Check` (§4.2) may raise the mode.

### 4.2 ALSC table choice, temperature index, resolution check, CCM smoothing [R]
**`IQ_ALSC_Index_Check(ctx, S = ctx+0x53770, ctx, &mode)`** — choice among the 3 colour-temperature tables of ALSC: the three
table CCTs (S+0x7c / +0x80 / +0x84 = ctx+0x537ec / f0 / f4) come from the ALSC API struct (ctx+0x20755 / +0x21263 / +0x21d71)
when the ALSC API is on (ctx+0x4a67c = 1), else from the IQ file (ctx+0x3bd7 / +0x46e5 / +0x51f3). Target = the table whose CCT
is nearest to the current CT (S+0x34 = ctx+0x537a4), → ctx+0x53798. With previous p = ctx+0x53794 (must be ≤ 2, else
"cct_ind_alsc_pre > 2", −1): if target > p, distance d = CT − cct[p], hysteresis h = |(cct[p+1] − cct[p]) · 60 / 100| >> 5;
if target < p, d = cct[p] − CT, h from (cct[p] − cct[p−1]). d > h → p = target, mode = 3 if it was 0 else 1; otherwise the
target is reset to p (no switch).

**`IQ_Temp_Index_Check(ctx, &mode)`**: ctx+0x5351c = (u8) input+0x1c (temperature, 36). Only if the Temperature API is on
(ctx+0x53414 = 1): a change > 1 from ctx+0x57c34 → mode = 1 and +0x57c34 updated; temperature index = first k with temperature
≤ thr[k] (16 u8 at ctx+0x53418) → ctx+0x57c35; ctx+0x57c36 = max(index − 1, 0).

**`IQ_ResolutionCheck`**: input size (+0x3c / +0x40) ≠ stored (u16 ctx+0x55bf8 / +0x55bfa) → store, return 1 (`DoIq` then uses
mode 1).

**`IQ_CCM_smooth(ctx, target, current)`** (unreferenced): step = clamp((current − target) >> 5, −1000, 1000); a 31-word history
at ctx+0x5383c is filled with the step (when ctx+0x537a0 = target) or shifted by one; returns current, target or ctx+0x537a0 +
history[0] depending on which side the sum lies, clearing the history when it overshoots; +0x538b8 = step. A CT smoothing
filter that this build never calls.

### 4.3 Trigger machinery [R]
**Per-IP trigger class** (`hal_IQ_InitTriggerMode`, word ctx+0x544d8 + 4·ip): 0 = never re-run on a change, 1 = on ISO and CT
change, 2 = on ISO change, 3 = on CT change, 6 = none (IPs 42–47). Lists built from it: list 1 (ctx+0x545a0, count +0x54636) =
all IPs of class 1 / 2 / 3; list 2 (+0x545d2, count +0x54637) = class 1 / 2; list 3 (+0x54604, count +0x54638) = class 1 / 3.
The same loop sets every IP's mode (+0x54738) = 1 (load from file), trigger bits (+0x548c8) = 0 and last-run frame (+0x54990) =
0xff.

| class | IPs |
|---|---|
| 2 (ISO) | 0 DECOMP, 1 BNR, 2 NR3D, 6 OBC, 7 OBC_P1, 15 GAMA_16to12, 16 DPC, 18 ANTICT, 19 RGBIR, 20 WDRLOC, 21 SpikeNR, 22 SDC, 23 DM, 26 GAMA_C2A, 27 R2Y, 29 XNR, 30 YEE, 31 SIIR, 34 GNR, 35 UVM, 37 ADJUV, 38 MXNR, 39 YUVGAMA, 40 YCUVM10, 41 COLORTRANS |
| 3 (CT) | 10 LSC, 12 ALSC |
| 1 (ISO + CT) | 14 HDR, 24 CCM, 25 HSV, 33 PFC, 48 EFFECT |
| 0 | 3, 4, 5, 8, 9, 11, 13, 17, 28 NLM, 32 ACLUT, 36 WDRGBL, 49 DUMMY |
| 6 | 42–47 |

- **`IQ_ParamSetSwitch(ctx, mode, ip)`** (prints the state when debug print is on): mode 1 / 2 / 3 → OR trigger bit 1 / 2 / 4
  into every IP of list 1 / 2 / 3; 4 → bit 8 into one IP; 6 → bit 0x20 into all 50 and `IQ_Trigger_INIT`, return; 8 → return
  (nothing); 0, 5, 7 and others → nothing marked. Then (all but 6 / 8) `IQ_Trigger_RUNTIME`.
- **`IQ_Trigger_RUNTIME`**: for ip 0…49 with trigger bits ≠ 0: `hal_IQ_Set_ALL(ip, ISO index used, mode = IP mode)` (debug print
  when u16 ctx+0x57c30 = 0x101); every IP's trigger word is then cleared.
- **`IQ_Trigger_INIT`**: (print "case: Init IPs!!") `hal_IQ_Set_INIT` (§5.1), then `hal_IQ_Set_ALL(ip, iso, 1)` for ip 0…49 —
  mode 1 = load the IP's IQ-file rows, then run it. The trigger bits set by mode 6 are not cleared here; they are cleared by the
  next `IQ_Trigger_RUNTIME`, which then also re-runs every IP once more [R; effect I: the second `DoIq` recomputes everything].
- **`IQ_API20_Trigger_CHK`**: for each of the 53 API dirty flags (ctx+0x535b8 + 4k) = 1 calls `hal_IQ_API20_EnGet(k)` (result
  unused) and returns 1 if any was set.
- **`IQ_API20_IPMode_Update`**: for ip 0…49: the up-to-four API slots that feed the IP (`hal_IQ_API20_IP2API`, §6.1) — dirty =
  OR of their dirty flags, active = OR of their bypass values (ctx+0x5368c + 4k) and enables (`hal_IQ_API20_EnGet`); for IPs
  30, 37, 41 (`0xc81` mask over ip − 30: bits 0, 7, 11) also the dirty flag of slot 27 (EFFECT, ctx+0x53624) when the EFFECT
  API is enabled. IP mode := 3 if active else 0; trigger bit 0x10 if dirty.
- **`IQ_Trigger_API`**: `IQ_API20_IPMode_Update`, then for each dirty slot k: `hal_IQ_API20_SET_ALL(k, iso)` (the converter,
  §6.1; print when ctx+0x57c30 = 0x100); flag cleared.
- **`IQ_Trigger_Froce`**: if a force is pending (ctx+0x538d3), trigger bit 0x40 on every IP whose force word (+0x54800 + 4·ip) is
  1; then `pIQ_IQTrigger(9)`.

So the IP mode (ctx+0x54738 + 4·ip) is 1 only until the first run, then 0 (IQ-file tables) or 3 (API tables), recomputed by every
API Set. Its only writers are `hal_IQ_InitTriggerMode` (1) and `IQ_API20_IPMode_Update` (0 / 3) [R: every reference in the
block]; modes 2 and 4 are never written, so the "refused, return −1" branch for mode 4 in `hal_IQ_Set_ALL`,
`IQ_SET_INIT`, `IQ_SET_ITP` and `hal_DoIQPerFrame` is dead. `IQ_Trigger_RUNTIME` passes the IP's mode as `hal_IQ_Set_ALL`'s
mode argument, so mode 1 (reload from the file) can only happen on the very first pass.

### 4.4 Debug hooks [R]
- **`IQ_Debug_Check(ctx, &ip_out)`** (every `DoIq`): copies debug words 1, 2, 3, 5, 13, 14 into the bytes listed in §2.3,
  *ip_out = ctx+0x538d4 (0x32); if word 6 = 1 → `IQ_Debug_TriggerIQ` and `IQ_Debug_ApiReset`. Returns the force-pending byte
  ctx+0x538d3, or the result of `IQ_Debug_TriggerIQ` when that ran.
- **`IQ_Debug_TriggerIQ(ctx, &ip)`**: word 9 = 1 → print "IQTrigger", mode = word 10, ip = word 2, `pIQ_IQTrigger(mode, ip)`,
  returns 1; word 9 = 0 → print "IQTriggerOFF", `pIQ_IQTrigger(9)`, returns 0; other → `pIQ_IQTrigger(9)`, returns 0.
- **`IQ_Debug_ApiReset`** / **`IQ_ApiReset(ctx, k)`**: k = word 14; k < 53 → ctx+0x544d6 = k (print "ApiReset !! %d:"),
  returns 1; else ctx+0x544d6 = 0. The reset itself is the forced 0 of `hal_IQ_API20_EnGet` (§6.1) and `IQ_IP_Reset_ALL` (§11.1).
- **`IQ_Debug_FixISO`** (unreferenced): word 7 = 1 and no force yet → `IQ_SetIsoInd(word 8, or the current index if > 15)`,
  print "set ind %d-->%d", return 1; word 7 = 0 while forced → sets the forced index to 11 and keeps forcing (prints "ForceISO
  enable") [R — looks inverted, but unreachable].
- **`IQ_SetIsoInd(ctx, i)`**: i ≤ 15 → force index i (ctx+0x538d1 = 1, +0x538d2 = i, "ForceISO enable!!"); else force off.
- **`IQ_Set_LookUp`** (unreferenced): tail-calls `hal_IQ_Set_ALL` with the caller's arguments; `IQ_Get_LookUp`,
  `IQ_ParamGetSwitch`, `IQ_SetParamToCfg` return −1; `IQ_SetIQParaInitStatus(ctx, v)` stores ctx+0x55c08; `IQ_Property_Set(ctx,
  k, v)` / `IQ_Property_Get(ctx, k)` store / read word k of the array at *(ctx+0x58) — ctx+0x58 is inside the IR-ISO remap table
  (ctx+2…+0x101), so these would use table bytes as a pointer [R; unreferenced] [I: Mercury5 leftovers].

## 5. The per-IP runner

### 5.1 `hal_IQ_Set_INIT` = `IQ_SET_INIT(1)` + `IQ_SET_ITP(1)` [R]
- `IQ_SET_INIT(ctx, mode)`: mode 1 loads the 88 INIT enums 0x10b–0x162 (one row each) into ctx+0x186 + off[e] (offsets from the
  table at `.rodata` label VSDB_HDMI_IEEE_ID+0x4735), then `IspRegSetINIT` (§10.2) — on every call, not only mode 1.
- `IQ_SET_ITP(ctx, mode)`: mode 1 → `IQ_PARSE_ITP` loads the 31 ITP enums 0x40f–0x42d into ctx+0x538e4 + off[e] (table at
  +0x47e7). Then ctx+0x53b80 = Σ byte 0 of the 31 ITP records (number of IPs with interpolation on; §4.1 step 5), ctx+0x53b84 =
  0xc2; then it **forces two bytes of the CCM ITP record** (0x41f at ctx+0x53a76): byte 0 (CCM interpolation on) and byte 4
  (the flag of parameter 3, `IQ_CCM_COEFF`) := 1, whatever the file says — the sum above is taken before this, so it does not
  count them ("CCM COEF is forced to do interpolation!!!" is printed when debug print is on and the debug IP byte is 42).

### 5.2 `hal_IQ_Set_ALL(ctx, ip, iso, mode)` [R]
<!-- ev:isp-4-iq-engine.5.2 -->
Common prologue: pipe mode 4 (and mode ≠ 1) → `IQ_Yuv2Bayer_UpdateIPStatus(ip)`, pipe mode 5 → `IQ_Yuv_UpdateIPStatus(ip)`
(§11.2); IP-present word 0 → return 0; last-run frame (+0x54990 + 4·ip) = current frame ≠ 0 → return 0 (an IP runs at most once
per frame, except in frame 0); store the frame. CT index for LSC / ALSC = ctx+0x53790. Then a switch on ip; ips without a case (3,
4, 5, 8, 9, 11, 13, 17 GE, 28 NLM, 32 ACLUT, 36 WDRGBL, 42–49) do nothing and return 0.

Per case:
1. mode 4 → return −1 (never happens, §4.3).
2. mode 1: for each enum e of the IP, `IQ_Get_cfgdata(file, e, …, ctx + file_base + off[e], stride)` (§13.3) — the IP's rows
   from the IQ file; `off[e]` comes from a u16 table per IP in `.rodata` (the row offset of that parameter, the "row_off" column
 of); ITP length check: N(itp enum) − 1 must equal the IP's parameter count, else "ITP size
   mismatch!! ENU ID = %d, ITP size = %d, IP size = %d" (printed, no other effect); the per-parameter interpolation flags (u32
   array, "flags" column) are cleared and, if the ITP byte 0 is 1, set to the ITP bytes 1…n.
3. Blend, axis "gain": lo = max(iso − 1, 0), hi = iso; x = gain used (ctx+0x5377c), x0 = gain-table[lo], x1 = gain-table[hi];
   table = API table (mode 3) or IQ-file table (else); for every parameter p and every element j of it (N from `Dbg_Items`,
   element size from the type): out[off[p] + j·size] = `IQ_para_inter(0, flag[p], x, x0, x1, row_lo[…], row_hi[…])` (§14).
   Axis "CT" (LSC, ALSC): if the IP's ITP byte 0 is 1: hi = 0 if CT ≤ cct[0], 1 if cct[0] < CT ≤ cct[1], else 2; lo = max(hi − 1,
   0); x = CT (ctx+0x537a4), x0 / x1 = the ALSC table CCTs (ctx+0x537ec + 4·lo / hi) — LSC uses the ALSC's CCTs; rows lo / hi of
   the 3-row table. ITP off: memcpy of table row ctx+0x53798 (the hysteresis-chosen ALSC index, §4.2) — again for both.
   Axis "copy" (DECOMP, GAMA_16to12, SDC, R2Y, COLORTRANS): the single row of the API table (mode 3) or of the file table is
   copied.
4. `IQ_DoEffect(ctx, ip)` (§7.5), `IQ_Patch_<IP>` where it is not a stub (§9), `IspRegSet<IP>(ctx, out)` (§10.2).
5. IP 24 CCM runs `IQ_SET_CCM(ctx, iso, ct_index, mode)` and **falls through** into IP 25's `IQ_SET_HSV(ctx, iso, ct_index,
   mode)` — every CCM run also runs HSV (§7).

### 5.3 IP slots [R]
<!-- ev:isp-4-iq-engine.5.3 -->
Slot numbers 0–41 follow the `IQ_Patch_*` / `IspRegSet*` order in the binary (machine-readable:, columns "file_tbl", "stride", "api_tbl", "out_blk", "itp_enum"; per-case detail); 48 = EFFECT and 49 = DUMMY (BLKMAX) come from the
API→IP table (§6.1); 42–47 have no case and no API. File table = ctx offset, row stride × rows; ITP enum with its length n.

| ip | name | enums | file table (ctx+, stride × rows) | API table | out block | axis | ITP enum (n) | after the blend |
|---|---|---|---|---|---|---|---|---|
| 0 | DECOMP | 0x165–0x175 | 0x2c2, 0x24 × 1 | 0x1ce40 | 0x1a1dc | copy | 0x40f (17) | IspRegSetDECOMP |
| 1 | BNR | 0x178–0x181 | 0x2e6, 0x38 × 16 | 0x1ce64 | 0x1a200 | gain | 0x410 (10) | Patch_BNR, RegSet |
| 2 | NR3D | 0x184–0x1cb | 0x666, 0x2fa × 16 | 0x1d1e4 | 0x1a238 | gain | 0x411 (72) | Patch_NR3D, RegSet |
| 6 | OBC | 0x1ce–0x1d7 | 0x3636, 0x16 × 16 | 0x201b4 | 0x1a535 | gain | 0x412 (10) | Patch_OBC, RegSet |
| 7 | OBC_P1 | 0x1da–0x1e3 | 0x3796, 0x16 × 16 | 0x20314 | 0x1a54b | gain | 0x413 (10) | Patch_OBC_P1, RegSet (OBC's offset table) |
| 10 | LSC | 0x1e6–0x1f0 | 0x3916, 0xd2 × 3 | 0x20494 | 0x1a563 | CT | 0x414 (11) | Patch_LSC, RegSet |
| 12 | ALSC | 0x1f3–0x202 | 0x3b8f, 0xb0e × 3 | 0x2070d | 0x1a636 | CT | 0x415 (16) | Patch_ALSC, RegSet |
| 14 | HDR | 0x205–0x248 | 0x5cbc, 0xee × 16 | 0x2283a | 0x1b145 | gain | 0x416 (68) | RegSet |
| 15 | GAMA_16to12 | 0x24b–0x24f | 0x6b9c, 0xa0 × 1 | 0x2371a | 0x1b233 | copy | 0x417 (5) | RegSet |
| 16 | DPC | 0x252–0x279 | 0x6c3c, 0xb0 × 16 | 0x237ba | 0x1b2d3 | gain | 0x418 (40) | Patch_DPC, RegSet |
| 18 | ANTICT | 0x27c–0x283 | 0x774c, 0x20 × 16 | 0x242ca | 0x1b384 | gain | 0x419 (8) | Patch_ANTICT, RegSet |
| 19 | RGBIR | 0x286–0x291 | 0x794c, 0x3c × 16 | 0x244ca | 0x1b3a4 | gain | 0x41a (12) | Patch_RGBIR, RegSet |
| 20 | WDRLOC | 0x294–0x2bd | 0x7d0c, 0x202 × 16 | 0x2488a | 0x1b3e0 | gain | 0x41b (42) | Patch_WDRLOC, RegSet |
| 21 | SpikeNR | 0x2c0–0x2ce | 0x9d2c, 0x48 × 16 | 0x268aa | 0x1b5e2 | gain | 0x41c (15) | Patch_SpikeNR, RegSet |
| 22 | SDC | 0x2d1–0x2d7 | 0xa1ac, 0x40e × 1 | 0x26d2a | 0x1b62a | copy | 0x41d (7) | RegSet |
| 23 | DM | 0x2da–0x2fa | 0xa5ba, 0x6c × 16 | 0x27138 | 0x1ba38 | gain | 0x41e (33) | RegSet |
| 24 / 25 | CCM / HSV | 0x2fd–0x302 / 0x305–0x307 | 0xac7a, 0x3c × 16 / 0xb03a, 0x34 × 16 (own code) | 0x277f8 / 0x27bb8 | 0x1baa4 / 0x1bae0 | CT (SAT_GAIN by gain) / CT | 0x41f (6; bytes 0 and 4 forced on, §5.1) / 0x420 (3) | §7 |
| 26 | GAMA_C2A_ISP | 0x30a–0x30f | 0xb37a, 0x608 × 16 | 0x27ef8 | 0x1bb14 | gain | 0x421 (6) | RegSet |
| 27 | R2Y | 0x312–0x314 | 0x113fa, 0x18 × 1 | 0x2df78 | 0x1c11c | copy | 0x422 (3) | RegSet |
| 29 | XNR | 0x372–0x383 | 0x11422, 0x26 × 16 | 0x2dfa0 | 0x1c135 | gain | 0x424 (18) | RegSet |
| 30 | YEE | 0x317–0x36f | 0x11682, 0x290 × 16 | 0x2e200 | 0x1c15b | gain | 0x423 (89) | Patch_YEE, RegSet |
| 31 | SIIR | 0x386–0x397 | 0x13f82, 0x60 × 16 | 0x30b00 | 0x1c3eb | gain | 0x425 (18) | RegSet |
| 33 | PFC | 0x39a–0x3b8 | 0x14592, 0x66 × 16 | 0x312b0 | 0x1c44c | gain | 0x426 (31) | RegSet |
| 34 | GNR | 0x3bb–0x3d0 | 0x14bf3, 0xc0 × 16 | 0x31910 | 0x1c4b2 | gain | 0x427 (22) | RegSet |
| 35 | UVM | 0x3d3–0x3d5 | 0x157f3, 0x16 × 16 | 0x32510 | 0x1c572 | gain | 0x428 (3) | RegSet |
| 37 | ADJUV | 0x3d8–0x3e8 | 0x15953, 0x50 × 16 | 0x32671 | 0x1c589 | gain | 0x429 (17) | RegSet |
| 38 | MXNR | 0x3eb–0x3f1 | 0x15e53, 0x1a × 16 | 0x31100 | 0x1c5d9 | gain | 0x42a (7) | RegSet |
| 39 | YUVGAMA | 0x3f4–0x3fc | 0x15ff3, 0x40e × 16 | 0x32b71 | 0x1c5f3 | gain | 0x42b (9) | RegSet |
| 40 | YCUVM10 | 0x3ff–0x403 | 0x1a0d3, 0xe × 16 | 0x36c51 | 0x1ca01 | gain | 0x42c (5) | Patch_YCUVM10, RegSet |
| 41 | COLORTRANS | 0x406–0x40c | 0x1a1b3, 0x20 × 1 | 0x36d31 | 0x1ca0f | copy | 0x42d (7; flags always 0, §13.1) | RegSet |
| 3, 4, 5, 8, 9, 11, 13, 17, 28, 32, 36 | NR3D_P1, FPN, FPN_P1, GAMA_16to16_ISP, GAMA_C2C_P1_ISP, LSC_P1, ALSC_P1, GE, NLM, ACLUT, WDRGBL | — | — | — | — | — | — | no case |
| 42–47 | — | — | — | — | — | — | — | no case, no API |
| 48 | EFFECT | — | — | — | — | — | — | no case; `IQ_DoEffect` reads API slot 27 directly |
| 49 | DUMMY (BLKMAX) | — | — | — | — | — | — | no case |

The API table of MXNR (0x31100) lies before that of PFC (0x312b0) although MXNR is IP 38. The DECOMP API table (0x1ce40) is never
filled by a converter (no API feeds IP 0), so DECOMP always runs on the IQ file. The output blocks of LSC, ALSC, GAMA_16to12, SDC,
CCM, HSV, R2Y and COLORTRANS are read from `IQ_IP_Bypass_ALL` (§11.1) and `hal_IQ_Set_ALL`.

## 6. The API side (`hal_IQ_API20_*`)

### 6.1 The 53 API slots [R]
<!-- ev:isp-4-iq-engine.6.1 -->
One slot per API struct group; the slot number indexes the dirty flags (ctx+0x535b8 + 4k), the bypass values (+0x5368c + 4k) and
the enable results (+0x55b18 + 4k). The full table (slot, API ids, enable byte, converter, IPs fed) is the TSV; the ids and the
converters with an id are in isp-3-iq-api.md §3.3.
- **`hal_IQ_API20_IP2API(ip)`** returns a u32 of four slot numbers (0xff = none) from `.rodata` 0xafe4 (ip 1…49; ip 0 → −1).
  Non-obvious feeds: slot 16 (id 0x1010 NRLuma) feeds IP 1 **BNR** and IP 28 NLM; slot 10 (NR3D) feeds IP 2 NR3D, 28 NLM and 32
  ACLUT; slot 9 (Sharpness) feeds 30 YEE and 32 ACLUT; slot 7 (CrossTalk) feeds 17 GE and 18 ANTICT; slot 29 (0x102e
  NRChroma_Adv) feeds 31 SIIR and 35 UVM; slot 17 (Defog) feeds 20 WDRLOC and 39 YUVGAMA; slot 3 (Saturation) feeds 37 ADJUV and
  40 YCUVM10; IP 20 WDRLOC is fed by 4 slots (21 WDR, 17 Defog, 32 WDRCurveAdv, 47 WDRCurveFull), IP 26 gamma by slots 11–14; the
  BLKMAX entry (ip 49) lists slots 45 / 46 (Dummy, Dummy_EX), EFFECT (ip 48) slot 27. Slots 48 (AdaptiveGamma enable) and 50
  (Temperature) feed no IP.
- **`hal_IQ_API20_EnGet(ctx, k)`**: reads the slot's enable byte (the struct's first word in nearly all cases — the TSV column);
  slot 14 = 1 if any of Brightness / Lightness / Contrast (slots 11–13) is enabled, else the GammaLUT enable; slot 47 = OR of
  three bytes of WDRCurveFull (+0x475c8 / cc / d0); slots 0, 18, 24, 51, 52 and k > 50 → 0. The result is forced to 0 when the
  debug API-reset byte ctx+0x544d6 equals k or equals 53; it is stored at ctx+0x55b18 + 4k (dumped by debug code 0x102, §11.1)
  and returned.
- **`hal_IQ_API20_SET_ALL(ctx, k, iso)`**: switch k → converter (TSV column; decoded switch in):
  slots 11–14 and 48 all run `IQ_API20_SET_GAMA_C2A_ISP`; slot 3 runs `SET_YCUVM10` then `SET_ADJUV`; slots 0, 1, 18, 24, 31,
  32, 36, 38, 51, 52 have no converter (1 FPN, 31 NR3D_P1, 32 WDRCurveAdv have Set handlers without an ioctl id; 36 / 38 / 52
  are read by the IP code directly, §9.1, §9.3).
- `hal_IQ_API20_API2IP` (stub, unreferenced), `hal_IQ_API20_InitFuncPtr` (empty `bx lr`).
- Converters without a body: `IQ_API20_SET_FPN`, `_NR3D_P1`, `IQ_API20_SET_WDRCurveAdv`, `_WDR_GBL`, `_LSC_CTRL`, `_ALSC_CTRL` and
  `IQ_API20_LogPrint` (stubs returning 0, not reached from the `hal_IQ_API20_SET_ALL` switch).

### 6.2 Initial API state — `hal_IQ_API20_InitParameters(ctx, api = ctx+0x36d54)` [R by emulation and decompile]
<!-- ev:isp-4-iq-engine.6.2 -->
This 8.8 KB function (`.text` 0x69c62; called once from `IQ_InitParameters`, after the first full IP load) defines what every
IQ Get returns before the first Set, and what a converter leaves in the parameters it does not write. It was run in an emulator
seven times with independent random IQ-file tables and IP output blocks; every destination byte of the API tables and API structs
was matched to the source byte whose value followed it in all runs (named in
****: one line per copied field / constant / computed byte with the API field name on one
side and the IQ-file parameter — official enum name, row, element — on the other). Result: 3 573 copy runs, 3 995 non-zero
constants, 214 computed bytes; it calls nothing. On an all-zero context it stops with a NULL read at `.text+0x6bb36` (the LSC
branch reads the image size through the IqInput pointer ctx+0x53b88), before the ALSC code; with a non-zero background it runs
to the end.
1. The whole API area (API tables + structs, 0x1c938 B) and the 53-word array after it are zeroed.
2. **Whole-table copies IQ file → API table** for every IP that has an API table (BNR…YUVGAMA including NR3D, OBC / OBC_P1, LSC,
   ALSC, HDR, DPC, ANTICT / RGBIR / WDRLOC / SpikeNR / SDC, DM / CCM / HSV / GAMA_C2A / R2Y, XNR / YEE / SIIR, MXNR, PFC, GNR /
   UVM, ADJUV, YUVGAMA / YCUVM10 / COLORTRANS; e.g. SpikeNR 0x9d2c → 0x268aa, 0x480 B) — so API mode starts from the imx307
   values, and a converter that writes only some parameters of a row leaves the others at the IQ-file values.
3. **API structs**: for each group, auto[i] = the IQ-file row i values of the parameters the converter would write (the inverse
   of the converter), manual = the **IP output block** of the first load (the interpolated values at the gain of init). The gamma
   LUTs (0x1008 auto[i] / manual) and AdaptiveGamma curves are table copies of the GAMA_C2A rows / output block, YUVGamma of the
   YUVGAMA rows. Enables and OpType are 0 (zeroed), so nothing is in API mode after init.
4. Constants: Brightness / Lightness / Contrast LEVEL 50; CCM_SAT 100; DeMosaic DtlHScale / VScale 4 / 4; NR3D MdThdByY[] 0x40;
   Sharpness OverShootGain 0x40 0x40; DynamicDP Y_LUT[5] 200, DIFF_LUT[5] 4; HSV SATLUT_ByIso[], SATLUT_FORUNITCCM[] and
   GLOBAL_SAT 0x40 (1.0); WDRCurveFull defaults (2 856 constant bytes — a fixed curve table); LSC_CTRL / ALSC_CTRL and ALSC_ADJ
   constant words; Temperature struct constants (32 bytes) — exact values per byte in
   the TSV.
5. Computed fields (f = the IQ-file byte / word of row i of the named IP at the given row offset; manual = the same of the
   output block):

| API field | default |
|---|---|
| 0x100c CCM auto[i] / manual CCM[3], [7], [11] (the SUM column) | sum of that row's three coefficients (u16, 13-bit) |
| 0x100e NR3D YSfBlendLut[k] | 16 − NR3D row +0x42+k (u8; `IQ_NR3D_IIR_BLENDING_W[k]`) |
| 0x100e NR3D `YSfBlendOffset` (auto[i] ctx+0x37db6 + 0x7c·i, manual ctx+0x38576) | clamp(`IQ_NR3D_IIR_BLENDING_WB_W[0]` − `IQ_NR3D_IIR_BLENDING_W[0]`, 0, 16) of NR3D row i (+0x52 minus +0x42); manual from the output block (ctx+0x1a28a − ctx+0x1a27a). Together with YSfBlendLut it inverts the converter's `+0x52[j] = clamp(YSfBlendOffset + 16 − YSfBlendLut[j], 0, 16)`. imx307 row 0: 8 − 0 → 8. Missing from an earlier table of ours |
| 0x1013 CrossTalk Strength | 31 − ANTICT row +8 |
| 0x1017 DynamicDP HOTPIX / DARKPIX_COMPSLPOE | DPC row u16 +0x24 / +0x2e, >> 4 |
| 0x1018 HSV HUELUT[] (s16) | HSV row byte +2… converted from sign-magnitude (bit 7 = minus) to two's complement |
| 0x1011 NRChroma MatchRatio | 31 − MXNR row +4 |
| 0x1011 NRChroma StrengthByCEdge[0], [1] | 255 − MXNR +0xc, 1 − MXNR +0xd (mod 256) |
| 0x1037 NRChroma_Pre Strength | −XNR row +0x22 (mod 256) |
| 0x1016 WDR WDRStrByY[k] (u8) | WDRLOC row u16 at +0x118+2k, >> 4 (read for k = 0, 1) |
| 0x102e NRChroma_Adv `MotionColorReduce` (auto[i] ctx+0x49b12 + 45·i, manual ctx+0x49de2) | (255·(128 − b)) / 128, C signed division (truncates towards zero), stored mod 256; b = `IQ_UVM_DESAT_BYMOT[0]` of **UVM** row i (enum 0x3d5, ctx+0x157f3 + 0x16·i + 4; `.text` 0x6a1e8–0x6a1fe); manual from the UVM output block byte +4 (ctx+0x1c576; `.text` 0x6a176–0x6a18c). b = 0 → 255, 64 → 127, 128 → 0; b > 128 wraps (129 → 0xff, 200 → 113, 255 → 3). imx307: DESAT_BYMOT = 128 in every bin → 0 in all 16 auto entries and manual |
| 0x102e NRChroma_Adv, the same 45-byte entry | +0x29 `MotionClip` = SIIR row byte (copy); +0x2b `MotionColorRecover` = 0xff; +0x2c `PreStrength` = `IQ_UVM_STRENGTH` (UVM row +0) |
| 0x1028 ALSC `GridX` / `GridY` (ctx+0x4a680 / +0x4a681) | `IQ_ALSC_GTBL_WIDTH_M1` + 1 (enum 0x1f8, ALSC file row 0 +0xa, ctx+0x3b99) / `IQ_ALSC_GTBL_HEIGHT_M1` + 1 (enum 0x1f9, row 0 +0xc, ctx+0x3b9b), u8 wrap (0xff → 0) [R: a byte store of ctx+0x3b99 + 1 to the output +0x4e4b, and +0x1392d from
+0x3b9b] — the same "+1" `IQ_Patch_ALSC` applies (§9.3). imx307: 26 / 16 → 27, 17. An earlier table of ours listed the pair as one 2-byte "computed" GridX |
| 0x1021 LSC `CENTER_X` / `CENTER_Y` / `SHIFT` (ctx+0x4a39c / +0x4a39e / +0x4a3a4) | copies of LSC file row 0 (+4, +6, +8) when `IQ_LSC_SHIFT` (ctx+0x391e) ≠ 0; when it is 0, from the input size (IqInput+0x3c W, +0x40 H): W/2, H/2, clamp(⌊log2((W/2)² + (H/2)²)⌋ − 8, 0, 31) — the formula of `IspRegSetLSC` (§10.2); emulated 3840 × 2160 → 1920 / 1080 / 14, 1920 × 1080 → 960 / 540 / 12. `RATE_X` / `RATE_Y` are copies in both branches. an earlier table of ours shows these as unconditional copies. imx307: SHIFT 13 → copy branch, centre 960 / 540 (the centre of a 1920 × 1080 frame), shift 13; the imx307 file has `IQ_LSC_EN` = 0, so the centre has no effect unless the LSC API or the 0x2417 calibration enables LSC |

The forward converter of 0x102e (slot 29 → IPs 31 SIIR, 35 UVM) writes DESAT_BYMOT from MotionColorReduce, so the default makes a
Get return the value that maps back onto the file's DESAT_BYMOT[0] [I: the inverse relation was not emulated].

YEE `HighRatioDByState` / `HighRatioUDByState`: the init fills API [0] from `IQ_YEE_HIGH_DIR` / `IQ_YEE_HIGH_ALL` element 1,
[1] from element 0, [2] from element 2 (every auto row and manual; e.g. auto[0].AUTO_HighRatioDByState[0] ← file +0x117bc =
`IQ_YEE_HIGH_DIR[1]`), and the converter `IQ_API20_SET_YEE` swaps the same way, so a Get after a Set returns what was set and
API state 0 drives element 1, state 1 element 0: HighRatioDByState[0] → 1334.38, [1] → 1334.37, [2] → 1334.39;
HighRatioUDByState[0] → 1334.3b, [1] → 1334.3a, [2] → 1334.3c [R] [I: the vendor API's order of the three edge states, not a
one-sided bug]. imx307: `IQ_YEE_HIGH_ALL` = 200, 86, 86 → a Get returns HighRatioUDByState = 86, 200, 86.

All other conditionals of the function are value clamps (CCM SUM column, HSV sign-magnitude, NR3D YSfBlendLut) or loop bounds [R: 17 `if`
sites read].

## 7. CCM, HSV, effects, temperature

### 7.1 Tables [R]
- CCM IQ-file table ctx+0xac7a, 16 rows × 0x3c (row index = **colour-temperature** step for the matrix, ISO step for the gain
  field); row: +0 `IQ_CCM_EN` u8, +2 `IQ_CCM_KEEP_HUE_EN` u8, +4 `IQ_CCM_HUE_TH` u16, +6 `IQ_CCM_COEFF` 9 × u16 (13-bit two's
  complement, 1024 = 1.0, row-major), +0x18 `IQ_CCM_SAT_GAIN` u16 (256 = 1.0), +0x1a `IQ_CCM_CCT_THR` 16 × u16 (an array, the
  same in every row; row 0's copy is ctx+0xac94). API table ctx+0x277f8 (same layout); output block ctx+0x1baa4 (one row);
  per-parameter ITP flags ctx+0x5564c… (6 words), ITP record 0x41f at ctx+0x53a76.
- HSV IQ-file table ctx+0xb03a, 16 rows × 0x34 (by colour temperature); row: +0 `IQ_HSV_EN`, +2 `IQ_HSV_HUE` 24 × u8
  (sign-magnitude: bit 7 = minus, ±64), +0x1a `IQ_HSV_SAT` 24 × u8 (64 = 1.0). API table ctx+0x27bb8; output block ctx+0x1bae0;
  ITP flags ctx+0x5566c… (3 words), ITP record 0x420 at ctx+0x53a7e.
- Row offsets of both come from `.rodata` u16 tables (CCM {0, 2, 4, 6, 0x18, 0x1a}, HSV {0, 2, 0x1a}); default / identity
  matrix {1024,0,0, 0,1024,0, 0,0,1024} at `.rodata` label +0x46f3; luma weights {316, 624, 84} (= 0.309, 0.609, 0.082) at
  +0x468f.

### 7.2 `IQ_PARSE_CCM` (mode-1 load) [R]
Loads enums 0x2fd–0x302 into the 16 rows. Then checks `CCM_CCT_THR` (row 0 +0x1a, 16 values): all zero → replaced by {2500,
3000, 5000, 0…} and "Parse u16CCTthr Error!!!"; otherwise it must rise strictly until the first 0 (entries after the first 0 are
zeroed); a non-rising pair → replaced by the same defaults and "Check CCT thr to be L<M<H!!!". These are the CT thresholds
`IQ_ISOIndexGet` copies (§4.1 step 7).

### 7.3 `IQ_SET_CCM(ctx, iso, ct_index, mode)` [R]
1. mode 1: `IQ_PARSE_CCM`, ITP length check (6), flags cleared / filled from the ITP record (byte 0 forced on, §5.1).
2. **`IQ_INTERPA_CCM`** fills a work struct W (two rows L / H of 0x3c B + axis values): CT rows lo = ctx+0x53838 (0) if ≥
   ct_index, else ct_index − 1; hi = ct_index (both mapped through the identity table ctx+0x537f8). Axis values: W+0x88 / 0x8c /
   0x90 = CT threshold[lo] / current CT (ctx+0x537a0) / threshold[hi]; W+0x7c / 0x80 / 0x84 = gain-table[iso−1] / gain /
   gain-table[iso]; W+0x78 / 79 / 7a = iso, ct_index, mapped hi.
   - API mode (3): L / H = API rows lo / hi. A saturation percentage s is interpolated **by gain** between the per-ISO CCM_SAT
     bytes (API side table ctx+0x1caf0[iso−1], [iso]; capped at 100). If s = 100 the matrices are used as they are; s = 0 → both
     coefficient sets are replaced by the identity; otherwise each coefficient c := `IQ_CCM_inter(s, 0…100, identity, c)` — the
     API matrix is pulled towards the identity by (100 − s) % (this is what `CCM_SAT` does in API mode).
   - File mode: L / H = file rows lo / hi; their SAT_GAIN fields (+0x18) are replaced by the file rows **iso−1 / iso** (the gain
     field is indexed by ISO, the matrix by CT).
3. **`IQ_DOINTER_CCM(ctx, W, …, out)`**: for EN, KEEP_HUE_EN, HUE_TH, SAT_GAIN, CCT_THR: `IQ_para_inter` **by CT** (x = current
   CT between threshold[lo] and [hi]) with the parameter's ITP flag; COEFF: each of the 9 coefficients `IQ_CCM_inter(flag, CT, …,
   L, H)` — linear, on 13-bit signed values (> 0x1000 = negative), result back to 13-bit — then the result B is multiplied by the
   matrix A = IqInput+0x64 (9 × u16, ch+0x154, §2.2) by **`IQ_CCM_multiplier`**: out[r][c] = (Σk A[r][k]·B[k][c]) >> 10 for c =
   0, 1 and out[r][2] = 1024 − out[r][0] − out[r][1]. Finally SAT_GAIN (+0x18) is blended again, **by gain** (W+0x7c…0x84), with
   flag 4. On this camera A is the identity (§2.2), so the product is B itself except that each row's third coefficient is
   replaced by 1024 − the other two (row sums forced to 1.0) [R formula] [I: the IQ file's third column only matters through the
   row sum].
4. `IQ_DoEffect(24)` (no CCM case in the effect table, so nothing); then three channel saturations, each 256 × SAT_GAIN
   (out+0x18) >> 8 × t/50, where t = `IQ_DOINTER_TEMP(entry[lo].ColorToneRatio[k], entry[hi]…, manual…, 50)` for k = 0, 1, 2
   (Temperature API: 50 = neutral; with the Temperature API off t = 50). The division is the multiply by 0x51eb851f >> 36 (= /50).
5. **`IQ_CCM_SaturctionCalc(out, sat[3], 0)`**: per row r, s_r = sat_r·100 >> 8 (%); saturation matrix S[r][r] = s_r +
   L_r·(100 − s_r)/1024, S[r][c≠r] = L_c·(100 − s_r)/1024 (L = 316, 624, 84), each row of S renormalised to sum 100 by adjusting
   the diagonal; M' = S·M / 100; rows renormalised to 1024 on the diagonal; diagonal clipped to 0xfff; stored 13-bit. Overflow
   guard: if max(sat) > (0x800000 / max diagonal of M) >> 3 all three sats are set to that limit and "CCM primary coef overflow
   (4095)" is printed. (Argument 1 = the column-wise L table variant, never used.) The matrix is thus desaturated towards luma by
   SAT_GAIN × the Temperature ratio.
6. `IspRegSetCCM(out)` (§10.2). Then the switch falls through into IP 25: `IQ_SET_HSV` (§7.4).

`IQ_CCM_inter(dbg, flag, x, x0, x1, y0, y1)`: like `IQ_para_inter` but exact linear in 13-bit signed (y0, y1 > 0x1000 → −0x2000;
result < 0 → +0x2000); flag ≠ 1 / y0 = y1 / x > x1 → y1, x < x0 → y0; dbg prints.

### 7.4 `IQ_SET_HSV(ctx, iso, ct_index, mode)` [R]
1. mode 1: load 0x305–0x307 into the 16 rows; ITP check (3); flags from the ITP record when its byte 0 is 1.
2. `IQ_INTERPA_HSV`: rows lo / hi chosen as for CCM (by CT) from the API table (mode 3) or the file; axis values CT (W+0x78 /
   0x7c / 0x80) and gain (W+0x6c / 0x70 / 0x74).
3. `IQ_DOINTER_HSV`: EN and SAT[] by `IQ_para_inter` over CT; HUE[] by `IQ_HSV_hue_inter` — linear in signed values decoded from
   sign-magnitude (v > 0x80 → 0x80 − v) and re-encoded (negative → −y | 0x80).
4. `IQ_DoEffect(25)` (no HSV case).
5. **`IQ_HSV_Update_Iqsub_ByIso`** — only when the HSV API (0x1018, struct ctx+0x45c10) is enabled; it recomputes HUE[] and SAT[]
   of the output block from the API struct, ignoring step 3's values:
   - w = CCM saturation % interpolated by gain (CCM_SAT bytes ctx+0x1caf0[iso−1], [iso]); g = GLOBAL_SAT interpolated by gain
     (auto) or the manual GLOBAL_SAT (OpType manual, +4 ≠ 0).
   - Per hue bin j (24): hISO = HUELUT_ByIso by gain, hCT = HUELUT by CT, hU = HUELUT_FORUNITCCM by CT; sISO, sCT, sU likewise
     from the SATLUT arrays (auto entries interpolated between the two rows; manual = the manual arrays). Hues clamped to
     [−64, 64], sats to 255.
   - hue = blend(w: 0 → clamp(hISO + hU), 100 → clamp(hISO + hCT)) (sign-magnitude); sat = blend(w: 0 → g·sU·sISO >> 12, 100 →
     g·sCT·sISO >> 12), clipped to 255. So when the CCM is pulled to the identity (low CCM_SAT) the "for unit CCM" LUTs take over.
   - out HUE[j] = hue (ctx+0x1bae2+j), SAT[j] = sat (+0x1bafa+j).
6. `IspRegSetHSV(out)`.
The struct fields are indexed as auto[CT index] for HUELUT / SATLUT / FORUNITCCM and auto[ISO index] for the _ByIso arrays and
GLOBAL_SAT — the 16 "auto" entries of this API mean different axes per field [R offsets +8 / +0x68 / +0x128 with stride 0x122].

### 7.5 Effects and the Temperature ratio [R]
- `IQ_DoEffect(ctx, ip)` applies the Effect API (slot 27; eight styles; the table of styles and the enums each overwrites:
  isp-3-iq-api.md §3.5) to the output block of the IP being run. With the official names: Emboss / Sketch write `IQ_YEE_*` items of
  the YEE output block; Sepia sets `IQ_COLORTRANS_MIX_EN` (0x406) and `IQ_COLORTRANS_PREOFFSET` (0x407) and `IQ_COLORTRANS_U_OFFSET`
  = 0x762, `IQ_COLORTRANS_V_OFFSET` = 100. No enum named `IQ_COLORTRANS_EN` exists. The COLORTRANS row: +0 = 0x406
  `IQ_COLORTRANS_MIX_EN` (the bit Enable / bypass drive), +2 = 0x407 `IQ_COLORTRANS_PREOFFSET`, +4 = 0x408
  `IQ_COLORTRANS_DITHERING_MODE` (133E.68 bits 0, 1, 2; PREOFFSET = 1 in the imx307 row and in every effect); the API struct
  feeding it is 0x101d Colortrans (isp-3-iq-api.md §3.5). Offsets: 133E.72[10:0] `IQ_COLORTRANS_Y_OFFSET`, 133E.73[10:0]
  `IQ_COLORTRANS_U_OFFSET`, 133E.74[10:0] `IQ_COLORTRANS_V_OFFSET`; they are 11-bit two's complement in 10-bit units [I] (0x762
  = −158, V +100: Cb < 0, Cr > 0, a sepia brown).
- **`IQ_DOINTER_TEMP(ctx, lo, hi, def, dflt)`**: Temperature API on (ctx+0x53414) and its mode word ctx+0x53428 = 0 →
  `IQ_para_inter(1, temperature<<24, thr[idx−1]<<24, thr[idx]<<24, lo, hi)`; else `def`. Users: the patches of BNR, NR3D, OBC,
  OBC_P1, DPC, ANTICT, SpikeNR, YEE, YCUVM10 (§9) and `IQ_SET_CCM` (which reads ColorToneRatio[0], [1], [2]). The temperature is
  the constant 36 of IqInput+0x1c (§2.2).

## 8. Calibration apply (`pIQ_Cali_*`, from the calibration ids 0x24xx) [R]
All four parse a calibration blob (`d`) produced by the vendor's calibration tool, write the result into an API struct (or the
IQ-file SDC table) and re-run the IP. Formats as read by the code (little-endian); the ids themselves: isp-3-iq-api.md §4.5.

**OBC, id 0x2410 → `pIQ_Cali_OBC_Apply(ctx, d)`**: d[0] = version 3 (else "not support version %d", −1). 16 entries of 8 bytes
at d+0x34: {u16 R, Gr, Gb, B} → BlackLevel (0x1014) auto[i]; manual = entry 0; Enable = 1, OpType = 0; then
`IQ_API20_SET_OBC(iso)` at once and dirty[4] = 1.

**SDC, id 0x2414 → `pIQ_Cali_SDC_Apply`**: d[0] = 4; d+0x18 (u32) item type must be 2 ("Item (%d) is not mixer mode"); d+0x1e u16
n ≤ 256 ("Table size (%d) is larger than 256"). A block of n × 16 bytes follows the 0x40-byte header (not used); then 8-byte
records {u16 x, u16 y, u16 z, u16 pad} from d+0x40+16n — **always 256 of them, whatever n is** — packed as z·2^26 + y·2^13 + x
into the 256 words of the SDC IQ-file row at ctx+0xa1b8 (`IQ_SDC_*` table), SDC row enable ctx+0xa1ac = 1; `pIQ_IQTrigger(4,
22)`. No SDC API exists; a later mode-1 reload of the IQ file would overwrite it.

**LSC, id 0x2417 → `pIQ_Cali_LSC_Apply`**: d[0] = 5. Header 0x14 B: d+0xc u8 c = number of CCT records (≤ 3), d+0x10: byte 0 =
table size s (1…32), byte 2 = layout flag f. Then c × 0x3c B per-CCT headers (copied, unused). Then c records: {u8 valid-mask,
u8 shift, u8 ?; pad (1 B if f = 1, none otherwise); u16 center X, u16 center Y, u16 CCT; u16 R[s], G[s], B[s]; pad 2 (f = 1) /
3 (f = 0) / 0}. Record k counts as valid for colour-temperature slot k if bit k of its mask is set. Slot choice (low, mid, high
CT): 3 valid → {0, 1, 2}; 2 valid → {f0 ? 0 : 1, f1 ? 1 : 2, f2 ? 2 : 1}; 1 valid → all three = that record; 0 or > 3 → "Total
table number is %d", nothing applied. Errors: c > 3 ("CCT Number (%d) is larger than %d"), s = 0, s > 32. Applied: LSC struct
(0x1021) EN = 1, CENTER_X / Y and SHIFT from the low-slot record (RATE_X / Y untouched), R / G / B_GAIN_Table[k] = 32 u16 of the
slot-k record — 64 bytes are copied even when s < 32 (reads past the s-entry heap row); the slot CCTs go to the **ALSC**
struct's CCTThr[k]; then `SET_LSC`, dirty[35], `SET_ALSC`, dirty[37].

**ALSC, id 0x2415 → `pIQ_Cali_ALSC_Apply`**: d[0] = 5. d+0xc u8 c ≤ 3; d+0x10 u8 GridX ≤ 27, u8 GridY ≤ 17 (product > 0). Then
c × 0x34 B headers (unused). Records start at d + 0x18 + 0x34·c: {u8 valid-mask at −4, u16 CCT at −2; u16 R[X·Y], G[X·Y],
B[X·Y]; u8 DeltaLutX[max(X,Y)], u8 DeltaLutY[max(X,Y)]}, next record at +6XY + 2·max + 4. Slot choice as for LSC. Applied:
ctx+0x55c14 (grid used by the ALSC patch and the debug pattern) = GridX | GridY << 8; ALSC struct (0x1028) EN = 1,
DeltaModeAdvEn = 1, DeltaMode = 0, GridX / Y, DeltaLutX / Y from the low-slot record (26 bytes each, zero-filled first),
CCTThr[k] and the three gain pages per slot (row stride 27 u16, GridY rows of GridX values); `SET_ALSC`, dirty[37].

## 9. Per-IP patches (`IQ_Patch_*`) [R]
Run by `hal_IQ_Set_ALL` on the IP's output block after the blend and the effect, before `IspRegSet`. 27 of the 44 are stubs
(DECOMP, NR3D_P1, FPN, FPN_P1, GAMA_16to16_ISP, GAMA_C2C_P1_ISP, LSC_P1, ALSC_P1, HDR, GAMA_16to12, GE, SDC, DM, GAMA_C2A_ISP,
R2Y, NLM, XNR, SIIR, ACLUT, PFC, GNR, UVM, WDRGBL, ADJUV, MXNR, YUVGAMA, COLORTRANS — `movs r0,#0; bx lr`, calls removed by the
compiler). "t(X)" below = `IQ_DOINTER_TEMP` of the Temperature API ratio X (50 = neutral; API off → 50); "·t/50" is computed as
(v·t·0x51eb851f >> 32 & 0xfff) >> 4 or the equivalent >> 36 form, i.e. v·t/50 truncated.

### 9.1 Simple ones
| patch | field (output block, official name) | rule |
|---|---|---|
| `IQ_Patch_BNR` | `IQ_BNR_FILTER_STR` (+8) | v·t(NRLumaRatio)/50, 63 if v·t ≥ 3200 |
| `IQ_Patch_OBC`, `_OBC_P1` | `IQ_OBC[_P1]_{R,GR,GB,B}_OFST` (+4…+0xa) | v + t(ObcOffset) (s16, API off → 0), clamped 0…0xffff |
| `IQ_Patch_DPC` | `IQ_DPC_TH_{R,GR,GB,B}` (+0xc…+0x12) | 0xfff − (0xfff − v)·t(DynamicDPRatio)/50, clamped 0…0xfff (a higher ratio lowers the threshold) |
| `IQ_Patch_ANTICT` | `IQ_ANTICT_SFT` (+8) | 31 − (31 − v)·t(CrosstalkRatio)/50, clamped 0…31 |
| `IQ_Patch_SpikeNR` | `IQ_SpikeNR_NR_COEF` (+4) | v·t(NRDeSpikeRatio)/50, 15 if v·t ≥ 800 |
| `IQ_Patch_YCUVM10` | `IQ_UVM10_M[0]`, `[3]` (the U and V gains) | v·t(SaturationRatio)/50, 127 if v·t ≥ 6400; then if ColorToGray (ctx+0x53762) = 1: `IQ_UVM10_M[0…3]` = 0 and `IQ_YC10_EN` = 1 (gray) |
| `IQ_Patch_RGBIR` | `IQ_RGBIR_OFT_R_RATIO_BY_Y[…]` (+0x12, 0x24 B) | zeroed when IQ mode is night (ctx+0x538dc = 1) and RGBIR is enabled |
| `IQ_Patch_WDRLOC` | `IQ_WDRLOC_FB_V_HDR[]` (+0xd8, 0x40 B), `IQ_WDRLOC_EN` | only in an HDR pipe (IqInput+0x50 = 1) and IP 20 not in API mode: enable := 1 and the array is copied from **IQ-file row 12** (ctx+0x95fc) whatever the ISO |
| `IQ_Patch_YEE` | — | meant to scale `IQ_YEE_SCALE_P` / `_N` by t(SharpnessRatio[0] / [1])/50 (255 if ≥ 12800), but it writes the **YEE API table** at ctx+0x2e3d8 + k·0x291 and +10 (k = 0…8) — row k's element k of SCALE_P / SCALE_N (the row stride is 0x290, so the pointer drifts by one per row), not the output block [R disassembly 0x70950–0x7099a]. With t = 50 the values are unchanged; with any other ratio the API table is rescaled cumulatively on every run and the effect reaches the registers only through the next API-mode blend — a defect |
| `IQ_Patch_LSC` | `IQ_LSC_{R,G,B}_GAIN_TABLE[0…31]` | only when LSC_CTRL (0x1029) is enabled: r = `Interp_LUT_IQ`(POINTS_NUM, RATIO[16], BV[16], IqInput+8 = BV); per channel c: r_c = r·CTRL_c_RATIO_BY_CCT[ALSC index] >> 7 (128 = 1.0); v := (r_c·v + 0x40) >> 7 |

### 9.2 NR3D
`IQ_Patch_NR3D` (IP 2): first `IQ_Patch_NR3D_AEAccControlMdThGain`; then `IQ_NR3D_SW_K1` (a u16 at +0x192 that this patch reads
and writes through its low byte only) := k1·t(NR3DRatio[0])/50 (255 if ≥ 12800), `IQ_NR3D_SW_K2` (u16 +0x194) :=
k2·t(NR3DRatio[1])/50 (0x27f6 if k2·t > 0x7ce3d); then if the pipe is the normal one (ctx+0x58038 ≠ 0)
`…_calc_motion_th_gain(0, iso)` directly; in pipe mode 4 only after `…_calc_noise_profile` succeeded. So **in the normal Bayer
pipe the hardware noise estimate is not used** — the motion thresholds come from the IQ file's `IQ_NR3D_NOISE_MEAN_*` / `_STD_*`
(interpolated).
- **`IQ_Patch_NR3D_calc_noise_profile(ctx, 0)`** — input: the NE block (IqInput+0x78 = ch+0x19c80, unpacked every frame start
  by `ISP_DNRReadIQNoiseEst`, isp-1-hal.md §8.3): for channel c = R, G, B (block stride 0xa8): 12 u16 luma levels at +0, 12 u32
  sums at +0x18, 12 u16 counts at +0x48, 12 u32 sums at +0x60, 12 u16 counts at +0x90. Returns −1 if a pointer is 0 or all 12
  levels are 0. mean[i] = sum1[i]/count1[i], std[i] = sum2[i]/count2[i] when the count > ctx+0x53bd8 (16), else 0. Holes are
  filled: a zero first entry takes the first non-zero one (all zero → default 0x20 = ctx+0x53bda / std 0x20 = ctx+0x53bdc), a
  zero last entry the last non-zero one, inner zeros the mean of the nearest non-zero neighbours. Then an IIR with weight
  ctx+0x53bde/4 (= 1/4): state levels (ctx+0x53bf4 + 0x18c), means (+0x53c3c), stds (+0x53c84) += (new − state)/4. The states
  are copied into `IQ_NR3D_NOISE_MEAN_{R,G,B}[12]` and `IQ_NR3D_NOISE_STD_{R,G,B}[12]` of the output block; returns 0.
- **`IQ_Patch_NR3D_calc_motion_th_gain(ctx, 0, iso)`**: noise source = the IIR state (pipe mode 4), the `…_HDR` noise arrays of
  the output block (HDR pipe) or `IQ_NR3D_NOISE_MEAN/STD_*` (otherwise). k1 = `IQ_NR3D_SW_K1`, k2 = `IQ_NR3D_SW_K2`. A debug
  override takes them from NE-block word +0x1fa when NE-block word +0x1f8 (IqInput+0x78 +0x1f8) is 0xdb01 (0xdb02 prints k1, k2)
  — unreachable, `ISP_DNRReadIQNoiseEst` zeroes that word every frame. For each of 12 luma bins j: mt = MdThdByY side table
  (ctx+0x53d8c + 12·row + j) and mg = MdGainByY (ctx+0x53ccc + 12·row + j), each interpolated by gain between rows iso−1 and iso
  (`IQ_para_inter`, flag 1); `IQ_NR3D_MOTION_TH_{R,G,B}[j]` = min(k1·mt·NOISE_MEAN_c[j] >> 11, 0x3fff);
  `IQ_NR3D_MOTION_GAIN_{R,G,B}[j]` = 0x1000 / max(k2·mg·NOISE_STD_c[j] >> 11, 1), or 1 if that product exceeds 0x1000. The
  per-bin factors are the noise mean (thresholds) and noise std (gains) arrays.
- **`IQ_Patch_NR3D_AEAccControlMdThGain`** (inside the NR3D patch): AE change d = max over the 1024 AE blocks of |Y now − Y last|
  (byte 3 of each 4-byte block of the AE statistics at IqInput+0x54; the last frame's Y values are kept in a **static 1024-byte
  `.bss` array** shared by all channels). Active only when the Dummy API (0x1033, ctx+0x4c7b8) is enabled — this "Dummy" struct
  is the AE-accuracy control of 3DNR: auto[iso].Dummy0[0…3] (OpType 0, interpolated by gain between rows iso−1 and iso) or
  manual.Dummy0[0…3] = {AEDiff_Th, MdTh_Ratio, MdGain_Ratio, LogEn} (any entry −1 → do nothing); the other Dummy0 entries are not
  read. If d < AEDiff_Th: SW_K1 := k1·MdTh_Ratio/64 (255 if ≥ 0x4000), SW_K2 := k2·MdGain_Ratio/64 (0x27f6 if > 0x9fdbf).
  LogEn ≠ 0 prints "[AE Acc] AEDiff_Max", "[API] AEDiff_Th …", "[Final] MdTh …".
- **`IQ_Patch_NR3D_bAEAccControlMdThGain`** (`hal_DoIQ`, `hal_DoIQPerFrame`): when the Dummy API is enabled and its four values
  are set, re-runs IP 2 (`hal_IQ_Set_ALL(2, iso, NR3D's mode)`) — i.e. NR3D every frame while the AE-accuracy control is on.

### 9.3 ALSC (`IQ_Patch_ALSC`) — ALSC_CTRL and the ALSC_ADJ state machine
Grid: ctx+0x55c14 / +0x55c15 := `IQ_ALSC_GTBL_WIDTH_M1` + 1, `IQ_ALSC_GTBL_HEIGHT_M1` + 1 (27 × 17 at most). Gain tables in the
output block: R at ctx+0x1a680, G at +0x1aa16, B at +0x1adac (u16, 1024 = 1.0, 460-entry stride).
1. **ALSC_CTRL** (0x102a, ctx+0x4c70c) enabled: r = `Interp_LUT_IQ`(POINTS_NUM, RATIO[16], BV[16], BV); r_c =
   r·CTRL_c_RATIO_BY_CCT[ALSC index] >> 7. For each grid point: m = min(R, G, B); n_c = c·1024/m (the colour part, luma shading
   removed); new_c = 1024 + r_c·(n_c − 1024)/28 if r_c < 28, else n_c + ((r_c − 28)·(c − n_c) + 64)/100. So r_c = 0 → flat 1.0,
   28 → colour-only correction, 128 → the table as it is.
2. **ALSC_ADJ** (0x103c, ctx+0x53520) — a closed-loop R / B shading trim from the AE block statistics. Off → one-shot byte
   ctx+0x55c2c := 0, return. Fields: CPX_TH (u16), MAX_R / MIN_R / MAX_B / MIN_B (u16, 256 = 1.0), CHK_RANGE (clamped 8…16),
   UNIFORM_TH, DBG, POINTS_NUM, RATIO[16] (%), BV[16]. State at ctx+0x55c18: gainR, gainB (u16, 256 = 1.0), lastRatioR / B
   (0xffff = none), dirR / dirB (1 down, 2 up), window counter (+0xa), converged count (+0xb), fail count (+0xc), retry count
   (+0xd), state (ctx+0x55c28), pause counter (ctx+0x55c2d). State names (`.rodata`): 0 SHADING_CHK, 1 COMPLEX_PAUSE,
   2 COMPLEX_WATCH, 3 STABLE_PAUSE, 4 STABLE_WATCH, 5 NO_IMPROVE_PAUSE, 6 STATE_ERR, 7 STATE_TESTING.
   - w = `Interp_LUT_IQ`(POINTS_NUM, RATIO, BV, BV) (how much of the trim applies at this brightness, %).
   - First run (one-shot 0): state 0, gains 256 / 256, last ratios none, dirs 0, window 30, counts 0; one-shot := 1 (this is the
     byte `hal_DoIQPerFrame` uses to run IP 12 once more).
   - No AE statistics (IqInput+0x54 = 0): state 6; in state 6 the gains decay each run, g := (9g + 256)/10. DBG = 1 → state 7:
     the check runs every frame with prints and the gains are reset to 256.
   - The check `Check_Shading_Performance` (§9.4) returns 1 with ratioR / ratioB (256 = flat) or 0 (invalid).
   - State 0 SHADING_CHK: valid → for R (and B alike): ratio < 256 → gain += 1 (skipped if already going up and the ratio got
     worse), dir 2; ratio > 256 → gain −= 1 (same rule), dir 1; ratio = 256 → converged + 1; last := ratio. Invalid → last
     ratios none, dirs 0, gains decay as above, fail + 1. At the end of a 30-run window: converged ≥ 32 → state 3, pause 15;
     fail > 14 → state 1, pause 15; else retry + 1 and a new window (`Check_Shading_Init(st, 0)`), after 5 retries state 5 with
     pause 45.
   - 1 COMPLEX_PAUSE / 3 STABLE_PAUSE / 5 NO_IMPROVE_PAUSE count the pause down and go to 2 / 4 / 0 (0 with a full re-init).
     2 COMPLEX_WATCH: check valid → 0 (re-init), invalid → 1 (pause 15). 4 STABLE_WATCH: invalid → 1; valid with both ratios in
     255…257 → 3 (pause 15); otherwise → 0 (re-init).
   - Limits each run: maxR = (w·clamp(MAX_R, 256, 512) + (100 − w)·256)/100, minR = (w·clamp(MIN_R, 128, 256) + (100 −
     w)·256)/100, likewise B; gains clamped into [min, max]. DBG = 0xf0 prints w and the limits, DBG = 2 prints state name,
     gains, ratios.
   - Apply: for every grid point R := (R − 1024)·gainR/256 + 1024, B := (B − 1024)·gainB/256 + 1024 (G untouched).

### 9.4 `Check_Shading_Performance` [R: decompile + emulation on synthetic 32 × 32 AE grids]
<!-- ev:isp-4-iq-engine.9.4 -->
Functions `Check_Shading_Performance` (`.text` 0x613b0), `Calc_STDEV_Power` (0x61118), `Get_Color_Ratio` (0x6114c),
`Get_Uniform_Rows_average` (0x61296).

**Call** (from `IQ_Patch_ALSC`): `Check_Shading_Performance(ae, VR = ctx+0x55c30, VB = ctx+0x56430, HR = ctx+0x56c30, HB =
ctx+0x57430, &ratioR, &ratioB, CPX_TH, range, dbg, frame = ctx+0x55bfc, UNIFORM_TH)`; ae = IqInput+0x54 (layout §2.2: 0x80 bytes per grid
row); dbg = the ALSC_ADJ DBG field in state 0, the constant 1 in state 7
(STATE_TESTING); range = CHK_RANGE clamped 8…16 by the caller.

**Lines.** L runs over 16 − range … 16 + range − 1. Two passes; each pass forms, per L, one horizontal and one vertical line of
16 blocks [R emulation, every slot checked block by block for range 8 and 16]:

| pass | line kind | blocks (row, col) | buffer order k = 0…15 | line index | buffer slot |
|---|---|---|---|---|---|
| 0 | horizontal (half-row) | (L, 0…15) — left half of row L | col 0 → 15 (edge → centre) | L | L |
| 0 | vertical (half-column) | (0…15, L) — top half of column L | row 0 → 15 (edge → centre) | L + 64 | L |
| 1 | horizontal | (L, 16…31) — right half of row L | col 16 → 31 (centre → edge) | L + 32 | L + 32 |
| 1 | vertical | (16…31, L) — bottom half of column L | row 16 → 31 (centre → edge) | L + 96 | L + 32 |

Buffers: HR / HB hold horizontal lines, VR / VB vertical lines, 64 slots × 16 u16 (0x800 B each); slot s at byte 32·s. Within a
pass the lines are visited in L order, horizontal before vertical, and the uniform list is filled in that order (pass 0 first)
[R: list for range 2 = 14, 78, 15, 79, 16, 80, 17, 81, 46, 110, 47, 111, 48, 112, 49, 113].

**Per block** [R]: R/G = (255·R + G/2) / G and B/G = (255·B + G/2) / G (integer); G = 0 → 255·R and 255·B
(no division). Values are u16 and are not clipped (R 255, G 1 → 65025). A block is **dropped** (its R/G and B/G stored as 0)
unless 10 ≤ Y ≤ 128 (unsigned test Y − 10 ≤ 0x76: Y 9 and 129 dropped, 10 and 128 kept). Drops are counted separately for the
horizontal and the vertical line.

**Per line** (each of the two ratios):
- n = 16 − dropped; mean = (Σ ratio + n/2) / n (the zeros of dropped blocks add nothing to the sum).
- spread = `Calc_STDEV_Power(line, 16, mean)` = Σ over non-zero entries (4·|v − mean|)² / (number of non-zero entries) — a
  variance of 4·v, no square root (the decompile shows `void`; r0 carries the quotient [R disassembly]); a block whose ratio is
  genuinely 0 (R = 0) is excluded like a dropped one [R: ±7 zig-zag → 784, also with two zeros].
- The line is **uniform** when dropped < 8 **and** spreadR < CPX_TH **and** spreadB < CPX_TH, all strict [R: spread 64 is
  uniform at CPX_TH 65, not at 64; 7 drops pass, 8 do not]. Its line index and the two spreads are appended to the uniform list
  (≤ 128 entries).
- dbg = 4 prints every line ("Complex R_H … BDCount_V"); dbg = 1 and frame % 5 = 0 prints the uniform list.

**Result**:
- Luma gate: ΣY over all 1024 blocks is accumulated in a **u16** and compared ≥ 0x3c00. The sum wraps, so the gate is ΣY mod
  65536 ≥ 15360: a flat field of Y 15…63 passes, Y 64 (sum 65536 → 0) and Y 65 (→ 1024) fail, Y 79 passes again, Y 128 fails
  [R emulation: nine cases]. Bright scenes are rejected or accepted depending on where the 16-bit sum lands — a defect [I: the
  intent is a minimum-brightness test].
- If (number of uniform lines) > UNIFORM_TH (strict) and the gate passes: ratioR = `Get_Uniform_Rows_average(HR, VR, list)`,
  ratioB = the same over HB, VB; return 1. Otherwise ratioR / ratioB are left untouched and it returns 0.
- `Get_Color_Ratio(u16 *v, n = 16, idx, char *txt, int *err)` per listed line (idx = line index, minus 64 for vertical lines, i.e.
  the buffer slot): a least-squares line through the non-zero entries at x = k + 1, y = 32·v (b = (Σi²·Σy − Σiy·Σi)/D, a =
  (n'·Σiy − Σy·Σi)/D); slope32 = its slope (integer), residual *err = mean |y − fit| over the non-zero entries. Ratio = 256 +
  (slope32 + 4) / 8 (C division, truncating towards zero, so +1/block → +4, −1/block → −3), **negated (256 − …) when idx < 16**
  [R emulation: the same line gives 252 at idx 8 and 15, 260 at idx 16, 31, 32, 40; vertical idx 72 → slot 8 is negated, 104 →
  slot 40 is not]. It also `sprintf`s the 16 values + intercept + slope into txt, printed only when dbg = 0xff.
- `Get_Uniform_Rows_average(R, B, rows[], n, dbg)`: selection-sorts the lines by residual, largest first (unstable: equal
  residuals may change order through the swaps), and returns (Σ (i + 1)·ratio_i + W/2) / W with W = Σ (i + 1) (u16) over the
  sorted positions — the cleanest lines weigh most [R: (256, 264) → 261; noisy 273 + clean 264, 256 → 262].
- The sign rule splits the pass-0 lines at L = 16: lines L < 16 (rows above the centre for horizontal lines, columns left of the
  centre for vertical ones) are negated, pass-0 lines with L ≥ 16 and all pass-1 lines are not. Pass-0 buffers run edge → centre
  and pass-1 buffers centre → edge, so a centre-symmetric colour shading gives pass-0 L < 16 and pass-1 lines the same sign but
  pass-0 L ≥ 16 the opposite one [R arithmetic] [I: the split was probably meant to be at slot 32 (pass 0 vs 1), i.e. a defect
  that dilutes the measured shading by the share of pass-0 L ≥ 16 lines].
- `Check_Shading_Init(st, full)`: st+10 = 30, st+0xc = 0; full → st+8 = 0, st+4 = −1, st+0xd = 0.

## 10. Register writers (`WriteRegs`, `IspRegSet*`, `IspMLoadSet*`) [R]

### 10.1 `WriteRegs(cfg, shadow, shadow+0x4920, src, e)` — one cfg enum into the shadow image
- `Regmap[e]` (§12.1) gives bank code b, register r, mask m; N (element count) and the element size come from `Dbg_Items[e]`
  (`cfg_getcfg_info_byenum`, `IQ_PARAMETER_TYPE`, §13.3).
- Shadow base by bank code (the switch; RIU bank = 0x1300 + b): 0x02 → shadow+0 (bank 1302), 0x11 → +0x400, **0x12 → +0x4920**,
  **0x13 → +0x4b20**, 0x16 +0x600, 0x17 +0x800, 0x18 +0xa00, 0x19 +0xc00, 0x1a +0xe00, 0x1b +0x1000, 0x1c +0x1200, 0x1d +0x1400,
  0x1e +0x1600, 0x20 +0x1800, 0x21 +0x1a00, 0x22 +0x1c00, 0x25 +0x1e00, 0x26 +0x2000, 0x27 +0x2200, 0x28 +0x2400, 0x29 +0x2600,
  0x2a +0x2800, 0x2b +0x2a00, 0x2c +0x2c00, 0x2d +0x2e00, 0x30 +0x3000, 0x31 +0x3200, **0x3d +0x3400**, 0x33 +0x3800, 0x34
  +0x3a00, 0x35 +0x3c00, 0x38 +0x3e00, 0x39 +0x4000, 0x3a +0x4200, 0x3e +0x4400; register r at base + 4r (a 16-bit register per
  32-bit slot, high byte at +1). 0xff → return; any other code → base 0 (**a NULL write** if such an enum were ever passed; the
  IspRegSet loops only pass enums that have a register [R ranges]).
- Packing, with bits = popcount(m & 0xffff), shift = trailing zeros of m, per = 16 / bits:
  - m ≥ 0x10000 (32-bit fields): each element u32 → low half at register r, high half at r+1; next element at r+2.
  - bits ≥ 9 or N = 1: one element per register, value << shift, read-modify-write under m. **For a u32-typed element with a
    16-bit mask the value is not loaded** (the code only loads u8 / u16) and the previous element's value is written again — a
    defect.
  - bits < 9 and N > 1: `per` elements per register at successive bit positions; the first register starts at `shift` and holds
    (16 − shift)/bits elements, the last register the remainder (N − first) mod per.
  - Special enums: 0x19f `IQ_NR3D_PR_RATIO_CONF` (pairs e[i] | e[i+1] << 5 per register), 0x1b5 `IQ_NR3D_NPF_NODE_DIST` (one per
    register, 6 registers skipped after element 10), 0x33b `IQ_YEE_MOT_NR_STR` (e[0] alone, then pairs (e[i] << 8 | e[i+1]) for
    i = 1…14, then e[15] << 8), 0x352 `IQ_YEE_LABELN_WEISTR` (u16 one per register, one register skipped after element 6).

### 10.2 `IspRegSet<IP>(ctx, out)`
Every one: for each enum e of the IP, `WriteRegs(cfg, shadow, shadow+0x4920, out + off[e], e)` with `off[e]` from a `.rodata`
u16 table (the same row offsets as §5.2), then `shadow[0x4d20 + id] = 1` (the shadow id, §10.3). The per-enum register map is the Regmap table of §12.1. 16 IspRegSet functions are stubs
(among them NR3D_P1, FPN, FPN_P1, GAMA_16to16_ISP, GAMA_C2C_P1_ISP, LSC_P1, ALSC_P1, GE, NLM, ACLUT, WDRGBL) — their shadow ids are
never set. Derived register writes after the loop:

| function | extra writes |
|---|---|
| `IspRegSetINIT` | loops 0x10b–0x162 from ctx+0x186+off (the INIT block of the IQ file, §13.4); no dirty byte [I: the init values ride on the other IPs' applies] |
| `IspRegSetOBC` (OBC_P1: loop and dirty byte only) | pipe mode 1–3: 1312.50[0] (OBC_b enable) := 0. Pipe mode 0 (linear): with g = ISP digital gain (IqInput+0xc, 1024 = 1) and each channel offset o: v = g·o >> 10; v ≤ 1024 → a = 0, b = v; else a = (v − 1024)·1024/g, b = v − (g·a >> 10). Then 1312.00[0] = OBC_EN, 1312.01–.04 = a (pre-gain part), 1312.05–.08[13:0] = 0x400 (gain 1.0); 1312.50[0] = OBC_EN, 1312.51–.54 = b (≤ 1024, post-gain part), 1312.55–.58[13:0] = the `IQ_OBC_*_GAIN` values — the black level is split so that the second stage never removes more than 1024 [I: stage order] |
| `IspRegSetBNR` | 1326.17 + 1326.18[0] (17-bit signed) = clamp(((1326.16[8:0] − 1326.15[8:0])·256) / (1326.14[15:8] − 1326.14[7:0]), −65535, 65535) — a slope from two points |
| `IspRegSetNR3D` | 1328.47[12:0] = −4096 / max(1328.45[15:8] − [7:0], 1); 1328.48 likewise from 1328.46; 1328.73[12:0] = 4096 / max(1328.72[15:8] − [7:0], 1); 1328.75[8:0] = 256 / max(1328.74[7:4] − [3:0], 1); vertical flip (ctx+0x55bf4) → 1328.70[0] := 1; the first non-zero TfStr → 1328.04[0] pulse (shadow +0x2410; the 3DNR first-frame trigger `ISP_DNRTrigger1stFrame`) + re-run (isp-3-iq-api.md §3.7); fixed constants: 1329.00[4:0] = 0, [15:8] = 0xf8; 1329.01[4:0] = 4; 1329.02 = 0xff, [11:8] \|= 0xf; 1329.03[5:0] = 0x20; 1329.04[3:0] \|= 0xf, [11:8] = 2; 1329.05–.10 and 1329.1a–.55 = 0x20 with [13:8] = 0; 1329.56[4:0] = 12; 1329.57–.5c bytes = 1, 3, 5, 7, 10, 14, 20, 28, 40, 64, 112; 1329.70[4:0] and [12:8] = 12 (9 if the image width ≤ 3000) |
| `IspRegSetLSC` | if `IQ_LSC_SHIFT` = 0 (auto): 1319.01[12:0] = W/2, 1319.02[12:0] = H/2 (centre), 1319.03[4:0] = clamp(⌊log2((W/2)² + (H/2)²)⌋ − 8, 0, 31) |
| `IspRegSetALSC` | enabled: mirror / flip mode (ctx+0x55bec) ≠ 0 → `IspRegSetALSC_MirrorFlip` reorders the R / G / B gain planes of the output block in place (`…_MirrorFlipTbl`: mode bit 0 → mirror rows, bit 1 → flip, modes 4–7 → transpose; through two temporary buffers); then `IspMLoadSetALSC`. Modes 4–7: 131D.40[2] = 1, 131D.43 = {GridY−1, GridX−1} (swapped), 131D.44 = GridY, DeltaLutX / Y written swapped (two extra `WriteRegs`); else 131D.40[2] = 0, 131D.43 = {GridX−1, GridY−1}, 131D.44 = GridX. 131D.4a[11:0] = Σ_{i < GridX−1} 16·DeltaLutX[i]·1024 / W, 131D.4b = the same for Y / H |
| `IspRegSetHDR` | with e = `IQ_HDR_SENSOR_EXP_RATIO` and r = min(0x1000000/e, 0x1fff): 131E.01[12:0] = r, 131E.08[12:0] = r·10/1024, 131E.09[12:0] = r·1000/1024, 131E.07 = 0x10000 / (((r·1000 >> 10) − (r·10 >> 10)) >> 4); luma level S (**one static u16 in `.data`**, label `Dbg_Items`+0xa111, shared by all channels): mean of the 16 HDR-alignment values (IqInput+0x80) lying in (S/2, 3S/2) (0x400 if none), S := (3S + mean)/4 → 131E.0b–.1a[10:0] (16 registers); 131E.05 = min(0x10000/max(131E.04[13:0] − 131E.03[13:0], 1), 0xffff); 131E.2e = the same from 131E.2d / .2c[9:0]; 131E.00[5:2] = 0b1010; 131E.40[11:0] = min(0x100000/r, 0xfff); 131E.54[11:0] = e >> 4 (0x80 if e·64 < 0x20000); 131E.55–.58, .5a, .5c, .5e, .60 := 0, 131E.5b / .5d / .5f / .61[1:0] := 0; WB gains R, G, G, B (IqInput+0x28 / 0x2c / 0x2c / 0x30, [13:0]) → 131E.66–.69 and 131E.6f–.72; 131E.6a = ((r·0xfff + 0x200)·64) >> 16; 131E.59[11:0] = 0x3ffc00 / e; 131E.73 = clamp(S·64, 0, 0xffff) |
| `IspRegSetSpikeNR` | slope table of the luma curve: x[i] = 1317.30–.34[11:0], y[i] = 1317.35–.39[10:0]; 1317.3a = y0·1024/x0, 1317.3b–.3e = (y[i] − y[i−1])·1024/(x[i] − x[i−1]), 1317.3f = (1024 − y4)·1024/(600 − x4) |
| `IspRegSetWDRLOC` | `IspMLoadSetWDRLOC` when enabled; `IspRegSetWDRLOC_LocalSizeEX(shadow, W, H, IQ_WDRLOC_BOXNUM, 3)`: n = clamp(BOXNUM, 1, 4) − 1; 1322.0b / .0c[12:0] = W / H; the box count along the longer side comes from `.rodata` {8, 16, 24, 30} (short/long < 10/16), {4, 8, 16, 24} (< 13/16) or {4, 8, 16, 24} (else) indexed by n; box size s = ⌈long / count⌉ → 1322.06 and 1322.07[11:0] (square boxes); count on the short side = round(short / s); 1322.08 = {count_W − 1 [4:0], count_H − 1 [12:8]}; 1322.09 / .0a[12:0] = 0x10000/s; 1321.00 / .01[8:0] = s/3; 1321.02 / .03[11:0] = 0x3fff/(s/3). Frame 0: 1322.00[9] := 1 if WDRLOC is enabled and the latch is clear, re-run IP 20 next frame; later frames: latch := enable, 1322.00[9] := 0. Exposure-change ratio: if total gain or shutter changed since the last run (stored at ctx+0x55c00 / 04, initial 1024 / 300): q1 = old·1024/new, q2 = new·1024/old (gain × shutter products, 64-bit); each normalised by e = 0…3 (q ≥ 0x1000 / 0x2000 / 0x4000) → 1321.60[11:0] = min(q1 >> e, 0xfff), 1321.61[1:0] = e, 1321.62 / .63 for q2; unchanged → 0x400 / 0 |
| `IspRegSetPFC` | HDR pipe: 1327.04–.07 = 0, 1327.08–.0b[12:0] = 0x400, 1327.0c–.0f[12:0] = 0x1000. Otherwise 1327.04–.07[11:0] = OBC_b offsets (1312.51–.54) >> 4, 1327.08–.0b[12:0] = WB gains R, G, G, B (IqInput+0x28 / 0x2c / 0x2c / 0x30), 1327.0c–.0f[12:0] = 0x400000 / gain (inverse gains). Vertical flip → 1327.00[8] := 0 and 1327.30[0] := 0, else 1327.00[8] := 1 |
| `IspRegSetYEE` | 133A.5d[12:0] = 0x1000 / max(133A.5c[14:8] − [6:0], 1) |
| `IspRegSetGNR` | 1330.4d[12:0] = 0x1000 / max(1330.4c[14:8] − [6:0], 1) |
| `IspRegSetSDC` | `IspMLoadSetSDC` when enabled; 1316.6e[12:0] = W − 1, 1316.6f[12:0] = H − 1 |
| `IspRegSetGAMA_C2A_ISP`, `IspRegSetYUVGAMA` | `IspMLoadSet…` when the output block's enable byte is 1 |
| all others | loop + dirty byte only |

The 1328.04[0] pulse: whether the bit clears itself in hardware is not known (a register read after `MhalCameraOpen` would
show it).

### 10.3 Shadow ids and the apply functions [R]
<!-- ev:isp-4-iq-engine.10.3 -->
"Shadow id" = the dirty byte `shadow+0x4d20+id` (46 bytes, ids 0x00–0x2d) that `IspRegSet` sets and `DrvIsp_ApplyShadow(id)`
consumes (the 46-entry switch; the apply functions themselves: isp-1-hal.md §10.4). Registers are those written by the apply.

| ip | name | shadow id → apply | registers written by the apply |
|---|---|---|---|
| 0 | DECOMP | 0x00 DECOMP | 1311.00–10 |
| 1 | BNR | 0x2b BNR | 1326.00–01, 10–11, 13–18, 20–2f |
| 2 | NR3D | 0x01 / 0x02 3DNR (+ 0x1b / 0x1c 3DNR_NE, empty) | 1328.03–06, 20–23, 30–3f, 44–48, 5a–69, 70–77; 1329.00–10, 1a–5c, 70–79; 132A.00–71 (gaps); 132B.31–39, 48–7f |
| 3 | NR3D_P1 | — | — (`IspRegSetNR3D_P1` empty) |
| 4, 5 | FPN, FPN_P1 | 0x03 / 0x04 (empty apply) | — (`IspMLoadSetFPN` empty) |
| 6 | OBC | 0x05 OBC + OBC_b | 1312.00–08, 50–58 |
| 7 | OBC_P1 | 0x06 | 1313.00–08 |
| 8, 9 | GAMA_16to16_ISP, GAMA_C2C_P1_ISP | 0x07 / 0x08 (empty apply) | — |
| 10 | LSC | 0x09 | 1319.00–06, 10–6f |
| 11 | LSC_P1 | 0x0a (empty) | — |
| 12 | ALSC | 0x0b (+ menuload ALSC grid) | 131D.40–44, 4a–4b, 50–5c, 60–6c |
| 13 | ALSC_P1 | 0x0c (empty) | — |
| 14 | HDR | 0x0d | 131E.00–1a, 2b–65, 6a–6e, 73 |
| 15 | GAMA_16to12 | 0x2a | 1320.01–1e, 20–4f |
| 16 | DPC | 0x0e | 1316.00–05, 08–39, 50–64 |
| 17 | GE | 0x0f (empty) | — |
| 18 | ANTICT | 0x10 | 1317.00, 04–0f |
| 19 | RGBIR | 0x11 | 1318.00, 04–17, 19 |
| 20 | WDRLOC | 0x12 (+ 0x1a WDR table, empty apply; menuload) | 1321.00–04, 29–2b, 60–63; 1322.00–01, 05–0c, 10–1f, 30, 32, 34–37, 73–7d |
| 21 | SpikeNR | 0x13 | 1317.20–28, 30–48 |
| 22 | SDC | 0x14 (+ menuload SDC table) | 1316.6a–6f |
| 23 | DM | 0x15 | 1325.00–25 |
| 24 | CCM | 0x16 | 1338.00–01, 03–06, 08–10 |
| 25 | HSV | 0x17 | 1338.20–38 |
| 26 | GAMA_C2A_ISP | 0x18 (+ menuload RGB gamma 256 × {R, G, B}) | 1339.00, 02–03 |
| 27 | R2Y | 0x19 | 1339.10–1d |
| 28 | NLM | 0x1d (no apply) | — |
| 29 | XNR | 0x1e | 133D.00, 04–0d, 10, 12–13, 16–18, 1c |
| 30 | YEE | 0x1f | 1333.00–79 (gaps); 1334.00–6f (gaps); 1335.00–74 (gaps); 133A.4a–72 |
| 31 | SIIR | 0x20 | 133A.00–3f |
| 32 | ACLUT | 0x21 (no apply) | — |
| 33 | PFC | 0x22 | 1327.00–3f (gaps) |
| 34 | GNR | 0x2c | 1330.00–14, 18–53, 58–5f; 1331.00–3d |
| 35 | UVM | 0x2d | 133D.1b, 1d, 20–2f |
| 36 | WDRGBL | 0x23 (no apply) | — |
| 37 | ADJUV | 0x24 | 133E.20–3a |
| 38 | MXNR | 0x25 | 133E.00–09 |
| 39 | YUVGAMA | 0x26 (+ menuload YUV gamma) | 133E.40, 4b–4e |
| 40 | YCUVM10 | 0x27 | 133E.60–66 |
| 41 | COLORTRANS | 0x28 | 133E.68–74 |
| 42–47 | — | 0x29 (no apply) | — |
| 48 | EFFECT | — | — |
| 49 | DUMMY | — | — |

### 10.4 `IspMLoadSet*`
The five live ones (ALSC, YUVGAMA, SDC, WDRLOC, GAMA_C2A_ISP) stage tables for menuload (staging offsets, entry formats, SRAM
ids, dirty bytes at staging+0x38b0+id: isp-2-ispmid-mload-ispscl.md §12.3). With the official names: ALSC =
`IQ_ALSC_{R,G,B}_GAIN_TABLE` → SRAM 2; YUV gamma = `IQ_YUVGAMA_LUT_Y` + the two 128-entry UV LUTs → SRAM 3; SDC = `IQ_SDC_TBL` →
SRAM 6; WDR curve = `IQ_WDRLOC_FB_V` + `IQ_WDRLOC_FB_S` nibbles (`…_HDR` variants in an HDR pipe) → SRAM 12 (13 = previous copy),
`IQ_WDRLOC_NONLINEAR_C` → 9, `IQ_WDRLOC_LINEAR_C` → 10, `IQ_WDRLOC_ALPHA_C` → 14, `IQ_WDRLOC_SAT_C` (packed) → 15; RGB gamma =
`IQ_GAMA_C2A_ISP_LUT_{R,G,B}` → SRAM 17. Stubs: `IspMLoadSetFPN`, `_FPN_P1`, `_GAMA_16to16_ISP`, `_GAMA_C2C_P1_ISP`, `_ALSC_P1`.

## 11. Debug forcing and YUV pipe modes [R]

### 11.1 The IQ debug command word — register 1321.7E
`IQ_Debug_Log` (end of every `hal_DoIQPerFrame`) reads a u16 command c from NE-block word +0x1fc, i.e. ch+0x19e7c, which
`ISP_DNRReadIQNoiseEst` refreshes at every frame start from **register 1321.7E** (isp-1-hal.md §8.3); ispmid also treats the
value 0xdb as "freeze": it skips its IQ / 3A / menuload work while ch+0x19e7c = 0xdb (isp-2-ispmid-mload-ispscl.md §2.2). So a
tuning tool drives these hooks by writing a scratch register [I: 1321.7E / .7F are scratch words; `ISP_SaveDebugValue` writes
.7F]. The value on the running camera has not been read (expected 0 [I]).
- c = 0 or 0xdb → clear the print mode (ctx+0x57c30) and the last code (+0x57c32); done.
- Otherwise print mode := c & 0xfff (0x100 prints every triggered API id, 0x101 every triggered IP, §4.3). A new code (or one
  with c >> 12 = 1, "repeat") prints "Frame Count = %d, Debug Value = 0x%x" and, by c & 0xfff: 0x001 ISO index and gain, 0x002
  CT index and CT, 0x003 ALSC CT index, CT and the three ALSC CCTs, 0x102 the 53 API enables (ctx+0x55b18), 0x103 the 50 IP
  modes, 0x104 the 50 IP-present words. The actions below then run with verbose = 1; a repeated code runs them silently.
- Actions by c & 0xfff: < 0x500 with c & 0xc00 ≠ 0 → `IQ_IP_ITP_EnGet_ALL(ip = c & 0xff)`; 0x500–0x5ff →
  `IQ_IP_Bypass_ALL(c & 0xff)` (0x5ff: all IPs except 23 DM and 27 R2Y); 0x600–0x6ff → `IQ_IP_Reset_ALL(c & 0xff)` (0x6ff: all
  50); ≥ 0x700 with bit 11 clear → `IQ_IP_Special_ALL(c & 0xff)`.
- **`IQ_IP_ITP_EnGet_ALL(ip)`**: prints "name: flag" for every parameter of the IP — the per-parameter interpolation flags
  (ctx+0x54fec…, §5.2), with the official parameter names of §12.2 (the same `.data` name table, 48-byte entries; this is the
  code that uses it, which confirms the enum ↔ name alignment).
- **`IQ_IP_Bypass_ALL(ip)`**: switches one IP off until reset: output-block enable byte(s) := 0 (NR3D: `IQ_NR3D_STR` and
  `IQ_NR3D_STR_FIR`; UVM: +0 and +2; ADJUV: +0 and +4; GNR: `IQ_GNR_NRSTR[0..1]` and the three `IQ_GNR_STR_BY_{MOT,HUE,ROI}_EN`;
  others the IP's EN byte), `IspRegSet<IP>` at once, IP-present word := 0 (so `hal_IQ_Set_ALL` skips it), "Disable <name>".
  Cases exist for every IP with a register writer (0, 1, 2, 6, 7, 10, 12, 16, 18, 20–27, 29–31, 33–35, 37–41).
- **`IQ_IP_Reset_ALL(ip)`**: if the IP is not present: present := 1 and `pIQ_IQTrigger(4, ip)` (re-run); prints "Reset <name>".
- **`IQ_IP_Special_ALL(ip)`** — menuload test patterns, each leaving the IP disabled afterwards: ip 26 → linear RGB gamma (entry
  k = min(32k, 1023)) "Test Mload (bright image)"; ip 12 → ALSC grid of 1.0 with 0x1fff on the border points ("outside
  larger"); ip 20 → WDR curve 0, 0x80, … 0xf80 with +0x20 = 0xff ("bright image"); ip 39 → YUV gamma Y and UV LUTs all 0
  ("gray image").

### 11.2 YUV pipe modes
Pipe mode (IqInput+0x50) 4 = simple-Bayer / YUV→Bayer front (ch+0x199fc), 5 = YUV sensor:
- `hal_IQ_InitParameters` → `IQ_Yuv2Bayer_Init` (bypass every IP except 23 DM and 27 R2Y) or `IQ_Yuv_Init` (bypass all 50).
- Every `hal_IQ_Set_ALL` (mode ≠ 1) → `IQ_Yuv2Bayer_UpdateIPStatus(ip)`: for IPs {1 BNR, 2 NR3D, 20 WDRLOC, 23 DM, 24 CCM, 25
  HSV, 26 GAMA_C2A, 30 YEE, 34 GNR} (mask 0x47900006 plus 0x22): present := 1 only while the IP is in API mode (and NR3D
  re-triggered), else present := 0 and bypassed (DM and R2Y excepted from the bypass call); all other IPs present := 0.
  `IQ_Yuv_UpdateIPStatus(ip)` does the same for the YUV-domain IPs {29 XNR, 30 YEE, 31 SIIR, 34 GNR, 35 UVM, 37 ADJUV, 38 MXNR,
  39 YUVGAMA, 40 YCUVM10, 41 COLORTRANS} (mask 0x1f67 over ip − 29). So in these pipes an IP only runs when the application sets
  its API.

## 12. `Regmap` and the official names [R]

### 12.1 `Regmap`
<!-- ev:isp-4-iq-engine.12.1 -->
`Regmap` (`.data`, symbol `Regmap`) = 1072 × 8 B {u8 bank code (RIU bank − 0x1300; 0 = no register, 0xff = skip), u8 register,
u16 pad, u32 mask}, indexed by cfg enum (the same index as `Dbg_Items` and the IQ file). 671 enums have a register; 80 are section
markers. The enums with no register (bank code 0) are tables the IQ engine reads itself (ISO thresholds, CT thresholds, LUTs,
AE / AWB / AF defaults, ITP flags). The full per-enum table (names, registers, bit fields, row offsets, types, N, M and the imx307 values) is
[`isp-iq-regmap.tsv`](../data/isp-iq-regmap.tsv): one row per enum; `name_source` is `mhal` for the official names of §12.2
and `mercury5_guess` for the Mercury5-aligned guesses.

### 12.2 Official parameter names
<!-- ev:isp-4-iq-engine.12.2 -->
mhal carries the official names itself: right after `Dbg_Items` in `.data` (`Dbg_Items`+0x29e0) lies a table of char[48] names
for every IP-parameter enum — 531 names in IP-slot order for all IPs except YEE, then the char[15] IP-name table (50 entries:
DECOMP … COLORTRANS, ITP, IQINFO, SHOWLUT, INDTBL, TEMPTURE, PARAINTER, EFFECT, BLKMAX), then the 89 YEE names. They align
exactly with the enum ranges of §5.3 (counts match per IP, every name carries its IP prefix; `IQ_IP_ITP_EnGet_ALL` prints them
with the same indexing); 620 enums are named so. Examples: 0x1ce `IQ_OBC_EN` (1312.00 bit 0), 0x1cf
`IQ_OBC_DITHER_EN`.

The AE / AWB / IQ_INFO / INIT / ITP enums have no name in mhal: for them only the Mercury5-aligned guesses exist;
the 88 INIT enums (0x10b–0x162) are fixed register
values in 1316, 1321, 1322, 1328, 1333, 1338 and 1339 whose identity is the register itself. Those guesses were made by aligning
the item list with Mercury5's `isp_dbg_buf.xml` (same library family, 1195 items, another IP set): sections matched by name,
items inside a section by (base type, inner count); the anchor that the alignment is right in AE: the first AE items carry the
Mercury5 example values exactly (`AE_CALI_BVOFFSET` 13102, `AE_CALI_ISO100GAIN` 1782, `AE_CALI_MINISO` 57). For the 620 IP enums
the guesses agree with the official names for only 131 (489 differ; a short run of same-typed items is easily shifted by one,
e.g. the guess put `OBC_EN` on 0x1cf) — the official table supersedes them. The bit meanings below the names (what each
field does in the hardware) are not in any file on hand.

### 12.3 Banks written by the IQ engine [R counts from the Regmap table]
| bank | Regmap fields | IPs (cfg sections) | written outside Regmap (§10.2) |
|---|---|---|---|
| 1311 | 17 | DECOMP | — |
| 1312 / 1313 | 10 / 10 | OBC / OBC_P1 | OBC_b 1312.50–.58 (two-stage black level) |
| 1316 | 56 | INIT, DPC, SDC | 1316.6e / .6f image size |
| 1317 | 23 | ANTICT, SpikeNR | SpikeNR slopes 1317.3a–.3f |
| 1318 | 12 | RGBIR | — |
| 1319 | 11 | LSC | auto centre / shift 1319.01–.03 |
| 131D | 12 | ALSC | 131D.40[2], .43, .44, .4a, .4b (grid geometry) |
| 131E | 67 | HDR | exposure-ratio derived 131E.01 / .05 / .07–.1a / .2e / .40 / .54–.73 |
| 1320 | 5 | GAMA_16to12 | — |
| 1321 / 1322 | 12 / 35 | INIT, WDRLOC | box geometry 1321.00–.03, 1322.06–.0c; exposure ratio 1321.60–.63; 1322.00[9] |
| 1325 | 33 | DM | — |
| 1326 | 10 | BNR | slope 1326.17 / .18 |
| 1327 | 31 | PFC | 1327.00[8], .04–.0f, .30[0] (black level, WB and inverse WB gains) |
| 1328 / 132A / 132B | 35 / 8 / 22 | INIT, NR3D | 1328.04 / .47 / .48 / .70 / .73 / .75 |
| 1329 | — | — | NR3D constants 1329.00–.70 (no Regmap field at all) |
| 1330 | 22 | GNR | 1330.4d |
| 1333 / 1334 / 1335 | 60 / 27 / 25 | INIT, YEE | — |
| 1338 | 18 | INIT, CCM, HSV | — |
| 1339 | 16 | INIT, GAMA_C2A_ISP, R2Y | — |
| 133A | 31 | YEE, SIIR | 133A.5d |
| 133D | 21 | XNR, UVM | — |
| 133E | 42 | ADJUV, MXNR, YUVGAMA, YCUVM10, COLORTRANS | — |

Bank 1329 (NR3D) gets its values only from the constants `IspRegSetNR3D` writes into the shadow (applied by `HalIsp_Apply3DNR`).
**Bank 1331 (GNR, 1331.00–.3d) is copied to the hardware by `HalIsp_ApplyGNR` from shadow+0x3200…+0x32f4, which no IQ code
writes** — no Regmap entry has bank code 0x31 and no IspRegSet stores there [R: every IspRegSet and apply function searched]; so 1331 receives whatever the shadow slot holds [I: zero after allocation]. Likewise 1329.71 is in the apply list but
never written. Bank 131D.45–.47 (the ALSC grid-interpolation kernel) is written only by `ISP_IQ_Init_Cfg` (isp-1-hal.md §10.3),
not by the IQ engine.

## 13. The IQ file (`/config/iqfile/iqfile<ch>.bin`)

### 13.1 Loading [R]
`MhalCameraOpen` → `CameraIspAlgo_LoadIQFile(ch, &cfg)`:
1. `Cfg_InitDbgBuf`: walks the static table **`Dbg_Items`** (mhal `.data`, 1072 × 10 B: `{u16 enum; u8 type; u16 N; u8 M; u32
   offset}`, offset filled at run time) and computes each entry's offset and the total **DbgBufSize = 39 127 B (0x98d7)**.
2. `CameraReadIqData`: `filp_open("<isproot>/iqfile<ch>.bin")` (isproot = `/config/iqfile`, set by mi_isp / the ispmid `isproot`
   sysfs), `vfs_read` of exactly DbgBufSize bytes. On failure (file missing, 0 bytes read) `IspAlgoReadIqBinH` copies the
   built-in table instead (§13.5; log "Load default iq size = …").
3. `cfg_InitBinInfo`: copies DbgBufSize bytes from buffer+8 into the cfg data area and sums its first DbgBufSize/4 u32 words;
   the sum must equal the file's first word, else "error! RetFileSize … checksum …" and the open fails.

The file is not self-describing: its layout is fixed by the kernel build's `Dbg_Items`. A file made for another build (another
item list) fails the checksum or, worse, passes with shifted fields [I].

Quirk [R by arithmetic]: only DbgBufSize bytes are read but the data starts at +8, so the last 8 data bytes are never read from
the file (calloc zeros). They are the payload of enum 0x42d (the COLORTRANS ITP flags) — always zero, i.e. no interpolation for
COLORTRANS, whatever the file says.

### 13.2 Layout [R]
```
+0   u32 checksum      = Σ u32 words of data[0 .. DbgBufSize & ~3)   (imx307: 0xdb643901)
+4   u32 0
+8   data: one entry per enum 0x000 … 0x42f, in enum order, no padding
       entry = { u32 enum; payload }
       type&0x1f  0–5   scalar u8/s8/u16/s16/u32/s32: payload 12 B (value little-endian, rest 0) → entry 16 B
                  8–13  array u8…s32[N] (M = 1), the same for every ISO row
                  15–20 table u8…s32[M rows][N] (M = 16 ISO rows, or 3 colour-temperature rows for LSC/ALSC, 1 for others)
                  22    marker (section START/END): no entry at all
       type&0xe0  0x40 run-time value (Mercury5 "TMPVALUE": AE/AWB state written back for the tuning tool), 0x20 hex display,
                  0x80 command
```
Even type codes are unsigned, odd signed (8 u8, 9 s8, 10 u16, …); codes 6, 7, 14, 21 are not used. `IQ_PARAMETER_TYPE` gives the
element size (§13.3). When an IP loads a scalar or a plain array into its 16 ISO rows, the one value is replicated into every
row; only types 15–20 carry per-row values. The row count per enum is `IQ_Get_cfgcnt` (§13.3).

### 13.3 Access functions [R]
- `Cfg_InitDbgBuf(&h)`: h = calloc(12): {+0 unused, +4 data, +8 size}; size = `Cfg_InitDbgBufInfo()` = walk of `Dbg_Items` (the
  loop runs 0x430 = 1072 times), filling each offset and summing 16 B per scalar (type 0–5), 4 + N·M·{1, 2, 4} per array / table
  (types 8 / 9 / 15 / 16, 10 / 11 / 17 / 18, 12 / 13 / 19 / 20), 0 for any other type (markers 22 and the unused codes); data =
  calloc(size). `Cfg_GetDbgBufSize(h)` = h+8.
- `cfg_InitBinInfo(h, file)`: memcpy(h.data, file+8, size), returns Σ of the first size/4 u32 of the copy (the caller compares it
  with the file's word 0). `cfg_ReleaseDBGInfo(&h)` frees both.
- `cfg_getcfg_info_byenum(h, e, _, out[4])`: out = {N, M, type, offset}. `cfg_getcfgbyenum(h, e, dst)` / `cfg_setcfgbyenum(h, e,
  src)`: copy `IQ_PARAMETER_TYPE(type) · M · N` bytes between dst / src and data+offset+4 (the 4 skips the stored enum word).
  `cfg_getcfgbyenum_ptr(h, e, &p)`: p = data+offset+4. `cfg_getcfgbyenum` is shared with the in-kernel 3A (isp-5-3a.md §2.4, §3.2).
- `IQ_PARAMETER_TYPE(t)`: 2 for t ∈ {2, 3, 10, 11, 17, 18}, 4 for {4, 5, 12, 13, 19, 20}, else 1.
- `IQ_Get_cfgcnt(e)`: 1 for 0x001–0x003 (VERSION), 0x10b–0x162 (INIT), 0x165–0x175 (DECOMP), 0x24b–0x24f (GAMA_16to12),
  0x2d1–0x2d7 (SDC), 0x312–0x314 (R2Y), 0x406–0x40c (COLORTRANS), 0x40f–0x42d (ITP); 3 for 0x1e6–0x1f0 (LSC) and 0x1f3–0x202
  (ALSC); else 16.
- `IQ_Get_cfgdata(h, e, _, dst, stride)`: rows = `IQ_Get_cfgcnt(e)`; scalar types copy the one value (1 / 2 / 4 B) into each of
  the rows at dst + k·stride; array types (8–13) copy the same N elements into each row; table types (15–20) copy row k (N
  elements) into row k. Other types copy nothing.
- `IQ_Set_cfgdata(h, e, src, tmp, stride)` (unreferenced): the inverse into a temporary, then `cfg_setcfgbyenum`; for scalar /
  array types it copies once, for tables 16 rows — with a defect: u16 tables are packed at tmp + N·k·**4** and u32 tables at
  tmp + N·k·**16** (the pointer is a `u32 *`), so only u8 tables come out right [R disassembly 0x6d966: `add r0, r5, r0, lsl #2`].
- `IQ_CFG_Print`: stub.

### 13.4 Contents [R values; section roles R, meaning I]
The file holds 1072 enums, each with type, N, M, offset in the data and size; for each we know the IP that loads it, the
parameter's offset in that IP's row, the register and the imx307 value(s) (that per-enum table is [`isp-iq-regmap.tsv`](../data/isp-iq-regmap.tsv), §12.1). Totals: 80 markers, 992 entries; 737 are loaded by an IP, 671 have a register.

| enums | section | what |
|---|---|---|
| 0x000–0x004 | VERSION | vendor 0, major 1, minor 0 |
| 0x005–0x0d4 | AE | the in-kernel AE's tuning (calibration, targets, metering / strategy tables, exposure tables, window-weight tables at 0x031 (u8, 96 rows × 32), flicker, convergence …) and its run-time values (0x40 flag) — read by `AE_GetParamFromCfg` at `AeInit` (isp-5-3a.md §2.4); not used on this camera (user AE) |
| 0x0d5–0x0fb | AWB | in-kernel AWB tuning (light areas, CT tables, gains) — `AwbInit`; not used on this camera (user AWB) |
| 0x0fc–0x0fd | AF | empty section |
| 0x0fe–0x100 | EIS | one item |
| 0x101–0x108 | IQ_INFO | chip version 34931, IR-ISO remap table 16 × 16 (identity), **ISO gain tables 0 / 1** (16 × u32: 1024·2^k = gain 1×…32768×, the thresholds `IQ_ISOIndexGet` compares the gain with; table 1 is used in IQ mode 1 = night, id 0x1022), block tables 0 / 1 (not read, §3.3), ISO index overrides |
| 0x10b–0x162 | INIT | 88 one-off register values written by `IspRegSetINIT`: 1316 ×10, 1321 ×8, 1322 ×6, 1328 ×7, 1333 ×36, 1338 ×11, 1339 ×10 (fixed parts of DPC, WDR, 3DNR, YEE, CCM / HSV, gamma / R2Y) |
| 0x165–0x40c | IP sections | the parameters of IP slots 0–41, ranges as in §5.3 |
| 0x40f–0x42d | ITP | per IP: byte 0 = interpolate on / off, then one flag per parameter (0x40f DECOMP … 0x42d COLORTRANS) |

**The file on this camera** [R files; consequence I]: `/config/iqfile/imx307_iqfile.bin` (39 135 B) with `/config/iqfile/iqfile0…3.bin` as
symlinks to it — the generic imx307 tuning, the same file (checksum 0xdb643901) in every M6 firmware examined
(Runcam Split-HD, Siyi A8, Anjoy MCH12S and this camera). No GC8613 IQ file exists on the camera, so every IP parameter not
overridden by the app's API bin (`/customer/iq_file_gc8613.bin`, isp-3-iq-api.md §6) is imx307 tuning. The imx307 file has almost
no per-ISO variation: most entries are scalars replicated into the 16 rows. The user 3A loads the same file (isp-6-userspace.md §2.8).
The directory also holds `imx307_api.bin` (an API bin; nothing on this camera loads it — the app loads its own bin; inventory
row: firmware-image.md §3.6) and
`isp_api.xml` (165 KB, the IQ tuning tool's description of the API structs; isp-6-userspace.md §8).

### 13.5 Built-in IQ table (`iq_bin`, `.data`, 39 175 B) [R]
`{char uuid[36] = "50e7b4dc-636e-4095-b3f5-d6d48438548e"; u32 51316}` followed by a byte-exact copy of `imx307_iqfile.bin`
(39 135 B). `IspAlgoReadIqBinH` copies from `iq_bin+0x28`, i.e. the file image, so the fallback is the same imx307 tuning. The
40-byte prefix is not used by the code; 51316 equals the size of the SSD21x `gc1054_iqfile.bin` in the qiming tree [I: a size
field left by the generator].

## 14. Math helpers [R]
Shared with the in-kernel 3A: `Interp`, `Interp_LUT`, `cfg_getcfgbyenum` (§13.3).

| function | users | what it computes |
|---|---|---|
| `IQ_para_inter(dbg, flag, x, x0, x1, y0, y1)` | every IP blend (§5.2), CCM / HSV / TEMP | if flag = 1, y0 ≠ y1 and x0 ≤ x ≤ x1: k = number of whole steps (x1 − x0) >> 4 in x − x0 (0…16); y = y0 (k = 0), y1 (k = 16), y0 + (Δ+1)>>1 (k = 8); for k < 8: y0 + f(k), k > 8: y1 − f(16 − k), where Δ = y1 − y0 and f(1) = (Δ+8)>>4, f(2) = (Δ+4)>>3, f(3) = f(1)+f(2), f(4) = (Δ+2)>>2, f(4+j) = f(4) + f(j) — i.e. Δ·k/16 from rounded shifts, not exact linear. If no k is found (step = 0, i.e. x1 − x0 < 16, or a remainder ≥ step after 16 steps) the result is **0** — a defect for close thresholds. flag ≠ 1, y0 = y1 or x > x1 → y1; x < x0 → y0. dbg = 1 prints the three x and y values |
| `Interp(x0, x1, y0, y1, x)` | `Interp_LUT*`, AE | linear with clamping: x outside [min(x0, x1), max] → the y of that end; x0 = x1 → y0; else y0 + ((x − x0)(y1 − y0)·1024 / (x1 − x0)) >> 10 in 64-bit signed |
| `InterpU64(x0, x1, y0, y1, x)` (all u64) | AE | the same in unsigned 64 bit, the sign of y1 − y0 handled separately |
| `Interp_LUT(n, y[16], x[16], x)`, `Interp_LUT_IQ` (byte-identical copy) | AE / LSC, ALSC, WDR_LOC converter | piecewise-linear over n points passed by value (y array in r1–r3 + stack, x array after it): x increasing or decreasing (decided by x[0] vs x[n−1]); outside → y[0] / y[n−1]; else `Interp` on the segment |
| `Interp_LUT_IQ_P(n, u16 *y, u16 *x, x)` | unreferenced | the same over u16 arrays by pointer |
| `Interp_IQ` | unreferenced | stub |
| `IQ_InterpXBase(x, x0, x1, y0, y1, scale, round)` | unreferenced | returns (scale·y0 + round + scale·(x − x0)/(x1 − x0) · (y1 − y0)) / scale (64-bit) |
| `_exp(x, n)` | gamma S-curves, ADJUV converter | integer xⁿ (n = 0 → 1) |
| `roundRSl(s64 v, n)` | COLORTRANS converter | v >> n rounded half away from zero |
| `TwoComp2Int(v, r)` | COLORTRANS converter | r < 0x200: bit 9 set → v \| ~0x1ff; 0x200 ≤ r < 0x400: bit 10 set → v \| ~0x3ff; r ≥ 0x400 → 0; else v |
| `Int2TwoComp(v, r)` | unreferenced | v < 0: (v & 0x1ff) + 0x300 (r < 0x200) or (v & 0x3ff) + 0x400 (r < 0x400); v ≥ 0 → **0** (r < 0x400) or v [R disassembly; a defect] |
| `interto_twos_complement(v)` | COLORTRANS converter | v < 0 → (v & 0xfff) + 0x1000 (13-bit form) |
| `twos_complement_to_inter(v)` | unreferenced | bit 12 set → v \| ~0xfff |
| `compare(a, b)` | AE (`pAE_CalLumY`, as a `sort` comparator) | −1 / 0 / 1 on two ints |
| `Calc_STDEV_Power`, `Get_Color_Ratio`, `Get_Uniform_Rows_average`, `Check_Shading_Init`, `Check_Shading_Performance` | `IQ_Patch_ALSC` | §9.4 |
| `Callocate2dArray(rows, cols, esize)` / `Decallocate2dArray(p)` | LSC / ALSC calibration | one calloc of rows·cols·esize + rows pointers, row pointers filled; free |
| `MinMaxNormalization(x, x0, x1, y0, y1)` | unreferenced | y0 + (y1 − y0)(x − x0)/(x1 − x0) |

## 15. Defects and dead parts [R unless tagged]
Each is specified where it lives; this is the index.
- Defects: `IQ_Patch_YEE` scales the API table diagonally and cumulatively (§9.1); `IQ_para_inter` returns 0 for thresholds less
  than 16 apart (§14); `WriteRegs` writes a stale value for u32 elements under a 16-bit mask (§10.1); `IQ_Set_cfgdata` and
  `Int2TwoComp` are broken but unreferenced (§13.3, §14); static, not per-channel, state in `hal_DoIQPerFrame` (§3.5), in the
  AE-accuracy patch (§9.2) and in the HDR luma level (§10.2); LSC calibration copies 32 entries whatever the table size (§8);
  SDC calibration always reads 256 records (§8); `dbgiq` never parses its value (§2.3); `Check_Shading_Performance` sums Y in a
  u16 and splits the sign of pass 0 at L = 16 (§9.4) [the second I]; `pAWB_Ccm_Inter` writes 16-bit two's complement into a
  matrix the IQ engine decodes as 13-bit (§2.2).
- Dead or unreachable: IP modes 2 and 4 and the mode-4 branches (§4.3); `IQ_Debug_FixISO` (§4.4); the NE-block override words
  0xdb01 / 0xdb02 (§9.2); ctx+0x53761 (read by `hal_DoIQ`, written by nothing, §3.4); the second NR3D state set (§3.3); the
  `BLK_TBL0/1` enums (§3.3); the COLORTRANS ITP flags (§13.1); bank 1331 and 1329.71 (applied, never written, §12.3); the stub
  converters, patches and register writers (§6.1, §9, §10.2, §10.4); `IQ_CCM_smooth` (§4.2); `IQ_Property_Set/Get` (§4.4);
  `IqInit`'s 1-byte buffer (§3.1).

## 16. What is not known
- The values on the running camera [I expectations]: 1321.7E (expect 0, §11.1); 1331.00–.3d (expect 0, §12.3); the two-stage
  black level 1312.01–.08 / .50–.58 (§10.2); the WDR box geometry 1322.06–.0c for 3840 × 2160 (§10.2); whether the banks of YEE
  (1333 / 1334 / 1335 / 133A), NR3D (1328), DPC (1316), WDR (1322) and GNR (1330) hold the API bin's Sharpness / Sharpness_EX /
  NR3D / DynamicDP / WDR / NRLuma_Adv manual sets through the maps of §10 after boot, or something (prog_uvc menu, Dummy_EX)
  overwrote them; whether 1316.6a–.6f and the SDC menuload table hold a static defect-pixel calibration (0x2414) or the imx307
  default list; Get 0x1039 TemperatureInfo (expect 36, §2.2); whether 1328.04[0] clears itself (§10.2) (CAMERA-CHECKS K18–K20,
  K22).
- The register-level meaning of the fields below their official names (§12.2): no register reference is on hand.
- Names of the 88 INIT enums and of the AE / AWB / IQ_INFO / ITP enums: not present in mhal (§12.2).
- The format of the COLORTRANS offsets (11-bit two's complement in 10-bit units) is an inference from three read facts (§7.5);
  it has not been checked on the camera (a U offset 0x7f0 means −16 if signed, 2032 if not).
- How the in-kernel AWB derives the MultiLS target ratios (`pAWB_MultiLS_TargetRatioCalc`): isp-5-3a.md §3.8.
