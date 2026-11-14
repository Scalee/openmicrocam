# audio — mi_ai, mi_ao, libmi_ai, libmi_ao and mhal AUDIO ("bach")
<!-- ev:audio -->

The audio stack of the SigmaStar media software on this camera, from the user libraries down to the registers. Loaded,
never used by the product (§1):

- **mi_ai** (`mi_ai.ko`, `/dev/mi_ai`, MI module id 4) — audio input. 9 capture devices (ADC0/1, Dmic, I2S RX, LineIn,
  ADC2/3, ADC0–3, Dmic and I2S RX on the second capture DMA, 4 Amic + 4 Dmic), up to 8 channels each, 16-bit at
  8/16/32/48 kHz. Enable allocates a DMA ring, configures and opens the MHAL capture device and starts a mi_sys HW timer
  at half the frame time; the device's mi_sys worker reads one period of interleaved DMA slots (data, echo reference,
  padding) per wake-up and splits it into one mi_sys output buffer per channel (§3).
- **mi_ao** (`mi_ao.ko`, `/dev/mi_ao`, MI module id 5) — audio output. 6 playback devices (DAC0/1, I2S TX, DAC0, DAC1,
  HDMI, HDMI + line out), 2 channels, 8–48 kHz (HDMI 32/48 kHz). No mi_sys data path: `SendFrame` copies the already
  interleaved frame into a staging buffer, applies software mute and writes it into the MHAL DMA ring (§4).
- **libmi_ai.so / libmi_ao.so** — ioctl wrappers plus two pieces of logic: `MI_AI_GetFrame` (select on the mi_sys port
  fd, hand the buffer out as data and AEC-reference frames, mute by zeroing) and `MI_AO_SendFrame` (channel 0 only,
  per-channel 0xC800-byte software queues, stereo interleave, wait for room in 20 ms steps). Resampling, VQE, encoders and
  the AED / SSL / BF detectors are stubs (§2).
- **mhal AUDIO ("bach")** — the codec / I2S / DMA HAL inside `mhal.ko`: four DMA engines, RIU banks 0x1502–0x1504 and the
  analog front end ("ATOP") 0x1034, IRQ 0x4A; a newer revision of the mercury6 HAL of the OpenIPC u-boot tree (§5).

Other documents: the user-side ioctl wrapper, fd handling, return convention, MI error-code format and module ids —
`mi-user-abi.md` §2 "The device fd", §3 "The ioctl call", §4 "Return values and error codes"; the mi_common device frame
that creates `/dev/mi_*`, the per-process hooks and the per-module procfs files — `mi_sys-2-interfaces.md` §1 "mi_common.ko
— the device frame of every MI module"; the registration contract, pass callbacks, frame flow and worker threads —
`mi_sys-1-engine.md` §2 "Registration contract", §4 "Frame flow", §6 "Worker threads and the MAIN loop"; the mi_sys HW
timer, private heaps and `MI_SYSCFG_GetMmapInfo` — `mi_sys-2-interfaces.md` §8, §7.5, §2.6; mhal.ko as a module, its block table and DT
reads — `mhal-core.md` §1, §2.1, §7; RIU addressing, the clock tree (`bach`), the IRQ and DT node list, padmux and board wiring (the
amplifier pad) — `kernel-platform.md` §3.1, §5.1, §6.2, §6.3, §14; the `mmap.ini` items — `config_tool.md` §2.5; the load
lines in `demo.sh` and the inventory rows — `firmware-image.md` §2, §3.3, §3.5; prog_uvc's audio path (options `-d`, `-D`, `-S`, `-f`,
`st_uac.c`, the statically linked tinyalsa and the static copies of libmi_ai / libmi_ao) — `prog_uvc.md` §9; UAC in the USB
gadget — `usb-gadget.md` §1, §4; the audio algorithm libraries (libAEC, libAPC, libSRC, libSSL, libAED, libBF) —
`audio-algorithms.md`.

Tags: **[R]** read from the binary, a decompile, disassembly or a file; **[S]** stated by a source (the SDK headers, the
mercury6 source tree and its `hal_audio_reg.h`); **[I]** inference. A heading tagged [R] means every statement under it
is [R] unless marked otherwise. MI struct and field names (`MI_AUDIO_Attr_t`, `u32PtNumPerFrm` …) are not in the
binaries; they are those of the public SDK header set (local copy, which
matches the layouts read) [S]. Function names of mhal AUDIO are the binary's symbols [R]; register and field names are
the mercury6 `hal_audio_reg.h`'s [S].

Binaries (dump of 2026-10-16) [R]: `/config/modules/4.9.227/mi_ai.ko` (143 KB file, 125 functions,
53 KB code), `…/modules/4.9.227/mi_ao.ko` (126 KB file, 131 functions, 46 KB code), `/config/lib/libmi_ai.so`,
`…/lib/libmi_ao.so`, `…/modules/4.9.227/mhal.ko` block AUDIO (178 functions, 21.8 KB). Build string of mi_ai and mi_ao:
"Sigmastar Module mi_ai version: project_commit.7251436 sdk_commit.80327e1 build_time.20220721223317" (mi_ao the same).

## 1. The stack on this camera [R]

- `customer/demo.sh` loads `mhal.ko` (line 28), `mi_ao.ko` (line 43) and `mi_ai.ko` (line 49), so `/dev/mi_ai` and
  `/dev/mi_ao` exist at run time.
- **Nothing opens them.** The only user-space caller is prog_uvc, which links its own static copies of the libmi_ai /
  libmi_ao functions it calls (same build as the .so) and reaches them only through its audio backend, run when `-d`,
  `-D` or `-f` is given; `demo.sh` passes none of them (`prog_uvc.md` §9.1). No binary in the dump links `libmi_ai.so` /
  `libmi_ao.so` dynamically.
- Neither module touches hardware at load. Hardware is touched only when a process opens the device: the MI_DEVICE
  `process_init` hook runs `_MI_AI_Init` / `_MI_AO_Init`, which call `MHAL_AUDIO_Init` (§3.1, §4.1). The AUDIO block of
  mhal.ko has no initcall of its own; `MHAL_AUDIO_Init` is reached only through the export (§5.1). So with the shipped
  start line no AUDIO register is written, the DMA engines stay idle and the amplifier pads are not driven.
- Whether a microphone, a line output or the amplifier pads are wired on this board is not known from static data
  (§5.6). Audio over UAC would also need UAC enabled in the `g_webcam` USB gadget (`uac_function_enable`, `usb-gadget.md` §1).

## 2. User libraries libmi_ai.so / libmi_ao.so and the ioctl ABI

### 2.1 Common mechanics [R]
- Both libraries follow the common MI wrapper (`mi-user-abi.md` §2, §3): one fd per process (`/dev/mi_ai`, `/dev/mi_ao`),
  opened lazily by an internal `_MI_WRAPPER_DEVICE_Open` (libmi_ai 0x11620; source name from the assert string
  `ai_api.c:0x33`) under a CamOs semaphore, re-opened after `fork` (pid compared), `FD_CLOEXEC` set; ioctl argument
  `struct {u32 len; u32 pad; u64 ptr}`; the ioctl result is the MI error code. `MI_AI_InitDev` / `MI_AO_InitDev` only
  open the fd (0, or 0x1f on failure); `MI_AI_DeInitDev` / `MI_AO_DeInitDev` close it.
- The device count is asked once from the kernel — ioctl `GetChipDevNum` (AI nr 0x0E, AO nr 0x1B, payload 1 byte) — and
  cached (libmi_ai `.bss+0x369`). The kernel answers **9 AI devices** and **6 AO devices**, fixed constants
  (`MI_AI_IMPL_GetChipDevNum` stores 9, `MI_AO_IMPL_GetChipDevNum` 6).
- Error codes: `0xA004xxxx` for AI (module id 4), `0xA005xxxx` for AO (module id 5). Low parts seen and their trigger:

| low part | trigger |
|---|---|
| `2001` | dev id out of range |
| `2002` | chn id out of range / no channels |
| `2003` | attribute or parameter rejected |
| `2006` | NULL pointer |
| `2007` | attr not set before Enable |
| `2008` | SetPubAttr while enabled (AI); every "no longer supported" stub |
| `2009` | AO: SetPubAttr while enabled; Pause / Resume when already in that state |
| `200C` | allocation failed |
| `200D` | GetFrame timeout (select ≤ 0); AO: no room / frame dropped / paused |
| `2010` | channel has no MI_SYS fd |
| `2012` | mi_sys registration / heap failure; Disable or ClrPubAttr while still in use; MHAL failure in a gain call |
| `2017` | device or channel not enabled |

  The decompile shows them as negative ints (`-0x5ffbdfff` = `0xA0042001`). Enum names are not in the binary.

### 2.2 Structures [R layouts; S names]
Layouts read from the copy loops and offsets; names from the SDK header set.
- `MI_AUDIO_Attr_t`, 0x34 bytes: +0x00 `eSamplerate` (Hz value), +0x04 `eBitwidth` (0 = 16, 1 = 24, 2 = 32 bit), +0x08
  `eWorkmode` (0 I2S master, 1 I2S slave, 2 TDM master, 3 TDM slave), +0x0C `eSoundmode` (0 mono, 1 stereo, 2 queue),
  +0x10 `u32FrmNum`, +0x14 `u32PtNumPerFrm`, +0x18 `u32CodecChnCnt`, +0x1C `u32ChnCnt`, +0x20 I2S config: `eFmt`
  (0 I2S-MSB, 1 left-justify), +0x24 `eMclk` (0 off, 1 12.288, 2 16.384, 3 18.432, 4 24.576, 5 24, 6 48 MHz), +0x28
  `bSyncClock` (u8), +0x2C `u32TdmSlots`, +0x30 `eI2sBitWidth`.
- `MI_AUDIO_Frame_t`, 0x120 bytes: +0x00 `eBitwidth`, +0x04 `eSoundmode`, +0x08 `apVirAddr[16]`, +0x48 `u64TimeStamp`,
  +0x50 `u32Seq`, +0x54 `u32Len[16]`, +0x94 `au32PoolId[2]`, +0x9C `apSrcPcmVirAddr[16]`, +0xDC `u32SrcPcmLen[16]`.
- `MI_AUDIO_AecFrame_t`, 0x124 bytes: `MI_AUDIO_Frame_t stRefFrame` + u8 `bValid` at +0x120.
- `MI_AI_ChnParam_t` / `MI_AO_ChnParam_t`: 12 bytes, copied as three words (ioctl payload +0x08..+0x10). AI:
  {u8 `bEnableGainSet`, s16 front gain, s16 rear gain} (§3.7); AO: {u8 `bEnableGainSet`, s16 gain, u32 fading} (§4.5).
- `MI_AO_ChnState_t`: {u32 total, u32 free, u32 busy}, bytes (§4.4).

### 2.3 `/dev/mi_ai` ioctls (type 'i') [R]
<!-- ev:audio.2.3 -->
Every kernel handler takes the module mutex and calls the IMPL function named (kernel table).

| nr | dir, size | payload | kernel IMPL | libmi_ai caller |
|---|---|---|---|---|
| 0x00 | W 0x38 | s32 dev, `MI_AUDIO_Attr_t` | SetPubAttr | MI_AI_SetPubAttr |
| 0x01 | WR 0x38 | dev → attr | GetPubAttr | MI_AI_GetPubAttr |
| 0x02 | W 4 | dev | Enable | MI_AI_Enable |
| 0x03 | W 4 | dev | Disable | MI_AI_Disable |
| 0x04 | W 8 | dev, chn | EnableChn | MI_AI_EnableChn |
| 0x05 | W 8 | dev, chn | DisableChn | MI_AI_DisableChn |
| 0x06 | W 0x14 | dev, chn, ChnParam (12) | SetChnParam | MI_AI_SetChnParam |
| 0x07 | WR 0x14 | dev, chn → ChnParam | GetChnParam | MI_AI_GetChnParam |
| 0x08 | W 4 | dev | ClrPubAttr | MI_AI_ClrPubAttr |
| 0x09 | W 0xC | dev, chn, s32 volume | SetVqeVolume | MI_AI_SetVqeVolume |
| 0x0A | WR 0xC | dev, chn → s32 volume | GetVqeVolume | MI_AI_GetVqeVolume |
| 0x0B | — 0 | — | `_MI_AI_Init` (module init, §3.1) | MI_AI_Init |
| 0x0C | — 0 | — | `_MI_AI_DeInit` | MI_AI_DeInit |
| 0x0D | W 0xC | dev, chn, ref chn | SetExtAecChn | MI_AI_SetExtAecChn |
| 0x0E | WR 1 | → u8 device count (9) | GetChipDevNum | internal (dev-count cache) |
| 0x0F | W 0xC | dev, chn, u8 mute | MuteChn | MI_AI_SetMute |
| 0x10 | WR 0xC | dev, chn → u8 mute | GetChnMuteStatus | MI_AI_GetMute, MI_AI_GetFrame |
| 0x11 | WR 0x10 | dev, chn → u32 data chn count, u32 ref chn count | GetChnDataInfo | MI_AI_GetFrame |

### 2.4 libmi_ai.so — per function [R]
State: an array of 9 × 0x228-byte device records allocated on first use (`.bss+4`): +0x00 u8 enabled, +0x01 u8 attr
cached, +0x04 copy of `MI_AUDIO_Attr_t` (so +0x10 = sound mode, +0x20 = channel count), then 8 channel records of 0x38
from +0x38: +0x00 u8 enabled, +0x04 MI_SYS buffer handle of the frame handed out, +0x0C channel rwsem, +0x30 last seen
sequence, +0x34 MI_SYS output-port fd (initialised to −1); device rwsem at +0x200. Plus per (dev, chn) dump `FILE*`
tables (`.bss+0x248` data, `.bss+8` echo).
- `MI_AI_SetPubAttr(dev, attr)`: range check, ioctl 0x00, on success caches the attr (+0x04) and sets +0x01.
  `MI_AI_GetPubAttr`: ioctl 0x01, copies 0x34 bytes back.
- `MI_AI_Enable(dev)`: ioctl 0x02; on success sets +0x00 and clears every channel's "last sequence".
  `MI_AI_Disable`: ioctl 0x03; on success clears +0x00 and every channel record (enabled, handle, sequence; the fd field
  is set to −1 **without** `MI_SYS_CloseFd`, so a fd still open from EnableChn leaks if DisableChn was skipped).
- `MI_AI_EnableChn(dev, chn)`: ioctl 0x04; on success, if the channel has no fd yet, `MI_SYS_GetFd({module 4, dev, chn,
  port = chn})` and keeps it; opens the dump files (below); marks the channel enabled. `MI_AI_DisableChn`: ioctl 0x05; on
  success `MI_SYS_CloseFd` of the channel fd (field → −1), closes the dump files, clears the record.
- `MI_AI_DupChn(dev, chn)`: if no attr is cached, fetches it with `MI_AI_GetPubAttr`; if the device is not enabled calls
  `MI_AI_Enable`; then `MI_AI_EnableChn` — lets a second process attach to a device another process configured.
- **`MI_AI_GetFrame(dev, chn, *frame, *aecFrame, s32 timeoutMs)`** (0x12ee8): checks dev, chn, `frame != NULL`,
  `timeoutMs >= -1`; device and channel must be enabled (`0xA0042017`) and the channel fd valid (`0xA0042010`). Then:
  1. `select()` on the channel fd with the timeout (−1 = block, else `timeoutMs` ms); 0 ready → `0xA004200D`.
  2. `MI_SYS_ChnOutputPortGetBuf({4, dev, chn, chn}, &bufInfo, &handle)`; the handle is stored in the channel record.
  3. ioctl 0x10 → mute flag; ioctl 0x11 → `dataChn`, `refChn`.
  4. If the buffer sequence differs from the last one seen, the new value is stored (a log line notes skipped frames).
  5. Fills `frame`: `eBitwidth` and `eSoundmode` from the cached attr, `u64TimeStamp` = buffer PTS, `u32Seq` = buffer
     sequence, `perChn = bufSize / (dataChn + refChn)` bytes. For mono / stereo: `apVirAddr[0] = apSrcPcmVirAddr[0] =`
     buffer virtual address, `u32Len[0] = u32SrcPcmLen[0] = perChn × dataChn`; if muted both are zero-filled. Queue mode
     (sound mode 2) describes the one buffer the same way (all channels inside it, `dataChn = chnCnt`).
  6. Dump: if a data dump file is open for (dev, chn) the frame is appended to it; reference channels go to the echo
     files.
  7. If `aecFrame != NULL`: `bValid = (refChn != 0)`; `stRefFrame` gets the same header; for mono / queue reference
     channel *i* is `apVirAddr[i] = base + perChn × (dataChn + i)`, `u32Len[i] = perChn`; for stereo the reference pair
     is described as one block at `base + perChn × dataChn` of `perChn × refChn` bytes split in halves.
  The buffer stays held until `MI_AI_ReleaseFrame`. The buffer layout this reads is the enqueue's (§3.6).
- `MI_AI_ReleaseFrame(dev, chn)`: `MI_SYS_ChnOutputPortPutBuf(handle)` for the held buffer (`0xA0042017` if not enabled).
- `MI_AI_SetChnParam` / `GetChnParam`, `SetVqeVolume` / `GetVqeVolume`, `SetExtAecChn`, `ClrPubAttr`, `SetMute` /
  `GetMute`, `Init`, `DeInit`: pack and issue the ioctl of the table; no other logic.
- **Stubs** — print "This interface is no longer supported in this version." (only when the `ai` debug level is set) and
  return `0xA0042008`: `MI_AI_EnableReSmp`, `DisableReSmp`, `SetVqeAttr`, `GetVqeAttr`, `EnableVqe`, `DisableVqe`,
  `SetAencAttr`, `GetAencAttr`, `EnableAenc`, `DisableAenc`, `SaveFile`, `SetAedAttr`, `GetAedAttr`, `EnableAed`,
  `DisableAed`, `GetAedResult`, `SetSslInitAttr`, `GetSslInitAttr`, `SetSslConfigAttr`, `GetSslConfigAttr`, `EnableSsl`,
  `DisableSsl`, `GetSslDoa`, `SetBfInitAttr`, `GetBfInitAttr`, `SetBfConfigAttr`, `GetBfConfigAttr`, `EnableBf`,
  `DisableBf`, `SetBfAngle`. This build of libmi_ai has **no link to the audio algorithm libraries**
  (`audio-algorithms.md` §1.3): no import of any `*_LINUX.so` symbol, so resampling, VQE, encoding and the AED / SSL / BF
  detectors are absent from the MI path. (Of those libraries, libBF carries SILK's VAD (`psSilk_VAD`) and links
  Ne10, `audio-algorithms.md` §8.10, §2.6.)
- Dump to files (user side, 0x11ad8): enabled per channel at `EnableChn` by the environment — `MI_AI_DUMP_PCM_ENABLE=1`
  (data) and / or `MI_AI_DUMP_ECHO_ENABLE=1` (reference) plus `MI_AI_DUMP_PATH`; files `%s/MI_AI_Dev%dChn%d_%dK_%s.pcm`
  (rate in kHz, "Mono" / "Stereo") and `%s/MI_AI_Dev%dChn%dSrc%d_%dK_Echo_%s.pcm` (two per channel), mode "w+"; closed
  at `DisableChn` (0x11840).

### 2.5 `/dev/mi_ao` ioctls (type 'i') [R]
The kernel's table has 29 entries. Sizes are those libmi_ao encodes; rows marked "—"
have no libmi_ao caller, so only the words the kernel wrapper reads are known (the MI_DEVICE frame copies whatever length
user space passes, `mi-user-abi.md` §3).

| nr | dir, size | payload (words) | kernel IMPL | libmi_ao caller |
|---|---|---|---|---|
| 0x00 | W 0x38 | dev, `MI_AUDIO_Attr_t` | SetPubAttr | MI_AO_SetPubAttr |
| 0x01 | WR 0x38 | dev → attr | GetPubAttr | MI_AO_GetPubAttr |
| 0x02 | W 4 | dev | Enable | MI_AO_Enable |
| 0x03 | W 4 | dev | Disable | MI_AO_Disable |
| 0x04 | W 8 | dev, chn | EnableChn | MI_AO_EnableChn |
| 0x05 | W 8 | dev, chn | DisableChn | MI_AO_DisableChn (also EnableChn's error path) |
| 0x06 | W 0x130 | dev, chn, `MI_AUDIO_Frame_t` (0x120), s32 timeout | SendFrame | `_MI_AO_SendToChnInputPort` (inside MI_AO_SendFrame) |
| 0x07 | W 4 | dev | PauseDev | MI_AO_PauseChn |
| 0x08 | W 4 | dev | ResumeDev | MI_AO_ResumeChn |
| 0x09 | W 8 | dev, chn | ClearChnBuf | MI_AO_ClearChnBuf |
| 0x0A | WR 0x14 | dev, chn → `MI_AO_ChnState_t` {u32 total, u32 free, u32 busy} | QueryChnStat | MI_AO_QueryChnStat |
| 0x0B | W 0x10 | dev, chn, s32 volume dB, u32 fading | SetVolume | MI_AO_SetVolume |
| 0x0C | WR 0xC | dev, chn → s32 volume | GetVolume | MI_AO_GetVolume |
| 0x0D | W 0x14 | dev, chn, s32 left dB, s32 right dB, u32 fading | SetLRVolume | MI_AO_SetLRVolume |
| 0x0E | WR 0x10 | dev, chn → s32 left, s32 right | GetLRVolume | MI_AO_GetLRVolume |
| 0x0F | W 0xC | dev, chn, u8 mute | SetMute | MI_AO_SetMute |
| 0x10 | WR 0xC | dev, chn → u8 mute | GetMute | MI_AO_GetMute |
| 0x11 | W 4 | dev | ClrPubAttr | MI_AO_ClrPubAttr |
| 0x12 | W 0x20 | dev, chn, then 6 words (queue size, queue used, u64 bytes queued so far, u64 time since Enable) | UpdateQueueStatus | `_MI_AO_SendToChnInputPort` |
| 0x13 | — | dev, chn | GetAoStartPts | none (MI_AO_GetAoStartPts is a stub) |
| 0x14 | — | dev, chn, word 2 | ReadAoData | none (stub) |
| 0x15 | — | dev, (word 1), words 2–3 (u64 offset), word 4 (whence) | LseekAoData | none (stub) |
| 0x16 | — 0 | — | `_MI_AO_Init` | MI_AO_Init |
| 0x17 | — 0 | — | `_MI_AO_DeInit` | MI_AO_DeInit |
| 0x18 | W 0x14 | dev, chn, ChnParam (12) | SetChnParam | MI_AO_SetChnParam |
| 0x19 | WR 0x14 | dev, chn → ChnParam | GetChnParam | MI_AO_GetChnParam |
| 0x1A | W 8 | dev, s32 gain | SetSrcGain | MI_AO_SetSrcGain |
| 0x1B | WR 1 | → u8 device count | GetChipDevNum | internal (dev-count cache) |
| 0x1C | — | dev, u8 flag (word 1), u64 address (words 2–3) | SetDevMmaBufAddr | none |

### 2.6 libmi_ao.so — per function [R]
State: per device a 0x110-byte record (`.bss+4`, count from ioctl 0x1B): +0x00 u8 enabled, +0x01 u8 attr cached, +0x04
attr copy (so +0x08 bit width, +0x10 sound mode, +0x20 channel count); per channel *c* (2 records, stride 0x50, from
+0x38): +0x38 u8 enabled, +0x3C and +0x40 two 0xC800-byte scratch buffers, **+0x44 software queue** {+0x44 write pos,
+0x48 read pos, +0x4C size = 0xC800, +0x50 bytes queued, +0x54 base (0xC800-byte buffer)}, +0x58 channel rwsem, +0x80
u64 bytes sent; device rwsem +0xD8, +0x100 Enable time (`CamOsGetTimeOfDay` seconds), +0x108 device scratch buffer
(0xC800), +0x10C u8 paused flag (set by `MI_AO_PauseChn`, cleared by `MI_AO_ResumeChn`; each refuses a second call).
- `MI_AO_SetPubAttr` / `GetPubAttr`: ioctl 0x00 / 0x01, cached as in libmi_ai.
- `MI_AO_Enable(dev)`: needs the attr cached (`0xA0052007`); allocates the 0xC800-byte device scratch buffer, ioctl 0x02,
  zeroes the channels' byte counters and records the Enable time. `MI_AO_Disable`: frees the buffers, ioctl 0x03; returns
  `0xA0052012` ("Dev%d Chn%d is enable.") while a channel is still enabled.
- `MI_AO_EnableChn(dev, chn)`: device must be enabled (`0xA0052017`); ioctl 0x04; allocates the channel's queue buffer
  (0xC800) and two scratch buffers, initialises the queue {0, 0, 0xC800, 0}, opens the dump file; on an allocation
  failure frees what it got, issues ioctl 0x05 and returns `0xA005200C`. `MI_AO_DisableChn`: ioctl 0x05 and frees the
  three buffers. `MI_AO_DupChn`: as `MI_AI_DupChn`.
- **`MI_AO_SendFrame(dev, chn, *frame, s32 timeoutMs)`** (0x1461c) — the AO "ring buffer" is this per-channel
  user-space queue in front of the MHAL DMA ring:
  1. Checks: dev / chn range, `frame != NULL`, `timeoutMs >= -1`, **chn must be 0** (`0xA0052003`, "only support Chn0 to
     send frame": all channels' data arrive in one call on channel 0); device and channel 0 enabled; for every channel
     *i* < chnCnt, `0 < frame.u32Len[i] < 0xC800`.
  2. Bytes needed in the kernel buffer: `u32Len[0]` (+ the bytes already queued in the user queue, capped at the queue
     size); for stereo sound mode with 2 channels the larger of the two channel amounts × 2 (the two mono streams are
     interleaved into one stereo stream).
  3. Wait for room: loop `MI_AO_QueryChnStat(dev, 0)` until `free > needed + 0x80` (or `busy` small enough); sleep 20 ms
     between polls; `timeoutMs = -1` waits forever, `0` does not wait, otherwise `timeoutMs / 20` tries (at least 1);
     returns `0xA005200D` on timeout or while the device is paused (+0x10C).
  4. Appends each channel's data (`apVirAddr[i]`, `u32Len[i]`) to channel *i*'s software queue (wrapping copy; a frame
     that does not fit is dropped and the call ends with `0xA005200D`).
  5. `_MI_AO_SendToChnInputPort(dev, queue)` (0x13eac) drains the queues in chunks of at most 0x6400 bytes: mono (1
     channel): copy from the channel 0 queue; stereo with 2 channels: take equal amounts from both queues and interleave
     16-bit samples L / R into the device scratch buffer; then ioctl 0x06 with a `MI_AUDIO_Frame_t` whose `apVirAddr[0]`
     = scratch buffer and `u32Len[0]` = chunk × channels; after each chunk, per channel, ioctl 0x12 `UpdateQueueStatus`
     {queue size, queue used, total bytes sent, time since Enable}. The data dump file (if open) receives each chunk.
- `MI_AO_QueryChnStat(dev, chn, *state)`: ioctl 0x0A, copies three words back.
- `MI_AO_PauseChn(dev, chn)` / `MI_AO_ResumeChn`: ioctl 0x07 / 0x08 with the **device** id only (pause is per device).
- `MI_AO_ClearChnBuf(dev, chn)`: ioctl 0x09 and resets the user queue (positions and count to 0).
- `MI_AO_SetVolume` / `GetVolume` / `SetLRVolume` / `GetLRVolume` / `SetMute` / `GetMute` / `SetChnParam` /
  `GetChnParam` / `SetSrcGain` / `ClrPubAttr` / `Init` / `DeInit`: pack and issue the ioctl of the table.
- **Stubs** returning `0xA0052008` with "This interface is no longer supported in this version.": `MI_AO_GetAoStartPts`,
  `MI_AO_ReadAoData`, `MI_AO_LseekAoData`, `MI_AO_EnableReSmp`, `DisableReSmp`, `SetVqeAttr`, `GetVqeAttr`, `EnableVqe`,
  `DisableVqe`, `SetAdecAttr`, `GetAdecAttr`, `EnableAdec`, `DisableAdec` (the first three have kernel ioctls 0x13–0x15
  that nothing calls, and those are empty too, §4.4).
- Dump: `MI_AO_DUMP_PCM_ENABLE=1` + `MI_AO_DUMP_PATH` → `%s/MI_AO_Dev%dChn%d_%dK_%s.pcm`, opened at EnableChn (the
  missing-path message wrongly names `MI_AI_DUMP_PATH`).

## 3. mi_ai.ko — audio input

### 3.1 Load, parameters, life cycle [R]
- `init_module`: `/proc/mi_modules/mi_ai/` with `debug_level`, `debug_file`, `debug_func`, `module_version_file` (the
  build string above); allocates the global device table (`_MI_AI_IMPL_InitGlobalData`: 0x1B00 bytes = 9 × 0x300);
  `MI_DEVICE_Register` → `/dev/mi_ai`. Nothing touches hardware at load.
- Parameters: `debug_level` (uint, default 2), `debug_file`, `debug_func` (strings), **`ThreadPriority`** (int, default
  98: priority of the mi_sys output thread of each AI device), **`DmaBufferSize`** (int, default 0 = automatic, §3.5).
- `process_init` / `process_exit` of the MI_DEVICE (every process that opens `/dev/mi_ai`) call `_MI_AI_Init` /
  `_MI_AI_DeInit`; ioctls 0x0B / 0x0C call the same. Both are reference-counted with an atomic: only the first Init and
  the last DeInit act.
- **`_MI_AI_Init`** (first user): for each of the 9 devices clear the 0x300-byte record, set its id, init its three
  mutexes (+0x27C capture, +0x2A4 DMA buffer, +0x2CC temp buffer), set the 8 channel ids; clear the dump tables; then
  `MHAL_AUDIO_Init(NULL)` and default gains: for channels 0 and 1 `SetAdcGain(dev 0, 2, ch)`, `SetGainIn(dev 0, 0, ch)`,
  `SetAdcGain(dev 4, 2, ch)`, `SetGainIn(dev 4, 0, ch)` (ADC gain index 2 = 0 dB, digital 0 dB), then `SetDmicGain(dev 1,
  0, ch)` for ch 0–3. A non-zero MHAL return aborts Init with that code.
- **`_MI_AI_DeInit`** (last user): for each device disable every enabled channel (`MI_AI_IMPL_DisableChn`), disable the
  device if enabled, destroy the mutexes, clear the record; then `MHAL_AUDIO_DeInit()`.
- `cleanup_module`: removes the proc files and unregisters the MI device.

### 3.2 Devices [R]
Nine AI devices; the MI id is passed unchanged to MHAL as the MHAL AI device id (`MHAL_AUDIO_*In(dev & 0xffff, …)`).
Names from the proc dump string table; "kind" is the class the checks below use. The hardware side of each device (DMA
engine, inputs, slot binding) is in §5.4.

| dev | proc name | kind | channels allowed (`u32ChnCnt`) mono/queue · stereo | sample rates | notes |
|---|---|---|---|---|---|
| 0 | ADC0/1 | Amic | 1, 2 · 1 | 8/16/32/48 k | analog mic, codec ADC 0/1 |
| 1 | Dmic | Dmic | 1, 2 · 1 | 8/16/32/48 k | digital mic |
| 2 | I2sRx | I2S | normal I2S: 2 · 1; TDM: 4 or 8 · 2 or 4 | 8/16/32/48 k | I2S RX; attr I2S fields used |
| 3 | LineIn | Amic | 1, 2 · 1 | 8/16/32/48 k | line input (ADC path, own gain range) |
| 4 | ADC2/3 | Amic | 1, 2 · 1 | 8/16/32/48 k | second ADC pair |
| 5 | ADC0/1/2/3 | Amic 4-ch | 1, 2, 4 · 1, 2 | 8/16/32/48 k | all four ADCs |
| 6 | Dmic(DMA2) | Dmic | 1, 2 · 1 | 8/16/32/48 k | Dmic on the second capture DMA |
| 7 | I2sRx(DMA2) | I2S | as dev 2 | 8/16/32/48 k | I2S RX on the second capture DMA |
| 8 | Amic+Dmic | Amic+Dmic | 8 · 4 | 8/16/32/48 k | 4 analog + 4 digital channels |

Up to **8 channels** per device (`MI_AI_MAX_CHN_NUM`); each MI channel *c* is mi_sys channel *c* with one output port
(port = chn). The channel-count rules are `_MI_AI_CheckChannelCount` (messages "Amic/LineIn Mono/Queue mode only supports
1 or 2 channel(s)", "Mono/Queue mode: Tdm I2S only supports 4 or 8 channels", "4ChnAmic+4ChnDmic … 8 channel(s)" …).

### 3.3 Device record (0x300 bytes, `_LANCHOR2 + dev × 0x300`) [R offsets; I names]
| off | field |
|---|---|
| +0x000 | u8 enabled |
| +0x001 | u8 attr set |
| +0x004 | dev id |
| +0x008 | `MI_AUDIO_Attr_t` copy (0x34): +0x08 rate, +0x0C bit width, +0x10 work mode, +0x14 sound mode, +0x18 frm num, +0x1C pt num per frame, +0x20 codec chn cnt, +0x24 chn cnt, +0x28 I2S fmt, +0x2C mclk, +0x30 sync clock, +0x34 TDM slots, +0x38 I2S bit width |
| +0x040 + 0x40·c | channel *c* record: +0x00 u8 enabled, +0x04 chn id, +0x08 u8 gain set, +0x0A s16 front gain, +0x0C s16 rear gain, +0x14 u8 has external AEC ref, +0x18 ref chn, +0x1C u8 is a ref chn for another chn, +0x20 / +0x24 u64 last enqueue PTS, +0x28 u32 frames read in this period, +0x30 u64 frames delivered, +0x38 u8 mute |
| +0x240 | u8 PCM configured |
| +0x248 | u64 DMA buffer physical address |
| +0x250 | DMA buffer kernel virtual address |
| +0x254 | DMA buffer size |
| +0x258 | temp buffer (one period, interleaved) |
| +0x25C | temp buffer size |
| +0x260 | mi_sys device handle |
| +0x264 | round-robin channel cursor of the output thread |
| +0x268 | u8 capture started (first period PTS taken) |
| +0x270 | u64 PTS of the period in the temp buffer |
| +0x278 | u8 temp buffer holds a fresh period |
| +0x279 | u8 PCM in started |
| +0x27C / +0x2A4 / +0x2CC | mutexes: capture / DMA buffer / temp buffer |
| +0x2F4 | u8 PCM opened |
| +0x2F8 | output buffer size one channel needs for the current period (`perSlotBytes × (dataChn + refChn)`) |
| +0x2FC | mi_sys HW timer id |

### 3.4 Attribute check (`MI_AI_IMPL_SetPubAttr`, 0x15b98) [R]
Refused while enabled (`0xA0042008`). Rate must be 8000, 16000, 32000 or 48000; bit width must be 16 (`eBitwidth` 0);
`u32PtNumPerFrm` must be non-zero; Dmic devices (1, 6, 8) pass `_MI_AI_DmicAttrCheck` (always true in this build, the
"Dmic not support 48k" message is unreachable); I2S devices (2, 7) pass `_MI_AI_I2sRxAttrCheck`: work mode 0–3, rate
8/16/32/48 k, in TDM modes TDM slots must be 4 or 8, I2S bit width 16 or 32 ("This chip only supports 16/32 I2s bit
width"); then the channel-count table of §3.2. On success the attr is stored and +0x01 set. `GetPubAttr` copies it back;
`ClrPubAttr` (only while disabled, else `0xA0042012`) clears it.

### 3.5 Enable, Disable, channels [R]
**`MI_AI_IMPL_Enable(dev)`** (0x164ec) — attr must be set (`0xA0042007`); a second Enable returns 0. Steps:
1. Temp buffer (one period, all DMA slots interleaved) `CamOsMemAlloc(ptNumPerFrm × 16)` for 16-bit (× 32 for 24-bit;
   0 otherwise), zeroed (+0x258 / +0x25C).
2. `_MI_AI_CreateDevice` (0x12cec): `mi_sys_RegisterDev` with info {module 4, dev, 0 input ports, **1 output port, 8
   channels, 1 pass**, priority `ThreadPriority`}, pass callbacks EnqueueOutputTask = `_MI_AI_EnqueueAsyncOutputTask`,
   BarrierOutputTask (returns 0), CheckOutputTaskStatus and DequeueOutputTask (both return 1 = done at once),
   OnPollingAsyncOutputTaskConfig = `_MI_AI_OnPollingAsyncOutputTaskConfig`; the five proc dump callbacks; proc commands
   `set_ai_volume` (3 args), `dump` (2), `dump_mhal_chn` (2) (§3.8). Handle → +0x260.
3. DMA buffer: size = `DmaBufferSize` if non-zero and a multiple of 4 KiB, else from `(rate × bytesPerSample + 1) × 8`
   rounded up to 64 KiB, 128 KiB or 256 KiB (≥ 0x40000 → 256 KiB). If the mmap table has `E_MMAP_ID_AI`
   (`MI_SYSCFG_GetMmapInfo`) and it is large enough, that region is used; otherwise `mi_sys_PrivateDevChnHeapAlloc(h,
   chn 0, "ai-Dev%d-dma", size)`. Mapped with `mi_sys_Vmap(phys, size, uncached)` and zeroed (+0x248 / +0x250 /
   +0x254). This camera's `mmap.ini` has no `E_MMAP_ID_AI` entry (`config_tool.md` §2.6, `mi_sys-2-interfaces.md` §2.6) [I], so the
   private-heap path would be taken.
4. Under the capture mutex (rate re-checked; bit width must be 16, guaranteed by SetPubAttr):
   `MHAL_AUDIO_ConfigPcmIn(dev, PcmCfg)` with {rate, width 0 (16-bit), u16 channels = chnCnt (mono / queue) or 2 × chnCnt
   (stereo), interleaved 1, DMA virt, DMA phys (u64), buffer size, **period size = channels × 2 × ptNumPerFrm bytes**,
   start threshold 0}; for I2S devices (2, 7) also `MHAL_AUDIO_ConfigI2sIn(dev, I2sCfg)` {mode = attr work mode, width =
   1 if attr I2S bit width is 32 else 0, fmt, rate, mclk, 4-wire = (bSyncClock == 1), channels = max(PCM channels, 2) for
   normal I2S, TDM slots in TDM modes}; then `MHAL_AUDIO_OpenPcmIn(dev)`; +0x2F4 set.
5. HW timer: period = `ptNumPerFrm × 10⁹ / rate / chnCnt` ns for mono / stereo (`× 10⁹ / rate` for queue), divided by
   2 000 000 (→ half a frame time in ms), at least 1; `mi_sys_RegisterHwTimer(h, mi_ai_timer_func, rec, period, &id)`,
   `mi_sys_EnableTimer`. The timer callback only calls `mi_sys_NotifyWakeupISREvent(h)` — it wakes the device's mi_sys
   worker thread.
6. Clears the cursor, "capture started", every channel's frame counters and both dump flags; sets enabled.

Failures unwind in reverse: `ClosePcmIn` (and `StopPcmIn` when started), free / unmap the DMA buffer, unregister the
mi_sys device, free the temp buffer.

**`MI_AI_IMPL_Disable(dev)`** — refuses (`0xA0042012`) while any channel is enabled; `mi_sys_DisableTimer` +
`UnRegisterHwTimer`; `StopPcmIn` (if started) and `ClosePcmIn`; unmap and free the DMA buffer (unless it came from the
mmap region); free the temp buffer; `mi_sys_UnRegisterDev`; close the `dump_mhal_chn` files; clear enabled, started, PTS
and the channels' counters.

**Channels** — `EnableChn(dev, c)`: device enabled (`0xA0042017`), c < chnCnt, and in queue sound mode only channel 0
(`0xA0042003`); `mi_sys_EnableChannel(h, c)` + `mi_sys_EnableOutputPort(h, c, 0)`; record: enabled, id, counters 0.
`DisableChn`: `mi_sys_DisableChannel` + `DisableOutputPort`, clear enabled and counter.

### 3.6 Capture data path [R]
<!-- ev:audio.3.6 -->
**DMA slot layout** (`_MI_AI_GetDmaChannelInfo`, 0x14e64) — one interleaved sample frame of the capture DMA: {total
slots, data slots, echo slots, pad slots, slot type[8]} with type 1 = data (mic), 2 = echo (playback loopback, the AEC
reference), 3 = pad. `n` = chnCnt (mono / queue) or 2 × chnCnt (stereo). This is the MI view of the multichannel slot
binding of §5.4.

| devices | n | total | slot types in order |
|---|---|---|---|
| 0, 3, 4, 5 (Amic) | 1 | 2 | D E |
| 0, 3, 4, 5 | 2 | 4 | D D E E |
| 0, 3, 4, 5 | 4 | 8 | D D D D E E P P |
| 1, 6 (Dmic) | 2 | 4 | D D E E |
| 1, 6 | 4 | 8 | P P E E D D D D |
| 2, 7 (I2S) | 2 / 4 / 8 | n | all D |
| 8 (Amic+Dmic) | 8 | 8 | all D |

`GetChnDataInfo(dev, c)` (ioctl 0x11) gives the per-channel split the user library uses: data slots = 1 (mono), 2
(stereo), chnCnt (queue); echo slots = 1 if the channel has an external AEC reference (mono), else 2 for Amic / Dmic with
2 or 4 data slots, 1 with 1, 0 for I2S and dev 8.

**Output thread** (the mi_sys worker woken by the timer; `_MI_AI_OnPollingAsyncOutputTaskConfig`, 0x1aa50). Under the
three mutexes, PCM must be open and the device and at least one channel enabled. With the cursor at 0 a new period is
fetched:
1. First time: `MHAL_AUDIO_StartPcmIn(dev)`; sets "started".
2. `MHAL_AUDIO_IsPcmInXrun` → if overrun, `StartPcmIn` again (restart); `MHAL_AUDIO_GetPcmInCurrDataLen(dev, &avail)`.
3. Needs `avail ≥ period + 2 sample frames` (period = ptNumPerFrm × total slots × 2 bytes); otherwise no task this time.
4. PTS of the period = `mi_sys_GetCurPts() − (avail − period) × 10⁶ / (2 × slots × rate)` µs (+0x270).
5. `MHAL_AUDIO_ReadDataIn(dev, temp, period, bBlock = 1)`; a positive return marks the temp buffer fresh (+0x278).
6. `dump_mhal_chn` on: each DMA slot's samples are appended to its own file (16-bit, de-interleaved).
7. +0x2F8 = `period / slots × (dataChn + echoChn)` = the output buffer size of one channel.

Then the cursor walks the channels; for the first enabled channel that is not only a reference channel (+0x1C) it fills
the task: channel = cursor, valid, PTS = +0x270, buffer size = +0x2F8; mi_sys allocates an output buffer of that size on
port c and calls the enqueue callback. When every channel has been served the cursor returns to 0 and the next wake-up
fetches the next period. Return 0 = task configured; other values (0x13, 0x1C) = nothing to do now.

**Enqueue** (`_MI_AI_EnqueueAsyncOutputTask`, 0x1b950): checks that the buffer size equals +0x2F8 (else drops the task); increments the channel's read counter;
then de-interleaves 16-bit samples from the temp buffer into the output buffer, by sound mode (device record +0x14):
- **Mono** (0): the channel's data slot (the c-th type-1 slot) as the first block of `ptNumPerFrm × 2` bytes, then the
  echo slot(s) as following blocks — or, if the channel has an external AEC reference (`SetExtAecChn`, channel record
  +0x14 / +0x18, i.e. +0x54 / +0x58 for channel 0), the reference channel's data slot instead of the echo slots.
- **Stereo** (1): per sample index i < samples per slot, the (2c)-th data slot → `out[2i]`, the (2c+1)-th → `out[2i + 1]`
  — the two data slots **interleaved L, R** in the first 2 × perChn bytes (perChn = buffer size / (dataChn + refChn));
  then, only for non-I2S devices, the first two echo slots (type 2) interleaved the same way in the second block
  (`out[2i + perChnSamples·2]`, `out[2i + 1 + …]`). The external AEC reference is **not used** in stereo; the echo always
  comes from the DMA echo slots. The bounds-check assertion strings compiled into this branch name each store
  (`ps16DestAddr[u32SampleIdx * 2]`, `[u32SampleIdx * 2 + 1]`, `[u32SampleIdx * 2 + (u32CopySize * 2 / sizeof(MI_S16))]`,
  `[u32SampleIdx * 2 + 1 + (u32CopySize * 2 / sizeof(MI_S16))]`).
- **Queue** (2): `dataChn` data slots then `refChn` echo slots as consecutive blocks.
- I2S devices (2, 7) never append echo.

Buffer metadata: PTS = +0x270, sequence (+0x40 of the buffer info) = the channel's delivered-frame counter, which is then
incremented. A gap of more than 3 frame times between two enqueues of a channel is logged. With `dump` on, the channel's
data (mono) or each data slot (queue) is appended to its file. Nothing in the kernel path resamples, mixes or applies
VQE; those features exist only as the user-library stubs of §2.4.

### 3.7 Gains, mute, external AEC reference [R]
Gain units are those of the MHAL functions (§5.3): **front gain** = codec ADC / PGA step index (Amic: 0–21 = −6…+57 dB
in 3 dB steps; LineIn: 0–7 = −6…+15 dB) or DMIC gain (0–4); **rear gain** = digital input gain in dB (−60…+30). MHAL
channel index: mono / queue → c; stereo → 2c and 2c+1.
- `SetChnParam(dev, c, {u8 bEnableGainSet, s16 front, s16 rear})` (ioctl 0x06): device enabled; ignored (returns 0)
  unless `bEnableGainSet == 1`. Amic devs 0 / 4 / 5: front ≤ 21, rear in −60…30 → `SetAdcGain(dev, front, ch)` +
  `SetGainIn(dev, rear, ch)`; Dmic devs 1 / 6: front ≤ 4 → `SetDmicGain(dev, front, ch)`; LineIn dev 3: front ≤ 7, rear
  −60…30 → `SetAdcGain(3, …)` + `SetGainIn(3, …)`; dev 8: channels 0–3 as Amic (front ≤ 21), channels 4–7 as Dmic (front
  ≤ 4, MHAL channel c − 4); I2S devs 2 / 7: nothing applied. Out of range → `0xA0042003`; MHAL failure → `0xA0042012`.
  The three values are stored in the channel record; `GetChnParam` returns them.
- `SetVqeVolume(dev, c, vol)` (ioctl 0x09): the "volume" is the **front gain only**, with the same per-device range and
  calls (`SetAdcGain` / `SetDmicGain`; dev 8 checks 0–21 for channels 0–3 and 0–4 for 4–7); stored as front gain;
  `GetVqeVolume` returns it. I2S: accepted, nothing applied.
- `MuteChn(dev, c, u8)` (ioctl 0x0F) only stores the flag (+0x38 of the channel record); the kernel never zeroes data —
  the user library does it in `MI_AI_GetFrame` after reading the flag with ioctl 0x10.
- `SetExtAecChn(dev, c, refChn)` (ioctl 0x0D): mono sound mode only (`0xA0042003`), device enabled; marks channel c "has
  external reference = refChn" and refChn "is a reference" (such a channel gets no output tasks of its own); the mono
  enqueue then appends refChn's data slot after c's data instead of the echo slots (§3.6). Nothing undoes it except
  Disable / DeInit.
- prog_uvc's audio demo sets a default "volume" of 30 on the Dmic device, outside this mi_ai's Dmic range 0–4, so its
  `-d 1` fails with `0xA0042003` (`prog_uvc.md` §9.2).

### 3.8 Proc interface (`/proc/mi_modules/mi_ai/mi_ai<dev>`) [R]
Dump callbacks: `OnDumpDevAttr` prints "Private AI%d Info" and the attr table (`AiDev SampR BitWidth SoundMod PtNumPerFrm
TotalReadFrmCnt ChnCnt` and `I2sMode I2sMclk I2sFmt bI2sSync TdmSlots I2sBitWidth`, with the device names of §3.2 and
"i2s-mas/i2s-sla/tdm-mas/tdm-sla", "16bit/24bit/32bit", "mono/stereo/queue", "I2S-MSB/LEFT-MSB", "disable/12.228M/16.384M/
18.432M/24.576M"); `OnDumpChannelAttr` prints per channel `AiDev AiChn bChnEnable Volume(front,rear) ReadFrmCnt bMute`;
the input / output-port dumps print nothing of their own; `OnHelp` lists the three commands.
Commands (`mi_sys_RegistCommand`):
- `set_ai_volume <chn> <front> <rear>`: device must be enabled; same ranges and MHAL calls as SetChnParam ("Illegal Ai
  ChnID.Dev%d Only support[0,1]" for 2-channel devices, "[0,3]" / "[0,1,2,3]" for 4-channel ones; "I2S_Rx doesn't
  support setting volume"); stores the gains.
- `dump <path> <on|off>`: per enabled channel opens `<path>/Dev%d_Chn%d_%s_%d.pcm` ("Mono" / "Stereo", running index)
  with `filp_open(O_CREAT|O_TRUNC|O_RDWR, 0644)`; the enqueue appends each delivered channel frame (§3.6). `off` / `OFF`
  / `0` closes them.
- `dump_mhal_chn <path> <on|off>`: per DMA slot opens `<path>/Dev%d_Chn%d_%d.pcm`; the output thread appends each slot's
  de-interleaved samples of every period read from MHAL (§3.6 step 6) and logs "MI AI Dump, Frame Count:%d,
  TotalSize:%d" every 100 periods [I: the constant 0xC28F5C29 is the modular inverse used for "count % 100 == 0"].

## 4. mi_ao.ko — audio output

### 4.1 Load, parameters, life cycle [R]
- `init_module`: `/proc/mi_modules/mi_ao/` with the four standard files (version string as mi_ai), global table
  `CamOsMemAlloc(0x690)` = 6 × 0x118, `MI_DEVICE_Register` → `/dev/mi_ao`. Parameters: `debug_level` (2), `debug_file`,
  `debug_func`, **`DmaBufferSize`** (int, 0 = automatic; used only if a multiple of 2 KiB).
- `process_init` / `process_exit` and ioctls 0x16 / 0x17 call `_MI_AO_Init` / `_MI_AO_DeInit` (atomic reference count).
  **`_MI_AO_Init`** (first user): clears the 6 records and sets their ids, `MHAL_AUDIO_Init(NULL)`, then
  `MHAL_AUDIO_SetGainOut(dev 0, 0 dB, ch 0, fading 7)` and the same for ch 1. **`_MI_AO_DeInit`** (last user): per device,
  if a buffer address was registered with ioctl 0x1C free it (`mi_sys_MMA_Free`), disable enabled channels, disable the
  device if enabled; then `MHAL_AUDIO_DeInit()`.
- Unlike mi_ai there is **no mi_sys data path**: the mi_sys device registered at Enable (`mi_sys_RegisterDev` {module 5,
  dev, 1 pass, no ports, no channels}, handle at +0xC8) exists only for the proc file and its commands; samples go from
  the ioctl straight into the MHAL DMA ring.

### 4.2 Devices [R]
| dev | proc name | MHAL AO device | rates | channels (mono · stereo) |
|---|---|---|---|---|
| 0 | DAC0/1 | line out, both DACs | 8, 11.025, 12, 16, 22.05, 24, 32, 44.1, 48 k | 1, 2 · 1 |
| 1 | I2sTx | I2S TX | as dev 0, plus the I2S check (rates 8/16/32/48 k, work mode 0–3, TDM slots 4 or 8, bit width 16/32) | 1, 2 · 1 |
| 2 | DAC0 | line out, DAC 0 only | as dev 0 | 1 · — (mono only) |
| 3 | DAC1 | line out, DAC 1 only | as dev 0 | 1 · — (mono only) |
| 4 | HDMI | HDMI audio | 32 or 48 k only | 1, 2 · 1 |
| 5 | HDMI + Lineout | HDMI and line out together | 32 or 48 k only | 1, 2 · 1 |

Two channels per device (chn 0, 1). Queue sound mode (2) is refused for every device. `MI_AO_IMPL_SetPubAttr` also
requires `0 < u32PtNumPerFrm ≤ rate` and 16-bit samples, and refuses while enabled (`0xA0052009`). MHAL rates are passed
as the enum values (11025 → 11000, 22050 → 22000, 44100 → 44000). The hardware side of each device is in §5.4.

### 4.3 Device record (0x118 bytes, `_LANCHOR1 + dev × 0x118`) [R offsets; I names]
| off | field |
|---|---|
| +0x00 / +0x01 / +0x04 | u8 enabled / u8 attr set / dev id |
| +0x08 | `MI_AUDIO_Attr_t` (0x34) — +0x08 rate, +0x0C width, +0x10 work mode, +0x14 sound mode, +0x1C pt num, +0x24 chn cnt, +0x28… I2S fields |
| +0x40 + 0x30·c | channel c: +0x00 u8 enabled, +0x01 u8, +0x04 id, +0x08 counter, +0x10…+0x27 user-queue status from ioctl 0x12 (queue size, used, u64 total queued, u64 run time), +0x28 u8 mute |
| +0xA0 / +0xA4 | s32 volume dB of chn 0 / 1 (SetVolume, SetLRVolume, SetChnParam) |
| +0xA8 / +0xB0 / +0xB4 | DMA buffer: u64 phys / kernel virt / size |
| +0xB8 / +0xC4 / +0xC0 | TMP buffer (staging for one frame): u64 phys / virt / size |
| +0xC8 | mi_sys device handle |
| +0xCC | bytes written since (re)start, for the start threshold |
| +0xD0 | frames written |
| +0xD8 | Enable time (`CamOsGetMonotonicTime`) |
| +0xE0 | u8 PCM out started |
| +0xE1 | u8 PCM configured |
| +0xE8 | u64 MMA buffer address from ioctl 0x1C (freed at DeInit) |
| +0xF0 | u8 paused |
| +0xF4 + 0xC·c | ChnParam copy (u8 bEnableGainSet, s16 gain) |
| +0x110 | u64 total bytes written to MHAL |

### 4.4 Enable, SendFrame, status [R]
**`MI_AO_IMPL_Enable(dev)`** (0x16644): attr set (`0xA0052007`); a second call returns 0.
1. Registers the mi_sys device and the proc commands (§4.5).
2. Two buffers, each sized `DmaBufferSize` (if non-zero and a multiple of 2 KiB) or from `(rate × bytesPerSample / 2
   rounded down to 8) + 8` rounded up to 64 / 128 / 256 KiB: the **TMP** buffer from mmap `E_MMAP_ID_AO_TMP` or the
   private heap "ao-Dev%d-tmp", and the **DMA** buffer from `E_MMAP_ID_AO` or "ao-Dev%d-dma"; both `mi_sys_Vmap`ped and
   zeroed.
3. `MHAL_AUDIO_ConfigPcmOut(dev, {rate (MHAL value), width 0, u16 channels = chnCnt (mono) or 2 × chnCnt (stereo),
   interleaved 1, DMA virt, DMA phys, DMA size, period = ptNumPerFrm × channels × 2, start threshold = the same})`.
4. Device 1 (I2S TX) only: `MHAL_AUDIO_ConfigI2sOut(1, {mode, width (1 if I2S bit width 32), fmt, rate, mclk, 4-wire =
   bSyncClock, channels = max(2, channels) or TDM slots in TDM modes})` from device 1's attr.
5. `MHAL_AUDIO_OpenPcmOut(dev)`; records the Enable time; sets enabled; clears the dump flag.

Failure unwinds: free the DMA buffer, the TMP buffer, unregister the mi_sys device (`0xA0052012`).

**`MI_AO_IMPL_Disable`**: `0xA0052012` while a channel is enabled; `MHAL_AUDIO_StopPcmOut` + `ClosePcmOut`; frees both
buffers; unregisters; clears totals, threshold counter, started, enabled.
**`EnableChn` / `DisableChn`** (c < 2, device enabled): only set / clear the channel record — no hardware action.

**`MI_AO_IMPL_SendFrame(dev, c, frame)`** (0x1a254; ioctl 0x06 copies the 0x120-byte frame from the payload):
1. c < 2, device and channel enabled (`0xA0052017`), TMP buffer present.
2. Copies `frame.u32Len[0]` bytes from `frame.apVirAddr[0]` into the TMP buffer — `CamOsCopyFromUpperLayer` when the
   address is a user address (< 0xC0000000), `memcpy` for a kernel caller. Only element 0 is used: the user library has
   already interleaved the channels (§2.6).
3. Mute is applied in software on the staged data: 1 channel (mono / stereo mode) → the whole frame is zeroed if chn 0 is
   muted; 2 channels in mono mode → the even (chn 0) or odd (chn 1) 16-bit samples of the muted channel are zeroed.
4. If `MHAL_AUDIO_IsPcmOutXrun` reports an underrun, the start-threshold counter and "started" are reset (playback
   re-primes). Room check: `DMA size − GetPcmOutCurrDataLen() ≥ len`, else `0xA005200D`.
5. `MHAL_AUDIO_WriteDataOut(dev, tmp, len, bBlock = 1)`, repeated while it returns a negative value other than −2 (−2 =
   full → `0xA005200D`); the written bytes are added to the u64 total.
6. Until started: adds to the threshold counter; when it reaches `ptNumPerFrm × 2 × chnCnt` (× 2 for stereo) calls
   `MHAL_AUDIO_StartPcmOut(dev)` (retried until it succeeds) and sets started. So playback starts after one frame is
   buffered.
7. Counts the frame; with `dump` on, appends the written bytes to the dump file.

**`QueryChnStat(dev, c)`** (ioctl 0x0A): device and channel enabled; `busy = MHAL_AUDIO_GetPcmOutCurrDataLen(dev)`,
`total = DMA size`, `free = total − busy` (bytes, not frames). **`UpdateQueueStatus`** (ioctl 0x12) only stores the six
words for the proc dump. **`PauseDev` / `ResumeDev`**: `MHAL_AUDIO_PausePcmOut` / `ResumePcmOut`, `0xA0052009` if
already in that state. **`ClearChnBuf`**: `MHAL_AUDIO_PrepareToRestartPcmOut(dev)` and resets threshold counter and
started (the next frames re-prime and restart the DMA). **`GetAoStartPts`, `ReadAoData`, `LseekAoData`**: accept a
non-NULL pointer and return 0 without doing anything (empty in this build). **`SetDevMmaBufAddr(dev, flag, u64
addr)`**: if flag ≠ 0 stores the address (only consumer: `mi_sys_MMA_Free` at DeInit). **`GetChipDevNum`** → 6.

### 4.5 Volume, mute, proc [R]
- `SetVolume(dev, c, dB, fading)`: dB −60…+30, fading 0–7 (`MHAL_AUDIO_GainFading_e`: off, 1, 2, 4 … 64 samples),
  device enabled; with 1 channel in mono / stereo mode both MHAL channels 0 and 1 get the gain, otherwise channel c:
  `MHAL_AUDIO_SetGainOut(dev, dB, ch, fading)`; stored at +0xA0 / +0xA4. `GetVolume` returns it.
- `SetLRVolume(dev, c, left, right, fading)`: `SetGainOut(dev, left, 0, f)` and `(dev, right, 1, f)`; stored.
- `SetChnParam(dev, c, {u8 bEnableGainSet, s16 gain, u32 fading})`: only if bEnableGainSet == 1; gain −60…30, fading < 8
  → `SetGainOut(dev, gain, c)`; stored. `GetChnParam` returns the copy.
- `SetSrcGain(dev, dB)` (−60…30): `MHAL_AUDIO_SetSrcGain(dev, dB, 0)` and `(…, 1)` — the gain of the device's SRC DPGA
  (§5.3).
- `SetMute(dev, c, u8)`: stores the flag (applied in SendFrame) and calls `MHAL_AUDIO_AmpEnable(!mute, c)` — the speaker
  amplifier GPIO of that channel (§5.6); returns what AmpEnable returns when the device is enabled, `0xA0052017`
  otherwise. `GetMute` returns the flag.
- Proc `/proc/mi_modules/mi_ao/mi_ao<dev>`: device dump `AoDev SampR BitWidth SondMod PtNumPerFrm VolumeDb` + the I2S
  line; channel dump `AoDev AoChn bMuteEnable RunTime` and "Usr Queue STATUS" `MaxSize RemainSize TotalSize RunTime`
  (from ioctl 0x12). Commands: `set_ao_mute <chn 0/1> <on|off>`, `set_ao_volume <chn> <−60…30>` (SetGainOut),
  `set_ao_src_volume <chn> <−60…30>` (SetSrcGain), `dump <path> <on|off>` → `<path>/Dev%d_%d_%s.pcm`, written by
  SendFrame.

## 5. mhal.ko block AUDIO ("bach")

### 5.1 What it is, layers, relation to the source [R unless tagged]
<!-- ev:audio.5.1 -->
- 178 functions, 21.8 KB of code, 34 exports (`mhal-core.md` §2.1). Layers as in the OpenIPC u-boot tree
  `drivers/mstar/aio` (mercury6): MHAL API (`mhal_audio.c`, the exports, §5.3) → DRV (`drv_audio.c`: DMA ring / period
  logic, IRQ, PTS, §5.5) → OS glue (`drv/src/linux/drv_audio_api_linux.c`, `audio_proc.c`: DT, IRQ request, amplifier
  GPIO, `/proc/audio`, §5.6; `hal_audio_os_api_linux.c`: padmux and amplifier pad queries, §5.7) → HAL (`hal_audio.c`:
  registers, §5.8–§5.14; `hal_audio_config.c`: the `card` configuration tables, §5.4; `hal_audio_reg.c`: register
  access, §5.2).
- The source tree is an older revision of the same HAL: it has 6 AI / 5 AO devices where this mhal has 9 / 6 (§5.4). One
  export, `MHAL_AUDIO_DmaRdSwap`, is not in the published `mhal_audio.h`.
- Names: 172 of the binary's 178 symbols are in the source; 6 are binary-only — `MHAL_AUDIO_DmaRdSwap`, `DrvAudAoSwap`,
  `HalAudDmaRdSwap` (§5.10), `HalAudAtopSetAdcMode` (§5.9), `HalAudDigMicBckMode` (§5.14), `_AudDmaStopChannelIrq`
  (§5.5). Four of the source's static helpers are in the binary as unnamed functions, identified by body: `.text`
  0x2CD0 `_HalAudAtopAdc`, 0x2D28 `_HalAudDmaGetRawMiuUnitLevelCnt`, 0x2DE0 `_HalAudDmaGetMchSelConfigValue`, 0x2E44
  `_HalAudAtopSwitch`. Other static helpers of the source (`_HalAudDmaFirstInit`, `_AudPcmCaptureAvail`, `_WaitForAvail`
  …) have no symbol in the binary [I: inlined].
- Comparison with the source: every HAL function, the 17 functions whose source is the Linux layer `drv/src/linux/*.c`
  (`_AudDmaIrq`, `DrvAudApi*`, `AudioProc*`) and every DRV / MHAL function whose HAL / DRV call list differs from the
  source were read against the source body; the two functions of `hal_audio_os_api_linux.c` were also compiled from the
  source and compared call list by call list and instruction by instruction. A difference here means a register write or
  read the other side does not have, a different bank / register / mask / value, a different order of writes, or a
  different condition choosing a write; log `__LINE__` and format arguments, offsets of the driver's own state structs,
  `.rodata` tables holding the source's values and compiler layout (switch tables, inlining, merged writes) are not
  differences. The binary's `hal_audio.c` is about 70–90 lines longer than the source's (log line numbers shifted by
  0x40–0x5C). The constant-set comparison finds 17 of 146 name-matched
  functions with identical constant sets and, for every `MHAL_AUDIO_*` export, all of the source's constants present in
  the binary. **Real differences** (everything not listed is the same as the source):

| layer | function | difference (details in the section) |
|---|---|---|
| HAL ATOP §5.9 | `_HalAudAtopAdc` | power-down leaves ADC1 on; `EN_SW_MIC2_L/R` taken from DT `adc23-mode` |
| | `_HalAudAtopSwitch` MICIN off | mic pre-amp stage 2 stays on |
| | `_HalAudAtopSwitch` DAC on / off | ramp on unnamed regs 0x1503:0x7A / 0x7C with a 150 ms sleep each way; `CTRL0e` soft-function write dropped |
| | `HalAudAtopClose` | reference (`PD_VI` / `PD_VREF`) really powered down (source writes 0) |
| HAL mux §5.10 | `HalAudSetPadMux` | I2S TX / RX requests redirected to the shared RX/TX pad mode when that mode ≠ 0 |
| HAL DMA §5.12 | `HalAudDmaReset` | 2 ms sleep at entry and between INIT set and clear |
| | `HalAudDmaEnable` | 10 µs between the two writes also on reader disable |
| | `HalAudDmaGetLevelCnt` | reader level ≤ 4 MIU words (64 B) reported as 0 |
| HAL DMIC §5.14 | `HalAudDigMicSetChannel` | 8 channels → mode 2 (source 3) |
| | `HalAudDigMicSetRate` | `FS_SEL` from DT `dmic-bck-mode-*`, range-checked |
| HAL HDMI §5.14 | `HalAud2Hdmi` | extra DMA argument selects READER1 / READER2 for HDMI |
| Linux glue §5.6 | `DrvAudApiDtsInit`, `DrvAudApiAoAmpEnable`, `DrvAudApiAoAmpCtrlInit` | new DT keys; amplifier level from DT; no pad set-up when the padmux driver is active |
| DRV §5.5 | `DrvAudPrepareDma`, `DrvAudStartDma`, `_AudDmaKernelCopy`, `DrvAudReadPcm`, `DrvAudIsXrun`, `DrvAudEnableHdmi` | interrupts off before reset; threshold interrupt at start, re-armed from the copy path; one extra level latch per read; XRUN from the IRQ only; HDMI DMA argument |
| MHAL §5.3 | `MHAL_AUDIO_IsPcmOutXrun`, `Open/ClosePcmOut` | playback re-prepared on XRUN; HDMI DMA argument |

- The source's `drv/src/common/drv_audio.c` is a u-boot adaptation of the Linux driver (spinlock calls commented out, no
  IRQ path, `DrvAudIsXrun` polls the hardware); the binary is the Linux form (spinlock per engine, semaphore wake-up,
  IRQ).
- `MHAL_AUDIO_Init` (first call only) takes a mutex, runs `DrvAudInit` (§5.5) and `AudioProcInit`, which creates
  `/proc/audio/` (§5.6). The block has no initcall of its own: nothing in it runs at `insmod mhal.ko`.

### 5.2 Register access, banks, IRQ, clock [R unless tagged]
- `HalBachWriteReg(bank, reg, mask, value)` / `HalBachReadReg`; `reg` = even byte offset in the bank;
  `HalBachWriteRegByte(addr, mask, value)` / `HalBachReadRegByte` for single bytes; `HalBachWriteReg2Byte` /
  `HalBachReadReg2Byte` by absolute address. RIU 16-bit registers at `riu_base + 2 × (B + reg)` (bytes: − (addr & 1)),
  masked read-modify-write. The base pointer is set by `HalBachSysSetBankBaseAddr(IO_ADDRESS(0x1F000000))` in
  `DrvAudInit` and lives right after `card` (`card+0x38`). `DrvAudApiIoAddress` = addr − 0x2200_0000 (`IO_ADDRESS`,
  0x1F00_0000 → 0xFD00_0000). RIU addressing in general: `kernel-platform.md` §3.1.
- The HAL's banks: B = 0x150200 (**bank 0 = RIU 0x1502**, DMA, mixers, DPGA), 0x150300 (**bank 1 = 0x1503**, I2S / TDM,
  routing, interrupt enable), 0x150400 (**bank 2 = 0x1504**, DMIC), 0x103400 (**bank 3 = 0x1034**, the analog front end
  "ATOP"). Below, `bank:reg` uses the RIU bank number. Register and field names follow the mercury6 `hal_audio_reg.h`
  (991 lines) [S], which numbers the banks its own way.
- Bank **0x103C** (chip-top padmux; byte addresses `0x103CC4` = reg 0x62, `0x103CF2` = reg 0x79) is written by the AUDIO
  HAL directly through `HalBachReadReg2Byte` / `WriteReg2Byte`, not through the padmux driver (§5.10).
- 0x1503:0x7A and 0x1503:0x7C have no name in `hal_audio_reg.h`: its list for that bank jumps from `BACH_I2S_CFG00` 0x78
  to `BACH_NF_SYNTH_RX_BCK00` 0x82 [S]. Their bits [15:13] are written as a DAC ramp (§5.9); what they mean is not known
  from any header or datasheet [I: a DAC output ramp ("depop"), one register per DAC channel].
- IRQ: DT `interrupts = <0 0x4a 4>` (GIC SPI 0x4A, level), one handler `_AudDmaIrq` for the four DMA engines (§5.5).
  Clock `CLK_bach` (gate at 0x1F2071B4 bit 7; `kernel-platform.md` §5.1), enabled by `DrvAudApiDtsInit` (§5.6).

### 5.3 MHAL API (exports) and their use by mi_ai / mi_ao [R names, callers and call graph; behaviour as in the source except where marked]
Return 0 = success, −1 = failure, unless noted. Device ids are the MI ids (§3.2, §4.2); each call first looks the device
up in `card` (§5.4) and fails for an id ≥ the device count.

| export | behaviour | caller |
|---|---|---|
| `MHAL_AUDIO_Init(void*)` / `DeInit()` | once, under a mutex: `DrvAudInit` (DT parse, padmux, amplifier GPIOs, IRQ, `HalAudMainInit` register defaults, §5.5, §5.8) and `AudioProcInit` / reverse with `AudioProcDeInit` | mi_ai, mi_ao |
| `ConfigPcmIn(dev, PcmCfg*)` / `ConfigPcmOut` | `PcmCfg` = {rate, width 0 = 16 / 1 = 32, u16 channels, u8 interleaved, DMA virt, u64 phys, size, period bytes, start threshold}: sets the device's DMA engine (§5.4) parameters, its mux list, the multichannel binding for the channel count (AI), DMIC rate / channels, ADC selection, AEC (echo) path | mi_ai / mi_ao |
| `ConfigI2sIn(dev, I2sCfg*)` / `ConfigI2sOut` | `I2sCfg` = {mode, width, fmt, rate, mclk, 4-wire, u16 channels}: I2S / TDM interface of the device | mi_ai (devs 2, 7) / mi_ao (dev 1) |
| `OpenPcmIn/Out(dev)` | DMIC enable (if used), DMA open, analog path on (`DrvAudEnableAtop`), I2S enable, DMA prepare; `OpenPcmOut` / `ClosePcmOut` pass the device's reader DMA to `DrvAudEnableHdmi(b, dma)` (binary only, §5.14) | both |
| `ClosePcmIn/Out(dev)` | reverse of Open | both |
| `StartPcmIn/Out`, `StopPcmIn/Out` | DMA engine start / stop | both |
| `PausePcmOut`, `ResumePcmOut` | reader DMA pause / resume | mi_ao |
| `ReadDataIn(dev, buf, size, bBlock)` | copies `size` bytes out of the writer ring; returns bytes, −2 when empty and not blocking | mi_ai |
| `WriteDataOut(dev, buf, size, bBlock)` | copies into the reader ring; returns bytes, −2 when full | mi_ao |
| `IsPcmInXrun` / `IsPcmOutXrun` | TRUE on over / underrun, and then re-prepares the DMA (`DrvAudPrepareDma`) so the status resets; for `IsPcmOutXrun` the re-prepare is binary only (the source only reports) | both |
| `PrepareToRestartPcmOut/In` | stop and re-prepare so a later Start restarts cleanly | mi_ao (ClearChnBuf) / — |
| `GetPcmInCurrDataLen` / `GetPcmOutCurrDataLen(dev, u32*)` | bytes currently in the ring | both |
| `GetPcmOutStartPts(dev, u64*)` | PTS of playback start | — (unused) |
| `SetGainIn(dev, s16 dB, ch)` | each DPGA of the device's list ← dB (digital, −60…+30 per the header; `BACH_DPGA_GAIN_MIN_DB` −64; §5.13) | mi_ai |
| `SetAdcGain(dev, idx 0–21, ch)` | analog: `_gaMicInCombineGainTable[idx]` (22 × {ADC gain, pre-gain} in `.rodata`, the source's table) → ADC gain + mic pre-amp gain for the device's ADC selection (MICIN, MICIN2, MICIN_4CH; LineIn uses its own 0–7 range) (§5.9) | mi_ai |
| `SetDmicGain(dev, gain, ch)` | DMIC digital gain (0–4, 6 dB steps; §5.14) | mi_ai |
| `SetGainOut(dev, s16 dB, ch, fading)` | output DPGA with fading 0–7 | mi_ao |
| `SetSrcGain(dev, dB, ch)` | the device's `src_dpga` (DEC1) [I: the gain of the playback signal as it is looped back as the "SRC" echo reference of §5.4] | mi_ao |
| `DmaSineGenOut/In(dev, {en, freq, gain})` | test tone generator on the DMA (48 kHz, freq code 2 = 1 kHz, gain −6 dB per step; §5.14) | — (unused) |
| `AmpEnable(bool, ch)` | drives the amplifier GPIO of ch (§5.6) | mi_ao (SetMute) |
| `DmaRdSwap(sel)` (binary only) | → `DrvAudAoSwap` (a tail jump) → `HalAudDmaRdSwap(sel)` (§5.10) | — (no importer on the camera) |

### 5.4 Device configuration (`card`, `.data` 0x30) [R; enum names from the mercury6 `hal_audio_types.h` (S), numbering checked on the six source devices]
`card` (56 bytes, symbol size 56) has exactly the source layout of `struct audio_card` (`hal/common/hal_audio_config.h`,
14 words), confirmed by its consumers (`MHAL_AUDIO_*` read +0x04 / +0x0C and +0x08 / +0x10; `DrvAudEnableI2s`
+0x14 / +0x18; `DrvAudDigMicEnable` +0x2C; `DrvAudApiDtsInit` +0x34):

| off | field | value |
|---|---|---|
| +0x00 | name | "mercury6" |
| +0x04 / +0x0C | `ao_devs` / `num_ao_devs` | 6 AO devices |
| +0x08 / +0x10 | `ai_devs` / `num_ai_devs` | 9 AI devices |
| +0x14 / +0x18 | `i2sIfs` / `num_i2sIfs` | 2 |
| +0x1C / +0x20 | `i2ss` / `num_i2ss` | 2 |
| +0x24 / +0x28 | `dmas` / `num_dmas` | not recorded |
| +0x2C / +0x30 | `dmics` / `num_dmic` | not recorded |
| +0x34 | `dts` | the DT key-name table (§5.6) |

The register base pointer of `HalBachWriteReg` follows at `card+0x38`. Per device a record `{id, info*, op*}`; every device points to
the same `op` record (`.rodata+0x1080`), which holds no operations (empty).

**AI device info** — 20 words, the source `struct audio_ai_dev_info` (… +0x44 `adc_sel`, +0x48 `use_dmic`, +0x4C
`use_aec`):

| AI dev | DMA | inputs (mux) | DPGA | multichannel slots per DMA channel count | ADC sel / DMIC / AEC |
|---|---|---|---|---|---|
| 0 Amic | WRITER1 | DMAWR1, DMAWR1_MCH | ADC, AEC1 | 1,2 → 4: AMIC01, SRC, –, – | MICIN / – / yes |
| 1 Dmic | **WRITER1** (source: WRITER2) | DMAWR1_MCH | – | 1,2 → 4: DMIC01, SRC; 4 → 8: –, SRC, DMIC01, DMIC23 | – / yes / yes |
| 2 I2S RX | WRITER1 | I2STDM_RX, DMAWR1_MCH | – | 2 → 2 RX01; 4 → 4 RX01, RX23; 8 → 8 RX01…RX67 | – / – / no |
| 3 LineIn | WRITER1 | DMAWR1, DMAWR1_MCH | ADC, AEC1 | as dev 0 | LINEIN / – / yes |
| 4 Amic D2 | WRITER2 | DMAWR2_MCH | DEC2 | 1,2 → 4: AMIC23, SRC | MICIN2 / – / yes |
| 5 Amic 4ch | WRITER2 | DMAWR1, DMAWR2_MCH | ADC, AEC1, DEC2 | 4 → 8: AMIC01, AMIC23, SRC, – | MICIN_4CH / – / yes |
| 6 Dmic D2 (not in source) | WRITER2 | DMAWR2_MCH | – | as dev 1 | – / yes / yes |
| 7 I2S RX D2 (not in source) | WRITER2 | I2STDM_RX, DMAWR2_MCH | – | as dev 2 | – / – / no |
| 8 Amic+Dmic (not in source) | WRITER2 | DMAWR1, DMAWR2_MCH | ADC, AEC1, DEC2 | 8 → 8: AMIC01, AMIC23, DMIC01, DMIC23 | MICIN_4CH / yes / no |

Slot pairs are 2 channels each ("SRC" = the echo reference from the playback path, "–" = unused pair); this is the
hardware side of mi_ai's DMA slot table (§3.6). The slot codes the HAL writes are in §5.12 (`HalAudDmaWrConfigMch`).

**AO device info** — 21 words: the source `struct audio_ao_dev_info`'s 20 (`dma`, `i2s_if`, `i2s`, `tdm`, `atop_path`,
`use_i2s`, `use_tdm`, `use_atop`, pcm mux list + count, i2s mux list + count, dpga list + count, `use_mch`, `info_mch`,
`i2sIf_cfg_private_data`, +0x44 `use_hdmi`, +0x48 `use_src_dpga`, +0x4C `src_dpga` — positions confirmed by
`MHAL_AUDIO_OpenPcmOut` (+0x10, +0x14, +0x1C, +0x44), `ConfigPcmOut` (+0x20 / +0x24, +0x38 / +0x3C), `SetSrcGain`
(+0x48 / +0x4C)) plus **one appended word at +0x50**:

| AO dev | DMA | mux list | DPGA | analog / I2S / HDMI | src DPGA | word +0x50 |
|---|---|---|---|---|---|---|
| 0 Line out | READER1 | MMC1←1, DMARD1, SDM_DINL, SDM_DINR | MMC1 | DAC line out / – / – | DEC1 | 1 |
| 1 I2S TX | READER2 | I2S_CFG_00←2, I2STDM_RX←1, DMARD2←1 | TDM | – / CODEC_TX / – | DEC1 | 2 |
| 2 DAC0 | READER1 | MMC1, MMC2, DMARD1, DMARD2, SDM_DINL←0, SDM_DINR←5 | MMC1, MMC2 | DAC / – / – | DEC1 | 3 |
| 3 DAC1 | READER2 | as dev 2 | MMC1, MMC2 | DAC / – / – | DEC1 | 2 |
| 4 HDMI | READER1 | same values as dev 0 (source: the HDMI I2S list) | MMC1 | DAC / – / yes | DEC1 | 3 |
| 5 HDMI + line out (not in source) | READER2 | MMC2←1, DMARD2←0 | MMC2 | – / – / yes | DEC1 | 2 |

The word at +0x50 has no reader: the only 32-bit load with offset 0x50 in the AUDIO `.text` (0–0x5A00) is
`DrvAudApiDtsInit` reading the DT key-name table (`.text` 0x2A34), and no other module imports `card`
(the import tables of every module, the `mi_ai.ko` / `mi_ao.ko` symbol tables). [I] It is dead data in this build; its values look like
an output channel mask (1 = ch 0, 2 = ch 1, 3 = both).

### 5.5 DRV layer: DMA engines, ring, IRQ, PTS [R binary; names from the source]
- Four DMA engines: WRITER1, WRITER2 (capture, memory ← codec), READER1, READER2 (playback, memory → codec); their
  register groups are in §5.12. Per engine a runtime record (0x58 bytes in the binary, base symbol `nMchExt`): buffer
  virt / phys / size, period size, frame size in bits (+0xEC / +0xEE), level reserve (+0xDC), software pointer (+0x2C),
  PTS pair (+0x48 / +0x4C), a spin lock, a wait queue, and the state (+0xF0) with the ALSA-like bit values
  `SND_PCM_STATE_INIT 1, SETUP 2, OPEN 4, PREPARED 8, RUNNING 0x10, XRUN 0x20, PAUSED 0x40` (`drv_audio_pri.h`) [S
  names]. Besides that state the copy path and the IRQ keep a ring condition — NORMAL, UNDERRUN / EMPTY (playback),
  OVERRUN / FULL (capture) (source names) [I: a separate field; the sources call both "state" and give no offset]. Each open engine also gets a 0x60-byte private record (`CamOsMemCalloc`).
  The record offsets above reach past 0x58; how they relate to the 0x58-byte size given for the record is not settled in
  the sources [I: they are counted from a base other than the record start].
- **`DrvAudInit`** (same order as the source): `DrvAudApiDtsInit` (§5.6; padmux modes, DT values) → IRQ init and the
  DMA-stop callback `_AudDmaStopChannelIrq` → `HalAudSetAoAmpCb(_DrvAudAoAmpEnable)` → `DrvAudApiAoAmpCtrlInit` →
  `HalBachSysSetBankBaseAddr(IO_ADDRESS(0x1F000000))` → `HalAudMainInit` (§5.8) → per engine runtime reset +
  `HalAudDmaIntEnable(dma, 0, 0)`. So the DT / padmux state is in place before `HalAudMainInit` runs its padmux writes.
- **`DrvAudReadPcm(dma, buf, bytes, block, *pts)`**: needs state PREPARED, RUNNING or PAUSED (XRUN → −EPIPE (−32),
  other → −EBADFD (−77)); available frames = `(HalAudDmaGetLevelCnt − reserve) / frame bytes`; while frames remain: if
  none available, returns −EAGAIN (−11) when not blocking, or sleeps on the wait queue (woken by the IRQ); copies the
  contiguous part up to the ring end with `_AudDmaKernelCopy` (variants `_AudDmaWrCopyN16/I16/N32/I32`,
  `_AudDmaRdCopyN16` …: N = non-interleaved, I = interleaved, 16 / 32-bit; user or kernel buffer), advances the software
  pointer (+0x2C) with wrap; returns bytes. After the copy loop it reads `HalAudDmaGetRawLevelCnt` once more for the PTS
  (one more level-count latch pulse, `CTRL_9` / `CTRL_1` bit 12, per call; binary only). The PTS output = monotonic time −
  frames buffered × 10⁶ / rate (µs); `MHAL_AUDIO_ReadDataIn` discards it. **`DrvAudWritePcm`** is the mirror for readers
  (level count → free space → `_AudDmaKernelCopy`; full → −EAGAIN → MHAL −2).
- **`_AudDmaIrq`** (SPI 0x4A, shared by the four engines, `IRQF_SHARED`): reads the engine's flags (`HalAudDmaGetFlags`:
  underrun, overrun, trigger level) and interrupt status. Playback: boundary (empty) interrupt → that engine's interrupts
  off, the stop callback (`_AudDmaStopChannelIrq`: `HalAudDmaEnable(dma, 0)` and clears the PTS pair +0x48 / +0x4C),
  state XRUN, condition EMPTY; threshold (underrun) interrupt → only the boundary interrupt left on, condition UNDERRUN,
  wake the writer. Capture is the mirror (FULL / OVERRUN, wake the reader). A state other than RUNNING at an interrupt
  prints "[AUDIO ERROR]DMA Id = %d, Current Status = %d" and switches all interrupts of the engine off. The source's
  level-count debug print is compiled out.
- XRUN is set only by `_AudDmaIrq`: the binary's `DrvAudIsXrun` is `status == XRUN` with no register access (the
  source's active branch reads `GetFlags` / `GetInt` and disables the interrupts itself; the binary has the source's
  `#if 0` branch). `MHAL_AUDIO_IsPcm*Xrun` reports XRUN and clears it by re-preparing the engine; mi_ai / mi_ao then call
  Start again (§3.6, §4.4).

DRV functions with a register-relevant behaviour (HAL calls, their arguments and order; OS-level locking aside):

| function | binary (differences from the source marked **DIFF**) |
|---|---|
| `DrvAudConfigDmaParam` (+ inlined `_AudDmaInitChannel`) | `HalAudDmaSetPhyAddr`, then writers `WrSetThreshold(period + 64)` / readers `RdSetThreshold(buffer − period)`, then `HalAudDmaWrConfigMch` for the channel count's binding |
| `DrvAudOpenDma` | `request_threaded_irq` for the engine, then `HalAudDmaSetRate` |
| `DrvAudPrepareDma` | **DIFF** (order, one write): under the engine lock `HalAudDmaIntEnable(dma, 0, 0)` (both interrupts off), ring / state reset (state INIT, `memset` of the buffer), then `HalAudDmaReset`. Source: `HalAudDmaReset` first, then the ring reset; no interrupt write |
| `DrvAudStartDma` (+ inlined `_AudDmaStartChannel`) | writers flush the MIU pipe, `HalAudDmaClearInt`, `HalAudDmaEnable(dma, 1)`, state RUNNING; then **DIFF**: `HalAudDmaIntEnable(dma, 1, 0)` = the **threshold** interrupt (writer overrun-threshold / reader underrun-threshold, bits 14 / 13); source `(0, 1)` = the **full / empty** interrupt (bits 11 / 10) |
| `_AudDmaKernelCopy` | **DIFF**: `HalAudDmaClearInt` only when the state is INIT (source: also after EMPTY / FULL); returns early whenever the condition is EMPTY (playback) / FULL (capture) after the copy (source: only on a transition into it); when the ring is back above (playback) / below (capture) one period and RUNNING, condition NORMAL **and `HalAudDmaIntEnable(dma, 1, 0)`** — re-arms the threshold interrupt `_AudDmaIrq` switched off on its first hit (source: condition only); the sub-16-byte remainder is carried on every call (source: only in the overflow case, a source bug); an "l:%u, t:%u, size %u" error is printed on overflow |
| `DrvAudReadPcm` | **DIFF**: the extra `HalAudDmaGetRawLevelCnt` above |
| `DrvAudWritePcm` | same |
| `DrvAudStopDma` (+ inlined `_AudDmaStopChannel`) | interrupts off, `HalAudDmaEnable(dma, 0)`, `HalAudDmaReset` (with its 2 × 2 ms sleeps, §5.12), PTS cleared |
| `DrvAudIsXrun` | **DIFF**: state check only (above) |
| `DrvAudCloseDma` | `free_irq`, private data freed, state INIT |
| `DrvAudEnableHdmi(b, dma)` | **DIFF** (signature): passes the reader DMA to `HalAud2Hdmi` (§5.14) |
| other `DrvAud*` (`ConfigI2s`, `EnableI2s`, `SetMux`, `MonoEnable`, `EnableAtop`, `AdcSetMux`, `ConfigDigMicParam`, `ConfigAecParam`, `DigMicEnable`, `SetDigMicGain`, `SetAdcGain`, `SetMicAmpGain`, `DpgaSetGain*`, `DmaSineGen`, `EnableAmp`, `GetBufCurrDataLen`, `GetStartPts`, `Pause/ResumeDma`, `_DrvAudAoAmpEnable`, `_AudDmaWr/RdCopy*`) | same HAL calls, arguments and order as the source |

[I] Net effect: the binary drives capture and playback by the threshold interrupt (one wake-up per period), falls back
to the full / empty (boundary) interrupt after the first threshold hit, and re-arms the threshold interrupt from the copy
path; the source's polling design (u-boot) has neither.

### 5.6 Linux glue: device tree, IRQ, amplifier GPIO, `/proc/audio` [R unless tagged]
<!-- ev:audio.5.6 -->
The source of these functions is the tree's `drv/src/linux/` (`drv_audio_api_linux.c`, `audio_proc.c`,
`drv_audio_export.c`); none of them writes an AUDIO register.

**DT node `sound`** (compatible `sstar,audio`) as read by `DrvAudApiDtsInit`:

| property | value on this camera | use |
|---|---|---|
| `interrupts` | `<0 0x4a 4>` | IRQ (§5.2) |
| `clocks` | `<CLK_bach>` | `of_clk_get` + prepare + enable |
| `amp-gpio` | `<23 1 24 1>` | {pad, active level} of amplifier channel 0 and 1 (`_gaAmpGpio[4]`) |
| `digmic-padmux`, `i2s-trx-shared-padmux`, `i2s-tx-padmux`, `i2s-rx-padmux`, `i2smck-padmux` | 5, 0, 6, 7, 7 | HAL padmux modes, used only when the padmux driver is inactive (below) |
| `keep-i2s-clk`, `i2s-pcm` | 0, 0 | whether the I2S MCK pad mode is kept when RX and TX close (§5.10); I2S PCM mode (stored nowhere, §5.10) |
| `i2s-rx-tdm-ws-pgm`, `i2s-rx-tdm-ws-width`, `i2s-rx-tdm-ws-inv`, `i2s-rx-tdm-ch-swap` | 0, 0, 0, `<0 0>` | TDM RX details (§5.11) |
| the same four for TX, `i2s-tx-tdm-active-slot` | 0, 0, 0, `<0 0>`, 3 | TDM TX details (§5.11) |
| keep-ADC-power / keep-DAC-power keys | not given | read as in the source |
| `dmic-bck-mode-8k/16k/32k/48k` (binary only) | 7 / 12 / 15 / 16 | DMIC `FS_SEL` code per rate (§5.14) |
| `adc23-mode` (binary only) | 1 | `EN_SW_MIC2_L/R` of the ATOP (§5.9) |

- `DrvAudApiDtsInit` — **DIFF**: the source part is the same (IRQ, `amp-gpio`, padmux choice, `keep-i2s-clk`, `i2s-pcm`,
  the TDM keys, keep-ADC/DAC-power keys, clock); added are the four `dmic-bck-mode-*` keys, passed to
  `HalAudDigMicBckMode` only when all four are present, and `adc23-mode` → `HalAudAtopSetAdcMode`. The DT key-name table
  (`card.dts`) is therefore 0x5C bytes, longer than the source's 19-word `struct audio_dts`: after +0x44 (keep-DAC-power)
  the binary has the four `dmic-bck-mode-*` names at +0x48…+0x54 and `adc23-mode` at +0x58, where the source has
  `template_fp_list` (+0x48).
- **Padmux choice.** When the padmux driver is active, `DrvAudApiDtsInit` takes the HAL padmux modes from
  `HalAudOsApiPreSetAllPadmux` (§5.7); otherwise from the `sound` node's `*-padmux` values. `mdrv_padmux_active()`
  returns `_pPadInfo != 0` (kernel `padmux.c` decompile; the literal at 0xC01FF180 = 0xC047C7B4 = `_pPadInfo` in
  the kernel's symbol table), and `_pPadInfo` is allocated by `_mdrv_padmux_dts` from the DT `padmux` node's `schematic`, which is
  present with status "ok" in `board.dts` (`kernel-platform.md` §6.2). [I] The padmux driver probed at boot, so it is
  active on this camera — the stock boot log has no padmux line, so this is not
  confirmed; the camera registers `0x103CC4` / `0x103CF2` (`riu_r 103c 62` / `riu_r 103c 79` [I]) or the amplifier
  pad's GPIO state after an `MHAL_AUDIO_Init` would show it. Everything below that depends on it says so.
- **Amplifier.** `amp-gpio` gives pad 23 (ch 0) and pad 24 (ch 1), active level 1. With the padmux driver active, ch 0's
  pad is replaced by `HalAudOsApiChangeAmpGpio` with `mdrv_padmux_getpad(AIO_AMP_PWR 0xF0000)` = the first schematic
  triple with that use = **pad 0x15 (21)** (the schematic lists pads 0x15, 0x16, 0x17 with use 0xF0000, mode 0;
  `kernel-platform.md` §6.3); ch 1 keeps pad 24. Both are requested with `CamGpioRequest`.
  - `DrvAudApiAoAmpEnable(b, ch)` — **DIFF**: `camdriver_gpio_direction_output(0, pad, level)` with level = the DT active
    level of the channel (`amp-gpio` word 1 / 3) on enable and its inverse on disable. Source: `MDrv_GPIO_Pull_High` /
    `Pull_Low`, ignoring the level word. `MHAL_AUDIO_AmpEnable` and the HAL's amplifier callback (§5.9) end here.
  - `DrvAudApiAoAmpCtrlInit` — **DIFF**: only when the padmux driver is **not** active, `MDrv_GPIO_PadVal_Set(pad, 0)`
    (GPIO mode) for each known amplifier pad, and no `MDrv_GPIO_Pad_Oen`. Source: always GPIO mode + output enable. On
    this camera [I: padmux active] it does nothing.
  - Whether pads 21 / 24 drive an amplifier, and whether a microphone or a line out is wired at all, is not known from
    static data (board hardware).
- **IRQ** (same as the source): `DrvAudApiIrqRequest` / `IrqFree` / `IrqInit` / `IrqSetDmaStopCb` —
  `request_threaded_irq(irq, _AudDmaIrq, NULL, IRQF_SHARED (0x80), name, dev)`.
- **`/proc/audio/`** (same as the source): `AudioProcInit` / `DeInit`, `AudioAi/AoProcShow/Open/Write` — read-only `ai`
  and `ao` (mode 0444), contents from `gAudProInfoAiList` / `gAudProInfoAoList`; Show prints per device enable, rate,
  channels and width, the rate from `HalAudDmaGetRate` (always "not supported", so 0); Write only parses an id and
  rejects id ≥ 9 (AI) — it changes nothing.

### 5.7 OS glue `hal_audio_os_api_linux.c`: padmux modes and the amplifier pad [R]
Same as the source; for these two functions the identity also holds call list by call list and instruction by
instruction against the source compiled with use-code stubs for `mdrv_padmux.h` (the two prototypes) and `mdrv_puse.h`
(`-O2 -mthumb -march=armv7-a`); the only differences are how
the five use codes are loaded (literal pool in the vendor build) and, in `HalAudOsApiChangeAmpGpio`, prologue placement.
- `HalAudOsApiPreSetAllPadmux`: asks the padmux driver (`mdrv_padmux_getmode`) for the modes of the pad uses
  `I2S_TX_BCK` (0x110004), `I2S_RX_BCK` (0x110001), `DMIC_CLK` (0x90002), `AIO_MCK` (0xF0001), `I2S_BCK` (0xA0001), in
  that order, each followed by the matching `HalAudPriSetPadMux{I2sTdmTx, I2sTdmRx, Dmic, Mck, I2sMode}`, and converts
  them to the HAL mode numbers: TX 0x105–0x10E → 1–10, RX 0xFA–0x104 → 1–11, DMIC 0x11F–0x128 → 1–10, MCK 0xF3–0xF9 →
  1–7, RXTX 0x10F–0x115 → 1–7, anything else → 0. Each switch is compiled as `mode − base ≤ n ? mode − (base − 1) : 0`
  (bases and counts TX 0x105 / 10, RX 0xFA / 11, DMIC 0x11F / 10, MCK 0xF3 / 7, RXTX 0x10F / 7; compiler range-check
  biases −0x105 for TX, −0x11F for DMIC). These ranges are the u-boot `gpio/mercury6/padmux.h` enum values
  (`PINMUX_FOR_I2S_TX_MODE_1` = 0x105 … `PINMUX_FOR_I2S_RXTX_MODE_7` = 0x115) [S].
- Use codes: `MDRV_PUSE_DMIC_CLK` 0x00090002, `MDRV_PUSE_I2S_BCK` 0x000A0001, `MDRV_PUSE_AIO_AMP_PWR` 0x000F0000,
  `MDRV_PUSE_AIO_MCK` 0x000F0001 match the I6E kernel header
  `dl/src-kernel/openipc-infinity6e/drivers/sstar/include/mdrv_puse.h` [S]. `MDRV_PUSE_I2S_TX_BCK` 0x00110004 /
  `MDRV_PUSE_I2S_RX_BCK` 0x00110001 are in no available header (the I6E header has no TDM uses; the board DT's padmux
  schematic has no 0x0011xxxx use); these two values are read from the binary only.
- `mdrv_padmux_getmode` returns the mode of the first schematic triple with the asked pad use, else 0xFFFF. This
  camera's schematic has no entry for any of the five uses (list of the triples' third words), so all five HAL modes are
  **0** [R; I: padmux driver active].
- `HalAudPriSetPadMux{I2sTdmTx, I2sTdmRx, Dmic, Mck, I2sMode}` (in `hal_audio.c`): store the mode (`.data+0x10` words
  +8, +0xC, +4, +0x10, +0x14).
- `HalAudOsApiChangeAmpGpio(pad*, ch)`: `ch == 0 → *pad = mdrv_padmux_getpad(0xF0000)`, return 1; other channels
  return 1 unchanged.

### 5.8 Init: `HalAudMainInit` (called from `DrvAudInit`) [R unless tagged]
Same call order as the source: `_HalAudSysInit` (including `_HalAudAtopInit`: 30 ATOP byte writes,
`_HalAudAtopSwitch(LINEIN / MICIN / DAC, FALSE)`, `HalAudAtopSetAdcMux(LINEIN)`), `_HalAudDmicRegInit`,
`_HalAudDmaFirstInit`, `_HalAudDmaWrMchInit`, `_HalAudDpgaInit` (7 × `HalAudDpgaCtrl(i, 1, 1, 1)`), `_HalAudPadMuxInit`
(`HalAudSetPadMux(DMIC / I2S_TX / I2S_RX, FALSE)`), `_HalAudIntEnable`. Every byte write below is in the source with the
same value and order; the differences at Init come only from `_HalAudAtopSwitch` (§5.9): the DAC power-down now runs the
ramp with a 150 ms sleep, so `MHAL_AUDIO_Init` takes at least 150 ms.

Register writes in order, extracted from the disassembly (100 byte writes + 21 masked writes); written as bank:byte=value:
- ATOP 0x1034: `00=14 01=0a 02=30 03=00 04=80 05..09=00 0a=77 0b..10=00 11=30 24=00 25=00 12=04 13=02 14=10 15..17=00
  18=77 19=00 1a=00 1b=30`.
- Then the three `_HalAudAtopSwitch(path, FALSE)` calls (`.text` 0x2E44, Ghidra 0x2e6c) — power-**down** of LINEIN, MICIN
  and the DAC, with constant arguments:
  1. LINEIN off: `_HalAudAtopAdc(0)` — ATOP `0x06` mask 0x2D ← 0x25 and the two `0x04` fields ← `adc23-mode` & 3 (DT 1 →
     0x4000, 0x0400).
  2. MICIN off: the same, then `0x06` mask 0x180 ← 0x180.
  3. DAC off: the amplifier callbacks drive both amplifier pads to their inactive level (on this camera pads 21 and 24
     to 0 with `camdriver_gpio_direction_output` [I: padmux driver active], §5.6), then the ramp down and the DAC
     power-down of §5.9 with its **150 ms** sleep.

  `HalAudAtopSetAdcMux(LINEIN)` then writes ATOP `0x0A` 0xFF ← 0.
- BACH 0x1502 / 0x1503 / 0x1504: `1502:00=00 01=40`, then `1502:00=ff 01=89 02=88 03=ff 04=03 05=00 06=b4 07=19 08=00
  09=f0 0a=00 0b=80 0c=9a 0d=c0 0e=5a 0f=55 12=09 13=02 14=00 15=00 16=7d 17=00`, `1502:3a/3b = 1d 02` then `00 00` (a
  pulse), `1503:1c=03 1d=00 2c=03 2d=c0 1d=00`, `1502:26=00 27=d4 48=07 49=00 50=07`, `1503:78=1c 79=11`, `1502:c8=fd
  c9=09 cc=00 cd=f0`, `1504:0c/0d = 00 80` then `00 00`, `1504:02=21 03=00 06=00 07=00 08=0a 09=00 0c=1f 0d=01 20=00
  21=00 22=01 23=00 24=00 25=80 26=01 27=00 28=00 29=80 40=01 41=00`, `1503:9e=00 9f=00`.
- Masked word writes (bank, reg, mask, value): DMA writer / reader set-up `(0,0x88,ffff,0) (0,0x98,ffff,0) (0,0x80,1,1)
  (0,0x80,1,0) (0,0x80,0x14,0x14) (0,0xa8,ffff,0) (0,0xb8,ffff,0) (0,0xa0,1,1) (0,0xa0,1,0) (0,0xa0,0x14,0x14)` (reset
  pulse and flags of the two DMA register groups at 0x80 and 0xA0: `BACH_DMA1_CTRL_0/4/12` 0x80 / 0x88 / 0x98,
  `BACH_DMA2_CTRL_0` 0xA0 [S names]), `(1,0x96,0x20,0) (1,0x96,0x10,0) (1,0x96,1,1) (1,0x98,0x20,0) (1,0x98,0x10,0)
  (1,0x98,1,1)` (`DMA_SRC_CFG00/01`, §5.12), the seven `HalAudDpgaCtrl(i, 1, 1, 1)` (§5.13), the padmux clears (below),
  and `(1,0x0e,2,2)` (bank 0x1503 `BACH_INT_EN` [S name]).

Per-field meanings of these defaults are those of `hal_audio_reg.h` and are not repeated here.

**Padmux at Init on this camera.** `_HalAudPadMuxInit` only clears mode bits (`HalAudSetPadMux(…, FALSE)`); it never sets
a pad mode. With all five HAL padmux modes 0 (§5.7) it reads `0x103CF2` and `0x103CC4` and writes the same values back
(clearing mode 0 changes no bit) — no padmux change at Init; later Opens of DMIC / I2S devices would not set any audio pad
mode either [R; I: padmux driver active]. [I] With the padmux driver inactive the DT values would apply: Init would clear
`0x103CF2` bits 0x5 and `0x103CC4` bits 0x0600, 0x0007, 0x0070, and Open would set them.

All of this happens only when a process makes `MHAL_AUDIO_Init` run (§1), which the shipped start line never does.

### 5.9 Analog front end (ATOP, bank 0x1034) [R unless tagged]
Register offsets are ATOP byte offsets; names from `hal_audio_reg.h` [S].

| function | result | binary (and the source where it differs) |
|---|---|---|
| `_HalAudAtopAdc(bEnable)` (`.text` 0x2CD0) | **DIFF** | enable: `CTRL03 (0x06)` and `CTRL0b (0x16)` mask 0x2D ← 0 (ADC0 and ADC1 on) — same. Disable: only `CTRL03 (0x06)` mask 0x2D ← 0x25 (`PD_ADC0 | PD_INMUX=1 | PD_LDO_ADC`); the source also writes `CTRL0b (0x16)` ← 0x25 (ADC1 off), so in the binary **ADC1 stays powered once opened**. Both: `CTRL02 (0x04)` `EN_SW_MIC2_L` [15:14] and `EN_SW_MIC2_R` [11:10] — source constant 1 each; binary = `adc23-mode` & 3 (the byte stored by `HalAudAtopSetAdcMode`) for both fields. With this DT (`adc23-mode = 1`) the written values equal the source's (0x4000, 0x0400) |
| `_HalAudAtopSwitch(path, bEnable)` (`.text` 0x2E44) LINEIN | same | `_HalAudAtopAdc` + 50 ms sleep, state flags |
| — MICIN | **DIFF** (disable) | enable same (`_HalAudAtopAdc(1)`, pre-amp on: `0x06` and `0x16` mask 0x180 ← 0, 50 ms). Disable: `_HalAudAtopAdc(0)` and only `0x06` mask 0x180 ← 0x180 (`PD_MIC_STG1_L/R`); the source also powers down stage 2 (`0x16` mask 0x180 ← 0x180, `PD_MIC_STG2_L/R`), so in the binary **mic stage 2 stays powered** |
| — DAC LINEOUT enable | **DIFF** | binary: 0x1503 regs **0x7A** and **0x7C** bits [15:13] ← 7, `CTRL03 (0x06)` mask 0x652 ← 0 (`PD_BIAS_DAC, PD_L0_DAC, PD_LDO_DAC, PD_R0_DAC, PD_REF_DAC` cleared = DAC on), 0x7A / 0x7C [15:13] ← 5, **150 ms** sleep, 0x7A / 0x7C [15:13] ← 0, state, amplifier callback (1, ch 0) and (1, ch 1). Source: `CTRL03` mask 0x652 ← 0, 10 ms, `CTRL0e (0x1C)` mask 0x3800 ← 0 (`DAC_SOFT_FUNC_EN, DAC_PD_R_EN, DAC_PD_L_EN` cleared), amplifier callbacks. The binary never writes `CTRL0e` here |
| — DAC LINEOUT disable | **DIFF** | binary: amplifier callback (0, ch 0) and (0, ch 1), 0x7A / 0x7C [15:13] ← 4, then ← 6, **150 ms** sleep, `CTRL03` mask 0x652 ← 0x652 (DAC off). Source: amplifier callbacks, `CTRL03` mask 0x652 ← 0x652, no ramp, no sleep |
| `HalAudAtopOpen` | same | reference on (`CTRL03` mask 0x1800 (`PD_VI | PD_VREF`) ← 0) when neither ADC nor DAC is active, LINEIN / MICIN exclusion, `_HalAudAtopSwitch(path, 1)` |
| `HalAudAtopClose` | **DIFF** | after `_HalAudAtopSwitch(path, 0)`, when nothing is active: binary `CTRL03` mask 0x1800 ← **0x1800** (reference powered down); the source's `_HalAudAtopEnableRef(FALSE)` writes 0 (its `bEnable` is ignored), i.e. the source never powers the reference down |
| `HalAudAtopMicAmpGain(sel, nSel, ch)` | same | `CTRL08 (0x10)` / `CTRL0d (0x1A)`, masks 0x100C (L: gain [3:2] + ext bit 12) / 0x2030 (R: [5:4] + bit 13); nSel < 2 → gain nSel + ext bit, 2–5 → nSel − 2; > 5 rejected |
| `HalAudAtopSetAdcGain(sel, nSel, ch)` | same | `MICGAIN_INMUX` bit (1 = L, 2 = R) of `0x10` / `0x1A` set for nSel 8–9 (then nSel − 2), cleared otherwise; gain into `CTRL06 (0x0C)` masks 0x7 / 0x70 / 0x1C00 / 0xE000 with the source's 2 → 0, 0 / 1 → 1 / 2 mapping |
| `HalAudAtopSetAdcMux(sel)` | same | `CTRL05 (0x0A)` 0xFF ← 0 (LINEIN) / 0x77 (MICIN), `CTRL0c (0x18)` ← 0x77 (MICIN2), both (4CH). The source's `_gnAdcSelect` store is gone (a write-only variable) |
| `HalAudAtopSwap(b)` | same | `STAT03 (0x24)` bits 0 and 6 |
| `HalAudKeepAdcPowerOn`, `HalAudKeepDacPowerOn` | same | `return TRUE` in both |
| `HalAudAtopSetAdcMode(u8)` | binary only | stores the DT `adc23-mode` byte in the HAL state (`.data+0x10` struct, byte 0), read by `_HalAudAtopAdc` |

- The amplifier callback is the one `DrvAudInit` installs with `HalAudSetAoAmpCb` (`_DrvAudAoAmpEnable` →
  `DrvAudApiAoAmpEnable`, §5.6).
- [I] The 0x7A / 0x7C pattern — 7 → 5 → (150 ms) → 0 on power-up and 4 → 6 → (150 ms) → power-down, on two registers —
  is a DAC output ramp ("depop") that replaces the source's `DAC_SOFT_FUNC` bits; the 150 ms waits are added to every
  DAC open and close and to `MHAL_AUDIO_Init` (the DAC power-down in `_HalAudAtopInit`). The bits are named in no header
  or datasheet available (§5.2).

### 5.10 Routing muxes, padmux, SRC rate, reader swap [R unless tagged]
| function | result | binary |
|---|---|---|
| `HalAudSetMux(mux, choice)` | same | all 12 cases: MMC1 / MMC2 → 0x1502 `MUX0_SEL (0x06)` bits 5 / 4; DMARD1 / DMARD2 → 0x1503 `DMA_SRC_CFG02 (0x9A)` bits 3 / 2; DMAWR1 → 0x1503 `MUX1_SEL (0x1C)` bit 1; DMAWR1_MCH / DMAWR2_MCH → 0x1503 `DMA_SRC_CFG00/01 (0x96 / 0x98)` bit 1 (`WR_NEW_MODE`); I2STDM_RX → 0x1503 `I2S_CFG00 (0x78)` bit 0; I2S_TX → 0x1503 `AU_SYS_CTRL1 (0x2C)` [3:2]; I2S_CFG_00 → `0x78` [2:1]; SDM_DINL / SDM_DINR → 0x1502 `MUX4_SEL (0xCC)` [10:8] / [6:4] |
| `HalAudSetPadMux(pad, bEnable)` | **DIFF** (one condition) | when the saved `i2s-trx-shared-padmux` mode is non-zero, requests for I2S_TX (1) and I2S_RX (2) are handled as I2S_RXTX (4): `0x103CC4` bits [15:12] ← mode, both RX and TX active flags set / cleared together. The source has no redirection (TX and RX always use their own fields). Otherwise as the source: DMIC field at bit 0 of `0x103CF2`; in `0x103CC4` TDM RX at bit 4, TDM TX at bit 8, RXTX at bit 12, MCK at bit 0 (read-modify-write through `HalBachReadReg2Byte` / `WriteReg2Byte`); MCK only cleared when keep-clk is off and neither RX nor TX is active. The source's trailing `_HalAudI2sPcmMode()` has an `#if 0` body; nothing in the binary. With this DT (`i2s-trx-shared-padmux = 0`, and mode 0 from the padmux driver) the redirection is not taken |
| `HalAudSrcSetRate(rate)` | same | 0x1502 `SR0_SEL (0x02)` `CIC_1_SEL` [15:14] ← 0 / 1 / 2 / 3 for 8 / 16 / 32 / 48 kHz |
| `HalAudSetAoAmpCb`, `HalAudI2sSetPcmMode` | same | store a pointer / nothing (the source's `_gbI2sPcmMode` is write-only) |
| `HalAudDmaRdSwap(sel)` | binary only | 0x1502 reg 0x0E (`MIX1_SEL`, `BACH_MIX1_SEL` in the header [S]): sel 0 → bits [15:14] = 01 and [13:12] = 01; sel ≠ 0 → [15:14] = 10, [13:12] = 10. Reached from `MHAL_AUDIO_DmaRdSwap` → `DrvAudAoSwap`; exported, imported by nothing on the camera. [I: swaps which reader DMA feeds which mixer input, the switch behind AO devices on READER1 vs READER2] |

### 5.11 I2S / TDM interface (bank 0x1503) [R]
All the same as the source. The binary keeps the source's statics in one struct at `gpRootAudioDir+0xc`, with per-I2S
config records of 0x24 bytes at +0x30.

| function | registers (bank 0x1503) |
|---|---|
| `HalAudI2sSave{Rx,Tx}TdmWsPgm/WsWidth/WsInv/ChSwap`, `SaveTxTdmActiveSlot`, `HalAudI2sSetKeepClk` | state only (ChSwap: bit 0 = swap 0↔2, bit 1 = swap 0↔4) |
| `HalAudI2sSetTdmDetails(i2s)` | RX: `TDM_CFG00 (0x60)` bit 7 `WS_FMT`, bit 5 `WS_INV`; `TDM_CFG01 (0x62)` [9:5] WS width; `TDM_CFG02 (0x64)` [1:0] ch swap. TX: `TDM_CFG05 (0x6A)` bits 7 / 5; `TDM_CFG06 (0x6C)` [12:8] WS width, [14:13] ch swap, [7:0] active slots |
| `HalAudI2sSetRate(i2s, rate)` | NF value = (2²² × 384 / 8) / (rate_kHz × width × ch / 2) × 1000 (binary constant 0xC000000), 0 for an unsupported rate / width; RX: `I2S_CFG00 (0x78)` [15:14] ← 3 (div 8), `NF_SYNTH_RX_BCK01/02 (0x84 / 0x86)` ← NF lo / hi; TX: `0x78` [11:10] ← 3, `0x8A / 0x8C` |
| `HalAudI2sSetTdmMode` / `SetMsMode` / `SetFmt` / `SetWidth` | `0x60` (RX) / `0x6A` (TX): bit 13 TDM, bit 14 master, bit 6 left-justified, bit 9 32-bit |
| `HalAudI2sSetChannel(i2s, n)` | n = 2: state only; 4 / 8: bit 8 (`CHLEN`) of `0x60` / `0x6A` ← 0 / 1 |
| `HalAudI2sSetWireMode(i2s, m)` | TX only: `0x6A` bit 12 ← 1 (4-wire) / 0 (6-wire); 6-wire refused when the shared RX/TX padmux mode is 1; RX stores 0xFF |
| `HalAudI2sSetTdmSlotConfig(i2s, mask, map)` | TX: `0x6C` [7:0] ← mask, [14:13] ← map |
| `HalAudI2sEnableMck(i2s, b)` | `NF_SYNTH_MCK00 (0x8E)` mask 0x15 ← 0x11, 0x15, 0x11 (trigger pulse) on enable when an MCK is configured; ← 0 when both the RX and the TX MCK user are off |
| `HalAudI2sSetMck(i2s, mck)` | 48 MHz: `0x78` [7:6] ← 0; 24 MHz: ← 1; else ← 2 and, for 12.288 / 16.384 / 18.432 / 24.576 MHz, `MCK02 (0x92)` [7:0] ← NF hi, `MCK01 (0x90)` ← NF lo, `0x92` [15:8] ← duty; table in `.rodata` = NF 0x1F4000 / 0x177000 / 0x14D555 / 0xFA000, duty 15 / 11 / 10 / 7 (the source's values) |
| `HalAudI2sEnable(i2s, b)` | RX `NF_SYNTH_RX_BCK00 (0x82)`, TX `NF_SYNTH_TX_BCK00 (0x88)`: mask 0x15 ← 0x11, 0x15, 0x11 on enable, ← 0 on disable |

### 5.12 DMA engines (bank 0x1502; DMA1 group regs 0x80–0x9E, DMA2 group 0xA0–0xBE) [R unless tagged]
Engine → register group: WRITER1 = DMA1 writer side (`CTRL_8..15`, 0x90–0x9E), READER1 = DMA1 reader side
(`CTRL_0..7`, 0x80–0x8E), WRITER2 / READER2 = the same in DMA2 (+0x20).

| function | result | binary (and the source where it differs) |
|---|---|---|
| `HalAudDmaSetRate(dma, rate)` | same | WRITER1: `SR0_SEL (0x02)` `WRITER_SEL` [9:8] and `CIC_3_SEL` [11:10] ← 0–3 for 8 / 16 / 32 / 48 kHz, then `HalAudDpgaSetGainOffset(DEC1, 3 at 48 kHz else 0, ch 0 and 1)`; WRITER2: `SR0_SEL_AU2 (0xCA)` `CIC_3_SEL_AU2` [11:10] and `CODEC_SEL_AU2` [9:8], same DEC1 offset; READER1: `0x02` `SRC1_SEL` [7:4] ← 0–8 for 8…48 kHz (all nine rates); READER2: `SRC2_SEL` [3:0] |
| `HalAudDmaReset(dma)` (source `_HalAudDmaReInit`) | **DIFF** (timing) | same registers and order (`RD/WR_TRIG` bit 13 ← 0, `RD/WR_INIT` bit 14 ← 1, ← 0 on `CTRL_1` / `CTRL_9` = 0x82 / 0x92 / 0xA2 / 0xB2), but the binary sleeps **2 ms** (`CamOsMsSleep(2)`) at entry and again between INIT set and INIT clear; the source has no delay |
| `HalAudDmaEnable(dma, b)` | **DIFF** (timing) | readers `CTRL_0 (0x80 / 0xA0)` bit 1 (`REG_ENABLE`, prefetch), then 10 µs, then `CTRL_1` bit 15 (`RD_ENABLE`); writers `CTRL_9 (0x92 / 0xB2)` bit 15. On reader disable the binary also waits 10 µs between clearing `RD_ENABLE` and `REG_ENABLE` (source: no delay) |
| `HalAudDmaPause` / `Resume` | same | `HalAudDmaEnable(dma, 0 / 1)` |
| `HalAudDmaIntEnable` / `GetInt` / `ClearInt` | same | `CTRL_0` of the group: writers bits 14 (overrun) and 11 (full), readers bits 13 (underrun) and 10 (empty), read-modify-write; clear = pulse of bit 9 (`WR_FULL_FLAG_CLR`) / bit 8 (`RD_EMPTY_FLAG_CLR`) |
| `_HalAudDmaGetRawMiuUnitLevelCnt` (`.text` 0x2D28) | same | latch bit 12 (`LEVEL_CNT_MASK`) of `CTRL_9` / `CTRL_1` ← 1, read `CTRL_15 (0x9E / 0xBE)` / `CTRL_7 (0x8E / 0xAE)`, ← 0 |
| `HalAudDmaGetLevelCnt(dma)` | **DIFF** | writers: (raw − 4, floor 0) × 16 bytes — same (`DMA_LOCALBUF_LINE` = 4). Readers: source raw × 16; binary **0 when raw ≤ 4**, else raw × 16 (`.text` 0x3ff4: `cmp r0,#4; it ls; movls r0,#0`). So a reader ring holding ≤ 64 bytes reports empty |
| `HalAudDmaGetRawLevelCnt` | same | raw × 16 |
| `HalAudDmaTrigLevelCnt(dma, bytes)` | same | size register (`CTRL_12` 0x98 / 0xB8 writers, `CTRL_4` 0x88 / 0xA8 readers) ← bytes / 16 (16 bits), then toggle `WR/RD_TRIG` (bit 13 of `CTRL_9` / `CTRL_1`) |
| `HalAudDmaSetPhyAddr(dma, offset, size)` | same | size ≤ 0xFFFFF, both multiples of 16; `DMA_TEST_CTRL7 (0xEE)` `DMA1_MIU_SEL` [5:4] / `DMA2_MIU_SEL` [7:6] ← 1 when offset ≥ 0x8000_0000 else 0; base lo [11:0] of offset / 16 into `CTRL_9` / `CTRL_1`, the high 15 bits into `CTRL_10` / `CTRL_2`, size / 16 into `CTRL_11` / `CTRL_3` |
| `HalAudDmaWrSetThreshold` / `RdSetThreshold` | same | `CTRL_13 (0x9A / 0xBA)` / `CTRL_6 (0x8C / 0xAC)` ← bytes / 16 |
| `HalAudDmaGetFlags` | same | `CTRL_8 (0x90 / 0xB0)`: writers bit 1 overrun, 5 full, 6 local full; readers bit 2 underrun, 4 empty, 7 local empty |
| `HalAudDmaWrConfigMch(set, nCh, clkRef, sel[4])` | same | 0x1503 `DMA_SRC_CFG00 (0x96)` (W1) / `CFG01 (0x98)` (W2): ch23 [15:12], ch01 [11:8], valid / clock ref [7:6], channel mode [3:2] (1 / 2 / 4 / 8 ch → 0–3); ch67 / ch45 in `CFG03 (0x9C)` [7:4] / [3:0] (W1) or [15:12] / [11:8] (W2). Slot codes (`_HalAudDmaGetMchSelConfigValue`, `.text` 0x2DE0): DMIC01 0, DMIC23 1, AMIC01 4, AMIC23 5, I2S RX01 / 23 / 45 / 67 8–11, SRC 12, NULL 15 |
| `HalAudDmaMonoEnable` | same | `0xEE`: W1 bit 14, W2 bit 10, R1 bits 15 + 13, R2 bits 11 + 9 |
| `HalAudDmaGetRate` | same | prints "have NOT support yet" |

[I] The reader level-count floor makes `DrvAudWritePcm`'s free-space computation see an almost-empty reader ring as
empty; the 2 ms sleeps make every `HalAudDmaReset` (called from `DrvAudPrepareDma` and `DrvAudStopDma`, §5.5) take at
least 4 ms of sleep.

### 5.13 Digital gain stages (DPGA) [R]
All the same as the source. DPGA → config register pair CFG1 / CFG2 (bank 0x1502 unless noted; a table in `.rodata` of
the binary, a switch in the source): MMC1 0x40 / 0x42, MMC2 0x44 / 0x46, ADC 0x48 / 0x4A, AEC1 0x50 / 0x52, DEC1 0x60 /
0x62, DEC2 0x64 / 0x66, TDM bank 0x1503 0x70 / 0x72.

| function | registers |
|---|---|
| `HalAudDpgaCtrl(dpga, en, mute, fade)` | CFG1 bits 0 (`DPGA_EN`), 2 (`MUTE_2_ZERO`), 1 (`FADING_EN`), then `STEP` [5:3] ← 6 |
| `HalAudDpgaSetGain(dpga, dB, ch)` | dB clamped to ≤ 34; dB ≤ −64 → index 0x7F, else index = −2 × dB (8 bits, so a positive dB wraps to 0x100 − 2·dB); CFG2 [7:0] = ch 0 (L), [15:8] = ch 1 (R), read-modify-write; ADC accepts only ch 0, AEC1 only ch 1 (both write the L byte) |
| `HalAudDpgaSetGainOffset(dpga, v, ch)` | v saturated to −16…15; CFG1 [10:6] (L) / [15:11] (R), read-modify-write with mask 0xFFC0 (0x07C0 for ADC / AEC1). A negative v is shifted as an 8-bit value, so it spills into the R field (in the source as well) |
| `HalAudDpgaSetGainFading(dpga, n, ch)` | n = 0 → fading off, else `FADING_EN` + `STEP` ← n − 1, mask 0x3A of CFG1; ADC / AEC1 / DEC1 / DEC2 refused ("Not Support AI Device Fading") |

### 5.14 DMIC, sine generator, HDMI, I2S clock reference (bank 0x1504 for DMIC) [R unless tagged]
| function | result | binary (and the source where it differs) |
|---|---|---|
| `HalAudDigMicBckMode(u8 m8k, m16k, m32k, m48k)` | binary only | stores the DT values `dmic-bck-mode-8k/16k/32k/48k` (bytes +0x18…+0x1B of the `.data+0x10` HAL state); this camera: 7 / 12 / 15 / 16 |
| `HalAudDigMicEnable(b)` | same | 0x1504 `VREC_CTRL02 (0x04)` bit 1 (`SEL_RECORD_SRC`) ← 0, whatever `b` |
| `HalAudDigMicSetChannel(n)` | **DIFF** | 0x1504 `VREC_CTRL01 (0x02)` `CHN_MODE_DMIC` [5:4]: 1 ch → 0, 2 → 1, 4 → 2 in both; **8 ch → 2** in the binary (source 3) |
| `HalAudDigMicSetRate(rate)` | **DIFF** | the binary first checks the four `dmic-bck-mode-*` values (8 k ∈ 1–7, 16 k ∈ 8–14, 32 k = 15, 48 k = 16) and returns FALSE with "Bck mode … Out of range" before touching anything if one is outside; then stores the rate, re-applies the four saved DMIC gains (same), and writes 0x1504 `VREC_CTRL04 (0x08)` `FS_SEL` [4:0] ← **the DT value for the rate** (source: constants 7 / 12 / 15 / 16), `CIC_CTRL01/02 (0x42 / 0x44)` bit 0 `DEC_FILTER` ← 1 for 8 / 16 kHz (3-stage), 0 for 32 / 48 kHz — same. With this DT the written values equal the source's |
| `HalAudDigMicSetGain(g, ch)` | same | g ≤ 4 (stored per ch); +1 at 32 / 48 kHz; `CIC_CTRL01 (0x42)` [10:8] ch 0, [14:12] ch 1; `CIC_CTRL02 (0x44)` the same for ch 2 / 3 |
| `HalAudDmaSineGen(dma, en, freq, gain)` | same | 0x1502 `DMA_TEST_CTRL5 (0xEA)` for WRITER1 / READER1, `CTRL6 (0xEC)` for WRITER2 / READER2: bit 15 enable, bits 14 + 13 ← 1, bit 12 (`RD_WR`) written with value 1 for writers / 0 for readers but under mask 0x1000, so 0 in both (in the source as well), gain [7:4], freq [3:0] |
| `HalAud2Hdmi(b, dma)` | **DIFF** (signature) | a second argument, the reader DMA: `AU2HDMI_CTRL (0xF0)` bit 15 ← 1, `AU2HDMI_MUX_SEL` [13:12] ← **0 for READER1, 1 for READER2** (any other dma: error, nothing written, also when disabling), `AU2HDMI_REF_FS (0xF2)` ← 0; disable: bit 15 ← 0. Source: `HalAud2Hdmi(b)`, mux always 0 (READER1). So AO device 4 (READER1) and 5 (READER2) both reach HDMI |
| `HalAudCfgI2sIfPrivateCb(pri)` (source `_HalAudI2sSetClkRef`) | same | RX only: 0x1503 `DMA_SRC_CFG02 (0x9A)` [1:0] ← clock ref 0–3 (DMIC, ADC, I2S TDM RX, SRC); TX returns FALSE |

[I] `dmic-bck-mode` therefore is the DMIC `FS_SEL` code per rate, with the source's constants as the only accepted values
for 32 and 48 kHz and ranges for 8 and 16 kHz.
