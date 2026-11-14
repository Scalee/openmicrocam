# panel-mipitx — mi_panel, mi_mipitx, libmi_panel, libmi_mipitx, mhal PNL and MIPITX, board.ini / Customer.ini
<!-- ev:panel-mipitx -->

The display-panel and MIPI-transmitter stack of the SigmaStar media software on this camera, from the user libraries down
to the registers, and the two "system ini" files that belong to the panel scheme. Loaded, never used by the product (§1):

- **libmi_panel.so / mi_panel.ko** (`/dev/mi_panel`, MI module id 0x19) — LCD / MIPI-DSI panel control: one instance per
  link type, created from the config_tool panel record of that type and handed to mhal PNL as one "unified" parameter
  block; runtime setters for power, backlight, SSC, timing, DSI configuration, drive current and test pattern (§2, §3).
- **mhal PNL** (block of `mhal.ko`) — the panel HAL: two contexts on the DISP timing generators OP2 (bank 1151) and
  OP2_1 (1161), LCD timing and MCU interface (1154), LPLL (1033), pad mux, and the MIPI-DSI controller (1A38 / 1A39) with
  its D-PHY (1535). Same chip as the public OpenIPC u-boot mercury6 panel source, with the differences of §4.5 (§4).
- **libmi_mipitx.so / mi_mipitx.ko** (`/dev/mi_mipitx`, MI module id 0x1e) — one MIPI CSI-2 transmit channel, an
  mi_sys sink with one input port: each frame bound into it is sent out as one CSI-2 frame (§5, §6).
- **mhal MIPITX** (block of `mhal.ko`) — the CSI-2 transmitter: MAC / packet engine (1536), the same D-PHY bank 1535 and
  the same LPLL as the panel's DSI path, clock 1038.5C. No public source exists (§7).
- **`/config/board.ini`, `/config/model/Customer.ini`** — MStar "system ini" files with no reader on this camera (§9).

Owned elsewhere: the MI ioctl wrapper, `'i'` numbering, payload packing, return codes and module ids — `mi-user-abi.md`;
`MI_DEVICE` dispatch, per-module procfs and debug parameters — `mi_sys-2-interfaces.md` §1; the mi_sys registration
contract (info record, pass slots), bind capability and frame flow — `mi_sys-1-engine.md` §2–§4; the 0x170-byte panel
record of `/proc/mi_modules/common/*` that config_tool writes and `MI_SYSCFG_GetPanelInfo` returns, and the `config.ini`
panel sections — `config_tool.md` §5.2, §4.1; mi_disp's own use of mhal PNL for BT656 / BT1120 outputs and the record → unified
conversion mi_disp shares with mi_panel — `mi_disp-mi_hdmi.md` §4.6.3; the DISP banks 1150 / 1151 / 1154 / 1161 as the display
path uses them and the display timing tables — `mhal-disp.md` §5.1, §7; mhal's block table, `camdriver_init` order, the
character-device quirk and the DT properties per node — `mhal-core.md` §2, §3.2, §4, §7; the DT clock tree
(`CLK_mipi_tx_csi`) and RIU addressing — `kernel-platform.md` §3.1, §5.1; the loading of both modules by `demo.sh` —
`rootfs-customer.md` §3.2; the partition and file inventory — `firmware-image.md` §1, §3.

Tags: **[R]** read from the binary, a decompile, disassembly or a file; **[S]** stated by a source (the SDK headers, the
mercury6 panel source tree and its register headers); **[I]** inference. A heading tagged [R] means every statement under
it is [R] unless marked otherwise. MI panel struct and field names are those of the public `mi_panel.h` /
`mi_panel_datatype.h` version 2.12 (a newer user API used for names only) [S]; there is no public header for mi_mipitx, libmi_mipitx or mhal MIPITX, so their
export and function names are the binaries' symbols [R], field names follow the kernel's log strings [R] or are chosen
in SigmaStar style [I] where said. Function names of mhal PNL and MIPITX are the binary's symbols [R]; PNL register names
are those of the mercury6 `hal_pnl_reg.h` / `hal_pnl_dsi_reg.h` [S]. Register addresses are written `BANK.RR` (16-bit
register index in the RIU bank, `kernel-platform.md` §3.1); `BANK.RR[h:l]` is a bit field.

Build strings: libmi_panel "Sigmastar Module mi_panel version: project_commit.7251436 sdk_commit.80327e1
build_time.20220721223320", mi_panel.ko the same with `build_time.20220721223317` (the library is two seconds younger);
libmi_mipitx `… mi_mipitx version: project_commit.7251436 sdk_commit.80327e1 build_time.20220721223312`, mi_mipitx.ko `…
build_time.20220721223310`.

Binaries [R]: `/config/lib/libmi_panel.so`, `/config/lib/libmi_mipitx.so` (9.5 KB each, stripped),
`/config/modules/4.9.227/mi_panel.ko` (48 KB file, local symbols kept), `/config/modules/4.9.227/mi_mipitx.ko` (30 KB), the
PNL (263 functions) and MIPITX (101) blocks of `/config/modules/4.9.227/mhal.ko` (block map: `mhal-core.md`),
`/config/board.ini`, `/config/model/Customer.ini`.

## 1. The stack on this camera [R]
<!-- ev:panel-mipitx.1 -->

- **Loaded, not used.** `/customer/demo.sh` loads both modules without parameters (`insmod mi_mipitx.ko` after
  mi_sensor, `insmod mi_panel.ko` before mi_disp; `rootfs-customer.md` §3.2). Loading mi_panel only registers `/dev/mi_panel`
  (§3.1); loading mi_mipitx only registers `/dev/mi_mipitx` — its mi_sys device and interrupt handler come with the first
  open of the device (§6.1). No process opens either device: no binary links `libmi_panel.so` or `libmi_mipitx.so`, and
  prog_uvc and kjd_lvgl contain no `MI_PANEL_*` / `MI_MipiTx_*` string (`prog_uvc.md`, `kjd_lvgl.md`).
- **Other paths into mhal PNL**: only mi_disp's BT656 / BT1120 output set-up (`MhalPnlCreateInstanceEx`,
  `MhalPnlSetUnifiedParamConfig`, `MhalPnlDestroyInstance`; `mi_disp-mi_hdmi.md` §4.6.3), which the HDMI-only product never
  takes. **mhal MIPITX** is reached only from mi_mipitx; the "SCL → MIPITX" cross-block calls go to five helpers that sit
  in the MIPITX address range — `DrvMipiTxCsiOsMemAlloc`, `DrvMipiTxCsiOsMemFree`, `DrvMipiTxCsiSplitString`,
  `DrvMipiTxCsiStrToL`, `MhalMipiTxCsiSetDebugLevel` — not to the transmitter. The
  product's pipeline has no MIPI-TX output.
- **What runs**: mhal's module-init part of both blocks. At mhal load `camdriver_init` creates the devices `mpnl` and
  `mmipitx` with their sysfs attributes and looks up their DT nodes; no register is written (§4.2, §7.1).
- **`board.ini` and `model/Customer.ini`** have no reader in any binary of the dump (§9).

Every part below is therefore unused on this camera; the status of all of it is UNUSED.

## 2. libmi_panel.so — user library [R]
<!-- ev:panel-mipitx.2 -->

### 2.1 Frame
19 exports: the 18 `MI_PANEL_*` of the public `mi_panel.h` 2.12 and `sstar_mi_so_version_string`. Imports
`CamOsTsemDown`, `CamOsTsemUp`, `g_mi_fd_semalock`, `g_last_fd_index`, `g_mi_fd_table` from libmi_sys. Device
`/dev/mi_panel`, opened lazily by the common open helper (per-pid fd, `F_DUPFD` above the last MI fd, `FD_CLOEXEC`;
`mi-user-abi.md` §2.1). Every call sends the 16-byte wrapper `{u32 len; u32 pad; u64 ptr}` (`mi-user-abi.md` §3.1); the
payload is `{u32 eIntfType; <struct>}` packed on the library's stack, ioctl type `'i'` (0x69). An ioctl result of −1
prints `[%s] failed to ioctl 0x%08lx!(%s)` and returns −1; any other result is returned as is. No library-side
validation.

### 2.2 Exports → ioctls
| export | cmd | payload (after `u32 intf` at +0) | copy-back |
|---|---|---|---|
| `MI_PANEL_Init(intf)` | 0x40046900 `_IOW 0` len 4 | — (payload is just intf) | — |
| `MI_PANEL_DeInit()` | 0x6901 `_IO 1` | none; **third ioctl argument NULL** (no wrapper) | — |
| `MI_PANEL_GetTotalNum(u32*)` | 0xc0046902 `_IOWR 2` len 4 | the payload is only the output u32 (no intf) | u32 if result 0 — never (the kernel always returns 0x1F, §3.3) |
| `MI_PANEL_SetPowerOn(intf, PowerConfig*)` | 0x40086903 len 8 | +4 u8 `bEn`, +5..7 zero | — |
| `MI_PANEL_GetPowerOn(intf, PowerConfig*)` | 0xc0086904 len 8 | +4 u32 out (zeroed) | low byte → `bEn` if 0 |
| `MI_PANEL_SetBackLight(intf, BackLightConfig*)` | **0x6901 len 0** | a 12-byte payload `{intf, cfg[0..3], cfg u16 @4}` is built but the command is DeInit's: **the call tears down every panel instance** (§3.3 DeInit) and never reaches kernel handler 5 | — |
| `MI_PANEL_GetBackLight(intf, BackLightConfig*)` | 0xc00c6906 len 0xc | +4 8 bytes zeroed | u32 @+4 and u16 @+8 → cfg+0, cfg+4 (6 bytes) if 0 |
| `MI_PANEL_SetBackLightLevel(intf, BackLightConfig*)` | 0x400c6907 len 0xc | +4 cfg bytes 0..5, +0xa..0xb zero | — |
| `MI_PANEL_GetBackLightLevel(intf, BackLightConfig*)` | 0xc00c6908 len 0xc | zeroed | 6 bytes back if 0 |
| `MI_PANEL_SetSscConfig(intf, SscConfig*)` | 0x400c6909 len 0xc | +4 cfg bytes 0..5 | — |
| `MI_PANEL_SetMipiDsiConfig(intf, MipiDsiConfig*)` | 0x4050690a len 0x50 | +4 the 0x4c-byte struct (memcpy) | — |
| `MI_PANEL_SetTimingConfig(intf, TimingConfig*)` | 0x4020690b len 0x20 | +4 the 0x1a-byte struct, +0x1e zero | — |
| `MI_PANEL_SetDrvCurrentConfig(intf, DrvCurrentConfig*)` | 0x4008690c len 8 | +4 u16, +6 zero | — |
| `MI_PANEL_SetOutputPattern(intf, TestPatternConfig*)` | 0x400c690d len 0xc | +4 cfg bytes 0..7 | — |
| `MI_PANEL_SetPanelParam(intf, ParamConfig*)` | 0x4094690e len 0x94 | +4 the 0x90-byte struct (memcpy) | — |
| `MI_PANEL_GetPanelParam(intf, ParamConfig*)` | 0xc0946912 len 0x94 | +4 0x90 zero bytes | 0x90 bytes if 0 |
| `MI_PANEL_InitDev(InitParam*)` | open; then 0x40046900 with **the caller's pointer as payload** (reads `eIntfType` at +0) | — | returns **0x1F** if the open failed |
| `MI_PANEL_DeInitDev()` | 0x6901 (failure only printed), then the common close helper (`mi-user-abi.md` §2.2 scheme, no counter) | — | |

Commands 0x0f–0x11 are not produced by the library and are NULL in the kernel table (§3.2). No export asks for
`GetTimingConfig`, although the kernel implements it (§3.3).

### 2.3 Data types of this build [R sizes from the ioctl lengths and the kernel's field use; S names from the 2.12 header]
- `MI_PANEL_IntfType_e` u32: 0 TTL, 1 LVDS, 2 RSDS, 3 MINILVDS, 4 ANALOG_MINILVDS, 5 DIGITAL_MINILVDS, 6 MFC, 7 DAC_I,
  8 DAC_P, 9 PDPLVDS, 10 EXT, 11 MIPI_DSI, 12 BT656, 13 BT601, 14 BT1120, 15 MCU_TYPE, 16 SRGB, 17 TTL_SPI_IF (same values
  as mhal `MhalPnlLinkType_e`; the kernel's proc dump prints these names, §3.4).
- `PowerConfig_t` {u8 bEn} (1). `BackLightConfig_t` {u8 bEn; +2 u16 u16Duty; +4 u16 u16Period} (6).
  `SscConfig_t` {u8 bEn; +2 u16 u16Freq (kHz); +4 u16 u16Ratio (1/1000)} (6). `DrvCurrentConfig_t` {u16} (2).
  `TestPatternConfig_t` {u8 bEn; +2 u16 R; +4 G; +6 B} (8).
- `TimingConfig_t` (0x1a): u16 HSyncWidth, HSyncBackPorch, HSyncFrontPorch, VSyncWidth, VSyncBackPorch,
  VSyncFrontPorch, HStart, VStart, **Height (+0x10), Width (+0x12)**, HTotal, VTotal, Dclk.
- `MipiDsiConfig_t` (0x4c): u8 ×14 D-PHY timings (HsTrail, HsPrpr, HsZero, ClkHsPrpr, ClkHsExit, ClkTrail, ClkZero,
  ClkHsPost, DaHsExit, ContDet, Lpx, TaGet, TaSure, TaGo) at 0x00–0x0d; u16 Hactive 0x0e, Hpw, Hbp, Hfp, Vactive, Vpw,
  Vbp, Vfp, Bllp, Fps (0x20); u32 enLaneNum 0x24, enFormat 0x28, enCtrl 0x2c; ptr pu8CmdBuf 0x30 (user address);
  u32 u32CmdBufSize 0x34; u8 SyncCalibrate 0x38; u16 VirHsyncSt 0x3a, VirHsyncEnd 0x3c, VsyncRef 0x3e, DataClkSkew
  0x40; u8 PolCh0–4 0x42–0x46; u32 ePacketType 0x48 (0 DCS, 1 GENERIC).
- `ParamConfig_t` (0x90) = the 2.12 header's layout without padding surprises: ptr pPanelName 0x00, u8 u8Dither 0x04,
  u32 eIntfType 0x08, u8 DualPort…LVDS_TI_MODE 0x0c–0x15, DCLKDelay 0x16, InvDCLK 0x17, InvDE 0x18, InvHSync 0x19,
  InvVSync 0x1a, DCLK/DE/Odd/Even current 0x1b–0x1e, u16 OnTiming1/2 OffTiming1/2 0x20–0x26, HSyncWidth 0x28,
  HSyncBackPorch 0x2a, VSyncWidth 0x2c, VSyncBackPorch 0x2e, HStart 0x30, VStart 0x32, Width 0x34, Height 0x36,
  Max/H/Min HTotal 0x38–0x3c, Max/V/Min VTotal 0x3e–0x42, Max/DCLK/Min DCLK 0x44–0x48 (u16), SpreadSpectrumStep 0x4a,
  Span 0x4c, u8 PwmPeriodL/H/Duty/DeinterMode 0x4e–0x51, u32 ePanelAspectRatio 0x54, u16 LVDSTxSwapValue 0x58,
  u32 eTiBitMode 0x5c, eOutputFormatBitMode 0x60, u8 SwapOdd_RG/Even_RG/Odd_GB/Even_GB 0x64–0x67, u8 DoubleClk 0x68,
  u32 MaxSET 0x6c, MinSET 0x70, eOutTimingMode 0x74, u8 NoiseDith 0x78, u32 eCh0–eCh4 0x7c–0x8c.

## 3. mi_panel.ko — kernel module [R]
<!-- ev:panel-mipitx.3 -->

### 3.1 Load, device, imports
`init_module`: `proc_mkdir("mi_panel", /proc/mi_modules)` with the four standard files `debug_level`, `debug_file`,
`debug_func`, `module_version_file` (the build string of the header), prints `module [panel] init`,
`MI_DEVICE_Register(&dev)` → `/dev/mi_panel`. Module parameters (modinfo): `debug_level` (uint), `debug_file`,
`debug_func` (strings) — the same three values as the proc files (the MI_DEVICE debug filter, `mi_sys-2-interfaces.md`
§1.6); demo.sh passes none. Nothing else happens at load: no mhal call, no hardware access. `cleanup_module` removes the
proc files and unregisters the device.

Device callbacks: `mi_panel_process_init` only prints `client [%d] connected, module:panel`;
**`mi_panel_process_exit` (any process closing its `/dev/mi_panel` fd, including process death) calls
`MI_PANEL_IMPL_DeInit()`**, i.e. destroys every panel instance of every process, then prints `client [%d] disconnected`.

Exported symbols: only the generic `MI_DEVICE_*` helpers. mhal imports: 17 `MhalPnl*` — every PNL export except the four
of §4.1 that nobody imports; one mi_sys import of substance, `MI_SYSCFG_GetPanelInfo`.

### 3.2 ioctl handlers [R]
<!-- ev:panel-mipitx.3.2 -->
`MI_DEVICE_WrapperFunction(nr)` returns `table[nr]` for nr < 0x13, else NULL. Every handler locks the single module
mutex, stores it in `ctx+8` (mi_common unlocks it after the call, `mi_sys-2-interfaces.md` §1.3), splits the payload into
`intf = *(u32*)p` and `p+4`, and calls the IMPL function; the IMPL result is the ioctl result.

| nr | handler → IMPL | nr | handler → IMPL |
|---|---|---|---|
| 0x00 | Init(intf) | 0x0a | SetMipiDsiConfig |
| 0x01 | DeInit() | 0x0b | SetTimingConifg [sic] → SetTimingConfig |
| 0x02 | GetTotalNum (payload pointer only) | 0x0c | SetDrvCurrentConfig |
| 0x03 / 0x04 | SetPowerOn / GetPowerOn | 0x0d | SetOutputPattern |
| 0x05 / 0x06 | SetBackLight / GetBackLight | 0x0e | SetParamConfig → SetPanelParam |
| 0x07 / 0x08 | SetBackLightLevel / GetBackLightLevel | 0x0f–0x11 | NULL (unused numbers) |
| 0x09 | SetSscConfig | 0x12 | GetPanelParam |

`MI_PANEL_IMPL_GetTimingConfig` exists and is a global symbol (`T`), but no table entry points to it — dead code.

### 3.3 Objects and IMPL semantics
**Instance table**: 100 slots of `MI_PANEL_Instance_t *` in `.bss` (names [I]). An instance is 0x1f0 bytes
(`CamOsMemAlloc` + memset 0) and is keyed by the **MI interface type** stored at +8; at most one instance per type.

| off | size | content |
|---|---|---|
| 0x000 | ptr | mhal PNL context (`MhalPnlCreateInstanceEx` fills it) |
| 0x004 | u8 | set to 1 after a successful SetMipiDsiConfig (never read) |
| 0x005 | u8 | set to 1 after a successful SetTimingConfig (never read) |
| 0x008 | u32 | MI interface type as passed to Init (unclamped) |
| 0x00c | u8 | last power state set |
| 0x00e | u8 | last backlight on/off set |
| 0x010, 0x012 | u16 ×2 | last backlight duty, period |
| 0x014 | u8 + u16 ×2 | SSC cache: enable, +0x16 freq/step, +0x18 ratio/span |
| 0x01c | 0x4c | copy of the last `MipiDsiConfig_t` (with the caller's user pointer) |
| 0x068 | 0x1a | copy of the last `TimingConfig_t` |
| 0x082 | u16 | last drive current |
| 0x084 | 8 | last `TestPatternConfig_t` |
| 0x08c | 0x90 | copy of the last `ParamConfig_t` |
| 0x11c | 0xd4 | copy of the `MhalPnlUnifiedParamConfig_t` built at Init (§4.8; read by GetPanelParam and the proc dump) |

Every IMPL function except Init, DeInit and GetTotalNum finds the instance by `intf == inst+8`; **no instance → returns 0
(success) and does nothing**. mhal entry points return `bool` (1 = success); a 0 maps to MI result **0x1F** (generic
failure; mi_panel has no other error code). The cache fields above are written only after mhal returned 1.

- **Init(intf)**. Accepted interfaces: 0 TTL, 1 LVDS, 11 MIPI_DSI, 12 BT656, 14 BT1120, 15 MCU, 16 SRGB, 17 TTL_SPI;
  any other value is logged (an `invalid intf type %d`-style message, `_LC215`) and **mapped to 0 (TTL)** for the mhal
  link type and the config lookup, while the instance is still keyed by the original value. **BT656 and BT1120
  (`(intf & ~2) == 12`) return 0 at once without creating anything** — mi_disp sets those two up itself
  (`mi_disp-mi_hdmi.md` §4.6.3), so the two modules split the link types between them. An existing instance for `intf` →
  return 0. Free slot: the first NULL of the 100; **if all 100 are taken the code goes on with a NULL instance pointer**
  (no check; it would crash in mhal) — unreachable, since at most 8 types are accepted.
  Steps: `rec = MI_SYSCFG_GetPanelInfo(mapped_intf, 0x38)` = the first config_tool record of that interface (timing id
  0x38 = "any"; `config_tool.md` §5.3); NULL → log, free, 0x1F. `MhalPnlCreateInstanceEx(&inst->ctx, mapped_intf,
  rec->u16DispPath)` — **the record's first field `m_wPanelDispPath` is read here and only here: it is the `u16 u16Id`
  argument, the mhal PNL context id** (0 → timing generator bank 1151, 1 → 1161, > 1 fails; §4.3); failure → free,
  0x1F. Then the instance is stored and the record is converted to `MhalPnlUnifiedParamConfig_t` by the inlined
  `_MI_PANEL_ConvertToHalPnlInfo` — field for field the conversion mi_disp uses (`mi_disp-mi_hdmi.md` §4.6.3), with the same
  clamps: link type outside the accepted list → 0; RGB swap R∉{1,2} → 3, G∉{1,3} → 2, B∉{1,2,3} → 1; output bit mode
  > 3 → 2; DSI lanes outside 1–4 → 1, format > 3 → 3, ctrl > 3 → 1, packet type ∉{0,1} → 0; MCU type ∉{0,1} → 0, MCU
  bus cfg > 0x12 → 0; the name pointer = `&rec->m_pPanelName`; the DSI and MCU command pointers = the kernel copies that
  `MI_SYSCFG` made (`config_tool.md` §5.4). The block is handed to `MhalPnlSetUnifiedParamConfig(ctx, &u)` (**result
  ignored**), copied to inst+0x11c, and the proc device is registered (§3.4). Returns 0.
- **DeInit()**: unregisters the proc device (if registered); for each instance `MhalPnlDestroyInstance(ctx)` — a
  failure returns 0x1F at once, leaving the remaining instances alive; frees and clears the slot. Returns 0.
- **GetTotalNum**: returns 0x1F always (not implemented).
- **SetPowerOn(intf, {bEn})** → `MhalPnlSetPowerConfig(ctx, &(bool)(bEn != 0))`; **GetPowerOn** →
  `MhalPnlGetPowerConfig` → writes the byte; failure prints `mi panel get power status failed`.
- **SetBackLight / GetBackLight** → `MhalPnlSetBackLightOnOffConfig` / `MhalPnlGetBackLightOnOffConfig` with the bool
  only; Get writes only `bEn`.
- **SetBackLightLevel(intf, cfg)** → `MhalPnlSetBackLightLevelConfig(ctx, {u16 Duty = cfg+2, u16 Period = cfg+4})`;
  **GetBackLightLevel** writes Duty / Period back at cfg+2 / +4 (`bEn` untouched).
- **SetSscConfig(intf, {bEn, Freq, Ratio})** → `MhalPnlSetSscConfig(ctx, {bool bEn; u16 = Freq; u16 = Ratio})` — a
  6-byte local; the mhal struct of the public source is 10 bytes (+ `u16Modulation`, `u16Deviation`), whose last two
  fields are **uninitialised stack**; the Set code that would read them is unreachable, because the SSC query fails its
  "param first" check (§4.3).
- **SetMipiDsiConfig(intf, cfg)**: copies the 0x4c bytes into a local `MhalPnlMipiDsiConfig_t` (same layout), clamps
  `enLaneNum` ∉{0,1,2,3} → **4** (so 0 = "none" passes), `enFormat` ∉{0,1,2} → 3, `enCtrl` ∉{0,1,2} → 3,
  `ePacketType` → (== 1); allocates `u32CmdBufSize` bytes, `CamOsCopyFromUpperLayer` from the user `pu8CmdBuf`, calls
  `MhalPnlSetMipiDsiConfig`, **frees the copy right after the call** (mhal consumes the command buffer synchronously,
  §4.7), then caches the user struct. Allocation failure → returns 0 (success) without calling mhal.
- **SetTimingConfig(intf, cfg)** → `MhalPnlSetTimingConfig` with the fields in mhal order — **the MI struct has Height
  before Width, mhal has HActive before VActive; the copy swaps them correctly** (MI +0x12 → mhal +0x10, MI +0x10 → mhal
  +0x12). `GetTimingConfig` (dead) copies mhal's 0x1a bytes back **without** the swap.
- **SetDrvCurrentConfig(intf, {u16})** → `MhalPnlSetDrvCurrentConfig`.
- **SetOutputPattern(intf, {bEn, R, G, B})** → `MhalPnlSetTestPatternConfig(ctx, {bool bEn != 0, R, G, B})`.
- **SetPanelParam(intf, cfg)** → builds `MhalPnlParamConfig_t` (the 32-bit source layout: `pPanelName` +0,
  `u16 u32AlignmentDummy0` +4, `u8Dither` **+6**, `eLinkType` +8, then the MI fields in order at the same offsets from
  +0xc on; size 0x94 = MI 0x90 + `enPacketType` at +0x90, **which this function never writes** — uninitialised stack)
  with clamps: link type → accepted list, else 0; aspect ratio ∉{0,1} → 2; `eTiBitMode` ∉{0,2} → 3; output bit mode
  ∉{0,1,3} → 2; `eOutTimingMode` ∉{0,1} → 2. Calls `MhalPnlSetParamConfig`; on success caches the 0x90 bytes and, if
  `u16SpreadSpectrumStep` or `Span` is non-zero, sets the SSC cache (en = 1, step, span). Returns 0 also when no instance
  exists.
- **GetPanelParam(intf, out)**: no instance → 0x1F (the only getter that fails on a missing instance). Otherwise fills
  only `pPanelName` and the timing fields from the Init-time unified copy: HSyncWidth, HSyncBackPorch, VSyncWidth,
  VSyncBackPorch, HStart, VStart, Width (= HActive), Height (= VActive), HTotal (+0x3a), VTotal (+0x40) and `u16DCLK`
  (+0x46) = the **low 16 bits of the u32 Hz value** (so a 148.5 MHz clock reads back as 0x9530). Everything else stays as
  the caller sent it (zero from the library). It reflects neither SetTimingConfig nor SetPanelParam. `pPanelName` is a
  kernel address (`&rec->m_pPanelName`), meaningless in user space.

**Net effect against this mhal build** (§4.3): only Init / DeInit (instance and unified bring-up), SetMipiDsiConfig,
SetTimingConfig and SetOutputPattern reach the hardware; SetPowerOn, SetBackLight, SetBackLightLevel,
SetDrvCurrentConfig and SetSscConfig always fail with 0x1F (NOT_SUPPORT, or the missing PARAM flag for SSC);
SetPanelParam succeeds without effect. The MI layer itself adds only dispatch and caching.

### 3.4 Proc interface
On the first successful Init the module registers an mi_sys device: `mi_sys_RegisterDev(info{module id 0x19, dev 0, no
ports or channels, 1 pass}, …)` (`mi_sys-1-engine.md` §2.1; a second Init gets the same handle back), with
`OnDumpDevAttr`, `OnHelp` and six commands (`mi_sys_RegistCommand(name, maxArgs, fn, h)`). With no ports, the bind
capability slots stay NULL (`mi_sys-1-engine.md` §3.3).

`cat /proc/mi_modules/mi_panel/mi_panel0` prints, per instance, from the unified copy: panel name, link type name (the
18 names of §2.3, `UNKNOWN` otherwise), timing (H/V total, width, height, H/V start; then hbp, hspw, hfp = HTotal −
HSyncWidth − HBackPorch − Width, vbp, vspw, vfp likewise, dclk, fdclk and the frame rate = dclk / (HTotal·VTotal), where a
dclk below 10^6 is taken as MHz and multiplied by 10^6), polarity, SSC step/span, RGB swap, output format
(10BIT/6BIT/8BIT/565BIT), sRGB data type/swap/delta modes; for MIPI_DSI the 14 D-PHY values, porches, lane mode
(1lane…4lane), format (rgb565/rgb666/loosely_rgb666/rgb888), ctrl mode (cmd_mode/sync_pulse/sync_event/burst_mode),
sync calibrate, virtual H-sync, V-sync ref, data/clock skew, lane map, packet type (DCS/GENERIC), polarities; for MCU the
phase/polarity flags and values, H/V active, WR/CS/RS cycle counts, MCU type (68SYS/80SYS) and the 19 bus
configurations (`RGB565_BUS16` … `RGBB9_9_17`).

Commands (`echo <cmd> <args> > /proc/mi_modules/mi_panel/mi_panel0`); the first argument selects the instance:
**1 → MIPI_DSI, 2 → TTL_SPI_IF, anything else → TTL** (the other types cannot be addressed); no matching instance →
return 0 silently.

| command | args (exact count) | action |
|---|---|---|
| `pattern <sel> <r> <g> <b>` | 5 | `MhalPnlSetTestPatternConfig({1, r, g, b})`, cache on success |
| `setssc <sel> enable\|disable <span> <step>` | 5 | `MhalPnlSetSscConfig({en, +2 = 5th arg (step), +4 = 4th arg (span)})`, cache |
| `settiming <sel> <name> <val> [<name> <val>…]` | ≥ 4, pairs | names `width height hpw hbp htotal vpw vbp vtotal hstart vstart dclk` edit a copy of the cached ParamConfig, then `MI_PANEL_IMPL_SetPanelParam`; an unknown name aborts with 0x1F |
| `setmipidsi <sel> <name> <val>` | 4 | `MhalPnlGetMipiDsiConfig` into a local, then the name (`width hpw hbp hfp height vpw vbp vfp lanenum format ctlmode`) is written into the **instance cache** (+0x1c copy), and `MhalPnlSetMipiDsiConfig` is called with the **unchanged local** — the edit never reaches mhal (bug); an unknown name prints `param is Invalid` |
| `dumpmipicmd` | 0 | prints the DSI command buffer of the MIPI_DSI instance in the `{cmd, count, data…}` / `0xFE` delay / `0xFF 0xFF` end format (§4.7), from the unified copy (+0x1d0 pointer, +0x1d4 size of the instance) |
| `dumpmcucmd` | 0 | prints the MCU auto (+0x188 / +0x18c) and init (+0x180 / +0x184) command arrays of the MCU instance as `[reg]:value` pairs |

Handlers called without the registered device handle (argument 4 = 0) log and return 0x1F. `OnHelp` prints the command
list (with the misleading label `pattern:` before every line).

## 4. mhal.ko block PNL — LCD / MIPI-DSI panel HAL [R]
<!-- ev:panel-mipitx.4 -->

### 4.1 Source match, exports, consumers
<!-- ev:panel-mipitx.4.1 -->
263 functions; 255 decompiled, the other 8 are 4-byte tail jumps (`MhalPnlCreateInstanceEx` → `DrvPnlIfCreateInstance`,
`DrvPnlOsMemAlloc` / `DrvPnlOsMemRelease` / `DrvPnlOsMsSleep` / `DrvPnlOsUsSleep` → CamOs, `_HalPnlIfSetInitConfig` →
`HalPnlSetAllPadIn`, `_HalPnlIfSetTimingConfig` → `__HalPnlIfSetTimeGenConfig`, `_HalPnlIfSetTestPatConfig` →
`__HalPnlIfSetTestPatConfig`). **250 of the 255 names are defined in the same-chip source** OpenIPC u-boot
`drivers/mstar/panel/{drv/mipnl,drv/pnl,hal/mercury6}`; binary-only:
`_DrvPnlIfSetClkOn`, `_DrvPnlIfSetClkOff`, `_HalPnlIfGetInfoDeInitConfig`, `_HalPnlIfSetDeInitConfig`,
`__HalPnlIfIsDispPll`; source-only: `Pnl_ClkRegister` / `Pnl_ClkUnregister` (not built).

Agreement with the source: the API, context, translator and query layers and the LPLL, SSC, unified and DSI functions,
all 113 functions of `hal_pnl.c` (incl. `__HalPnlSetMcuConfigIDXCmd`) and all 17 remaining functions of `hal_pnl_if.c`
(pad-mux, prototype, polarity, test-pattern and time-generator setters of both the legacy and the unified path,
`HalPnlIfInit` / `HalPnlIfDeInit`, the small GetInfo checks, `_HalPnlIfGetCallBack`) were read side by side with the
binary. Of the 130 `hal_pnl.c` / `hal_pnl_if.c` functions, 121 have the same register, mask, value
expression, order and condition as the source; the other 9 carry the differences of §4.5. The source is a slightly older
revision [I]: a constant comparison with addressing normalised (the raw one is dominated by addressing style — the source uses RIU addresses `0x1033xx`, the binary virtual
addresses) leaves 55 of 250 functions identical and the rest differing only by `__LINE__` numbers, structure offsets and
base-pointer-relative register access. **The source therefore specifies this block, apart from §4.5.**

23 exports = the 21 functions of `mhal_pnl.h` + `DrvPnlOsMemAlloc` / `DrvPnlOsMemRelease`. Importers: mi_disp
(`MhalPnlCreateInstanceEx`, `MhalPnlSetUnifiedParamConfig`, `MhalPnlDestroyInstance`, only for BT656 / BT1120 outputs)
and mi_panel (17, §3.1); never imported: `MhalPnlCreateInstance`, `MhalPnlGetParamConfig`,
`MhalPnlGetUnifiedParamConfig`, `MhalPnlSetDebugLevel`.

### 4.2 Boot-time part (runs on this camera)
<!-- ev:panel-mipitx.4.2 -->
mhal's `camdriver_init` calls `_DrvPnlModuleInit()` directly (between HDMITX and DISP init; `camdriver_exit` calls
`_DrvPnlModuleDeInit`) [R: order: `mhal-core.md` §3.2]. It allocates the char device
region `mpnl`; the cdev is added only when the class pointer was already set, which is not the case on this first call,
so no `/dev` node gets file operations (`mhal-core.md` §3.2, §4.2). It creates device `mpnl` in the msys sysfs class with
two attributes, `dbgmg` and `clk` (§4.10), looks up the DT node `compatible = "sstar,pnl"` and stores it. The DT node has
`status = "ok"` and a property literally named `#clocks = <0x5f>`, which `of_clk_get` ignores, so PNL gets no clock from
the DT; the DT properties PNL can read (`ttl-16bit-mode`, `ttl-24bit-mode` through
`DrvPnlOsGetMode`, both absent) are in `mhal-core.md` §7. **No register is written at boot.** The platform-driver
variant (`DrvPnlModuleInit`, `DrvPnlModuleProbe`, …) is built but camdriver does not register it.

### 4.3 Contexts and the query dispatcher (same as the source)
- **Contexts**: 2 (`gstPnlCtxTbl`, 0x14 bytes each: u8 used; u16 id = the CreateInstanceEx id; pointers to a 300-byte
  "hal ctx", a 0x48-byte block and the 0xe0-byte unified hal copy). The id selects the timing-generator bank:
  **id 0 → bank 1151 (DISP OP2), id 1 → 1161 (OP2_1)**; ids > 1 fail INIT. A third CreateInstance fails.
- **Query dispatcher**: every setter builds `{u32 id; void *cfg; u32 size; ret; cb}` and calls `HalPnlIfQuery`; a table
  of 14 `{GetInfo, Set, u16 size}` entries (filled by `HalPnlIfInit`) checks the size, runs `GetInfo` (0 OK, 1 CFGERR,
  2 NOT_SUPPORT) and then `Set`. Ids and sizes: 0 PARAM 0x90, 1 MIPIDSI_PARAM 0x60, 2 SSC 10, 3 TIMING 0x1a, 4 POWER 1,
  5 BACKLIGHT_ONOFF 1, 6 BACKLIGHT_LEVEL 4, 7 CURRENT 2, 8 TEST_PAT 8, 9 CLK_SET 4, 10 CLK_GET 4, 11 INIT 0, 12 DEINIT 0,
  13 UNIFIED_PARAM 0xe0.
- **HAL enums** (`hal/pub/hal_pnl_st.h`, matching the binary's switch statements) [S, R]: link type 12 BT656, 13 BT601,
  14 BT1120, 15 SRGB, 16 MCU, 17 TTL_SPI — the HAL enum swaps MCU and SRGB relative to mhal / MI (mhal 15 MCU → hal 16,
  mhal 16 SRGB → hal 15); MCU bus 0–0x13; DSI lanes 0–4; packet type DCS 0 / GENERIC 1; output bit mode 6BIT 1, 8BIT 2,
  565 3; swap B 1 / G 2 / R 3.
- **What each entry does in this build**:

  | query | effect |
  |---|---|
  | INIT (CreateInstance) | ctx id ≤ 1 check; `HalPnlSetAllPadIn`: 101E.50 = 0 and 116D.71 = 0 (all pads in) |
  | DEINIT (DestroyInstance) | ctx id ≤ 1 check; `HalPnlSetLcdRgbEn(0)` (1154.18) |
  | UNIFIED_PARAM | stores the 0xe0 copy and programs everything, §4.4 |
  | MIPIDSI_PARAM | stores 0x60 bytes at hal ctx +0x98, flag bit 1; fails if lanes = 0, if the link type is not MIPI_DSI, or for burst mode (ctrl 3) on chip revision 1 ("U01"); Set = the DSI bring-up of §4.7 |
  | TIMING | stores at hal ctx +0x102, flag bit 3; Set = `__HalPnlIfSetTimeGenConfig` |
  | TEST_PAT | stores at +0x124; Set = `__HalPnlIfSetTestPatConfig` (frame colour R/G/B + enable on the context's OP2 bank, 1151/1161.1B–1C) |
  | SSC | needs flag bit 0 ("Param Need to Set First") — **nothing in this build sets bit 0** (the PARAM path that set it is gone, §4.5 item 2), so SSC always fails; its Set code (step / span given, or computed from modulation ≥ 15 kHz and deviation: span = (432000/modulation·2^17)/LPLL_SET, step = LPLL_SET·deviation/1000/span) is unreachable through the query |
  | POWER, BACKLIGHT_ONOFF, BACKLIGHT_LEVEL, CURRENT | GetInfo returns NOT_SUPPORT → the Mhal call returns 0 (mi_panel then returns 0x1F) |
  | PARAM | GetInfo and Set are empty: **`MhalPnlSetParamConfig` succeeds and does nothing** |
  | CLK_SET / CLK_GET | the clock count must be 0 (the HAL owns no CCF clocks); CLK_GET reads the MIPI-TX-DSI clock register 1038.5E (§4.9) |

  Getters (`Get*Config`) copy from the hal ctx, i.e. return what was last stored (power and backlight bytes are never
  stored → 0).

### 4.4 Unified bring-up (`_HalPnlIfSetUnifiedParamConfig`, same order as the source)
Prototype config; if the timing flag: time generator (`__HalPnlIfSetUnifiedTimeGenConfig`: H/V sync start/end, DE,
total on 1151/1161.10–20, LCD timing 1154.12–16/24/25; LCD H back porch 1154.14 per §4.5 item 7) + LPLL; polarity
(1154.18); RGB channel swap / ML swap (1151.7E); output bit mode (`HalPnlSetRgbMode`); SSC enable (1033.4E) and
step/span (1033.4E/4F) if the SSC flag, else SSC off; MCU phase / polarity / RS polarity (1154.50/51); RGB delta mode
(1154.1B); pad mux by link type (TTL / TTL_SPI → 103C.6C, BT656 / 601 / 1120 → 103C.60/72, SRGB → 103C.74, MCU →
103C.78); MCU set-up (1154.10, active size 1154.24/25, pixel count 1154.38, bus 1154.3A/50/52, cycle counts 1154.58, init
command list sent with LCD RGB enabled, auto command list 1154.4E/5E); MIPI-DSI bring-up (§4.7) if the MPD flag.

### 4.5 Differences from the u-boot mercury6 source [R binary; S source]
<!-- ev:panel-mipitx.4.5 -->
1. **The LPLL table has 6 entries instead of 4.** `HalPnlGetLpllIdx` adds exact matches **74.25 MHz → idx 3 and
   148.5 MHz → idx 1** (loop gain 22) before the range checks; the ranges map to idx 0 (100–187.5 MHz), 2 (50–100),
   4 (25–50), 5 (12.5–25); below 9 MHz, 9.5–12.5 MHz and above 187.5 MHz are unsupported (idx 6); 9–9.5 MHz returns idx 6
   with "success", as in the source. DSI (lane bit rate): 100–200 Mbps → 5, 200–400 → 4, 400–800 → 2, 800–1500 → 0, else
   unsupported. Tables [R `.data`]: gain (16, 22, 8, 22, 4, 2), divider LCD (8, 8, 8, 16, 8, 8), DSI (1, 1, 1, 2, 1, 1);
   `LPLLSettingTBL` (6 × 6 register writes): 1033.40 = 0x2201, 1033.41 = 0x0420
   (0x0B10 for the two exact entries), 1033.42 = 0x41/0x41/0x42/0x42/0x43/0x83, 1033.43 = 0/0/1/1/2/3, 1033.4A = 1,
   1033.4B = 0. The LPLL SET value goes to 1033.48/49 (`HalPnlSetLpllSet`).
2. **The legacy ParamConfig path is removed**: `_HalPnlIfGetInfoParamConfig` returns OK without storing and
   `_HalPnlIfSetParamConfig` is empty (the source stores the param, sets flag bit 0 and programs pad mux, time generator,
   prototype, polarity, LPLL, SSC and RGB mode). Consequences: SetParamConfig is a no-op, and SSC can never pass its
   "param first" check (§4.3). The helper functions of that path (`__HalPnlIfSetPadMux`, `__HalPnlIfSetPrototypeConfig`,
   `__HalPnlIfSetPolarityConfig`, `__HalPnlIfSetLpllConfig`) are still built; the last one is reached from the DSI path.
3. **DEINIT query added** (id 12 in the table `HalPnlIfInit` fills: range check + LCD RGB disable); INIT gets the same
   range check (source: none).
4. **Second PLL: `__HalPnlIfIsDispPll`** (new): for ctx id 0 with link type TTL it looks up ctx 1 and returns true if
   that one is MIPI_DSI. `__HalPnlIfSetLpllConfig` and `__HalPnlIfSetUnifiedLpllConfig` both pass its result to
   `HalPnlDumpLpllSetting(idx, bDispPll)` and `HalPnlSetLpllSet`, which carry a PLL-select argument the source lacks
   (also `HalPnlGetLpllSet`): when non-zero, every register of the LPLL table row (regs 40, 41, 42, 43, 4A, 4B) is
   redirected from bank 1033 to bank **0x1037**, the DISP PLL (address `(reg << 1 & 0x1FE) | 0x206E00`, same register
   index), and the SET value goes to 1037.48/49 instead of 1033.48/49. The DSI then keeps the LPLL, and the SSC
   computation reads the selected PLL [R call sites; I purpose: a TTL panel on context 0 and a DSI panel on context 1
   each get their own PLL].
5. **Clocks**: `_DrvPnlIfSetClkOn` / `_DrvPnlIfSetClkOff` are stubs returning 1; `DrvPnlIfSetClkConfig` accepts only a
   clock count of 0, and `_HalPnlIfGetInfoClkGetConfig` / `_HalPnlIfGetInfoClkSetConfig` check a count of 0 (source 1).
   The DSI clock is switched by register (1038.5E, §4.9) instead of a CCF clock.
6. **`_HalPnlMcuSendCmd`** (MCU-interface command trigger): the source first clears the trigger bit, then sets it —
   `1154.08` bit 2 (index) or bit 1 (command) := 0, `1154.0D` same bit := 1 if `1154.0B` has it, `1154.00` |= 0x10 /
   0x04, `1154.08` bit := 1. The binary **omits the initial clear** of `1154.08` (disassembly `.text` 0xa3acc–0xa3b14:
   only the 0B test and the 0D / 00 / 08 OR-writes). [I: if the trigger is edge-sensitive and not self-clearing, a
   second command of the same kind finds the bit already set and produces no edge.] Whether `1154.08` is edge- or
   level-triggered is not known; it shows only with an MCU panel on hardware, and this camera has none.
7. **`__HalPnlIfSetUnifiedTimeGenConfig`** (time generator of the unified path, the one mi_disp and mi_panel use): LCD H
   back porch `1154.14` = **HTotal − HActive for link types 12–14 (BT656, BT601, BT1120)**; the source uses this only for
   BT656 and BT1120 and gives BT601 `HSyncWidth + HSyncBackPorch` (disassembly `.text` 0xa5d96: `subs r3, #12; cmp r3,
   #2; ls → HTotal − HActive`).
8. **Debug prints only** (no register effect) added in `__HalPnlIfSetPolarityConfig`, `__HalPnlIfSetTestPatConfig`,
   `__HalPnlIfSetTimeGenConfig`, `__HalPnlIfSetUnifiedTimeGenConfig`, `__HalPnlIfSetUnifiedPolarityConfig`,
   `__HalPnlIfSetUnifiedSwapChnConfig`, `_HalPnlIfGetInfoClkGetConfig` (debug mask bit 5).

### 4.6 Equal in source and binary, but not what the names say [R]
- `HalPnlSetTtlPadCotnrol` writes `101E.3F = (u32Drv > 16)` (a 0/1 value) — the source's `>` for `>>` typo, compiled as
  written in both; the upper half of the drive-strength word never reaches the register.
- `HalPnlSetRGBPadMux`: `else if (E_HAL_PNL_OUTPUT_565BIT_MODE)` is a constant-true condition, so every bit mode other
  than 8-bit writes `103C.74` bit 12.
- `HalPnlSetMipiDsiPhyTimConfig`: the default ClkZero is `(400 − ClkHsPrpr·cycle·1000)/cycle` (+1 on remainder) — the
  ×1000 applies only to the product.
- `_HalPnlSetMcuAutoConfigIDXCmd` with a slot number outside 1–7 writes both halves of the value to RIU address 0
  (`0xFD000000`).
- `HalPnlSetMipiDsiLongPacket` / `HalPnlSetMipiDsiShortPacket` print "Time Out" also when the done bit arrives on the
  last of the 255 polls.
- Empty: `HalPnlSetDeInv`, `HalPnlSetVsynRefMd`, `HalPnlSetMipiDsiPadOutSel`. The DAC setters (`HalPnlSetDac*`) take a
  context argument and ignore it (fixed bank 1151).
- The drive-current translators are swapped (`…ToHal` copies hal → mhal): `MhalPnlSetDrvCurrentConfig` sends 0 and
  **writes 0 into the caller's struct** — without effect, since CURRENT is NOT_SUPPORT (§4.3).
- Context free (`DrvPnlCtxFree`) clears only 0x48 bytes of the 0xe0-byte unified copy (the same size constant in the
  source); harmless, since the next allocation clears all of it.

### 4.7 MIPI-DSI bring-up and the init-command format [R `_HalPnlIfSetMipiDsiParamConfig`, `__HalPnlIfSetMipiDsiPacket`]
Order: polarity per channel (`PolCh != 2` → positive) → 1535.06…; TTL pad control off (101E.38–3E); time-generator
external H-sync enable; LPLL skew 1033.44[15:8] = DataClkSkew (7 if 0); OP2 "disp to DSI" mode (1150.32); LPLL for the
DSI rate; `HalPnlInitMipiDsiDphy` (1535.00/01/04/14 — the D-PHY bank the CSI-TX block uses as well, §8); channel select
from the lane map (1535.03/06/0A/0E/22/25); pad out select and lane count (1A38.06); if a command buffer is given:
command mode (1A38.05 = 0), clock HS mode (1A38.41), send the commands; then the final LPLL, PHY timings (1A38.44), video
timing (1A38.07/0B/14–16), the requested control mode, DSI clock enable (1A38.00).

Command buffer (`pu8CmdBuf`, `u32CmdBufSize`): records `{u8 cmd; u8 n; u8 data[n]}`, walked until fewer than 2 bytes
remain: `FE FE <ms>` = delay (3 bytes); `FF FF` = end; n = 0 → short packet, the record is 3 bytes (`cmd 00 xx`); n = 1 →
short packet with one parameter; n ≥ 2 → long packet of n + 1 bytes (`cmd` + data). DCS or generic packets per
`enPacketType`. The buffer is consumed within the call; it needs to stay valid only for the call (mi_panel frees its copy
right after, §3.3).

### 4.8 `MhalPnlUnifiedParamConfig_t` as this build uses it (0xd4 bytes) [R offsets from mi_panel's writer and mhal's reader]
The public header's member order holds, but **this build has a 4-byte gap at +0x30 and a 2-byte gap at +0x44** (never
read, left 0 by both writers): +0x00 `pPanelName`; +0x04 `eLinkType`; +0x08–0x15 the 14 u8 flags (timing, polarity,
RGB swap, output bit mode, fixed DCLK, SSC, pad mux, MCU phase, MCU polarity, MCU RS polarity, MCU config, RGB data, RGB
delta, MPD); +0x18 timing (u16 HSyncWidth, HSyncBackPorch, VSyncWidth, VSyncBackPorch, HStart, VStart, HActive, VActive,
HTotal, VTotal; +0x2c u32 Dclk); **+0x34** polarity (InvDCLK, InvDE, InvHSync, InvVSync); +0x38 RGB swap (R, G, B, ML);
+0x3c `eOutputFormatBitMode` (the HAL clamps > 2 → 3); +0x40 `u32FDclk`; **+0x46** SSC step, +0x48 span; +0x4a
`u16PadMux`; +0x4c MCU phase, +0x4d polarity, +0x4e RS polarity; +0x50 MCU HActive, +0x54 VActive, +0x58–0x5a
WR / CS-lead / RS-lead cycle counts, +0x5c MCU type, +0x60 data-bus cfg, +0x64 / +0x68 init command pointer / count,
+0x6c / +0x70 auto command pointer / count; +0x74 u8 RGB data swap, +0x78 RGB data type; +0x7c / +0x80 RGB delta odd /
even; +0x84 the MIPI-DSI block (`MhalPnlUnifiedMipiDsiConfig_t`: 14 u8 PHY timings, 10 u16 porches / active / BLLP /
fps, +0x24 lanes, +0x28 format, +0x2c ctrl, +0x30 cmd pointer, +0x34 size, +0x38 sync calibrate, +0x3a–0x40 virtual
H-sync start / end, V-sync ref, data / clock skew, +0x42 PolCh0–4, +0x47 SwapCh0–4, +0x4c packet type) to +0xd0.
mi_disp and mi_panel both write this layout; the member names by position are the header's, the offsets the binary's.
The HAL keeps its own 0xe0-byte unified copy per context (§4.3).

### 4.9 Register banks [R]
<!-- ev:panel-mipitx.4.9 -->
| bank | registers → function |
|---|---|
| 1151 / 1161 (OP2 / OP2_1, by context) | .10 `HalPnlSetTgenExtHsEn`; .11 Htt, .12 Vtt; .13 / .14 H-sync start / end, .15 / .16 V-sync start / end; .17 / .18 H frame-DE start / end, .19 / .1A V frame-DE; .1B / .1C frame colour and enable (test pattern); .1D / .1E H DE, .1F / .20 V DE; .76 dither enable; .7E RGB mode, RGB swap, ML swap |
| 1151 only | .23 / .24 DAC H-sync start / end, .27 / .28 DAC H DE, .29 / .2A DAC V DE (`HalPnlSetDac*`) |
| 1150 | 1150.25 `HalPnlSetDsiGateEn`; 1150.32 `HalPnlSetDispToDsiMd` |
| 1154 (LCD) | .12 V back porch, .13 V porch total, .14 H back porch, .15 H porch total, .16 H / V sync width, .24 / .25 H / V active, .38 frame pixel count; .18 LCD RGB enable and V-sync / H-sync / clock polarity; .19 RGB data swap and type; .1B delta mode; .10 disp1 used / type and MCU control; .60 BT encoder mode, clip, enable, BT601 enable; MCU: .00, .08, .0B, .0D (command trigger, §4.5 item 6), .3A / .50 / .52 bus, .4E index command, .50 type / phase / polarity, .51 RS polarity, .58 cycle counts, .5E command count |
| 1156 (LCD misc, `REG_LCD_MISC_BASE`) | 1156.23 `HalPnlSetLcdMiscTgenSyncEn`; 1156.32 `HalPnlSetLcdMiscDacMux` |
| 1033 (LPLL) | .40–.43 / .4A / .4B table row, .48 / .49 SET, .44 skew, .4E SSC enable and step, .4F SSC span |
| 1037 (DISP PLL) | the same registers when `__HalPnlIfIsDispPll` (§4.5 item 4) |
| 1A38 / 1A39 (MIPI-DSI controller) | 1A38.00 DSI clock enable / disable; .04 reset; .05 control mode; .06 lane count; .07 / .0B / .14–.16 video timing; .41 clock HS mode; .44 PHY timing; .5F pattern generator; short / long packets 1A38.00 / .03 / .18, 1A39.00 / .01 (read-back also 1A38.06 / .1D–.20) |
| 1535 (MIPI D-PHY, §8) | .00 / .01 / .04 / .14 `HalPnlInitMipiDsiDphy`; .03 / .06 / .25 channel select; .06 channel polarity |
| 1038 (clkgen) | .5E MIPI-TX-DSI clock mux [12:10], gate bit 8 (`HalPnlClkSetMipiTxDsi` / `HalPnlClkGetMipiTxDsi`) — written directly |
| 103C (pads) | .6C TTL, .60 / .72 BT, .74 RGB, .78 MCU pad mux |
| 101E (pads) | .38 / .3A / .3C / .3E (and .3F, §4.6) TTL pad control; .09 MIPI-DSI pad mux; .50 = 0 (all pads in) |
| 116D | .71 = 0 (all pads in) |
| 001E | 001E.67 read: chip revision (`HalPnlGetRevision`) |

The banks 1151, 1154 and 1161 are shared with mhal DISP (`mhal-core.md` §2.5, `mhal-disp.md` §5.1).

### 4.10 Debug interface
`/sys/class/<msys>/mpnl/dbgmg` (debug mask; its bits enable DRV / HAL / CTX / clock messages) and `…/clk` (show:
MIPI-TX-DSI clock enable and rate index; store: parsed into a CLK_SET query, which accepts no clock — in effect
read-only). DSI clock rates by mux index (`HalPnlClkMapMipiTxDsiToRate`): 1 160 MHz, 2 144 MHz, 3 108 MHz, 4 216 MHz,
5 240 MHz.

## 5. libmi_mipitx.so — user library [R]
<!-- ev:panel-mipitx.5 -->

10 exports: 9 `MI_MipiTx_*` and `sstar_mi_so_version_string`. Same frame as libmi_panel (§2.1): device `/dev/mi_mipitx`,
the common lazy open helper, the 16-byte wrapper, payload `{u32 chn; <struct>}`, type `'i'`.

| export | cmd | payload after `u32 chn` | copy-back |
|---|---|---|---|
| `MI_MipiTx_CreateChannel(chn, ChnAttr*)` | 0x40206900 len 0x20 | the 0x1c-byte attr | — |
| `MI_MipiTx_DestroyChannel(chn)` | 0x40046901 len 4 | — | — |
| `MI_MipiTx_GetChannelAttr(chn, ChnAttr*)` | 0xc0206902 len 0x20 | 0x1c zero bytes (so the kernel sees `pu32LaneSwap` = NULL, §6.3) | 0x1c bytes, **also when the ioctl failed** (then zeros) |
| `MI_MipiTx_StartChannel(chn)` / `MI_MipiTx_StopChannel(chn)` | 0x40046903 / 0x40046904 len 4 | — | — |
| `MI_MipiTx_GetTimingConfig(chn, TimingCfg*)` | 0xc0106905 len 0x10 | 9 zero bytes + 3 pad | 9 bytes, also on failure |
| `MI_MipiTx_SetTimingConfig(chn, TimingCfg*)` | 0x40106906 len 0x10 | the 9 bytes, 3 zero pad | — |
| `MI_MipiTx_InitDev()` | open only; 0, or **0x1F** if the open failed | | |
| `MI_MipiTx_DeInitDev()` | common close helper only (no ioctl) | | |

Types [R sizes and kernel use]:
- `MI_MipiTx_ChannelAttr_t` (0x1c) [names I]: +0x00 u32 `u32Width`, +0x04 u32 `u32Height`, +0x08 u32 `ePixelFormat`
  (`MI_SYS_PixelFormat_e`), +0x0c u32 `eLaneNum` (1–4), +0x10 u8 `u8DclkDelay` (log name "DclkDelay"), +0x14 u32
  `u32Dclk` (lane bit rate in bit/s, §7.4), +0x18 ptr `pu32LaneSwap` (user pointer to `u32[5]`, or NULL for the default
  map).
- `MI_MipiTx_TimingConfig_t` (9 × u8, order from the kernel's log string) [R]: `Lpx`, `ClkHsPrpr`, `ClkZero`, `ClkHsPre`,
  `ClkHsPost`, `ClkTrail`, `HsPrpr`, `HsZero`, `HsTrail` (D-PHY timings in PHY clock units [I]).

## 6. mi_mipitx.ko — kernel module [R]
<!-- ev:panel-mipitx.6 -->

### 6.1 Load, open, close
`init_module`: `/proc/mi_modules/mi_mipitx/` with the four standard files (version string of the header); module
parameters `debug_level` / `debug_file` / `debug_func` as in mi_panel, plus `thread_priority` (int, default **70**), the
worker-thread priority handed to `mi_sys_RegisterDev`; prints `module [mipitx] init`; `MI_DEVICE_Register` →
`/dev/mi_mipitx`. Nothing else at load.

Unlike mi_panel, the work is done per open: **`mi_MipiTx_process_init` (every client's open) calls
`MI_MipiTx_IMPL_Init`**, `process_exit` (every close) calls `MI_MipiTx_IMPL_DeInit`; a u8 open counter makes only the
first open and the last close do anything (other calls log `already inited.` / the DeInit message):
- **First open (Init)**: `mi_sys_RegisterDev(info, usr = &devRes, procfs ops, dir)` with info {module id **0x1e**, dev 0,
  1 input port, 1 output port, 1 channel, 1 pass, thread priority = the `thread_priority` parameter}; pass callbacks
  (slots of `mi_sys-1-engine.md` §2.3): 0x00 `OnPreProcessInputTask`, 0x08 `EnqueueInputTask`, 0x0c `BarrierInputTask`
  (returns 0), 0x1c `CheckInputTaskStatus`, 0x20 `DequeueInputTask`, 0x24 `CheckOutputTaskStatus` (returns 1), 0x28
  `DequeueOutputTask` (returns 1), 0x40 `OnGetInputPortBindCapability` (writes 1 to cap+0xc, no NULL check), 0x44
  `OnGetOutputPortBindCapability` (a stub that writes nothing, so binds from mipitx are refused; `mi_sys-1-engine.md`
  §3.3), 0x48 / 0x50 / 0x58 / 0x60 bind / unbind input / output (all return 0); procfs ops DumpDev, DumpChannel,
  DumpInputPort (0), DumpOutPort (0), OnHelp (0). `_MI_MipiTx_OnGetPassInputPortNum` (returns 1) and
  `_MI_MipiTx_OnGetPassOutputPortNum` (0) exist but are not registered. Registration failure → −1. Then the proc command
  `clearbuf`, `CamOsTsemInit(&chnSem, 1)`, `MHalMipiTxCsiGetIrqNum(&irq)` and `CamOsIrqRequest(irq, _MI_MipiTx_Isr,
  "MI_MipiTx-IRQ", &devRes)` (failure only logged). On this board the IRQ is GIC SPI 0x56 (DT `mipitx_csi`, §7.1).
- **Last close (DeInit)**: `CamOsIrqFree`; if a channel exists: stop it if started, destroy it; `CamOsTsemDeinit`,
  `mi_sys_UnRegisterDev`, clear the device state (0xa0 bytes). A close by any process counts; the channel created by one
  process is torn down only when the counter reaches 0.

### 6.2 Device state (one channel) [R addresses; I names]
`devRes` (.bss, 0xa0 bytes + semaphore): +0x00 u8 `bCreated`; +0x04 mhal instance; +0x08 channel id (always 0); +0x0c
status (0 created, **1 started, 2 stopped**); +0x10 the MI pixel format; +0x14 the 0x34-byte mhal param copy (§7.4:
width, height, mhal data type, lanes, DCLK delay, DCLK, lane map, polarity); +0x48 `clearbuf` value (stored, never
read); +0x4c mi_sys device handle; +0x50 u8 init counter; +0x52 u16 channel count; +0x54 atomic "frames in flight";
+0x58 rewind (drop) counter; +0x5c IRQ number; +0x60 ISR count; +0x64 send count; +0x68 u64 time of the last enqueue
(µs); +0x70 u64 time of the last ISR; +0x78 u64 ISR→ISR interval ("IsrInv"); +0x80 u64 enqueue→ISR latency
("ProToIsr"); +0x88 u64 last ISR→enqueue interval ("IsrToPro"); +0x90 u64 time of the last "all done"; +0x98 u64
dequeue count; +0xa0 `CamOsTsem` (per channel, stride 0x20).

### 6.3 ioctl handlers and IMPL semantics
`MI_DEVICE_WrapperFunction(nr)`: a table of 7 for nr < 7: 0 CreateChannel, 1 DestroyChannel, 2 GetChannelAttr,
3 StartChannel, 4 StopChannel, 5 GetTimingConfig, 6 SetTimingConfig. Each handler locks the module mutex (unlocked by
mi_common), logs `<%s> Ch: %d.` at debug level > 4, and calls the IMPL with `(chn, payload+4)`. Error codes are
`MI_DEF_ERR(0x1e, level, id)` (`mi-user-abi.md` §4.2): `0xA01E1002` (warning, invalid channel), `0xA01E1003` (warning,
illegal parameter — here used for "wrong state"), `0xA01E1014` (not stopped), `0xA01E1016` (already created), `0xA01E2004`
(error, used for every mhal failure), `0xA01E2006` (null pointer).
- **CreateChannel(chn, attr)**: attr NULL → 2006; chn ≠ 0 → 1002; already created → 1016; channel count ≠ 0 → 1002.
  Under the channel semaphore: `MhalMipiTxCsiCreateInstance(&devRes.inst)` (≠ 1 → 2004). Builds the mhal param (0x34,
  zeroed): width, height; **data type 4 (YUV422) for MI pixel formats 0 (YUYV) and 14–16 (UYVY / YVYU / VYUY), 5 (RAW8)
  for 20–31 (the Bayer formats); any other format logs `MI Pixel %d, no support` and uses 4**; lanes, DCLK delay, DCLK
  as given; lane map: if `pu32LaneSwap` ≠ NULL, 20 bytes are copied from the user and placed as **mhal ch0 = arr[4],
  ch1 = arr[0], ch2 = arr[1], ch3 = arr[2], ch4 = arr[3]** (printed as `Chn0~4, …`), else the default map ch0..ch4 =
  **2, 0, 1, 3, 4**; the five polarity bytes stay 0. `MhalMipiTxCsiSetParamConfig` (≠ 1 → 2004; the instance is **not**
  destroyed). Success: count++, bCreated = 1, status = 0 (created), param copy stored; on the first channel
  `MHalMipiTxCsiEnableIrq(1)`.
- **DestroyChannel(chn)**: chn ≠ 0 or not created → 1002; status ≠ 2 (stopped) → 1014 — **a channel that was created
  but never started cannot be destroyed** (only the last close tears it down; DeInit calls Destroy regardless).
  `MhalMipiTxCsiDestroyInstance` (≠ 1 → 2004); clears 0x4c bytes of the state; count−−; on 0
  `MHalMipiTxCsiEnableIrq(0)`.
- **GetChannelAttr(chn, attr)**: `MhalMipiTxCsiGetParamConfig` → width, height, lanes, DCLK delay, DCLK; pixel format =
  the stored MI value; if the caller's `pu32LaneSwap` ≠ NULL the five map words are written **straight to that user
  address from kernel code (no copy-to-user)** and in mhal order (ch0..ch4), i.e. not the inverse of the Create rotation.
  The library always sends NULL there, so through libmi this path never runs.
- **StartChannel(chn)**: not created → 1002; already started → 1003; `mi_sys_EnableInputPort(h, 0, −1, 0)` and
  `mi_sys_EnableChannel(h, 0)` (failure = BUG_ON); status 1. Nothing is written to the hardware here: frames start the
  transfer (§6.4).
- **StopChannel(chn)**: status ≠ 1 → 1003; `mi_sys_DisableInputPort` + `mi_sys_DisableChannel`; status 2.
- **SetTimingConfig(chn, t)**: logs the nine values; `MhalMipiTxCsiSetMipiTimingConfig(inst, copy)` (≠ 1 → 2004).
- **GetTimingConfig(chn, t)**: **zeroes the caller's struct and calls `MhalMipiTxCsiSetMipiTimingConfig` with the
  zeros** — a "get" that programs all D-PHY timings to 0 (bug; `MhalMipiTxCsiGetMipiTimingConfig` is never called).

### 6.4 Frame path (mi_sys pass callbacks)
The module is a sink: one input port, bound from a frame source (e.g. SCL) by the normal MI bind.
- **OnPreProcessInputTask**: accepts a task only if chn 0 is created and **started**, the buffer is a frame
  (`eBufType` = 1), the pixel format is supported (0, 14–16, 20–31) and `u16Width` / `u16Height` equal the channel attr;
  then, if fewer than 2 frames are in flight, returns 0 with "no output buffer" (`meta+4` = 0), else counts a rewind and
  returns 1 ("not now", the task stays queued). Mismatch → log (`chn %d, status %d, input pixel %d, in size(%d, %d),
  cfgsize (%d,%d) err`) and 2 (drop).
- **EnqueueInputTask**: builds the mhal process descriptor `{u8 bTrig = 1; u32 TotalPixel = width·height; u32 StartAddr
  = phyAddr[0] (low 32 bits, +0x48 of the buffer info); u32 DataType = the frame's MI pixel format}` and calls
  `MhalMipiTxCsiProcess`; in-flight++, send count++, times updated. ≠ 1 → 3 (drop).
  **The DataType handed to mhal is the MI pixel enum, not the mhal data type (4 / 5) the channel was configured with**;
  mhal's byte-count switch knows only 4 / 5 / 6 / 8 / 9 (§7.4), so every accepted format (0, 14–16, 20–31) takes its
  default arm `0·pixels / 0` — an integer division by zero in the kernel (`__aeabi_uidiv` → `__div0` warning) and a byte
  count of 0 [R values]. As built, the MIPI-TX frame path through mi_mipitx cannot have worked [I: strong].
- **CheckInputTaskStatus**: done (1) when send count ≤ ISR count; on done the "last done" time is reset to now; if more
  than **3 s** pass without a done → log `over 3s` and return 3 (timeout).
- **DequeueInputTask**: dequeue count++, in-flight−− (an underflow attempt is logged `Atom ==0, err`). Returns 1.
- **ISR** `_MI_MipiTx_Isr`: `MHalMipiTxCsiCheckIrq()` result bit 0 (TX done, §7.6) → ISR count++, timestamps, then
  `mi_sys_NotifyWakeupISREvent(h, 0)`.

### 6.5 Proc interface
`cat /proc/mi_modules/mi_mipitx/mi_mipitx0`: the device part prints the header `IsrCnt IsrInv ProToIsr IsrToPro` and the
four counters / intervals; the channel part (if created) prints `ChnId Status Width Height Pixel LaneNum Dclk DclkDl atom
SendBufCnt FinishBufCnt RewindCnt` (Pixel = the mhal data type). Both return `0xA01E2004` for a device id ≠ 0. One
command: `echo clearbuf <chn> <value>` (≥ 3 words; the channel must exist) stores `value` at +0x48 — read by nobody.

## 7. mhal.ko block MIPITX — MIPI CSI-2 transmitter [R]
<!-- ev:panel-mipitx.7 -->

The block is a **CSI-2** transmitter only; MIPI-DSI output lives in the PNL block (DSI controller 1A38 / 1A39, §4.7). No
public source or header exists; everything below is read from the binary. Names of functions and globals are the
binary's symbols; field names follow its log strings (`bTrig`, `DataType`, `TotalPixel`, `StartAddr`, `LaneSwap`,
`Lpx`, …) or are chosen [I] where said.

### 7.1 Block, exports, boot-time part
<!-- ev:panel-mipitx.7.1 -->
101 functions (all decompiled), 12 exports: `MhalMipiTxCsiCreateInstance(void **)`, `MhalMipiTxCsiDestroyInstance(ctx)`,
`MhalMipiTxCsiSetParamConfig(ctx, p)` / `MhalMipiTxCsiGetParamConfig(ctx, p)`, `MhalMipiTxCsiSetMipiTimingConfig(ctx, t)`
/ `MhalMipiTxCsiGetMipiTimingConfig(ctx, t)`, `MhalMipiTxCsiProcess(ctx, buf)`, `MHalMipiTxCsiGetIrqNum(u32*)`,
`MHalMipiTxCsiEnableIrq(bool)`, `MHalMipiTxCsiCheckIrq()`, `MhalMipiTxCsiSetDebugLevel(u32*)`, and
`_DrvMipiTxCsiModuleInit`. All return `bool`-style 1 on success; a NULL argument logs `… is Empty` and returns 0. Only
mi_mipitx imports them (9).

**Boot (runs on this camera)**: mhal's `camdriver_init` calls `_DrvMipiTxCsiModuleInit` through `find_exec_symbol` (last
but two in the init order, `mhal-core.md` §3.2; `camdriver_exit` has no MIPITX de-init). It registers the char device
region and device **`mmipitx`** (the cdev is added only if the msys class already existed, which it does not on this
call: device and sysfs only, `mhal-core.md` §4.2), three sysfs attributes (§7.7), looks up DT `compatible =
"sstar,mipitx"` and takes IRQ index 0 of that node (`CamOfIrqToResource`). The DT node `mipitx_csi`: `status = "ok"`,
`clocks = <0x64>` with `clock-names = "CLK_mipi_tx_csi"`, `interrupts = <0 0x56 4>` (SPI 0x56, level)
. **No register is written at boot.** A platform-driver variant
(`DrvMipiTxCsiModuleInit`, `DrvMipiTxCsiModuleProbe`, `…Remove`, `…Suspend`, `…Resume`) is built but not registered.

### 7.2 Context and query layer
- One instance only (`gbMipiTxCsiCtxInit`: u8 init; the instance = {u8 used; ptr 0x50-byte "mhal copy"; ptr 0x3c-byte
  "hal ctx"}); a second CreateInstance fails (`CtxConfig is Empty`). The 0x50-byte copy holds the param (+0x00, 0x34),
  the timing (+0x34, 9) and the last process descriptor (+0x40, 0x10).
- The same query scheme as PNL (§4.3): `{id; cfg; size; ret; cb}`, table `gpMipiTxCsiCbTbl` of 4 `{GetInfo, Set, size}`:
  **0 PARAM 0x34, 1 MIPI_TIMING_CFG 9, 2 PROCESS 0x10, 3 CLK 0xc**. GetInfo copies the config into the instance (CLK
  checks a count of 1); Set programs the hardware. Ret codes 0 OK, 1 CFGERR (size mismatch), 2 NOT_SUPPORT.
- `CreateInstance`: ctx init (allocations), `HalMipiTxCsiIfInit` (fills the table **and runs `HalMipiTxCsiHwInit`**,
  once), allocate the instance, clock on (§7.3). `DestroyInstance`: clock off, free; when nothing is left, ctx de-init
  and `HalMipiTxCsiIfDeInit` (clears the table; no hardware de-init).
- `Get*Config` return the stored copies; `GetParamConfig` re-applies the clamps of §7.4 and does not return the five
  polarity bytes.

### 7.3 Clock and one-time hardware init
- **Clock on** (`_DrvMipiTxCsiIfSetClkOn`): the DT clock `CLK_mipi_tx_csi` (index 0) is set to **288 MHz**
  (`gu32MipiTxCsiClkRate`; `CamClkRoundRate`, then `CamClkSetRate(rounded + 1 MHz)`), prepared and enabled; then the CLK
  query writes the same clock-gen register directly: `HalMipiTxCsiSetClk(en, rate)` → **1038.5C[12:8]** = 0x04 for
  160 MHz, 0x10 for 216, 0x14 for 240, 0x18 for 288 (also for anything above 320 MHz), 0x1C otherwise; disable → 0x01
  (gate bit 8 set, mux 0); and **1536.6B[8]** = enable. 1038.5C is the register of the DT composite clock (`reg =
  0x1f207170`, mux bits 12:10, gate bit 8; `kernel-platform.md` §5.1), so the HAL and the CCF write the same field. Mux
  index → rate from `HalMipiTxCsiGetAllClkSetting` (debug print only): 4 → 216, 5 → 240, 6 → 288, 7 → 320 MHz; index 1 =
  160 MHz [I from the 0x04 code].
- **Clock off**: CCF disable / unprepare, then 1038.5C[12:8] = 0x01, 1536.6B[8] = 0.
- **`HalMipiTxCsiHwInit`** (first CreateInstance): 103C.64[1:0] = 1 (pad top, MIPI-TX pad mode [I]); low bytes of
  103E.43…103E.4C (ten registers) = 4 (pad drive / mode of the 5 lanes × P/N [I]); 1536.01 |= 2; 1536.14 = 2;
  1536.68 = 1; **1536.47[7:0] = buffer threshold** and **1536.40[5:0] = burst-count select**, both from
  `gu16MipiTxCsiBcSel` unless `gu8MipiTxCsiBufThd` ≠ 0 overrides the threshold: BcSel 0x80 → thd 0x78 / sel 0x01;
  0x100 → 0x70 / 0x00; 0x400 → 0x40 / 0x20; anything else (default **0x200**) → **0x60 / 0x10**; 1132.01 = 0 (meaning
  not known); **1536.70 = 0x0FFF** (all 12 interrupt sources masked).

### 7.4 Channel parameters (query PARAM, `MhalMipiTxCsiSetParamConfig`)
Param struct (0x34): +0x00 u32 width (pixels), +0x04 u32 height, +0x08 u32 data type (mhal enum, names from the debug
printer: 0 RGB565, 1 YUV420, 2 YUV420_LEGACY, 4 YUV422, 5 RAW8, 6 RAW10, 7 GENERIC_8BIT, 8 RAW12, 9 RAW14), +0x0c u32
lane count (1–4, else 0), +0x10 u8 DCLK delay (**not used by the HAL**), +0x14 u32 lane bit rate in bit/s, +0x18…+0x28
u32 ×5 lane map for PHY channels 0–4 (values 0–3 kept, else 4), +0x2c…+0x30 u8 ×5 per-channel P/N polarity. Set order:
1. **LPLL** (`__HalMipiTxCsiIfSetLpllConfig`): kbps = rate/1000; loop divider = 2 (100–200 Mbps), 4 (200–400), 8
   (400–800), else 16. SET = ((0x36·2^32 / kbps) · loopdiv · 1000) >> 10 → **1033.48 = SET[15:0], 1033.49 =
   SET[31:16]**. Then by range: 1033.40 = 0x2201, 1033.41 = 0x0420, and 1033.42 / 1033.43 = 0x83/3 (100–200 Mbps),
   0x43/2 (200–400), 0x42/1 (400–800), 0x41/0 (800–1500), followed by 1033.4A = 1, 1033.4B = 0, 1033.44[11:8] = 5;
   outside 100–1500 Mbps only SET is written. These are the panel's LPLL registers with the same per-range values as
   its `LPLLSettingTBL` (§4.5 item 1, §8).
2. **Lane map and polarity** (`HalMipiTxCsiSetLaneSwap`): 1535.03 |= 0x80; for PHY channel i with register R = 1535.06,
   1535.0A, 1535.0E, 1535.22, 1535.25: R[0] = polarity_i & 1, R[14:12] = map_i & 7.
3. **Lane count** (`_HalMipiTxCsiSetLaneNum`): 1536.10[10:8] = lanes (0–4).
4. **Output format** (`_HalMipiTxCsiSetOutputCfg`): by data type — 4 YUV422: CSI data type 0x1E, 1536.0A[2:0] = 4, bytes
   per pixel 2/1; 5 RAW8: 0x2A, 1536.0A = 5, 1/1; 6 RAW10: 0x2B, 5, 5/4; 8 RAW12: 0x2C, 5, 3/2; 9 RAW14: **0x2C**
   (RAW12's code; CSI-2 RAW14 is 0x2D — a bug [R value]), 5, 7/4; any other type: no data-type write and a 0/0 division.
   1536.18[7:0] = data type, 1536.18[15:8] and 1536.19[7:0] = line byte count (= width × bpp, the long-packet word count,
   low / high byte [I field split]); frame bytes (width × height × bpp) → 1536.50 = [15:0], 1536.51 = [23:16], repeated
   in 1536.54 / 1536.55; **1536.28[12:0] = height − 1**; 1536.2A = `gu16MipiTxCsiTransInterval_1` (H-sync blanking,
   `__HalMipiTxCsiSetHSyncBlank`, default 0x78), 1536.2B = `gu16MipiTxCsiTransInterval_2` (long-to-short packet
   blanking, `__HalMipiTxCsiSetLong2ShortPktBlank`, default 0x10); then MAC init (`__HalMipiTxCsiMacInit`) 1536.10 |= 1,
   1536.14 |= 2.
5. **Sync packets** (`_HalMipiTxCsiSetSyncPacket`): 1536.0C = 1, 1536.10 |= 1.

### 7.5 D-PHY timing (query MIPI_TIMING_CFG)
Input: 9 bytes in the order Lpx, ClkHsPrpr, ClkZero, ClkHsPre, ClkHsPost, ClkTrail, HsPrpr, HsZero, HsTrail (§5). First
`HalMipiTxCsiHwPhySetting`: 1535.00 &= ~0xC0, **sleep 100 ms**, 1535.01 = 0, 1535.03 |= 1, 1535.34 |= 0x100,
1535.35 &= 0xFFF0. Then `HalMipiTxCsiHwSetDphyTiming`: 1535.60 [4:0] Lpx, [11:8] ClkHsPrpr; 1535.61 [5:0] ClkZero,
[9:8] ClkHsPre; 1535.62 [4:0] ClkHsPost, [11:8] ClkTrail; 1535.63 [3:0] HsPrpr, [11:8] HsZero; 1535.64 [3:0] HsTrail.
The bank is the panel's DSI D-PHY; the names the panel source gives its registers are in §8.

### 7.6 Frame transfer and interrupt
- **Process** (`MhalMipiTxCsiProcess(ctx, {u8 bTrig; u32 TotalPixel; u32 StartAddr; u32 DataType})`,
  `HalMipiTxCsiHwSetProcess`): bytes = TotalPixel × bpp(DataType) with the table of §7.4 (types other than 4 / 5 / 6 /
  8 / 9 divide 0 by 0); **1536.30 = StartAddr[15:0], 1536.31 = StartAddr[31:16]** (MIU bus address of the frame [I]),
  **1536.32 = bytes[15:0], 1536.33 = bytes[23:16]**, **1536.14[8] = bTrig** (start). One frame per call; the engine
  reads the frame from DRAM and sends it as one CSI-2 frame (FS, lines of the configured word count, FE) [I].
- **IRQ** (SPI 0x56): 1536.70 = mask (1 = masked; 12 sources; `HalMipiTxCsiIrqSetMask` / `HalMipiTxCsiIrqGetMask`),
  1536.72 = clear (write 1 then 0; `HalMipiTxCsiIrqSetClear`), 1536.73 = status (`HalMipiTxCsiIrqGetFlag`,
  `HalMipiTxCsiIrqIsr`). Only **bit 1 = "TX_DONE"** is used. `MHalMipiTxCsiEnableIrq(1)`: unmask bit 1, release the
  clear; `MHalMipiTxCsiEnableIrq(0)`: mask bit 1, pulse its clear. `MHalMipiTxCsiCheckIrq` (from the ISR): reads status
  & ~mask; if bit 1 is set → clears it, releases all clears, returns 1. mi_mipitx counts one done per frame (§6.4).

### 7.7 Debug interface
`/sys/class/<msys>/mmipitx/` (mode 0644): `interval` (show `Interval_1: 0x%x, Interval_2: 0x%x`; store two hex values →
the 1536.2A / 2B blanking values, applied at the next SetParamConfig), `thd` (show `BC: %d, Thd: 0x%x`; store decimal
BcSel + hex threshold, applied only at the next first-instance hardware init), `dbgmg` (debug mask: 0x1 DRV, 0x2 HAL,
0x4 MODULE, 0x8 MHAL, 0x10 IRQ; also forwarded to `MhalMipiTxCsiSetDebugLevel`).

### 7.8 Register banks and what is not known
Banks [R]: 1536 (CSI-2 MAC / packet engine, interrupt), 1535 (D-PHY, shared with PNL, §8), the LPLL
1033.40–4B (shared with PNL), clock-gen 1038.5C, pads 103C.64 and 103E.43–4C, and 1132.01. No source names any field of
1536, the bit layout of 1535.60–64 beyond the field widths, 103E.43–4C or 1132.01: their positions and values are [R],
the names given above are [I] where marked, and their exact meaning is not known — no available source (SDK header,
panel source, M6 register reference) names them.

## 8. Hardware shared by PNL and MIPITX [R unless tagged]

- **LPLL (bank 1033)**: the panel programs it for the LCD pixel clock or the DSI lane rate (§4.5 item 1), MIPITX for the
  CSI-2 lane rate (§7.4 step 1); both write 1033.40–43 / 4A / 4B with the same per-range values and the SET value to
  1033.48 / 49. Only the panel can move its use to the DISP PLL 1037 (§4.5 item 4).
- **MIPI D-PHY (bank 1535)**: the panel's DSI path (`HalPnlInitMipiDsiDphy` 1535.00 / 01 / 04 / 14, channel select and
  polarity 1535.03 / 06 / 0A / 0E / 22 / 25) and MIPITX (power-up 1535.00 / 01 / 03 / 34 / 35, the same lane-map and
  polarity registers, timings 1535.60–64). Register names from the panel source's comments in `hal_pnl.c`
  `HalPnlInitMipiDsiDphy` [S]: 1535.00 bit 0 `reg_pd_ldo`, 1535.01 `reg_pd_dphy` / `reg_pd_ib_dphy`, 1535.03
  `CLKFIX_1X_4DIGTAL` / `re_sw_dphy_cken`, 1535.35[3:0] `reg_clk_dsi_phy`; 1535.04 / 08 / 0C / 20 / 23 lane P/N control,
  1535.06 / 0A / 0E / 22 / 25 per-channel select / polarity.
- **Clock-gen bank 1038**: .5E MIPI-TX-DSI clock (PNL), .5C MIPI-TX-CSI clock (MIPITX).

[I] Because both blocks program the one D-PHY and the one LPLL, a MIPI-DSI panel and the CSI-2 transmitter cannot run at
the same time.

## 9. The board ini files: `/config/board.ini`, `/config/model/Customer.ini` [R]

**Readers: none.** No binary of the dump — rootfs, `/config` (miservice) including config_tool and every module,
`/customer`, the kernel image, U-Boot, the raw NOR image — contains any of their section
or key names (`PanelRelativeSetting`, `m_u8BOARD_PWM_PORT`, `m_pBoardName`, `IniVersion`, `LCD_NAME`,
`m_p480_60PPanelName`, …) or the file names `board.ini` / `Customer.ini` [R string search]; config_tool reads only
`config.ini` and `PQConfig.ini` (`config_tool.md` §4.3). Neither mi_panel, mhal PNL nor config_tool reads them. Both are
leftovers of the MStar TV-style "system ini" scheme — the comment in Customer.ini refers to the MBOOT `dbtable 1`
command, which this U-Boot does not have [I]. Status: UNUSED.

### 9.1 `board.ini` (472 bytes, verbatim)
```
[PanelRelativeSetting]
m_u8BOARD_PWM_PORT                =  2;
m_u16BOARD_LVDS_CONNECT_TYPE      =  0x00;
m_bPANEL_PDP_10BIT                =  1;	#MAPI_FALSE:0, MAPI_TRUE:1
m_bPANEL_SWAP_LVDS_POL            =  0;	#MAPI_FALSE:0, MAPI_TRUE:1
m_bPANEL_SWAP_LVDS_CH             =  0;	#MAPI_FALSE:0, MAPI_TRUE:1
m_bPANEL_CONNECTOR_SWAP_PORT      =  0;	#MAPI_FALSE:0, MAPI_TRUE:1
m_u16LVDS_PN_SWAP_L               =  0x00;
m_u16LVDS_PN_SWAP_H               =  0x00;
#@CRC=0x725D
```
**The trailer `#@CRC=0x725D` is the 16-bit sum of all bytes before it** (the first 460 bytes; byte sum = 0x725D) [R
computed], not a CRC polynomial. The keys correspond to fields of `MhalPnlParamConfig_t` (`u8PDP10BIT`,
`u8SwapLVDS_POL`, `u8SwapLVDS_CH`, `u8SwapPort`, `u16LVDSTxSwapValue` = `PN_SWAP_H<<8 | PN_SWAP_L` per the comment in
`mhal_pnl_datatype.h`) [S header comment; I mapping] — fields this mhal build ignores anyway (§4.5 item 2).

### 9.2 `model/Customer.ini` (2808 bytes)
- `[IniVersion]` `Type = 00` (comment: 00 system ini, 01 panel ini, 02 DLC ini), `Ver = 0x00000001`.
- `[panel]` `m_pPanelName = "/config/panel/DACOUT_1080P_50.ini"` and 26 per-timing names `m_p<res>_<rate><P|I>PanelName`
  (27 `…PanelName` keys in all) [R file]: `m_p480_60P`, `576_50P`, `720_50P`, `720_60P`, `2K1K_24P / 25P / 30P / 50P /
  60P` (1080P), `480_60I`, `576_50I`, `2K1K_50I / 60I` (1080I), `2K2K_24P` (file `DACOUT_2205P_24.ini`), `4K2K_24P / 25P /
  30P / 50P / 60P`, and @60 `768` (1024×768), `1024` (1280×1024), `900` (1440×900), `1200` (1600×1200), `1050`
  (1680×1050), `1366_768`, `1280_800`; each points to `/config/panel/DACOUT_<x>.ini` — **the directory `/config/panel/`
  does not exist** in the image.
- `[board]` `m_pBoardName = "/config/board.ini"`.
- `[LCD]` `LCD_NAME = {FPGA_800x480_TTL}` and six `<name> = "/customer/LCM/<name>.ini"` entries (GM8775C 1920×1080 LVDS
  via a MIPI bridge, RM68200 720×1280 MIPI, three SAT070 TTL panels, FPGA 800×480 TTL).

The same timing names exist as sections of `config.ini` (`DACOUT_*`, `config_tool.md` §4.1), which is what the live system
uses. The `/customer/LCM/` files (five external panels) and `model/LCM.ini` describe external LCD panels and are out of
scope of this specification (external panels, not on this camera); only their existence is recorded.
