# audio-algorithms — libSRC, libSSL, libAEC, libAPC, libAED, libBF and the mechanics of the nine algorithm libraries
<!-- ev:audio-algorithms -->

SigmaStar's audio algorithm libraries of the Tiramisu SDK V012, shipped in `/config/lib` of this camera and used by
nothing on it (§1.3). Each is a stripped ARM shared object with a small C API documented by a public header; each checks
the chip id through `/dev/msys` + `/dev/mem` before it works (§2.2):

- **libSRC_LINUX.so** — sample-rate converter 8/16/32/48 kHz, mono or stereo: zero-order-hold upsampling and a symmetric
  float FIR (51 or 243 taps) with decimation (§3).
- **libSSL_LINUX.so** — sound-source localisation with two microphones: per 128-sample frame a GCC-PHAT (or a
  time-domain) delay estimate, a cumulative vote over nine direction bins, the direction in degrees (§4).
- **libAEC_LINUX.so** — acoustic echo canceller on 128-sample hops: a 2008 libspeex MDF linear filter (8 partitions)
  followed by WebRTC-AEC's non-linear processor with MStar's 26-level suppression table, on Arm Ne10's real FFT (§5).
- **libAPC_LINUX.so** — audio processing chain on 128-sample hops: noise reduction (WebRTC's float noise suppressor with
  per-band intensities), a 129-bin equaliser, an FFT-mask high-pass filter and an AGC with a 7-point compression curve
  (§6).
- **libAED_LINUX.so** — acoustic event detection (baby cry) and loud-sound detection at 8 kHz: private SRC and APC
  copies, 12 MFCCs and the log level per frame, median / robust spread per segment, a 26 → 20×6 → 1 tanh MLP and a vote
  (§7).
- **libBF_LINUX.so** — 4-microphone beamformer: an MVDR stage with a fixed broadside look direction and a learned noise
  covariance, then an IMCRA / OM-LSA post-filter; carries a modified SILK VAD (`IaaVad_*`) and links Ne10 (§8).

The video analytics libraries libMD, libOD and libVG share the mechanics of §1–§2 and are specified in
`ive-video-analytics.md`.

Other documents: the inventory rows of the nine libraries and the licence files — `firmware-image.md` §3.5
"`/config/lib` (SDK user libraries)"; the `/dev/msys` ioctls (`IOCTL_MSYS_GET_RIU_MAP` 0x5315) and RIU addressing —
`kernel-platform.md` §10.1 "`/dev/msys`" and §3.1 "Static I/O map and RIU addressing"; the chip id register 001E.00 and its value 0xF4 — `kernel-platform.md` §3.2 "Chip ops, chip id
and revision" and `ipl.md` §4.5; the audio stack mi_ai / mi_ao, whose stub VQE / AED / SSL / BF paths do not reach these
libraries — `audio.md` §2.4 "libmi_ai.so — per function"; libmi_ive / MVE, libMD, libOD, libVG and libmi_vdf's use of
them — `ive-video-analytics.md`.

Tags: **[R]** read from the binary (its Ghidra decompile, capstone disassembly, strings, emulation of its code) or from a
file; **[S]** stated by a source (the SDK headers, the API manuals, public code: speexdsp / libspeex, WebRTC, Opus/SILK,
Ne10, Cohen's IMCRA / OM-LSA code, Numerical Recipes); **[I]** inference. A heading tagged [R] means every statement
under it is [R] unless marked otherwise. Struct, field and enum names are the public headers' [S]; exported function
names are the binaries' symbols [R]; names of internal functions and handle fields are given by their role [I] unless a
public source is named.

Addresses are VAs (= file offsets for `.text`); an address given with image base 0x10000 (written "Ghidra address") is
VA + 0x10000 [R].

Binaries (dump of 2026-10-16) [R]: `/config/lib/lib{SRC,SSL,AEC,APC,AED,BF,MD,OD,VG}_LINUX.so`
and `AEC_LICENSE_{1,2}.txt`, `APC_LICENSE_{1,2}.txt` beside them.

## 1. The nine libraries

### 1.1 Files [R]

All nine are ARM EABI5 shared objects, stripped (`.dynsym` only), built by GCC 9.1.0, with no `NEEDED` entry beyond
libc / libm / libgcc_s. The three video libraries (MD, OD, VG) leave `MI_MVE_*` (libmi_ive.so) and `MI_SYS_*`
(libmi_sys.so) undefined without a `NEEDED` entry for those libraries, so the application must load libmi_ive /
libmi_sys first [R: no NEEDED; I: consequence for the loader]. Together the nine are 845 928 bytes.

| lib | bytes | .text | exports (code) | what it is [R: header names and messages; I: wording] |
|---|---|---|---|---|
| libSRC | 37 152 | 0x38f8 | `IaaSrc_*` 4 | sample-rate converter 8/16/32/48 kHz |
| libSSL | 30 900 | 0x44c4 | `IaaSsl_*` 8 | sound-source localisation (2-mic direction of arrival) |
| libAEC | 79 636 | 0xcd04 | `IaaAec_*` 10 | acoustic echo canceller |
| libAPC | 125 772 | 0x16078 | `IaaApc_*` 11, `IaaAnr_*` 7, `IaaEq_*` 7, `IaaAgc_*` 7, `AGC_Process` | audio processing chain: noise reduction, equaliser, AGC |
| libAED | 115 480 | 0xf620 | `IaaAed_*` 13 | acoustic event detection (baby cry) + loud-sound detection (LSD) |
| libBF | 142 680 | 0x1c379 | `IaaBf_*` 8, `IaaVad_*` 5, `ne10_*` 92 | microphone-array beamformer + voice-activity detector; links Ne10 math |
| libMD | 150 148 | 0x1f154 | `MI_MD_*` 13, `_md_sym_106…155` 50 | motion detection over luma frames (on MVE) |
| libOD | 109 240 | 0x151ec | `MI_OD_*` 9, `_td_sym_90…140` 51 | occlusion / tamper detection (on MVE) |
| libVG | 54 920 | 0x97e0 | `MI_VG_*` 18, `IvaVG_*` 9, `vg_cross_*` 4 | virtual gate / region intrusion (line crossing) |

MVE imports of the video libraries [R]: libMD and libOD carry a private static copy of the MVE core with the same 30
`*_Acc` hardware builders as libmi_ive but none of its CPU implementations; of their 41 / 40 weak `MI_MVE_*` imports
(`*_Neon` / `*_PureC` back-ends) only the five exported `_PureC` ones (CannyEdge, CannyHysEdge, GMM, LKOpticalFlow,
NormGrad) resolve against this libmi_ive.so, the other 36 / 35 stay 0; every such call is guarded and returns 0x10000106
when the symbol is absent, so their MVE calls work only on hardware-eligible images. libVG's 8 public MVE imports are all
exported by libmi_ive. (Details: `ive-video-analytics.md` §8.2, §8.3.)

Licence files beside them: `AEC_LICENSE_1.txt` = `APC_LICENSE_1.txt` (391 bytes, the Ooura FFT notice) and
`AEC_LICENSE_2.txt` = `APC_LICENSE_2.txt` (1510 bytes, the WebRTC BSD-3 notice) — md5 equal pairwise; text and what they
cover in §9.

### 1.2 Origin: the public Tiramisu SDK drop [R]

- All nine files are byte-identical to `soc_sdk/dynamic/` of GitHub `qanthink/skyipc_tiramisu` (git blob SHA-1 equal for
  every file), whose `soc_sdk/readme.txt` names the source "tiramisuV012_20220606_usb" = SigmaStar SSC8836 / Tiramisu SDK
  V012. Tiramisu is this camera's SoC family. They are stock SigmaStar binaries, not ODM additions. The headers in that
  repository's `soc_sdk/include/` therefore describe exactly these builds [I, strong: same SDK drop].
- The same nine (by md5) are in the Anjoy MC-H12S firmware. That firmware's `opt/ch/media_server` links
  libAEC_LINUX.so and dlopens `/lib/libAED_LINUX.so`, and contains a static libmi_ai with a working VQE
  (`MI_AI_EnableVqe` …) that calls `IaaAec_*`, `IaaApc_*`, `IaaAnr_*`, `IaaEq_*`, `IaaAgc_*`, `IvaMD_*` [R: strings] — a
  real-world caller of the API.

### 1.3 Users on this camera [R]

None. prog_uvc and kjd_lvgl import none of these symbols, and the camera's libmi_ai.so / mi_ai.ko have stub VQE / AED /
SSL / BF paths with no reference to any `*_LINUX.so` (`audio.md` §2.4). All nine are unused on the product. The SDK's own
libmi_vdf.so references `MI_MD_*` / `MI_OD_*` / `MI_VG_*`, so the video libraries are not stand-alone orphans
(`ive-video-analytics.md` §1.3).

### 1.4 Third-party code inside

The libraries are self-contained DSP / vision code behind a small, header-documented C API [I]. Public code identified
in the audio libraries, with no licence notice on the device except the two of §9 [R strings / constants; I origin]:

| code | where | section |
|---|---|---|
| libspeex `mdf.c` (MDF echo canceller, master 2008-05-15…05-30) | libAEC | §5.7 |
| WebRTC AEC non-linear processor and its (unused) NLMS filter hooks | libAEC | §5.8, §5.10 |
| WebRTC float noise suppressor `ns_core.c` | libAPC ANR (and libAED's private APC copy) | §6.7 |
| Arm Ne10 real FFT | libAEC, libAPC, libBF (libBF also its matrix routines and 92 `ne10_*` exports) | §2.6, §8 |
| Ooura `fft4g` / `fftsg` `rdft` | libSSL, libAED | §4, §7.4 |
| Opus / SILK VAD `silk_VAD_GetSA_Q8`, modified | libBF (`IaaVad_*`) | §8.10 |
| Cohen's IMCRA / OM-LSA reference `omlsa.m`, ported | libBF post-filter | §8.8 |
| Numerical Recipes `expint` | libBF post-filter | §8.8 |
| SPP noise PSD estimator of Gerkmann & Hendriks (2012) | libBF stage 1 | §8.6 |

## 2. Mechanics shared by the nine libraries

### 2.1 Build tags [R]

Each library carries one or more SigmaStar "MVX" version strings in `.data`
(`MVX1##<chip>######g<commit5>##<module>###…XVM`, some followed by `#APIVERSION#<nn>#`). All say chip `M6`. Per library
(module, short git commit):

| lib | tags |
|---|---|
| SRC | COM cfdad, SRC e4385, APIVERSION 12 |
| SSL | SSL e70bb |
| AEC | COM cfdad, AEC aa71f, APIVERSION 11 |
| APC | AGC 64a10, APC 98cf6, COM cfdad, EQ 11b48, NR 3759d |
| AED | APC 5fc59, AGC 261f8, SRC 8483e, COM d9545, BCRY 6009e |
| BF | COM 84a7e, BF 69a8d, VAD 1e1f2 |
| MD / OD / VG | MD ae163 + MVE 5985d / OD 8ad94 + MVE a5131 / VG 5d7b7 |

libAED therefore contains private copies of APC, AGC and SRC code of other revisions than libAPC / libSRC (§7.2, §7.7).
The header macros `SRC_API_VERSION {'1','2'}` and `AEC_API_VERSION {'1','1'}` match the APIVERSION tags.

### 2.2 Chip gate (`/dev/msys` + `/dev/mem`) [R]
<!-- ev:audio-algorithms.2.2 -->

Every library has two copies of a chip-id reader, instruction-for-instruction identical in all nine (only addresses, literal-pool offsets and call targets differ; comparison of the normalised disassembly of every library against
libSRC's). The used copy (118 instructions):

1. `open("/dev/mem", O_RDWR|O_SYNC)` (flags 0x101002), `open("/dev/msys", O_RDWR|O_SYNC)`.
2. `ioctl(msys, 0x5315, &info)` — `IOCTL_MSYS_GET_RIU_MAP` = `_IO('S', 0x15)` with the kernel's packed `MSYS_MMIO_INFO`
   {u32 `VerChk_Version` = 0x4D530100, u64 `addr` (out), u32 `size` (out), u32 `VerChk_Size` = 0x14}.
3. `mmap(NULL, size, PROT_READ|PROT_WRITE, MAP_SHARED, memfd, (off_t)addr)` — the RIU window; read the u16 at byte
   offset 0x3C00 = register bank 0x001E, reg 0x00 = chip id; `munmap`, close both.
4. Returns 1 iff the id is **0xF4** (the M6 / Tiramisu chip id) and caches the answer in two `.bss` words (done flag,
   result), so later calls do not touch the devices again. Any failure prints `[ALGO_MSG_ERR] open %s device error.` /
   `MIOMAP_IOC_GET_RIU_MAP failed!!` / `mmap failed!!` and then `Unexpected IC version`, and caches 0.

The second copy (106 instructions) returns the u64 0x3050305030503050 (`"P0P0P0P0"` read as bytes) + chip id instead; no
library calls it.

libAEC is the one exception for the used copy: the same steps split into a raw reader 0x6b58 (returns the u16 at RIU
0x3C00, 0 after any failure, the same three error messages) and a cached comparator 0x6d1c (`Unexpected IC version` when
≠ 0xF4, caches result and done flag) — behaviour equal to the single function of the others [I: from the two listings].

The gate needs read-write access to `/dev/mem`, i.e. root [I]; it is copy protection, not a hardware dependency of the
algorithms [I]. In libSRC, libSSL, libAED, libBF and the VAD a failed gate makes Init print `IC version check wrong` /
`Platform/License check failed!!` and return NULL; in libAEC Init succeeds and every Run fails (§5.3).

### 2.3 Base-62 codec (`EN_OR_DE`) [R]

`void codec(char *out, const char *in, int en_or_de)`, 500 instructions, identical in all nine, over exactly 10
characters; alphabet `0-9A-Za-z` mapped to 0..61:
- `en_or_de = 1` (encode): out[i] = sym((3·v(in[i])) mod 62); a character outside the alphabet prints
  `The code is not permission...` and gives `*`.
- `en_or_de = -1` (decode): out[i] = sym(k) with 3k ≡ v (mod 62), i.e. k = 21·v mod 62 (the code searches k = 0..61 with a
  divisible-by-62 test); no match gives `?`. out[10] = 0.
- any other value prints `The parameter: EN_OR_DE -- only support -1 or 1`.

Only libAEC calls it, for its key API (§5.6); libAEC also has a second variant 0x6954.

### 2.4 Other shared helpers and the address table [R]
<!-- ev:audio-algorithms.2.4 -->

- **Inverted cached gate** (17 instructions; SRC, SSL, APC, AED, BF only): `if (done) return 0; if (gate()) { done = 1;
  return 0 } return 1` — 0 = chip OK. It guards a library-wide "in use" word (libSRC §3.4 step 6).
- **Timestamp** (all nine, never called): `time` + `localtime` → `sprintf("%d")` of (year − 100)·10⁸ + (mon + 1)·10⁶ +
  mday·10⁴ + hour·100 + min (YYMMDDhhmm) into a `.bss` buffer, which it returns. Two compilations: A (46 instructions) in
  SRC, SSL, AED, BF, VG and B (44, different summation order) in AEC, APC, MD, OD; every term is an integer below 2⁵³, so
  both give the same number [I: exact double arithmetic].

Addresses (VA) and direct callers; "–" = no caller:

| lib | used gate (callers) | 2nd copy | codec (callers) | inverted gate (callers) | timestamp |
|---|---|---|---|---|---|
| SRC | 0x2380 (0x914, 0x3ac4) | 0x25a8 | 0x1928 (–) | 0x3aa4 (0xbc8) | 0x1848 A |
| SSL | 0x29c8 (0x9b0, 0x4334) | 0x2bf0 | 0x1f70 (–) | 0x4314 (–) | 0x1e90 A |
| AEC | 0x6b58 + 0x6d1c (0x4868, 0xa338) | 0x6da0 | 0x6100 (0x6f98) and variant 0x6954 (0x7018) | none | 0x6028 B |
| APC | 0xcb60 (≥ 8 sites) | 0xcd88 | 0xc108 (–) | 0xf670 (8 sites) | 0xc030 B |
| AED | 0x92e0 (5 sites) | 0x9508 | 0x8888 (–) | 0xd330 (5 sites) | 0x87a8 A |
| BF | 0x7b10 (0x283c Init, 0x4618 VAD Init, 0xacc8) | 0x7d38 | 0x63c0 (–) | 0xaca8 (–) | 0x62e0 A |
| MD | 0x8a10 (0x36e4, 0x7178, 0x9920) | 0x8c38 | 0x7ef0 (–) | none | 0x7e18 B |
| OD | 0x55c8 (0x3bbc, 0x64d8) | 0x57f0 | 0x4aa8 (–) | none | 0x49d0 B |
| VG | 0xa320 (0x25b8) | 0xa548 | 0x9800 (–) | none | 0x9720 A |

(In libSRC the first three have no symbol: 0x12380, 0x125a8, 0x11928 with base 0x10000.)

### 2.5 Logging, return codes, handles and state flags [R]

- Messages go to stdout by `printf("[%s] %s…", "ALGO_MSG_ERR" | "ALGO_MSG_MSG", "")` (the second `%s` is an empty
  string, an unused colour prefix [I]) or by bare `puts`. There is no log level and no way to silence them.
- Return codes are `0x1000LLnn` with module LL (SSL 01, AED 02, BF 03, AEC 04, APC 05, SRC 06 [S: headers]) and nn per
  error, exactly as the headers' `ALGO_*_RET` enums; the single APC modules (ANR, EQ, AGC) return small enums (§6.1), the
  video libraries their own enums (`ive-video-analytics.md` §1.2).
- Handles: in SRC, SSL, AEC and BF the handle is the caller's working buffer itself and nothing is allocated; the same
  holds for APC, which imports no allocator [I]. AED, BF, MD, OD and VG import `malloc` / `calloc`.
  libAED allocates its private SRC instance with `malloc` (§7.2); libBF's `malloc` / `free` imports serve the SILK VAD's
  320-byte scratch (allocated and freed on every VAD call) and `IaaVad_Free` (§8.10). MD, OD and VG:
  `ive-video-analytics.md` §1.2.
- Each SRC / SSL / AEC handle has an "initialised" flag and a "busy" or state word; a call while busy returns
  `…_API_CONFLICT`, a call before Init `…_INVALID_CALLING`. The busy word is a plain word, not atomic — it detects
  re-entry, it does not lock [I].

### 2.6 The Ne10 FFT wrapper (libAEC, libAPC, libBF) [R]
<!-- ev:audio-algorithms.2.6 -->

libAEC (0x7304) and libBF (0x8054) carry the same wrapper over Arm **Ne10**'s real FFT; libAPC carries the same Ne10
code (strings `[FFT]Ne10 has init already`, `NE10_init() failed` in libAEC and libAPC) and calls its 256-point real FFT
through the wrapper 0xe040 [R; I: the same wrapper]:
- `fft(work, n, dir, data)`: work NULL → `[Audio] Ne10_workingBufPtr is Null in fft`. dir = 1: r2c (out = work,
  in = data, cfg = work + 0x808) through a pointer slot, then memcpy (n/2 + 1) complex back into data. dir = −1: memcpy
  (n/2 + 1) complex from data to work, memset data (n + 2 floats), c2r (out = data, in = work, cfg) through a second
  slot. Any other dir: returns doing nothing.
- Result layout: n/2 + 1 interleaved complex (re, im) pairs, bins 0…n/2. **Emulated** (on libAEC, n = 256; on libBF, n = 512, random input): the forward result equals
  numpy `rfft(x)` (unscaled DFT, e^(−j2πkn/N)) to 4·10⁻⁶ (libBF: error 5.6·10⁻⁶ of 46.9); the inverse of that output
  gives back x·1.000000 (residual 5·10⁻⁷). **The inverse includes the 1/N normalisation.**
- FFT context (libAEC at handle +0x12578, libBF at h+0x2068): flag +0x80C, set-up buffer pointer +0x808, length +0x810.
  libAEC's set-up 0x724c prints `[FFT]Ne10 has init already`, `[FFT]: NEON init error.code :%d`,
  `[FFT] Please assigned the begining address about FFT workingbuffer in Init.`; its release 0x73d4 only clears +0x80C
  (`handle is NULL in FFT_Free!`).
- Set-up size, placed at the end of every working buffer: fftsize(n) = 16n + 2n + 0x60·(n/32) + 0x28C (fftsize(256) =
  0x178C, fftsize(512) = 0x2C8C) — libAEC 0xd158, libAPC 0x224b0 (cited in this form; above libAPC's `.text`, so [I] a
  Ghidra address = VA 0x124b0), libBF 0x29990 ([I] likewise VA 0x19990). The set-up itself (libAEC 0xd188, libBF
  0x199c0) is Ne10's r2c configuration allocation into a caller buffer: factors (libAEC 0xa390), twiddles (0xa818,
  0xaa60), super-twiddles via `sincosf`.
- Ne10 `init` (libAEC 0xae14) reads `/proc/cpuinfo` looking for `neon` (fopen / fread, lower-cases the text, `strstr`)
  and prints `ERROR: couldn't read file "/proc/cpuinfo".` / `init Ne10 DSP fail` / `ERROR: init dsp failed` on failure,
  but the result does not choose the implementation: `init_dsp` (libAEC 0xacc0; libBF 0x102d8) fills the two slots with
  one r2c / c2r pair only (libAEC 0xae50 / 0xbb40, slots 0x231E0 / 0x231E4; libBF 0x17730 / 0x18688).
- libAEC's 0xc8c0 / 0xcfc4 are radix-8 butterflies; the c2r path computes `1.0/nfft` (0xcfc4) and `0.25/nfft` (0xc610,
  0xc850, 0xc878) at run time — Ne10's built-in inverse normalisation. The bodies mix scalar and NEON code (vld2 / vzip),
  i.e. Ne10's C sources compiled with auto-vectorisation [I].
- The Ne10 version is not identified.

## 3. libSRC_LINUX.so — sample-rate converter
<!-- ev:audio-algorithms.3 -->

Functions [R]: `IaaSrc_GetBufferSize`
0x088c, `IaaSrc_Init` 0x0908, `IaaSrc_Run` 0x1144, `IaaSrc_Release` 0x1734; internal: parameter check 0x0760, codec
0x1928 (§2.3, unused), chip gate 0x2380 / 0x25a8 (§2.2), inverted gate 0x3aa4 (§2.4) / "in use" clear 0x3aec, FIR core
0x3b18, zero-order-hold upsampler 0x3dc8.

### 3.1 API (header `AudioSRCProcess.h`, checked against the binary) [R]

```c
typedef enum { SRC_8k_to_16k, SRC_8k_to_32k, SRC_8k_to_48k, SRC_16k_to_8k, SRC_16k_to_32k, SRC_16k_to_48k,
               SRC_32k_to_8k, SRC_32k_to_16k, SRC_32k_to_48k, SRC_48k_to_8k, SRC_48k_to_16k, SRC_48k_to_32k } SrcConversionMode;
typedef enum { SRATE_8K = 8, SRATE_16K = 16, SRATE_32K = 32, SRATE_48K = 48 } SrcInSrate;   /* kHz, not Hz */
typedef enum { ORDER_LOW = 0, ORDER_HIGH = 1 } SrcFilterOrder;
typedef struct { SrcInSrate WaveIn_srate; SrcConversionMode mode; SrcFilterOrder order;
                 unsigned channel; unsigned point_number; } SRCStructProcess;            /* 5 × u32 */
unsigned     IaaSrc_GetBufferSize(SrcConversionMode mode);
SRC_HANDLE   IaaSrc_Init(char *workingBufferAddress, SRCStructProcess *p);
ALGO_SRC_RET IaaSrc_Run(SRC_HANDLE h, short *in, short *out, int *output_size);
ALGO_SRC_RET IaaSrc_Release(SRC_HANDLE h);
/* ALGO_SRC_RET: 0 ok; 0x10000601 INIT_ERROR, …602 INVALID_MODE, …603 INVALID_HANDLE, …604 INVALID_CHANNEL,
   …605 INVALID_POINT_NUMBER, …606 INVALID_SAMPLE_RATE, …607 API_CONFLICT, …608 INVALID_CALLING */
```

Per mode the library keeps `{M, L}` = (decimation, interpolation) (`.rodata` pairs written to handle +0x00):

| mode | 0 | 1 | 2 | 3 | 4 | 5 | 6 | 7 | 8 | 9 | 10 | 11 |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| in→out kHz | 8→16 | 8→32 | 8→48 | 16→8 | 16→32 | 16→48 | 32→8 | 32→16 | 32→48 | 48→8 | 48→16 | 48→32 |
| L (up) | 2 | 4 | 6 | 1 | 2 | 3 | 1 | 1 | 3 | 1 | 1 | 2 |
| M (down) | 1 | 1 | 1 | 2 | 1 | 1 | 4 | 2 | 2 | 6 | 3 | 3 |
| FIR table | ×2 | ×4 | ×6 | ×2 | ×2 | ×3 | ×4 | ×2 | ×3 | ×6 | ×3 | ×3 |

FIR table "×n" = the low-pass designed for cut-off π/n of the upsampled rate, n = max(L, M) [I: from the table choice and
the centre tap ≈ 1/n].

### 3.2 `IaaSrc_GetBufferSize(mode)` [R]

mode > 11: `puts("Unsupported SRC mode")`, return 0. Otherwise `size = L·0x4800 + 0x1FEC` bytes, printed as
`Working buffer size: %u bytes` (L = 2 → 45 036 B; L = 6 → 118 764 B). It does not depend on channels, order or frame
size.

### 3.3 Handle layout (= the working buffer) [R offsets; I names]

| off | field |
|---|---|
| 0x00 / 0x04 | u32 M, u32 L (from the mode table) |
| 0x08 | u32 L again (from the per-mode L table used by GetBufferSize; sizes the buffers) |
| 0x0C | WaveIn_srate (kHz) |
| 0x10 | not written (the `mode` slot of a copied `SRCStructProcess`; never read) |
| 0x14 | order · 0x18 channel · 0x1C point_number |
| 0x24 / 0x28 | input buffer per channel (s16, 0xC00 bytes = 1536 samples): ch0 = h+0x54, ch1 = see below |
| 0x2C / 0x30 | output / upsample buffer per channel (s16, L·0xC00 bytes): ch0 = h+0xC54 |
| 0x34 | half filter order N: 50 (LOW) or 242 (HIGH); the filter has N+1 taps |
| 0x38 / 0x3C | float history per channel (L·0x1800 + 0x3CC bytes) |
| 0x40 / 0x44 | s32 phase per channel (decimator position carried between calls), zeroed by Init |
| 0x48 | pointer to the FIR half-table (float[N/2+1]) |
| 0x4C | u32 initialised · 0x50 u32 busy |
| 0x54 … | the buffers, laid out consecutively: in0 (0xC00), out0 (L·0xC00), in1 (0xC00), out1 (L·0xC00), hist0, hist1 |

### 3.4 `IaaSrc_Init(buf, p)` [R]

1. Chip gate (§2.2); failure → `puts("****…Warning!! IC version check wrong, disable SRC Lib ****…")`, return NULL.
2. `printf("SRC IC check pass. ver.%04x.%04x.%04x", 0x2021, 0x728, 0)` — a fixed version 2021.0728.0000.
3. Parameter check (0x0760, also run by every `Run`): `point_number ≤ 1536` else 0x…605; `WaveIn_srate ∈ {8,16,32,48}`
   else …606; `channel ∈ {1,2}` else …604; `order ≤ 1` else …602 (each with its `[SRC] Error: … is configuration error`
   message). Init turns any failure into `puts("[SRC]Error: Initial Configuration Fail!!")` and NULL.
4. Clear initialised / busy, store the fields of §3.3, look up L (mode > 11 → message, L = 0), re-check
   `point_number ≤ 0x600`. Print `[SRC] Channel: Mono|Stereo`, carve and zero all buffers.
5. Set N = 50 (`Filter order: LOW `) or 242 (`Filter order: HIGH `), else `Unsupported SRC filter order mode`, NULL.
   Pick M / L and the FIR table for the mode; if `WaveIn_srate` is not the mode's input rate →
   `Error! Input/Output sample rate is not correct.`, NULL. Unknown mode → `Unsupported SRC mode`, NULL.
6. `Sample rate conversion : %d-> %d kHz`; set the library-wide "in use" word through the inverted gate 0x3aa4 (it runs
   the gate once more if the word is clear; on failure `[SRC] Error: Profile init failed.`, NULL); initialised := 1,
   phases := 0, return `buf`.

### 3.5 `IaaSrc_Run(h, in, out, &n_out)` [R]

1. NULL → …603; not initialised → …608; busy → …607; set busy. Re-run the parameter check of §3.4 step 3 and **return
   its error with busy still set** (every later call then fails with API_CONFLICT until Release) [R: no clear on that
   path].
2. Input: `point_number` samples per channel; stereo input is interleaved L, R (de-interleaved into in0 / in1), mono is
   copied.
3. Per channel c: if L ≥ 2 the frame is first upsampled by **sample repetition** (zero-order hold, 0x3dc8: every input
   sample written L times) into out_c and that L·point_number-sample block is filtered in place; if L = 1 the input
   buffer is filtered straight into out_c.
4. FIR core (0x3b18), per channel, input block x[0..n) of s16:
   - append x as float to the history: hist[N + i] = (float)x[i] (the first N floats are the tail of the previous
     block);
   - for i = phase; i < n; i += M: y = Σ_{k=0}^{N/2−1} (hist[i+k] + hist[i+N−k])·h[k] + hist[i+N/2]·h[N/2] (symmetric
     linear-phase FIR of N+1 taps, float accumulate starting at 0.0); saturate to [−32768, 32767] and truncate toward
     zero to s16; write consecutively;
   - phase = i − n (the next block's first output position), hist[0..N) = hist[n..n+N) (memcpy);
   - returns the number of outputs.
   The output is delayed by N/2 samples of the upsampled rate (25 or 121) [I: linear-phase FIR on a history of N].
5. Output: stereo outputs are interleaved back (out0 / out1), mono copied; `*output_size` = outputs **per channel**;
   clear busy, return 0. The output count per call is ⌈(L·point_number − phase)/M⌉, so it varies by one between calls
   when M ∤ L·point_number [I].

### 3.6 `IaaSrc_Release(h)` [R]

NULL → …603; not initialised → …608; busy → …607 (message "Other SRC function is running"); else set busy, clear the
library-wide "in use" word (0x3aec), clear initialised and busy (one 64-bit store), return 0. The buffer is the caller's.

### 3.7 FIR tables [R values; I design]

Eight distinct float32 half-tables in `.data` (VA 0x18060…0x18b3f), DC gain 2·Σh[0..N/2−1] + h[N/2] = 1.000 ± 0.001:

| table | VA | taps (N+1) | centre tap h[N/2] | used by modes |
|---|---|---|---|---|
| LOW ×2 | 0x18060 | 51 | 0.48104 | 0, 3, 4, 7 |
| LOW ×4 | 0x18130 | 51 | 0.23299 | 1, 6 |
| LOW ×6 | 0x18200 | 51 | 0.14673 | 2, 9 |
| LOW ×3 | 0x182D0 | 51 | 0.31543 | 5, 8, 10, 11 |
| HIGH ×2 | 0x183A0 | 243 | 0.48500 | 0, 3, 4, 7 |
| HIGH ×4 | 0x18588 | 243 | 0.23438 | 1, 6 |
| HIGH ×6 | 0x18770 | 243 | 0.15000 | 2, 9 |
| HIGH ×3 | 0x18958 | 243 | 0.31667 | 5, 8, 10, 11 |

LOW tables are stored as 26 coefficients + 26 zero floats (208-byte stride); HIGH as 122 coefficients (488-byte stride).
The coefficient values are in [`audio-filter-tables.tsv`](../data/audio-filter-tables.tsv). "×2" serves both 16→8 (anti-alias before decimation) and 8→16
(anti-image after the hold) [R].

### 3.8 Unreferenced data and code [R]

- `.data` 0x18B40–0x18B7F: 16 u32 = round(2^24 · 2^(1/2^k)), k = 1…16 (23726566 = 2^24·√2 … 16777393), a fixed-point
  root table; no code references it (no resolved pointer into that range) — a leftover of the common ("COM") math
  module [I].
- `.data` 0x18B80…: the MVX tags of §2.1.
- The codec (§2.3), the second gate copy (§2.2) and the timestamp helper (§2.4) are present but not called.

## 4. libSSL_LINUX.so — sound-source localisation (2 microphones)

Exports [R]: `GetBufferSize` 0x0968, `Init` 0x09a0, `Reset` 0x0d28, `Config`
0x1064, `Get_Config` 0x1394, `Run` 0x1438, `Get_Direction` 0x19e4, `Free` 0x1e38. Internal: codec 0x1f70 (§2.3,
unused), chip gate 0x29c8 (used) / 0x2bf0 (second copy, unused), Ooura real FFT `rdft` 0x3aa0 with `makewt` 0x2e50,
`makect` 0x38c4, `cftfsub` / `bitrv` 0x311c / 0x342c, NEON memcpy 0x424c (unused), inverted gate 0x4314 (unused), empty
free hook 0x4248, dB→linear lookup 0x4410, FFT / lag-table init 0x4454, zero-crossing counter 0x44f0, time-domain delay
0x4588, GCC-PHAT delay 0x46f4.

### 4.1 API (header `AudioSslProcess.h`) [R]

```c
typedef struct { unsigned point_number, sample_rate, mic_distance /*cm*/, bf_mode; } AudioSslInit;
typedef struct { unsigned temperature /*°C*/; int noise_gate_dbfs; int direction_frame_num; } AudioSslConfig;
unsigned     IaaSsl_GetBufferSize(void);                                   /* = 0x44E0 = 17 632 */
SSL_HANDLE   IaaSsl_Init(char *buf, AudioSslInit *);
ALGO_SSL_RET IaaSsl_Config(SSL_HANDLE, AudioSslConfig *);
ALGO_SSL_RET IaaSsl_Get_Config(SSL_HANDLE, AudioSslConfig *);
ALGO_SSL_RET IaaSsl_Run(SSL_HANDLE, short *mic_in /*2ch interleaved*/, int *delay_sample);
ALGO_SSL_RET IaaSsl_Get_Direction(SSL_HANDLE, int *direction /*degrees*/);
SSL_HANDLE   IaaSsl_Reset(SSL_HANDLE buf, AudioSslInit *);
ALGO_SSL_RET IaaSsl_Free(SSL_HANDLE);
/* 0x10000101 INIT_ERROR, 102 INVALID_CONFIG, 103 INVALID_HANDLE, 104 INVALID_SAMPLERATE, 105 INVALID_POINTNUMBER,
   106 INVALID_BFMODE, 107 DELAY_SAMPLE_TOO_LARGE, 108 INVALID_CALLING, 109 API_CONFLICT */
```

### 4.2 Handle layout (0x44E0 bytes) [R offsets; I names]

| off | content |
|---|---|
| 0x000 | magic 0x30513057 (checked by Config / Run: "handle pointer seems invalid") |
| 0x004 / 0x008 / 0x00C | point_number, sample_rate (Hz), mic_distance (cm) |
| 0x010 / 0x014 | temperature (default 25), noise_gate_dbfs (default −80) |
| 0x018 | s16 linear gate = dB table(noise_gate_dbfs) (§4.7); only Get_Config's 8-byte load touches it, nothing uses it |
| 0x01C / 0x020 | direction_frame_num (0 until Config), bf_mode |
| 0x024 / 0x224 | newest 128 samples of ch0 / ch1 as float (s16 / 32767) — input of the time-domain mode |
| 0x424 / 0x824 | 256-sample analysis frame of ch0 / ch1 (float), windowed in place |
| 0x0C24 / 0x102C | spectrum X0 / X1, 129 complex (re, im) |
| 0x1434 | G = X1·conj(X0), then PHAT-normalised; 0x183C \|G\| (129 floats) |
| 0x1C40 | r = IFFT(G) (256 floats); 0x2040 r re-ordered so that index 128 = lag 0 |
| 0x2C48 | int lag[256] = −128 … 127 |
| 0x3448 | speed of sound: float 346.0 after Init / Reset, **int** (331 + 0.6·T) after Config; never read |
| 0x344C | frame-energy accumulator (float) |
| 0x3450 | D = max lag in samples |
| 0x3454 | u32 count[9] per direction bin · 0x3478 u32 "no decision" count |
| 0x347C | float centre[9] = D·sin(θᵢ), θᵢ = −90° + 22.5°·i |
| 0x34A0 | init flag · 0x34A4 config state · 0x34A8 run state · 0x34AC result state (2 = not yet, 0 = inside the call, 1 = done) |
| 0x34B0 | float lagsum[9] (sum of sub-sample lags per bin) |
| 0x34D4 / 0x38D4 / 0x40D4 | Ooura `ip[256]`, `w[512]`, transform array a[258] |
| 0x44DC | u32 0x100 (first argument of the rdft wrapper, unused by it [I]) |

### 4.3 Init / Reset / Free [R]

- **Init**: chip gate (§2.2) ≠ 1 → `[SSL] Platform/License check failed!!`, NULL. Prints `IC check pass` and
  `ver.2021.0730.0000`. NULL buf or init → NULL. `point_number` must be **128** (else …"point_number is invalid", NULL);
  `bf_mode` ≤ 1 (else NULL). `sample_rate` is **not** checked here. Then memset(buf, 0, 0x44E0), magic, copy the four
  fields, temperature = 25, gate = −80 dBFS (+ linear value), c = 346.0, D = (int)(mic_distance·0.01 / 346 · fs) + 1,
  `ip` / `w` built by a forward call of the FFT set-up, lag table, centre[9], init = 1, the three states = 2,
  `Initialization Success!`.
- **Reset**: the same argument checks (no chip gate, no version print), requires init = 1 ("Please call IaaSsl_Init
  first!"), calls the empty free hook, then does exactly what Init does from the memset on.
- **Free**: NULL → …103; else calls the empty hook and returns 0 — the flags are left as they are.

### 4.4 Config / Get_Config [R]

Config: NULL handle → …103, NULL config → …102; config state := 0; bad magic → …103; not initialised → …101; run state 0
or result state 0 → …109 (API_CONFLICT; the config state stays 0, see §4.6); sample_rate ∉ {8000, 16000, 32000, 48000} →
…104; point_number ≠ 128 → …105; bf_mode > 1 → …106. Then: temperature stored; c = (float)(331 + 0.6·T);
D = (int)(mic_distance·0.01 / c · fs) + 1; noise_gate = clamp(noise_gate_dbfs, −80, 0) (+ linear value);
direction_frame_num stored unchecked; config state := 1, return 0.

Get_Config: NULL config → …102, NULL handle → …103; returns {temperature, noise_gate_dbfs (clamped),
direction_frame_num} (+0x10, +0x14, +0x1C; the NEON `vtrn` in the binary picks those three) — no state checks.

### 4.5 Run (one 128-sample stereo frame) [R]

1. NULL → …103; run state := 0; magic, init (…101), config state 2 → …108 "Config must called before Run", config
   state 0 → …109, result state 0 → …109.
2. If `mic_in` is NULL the frame update is skipped (message only) and the old frame is analysed again. Otherwise, per
   channel: frame[0..128) := frame[128..256); frame[128+i] := in[2i+ch] / 32767 for i < 128 (ch0 = even samples); the
   128 new floats are also copied to +0x24 / +0x224. Then the whole 256-sample frame is windowed **in place**:
   frame[0] := 0, frame[i] *= sin(2π·i/256) for i = 1…255 (a full sine period: the second half is negated). Because the
   window is applied in place, the half kept for the next call has already been windowed once [R; I: an implementation
   slip of the vendor code].
3. The zero-crossing rate of ch0's newest 128 samples is computed (0x44f0) and discarded.
4. bf_mode = 1 → `*delay_sample` = time-domain delay (§4.5.1); bf_mode = 0 → GCC-PHAT delay (§4.5.2). Run state := 1,
   return 0.

**4.5.1 Time-domain delay (0x4588)**: on the newest 128 samples x0, x1 (+0x24, +0x224), for each lag τ in −D…D a mean
squared difference over the overlap (128 − |τ| samples) is formed; the result is the τ with the smallest value (first
minimum), returned as an int in samples; D < 0 → −D, D = 0 → 0 [R structure; I: the exact sign convention of τ — the
code pairs x1 at higher indices with x0 for negative τ].

**4.5.2 GCC-PHAT (0x46f4)**:
- energy accumulator += Σ_{i<256} (f0[i]² + f1[i]²)/2 (windowed frames); two further RMS sums are computed and
  discarded.
- X0 = rdft(f0), X1 = rdft(f1) (n = 256, forward; output re-packed as 129 bins with a[1] = 0 and the Nyquist real part
  at a[256], a[257] = 0 — Ooura `fftsg.c` `rdft` in float, with the sign convention of Ooura's forward transform).
- G[k] = X1[k]·conj(X0[k]) (re = X1r·X0r + X1i·X0i, im = X0r·X1i − X1r·X0i), |G[k]| stored; G[k] /= (|G[k]| + 1e−9).
- r = rdft⁻¹(G) (isgn = −1: a[1] := a[256], a[256] := 0 first; no 2/n scaling), then rotated so r'[j] = r[(j + 128) mod
  256] (lag = j − 128).
- p = argmax r' (first maximum), q = argmin r' (first minimum), S = Σ|r'|. If |lag[p]| ≤ D: lag = lag[p],
  confidence = (|r'[p]| − |r'[q]|) / (S/256); else lag = 0, confidence = −1.
- bin b = index of the centre nearest to lag (first on ties; the first distance is clamped at 1e9).
- confidence < 2.0 → return **−32768** ("no decision"), no-decision count += 1. Else count[b] += 1, lagsum[b] += p̂ − 128
  with the parabolic peak p̂ = p + (r'[p−1] − r'[p+1]) / (2(r'[p−1] − 2r'[p] + r'[p+1])) (computed as
  −0.5·((−2p−1)·r'[p−1]/2 + 2p·r'[p] + (1−2p)·r'[p+1]/2) / (−r'[p] + r'[p−1]/2 + r'[p+1]/2) − 128), and the return value
  is the integer lag[p].

### 4.6 Get_Direction (vote over the accumulated frames) [R]

1. `*direction` := −112 first. NULL → …103 (the message names ssl_config). Result state := 0. Not initialised → …101;
   config state 0 → …109; run state 2 → …108 "Run must called before GetDirection"; run state 0 → …109; bf_mode = 1 →
   …106 ("You don't need to call IaaSsl_Get_Direction").
2. Level: E = energy / (point_number · direction_frame_num); level = 20·log10(√E) dBFS, **stored into the energy
   accumulator**. If level ≤ noise_gate_dbfs → return 0 (direction −112) — with the result state left at 0, so every
   later Config / Run returns API_CONFLICT ("GetResult is being called") until a Get_Direction call completes.
3. Thresholds T[i] = tab[i]·(direction_frame_num / 100) (integer division), tab = {5, 10, 10, 10, 10, 10, 10, 10, 5, 60}
   (`.rodata` 0x4d70; i = 0…8 the bins, 9 the no-decision count). Energy accumulator := 0 (only on this path).
4. Decision, in this order: d := (nodec ≥ T9) ? 9 : −1; then d := 4 if count4 ≥ T4; 3 if count3 ≥ T3; 5 if count5 ≥ T5;
   2 if count2 ≥ T2; 6 if count6 ≥ T6; 1 if count1 ≥ T1 (each later test overrides). Then the edge bins:
   - count7 < T7: if count0 ≥ T0 → d := (count8 ≥ T8) ? 8 : 0; else if count8 ≥ T8 → d := 8; else if d = 9 →
     `*direction` = −112, result state 1, return 0;
   - count7 ≥ T7 and count0 < T0 → d := (count8 ≥ T8) ? 8 : 7;
   - count7 ≥ T7 and count0 ≥ T0 → d := (count8 ≥ T8) ? 8 : 0.
   d = −1 (nothing reached) is not caught: the code then reads count[−1] (= D) and lagsum[−1] (= the result-state word)
   [R; a defect].
5. a = lagsum[d] / count[d] / D; |a| > 1 → `Delay sample too large…`, …107 (the result state stays 0). Else
   `*direction` = (int)(−asin(a)·180 / 3.1415926), result state := 1, return 0. Range −90…+90°; lag −D (bin 0) gives
   +90°. Counts and lag sums are never cleared except by Init / Reset — the vote is cumulative.

### 4.7 Tables and constants [R]

- dB→linear (`.data` 0x17070): pairs {s32 dB, s16 value} for dB = 0, −1, …, −90: value = round(32767·10^(dB/20))
  (32767, 29203, 26027, …). The lookup (0x4410) scans until the key matches, without a bound (the caller clamps to
  −80…0).
- Direction thresholds `{5,10,10,10,10,10,10,10,5,60}` (0x4d70); lag-table seed {−128, −127, −126, −125} + step 4
  (0x44e0).
- Constants: 346.0 (Init c), 331.0 + 0.6·T (Config c), 0.01 (cm→m), 32767.0, 1/256, 6.2831852, 1e−9, 2.0 (confidence
  threshold), 128.0, 180.0, 3.1415926.

## 5. libAEC_LINUX.so — acoustic echo canceller (Speex MDF + WebRTC NLP)
<!-- ev:audio-algorithms.5 -->

Exports [R]: `GenKey` 0x4448, `VerifyKey` 0x444c, `GetKeyLen` 0x4470,
`Config` 0x4474, `Init` 0x4808, `Reset` 0x4c34, `Run` 0x5008, `Free` 0x5f20, `GetLibVersion` 0x5f88, `GetBufferSize`
0x5fa8. Floating-point build with NEON (`[AEC] Floating point mode.` and a `NEON ENABLE` banner at Init) [R]. Its FFT is
the Ne10 wrapper 0x7304 (§2.6); there is no Ooura code in libAEC (no `makewt` constant π/4 anywhere; the only 2π/N
literal, 0.0245436926 at 0x8630, is the Speex window) [R].

### 5.1 API (header `AudioAecProcess.h`) [R]

```c
typedef struct { unsigned point_number, nearend_channel, farend_channel; IAA_AEC_SAMPLE_RATE sample_rate; } AudioAecInit;
typedef struct { IAA_AEC_BOOL comfort_noise_enable;          /* +0  */
                 short delay_sample;                         /* +4 (u32 slot) */
                 unsigned suppression_mode_freq[6];          /* +8  */
                 unsigned suppression_mode_intensity[7]; }   /* +32 */ AudioAecConfig;   /* 60 bytes */
unsigned     IaaAec_GetBufferSize(void);
AEC_HANDLE   IaaAec_Init(char *buf, AudioAecInit *);
ALGO_AEC_RET IaaAec_Config(AEC_HANDLE, AudioAecConfig *);
AEC_HANDLE   IaaAec_Reset(char *buf, AudioAecInit *);
ALGO_AEC_RET IaaAec_Free(AEC_HANDLE);
ALGO_AEC_RET IaaAec_Run(AEC_HANDLE, short *near_end, short *far_end);   /* output replaces near_end */
ALGO_AEC_RET IaaAec_GetLibVersion(unsigned short *year, unsigned short *date, unsigned short *time);
void IaaAec_GenKey(char *code_out); void IaaAec_VerifyKey(char *code_in); int IaaAec_GetKeyLen(void);
/* 0x10000401 INIT_ERROR, 402 INVALID_HANDLE, 403 INVALID_SAMPLE_RATE, 404 INVALID_POINT_NUMBER, 405 INVALID_CHANNEL,
   406 INVALID_ENABLE, 407 INVALID_SUP_BAND, 408 INVALID_SUP_MODE, 409 INVALID_COMFORT_NOISE, 410 INVALID_DELAY_SAMPLE,
   411 API_CONFLICT, 412 INVALID_CALLING */
```

### 5.2 Sizes, version, handle fields [R offsets; I names]

- `GetBufferSize` = 0x1358C + fftsize(256) = 0x1358C + 0x178C = **85 272 bytes** (fftsize 0xd158, §2.6 — the FFT set-up
  placed at the end of the buffer, pointer at +0x12D80).
- `GetLibVersion` writes 0x2021, 0x0728, 0 and returns 0; Init prints `[AEC] AEC IC check pass. ver.2021.0728.0000` when
  the chip gate passes.
- Handle fields of the API layer: +0x8188 sample_rate; +0x81A8 point_number (= 128); +0x81AC near-end channels; +0x81B0
  far-end channels; +0x81B4 s16 delay_sample; +0x81B8 comfort_noise_enable; +0x81BC…+0x81D0 suppression_mode_freq[6];
  +0x81D4…+0x81EC suppression_mode_intensity[7] (written through the mode setter 0x3c30, §5.9); +0x81F4 u64 cleared by
  Reset (far-end clipping words, §5.8 step 11); +0x8200 initialised, +0x8204 second word of the same 64-bit store
  (`.rodata` constant {1, 0}); +0x8208 cleared; +0x820C chip-gate result (ORed in at Init, cleared by Free); +0x8218
  Speex `saturated` counter (§5.7); +0xEB30 near-end frame (256 floats); +0xF330 far-end frame (256 floats); +0xFB30
  output / overlap (256 floats); +0xFF30 FFT work (258 floats); +0x12578 FFT context; +0x12D88 = 0x100 (FFT length);
  +0x1358C FFT set-up. The core's fields follow in §5.7–§5.10.

### 5.3 Init / Reset / Free [R]

- **Init**: NULL buf / struct → NULL. `sample_rate` ∉ {8000, 16000} → NULL (message mislabelled "AudioAecProcessStruct is
  NULL"). memset(buf, 0, 0x1358C), FFT set-up pointer := buf + 0x1358C. Chip gate (0x6d1c, the cached check of §2.2, id
  0xF4); its result is ORed into +0x820C — **a failed gate does not fail Init**, it makes every later `Run` fail (§5.5).
  Then `point_number` must be 128 (`This AEC Lib only support point_number = 128`), near-end and far-end channels 1 or 2
  (checked in that order with the matching messages). Store rate / channels / 128, print `[AEC] Setting: [128][8]`, FFT
  length 0x100, core init (0x405c, with the sample rate; failure → `[AEC] Initialization Error at Line 276`, NULL),
  library "in use" word set (0xa318), defaults comfort-noise 0, delay 0, freq = {4, 6, 36, 49, 50, 51}, intensity = {5,
  4, 4, 5, 10, 10, 10} (`.rodata` 0xee20), core tables (0x85d8 with 128, 0x400; 0xa134 with 0x18 and &rate), print the
  banners, initialised := 1, return buf.
- **Reset**: NULL → message, returns 0 (NULL). Not initialised → `[AEC] Please run IaaAec_Init before run IaaAec_Reset`,
  NULL. Else: release hooks (0x3adc no-op, clear "in use", FFT context release 0x73d4), the same point / channel / rate
  checks and stores as Init (no memset of the whole buffer, no chip gate), then core init with **16000 hard-coded**
  whatever the stored rate (`aec_core_init(16000, h)` (0x1405c)), which also re-runs the Speex rate set-up with 16000 — an 8 kHz
  instance is re-initialised with 16 kHz core constants after Reset [I: defect]. Defaults as Init, the three 1 KB frames
  (+0xEB30, +0xF330, +0xFB30) zeroed, +0x81F4 := 0, initialised := 1. Failure message line number 392.
- **Free**: NULL → message, returns **−1** (not an ALGO code). Else release hooks, initialised := 0, gate flag := 0, 0.

### 5.4 Config [R]

NULL handle or config → 0x10000402. Not initialised → …412 ("Need init before IaaAec_Config"). Re-checks the stored init
values: point_number ≠ 128 → …404; channels (the "NearEnd" / "FarEnd" messages are swapped relative to Init) → …405;
rate → …403. `comfort_noise_enable` > 1 → …409 ("Please Config the CngMode"). Every intensity must be 0…25 (else …408) —
the header comment "0~15" does not hold for this build; every freq element must be 1…126 (else …407); **ascending order
is not checked**. Then delay_sample (s16) and comfort-noise are stored; the mode setter 0x3c30 gets the intensity array
(a negative return prints `[AEC] Set mode error.` but Config still succeeds); the six freq values are stored; return 0.
`delay_sample` is not range-checked (the manual says −9…9 [S]); it is stored at +0x81B4 and never read.

### 5.5 Run (one 128-sample hop) [R]

1. NULL → −1. Not initialised or chip-gate flag 0 → `[AEC] Platform/License check failed!!`, −1 (not an ALGO code).
2. Near-end frame (+0xEB30, 256 floats): shift left by 128, append the 128 new samples as float — stereo input is
   **down-mixed** to (L + R) >> 1 (integer average before conversion). Far-end frame (+0xF330): the same with the
   far-end channel count. Far-end clipping words: +0x81F4 := (some far-end sample of this hop has |x| > 32700), +0x81F8
   the previous hop's value, +0x81FC = either (§5.8 step 11).
3. The mean absolute near-end value over the hop is computed; the far analysis 0x3aec runs: copy the far frame into the
   FFT work area, forward 256-point real FFT (0x7304), keep the spectrum at +0x1073C; copy again, multiply by a window
   from `.rodata` 0xDC08…0xDE07 (128 floats w[i] = sin(π·i/256), the rising half of a 256-point sine window; the NEON
   loop applies it to two pointers, i.e. to both halves of the frame [I: mirrored]), FFT again, keep at +0x10B44 [R
   structure; I: the second is a windowed far-end spectrum]; what reads them: §5.8 step 3.
4. Saturation check: count adjacent sample pairs in the near-end frame (+0xED30…+0xEF2F) that are both 32767.0 or both
   −32768.0; more than 42 → `[AEC] The Microphone data is saturate` (processing continues; the message is all it does).
5. If the mean |near| > 0: the echo canceller proper (0x3c0c, with +0x81FC as an argument and the output area +0xFB30)
   runs (§5.8). An all-zero hop skips it (the frame passes through unchanged).
6. Output: 128 samples written back into `near_end`, float → s16 (round toward zero, saturated to [−32768, 32767]); for a
   2-channel near end the processed mono result goes to the left slots and is then copied to the right slots
   (0x5c44 / 0x5e90). Return 0.

### 5.6 Key API and licence state [R]

- `GetKeyLen` returns 11 (10 code characters + NUL).
- `GenKey(code_out)` is a tail jump to 0x6f80: codec 0x6100 (out = an 11-byte `.bss` buffer at 0x231CC, in = `code_out`,
  encode) — it reads the caller's buffer and writes **nothing** back to it.
- `VerifyKey(code_in)` → 0x6fa8: clears the `.bss` buffer and reads the chip id by calling the raw reader 0x6b58 (§2.2)
  directly; an id ≠ 0xF4 gives `Unexpected IC version` and result 0 (cache words `.bss` 0x231C4 / 0x231C8); otherwise
  it decodes `code_in` with the codec variant 0x6954 into the buffer and returns 1. The result is ORed into a `.bss` word
  0x231B4 that **no code reads**. The key therefore has no effect; only the chip id matters. (Whether the message and
  cache on this path come from an inline comparison or from the comparator 0x6d1c is not settled by the decompile and
  the helper comparison.)
- Codec variants: 0x6100 is the codec of §2.3; 0x6954 is the same base-62 ×3 cipher, but its "plain" side uses the
  alphabet shifted by −0x11 (digits → 0x1F…0x28, upper case → 0x30…0x49, lower case → 0x50…0x69), and its encode
  direction reads the first argument and writes the second (the opposite of 0x6100).

### 5.7 Linear stage: the libspeex MDF copy [R unless tagged]

The canceller 0x89f0 is speexdsp / libspeex `speex_echo_cancellation` (MDF), float build, `TWO_PATH`, one microphone and
one speaker channel. Identification: the string `Unknown speex_echo_ctl request: ` in 0xa134 (= `speex_echo_ctl`), the
Speex warning text "The echo canceller started acting funny and got slapped (reset)…", and every numeric constant below
matching speexdsp head 7a15878 (`libspeexdsp/mdf.c`) [R constants; S source].

**Revision.** speexdsp has only cosmetic changes to `mdf.c` since the 2009-12-22 split (includes, whitespace, a duplicate
`WORD2INT`); all functional history is in libspeex [R: git log of xiph/speex and xiph/speexdsp]. The candidates:

| revision | date | single-channel code | output loop | `GET_IMPULSE_RESPONSE` ctl |
|---|---|---|---|---|
| Speex 1.2beta3 tag | 2007-12-08 | yes | saturate before pre-emphasis; mutes the sample (`tmp_out = 0`) when \|in\| ≥ 32000 | no |
| 0b34061 | 2008-02-13 | yes | as beta3 | added |
| 4973799 + 928e278 | 2008-05-15 | yes | `WORD2INT` after pre-emphasis, no muting | yes |
| d794a6a (multi-channel merged) → Speex 1.2rc1 (2008-07-23) … speexdsp head | 2008-05-30 … | no: C mics × K speakers, `power_spectrum_accum`, Sxx computed twice | as 928e278, per channel | yes |

What the binary shows:
- **Single-channel structure.** 0x89f0 calls `mdf_inner_prod` (slot 0x231F8 → 0x1370) **seven** times — Sxx (0x8cc8),
  Sff (0x8e3c), Dbf and See (0x93a4, 0x9420), Sey, Syy, Sdd (0x95e4, 0x9600, 0x961c); the power-spectrum helper (slot
  0x231F4 → 0x1430) **three times in a row** (0x9850, 0x9868, 0x987c), after the See floor (25600 at 0x96b8) — the order
  `power_spectrum(E,Rf); power_spectrum(Y,Yf); power_spectrum(X,Xf)` of the single-channel code. The multi-channel code
  computes Xf at the top, accumulates Rf / Yf inside the per-channel loop before the sanity check and calls
  `mdf_inner_prod` eight times. 0x1430 **overwrites** its output (ps[k] = re² + im², no `+=`): it is `power_spectrum`.
- **Output loop** (0x9540…0x95a4): tmp = input + preemph·memE − e; clamped to [−32768, 32767] **after** the pre-emphasis
  term; out[i] = memE = the clamped value (float, no rounding); no microphone-saturation test and no muting in the loop.
  This is the 2008-05-15 shape (clamp on the final value, no muting) with two MStar edits: the result stays float (Run
  converts to s16, §5.5) and memE takes the clamped value (Speex keeps the unclamped one).
- **ctl** 0xa134 handles only SET_SAMPLING_RATE (24), GET_SAMPLING_RATE (25) and GET_FRAME_SIZE (3, returns the constant
  128 instead of `st->frame_size`); the SET case sets spec_average = 128/rate, beta0 = 256/rate, beta_max = 64/rate
  (`.rodata` 0xa234: 128.0, 256.0, 64.0) and notch_radius 0.9 / 0.982 / 0.992 as Speex. No impulse-response requests.

Conclusion [I, strong]: the copy is libspeex `mdf.c` as it stood on master between **928e278 (2008-05-15)** and the
multi-channel merge **d794a6a (2008-05-30)** — the only single-channel revision without muting. No release tag holds that
text (1.2beta3 still mutes, 1.2rc1 is multi-channel), so MStar took a development snapshot of those two weeks or ported
a 1.2rc1+ copy back to one channel by hand; the first fits the evidence more simply. The impulse-response ctl cases
(present in that revision) and `st->frame_size` in GET_FRAME_SIZE were cut by MStar [I].

**Parameters and matching constants**: frame 128, window N = 256, **M = 8 partitions** (filter 1024 taps = 64 ms at
16 kHz, 128 ms at 8 kHz), state set up by `speex_echo_state_init(128, 1024)` (0x85d8) and `SPEEX_ECHO_SET_SAMPLING_RATE`
(0xa134); it runs on the near hop (+0xED30) and far hop (+0xF530), floats in s16 scale. DC notch (`filter_dc_notch16`,
den2 = r² + 0.7(1 − r)²) with r = 0.9 below 12 kHz, 0.982 below 24 kHz, 0.992 above; pre-emphasis 0.9; Hann window
0.5 − 0.5·cos(2πi/256); `prop` 0.7·e^(−0.3k) normalised to sum 0.8; ss = 0.35/M = 0.04375; Davg 0.6/0.4 and 0.85/0.15,
Dvar 0.36/0.16 and 0.7225/0.0225; RER = (0.0001·Sxx + 3·leak·Syy)/See capped at 0.5; adaptation 0.7·r + 0.3·RER·e,
power_1 = …/(power + 10); MIN_LEAK 0.005; leak > 16383 → 32767; sanity limits N·10⁹ = 2.56·10¹¹, See floor N·100 =
25600, "screwed up" test N·10000 = 2.56·10⁶ and the reset after 50; output saturated to [−32768, 32767]; last_y = in −
out.

**Storage**: the Speex filter W shares storage with WebRTC's wfBuf (+0x3054, 0x2000 bytes = 8 × 256 floats); the
foreground filter is at +0xB484, the far-spectrum history X at +0x8634 (9 × 129 complex). The `saturated` counter
(+0x8218) is read before the gradient loop (0x8ef0: non-zero → skip the M weighted updates and decrement, branch 0x9ce8)
and cleared by init / reset (0xa054), but **nothing sets it non-zero** — the float build never had Speex's far-end
saturation freeze (`#ifdef FIXED_POINT`) and the microphone one was removed with the muting: the freeze is dead code.

**FFT scaling**: the stage uses the Ne10 wrapper 0x7304 (§2.6) with a re-pack to Speex's layout and keeps Speex's float
convention by hand — every forward call is followed by a ×1/256 (`.rodata` 0.00390625; uses at 0x8c14, 0x9200, 0x9750,
0x97dc) and every inverse is preceded by a ×256 (0x8d2c, 0x915c, 0x92f8), giving Speex's "forward scaled 1/N, inverse
unscaled" [R positions; I pairing].

**Outputs**: stack array 1 = Speex `out` (echo-reduced near end); stack array 3 = a copy of the internal block at +0x600
(the second half of a Speex scratch vector) [I, strong: the echo estimate y]; the 2nd and 4th output pointers are never
used.

### 5.8 Per-hop pipeline and the non-linear processor (WebRTC AEC) [R unless tagged]

The non-linear stage is WebRTC-AEC's (`aec_core.c` of webrtc-audio-processing v0.3.1) [R constants; S source; I match]:
WebRTC's seed 777, `normal_error_threshold` 1.5e−6, overdrive 2, the dMinPow initialisation,
`kNormalSmoothingCoefficients`, `kMinFarendPSD` 15 and the −6.9 / −11.5 target-suppression values are present, and every
constant below matches it unless a difference is named.

Run (§5.5) → 0x3c0c, which stores its arguments at +0x8198 (near frame), +0x819c, +0x81a0, +0x81f0 and jumps to the
canceller 0x2598. Per 128-sample hop:
1. Comfort-noise preparation (only if `comfort_noise_enable` = 1, branch 0x379c): FFT of the 256-sample near frame
   (+0xEB30) and WebRTC's noise-floor tracker: dPow = 0.9·dPow + 0.1·|D|²/4; if noiseEstCtr (+0x8194) > 50: dMinPow =
   (dPow < dMinPow) ? (dPow + 0.1·(dMinPow − dPow))·1.0003 : dMinPow·1.0003; while noiseEstCtr < 500·mult: ctr++,
   dInitMinPow = (dMinPow > dInitMinPow) ? dInitMinPow + 0.1·(dMinPow − dInitMinPow) : dMinPow and noisePow →
   dInitMinPow, afterwards noisePow → dMinPow. Differences from WebRTC: ramp 1.0003 (WebRTC 1.0002), initial blend 0.1
   (WebRTC 0.001), dMinPow initial 5·10⁵ (WebRTC 10⁶), and the whole tracker runs only when comfort noise is on.
2. Linear echo cancellation: the Speex MDF of §5.7 (0x89f0).
3. Buffers for the NLP: dBuf (+0x0000, 256) ← [previous, current] Speex out; buffer +0x0400 (256) ← [previous, current]
   of the second Speex output; near frame (+0xEB30). Each is multiplied by the 256-point sqrt-Hann window (`.rodata`
   0xDC08, w[i] = sin(πi/256), mirrored) and transformed: near → D (+0x11F6C), Speex out → E (+0x11B64), second stream →
   "X" (+0x11354). WebRTC's far-end role in the coherence is thus played by the Speex stage's second output, and WebRTC's
   own linear filter, far buffer and delay estimator are not used [R: their function pointers are set but never called,
   §5.10; the far spectra computed by 0x3aec at +0x1073C / +0x10B44 are never read].
4. Smoothed PSDs (0x20f4, WebRTC `SmoothedPSD`): with g = {0.9, 0.1} (mult 1, 8 kHz) or {0.93, 0.07} (mult 2, 16 kHz),
   stored as 256·g at `.rodata` 0xDBF4 and applied as s += (g₁/256)·(P − s), i.e. s = g₀·s + g₁·P: sd ← |D|², se ← |E|²,
   sx ← max(|X|², 15), sde ← D·conj(E)-type cross terms, sxd ← D·conj(X) (WebRTC's formulas). Arrays: sx +0x78E4, sd
   +0x7AE8, se +0x7CEC, sde +0x70D4, sxd +0x74DC (129 each; complex for the cross terms).
5. Divergence (WebRTC): Σse > Σsd starts it, Σse·1.05 < Σsd ends it (divergeState, s16 at +0x8178); while diverged
   E := D. WebRTC's "Σse > 19.95·Σsd → clear wfBuf" reset is absent.
6. Coherence: cohde[i] = |sde|² / (sd·se + 1e−10), cohxd[i] = |sxd|² / (sd·sx + 1e−10); both 0 where sd = 0.
7. Averages over the preferred band, using **odd bins** to map WebRTC's 65-bin indices onto 129 bins: bins 2k − 1 for
   k = minPref … minPref + prefSize − 1 with minPref = 4/mult, prefSize = 24/mult: hNlXdAvg = 1 − mean cohxd,
   hNlDeAvg = mean cohde.
8. hNlXdAvgMin (+0x7EF8): if hNlXdAvg < 0.75 and below the minimum, it becomes the minimum. Near-end state, **without
   WebRTC's hysteresis**: near = (hNlDeAvg > 0.98 and hNlXdAvg > 0.9), recomputed every hop (no stored state, no
   echoState).
9. Gains: if hNlXdAvgMin = 1 (no echo seen yet): overDrive[b] := minOverDrive[b] for all 7 bands, hNl = near ? cohde :
   1 − cohxd, hNlFb = hNlFbLow = the matching average. Else near → hNl = cohde, hNlFb = hNlFbLow = hNlDeAvg; echo →
   hNl[i] = min(cohde[i], 1 − cohxd[i]), hNlFb / hNlFbLow = the 0.75 / 0.5 quantiles (indices ⌊(n−1)·96/128⌋ and
   ⌊(n−1)/2⌋ after an in-place quicksort 0x0bd8) of hNl at the same odd bins. hNl is kept at +0x12374.
10. Minimum tracking (WebRTC): hNlFbLow < 0.6 and < hNlFbLocalMin → hNlFbMin = hNlFbLocalMin = hNlFbLow, hNlNewMin = 1,
    hNlMinCtr = 0 (fields +0x7EF0, +0x7EF4, +0x7EFC, +0x7F00); hNlFbLocalMin = min(+0.0008/mult, 1); hNlXdAvgMin =
    min(+**0.0004**/mult, 1) (WebRTC 0.0006); ctr counts while hNlNewMin; at ctr = 2: reset, and for each band b
    overDrive[b] = max(targetSupp[b] / (ln(hNlFbMin + 1e−10) + 1e−10), minOverDrive[b]) (computed in double).
11. Far-end clipping boost: the 7 overDrive values are copied and multiplied by **4** when the argument +0x81F0 is 1 —
    Run passes +0x81FC, the far-end clipping word of §5.5 step 2.
12. overDriveSm[b] (+0x7F20) = od ≥ sm ? 0.9·sm + 0.1·od : 0.99·sm + 0.01·od (per band).
13. OverdriveAndSuppress (0x0ff8 → 0x10cb0): per bin i the band b(i) is found by walking the band edges
    `suppression_mode_freq[6]` (+0x81BC): b starts at 0 and increments when i equals the current edge, so band 0 =
    bins 0…f₀−1, band 6 = bins f₅…128; if hNl[i] > hNlFb: hNl[i] = w[i]·hNlFb + (1 − w[i])·hNl[i]; hNl[i] =
    hNl[i]^(overDriveSm[b(i)]·c[i]) (NEON exp2 / log2 polynomial for bins 0–127, `pow` for bin 128); E[i] *= hNl[i] with
    the imaginary part negated. w = weight curve (`.rodata` 0xD7E8, 129 values, 0 … 0.4), c = 1 + √(i/128) (0xD9F0).
14. Comfort noise (if enabled, 0x3668): for every bin i, a = √(max(noisePow[i], 0.0002) · max(0.9999 − hNl[i]², 0));
    E.re += a·cos φ, E.im −= a·sin φ with (cos φ, sin φ) from two 1024-entry Q15 tables (`.rodata` 0xDE10 / 0xE610,
    random phases, amplitude 32767). The table index runs 0…1023; at the end it restarts at the low byte of
    table[cursor], cursor = 0…1023 — **both counters are global** (`.bss` 0x231AC / 0x231B0), shared by all AEC
    instances in the process. Bin 0 and the Nyquist bin also receive noise (WebRTC leaves them at zero).
15. Synthesis: E → inverse FFT (0x7304, isgn −1; WebRTC's packing with the imaginary sign flipped), overlap-add with the
    sqrt-Hann window and the 128-float outBuf (+0x7F74): out[i] = y[i]·w[i] + outBuf[i], outBuf[i] = y[128+i]·w[127−i];
    the 128 outputs overwrite the start of the near frame (+0xEB30), which Run then saturates to s16 (§5.5). No 2/N
    factor is applied because Ne10's inverse is already normalised (§2.6); the level equals WebRTC's (Ooura inverse ×
    2/N) [I]. Finally dBuf is shifted by 128.

### 5.9 Suppression-mode setter (0x3c30) [R values; I intent]

For each of the 7 bands, intensity k → (targetSupp, minOverDrive) at +0x7F3C[b] / +0x7F58[b]:

| k | 0 | 1 | 2 | 3 | 4 | 5 | 6 | 7 | 8 | 9 | 10 | 11 | 12 |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| targetSupp | 0 | −0.5 | −1 | 0 | −3 | 0 | −1.5 | −5 | 0 | −1.5 | −6.9 | −6.9 | 0 |
| minOverDrive | 0 | 0 | 0 | 1 | 1 | 2 | 2 | 1 | 3 | 3 | 1 | 2 | 4 |

| k | 13 | 14 | 15 | 16 | 17 | 18 | 19 | 20 | 21 | 22 | 23 | 24 | 25 |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| targetSupp | 0 | −11.5 | −11.5 | −15 | −20 | −25 | −30 | −45 | −50 | −55 | −60 | −70 | −80 |
| minOverDrive | 5 | 1 | 5 | 8 | 8 | 8 | 8 | 8 | 8 | 8 | 8 | 8 | 90 |

It also rewrites +0x8190 = 1.5e−6 (the error threshold read by the unused `ScaleErrorSignal`, §5.10). k > 25 → −1
(Config only prints "Set mode error"). Of WebRTC's three NLP modes (−6.9, 1), (−11.5, 2), (−18.4, 5) only the first
appears exactly (k = 10); the table is MStar's own extension to 26 levels [I]. Core init (0x405c) sets every band to
(−1.5, 1.0); **Init does not call the setter**, so until the first Config the core runs with (−1.5, 1) everywhere
although the config record holds the defaults {5, 4, 4, 5, 10, 10, 10} (§5.3).

### 5.10 Function-pointer table and the unused WebRTC NLMS filter [R]

Core init 0x405c (WebRTC `InitAec` + this table) fills nine `.bss` function pointers through the GOT (GOT words
resolved):

| slot | target | function | called by |
|---|---|---|---|
| 0x231E8 | 0x1fa4 | WebRTC `FilterFar` (NEON form) | nobody |
| 0x231EC | 0x112c | WebRTC `ScaleErrorSignal` (NEON form) | nobody |
| 0x231F0 | 0x19a4 | Speex `weighted_spectral_mul_conj` | 0x89f0 (0x9020, gradient loop) |
| 0x231F4 | 0x1430 | Speex `power_spectrum` (interleaved layout) | 0x89f0 ×3 |
| 0x231F8 | 0x1370 | Speex `mdf_inner_prod` | 0x89f0 ×7 |
| 0x231FC | 0x1618 | Speex `spectral_mul_accum` | 0x89f0 (0x8d0c foreground, …) |
| 0x23200 | 0x20f4 | WebRTC `SmoothedPSD` | NLP |
| 0x23204 | 0x1038 | WebRTC `FilterAdaptation` | nobody |
| 0x23208 | 0x0ff8 | WebRTC `OverdriveAndSuppress` | NLP |

(The slots 0x231E0 / 0x231E4 are Ne10's r2c / c2r, §2.6.) The table mirrors WebRTC's `WebRtcAec_FilterFar /
ScaleErrorSignal / FilterAdaptation / OverdriveAndSuppress` hooks, extended with the four Speex helpers [I].

**0x1fa4 — FilterFar** (unused): for p = 0…7 (8 partitions): xPos = (p + xfBufBlockPos)·258 floats, minus 8·258 when
p + xfBufBlockPos > 7 (xfBufBlockPos = handle +0x817C); pos = p·258. For bins 0…127 (NEON, 4 at a time, de-interleaved
by vld2): yf.re += xf.re·wf.re − xf.im·wf.im, yf.im += xf.re·wf.im + xf.im·wf.re (no conjugate); bin 128 through the
scalar `MulRe` 0x0ba0 / `MulIm` 0x0bb0. Buffers (interleaved complex, 129 per partition): xfBuf +0x1014 (8 × 0x408
bytes), wfBuf +0x3054 (8 × 0x408; shared with the Speex W, §5.7), yf +0x10F4C (129 complex, bin 128 at
+0x1134C / +0x11350). It is WebRTC's `FilterFarNEON` with PART_LEN1 = 129 and 8 partitions instead of 65 and 12 / 32 [S:
`aec_core_neon.c`; I: match].

**0x112c — ScaleErrorSignal** (unused): for bins 0…127 (NEON): e = ef / (xPow + 0.001) · 0.25 (xPow = 129 floats at
handle +0x800; ef = 129 interleaved complex at +0x1175C); |e| = √(re² + im²) (vrsqrte with an ∞ guard); if |e| > thr
(thr = handle +0x8190, 1.5·10⁻⁶ after the mode setter) then e ·= thr / (|e| + 10⁻¹⁰); finally e ·= 0.5 (μ). Bin 128
(+0x11B5C / +0x11B60) the same in scalar code with `vdiv` / `vsqrt`. Differences from WebRTC's `ScaleErrorSignalNEON`:
the regulariser is 0.001 instead of 10⁻¹⁰, the extra factor 0.25, μ fixed at 0.5 (WebRTC: 0.6 at 8 kHz, 0.5 otherwise)
and the threshold read from the handle [R constants; S WebRTC; I comparison].

Together with FilterAdaptation 0x1038 these are WebRTC's whole NLMS linear filter, set up and never run: the Speex MDF
took its place [I].

### 5.11 Band-walk quirk and function map

The band walk of §5.8 step 13 only advances on exact equality and, after band 6, compares with the next word in memory,
which is `suppression_mode_intensity[0]` [R]; non-ascending edges freeze the band index, and an intensity[0] value above
f₅ would move the upper bins to "band 7" = targetSupp[0] (a non-positive exponent → gain ≥ 1) [I: effect]. Config checks
neither the order nor this (§5.4).

Function map [R]: 0x0ba0 / 0x0bb0 `MulRe` / `MulIm`; 0x0bd8 quicksort; 0x0ff8 OverdriveAndSuppress → 0x10cb0 weighting /
pow; 0x1038 FilterAdaptation (unused); 0x112c ScaleErrorSignal (unused); 0x1370 `mdf_inner_prod`; 0x1430
`power_spectrum`; 0x1618 `spectral_mul_accum`; 0x19a4 `weighted_spectral_mul_conj`; 0x1fa4 FilterFar (unused); 0x20f4
SmoothedPSD; 0x2598 canceller (entry 0x3c0c); 0x3668 comfort noise; 0x3adc no-op release hook; 0x3aec far analysis
(dead output); 0x3c30 mode setter; 0x405c core init; 0x6100 / 0x6954 codecs; 0x6b58 / 0x6d1c chip gate; 0x6f80 GenKey
body; 0x6fa8 VerifyKey body; 0x724c FFT context set-up; 0x7304 FFT wrapper; 0x73d4 FFT context release; 0x85d8 Speex
state init; 0x89f0 Speex echo cancellation; 0xa134 `speex_echo_ctl`; 0xa318 "in use" set; 0xacc0 / 0xae14 Ne10
`init_dsp` / `init`; 0xae50 / 0xbb40 Ne10 r2c / c2r; 0xd158 / 0xd188 FFT set-up size / set-up.

## 6. libAPC_LINUX.so — audio processing chain ANR / EQ + HPF / AGC
<!-- ev:audio-algorithms.6 -->

Four sub-libraries in one file, each with its own Init / Config / Run / Free / Reset /
GetConfig / GetBufferSize — `IaaAnr_*` 0x5198…0x5f7c, `IaaEq_*` 0x6a68…0x7798, `IaaAgc_*` 0x3640…0x4274 + `AGC_Process`
0x2f38 — and the combined `IaaApc_*` (0x8278…0xbecc) [R]. Header `AudioProcess.h`: NR 7 bands / 6 edges, EQ 129 bins,
AGC compression curve 7 points [S].

### 6.1 API (header `AudioProcess.h`) [S, checked against the binary R]

`IaaApc_GetBufferSize(AudioApcBufferConfig *sw)`, `IaaApc_Init(buf, AudioProcessInit *{point_number, channel,
sample_rate}, sw)`, `IaaApc_Config(h, AudioAnrConfig*, AudioEqConfig*, AudioHpfConfig*, AudioVadConfig*,
AudioDereverbConfig*, AudioAgcConfig*)`, `IaaApc_Run(h, short *io)` (in place), `IaaApc_GetNrResult(h, out)` /
`IaaApc_GetNrEqResult(h, out)` (taps after NR and after NR + EQ, §6.8), `IaaApc_Free`, `IaaApc_Reset`,
`IaaApc_GetConfig(h, init, anr, eq, hpf, vad, dereverb, agc)`; the single modules take `AudioProcessInit` and their own
config. Structures:
- `AudioApcBufferConfig` {anr_enable, eq_enable, dr_enable, vad_enable, agc_enable};
- `AudioAnrConfig` {anr_enable, user_mode, anr_intensity_band[6], anr_intensity[7], anr_smooth_level,
  anr_converge_speed};
- `AudioEqConfig` {eq_enable, user_mode, short eq_gain_db[129]};
- `AudioHpfConfig` {hpf_enable, user_mode, cutoff_frequency 0 = 80 / 1 = 120 / 2 = 150 / 3 = 500 Hz};
- `AudioVadConfig` {vad_enable, user_mode, vad_threshold}, `AudioDereverbConfig` {dereverb_enable} — placeholders
  (§6.4);
- `AudioAgcConfig` (0x4C bytes) {agc_enable, user_mode, `AgcGainInfo` gain_info {gain_max, gain_min, gain_init},
  drop_gain_max, attack_time, release_time, short compression_ratio_input[7], compression_ratio_output[7],
  drop_gain_threshold, noise_gate_db, noise_gate_attenuation_db, gain_step}; below cr_in / cr_out stand for the two
  compression-ratio arrays.

Return codes 0x10000501…0x10000530 for APC; the single modules return the small enums `ALGO_ANR_RET` / `ALGO_EQ_RET` /
`ALGO_AGC_RET` (0, 1, 2 …) [R: e.g. AGC Config returns 2 for a NULL handle, 0x12 for "Need init"]. The AGC's codes
follow `ALGO_APC_RET`'s order with GAIN_STEP after DROP_GAIN_MAX; the header's small `ALGO_AGC_RET` enum lacks GAIN_STEP
and is off by one from 10 on (§6.5).

### 6.2 Buffer sizes [R]

fftsize(256) = 0x178C (§2.6).
- `IaaAgc_GetBufferSize` = 0x3D0C + 0x178C; `IaaAnr_GetBufferSize` = 0xFFE8 + 0x178C; `IaaEq_GetBufferSize` = 0x2DB8 +
  0x178C.
- `IaaApc_GetBufferSize(sw)` (0x8278) = base + 0x178C, base = (eq ? (anr ? 0x119A4 : 0x4CB4) : (anr ? 0x10D7C : 0x408C)) +
  (agc ? 0x2A70 : 0); only `anr_enable`, `eq_enable` and `agc_enable` are read.

### 6.3 Init and checks [R: code and messages]

- Init (all five): NULL buffer / struct → NULL; `point_number` must be **128**; channel 1 or 2 ("only support mono or
  stereo"); sample rate: APC accepts 8000 / 16000 / 32000 / 48000 in code although its message says "8K 16K 48K"; chip
  gate (§2.2) per module (`[APC|ANR|EQ|AGC] … IC check pass. ver.…`); "FFT mode: floating point mode"; FFT length 0x100.
  `IaaApc_Init` / `Reset` copy `AudioApcBufferConfig` as a block to handle +0x4…+0x14 (dr at +0xC, vad at +0x10) and
  branch only on anr, eq and agc.
- ANR config: §6.7. EQ / HPF config: §6.6; "Please enable EQ or HPF" when Run is called with both off. AGC config: §6.5.
- Run: "Need config before …"; "handle is dirty" (a handle-consistency word mismatched) [R strings].

### 6.4 VAD and de-reverberation reach no code [R]

- `IaaApc_Config(h, anr, eq, hpf, vad, dereverb, agc)`: after `push` of ten registers the stack arguments are vad
  [sp+0x28], dereverb [sp+0x2c], agc [sp+0x30]; only [sp+0x30] is ever loaded (0x9150). The vad and dereverb pointers are
  never read — any value, NULL included, is accepted and ignored.
- `IaaApc_GetConfig`: loads only hpf [sp+0x28] and agc [sp+0x34] of its stack arguments; the vad and dereverb structures
  are left untouched.
- No load of handle +0xC / +0x10 (dr_enable, vad_enable) exists in any exported function, `Run` included, and
  `GetBufferSize` ignores them.
- libAPC has no VAD or de-reverberation code at all: no string, symbol or table naming them (`strings`, `.dynsym`).

`AudioVadConfig`, `AudioDereverbConfig`, `dr_enable` and `vad_enable` are header-only placeholders in this build.

### 6.5 AGC (`IaaAgc_*`, `AGC_Process`) [R unless tagged]

The AGC engine spans 0x1578…0x4300: internal functions 0x1578…0x2d90, then `AGC_Process` and the exports (§6).

Units: gains are kept in **half-dB** (ints in the configuration, floats in the running state).

**Config → instance.** `IaaAgc_Config`: NULL → 2; not initialised → 0x12. It copies the 0x4C-byte `AudioAgcConfig` to
handle +0x10 (the header layout), sets a busy word +0x1290 = 1, validates (0x2354) and configures one instance per
channel (0x283c; instances at handle +0x660 / +0x664; a failing second one prints "Channel 2 Config Fail!!"); success
writes {+0x1290, +0x1294} = {0, 1}. **On any failure +0x1290 stays 1**, so every later Config returns 0x11 (API
conflict) [R: no reset on the error paths]. Point number must be 128, 64 or 32 (error 4; message "%d or %d or %d"),
channels 1 / 2 (5), rate 8 / 16 / 48 kHz (3).

**Validation (0x2354)**, returned codes: cr_in[k], cr_out[k] each in −80…0 dB and both non-decreasing in k (8, prints
the offending pair); drop_gain_max ≤ 60 (9); gain_step 1…10 (10); release_time ≠ 0 (11); attack_time ≠ 0 (12);
noise_gate_db −80…80 (13); noise_gate_attenuation_db ≤ 100 (14); drop_gain_threshold −80…0 (15, message
"TargetPowerDB"); gain_max ≤ 60, gain_min −20…30, gain_init −20…60 (16). The messages name each field ("value error":
gain max / min / init, DropGainMax, AttackTime, ReleaseTime, compression ratio, NoiseGateDB, noise_gate_attenuation_db,
GainStep, TargetPowerDB).

**Instance fields (user_mode ≠ 0)**: +0x4F0 drop_gain_threshold (float); +0x500 its linear value from the `.data`
0x2E0C8 table {s32 dB, s16 round(32767·10^(dB/20))}, dB = 0…−80 (exact-match lookup); +0x50 / +0x54 / +0x58 / +0x5C =
2·gain_max, 2·gain_min, 2·gain_init, 2·gain_init (ints); +0x60 = 2·gain_init; +0x64 A := 0 and +0x68 G := (float)2·gain_init
(the two parts of the gain, below); +0x4B0…+0x4C8 cr_in[7], +0x4CC…+0x4E4 cr_out[7] (floats); +0x30 = noise_gate_db,
+0x2C = −|noise_gate_db|, +0x48C = (noise_gate_db ≥ 0); +0x4EC = noise_gate_attenuation_db; +0x70 = release_time, +0x74
= attack_time (frame counts), +0x78 = max(2·release, 63) + 1; +0x28 = max(drop_gain_threshold, noise_gate_db); +0x4A8 =
drop_gain_max, +0x4AC = gain_step. Further running fields: +0x6C gain at the start of the frame, +0x40 / +0x44 upper /
lower bound of A, +0x494 "G enabled", +0x510 / +0x514 up / down counters, +0x4F4 gate counter, +0x1C "gated" flag, +0x7C
"processed" flag, +0x34 drop flag, +0x50C / +0x520 input / gained peak, +0x4F8 rms, +0x4FC level L, +0x524 samples per
sub-frame.

**user_mode = 0 defaults**: curve in {−50, −30, −20, −10, −3, −3, −3} → out {−30, −20, −15, −5, −5, −3, −3} dB (`.rodata`
0x16F78 / 0x16F88), target −3 dB (+0x4F0, +0x28), noise gate −50 dB, attack = release = 1, +0x78 = 64, gain limits {40,
0, 0, 0} (half-dB), drop_gain_max 12, gain_step 1.

**Per frame (`AGC_Process` 0x2f38)**:
1. NULL instance → `AGCinst working buffer is NULL!`, returns −1.0. +0x7C := 0, +0x1C := 0. The n = +0x524 input
   samples are copied twice: B (+0xD34, to be gained) and C (+0x534, kept).
2. +0x6C := G; gi = (int)G (truncated); f = 2^(0.0830011877·gi) = 10^(gi/40) (`exp2`).
3. B[i] = sat16((int)(B[i]·f)) (truncation toward zero, NEON in blocks of 8 plus a scalar tail); when gi = 0 B is only
   saturated (a copy).
4. Peaks: +0x520 = max |B| (gained), drop flag +0x34 = (some |B[i]| ≥ (int)+0x500, the linear drop threshold); +0x50C =
   max |C| (input). n ≤ 0 → both peaks 0, flag 0.
5. Δ = level step 0x16f0(C) — the level is measured on the **input**, not on the gained signal.
6. Gain update 0x1fdc(Δ).
7. Noise-gate attenuation: if noise_gate_attenuation_db (+0x4EC) > 0 and the frame was gated (+0x1C = 1), G :=
   (float)(int)((−80 − L)·att·0.01), L = frame level (+0x4FC) — this **overrides** whatever steps 5–6 did this frame.
8. +0x6C := G, +0x7C := 1; the function returns G. The gained B is the output.
The new G applies from the **next** frame (step 2 uses the G of the previous call).

**Level step 0x16f0**: L = 20·log10(rms·(1/32767)) with rms = √(Σx²/n) over C (1/32767 = 3.0518509·10⁻⁵).
- **L > noise_gate_db (+0x30)**: T = curve(L), +0x28 := T; return Δ = 2·(T − L) − G.
- **L ≤ gate**, in this order: if +0x48C ≠ 0 (noise_gate_db ≥ 0) → nothing. Else gate counter +0x4F4 += 1; if G < 0 and
  counter > attack_time (+0x74): counter := 0, t = G + A + 1; else if G > 0 and counter > release_time (+0x70):
  counter := 0, t = G + A − 1; else (G = 0, or the counter not yet over) nothing. When t was formed it is split (below).
  Every gated path ends with +0x1C := 1 and returns Δ = 0. The gain thus steps by one half-dB toward 0.
- **curve(L)** 0x1578: 7 points (cr_in[k], cr_out[k]) at +0x4B0 / +0x4CC: L ≤ in₁ → extrapolate the segment 0–1;
  in₁ < L ≤ in₂ → segment 1–2; … ; L > in₅ → segment 5–6 (also extrapolated beyond in₆); i.e. linear interpolation
  out_k + (out_{k+1} − out_k)/(in_{k+1} − in_k)·(L − in_k). Result raised to at least noise_gate_db (+0x30). Equal
  neighbouring cr_in values divide by zero (the validation allows them: only non-decreasing is checked) [R; I effect].

**Gain update 0x1fdc(Δ)**:
1. Counters: Δ ≥ 0.01 → down := 0, up += 1. Δ ≤ −0.01 → up := 0, down += 1. Otherwise both := 0.
2. **Up step** if up ≥ release_time (+0x70): t = A + G + min(gain_step, Δ) (gain_step +0x4AC, float, half-dB); split(t);
   up := 0.
3. **Down step** if down ≥ attack_time (+0x74): t = A + G + max(−gain_step, Δ); split(t); down := 0. (Only one of 2 / 3
   can fire in a frame: attack_time ≥ 1 is validated.)
4. **Drop limiter** if the drop flag +0x34 is set: P = 20·log10((max|C| + 1.5258789·10⁻⁸)/32768) clamped to [−80, 0];
   Δp = 2·(drop_gain_threshold (+0x4F0) − P) − G_start (+0x6C); Δp = max(Δp, −drop_gain_max (+0x4A8, truncated to an
   integer for the compare)); t = A + G + Δp; split(t); up := 0. P is the **input** peak (+0x50C), while the flag was
   raised by the **gained** signal.
5. **split(t)**: if t ≥ U (+0x40) → A := U, d = t − U; else if t ≤ Lw (+0x44) → A := Lw, d = t − Lw; else A := t, d = 0.
   Then, if +0x494 = 1: G := max(2·gain_min (+0x54), min(2·gain_max (+0x50), d)); else G := 0.

What the split reduces to: +0x40…+0x4C are zeroed by every configuration path (0x2974, 0x2b30, 0x1db0) and written
nowhere else; +0x494 is set to 1 by both init paths (0x2d38, 0x2e98) and never cleared. So A is always 0, d = t, and G
simply becomes clamp(t, [2·gain_min, 2·gain_max]); the A / U / Lw mechanism and the "G := 0" branch are vestigial [I:
remnants of a two-stage gain whose second stage was removed].

Fields written but never read in the AGC engine: +0x78, +0x7C, +0x60, +0x58 / +0x5C, +0x520 (gained peak, only stored).

**`IaaAgc_Run`** splits the frame into instance sub-frames (+0x524 samples) and processes the channel 1 / channel 2
instances, copying the gained B out. The build-option string `AGCSET##REDUCE1PEAKEST1DGAINSTEP2BYPASS5STEREOCOSTDOWN0##SETAGC`
records the compile options.

**Unreferenced per-field set-up API** (no call, no data reference; all take the instance as r0):

| address | effect |
|---|---|
| 0x1d7c | set gains (max, min, init): A := 0, +0x40…+0x4C := 0, +0x50 = 2·max, +0x54 = 2·min, +0x58 = +0x5C = +0x60 = 2·init, G = (float)2·init |
| 0x1ddc | set curve (short in[7], short out[7]) → floats +0x4B0… / +0x4CC… |
| 0x1ea0 | set noise-gate attenuation → +0x4EC (float) |
| 0x1eb4 | set noise gate dB: +0x30 = (float)db; db ≥ 0 → +0x2C = −db, +0x48C = 1; db < 0 → +0x2C = db, +0x48C = 0 |
| 0x1ee0 | set attack → +0x74 |
| 0x1ee8 | set release → +0x70, +0x78 = max(2·n, 63) + 1 |
| 0x1f04 | set drop_gain_max → +0x4A8 (float) |
| 0x1f18 | set gain_step → +0x4AC (float) |
| 0x1f2c | peak / count of two buffers — the same loop as `AGC_Process` step 4, inlined there instead |

### 6.6 EQ and HPF (`IaaEq_*`; config 0xda80 EQ / 0xd40c HPF, apply 0xd344 EQ / 0xcf68 HPF) [R]

- **Framing**: 256-sample analysis window with a 128-sample hop (history in the handle), window multiply, real FFT (0xe040
  wrapper, 256 points), per-bin gains, inverse FFT, window and overlap-add; mono or 2 channels (one state each).
- **EQ config** (0xda80): eq_enable and user_mode must be 0 / 1 (codes 6 / 7). Each of the 129 `eq_gain_db` values
  becomes a Q8 linear gain round(256·10^(g/20)) (via exp2); g > 20 → a WARNING message and 0x0A00 (+20 dB), g < −100 → a
  message and 1 (1/256). Stored as s16 at +0x30A…, as floats at +0x4…; applied (0xd344) as X[k] *= gain[k]/256 (a real
  gain on both parts).
- **HPF config** (0xd40c): the cut-off index (0 = 80, 1 = 120, 2 = 150, 3 = 500 Hz) and the rate (8 / 16 / 32 / 48 kHz,
  else code 3 and an "unsupported sample rate" message) pick one of 16 tables of 129 s16 in Q8: 1 (≈ −48 dB) or 0 below
  the cut-off bin, 256 above. Copied as floats to +0x40C…+0x60C. The table values are in [`audio-filter-tables.tsv`](../data/audio-filter-tables.tsv).
- **HPF apply** (0xcf68): for cut-offs 0–2, X[k] *= table[k]/256. For the 500 Hz setting the mask is chosen per frame:
  K = first bin ≥ 9 where the table is non-zero; per bin L_k = 20·log10(|X_k|/32768²·…) dBFS (NaN → −120) — and **each
  bin's level is printed to stdout with `printf("%d\t%f\n")`, 128 lines per frame** (a debug print left in the release
  build); mean low (bins < K), mean high, max low, overall peak bin. Peak above K: if mean low < −40 and mean high − peak
  < −65 → pass everything (mask 1); else low bins 0, high bins 1. Peak at or below K: if mean high − max low < −75 and
  max low > −70 → mask 0 everywhere; else if < −80 and mean low > −30 → mask 0.01 everywhere; else the table/256.

### 6.7 ANR (`IaaAnr_*`; intensity → per-bin parameters 0x12d7c, config 0x13494, core 0x13c10) [R unless tagged]

The core is one function 0x13c10…0x16f04 with out-of-line blocks (Ghidra addresses 0x22d7c / 0x23494 / 0x23c10).

**Config (0x13494)**: anr_enable 0 / 1 (code 6), user_mode 0…2 (7). The range checks — every intensity 0…30 (−1; message
"NR mode must between [0, 30]"), every band edge 0…128 (−1), smooth_level 0…10 (9), converge_speed 0…2 (10) — run **only
when enable = 1 and user_mode ∈ {1, 2}**; with user_mode 0 (or enable 0) the user's edge and intensity arrays are still
applied, unchecked. smooth_level → +0x4E20 and 1/(smooth_level + 1) at +0x4E24 (user_mode 0: 10 and 1/11);
converge_speed 0 / 1 / 2 → +0x4E44 = 3 / 2 / 1 (user_mode 0: 2) — the mapping is inlined at 0x1354c; the setter 0x16f4c
(converge_speed → 2 / 1 / 3) is an unreferenced copy.

**Intensity → per-bin parameters (0x12d7c)**: bands are bins [edge_{b−1}, edge_b) for the 6 `anr_intensity_band` edges
(bin 128 included in the last band; an edge not above the previous one is skipped); each band's intensity m gives two
per-bin values, A (+0xC60, 129 floats) and B (+0xE60):

| m | A (overdrive) | B (gain floor) |
|---|---|---|
| 0 | 1.0 | 0.5 |
| 1…9 | 1.0 | 0.5 − 0.025·m |
| 10…19 | 1.0 + 0.01·(m − 10) | 0.25 − 0.0125·(m − 10) |
| 20…30 | 1.1 + 0.015·(m − 20) | 0.125 − 0.0124·(m − 20) |

m > 30 prints `NR mode must between [0 , 30].` and skips that band. The 7 intensities are also kept at +0x18…+0x30.

**Identification [R constants; S source; I match, strong].** The core is WebRTC's floating-point noise suppressor
`ns_core.c` (`WebRtcNs_AnalyzeCore` + `WebRtcNs_ProcessCore` merged, working on the spectrum APC's Run has already
computed; webrtc-audio-processing v0.3.1 `modules/audio_processing/ns/`). Every NS constant is present: quantile
estimator `FACTOR` 40, `QUANTILE` 0.25, `END_STARTUP_LONG` 200 (compares with 199 / 200 at 0x148a0, 0x1499c, 0x149a8,
0x14ab8, 0x16354, 0x1659c), `HIST_PAR_EST` 1000 (0x15168, 0x15204, 0x16994; float 1000 at 0x1682c…), `DD_PR_SNR` 0.98,
`SPECT_FL_TAVG` / `SPECT_DIFF_TAVG` 0.3, `PRIOR_UPDATE` 0.1, `NOISE_UPDATE` 0.9, `SPEECH_UPDATE` 0.99, `GAMMA_PAUSE`
0.05, `WIDTH_PR_MAP` 4 and 2·4 = 8 with the **three `tanhf` calls** of `SpeechNoiseProb` (0x1560c, 0x15684, 0x156b0),
`minSpecDiff` 0.16, the 0.0001 regulariser, `B_LIM` 0.5 with the energy-gain factors 1 + 1.3·(g − 0.5) and
1 − 0.3·(0.5 − g) (0x16470…0x164d4), `kStartBand` 5 (ln 5 = 1.6094379 at 0x167c8) and ln 1000 = 6.9078 (0x16730).

**Gain rule**: per bin k the Wiener-type gain **G = ξ / (A_k + ξ)**, **clamped to [B_k, 1]** — NS's
`ComputeDdBasedWienerFilter` (`snrPrior/(overdrive + snrPrior)`) and its `denoiseBound` clamp, with A = overdrive and
B = denoiseBound per bin [R formulas; S names]. The a-posteriori SNR is γ = P_k / (N_k + ε); the a-priori SNR ξ, as read
in parts, is γ − 1 when γ ≥ 4 and otherwise follows its previous value with a smoothing step only while the change is
< 4 — a decision-directed variant [I]. The intensity table passes exactly through NS's policies: m = 0 → (1.0, 0.5) =
mode 0, m = 10 → (1.0, 0.25) = mode 1, m = 20 → (1.1, 0.125) = mode 2; m = 30 → (1.25, 0.001) has mode 3's overdrive 1.25
but a floor of 0.001 instead of 0.09 [R table; S policies]. MStar turned NS's four global modes into a per-band
continuous 0…30 scale [I].

**Start-up noise model** [R loop; S formula]: in the magnitude loop (0x141ac…0x14290, unrolled ×2) magn[i] = √(re² +
im²) + 2⁻¹⁶ (NS: + 1.0; the spectrum is scaled by 2⁻¹⁶ before, 0x13f7c), signalEnergy and sumMagn are accumulated, and
for i ≥ 5 (`cmp r7, #4`) four `logf` calls give log i and log magn[i] into sum_log_i, sum_log_i², sum_log_magn,
sum_log_i·log_magn — NS's start-up sums. The pink-noise fit that follows is NS's least-squares line of log magn over
log i (numerator = intercept, exponent = −slope, clamped ≥ 0 and to [0, 1]) giving `parametricNoise[i] = num / i^exp`
(bins < 5 use 5; the single `pow` at 0x15500 and `exp` / `expf` at 0x152d8, 0x167c0, 0x167d4), blended with the
quantile estimate during the start-up of 200 frames [S: NS AnalyzeCore; I: the binary's blend follows it — the per-step
arithmetic of this block is not derived]. Constants present in the binary but **not** in NS: ln 65536 = 11.0903549
(0x152b8, in the parametric-noise block — the 2¹⁶ input scale moved into the log domain [I]), and 11.4929695,
−2.32263041, 98.0198059, 0.000932693481, −13.8146677, 0.00100088795 (loaded together in two blocks, 0x15708 and 0x16168)
— their role is not known. Other constants whose role is not known: doubles 0.4 (0x143b8), 0.7 (0x1487c, 0x157ec), 0.3
(0x152ec), 0.1 (0x156c0), 1/6 (0x157f4), 0.9 (0x157f8), 0.05 (0x157fc). NS's `END_STARTUP_SHORT` (50) has no literal in
the core; whether the start-up blend uses another length (the 120 of the silence counter, below) is not known.

**Quantile noise estimate** [R landmarks; S]: `logf` of the magnitudes (0x14818), the FACTOR / density update with 40
and 0.1, three simultaneous estimates with counters wrapping at 200 (the 0xc7 compares), `expf` of the log quantile
(0x149c8) — NS's `NoiseEstimation`.

**MStar additions (not in NS)**:
- **smooth_level** (+0x4E20, with 1/(smooth_level + 1) at +0x4E24): after the Wiener gains are formed, a **moving
  average across frequency**: half-width h = smooth_level >> 1, a running sum that adds bin k + h and subtracts bin
  k − h − 1, multiplied by +0x4E24 (0x15b14…0x15c3c; edges handled by separate branches). smooth_level 0 (the Init value)
  leaves the gains unchanged; user_mode 0 sets 10 (an 11-bin average). It is not a time smoothing.
- **converge_speed** (+0x4E44 = 3 / 2 / 1 for 0 / 1 / 2, Init 1): the period of a frame counter +0x4E40 incremented at
  the top of every core call and wrapped to 0 when it reaches +0x4E44 (0x13c3c…0x13c68); +0x4E48 counts all calls. Where
  the counter's phase gates an update is not known (it is not re-read through r0 + 0x4000) [I: it paces a noise /
  parameter update every 1, 2 or 3 frames].
- **Upper-band hold** +0x4E2C (Init 129 = no effect): gains of bins ≥ +0x4E2C are set to the gain of bin +0x4E2C − 1
  (0x15a68…0x15b04). Only Init writes it; the unreferenced setter 0x16f08 would set it to 129 (argument 0) or
  max(250 − 10·v, 10).
- **Silence counter** +0xA58: at the end of each call, if a spectrum energy (Σ(re² + im²) with the 2⁻²⁴ scale; [I] of the
  processed frame) is < 1.0 the counter increments, else it resets to 0 (0x14740…0x14784). An all-zero input spectrum
  returns 0 at the top without touching any state (0x13d58, as NS's `energy == 0` early return). After more than 120
  such frames, every per-bin value at +0x64C… is relaxed toward 1 each frame (x = 0.99·x + 0.01, 0x13d78…0x13e48) and
  the post-processing takes an alternative branch with 0.99 / 0.08 / 0.003 constants (0x15c9c → 0x1650c) [R mechanism;
  I: recovery of the suppressor after long silence]; that branch's arithmetic is not derived.
- **Low-level detector** +0x4E30: a 16-bit history shifted left each frame, a bit set when +0x4E38 < 0.1; when the low
  16 bits are all ones (16 consecutive such frames) a factor is clamped to ≤ 0.5 and multiplied in, otherwise +0x4E34 :=
  1.0 (0x15cb4…0x15d20) [R; I role].

**Init values** (0x13100…0x13134): anaLen +0x10 = 256, magnLen +0x14 = 129, +0x4E28 = 0 (when ≠ 0 the core skips the
smoothing / hold block, 0x15a48), smooth_level 0, hold 129, counters 0, converge period 1.

Not derived: a stage-by-stage arithmetic comparison of the core with `ns_core.c` (the feature histograms of
`FeatureParameterExtraction`, the weights of `SpeechNoiseProb`, `UpdateNoiseEstimate`, the start-up blend).

### 6.8 `IaaApc_Run` chain and the result taps [R]

Per 128-sample hop and channel: window + 256-point FFT (0xe040) → **ANR** (0x13c10) → the NR spectrum is saved (+0xC98 +
ch·0x102 floats) → **EQ** gains (0xd344) → **HPF** mask (0xcf68) → inverse FFT and overlap-add → the time signal is copied
to +16000 → **AGC** (`AGC_Process`, per instance and sub-frame) with gain interpolation (`exp2`) → output in place.
`IaaApc_GetNrResult` inverse-transforms the saved NR spectrum on demand (NR-only signal); `IaaApc_GetNrEqResult` copies
the +16000 buffer (NR + EQ / HPF, before AGC); both require the configured flag (+0x4088).

## 7. libAED_LINUX.so — acoustic event (baby-cry) and loud-sound detection
<!-- ev:audio-algorithms.7 -->

The detector works on mono 8 kHz audio in 256-sample (32 ms) frames; for other
rates and for stereo it converts the caller's buffer in place first (§7.3).

### 7.1 API, buffer, Init [S header; R binary]

```c
#define AED_API_VERSION {'1','1'}
typedef struct { unsigned int point_number; unsigned int channel; } AedProcessStruct;
typedef enum { AED_SRATE_8K = 8000, AED_SRATE_16K = 16000, AED_SRATE_32K = 32000, AED_SRATE_48K = 48000 } AedSampleRate;
typedef enum { AED_SEN_LOW, AED_SEN_MID, AED_SEN_HIGH } AedSensitivity;
unsigned int IaaAed_GetBufferSize();
AedHandle    IaaAed_Init(char *working_buf_ptr, AedProcessStruct *aed_struct);
ALGO_AED_RET IaaAed_Config(AedHandle handle);
ALGO_AED_RET IaaAed_Run(AedHandle handle, short *audio_io);
ALGO_AED_RET IaaAed_GetResult(AedHandle handle, int *aed_result);
ALGO_AED_RET IaaAed_SetSensitivity(AedHandle handle, AedSensitivity sensitivity);
ALGO_AED_RET IaaAed_SetSampleRate(AedHandle handle, AedSampleRate srate);
ALGO_AED_RET IaaAed_SetOperatingPoint(AedHandle handle, int operating_point);
ALGO_AED_RET IaaAed_SetVadThreshold(AedHandle handle, int threshold_db);
ALGO_AED_RET IaaAed_SetLsdThreshold(AedHandle handle, int threshold_db);
ALGO_AED_RET IaaAed_RunLsd(AedHandle handle, short *audio_input, int *lsd_db);
ALGO_AED_RET IaaAed_GetLsdResult(AedHandle handle, int *lsd_result);
ALGO_AED_RET IaaAed_Release(AedHandle handle);
/* 0x10000201 INIT_ERROR, 202 INVALID_CONFIG, 203 INVALID_HANDLE, 204 INVALID_SAMPLERATE, 205 INVALID_POINTNUMBER,
   206 INVALID_CHANNEL, 207 INVALID_SENSITIVITY, 208 INVALID_CALLING, 209 API_CONFLICT (the header comments call the
   module "BCRY") */
```

- `GetBufferSize` = 0x5D60 + the private APC's size for {anr = 1, eq = 0, agc = 1} (set-up VA 0x5498), printed as
  `[AED] Working buffer size: %u bytes` [R].
- `Init(buf, s)` (VA 0x5188): chip gate (failure → `Warning!! IC version check wrong, disable AED Lib`, NULL); prints
  ver.2020.1127.0000; memset 0x5D60; FFT length 0x200 (512) with Ooura's set-up (ip[0] := 512 at +0x80C, w at +0x140C);
  copies point_number / channel; builds the private APC (§7.7) at +0x5D60 for 256 points, 1 channel, 8000 Hz; DCT matrix
  (§7.4); then defaults: sample rate 8000, sensitivity MID (1), operating point −5, VAD threshold −40 dB, LSD threshold
  −15 dB; internal NR on, internal AGC on; +0x3D00 = 10 (window shift, §7.3); prints `ANN classify` [R]. Init works in
  the caller's buffer [R].
- Frame length is tied to the rate: 256 samples at 8 kHz, 512 at 16, 1024 at 32, 1536 at 48 kHz (32 ms frames) — the
  `Input point number for N kHz should be …` messages [R].
- State machine as libSSL's, with the messages "Init must called before Config", "Config must called before Run",
  "Config is being called", "GetResult is being called" (busy → 0x10000208 / 209) [R].
- **Config** (`IaaAed_Config`, VA 0x3d98): channel must be 1 or 2 (`[AED] Error setting channel`, 0x10000206); then the
  fixed configuration of the private APC (§7.7).

### 7.2 Sample rate and the private SRC [R]

`SetSampleRate`: for 16 / 32 / 48 kHz input a private SRC instance (a copy of the §3 code, revision SRC 8483e, §2.1;
`SRCStructProcess` {rate 16 / 32 / 48, mode 3 / 6 / 9 = → 8 kHz, order HIGH, 1 channel, point 512 / 1024 / 1536}) is
`malloc`ed and its pointer kept at +0x451C; failure → `Error initializing SRC`, 0x10000201; other rates → 0x10000204.
Calling it again allocates a new block without freeing the old one, and `Release` frees nothing (it only clears a flag) —
the SRC memory leaks [R: no `free` on these paths]. Run reaches the instance through +0x4518 (§7.3 step 4); the two
offsets are as read by the two sources, and how +0x4518 and +0x451C relate is not settled.

### 7.3 Run front end (`IaaAed_Run`, VA 0x4474) [R]

1. NULL handle → `[AED] The handle is null.`, 0x10000203. **Not initialised (+0x5D50 = 0) → `exit(1)`** — the library
   terminates the process. State checks (§7.1).
2. Stereo: (L + R)·0.5 truncated to s16, written over the first n samples of the **caller's buffer** (n = 256·rate/8000).
3. Frame length must be 256 / 512 / 1024 / 1536 for 8 / 16 / 32 / 48 kHz; otherwise the message of §7.1 and **`exit(1)`**
   (not an error code).
4. Rate ≠ 8000: the private SRC (`IaaSrc_Run` copy VA 0x85c4, instance +0x4518) converts to 256 samples at 8 kHz into
   +0x3D18, which is copied back over the first 256 samples of the caller's buffer.
5. The private APC (`IaaApc_Run` copy VA 0x70c4, instance pointer +0x4520) processes those 256 samples **in place** —
   internal NR, then AGC (§7.7). The caller's buffer therefore returns modified (mono-mixed, resampled, noise-reduced,
   gain-controlled).
6. Framing: a 512-sample s16 history at +0x8 is shifted by 256 and the new 256 samples appended; converted to float at
   +0xC0C (S).
7. Pre-emphasis (VA 0xd0b8): y[i] = x[i] − 0.97·x[i−1] over the 512 samples, x[−1] = the sample 255 of the previous call
   kept in `.bss` 0x2C5EC — **one memory per process**, shared by all instances.
8. S is copied to D (+0x2190); S[i] ·= w[i]/2¹⁰ with w = a periodic 512-point Hamming window in Q10
   (1024·(0.54 − 0.46·cos(2πi/512)): w[0] = 81.92 (literal at 0x48c8), w[1…511] at `.rodata` 0x13508, peak 1024 at
   i = 256); D[i] /= 2⁵ (the shifts come from +0x3D00 = 10).
9. Level (VA 0x9e90): E = Σ D[i]² over i = 256…511 (the new hop); RMS = √((E + E_prev)/512) with E_prev in `.bss`
   0x2BDA8 (process-global).
10. Loud-environment counter `.bss` 0x2BDAC: RMS > 100 → +1, else −1 while > 0 (§7.5).
11. **Active frame** iff RMS > the VAD threshold +0x218C (`SetVadThreshold`, §7.8).

### 7.4 The 13 per-frame tracks [R unless tagged]

For an active frame:
- S goes through a 512-point real FFT (VA 0xb0e8, called as rdft(512, 1, S, ip = +0x80C, w = +0x140C)) — Ooura's `rdft`
  signature, and libAED carries Ooura's π/4 constant (0xb1a8, 0xb388) [R call shape and constant; I Ooura].
- **MFCC** (VA 0xd1bc): for FFT bins k = 1…255 (DC and Nyquist unused), |X_k| = √(re² + im²) — magnitude, not power — is
  split between two neighbouring triangular filters: w_k·|X_k| to filter b_k − 1 and (1 − w_k)·|X_k| to filter b_k
  (filters 0…25; b_k = 26 feeds only filter 25). b_k: bytes at `.rodata` 0x18E5A… (bin 1 uses 0); w_k: floats at `.data`
  0x2B8E0 (255). Filter b rises from the bin where b_k becomes b: 47, 78, 125, 172, 219, 266, 328, 391, 469, 547, 641,
  734, 828, 953, 1078, 1219, 1375, 1531, 1719, 1922, 2141, 2391, 2656, 2953, 3266, 3625 Hz for b = 1…26 (bin·8000/512) —
  a 26-band mel-spaced bank over 0–4 kHz (full table: [`audio-aed-mel.tsv`](../data/audio-aed-mel.tsv), one row per FFT bin: its filter and weight). Each filter energy e_b → ln(max(e_b, 1)).
  Then 12 coefficients c_j = √(2/26)·Σ_b ln e_b · cos(j·(b + 0.5)·π/26), j = 1…12 (DCT-II without c₀; matrix built by
  VA 0xd10c at Init into +0x1CA8, π/26 = 0.12083048 at 0xd1b8, √(2/26) = 0.27735 stored at +0x2188) → +0x1C78…+0x1CA4.
- The 13 tracks (buffer +0x29F8, track t at +0x29F8 + t·0x174, 93 floats each, column = segment frame index n):
  **t = 0…11: MFCC c₁…c₁₂; t = 12: ln(RMS)** (`log` in double). A segment holds up to 93 frames (0x5D).
- Per active frame: n += 1 (`.bss` 0x2BDB0), in-segment flags `.bss` 0x2BDB4 = 0x2BDB8 = 1, silence counter (s16 `.bss`
  0x2BDBC) := 0. **n = 93 → segment end.**

For an inactive frame: silence counter += 1 (wraps 94 → 0; nothing else reads it). If a segment is open: n ≥ 16 →
segment end; n ≤ 15 → the segment is **discarded** (tracks memset 0x12E4 bytes = 13·93·4, n and the flags := 0). A single
inactive frame closes a segment.

### 7.5 Segment end: statistics, features, hold [R]

1. Each of the 13 tracks is sorted ascending over its n values (quicksort VA 0xa248, (array, n, 0, n − 1)).
2. **Statistic 1 (VA 0xa330) = median**: a[(n−1)/2] for odd n, (a[n/2 − 1] + a[n/2])/2 for even n.
   **Statistic 2 (VA 0xa358) = interquartile range**: h = ⌊n/2⌋; Q1 = median of a[0…h−1], Q3 = median of the last h
   values a[h + (n mod 2) … n−1]; returns Q3 − Q1. The caller multiplies it by **0.7413** (= 1/1.349, `.rodata` 0x4940) —
   the IQR-based robust standard deviation.
3. Features (+0x2990, 26 floats): for t = 0…11: f[2t] = median, f[2t+1] = 0.7413·IQR; t = 12 (ln RMS): **only**
   f[24] = 0.7413·IQR (its median is not used); f[25] = n (segment length, float). The 26 classifier inputs are thus the
   median and robust spread of c₁…c₁₂, the spread of the log level, and the length — matching the min / max table of
   §7.6 (feature 26 = 16…93).
4. Normalisation (VA 0x3b88) + MLP (VA 0x3be4) and the vote (§7.6); afterwards the tracks are cleared and the segment
   state reset (the same memset path).
5. **Detection hold**: once a detection is flagged (`.bss` 0x2BDC0), every call counts `.bss` 0x2BDC4; above 469 calls the
   vote history is refilled with the operating-point threshold and both words cleared (≈ 15 s at 32 ms per call [I]).
6. **Loud-environment warning**: when a call ends with no detection, no open segment and the §7.3 step-10 counter > 312,
   `Warning! There is loud sound in the baby environment!` is printed and the counter cleared; a detection clears it too.

### 7.6 Classifier and decision [R unless tagged]

- **Classifier** (VA 0x3b88 normalisation + 0x3be4 network): 26 features, each normalised x' = 2(x − min)/(max − min) − 1,
  then an MLP **26 → 20 → 20 → 20 → 20 → 20 → 20 → 1** with tanh on every layer (computed as 2/(1 + e^(−2x)) − 1;
  `tanh` / `exp` imported); output in (−1, 1). Layer sizes at `.rodata` 0x10B38, biases 0x10B58 (121), weights 0x10D3C
  (2540, one row of inputs per neuron), feature min / max: the first pair from literals (−15.507, 5.0026), then max
  0x10A6C… and min 0x10AD4….
- **Sensitivity** (`SetSensitivity`): LOW → (window 5, votes 3), MID → (5, 2), HIGH → (2, 1) at +0x3CF4 / +0x3CF8; it also
  copies the operating-point threshold into the 5-slot output history.
- **Operating point** p (`SetOperatingPoint`) is clamped to −10…10 and stored as p/10 (+0x3D04, the classifier threshold)
  and into the history; prints `[AED] Operating point: %d`.
- **Decision**: an output > p/10 counts as a vote; the last `window` outputs are kept; votes ≥ `votes` → detection
  (+0x3CFC = 1, the history is reset to the threshold, then the hold of §7.5). `GetResult` returns 1 for a detection
  frame.

### 7.7 The private APC: internal NR and AGC [R]

- Built by Init: private APC instance at handle +0x5D60 (pointer +0x4520), `IaaApc_Init` copy (VA 0x58b4) with
  {point_number 256, 1 channel, 8000 Hz} and switches {anr 1, eq 0, dr 0, vad 0, agc 1}; its size is part of
  GetBufferSize. Words +0x4528 / +0x452C ("internal NR" / "internal AGC") are set to 1 and only read to print
  `[AED] Internal NR is on` / `[AED] Internal AGC is on` (the "off" texts are unreachable: nothing else writes them).
- Configured on every `IaaAed_Config` by the private `IaaApc_Config` (VA 0x67b8) with stack-built structures in the
  **older** APC layout of this copy (APC 5fc59 / AGC 261f8, §2.1):
  - ANR (5 ints): enable 1, user_mode 2, intensity 20 (one value, not per band), smooth_level 10, converge_speed 1;
  - EQ: enable 0, user_mode 1 (the 129 gains are left as stack contents — unused since disabled);
  - HPF: enable 0, user_mode 0, cutoff 2 (150 Hz);
  - VAD, dereverb: NULL;
  - AGC (0x40 bytes: **5-point** curve, **no gain_step**): enable 1, user_mode 1, gain max 20 / min 0 / init 0,
    drop_gain_max 60, attack 1, release 1, cr_in {−70, −60, −30, 0, 0}, cr_out {−70, −45, −18, 0, 0} dB (`.rodata`
    0x134EC / 0x134F8), drop_gain_threshold −20, noise_gate_db −40, noise_gate_attenuation_db 0.
  Failure → `[AED] APC Config ERROR`, 0x10000202. Nothing else writes the private configuration, so it cannot be changed
  through the AED API.
- Order per call: mono mix → SRC → **NR → AGC** (inside the private `IaaApc_Run`) → framing / features.
- The private APC / AGC / SRC engines are older revisions of libAPC's / libSRC's (§2.1) [I: same family, older struct
  layout]; they are not compared with libAPC / libSRC instruction by instruction, only their set-up values are known.

### 7.8 Loud-sound detector and thresholds [R]

- `RunLsd` (VA 0x42c0): stereo input is mixed to mono **in the caller's buffer** ((L + R)/2 written over the first n
  samples); rms of the frame → dB by scanning the `.data` 0x2B384 table {dB, s16 level} (rms < 1 → −80); `*lsd_db` = that
  dB; detection if dB > the LSD threshold (+0x4D34, default −15).
- `GetLsdResult` holds the result: 1 on a detection and for the next 20 calls, then 0.
- `SetVadThreshold(dB)` stores round(10^(dB/20)·K) at +0x218C (default −40 dB; the scale K is not recorded);
  `SetLsdThreshold` stores the dB value; both print the value.

### 7.9 Process-global state [R]

`.bss` words shared by all AED instances of a process: 0x2BDA8 E_prev, 0x2BDAC loud-environment counter, 0x2BDB0
segment length n, 0x2BDB4 / 0x2BDB8 in-segment flags, 0x2BDBC silence counter, 0x2BDC0 detection flag, 0x2BDC4 hold
counter, 0x2C5EC pre-emphasis memory. Two instances in one process therefore interfere [I].

## 8. libBF_LINUX.so — 4-microphone beamformer + VAD

Results marked **emulated** below come from running the library's own code under unicorn (image mapped, libc / libm stubbed, GOT relocations applied). Ne10 is linked whole
(0xad28…0x1d430, ARM and Thumb; 92 exported `ne10_*`) [R].

### 8.1 API (header `AudioBfProcess.h`) [S; R binary]

```c
#define BF_API_VERSION {'1','1'}
typedef struct { unsigned int point_number; unsigned int sample_rate; /* 8k or 16k */
                 unsigned int mic_distance; /* cm */ unsigned int channel; } AudioBfInit;
typedef struct { unsigned int temperature; int noise_gate_dbfs; int noise_supression_mode; int noise_estimation;
                 float output_gain; int vad_enable; } AudioBfConfig;
unsigned int IaaBf_GetBufferSize(void);
BF_HANDLE    IaaBf_Init(char *working_buffer, AudioBfInit *bf_init);
ALGO_BF_RET  IaaBf_Config(BF_HANDLE handle, AudioBfConfig *bf_config);
ALGO_BF_RET  IaaBf_Get_Config(BF_HANDLE handle, AudioBfConfig *bf_config);
ALGO_BF_RET  IaaBf_Run(BF_HANDLE handle, short *microphone_input, short *microphone_output, int *microphone_doa);
BF_HANDLE    IaaBf_Reset(BF_HANDLE working_buffer, AudioBfInit *bf_init);
ALGO_BF_RET  IaaBf_Free(BF_HANDLE handle);
ALGO_BF_RET  IaaBf_Setshape(BF_HANDLE handle, int shape);
/* 0x10000301 INIT_ERROR, 302 INVALID_CONFIG, 303 INVALID_HANDLE, 304 INVALID_NOISE_ESTIMATION, 305 INVALID_VAD_ENABLE,
   306 INVALID_OUTPUT_GAIN, 307 INVALID_NOISE_SUPMODE, 308 INVALID_INPUT_POINTER, 309 INVALID_SAMPLERATE,
   310 INVALID_POINTNUMBER, 311 INVALID_CHANNEL, 312 INVALID_CALLING, 313 API_CONFLICT, 314 INVALID_GEOMETRY_TYPE */
```

`IaaBf_GetBufferSize` = 0xE8274 + fftsize(512) = 0xE8274 + 0x2C8C = **962 304 bytes** (§2.6), printed as
`Working buffer size = 962304 bytes` [R]. The `IaaVad_*` exports are §8.10.

### 8.2 Init, Reset, Free, Get_Config [R]

- `IaaBf_Init(buf, init)` in order: NULL → `The working buffer or init parameter is NULL!!`, NULL; chip gate 0x7b10 ≠ 1 →
  `[BF] Platform/License check failed!!`, NULL; prints `IC check pass` and `ver.2021.0730.0000`; Ne10 init 0x10dbc
  (result ignored); memset of GetBufferSize bytes; point_number ≠ 128 / channel ≠ 4 → message (`This library only support
  %d Microphones!!` for the channel), NULL (after the memset). Then h+0 = 128, h+4 = fs (not checked here — Config checks
  it), h+8 = mic_distance/100.0 (u32 → float, metres), h+0xC = 4, h+0x14 = −80, h+0x18 = lin(−80) = 1, h+0x1C = 1
  (noise_estimation), h+0x20 = 0.7 (0x3F333333, output_gain), h+0x24 = 0 (vad_enable), h+0x4C = 0x30513057 (the same
  magic word as libSSL's), h+0x2878 = 512 (FFT length 0x200), h+0xD194C = 8.0 (the post-filter bias β before the first
  Config, as if mode 80), the four post-filter bin indices (§8.8), the VAD instance (§8.10), the FFT context (0x4940;
  failure → `Initialization fail!`, NULL), Φ[k] = I (§8.6), B+0x2828 = 1.0 and its copy B+0xF378 (§8.8), h+0xD2960 =
  √(2Σw²) (§8.4), h+0xE1CE0 = 1 (initialised), h+0xE1CE4 = 2 (not configured), h+0xE1CE8 = 2. The speed of sound h+0x28
  and the mic coordinates stay 0 until Config / Setshape.
- dB → linear 0x4970: an 81-entry `.data` table at 0x32388 of {int dB, s16 value} for 0…−80 dB, value ≈ 32767·10^(dB/20)
  truncated (0 → 32767, −10 → 10361, −20 → 3276, −40 → 327, −60 → 32), the last entries flattened (−77 … −80 → 3, 2, 1,
  1); above 0 → 32767, below −80 → 1; exact match otherwise (the argument is an int). The gate is therefore in int16
  sample units, compared with the RMS of channel 0 (§8.5 step 6).
- `IaaBf_Reset(buf, init)`: NULL → message, NULL; requires h+0xE1CE0 = 1 (`Please call IaaBf_Init first!`); memset of
  0xE8274 bytes (everything except the Ne10 configuration), then the Init assignments without the chip gate, the version
  prints, the Ne10 init and the VAD Init / Config (consequence in §8.10). Config and Setshape must be called again
  (h+0xE1CE4 = 2, coordinates 0).
- `IaaBf_Free(h)`: only clears the FFT context's init flag (0x8124); NULL → 0x10000303. Nothing is freed (the handle is
  the caller's buffer).
- `IaaBf_Get_Config`: returns temperature, noise gate dB, (int)(h+0xD194C·10) — the mode as configured, or 80 before the
  first Config —, noise_estimation, output_gain, vad_enable.

### 8.3 Config and Setshape [R]

- **Config** (0x2fbc): the handle is dereferenced before any NULL check (a NULL handle crashes); config NULL → 302;
  noise_estimation > 1 → 304; vad_enable > 1 → 305; output_gain < 0 → 306; noise_supression_mode > **30** → 307 (the
  message says "between 0 and 15"); noise_gate_dbfs clamped to −80…0 and the clamped value **written back into the
  caller's struct**; magic, init and Run-busy checks (§8.5, end); rate 8 / 16 / 32 / 48 kHz (309); point_number 128 (310)
  and 4 channels (311). Messages also name array_geometry 0 / 1. Stored: temperature; speed of sound h+0x28 =
  (float)(331 + 0.6·T) with T converted as **unsigned** (a negative temperature gives a huge speed of sound); the noise
  gate's linear value (0x4970) at h+0x18; noise_estimation; output_gain; vad_enable; and β = h+0xD194C = **(float)
  mode/10** (0.0…3.0 — modes 0…30 stay distinct). Emulated: T = 20 → c = 343.0, mode 10 → β = 1.0.
- **Setshape** (0x34f0): shape 0 = uniform linear array, mic x = d·{−1.5, −0.5, +0.5, +1.5}, y = 0; shape 1 = four mics
  on a half circle of radius r = d/2 at 180°, 120°, 60°, 0° ((−r, 0), (−r/2, 0.866r), (r/2, 0.866r), (r, 0)); other
  values → 314. Coordinates at h+0x2C + 8m. (The manual [S] calls shape 1 a "uniform circular array"; the coordinates are
  a half circle.)

### 8.4 Overview: two cascaded STFT stages [R]
<!-- ev:audio-algorithms.8.4 -->

`IaaBf_Run(h, in, out, doa)` processes one hop of 128 samples per channel, 4 channels interleaved int16. Both stages use
a 512-point frame, hop 128 (75 % overlap), a **512-point symmetric Hamming window** 0.54 − 0.46·cos(2πn/511) as analysis
and synthesis window, and the Ne10 real FFT. Stage 1 is a 4-microphone **MVDR beamformer** with a fixed look direction
and a noise covariance learned from noise-only frames (§8.6, §8.7); its int16 output (written into `out`) is fed back as
the input of stage 2, a single-channel **spectral post-filter** (§8.8) whose output overwrites `out`.
- Window: the two `.rodata` tables at 0x1e9d8 and 0x1ee38 are byte-identical 256-float half windows; the code reads
  w[0..255] forwards and then backwards, giving the 512 points; they match the symmetric Hamming of length 512 to
  2.9·10⁻⁸. Σw² over the half table = 101.5389; Init stores √(2·Σw²) = 14.2505 at
  +0xD2960.
- FFT: the Ne10 wrapper 0x8054 of §2.6, context at h+0x2068 with a 257-complex work buffer and n = 512; configuration
  buffer at h+0xE8274 (0x2C8C bytes; set-up 0x199c0, init 0x7f9c via 0x4940).
- Matrix helpers, **emulated** against numpy: 0x7858 = 4×4 complex product C = A·B (row-major,
  interleaved re / im; error 5.5·10⁻⁷), built from `ne10_mulmat_4x4f` ×2 + `ne10_sub_vec4f` + `ne10_add_vec4f` on the
  real / imaginary parts; 0x75ac = 4×4 complex inverse (error 2–4·10⁻⁶), built from `ne10_invmat_4x4f`,
  `ne10_mulmat_4x4f`, `ne10_add_vec4f`, `ne10_invmat_4x4f`, `ne10_mulmat_4x4f`, i.e. the real-block
  formula M = (Aᵣ + Aᵢ·Aᵣ⁻¹·Aᵢ)⁻¹, A⁻¹ = M − i·Aᵣ⁻¹·Aᵢ·M [R call sequence; I formula]. It needs Aᵣ invertible; there is
  no check.

### 8.5 Run, step by step [R: disassembly 0x3764–0x40b4]
<!-- ev:audio-algorithms.8.5 -->

1. h+0xE1CE8 := 0 (Run-busy marker; set back to 1 at the end). Checks in this order: `in` or `doa` NULL → 0x10000308
   (`out` is not checked); magic h+0x4C ≠ 0x30513057 → 0x10000303; h+0xE1CE0 ≠ 1 → 0x10000301 ("call IaaBf_Init
   first"); h+0xE1CE4 == 2 → 0x10000312 (the state after Init / Reset until the first Config: **Run fails until Config
   has been called once**); h+0xE1CE4 == 0 → 0x10000313 ("Config is being called"; Config clears it on entry and sets 1
   on success).
2. VAD input: channel 0 of the hop (every 4th sample, h+0xC = 4 channels) is copied to a 128-sample mono buffer and
   `IaaVad_Run(h+0xE7E50, mono)` is called **unconditionally** (also when vad_enable = 0); its return value `v` is used
   in steps 6 and 10.
3. `doa[0..2]`: §8.7.
4. Frame counter h+0xDFCC0 += 1. Per channel c the 512-float history at h+0x5C + c·0x800 is shifted left by 128 and the
   new 128 samples are appended as floats **without scaling** (int16 value → float, NEON `vld4.16` de-interleave).
5. Energy gate: E = Σ x² over channel 0's whole 512-sample history; **E < 10⁻⁶ → return 0 without writing `out`** (the
   caller's buffer keeps its old content). Otherwise rms = √(E/512).
6. Analysis 0x49d8: per channel, window × history → FFT → X[k][c] stored at h+0x3C84 + 32k + 8c (k = 0…256, 4 complex
   per bin). Noise-update decision: vad_enable (h+0x24) = 0: update if the gate (h+0x18, linear int) > rms, else update
   if h+0xE1CD4 ≤ 23; vad_enable = 1: v ≠ 0 → no update, v = 0 → update if h+0xE1CD4 ≤ 23. h+0xE1CD4 is the 24-slot ring
   index (step 11 wraps it to 0 after it passes 23), so the `≤ 23` test always holds [R: its only writers are 0x3a14 and
   0x3eec]: **with vad_enable = 0 every frame updates the noise model and the noise gate has no effect; with vad_enable =
   1 the update runs exactly on frames the VAD calls non-speech** [I: consequence].
7. Update (only when decided): if h+0xE1CDC = 0, h+0xE1CD8 += 1; 0x51ec (noise model, §8.6); h+0xE1CD4 += 1; h+0xE5D2C
   += 1 (update count); 0x5a38 (MVDR weights, §8.7).
8. Beamforming 0x5e14: Y[k] = Σ_c W[k][c]·X[k][c] (plain complex product; W at h+0xCF12C, 32 bytes per bin) into
   h+0xCE924 (257 complex).
9. Synthesis 0x4af0(g1 = 1.0, g2 = 1.0): copy Y to the FFT ctx, inverse FFT, multiply by window × g1 × g2 → h+0x287C.
   Overlap-add 0x2720(gain = output_gain h+0x20): accumulate the 512 values into h+0xE14D4, output the first 128 × gain,
   saturated to int16 (> 32767 → 0x7FFF, < −32768 → 0x8000, else truncation toward zero), shift the accumulator by 128
   and zero its last 128. This **stage-1 output is written to `out`**.
10. Stage 2 input: `out` (128 int16) is converted to floats and appended to a second 512-sample history at h+0xD114C
    (shift by 128, new samples at h+0xD174C); windowed with the Hamming window divided by √(2Σw²) (h+0xD2960), FFT into
    h+0x287C and copied to h+0xD1950 (257 complex = the post-filter's input spectrum); the seven steps 0x8248, 0x83ac,
    0x8468, 0x8510, 0x87f0, 0x8968, 0x8cc4 run on the block at h+0xD114C (§8.8); synthesis 0x4af0(g1 = h+0xD2960,
    g2 = 1.0) of the spectrum at h+0xD2158; overlap-add 0x2720 into h+0xDFCC4 with gain **0.63 if v == 1, else
    output_gain × 0.63** (the latter in double) — **this overwrites `out`** with the final output.
11. h+0xE1CD4 > 23 → 0; h+0xE1CD8 > 23 → h+0xE1CDC := 1 (ring full); counter h+0xE7E4C += 1 (read only by the
    unreferenced 0x5f34, §8.11); h+0xE1CE8 := 1; return 0.

**Busy / state words** [R; **emulated**]: Run clears h+0xE1CE8 on entry and sets
it back only at the very end, so **every early return — a failed check, or the silent-frame return of step 5 — leaves it
0**; Config then refuses with 0x10000313 and, because Config clears h+0xE1CE4 on entry, the following Runs also return
0x10000313. Emulated: Run before Config → 0x10000312; Config → 0x10000313 (h+0xE1CE8 = 0); Run → 0x10000313; only a
Reset / Init (or a Run that reaches the end) clears the state.

**Level and delay** — **emulated** whole-library runs (Init, Config {20 °C, −40 dB, mode 10, noise_estimation 1, gain 1.0, vad 0}, Setshape 0, white noise of rms 300 per channel for 250 frames, then a 440 Hz tone of
amplitude 3000 common to all channels; output in):
- the first non-zero output comes at frame 4–6: 384 samples (512 − 128) per stage, 768 in total;
- each stage's synthesis multiplies by the window again without normalisation, so overlap-add gives Σₖw²(n + 128k) =
  1.5865 per stage; the fixed 0.63 of step 10 undoes one of them: overall gain 1.5865²·0.63 = 1.586 (× output_gain on
  speech frames, × output_gain² otherwise). Emulated: tone in rms 2121 → out 3320–3390 with v = 1;
- with v ≠ 1 the factor is output_gain² (stage 1 and stage 2), with v = 1 output_gain once — the level switches with the
  VAD decision when output_gain ≠ 1 [R code; the default output_gain 0.7 gives 0.441 vs 0.63 relative to the 1.5865²
  path].

### 8.6 Stage-1 noise model 0x51ec [R: disassembly 0x51ec–0x5a38; constants from the literal pools]

Per bin k (0…256) and channel c (0…3), with P = |X[k][c]|², n = h+0xE5D2C (number of earlier updates) and four
257×4-float arrays indexed 4k + c: λ (noise PSD) at h+0xE1CEC, S = G²·P at h+0xE2CFC, p̄ (smoothed speech-presence
probability) at h+0xE3D0C, R = G²·P/λ at h+0xE4D1C:
1. n = 0: λ := P (the code computes the running mean (P + n·λ)/(n + 1), which for n = 0 is P); the decision-directed
   prior starts at 0 and R := 0.
2. n ≥ 1: speech-presence probability with a fixed a-priori SNR ξ₀ = 10^1.5 = 31.6227766 (15 dB) and equal priors:
   p = 1 / (1 + (1 + ξ₀)·exp(−(ξ₀/(1 + ξ₀))·P/λ)) (constants 31.6227766 and 32.6227766, `exp` in double). Smoothing:
   p̄ := p when n = 1, else p̄ := 0.1·p + 0.9·p̄. Stagnation guard: if p̄ > 0.9 **and** p > 0.9, p is replaced by 0.9 in
   the next line (float constants 0.1000000238 / 0.8999999762). Noise update: λ := 0.8·λ + 0.2·((1 − p)·P + p·λ). This
   is the SPP-based "unbiased MMSE" noise PSD estimator of Gerkmann & Hendriks (2012) [I: the formula and the 15 dB /
   0.8 / 0.1–0.9 constants match it; the paper's 0.99 guard is 0.9 here].
3. Both cases continue: γ = P/λ; ξ = 0.98·R_prev + 0.02·max(γ − 1, 0) (decision-directed, R_prev = the value stored last
   time); G = ξ/(1 + ξ) (Wiener); S := G²·P; R := G²·P/λ.
4. The noise part of the channel vector: N[c] = (1 − G)·X[k][c]; its outer product N·Nᴴ (0x7858 with the row vector and
   its conjugate) is divided by its trace (complex division; |trace|² = 0 → 10⁻⁶ instead) → R̃ (4×4, trace 1).
5. Covariance Φ[k] (16 complex at h+0x5CA4 + 0x80·k), chosen by **noise_estimation** (h+0x1C):
   - 0: recursive, Φ := 0.1·R̃ + 0.9·Φ;
   - 1 (the Init default): moving sum over the last 24 updates, Φ := Φ + R̃/24, and once the ring is full (h+0xE1CDC = 1)
     Φ := Φ − R̃_old/24, where R̃_old is the matrix stored 24 updates ago in the ring at h+0xDD24 +
     8·(16·(257·slot + k) + j) (slot = h+0xE1CD4, 24 × 257 × 16 complex = 789 504 bytes); the new R̃ overwrites it.
   Init / Reset set every Φ[k] to the identity. In mode 1 the identity is never removed (only ring entries are
   subtracted), so Φ = I + (1/24)·Σ R̃ over the last 24 noise frames; in mode 0 it decays as 0.9ⁿ [I: from the update
   rules].
S and R are written but read only inside this function (R as R_prev) [R: no other reference].

### 8.7 MVDR weights 0x5a38 and `microphone_doa` [R]

For every bin k (0…256), after each noise update:
1. Steering vector d[m] = e^{iθ_m}, θ_m = 2π·(k·fs/512)·(x_m·cos φ + y_m·sin φ)/c with the constants 2π = 6.2831852,
   1/512 = 0.001953125, **cos φ = 2.6794896585·10⁻⁸ and sin φ = 0.9999999999999997** — i.e. φ = 90° (computed with
   π ≈ 3.1415926) — fs = h+4, c = speed of sound h+0x28 (set only by Config), mic coordinates (x_m, y_m) at h+0x2C + 8m
   from Setshape (§8.3; mic_distance is stored in metres). **The look direction is fixed at 90° (broadside,
   perpendicular to the linear array; the y axis of the half circle)**; nothing else feeds θ_m. With shape 0 (linear
   array, all y_m = 0) or without Setshape (coordinates 0) every θ_m is 0 and d = (1, 1, 1, 1) at all frequencies; with
   shape 1 the phases follow the y coordinates (0, 0.866r, 0.866r, 0) [R values; I consequence].
2. Diagonal loading: Φ' = Φ[k] + 10⁻⁴·I (added to the real parts of the four diagonal entries).
3. A = Φ'⁻¹ (0x75ac); u = A·d; s = dᴴ·A·d (two 0x7858 products; d is held as column 0 of a 4×4 matrix, dᴴ as row 0).
4. W[k][m] = conj(u[m]/s) stored at h+0xCF12C + 32k + 8m; Y = Σ W·X in §8.5 step 8 is therefore the MVDR output wᴴX
   with w = Φ⁻¹d/(dᴴΦ⁻¹d) [R arithmetic; I name].
Before the first update the weights are zero (memset by Init / Reset), so Y = 0 and stage 1 outputs silence until a
noise update has run — with vad_enable = 1 that needs a frame the VAD calls non-speech [I].

**`microphone_doa` — three ints.** Run reads doa[0], doa[1], doa[2]; each value −32768 means "keep the previous one"
(kept as floats at h+0x50, +0x54, +0x58); each value divided by fs is stored at h+0x205C, +0x2060, +0x2064. **None of
these six words is read anywhere in the library** (decompile and disassembly: the only accesses are these stores and the
keep-previous loads). The DOA argument has no effect on the output; the beam always points at 90°. What the three values
were meant to be is not recoverable from the binary; dividing by fs suggests sample counts or delays [I, weak]. The
header and the manual describe a single steering angle in degrees (0 = front, ±90 = sides) [S], which matches no code
path. The library reads all three ints, so the pointer must address a readable int[3]; a caller that passes the address
of one int makes it read the two ints after it [R reads; I consequence].

### 8.8 Stage 2: the seven post-filter steps = IMCRA noise estimation + OM-LSA gain [R unless tagged]

All seven functions take B = h+0xD114C. The algorithm is Israel Cohen's **IMCRA** noise estimator (steps 1–6) and
**OM-LSA** gain with a-priori speech-absence estimation (step 7), in the form of Cohen's MATLAB reference `omlsa.m` [I,
strong: every constant below — 1.66, 4.6·1.66, 1.67·1.66, γ₁ = 3, ζ₀ = 1.67, α_s = 0.9, α_d = 0.85, U = 8 × V = 15,
β = 0.7 for ξ smoothing, local / global windows 3 and 31, −10 / −5 dB, 0…10 dB peak, q_max = 0.998, G_min = −15 dB, the
Numerical-Recipes `expint` with its three messages — is that code's; the frequency-smoothing window is 21 points where
IMCRA's default is 3].

Block layout (B-relative, 257 floats = 0x404 bytes each unless stated): 0x000 time history (512) · 0x800 β =
noise_supression_mode/10 · 0x804 input spectrum Y (257 complex) · 0x100C output spectrum (257 complex, = h+0xD2158) ·
0x1818 P = |Y|² · 0x1C1C γ · 0x2020 λ · 0x2424 ξ · 0x2828 G_H1²·γ of the previous frame (Init: 1.0) · 0x2C2C ν · 0x3030
S_f · 0x3434 S · 0x3838 S_min · 0x3C3C S_tmp · 0x4040 I · 0x4444 conv(I) · 0x4848 S̃ · 0x4C4C S̃ before this frame ·
0x5050 conv(I·P) · 0x5454 S̃_min · 0x5858 S̃_tmp · 0x5C5C p (IMCRA) · 0x6060 γ_min · 0x6464 λ̃_d · 0x6868 λ̃_slow ·
0x6C6C…0x8888 the 8 stored S_tmp blocks · 0x8C8C…0xA8A8 the 8 stored S̃_tmp blocks · 0xACAC ζ (smoothed ξ) · 0xB0B0
ζ_frame · 0xB0B4 ζ_peak (dB) · 0xB0B8 / BC / C0 / C4 bin indices k(10 kHz), k(50 Hz), k(500 Hz), k(3500 Hz) · 0xB0C8
ζ_local · 0xB4CC ζ_global · 0xB8D0 / 0xBCD4 the same in dB · 0xC0D8 p (OM-LSA) · 0xC4DC G_H1 · 0xC8E0 q · 0xCCE4
convolution scratch (299 floats) · 0xD190 convolution output · 0xD5E8 P_local · 0xD9EC P_global · 0xDDF0 G · 0xE1F4
scratch (319 floats) · 0xE6F0 global convolution output · 0xEB70 sub-window frame counter · 0xF378 a 257-float array of
1.0 (Init) · 0xF77C / 0xFB80 / 0xFF84 gain scratch.

Bin indices (Init / Reset): k = round(f/fs·512 + 1) for f = 50, 10000 (capped at 257), 500, 3500 Hz (1-based, as
MATLAB). Smoothing windows (normalised to sum 1): **b₂₁** = `.data` 0x32688, 21 taps sin²(πj/22)/11 rounded to 4
decimals (0.0018 0.0072 0.0157 0.0266 0.0390 0.0519 0.0643 0.0752 0.0837 0.0891 0.0909 …symmetric), applied by 0x8150 to
a copy zero-padded by 21 on each side, output delayed by 11; **local** = `.data` 0x326E0 {0.25, 0.5, 0.25}; **global** =
`.data` 0x326F0, 31 taps sin²(πj/32)/16 (Hann, full precision, 0.000600460 … 0.0625 …).

1. **0x8248** P = |Y|²; γ = P / max(λ, 10⁻⁶); ξ' = 0.95·(previous G_H1²·γ) + 0.05·max(γ − 1, 0); ξ' ≤ 0.0316227766
   (−15 dB) → ξ = 0.0316227749, ν = γ·ξ/1.0316227 (both float constants); else ξ = ξ', ν = γ·ξ/(1 + ξ).
2. **0x83ac** S_f = b₂₁ ∗ P (frequency smoothing); S = 0.9·S + 0.1·S_f.
3. **0x8468** S_min = min(S_min, S); S_tmp = min(S_tmp, S) (NEON for bins 0–255, scalar for bin 256).
4. **0x8510** I = 1 where S < 2.7722·S_min **and** P < 7.636·S_min (ζ₀·B_min, γ₀·B_min), else 0. S̃_f = (b₂₁∗(I·P)) /
   (b₂₁∗I) where the denominator > 0, else S̃ of the previous frame; S̃ = 0.9·S̃ + 0.1·S̃_f; S̃_min = min(S̃_min, S̃),
   S̃_tmp = min(S̃_tmp, S̃).
5. **0x87f0** γ_min = P/1.66/max(S̃_min, 10⁻¹⁰) (stored at 0x6060), ζ = S/1.66/max(S̃_min, 10⁻¹⁰). p := 0, then p = 1
   where γ_min ≥ 3 or ζ ≥ 1.67; where 1 < γ_min < 3 and ζ < 1.67: q = (3 − γ_min)/2, p = 1/(1 + q/(1 − q)·(1 + ξ)·e^{−ν}).
6. **0x8968** α̃ = 0.85 + 0.15·p: λ̃_d = α̃·λ̃_d + (1 − α̃)·P; a second estimate with α̃ = 0.99 + 0.01·p goes to 0x6868
   and is **never read**. Sub-window counter 0xEB70: when it equals 14 (every 15th frame) it is reset and both minimum
   trackers are renewed — the 7 older stored blocks shift down, S_tmp (S̃_tmp) is appended, S_min (S̃_min) := the
   element-wise minimum of the 8 blocks (for S̃ the minimum also includes 32767.0), then S_tmp := S, S̃_tmp := S̃;
   otherwise the counter is incremented. Finally **λ = β·λ̃_d** with β = h+0xD194C = noise_supression_mode/10 (0.0…3.0;
   IMCRA's bias factor, default 1.47 there [S]).
7. **0x8cc4** (OM-LSA):
   - ζ = 0.7·ζ + 0.3·ξ (ξ of step 1); ζ_local = local ∗ ζ, ζ_global = global ∗ ζ (zero padded); negative values → 10⁻⁶;
     both to dB (10·log10 via 0xa5a8).
   - ζ_frame = mean of ζ over bins k(50 Hz)…k(10 kHz); ζ_frame,dB = 10·log10(ζ_frame) (−100 if ζ_frame ≤ 0).
   - P_local, P_global per bin (each starts from the all-ones array at 0xF378, which nothing writes after Init):
     ≤ −10 dB → 0.005; between −10 and −5 dB → 0.005 + 0.995·(dB + 10)/5; ≥ −5 dB → 1. If the mean of P_local over bins
     3 … k(500)+k(3500)−3 (1-based) is < 0.25, P_local := 0.005 for bins k(500) … k(3500)−1.
   - P_frame: ζ_frame,dB ≤ −10 → 0.005; otherwise if ζ_frame rose since the last frame: ζ_peak := clamp(ζ_frame,dB, 0,
     10), P_frame = 1; else ζ_frame,dB ≥ ζ_peak − 5 → 1, > ζ_peak − 10 → 0.005 + 0.995·(ζ_frame,dB − ζ_peak + 10)/5, else
     0.005.
   - q = min(1 − P_local·P_global·P_frame, 0.998).
   - γ, ξ, ν are recomputed from the new λ of step 6 exactly as in step 1 (overwriting them; the step-1 values fed steps
     2–6 and ζ).
   - p = 0 where q ≥ 0.9, else p = 1/(1 + q/(1 − q)·(1 + ξ)·e^{−ν}) (e^{−ν} by the vector exp 0xa668).
   - G_H1 = 1 where ν ≤ 0; ξ/(1 + ξ) where ν > 5; ξ/(1 + ξ)·exp(½·E₁(ν)) for 0 < ν ≤ 5, E₁ by an inline copy of Numerical
     Recipes `expint(1, x)` in float: x < 1 power series with Euler's 0.5772156649, x ≥ 1 Lentz continued fraction
     (BIG 1·10³⁰), EPS 10⁻⁷, at most 100 iterations; messages `bad arguments in expint`, `series failed in expint`,
     `continued fraction failed in expint` (the last two then use the partial value / 0).
   - G = max(G_H1^p, 10⁻⁶) · max(G_min^(1−p), 10⁻⁶) with **G_min = 0.17782794 (−15 dB)**, then clamped to [10⁻⁶, 1];
     the powers by the vector pow 0xa834. The next frame's ξ term: G_H1²·γ (0xa834 with exponent 2.0) → 0x2828.
   - Output spectrum (B+0x100C) = G · Y for all 257 bins (bin 256 scalar).
   Steps 1–7 run on every frame (also frames without a stage-1 noise update).

Start-up: S_min, S̃_min, the stored blocks and λ̃_d start at 0, so γ_min is huge, p = 1, λ̃_d stays 0 and λ = 0 until the
minimum blocks have been renewed eight times (8 × 15 = 120 frames, 0.96 s at 16 kHz); until then G ≈ 1 [I: from the zero
state]. **Emulated** (white noise, β = 1.0): output rms ≈ 180–270 (the stage-1 level, input 300) through
frame 130, then 179 (140), 85 (150), 52 (200), 44 (240) — the suppression sets in after ≈ 130 frames [R].

### 8.9 Vector math helpers [R]
<!-- ev:audio-algorithms.8.9 -->

n = 257: four at a time in NEON, the remainder by the libm call — **emulated** against libm
: 0xa5a8 log10 (via 0x9c30 = vector ln, × 1/ln 10): abs error ≤ 5·10⁻⁷;
0xa668 exp: rel. error ≤ 7·10⁻⁸, input clamped to about ±88 (e^{−88} → 0, e^{89} → 2.4·10³⁸); **0xa834 pow: a coarse
approximation, relative error up to 6.5 % for a ∈ [10⁻³, 1], b ∈ [0, 1], pow(0.5, 0) = 1.002** (exp2 / log2 polynomial
of low order); 0xab68 fill with a constant. Only bin 256 of every vector call goes through exact libm; bins 0–255 get the
approximations. The polynomial coefficients of the log (0x9c30), exp (0xa668, `.rodata` 0x1F238…) and pow (0xa834)
approximations are not listed; only their accuracy is known.

### 8.10 The VAD: `IaaVad_*` wrapper and its modified SILK core [R unless tagged]

**Wrapper.** Handle = 0x424 bytes (`IaaVad_GetBufferSize`), caller-provided: +0 fs, +4 channels, +8 samples per frame,
+0xC sample buffer (0x200 bytes), +0x20C delay frames, +0x210 ring write counter, +0x218 vote threshold, +0x21C vote ring
(100 ints), +0x3AC SILK VAD state (0x70 bytes) + two extra words +0x41C / +0x420 (= state +0x70 / +0x74, below).
- `IaaVad_Init(buf, init)`: NULL → message, return NULL; prints `Checking IC....`; its own chip-gate call (0x7b10 from
  0x4618, §2.2): 0 →
  `Warning!! IC version check wrong, disable VAD Lib` and return NULL; otherwise prints `VAD IC check pass.
  ver.2021.0730.0000`, memsets the handle and stores init = {u32 samples, u32 channels, u32 fs} as +8, +4, +0 (the struct
  order is samples, channels, fs). The SILK state is only zeroed, not initialised (the core does that itself on every
  call, below). The message `[VAD] psSilk_VAD init ERROR!` is printed when the gate result is non-zero and ≠ 1 —
  unreachable, the gate returns 0 or 1.
- `IaaVad_Config(h, cfg)` = {u32 delay_frames, u32 sensitivity}: NULL handle → −1; sensitivity > 2 →
  `Sensitivity config setting ERROR` + `The Configuration setting ERROR`, −1; prints `[VAD] Delay frame: %d`; threshold
  = 5 / 3 / 1 for sensitivity 0 / 1 / 2 (`Sensitivity: Low` / `Middle` / `High`); returns 0. **delay_frames is not
  checked**: the slot is computed as `udiv` + `mls` (0x450c), and ARMv7-A `udiv` by 0 returns 0, so with 0 the slot equals
  the running counter and every call writes one word further; values > 100 also write past the 100-slot ring into the
  SILK state [R: code; I: effects].
- `IaaVad_Run(h, pcm)`: NULL → −1. channels = 1: copy `samples` int16; otherwise the input is treated as stereo and each
  output sample is (L + R)/2 (C division, truncation toward zero; NEON for blocks of 8). Core 0x1db18 → r; the slot
  (counter mod delay_frames) gets (r > 0); counter += 1; returns **1 if the sum over all 100 slots > threshold**, else 0.
  The sum always runs over the 100 slots, so with delay_frames < 100 the unused slots stay 0 and the window is
  effectively delay_frames frames.
- `IaaVad_Free(h)`: **calls `free(h)`** (−1 and a message for NULL), although Init never allocates — freeing a caller
  buffer or the inside of a BF handle is invalid; libBF never calls it [R: no caller].
- Use inside libBF: `IaaBf_Init` creates the instance in its own handle (+0xE7E50) with {128, 1, fs} and configures
  {delay 100, sensitivity 0} → **speech = more than 5 of the last 100 frames active**. `IaaBf_Reset` zeroes the whole BF
  handle including the VAD instance and does **not** re-run `IaaVad_Init` / `Config`, so after a Reset channels, samples,
  delay_frames and threshold are 0: nothing is copied into the sample buffer, and the vote of call n is written at
  VAD+0x21C + 4n — after 100 calls into the SILK state, after 130 calls past the VAD instance into the BF handle's Ne10 FFT
  configuration at +0xE8274 (zeros, as r = 0 on the zeroed buffer) [R: Reset and Run code; I: effect]. **Emulated**
  after Reset + Config, frame 132 stops the emulation with an unmapped read inside the FFT (the Ne10
  configuration at +0xE8274 has been overwritten; VAD write index 132).

**Core 0x1db18 = SILK `silk_VAD_GetSA_Q8` (fixed point), modified.** Helpers identified by their constants: 0x1d430
`silk_SQRT_APPROX` (46214 / 32768, 213), 0x1d5e0 `silk_lin2log` (179), 0x1d6d4 `silk_ana_filt_bank_1` (A_fb1_20 = 10788,
A_fb1_21 = −24290; the core carries three inline copies), 0x1d86c `silk_VAD_GetNoiseLevels` (counter < 1000,
0x7FFF/((counter >> 4) + 1), 0x7FFFFFFF/x, limit 0xFFFFFF), the sigmoid `silk_sigm_Q15` inline with its tables at
0x1FAF8 [I, strong: constants and structure equal Opus / SILK's `VAD.c`, `ana_filt_bank_1.c`, `lin2log.c`,
`sigm_Q15.c`; string `psSilk_VAD`]. State (0x70 bytes, the `silk_VAD_state` layout): +0 AnaState[2], +8 AnaState1[2],
+0x10 AnaState2[2], +0x18 XnrgSubfr[4], +0x28 NrgRatioSmth_Q8[4], +0x38 HPstate (s16), +0x3C NL[4], +0x4C inv_NL[4],
+0x5C NoiseLevelBias[4], +0x6C counter. Differences from the public code:
1. **Every call re-initialises** NoiseLevelBias = {50, 25, 16, 12}, NL = 100·bias, inv_NL = 0x7FFFFFFF/NL,
   NrgRatioSmth_Q8 = 100·256, counter = 15 — the body of `silk_VAD_Init` placed at the top of GetSA. Only the filter
   states (AnaState*, HPstate) and XnrgSubfr carry over between frames; the noise levels never accumulate.
2. Frame length fixed at **128 samples** (constant 0x80; 4 bands of 64 / 32 / 16 / 16 samples, 4 sub-frames); the scratch
   X buffer is `malloc(0x140)` and `free`d on every call.
3. Two extra words: +0x70 frame counter and +0x74 baseline SNR. For the first 51 calls (counter ≤ 50) pSNR_dB_Q7 =
   3·SQRT_APPROX(Σ SNR_Q7²/4) is summed into +0x74, at counter = 50 replaced by the mean (sum/50), and SA_Q15 is forced to
   589 (= sigm_Q15(−128), no speech). After that SA_Q15 = sigm_Q15(SMULWB(45000, pSNR − baseline) − 128) — the SNR
   relative to the learned baseline instead of the absolute one.
4. Return value: the number of bands b (0…3) whose `input_quality_bands_Q15[b]` ≠ 0x5CB3 (23731 = the value for
   NrgRatioSmth_Q8 = 100·256, i.e. "not moved this frame"). Because the smoothing coefficient is SMULWB(4096,
   SMULWB(SA_Q15, SA_Q15)), the count is 0 whenever SA_Q15 < 1024 [I: arithmetic], so r > 0 means "speech activity of at
   least about 3 % in this frame" [I].
5. If bands 0 and 1 are both unmoved, the baseline follows: +0x74 = (int)(0.9·base + 0.1·pSNR) (double, truncated).
The rest (band energies, noise-level update, speech_nrg tilt, SA adjustment) is the public algorithm [I: matched by
structure and constants, not compared line by line]. The SILK / Opus revision the copy came from is not identified.

### 8.11 Unreferenced code in libBF [R: no direct call, no pointer to it in the file]

- **0x4bb0** — an earlier form of the noise-covariance update 0x51ec: R̃ = X·Xᴴ/trace from the **raw** channel vector (no
  speech-presence / Wiener step), then the same mode-0 (0.9 / 0.1) or mode-1 (24-frame ring) update of Φ [I: earlier
  version].
- **0x5f34** — an alternative single-channel post-filter on the stage-2 spectrum (h+0xD1950 → h+0xD2158): for the first
  three frames (counter h+0xE7E4C < 3) a straight copy; then the same SPP noise estimator as 0x51ec (ξ₀ = 15 dB, p̄ =
  0.1·p + 0.9·p̄, guard 0.99 → p = 0.99 here, λ = 0.8λ + 0.2·…), decision-directed ξ (0.98 / 0.02), Wiener gain ξ/(1 + ξ),
  the gain smoothed over frequency with the 22-tap table b₂₁, and Y·gain. Its state lives in h+0xE5D30 … h+0xE7E4C, a
  region nothing else uses. The counter h+0xE7E4C that Run increments exists only for it.
- **0x6e18 / 0x6f94** — 2×2 complex inverse / 2×2 complex product (Ne10 2×2 pointers); **emulated**
 errors 4·10⁻⁶ / 3·10⁻⁷ against numpy — the two-microphone versions of 0x75ac / 0x7858 [I: earlier
  version].
- **0xabe0** — a memcpy for ≥ 256-byte, word-co-aligned blocks (64-byte loop with preload), plain `memcpy` otherwise.
- **0x47f4** — prints `[VAD] Sensitivity config setting ERROR....` (the text block that `IaaVad_Config` builds inline).
- **0x4860** — the VAD vote of `IaaVad_Run` as a separate function (ring slot = counter mod delay, (r > 0), sum of 100
  slots > threshold); `IaaVad_Run` carries its own inline copy.
- The shared helpers: timestamp 0x62e0, codec 0x63c0, second gate copy 0x7d38, inverted gate 0xaca8 (§2.4).
- Inside the linked Ne10 only the r2c / c2r FFT, `init`, `mulmat_4x4f`, `invmat_4x4f`, `add/sub_vec4f` (and the 2×2 ones
  of the dead helpers) are reached; the other exported `ne10_*` functions are library code nothing in libBF calls [R:
  call graph + trace]; they are Arm's public Ne10 functions (BSD-3) and are specified by its sources [S]. The
  unreached internal functions 0x196d8, 0x197f0, 0x1a3e8, 0x1a618, 0x1ab8c, 0x1accc, 0x1b428, 0x1b658, 0x1ca4c (inside
  the Ne10 range) are not mapped to Ne10 source names.

### 8.12 Handle map [R]

Offsets in bytes: 0x00–0x58 parameters / geometry / DOA (§8.2, §8.3, §8.7) · 0x5C 4 × 512 input history · 0x205C doa/fs
×3 · 0x2068 FFT context · 0x287C FFT scratch (514 floats) · 0x3084 / 0x3484 unused FFT arguments · 0x3C84 X[257][4] ·
0x5CA4 Φ[257] · 0xDD24 24-slot ring of R̃ · 0xCE924 Y · 0xCF12C W[257][4] · 0xD114C post-filter block B (§8.8) · 0xDFCC0
frame counter · 0xDFCC4 stage-2 overlap-add accumulator · 0xE14D4 stage-1 accumulator · 0xE1CD4 / D8 / DC ring index /
fill counter / full flag · 0xE1CE0 / E4 / E8 state words · 0xE1CEC λ, S, p̄, R (4 × 257×4 floats) · 0xE5D2C update count
· 0xE5D30–0xE7E4B state of the unreferenced 0x5f34 · 0xE7E4C its frame counter · 0xE7E50 VAD instance (0x424) · 0xE8274
Ne10 FFT configuration (0x2C8C) — total 962 304 bytes.

## 9. Licence texts `AEC_LICENSE_{1,2}.txt`, `APC_LICENSE_{1,2}.txt` [R]

- `AEC_LICENSE_1.txt` = `APC_LICENSE_1.txt` (391 bytes, md5 72037a5d…): the Ooura FFT notice — "http://www.kurims.kyoto-u.ac.jp/
  ~ooura/fft.html, Copyright Takuya OOURA, 1996-2001 … You may use, copy, modify and distribute this code for any purpose
  (include commercial use) and without fee. Please refer to this package when you modify this code." with the change log
  "Trivial type modifications by the WebRTC authors. Trivial type modifications by the MSTAR authors." — i.e. the copy of
  Ooura's `fft4g` / `fftsg` that WebRTC carries, modified again by MStar (SigmaStar's predecessor) [I: lineage].
- `AEC_LICENSE_2.txt` = `APC_LICENSE_2.txt` (1510 bytes, md5 e0849e85…): the WebRTC project's 3-clause BSD licence
  ("Copyright (c) 2011, The WebRTC project authors. All rights reserved." + the three conditions + disclaimer).
- Neither library contains these texts or any "ooura" / "webrtc" string itself [R: strings]; the files are the only
  attribution on the device.
- What the notices cover against the code: libAEC and libAPC contain no Ooura code — their FFT is Ne10's (§2.6) — so the
  Ooura notice has no matching code in the two libraries that ship it [I]; Ooura's `rdft` is in libSSL (§4) and libAED
  (§7.4), which ship no notice. The WebRTC notice matches libAEC's NLP (§5.8) and libAPC's ANR (§6.7).
- The rest of the third-party code of §1.4 ships with no notice: the libspeex MDF (Xiph.Org / Jean-Marc Valin,
  BSD-3-style licence), Arm Ne10 (BSD-3), SILK's VAD, Cohen's IMCRA / OM-LSA code, Numerical Recipes' `expint` and the
  Gerkmann & Hendriks estimator [I: licence families].
