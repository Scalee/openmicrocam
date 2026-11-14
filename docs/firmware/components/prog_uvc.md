# prog_uvc — the vendor pipeline app (SigmaStar UVC demo with the ODM's additions)
<!-- ev:prog_uvc -->

`/customer/prog_uvc` is the one program on the camera that drives the media pipeline. It is SigmaStar's SDK UVC demo
`st_main_uvc.cpp` with its helper sources (`st_uvc.c`, `st_uvc_xu.c`, `st_uac.c`, `st_common.c`, `st_hdmi.c`), grown by the
ODM to about 7000 lines with the product's features inside the same file: the sensor → VIF → ISP → SCL pipeline, the USB
camera (UVC streaming, the camera-terminal / processing-unit controls, two extension units with an AES challenge-response, a
reboot-into-upgrade request, a field-update path, still images), HDMI output with an EDID-based 4K / 1080p decision, the
server side of the menu link (TCP 8888) with its saved settings `ws.bin`, picture functions (mirror, crop, zoom, split view),
photo to SD, MP4 recording and playback, a JPEG viewer, an unused LDC path and an unused UAC audio path. Every MI library it
calls is linked in statically, together with libispalgo, libcus3a, libmi_iqserver, libfbc_decode, libcam_os_wrapper and a
copy of tinyalsa.

Elsewhere: the start line in `demo.sh`, the boot order, the rcS register writes, the SD hot-plug scripts and the files in
`/customer` — `rootfs-customer.md` (§1 "Boot order", §3 "`/customer/demo.sh`", §4 "Hotplug and USB scripts"); the inventory
rows of prog_uvc and the libraries — `firmware-image.md` §3.5, §3.7; the UVC gadget modules, the descriptors and formats g_webcam offers,
the gadget side of the QBUF `reserved` word and the isochronous alternate settings — `usb-gadget.md` (§3.5, §3.6, §4); the MI
user-library ABI (ioctl wrapper, ioctl numbers per device, error codes, libcam_os_wrapper) — `mi-user-abi.md`; libcus3a,
libispalgo's user 3A, libmi_iqserver (including `/tmp/isp_uart` and `/tmp/raw_data.raw`) and libfbc_decode, of which prog_uvc
carries the static copies — `isp-6-userspace.md`; the ISP API ids and the API bin format loaded by `-i` — `isp-3-iq-api.md` §2, §6;
libmi_ai / libmi_ao, mi_ai, mi_ao and the audio HAL — `audio.md`; `ota_upgrade_status` and the `ufu` USB download in U-Boot —
`uboot.md` (§3.2 "Variables", §4.4 "`ufu` — USB download"); mi_ldc, libmi_ldc, the JSON and `CalibPoly_new.bin` formats —
`ldc-1-stack.md`; the menu-link client, `ws.ini` and `Line.ini` — `kjd_lvgl.md` §4, §3.3; mi_sys bind types and the bind contract —
`mi_sys-1-engine.md` §3, §2; mi_scl / mi_isp / mi_vif / mi_sensor — `mi-capture.md`; mi_disp / mi_hdmi and the display timing ids —
`mi_disp-mi_hdmi.md`; mi_venc — `mi_venc.md`; mi_jpd / mi_vdec — `mi_jpd-mi_vdec.md`.

Tags: **[R]** read from the binary, its decompile or disassembly, a file of the dump, an emulation of the binary's own code, or
observed on the running camera; **[S]** stated by a source (SDK demo source, header, the user); **[I]** inference.

Binary: `/customer/prog_uvc` (sha256 972ae0a6…53c1), ARM32 Thumb-2, stripped, dynamically linked, not
position-independent; GOT at 0xa9000 [R]. **Addresses** are prog_uvc virtual addresses (the values `readelf` prints) unless a library is named; `g_xxxxx` names the global at that address, and functions
without a symbol are named here by what they do [I].
The `.data` file offset is the virtual address − 0x20000 (`g_maxbuf_cnt` 0xa97b3 is file offset 0x897b3) [R].

## 1. What the binary is made of

### 1.1 Origin [R unless tagged]
<!-- ev:prog_uvc.1.1 -->

- The base is the SDK UVC demo `st_main_uvc.cpp`; its file names survive in strings (`uvc/st_main_uvc.cpp`,
  `internal//uvc/st_uvc.c`). The closest open copy is `opera_st_main_uvc.cpp`: same getopt string
  `a:A:b:B:M:N:m:p:I:t:i:qT:Q:V:d:D:S:Hhl:f:` plus the vendor's `c:L:P`. `__LINE__` values (`UVC_StartCapture` ≈ 3800–4200,
  main ≈ 7000) put the vendor's file at about 7000+ lines, with its features inside the same file.
- [I] The demo version lies between two public copies of the SigmaStar UVC demo. Video matches `opera_st_main_uvc.cpp` (M6 ISP / SCL
  API, same bind and depth logic). Audio matches the older `ikayaki` demo (`MI_AO_SetPubAttr/Enable/EnableChn`,
  `MI_AI_SetVqeVolume`, a 5-entry ops table with SetMute). Other old-version traits: no `MI_VENC_CreateDev`, no
  `MI_SYS_ConfigPrivateMMAPool` ring pools, ring input mode `E_MI_VENC_INPUT_MODE_RING_ONE_FRM` = 1 (the enum in the Xsyt M6
  header has no `RING_UNIFIED_DMA`), `MAX_UVC_DEV_NUM` 4 (opera 6, ikayaki 3), maxpacket defaults {1024, 1024, 192, 64}
  (ikayaki {1024, 1024, 192}). The camera's libmi_venc / libmi_sys do export `CreateDev` / `ConfigPrivateMMAPool`, so their
  absence is the demo's choice [R].
- `st_uvc.c` and `st_uvc_xu.c` match the skyipc_tiramisu copies (same `ST_UVC_Device_t` size 0x178, same XU config tables);
  `st_common.c` and `st_hdmi.c` match the ssd26x_zk copies.
- st_uvc.c log macros print `__LINE__`; pairing each of the binary's 74 log calls with the same format string in the same
  function of the skyipc source gives a piecewise-constant offset `binary_line − source_line`
 :

| source lines | offset | meaning |
|---|---|---|
| 1 – 643 (up to `_UVC_Video_Stream_on_off`) | +6 | 6 lines more somewhere before line 222 [I: in the header / defines area, since every function in this span reads identical] |
| 760 – 863 (`UVC_Sti_Capture` … `_UVC_StartCapture`) | +7 | one line more between 643 and 760 (tail of `_UVC_Video_Stream_on_off`, `UVC_Change_Input_Frame`, head of `UVC_Sti_Capture`); the only code difference there is the MJPEG constant in `UVC_Sti_Capture` (§3.6) [I: the count alone does not say which edit] |
| 1990 – 2238 (`_UVC_Events_Process_Control`, `_Class`, `_Setup`, `_Data`) | +748 … +799 | the vendor's rewrite of `_UVC_Events_Process_Control` (≈ 750 lines, §4.2) and +32 lines in `_UVC_Events_Process_Data` (the OTA / "encrypt_text" branch, §4.7) |
| 2254 – 2624 (`_UVC_Events_Process` … `UVC_Event_Handle_Task`) | +799 | unchanged |
| 2662 – 2709 (`UVC_Video_Process_Task`) | +803 | 4 lines: a dead loop counter (§3.6) |
| 3047 – 3304 (`ST_UVC_Init` … `ST_UVC_StopDev`) | +810 | 7 lines more somewhere in 2709–3047 (the MTD / DFU block; all its functions read identical, so the lines are outside them [I: comments or blank lines]) |

### 1.2 Code areas and function classes [R]

Our inventory of prog_uvc lists 1318 functions with a class: LIB (SDK library, 779), LIBAREA (inside
the static library area, left unnamed by the name matching, 281), SDK (demo or SDK helper as is, 60), SDK-LIB (tinyalsa etc., 37),
SDK-MOD (changed by the vendor, 51), VENDOR (108), with corrections applied. It still mixes up modules among twin MI wrappers — 61 LIB names differ from what the device the function opens
(`/dev/mi_<mod>`) and its ioctl number say, e.g. 0x3e1f8 is `MI_SCL_CreateDevice` (inventory: `MI_ISP_CreateDevice`), 0x3e704
`MI_SCL_DisableOutputPort`, 0x362f4 `MI_ISP_StopChannel` (inventory: `MI_VDISP_DisableInputChannel`), 0x7edc8
`MI_DISP_EnableVideoLayer` (inventory: `MI_HDMI_Start`); the VENC wrappers 0x3fff4..0x42a2e were unnamed. These, about 50
app-area functions re-classified as SDK code, the AES block and tinyalsa are corrected in it.

Layout of `.text`:

| range | content |
|---|---|
| 0x13f58 | `.init` + PLT (not a function) |
| 0x14ba4 … 0x14c60 | crt: `_start` 0x14ba4 (literal pool 0x14bd4), `deregister_tm_clones` 0x14bf8, `register_tm_clones` 0x14c1c, `__do_global_dtors_aux` 0x14c48, `frame_dummy` 0x14c60 |
| 0x14c98 … 0x275a0 | demo and vendor code (st_main_uvc.cpp); the accept loop's literal pool ends at 0x275a0 |
| 0x275a4 | main |
| … 0x31dd4 … | st_uvc.c, st_uvc_xu.c (incl. the vendor control handlers 0x2fb0c..0x31dd4), the AES block 0x3163c..0x31d36 (`internal//uvc/aes.cpp`) |
| 0x31dd4 … 0x337d0 | st_uac.c (§9.3) |
| 0x33820 – 0x34ebc | tinyalsa (§9.4) |
| from 0x35154 | statically linked SDK libraries (§1.3) |
| 0x84624 … 0x84668 | C runtime: `__libc_csu_init` 0x84624, `__libc_csu_fini` 0x84664 (`bx lr`), `stat64` → `__xstat64(3, …)` 0x84668 (the standard glibc 2.30 `csu` objects [R: code shape]) |
| 0x84674 | `.fini` section start (not a function) |

Class fixes outside the library area: 0x33c0c, 0x33d40, 0x33d6c are tinyalsa (`mixer_get_ctl`, `mixer_ctl_get_range_min`,
`_max`), not VENDOR; the crt rows above.

### 1.3 Statically linked libraries [R unless tagged]
<!-- ev:prog_uvc.1.3 -->

prog_uvc links no MI `.so`; all MI code is static. Each exported function of `/config/lib/lib*.so` was looked up in
prog_uvc's code with immediates (ioctl numbers, struct sizes) kept, independent of the disassembler's function boundaries: libmi_isp 384 identical, libispalgo 259, libfbc_decode 244, libmi_iqserver 44 (the count of
functions found byte-identical, not the export count — libmi_iqserver exports 48 functions), libmi_sys 25, libmi_venc 23,
libcus3a 30, the other used modules all identical. The "const-differ" hits all land on addresses another export matches
exactly (modules have twin functions with the same shape and different ioctl numbers); no export matches only with changed
constants. Exports not found are not linked in (their `__func__` strings are absent too). `MI_ISP_GetIQApiData` is not a
difference: automatic analysis merged three functions at 0x36500 (`Free_IQDataBuf`, `SetIQApiData` at 0x36560 using ioctl 0x6910,
`GetIQApiData` at 0x365d4 using 0x6911); the `.so` uses the same numbers for the same names. **The static copies are the
camera's `.so`, same build.**

**The static area is the `.so` code copied object file by object file:** within one object the offset
`delta = prog_addr − so_addr` is constant (e.g. libispalgo: 0x44a68 for 82 functions, 0x44348 for 56, …), and objects the
link did not need are absent. A disassembler's function cuts in this area are often wrong — a cut starts 2–8 bytes after the real start (the
prologue left at the end of the previous cut) or spans several small functions (the real `pAE_SetConvergeThd` starts at
0x4c1c0 = `.so` 0x7758, the cut is 0x4c1c4); 196 of the 281 LIBAREA cuts overlap exports, the rest are the libraries'
non-exported functions. Every one of the 281 is library or C-runtime code; none is vendor or demo code
(cut address, size, library, `.so` range, the real functions with their prog_uvc start, name source, static reachability). libispalgo's static functions are named through its kernel twin: the user build fills three
function-pointer tables at run time — `AE_InitFuncPtr` (0x54b4c), `AF_InitFuncPtr` (0x5a40c), `pAWB_InitFuncPtr` (0x5f690) —
and executing each initialiser in prog_uvc and in mhal.ko pairs the slots (96 + 35 + 41 slots, all
filled on both sides), e.g. slot 0x00 `pAE_CalLumY` = 0x57980, 0x38 `pAE_CalPvExpo` = 0x52c88, 0x74 `pAE_SetExposure` =
0x53148.

| library | cuts | bytes | real functions | statically reachable | what they are |
|---|---|---|---|---|---|
| libispalgo.so | 136 | 44 576 | 215 | 215 | user-space 3A algorithms: `pAE_*`, `pAWB_*`, `pAF_*`, `DrvAlgo_IF_*`, `hal_AF_*`, `AE_*`, IQ interpolation |
| libfbc_decode.so | 59 | 8 244 | 56 | 35 | FBC model used by the IQ server: `Compression1x8`, `IPBase`, `RandomTop`, `IniFileIO`, `RegisterTable`, minIni, PPM writers; the library also holds the full FBC encoder model and three INI layers (`isp-6-userspace.md` §7.1) |
| libmi_iqserver.so | 25 | 18 668 | 26 | 26 | IQ Tool server (`-q`): TCP / UART listeners, command parser, raw / JPEG / SCL capture for the tool, `MI_IQSERVER_SetApi/GetApi`, `MI_IQSERVER_GetJpeg` |
| libcus3a.so | 14 | 2 908 | 14 | 14 | CUS3A routing: `Cus3A_ProcAE`, `Cus3A_ProcAF` [I name], `CUS3A_RegInterfaceEX`, `SigmaAe/Awb/Af{Run,Ctrl,Release,Init}` |
| libmi_isp.so | 11 | 8 200 | 11 | 11 | `MI_ISP_SET_ALL`, `MI_ISP_ALGO_SET_ALL`, `MI_ISP_ALGO_API_CmdLoadCaliData`, `MI_ISP_Enable/DisableUserspace3A`, `MI_ISP_RegisterIspApiAgent`, DayNight wrappers, the isp device opener |
| libmi_ao.so / libmi_ai.so | 6 / 5 | 2 992 / 2 564 | 6 / 5 | all | audio internals: AO queue (`_MI_AO_QueueDraw`, `_MI_AO_QueueDeinit`, send-frame worker, global data / init), AI dump file and global data, both device openers |
| libmi_venc / vdec / hdmi | 3 / 3 / 2 | 942 / 648 / 496 | 4 / 3 / 2 | all | openers, `MI_VENC_FreeCustomMap`, `MI_VENC_CloseFd`, `_MI_VDEC_SendStream`, `_MI_VDEC_ForceToPanic`, the HDMI event poll thread |
| libcam_os_wrapper.so | 8 | 448 | 7 | 7 | `CamOsMsSleep`, `CamOsMutexLock/Unlock`, `CamOsTsemInit`, `CamOsMemRelease`, `CamOsMathDivU64`, `CamOsMathDivS64` |
| libmi_vif / sys / scl / ldc / jpd / disp | 1 each | 348 each | 1 each | all | `_MI_WRAPPER_DEVICE_Open` copies (open `/dev/mi_<mod>`, fd table) |
| C runtime (libc_nonshared.a, crt) | 3 | 80 | 3 | all | the `csu` functions of §1.2 |

Other static functions are named by their own `__func__` / `assert` strings (`_MI_AO_QueueDraw`, `MI_IQSERVER_GetJpeg`,
minIni's `save_strncpy` / `skiptrailing` / …) or by the device each `_MI_WRAPPER_DEVICE_Open` copy opens. Reachability is an
upper bound (referenced from code reachable from `_start` and `.init_array`, not a run-time trace). The 25 cuts with no
reference at all are all in libfbc_decode and dead in prog_uvc (linked because their object files were needed): the
`RandomTop` test-pattern randomiser (`loadSetting`, `isRandomEnabled`, `randomVal`, `random_lutX`, `randomGlobalParameter`),
the PPM dump writers (`StorePPM`, `StorePPM16B`, `StorePPMFrame`), `Bayer2RGB` / `splitChannel` / `Frame::toMat`,
`IniFileIO::setVariable` (string and double), `~IniFileIO`, `RegisterTable::getBankAddr`, `IPBase::getParamRange` /
`getNameUpper` / `getSectionNameLower`, the tail of `ini_putf` and two std containers.

Twin-body exports (identical code and ioctl, so only the position inside their object piece separates them) [I]: 0x4b110
`Sigma3AGetAeInterface`, 0x4b1e0 `Sigma3AGetAfInterface` (inventory: both `Sigma3AGetAwbInterface`); 0x60190 `Interp`,
0x60390 `Interp_LUT` (inventory `Interp_IQ`, `Interp_LUT_IQ`); 0x36f40 `MI_ISP_AWB_GetAwbHwAvgStats`, 0x3ad48
`MI_ISP_AWB_GetHWStats_Cus`, 0x3aee0 `MI_ISP_AWB_GetDayNightDetection`, 0x794f8 `_MI_AO_OpenDumpFile`, 0x79968
`_MI_AO_CloseDumpFile`, 0x83fa0 `MI_SYS_ChnInputPortGetBuf`, 0x84048 `…InputPortPutBuf`, 0x840c0 `…OutputPortGetBuf`,
0x84154 `…OutputPortPutBufPa`. The inventory names of 0x7ee28 `MI_DISP_DisableVideoLayer` and 0x7ee88
`MI_DISP_SetVideoLayerAttr` are right (ioctl 0x40046907 / 0x40186908) [R].

Which pieces run: libispalgo + libcus3a on the real start line (`MI_ISP_EnableUserspace3A` is called from
`MI_ISP_CreateChannel`); libmi_iqserver + libfbc_decode only with `-q`, which demo.sh passes; libcam_os_wrapper. The ai / ao,
vdec, jpd and ldc pieces are reachable only through the `-d` / `-D` audio path, the player (VDEC), the photo viewer (JPD) and
`-L` (LDC). No vendor code reaches any library function except through the MI / CUS3A / IQSERVER API. prog_uvc carries no
copy of mi_vdisp (no `/dev/mi_vdisp` string, no opener, no libmi_vdisp code; the only "vdisp" strings are the vendor's own
function names) and none of the algorithm libraries (libMD, libOD, libVG, the audio algorithms).

Dynamic side: NEEDED are `libmp4v2.so.2` (from `/customer/lib`; `set.sh` sets `LD_LIBRARY_PATH=/customer/lib`), librt,
libpthread, libdl, libstdc++, libm, libgcc_s, libc and `ld-linux-armhf.so.3` [R readelf]. The `/config/lib` `.so` files have no NEEDED entries among themselves (only libc): e.g.
libmi_isp.so leaves `g_mi_fd_table`, `Cus3A_*`, `CamOs*` undefined; nothing on the camera loads them dynamically. The
executable has no relocations.

### 1.4 Build, toolchain, MI API [R unless tagged]

- Camera userland: glibc 2.30, libstdc++.so.6.0.26. prog_uvc and the `.so` files were built with `GCC (GNU) 9.1.0`, armv7-a
  Thumb-2, VFPv3 / NEON, hard-float. Highest symbol versions needed: prog_uvc GLIBC_2.29 (`log2`, `pow`); the `.so` GLIBC_2.28.
- 155 MI / CUS3A functions are called from the app area (the ABI and the list: `mi-user-abi.md` §10); every one
  is exported by a camera `.so` in `/config/lib`. Coverage by public headers:

| module | functions | M6 / Tiramisu header | gap |
|---|---|---|---|
| SYS, SCL, VENC | 13, 11, 9 | Xsyt `m6/MI/include` (+ skyipc `mi_sys.h`) | none |
| VIF, DISP, HDMI | 9, 16, 10 | disp / skyipc_tiramisu_include_* | none |
| ISP IQ / AE / AWB | 30+ | skyipc `mi_isp_iq.h` + tiramisu IQ datatypes; 3A datatypes only from the ssd2xx tree (`isp/mi_isp_3a_datatype.h`) | 3A header from another tree, sizes verified |
| ISP device / channel / port (Create / Start / SetChnParam / SetOutputPortParam / GetInputPortCrop) | 11 | none | OpenIPC divinus `m6_isp.h` (misc/ap_cameragimbal, a8/) matches the binary |
| sensor | 8 | only datatypes (skyipc `mi_sensor_datatype.h`); prototypes in i6e / ssd20x `mi_sensor.h` | divinus `m6_snr.h` |
| LDC, JPD, VDEC, AI, AO, IQSERVER, CUS3A | 5, 7, 7, 9, 6, 2, 1 | none | — |

- Struct sizes the app fills, checked by compiling with the M6 headers: SNR PADInfo 0x34 / PlaneInfo 0x48 / Res 0x34, VIF
  Group 0x1c / Dev 0x14 / Port 0x14, SYS ChnPort 0x10 / BufInfo 0x108, SCL OutPortParam 0x18, VENC ChnAttr 0x4c / Stream 0x48 /
  Pack 0xb8 / SliceSplit 8 / InputSourceConfig 4, HDMI Attr 0x30, ISP IQ Brightness / Contrast / Lightness 0x4c, Saturation
  0x1a0, Sharpness 0x1328, WDR 0x668, HSV 0x134c, Defog 0x1c, AWB attr 0x5b8, AE expo info 0x23c, limit 0x20, manual expo 0x10.
  VENC field offsets match the stores in `UVC_StartCapture`. The divinus ISP structs match too: channel attr 0x50, chn param
  0x10, port param 0xc. LDC chn attr 0x54 is laid out in `ldc-1-stack.md` §4.1; the JPD, VDEC and audio structs were not checked
  this way.

## 2. Command line and start-up

### 2.1 Options [R unless tagged]

The start line in `customer/demo.sh` is `prog_uvc -H -A 0 -a 0 -q -i /customer/iq_file_gc8613.bin`: HDMI on, sensor mode 0,
pad 0, IQ server on, one UVC stream, mmap buffers, isochronous transfer, `g_maxbuf_cnt` 3. Getopt string
`a:A:b:B:M:N:m:p:I:t:i:qT:Q:V:d:D:S:Hhl:f:c:L:P`. The usage text (`help_message` 0x21370) lists `-a sensor pad`, `-A sensor
resolution index`, `-b bitrate`, `-B burst`, `-m mult`, `-p maxpacket`, `-M 0:mmap 1:userptr`, `-N num of uvc stream`,
`-i iq api.bin`, `-I c_intf,s_intf`, `-t trace level`, `-q open iqserver`, `-T 0:Isoc 1:Bulk`, `-Q qfactor`, `-f file instead
of MI`, `-V video on/off`, the audio lines of the older demo printed unconditionally (`-d : AI Device Id: Amic[0] Dmic[1] I2S
RX[2] Linein[3]`, `-D : AO Device Id: Lineout[0] I2S TX[1]`, `-S : AO Samle Rate …`) and `-h`; `-H`, `-c`, `-L`, `-P` are
not in it.

| option | stored in | effect |
|---|---|---|
| `-A <n>` | byte at params +0xd (params = 0xad548) | sensor mode index; out of range → 0, negative → read with `scanf` (§2.3) |
| `-a <n>` | params | sensor pad |
| `-H` | flag [I: 0xad5ad] | HDMI path (§5); also changes the USB pipeline (§3.2) |
| `-q` | flag 0xad568 | `MI_IQSERVER_Open` at start-up (`isp-6-userspace.md` §6) |
| `-i <path>` | 64-byte buffer 0xad56c (`strcpy`) | ISP API bin loaded in main (§2.2) |
| `-l` | params | 3DNR in the ISP channel parameters (default off) |
| `-T 0/1` | — | isochronous / bulk streaming |
| `-p`, `-m`, `-B`, `-I`, `-N`, `-M` | arrays and counts passed to `ST_UvcInit` | maxpacket list (defaults §1.1), mult, burst, interface ids `c_intf,s_intf`, stream count, mmap / userptr |
| `-b <Mbit>`, `-Q <q>` | — | H.26x bitrate, MJPEG Qfactor (§3.2) |
| `-V`, `-t` | — | video on / off, trace level |
| `-f <path>` | `g_ad5ae` = 1; the path copied into four 0x54-byte stream slots and two 0x44-byte audio slots [I: the two audio slots are 0xa97bc and 0xa9800] | file source instead of MI: `ST_VideoModuleInit` runs only when `g_ad5ae` == 0, the UVC stream reads files (§3.6), the audio ops use files (§9.2) |
| `-L <json>` | path at 0xad5b4 (`strcpy`, no length check), `g_ad5b0` = 1; prints "g_LdcJsonPath:%s" | LDC between SCL and VENC (§8) |
| `-d <n>` | `g_ad636` = 1, `g_ad640` = n | UAC with AI device n (§9) |
| `-D <n>` | `g_ad637` = 1, `g_ad638` = n | UAC with AO device n (§9) |
| `-S <hz>` | `g_a9a20` (default 0xbb80 = 48000) | sample rate of AI, AO and the UAC streams |
| `-c <arg>` | `g_ad5af` = 1 (case 'c' at 0x27cba, `strb #1`; the argument is ignored) | none (§10) |
| `-P` | `g_ad5b1` = 1 (case 'P' at 0x27d14, `strb #1`) | none (§10) |

demo.sh passes none of `-c`, `-P`, `-L`, `-d`, `-D`, `-S`, `-f`, `-p`, `-m`, `-B`, `-I`. `ST_DefaultArgs` memsets 0x14 bytes at
0xad548. [I] A `-i` argument longer than 63 characters overflows the buffer at 0xad56c into the flags at 0xad5ac and up; 71 or
more characters put a non-zero byte into `g_ad5b2` (§7.3); demo.sh's path is 28 characters.

### 2.2 Order of start-up in main (0x275a4) [R unless tagged]

1. `ST_Sys_Init`; `ST_UAC_SetTraceLevel`; `ST_UacInit` (returns at once without `-d` / `-D`, §9.2).
2. `ST_VideoModuleInit` (§2.3), unless `-f`.
3. IQ bin from `-i` through `ST_DoSetIqBin` — here in main right after video init (the SDK loads it at the first
   `UVC_StartCapture`). `ST_DoSetIqBin` polls `CUS3A_GetAlgoStatus` every 10 ms up to 101 times (the SDK polls
   `GetParaInitStatus` every 300 ms). Then a 1 s sleep.
4. With `-H`: `St_HdmiModuleInit`, `St_DispModuleInit`, `change_scl_resolution`, bind SCL(0,0,0) → DISP 60:60 (§5).
5. `echo setmirrorflip 0 1 1` into `/proc/mi_modules/mi_sensor/mi_sensor0`.
6. If the VIF counted more than 5 interrupts (`IntCnt` > 5) and `/tmp/sdcard_info` does not exist: `insmod g_webcam.ko
   streaming_maxpacket=3072 uvc_function_enable=1 uac_function_enable=0 rndis_function_enable=1`, usb0 up, `ST_UvcInit`
   (0x21234) with the getopt arrays, `g_ad490` = 1. So the USB gadget is brought up only when the sensor delivers frames and no
   SD card was mounted before prog_uvc got there; with a card inserted at power-on there is no USB camera and no RNDIS
   [I from the boot order: mdev mounts the card first, `rootfs-customer.md` §4.1].
7. Menu-link server `menu_server_start` (loads or creates `ws.bin`, §6.4), then the accept loop `menu_server_accept`, which never
   returns (§6.1).
8. Only reached when the sensor delivered ≤ 5 VIF interrupts: HDMI rebind, `ST_UvcDeinit`, `ST_VideoModuleUnInit`,
   `ST_UacDeinit` (0x1a204: if either audio flag is set, `ST_UAC_StoptDev` + `ST_UAC_FreeStream`), exit. The exit path calls
   `ST_Sys_Bind` where it means UnBind (the SDK demo has the same slip).

Startup ordering with kjd_lvgl: demo.sh starts prog_uvc in the background, sleeps 3 s and starts kjd_lvgl (`rootfs-customer.md`
§3.4).

### 2.3 The pipeline at start-up: `ST_VideoModuleInit` (0x1a3e8) [R]

Module ids (matched to `E_MI_MODULE_ID_*` in `D3_sys.txt`): VIF 6, ISP 0x21, SCL 0x22, LDC 0x17, DISP 0xf, VENC 2, AI 4. Bind
types: 1 FRAME_BASE, 4 REALTIME, 0x10 HW_RING. Pixel: 0 YUYV, 0xb NV12; the Bayer formula `0x14 + bits·12 + id` in the code
matches the header's enum. `g_stStreamAttr` (0xa9990): stream 0 = ISP port 0, SCL dev 0 (the SDK default is port 1 / dev 1,
switched only when HDMI is off). Unlike the SDK, which builds ISP → SCL per stream, the vendor builds it once here, with extra
depths, an optional LDC channel and no SCL dev 3 HDMI branch.

| module | settings |
|---|---|
| sensor | `SetPlaneMode(pad, HDRtype > 0)`; mode = `-A`; `u32MaxFps` (params +0x10) = the mode's max fps; SetRes, Enable |
| VIF | group = dev = 0 for pad 0; GroupAttr {MIPI, 1MULTIPLEX, HDR off, clk DOUBLE}; DevAttr / PortAttr = plane-info cap rect, pixel = RGB_BAYER(precision, bayer id) or the plane pixel; port frame rate FULL; depth (0, 6), later set again to (0, 4) |
| ISP | DevAttr stitch mask 1; ChnAttr sensor bind mask 1 << pad; ChnParam {HDR off, 3DNR = `-l` (default off), no mirror / flip / rotation}; **port 0 out param {crop 0, NV12}, enabled**; depth port 0 (0, 5) |
| bind | VIF(0,0,0) → ISP(0,0) REALTIME, maxfps → maxfps |
| SCL | dev 0, HW out mask 7 (HWSCL0..2); channels 0..N−1 created and started; **chn 0 port 0 {crop (0, 0, capW, capH), out capW × capH, NV12}, enabled**; depth (0, 5) |
| bind | ISP(0,0,0) → SCL(0,0) REALTIME, maxfps → maxfps |
| LDC (`-L`) | §8 |
| IQ server (`-q`) | `MI_IQSERVER_Open` |

### 2.4 Tear-down: `ST_VideoModuleUnInit` (0x1bc68) [R]

It uses SCL dev 0 everywhere; its HDMI branch destroys SCL(0,0) a second time instead of dev 3. With 0xad5ad set it stops SCL
chn 0 twice and returns before ISP, VIF and the sensor are torn down, so a following `ST_VideoModuleInit` fails at VIF
(0xa0062082). SCL device destroy hangs in `_HalSclWaitInputSrcIdle` («NO frame done», fence not finished) when a port is still
enabled or its source stopped mid-frame; a tear-down that disables the SCL ports and destroys the SCL channel and device before
stopping ISP, VIF and the sensor does not hang. Each open of a `/dev/mi_*` node costs about 2 s in the kernel.

## 3. UVC streaming

### 3.1 The app's format table and the 30 fps cap [R]

`_UVC_Fill_Streaming_Control` (0x2d9c0) maps the host's probe / commit through the app's own `uvc_formats[]` (0xa8398) and
picks the first table interval ≥ the requested one. The table holds MJPEG, H.264 and H.265 only, 7 frame sizes each, every
size with the intervals 333333 / 666666 / 1000000 (30 / 15 / 10 fps), so a 60 fps request becomes 30 fps before
`UVC_StartCapture` sees it. The table is matched by **index**, so the g_webcam.ko descriptors must list the formats in the
order MJPEG, H.264, H.265 with the frames in the table's order (`usb-gadget.md` §4.4). The vendor reordered the MJPEG list
(3840×2160 first) and added 424×240 to all three; YUYV and NV12 are in the code but not in the table, so the host cannot
select them.

| format (fcc) | frame list (.rodata) | frames 1 … 7 |
|---|---|---|
| 1 MJPEG (0x47504a4d) | 0x8729c | 3840×2160, 640×480, 1280×720, 1920×1080, 2560×1440, 320×240, 424×240 |
| 2 H.264 (0x34363248) | 0x873dc | 320×240, 640×480, 1280×720, 1920×1080, 2560×1440, 3840×2160, 424×240 |
| 3 H.265 (0x35363248) | 0x8751c | 320×240, 640×480, 1280×720, 1920×1080, 2560×1440, 3840×2160, 424×240 |

The still-image table `uvc_still_formats` is in §3.4. Without an explicit rate request the host commits the descriptor
default (30 fps).

### 3.2 `UVC_StartCapture` (0x1ec08) and `UVC_StopCapture` (0x205b8) [R unless tagged]

`UVC_StartCapture` is the callback for the host's stream start; it begins at 0x1ec08 (`sub sp,#8` for the by-value
`Stream_Params_t`; automatic analysis split the function at 0x1ec0a and lost the arguments). It stores the host's format (fcc, width,
height, frame rate from the interval). Vendor changes against the SDK:
- returns 0 at once in "fake power-off" (`g_ad480` == 1, set by menu id 0x1e, §6.3);
- **unbinds SCL → DISP and shuts HDMI down on every stream start** (the tear-down starts at 0x1ec64 and ends before
  0x1eeb8); SCL dev / chn / port fixed to 0, the port HDMI uses too (§5.3; the port number is the constant used at 0x1f03c here and at
  0x20638 in `UVC_StopCapture`);
- for MJPEG / H.26x **no ISP → SCL bind and no ISP / SCL depths here** — it relies on the REALTIME bind from start-up (the SDK
  binds both);
- MJPEG input NV12 when `-H`; the LDC variant (§8); a 1 s sleep at the end.

`UVC_StopCapture` has the same fake-power-off guard; with `-H` it keeps the ISP port and the ISP → SCL bind; LDC tear-down;
it ends with `init_hdmi()` (the call at 0x20ce6) and a white-balance restore (the ws apply function 0x251f0, §6.4).

Per format, with `-H` and one stream. Common: ISP port 0 out param {crop = ISP input crop, pixel per table} and SCL(0,0,0)
out param {same crop, size = host W × H, no mirror / flip / compress}; ISP and SCL ports enabled again. The ISP → SCL bind
(src `u32MaxFps`, dst host fps; FRAME_BASE with `-H`, REALTIME without) is issued only in the YUYV / NV12 branch. VENC channel
= stream index (0).

| | YUYV / NV12 | MJPEG | H.264 | H.265 |
|---|---|---|---|---|
| reachable from the host | no (not in `uvc_formats`) | yes | yes | yes |
| ISP port 0 pixel | YUYV / NV12 | NV12 (YUYV only without `-H`, 1 stream) | NV12 | NV12 |
| SCL pixel | YUYV / NV12 | as ISP; NV12 forced with `-L` | NV12 | NV12 |
| VENC dev | — | 8 | 0 | 0 |
| VENC type | — | JPEGE (4) | H264E (2) | H265E (3) |
| pic / max size | — | W×H / W×ALIGN16(H) | W×H / W×H | W×H / W×H |
| bByFrame, profile, B-frames | — | 1, —, — | 1, 1, 2 | 1, 0, 0 |
| rate control | — | MJPEGFIXQP, Qfactor 80 (or `-Q`) | H264CBR, GOP 30, stat 0, fluct 0 | H265CBR, GOP 30 |
| bitrate | — | — | 8 Mbit if W·H > 2560·1440, 4 M if ≥ 1920·1080, 512 kbit if < 640·480, else 2 M (or `-b` Mbit) | same |
| frame rate | — | num = host fps, den 1 | same | same |
| input source | — | — | NORMAL_FRMBASE + SliceSplit {1, 15} (RING_ONE_FRM, bind param H, without `-H`) | NORMAL_FRMBASE + SliceSplit {1, (H+31)/32} (ring likewise) |
| max stream cnt | — | 4 | 4 | 4 |
| bind SCL → VENC | — | FRAME_BASE (REALTIME without `-H`), fps host → host | FRAME_BASE (HW_RING without `-H`) | same |
| depths | ISP (0, 5), SCL (4, 5) set here | not set here (start-up: ISP (0, 5), SCL (0, 5)); LDC (0, 5) | same | same |
| buffer to host | GetBuf, copy H·stride (NV12 + H·stride1/2) | Query, GetStream (40 ms), copy all packs, keyframe flag from IDR | same | same |

The H.264 "B-frames 2" sits in `MI_VENC_ChnAttr_t` but mi_venc never passes it to mhal, whose default GOP is IPPP with one
reference — the stream has no B-frames (`mi_venc.md` §6.1). Stream slots for user-pointer mode: `calloc(3, 0x4c)` with 4 packs
(0xb8) each. The number of V4L2 buffers is `g_maxbuf_cnt` (u8 0xa97b3 = 3); the VENC `MaxStreamCnt` is computed by the
instruction at 0x2015e (`adds r3,#1`).

**The host's rate is not applied to the sensor [R]:** for MJPEG / H.26x the only ISP → SCL bind is the start-up REALTIME
maxfps → maxfps bind (no frame dropping), so the sensor's maximum rate goes over USB whatever interval the host committed;
the committed rate only reaches the SCL → VENC bind and the VENC frame rate.

Observed with altered values [R]: `g_maxbuf_cnt` 2 with `MaxStreamCnt` 2 works (bufcnt:2) and lowers 4K40 MJPEG
glass-to-glass latency from 295 to 215 ms in a GStreamer window (≈ 99 ms with mpv fullscreen on the laptop panel); bufs 2
with `MaxStreamCnt` 1 fails (in `UVC_StartCapture`, error 0xa0022003). A REALTIME SCL → VENC bind for MJPEG with `-H`
(the `-H` test at 0x1f44e, `27 d1`, skipped) starts the stream but delivers no frames. Without `-H` the stock start fails at
`ST_Sys_UnBind`: it unbinds the SCL → DISP bind that was never made.

### 3.3 Behaviour without `-H` [I unless tagged]

Without `-H`, `UVC_StopCapture` unbinds ISP → SCL, but the next MJPEG / H.26x `UVC_StartCapture` never binds it again, so a
second stream start would get no frames [I from code, not run]. With `-H` (the shipped start line) the start-up bind is never
removed. (Without `-H` the first start already fails, §3.2 [R].)

### 3.4 Still images [R]
<!-- ev:prog_uvc.3.4 -->

The gadget advertises still-image method 2 (`bStillCaptureMethod` = 2) and emits only MJPEG stills (`usb-gadget.md` §4.4); prog_uvc
answers the still probe / commit / trigger controls. `uvc_still_formats` (5 entries; the vendor extended the MJPEG list from
the SDK's single 1080p to three frames):

| format (fcc) | frame list (.rodata) | frames |
|---|---|---|
| 1 YUYV (0x56595559) | 0x8765c | 1920×1080 |
| 2 NV12 (0x3231564e) | 0x876ac | 1920×1080 |
| 3 MJPEG (0x47504a4d) | 0x876fc | 1920×1080, 3840×2160, 1280×720 |
| 4 H.264 (0x34363248) | 0x8779c | 1920×1080 |
| 5 H.265 (0x35363248) | 0x877ec | 1920×1080 |

- **Probe / commit** (`_UVC_Fill_Still_Image_Streaming_Control`, 0x2d6c0, as the source): clamps bFormatIndex to 1..5 and
  bFrameIndex to the format's frame count, bCompressionIndex = 3; computes a frame size per fcc (YUYV w·h·2, NV12 w·h·1.5,
  MJPEG / H.264 / H.265 w·h) and from it dwMaxPayloadTransferSize via `calculate_payloadsize(…, 333333)`; a bad format / frame
  logs "No Support Format/Frame". The answer (`uvc_still_image_streaming_control` = {bFormatIndex, bFrameIndex,
  bCompressionIndex, dwMaxVideoFrameSize, dwMaxPayloadTransferSize}) is written as three words: {indices, compression 3},
  **1000**, the payload size; the per-fcc size itself is stored only on the "No Support Frame" path [R decompile]. (reads the 1000 as dwMaxVideoFrameSize, as dwFrameInterval.)
- **Trigger:** the DATA event of the still-trigger control (streaming interface) with payload byte 0 == 1 (TRANSMIT) calls
  `UVC_Sti_Capture` (0x29d40); other trigger codes (bulk, abort, normal) are ignored.
- **`UVC_Sti_Capture`:** (1) builds a stream-params copy from the current params with fcc forced to MJPEG (the constant
  0x47504a4d, `movw/movt` at 0x29d96, where the source takes `uvc_still_formats[bFormatIndex−1].fcc`) and width / height
  from `uvc_still_formats[sti_commit.bFormatIndex−1].frames[sti_commit.bFrameIndex−1]`; (2) under the device mutex calls
  `UVC_Change_Input_Frame` = StopCapture then StartCapture with the still params; (3) in MMAP mode allocates a w·h·2 bounce
  buffer and calls the device fill op until a frame arrives (`Get A Still Buf, length:%ld`); (4) slices the frame across the
  queued V4L2 buffers — per DQ'd buffer, `is_tail` = 0 and `reserved` = FRAME_NOEND (bit 0) while the rest exceeds the buffer,
  else `is_tail` = 1; every slice gets `reserved |= STILL_IMAGE` (bit 1); `VIDIOC_QBUF`; until the whole frame is queued;
  (5) restores the normal params with another `UVC_Change_Input_Frame`. So a still capture is a brief reconfigure to the still
  size, one MJPEG frame sliced into UVC buffers with the still bit, and a reconfigure back. What a host shows for it is not
  known.

### 3.5 The QBUF `reserved` word [R]

prog_uvc fills `v4l2_buffer.reserved` on every QBUF (`_UVC_QBuf` 0x28db0, `UVC_Sti_Capture`): a frame that continues in the
next buffer (`is_tail` == 0) → 1 (`UVC_BUFFER_FLAGS_FRAME_NOEND` = 1 << 0); a still-image slice additionally |= 2
(`UVC_BUFFER_FLAGS_STILL_IMAGE` = 1 << 1); a complete single-buffer frame → 0 (definitions in `st_uvc_datatype.h`). The
gadget (usb_f_uvc.ko) copies the word at QBUF into `uvc_buffer`+0x170 and turns bit 0 into "no EOF, no FID toggle" and bit 1 into the payload header's STI bit
(`bmHeaderInfo` |= 0x20) (`usb-gadget.md` §3.5). The model is "one encoded frame may span several V4L2 buffers".

### 3.6 The SDK streaming code (`st_uvc.c`, the demo's stream functions) [R]
<!-- ev:prog_uvc.3.6 -->

Function verdicts against the source: 79 identical (18 of them read instruction by
instruction, the rest matched by string set and call shape), 12 functions + 3 data tables changed by the vendor (§2, §3.1,
§3.2, §4), 18 differing by SDK version (6 read), unresolved: the 18 st_uac.c functions (§9.3) and tinyalsa (§9.4). The 73
functions matched only by strings (61 "same", 12 "version") were then read instruction by instruction against the source
: 61 are the same, 6 differ, 5 are the old audio API (§9.2) and one is st_uac's
`TraceLevel2Str`. The version differences that change behaviour are the old audio API (§9.2), `ST_DoSetIqBin`'s polling
(§2.2) and `MAX_UVC_DEV_NUM` 4 (§1.1).

"Same" means same control flow, same constants, same calls in the same order, same struct offsets, macros resolved
(`UVC_GET_STATUS` bit tests, `DOWN/UP` = the device mutex, `CLEAR` = memset, `fops.*` = indirect calls through the
`ST_UVC_OPS_t` copy at +0x64…+0x7c); `#ifdef UVC_USE_MAXSZ_BUF` blocks are not compiled in (the binary follows the `#ifndef`
branches).

Same as the source:
- opera main file (17): `ST_AO_FILE_Init` (`open(path, O_RDWR|O_CREAT|O_TRUNC, 0777)`), `ST_AI_FILE_Init`,
  `ST_AO_FILE_Deinit`, `ST_AI_FILE_Deinit`, `AO_FILE_TakeBuffer`, `AI_FILE_FillBuffer`, `ST_UacDeinit`, `ST_Flush` (the
  `c != EOF` test is folded away because `char` is unsigned on ARM — same machine behaviour as the source built for ARM),
  `ST_DefaultArgs`, `ST_HandleSig`, `UVC_Init`, `UVC_Deinit`, `Get_UVC_Device`, `UVC_ForceIdr`, `getFramePos`,
  `UVC_MM_FILE_FillBuffer` (33 ms pacing), `UVC_StopReadFile` (`MAX_FRAME` = 10000).
- st_common.c (1): `ST_Sys_UnBind` (returns the MI error on failure, via `ExecFunc`).
- st_uvc.c (37): `TraceLevel2Str` (all seven strings, incl. "all"), `ST_UVC_SetTraceLevel` (levels 1–6),
  `ST_UVC_GetCurrentSpeed`, `_UVC_FillBuffer`, `_UVC_FinishBuf`, `_UVC_QBuf`, `_UVC_DQBuf`, `_UVC_Video_QBuf`,
  `_UVC_Video_ReqBufs` (≥ 2 buffers, mmap `PROT_READ|PROT_WRITE`, `MAP_SHARED`), `_UVC_SetFormat`, `_UVC_Video_Stream_on_off`,
  `UVC_Change_Input_Frame`, `_UVC_SYS_Exit`, `_UVC_StopCapture`, `_UVC_SYS_Init`, `_UVC_StartCapture` (`active_indicator` =
  3), `_UVC_ForceIdr`, `_UVC_Events_Process_Streaming` (probe 0x22 / still 0xc bytes, GET_INFO 3; `CLEAR(ctrl)` on GET_RES
  clears only the local pointer and is optimised away in both), `_UVC_Events_Process_Class`, `_UVC_Events_Process_Standard`,
  `_UVC_Events_Process_Setup`, `_UVC_Video_Process`, `_UVC_Fill_Still_Image_Streaming_Control`, `UVC_Event_Handle_Task` (2 s
  select on exception fds), `proc_parse_start`, `proc_parse_next`, `UVC_Find_MTD`, `UVC_MTD_Write`, `UVC_Fw_Update_Task`,
  `UVC_DFU_Init`, `UVC_DFU_Update`, `UVC_DFU_DnlEnd` (the field-update functions keep their progress prints and the partition
  table, §4.8), `ST_UVC_Uninit`, `ST_UVC_CreateDev` (mult capped at 2, burst at 15), `ST_UVC_DestroyDev`, `ST_UVC_StartDev`
  (three threads `UVC_Event_Handle_Task`, `UVC_Video_Process_Task`, `UVC_Fw_Update_Task`), `ST_UVC_StopDev`.
- st_uvc_xu.c (6): `usb_vc_eu1_isp_cmd`, `usb_vc_eu1_mmp_cmd`, `usb_cus_set_isp`, `usb_cus_get_isp`, `usb_cus_set_isp_ut`,
  `usb_cus_get_isp_ut` (behaviour in §4.4, §4.6).

Differ from the source:

| function | difference | effect |
|---|---|---|
| `UVC_UP_FILE_FinishBuffer` 0x1c5ce, `UVC_UP_FILE_FillBuffer` 0x1c5e6 | no "not support" print and **no return value**: the body is empty and returns whatever r3 holds | only with `-f` + `-M 1` (userptr); the caller treats a non-negative garbage value as success [I] |
| `UVC_StartReadFile` 0x1cafc | the two debug prints "V4L2_PIX_FMT_NV12/YUYV" are absent | log only (`-f` path) |
| `UVC_Sti_Capture` 0x29d40 | fcc forced to MJPEG (§3.4) | still capture always restarts the input as MJPEG |
| `UVC_Video_Process_Task` 0x2e100 | an extra loop counter incremented per pass and reset at 3000, never read | none |
| `help_message` 0x21370 | the older demo's audio lines printed unconditionally (§2.1) | text only |
| `Vol_to_Db`, `ST_AI_SetMute`, `ST_AI_SetVqeVolume`, `AO_TakeBuffer`, `AI_FillBuffer` | the old audio API, no exact source (§9.2) | only with `-d` / `-D` |

`UVC_Find_MTD` / `proc_parse_*` read `/proc/mtd` (0x1000 bytes, header `"dev:    size   erasesize  name\n"`, lines
`"mtd%d: %llx %x"`, name < 128).

## 4. UVC control requests

### 4.1 Event loop and routing [R]

prog_uvc opens the gadget's `/dev/videoN`, subscribes V4L2 events 0..5 and runs `UVC_Event_Handle_Task`, which calls
`_UVC_Events_Process` (0x2d0a4) per event. Routing (byte-for-byte the st_uvc.c source except the vendor control handlers):

| V4L2 event | enum | action |
|---|---|---|
| +0 | CONNECT | `pdev->exit_request = 0`; log only |
| +1 | DISCONNECT | `pdev->exit_request = 1` |
| +2 | SETUP | `_UVC_Events_Process_Setup` → Standard (no-op) or Class |
| +3 | DATA | `_UVC_Events_Process_Data` (0x2cc18) |
| +4 | STREAMON | lock → `UVC_StartCapture` → unlock (isoc path only) |
| +5 | STREAMOFF | lock → `UVC_StopCapture` → unlock |
| +6 | FORCEIDR | `_UVC_ForceIdr` (only in a STREAM_ERR build of the gadget; not this one, `usb-gadget.md` §3) |

A control request is answered with `UVCIOC_SEND_RESPONSE` = **0x40405501** (`_IOW('U', 1, struct uvc_request_data)`, the
struct = `s32 length` + `u8 data[60]` = 64 bytes; built with `movw r1,#0x5501; movt r1,#0x4040` at file offset 0x1D2CE).

**Class set-up** (`_UVC_Events_Process_Class`, 0x2c910): the request type must be RECIP_INTERFACE, else -EINVAL. Control vs
streaming interface is decided from `wIndex & 0xff`: if the app's `c_intf` / `s_intf` are both 0 (no `-I`, the demo.sh case)
`ctype = (wIndex & 0xff) % 2`, so the even-numbered interface is the control interface; otherwise it compares with `c_intf`.
Control interface → `_UVC_Events_Process_Control` with (bRequest = req, wValue >> 8 = CS, wIndex >> 8 = `entity_id`,
wLength, resp); streaming interface → `_UVC_Events_Process_Streaming` (0x2c790: probe / commit / still probe / still commit / still
trigger; GET returns the stored `probe` / `commit` / `sti_probe` / `sti_commit`, SET_CUR records which control is pending so
the following DATA event fills it, GET_INFO = 3, GET_LEN = the struct size).

**`_UVC_Events_Process_Control` (0x2a468)** stores req / cs / entity / length in the device struct (+0x110 cs, +0x111 entity
id, +0x112 length; +0x10c ctype, set elsewhere) and switches on the entity id. The ids are the unit ids of g_webcam.ko's
descriptors (IT 1, XU2 2 "customer", PU 3, XU1 6 "AIT", OT 7; `usb-gadget.md` §4.3):

| entity id | unit | GET handler | SET (DATA) handler via `usb_vc_out_data` (0x2cb8c) |
|---|---|---|---|
| 0 | interface (error-code probe) | returns the stored request_error_code; GET cs 2 → data[0] = errcode, else sets the default errcode 6 | — |
| 1 | camera terminal (CT) | rebuilt from live ISP values (§4.2); some AE-mode fast paths inline | `usb_vc_ct_cs_out` (0x30b4c) |
| 2 | XU2 "customer" | `usb_vc_eu2_cs(cs, req, resp)` | `usb_vc_eu2_cs_out` (0x30aec) |
| 3 | processing unit (PU) | from live ISP values (§4.2); the 0x81..0x87 arm reads WDR etc. | `usb_vc_pu_cs_out` (0x30cb8) |
| 6 | XU1 "AIT" | `usb_vc_eu1_cs(cs, req, resp)` (0x2fc30) | `usb_vc_eu1_cs_out` (0x3035c) |
| other | — | resp = 0xffffffcd, errcode 5 (stall) | — |

A non-zero return of a unit handler gives resp = 0xffffffcd and errcode 6; success gives resp = length, errcode 0.

### 4.2 Camera-terminal and processing-unit controls [R unless tagged]

The SDK base handles only CT SCANNING_MODE; the vendor rewrote the GET answers of `_UVC_Events_Process_Control` from live ISP
values and the SET handlers `usb_vc_ct_cs_out` / `usb_vc_pu_cs_out` into MI_ISP calls plus a line in `/customer/camp.txt`
(§4.3). GET_DEF returns the current ISP value, GET_LEN is not answered, GET_INFO = 3. All calls use ISP device 0, channel 0:
GET reads the channel from `g_ad678`, SET from `g_ada8c`, and neither global is ever written (§10).

| control (cs) | GET_CUR source | min..max, res | SET action | camp.txt line |
|---|---|---|---|---|
| CT AE mode (2) | ExpoMode: M (4) → 4, A (0) → 2 | 0..10, 0 | any non-zero → ExpoMode A; 0 → nothing | 12 AUTO_EXPOSURE:1 |
| CT exposure absolute (4) | AE exposure info (+0x14), 3 bytes | 156..40000, 1; DEF = manual u32US | ExpoMode M + ManualExpo {FNx10 16, sensor gain 1024, ISP gain 1024, us = raw value} — UVC's 100 µs unit is used as µs | 8 EXPOSURE, 12 AUTO_EXPOSURE:0 |
| PU backlight (1) | WDR bEnable | 0..1, 0 | WDR {bEnable = value, manual} | 7 |
| PU brightness (2) | Brightness manual level | 0..100, 2 | Brightness {on, manual, level} | 1 |
| PU contrast (3) | Contrast manual level | 0..100, 1 | Contrast likewise | 2 |
| PU gain (4) | **Defog** manual strength | 30..70, 1 | Defog {on, manual, strength} | 9 GAIN |
| PU power line (5) | AE flicker 2 → 2, else 1 | 1..2, 1 | always `SetFlicker(1)` (60 Hz in the SigmaStar enum), value ignored | 10 |
| PU hue (6) | not answered | — | HSV hue offset value − 64 | — |
| PU saturation (7) | Saturation manual | 0..100, 1 | Saturation manual | 3 |
| PU sharpness (8) | Sharpness manual | 0..128, 1 | Sharpness manual | 4 |
| PU gamma (9) | **Lightness** manual | 0..100, 1 | Lightness manual | 5 GAMMA |
| PU WB temperature (10) | AWB manual R gain | 0..8192, 1 | AWB manual {R = value, Gr = Gb = B = 1024} | 6 BALANCE, 11 AUTO_BALANCE:0 |
| PU WB auto (11) | AWB op type manual → 0, auto → 1 | 0..8192 (sic), 1 | AWB auto | 11 AUTO_BALANCE:1 |
| PU hue auto (0x10) | — | — | HSV off | — |

### 4.3 `/customer/camp.txt` — the UVC controls persisted [R unless tagged]
<!-- ev:prog_uvc.4.3 -->

Text file, 12 lines `NAME:%05d\r\n` in fixed order, opened `"r+"`. Writer: the SET handlers of §4.2 and XU1 cs 5 (§4.4);
`settings_rewrite_line` rewrites one numbered line in place; the file is written only when `g_ad67c` is set. Whether `g_ad67c` is ever
set is not traced (calls it the `-c` option, but `-c` sets `g_ad5af`, §2.1); the dump's copy differs
from the built-in defaults. Reader: `ws_ini_apply`,
at the start of every USB stream (`UVC_StartCapture`), ignores the names and goes by line number (lines past 12 ignored); if
the file cannot be opened the built-in defaults are applied (`ws_ini_defaults`, table at 0x861c8). The scales are the raw ISP
values (the menu link doubles some of them, §6.2).

| line | name | default | applied as |
|---|---|---|---|
| 1 | BRIGHTNESS | 50 | IQ brightness {1, manual, Lev = v} (struct zeroed, no Get first) |
| 2 | CONTRAST | 90 | IQ contrast Lev = v |
| 3 | SATURATION | 35 | IQ saturation u8 +0x188 = v |
| 4 | SHARPNESS | 20 | IQ sharpness u8 +0x12fb = v |
| 5 | GAMMA | 50 | `MI_ISP_IQ_SetLightness` Lev = v |
| 6 | BALANCE | 2656 | AWB manual: R = v, Gr = Gb = 0x400, B = 0x714 |
| 7 | BACKLIGHT | 1 | WDR bEnable = v |
| 8 | EXPOSURE | 24992 | AE manual {FNx10 18, SensorGain 0xb3b, ISPGain 0x400, µs = v} |
| 9 | GAIN | 1431 | AE manual, SensorGain = v |
| 10 | FREQUENCE | 2 | flicker enum = v (1 = 60 Hz, 2 = 50 Hz) |
| 11 | AUTO_BALANCE | 0 | AWB auto if v ≠ 0, manual if 0 |
| 12 | AUTO_EXPOSURE | 0 | AE auto if v ≠ 0, manual if 0 |

The dump's copy holds 52, 90, 35, 20, 50, 2656, 1, 20000, 1431, 2, 1, 1. Write and apply sides disagree: GAIN is written from
the Defog strength but replayed as sensor gain; BALANCE is written with B = 1024 but replayed with B = 0x714. [I] The order
means lines 11–12 decide the final AE / AWB mode (lines 6, 8, 9 switch to manual first).

### 4.4 XU1 — the "AIT ISP/MMP/FW" unit (entity 6) [R unless tagged]
<!-- ev:prog_uvc.4.4 -->

GUID `{23E49ED0-1178-4F31-AE52-D2FB8A8D3B48}`, bNumControls 0x10, bmControls `FF FF` (16 bits, 14 used) (`usb-gadget.md`
§4.3). The selector set is the SDK `st_uvc_xu.c` AIT table with the same selector numbers and config structs; the vendor
rewrote selector 1, added selector 0xd and rewrote the out-data side (cs 5, the challenge store, the `xu.txt` write).

Each selector owns a `VC_CMD_CFG` struct (0x20 bytes) in `.data`: +0 bCmdCap (u32 capability mask), +4 bInfoCap (u8), +8
bCmdLen (u16, data length), +0xa bUnitId (u8, = 1 in every entry), +0xc dwCurVal, +0x14 dwMinVal, +0x18 dwMaxVal, +0x1c
dwResVal. The 14 structs start at 0xa9a44 (`g_a9a44`):

| cs | struct | bCmdLen | meaning (`st_uvc_xu_datatype.h`) | GET behaviour (`usb_vc_eu1_cs`) |
|---|---|---|---|---|
| 1 | 0xa9a44 | 8 | EU1_SET_ISP | **vendor**: GET_CUR → AES-128 of the stored challenge (§4.5); cur / min / res zeroed first |
| 2 | 0xa9a64 | 8 | EU1_GET_ISP_RESULT | cur = `g_dc6c0` (ISP command result, filled by `usb_vc_eu1_isp_cmd`) |
| 3 | 0xa9a84 | 0x20 | EU1_SET_FW_DATA (32 B) | cur / min / max / res zeroed |
| 4 | 0xa9aa4 | 8 | EU1_SET_MMP | cur / min / max zeroed |
| 5 | 0xa9ac4 | 8 | EU1_GET_MMP_RESULT | cur = `g_dc6fc` (MMP result) |
| 6 | 0xa9ae4 | 0x10 | EU1_SET_ISP_EX (16 B) | passes the literal 0x10 as cur value to `usb_vc_cmd_cfg` |
| 7 | 0xa9b04 | 0x10 | EU1_GET_ISP_EX_RESULT | cur = `g_dc688` |
| 8 | 0xa9b24 | 0x10 | EU1_READ_MMP_MEM | GET_CUR copies up to 16 B from a running read pointer (`g_ada80` / `g_ada84`), rest 0 |
| 9 | 0xa9b44 | 0x10 | EU1_WRITE_MMP_MEM | cur 0 (no store in this build) |
| 0xa | 0xa9b64 | 0x10 | EU1_GET_CHIP_INFO | cur = `g_dc6a0`, prefilled with ASCII `'8' '4' '5' '5'` and 12 zero bytes — the chip-family string "8455" (SSC8455 / Tiramisu family), as in the SDK source's `eu1_get_chip_info_val` |
| 0xb | 0xa9b84 | 0x20 | EU1_GET_DATA_32 | cur = `g_dc4c4`; max buffer 32 × 0xff, res[0] = 1 (IQ-server data-back path) |
| 0xc | 0xa9ba4 | 0x20 | EU1_SET_DATA_32 | cur / min / max / res zeroed |
| 0xe | 0xa9bc4 | 0x10 | EU1_SET_MMP_CMD16 | cur / min / max zeroed |
| 0xf | 0xa9be4 | 0x10 | EU1_GET_MMP_CMD16_RESULT | cur = `g_dc6b0` |
| 0xd | — (handled inline) | 0x10 | not in the SDK enum | **vendor**: AWB gain read (below) |

**`usb_vc_cmd_cfg(req, cfg, cur_val, resp)`** (0x30720) builds every answer (a decompiler mis-structures it —
it shows an OTA `puts()` and a case switch that do not match the flow; the disassembly 
is the reference). `req` == 1 (SET_CUR): returns if `cmdCap & 1` (the store happens in the out-data handler). The 0x81..0x87
arms are the GET family (the `UVC_GET_*` codes the kernel passes: `UVC_SET_CUR` 1, GET_CUR 0x81, GET_MIN 0x82, GET_MAX
0x83, GET_RES 0x84, GET_LEN 0x85, GET_INFO 0x86, GET_DEF 0x87 [S: UVC class codes]), each gated by its `cmdCap` bit, writing `resp->data` and `resp->length`: GET_CUR (0x81) runs the AES
block for selector 1 and otherwise copies bCmdLen bytes of the `cur_val` buffer; GET_MIN / GET_MAX / GET_DEF / GET_RES return
small fixed 1- or 2-byte constants (0 / 1); GET_LEN (0x85) returns the u16 bCmdLen; GET_INFO returns the 1-byte info cap.

**Selector 0xd — AWB gain read (vendor):** GET_CUR (0x81): `MI_ISP_AWB_GetAttr(0, 0, buf[0x5b8])`; if the op-type field
(buf+4) is 1 (manual) it takes three u16 gains at buf+8 / +0xa / +0xc, else `MI_ISP_AWB_QueryInfo(0, 0, buf[0x18])` and the
three u16 at +0 / +2 / +4. Each gain is remapped `g' = ((g − 1) if g ≥ 1 else (g + 30)) >> 5` and packed: resp = 16,
data[1..] = {g0', g1'} as a packed CONCAT, data[2] = g2'. GET_LEN (0x85) returns 0x10. It is the read twin of cs 5 below.

**Out-data (`usb_vc_eu1_cs_out`, 0x3035c):**

| cs | action |
|---|---|
| 1 (SET_ISP) | `usb_vc_eu1_isp_cmd(data+1)`: an 8-byte ISP command; only sub-command 11 (0x0b, FW version) is implemented: the cs 2 result bytes [2..7] = 00 01 00 00 00 02 from `gbFwVersion` {u8 1, u8 0, u16 0x0002} at 0xa9a30 (`g_a9a30`; major / minor / build) |
| 3 (SET_FW_DATA) | `UVC_DFU_Update(pdev, data+1, data->length)` — field-update data chunk (§4.8) |
| 4 (SET_MMP) | `usb_vc_eu1_mmp_cmd(pdev, data+1)` (the non-IQSERVER branch): sub-command 1/0 → `UVC_DFU_Init(pdev, size = bytes 4..7)`, 1/1 → `UVC_DFU_DnlEnd(pdev)` |
| 5 (AWB / AE write, vendor) | by `data[1]`: 1 → AWB auto (GetAttr, op-type +4 = 0, SetAttr); 2 → AWB manual with R = byte5 << 5, Gr = 0x400, Gb = byte6 << 5, B = byte7 << 5, SetAttr, then camp.txt line 6 `BALANCE:%05d` (R) and `AUTO_BALANCE:00000` into line **12** (`settings_rewrite_line(0xc, …)`; the PU handlers write AUTO_BALANCE into line 11), `sync` after each [R decompile; I: line 12 is read back as AUTO_EXPOSURE, so the next stream start sets AE manual]; 3 → AE ExpoMode 0 (auto); 4 → AE ExpoMode 4 (manual) |
| 6 (SET_ISP_EX) | store the 16 bytes into the challenge buffer `g_a9a34` (§4.5); write them to `/customer/xu.txt` (`"w+"`, 16 B) and set `g_ada88` = 1 on success |
| 0xb (GET_DATA_32 set-up) | prepares max / res scratch buffers for the IQ-server read-back; no lasting effect in this build |
| 0xc (SET_DATA_32) | store 16 bytes into `g_a9a34`, no file write |

`/customer/xu.txt` is not in the dump (only `camp.txt` is), so no host has stored a challenge on this unit.

### 4.5 XU1 challenge-response (AES-128 ECB) [R unless tagged]

Selector 1's GET_CUR answer and the stores on selectors 6 and 0xc form a challenge-response; the code is `internal//uvc/aes.cpp`
(assert strings in 0x3163c..0x31d36).
- **Key:** 16 bytes at `.rodata` 0x89a44, `53 5a ad 18 22 2f 91 03 a9 3c 56 30 d8 42 bb 75`, used directly as the AES-128 key —
  no derivation. (The GET_CUR arm loads 32 bytes into the key-schedule input, but the key size passed is 0x80 = 128 bits, so
  only the first 16 are used.)
- **Challenge:** a 16-byte buffer at `.data` 0xa9a34 (`g_a9a34`), preloaded with `f9 80 fd d3 16 5a 78 44 d1 e4 ad 69 88 09 a1
  60`; the host overwrites it through selector 6 or 0xc (§4.4).
- **Response** (selector 1 GET_CUR → `usb_vc_cmd_cfg`, AES arm): load the key; `aes_init_ctx(ctx, key, 0x80)` sets Nr = 10
  and runs the key expansion (44 round-key words); `aes_encrypt_block(ctx, out, in)` (0x31d36) encrypts one block = the current `g_a9a34`
  (AddRoundKey, 9 full rounds, a final round without MixColumns); the 16-byte ciphertext goes into `resp->data` with
  `resp->length` = the selector's bCmdLen = **8**. So `response = AES128_ECB_encrypt(key, g_a9a34)[0:8]`.
- **Check:** the binary's own routines (0x31cc4 set-key, 0x31d36 encrypt-block and the helpers), run under emulation, give the
  FIPS-197 results (key `000102…0f` / plaintext `00112233…ff` → `69c4e0d8 6a7b0430 d8cdb780 70b4c55a`; Appendix B key
  `2b7e…` / plaintext `3243f6…` → `3925841d…196a0b32`). The device key over the preloaded challenge gives
  `82066163 7050ab49 b8ccb385 5e8d221d`, which is exactly the XU2 GUID `{63610682-5070-49AB-B8CC-B3855E8D221D}` in its
  little-endian byte layout: a host holding the key can check the device by sending `f980fdd3…` and comparing the answer with
  `82 06 61 63 70 50 ab 49` [R pairing; I that the vendor's PC tool uses it that way].
- **Cipher routines:** `key_expansion` (0x31a90) (ctx[0] = Nr in the 10 / 12 / 14 form, ctx[1] = pointer to the 44 / 52 / 60
  round-key words at ctx+2; SubWord through the S-box, RotWord, Rcon; the 256-bit extra-SubWord branch `Nk == 8 && i % Nk == 4`
  present but unused at 128 bits); `aes_set_key` (0x31cc4) (bits 0x80 → Nr 10, 0xc0 → 12, 0x100 → 14, else error 1; ctx[1] =
  ctx+2; returns 0); `aes_encrypt_block` (0x31d36) (checks ctx[1] == ctx+2; returns 0, 2 if the context is not initialised, 1 on
  NULL); helpers `aes_transpose` transpose used by ShiftRows, `add_round_key` (0x316cc) (16-byte XOR), `_sub_bytes` (0x31754)
  (S-box pointer `g_89b68`), `gf_mul` (0x317d8) GF(2^8) multiply (xtime with 0x1b), `_mix_columns` (0x318ac) (MDS matrix at
  `g_89c68`), round shell `aes_round` (0x319b0) (SubBytes + ShiftRows as word rotations by 8 / 16 / 24 + MixColumns + AddRoundKey),
  `aes_final_round` (0x31a28) the same without MixColumns (final round). It is textbook AES-128.

### 4.6 XU2 — the "customer" unit (entity 2) [R]

GUID `{63610682-5070-49AB-B8CC-B3855E8D221D}`, 4 selectors, bmControls `3F` (`usb-gadget.md` §4.3); the SDK `usb_vc_eu2_cs` /
`usb_cus_*` unchanged:

| cs | GET (`usb_vc_eu2_cs`) | SET (`usb_vc_eu2_cs_out`) |
|---|---|---|
| 1 CUS_XU_SET_ISP (4 B) | cur = `g_dc6f8` | `usb_cus_set_isp`: accepts a 4-byte command; value 4 (REQUEST_IFRAME as the code reads it) → ok, else error; no ISP call |
| 2 CUS_XU_GET_ISP (4 B) | cur = `g_dc638` | `usb_cus_get_isp`: returns 0 |
| 3 CUS_XU_SET_ISP_UT (60 B) | cur = `g_dc4e4` | `usb_cus_set_isp_ut`: if length == 0x3c, copy the 60 bytes to `g_dc64c` |
| 4 CUS_XU_GET_ISP_UT (60 B) | cur = `g_dc64c` | `usb_cus_get_isp_ut`: returns nothing |

The only live effect is the 60-byte store → read-back through `g_dc64c` ("UT" = unit-test scratch [I]).

### 4.7 Host-triggered reboot into the USB upgrade [R]

A branch of `_UVC_Events_Process_Data` (0x2cc18), not an XU selector: taken for a pending control-interface request of the
control class whose DATA payload is 8 or 16 bytes long (around 0x2cf6e). After `usb_vc_out_data` returns it prints
`data->length = %d`; 16 bytes → hex dump and the word `encrypt_text`, nothing else; 8 bytes → payload bytes 4..7 are compared
with the constant at `.rodata` 0x88728 (`00 8e 21 77`, loaded as the 8-byte literal `00 8e 21 77 ab ab ab ab`, only 4 bytes
compared); if all four match, it prints and runs through `system()`:
```
/etc/fw_setenv ota_upgrade_status 1
reboot
```
(`fw_setenv` is the U-Boot environment writer, `rootfs-customer.md` §2.7.) U-Boot then takes `ota_upgrade_status` = "1" into the `ufu` USB download (`uboot.md` §4.4); prog_uvc does no transfer itself.

### 4.8 Field update over XU1 (DFU / MTD) [R]

The SDK's receive-and-write path, identical to the skyipc source (with its progress prints `Downloading…` and `"\rWriting… %3d%%"`). Entry
points are XU1 cs 4 sub-commands 0 / 1 (init / end) and cs 3 (data). State in the device struct at +0x170
(`UVC_DFU_STATE_*`):

| function | addr | action |
|---|---|---|
| `UVC_DFU_Init(pdev, fw_size)` | 0x2ed10 | state must be IDLE (0); `malloc(fw_size)` → +0x160, size → +0x164, cur = 0, cnt = 0, state → INIT (1) |
| `UVC_DFU_Update(pdev, data, size)` | 0x2eda4 | state INIT or DNL; size clamped to the rest; copy into the buffer at cur; cur += size; state → DNL (2); progress every 32 chunks |
| `UVC_DFU_DnlEnd(pdev)` | 0x2ee88 | state must be DNL; state → DNL_END (3); `sem_post(dfu_sem)` |
| `UVC_Fw_Update_Task` | 0x2ea88 | thread started by `ST_UVC_CreateDev` (`sem_init(dfu_sem, 0, 0)`, `pthread_create`); waits on the semaphore; on DNL_END parses and writes |

**Image container:** a sequence of records, each a 32-byte header + payload: header[4..5] u16 `partition_num` (≤ 4 used),
header[6+2i..7+2i] u16 `partition_type[i]` (index into the name table), header[16..19] u32 LE `FwDataSize`, payload at
header+32. For each entry the same payload is written to each named partition (`UVC_MTD_Write(payload, FwDataSize,
UVC_Find_MTD(name))`), then the parser advances by 32 + `FwDataSize` until the buffer is consumed.

**Partition-name table** (12 entries of 64 bytes at `.rodata` 0x88e9c, copied with one memcpy): 0 IPL, 1 IPL_CUST0, 2
IPL_CUST1, 3 BOOT, 4 UBOOT1, 5 ENV0, 6 MISC, 7 nvrservice, 8 rootfs, 9 KERNEL, 10 RECOVERY, 11 customer. The camera's
`/proc/mtd` has `BOOT KERNEL rootfs miservice customer MISC` (`firmware-image.md` §1.1), so only BOOT, KERNEL, rootfs,
customer and MISC can be targets; for the other names `UVC_Find_MTD` returns −1 and `UVC_MTD_Write` rejects it ("Invalid
mtd_num") [R table; I that a host image can only land on those five].

**`UVC_MTD_Write(data, size, mtd_num)`** (0x2e8b8): opens `/dev/mtdN` O_RDWR, `MEMGETINFO` (0x80204d01), erases every erase
block of the whole device (`MEMERASE` 0x40084d02), write unit 512 for NOR (`type == MTD_NORFLASH == 3`), else `writesize`;
writes `size` bytes in write-unit steps with `lseek` + `write`. **`UVC_Find_MTD`** (0x2e84c) returns the index of the
matching name in `/proc/mtd` (§3.6).

The streaming interface's alternate settings with `streaming_maxpacket=3072` (alt 0 + 15 high-speed alts, the largest
3072 B / µframe) are the gadget's — `usb-gadget.md` §3.6.

## 5. HDMI output and the display path (`-H`)

### 5.1 The 4K / 1080p decision [R]

`St_HdmiModuleInit` (0x18384) with `hdmi_pick_mode`: read the EDID (`MI_HDMI_ForceGetEdid`) and the sink info. **4K30 (HDMI
timing 13)** only if all three hold: the first detailed timing's horizontal active (EDID byte 0x38 + the high nibble of 0x3a)
is ≥ 3840; the sink info's native timing is not 1080P60 (9); the monitor-range-limits descriptor (`00 00 00 FD`) has a minimum
horizontal rate < 35 kHz. Otherwise **1080P60 (timing 9)**. The result goes to `g_ad484` (1 = 4K). The first call only
detects; later calls do `MI_HDMI_SetAttr` {bEnableVideo = 1, timing 9 / 13, output mode HDMI, colour YCbCr444, deep colour 24
bit} and `MI_HDMI_Start`. `MI_HDMI_Init` registers the event callback 0x1912d (= `hdmi_event_cb` in Thumb). The binary also
holds an EDID parser and a timing-name table `[1] 720P_50 [2] 720P_60 [3] 1080P_50 [4] 1080P_60 [5] 1600x1200_60
[6] 1440x900_60 [7] 1280x1024_60 [8] 1024x768_60 [9] 2560x1440_30 [10] 3840x2160_30` with the strings `Disp_ChangeTiming`,
`RecognizeHDMI`, `flag_4k`, `1080P value==0/!=0`, `flag_Horizontal_scan_range_30fps`.

Hot-plug: the callback re-runs the EDID decision and rebuilds HDMI + DISP + SCL when 4K / 1080p changes. The statically linked
libmi_hdmi polls `MI_HDMI_GetAttr` (ioctl 0xc0346905; `Attr.bConnect` is the first field) every 500 ms in its event thread;
switching the monitor's input shows as a short unplug / replug. The ioctl 0xc00c690d is `MI_HDMI_SupportTiming`, whose status
word is a timing id. Timing ids are this build's `MI_HDMI_TimingType_e` (`D3_hdmi.txt`) and, for DISP, mi_disp's timing order
(1080P60 0x10, 3840x2160P30 0x21; `mi_disp-mi_hdmi.md` §4.6.1).

### 5.2 Display, HDMI start and stop, the SCL output for HDMI [R]

- **`St_DispModuleInit`** (0x18d74): `MI_DISP_SetPubAttr` {background 0x800080, interface 5 (HDMI), sync 0x10 (1080P60) or
  0x21 (3840x2160_30)} → Enable → BindVideoLayer → layer attr 1920×1080 or 3840×2160 → EnableVideoLayer → input port 0 {window
  0, 0, W, H; source W × H; NV12} → enable → sync mode 2 (FREE_RUN). `St_DispModuleDeinit` 0x18bf8. The DISP device
  (`g_ad488`) and video layer (`g_ad48c`) are both 0 (§10).
- **`init_hdmi`** (0x1e950): crop mode from `hdmi_crop_mode` → HDMI init → DISP init → `change_scl_resolution` → bind SCL(dev 0,
  chn 0, port 0) → DISP(dev, 0, 0) FRAME_BASE 60:60 (no rate conversion). **`deinit_hdmi`** (0x1ea18) reverses it (unbind,
  DISP deinit, HDMI stop / close / deinit). `St_HdmiModuleDeinit` (0x18aa8) also kills the boot animation once (`pkill -f
  "/etc/bootAnim/bootAnim -f /misc/sigmastar_640x368.es -i 1"`).
- **`change_scl_resolution`** (0x17d4c): disables SCL dev 0 chn 0 port 0, sets its output to the HDMI size (1920×1080 or
  3840×2160) with the input crop of the crop mode `g_a97b1`, enables it:

| crop mode `g_a97b1` | input crop of the 3840×2160 frame | [I] aspect |
|---|---|---|
| 1 | none (3840 × 2160) | 16:9 |
| 2 | x 192 .. 3648 (3456 × 2160) | ≈ 16:10 |
| 3 | x 480 .. 3360 (2880 × 2160) | 4:3 |

- The crop mode comes from the first character of `/misc/ws.txt` ('1'..'3', default 1), read by `hdmi_crop_mode`; the menu's
  `tailoring_init` changes it at run time (§7.1).

### 5.3 HDMI and USB share SCL port 0 [R unless tagged]

The HDMI picture and the USB stream are both SCL dev 0 chn 0 port 0, which is why `UVC_StartCapture` turns HDMI off and
`UVC_StopCapture` turns it back on [I]. Port 1 is used by the photo, the 4K recording in 1080p HDMI mode and the split view.
On the scaler: at an SCL clock of 432 MHz the SCL cannot write a 3840-wide output at a 9.48 µs sensor line
(ISP P0 FIFO FULL → about 12 % green frames: well-formed JPEGs holding a broken picture, ~1.34× the neighbours' size); at
480 MHz it is clean [R; I ~1 px / clk]. Enabled ports share the scaler — the cost is output pixels per sensor line time summed
over every enabled port: HDMI 1080p + USB 4K at 9.48 µs gives a few overflows, at 9.85 µs none; HDMI 4K together with a fast
USB mode overflows heavily on both outputs (1080-row modes upscaled to 4K ≈ 1 Gpix/s), only a 10.8 µs line (4K40) is clean;
frame-rate control on the SCL → DISP bind does not reduce it [I: the SCL computes every input line for every enabled port].
MIU arbiter settings (`/sys/devices/system/miu/miu_arb0`: SC0_FRM_W priority, JPE_R flow control, group-2 / 3 high-priority
masks, USB20 priority / burst no-limit) have no measurable effect on these limits. The MIU group-0 high-priority mask
0x1F202490 is 0xFFFF after boot; writing 0 brings back about 45 green frames per 10 s
(`kernel-platform.md` §12.4 for the MIU arbiter).

## 6. The menu link: TCP 8888 server (kjd_lvgl is the client)

### 6.1 Server and threads [R unless tagged]

`menu_server_start`: first loads the saved settings (`exposure_file_create`, §6.4), then `socket(AF_INET, SOCK_STREAM)`, bind INADDR_ANY:
**8888** (`sockaddr_in` at 0xad660, `socklen` at 0xad65c), `listen(3)`. The accept loop `menu_server_accept` is called from main
after the UVC init and never returns; it starts one thread per client (`pthread_t` at 0xad670, handler `menu_client_thread`; the
string `"Could not create thread\n"` 0x86e58 is its error text). The thread receives the address of the global socket-fd
variable, and on disconnect or a recv error does `close(fd); free(arg)` — `free()` of a non-heap address. [I] glibc aborts on
that, so prog_uvc dies whenever the menu client disconnects; not checked on the camera, and the exact glibc behaviour is
inference too. kjd_lvgl connects to
127.0.0.1 (`rootfs-customer.md` §3.3); the app side logs `Received msg_node: ID = %d, msg = %s`. Any connection that writes to
:8888 is a command (a probe sending "\n" is id 0x0a = freeze exposure, answered by a 0xaa reply).

### 6.2 Packet and command table [R unless tagged]

Packet (both directions): fixed **0x41 bytes** = `u8 id` + `char text[0x40]` (NUL-padded ASCII, numbers as decimal text).
Requests with id 0 or > 0x20 are ignored; replies are sent only where listed. "v" = `atoi(text)`, "f" = `atof(text)`. ISP calls
are Get → modify → Set on ISP channel (0, 0); struct names are the SigmaStar ISP headers' (fields at the offsets the code
writes). kjd_lvgl sends every id except 0x10, 0x1c (reply only) and 0x20 (`kjd_lvgl.md` §4.2).

| id | action in prog_uvc | reply |
|---|---|---|
| 0x01 | brightness: `IQ_BRIGHTNESS` {bEnable = 1, enOpType = 1 manual, manual.u32Lev (+0x48) = v·2} | — |
| 0x02 | contrast: same layout, u32Lev (+0x48) = v·2 | — |
| 0x03 | saturation: `IQ_SATURATION` (0x1a0) {1, 1, u8 at +0x188 = (u8)(v·2.5)} | — |
| 0x04 | sharpness: `IQ_SHARPNESS` (0x1328) {1, 1, u8 +0x12fb = v, u8 +0x12fc = v < 4 ? 0 : v − 4} | — |
| 0x05 | `IQ_SetColorToGray` = (v == 1) | — |
| 0x06 | anti-flicker: v == 1 → 50 Hz (2), else 60 Hz (1) | — |
| 0x07 | `mirror_display(v)` (§7.1) | — |
| 0x08 | `tailoring_init(v)` (§7.1) | — |
| 0x09 | AE mode: v == 1 → manual (4), else auto (0) | — |
| 0x0a | v == 1: AE target LUT u32Y[3..15] = 350, AE auto. Else: freeze — read `AE_QueryExposureInfo`, AE manual with that FNx10 / SensorGain / ISPGain / US | else-branch: `0xaa "SensorGain-FNx10-ISPGain-US"` |
| 0x0b | EV compensation f: 0 < f ≤ 3 → u32Y[3..15] = 300f + 350; f < 0 or f > 3 → 100(f + 3) + 50; f == 0 → targets untouched; then AE auto | — |
| 0x0c | AWB: v == 1 → eOpType = 1 manual; else eOpType = 0 auto and auto-attr field +0x14 = 1 | — |
| 0x0d | v == 1: AWB auto. v == 2: AWB manual with the current R / Gr / B from `AWB_QueryInfo` (Gb kept) → reply. v == 3: remember the current exposure, AE auto, AWB auto. Other: AWB manual (current gains) + AE manual with the remembered exposure → reply | v == 2 / other: `0xdd "R-Gr-B"` (gains >> 5) |
| 0x0e / 0x0f / 0x11 | AWB manual gain R / Gr / B = v << 5 (Gb never set; 0x10 has no handler) | — |
| 0x12 | `zoom_init(v)` (§7.1) | — |
| 0x13 / 0x14 / 0x15 | photo 1280×720 / 1920×1080 / 3840×2160 to `/mnt/PHOTO/P%05d.jpg` (number = v + 1) (§7.3) | — |
| 0x16 | start MP4 recording 1920×1080 to `/mnt/VIDEO/V%05d.mp4` (number = v + 1) (§7.4) | — |
| 0x17 | start MP4 recording 3840×2160, same name scheme; HDMI in 4K → straight from SCL port 0, else through port 1 scaled to 3840×2160 | — |
| 0x18 | stop recording; v == 2 (4K) with HDMI in 4K leaves port 1 alone, otherwise SCL port 1 is disabled | — |
| 0x19 | compare / freeze (§6.3) | — |
| 0x1a | photo viewer: on first use `sync` + `kjd_photo_init()`; decode the JPEG at path `text` through JPD (`photo_view_decode`) (§7.6) | — |
| 0x1b | video player: play the file `text` (VDEC, `mp4_player_start`) (§7.5) | `0x1c "<duration s>"` (MP4 video-track duration / 90000, as 64-bit decimal) |
| 0x1d | viewer / player control: 1 = `sync`, `kjd_photo_deinit()`, `sync`; 2 = pause player; 3 = resume player; 4 = stop player (DISP back to the camera) | — |
| 0x1e | HDMI ↔ USB / power (§6.3) | — |
| 0x1f | v == 1: restore defaults — apply the in-memory default settings (`exposure_file_apply`), `mirror_display`, `zoom_init` | `0x1f "2"` |
| 0x20 | WDR: `IQ_WDR` (0x668) {1, 1, u8 +0x611 = v} | — |

### 6.3 Ids 0x19 and 0x1e [R unless tagged]
<!-- ev:prog_uvc.6.3 -->

The calls were identified by the device each wrapper's `_MI_WRAPPER_DEVICE_Open` copy opens and the ioctl it issues, matched to
the camera's `.so` exports; no MI_VDISP call is involved:

| prog addr | device | ioctl | function |
|---|---|---|---|
| 0x36294 | /dev/mi_isp | 0x40086908 | `MI_ISP_StartChannel(dev, chn)` |
| 0x362f4 | /dev/mi_isp | 0x40086909 | `MI_ISP_StopChannel(dev, chn)` |
| 0x7f170 | /dev/mi_disp | 0x4008691a | `MI_DISP_PauseInputPort(layer, port)` |
| 0x7f1d0 | /dev/mi_disp | 0x4008691b | `MI_DISP_ResumeInputPort(layer, port)` |
| 0x7f294 | /dev/mi_disp | 0x400c692a | `MI_DISP_ClearInputPortBuffer(layer, port, bClear)` |
| 0x7f040 / 0x7f0b0 / 0x7f110 | /dev/mi_disp | 0x40146914 / 0x40086916 / 0x40086917 | `MI_DISP_SetInputPortAttr` / `EnableInputPort` / `DisableInputPort` |
| 0x3e5a4 / 0x3e620 / 0x3e6a0 / 0x3e704 | /dev/mi_scl | 0x4024690a / 0xc024690b / 0x400c690c / 0x400c690d | `MI_SCL_Set/GetOutputPortParam`, `Enable/DisableOutputPort` |
| 0x841b4 | /dev/mi_sys | 0x40186915 | `MI_SYS_SetChnOutputPortDepth` |

**Id 0x19 — compare / freeze** (v = `atoi(text)`):

| v | calls | effect [I unless noted] |
|---|---|---|
| 1 | `MI_DISP_ClearInputPortBuffer(0, 0, 1)`; `MI_DISP_ResumeInputPort(0, 0)`; `kjd_stop_vdisp()` | leave the split / frozen view: drop the frozen frame, resume input port 0, back to one full-screen SCL port 0 → DISP |
| 2 | `MI_ISP_StopChannel(0, 0)` | freeze the whole picture: the ISP stops producing frames, HDMI keeps the last one |
| 3 | `MI_ISP_StartChannel(0, 0)`; `kjd_start_vdisp()`; `usleep(300000)`; `MI_DISP_PauseInputPort(0, 0)`; unbind {SCL,0,0,0} → {DISP,0,0,0} 30:30 type 1 | compare view: left half (input port 0) paused on its last frame and cut off from SCL, right half live |
| 4 | `MI_ISP_StartChannel(0, 0)` | unfreeze after v = 2 |
| other | nothing | |

What the compare view looks like on a monitor has not been observed.

**Id 0x1e — HDMI ↔ USB / power**, through `standby_set(m)`: stores `g_ad480 = m` (the fake power-off flag that makes
`UVC_StartCapture` return 0 at once, §3.2); m = 1 → `deinit_hdmi()` and, if the gadget was brought up at boot (`g_ad490`),
`ST_UvcDeinit()`; m = 0 → `init_hdmi()` and, if `g_ad490`, `ST_UvcInit(stream count g_a97b4, maxpacket {1024, 1024, 192, 64}
(.rodata 0x86d04), mult bytes {2, 2, 2, 2}, burst bytes {13, 13, 0, 0}, interface ids 0)` — hard-coded here, where main passes
its getopt arrays; with demo.sh's start line (no `-p` / `-m` / `-B` / `-I`) they are the defaults [I]. The case first prints
`"llllllllllllllllllllllllllll"`; neither 0x19 nor 0x1e sends a reply.

| v | calls |
|---|---|
| 0 | `standby_set(1)` — HDMI and USB camera off ("sleep") |
| 1 | `MI_ISP_StartChannel(0, 0)`; `usleep(100000)`; `standby_set(1)` — undo a 0x19 / 2 freeze first |
| 2 | `MI_DISP_ClearInputPortBuffer(1, 0, 1)`; `MI_DISP_ResumeInputPort(1, 0)` (**layer 1**, which does not exist — the active layer is 0, so both calls fail harmlessly [I]); `kjd_stop_vdisp()` (which itself resumes layer 0 port 0); `usleep(100000)`; `standby_set(1)` — leave split view first |
| 3 | `standby_set(0)` — HDMI and USB camera on again |

### 6.4 Saved settings: `/customer/ws.bin` and `/customer/default_ws.bin` (0x58 bytes, u32 little-endian) [R unless tagged]
<!-- ev:prog_uvc.6.4 -->

At server start (`exposure_file_create`): if `/customer/ws.bin` is missing → create both files (touch + chmod 777), fill a struct from
the current ISP state, write it to **both** files, read `default_ws.bin` back. Otherwise → read `ws.bin` and apply it
(`exposure_file_apply`), then read `default_ws.bin`. [I] So `default_ws.bin` is the ISP state at first boot. Who writes `ws.bin` after the first boot is not traced (the client side
is in `kjd_lvgl.md` §3.2).

| off | field | values / apply rule |
|---|---|---|
| 0x00 | brightness | IQ brightness u32Lev / 2 |
| 0x04 | contrast | / 2 |
| 0x08 | saturation | u8 / 2.5 |
| 0x0c | sharpness | applied as in id 0x04 |
| 0x10 | grey | 1 = colour-to-grey on, 2 = off |
| 0x14 | flicker | 1 = 50 Hz, 2 = 60 Hz |
| 0x18 | mirror | `mirror_display(x)` unless 1 |
| 0x1c | tailoring (crop) | `tailoring_init(x)` unless 1; first-boot value = a global byte |
| 0x20 | AE manual flag | 1 → AE manual with 0x38..0x44 |
| 0x24 | EV compensation (**float**) | same rule as id 0x0b when 0x20 ≠ 1 |
| 0x28 | AWB | 1 = manual with 0x48..0x50, else auto |
| 0x2c | — | not used by load / apply |
| 0x30, 0x34 | sharpness bytes (+0x12fb / +0x12fc at first boot, 0 → 1) | not used by apply |
| 0x38, 0x3c, 0x40, 0x44 | SensorGain, FNx10, ISPGain, exposure µs | AE manual values |
| 0x48, 0x4c, 0x50 | AWB R, B, Gr gain >> 5 | AWB manual values |
| 0x54 | zoom | `zoom_init(x)` unless 0 |

Apply order (`exposure_file_apply`, also called by `UVC_StopCapture` and by id 0x1f with the default copy): AE (manual, or EV target +
auto) → AWB → brightness → contrast → saturation → sharpness → flicker → grey → mirror → zoom → crop. Functions: 0x258cc
ws.bin write, 0x25944 ws.bin read + apply, 0x259c0 default_ws.bin write, 0x25a38 default_ws.bin read.

Decoded from the dump: `ws.bin` = brightness 25, contrast 25, saturation 14, sharpness 24
(bytes 24 / 20), grey 2 (off), flicker 1, mirror 1, crop 1, AE flag 2 (EV path, EV 0.0), AWB 2 (auto), exposure SensorGain 1024
/ FNx10 16 / ISPGain 1024 / 16666 µs, AWB R 52 B 50 Gr 32, zoom 0. `default_ws.bin` is the same except AE flag 1 (manual), AWB 1
(manual), R 70 B 51.

## 7. Picture functions of the menu

### 7.1 Mirror, crop, zoom (SCL dev 0 chn 0 port 0 — the picture going to HDMI and USB) [R unless tagged]

- **`mirror_display(v)`** (0x214f8): SCL output-port param bMirror / bFlip = (0, 0) for v = 1, (1, 0) v = 2, (0, 1) v = 3,
  (1, 1) v = 4.
- **`tailoring_init(v)`** (0x21b60): stores the crop mode in `g_a97b1` and sets the port's input crop of that mode exactly as
  `change_scl_resolution` (§5.2), output = HDMI size.
- **`zoom_init(z)`** (0x21e48): z = 0 → crop back to the current crop mode's full area. z > 0 → factor = 1 + 0.1·z; window =
  (area width / factor) × (2160 / factor), centred in the crop-mode area (area width 3840 / 3456 / 2880, the area's x offset
  added). Output size: 1080p mode → always 1920×1080; 4K mode → the window size itself (no upscale back to 3840×2160), and with
  z = 0 in crop modes 2 / 3 the 4K output is the crop width (3456 / 2880) × 2160. In 4K the DISP input port stays configured
  for 3840×2160; whether DISP then scales a zoomed 4K picture or shows it smaller is not known (only the camera can show it).

### 7.2 Split view (the vendor's `kjd_*vdisp*` functions; no mi_vdisp) [R]

All on SCL dev 0 chn 0 and DISP dev 0 layer 0. Bind records are `ST_Sys_Bind/UnBind` {src module, dev, chn, port} → {dst …},
src fps : dst fps, type (1 = FRAME_BASE); for DISP the bind "chn" is the input port.
- **`kjd_vdisp_init`** (0x242fc): `MI_SCL_GetOutputPortParam(0, 0, 0)` (struct 0x18: crop x, y, w, h u16; out w, h u16;
  bMirror u8 +0xc; bFlip u8 +0xd; pixel u32; compress u32) → `MI_SCL_DisableOutputPort(0, 0, 0)` → out width /= 2, crop width
  /= 2 (crop width 0 → crop 1920×2160). If bMirror and bFlip are both set (mirror mode 4): port 1 gets the left half (crop x
  unchanged) and is enabled, then port 0 gets crop x += half width and is enabled. Otherwise port 0 = left half, port 1 = right
  half (crop x += half width), both enabled. `MI_SYS_SetChnOutputPortDepth({0x22, 0, 0, 1}, 1, 8)`. DISP: `DisableInputPort(0,
  0)`; input port 0 attr {window x 0, y 0, w 960, h 1080 (1080p) / w 1920, h 2160 (4K, `g_ad484` = 1); src w = half the crop
  width, src h = the crop height}; enable; input port 1 attr {x 960 / 1920, same size and source}; enable. Each half of the
  crop-mode picture is on its own SCL port, shown side by side; the output per port is half the HDMI width × full height
  (960×1080 or 1920×2160).
- **`kjd_vdisp_deinit`** (0x24a64): disable SCL port 0; take port 0's param (port 1's in mirror mode 4, which holds the original
  crop x), double width and crop width, set and enable port 0; disable SCL port 1; disable DISP input ports 0 and 1; DISP input
  port 0 attr {0, 0, 1920×1080 or 3840×2160, source = the full crop}; enable; `ResumeInputPort(0, 0)`.
- **`kjd_start_vdisp`** (0x24edc): unbind {SCL,0,0,0} → {DISP,0,0,0} 30:30 type 1; `kjd_vdisp_init`; bind {SCL,0,0,0} →
  {DISP,0,0,0} 30:30 type 1; bind {SCL,0,0,1} → {DISP,0,1,0} 30:30 type 1.
- **`kjd_stop_vdisp`** (0x25090): unbind {SCL,0,0,1} → {DISP,0,1,0}; `kjd_vdisp_deinit`; bind {SCL,0,0,0} → {DISP,0,0,0}
  30:30 type 1. So after a split-view round trip the SCL → DISP bind is 30:30, not the 60:60 of `init_hdmi`.

### 7.3 Photo (ids 0x13–0x15) [R]

`UVC_StartCapture_photo(n)` (0x15638) sets SCL dev 0 chn 0 **port 1** to 1280×720 / 1920×1080 / 3840×2160 (n = 0 / 1 / 2),
NV12, mirror / flip from `g_ad5b2` (0 none, 1 flip, 2 mirror + flip; read once at 0x156f2); JPEG VENC dev 8 chn 1 (max
3840×2160, 30 fps, Qfactor 50); bind SCL port 1 → VENC 30:30. `photo_write_first_frame` writes the first frame to
`/mnt/PHOTO/P%05d.jpg` (number = v + 1), then tears the channel down (`Destroy_photo_video_venc_1080p(8)`, 0x159f8) and runs
`sync`. Entry points 0x15f10 / 0x15f2c / 0x15f48 (720p / 1080p / 4K). `g_ad5b2` is never written (§10), so it stays 0 and photos are never
mirrored or flipped.

### 7.4 MP4 recording (ids 0x16–0x18) [R]

`KJD_Video_Record(path, W, H, scl_dev, scl_chn, use_port1)` (0x179f0): with `use_port1`, SCL port 1 = W × H, output depth 5;
an H.264 VENC created once (`St_VencModuleInit` 0x16ab4: max 3840×2160, picture W × H rounded down to 32, 30 fps, bitrate
10 Mbit/s); bind SCL → VENC (src 60, dst 30); thread `KJD_video_mp4_pth` (0x16ea4): waits for the first IDR, takes SPS / PPS
from it (`ParseSPSPPSFromVENCData` 0x16c2c), `MP4Create` + `MP4AddH264VideoTrack` (timescale 90000), writes every frame with
the duration from the VENC PTS delta (3000 = 1/30 s for the first), adds SPS / PPS again when they change, stops when
`g_ad43c` is set, `MP4Close` + `sync`. The flag is set by `record_set_stop_flag` (0 on start, 1 on stop); `KJD_Close_Record` 0x17c3c.
File name `/mnt/VIDEO/V%05d.mp4`. The MP4 code is libmp4v2 (`/customer/lib/libmp4v2.so.2`).

### 7.5 MP4 player (ids 0x1b, 0x1d) [R]

VDEC dev 0 chn 0, H.264 1920×1080, created once (`StartVdec` 0x14c98); `sstar_module_bind` (0x14e14) switches the DISP input
from SCL to VDEC (30:30), `sstar_module_unbind` (0x14fa0) back. `mp4_open` opens the MP4 (libmp4v2), its duration in s
(`mp4_duration`) goes to the reply `0x1c`, the thread `mp4_feed_thread` sends samples to VDEC every 33 ms; pause / resume = the
global `g_ad418` (0x1d v 2 / 3); stop (0x1d v 4) switches DISP back to SCL. Start / stop / pause / resume: 0x15578 / 0x155e4 /
0x15600 / 0x1561c. How a 4K MP4 plays through the 1920×1080 VDEC channel is not known (needs the camera).

### 7.6 Photo viewer (ids 0x1a, 0x1d v 1) [R]

`kjd_photo_init` (0x162d4) stops the camera's ISP channel, creates SCL dev 1 chn 0 (output = the DISP layer size from
0x160e0, NV12), JPD dev 0 (`St_JpdModuleInit` 0x161d4: max 8192×8192, 10 MB buffer), binds JPD → SCL1 → DISP; each 0x1a
decodes one JPEG file through JPD (`photo_view_decode` reads the file into the JPD stream buffer); `kjd_photo_deinit` (0x167a8)
restores SCL0 → DISP and restarts the ISP. JPD module deinit: `St_JpdModuleDeinit` (0x16124).

## 8. The LDC path (`-L <json>`, unused) [R unless tagged]

The shipped start line has no `-L`, so none of this runs; demo.sh loads mi_ldc.ko at every boot all the same
(`rootfs-customer.md` §3.2). The channel attribute (0x54 bytes), the JSON and `CalibPoly_new.bin` formats and the kernel side
are in `ldc-1-stack.md`.

1. **Option** (main, case 'L'): `strcpy(g_ad5b4, optarg)` (no length check), prints "g_LdcJsonPath:%s", `g_ad5b0` = 1.
2. **Start-up** (`ST_VideoModuleInit`, after the ISP → SCL bind): `MI_LDC_CreateDevice(0,
   &attr4)` (attr zeroed, not sent); channel attr 0x54 zeroed, **eMode = 1** (LDC); JSON: `access(path)`, then
   `ReadBufFromFile` (stat size, `malloc`, `fread` of the whole file, no NUL) → attr+0x38 / +0x3C; a missing file → "Json Path
   is not set" and the attr keeps NULL / 0; **`./CalibPoly_new.bin` relative to the working directory** read the same way →
   attr+0x44 / +0x48, **eInfoType (+0x40) = 1** (sensor calibration). Only a failed *read* (stat ok, malloc / fread failed) skips
   `MI_LDC_CreateChannel(0, 0, &attr)`; a *missing* file only prints and the channel is created with NULL / 0 there (the kernel
   then fails at table build, "Invalid Config", CreateChannelV3 returns an error and init returns it) [R control flow; I kernel
   result]. Both buffers are freed (the kernel has copied them); `MI_LDC_StartChannel(0, 0)` is called unconditionally after
   that block. `MI_LDC_SetOutputPortAttr` is never called, so the channel's output pixel format stays 0.
3. **Per stream** (`UVC_StartCapture`, MJPEG / H.26x branch): SCL output pixel forced to NV12; the bind list grows from 2 to 3
   entries: ISP(0x21) → SCL(0x22) as before, **SCL(0x22, dev, chn, port) → LDC(0x17, 0, 0) FRAME_BASE** with src / dst fps =
   host fps, and **LDC(0x17, 0, 0, port 0) → VENC(2, venc dev, chn) FRAME_BASE** — both FRAME_BASE whatever `-H` says (the
   non-LDC SCL → VENC bind would be REALTIME / HW_RING without `-H`); the depth list adds LDC (0, 0, port 0) = (0, 5).
4. **Stream stop** (`UVC_StopCapture`): `MI_VENC_StopRecvPic`, **`MI_LDC_StopChannel(0, 0)`**, SCL / ISP output ports off,
   unbind LDC → VENC, SCL → LDC (and ISP → SCL when allowed), `MI_VENC_DestroyChn`, **`MI_LDC_DestroyChannel(0, 0)`**. The LDC
   channel is created only at start-up, so **after the first stream stop there is no LDC channel**: the next stream's
   SCL → LDC → VENC binds go to a destroyed channel and the USB stream gets no frames [R create / destroy sites; I effect]. The
   device is never destroyed; process exit closes `/dev/mi_ldc` and mi_ldc's `DeInit` cleans up.

What the hardware does with a real JSON through this path (the tile layout and clock in `/proc/mi_modules/mi_ldc/mi_ldc0` —
NumRuns, block sizes — and mhal's `check_*` attributes such as `check_clk`, `check_inst_table_info`; a stock table dumped with `dumptablebin` or a DBIN container fed through `-L`) is known only from
static analysis.

## 9. Audio: the UAC gadget side and the old demo audio API (unused) [R unless tagged]

### 9.1 Why it does not run

`ST_UacInit` (0x1a038) returns at once unless `-d` (`g_ad636`) or `-D` (`g_ad637`) is given, and demo.sh passes neither;
g_webcam.ko is also loaded with `uac_function_enable=0` (§2.2). The camera's audio modules (mi_ai 9 AI devices, mi_ao 6 AO
devices, the HAL) are `audio.md`.

### 9.2 The MI side (`st_main_uvc.cpp`, old demo API)

`ST_UacInit`: mode = 2 if `-D`, | 1 if `-d`; `ST_UAC_AllocStream`; handle +0x18 = mode; the ops are copied into the handle —
**+0x40 the AO ops** (table `.data` 0xa9778), **+0x54 the AI ops** (table 0xa978c):

| ops | Init | Fill / Take | Deinit | SetVol | SetMute |
|---|---|---|---|---|---|
| AO (live) | `ST_AO_Init` 0x197bc | `AO_TakeBuffer` 0x19eb0 | `ST_AO_Deinit` 0x1991c | — | — |
| AI (live) | `ST_AI_Init` 0x199ec | `AI_FillBuffer` 0x19f5c | `ST_AI_Deinit` 0x19c84 | `ST_AI_SetVqeVolume` 0x19e4c | `ST_AI_SetMute` 0x19de8 |
| file mode (`-f`) | `ST_AO_FILE_Init` 0x194b8 / `ST_AI_FILE_Init` 0x195d0 | `AO_FILE_TakeBuffer` 0x19714 / `AI_FILE_FillBuffer` 0x19754 | `ST_AO_FILE_Deinit` / `ST_AI_FILE_Deinit` | — | — |

Then the playback config (handle+8): rate (+0x18) = the `-S` value, channels (+0x14) = 2; the capture config (handle+4):
+0x18 = 0, +0x14 = 2, +0x10 = −22; `ST_UAC_StartDev`. The calls:
- **`ST_AO_Init(rate)`**: `MI_AO_SetPubAttr(dev, {rate, 16-bit, I2S master, **stereo**, frmNum = (stack garbage),
  ptNumPerFrm = 1024, codecChnCnt = (garbage), chnCnt = 1, I2S fmt 0, mclk 0, sync 1, TDM slots 8, I2S width 16})`,
  `MI_AO_Enable(dev)`, `MI_AO_EnableChn(dev, 0)`; mi_ao does not read the two uninitialised fields.
- **`AO_TakeBuffer(buf)`**: `MI_AUDIO_Frame_t` (0x120 B) zeroed, `apVirAddr[0]` (+8) = buffer data, +0xc = 0, `u32Len[0]`
  (+0x54) = length; `MI_AO_SendFrame(dev g_ad638, chn g_ad63c, &frame, −1)` repeated while it returns 0xA005200D. The data is
  the interleaved stereo PCM the host sent over UAC.
- **`ST_AO_Deinit`**: `MI_AO_DisableChn(dev, 0)`, `MI_AO_Disable(dev)`.
- **`ST_AI_Init(rate)`**: `MI_AI_SetPubAttr(dev, {rate, 16-bit, master, **stereo**, frmNum 16, ptNumPerFrm 1024,
  codecChnCnt 0, chnCnt 1, fmt 0, mclk 0, sync 1, TDM 8, width 16})`, `MI_AI_Enable`, `MI_SYS_SetChnOutputPortDepth({4, dev,
  0, 0}, user depth 1, total 8)`, `MI_AI_EnableChn(dev, 0)`, then a default "volume" by device: dev 0 → 21, dev 3 → 7, dev 1 →
  **30**. The camera's mi_ai Dmic range is 0–4, so `-d 1` makes `MI_AI_SetVqeVolume` fail with 0xA0042003 and `ST_AI_Init`
  returns that error [R both sides; I: the demo was written for an older mi_ai where the Dmic value was dB].
- **`AI_FillBuffer(buf)`**: `MI_AI_GetFrame(dev g_ad640, chn g_ad644, &frame, NULL, −1)` (blocking), copy `u32Len[0]`
  (frame+0x54) bytes from `apVirAddr[0]` (frame+8), buffer length = `u32Len[0]`, `MI_AI_ReleaseFrame`. In stereo mode the frame
  is the interleaved L / R pair of the device's first two slots (`audio.md` §3.6 "Capture data path").
- **`ST_AI_SetVqeVolume(min, max, cur)`** (0x19e4c, the UAC volume control → AI; its error text still says SetIfGain):
  `Vol_to_Db` (0x19d54) maps the host value linearly to the device range by AI device `g_ad640` — 0 → `(v − min)·21/(max −
  min)`, 1 → `(v − min)·90/(max − min) − 60`, 3 → `(v − min)·7/(max − min)`, other → 0 (between ikayaki — no `−min`, DMIC range
  63 — and opera — interface ids); dev 1's −60…+30 is again outside the kernel's Dmic 0–4 — then `MI_AI_SetVqeVolume(dev, 0,
  value)`. **`ST_AI_SetMute(b)`** (0x19de8) → `MI_AI_SetMute(dev g_ad640, chn g_ad644, b != 0)`.
- **`ST_AI_Deinit`**: `MI_AI_DisableChn(dev, 0)`, `MI_AI_Disable(dev)`.
- **File mode:** `ST_AO_FILE_Init(rate)` opens `<path>` + `"/wav/out/"` + `"%dK_16bit_STERO_30s.wav"` (rate / 1000) for writing
  (O_CREAT | O_TRUNC | O_RDWR, 0777), `<path>` being the 0x44-byte slot at `.data` 0xa9800; `AO_FILE_TakeBuffer` writes each
  buffer. `ST_AI_FILE_Init` opens slot 0xa97bc + `"/wav_in/"` + the same name for reading; `AI_FILE_FillBuffer` reads a buffer
  and rewinds at end of file (endless loop).
- One reading of ours names the AO data call `MI_AO_Write` (ikayaki-era API, not the newer `MI_AO_Open` / `AttachIf`);
  two others read `MI_AO_SendFrame` in `AO_TakeBuffer` from the binary.
- The `-d` / `-D` device lists of the usage text (Amic0 / Dmic1 / I2S-RX2 / Linein3; Lineout0 / I2S-TX1) are the demo's; the
  channel is always 0 (`g_ad644`, `g_ad63c`).

Data flow: `-D n` (mode bit 1) runs `UAC_Audio_Capture_Task` with the AO ops — `ST_AO_Init(rate)`, then a loop of
`ST_UAC_GetStream` (`pcm_readi` on the UAC gadget's capture PCM = the audio the **host plays**) → `AO_TakeBuffer` →
`MI_AO_SendFrame` → (kernel) `MHAL_AUDIO_WriteDataOut` → DAC / I2S / HDMI. `-d n` (mode bit 0) runs `UAC_Audio_Playback_Task` with the AI ops — `ST_AI_Init(rate)`,
a loop of `AI_FillBuffer` (`MI_AI_GetFrame` ← (kernel) `MHAL_AUDIO_ReadDataIn` ← ADC / DMIC / I2S) → `ST_UAC_SendStream` (`pcm_writei` to the gadget = audio the
**host records**); the UAC volume / mute events call `ST_AI_SetVqeVolume` (handle +0x60) / `ST_AI_SetMute`. The kernel side of
both directions (user queue, ioctl, DMA) is `audio.md` §3.6, §4.4.

### 9.3 `st_uac.c` — the UAC PCM layer (18 functions, no source in)
<!-- ev:prog_uvc.9.3 -->

Matched against the demo callers (`opera_st_main_uvc.cpp` `ST_UacInit`, skyipc `uac_uvc.cpp`), which define the ops tables and
handle layout. **Handle** `ST_UAC_Device_t`, 0x68 bytes: +0 u32 magic, top half 0xefda0000 (every entry point rejects a wrong
magic); +4 config[0] pointer (capture / in, 0x34 bytes), +8 config[1] pointer (playback / out, 0x34 bytes); +0xc / +0x10 PCM
handles (in / out, tinyalsa `struct pcm *`); +0x14 stop flag; +0x18 mode bitmask (bit 1 = OUT, bit 0 = IN); +0x1c / +0x20 /
+0x24 thread handles (control / capture / playback); +0x28 mutex; +0x40..+0x50 the first ops slot {Init, Fill / Take, Deinit,
SetVol, SetMute} (AO), +0x54..+0x64 the second (AI) (offsets from the `ST_UacInit` stores and `ST_UAC_StartDev`). **PCM
config** (0x34 bytes, built by `uac_pcm_config` (0x31e70)): {u8 card, u8 device, `pcm_config` at +4 (rate, channels, period 0x400,
period_count 4, format), volume at +0x14 (cur, min, max)}; the format argument 0x10000000 is the capture-mode marker (sets zero
byte counts), 0 = playback.

| function | addr | role |
|---|---|---|
| ST_UAC_AllocStream | 0x31fa4 | calloc the 0x68 handle + two 0x34 configs; set the magic |
| ST_UAC_FreeStream | 0x32134 | free the configs and the handle (magic-checked) |
| ST_UAC_InitStream | 0x321e0 | per direction (mask bit): copy the config, `pcm_open`, check ready |
| ST_UAC_UninitStream | 0x32370 | `pcm_close` the direction's handle |
| ST_UAC_SendStream | 0x32490 | playback: `pcm_writei` of the frame struct's frames |
| ST_UAC_GetStream | 0x32554 | capture: `pcm_readi`; sets the returned byte count |
| ST_UAC_GetPcm_BufSize | 0x31f10 | `pcm_frames_to_bytes(pcm_get_buffer_size)` for a direction |
| ST_UAC_InitCtl | 0x3263c | `mixer_open(0)` + `mixer_subscribe_events(1)` |
| ST_UAC_UninitCtl | 0x32710 | unsubscribe + `mixer_close` |
| ST_UAC_GetCtlVal | 0x327b0 | waits for a mixer event (`mixer_wait_event` / `mixer_ctl_get_event`; the call goes to the event reader 0x33d98, §9.4) → index; get the control value |
| ST_UAC_GetCtlRange | 0x3292c | `mixer_ctl_get_range_min/max` |
| UAC_Audio_Control_Task | 0x32a64 | thread: polls mixer events; event idx 0 = volume target, 1 = mute (calls SetMute), 2 = volume range → stored into the config |
| UAC_Audio_Playback_Task | 0x32cb4 | thread: on a rate change tears the stream down and re-inits it; ops Fill → `ST_UAC_SendStream`; applies SetVol |
| UAC_Audio_Capture_Task | 0x332c4 | thread: ops Init, `ST_UAC_InitStream` (mode 2), loop `ST_UAC_GetStream` → ops Take |
| ST_UAC_StartDev | 0x33548 | validate the ops per mode; init the mutex; spawn the control thread, + the capture thread if the OUT bit, + the playback thread if the IN bit |
| ST_UAC_StoptDev | 0x33754 | set the stop flag; join the active threads; destroy the mutex |
| ST_UAC_SetTraceLevel | 0x337d0 | sets the trace level `g_a9c84` (1..6) |
| TraceLevel2Str (0x31dec) + helper (0x31dd4) | — | trace-string map; a stub returning 0 |

The control thread's mixer events are the ALSA gadget controls "Playback Mute Control" (idx 0) and "Playback Volume Control"
(idx 1) of u_audio.c's `E_UAC_CONTROL_TYPE_*`; volume step 256, capture range −10..+10 dB (u_audio.h) [S]. The task's own switch (idx 0 = volume target, 1 = mute) and the
u_audio enum (idx 0 = mute, 1 = volume) disagree on which index is mute; both readings occur and nothing settles it.

### 9.4 tinyalsa copy (0x33820–0x34ebc)
<!-- ev:prog_uvc.9.4 -->

In the application area, compiled with the demo at `-O0`, Thumb-2, PIC. Its strings form one `.rodata` block 0x8a930–0x8ac57 in
source order: mixer.c (`/dev/snd/controlC%u`, the type names BOOL … INT64 Unknown), then pcm.c (`: %s` … `write error: hw 0x%x
app 0x%x avail 0x%x\n`). The block equals the string literals of upstream master from e40758b (2017-09-07) to 4c5a11d
(2018-11-28) (only `"%d\n"` extra, merged by the linker with another copy); the AOSP `google-origin` line does not match (lacks
`cannot link PCM`, `failed to sync mmap ptr`, has the access / format name tables). The code narrows it: `pcm_open` honours
`PCM_NONBLOCK` (flag 0x10 → `O_RDWR|O_NONBLOCK`) → at or after 093b878 "added PCM_NONBLOCK macro" (2017-10-12); `pcm_writei`
returns 0 after the implicit start, `pcm_sync_ptr` returns −1 without a sync_ptr, `pcm_start` syncs the pointer only with
`PCM_MMAP` and starts unconditionally → before f6fe523 (2018-04-10), c9f97da and cf5f063. No commit between 093b878 and f6fe523
touches `src/pcm.c` or `src/mixer.c`, so the copy is **upstream tinyalsa master at 093b878 (October 2017)**, between tags 1.1.1
and 2.0.0. 

40 functions are present (37 of class SDK-LIB plus 0x33c0c, 0x33d40, 0x33d6c): mixer.c — `mixer_cleanup_control`,
`mixer_close` (0x33874), `mixer_realloc_z`, `add_controls`, `mixer_open` (0x33ab8), `mixer_subscribe_events` (0x33b6c),
`mixer_wait_event`, `mixer_get_ctl` (or `_const`, identical bodies; 0x33c0c), `mixer_ctl_get_value` (0x33c4c),
`mixer_ctl_get_range_min` (0x33d40), `mixer_ctl_get_range_max` (0x33d6c), the event reader 0x33d98; pcm.c — `param_is_mask`,
`param_is_interval`, `param_to_interval`, `param_to_mask`, `param_set_mask`, `param_set_min`, `param_set_int`, `param_get_int`,
`param_init`, `pcm_format_to_alsa`, `oops`, `pcm_get_buffer_size` (0x340e0), `pcm_get_error` (0x340f8) [I: the two 11-insn
getters could be any two of the identical getters], `pcm_set_config`, `pcm_format_to_bits`, `pcm_bytes_to_frames` (0x3467a),
`pcm_frames_to_bytes` (0x34644), `pcm_sync_ptr`, `pcm_hw_mmap_status`, `pcm_hw_munmap_status`, `pcm_writei` (0x348b4),
`pcm_readi` (0x349dc), `pcm_close` (0x34ab4), `pcm_open` (0x34b48), `pcm_is_ready` (0x34d68), `pcm_prepare`, `pcm_start`,
`pcm_stop`. Struct sizes: `struct pcm` 0xd4, `struct mixer` 0x184, `struct mixer_ctl` 0x120; flags `PCM_IN` 0x10000000,
`PCM_MMAP` 1, `PCM_NOIRQ` 2, `PCM_MONOTONIC` 8, `PCM_NONBLOCK` 0x10. The st_uac layer also calls an entry at 0x33ba0, which an earlier reading of ours lists as
`mixer_consume_event/get_ctl` and the tinyalsa match does not name [R address; name not settled].

All are identical to 093b878 except two changes:

| function | change |
|---|---|
| `mixer_open` 0x33ab8 | `calloc(1, 0x184)` first, then `open("/dev/snd/controlC%u", O_RDWR)` stored straight into `mixer->fd`, `CARD_INFO`, `add_controls`; any failure → `mixer_close(mixer)` (which closes the fd only if ≥ 0). Same result as upstream (NULL on any failure, fd not leaked). |
| 0x33d98 (after `mixer_ctl_get_range_max`, end of mixer.c) | a `mixer_consume_event`-like function that upstream master only got in 2020 (bdc1fcf): `read(mixer->fd, &ev, 0x48)` (one `snd_ctl_event`); returns **−1** on a read error and otherwise **an uninitialised stack word** (upstream returns 0 / −errno). Called only by `ST_UAC_GetCtlVal`. (lists 0x33d98 as `mixer_wait_event`.) |

## 10. Writer search over the globals [R]
<!-- ev:prog_uvc.10 -->

A scan of the whole `.text` (constants propagated through registers inside each function: literal loads, `add rX, pc`, GOT
loads through the PIC base 0xa9000, `movw/movt`, `add/sub` immediates) lists every store whose address is a global and every
call passing a pointer near it; it finds the known reads too (the one read of
`g_ad5b2` at 0x156f2, all 27 reads of `g_ad678` / `g_ada8c`). No 32-bit word in the file points within ±0x10 of the first five
globals below except GOT slots (0xa9378 → 0xad5b2, 0xa9714 → 0xad5b3, and the bases 0xad670 / 0xad674 / 0xad67c, 0xada80 /
0xada84 / 0xada98 through which the code reaches 0xad678 and 0xada8c with an offset), so no write comes through a pointer held
in memory. Bulk writes near them, none reaching one on the shipped start line: `ST_DefaultArgs` memsets 0x14 bytes at 0xad548;
the `-i` path at 0xad56c and the `-L` path at 0xad5b4 are `strcpy` targets; the menu server writes `sockaddr_in` at 0xad660
(16 B), `socklen` at 0xad65c, `pthread_t` at 0xad670; the UVC control handlers `memset(0xad680, 0, 0x400)` (ends at 0xada7f)
and `sprintf` into 0xad680 (the camp.txt line buffer).

| global | meaning | writes | value at run time |
|---|---|---|---|
| `g_ad5b2` | u8, photo mirror / flip (§7.3) | **none** | 0 |
| `g_ad5af` | u8, `-c` flag (§2.1) | main only | 1 with `-c`, else 0; **never read** |
| `g_ad5b1` | u8, `-P` flag (§2.1) | main only | 1 with `-P`, else 0; **never read** |
| `g_ad678` | u32, ISP channel of the UVC GET answers (`_UVC_Events_Process_Control`, 14 loads) | **none** | 0 |
| `g_ada8c` | u32, ISP channel of the UVC SET handlers (`usb_vc_pu_cs_out`, 13 loads) | **none** | 0 |
| `g_ad488`, `g_ad48c` | DISP device, video layer (§5.2) | **none** | 0, 0 |

## 11. Code that is not reached or not there [R unless tagged]

- **SAR-ADC key reader: strings only.** `.rodata` holds, in source order between the menu-server strings (`"Could not create
  thread\n"` 0x86e58) and main's getopt string (0x86f3c): `"/dev/sar"` 0x86e74, `"+++++++++++%d++++++++ /dev/sar open
  error\n"` 0x86e80, `"+++…sar_fd+++…%d+++…"` 0x86eac, `"ADC read stCfg.adc_value=:%d\n"` 0x86f1c. No pc-relative address
  build lands in 0x86e5c–0x86f3b, no 32-bit word in the file points into that range, and there is no function body: the code
  ends with the accept loop's literal pool at 0x275a0 and main starts at 0x275a4, with nothing unreferenced between them, and
  the library-area cuts are all library code. [I] The vendor compiled a SAR reader into st_main_uvc.cpp and the linker dropped
  its unused function section (`--gc-sections`) while the merged string section stayed; its strings describe the same
  `/dev/sar` protocol kjd_lvgl uses (`ioctl 0x6100` INIT, `ioctl 0x6101 {u32 ch, s32 value}`; `kjd_lvgl.md` §6.1,
  `kernel-drivers.md` §3.1 "SAR ADC"). prog_uvc reads no keys.
- The library pieces that are dead in prog_uvc (§1.3); the audio, LDC and file-source paths that the shipped start line does
  not reach (§9, §8, §3.6); `-c` / `-P` (§2.1).
- `uvc_config_zero` (0x1c398) only zero-initialises the UVC config struct.

## 12. Vendor function index [R]

Names are vendor strings, SDK names or descriptive (in parentheses).

| addr | name | role |
|---|---|---|
| 0x14c98 | StartVdec | VDEC dev 0 chn 0 H.264 1920×1080 (§7.5) |
| 0x14e14 / 0x14fa0 | sstar_module_bind / unbind | DISP source SCL ↔ VDEC |
| 0x150ec | (get duration) | MP4 duration in s |
| 0x15114 | (player thread) | MP4 samples → VDEC, 33 ms pacing, pause flag |
| 0x1528c | (open MP4) | libmp4v2 parse, start the player thread |
| 0x15578 / 0x155e4 / 0x15600 / 0x1561c | player start / stop / pause / resume | ids 0x1b, 0x1d |
| 0x15638 | UVC_StartCapture_photo | SCL port 1 + JPEG VENC dev 8 chn 1 (§7.3) |
| 0x159f8 | Destroy_photo_video_venc_1080p | its tear-down |
| 0x15bd0 | (save photo) | first JPEG frame → `/mnt/PHOTO/P%05d.jpg` |
| 0x15f10 / 0x15f2c / 0x15f48 | photo 720p / 1080p / 4K | ids 0x13–0x15 |
| 0x15f64 | (show JPEG) | file → JPD stream buffer (§7.6) |
| 0x160e0 | (get DISP layer size) | |
| 0x161d4 / 0x16124 | St_JpdModuleInit / Deinit | |
| 0x162d4 / 0x167a8 | kjd_photo_init / deinit | viewer pipeline JPD → SCL1 → DISP, ISP paused |
| 0x16ab4 | St_VencModuleInit | H.264 VENC for recording (§7.4) |
| 0x16c2c | ParseSPSPPSFromVENCData | |
| 0x16ea4 | KJD_video_mp4_pth | recording thread |
| 0x179cc | (set stop flag) | |
| 0x179f0 / 0x17c3c | KJD_Video_Record / KJD_Close_Record | |
| 0x17d4c | change_scl_resolution | crop by mode, output = HDMI size (§5.2) |
| 0x17fec | (read crop mode) | `/misc/ws.txt` first character |
| 0x1808c | (EDID → 4K?) | §5.1 |
| 0x18384 / 0x18aa8 | St_HdmiModuleInit / Deinit | |
| 0x18d74 / 0x18bf8 | St_DispModuleInit / Deinit | |
| 0x1912c | (HDMI event callback) | hot-plug rebuild |
| 0x1a038 / 0x1a204 | ST_UacInit / ST_UacDeinit | §9 |
| 0x1a3e8 / 0x1bc68 | ST_VideoModuleInit / ST_VideoModuleUnInit | §2.3, §2.4 |
| 0x1dc08 / 0x1e3b0 | (apply camp.txt / defaults) | §4.3 |
| 0x1e950 / 0x1ea18 | init_hdmi / deinit_hdmi | §5.2 |
| 0x1ec08 / 0x205b8 | UVC_StartCapture / UVC_StopCapture | §3.2 |
| 0x21234 | ST_UvcInit | §2.2 |
| 0x214f8 / 0x21b60 / 0x21e48 | mirror_display / tailoring_init / zoom_init | §7.1 |
| 0x242fc / 0x24a64 / 0x24edc / 0x25090 | kjd_vdisp_init / kjd_vdisp_deinit / kjd_start_vdisp / kjd_stop_vdisp | split view (§7.2) |
| 0x251f0 | (apply ws settings) | §6.4 |
| 0x258cc / 0x25944 / 0x259c0 / 0x25a38 | ws.bin write / read + apply, default_ws.bin write / read | |
| 0x25a98 | (load or create ws.bin) | |
| 0x25e50 | (HDMI ↔ USB / power switch) | §6.3 |
| 0x25f08 | (menu client thread) | §6.2 |
| 0x273fc / 0x274fc | (TCP 8888 server / accept loop) | §6.1 |
| 0x275a4 | main | §2.2 |
| 0x2a468 | _UVC_Events_Process_Control | vendor GET answers (§4.1, §4.2) |
| 0x2cc18 | _UVC_Events_Process_Data | OTA branch (§4.7) |
| 0x2fc30 / 0x3035c / 0x30720 | usb_vc_eu1_cs / usb_vc_eu1_cs_out / usb_vc_cmd_cfg | XU1 (§4.4) |
| 0x30aec / 0x30b4c / 0x30cb8 | usb_vc_eu2_cs_out / usb_vc_ct_cs_out / usb_vc_pu_cs_out | SET handlers |
| 0x31518 | (rewrite a camp.txt line) | §4.3 |
| 0x3163c … 0x31d36 | AES-128 (`aes.cpp`) | §4.5 |
