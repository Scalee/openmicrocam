# mi_disp-mi_hdmi — mi_disp, mi_hdmi and their user libraries
<!-- ev:mi_disp-mi_hdmi -->

The two MI kernel modules of the display output and the user libraries that drive them. `mi_hdmi.ko` keeps the HDMI
attributes and passes them to mhal's HDMITX block when the output is started; it has no thread, interrupt or timer, and
hot-plug is polled from user space. `mi_disp.ko` (display) is a **mi_sys device driver** with two display devices, four
video layers and their input ports: its ioctls validate and store the configuration, and frames reach the hardware from the
mi_sys task callbacks, paced by the display's vsync interrupt. User side: `libmi_hdmi.so` (with the EDID parser
`MI_HDMI_GetSinkInfo` and the hot-plug poll thread) and `libmi_disp.so`. The camera drives HDMI only: one device (0), one
video layer (0), one input port (0).

Other documents: the user-side ioctl wrapper, payload packing, return codes, error-code format, module ids, the fd lifetime of
every library and the debug level read by the libraries — `mi-user-abi.md`; the kernel-side device frame (`/dev/mi_*`, ioctl
dispatch, per-module procfs and debug parameters, mi_sys exports, mi_sys's proc-command writer) — `mi_sys-2-interfaces.md`;
the registration contract (info record, pass callback table), bind types and capability, task result codes, buffer geometry
and ext-conf, the sideband message, worker threads, CMDQ objects and SRAM — `mi_sys-1-engine.md`; the `MHAL_DISP_*` side
(structures, timing enum, MOP, interrupts, PQ, write-back DMA) — `mhal-disp.md`; the `MhalHdmitx*` side (attribute
structure and enums, DDC and the EDID read, hot-plug in the driver) — `mhal-hdmitx.md`; the SYSCFG panel records, their
timing ids and `/misc/config.ini` — `config_tool.md`; the `MhalPnl*` side and mi_panel — `panel-mipitx.md`; mi_gfx (used by
the 180° rotation) — `gfx-ge.md`; the app's calls and its use of the EDID — `prog_uvc.md` §5.

Tags: **[R]** read from the binary, a decompile, disassembly or a cited file; **[S]** stated by a source; **[I]** inference. A
heading tagged [R] means every statement under it is [R] unless marked otherwise.

Binaries: `/config/modules/4.9.227/mi_hdmi.ko` (15 KB code, 48 functions) and `mi_disp.ko` (207 KB file, 80 KB code, 184
functions), MI build `project_commit.7251436 sdk_commit.80327e1 build_time.20220721`, full ELF symbols, no debug info;
`/config/lib/libmi_hdmi.so`, `libmi_disp.so` (the same code as the static copies linked into prog_uvc). Source paths in
the strings: `alkaid/sdk/interface/src/<mod>/`, `alkaid/sdk/impl/<mod>/`; libmi_hdmi's source file `hdmi_api.c`.
Addresses are Ghidra addresses of those files (= file offset + 0xffd0; a `.text` offset = Ghidra address − 0x10028)
unless written "`.text`" or "file". Three Thumb `tbb/tbh` jump tables of mi_disp were lost by Ghidra and are decoded
from the disassembly (§4.6).

Public headers: skyipc_tiramisu `mi_disp_datatype.h`, `mi_hdmi_datatype.h`; DWARF debug information of 2022 Tiramisu
builds of libmi_disp and libmi_hdmi [S].

## 1. The component on this camera
<!-- ev:mi_disp-mi_hdmi.1 -->

| part | file | role on this camera |
|---|---|---|
| `mi_hdmi.ko` | `/dev/mi_hdmi` | HDMI attributes, start / stop, hot-plug query, EDID read; calls `MhalHdmitx*` |
| `mi_disp.ko` | `/dev/mi_disp` | display device 0 with HDMI interface, video layer 0, input port 0 (= mi_sys DISP dev 0 chn 0); calls `MHAL_DISP_*` |
| `libmi_hdmi.so` | user library | ioctl wrappers, hot-plug poll thread (500 ms), EDID parser `MI_HDMI_GetSinkInfo` |
| `libmi_disp.so` | user library | ioctl wrappers (58 exports) |

- The two modules do not call each other: mi_hdmi exports nothing and imports nothing from mi_disp, and the DISP device's
  HDMI output and mi_hdmi meet only in hardware / mhal [I]. mi_disp alone imports mi_gfx (180° rotation, §4.11) and `MhalPnl*`
  (BT656 / BT1120 panel set-up, §4.6.3).
- What the app reaches (prog_uvc with the shipped command line; its calls and their order are `prog_uvc.md` §5): mi_hdmi
  ioctls 0–7 and 9; mi_disp ioctls 0, 1, 2, 6, 7, 8, 9, 0xa, 0xb, 0x14, 0x16, 0x17, 0x1a, 0x1b, 0x1f, 0x2a; the mi_disp frame
  path for one input port in FREE_RUN sync mode; the vsync interrupt of device 0; MI timings 1080P60 and 3840x2160_30 on both
  modules. Everything else — mi_hdmi ioctls 8, 0xa–0xd and the proc commands; mi_disp device 1, the PIP layers 1 / 3,
  input ports 1–15, Attach / Detach, WBC, rotation, PTS sync, the VGA / CVBS / LCD / panel interfaces and parameters, colour
  temperature, gamma and the proc commands — is **unused** on this camera and specified below as found.
- The module-wide frame shared by every MI kernel module (device registration through mi_common, `open()` / `release()` and
  the wake lock, ioctl dispatch with the 16-byte wrapper — including the `arg == 0` call with no payload that HDMI Init /
  DeInit use —, the module mutex taken by every handler, the per-module procfs `debug_level` / `debug_file` / `debug_func` /
  `module_version_file` and the same-named module parameters) is `mi_sys-2-interfaces.md` §1; here only what each module fills
  in. `modinfo` `depends=` lists mi_common (+ mi_gfx for mi_disp); both modules also import mi_sys exports and mhal, which
  resolve through the load order [R].
- What SigmaStar's MI_DISP (v2.48) and MI_HDMI (v2.18) documentation states for Tiramisu [S]:
  DISP binds to other modules only through `E_MI_SYS_BIND_TYPE_FRAME_BASE`, its layer table has no REALTIME or LOWLATENCY layer
  for Tiramisu, and its input format is only `YUV_SEMIPLANAR_420`; an input port scales up only (at most 16×), never down;
  HDMI and VGA can output at once per device. Hardware structure (Tiramisu figure): DISP0 = MOP0 (16 ports) + MOP1 (PIP) +
  MACE PQ + CSC + OSD mixer, DISP1 = MOP2 + MOP3 + HPQ PQ, one write-back (WBC0); outputs HDMI, VGA, CVBS, TTL, sRGB, MIPI DSI,
  BT.601 / 656 / 1120, MCU through a mux. HDMI TX 1.4, "maximum resolution can only reach 4kp@30fps", no HDCP, no interlaced
  output, no CEC, 8-bit only, output YUV444 / RGB444 (not YUV422); the timing comes from DISP and must match the DISP setting.
  HDMI chip table, Tiramisu column: Y for 1080P 24 / 25 / 30 / 50 / 60, 720P 50 / 60, 576P / 480P, 3840×2160 24 / 25 / 30,
  2560×1440 30 / 50 / 60, the 59.94 / 29.97 variants, 800×600 and a VESA list; N for 720P 24 / 25 / 30, 1080P 23.98, 640×480,
  848×480 and later VESA modes; no 4K 50 / 60 row is Y for any chip.
- The current public documents describe a later ABI than the camera's 2022 libraries [S documents; R libraries]: `MI_DISP_PubAttr_t`
  gains `pu8PanelName` / `u32PanelNameSize` in front (v2.44), `MI_DISP_InputPortAttr_t` gains `eDecompressMode`,
  `MI_DISP_OutputTiming_e` USER moves from 40 to 52, the batch calls are named `…BatchBegin/End` (the libraries export
  `MI_DISP_SetVideoLayerAttrBegin/End`), `MI_HDMI_VideoAttr_t` gains `stUserTiming` and `MI_HDMI_AudioAttr_t` loses
  `eCodeType`; the `HDMI_SUB_VERSION` macro did not change with these struct changes. The pioneer3 (P3) 2022-05 release headers of
  the same SDK generation differ again (`MI_DISP_OutputTiming_e` 4K30 = 30, USER = 32; no `eInColorType`).

## 2. mi_hdmi.ko [R]

### 2.1 Load, device, open and close
- `init_module`: the per-module procfs, `MI_DEVICE_Register` → `/dev/mi_hdmi`. Nothing touches hardware at load.
  `cleanup_module` removes procfs and unregisters. Parameters: only `debug_level` / `debug_file` / `debug_func`.
- **`process_init` = `MI_HDMI_IMPL_Init()`, `process_exit` = `MI_HDMI_IMPL_DeInit()`**: opening `/dev/mi_hdmi` initialises
  the module (ioctl 0's body, §2.3), and **closing any fd of it stops HDMI and destroys the mhal instance** (Stop → Close →
  DeInit). libmi_hdmi closes its fd only in `MI_HDMI_DeInitDev` (§3.1; `mi-user-abi.md` §2.2).
- One device (id 0). No threads, no IRQ, no work queue, no timer. Hot-plug is **polled**: GetAttr asks mhal each time, and the
  library's thread calls GetAttr every 500 ms (§3.2).
- Module "software version" constant 0x210 (printed by `getCap`, §2.6).

### 2.2 State
<!-- ev:mi_disp-mi_hdmi.2.2 -->
All state is one 0x90-byte static struct `hdmi` (`0x162cc`; `.LANCHOR0` + 0x90 · dev) plus `bInited` (`_LANCHOR0`):

| off | field | init value (Init) |
|---|---|---|
| +0x00 | mhal HDMI-TX handle | from `MhalHdmitxCreateInstance(&h, 0)` |
| +0x04 | mi_sys dev handle | `mi_sys_RegisterDev` |
| +0x08 | dev id | 0 |
| +0x0c / +0x0d | bOpened / bStarted | 0 / 0 |
| +0x0e | GCP mute flag — last value sent with `SetMute(h, {mute, 4})` (§2.5); printed as `IsbAvMute` | 1 |
| +0x0f | AV-on flag — last value sent with `SetMute(h, {!on, 3})` | 0 |
| +0x10 | signal-on flag — last value sent with `SetSignal` | 0 |
| +0x14 | `MI_HDMI_Attr_t` (0x30; layout in the DWARF dump; +0 `bConnect`) | bEnableVideo 1, timing 9 (1080_60P), out mode 0 (HDMI), out colour 2 (YCbCr444), in colour 0, deep colour 1, audio on, sample rate 3, bit depth 0x10, PCM, infoframes off |
| +0x44 | colour value set to 0 by the sink check after SetAttr (purpose [I]: "output colour actually used") | 0 |
| +0x48 / +0x6c / +0x78 | last AVI (0x24) / audio (0xc) / SPD (0x18: `au8VendorName[8]`, `au8ProductDescription[16]`) infoframe | 0 |

A call of a mute / signal helper with the value already stored in +0x0e / +0x0f / +0x10 is skipped.

### 2.3 ioctls
User-side numbers and payload sizes §3.1 and `mi-user-abi.md` §3.3. All
handlers take the module mutex; `dev != 0` → `0xA00B2001`; not inited → `0xA00B2015` (NOT_INIT). Bold = used by the app.

| nr | handler → IMPL | what it does | mhal calls | returns |
|---|---|---|---|---|
| **0** | Init (`_IO`, no payload) | if not inited: fill the defaults of §2.2, register the mi_sys device (module id 0xB, no channels) and the proc commands (§2.6), create the mhal instance, `SetDebugLevel(h, 3)`, bInited = 1. Already inited → 0 | `MhalHdmitxCreateInstance(&h, 0)`, `MhalHdmitxSetDebugLevel` | 0; mi_sys failure → `0xA00B201F` |
| **1** | DeInit (`_IO`) | Stop if started, Close if open, destroy the instance, unregister the mi_sys device, zero the state, bInited = 0 | `MhalHdmitxDestroyInstance` | 0 / mhal or mi_sys error as is |
| **2** | Open `{dev}` | bOpened = 1 (no hardware access) | — | 0 |
| **3** | Close `{dev}` | Stop if started; bOpened = 0 | (Stop's) | 0 |
| **4** | SetAttr `{dev, Attr(0x30)}` | needs bOpened (else 0x15, the NOT_INIT number). Stores fields only, with range checks that silently keep the old value: if `bEnableVideo == 1`: deep colour := 24 bit (0), out mode, timing (< 0x30), out colour (< 5), in colour (< 5), the three infoframe enables; if `bEnableAudio == 1`: multichannel, sample rate (< 8), bit depth (< 0x21), code type (< 2). Then a sink check: `GetSinkInfo` type 2 (COLOR_FORMAT); if the sink answers and the timing is not one of the 4K ids 0xd / 0x18 / 0x1a / 0x1c, EDID block 1 byte 3 bits 4–5 (YCbCr support) are read; the result is only logged; +0x44 := 0. **No register programming** — that happens in Start | `MhalHdmitxGetSinkInfo` | 0; NULL attr → bare `3` (not an MI code) |
| **5** | GetAttr `{dev, Attr(out)}` | `GetSinkInfo` type 3 (HPD_STATUS) → hot-plug byte → `Attr.bConnect` (0 if mhal fails); returns the stored Attr | `MhalHdmitxGetSinkInfo` | 0 |
| **6** | Start `{dev}` | needs bOpened; already started → `0xA00B201F`. `SetAttrBegin`; attribute programming (§2.4); `SetAttrEnd`; AV on; signal on; sleep 200 ms; GCP unmute; bStarted = 1 | `SetAttrBegin`, `SetAttr`, `SetAttrEnd`, `SetMute`, `SetSignal` | 0 |
| **7** | Stop `{dev}` | if started: GCP mute, sleep 200 ms, signal off, `SetAttrBegin`, AV off — `SetAttrBegin` is left without `SetAttrEnd` [R disassembly]; bStarted = 0 | `SetMute`, `SetSignal`, `SetAttrBegin` | 0 |
| 8 | SetAvMute `{dev, bool}` (unused) | needs open + started; HDMI mode: mute = GCP mute then AV off, unmute = AV on then GCP unmute; DVI: AV only; + 100 ms after muting | `MhalHdmitxSetMute` | 0 |
| **9** | ForceGetEdid `{dev, Edid(out 0x208)}` | reads EDID blocks 0 and 1 (0x80 B each) into `au8Edid[0..0xff]`, `bEdidValid` = 1, `u32Edidlength` = 0x100; copied out only on success | `MhalHdmitxGetSinkInfo` type 0 (EDID_DATA), block 0 / 1 | 0 / `0xA00B201F` |
| 0xa / 0xb | Set / GetInfoFrame (unused) | type 0 AVI / 1 SPD / 2 audio, stored in §2.2 and sent; other type → `0xA00B2003` | `MhalHdmitxSetInfoFrame` | |
| 0xc | SetAnaLogDrvCur (unused) | passes 11 bytes of drive-current settings (`MhalHdmitxAnaloDrvCurConfig_t`, `mhal-hdmitx.md` §2.2) | `MhalHdmitxSetAnalogDrvCur` | |
| 0xd | SupportTimimg (unused by the app, called by `MI_HDMI_GetSinkInfo`) | `{0, u32 timing, u8 out}`: out = 1 for MI timings 1, 3, 4, 5, 7, 9, 0xa–0xf, 0x11, 0x18, 0x24–0x2a, else 0 | — | 0 |

### 2.4 Programming the attributes (`_mi_Hdmi_SetAttr`, `0x108b0`)
The one place that sends the mode to mhal. It builds a `MhalHdmitxAttrConfig_t` (field names and enums `mhal-hdmitx.md`
§2.2) from the stored Attr through `.rodata` tables [R values; the mhal names are the enums of `mhal-hdmitx.md` §2.2]:
- video enable;
- in colour and out colour, MI 0 RGB444 / 1 YUV422 / 2 YUV444 / 3 YUV420 / 4 AUTO → `{1, 4, 2, 8, 0x10}` = mhal RGB444 /
  YUV422 / YUV444 / YUV420 / AUTO;
- out mode, MI 0 HDMI / 1 HDMI_HDCP / 2 DVI / 3 DVI_HDCP / 4 AUTO → `{2, 3, 0, 1, 4}` = mhal HDMI / HDMI_HDCP / DVI / DVI_HDCP /
  AUTO;
- deep colour, MI 0 24 bit / 1 30 / 2 36 / 3 48 / 4 → `{1, 2, 3, 4, 0}` (mhal 0 = NO_ID); SetAttr has already forced MI 0, so mhal
  gets 1 (24 bit) [I: composition of the two];
- an out-of-range index in these three maps writes `0x1E1` … `0x1E3` (an invalid mhal value; no error is returned);
- timing, MI id → mhal id through a 48-byte table (`.rodata` + 0xc0), index = MI id:
  `0,0,1,1,3,2,7,7,8,8,6,5,4,11,27,28,30,21,30,30,30,30,30,30,10,30,13,9,12,30,30,30,30,30,30,30,14,18,16,20,17,15,19,26,25,24,23,29`
  (30 = 0x1e = mhal's MAX, i.e. unsupported). Read against the mhal timing enum (`mhal-hdmitx.md` §2.2) and the MI names of
  §2.6 [R both; I the pairing]:

  | MI | MI name (§2.6) | mhal | mhal name |
  |---|---|---|---|
  | 0, 1 | —, 720x480P@60 | 0 | 720x480p60 |
  | 2, 3 | —, 720x576P@50 | 1 | 720x576p50 |
  | 4 / 5 | 1280x720P@50 / @60 | 3 / 2 | 1280x720p50 / p60 |
  | 6, 7 | —, 1920x1080P@50 | 7 | 1920x1080p50 |
  | 8, **9** | —, **1920x1080P@60** | **8** | 1920x1080p60 |
  | 10 / 11 / 12 | 1920x1080P@30 / @25 / @24 | 6 / 5 / 4 | 1920x1080p30 / p25 / p24 |
  | **13** | **4k@30** | **0xb** | 3840x2160p30 |
  | 14 / 15 / 17 | 2560x1440@50 / @60 / @30 | 0x1b / 0x1c / 0x15 | 2560x1440p50 / p60 / p30 |
  | 24 | 4k@25 | 0xa | 3840x2160p25 |
  | 26 / 27 / 28 | — (EDID table T1 of §3.4: 26 = 2160p60) | 0xd / 9 / 0xc | 3840x2160p60 / p24 / p50 |
  | 36 / 37 / 38 / 39 / 40 / 41 / 42 | 1024x768 / 1280x1024 / 1440x900 / 1600x1200 / 1280x800 / 1366x768 / 1680x1050 @60 | 0xe / 0x12 / 0x10 / 0x14 / 0x11 / 0xf / 0x13 | the same |
  | 43 / 44 | 1920x1080@59.94 / @29.97 | 0x1a / 0x19 | the same |
  | 45 / 46 | 1280x720@59.94 / @29.97 | 0x18 / 0x17 | the same |
  | 47 | 3840x2160@29.97 | 0x1d | 3840x2160p29.97 |
  | 16, 18–23, 25, 29–35 | — | 0x1e | unsupported |
- audio enable, sample rate (an index > 7 writes `0xA00B21E5` into the struct — a vendor bug), multichannel, the NON_PCM flag;
- `MhalHdmitxSetAttr(h, &attr)`.

### 2.5 Helpers towards mhal: mute, signal, sink info
- Signal: `MhalHdmitxSetSignal(h, &bool)`.
- Mute: `MhalHdmitxSetMute(h, {bool mute, flags})` with the mhal flags VIDEO 1, AUDIO 2, AVMUTE 4: **"AV on / off" sends flags
  3** (video + audio), **"GCP mute / unmute" sends 4** (AVMUTE, the General Control Packet mute) [R values; names from mhal].
- `MhalHdmitxGetSinkInfo(h, {u32 type, u8 block, data…})`: type 0 EDID_DATA = one EDID block (0x80 B from +5); type 2
  COLOR_FORMAT = the sink's colour capability at +8, a bit mask 1 RGB / 2 YUV444 / 4 YUV422 / 8 YUV420; type 3 HPD_STATUS =
  hot-plug byte at +4. How mhal answers (DDC read, EDID check, hot-plug timer) is `mhal-hdmitx.md` §5, §6.

### 2.6 procfs [R `hdmi_proc_register` (0x1152c) (registration), `_MI_HDMI_IMPL_Cmd*`, `_MI_HDMI_IMPL_GetCapability`, `mi_hdmi_OnHelp`]
File `/proc/mi_modules/mi_hdmi/mi_hdmi0`, created at Init by registering a mi_sys device `{module 0xB, dev = the stored dev id
(0)}` with the dump callback `_MI_HDMI_IMPL_OnDumpDevAttr` and help `mi_hdmi_OnHelp` (no channels; registration failure → Init
returns `0xA00B201F`). Commands are registered with `mi_sys_RegistCommand`; the registered number is the maximum number of
arguments after the command word, and mi_sys's proc writer refuses more before the handler runs (`mi_sys-2-interfaces.md` §10.3).
Unless noted a command checks: module inited (else `0xA00B2015`), proc dev id 0 (else `0xA00B2001`). All are diagnostics.

| command (max args) | arguments | effect |
|---|---|---|
| `cat mi_hdmi0` = `getCap` (0) | — | prints `Hdmi Module swVerion = 0x210`; **then calls `MI_HDMI_IMPL_SetAttr(dev, stored attr)`** (reading the file re-runs the SetAttr store + sink check of §2.3, and fails silently if not opened); then a status table `InitFlag` (Init / UnInit), `bHdmiTxRuning` (Open / Close = opened), `PowerOnOff` (started), `bSignalOnOff` (signal), `IsbAvMute` (GCP mute flag → Mute / UnMute), `bAvOnOff` (**also printed from the GCP mute flag**, not from the AV flag); video `EnableVideo`, `TimingType` (name table below), `OutputMode` (HDMI, HDMI_HDCP, DVI, DVI_HDCP, AUTO), `InColor` / `OutColor` (RGB444, YUV422, YUV444, YUV420, AUTO), `DeepColorMode` (24Bit, 30Bit, 36Bit, 48Bit); audio `EnableAudio`, `IsMultiChannel`, `BitDepth` (8 / 16 / 18 / 20 / 24 / 32Bit for 8 … 32), `CodeType` (PCM / NON_PCM), `SampleRate` (32K, 44K, 48K, 88K, 96K, 176K, "196K" for 1 … 7); `bEnableAviInfoFrame`, `bEnableAudInfoFrame`, `bEnableSpdInfoFrame`. No inited / dev check |
| `getEdid` (1) | block (decimal, low byte used) | `MhalHdmitxGetSinkInfo({type 0, block})` → hex dump of 0x80 bytes in rows of 16 (`%02xh:` offsets), then `u32Sum: %#x, check OK/NOK` (byte sum mod 256 = 0). mhal failure → `0xA00B201F`; < 1 argument → `0x1F` |
| `setHdmiMode` (1) | `hdmi` \| `dvi` | **no inited check**: GCP mute, signal off, AV off, `SetAttrBegin`, output mode := 0 HDMI / 2 DVI (other words leave it), the full attribute programming of §2.4 (`_mi_Hdmi_SetAttr`), `SetAttrEnd`, signal on, AV on, GCP unmute — without the 200 ms pauses of Start; 0 |
| `setSignal` (1) | `On` \| `Off` | signal helper (`MhalHdmitxSetSignal`, skipped when unchanged) without any open / started check; returns its result; other word → `0x1F` |
| `setMute` (1) | `On` \| `Off` | `MI_HDMI_IMPL_SetAvMute(0, 1 / 0)` = ioctl 8 (needs opened + started) |
| `getInfoFrame` (1) | `avi` \| `audio` \| `spd` | prints the stored infoframe of §2.2: AVI `bEnableAfdOverWrite`, `A0Value`, `eColorType`, `eColorimetry`, `eExtColorimetry`, `eYccQuantRange`, `eTimingType`, `eAfdRatio`, `eScanInfo`, `eAspectRatio`; audio `u32ChannelCount`, `eAudioCodeType`, `eSampleRate`; SPD the 8 vendor-name and 16 product-description bytes. No inited check |
| `setInfoFrame` (3) | `avi` field value / `audio` field value / `spd` field index value | starts from the stored frame, replaces one field (AVI fields as above; audio `u32ChannelCount`, `eAudioCodeType`, `eSampleRate`), then `MI_HDMI_IMPL_SetInfoFrame(0, frame)` = ioctl 0xa; unknown field → `0x1F`. The `spd` form is below |
| `Timing` (2) | `list` | prints `Id : name` for the 21 timings `MI_HDMI_SupportTimimg` accepts (1, 3, 4, 5, 7, 9–15, 17, 24, 36–42) |
| | `change <id>` | id in that set → a copy of the stored attr with the new timing: `MI_HDMI_IMPL_Stop` → `SetAttr` → `Start` (the full Stop / Start of §2.3), "Change To Timing : %s"; other id → "Not Support Such Timing Id"; no inited / dev check |

**`setInfoFrame spd`** [R `_MI_HDMI_IMPL_CmdSetInfoFrame` 0x136b4; disassembly of file `.text` 0x368c…0x3916]: the frame is
built on the stack — sp+12 u32 type (1 = SPD), sp+16…+0x27 the stored SPD record copied from `.LANCHOR0` + 0x90 · dev + 0x80
(`ldmia/stmia` of 0x18 bytes, 0x371c…0x373e). The handler's argument count (command word included) must be 5 for `spd` (else
0x1F, 0x3730 / 0x3788); index = argv[3] (`simple_strtoul` base 10, low 16 bits), value = argv[4] (low byte).
`au8VendorName` (0x3866): `index − 1 > 6` → 0x1F; else `strb value, [sp + 16 + index]` → vendor byte *index* (1…7; byte 0
cannot be written). `au8ProductDescription` (0x3772…0x3786): `strcmp` = 0, `subs r5, #1; cmp r5, #15; bls 0x36fa` → straight to
`MI_HDMI_IMPL_SetInfoFrame(0, &frame)` (0x36fa) — **no instruction writes a product-description byte**; the branch only
range-checks the index (1…16, else 0x1F) and re-sends the stored SPD frame. Any other field name → 0x1F. The whole `spd` form
needs four arguments after the command word, so **mi_sys always refuses it** (maximum 3); it cannot run.

`TimingType` names (MI_HDMI timing id → text, `.rodata` 0x13d8c, index id − 1): 1 720x480P@60, 3 720x576P@50, 4 1280x720P@50,
5 1280x720P@60, 7 1920x1080P@50, 9 1920x1080P@60, 10 …P@30, 11 …P@25, 12 …P@24, 13 4k@30, 14 2560x1440@50, 15 2560x1440@60,
17 2560x1440@30, 24 4k@25, 36 1024x768@60, 37 1280x1024@60, 38 1440x900@60, 39 1600x1200@60, 40 1280x800@60, 41 1366x768@60,
42 1680x1050@60, 43 1920x1080@59.94, 44 1920x1080@29.97, 45 1280x720@59.94, 46 1280x720@29.97, 47 3840x2160@29.97; every
other id `UnKown` (id 0 and ≥ 0x30 print "UnKown" by range check). The MI ids are this build's `MI_HDMI_TimingType_e`
(`sgs_docs` `D3_hdmi.txt`) [S].

### 2.7 Imports and exports
- mhal (11): `MhalHdmitxCreateInstance`, `MhalHdmitxDestroyInstance`, `MhalHdmitxSetDebugLevel`, `MhalHdmitxSetAttrBegin`,
  `MhalHdmitxSetAttr`, `MhalHdmitxSetAttrEnd`, `MhalHdmitxSetSignal`, `MhalHdmitxSetMute`, `MhalHdmitxGetSinkInfo`,
  `MhalHdmitxSetInfoFrame`, `MhalHdmitxSetAnalogDrvCur`.
- mi_sys: `mi_sys_RegisterDev`, `mi_sys_UnRegisterDev`, `mi_sys_RegistCommand`, logging (`MI_SYS_LOG_IMPL_PrintLog`,
  `MI_COMMON_Print`); mi_common: the device frame; CamOs: `CamOsMsSleep`, `CamOsStrtoul`.
- Exports: none (`MI_HDMI_SupportTimimg` and the IMPL functions are global symbols but not in `__ksymtab`).
- `SetDebugLevel(h, 3)` at Init sets the mhal HDMI-TX driver's debug level (`mhal-hdmitx.md` §2.1); whether level 3 puts the
  MDrv HDMI prints into the kernel log is not known (§7).

## 3. libmi_hdmi.so [R]

### 3.1 Exports → ioctls
16 exports. The wrappers follow the packing of `mi-user-abi.md` §3.2, do no argument checks in user space unless a row says
so, and copy a Get result back only when the ioctl returned 0.

| function | nr (payload size) | payload / behaviour |
|---|---|---|
| `MI_HDMI_Init(MI_HDMI_InitParam_t *p)` (8 B: `pfnHdmiEventCallback`, `pCallBackArgs`) | 0 (`_IO`, arg 0) | §3.2 |
| `MI_HDMI_DeInit()` | 1 (`_IO`, arg 0) | |
| `MI_HDMI_InitDev(MI_HDMI_InitParam_t *p)` / `MI_HDMI_DeInitDev()` | 0 / 1 | = `MI_HDMI_Init` / `DeInit` plus the fd open / close: InitDev opens the fd (returns **0x1F**, a bare number, when the open fails); DeInitDev closes the fd unconditionally (no create count), clears the thread-running flag and the stored callback (`mi-user-abi.md` §2.2) |
| `MI_HDMI_Open(dev)` / `MI_HDMI_Close(dev)` | 2 / 3 | `{dev}` |
| `MI_HDMI_SetAttr(dev, MI_HDMI_Attr_t *a)` | 4 (0x34) | `{dev, Attr (0x30)}`; the timing is at payload +0xc |
| `MI_HDMI_GetAttr(dev, *a)` | 5 (0x34, ioctl `0xc0346905`) | `{dev, Attr}`; `Attr.bConnect` is the first field |
| `MI_HDMI_Start(dev)` / `MI_HDMI_Stop(dev)` | 6 / 7 | `{dev}` |
| `MI_HDMI_SetAvMute(dev, bool)` | 8 (8) | `{dev, u8}` |
| `MI_HDMI_ForceGetEdid(dev, MI_HDMI_Edid_t *e)` | 9 (0x20c, ioctl `0xC20C6909`) | `{dev, MI_HDMI_Edid_t (0x208)}`: `bEdidValid`, `u32Edidlength`, `au8Edid[]` |
| `MI_HDMI_SetInfoFrame(dev, MI_HDMI_InfoFrame_t *f)` | 0xa (0x2c) | `{dev, f (0x28)}` |
| `MI_HDMI_GetInfoFrame(dev, type, *f)` | 0xb (0x30) | NULL → **0xA00B2003**; `{dev, type}`; 0x28 back on 0 |
| `MI_HDMI_SetAnalogDrvCurrent(dev, MI_HDMI_AnalogDrvCurrent_t *c)` | 0xc (0x10) | `{dev, 2 words, u16, u8}` |
| `MI_HDMI_GetSinkInfo(dev, MI_HDMI_SinkInfo_t *s)` | 5, 9, 0xd | the EDID parser, §3.3–§3.7 |

ioctl 0xd (`MI_HDMI_SupportTimimg`, `_IOWR` 0xc, ioctl `0xC00C690D`, `{0, u32 timing, u8 supported}`) has no export of its own;
`MI_HDMI_GetSinkInfo` sends it. The library's error prints are gated on `_MI_PRINT_GetDebugLevel("hdmi")` = the number in
`/proc/mi_modules/mi_hdmi/debug_level` (`mi-user-abi.md` §6): ≥ 1 errors, ≥ 5 info such as "Thread alrady running...".

### 3.2 Init and the hot-plug poll thread
`MI_HDMI_Init(pstInitParam)` sends hdmi ioctl 0 (`_IO`, arg 0). On success, and once per process, it stores
`pfnHdmiEventCallback` and starts a **detached thread** that calls `MI_HDMI_GetAttr(0)` every 500 ms and invokes
`cb(0, bConnect != 1, 0, pCallBackArgs)` whenever `bConnect` changes. Hot-plug detection is therefore user-space polling of
GetAttr, which asks mhal's hot-plug state each time (§2.3 nr 5); the kernel needs no event mechanism for it. How a
monitor's input switch looks through it: §6.

### 3.3 `MI_HDMI_GetSinkInfo` — the EDID parser: functions and output struct
13.7 KB of code. Helpers inlined into it (names from the strings): `_MI_HDMI_PraseEdid`, `_MI_HDMI_CheckHeader`,
`_MI_HDMI_ParseEdidBlock0`, `_MI_HDMI_ParseEdidBlock1`, `_MI_HDMI_ParseVendorInfo`, `_MI_HDMI_ParseEstablishTiming`,
`_MI_HDMI_ParseStdTiming`, `_MI_HDMI_ParseBlock0DtdTiming`, `_MI_HDMI_ParseAudioDataBlock`, `_MI_HDMI_ParseVideoDataBlock`,
`_MI_HDMI_ParseVendorSpecDataBlock`, `_MI_HDMI_ParseSpeakerDataBlock`, `_MI_HDMI_ParseExtDataBlock`.

Output `MI_HDMI_SinkInfo_t` (0xa8; header `skyipc_tiramisu/soc_sdk_include_mi_hdmi_datatype.h`, lines 318–362). Offsets
derived from the header with `E_MI_HDMI_TIMING_MAX = 0x30`, `MI_HDMI_MAX_ACAP_CNT = 14`, `MI_HDMI_MAX_AUDIO_SAMPLE_RATE_CNT = 9`
— the counts that make the binary's stores land on the header fields [I counts, R stores]:

| off | field | off | field |
|---|---|---|---|
| +0 | `bConnected` | +0x80 | `u32IdSerialNumber` |
| +1 | `bSupportHdmi` | +0x84 | `u32WeekOfManufacture` |
| +4 | `eNativeTimingType` | +0x88 | `u32YearOfManufacture` |
| +8 | `abVideoFmtSupported[48]` | +0x8c / +0x8d | `u8Version` / `u8Revision` |
| +0x38 / +0x39 / +0x3a | `bSupportYCbCr444` / …422 / …YCbCr | +0x8e | `u8EdidExternBlockNum` |
| +0x3b / +0x3c | `xvYcc601` / `xvYcc709` | +0x8f | `au8IeeRegId[3]` |
| +0x3d | `u8MdBit` | +0x92 … +0x95 | `u8PhyAddr_A` … `_D` |
| +0x3e | `abAudioFmtSupported[14]` | +0x96 | `bSupportDviDual` |
| +0x4c | `au32AudioSampleRateSupported[9]` | +0x97 / +0x98 / +0x99 / +0x9a | DeepColorYcbcr444 / DeepColor30Bit / 36Bit / 48Bit |
| +0x70 | `u32MaxPcmChannels` | +0x9b | `bSupportAi` |
| +0x74 | `u8Speaker` | +0x9c | u32 MaxTmdsClock |
| +0x75 | `au8IdManufactureName[4]` | +0xa0 / +0xa1 / +0xa2 | `bILatencyFieldsPresent` / `bLatencyFieldsPresent` / `bHdmiVideoPresent` |
| +0x7c | `u32IdProductCode` | +0xa3 / +0xa4 / +0xa5 / +0xa6 | `u8VideoLatency` / `u8AudioLatency` / `u8InterlacedVideoLatency` / `u8InterlacedAudioLatency` |

### 3.4 Tables in the binary
<!-- ev:mi_disp-mi_hdmi.3.4 -->
- **T1** (`.data` 0x16540, 63 × 40 B): `{u32 code; u32 eTiming; char *name; u32 hActive, vActive, interlaced, hTotal, vTotal,
  refreshHz, pixelClock_kHz}`. `code`: CEA VIC 0x01–0x66 (VICs 1–7, 0x10–0x16, 0x1f–0x22, 0x5d–0x66), **0x1000 + bit** for the
  24 established-timing bits (only 0x1000–0x100f and 0x1017 listed), 0x2000–0x200e VESA DMT modes, 0x2010 "none". `eTiming` =
  the `MI_HDMI_TimingType_e` value (e.g. VIC 16 1080p60 → 9, VIC 97 2160p60 → 26, 1024×768@60 → 36), **48 for everything the
  driver cannot output** (= `E_MI_HDMI_TIMING_MAX`).
- **T2** (`.rodata` 0x5e1c, 35 × 24 B): `{u32 code; u8 dtd[18]; pad}` — the first 12 bytes of a Detailed Timing Descriptor
  (pixel clock, active / blank, sync offsets / widths and their high bits) for each T1 timing it can recognise; a DTD is
  recognised only if **all 12 bytes match** (sizes and polarity bytes 12–17 are ignored).
- Lookups: `code → T1 row → eTiming` (code not in T1 → index 48); **flag set = `abVideoFmtSupported[eTiming] = 1`**.

### 3.5 Algorithm [R: decompile; branches marked "disassembly" checked in]
<!-- ev:mi_disp-mi_hdmi.3.5 -->
1. `pstSinkInfo == NULL` → **0xA00B2003**. `memset(info, 0, 0xa8)`; `MI_HDMI_GetAttr(0)` → `bConnected` (an ioctl failure
   keeps 0); not connected → return 0.
2. hdmi ioctl **9** (`_IOWR` 0x20c `{dev, MI_HDMI_Edid_t}`, the same as ForceGetEdid); −1 or non-zero → "Get Edid fail",
   **0xA00B21E8**. `bEdidValid == 0` or `u32Edidlength == 0` → return 0 (only `bConnected` filled).
3. **Block 0**: the header must be `00 FF FF FF FF FF FF 00` (else failure → 0xA00B21E8, after `_MI_HDMI_CheckHeader` prints
   the first differing index). `u8Version`, `u8Revision` = bytes 0x12, 0x13; `u8EdidExternBlockNum = min(byte 0x7e, 3)`.
   Manufacturer: bytes 8–9 big-endian, three 5-bit letters, each must be 1..26 (else failure) → `'@' + letter` at
   +0x75..+0x77. Product code = u16 at 0x0a, serial = u32 at 0x0c, week = byte 0x10, **year = byte 0x11 + 1990** (disassembly
   0x17cc).
4. Established timings: each set bit *i* (0..23) of bytes 0x23–0x25 (bit 0 = LSB of 0x23) → code 0x1000 + *i* → flag.
5. Standard timings (8 × 2 B at 0x26): `01 01` = unused; hActive = (b0 + 31) · 8; aspect b1 bits 7–6: 0 16:10, 1 4:3, 2 5:4,
   3 16:9 → vActive; refresh = (b1 & 0x3f) + 60. **Only 60 Hz entries count**: the first T1 row with that hActive, vActive and
   60 Hz gives the code → flag. Other refresh rates are ignored.
6. Four 18-byte descriptors at 0x36 / 0x48 / 0x5a / 0x6c: pixel clock ≠ 0 → DTD → T2 match → code; the **first** descriptor
   also sets `eNativeTimingType = eTiming` (only when matched; unmatched leaves 0). Display descriptors (bytes 0–1 = 0, byte
   2 = 0, tag = byte 3): 0xFC monitor name (copied to a local, printed at level ≥ 5, dropped), 0xFF serial (printed, dropped),
   **0xFD range limits** — bytes 5–10 are read only as print arguments (§3.7): no store into the struct, no local kept, no
   effect on the return value; byte 4 (EDID 1.4 rate-offset flags) and bytes 11–17 (secondary GTF / CVT parameters) are not
   read. Tag 0x00 (dummy) runs through the DTD matcher and matches nothing; a descriptor with pixel clock 0 but byte 2 ≠ 0, and
   other tags, are ignored.
7. The block-0 checksum is computed only for a print: the printed value is the byte that byte 127 should hold,
   `(−Σ bytes 0–126) & 0xFF`; it is not compared with byte 127, and a bad checksum is **not** rejected.
8. **Block 1** — the checks run **unconditionally** after block 0 (no test of the extension count, byte 0x7E) [R disassembly
   0x2078–0x20b0, 0x4836–0x494e]: EDID byte 0x80 ≠ 2 (not CEA-861), revision byte 0x81 ≤ 2, DTD offset *d* = byte 0x82 = 0, or
   *d* ≤ 4 — each branches to 0x4844 and makes the call return **0xA00B21E8** (§3.7: ERR 0x44D / 0x452 / 0x458 / 0x45F → 0x4BA →
   0x1C40 → 0x56A), without step 9. Otherwise byte 0x83 bit 5 → `bSupportYCbCr444`, bit 4 → `bSupportYCbCr422`, and the data blocks
   from 0x84 to 0x80 + *d* are parsed, header byte = tag (bits 7–5) / length (bits 4–0):
   - **tag 1 Audio**: each 3-byte SAD; format (b0 bits 6–3) 1 LPCM → `u32MaxPcmChannels = b0 & 7` (the raw "channels − 1"
     field) and +0x3e = 1; 2 AC-3 → +0x3f; 3 MPEG-1 → +0x40; 4 MP3 → +0x41; 5 MPEG-2 → +0x42; 6 AAC → +0x43; 7 DTS → +0x44;
     8–15 ignored. The sample-rate byte b1 is tested as an **else-if chain**, so only the lowest set bit is recorded per SAD:
     bit 0 → [0] = 32000, bit 1 → [1] = 44100, bit 2 → [2] = 48000, bit 3 → [3] = 88200, bit 4 → [4] = 96000, bit 5 → [5] =
     176400, bit 6 → [6] = 192000. The sample-size / bit-rate byte b2 is unused.
   - **tag 2 Video**: each SVD: VIC = b & 0x7f when that is 1..64, else the whole byte (VIC ≥ 65) → T1 → flag. The native bit
     (0x80) is ignored; SVDs never set `eNativeTimingType`.
   - **tag 3 Vendor-specific**: OUI bytes 1–3 → `au8IeeRegId`; OUI `03 0C 00` (HDMI LLC) → `bSupportHdmi = 1`, otherwise 0
     ("DVI device, not parsed" — this applies only to the rest of this block; every other part of the EDID is parsed for DVI
     sinks too). If length ≥ 4 (test `(hdr & 0x1c) != 0`): physical address A = b4 >> 4, B = b4 & 0xf, C = b5 >> 4,
     D = b5 & 0xf. Length ≥ 6: b6 bit 0 DVI_Dual, bit 3 DC_Y444, bit 4 DC_30, bit 5 DC_36, bit 6 DC_48, bit 7 Supports_AI.
     Length ≥ 7: `u8MaxTmdsClock = b7 · 5` (MHz). Length ≥ 8: b8 bit 5 HDMI_Video_present, bit 7 Latency_Fields_present, bit 6
     I_Latency_Fields_present (b8 bits 3–0, the content types, are printed only). Length ≥ 9: +0xa3 = b9, +0xa4 = b10,
     **+0xa5 = b12** (b11 skipped, +0xa6 never written) — read whether or not the presence bits are set.
   - **tag 4 Speaker allocation**: `u8Speaker = 1` if either payload byte ≠ 0 (a flag, not the mask).
   - tag 5 (VESA DTC), 6, 0: ignored. **tag 7 extended**: ext tag 5 Colorimetry → b2 bit 0 xvYCC601, bit 1 xvYCC709; ext tags
     0–4, 0x10–0x12 recognised and ignored.
   - CEA DTDs: count = ((256 − *d*) & 0xff) / 18, each matched against T2 → flag. **The library reads them from EDID offset
     0x100 + *d* instead of 0x80 + *d*** (base register = EDID struct + (*d* ^ 0x80) + 0x88; disassembly 0x2198–0x21b6), i.e.
     from a third 128-byte block, which a 2-block EDID leaves zero — so in practice block-1 DTDs are never recognised.
9. **Driver capability pass**: for the native timing and then for each of the 48 flag slots, hdmi ioctl **0x0d SupportTimimg**
   (answer per §2.3: 1 for timings 1, 3, 4, 5, 7, 9, 0xa–0xf, 0x11, 0x18, 0x24–0x2a). Answer 0 with ret 0 → the flag is cleared
   (native: `eNativeTimingType = 48`). A slot whose flag is not set re-sends the previous timing (harmless). **Return value =
   0 if the last of these ioctls returned 0, else 0xA00B21E8.**

### 3.6 Consequences and quirks
<!-- ev:mi_disp-mi_hdmi.3.6 -->
- **`abVideoFmtSupported[48]` does not exist**: every code that maps to eTiming 48 (most established and DMT timings, unknown
  VICs, unmatched DTDs) writes 1 into `info[0x38]` = **`bSupportYCbCr444`**. Block 1 byte 3 overwrites it later (on the
  success path of step 8), and SVDs parsed after that can set it again. So `bSupportYCbCr444` is unreliable.
- On the failure paths of step 8 the caller is left with `bConnected`, the block-0 fields (manufacturer, product, version,
  established / standard / DTD flags) and `eNativeTimingType` = the T2 match of the first DTD (or 0), not checked against the
  driver [R stores; I summary].
- Return values: 0 / 0xA00B21E8 / 0xA00B2003. 0xA00B21E8 covers every EDID without a CEA-861 revision ≥ 3 extension with data
  blocks: a sink with a single-block EDID (DVI monitors; the extension bytes are whatever the kernel left after byte 127), a
  CEA extension of revision 1–2, or a CEA block with *d* ≤ 4 [I from step 8]; only a CEA-861 revision ≥ 3 extension with data
  blocks lets the call return 0.
- `eNativeTimingType` is 9 (1080p60) only when the first block-0 DTD is byte-identical (bytes 0–11) to the CEA 1080p60 DTD in
  T2 and the driver supports it; every other monitor yields its own id or 48 [I from steps 6 and 9].
- What prog_uvc uses [R]: the return value (stored as `flag_4_3`),
  `bConnected` and `eNativeTimingType != E_MI_HDMI_TIMING_1080_60P (9)`; it parses the raw EDID from `MI_HDMI_ForceGetEdid`
  itself (the range-limits descriptor and the first DTD's horizontal active) and does not depend on step 6's 0xFD branch
  (`prog_uvc.md` §5.1). Its "native ≠ 9" test therefore fires for any monitor whose preferred mode is not exactly CEA
  1080p60, and its behaviour differs between the monitors of the previous bullet and ordinary HDMI TVs [I]. The rest of the
  struct is read by nothing on this camera.

### 3.7 Debug prints of `MI_HDMI_GetSinkInfo` [R]
<!-- ev:mi_disp-mi_hdmi.3.7 -->
106 print sites; source line numbers are the `__LINE__` arguments (file `hdmi_api.c`). Three kinds, gated per call on
`_MI_PRINT_GetDebugLevel("hdmi")`, read again before every print:
- **ERR** (12 sites, level ≥ 1): red `"\x1b[1;31m[MI ERR ]: %s[%d]: "` (function, line) + message + `"\x1b[0m"`.
- **INFO** (92 sites, level ≥ 5): green `"\x1b[1;32m[MI INFO]:%s[%d]: "` + message + `"\x1b[0m"`.
- **RAW** (2 sites, always): `"[%s] failed to ioctl 0x%08lx!(%s)\n"` with `"hdmi"`, the command and `strerror(errno)` when
  `ioctl` returns −1 — for 0xC20C6909 (GetEdid, nr 9) and 0xC00C690D (SupportTiming, nr 0x0d).

No print changes control flow; the values printed are computed only for the print except where noted. All INFO paths are
**unused** on the camera unless `debug_level` ≥ 5 is written to the proc file; the ERR paths print at any non-zero level.

| helper | ERR (level ≥ 1) | INFO (level ≥ 5) |
|---|---|---|
| `MI_HDMI_GetSinkInfo` | 0x54B `"MI_ERR_INVALID_PARAMETER!"` (NULL info); 0x571 `"[hdmi_api.c][0x571]Get Edid fail!"` (ioctl 9 failed or ≠ 0); 0x56A `"…Prase Edid fail"` (parser failed) | 0x566 `"_MI_HDMI_PraseEdid.OK.."` at the end |
| `_MI_HDMI_PraseEdid` | `"[%s][%d]exec function failed"`: 0x4B9 block 0 failed (bad header or manufacturer letter), 0x4BA block 1 failed (any of the four block-1 ERRs below), 0x4BB the last SupportTiming ioctl of the driver pass returned ≠ 0 | 0x4BC `"Platform Support Timing: "`, 0x4BD `"\t NAtive Timing: %s \n"` (T1 name of `eNativeTimingType`, `"none"` if no row), 0x4C2 `"\t Timing: %s \n"` once per set `abVideoFmtSupported` flag after the driver pass |
| `_MI_HDMI_CheckHeader` | 0x1E6 `"Index:%d, 0x%02x\n"` (first header byte that differs) | — |
| `_MI_HDMI_ParseEdidBlock0` | 0x437 `"…exec function failed"` (vendor info failed) | 0x42B `"Start Parse EDID Block0"`; 0x434 `"EDID Struct Version: %d , Revision: %d , ExtBlockNum: %d\n"`; 0x441 `"Checksum: EDID Block0 checksum: %X\n"` = `(−Σ bytes 0–126) & 0xFF` |
| `_MI_HDMI_ParseVendorInfo` | — | 0x200 `"Can't parse manufacture name"` (letter outside 1–26, then the failure); 0x20B–0x20E manufacturer name, `"code: %x"`, `"serial: %d"`, `"year:%d ,week:%d"` (labels swapped: prints the week as year and the year as week) |
| `_MI_HDMI_ParseEstablishTiming` | — | 0x241 `"Established Timings"`, 0x242 `"\tEstablished Timings I"`, 0x24B `"\t\t %s \n"` T1 name per set bit (`"none"` if not in T1) |
| `_MI_HDMI_ParseStdTiming` | — | 0x258 `"\tStandard Timing %d\n"`, 0x25B `"\t\t Unused"`, 0x262 `"Horizontal active pixels: %d"`, 0x266–0x269 aspect `16:10` / `4:3` / `5:4` / `16:9`, 0x26C `"Refresh Rate: %d Hz"` |
| `_MI_HDMI_ParseBlock0DtdTiming` | — | 0x2E6 `"Block0 Detailed Timing Descriptor %d\n"`; for a DTD (0x2EC, 0x2EE): `"\tDTD support timing:%s \n"` (T1 name or `"none"`), `"\tMode \t\"%dx%d\"\n"` (H / V active), `"\t# vfreq %3.3fHz, hfreq %6.3fkHz\n"` = `clk·10000 / (Htotal·Vtotal)` and `clk·10000 / (Htotal·1000)`, `"\t\t DotClock\t%f\n"` (`clk·10000` Hz), `"HTimings\tHActive: %u HTotal:%u"`, `"VTimings\tVActive: %u VTotal:%u"` (total = active + blanking), `"\t\t Sync Profile\t\"Interlace\" "` if byte 17 bit 7, `"\tEndMode"`; 0x2FD–0x308 the 0xFD descriptor (table below); 0x2FA `"\tMonitor name: %s\n"` (tag 0xFC: up to 13 characters, cut at `'\n'`); 0x30C `"\tProduct Serial Number: "` then one INFO print per character via `putchar` (tag 0xFF, up to `'\n'` or byte 17) |
| `_MI_HDMI_ParseEdidBlock1` | 0x44D / 0x452 `"Extern block version err!"` (extension tag ≠ 2 / revision < 3); 0x458 `"No detailed timing data!"` (*d* = 0); 0x45F `"No reserved provided! len:%d\n"` (*d* ≤ 4, prints *d*) | 0x44A `"Start Parse EDID Block1"`; 0x469 `"TAG:0x%x\n"` (whole header byte per data block); 0x479 `"VESA_DTC parase"` (tag 5); 0x47F `"resvered block tag code define"` (tags 0, 6); 0x48E `"Block1 Detailed Timing Descriptor %d\n"` + 0x48F the DTD group as in block 0 |
| `_MI_HDMI_ParseAudioDataBlock` | — | 0x335–0x34D `"<n> output E_MI_HDMI_AUDIO_CODING_<PCM/AC3/MPEG1/MP3/MPEG2/AAC/DTS>"` per SAD format 1–7; 0x350 `"resvered block tag code define"` (formats 8–15) |
| `_MI_HDMI_ParseVideoDataBlock` | — | 0x37C `" Video Data Block"`, 0x386 `"\t\t %s \n"` T1 name per SVD |
| `_MI_HDMI_ParseVendorSpecDataBlock` | — | 0x394 `"This is HDMI Device"`; 0x39A `"This is DVI Device, we don't parse it"`; `"len:%d\n"` where parsing stops (0x3A0 length < 4, 0x3AA < 6, 0x3B6 = 6, 0x3BD = 7, 0x3C8 = 8); 0x3C0 `"support 3d:%d\n"` = byte 8 bit 5 (HDMI_Video_present, stored as `bHdmiVideoPresent`), 0x3C1 `"CNC:%d\n"` = byte 8 bits 3–0 (content types; printed only) |
| `_MI_HDMI_ParseSpeakerDataBlock` | — | 0x3D6 `"Speaker Data[0]:0x%x Data[0]:0x%x\n"` (payload bytes 1 and 2; the second label should read Data[1]) |
| `_MI_HDMI_ParseExtDataBlock` | — | ext tag 0 `"Video Capability Data Block"`, 1 `"vendor specific data block"`, 2 `"reserved vesa display device"`, 3 `"reserved vesa video data block"`, 4 `"reserved hdmi video data block"`, 5 `"Colorimetry:0x%02x\n"` (byte 2), 0x10 `"CEA miscellanenous audio data fileds"`, 0x11 `"vendor specific audio data block"`, 0x12 `"reserved hdmi audio data block"` (lines 0x3E5–0x405) |

The 0xFD descriptor (`_MI_HDMI_ParseBlock0DtdTiming`, lines 0x2E6–0x308), all prints at level ≥ 5:

| byte | print | value printed |
|---|---|---|
| 5 | `"\tMin. Vertical rate (Hz): %d\n"` | byte |
| 6 | `"\tMax. Vertical rate (Hz): %d\n"` | byte |
| 7 | `"\tMin. Horizontal rate (KHz): %d\n"` | byte |
| 8 | `"\tMax. Horizontal rate (KHz): %d\n"` | byte |
| 9 | `"\tMax. Supported Pixel Clock rate (KHz): %d\n"` | byte × 10 (the EDID unit is 10 MHz, so the "KHz" label is wrong by 1000 [I]) |
| 10 | `"\tDefault GTF supported"` if 0; `"\tSecondary GTF supported"` if 2; nothing for other values (1 range-limits-only, 4 CVT) | — |

## 4. mi_disp.ko [R]

### 4.1 Load, parameters, open and close
- `init_module`: the per-module procfs, a module semaphore, `MI_DEVICE_Register` → `/dev/mi_disp`. Parameters:
  `thread_priority` ("set disp work thread priority", int — the priority of the mi_sys work thread of each DISP device) and the
  debug parameters. `cleanup_module` reverses it.
- `process_init` = `MI_DISP_IMPL_Init` (0x193b4; refcounted: the first open initialises, later opens only increment the count
  at `_stModRes` + 0), `process_exit` = `MI_DISP_IMPL_DeInit` (0x1e898; the last close tears everything down, earlier closes only
  decrement). Both run under the module semaphore. Their return values are 0 / 1 (DeInit also `0xA00F8001`), not MI codes;
  `mi_disp_process_init` passes them to mi_common's open, which fails the `open()` on non-zero [I from the mi_common open path,
  `mi_sys-2-interfaces.md` §1.3]. libmi_disp closes its fd only in `MI_DISP_DeInitDev` (§5).

### 4.2 State objects [R offsets from the decompile; field names from the proc dumps of §4.13 and the public header]
All state is one static `.bss` block `_stModRes` (0x4c30 bytes): `+0` u8 open refcount, `+0x8` device 0, `+0x420` device 1
(0x418 each), `+0x838` video layers 0–3 (0x10d0 each), `+0x4b78` the WBC object (0xb8, §4.12), `+0x4c1c` … `+0x4c28` the SRAM
request and the rotating-layer mask (§4.11). Other `.bss`: `gbRejectCmdq` (u8, never written in this module — always 0, so the
CMDQ register path is on), `gbRejectCmdq` + 1 the proc `trace` flag, panel-instance slots (`gbRejectCmdq` + 4, 100 pointers,
§4.2.4), MMA hooks (`.bss` + 0x460 / + 0x464 = `_MI_DISP_IMPL_MiSysAlloc` / `Free`, handed to every `MHAL_DISP_*CreateInstance`),
`gstRotateBufConf` (0x600: 4 layers × 16 rotate buffers × 0x18), `gstSrcImage` / `gstDstImage` (0x28 each, mi_gfx surfaces of
the rotation path). Concurrency: the module mutex around every ioctl **plus** a CamOs semaphore per device (dev + 0x3ec), per
layer (layer + 0x10b0), per port (port + 0x64) and one for WBC (`_stModRes` + 0x4bb8); the ISRs take none.

| object | count | mhal object |
|---|---|---|
| device | 2 (dev 0, 1) | `MHAL_DISP_DeviceCreateInstance` |
| video layer | 4 (0, 1 used with device 0; 2, 3 with device 1 — by convention only, §4.9) | `MHAL_DISP_VideoLayerCreateInstance` |
| input port | 16 on layers 0 / 2, 1 on layers 1 / 3 | `MHAL_DISP_InputPortCreateInstance` (at layer enable) |
| mi_sys device | 1 per DISP device: module id **0xF**, 0x11 channels; plus the WBC device (§4.12) | `mi_sys_RegisterDev` |

#### 4.2.1 Device (0x418, two of them)
| off | field | set by / meaning |
|---|---|---|
| +0x000 | mhal device ctx | `MHAL_DISP_DeviceCreateInstance` at init |
| +0x004 | mi_sys device handle | `mi_sys_RegisterDev` (module 0xF, dev id, 0x11 channels) |
| +0x008 | created | 1 after init of that device |
| +0x009 | enabled | ioctl 0 / 1 (printed as `DevStatus`) |
| +0x00a | vsync IRQ requested | init |
| +0x00b | device id | init (also read by the LCD ISR) |
| +0x00c / +0x010 | vsync IRQ number / vsync counter (`IrqNum`, `IrqCnt`) | init / both ISRs |
| +0x014 | IRQ name `mi_disp%d_isr` | |
| +0x094 | wait queue `stWaitIsrTcond` | woken by the ISRs; waited on by the latch waits (§4.8.3) |
| +0x0ac / +0x0b0 / +0x0b1 | LCD IRQ number / requested / name `mi_disp%d_isr_lcd` | init |
| +0x134 | background colour `u32BgColor` (init 0x800080 [I: black in YUV]) | SetPubAttr, proc `bgcolor` |
| +0x138 | interface mask: bit *n* = MI interface *n* (§4.6.1) was ever set — **never cleared** | SetPubAttr |
| +0x140 / +0x148 | previous / last vsync time, µs (u64 fields, high word always written 0) | ISRs |
| +0x150 / +0x154 | vsync-interval sample count / accumulated interval (CHECK_PTS monitor, §4.10.4) | enqueue path |
| +0x158 | frame period µs = 10⁶ / fps(timing), 16666 for USER and out-of-range (fps column of §4.6.4) | SetPubAttr |
| +0x15c | MI output timing (`eIntfSync`) | SetPubAttr |
| +0x160 | `MI_DISP_SyncInfo_t` (0x28) as handed to mhal | SetPubAttr |
| +0x188 | colour temperature (6 × u16: R / G / B offset, R / G / B colour) | ioctl 0x2c, proc `colortemp` |
| +0x194 | gamma record as passed by the caller (`bEn`, `u16EntryNum`, three **user** pointers) | ioctl 0x2e |
| +0x1a4 | gain[15] (per MI interface) | Set{Vga,Hdmi}Param, proc `csc` |
| +0x1e0 | sharpness[15] | Set{Vga,Hdmi,Lcd}Param, proc `csc` |
| +0x21c | CSC[15] × 0x14 `{matrix, luma, contrast, hue, saturation}` — init `{3, 50, 50, 50, 50}` each | same |
| +0x348 | list head of the video layers bound to this device | Bind / UnBindVideoLayer |
| +0x350 | u8 passed to `mi_sys_NotifyWakeupISREvent` (always 0) | init |
| +0x354 | 0x98-byte mi_sys pass callback table (copied into the registration) — slot 0x8c = **0x33 concurrent input tasks** (`mi_sys-1-engine.md` §2.3) | init |
| +0x3ec | semaphore | |
| +0x40c | "fired" (flipped) count | enqueue path |
| +0x410 / +0x414 | barrier outstanding / the barrier's vsync count (§4.10) | `BarrierInputTask`, `CheckInputTaskStatus` |

#### 4.2.2 Video layer (0x10d0, four of them)
| off | field |
|---|---|
| +0x000 | layer id |
| +0x001 | bound device id |
| +0x002 | created (mhal instance exists — always after init) |
| +0x003 | enabled (ioctl 6 / 7) |
| +0x004 | play toleration (ms, ioctl 0xd) |
| +0x008 | list node in the device's layer list |
| +0x010 | `stVidLayerDispWin` {x, y, w, h} u16 |
| +0x018 / +0x01c | `stVidLayerSize` w / h (u32) |
| +0x020 | pixel format |
| +0x024 | rotate mode (`MI_DISP_RotateMode_e` 0 none, 1 = 90, 2 = 180, 3 = 270) |
| +0x028 | 16 input ports × 0x108 (only port 0 used on layers 1 / 3) |
| +0x10a8 | mhal video-layer ctx |
| +0x10ac | pointer to the bound device (0 = not bound) |
| +0x10b0 | semaphore |

#### 4.2.3 Input port (0x108)
Port *p* of layer *l* lies at `_stModRes` + 0x838 + 0x10d0 · *l* + 0x28 + 0x108 · *p*; code that addresses ports from
`_stModRes` + 0x10d0 · *l* + 0x108 · *p* uses offsets 0x860 larger (+0x8a4 / +0x8a8 / +0x8aa = the display window +0x44 / +0x48 / +0x4a, +0x8bc =
+0x5c) [I: arithmetic].

| off | field (proc name) |
|---|---|
| +0x00 | mhal input-port ctx (0 = layer not enabled) |
| +0x04 | port id |
| +0x08 | enabled (`enable`) |
| +0x09 | "clear all" pending (`bClearAllTask`) |
| +0x0b | first-frame flag (the next flip also enables the port in hardware) |
| +0x0c | crop on (ioctl 0x21) |
| +0x0d / +0x10 / +0x18 / +0x20 | CHECK_PTS state: drift sign, time of the last flip (µs), drift (u64), drift-sample count (u64) |
| +0x28 | sync mode (`syncmode`: 0 invalid, 1 CheckPts, 2 FreeRun) |
| +0x2c | requested status |
| +0x30 | current status (`CurStatus`: `MI_DISP_InputPortStatus_e` 1 pause, 2 resume, 3 step, 5 show, 6 hide) |
| +0x3c / +0x3e / +0x40 / +0x42 | crop x / y / w / h (proc `crop`, ioctl 0x21 zoom) |
| +0x44 / +0x46 / +0x48 / +0x4a | display window x / y / w / h (`show_*`; ioctl 0x14 / 0x18) |
| +0x4c / +0x4e | source w / h (`src_w/h`; ioctl 0x14) |
| +0x50 | pointer to the owning layer |
| +0x54 | bound source (set by `OnBindInputPort`) |
| +0x5c / +0x60 | pending ("fired") task / task on screen (`FiredTask`, `OnScreenTask`) |
| +0x64 | semaphore |
| +0x84 | "refresh input buff" counter (§4.10.3) |
| +0x8a | fps byte (`fps`, counted per jiffies-second by the CheckFramePts statistics) |
| +0xc4 / +0xc8 | release count / drop count |
| +0xcc | step-task count (`StepTaskCnt`) |
| +0xe8 … +0x100 | last received buffer: count, width, height, content w / h, stride, pixel format (`RecvBuf*`, `Content_*`, `PixFmt`) |

#### 4.2.4 Panel-instance slots (`gbRejectCmdq` + 4, 100 pointers) [R `disp_panel_slot_get` (0x1443c), `disp_panel_slot_alloc` (0x15b68)]
Each non-zero slot points to a `CamOsMemAlloc(0x1f0)` block: `+0` the MhalPnl instance (written by `MhalPnlCreateInstanceEx`),
`+8` the mhal link type, `+0x11c` a copy (0xd4 bytes) of the `MhalPnlUnifiedParamConfig_t` last applied. Created only by the
BT656 / BT1120 set-up of §4.6.3; destroyed all at once (`MhalPnlDestroyInstance` + free) by `disp_panel_slot_alloc` [name I: "destroy
panels"]; a destroy failure returns `0x1F` and leaves the remaining slots.

### 4.3 Init (first open) [R `MI_DISP_IMPL_Init` 0x193b4]
1. Both devices: zero 0x418, wait queue, semaphore, layer-list head, background 0x800080, the 15 CSC records
   `{3, 50, 50, 50, 50}`. All four layers: zero 0x10d0, layer semaphore, 16 port semaphores. WBC semaphore. MMA hooks.
2. **Panel / timing table for mhal.** `kmalloc(0x11ac)` = 0x27 × 0x74-byte `MHAL_DISP_PanelConfig_t` entries, zeroed. For every
   **mhal** timing index *i* = 0…0x26 (the mhal timing enum, `mhal-disp.md` §2.2) a SYSCFG timing id (`config_tool.md` §5.3) is
   chosen by the switch below and `MI_SYSCFG_GetPanelInfo(intf, id)` is called with intf **7** (DAC_I) for *i* ∈ {0, 1, 9, 0xa}
   and **8** (DAC_P) otherwise [R disassembly 0x97a0–0x97cc: `intf = (n > 1 && n−9 >u 1) ? 8 : 7` for the next index n]; a hit
   fills entry *i* and sets `eTiming` (+0x68) = *i* and `bValid` (+0x70) = 1. Indices without an id are left all zero (invalid):

   | mhal index | SYSCFG id | mhal index | SYSCFG id |
   |---|---|---|---|
   | 0 PAL | 4 576I | 0x13 1366x768_60 | 0x2f |
   | 1 NTSC | 2 480I | 0x14 1440x900_60 | 0x32 |
   | 2 960H_PAL, 3 960H_NTSC | — | 0x15 1280x800_60 | 0x30 |
   | 4 / 5 / 6 1080P24 / 25 / 30 | 10 / 11 / 12 | 0x16 1680x1050_60 | 0x34 |
   | 7 720P50 | 7 | 0x17 1920x2160_30 | — |
   | 8 720P60 | 9 | 0x18 1600x1200_60 | 0x33 |
   | 9 1080I50 | 0xe | 0x19 1920x1200_60 | — |
   | 0xa 1080I60 | 0x11 | 0x1a 2560x1440_30 | 0x1b (2K2KP_30) |
   | 0xb 1080P50 | 0xf | 0x1b 2560x1600_60 | — |
   | **0xc 1080P60** | **0x12** | **0x1c 3840x2160_30** | **0x24** |
   | 0xd 576P50 | 5 | 0x1d 3840x2160_60 | 0x26 |
   | 0xe 480P60 | 3 | 0x1e / 0x1f 1080P59.94 / 29.97 | 0x10 / 0xd |
   | 0xf 640x480_60, 0x10 800x600_60 | — | 0x20 / 0x21 720P59.94 / 29.97 | 8 / 6 |
   | 0x11 1024x768_60 | 0x2e | 0x22 / 0x23 2560x1440_50 / 60 | 0x13 / 0x14 |
   | 0x12 1280x1024_60 | 0x31 | 0x24 / 0x25 3840x2160_25 / 29.97 | 0x22 / 0x23 |
   | | | 0x26 USER | — |

   Entry fields are copied only when the record's group flag is set (record layout `config_tool.md` §5.2): name pointer and
   dither bit always; link type (record +0x84) into bits 1–4 of byte +8 with the map 0 / 0x11 → 0, 1…7, 9, 10 unchanged,
   everything else (8, 0xb–0x10) → 8; polarity bits (byte +0xb) if `TgnPolarityFlag`; the 14 timing u16 (Max / Min totals
   included) and the 3 DCLKs (+0x18…+0x3c) if `TgnTimingFlag`; SSC step / span (+0x42 / +0x44) if `TgnSscFlag`; bit mode
   (+0x54, clamped to ≤ 2) if `TgnOutputBitMdFlag`. Then `MHAL_DISP_InitPanelConfig(arr, 0x27)` — **called even when nothing
   was found** — and `kfree`. The records come from `/misc/config.ini` (`config_ini_path`; the `/config/config.ini` copy is
   identical and only a fallback) through config_tool (`config_tool.md` §4, §5); which fields of an entry mhal uses is
   `mhal-disp.md` §7.1. So the HDMI / VGA porch timings mhal uses come from mi_sys's SYSCFG records, not from code.
3. `MI_SYSCFG_GetMmapInfo("E_MMAP_ID_XC_MAIN_FB")` and `("E_MMAP_ID_XC_MLOAD")`: if present, the 5-word record is passed to
   `MHAL_DISP_InitMmapConfig(0 / 1, …)` — an empty function returning 1 in this mhal, so the areas are ignored (`mhal-disp.md`
   §2.1). `MI_SYSCFG_GetPqTbl(&size)`: non-NULL → `MHAL_DISP_DeviceSetPqConfig(0, {flags 1, size, data})` — the device argument
   is the integer 0, not a ctx.
4. Per device *d* = 0, 1 (stops at the first device whose "created" byte is already set, returning 0):
   - pass callback table (slot names `mi_sys-1-engine.md` §2.3): PreProcess `OnPreProcessInputTask` (clears byte +4 of the
     task), Enqueue `_MI_DISP_EnqueueInputTask`, Barrier `_MI_DISP_BarrierInputTask`, `CheckInputTaskStatus`,
     `DequeueInputTask` (returns 1), `CheckOutputTaskStatus` (1), `DequeueOutputTask` (1), input / output bind capability (both
     write 0x11 at +0xc of the caps record), `OnBind` / `OnUnBindInputPort`, `OnBind` / `OnUnBindOutputPort` (return 0),
     `RejectCmdq` (returns 0 and asks for CMDQ type 0x40000 while `gbRejectCmdq` = 0), `BarrierByPass` (`_MI_DISP_BarrierByPass`,
     always 1), `NeedIndividualWorkThread` (1 — one mi_sys work thread per device, priority `thread_priority`), slot 0x8c = 0x33;
   - registration record (0x298, `mi_sys-1-engine.md` §2.2): module **0xF**, dev *d*, 1, 1, **0x11 channels**, 1, the callback
     table copy, SRAM callbacks `OnConfigSramBuf` / `OnAllowPreemptionSram` / `OnAllowMoveableSram` (they return the module-wide
     SRAM request of §4.11.3), dump callbacks `OnDumpDevAttr`, `OnDumpChannelAttr` (empty), `OnDumpInputPortAttr`,
     `OnDumpOutPortAttr` (empty), `OnHelp` (§4.13). `mi_sys_RegisterDev` failure → **return 1**;
   - the 17 proc commands (§4.13); `MHAL_DISP_DeviceCreateInstance(hooks, d, &dev)` (failure → return 1);
   - **vsync IRQ**: `MHAL_DISP_GetDevIrq` → `CamOsIrqRequest(irq, _MI_DISP_DevISR, "mi_disp%d_isr", dev)` →
     `MHAL_DISP_EnableDevIrq(…, 1)`; on enable failure the IRQ is freed; a request failure is only logged. **LCD IRQ** the same
     with `GetLcdIrq` / `_MI_DISP_DevISRLcd` / `"mi_disp%d_isr_lcd"` / `EnableLcdIrq`;
   - `MHAL_DISP_SetRegAccessConfig({0 = CPU})`, `MHAL_DISP_SetRegFlipConfig({1})`, `MHAL_DISP_SetRegAccessConfig({1 = CMDQ})`
     (the CMDQ register path, on unless `gbRejectCmdq`); created := 1.
   **Device 1 is set up exactly like device 0** — two mi_sys devices, two mhal device contexts, both IRQ pairs requested (mhal
   returns the DT slots, `mhal-disp.md` §9.1).
5. Create the four video-layer instances (`MHAL_DISP_VideoLayerCreateInstance(hooks, l, &ctx)`; failure → return 1), layer
   id, created := 1; refcount := 1; return 0.

### 4.4 DeInit (last close) [R `MI_DISP_IMPL_DeInit` 0x1e898]
1. For each layer 0–3: rotate mode := 0 (`SetVideoLayerRotateMode`, frees the rotate buffers, §4.11); every enabled port →
   `DisableInputPort` (§4.8.3); an enabled layer → `DisableVideoLayer`.
2. WBC (under its semaphore): DMA IRQ off and freed; if a DMA instance exists and WBC is attached: CPU register path,
   `MHAL_DISP_DmaUnBind({src dev ctx, the other dev ctx})`, CMDQ path, attached := 0; `MHAL_DISP_DmaDestoryInstance`. Then the
   WBC mi_sys device is unregistered, and the three WBC in-flight counters are polled to 0 (1-jiffy sleeps, ≤ 1000).
3. Per device: LCD IRQ off + free, vsync IRQ off + free (a failure only sets the error flag), `MI_DISP_IMPL_Disable`.
4. Per device: `mi_sys_UnRegisterDev` (failure → result `0xA00F8001`), then `MHAL_DISP_DeviceDestroyInstance`, semaphore deinit.
   Per layer: the 16 port semaphores, `MHAL_DISP_VideoLayerDestoryInstance`, layer semaphore.
5. If **device 1's** interface mask has BT656 or BT1120 (bits 3 / 4 of `_stModRes` + 0x558) the panel instances are destroyed
   (`disp_panel_slot_alloc`) — device 0's mask is not checked, so panels created for device 0 survive a deinit [R; I: a vendor slip].
6. The refcount is decremented only when every step succeeded; otherwise the error (1 or `0xA00F8001`) is returned and the
   refcount stays at 1 — a later open then finds the module "already initialised" and only counts [I from step 1 of §4.3].

### 4.5 Interrupts [R `_MI_DISP_DevISR` 0x137b0, `_MI_DISP_DevISRLcd` 0x136a0]
Both ISRs get the device record. Common body ("vsync event"): vsync counter (+0x10) ++, `mi_sys_NotifyWakeupISREvent(sysdev,
0)` (wakes the mi_sys work thread of this device), wake the device wait queue (+0x94), previous := last vsync time, last := now
(µs, `CamOsGetTimeOfDay`). Both return the mhal result as the IRQ return value.
- `_MI_DISP_DevISR` (device vsync, the MOPG IRQ, `mhal-disp.md` §9.1): `MHAL_DISP_GetDevIrqFlag` → if any flag bit is set,
  `ClearDevInterrupt`; runs the vsync event **only when the device has no MCU interface** (mask bits 13 / 14 clear).
- `_MI_DISP_DevISRLcd` (LCD / MCU IRQ, DT slot 5): **on device 0 only** reads and clears the LCD flag (`GetLcdIrqFlag` /
  `ClearLcdInterrupt`, nothing done if no flag); on device 1 the flag is neither read nor cleared; runs the vsync event **only
  when the device has an MCU interface**.
So an MCU panel (frame-based, no continuous vsync) is paced by the LCD "frame done" interrupt instead of the vsync, and the
⅓-period timing window of §4.10.1 is skipped for it; a port disable also skips the latch wait (`disp_latch_wait` (0x187a0)) for MCU devices.
On this camera (HDMI only) the LCD IRQ is requested and enabled but never acts [I: no MCU interface is ever set].

### 4.6 SetPubAttr (ioctl 2) [R `MI_DISP_IMPL_SetPubAttr` 0x1a890, jump tables decoded from the disassembly]

#### 4.6.1 Checks, background, interface
<!-- ev:mi_disp-mi_hdmi.4.6.1 -->
Payload `{dev, MI_DISP_PubAttr_t}` (`+0` bg colour, `+4` `eIntfType`, `+8` `eIntfSync`, `+0xc` `stSyncInfo` 0x28; header, enum values confirmed by the DWARF). MI interfaces: 0 CVBS, 1 YPbPr, 2 VGA, 3 BT656, 4 BT1120, 5 HDMI,
6 LCD, 7 BT656_H, 8 BT656_L, 9 TTL, 10 MIPIDSI, 11 TTL_SPI_IF, 12 SRGB, 13 MCU, 14 MCU_NOFLM.
1. dev > 1 or not created → `0x1F`.
2. Timing filter for BT656 and BT1120 only (no other interface is filtered): accepted MI timings are BT656 {6 480P60, 7 576P50,
   8 720P50, 9 720P60, 11 1080P25, 12 1080P30, 36 1080P29.97, 37 720P59.94, 40 USER}, BT1120 the same + {15 1080P50, 16 1080P60,
   35 1080P59.94}; anything else → `0x1F` ("Not support resolution id[%d] for BT656/BT1120"). The masks are `0xc000006f` /
   `0xe000066f` over *t* − 6; PAL / NTSC are **rejected for BT656** [R; I: the mask was written for an older enum].
3. Device semaphore; CPU register path (`SetRegAccessConfig`); `MHAL_DISP_DeviceSetBackGroundColor(dev, bg)` (failure → `0x1F`).
4. mhal interface mask = OR over the device's sticky MI mask (§4.2.1 +0x138) and the new interface, with MI → mhal: CVBS 2,
   YPbPr 8, VGA 4, BT656 0x40000, BT1120 0x100000, **HDMI 1**, LCD 0x10, TTL and TTL_SPI_IF 0x10000, MIPIDSI 0x20000, SRGB
   0x400000, MCU 0x200000, MCU_NOFLM 0x800000; **BT656_H, BT656_L and any value > 14 are logged "invalid intf" and become 1 (the
   HDMI bit)** [R jump tables at `.text` 0xa924 / 0xa974]. The VGA value 4 is removed when the new timing is 28…34
   (2560x1440_30 … 3840x2160_60); if the mask is then empty → `0x1F`. `MHAL_DISP_DeviceAddOutInterface(dev, mask)` (failure →
   `0x1F`). Because the MI mask is never cleared, a device that once had VGA keeps adding VGA to every later SetPubAttr.

#### 4.6.2 SyncInfo source
- **User SyncInfo** (copied verbatim) for CVBS, YPbPr, VGA, HDMI, BT656_H, BT656_L.
- **From SYSCFG** for BT656, BT1120, LCD, TTL, MIPIDSI, TTL_SPI_IF, SRGB, MCU, MCU_NOFLM (mask `0xfcb` over intf − 3): link type
  BT656 0xc, BT1120 0xe, TTL 0, MIPIDSI 0xb, TTL_SPI_IF 0x11, SRGB 0x10, MCU / MCU_NOFLM 0xf, LCD (and anything else) 8; timing
  id = the MI → SYSCFG map of §4.6.4 for BT656 / BT1120, **0x38 ("first record of that link type") for all the others**. Record
  not found → warning "get panel param failed", SyncInfo stays all zero (continues). Found → SyncInfo built from the record
  (`config_tool.md` §5.2 offsets): bSynm = bIop = u8Intfb = 0; VStart = `m_wPanelVStart`, Vact = `m_wPanelHeight`, Vbb =
  `m_wPanelVBackPorch`, **Vfb = VTotal − VBackPorch − (Height + VSyncWidth)**; HStart = `m_wPanelHStart`, Hact =
  `m_wPanelWidth`, Hbb = `m_wPanelHSyncBackPorch`, **Hfb = HTotal − HBackPorch − (Width + HSyncWidth)**; Hmid / Bvact / Bvbb /
  Bvfb = 0; Hpw = HSyncWidth, Vpw = VSyncWidth; polarity bytes 0; **u32FrameRate = DCLK / (VTotal × HTotal)** (integer
  division).
The mhal SyncInfo is field-for-field the MI one (same order, `MHAL_DISP_SyncInfo_t`, `mhal-disp.md` §2.2).

#### 4.6.3 Timing, panel set-up, stored state
1. mhal timing = the MI → mhal column of §4.6.4, except **BT656 and BT1120 always use 0x26 (USER)**; LCD / TTL / MIPI / MCU keep
   the mapped timing. `MHAL_DISP_DeviceSetOutputTiming(dev, mhal mask, {timing, &sync})` — its return value is ignored.
2. BT656 / BT1120 only: destroy all panel instances (§4.2.4), then the panel set-up `disp_panel_slot_get(dev, attr)`: link type from
   the MI interface as in §4.6.2; if a slot with that link type exists → done (cannot happen after the destroy); else take a free
   slot (none of 100 → return quietly), `CamOsMemAlloc(0x1f0)`; record = `GetPanelInfo(link, id)` with *id* from the SYSCFG
   column of §4.6.4 (USER → 0x38, unmapped → 0x39) — link types other than 0, 1, 0xb, 0xc, 0xe, 0xf, 0x10, 0x11 are logged
   "unsupported" and looked up as **0 (TTL)** (jump table at `.text` 0x4518, lost by Ghidra); not found → free, `0x1F` (ignored
   by SetPubAttr). `MhalPnlCreateInstanceEx(slot, link, dev)` (0 → free, `0x1F`); slot link := link; the
   `MhalPnlUnifiedParamConfig_t` (0xd4) is filled from the record as `config_tool.md` §5.2's consumer column gives it, with the
   clamps listed there (link type outside the supported set → 0, bit mode > 3 → 2, RGB swap defaults R3 G2 B1, MIPI lanes
   outside 1–4 → 1, format > 3 → 3, ctrl > 3 → 1, packet type ∉ {0, 1} → 0, MCU type ∉ {0, 1} → 0, MCU bus cfg > 0x12 → 0);
   `MhalPnlSetUnifiedParamConfig(inst, &cfg)` (result ignored) and a copy kept in the slot. **No other interface creates a
   panel instance in mi_disp** — LCD / TTL / MIPI / MCU / SRGB only get their timing from SYSCFG here; their panel hardware
   set-up is mi_panel's (`panel-mipitx.md` §3) [R for mi_disp; I for mi_panel].
3. Store: bg colour, MI mask |= 1 << intf, frame period (µs) = 10⁶ / fps[t] for t < 0x28, else 16666 — so USER timings ignore
   the user's `u32FrameRate` for frame pacing —, MI timing, SyncInfo. CMDQ register path back on; semaphore up; return 0 (or the
   `0x1F` of the steps above). Nothing checks that the device is disabled.

#### 4.6.4 Full MI → mhal timing table and the per-timing constants [R; mhal names `mhal-disp.md` §2.2]
"mhal" = the timing passed to `SetOutputTiming` (switch at `.text` 0xad76); "SYSCFG" = the timing id used for BT656 / BT1120
SyncInfo and panel look-ups (switches at 0xafe8 and 0x454a — identical); "fps" = `.rodata` byte table (`.LANCHOR1` + 0x62c)
used for the frame period; "size" = the resolution `GetTimingSize` (0x1306c) [name I: GetTimingSize] returns for the DMA / WBC / position
checks and the window clipping (§4.8, §4.10, §4.12).

| MI | name | mhal | SYSCFG | fps | size w×h |
|---|---|---|---|---|---|
| 0 | PAL | 0 PAL | 4 | 25 | 720×576 |
| 1 | NTSC | 1 NTSC | 2 | 30 | 720×480 |
| 2 | 960H_PAL | 2 | 0x39 | 25 | 960×576 |
| 3 | 960H_NTSC | 3 | 0x39 | 30 | 960×480 |
| 4 | 480i60 | **0 PAL** | 0x39 | 60 | 720×480 |
| 5 | 576i50 | **1 NTSC** | 0x39 | 50 | 720×576 |
| 6 | 480P60 | 0xe | 3 | 60 | 720×480 |
| 7 | 576P50 | 0xd | 5 | 50 | 720×576 |
| 8 | 720P50 | 7 | 7 | 50 | 1280×720 |
| 9 | 720P60 | 8 | 9 | 60 | 1280×720 |
| 10 / 11 / 12 | 1080P24 / 25 / 30 | 4 / 5 / 6 | 10 / 11 / 12 | 24 / 25 / 30 | 1920×1080 |
| 13 | 1080I50 | 9 | 0xe | 50 | 1920×1080 |
| 14 | 1080I60 | **0xc 1080P60** ("invalid timing" logged; mhal 0xa is never produced) | 0x11 | 60 | 1920×1080 |
| 15 | 1080P50 | 0xb | 0xf | 50 | 1920×1080 |
| **16** | **1080P60** | **0xc** | 0x12 | 60 | 1920×1080 |
| 17 / 18 | 640×480 / 800×600 | 0xf / 0x10 | 0x39 | 60 | as named |
| 19 / 20 / 21 / 22 / 23 / 24 | 1024×768 / 1280×1024 / 1366×768 / 1440×900 / 1280×800 / 1680×1050 | 0x11…0x16 | 0x2e / 0x31 / 0x2f / 0x32 / 0x30 / 0x34 | 60 | as named (1366 → 0x556) |
| 25 | 1920x2160_30 | 0x17 | 0x39 | 30 | 1920×2160 |
| 26 / 27 | 1600×1200 / 1920×1200 | 0x18 / 0x19 | 0x33 / 0x39 | 60 | as named |
| 28 / 29 / 30 | 2560×1440 _30 / _50 / _60 | 0x1a / 0x22 / 0x23 | 0x1b / 0x13 / 0x14 | 30 / 50 / 60 | 2560×1440 |
| 31 | 2560×1600_60 | 0x1b | 0x39 | 60 | 2560×1600 |
| 32 / **33** / 34 | 3840×2160 _25 / **_30** / _60 | 0x24 / **0x1c** / 0x1d | 0x22 / 0x24 / 0x26 | 25 / 30 / 60 | 3840×2160 |
| 35 / 36 | 1080P59.94 / 29.97 | 0x1e / 0x1f | 0x10 / 0xd | 60 / 30 | 1920×1080 |
| 37 / 38 | 720P59.94 / 29.97 | 0x20 / 0x21 | 8 / 0x39 | 60 / 30 | 1280×720 |
| 39 | 3840×2160_29.97 | 0x25 | 0x23 | 30 | 3840×2160 |
| 40 | USER | 0x26 | 0x38 | (16666 µs fixed) | SyncInfo Hact × Vact |
| > 40 | — | 0xc (logged) | 0x39 | 16666 µs | 0×0 |

MI 4 / 5 swap PAL / NTSC and MI 14 falls to 1080P60 [R; I: vendor slips]. The app uses MI 16 (0x10) and 33 (0x21).

### 4.7 ioctl table
<!-- ev:mi_disp-mi_hdmi.4.7 -->
53 entries, nr 0xc empty; all under the module mutex. Errors are the **bare
`0x1F`** (not `0xA00F…`) unless noted. Payload sizes per wrapper: §5 and `mi-user-abi.md` §3.3. "Port checks" = layer < 4 and
port < 16 (no check that the layer has that many ports — ports 1–15 of layers 1 / 3 pass the range check and touch zeroed
state); "dev checks" = dev < 2. Bold = used by the app.

| nr | name | where |
|---|---|---|
| **0 / 1** | Enable / Disable `{dev}` | §4.8.1 |
| **2** | SetPubAttr `{dev, PubAttr(0x34)}` | §4.6 |
| 3 | GetPubAttr | §4.8.1 |
| 4 / 5 | DeviceAttach / DeviceDetach | §4.12.4 |
| **6 / 7** | EnableVideoLayer / DisableVideoLayer `{layer}` | §4.8.2 |
| **8 / 9** | SetVideoLayerAttr / GetVideoLayerAttr `{layer, attr(0x14)}` | §4.8.2 |
| **0xa / 0xb** | BindVideoLayer / UnBindVideoLayer `{layer, dev}` | §4.8.2 |
| 0xc | — (empty) | |
| 0xd / 0xe | SetPlayToleration / GetPlayToleration | §4.8.2 |
| 0xf | GetOutputTiming | §4.8.1 |
| 0x10 / 0x11 | GetScreenFrame / ReleaseScreenFrame | §4.8.1 |
| 0x12 / 0x13 | SetVideoLayerAttrBegin / End | §4.8.2 |
| **0x14** / 0x15 | SetInputPortAttr / GetInputPortAttr `{layer, port, attr(0xc)}` | §4.8.3 |
| **0x16 / 0x17** | EnableInputPort / DisableInputPort `{layer, port}` | §4.8.3 |
| 0x18 / 0x19 | SetInputPortDispPos / GetInputPortDispPos | §4.8.3 |
| **0x1a / 0x1b** | PauseInputPort / ResumeInputPort `{layer, port}` | §4.8.3 |
| 0x1c / 0x1d / 0x1e | StepInputPort / ShowInputPort / HideInputPort | §4.8.3 |
| **0x1f** | SetInputPortSyncMode `{layer, port, mode}` | §4.8.3 |
| 0x20 | QueryInputPortStat | §4.8.3 |
| 0x21 | SetZoomInWindow | §4.8.3 |
| 0x22–0x29 | Get / Set Vga, Hdmi, Lcd, Cvbs Param | §4.8.4 |
| **0x2a** | ClearInputPortBuffer `{layer, port, bClrAll}` | §4.8.3 |
| 0x2b | SetVideoLayerRotateMode | §4.11 |
| 0x2c / 0x2d | DeviceSetColorTempeture / DeviceGetColorTempeture | §4.8.4 |
| 0x2e | DeviceSetGammaParam | §4.8.4 |
| 0x2f–0x34 | Set / GetWBCSource, Set / GetWBCAttr, Enable / DisableWBC | §4.12.3 |

### 4.8 The ioctls by object

#### 4.8.1 Device
- **0 / 1 Enable / Disable `{dev}`**: dev < 2 and created; `MHAL_DISP_DeviceEnable(h, 1 / 0)`; enabled flag (+0x009).
- **3 GetPubAttr `{dev, PubAttr(out)}`**: device enabled required (else `0x1F`). Returns the bg colour (+0), **`eIntfSync` only if
  the caller's own `eIntfType` field is ≤ 14**, and the stored SyncInfo; **`eIntfType` is never written** — it stays what the
  caller put there (libmi_disp zero-fills the payload, so the app-level result is intf 0 = CVBS) [R disassembly 0xb316–0xb336].
- **0xf GetOutputTiming `{id, timing(out), SyncInfo(out 0x28)}`** (library: `mi_disp_GetScreenFrameSize`, §5): the id is used
  as a **layer** id (< 4): the layer's bound device must be enabled; returns that device's MI timing and SyncInfo. libmi_disp
  passes 0, i.e. "the device layer 0 is bound to".
- **0x10 / 0x11 Get / ReleaseScreenFrame**: both return `0x1F` unconditionally (ReleaseScreenFrame only range-checks first).
  libmi_disp does not call them (its `_mi_disp_GetScreenFrame` / `_mi_disp_ReleaseScreenFrame` return `0xA00F8012`, §5). The
  proc command `getcapframe` (§4.13) is the only other screen-frame path.

#### 4.8.2 Video layer
- **6 EnableVideoLayer `{layer}`**: needs the layer bound to a device and not enabled; creates its input-port instances
  (`MHAL_DISP_InputPortCreateInstance`; 16 if `(layer & ~2) == 0`, else 1) and disables port 0 in hardware
  (`InputPortEnable(0)`, `SetRegFlipConfig`); `MHAL_DISP_VideoLayerEnable(h, 1)`.
- **7 DisableVideoLayer**: the rotate-off sequence of §4.11.1 (without the sideband message), frees the layer's 16 rotate
  buffers, destroys the port instances (`InputPortDestroyInstance`; fails if a port is still enabled), `VideoLayerEnable(h, 0)`.
- **8 / 9 Set / GetVideoLayerAttr `{layer, attr(0x14)}`**: a pixel format > 0xb becomes 0 (silently);
  `MHAL_DISP_VideoLayerSetAttr(h, {win, size, fmt})`; stored (layer +0x010 … +0x020) for Get.
- **0xa / 0xb Bind / UnBindVideoLayer `{layer, dev}`**: `MHAL_DISP_VideoLayerBind` / `UnBind(lh, dh)`; the layer is added to /
  removed from the device's list (binding twice → `0x1F`).
- **0xd / 0xe Set / GetPlayToleration `{layer, u32 ms}`**: layer < 4; stores / returns the u32 at layer +0x4; no check and no
  other effect here — the CHECK_PTS branch of the enqueue path reads it (§4.10.1).
- **0x12 / 0x13 SetVideoLayerAttrBegin / End**: return 0, nothing else.

#### 4.8.3 Input port
- **0x14 SetInputPortAttr `{layer, port, attr(0xc)}`**: stores the display window (+0x44) and the source size (+0x4c); applied
  with the next frame. **0x15 GetInputPortAttr**: port checks; returns the display window (4 × u16) and the source size as
  stored by 0x14 / 0x18.
- **0x16 EnableInputPort `{layer, port}`**: the layer bound to dev ≤ 1; `mi_sys_SetInputPortBufExtConf(sysdev, chn, 0, 0,
  {0x10, 2, 0})` — `{0x80, 0x40, 0x40}` when the layer has a rotate mode (ext-conf: `mi_sys-1-engine.md` §4.2) —,
  `mi_sys_EnableChannel(sysdev, chn)`, `mi_sys_EnableInputPort(sysdev, chn, 0)`, chn by §4.9.2; port enabled, first-frame flag
  set.
- **0x17 DisableInputPort**: waits for the double-buffered registers to be latched (`disp_latch_wait`: up to 2 vsyncs, 50 jiffies
  each, on the device wait queue; skipped for MCU devices), `InputPortEnable(ph, 0)` + flip, `mi_sys_DisableInputPort`,
  `mi_sys_DisableChannel`; clears the port state; sync mode := 0.
- **0x18 / 0x19 Set / GetInputPortDispPos `{layer, port, {u16 x, u16 y}}`**: Set: port checks; size of the bound device's timing
  (§4.6.4); if both are non-zero and **x + disp_w ≥ W or y + disp_h ≥ H** → `0x1F` (a window touching the right or bottom edge is
  refused [R `<=`]); else (or if the size is unknown) x, y are stored into the display window and applied with the next frame.
  Get returns the stored x, y.
- **0x1a / 0x1b Pause / Resume, 0x1c / 0x1d / 0x1e Step / Show / Hide `{layer, port}`**: port checks, port enabled (else `0x1F`),
  then requested status (+0x2c) and current status (+0x30) := 1 pause / 2 resume / 3 step / 5 show / 6 hide. What each status
  does is decided in the enqueue path (§4.10.1).
- **0x1f SetInputPortSyncMode `{layer, port, mode}`**: mode 1 CHECK_PTS or 2 FREE_RUN (the app's); **accepted only while the port
  is enabled** (else `0x1F`) — so it must follow EnableInputPort (the app's `St_DispModuleInit` calls it last, `prog_uvc.md`
  §5.2).
- **0x20 QueryInputPortStat `{layer, port, {u8 bEnable, u32 eStatus}(out)}`**: port checks; returns enabled and the requested
  status (+0x2c). Always 0.
- **0x21 SetZoomInWindow `{layer, port, MI_DISP_VidWinRect_t crop}`**: port checks; w and h are first clamped to the port's source
  size; valid if x ≤ src_w, y ≤ src_h, x + w ≤ src_w, y + h ≤ src_h → the crop (+0x3c) is stored when it differs, "crop on"
  (+0xc) := 1, 0. Invalid → crop := 0, crop on := 0, `0x1F`. The crop is intersected with the frame's own crop in the enqueue
  path (§4.10.1).
- **0x2a ClearInputPortBuffer `{layer, port, bClrAll}`**: `bClrAll = 1`: the latch wait, port off in hardware,
  `mi_sys_DisableInputPort` + `EnableInputPort` (drops the queued buffers), first frame re-armed; otherwise only the "clear
  all" flag (+0x09).

#### 4.8.4 Picture parameters (all unused on this camera)
**0x22–0x29 VGA / HDMI / LCD / CVBS parameters `{dev, param}`** (requirements on the interface mask of §4.2.1):

| nr | requires | ranges (else `0x1F`) | mhal call / payload | stored in |
|---|---|---|---|---|
| 0x23 SetVgaParam | dev created; VGA bit | luma, contrast, hue, saturation ≤ 100; gain, sharpness ≤ 256 | `DeviceSetVgaParam({matrix, luma, contrast, hue, sat, gain, sharpness})` | VGA CSC record, gain[2], sharpness[2] |
| 0x22 GetVgaParam | VGA bit | — | — | returns the same 7 words |
| 0x25 SetHdmiParam | created; HDMI bit | luma, contrast, hue, saturation ≤ 100 (gain, sharpness unchecked) | `DeviceSetHdmiParam({matrix, luma, contrast, hue, sat, sharpness})` — **gain is not passed** | HDMI CSC, gain[5], sharpness[5] |
| 0x24 GetHdmiParam | HDMI bit | — | — | 7 words |
| 0x27 SetLcdParam | created; any of BT656, BT1120, LCD, TTL, MIPIDSI, TTL_SPI_IF, SRGB, MCU, MCU_NOFLM (mask 0x7e58) | 4 × ≤ 100, sharpness ≤ 256 | `DeviceSetLcdParam({matrix, luma, contrast, hue, sat, sharpness})` | LCD CSC (slot 6), sharpness[6] |
| 0x26 GetLcdParam | same mask | — | — | 6 words |
| 0x29 SetCvbsParam | created | — | `DeviceSetCvbsParam(&u8 bEnable)` (1 byte, `mhal-disp.md` §2.2) | not stored |
| 0x28 GetCvbsParam | dev < 2 | — | — | **returns 0 and writes nothing** |

The CSC matrix value is passed unchecked (mhal maps > 6 to 7 = invalid, `mhal-disp.md` §2.2). A Set succeeds only when mhal
returns non-zero; the stored copy changes only then. The proc command `csc` (§4.13) writes the same records.

**0x2c / 0x2d DeviceSet / GetColorTempeture `{dev, MI_DISP_ColorTemperature_t (6 × u16)}`**: Set: dev created;
`MHAL_DISP_DeviceSetColorTempeture(dev, copy of the 12 bytes)` (no range check; header: offsets, colours 0x00–0xFF with 0x80 =
unchanged); on success stored at dev +0x188. Get: dev < 2 (created not checked) → the stored copy (all zero until the first
Set — not the neutral 0x80 values).

**0x2e DeviceSetGammaParam `{dev, {u8 bEn, u16 u16EntryNum, u8 *R, *G, *B}}`**: dev created; three `CamOsMemAlloc(N)` (any
failure → `0x1F`), `CamOsCopyFromUpperLayer` of N bytes from each user pointer, `MHAL_DISP_DeviceSetGammaParam(dev, {bEn, N, kR,
kG, kB})` (must return 1), then the caller's record — with its user pointers — is stored at dev +0x194, and the kernel copies
are freed. No check of N against the hardware table size (mhal's MACE gamma, `mhal-disp.md` §12.3) [R for mi_disp].

### 4.9 Devices, layers, ports and mi_sys channels

#### 4.9.1 What is generic
Nothing in mi_disp special-cases device 0 except the LCD ISR's flag handling (§4.5), the DMA / WBC direction (§4.12) and the PQ
hand-over at init (device argument 0, §4.3). Every per-device path takes the device id and indexes the 0x418 record; every
layer / port path takes layer < 4 and port < 16. The layer ↔ device relation is **only** what BindVideoLayer establishes (any
layer can be bound to any device); layer ids 0 / 2 get 16 ports and 1 / 3 one — the port count depends on the layer id, not on
the device [R `EnableVideoLayer`: count = 16 if `(layer & ~2) == 0`]. mhal ties layers 0 / 1 to the MOP of device 0 and 2 / 3 to
device 1 (`mhal-disp.md` §11.1), so binding layer 2 to device 0 is accepted here and resolved (or refused) by mhal [I].

#### 4.9.2 mi_sys channel ↔ (layer, port) [R `_MI_DISP_EnqueueInputTask`, `OnBind` / `OnUnBindInputPort`]
| mi_sys device (= DISP device) | channel | layer, port |
|---|---|---|
| 0 | 0–15 | layer 0, port = channel |
| 0 | 16 | layer 1, port 0 |
| 1 | 0–15 | layer 2, port = channel |
| 1 | 16 | layer 3, port 0 |

Fixed in code: the layer used for a frame is chosen from the mi_sys device, **not** from the layer → device binding, so a frame
bound to `{DISP, dev 1, chn 3}` always lands on layer 2 port 3. `OnBindInputPort` stores the source in port +0x54 with the same
map (channel > 16 → layer 0 port 0). EnableInputPort uses the same map in reverse (chn = port for layers 0 / 2, 16 for 1 / 3)
for `mi_sys_SetInputPortBufExtConf` / `EnableChannel` / `EnableInputPort`. The app's bind SCL → `{DISP, dev 0, chn 0, port 0}`
therefore feeds layer 0, port 0.

### 4.10 Frame path (mi_sys task callbacks)
mi_sys (the bind engine) hands each input buffer of an enabled channel to the device's work thread, which calls the
callbacks below; the task result codes (0 queued, 1 done, 2 retry later, 3 drop) are mi_sys's (`mi_sys-1-engine.md` §4.1).
- `OnBind` / `OnUnBindInputPort`: remember / forget the bound source per port (+0x54).
- `OnPreProcessInputTask`: clears byte +4 of the task.
- `EnqueueInputTask`: §4.10.1. `BarrierInputTask`: kicks the CMDQ of the previous frame and marks a barrier outstanding
  (dev +0x410). `CheckInputTaskStatus`: §4.10.3. `DequeueInputTask` returns 1, `CheckOutputTaskStatus` 1, `RejectCmdq` and the
  SRAM callbacks as in §4.3 — glue for mi_sys's scheduler.

#### 4.10.1 `_MI_DISP_EnqueueInputTask` (0x16cac) [R: file `.text` 0x6c84…0x864c]
<!-- ev:mi_disp-mi_hdmi.4.10.1 -->
1. Reject (return 3) if the port has no instance, the buffer type is neither FRAME (1; **pixel format must be NV12 = 0xb**) nor
   META-frame (2), or the layer rotates and the frame comes from a bound source without the tile acknowledgement of §4.11.1;
   record the frame size / PTS for proc.
2. Under the port semaphore and then the device semaphore (always taken — the dev id < 2): port not enabled → 3 (drop; logged
   with the buffer address when `trace` is on); "clear all" pending with a pending or on-screen task → 2 ("wait clear task,
   retry later").
3. Port status (+0x30):

   | status | effect on this frame |
   |---|---|
   | 1 pause | return 2 (the buffer is kept; mi_sys retries) |
   | 3 step | status := 1 (pause), step count (+0xcc) ++, then the frame is shown normally — one frame per Step ioctl |
   | 5 show | if a frame is on screen: `InputPortEnable(port, 1)`; status := 0; the frame is shown |
   | 6 hide | `InputPortEnable(port, 0)`; status := 0; the frame is still flipped (the port stays hidden until Show) |
   | 0 / 2 | nothing |

   Status 0 ("none") after show / hide means `QueryInputPortStat` then reports 0, not 5 / 6.
4. **CHECK_PTS** (sync mode 1): expected time = last vsync + frame period ± toleration (layer +0x4; 0 = half a frame period); too
   early → 2, too late → 3 (drop); a drift estimate (port +0x0d…+0x20) is re-centred every 200 frames; the vsync-interval monitor
   of §4.10.4 runs. FREE_RUN (2, the app's) skips this step.
5. Before the flip, each → 2 with a debug-only message: the device has a barrier outstanding (dev +0x410 ≠ 0, set by
   `BarrierInputTask` = the previous frame's CMDQ kick, cleared by `CheckInputTaskStatus`); for non-MCU interfaces, more than ⅓
   of the frame period (dev +0x158) has passed since the last vsync ("timezone is not safe", `.LC304`, source line 0x6fd — the
   frame waits for the next vsync); the port already has a pending task ("abnormal case").
6. **Display rectangle** (`MHAL_DISP_InputPortAttr_t` `{u16 x, y, w, h; u16 srcW, srcH}` built at sp+0x98, `mhal-disp.md` §2.2;
   the attribute block starts at file 0x7098 with the memset of the flip payload and the timing-size lookup; window code file
   0x7188…0x71e0; the payload goes to `MHAL_DISP_InputPortSetAttr` at file 0x73fa):
   - timing size: `GetTimingSize(dev->timing byte + 1, &tW, &tH)` (file 0x3044) maps the device's stored timing to its active
     size (§4.6.4 "size"); if either is 0 a debug line (source line 0x562) is printed and the code continues with the zero;
   - source size (srcW, srcH at sp+0xa0): from the frame (FRAME buffers: width +0x38 / height +0x3a of the buffer info; META: the
     meta record +0x38 / +0x3a), the width rounded up to even, then capped to `stride / bytes-per-pixel` (2 for YUYV, 4 for ARGB /
     ABGR, 1 otherwise) when the stride is smaller;
   - window = the port's display window `{x, y, w, h}` (+0x44…+0x4a):

     | condition (as compared in the code) | window passed to mhal |
     |---|---|
     | w = 0 or h = 0 | `{0, 0, srcW, srcH}` |
     | tW = 0 or tH = 0 (size unknown) | `{x, y, w, h}` unchanged |
     | `x + w ≤ tW` **and** `y + h ≤ tH` (signed compares of the zero-extended u16s) | `{x, y, w, h}` unchanged |
     | otherwise | `{x, y, tW − x, tH − y}` |

     In the last row both sizes are replaced even when only one overflows (a window that overflows only horizontally gets its
     height stretched or shrunk to `tH − y`, and vice versa), and the subtraction is stored as u16 with no further check: an
     origin beyond the timing (x > tW or y > tH) gives a wrapped size near 65535 [R stores at file 0x71cc…0x71e0]. What mhal
     does with such a window is not known (§7).
   - **crop** (`InputPortSetCropAttr`, sp+0x90), computed afterwards and independently: the frame's own crop (FRAME buffers
     +0x7c…+0x82; META: the meta record) when it lies inside the frame, intersected with the port crop (+0x3c) when "crop on",
     all values rounded to even.
   **Every port flips independently**; there is no z-order or overlap handling in mi_disp (mhal composes the MOP windows,
   `mhal-disp.md` §11).
7. Rotation: 90° / 270° → `InputPortRotate` + `InputPortSetImiAddr` (§4.11.1); 180° → the mi_gfx copy (§4.11.2).
8. `MHAL_DISP_InputPortSetAttr(ph, {win, size})`, `InputPortSetCropAttr(ph, crop)`, **`MHAL_DISP_InputPortFlip(ph, frame)`**
   (file 0x740e) with `MHAL_DISP_VideoFrameData_t` (`mhal-disp.md` §2.2): FRAME buffers copy pixel format, compress mode, tile
   mode, three physical addresses and three strides from the mi_sys frame; META buffers take the format (+4), addresses
   (+0x18…+0x24) and strides (+0x28 / +0x2c) from the meta record.
   - Flip returns 1: first frame → `InputPortEnable(ph, 1)`; the task becomes the port's **pending** task (+0x5c), tagged with
     the vsync counter (task +8 := dev +0x10); dev "fired" count (+0x40c) ++; port flip time and flip count updated; return
     **0** (queued).
   - Flip returns anything else (failure path 0x7cfe): `CamOsTsemUp` × 2, the message `[MI ERR ]: %s[%d]: [dev:%d][chn:%d] frame
     flip falied` (sic, `.LC315`, source line 0x73e) **only when the debug level is ≠ 0 or the function / file debug filter
     matches**, and return **2** (`movs r3, #2 … mov r0, r3` at 0x6fa0; retry: the buffer stays with mi_sys and the task
     is offered again on its next pass). Nothing else is undone: the attribute and crop already handed to mhal stay set and the pending slot stays empty. A flip that keeps
     failing therefore becomes a retry loop at the pass's wake-up rate with no error reaching user space; mi_disp never drops the
     frame itself [I].

#### 4.10.2 One buffer on screen, one pending
The task on screen (+0x60) is released only when the pending task (+0x5c) replaces it (§4.10.3), so DISP keeps **one buffer on
screen + one pending** per port [I] — except for the 3-second refresh of §4.10.3.

#### 4.10.3 `_MI_DISP_CheckInputTaskStatus` (0x15f3c) — polled by the work thread after vsync wake-ups
- **Without CMDQ**: when the vsync count has passed the pending task's tag → pending becomes on screen; the previously shown task
  is returned with 3 (released to mi_sys); still on screen → 1.
- **CMDQ mode**: the pending task becomes on screen when the device's vsync count has passed the barrier's (dev +0x414) **and**
  the CMDQ's hardware dummy register (service +0x14) has reached the task's fence (task +0x4c, 16-bit wrap); otherwise the two
  "fence is not updated" diagnostics.
- **Disabled device or port / clear pending**: the checked task is simply detached (pending and / or on-screen slot cleared,
  barrier count − 1) and released ("drop input buff").
- **3-second refresh**: when the port has an on-screen task but no pending one, mi_sys has no further buffer queued for that
  channel (`disp_chn_buf_count` (0x14394) = queued + in-use − 1 from `mi_sys_GetChnBufInfo` [name I]) and the last flip is more than 3 000 001 µs
  ago, the on-screen task is **released while the hardware keeps scanning its address** ("refresh input buff", counter +0x84) —
  unless `stopgetbuff` is on for the port. A still picture held longer than 3 s therefore points at a buffer mi_sys may reuse
  [I: visible only if the producer writes into it again].
- The first on-screen frame of a port: `mi_sys_RecodeTimeStamp(0x83d, "DISP_FirstFrameDone")` once.
- Per-port statistics: release count (+0xc4), drop count (+0xc8), frames per second (+0x8a), and the `dumpframe` hook (§4.13)
  that writes the released frame to a file.

#### 4.10.4 Vsync-interval monitor (CHECK_PTS ports only)
On each CHECK_PTS frame the device accumulates the interval between the last two vsyncs (dev +0x154, count +0x150); every 200
samples the average is compared with the frame period and, if it differs by ≥ 10 %, logged "real interrupt interval is %dus,
but setted timing is %dus!!". Diagnostics only.

### 4.11 Rotation (unused) [R `MI_DISP_IMPL_SetVideoLayerRotateMode` 0x1c5d0, `_MI_DISP_EnqueueInputTask`, `_MI_DISP_RotateByGfx`]
The rotate mode is per **layer** (layer +0x24). There are two mechanisms; both need the producer's tiled output for bound
frames (below).

#### 4.11.1 90° / 270°: tiled input + MOP rotate from SRAM
ioctl 0x2b `{layer, {mode}}`: layer < 4, created, bound to device ≤ 1; layer semaphore.
- mode ≠ 0 and ≠ 2: create port 0's mhal instance if the layer is not enabled yet; `mi_sys_SetInputPortSidebandMsg(sysdev, chn
  (0 for layers 0 / 2, 16 for 1 / 3), 0, 0, mode & 0x3ff, 0x40000000)` — a sideband message of **type 2 = "deliver tiled
  frames"** to the producer bound to that channel (format, merge and producers: `mi_sys-1-engine.md` §5; the rotate mode in the
  low word is read by no producer); on platform "M6" (from `Chip_Get_Platform_Name`) and when no other layer is rotating: SRAM
  request := {need 1, size **0x7800**, keep 0, preempt 0, movable 0} (`_stModRes` + 0x4c1c…, returned by the SRAM callbacks of
  §4.11.3 and used by mi_sys, `mi_sys-1-engine.md` §9); rotating-layer mask (`_stModRes` + 0x4c28) |= 1 << layer; store the
  mode. In the enqueue path, while the SRAM request is set and the layer's mode is 1 or 3: `MHAL_DISP_InputPortRotate(port,
  {mode})` and `MHAL_DISP_InputPortSetImiAddr(port, task word 0x16)` (the SRAM address mi_sys put into the input task) before
  the flip. **The display rotates; the producer does not.**
- Bound frames on a rotating layer: `_MI_DISP_EnqueueInputTask` () rejects a frame (return 3, ERR
  "[dev:%d][chn:%d] sideband msg is not correct") when the layer's rotate mode is **non-zero** and the port has a bound source
  (+0x54), unless the high word of the frame's sideband (`MI_SYS_BufInfo_t` +0x0c) has **bits 31..29 = 2 and bit 28 (ACKED)
  set** — the stamp of a producer that switched to the tiled layout. The test applies to mode 2 (180°) too, which sends no
  request, so a 180° layer rejects bound frames unless the producer still stamps type 2 + ACK [I: consequence of the condition
  as read]. Which producers can stamp it (mi_scl on HWSCL5 with NV12 output, mi_vdec) and which do on this camera
  is `mi_sys-1-engine.md` §5.3, §5.5.
- mode 0: if the layer was rotating: port 0 off with the latch wait (`disp_latch_wait`), `InputPortRotate(port0, {0})` + flip +
  latch wait (`disp_rotate_latch_wait` (0x18cb4)), mode := 0, sideband message with all zero, mask bit cleared, SRAM "need" := 0 when the mask is
  empty.
- `DisableVideoLayer` does the same rotate-off sequence but **sends no sideband message**.

#### 4.11.2 180°: software HV-flip with mi_gfx
Mode 2 is only stored by the ioctl (no sideband message, no SRAM). In the enqueue path, for a port with a known source size
(port +0x4c non-zero): a per-(layer, port) MMA buffer `"disp_rotate"` (`gstRotateBufConf[layer][port]` = {u64 base, u64 next,
u32 size}) of **⌈src_w/16⌉ × ⌈src_h/2⌉ × 128 bytes** (= 2 × (src_w rounded to 16) × src_h, i.e. two NV12 frames with room) is
(re)allocated when the size changes; the frame is copied with `_MI_DISP_RotateByGfx` — two `MI_GFX_BitBlit` HV-flip blits
(`SstarBlitHVFlip`: mirror flags 2, rop / flags 1, colour keys 0xff000000) of the Y plane as an 8-bit surface (gfx format 3,
w × h) and the UV plane as a 16-bit surface (format 8, w/2 × h/2), each followed by `MI_GFX_WaitAllDone` (gfx structs:
`gfx-ge.md` §3) — into the **other half** of the buffer than last time (ping-pong), and that half is flipped as NV12
(`ePixelFormat` 0xb). Allocation failure → "there no mem for gfx to rotate 180" and the flip goes out with an all-zero frame
descriptor (format 0, addresses 0) [R: jumps to the flip with the zeroed payload]. The buffers are freed by `DisableVideoLayer`
(all 16 of the layer) — rotate mode 0 frees them only through it.

#### 4.11.3 SRAM callbacks and the proc command
`OnConfigSramBuf` returns {need (+0x4c1c), size (+0x4c20), keep (+0x4c24)}, `OnAllowPreemptionSram` +0x4c25,
`OnAllowMoveableSram` +0x4c26 — all module-wide (one request for both devices). Proc `rotate <layer> <0|1|2>` maps 1 → 90°,
2 → 270°, anything else → none (180° cannot be set from proc).

### 4.12 WBC — write-back capture of a display device (unused) [R `MI_DISP_IMPL_*WBC*`, `_MI_DISP_Wbc*`, `disp_wbc_start` (0x1565c), `disp_wbc_stop` (0x157bc)]
The DISP DMA (mhal bank 1158, `mhal-disp.md` §14) writes what a device shows (video + UI, or video only) back to memory, and
mi_disp offers it as the **output port** of an extra mi_sys device, so another module (VENC, …) can bind to it. One WBC object
(id 0 only: every WBC ioctl rejects id ≥ 1 with `0x1F`). The DMA is always bound "source device → the other device"
(`{dev d, dev 1 − d}`); mhal's DMA captures the source device into memory to mirror it onto the other device (`mhal-disp.md`
§14).

#### 4.12.1 Object (`_stModRes` + 0x4b78)
| off | field (proc `mi_wbc0` names) |
|---|---|
| +0x00 | enabled (EnableWBC / Attach) |
| +0x01 / +0x02 / +0x03 | source set / attribute set / attribute changed (re-program the DMA at the next frame) |
| +0x04 | DMA IRQ requested (`irq enable`) |
| +0x05 | DMA unbound (no completion checks while set) |
| +0x08 / +0x0c | mhal DMA ctx / mi_sys handle of the WBC device |
| +0x10 / +0x14 | DMA IRQ number / ISR count (`irqnum`, `isrcnt`) |
| +0x18 / +0x1c | source size = the source device's timing size, §4.6.4 (`srcw`, `srch`) |
| +0x20 / +0x24 | source type (`MI_DISP_WBC_SourceType_e`: 0 DEV = video + UI, 1 VIDEO) / source device (`src type`, `src dev`) |
| +0x28 / +0x2c / +0x30 | target width / height / pixel format (`targetW`, `targetH`) |
| +0x34 / +0x38 / +0x3c | spare task slot (only cleared) / fired task / "on screen" task (`FiredTask`, `OnScreenTask`) |
| +0x40 | semaphore |
| +0x60 / +0x61 / +0x62 | fps / dump-next-frame flag / dump directory (proc `dumpframe`) |

#### 4.12.2 The WBC mi_sys device — two registrations
- **EnableWBC path** (`disp_wbc_stop`): `mi_sys_RegisterDev({module **0x23**, dev 0, 0 input ports, 1 output port, 1 channel,
  1 pass}, wbc, procfs, dir)` → proc file `mi_wbc0` with the command `dumpframe <dir>`, then `MHAL_DISP_DmaCreateInstance(hooks,
  0, &wbc.dma)` and the CMDQ register path for it.
- **DeviceAttach path** (`disp_wbc_start`): the same but registered as **module 0xF, dev 3** and without the `dumpframe` command.
  Which of the two exists depends on what the caller did first; neither is unregistered before DeInit.
Pass callbacks (both): `NeedIndividualWorkThread` 1, `RejectCmdq` (CMDQ type **0x20000**), `OnPollingAsyncOutputTaskConfig`
(its presence gives the device mi_sys's ASYNC thread, `mi_sys-1-engine.md` §6.5), `EnqueueOutputTask` =
`_MI_DISP_WbcEnqueueAsyncOutputTask`, `BarrierOutputTask` = `_MI_DISP_WbcBarrierAsyncOutputTask`, `CheckOutputTaskStatus`,
`DequeueOutputTask` (1), output-port bind capability (0x11), `OnBind` / `UnBindOutputPort` (0). Module id 0x23 is one of the ids
mi_sys lets own a CMDQ (`mi_sys-1-engine.md` §7.1).

#### 4.12.3 WBC ioctls
| nr | call | behaviour |
|---|---|---|
| 0x2f | SetWBCSource `{wbc, {type, dev}}` | dev < 2 and **enabled**; source size := that device's timing size; type, dev stored; source set := 1 |
| 0x30 | GetWBCSource | returns type, dev |
| 0x31 | SetWBCAttr `{wbc, {w, h, fmt}}` | source set required; **no up-scaling**: w ≤ src w and h ≤ src h, else `0x1F` ("WBC can not support scaling up"); stores the target, attribute set := changed := 1 |
| 0x32 | GetWBCAttr | returns w, h, fmt |
| 0x33 | EnableWBC `{wbc}` | attribute set required (else `0x1F`); create the WBC device + DMA instance if missing (§4.12.2; still none → `0x1F`); `MHAL_DISP_DmaBind({source dev ctx, the other dev ctx})`; DMA IRQ once (`GetDmaIrq`, `CamOsIrqRequest(irq, _MI_DISP_WbcISR, "mi_disp_wbc_isr")`, `EnableDmaIrq(1)`); `mi_sys_EnableChannel(wbc, 0)` + `EnableOutputPort(wbc, 0, 0)`; enabled := 1, unbound := 0 |
| 0x34 | DisableWBC `{wbc}` | disable channel + output port, enabled := 0; wait ≤ 999 jiffies for the fired task to clear; CPU path, DMA IRQ off + free, `DmaUnBind`, CMDQ path, unbound := 1; then wait ≤ 1000 jiffies for the other slots; 0 (timeouts only logged) |

#### 4.12.4 DeviceAttach / DeviceDetach (ioctls 4 / 5) `{src dev, dst dev[, MI_DISP_DevAttachAttr_t]}` — mirroring through WBC
- **Attach**: both devs < 2 and created, both enabled, **both devices have at least one bound layer** [R list checks]; size of
  the source's timing (§4.6.4 "size"); for the first layer bound to the destination whose id is 0 or 2: lazily create the DMA
  instance (`disp_wbc_start`, §4.12.2); if a DMA instance exists: under the WBC semaphore fill the WBC request — target w×h =
  source timing size, `bVideoOnly` = (attach mode ∈ {0 DEFAULT, 2 VIDEO_MODE}) [R: `(mode & ~2) == 0`], source frame size =
  that layer's port-0 src w / h (port +0x4c / +0x4e), pixel format 0xb (NV12), two flag bytes 1 —, `MHAL_DISP_DmaBind({src ctx,
  dst ctx})`, DMA IRQ requested once (`GetDmaIrq` → `CamOsIrqRequest(irq, _MI_DISP_WbcISR, "mi_disp_wbc_isr")` →
  `EnableDmaIrq(…, 1)`), attached := 1; if the WBC mi_sys device exists: `mi_sys_EnableChannel(wbc, 0)`,
  `mi_sys_EnableOutputPort(wbc, 0, 0)`. Returns 0 even when no DMA instance could be made.
- **Detach**: both devs < 2; disable the WBC channel / port, attached := 0, wait ≤ 999 jiffies for the in-flight WBC task counter
  to reach 0; under the semaphore: CPU register path, DMA IRQ off + free, `MHAL_DISP_DmaUnBind({src, dst})`, CMDQ path,
  "unbound" := 1; then wait ≤ 1000 jiffies for both WBC counters; always returns 0 (timeouts only logged). The DMA instance itself
  survives until DeInit.

#### 4.12.5 Frame cycle (mi_sys ASYNC thread of the WBC device)
1. `OnPollingAsyncOutputTaskConfig`: if a DMA instance exists and WBC is enabled → "produce one output": flag +4 := 1, +0x18 := 1,
   size = target w × h, pixel format **0xb (NV12) always** (the stored target format is not used here), compress 0.
2. `EnqueueOutputTask` (only if no task is in flight): attribute changed → `MHAL_DISP_DmaSetAttr({in type = 1 for DEV / 0 for
   VIDEO, in format 0x37, compress 0, in size = source size, out access 1, out mode 0 (frame), out format = target fmt, compress
   0, out size = target})` and "changed" cleared; then for the task's output buffer `MHAL_DISP_DmaSetBufferAttr({bEn 1, Y and UV
   physical addresses (buffer +0x48 / +0x50), strides (+0x60 / +0x64), ring fields 0})`, `SetRegFlipConfig({1, cmdq})`; task tag
   := ISR count; fired := task.
3. `BarrierOutputTask`: `SetRegWaitDoneConfig(dma, {type, cmdq})`.
4. DMA IRQ (`_MI_DISP_WbcISR`): flag → clear → ISR count ++ → `mi_sys_NotifyWakeupISREvent(wbc, 0)`.
5. `CheckOutputTaskStatus(cmdq, task)`: when the fired task's tag < ISR count — without CMDQ at once, with CMDQ once the CMDQ
   dummy register reached the task fence (+0x54) — fired → "on screen". A task that is still one of the three slots → 0 (busy);
   any other → 1 (done; the `dumpframe` hook writes the last "on screen" buffer to
   `<dir>/dump_disp_wbc0_size<stride>_<h>_frame.yuv`). So a written buffer is handed to the consumer **one DMA frame later** than
   it was written [I from the slot rule].

### 4.13 procfs [R `_MI_DISP_Debug*`, `_MI_DISP_OnDump*`, `_MI_DISP_OnHelp`]
Files: `/proc/mi_modules/mi_disp/mi_disp0` and `mi_disp1` (one per DISP mi_sys device; the commands are registered on both,
and each acts on the ids given in its arguments, not on the file it was written to), `mi_wbc0` (§4.12.2, EnableWBC path only).
`cat` prints the mi_sys generic part and then the module's dump callbacks; `echo <cmd> args > file` runs a command (argument
count = words including the command name; too few → "Unsupport command count: %d."). The number registered with
`mi_sys_RegistCommand` is the **maximum number of arguments after the command word**; more are refused by mi_sys's proc
writer (`_MI_SYS_IMPL_Common_WriteProc`) before the handler runs; the handler gets `(print, ctx, dev id of the proc file, argc
incl. the command, argv, handle)` [R mi_sys decompile 0x4e92c]. 17 commands, all diagnostics.

**`cat mi_disp<d>`** — `_MI_DISP_OnDumpDevAttr` + `_MI_DISP_OnDumpInputPortAttr`:
- Device block: `DevStatus` (enabled), `IrqNum`, `IrqCnt` (vsync counter), `BgColor` (hex); then **for every interface bit ever
  set** a row `Interface` (name, §4.6.1 order; 13 / 14 print `MCU` / `MCU_NOFLM`), `DevTiming` (MI timing name of §4.6.4, `user`
  for 40), `CscMatrix`, `Luma`, `Contrast`, `Hue`, `Saturation`, `Sharpness`, `Gain` from that interface's records.
- Per bound layer: `LayerId`, `BindedDevID`, `LayerWidth`, `LayerHeight`; `LayDispWidth`, `LayDispHeight`, `Toleration`,
  `rotatemode` (`NONE`, `rotate_90/180/270`); per enabled port: `enable`, `CurStatus`, `src_w/h`, `crop_x/y/w/h`, `show_x/y/w/h`;
  `RecvBufCnt`, `RecvBuf_W/H`, `Content_W/H`, `RecvBufStride`, `PixFmt` (names for 0, 1, 2, 4, 5, 7–11), `syncmode` (`Invalid` /
  `CheckPts` / `FreeRun`); `OnScreenTask`, `FiredTask` (pointers), `StepTaskCnt`, `bClearAllTask`, `fps`.
- `OnDumpChannelAttr`, `OnDumpOutPortAttr`: empty. A device id ≥ 2 → `0x1F`.
- `cat mi_wbc0`: `irqnum`, `isrcnt`, `irq enable`; `src dev`, `srcw`, `srch`, `src type`, `targetW`, `targetH`; `OnScreenTask`,
  `FiredTask`, `fps`. Help: `dumpframe [path]`.

**Commands** (`echo … > /proc/mi_modules/mi_disp/mi_disp<d>`; words as registered):

| command (words) | arguments | effect |
|---|---|---|
| `getcapframe` (3) | devid layerid path | allocates MMA `"disp-screenLayer"` of layer w × h × 2 bytes, maps it and **writes the never-filled buffer** to `path` — no capture is performed; the MMA buffer and the mapping are never freed on success (leak) [R: no DMA / GFX call, `MMA_Free` only on the map-failure branch] |
| `checkframepts` (3) | layer port ON\|OFF | port must be enabled; ON prints one line per received buffer (`receive buffer ID`, PTS) from the enqueue path |
| `dumpframe` (3) | layer port dir | the next released frame of that port is written to `dir/dump_disp_layer<l>_inport<p>_size<stride>_<h>_frame.yuv` (`filp_open` O_CREAT\|O_TRUNC\|O_WRONLY 0644; FRAME: `mi_sys_Vmap` of the Y address, size +0x6c; META: Y + UV sizes) |
| `stopgetbuff` (3) | layer port ON\|on\|1 / OFF\|off\|0 | stop releasing the on-screen buffer (blocks the 3-second refresh of §4.10.3) |
| `bgcolor` (2) | devid value (decimal) | `MHAL_DISP_DeviceSetBackGroundColor` and store |
| `csc` (8) | devid CscMatrix Contrast Hue Luma Saturation Sharpness [Gain] | for each of HDMI, LCD (bit 6 only here) and VGA the device has: Get…Param, replace the fields, Set…Param (VGA also takes Gain) — through the range checks of §4.8.4 |
| `colortemp` (7) | devid BlueOffset GreenOffset RedOffset BlueColor GreenColor RedColor | `MI_DISP_IMPL_DeviceSetColorTempeture` |
| `rotate` (2) | layer 0\|1\|2 | 1 → 90°, 2 → 270°, else none (§4.11.3) |
| `crop` (6) | layer port x y w h | `SetZoomInWindow` (§4.8.3) |
| `hide`, `show`, `pause`, `resume`, `step` (2) | layer port | the IMPL functions of §4.8.3 |
| `clear` (2) | layer port | `ClearInputPortBuffer(l, p, bClrAll 0)`; `clear all 1` → `DisableInputPort` on all 64 ports; `clear all <other>` → `EnableInputPort` on all 64 |
| `syncmode` (3) | layer port 0\|1 | sync mode := 2 FreeRun for "0", 1 CheckPts for "1", 0 otherwise — **written directly, without the "port enabled" check of ioctl 0x1f** |
| `trace` (1) | (none) | toggles the trace flag (`gbRejectCmdq` + 1): on → prints per layer / port `enqueuetask`, `finishtask`, `droptask`, `rewindtask` counters, then every enqueue / release / drop / CMDQ kick is logged with timestamps |

### 4.14 Imports and exports
- mhal: 59 `MHAL_DISP_*` (device, video layer, input port, DMA, IRQ, register-access / flip / wait-done config, panel / mmap /
  PQ init) + `MhalPnl*` (`MhalPnlCreateInstanceEx`, `MhalPnlSetUnifiedParamConfig`, `MhalPnlDestroyInstance` — BT656 / BT1120
  set-up only).
- mi_sys: `mi_sys_RegisterDev`, `mi_sys_UnRegisterDev`, `mi_sys_RegistCommand`, `mi_sys_EnableChannel` / `DisableChannel`,
  `mi_sys_EnableInputPort` / `DisableInputPort`, `mi_sys_EnableOutputPort` (WBC; its disable counterpart is not named in the sources), `mi_sys_SetInputPortBufExtConf`,
  `mi_sys_SetInputPortSidebandMsg`, `mi_sys_GetChnBufInfo`, `mi_sys_NotifyWakeupISREvent`, `mi_sys_RecodeTimeStamp`,
  `mi_sys_MMA_Alloc` / `Free`, `mi_sys_Vmap` / `UnVmap`, `MI_SYSCFG_GetPanelInfo`, `MI_SYSCFG_GetMmapInfo`, `MI_SYSCFG_GetPqTbl`,
  logging. mi_gfx: `MI_GFX_BitBlit`, `MI_GFX_WaitAllDone` (180° rotation only). CamOs: Tsem, IRQ, time, memory,
  `CamOsCopyFromUpperLayer`; `Chip_Get_Platform_Name`; kernel: `filp_open` / `vfs_write` (the dump-to-file proc commands).
- Exports: none.

## 5. libmi_disp.so [R]
58 exports. The wrappers do no argument checks in user space and follow the packing of `mi-user-abi.md` §3.2; Get variants copy
back only on 0 unless stated. The 16 wrappers of the ioctls the app uses (bold in §4.7) are listed with their numbers and
payload sizes in `mi-user-abi.md` §3.3 (`MI_DISP_SetPubAttr`, `MI_DISP_Enable`, … `MI_DISP_ClearInputPortBuffer`); the other 42:

| function(s) | nr (size) | payload / copy-back |
|---|---|---|
| `MI_DISP_GetPubAttr(dev, *a)` | 3 (0x38) | {dev}; 0x34 back on 0 |
| `MI_DISP_DeviceAttach(src, dst, …)` / `DeviceDetach(src, dst)` | 4 (0xc) / 5 (8) | {src, dst[, word]} |
| `MI_DISP_Set/GetPlayToleration(layer, u32)` | 0xd / 0xe (8) | |
| `MI_DISP_SetVideoLayerAttrBegin/End(layer)` | 0x12 / 0x13 (4) | |
| `MI_DISP_GetInputPortAttr(layer, port, *a)` | 0x15 (0x14) | 3 words back on 0 |
| `MI_DISP_Set/GetInputPortDispPos(layer, port, MI_DISP_Position_t *p)` | 0x18 / 0x19 (0xc) | |
| `MI_DISP_Step/Show/HideInputPort(layer, port)` | 0x1c / 0x1d / 0x1e (8) | |
| `MI_DISP_QueryInputPortStat(layer, port, *s)` | 0x20 (0x10) | 2 words back |
| `MI_DISP_SetZoomInWindow(layer, port, rect*)` | 0x21 (0x10) | |
| `MI_DISP_Get/SetVgaParam`, `Get/SetHdmiParam` (dev, *p) | 0x22 / 0x23, 0x24 / 0x25 (0x20) | {dev, 7 words} |
| `MI_DISP_Get/SetLcdParam(dev, *p)` | 0x26 / 0x27 (0x1c) | {dev, 6 words} |
| `MI_DISP_Get/SetCvbsParam(dev, *p)` | 0x28 / 0x29 (8) | {dev, u8 enable} |
| `MI_DISP_SetVideoLayerRotateMode(layer, *cfg)` | 0x2b (8) | {layer, 1 word} |
| `MI_DISP_DeviceSetColorTempeture` / `DeviceGetColorTempeture` (dev, *c) | **0x2c / 0x2d** (0x10) | {dev, 3 words} |
| `MI_DISP_DeviceSetGammaParam(dev, *g)` | 0x2e (0x14) | {dev, 4 words — the LUT pointers inside are user addresses} |
| `MI_DISP_Set/GetWBCSource(wbc, *s)` | 0x2f / 0x30 (0xc) | |
| `MI_DISP_Set/GetWBCAttr(wbc, *a)` | 0x31 / 0x32 (0x10) | |
| `MI_DISP_Enable/DisableWBC(wbc)` | 0x33 / 0x34 (4) | |
| `MI_DISP_InitDev()` / `DeInitDev()` | — | InitDev only opens the fd (returns 0, or **0x1F** — a bare number — when the open fails); DeInitDev closes it unconditionally (no create count; `mi-user-abi.md` §2.2) |
| `mi_disp_GetScreenFrameSize(u32 *size, *w, *h, *stride)` | 0xf (0x30, kernel `GetOutputtiming`) | sends `{0}` — a **layer** id for the kernel (§4.8.1); on 0: (w, h) from the timing enum at +4 (`mi_disp_GetOutputResolution`: 0 / 5 / 7 → 720×576, 1 / 4 / 6 → 720×480, 2 → 960×576, 3 → 960×480, 8 / 9 / 0x25 / 0x26 → 1280×720, 0xA–0x10 / 0x23 / 0x24 → 1920×1080, 0x11 → 640×480, 0x12 → 800×600, 0x13 → 1024×768, 0x14 → 1280×1024, 0x15 → 1366×768, 0x16 → 1440×900, 0x17 → 1280×800, 0x18 → 1680×1050, 0x19 → 1920×2160, 0x1A → 1600×1200, 0x1B → 1920×1200, 0x1C → 2560×1440, 0x1F → 2560×1600, 0x21 / 0x22 / 0x27 → 3840×2160, 0x28 user → u16 at +0x24 / +0x26, else 0×0), then **capped to 1280×720** (`mi_disp_AdjustScreenFrameSize`); size = w·h·2 rounded up to 32, stride = 2w (YUYV) |
| `_mi_disp_GetScreenFrame` / `_mi_disp_ReleaseScreenFrame` | — | return **0xA00F8012** (not supported) |

The library's resolution table differs from the kernel's (§4.6.4) for MI 0x1d / 0x1e (2560×1440 _50 / _60) and 0x20
(3840×2160_25), which it maps to 0×0 [I: comparison of the two tables].

## 6. Behaviour observed on the camera [R]
- Hot-plug as the library sees it (§3.2): switching the monitor's input shows as a short unplug / replug.
- Output timings on a Philips monitor whose EDID offers 4K 24–60, 2560×1440@60, 1920×2160@60, 1080p50 / 60, 720p50 / 60, 480p
  and 576p, with the HDMI timing (§2.4), the DISP output timing (§4.6.4) and the layer / input-port sizes set to the same mode:
  picture at 480p60, 576p50, 720p50 / 59.94 / 60, 1080p24 / 25 / 30 / 50 / 59.94 / 60, 1440p30 / 50 / 60, 2160p25 / 30; **no
  signal at 2160p60 and 2160p29.97**. Why those two fail is not known (§7).
- [S] (SigmaStar Tiramisu MI_HDMI document `hdmi_zh` §1.1, §1.3, §3.6; MI_DISP `disp_zh` §3.5, §3.23, §5.1): the
  HDMI transmitter is HDMI 1.4; the Tiramisu column of the timing table marks 1080p24 / 25 / 30 / 50 / 60, 720p50 / 60, 576p50,
  480p60, the VESA modes 1280×1024, 1024×768, 1440×900, 1600×1200, 1280×800, 1366×768, 1680×1050 at 60 Hz, 3840×2160@30,
  2560×1440@30 and 1080 / 720 at 59.94 / 29.97 — no 2160p60, 2160p24 / 25, 2160p29.97, 1440p50 / 60. The HDMI timing must equal
  the DISP timing. DISP input ports scale up only, at most 16×; `E_MI_DISP_OUTPUT_3840x2160_2997` is marked «目前仅Tiramisu支持»
  (only Tiramisu at present). `MI_DISP_SetInputPortSyncMode` (FREE_RUN) is documented for MSR930 / MSR650x only. [R, from the
  list above] 2160p25 and 1440p50 / 60 give a picture though the table does not mark them.

## 7. What is not known
- How mhal (`MHAL_DISP_InputPortSetAttr`, the MOP) treats a window clipped to the timing, or one whose size wrapped to ~65535
  because its origin lies beyond the timing (§4.10.1) — `mhal-disp.md` §11.
- The 0x27 panel configs as built on the camera at init (§4.3) are not transcribed; they follow from the `DACOUT_*` records of
  `/misc/config.ini` (`config_tool.md` §4, §5), of which only the first eleven records were checked against the live kernel
  state (`config_tool.md` §5.6).
- Why 2160p60 and 2160p29.97 give no signal on the monitor of §6 while 2160p25 / 30 work.
- Whether DISP scales or shows the window smaller when the zoomed 4K picture reaches an input port fixed at 3840×2160 — a
  reading on a monitor (CAMERA-CHECKS K36).
- The default `debug_level` of `mi_hdmi` on the running camera (`/proc/mi_modules/mi_hdmi/debug_level`), which decides whether
  the ERR prints of §3.7 appear — not known statically (module parameter / proc value).
- Whether the mhal debug level 3 that mi_hdmi sets at Init (§2.3 nr 0) really puts the MDrv HDMI prints into the kernel log
  (CAMERA-CHECKS K34).
- The purpose of mi_hdmi's state word +0x44 (zeroed after every SetAttr) [I: "output colour actually used"].
