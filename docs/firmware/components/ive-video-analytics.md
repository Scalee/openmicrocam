# ive-video-analytics — libmi_ive (MI_IVE API and the MVE core) and the video analytics libraries libMD, libOD, libVG
<!-- ev:ive-video-analytics -->

SigmaStar's user-space vision stack of the SDK in `/config/lib`, used by nothing on this camera (§1.3):

- **libmi_ive.so** — the public `MI_IVE_*` API (45 functions, header `mi_ive.h`) as thin wrappers over SigmaStar's "MVE"
  vision library (`MI_MVE_*`, 76 exports, no public header). Every MVE operation that the IVE engine can do is sent to
  the kernel driver `/dev/mstar_ive0` as one 116-byte job when the images qualify (§3.5) and otherwise runs on the CPU
  (NEON or plain C); a further set of operations is CPU-only (§5). It carries a vendor copy of the Simd library and of
  KLT 1.3.4 (§6), and reproduces several OpenCV algorithms (GMM as MOG / MOG2, Canny, equalizeHist, bilinear resize).
- **libMD_LINUX.so** — motion detection on luma frames: a fixed-point mixture-of-Gaussians foreground model, block SAD
  against a maintained background or the previous frame, connected components on the macro-block map and a
  light-switch filter (§9).
- **libOD_LINUX.so** — occlusion (camera tamper) detection: a learnt background edge map per window and an alarm counter
  over the windows that lost their edges (§10).
- **libVG_LINUX.so** — virtual gate / region intrusion: a private per-line background model, connected components,
  frame-to-frame association and a line-crossing test (§11).

libMD and libOD carry their own static copy of the MVE dispatcher layer and drive the IVE engine themselves (§8); libVG
calls eight libmi_ive exports.

Owners of shared facts, referred to and not repeated here: the IVE kernel driver — device node, DT node, the
`IVE_IOC_PROCESS` ioctl, the 116-byte `ive_ioc_config` layout, the op numbering, the driver's return codes and job
behaviour — `kernel-drivers.md` §8.1 "IVE (`sstar,infinity-ive`, intelligent video engine) — probed, no user"; the MI
error-code format and module ids — `mi-user-abi.md` §4.2 "Code format and module ids"; the signatures of
`MI_SYS_MMA_Alloc`, `MI_SYS_MMA_Free`, `MI_SYS_Mmap`, `MI_SYS_Munmap`, `MI_SYS_FlushInvCache` — `mi_sys-2-interfaces.md`
§5 "libmi_sys.so — the 34 exports outside the shared user-ABI table"; `/dev/msys` and `IOCTL_MSYS_GET_RIU_MAP` —
`kernel-platform.md` §10.1 "`/dev/msys`"; RIU addressing — `kernel-platform.md` §3.1 "Static I/O map and RIU addressing";
the chip id 0x00F4 and the revision — `kernel-platform.md` §3.2 "Chip ops, chip id and revision"; the file table of the
nine algorithm libraries, their origin in the public Tiramisu SDK drop, their build tags, the chip gate, the base-62
codec, the timestamp helper, logging and return-code format shared by libMD / libOD / libVG with the six audio
libraries — `audio-algorithms.md` §1.1 "Files", §1.2 "Origin: the public Tiramisu SDK drop", §2.1 "Build tags", §2.2
"Chip gate (`/dev/msys` + `/dev/mem`)", §2.3 "Base-62 codec (`EN_OR_DE`)", §2.4 "Other shared helpers and the address
table", §2.5 "Logging, return codes, handles and state flags"; libmi_vdf.so, the SDK's only caller of libMD / libOD /
libVG — `sdk-orphan-libs.md` §7 "libmi_vdf.so — VDF "video detection framework" (MD / OD / VG host)"; the second copy of
the vendor Simd tree and of MVE, in mi_ldc.ko — `ldc-2-eptz-lib.md` §11 "MVE and the Simd library"; the inventory rows —
`firmware-image.md` §3.5 "`/config/lib` (SDK user libraries)".

Tags: **[R]** read from the binary (disassembly, decompile, strings, data, emulation of its code) or from a cited file;
**[S]** stated by a source (public header, upstream source); **[I]** inference. Names are the binaries' symbols or
`__func__` strings [R]; names of handle fields and internal functions are given by their role [I] unless a public
source is named.

Addresses are file VAs (= file offsets for `.text`). An address given with image base 0x10000 (written "Ghidra") is VA +
0x10000 [R].

Binaries (dump of 2026-10-16) [R]: `/config/lib/libmi_ive.so`,
`/config/lib/lib{MD,OD,VG}_LINUX.so`.

## 1. The libraries

### 1.1 libmi_ive.so: file and build [R]

- `libmi_ive.so`, 742 208 bytes, ARM EABI5, stripped (`.dynsym` only), BuildID 958ceca0…de123a. The copies in
  the Anjoy MC-H12S firmware and in the public Xsyt M6 SDK on gitee (`files/public/m6/MI/lib/`) are
  byte-identical (md5 f30a951c…99114), so that SDK's headers `mi_ive.h` / `mi_ive_datatype.h`
  describe exactly this build [I, strong: same file].
- NEEDED: only `libgcc_s.so.1`, `libc.so.6`. It imports `MI_SYS_MMA_Alloc`, `MI_SYS_MMA_Free`, `MI_SYS_Mmap`,
  `MI_SYS_Munmap`, `MI_SYS_FlushInvCache` without a NEEDED entry for libmi_sys, so the application must load libmi_sys
  first [R imports; I consequence].
- Exports: 45 `MI_IVE_*` (all 45 declared in `mi_ive.h`, none missing, none extra), 76 `MI_MVE_*` (no public header) and
  one libstdc++ template instance `std::vector<double>::_M_default_append` (0x731c0).
- The code is **ARM (A32)**, not Thumb; only ≈0.9 KB of Thumb-2 libgcc soft-float helpers sit at 0xa66bc–0xa6a34. Functions
  are found from `.ARM.exidx` (449 entries), every direct `bl` / `b` target, every `push {…lr}` outside a known body and the
  40 local function addresses in the GOT.
- `.comment` = `GCC: (GNU) 9.1.0`; EABI attributes: CPU "8-A", ARM + Thumb-2, VFPv3, NEONv1, hard-float VFP argument
  passing (`Tag_ABI_VFP_args`), enum size int, 8-byte alignment. C++ is present only for Simd (`__PRETTY_FUNCTION__`
  strings, and the one `std::vector<double>` instance used by `MI_MVE_MultibandBlending`).
- Source tree `./../C/src/Function_tools/{MVE_*.c, Simd/*.cpp}` (assert paths). Two vendor files are named by `__FILE__`
  (`MVE_LKOpticalFlow.c` holds all of KLT); the MVE C files themselves never assert, so their names are not in the binary.

Layers:

| layer | functions | bytes | what |
|---|---|---|---|
| MI_IVE wrappers | 45 | 31 K | §2 — handle table, struct conversion, error mapping |
| MVE exports + core | 73 | 70 K | §3, §4 — dispatcher per operation, buffers, chip check, HW job |
| MVE internal, named by `__func__` | 69 | 68 K | `*_Acc` (31 HW job builders, §4), `*_Neon` (24), `*_PureC` (21) |
| Simd library (Ermolaev, MIT; vendor copy v4.4.84…v4.6.93 + 3 vendor functions) | ≈180 | ≈370 K | §6.1 — of which **155 K are 41 template instances of `Simd::Neon::MedianFilter*`** reached only from `MI_MVE_Median_Filter` |
| KLT 1.3.4 (Birchfield, public domain) | 18 | 60 K | §6.2 — LK optical flow |
| unreached code | 124 + 2 named | ≈90 K | §7 — dead Simd entry points, a licence-code cipher, duplicated utility objects |
| libgcc (soft-float double, Thumb), crt | — | ~1 K | toolchain |

### 1.2 libMD, libOD, libVG [R]

The three video libraries are rows of the nine-library table in `audio-algorithms.md` §1.1 "Files": libMD 150 148 bytes,
`.text` 0x1f154, exports `MI_MD_*` 13 and `_md_sym_106…155` 50; libOD 109 240 bytes, `.text` 0x151ec, `MI_OD_*` 9 and
`_td_sym_90…140` 51; libVG 54 920 bytes, `.text` 0x97e0, `MI_VG_*` 18, `IvaVG_*` 9, `vg_cross_*` 4. All three are ARM
EABI5, stripped, GCC 9.1.0. They are stock SigmaStar binaries, byte-identical to the public Tiramisu SDK V012 drop, whose headers `mi_md.h`, `mi_od.h`, `mi_vg.h` describe them
(`audio-algorithms.md` §1.2). Build tags: MD ae163 + MVE 5985d, OD 8ad94 + MVE a5131, VG 5d7b7 (`audio-algorithms.md`
§2.1) — the "MVE" tag of MD and OD is their private MVE copy (§8).

All three leave `MI_MVE_*` (libmi_ive.so) and `MI_SYS_*` (libmi_sys.so) undefined without a NEEDED entry for those
libraries; the application must load libmi_ive / libmi_sys first [R: no NEEDED; I: consequence for the loader]. They
use the shared chip gate, logging and return-code scheme of the nine algorithm libraries (`audio-algorithms.md` §2.2,
§2.5); their own return-code enums are given per library below. Their handles are allocated by the library (libMD
`malloc` §9.2, libOD `malloc` §10.2, libVG `calloc` §11.3).

### 1.3 Users on this camera [R]

None. No program on the camera links libmi_ive (prog_uvc, kjd_lvgl) and nothing opens `/dev/mstar_ive0`
(`kernel-drivers.md` §8.1.4 "Use on this camera"); prog_uvc and kjd_lvgl import none of the `MI_MD_*`, `MI_OD_*`,
`MI_VG_*` symbols. All four libraries are unused on the product. The SDK's own libmi_vdf.so references `MI_MD_*`,
`MI_OD_*`, `MI_VG_*` (`sdk-orphan-libs.md` §7), so the three video libraries are not stand-alone orphans. The Anjoy MC-H12S firmware carries the same libraries, and its `opt/ch/media_server` contains `IvaMD_*` strings — a
real-world caller (`audio-algorithms.md` §1.2).

### 1.4 Third-party and reproduced code

| code | where | section |
|---|---|---|
| Simd (github.com/ermig1979/Simd, MIT), vendor copy v4.4.84 … v4.6.93 plus vendor additions | libmi_ive | §6.1 |
| KLT 1.3.4 (Stan Birchfield, public domain), trimmed and changed | libmi_ive | §6.2 |
| OpenCV legacy `BackgroundSubtractorMOG` and `BackgroundSubtractorMOG2` (reproduced in C) | libmi_ive GMM | §5.7 |
| OpenCV `Canny`, `equalizeHist`, fixed-point `INTER_LINEAR` resize (reproduced) | libmi_ive | §5.9, §5.8, §5.5 |
| Simd `ShiftDetector` | libmi_ive Shift_Detector | §5.14 |
| KaewTraKulPong–Bowden mixture re-done in integers | libMD FG mode | §9.6 |

The reproduced OpenCV algorithms are identified by their constants, branches and map coding [I: identity].

## 2. The MI_IVE layer (45 exports, 0x2db8–0xa6b8) — the public API

Prototypes and structs are those of `mi_ive.h` / `mi_ive_datatype.h` [R: every export name matches, and the argument use
below matches the struct layouts]. Each export is a thin wrapper around one `MI_MVE_*` function (§3) [R decompile].

### 2.1 Handle table [R]

One static block in `.bss` at **0xc4da8**: `+0x00 u8 inited`, `+0x04 pthread_mutex_t lock` (24 B), `+0x1C void *mve[128]`
(the MVE handle per `MI_IVE_HANDLE`), `+0x21C int busy[128]` (calls in flight per handle). `MI_IVE_HANDLE_MAX` = 128.

Every export first does lazy init: if `inited` = 0, under the lock zero both arrays and set `inited` = 1 — and then **return
`MI_IVE_ERR_NOT_INIT` (0xA0002015)** for every export except `MI_IVE_Create` (which continues). So the first call of a
process that is not `MI_IVE_Create` fails once even if it is valid [R behaviour; I: harmless, since nothing can be
created before Create].
- `MI_IVE_Create(h)`: `h` ≥ 128 (unsigned compare) → 0xA0002002 (`INVALID_CHNID`); under the lock: `mve[h]` ≠ 0 →
  0xA0002004 (`EXIST`); if `busy[h]` ≠ 0 it only `puts` a warning string; `mve[h] = MI_MVE_Init(0)`; NULL → 0xA000201F;
  else 0.
- `MI_IVE_Destroy(h)`: not inited → 0xA0002015; `h` > 127 (**signed** compare: a negative `h` indexes before the table) →
  0xA0002002; `mve[h]` = 0 → 0xA0002015; `busy[h]` ≠ 0 → 0xA0002012 (`BUSY`); `MI_MVE_Uninit(mve[h])`, on 0 clear `mve[h]`;
  MVE error → mapped (§2.3).
- Operation exports (the 43 others): not inited → 0xA0002015; `h` > 127 (signed) → **0xA0002015** (not `INVALID_CHNID` as
  in Create); under the lock read `mve[h]` (0 → 0xA0002015) and `busy[h]++`; unlock; convert the arguments (§2.2) into
  stack copies; call the MVE function with `bInstant ≠ 0` as its last argument; lock, `busy[h]--` if `mve[h]` is still
  set, unlock; map the result (§2.3).

No call is serialised against another on the same handle — the lock only guards the table; the MVE layer has no lock of
its own [R: no pthread call outside the MI_IVE wrappers].

### 2.2 Struct conversion MI → MVE [R]

- `MI_IVE_Image_t` (0x38 B: `eType`@0, `MI_PHY aphyPhyAddr[3]`@8 (u64), `apu8VirAddr[3]`@0x20, `azu16Stride[3]`@0x2C,
  `u16Width`@0x32, `u16Height`@0x34, `u16Reserved`@0x36) → **`MVE_IMAGE_S` (0x28 B)**: `+0x00 u32 type` (same numbering as
  `MI_IVE_ImageType_e`), `+0x04 u32 phys[3]` (**low 32 bits only** of each `MI_PHY`), `+0x10 u8 *virt[3]`,
  `+0x1C u16 stride[3]`, `+0x22 u16 width`, `+0x24 u16 height`, `+0x26 u16 reserved`.
- `MI_IVE_MemInfo_t` (0x10 B: `MI_PHY phy`@0, `vir`@8, `u32 size`@0xC) → **`MVE_MEM_INFO_S` (0x0C B)**: `+0 u32 phys` (low
  32 bits), `+4 u8 *virt`, `+8 u32 size`.
- `MI_IVE_Data_s` is not used by any export [R: no export takes it].
- Control structs are copied byte for byte (same layout in both layers), with these exceptions in the wrappers:
  `MI_IVE_LkOpticalFlow` builds the MVE control from `MI_IVE_LkOpticalFlowCtrl_t` and adds a constant field = **15**
  (§5.12); `MI_IVE_Shift_Detector` passes a context struct whose first word is the MVE handle (the MVE function takes no
  separate handle, §5.14); `MI_IVE_Matrix_Transform` copies the 9 `s32MatrixArray` words and the mode into a local block
  (§4.3); `MI_IVE_Map` copies `pstMap->virt[0..255]` to its stack and passes a `u8 *lut`; `MI_IVE_Ncc` and `MI_IVE_Ccl`
  copy their result out only when the caller's size field is exact (§4.1, §5.10).

### 2.3 Error mapping MVE → MI [R, an identical switch in every wrapper]

| MVE code | meaning (MVE strings, §3) | MI code | MI name |
|---|---|---|---|
| 0 | ok | 0 | `MI_SUCCESS` |
| 0x10000103 | handle NULL | 0xA0002002 | `MI_IVE_ERR_INVALID_CHNID` |
| 0x10000104 | parameter error | 0xA0002003 | `MI_IVE_ERR_ILLEGAL_PARAM` |
| 0x10000105 | NULL pointer | 0xA0002006 | `MI_IVE_ERR_NULL_PTR` |
| 0x10000106 | not supported | 0xA0002008 | `MI_IVE_ERR_NOT_SUPPORT` |
| 0x10000107 | timeout | 0xA0002019 | `MI_IVE_ERR_SYS_TIMEOUT` |
| 0x10000108 | HW failure | 0xA000201F | `MI_IVE_ERR_FAILED` (err 31) |
| 0x10000109 | busy | 0xA0002012 | `MI_IVE_ERR_BUSY` |
| 0x1000010A | invalid argument | 0xA0002003 | `MI_IVE_ERR_ILLEGAL_PARAM` |
| 0x1000010B | memory | 0xA000200E | err 14 (`BUF_EMPTY` in the enum; used as "no memory") |
| any other (incl. 0x10000101, 0x10000102) | — | 0xA000201F | `MI_IVE_ERR_FAILED` |

Module id of IVE = 0 (`E_MI_MODULE_ID_IVE`), level 2 (error): `MI_DEF_ERR(0, 2, e)` = 0xA0002000 | e [R
`mi_common_datatype.h`; format: `mi-user-abi.md` §4.2].

## 3. The MVE core — handle, chip check, buffers, the HW job, dispatch

`MI_MVE_*` is SigmaStar's "MVE" vision library (the `__FILE__` strings name `./../C/src/Function_tools/`) [R]. It has no
public header; every prototype below is read from the code. Return codes are the `0x100001xx` set of §2.3: `0x10000103`
(handle NULL) is returned by the per-function entry check; `0x10000101` and `0x10000102` are never produced [R: no
occurrence].

### 3.1 Handle (`MI_MVE_Init` → 0x50-byte calloc) [R offsets from Init / Uninit and the users; roles from use]

| off | field | set by / used by |
|---|---|---|
| +0x00 / +0x04 | CPU work buffer: `u32 size`, `void *buf` | `MVE_Utl_CheckWorkBuffer(h, n)` (0x3a218): if `n` > size, `free` + `malloc(n)` (grow only; failure → size 0, `0x1000010B`). Used by the CPU paths: Hist_PureC asks 0x1000; Integ COMBINE 20·W·H (§4.4); CCL align4(w·h) + 24·w·h (§5.10); the `_PureC` padding copies (§3.7); ImageHistMatch, OptimalSeam (§5.19, §5.20) |
| +0x08 / +0x0C | GMM scratch: `u32 size`, `void *buf` | GMM_PureC (re-`malloc`s `w·nModels·4` per call; the per-mode `sortKey` / `bgmodelUsedModes`, §5.7) |
| +0x10 | **padding mode** (border handling, §3.7); Init sets **2** | `MI_MVE_ConfigPadding(h, mode)` (no range check) |
| +0x14 / +0x18 | KLT context: `u32 size` (= 8), `void **ctx` (2 pointers: tracking context, feature list) | LK and SelectGoodFeatures create it on first use (§5.12, §5.13); Uninit frees both pointers and the block |
| +0x1C | fd of `/dev/mstar_ive0` (`open(…, O_RDWR)`) | every `*_Acc` |
| +0x20 … +0x24 | not used [R: no access found] | — |
| +0x28 | HW job pending flag | set to 1 after a successful ioctl, cleared by the wait (§3.4) |
| +0x38 / +0x3C | physical address of the 4 KiB **IVE work buffer** (u64, high word 0) | Map (LUT), NCC (result) |
| +0x40 | its virtual address | 〃 |
| +0x44 | its size = 0x1000 | Uninit |
| +0x48 … +0x4C | not used [R] | — |

`MI_MVE_Init(void)` (0x391ec; `MI_IVE_Create` passes 0, ignored): `calloc` 0x50; padding mode 2; **chip check** (§3.2) — on
failure print "The platform check is incorrect…" and return NULL; `open("/dev/mstar_ive0", O_RDWR)` (fd ≤ 0 → "Can't open HW
device", NULL); allocate the work buffer **`mma_heap_name0#IVEWorkBuff`**, 0x1000 B, mapped **cached** (§3.3; failure →
"Can't alloca physical buffer for IVE HW", NULL); return the handle. Two empty hooks surround it (0x3b58c memory-init and
0x20b20 trace-init, both `return 0`).

`MI_MVE_Uninit(h)`: NULL → 0x10000103; free +0x04, +0x0C, the KLT context; free the work buffer (§3.3); `close(fd)`; free the
handle.

There is **no per-handle lock**: two threads on one handle can both pass the +0x28 check before either sets it [R code;
I race].

### 3.2 Chip check (0x3f3b8, once per process) [R]

Result cached in `.bss` 0xc5528 (done flag) / 0xc552c (result). Opens `/dev/mem` and `/dev/msys` (`O_RDWR|O_SYNC`), issues
`IOCTL_MSYS_GET_RIU_MAP` (0x5315) with the 20-byte `MSYS_MMIO_INFO` {`0x4D530100`, u64 addr, u32 size}
(`kernel-platform.md` §10.1), `mmap`s the RIU window from `/dev/mem`, reads the **u16 at RIU + 0x3C00 = PMTOP bank 0x001E
reg 0x00 (chip id)**, unmaps, closes. Pass iff the id is **0x00F4** (Mercury6, the id the OpenIPC boot banner prints; `kernel-platform.md` §3.2). Any failure
prints "Unexpected IC version" and fails for the rest of the process. Two more copies of the routine exist unreferenced
at 0x0b6a8 and 0x0b8d0 [R: no caller] (§7). The same steps are the chip gate of the nine algorithm libraries
(`audio-algorithms.md` §2.2).

### 3.3 Buffers [R]

- **MMA allocation** (0x3b59c, `MVE_AllocPhys(h, name, ?, size, ?, *phys, *virt)`): heap name = `"mma_heap_name0#"` + up
  to 16 characters of `name`; `MI_SYS_MMA_Alloc(0, heapname, size, &phys)` then `MI_SYS_Mmap(phys, size, virt, cached = 1)`;
  on mmap failure `MI_SYS_MMA_Free` and `0x1000010B`. Free (0x3b6d4): `MI_SYS_FlushInvCache(virt, size)`, `MI_SYS_Munmap`,
  `MI_SYS_MMA_Free`. Signatures: `mi_sys-2-interfaces.md` §5.
- **`MI_MVE_AllocateBuffer(h, MVE_MEM_INFO_S *m, u32 size, const char *name, int phys)`**: `phys` = 1 → MMA as above
  (cached), else `malloc(size)` and **`m->phys = m->virt`** — the library's convention for "no physical address" (every HW
  eligibility test rejects a plane whose phys equals its virt, §3.5). `m->size` = size. `MI_MVE_FreeBuffer(h, m)`: phys
  == virt → `free` (NULL → 0x10000105), else the MMA free.
- **`MI_MVE_AllocateImage` / `MI_MVE_FreeImage`** (0x38780 / 0x38e3c): the same for an `MVE_IMAGE_S`, one allocation per
  plane, sized from type, the caller's stride and height (the stride is taken as given, not aligned) — §5.1. The MI_IVE
  layer never calls them (MI users allocate with `MI_SYS_MMA_Alloc` themselves).
- The IVE work buffer (+0x38 / +0x40) is the only physical scratch the HW path uses; at 4 KiB it holds the Map LUT (256 B)
  and the NCC result (24 B); every other HW op reads and writes the caller's buffers directly.

### 3.4 The HW job (`*_Acc` functions) [R]

1. Build a 116-byte `ive_ioc_config` on the stack: `memset` 0 (0x70 B after the op word, or all 0x74), `op_type`, input and
   output images (§3.4.1), the coefficient union (per op, §4.1). Layout and op numbers: `kernel-drivers.md` §8.1.2.
2. If `h+0x28` ≠ 0 → `0x10000109` (busy; a previous job of this handle was never waited for).
3. `ioctl(fd, IVE_IOC_PROCESS = 0x40044901, &cfg)`. Non-zero → print "IVE_IOC_PROCESS return %d, %s" with the text of a
   6-entry table in `.rodata` (0x1000 "The input and output size must be the same", 0x1001 "The image size is too small",
   0x1002 "Incorrect configuration", 0x1003 "The HW is processing now", 0x1004 "memory access failure", 0x1005 "HW enable
   failure") and return **`0x10000108`**.
4. Set `h+0x28` = 1 and **wait** (0x3fbf0): `poll(fd, POLLIN|POLLRDNORM, 5000 ms)`; `EINTR` → wait again (recursion);
   timeout → "Polling timeout!!", **`0x10000107`** (the flag is left set, so the next job on this handle fails busy [R: the
   flag is not cleared on this path]); revents has POLLIN / POLLRDNORM → clear the flag, success; other revents or error →
   `0x10000108`.
   Every HW op is synchronous. `bInstant` never reaches an `*_Acc` function; it is not a sync flag at all — Filter and
   Bernsen use it as an output-mode bit and in their HW-eligibility test; no other dispatcher's eligibility test reads it
   [R: scan of all `MI_MVE_*` dispatchers] (§3.5, §4.1, §4.4).
5. Post-process: the border fix-up of §3.7 on the output image(s) of neighbourhood ops; Map / NCC copy through the work
   buffer (§4.1). No cache maintenance is done on the caller's buffers [R: `MI_SYS_FlushInvCache` is called only for the
   work buffer] — the caller has to flush / invalidate its own cached buffers [I].

#### 3.4.1 Image → `ive_ioc_image` [R 0x3f7c0, 0x3f994, 0x3fba0]

`ive_ioc_image` = {u32 format; u16 width; u16 height; u32 address[3]; u16 stride[3]} (28 B; `kernel-drivers.md` §8.1.2).
Format map (table `.rodata` 0xa76c4, 8-byte pairs MVE → IOC):

| MVE / MI type | IOC format |
|---|---|
| 0 U8C1, 1 S8C1 | 0x00 B8C1 |
| 2 YUV420SP | 0x05 420SP |
| 3 YUV422SP | 0x06 422SP |
| 8 S16C1, 9 U16C1 | 0x03 B16C1 |
| 10 U8C3_PACKAGE | 0x04 B8C3_PACK |
| 11 U8C3_PLANAR | 0x02 B8C3_PLAN |
| 12 S32C1, 13 U32C1 | 0x80 B32C1 |
| 14 S64C1, 15 U64C1 | 0x81 B64C1 |
| 4, 5 (YUV planar), 6, 7 (S8C2) | none: "Unknow mve_format 0x%X", the image is left zero (the job then fails in the driver) |

The type numbers follow the Hi-IVE `IVE_IMAGE_TYPE_E` order (13 = U32C1, 15 = U64C1) [S: Hi-IVE enum; I: same order here].

Two builders: `img1(ioc, A)` copies all three planes of A (address = phys[i], stride = stride[i]); `img3(ioc, A, B, C)`
takes **plane 0 of each** of up to three images as planes 0 / 1 / 2 (A→0, B→1, C→2), format / width / height from the
first non-NULL of A, B, C (A first). The second form is how ops with two inputs or two outputs pass them (e.g. And: in =
{src1, src2}; Sobel: out = {dstH, dstV}). `mem(ioc, M0, M1, M2)` turns `MVE_MEM_INFO_S` into planes: address = phys,
stride = (u16)size, format B8C1, width = height = 1.

### 3.5 Dispatch rule (every MVE operation that has an `*_Acc`) [R]

`MI_MVE_<op>(h, …)`: handle NULL → 0x10000103. Then the **HW-eligibility test** over every image argument: width % 16 = 0,
height ≥ 5, and for each plane i whose `virt[i]` ≠ 0: `phys[i]` ≠ 0, `phys[i]` ≠ `virt[i]` (not a `malloc` buffer, §3.3),
`phys[i]` % 16 = 0, `stride[i]` % 16 = 0 (MemInfo arguments: phys ≠ 0, phys ≠ virt, phys % 16 = 0). Pass → the `*_Acc` HW
job (§3.4). Fail → the CPU implementation, chosen at link time through a **weak symbol**: if its address is NULL →
**0x10000106 (MI: `NOT_SUPPORT`)**. For the 5×5 neighbourhood ops the choice also depends on the padding mode (h+0x10):
mode ∈ {1, 2} → the `*_PureC` version, other modes → the `*_Neon` version. Per-op deviations (Sad's own size test,
Matrix_Transform per channel mode, Integ's routing) are in §4.1.

The CPU implementations are linked into libmi_ive.so as **weak, hidden** definitions; the dispatchers test their GOT
addresses. Only five are exported: `MI_MVE_{CannyEdge,CannyHysEdge,GMM,LKOpticalFlow,NormGrad}_PureC` (`.dynsym` WEAK
FUNC). The six `*_PureC` symbols `MI_MVE_{Sobel,MagAndAng,Dilate,Erode,OrdStatFilter,Bernsen}_PureC` are **weak undefined**
in this build [R `.dynsym`]; the handle starts in padding mode 2 and the MI_IVE layer never calls `MI_MVE_ConfigPadding`.
So **through the MI_IVE API these six operations work only on HW-eligible images and otherwise return
`MI_IVE_ERR_NOT_SUPPORT`** [R code path; I: a vendor bug or a deliberate trim]. Filter has a defined `_PureC` and works on
the CPU.

Two MVE functions also take the MI `bInstant` flag into the eligibility test: Filter and Bernsen go to the CPU when it is
0 (§4.1), so `bInstant` = 1 is needed for the HW path there although `mi_ive.h` calls it "Reserved" [R].

### 3.6 Simd dispatch [R]

Every Simd entry the MVE code calls goes through a small vendor dispatcher of the form
`if (Neon::Enable && width >= 16) Neon::F(…) else Base::F(…)` (e.g. 0x60a60 for DeinterleaveUv); `Neon::Enable` is a byte in
`.data` set at start-up [I: Simd's `Neon::Enable` runtime check]. Base and Neon versions give the same results by Simd's
design [I].

### 3.7 Border handling and padding [R]

**Border fix after neighbourhood ops** (0x39a58, `fix_border(dst, ksize, mode)`): applied to the output image(s) of the HW
path and of the `_Neon` path, with `ksize` 5 (border 2 px) or 3 (border 1 px): padding mode 3 → border pixels set to **0**;
mode 4 → border pixels **copied from the nearest inner pixel** (replicate); modes 0, 1, 2 → nothing (border left as the
engine / kernel wrote it). Per output type: U8 / S8 one plane; YUV420SP / 422SP both planes (the UV plane handled as 16-bit
pairs); planar U8C3 three planes; 16-bit (8, 9) element size 2; U8C3 package element 3; 32-bit ×4; 64-bit ×8; YUV planar
(4, 5) → 0x10000106.

**Padding routines of the `_PureC` 5×5 / 3×3 kernels** in padding modes 1 / 2 (the input is copied into the CPU work buffer
with a border of 2 or 1 pixels): **mode 1 (0x3a654) = mirror without repeating the edge (OpenCV `BORDER_REFLECT_101`: −k →
k, w−1+k → w−1−k)**, **mode 2 (0x3a294) = replicate the edge pixel** (`BORDER_REPLICATE`); both fail with 0x10000104 when the
buffer stride < width + 2·pad. The kernels run on the padded copy, so the output covers the full image; in modes 0 / 3 / 4
the `_Neon` kernels skip the border and the border fix above handles it afterwards.

## 4. Operations with a hardware path

### 4.1 Per operation [R unless marked]

"in" / "out" = images placed into the `ive_ioc_config` (planes as in §3.4.1, `{a, b}` = `img3`), "CPU" = the implementation
taken when the images are not HW-eligible (§3.5); "(absent)" = weak undefined in this build. Modes and enums pass through
unchanged unless noted (Thresh goes through the 8-pair table `.rodata` 0xa6fa4 and the Matrix_Transform control mode
through 0xa7194; both are identity).

| MI_IVE export → MVE function | IOC op | HW in → out, coefficients | CPU | notes |
|---|---|---|---|---|
| Filter → MI_MVE_Filter | 0x00 | src → dst (`img1`, all planes); mask[25] + norm as given | mode ∉ {1,2}: `_Neon` if the 25 mask coefficients sum to −127…127, else `_PureC`; mode 1/2: `_PureC` | same size required (else 0x10000106); border fix 5. The MVE bool argument (= MI `bInstant`) is an **output mode**: 1 → `min(max(Σ,0)>>norm,255)` (the engine's rule — only this value can take the HW path), **0 → `min(|Σ|>>norm,255)` and always CPU** even for HW-eligible images [R: `bool == 0` is part of the not-eligible test] |
| Csc → MI_MVE_CSC | 0x01 | src → dst (`img1`); 30-byte coefficient set from the table of §4.2 by mode and dst / src type | `_Neon` (`_PureC` exists but is unreached, §7) | width / height even on the CPU; MI mode 0–3 = MVE mode 0–3 |
| FilterAndCsc → MI_MVE_FilterAndCSC | 0x02 | src → dst; filter (26 B) + CSC table entry | `_Neon` | border fix 5 |
| Sobel → MI_MVE_Sobel | 0x03 | src → {dstH, dstV}; mode, mask[25] | mode ∉ {1,2} and mask sum within −127…127: `_Neon`; otherwise `_PureC` (absent → NOT_SUPPORT) | border fix 5 on the produced output(s) |
| MagAndAng → MI_MVE_MagAndAng | 0x04 | src → {mag, ang}; mode, mask[25], thr (re-ordered to the IOC layout) | `_PureC` (absent) / `_Neon` | border fix 5; ang only in mode 1 |
| OrdStatFilter → MI_MVE_OrdStatFilter | 0x05 | U8C1 src → U8C1 dst; mode (0 median / 1 max / 2 min) | `_PureC` (absent) / `_Neon` | border fix **3** (3×3 filter) |
| Bernsen → MI_MVE_Bernsen | 0x06 | src → dst (`img1`); IOC mode = 0 / 1 for (NORMAL, win 3 / 5), 2 / 3 for (THRESH, win 3 / 5); thr | `_PureC` (absent) / `_Neon` | win ∉ {3,5} or mode ∉ {0,1} → 0x10000106; border fix 5; the MVE bool (= MI `bInstant`) **= 0 forces the CPU path** (→ NOT_SUPPORT in the default padding mode) |
| Dilate / Erode → MI_MVE_Dilate / Erode | 0x07 / 0x08 | src → dst; mask[25] | `_PureC` (absent) / `_Neon` | same type and size; border fix 5 |
| Thresh → MI_MVE_Thresh | 0x09 | src → dst; mode, low / high (u8 widened to u16), min / mid / max | `_Neon` | — |
| ThreshS16 / ThreshU16 → MI_MVE_Thresh_S16 / _U16 | 0x0A / 0x0B | src → dst; mode, low, high (16-bit), min / mid / max | `_Neon` | — |
| And / Or / Xor → MI_MVE_And / Or / Xor | 0x0C / 0x0D / 0x0E | {src1, src2} → dst | `_Neon` | all three images the same size |
| Add → MI_MVE_Add | 0x0F | {src1, src2} → dst; mode, x, y (U0Q16) | `_Neon` | HW: **x + y must equal 65536** and src1 U8C1, else 0x10000104 |
| Sub → MI_MVE_Sub | 0x10 | {src1, src2} → dst; mode (abs / shift) | `_Neon` | — |
| 16BitTo8Bit → MI_MVE_16BitTo8Bit | 0x11 | src → dst; mode, denominator, numerator, bias | `_Neon` | — |
| Map → MI_MVE_Map | 0x12 | in plane **1** = src (plane 0 empty) → dst; the 256-byte LUT is copied into the IVE work buffer, flushed, and its phys passed as `coeff_map` | `_PureC` | the MI wrapper passes a `u8 *lut` (§2.2) |
| Hist → MI_MVE_Hist | 0x13 | src → `mem(dst)` (256 × u32 written by the engine) | `_PureC` | the dst MemInfo must be HW-eligible too |
| Integ → MI_MVE_Integ (0x13940) | 0x14 | src → dst (U32C1 → B32C1 or U64C1 → B64C1); mode 0–2 | mode 1 SUM → `_Neon` (0x465e4); modes 0 COMBINE and 2 SQSUM → `_PureC` (0x36660); either missing → 0x10000106 [R 0x13ab4–0x13b00] | dst the size of src (inclusive integral, §4.4) |
| Sad → MI_MVE_SAD | 0x15 | {src1, src2} → {sad, thr}, or {thr} alone in out-mode 4; block mode, out mode, thr, min, max | `_Neon` | per-mode size / type check (0x205ac) instead of the generic test; block mode ∉ 0–2 or out mode ∉ 0–4 → 0x10000104 |
| Ncc → MI_MVE_NCC | 0x16 | {src1, src2} → (dummy out = src1); `coeff_ncc` = work-buffer phys; after the job the 24-byte result {u64 numerator, sum1, sum2} is read back from the work buffer | `_Neon` | MI wrapper: copies the 24 bytes + 8 zero bytes to `pstDst->virt` **only if `pstDst->size` = 0x20**, else `ILLEGAL_PARAM` (even after a successful job) |
| Lbp → MI_MVE_LBP | 0x18 | {src1, src2} → dst; mode, channel mode, thr: mode 0 = (s8) thr, mode 1 = u8 thr, mode 2 (ABS_MUL) = min(thr, 7) **written back into the caller's ctrl** and sent as thr + 2 | `_PureC` (`_Neon` exists, unreached, §7) | channel mode 1 needs src2 the same size; border fix 3 |
| BAT → MI_MVE_BAT | 0x19 | src → `mem(hor, ver)`; h_times, v_times | `_PureC` | `src.height` = `hor.size` and `src.width` = `ver.size` (element counts, as the MVE layer reads them) |
| AdpThresh → MI_MVE_AdpThresh | 0x1A | in = {integ plane 0, integ shifted down by `u8HalfMasky` rows (phys + stride·halfY·4), src plane 0} → dst; 5 control bytes | `_PureC` | src, integ, dst the same size |
| Matrix_Transform → MI_MVE_Matrix_Transform | 0x1F (MATRIX_TRANSFORM) | {src1, src2, src3} → {dst1, dst2, dst3}; channel mode, control mode and **9 × s32** matrix words | `_PureC` | eligibility per channel mode (C1 checks src1 / dst1 …); stride ≥ width; the coefficient width §4.3 |
| Image_Dot → MI_MVE_Image_Dot | 0x20 | {src1, src2} → dst; mode | `_PureC` | — |
| AlphaBlending → MI_MVE_AlphaBlending | **0x21** (ALPHA_BLENDING) | {src1, src2, alpha} → dst; for U8C3_PLANAR the job runs **three times**, one plane each (the images are temporarily re-typed U8C1 and their phys[0] swapped to phys[1], phys[2]); U8C3_PACKAGE needs a U8C1 alpha | `_PureC` | op 0x21 exists only in the i6c driver numbering; the camera's kernel has its HAL function (`kernel-drivers.md` §8.1.2) |
| Acc → MI_MVE_Acc | — | no HW | `_Neon` only | §5.4 |
| Resize → MI_MVE_Resize | — | no HW | `_Neon` (vendor bilinear kernels) | §5.5 |
| Gmm, CannyHysEdge, CannyEdge, Ccl, EqualizeHist, NormGrad, LkOpticalFlow, LineFilterHor / Ver, NoiseRemoveHor / Ver, Shift_Detector | — | no HW | C / Simd | §5 |

### 4.2 CSC coefficient sets (`.rodata` 0xa6e40, 8 × 32 B, used by Csc and FilterAndCsc on the HW path) [R values; I meaning]

Layout = `ive_ioc_coeff_csc`: `u16 coeff[9]` (**12-bit** two's complement, Q8: 256 = 1.0 — the HAL register fields
`csc_coeff0…8` are 12 bits wide [S i6e `hal_ive_reg.h`]), `u16 offset[3]` (12-bit two's complement), `{u8 lo, u8 hi}
clamp[3]` (all 0 / 255 here). Selection (0x1bd34): MVE mode 0 (YUV→RGB) and 1 (YUV→BGR) pick by **dst** type, 2 (RGB→YUV)
and 3 (BGR→YUV) by **src** type: type 10 / 11 (U8C3) → the first entry of the pair, else → the second.

| VA | mode / type | coeff rows (signed) | offsets |
|---|---|---|---|
| 0xa6e40 | 0, U8C3 | [0 453 256] [−183 −88 256] [359 0 256] | −227, 135, −179 |
| 0xa6e60 | 0, other | [256 0 453] [256 −183 −88] [256 359 0] | −227, 135, −179 |
| 0xa6e80 | 1, U8C3 | [359 0 256] [−183 −88 256] [0 453 256] | −179, 135, −227 |
| 0xa6ea0 | 1, other | [256 359 0] [256 −183 −88] [256 0 453] | −179, 135, −227 |
| 0xa6ec0 | 2, U8C3 | [−21 −107 128] [128 −85 −43] [29 150 77] | 128, 128, 0 |
| 0xa6ee0 | 2, other | [29 150 77] [−21 −107 128] [128 −85 −43] | 0, 128, 128 |
| 0xa6f00 | 3, U8C3 | [128 −107 −21] [−43 −85 128] [77 150 29] | 128, 128, 0 |
| 0xa6f20 | 3, other | [77 150 29] [128 −107 −21] [−43 −85 128] | 0, 128, 128 |

These are BT.601 full-range matrices in Q8 (1.772·256 = 453, 1.402·256 = 359, 0.344·256 = 88, 0.714·256 = 183;
0.299 / 0.587 / 0.114·256 = 77 / 150 / 29; 0.5·256 = 128; 0.169 / 0.331 / 0.419 / 0.081·256 = 43 / 85 / 107 / 21) [R values].

**Engine channel order** [I, strong — every entry fits only this reading]: the offsets identify the output rows
(−1.772·128 = −227 is B, −1.402·128 = −179 is R, +135 is G; 0 is Y, 128 is U / V), and the coefficients the input columns.
In all four reachable ("U8C3") entries **matrix column c multiplies input channel 2−c and row r produces output channel
2−r**, channels counted in memory order (Y, U, V for the YUV side as the Y plane then U, V of the NV12-style UV plane;
package byte order for U8C3) — the engine numbers colour channels in reverse memory order:
- 0xa6e40 YUV2RGB: rows (B, G, R) × columns (V, U, Y) → memory output R, G, B;
- 0xa6e80 YUV2BGR: rows (R, G, B) → memory output B, G, R;
- 0xa6ec0 RGB2YUV: columns (B, G, R) × rows (V, U, Y) → memory input R, G, B, output Y, U, V;
- 0xa6f00 BGR2YUV: columns (R, G, B) → memory input B, G, R.

So the engine computes `out[2−r] = clamp(Σ_c coeff[r][c] · in[2−c] / 256 + offset[r])` with `coeff[r][c]` = `coeff[3r + c]`
[I: the Q8 division and rounding inside the engine are not visible, §4.5]. The "other" entries (dst / src not U8C3) are not
reachable through valid calls.

### 4.3 Matrix_Transform coefficient width [R]
<!-- ev:ive-video-analytics.4.3 -->

The library fills `{u32 chl_mode; u32 ctrl_mode; s32 matrix[9]}` (44 B) at config +0x3C, and the kernel's
`ive_hal_set_coeff_matrix_transform` reads the nine matrix entries as **32-bit words**
at +8…+0x28 of the coefficient block (low half into one register field, bytes of the full word into another). The i6e
`mdrv_ive_io_st.h` declares `__u16 s16MatrixArray[9]` [S], which would put the entries at 2-byte steps; the camera's ABI for
op 0x1F is the 32-bit one, in the library and the kernel alike. Channel mode 0 / 1 / 2 (C1 / C2 / C3) uses the first
**1 / 4 / 9** words (a 1×1, 2×2, 3×3 matrix) and writes 0x0d0d / 0x0e0e / 0x0f0f to its mode register; each 32-bit word goes
to the engine as two 16-bit register halves (low, high), so the coefficient is a full 32-bit fixed-point value [R kernel
code]. The CPU version reads the words as Q15 (§4.4), so the HW coefficient words are Q15 too [I]; neither source states
the engine's Q format.

### 4.4 CPU versions of the HW operations (what runs when the images are not HW-eligible) [R unless marked]
<!-- ev:ive-video-analytics.4.4 -->

| op | CPU implementation and arithmetic |
|---|---|
| Filter `_PureC` | 5×5 correlation `Σ mask[i]·p` (mask row-major, s8), then `bool ? min(max(Σ,0)>>norm,255) : min(|Σ|>>norm,255)` (§4.1) |
| Filter / Sobel `_Neon` | 5×5 correlation with **16-bit accumulators** (`vmla.i16`), so a sum outside ±32767 wraps; the dispatcher only checks that the 25 coefficients sum to −127…127 [R; I: large-coefficient masks give wrong results on this path] |
| Map `_PureC` | U8C1 → U8C1, `dst = lut[src]` (both strides honoured) |
| Hist `_PureC` | 256 × u32 counts over width × height (stride honoured), four partial tables summed; written to `mem.virt` |
| Image_Dot `_PureC` | S32C1 × S32C1 → S32C1, `p = (s64)a·b`; **mode 0: `sign(p)·((|p| + 0x4000) >> 15)`** (Q15, round half away from zero); **mode ≠ 0: `sign(p)·(|p| >> 15)`** (truncate) — no clipping despite the name; src2 is indexed with src1's stride (strides must match) |
| Matrix_Transform `_PureC` | S32 planes, matrix words in **Q15**; each product `m·s` rounded (mode 0) or truncated (mode 1) as for Image_Dot, **then** summed in 32 bits: C1 `d = r(m0·s)`, C2 `d1 = r(m0·s1)+r(m1·s2)`, `d2 = r(m2·s1)+r(m3·s2)`, C3 the same with a 3×3 matrix; other control mode → "CtrlMode not supported~" |
| AlphaBlending `_PureC` | dst ← copy of src2, then per plane Simd `AlphaBlending`: `dst = (s1·α + dst·(255−α) + 1 + ((s1·α + dst·(255−α)) >> 8)) >> 8` (Simd `DivideBy255`); YUV420SP: the UV plane is blended byte-wise with the alpha image's second plane; U8C3_PACKAGE with a U8C1 alpha uses channel count 3 |
| LBP `_PureC` | 3×3 code, bits 0…7 = neighbours TL, T, TR, L, R, BL, B, BR; centre from src1 (channel mode 0) or **src2** (channel mode 1); mode 0 NORMAL `n − c ≥ (s8)thr`, mode 1 ABS `|n − c| ≥ thr`, **mode 2 ABS_MUL `|n − c| > (c·k) >> 3` with k = min(thr + 1, 8) written back into the caller's ctrl** (the HW path writes min(thr, 7) and sends thr + 2 — the two paths disagree [R; I bug]); border fix 3 |
| BAT `_PureC` | binary-image transition counter: mode 0 / 1 → `hor[y] = 1` if the number of value changes along row y ≥ `u16HorTimes` (hor zeroed first, `hor.size` ≥ height); mode 0 / 2 → `ver[x] = 1` likewise down column x with `u16VerTimes` (`ver.size` ≥ width); other mode → "Operation mode is not supported" |
| AdpThresh `_PureC` | strides of src, integ, dst must be ≥ width and multiples of 16; dst zeroed; for each pixel with a full window (x ∈ [hx, w−hx), y ∈ [hy, h−hy)): window sum S from the **inclusive** U32 integral image over [x−hx, x+hx]×[y−hy, y+hy], A = window area; **dst = 0 iff p < valueThr and 10·A·(p + offset) ≤ rateThr·S**, else 255 (dark relative to the local mean → 0); the border band stays 0 |
| Integ `MI_MVE_Integ_PureC` (0x36660) / `MI_MVE_Integ_Neon` (0x465e4) | `_PureC` checks src type 0 (U8C1), equal src / dst width and height, and the dst type per mode: SUM needs type **13**, SQSUM and COMBINE type **15**, else 0x10000104; `_Neon` (SUM) accepts dst type 12 and 13 alike. Every mode is the **inclusive** integral `I(x, y) = Σ src[0..y][0..x]` with dst the same size as src — no extra zero row or column: SUM u32; SQSUM u64 `Σ src²`; **COMBINE u64 `(Q << 28) \| (S & 0x0FFFFFFF)`** (bits 0–27 = S mod 2²⁸, bits 28–63 = Q mod 2³⁶; word0 = sq<<28 \| sum & 0x0FFFFFFF, word1 = sq >> 4), row 0 included. COMBINE [R 0x367c4–0x369c0, 0x36c40–0x36c78] grows the handle's CPU work buffer (§3.1) to **20·W·H** bytes: packed output image (8·W·H), u64 square-sum image (8·W·H), u32 sum image (4·W·H). Row 0: a running row sum `s` and square sum `q` per pixel are stored into the two sum images and packed directly; rows ≥ 1: each pixel adds its row-prefix sums to the previous row of the sum images (`S = S_up + s`, `Q = Q_up + q`, 64-bit with carry) and packs the pair; finally each packed row is copied to dst with `memcpy(dst + y·stride·8, scratch + y·W·8, W·8)`. The packing truncates silently: S wraps past 2²⁸ − 1 (from about 1 Mpixel of value 255 [I arithmetic]), Q past 2³⁶ − 1 [R mask / shift; I consequence]. Verified by running the three routines under unicorn on random U8C1 images (24×6, 17×5) against the inclusive integral: 0 mismatches in every mode [R] |
| Add `_Neon` | requires `x + y = 65536` like the HW; U8C1: weights reduced to 8 bits (`x >> 8`, `y >> 8`), `dst = (a·xh + b·yh [+128]) >> 8` — `+128` (NEON `vrshrn`) in mode 0 ROUNDING, truncation in mode 1 CLIPPING; U16C1: Q16 weights, `>> 16` with the same rounding choice [R mnemonics; I: the 8-bit weight reduction makes U8 results differ from the HW's Q16 by up to ±1] |
| Sub `_Neon` | mode 0 ABS `|a − b|` (`vabd.u8`); mode 1 SHIFT `(a − b) >> 1` as a signed byte (`vhsub.u8`) |
| And / Or / Xor `_Neon` | byte-wise |
| Thresh `_Neon` | U8 compare-and-select per the eight `MI_IVE_ThreshMode_e` formulas (`vcgt` / `vcge` against low / high) |
| Thresh_S16 / Thresh_U16 `_Neon` | 16-bit compares then narrowed (`vqshrn #8` / `vmovn`) to the 8-bit min / mid / max / original |
| 16BitTo8Bit `_Neon` | src / dst the same size; `k = ((numerator << 16) / denominator) & 0xFFFF` (udiv) — **kept to 16 bits, so numerator = denominator gives k = 0**. S16 inputs: `t = sat_s16((x · k) >> 8)` with **`vmull.s16`, i.e. k read as a signed 16-bit value** — for numerator / denominator ≥ 1/2 (k ≥ 0x8000) the multiplier is negative and the sign of every result flips [R 0x5d184; I: vendor bug]; then S16→S8 `sat_s8(t >> 8)`, S16→U8 ABS `sat_u8(|t| >> 8)` (`vabs.s16` + `vqshrn.u16`), S16→U8 BIAS adds `bias` (scaled `<< 16` before the shifts); U16→U8 uses `vmull.u16` (unsigned, no sign problem) and `sat_u8` |
| NCC `_Neon` | three u64 sums Σa·b, Σa², Σb² over the image (u8 products widened with `vpaddl`) — the `MI_IVE_NccDstMem_t` fields |
| Dilate / Erode `_Neon` | 5×5: OR (Dilate, start 0) / AND (Erode, start 255) of the neighbours whose mask byte ≠ 0 — exact for 0 / 255 images [R decompile 0x3d118 / 0x3d530] |
| OrdStatFilter `_Neon` | 3×3 `vmin.u8` / `vmax.u8` network: median, maximum or minimum of the 9 pixels [R mnemonics] |
| Bernsen `_Neon` | 3×3 or 5×5 local min / max (`vmin` / `vmax`); NORMAL: `T = (max + min) >> 1`; THRESH: **`T = (((max + min) >> 1) + thr) >> 1`** (two `vhadd.u8`); `p ≥ T` → set. The MVE bool (= MI `bInstant`) picks the output levels: **0 → 0 / 255, ≠ 0 → 0 / 1** (`vshr #7`) [R 0x5b7f0 / 0x5bb44 (3×3) and 0x5becc / 0x5c6a4 (5×5)]; border fix by window size |
| MagAndAng `_Neon` (0x42474) | see below |
| SAD `_Neon` | src1 / src2 the same size; block mode 0–2, out mode 0–4; per 4×4 / 8×8 / 16×16 block `S = Σ|a − b|` (`vabal.u8`, widened pairwise), one output element per block; 16-bit outputs store S (u16); **8-bit outputs store S >> 4 / >> 6 / >> 8** for 4×4 / 8×8 / 16×16 (the shifts that map the block maximum 16·255 / 64·255 / 256·255 to ≤ 255); the threshold image gets `maxVal` if the 16-bit S > `u16Thr`, else `minVal` [R shifts and compare; I: shift-to-block-size pairing from the ranges] |
| CSC `_Neon` (0x463a8) | width and height even; matrices on the stack, rows in output memory order, columns in input order (Y, U, V / R, G, B): **YUV→RGB (modes 0, 1) in Q6** with kernel 0x44934 (`vqshrn #6`): mode 0 rows R [64 0 89], G [64 −22 −45], B [64 113 0]; mode 1 the R and B rows swapped (BGR); **RGB→YUV (modes 2, 3) in Q8** with kernel 0x432a8 (`vqshrn #8`, +128 bias constants for U / V): mode 2 rows Y [77 150 29], U [−43 −85 128], V [128 −107 −21] over (R, G, B); mode 3 the columns reversed (BGR). The CPU path therefore uses **coarser YUV→RGB coefficients (Q6) than the HW table (Q8)** — results differ by a few levels [R values; I: size of the difference]. FilterAndCSC `_Neon` (0x3fd28): the 5×5 filter on Y, then the same CSC [I: order from the call sequence] |
| Acc, Resize | §5.4, §5.5 |

**MagAndAng `_Neon`** (0x42474, Ghidra 0x52474) [R: emulation].
`MI_MVE_MagAndAng_Neon(h, src, mag, ang, ctrl)`, `ctrl` = `{u32 mode; u16 thr; s8 mask[25]}` (+0, +4, +6). The mask is
copied row-major by 0x3a0d8, which returns **3** when the outer ring of the 5×5 mask is all zero (then only the centre 3×3 is
copied) and **5** otherwise; the function then runs one of four loops: mode 0 / mode 1 × 3×3 (inline) / 5×5 (0x41500,
0x40560). Any other mode → 0x10000104. After the loop the border fix 0x39a58 runs with 5 (§3.7).
- Two 16-bit accumulators: **Gm** (q14) = Σ mask[r][c] · p(y+r−k, x+c−k), and **Gt** (q9) = the same pixels with the
  transposed mask.
- **mag = sat16(|Gm| + |Gt|)**, written only where `mag ≥ thr` (else 0).
- **angle** (mode 1) = `table[bit3|bit2|bit1|bit0]` by `vtbl.8`, table `.rodata` 0xa7724 = `0 7 6 7 0 1 2 1 4 5 6 5 4 3 2 3`,
  with [R: 3×3 angle loop 0x42c24–0x42ed8; constants set at 0x42af8–0x42b04] **bit3 = (Gm < 0)** — the sign of the
  mask-direction gradient (`vcgt q5=0 > q14` & `d9 = 8`), **bit2 = (Gt < 0)** — the sign of the transposed one (`& d14 = 4`),
  **bit1 = (|Gt| > |Gm|)** (`& 2`), **bit0 = (min(|Gm|,|Gt|)·2.4142 ≥ max(|Gm|,|Gt|,1))** (2.4142 ≈ tan 67.5° from shifts
  `<<1 + >>2 + >>3 + >>5 + >>7 + >>13`) — 8 direction bins of 45°.
- Runs of the code with x-derivative masks (3×3 Sobel `[-1 0 1; -2 0 2; -1 0 1]`, and a 5×5 one with the outer ring set so
  the 5×5 loop runs; image rows grow downward) [R]:

| image (row 8) | Gm | Gt | angle 3×3 | angle 5×5 | mag 3×3 / 5×5 |
|---|---|---|---|---|---|
| value grows to the right | + | 0 | 0 | 0 | 32 / 48 |
| value grows to the left | − | 0 | 4 | 4 | 32 / 48 |
| value grows downward | 0 | + | 6 | 6 | 32 / 48 |
| value grows upward | 0 | − | 2 | 2 | 32 / 48 |
| value grows right and down | + | + | 7 | 7 | 64 / 96 |

  So with a mask that differentiates along x (Gm = ∂/∂x, Gt = ∂/∂y, y down) the bin is
  **`k = round(atan2(−Gt, Gm) / 45°) mod 8`**: 0 = +x (right), 2 = up, 4 = −x (left), 6 = down — counter-clockwise on the
  screen [I from the table and the runs]. The 3×3 and 5×5 loops give the same bins [R]. Ties: `|Gt| = |Gm|` takes bit1 = 0
  (strict `>`); a pixel with Gm = Gt = 0 gets code 0 → bin 0 [R: table; I: no special case seen]. Magnitude 32 = 4 · (1+2+1)
  · 2 for slope 4 [R].

### 4.5 Engine-side arithmetic: not known

The arithmetic inside the IVE engine — the Q8 CSC division and rounding, the Filter shift of negative sums, Image_Dot /
Matrix_Transform rounding, the MagAndAng angle bins, the SAD 8-bit scaling, the AlphaBlending rounding — is neither in this
library nor in the kernel HAL. The CPU versions of §4.4 are the library's own reading; whether the engine matches them is
not known. Only runs of the HW ops on the engine with test images compared with the CPU versions (a test program on the
camera, CAMERA-CHECKS P16) or engine documentation can settle it.

## 5. CPU-only operations and MVE-only exports

The functions below have no `*_Acc` HW builder; several of them call HW-capable MVE operations inside and then follow the
dispatch rule of §3.5 for those steps. "MVE-only" = exported by MVE, with no MI_IVE wrapper.

### 5.1 `MI_MVE_AllocateImage(h, img, type, stride, width, height, name, phys)` / `MI_MVE_FreeImage(h, img)` [R]

Zeroes `img`, sets type / width / height, then allocates per plane (MMA with plane index 0 / 1 / 2 if `phys` = 1, else
`malloc` with phys = virt). `stride` is in elements and stored as given:

| type | planes and byte sizes (s = stride, h = height) | strides |
|---|---|---|
| 0 U8C1, 1 S8C1 | s·h | s |
| 2 YUV420SP | Y s·h, UV s·h/2 | s, s |
| 3 YUV422SP, 7 S8C2_PLANAR | s·h, s·h | s, s |
| 4 YUV420P | Y s·h, U s·h/4, V s·h/4 | s, s/2, s/2 |
| 5 YUV422P | Y s·h, U s·h/2, V s·h/2 | s, s/2, s/2 |
| 6 S8C2_PACKAGE, 8 S16C1, 9 U16C1 | s·h·2 | s |
| 10 U8C3_PACKAGE | s·h·3 | s |
| 11 U8C3_PLANAR | 3 × s·h | s, s, s |
| 12 / 13 32-bit | s·h·4 | s |
| 14 / 15 64-bit | s·h·8 | s |

`FreeImage`: if plane 0 phys == virt → `free` each plane, else the MMA free per plane with the same sizes.

### 5.2 Layout converters (MVE-only, no HW) [R]

| export | accepts | does |
|---|---|---|
| `MI_MVE_YUV420Pto444(h, src, dst)` | src YUV420P (4) → dst U8C3_PLANAR (11), same w / h | Y: one `memcpy` of h·src.stride bytes; U, V: `StretchGray2x2` (pixel doubling) |
| `MI_MVE_YUV444to420P(h, src, dst)` | U8C3_PLANAR → YUV420P | Y: Simd copy; U, V: `ReduceGray2x2` (2×2 average, rounded) |
| `MI_MVE_DeinterleaveUv(h, src, dst)` | YUV420SP→YUV420P or YUV422SP→YUV422P (types 2→4, 3→5) | Y copy, `DeinterleaveUv` (w/2 × h/2 or h) |
| `MI_MVE_InterleaveUv(h, src, dst)` | YUV420P→YUV420SP, YUV422P→YUV422SP | Y copy, `InterleaveUv` |
| `MI_MVE_DeinterleaveBgr(h, src, dst)` | U8C3_PACKAGE → U8C3_PLANAR | `DeinterleaveBgr` (row length 3·stride) |
| `MI_MVE_InterleaveBgr(h, src, dst)` | U8C3_PLANAR → U8C3_PACKAGE | `InterleaveBgr` |

Wrong formats → 0x10000104 with the messages "Only supports YUV420SP->YUV420P or YUV422SP->YUV422P!" and the like.

### 5.3 ANN stubs [R]

`MI_MVE_ANN_CreateTable()` and `MI_MVE_ANN_MLP_Predict()` return **0x10000106** and do nothing.
`MI_MVE_ANN_MLP_LoadModel(path, model)`: `fopen(path, "rb")` (no NULL check: a missing file crashes in `fread`), reads a
0x28-byte header into `model`, `malloc`s `model+0x0C` bytes into `model+0x08` and reads them; a short read → "Error while
reading model file", 0x10000104. `MI_MVE_ANN_MLP_UnloadModel(model)` frees `model+0x08` and `model` itself (so `model` must
come from `malloc`). No predictor exists: the MLP part is an unfinished stub.

### 5.4 Acc (`MI_MVE_Acc(h, src0, src1, dst, ctrl)` → `MI_MVE_Acc_Neon`, 0x4d948) [R]

All three images must have the same width, height, stride and type (any of the 16 types; the plane geometry comes from a
per-type table at `.rodata` 0xa7918, 28 B per type: plane count and width / height factors). Per byte, **wrapping 8-bit
arithmetic** (NEON `vadd.i8` / `vsub.i8`, no saturation): mode 0 INCREASE `dst = src0 + src1`; mode 1 DECREASE
`dst = src0 − src1`; mode 2 INCREASE_MAP_255TO1 `dst = src0 + (src1 & 1)` (so a 0 / 255 mask adds 0 / 1). Other modes →
0x10000104.

### 5.5 Resize (`MI_MVE_Resize(h, src, dst, ctrl)` → `MI_MVE_Resize_Neon`, 0x476e8) [R]

Width ≤ stride on both images, else 0x10000104. Mode (= `MVE_IVE_ResizeMode_e`): 0 U8C1 → one gray pass; 1 U8C3_PLANAR →
three gray passes; 2 U8C3_PACKAGE → the 3-channel kernel on rows of 3·stride; 3 YUV420SP → width, height and stride of both
images must be even, then the gray kernel on Y and the 2-channel kernel on the UV plane at half size; other → 0x10000104.
Kernels: 0x56ef4 (1 ch), 0x56664 (2 ch), 0x55dd0 (3 ch) — vendor code, not Simd (the Simd ResizeBilinear functions in the
library are unreached, §7).

**Bilinear, pixel-centre aligned**: for every destination column / row `s = (d + 0.5)·(src_size / dst_size) − 0.5`, index
`i = floor(s)`, weight of the right / lower neighbour `round((s − i)·2048)` (Q11, 2048 = 1.0); `s < 0` → index 0 with full
weight on it; `i ≥ src_size − 1` → index `src_size − 2` with full weight on the last pixel. Column / row tables are `malloc`ed
per call (failure → "Buf is NULL", 0x1000010B). Arithmetic [R disassembly 0x572e8–0x57440]: horizontal pass per source row
`t[x] = (p[i]·α0 + p[i+1]·α1) >> 4` (α0 + α1 = 2048, stored as s16); vertical pass
`out = min(255, max(0, ((β0·t0[x]) >> 16) + ((β1·t1[x]) >> 16) + 2) >> 2))` (β in Q11; NEON `vmull.s16` / `vsra #16` /
`vshrn #2` / `vqmovun`, the scalar tail the same with `+2 >> 2`). This is **bit-for-bit OpenCV's fixed-point
`INTER_LINEAR`** for 8-bit images (`INTER_RESIZE_COEF_BITS` 11, `VResizeLinearVec_32s8u`) [R arithmetic; I: identity with
OpenCV].

### 5.6 Median_Filter (`MI_MVE_Median_Filter(h, src, dst, ctrl {mode, size})`, 0x5d5cc, MVE-only) [R]

src and dst the same type, U8C1 (channel count 1) or U8C3_PACKAGE (3), the same width / height (messages "Src/dst image
format must be the same", "… must be MVE_IMAGE_TYPE_U8C1 or MVE_IMAGE_TYPE_U8C3_PACKAGE", "Src/dst width and height must be
the same", all 0x10000104). Mode 0 = **square** window, 1 = **rhomb** (diamond) window; size 0 = 3×3, 1 = 5×5; anything else
→ "Invalid enMode or enSize". The four cases call Simd's `MedianFilterSquare3x3 / Square5x5 / Rhomb3x3 / Rhomb5x5(src,
srcStride, w, h, channels, dst, dstStride)` (Neon versions behind the usual width ≥ 16 dispatcher, §3.6) — upstream
semantics, borders handled inside Simd [I: upstream clips the window at the edges; not traced in this binary]. These 41
template instances are the 155 KB of §1.1.

### 5.7 GMM (`MI_IVE_Gmm` → `MI_MVE_GMM` → `MI_MVE_GMM_PureC`, 0xda08, 7.2 KB) — two OpenCV algorithms behind one call [R constants and branches; I identity]

Arguments: `(h, src, fg, bg, MVE_MEM_INFO_S *model, ctrl)`; NULL fg / bg / model / ctrl → 0x10000105; src, fg, bg the same
width / height (else 0x10000104); src U8C1 (1 channel) or U8C3_PACKAGE (3 channels); fg U8C1; bg U8C1 or U8C3_PACKAGE.
`ctrl` = `MI_IVE_GmmCtrl_t` {u22q10NoiseVar @0, u22q10MaxVar @4, u22q10MinVar @8, u0q16LearnRate @0xC, u0q16BgRatio @0xE,
u8q8VarThr @0x10, u0q16InitWeight @0x12, u8ModelNum @0x14}. **Which algorithm runs is chosen by MaxVar / MinVar** [R branch at
0xdfd4]:

**A. MaxVar = 0 or MinVar = 0 → KaewTraKulPong–Bowden MOG, = OpenCV legacy `BackgroundSubtractorMOG` (`process8uC1/C3`)** —
the same defaults (history 200, 5 mixtures, background ratio 0.7, varThreshold 2.5², noise σ 15, initial weight 0.05) and the
same update: per pixel the modes {weight, mean, var} (model buffer, float; the per-mode `sortKey` is kept in the GMM scratch
h+0x08 / +0x0C, w·h·K floats) are scanned in order until weight < FLT_EPSILON; the first with `d² < vT·var` is updated
`w += α(1−w)`, `μ += α·d`, `var = max(var + α(d² − var), minVar)`, `sortKey = w/√var` and bubbled up by sortKey; no match →
the last (or first empty) mode becomes {w0, pix, var0, sk0}; weights renormalised; fg = 255 if the hit index ≥ the index
where the cumulative weight first exceeds T. **bg** = for each pixel `Σ w·μ / Σ w` over the modes until the cumulative
weight exceeds T, +0.5, truncated (OpenCV's `getBackgroundImage`, computed every call). Parameters:

| quantity | from ctrl | default (when the field is 0 / out of range) |
|---|---|---|
| noise σ | NoiseVar / 1024 (used as σ, not σ²; accepted 1…0xFFFFFF) | 15 |
| minVar | σ² | 225 |
| var0 (new mode) | 4·σ² | 900 |
| learning rate α | LearnRate / 65536 | 1 / min(frames, 200) |
| T (background ratio) | BgRatio / 65536 | 0.7 |
| vT | **VarThr / 65536** (the header says U8Q8, i.e. /256 [R constant 1/65536; I mismatch]) | 6.25 |
| w0 | InitWeight / 65536 | 0.05 |
| sk0 | w0 / 30 (1 channel), w0 · 0.019245 = w0/(30√3) (3 channels) | — |
| K | ModelNum 3 or 5, else 5 | 5 |

**B. MaxVar ≠ 0 and MinVar ≠ 0 → Zivkovic MOG2, = OpenCV `BackgroundSubtractorMOG2`** — the parameter block is
re-initialised (and the model cleared) when switching between A and B, and on a size or channel change: history **500**,
varThreshold **16** (4²), varThresholdGen **9** (3²), background ratio **0.9**, varInit **15**, fVarMax = MaxVar/1024 (default
**75**), fVarMin = MinVar/1024 if MinVar ≤ MaxVar else **4**, complexity reduction fCT **0.05**, shadow threshold τ **0.5**,
shadow value **127**; α = LearnRate/65536 or `1 / min(2·frames, history)`. In this branch only MaxVar, MinVar, ModelNum and
LearnRate are read from ctrl — **VarThr, BgRatio, NoiseVar and InitWeight are ignored** [R: no other ctrl access in the
branch]. fg is written 0 / 255 only (`−(bg ^ 1)`); the stored shadow value 127 and τ are never read, so there is **no
shadow marking** [R: no read found]. If MaxVar < MinVar the two fields are **swapped and written back into the caller's
ctrl** [R]. Model layout and the per-pixel loop are OpenCV's MOG2 `MOG2Invoker` (modes {weight, mean, var},
`bgmodelUsedModes` byte map in the `calloc`ed scratch, w·h bytes) [I: the structure matches the constants].

Both variants keep **process-global state** (`.bss` ≈ 0xc5100–0xc5270: geometry, frame counter, parameters, a 1 / 2 flag for
A / B): two GMM instances in one process share one model geometry and frame count [R addresses; I consequence].

### 5.8 EqualizeHist (`MI_IVE_EqualizeHist` → `MI_MVE_EqualizeHist`, 0x3e330) [R]

`(h, src, dst, MVE_MEM_INFO_S *mem)`; src must be U8C1 (else 0x10000104). `mem->virt` is used as
`MI_IVE_EqualizeHistCtrlMem_t` {u32 hist[256]; u8 map[256]} (0x500 B; its size is not checked). **Pixels are read and written
as one contiguous block of width·height bytes from `virt[0]`** — the stride is ignored, so the result is correct only when
stride = width.
1. `hist` = 0, `map` = identity; count every pixel.
2. `imin` = the first bin with a non-zero count; from there `hist` is made cumulative in place (`hist[i] += hist[i−1]` for
   i > imin).
3. `map[i]` = 0 for i ≤ imin; for i > imin `map[i] = lroundf((hist[i] − hist[imin]) · 255 · (1 / (N − hist[imin])))`, N = w·h
   (float, `lroundf` = round half away from zero).
4. `dst[k] = map[src[k]]` over the same contiguous block.

Differences from OpenCV `equalizeHist` (otherwise the same formula): the rounding (`lroundf` vs `cvRound`), and a constant
image maps to 0 (OpenCV keeps the value) [R formula; I comparison]. After the call `mem` holds the cumulative histogram and
the LUT.

### 5.9 Canny (`MI_IVE_CannyHysEdge` + `MI_IVE_CannyEdge`) = OpenCV's `Canny` split in two [R code; I identity: the same constants and map coding]

`MI_MVE_CannyHysEdge(h, src, edge, MVE_MEM_INFO_S *stack, ctrl)` → `_PureC` (0x37a24); ctrl = {MVE_MEM_INFO_S mem @0 (work
memory), u16 low @0x0C, u16 high @0x0E, s8 mask[25] @0x10}. Checks: src U8C1, edge the same size, NULL → 0x10000105;
`mem.size` ≥ 12·w + 2·align4(2·w·h), else it fails.
1. **Smoothing**: `MI_MVE_Filter(h, src, edge, {fixed 5×5 mask, norm 8}, 1)` with the mask at `.rodata` 0xa7528 =
   `3 6 9 6 3 / 6 15 18 15 6 / 9 18 24 18 9 / 6 15 18 15 6 / 3 6 9 6 3` (sum 252, /256, negative clamped to 0); the result goes
   into `edge` as a temporary.
2. **Gradients**: `MI_MVE_Sobel(h, edge, {S16 images in mem}, {mode VER (2), user mask})`, flush / invalidate, copy aside,
   then `MI_MVE_Sobel(…, mode HOR (1))`; any failure → "ERROR @ MI_MVE_Filter()" / "ERROR @ MI_MVE_Sobel - Vertical()" /
   "…- horizontal". Both calls go through the normal dispatch (§3.5), so **Canny works only where Filter and Sobel do: with
   the default padding mode the Sobel step needs HW-eligible images (16-aligned width, physical `mem`), otherwise the whole
   call returns NOT_SUPPORT** [R path; I consequence].
3. **Magnitude** L1: `|gx| + |gy|` (int), three rows kept in a ring.
4. **Non-maximum suppression** exactly as OpenCV (`CANNY_SHIFT` 15, `TG22 = 0x3505` = tan 22.5°·2¹⁵, horizontal / vertical /
   the two diagonals chosen by the sign of gx·gy), only where `m > low`.
5. **Map** (in `edge`, contiguous w·h bytes — stride ignored): 1 = not an edge (also the first / last row and column), 0 =
   weak candidate, 2 = strong (`m > high`, not after a strong left neighbour, upper neighbour not 2); the address of every
   strong pixel is pushed on the stack.
6. Stack (`stack->virt`): `u32 count` at +0, 12 reserved bytes (`MI_IVE_CannyStackSize_t`), then `count` **32-bit pixel
   addresses** (process virtual addresses into `edge`); overflow test before each row `size − 16 < count + w` →
   0x1000010B; the test compares bytes with 4-byte entries, so the stack can run up to four times past the end of the
   buffer before it trips [R code; I: a vendor bug].

`MI_MVE_CannyEdge(h, edge, stack)` → `_PureC` (0x383a8): pop addresses, mark every 8-neighbour that is 0 as 2 and push it
(stack bound checked against `stack->size`, 0x1000010B); then `edge[k] = (u8)−(edge[k] >> 1)` → 2 becomes 255, 0 / 1 become
0. Because the stack holds virtual addresses, the two calls must run in the same process with the same `edge` mapping [R/I].

### 5.10 CCL (`MI_IVE_Ccl` → `MI_MVE_CCL`, 0x19884) [R]

`(h, srcdst, MI_IVE_CcBlob_t *blob, MI_IVE_CclCtrl_t *ctrl)` — the MI wrapper runs it on a stack copy of the blob and copies it
out only if `pstBlob->size` = 0xBF8 (`sizeof(MI_IVE_CcBlob_t)`), else `ILLEGAL_PARAM`. src must be U8C1 (else 0x10000106)
and **at most 720 × 640** ("Source size (%dx%d), CCL limit (720x640)", 0x10000106). The image is addressed as contiguous w·h
bytes (stride ignored). Work memory: the CPU work buffer (§3.1) grown to `align4(w·h) + 24·w·h` bytes.
1. Foreground = pixels equal to **255**; everything else is background. Two-pass labelling with an equivalence table (union
   to the smaller root, helper 0x197f8) over the left, upper-left, upper and upper-right neighbours = **8-connectivity**; the
   first row and column are handled by separate loops.
2. Per label a 12-byte record {u32 area, u16 left, u16 right, u16 top, u16 bottom} (initialised to area 0, left = top = 9999,
   right = bottom = 0) accumulated over all its pixels.
3. Count the labels with area > 0. ≤ 255 → `s8LabelStatus` = 0, `u16CurAreaThr` = 0. More → `s8LabelStatus` = **−1** and
   `u16CurAreaThr` = `u16InitAreaThr`; then repeat: every region with area < `u16CurAreaThr` is folded into a spill record
   (area added, box widened) and its label marked 255; if fewer than 256 regions are left stop, else
   `u16CurAreaThr += u16Step`. (`s8LabelStatus` stays −1 although the result is then valid.)
4. The surviving regions are written to `astRegion[0…]` in label order, `u8RegionNum` counts them (reaching 255 → "CCL
   u8RegionNum overflow!!", 0x10000104); every foreground pixel of srcdst is **overwritten with its region number
   (1…255)**, folded ones with 255.
5. If the status is −1 the spill record's area and box are added into `astRegion[254]` [R offsets 0xBEC… of the blob].

### 5.11 NormGrad (`MI_IVE_NormGrad` → `MI_MVE_NormGrad` → `_PureC` 0xc408) [R]

`(h, src U8C1, dstH S8C1, dstV S8C1, dstHV S8C2_PACKAGE, ctrl {mode, s8 mask[25], u8 norm})`; wrong formats → 0x10000104 with
"Destination format must be MVE_IMAGE_TYPE_S8C1" / "…HV … S8C2_PACKAGE" (the HV check only in mode 3); w, h ≥ 5.
- Modes 0 HOR_AND_VER, 1 HOR, 3 COMBINE compute H; modes 0, 2 VER, 3 compute V (bit test `mode & 1` / `mode & 2`, mode 0 =
  both).
- H = 5×5 correlation of src with `mask`, `>> norm` (arithmetic), **saturated to s8** (`SignedSaturate(·, 8)`); V = the same
  with the **transposed** mask (transposed in place on a stack copy).
- Borders: padding mode 1 / 2 → src is first copied into the CPU work buffer with a 2-pixel border by the mode-1 / mode-2
  pad routine (§3.7) and the kernel runs on the padded copy; other modes → the kernel runs on src directly and the output
  border is fixed by §3.7.
- Mode 3: after H and V, `dstHV[2k] = H[k]`, `dstHV[2k+1] = V[k]` over w·h contiguous elements (strides ignored).

### 5.12 LK optical flow (`MI_IVE_LkOpticalFlow` → `MI_MVE_LKOpticalFlow` → `_PureC` 0x33a98 + KLT) [R; KLT field names from `klt.h` 1.3.4]

`(h, prev, cur, MVE_MEM_INFO_S *points, MVE_MEM_INFO_S *mv, ctrl)`; ctrl (MVE) = {u16 cornerNum @0, u8 minEigThr @2, u8
iterCount @3, u8 epsilon @4, u16 searchRange @6 (= **15**, set by the MI wrapper)}. Checks: NULL images / mems → 0x10000105;
cornerNum 1…200, iterCount 1…20, minEigThr ≠ 0, epsilon ≠ 0 [R range tests], else an error [I: 0x10000104; the decompiler
lost that branch body].
1. First call on the handle: KLT tracking context + a feature list of `cornerNum` features (h+0x18 holds both), the
   process-global KLT statics zeroed, defaults window 10, step 1, …, `-10.0` (one float field).
2. Every call: `tc->max_iterations = iterCount`, `tc->sequentialMode = 1`, `tc->min_displacement = epsilon / 255`,
   **`tc->min_eigenvalue = (int)(minEigThr / 255)`** — an integer field, so it is 0 for every value but 255 [R; I: a vendor
   bug], then `KLTChangeTCPyramid(tc, searchRange)` (pyramid levels / subsampling from 15). The first call of the process
   also allocates KLT's sequential-mode pyramids for the image size (0x274c8; failure → 0x1000010B); a later call with
   another size makes `KLTTrackFeatures` fail through `KLTError` — which crashes the process (§6.2) [R/I].
3. Features: point k = {S25Q7 x, S25Q7 y} (8 B) from `points->virt` → float `x/128, y/128`, `val = 1000 + k` (≥ 0 = to track).
4. `KLTTrackFeatures(tc, prev->virt[0], cur->virt[0], w, h, fl)` — images taken as contiguous w×h U8 (stride ignored).
5. Output `mv->virt` as `MI_IVE_MvS9Q7_t[mv->size / 8]` {s32 status, s16 dx, s16 dy}: all first set to {−1, 0, 0}; then for
   every feature still `val ≥ 0`: status 0, `dx = (s16)(x_new·128 − x_in)`, `dy` likewise (S9Q7 displacement, truncated).
   Lost features keep status −1.

### 5.13 `MI_MVE_LK_Optical_Flow_SelectGoodFeatures(h, img, MVE_MEM_INFO_S *out, ctrl {u16 n, u16 minDist}, u8 eig)` (0x33578, MVE-only) [R]

The corner selector that pairs with LK (§5.12); not reachable through MI_IVE. Checks: n 1…200, minDist 1…255, eig ≠ 0, else
0x10000104; NULL image / out → 0x10000105; no context → 0x1000010B. On the handle's first KLT use it zeroes the
process-global KLT statics as LK does. Then it sets the two **process-global KLT defaults** the vendor turned into variables
(`.bss` 0x33630 / 0x33634: KLT's `mindist` and `min_eigenvalue`) to `minDist` and `(int)(eig / 255)` (again an integer, so 0
for eig < 255 [R; I bug]), creates a tracking context (`KLTCreateTrackingContext`, `sequentialMode` = 1) and an n-feature
list, and runs `KLTSelectGoodFeatures(tc, img->virt[0], w, h, fl)` (0x2b86c; image contiguous, stride ignored). Output:
`out->virt` zeroed for `out->size` bytes, then for each feature with `val > 0`, in list order, an 8-byte point
`{(s32)x << 7, (s32)y << 7}` — the KLT float position truncated to an integer and stored as S25Q7 (fraction 0). Features KLT
could not place (val ≤ 0) leave a zero slot [R loop: the output pointer advances for every feature]. The new context
replaces the handle's LK context pointer without freeing the previous one [R: `*piVar19 = new tc`; I: a leak per call].

### 5.14 Shift_Detector (`MI_IVE_Shift_Detector` → `MI_MVE_Shift_Detector`, 0x54e58) = Simd `ShiftDetector` [R calls and asserts; I parameter roles]

The MI wrapper passes one context struct `{mve handle, src1, src2, dstX, dstY, ctrl}`; ctrl = `MI_IVE_SHIFT_DETECT_CTRL_t`
{enMode @0, u8 pyramid_level @4, u8 search_range @5, u16 left @6, top @8, width @10, height @12}.
Checks: src1 / src2 the same width, height, stride and type (else "Source 1/Source 2 are different!"), U8C1 only, dstX / dstY
identical descriptors, ROI width and height ≥ 2, ROI inside the image — all 0x10000104.
Flow: wrap src1 as the **background** and src2 as the **current** `Simd::View` (Gray8, stride as given; temporary aligned
copies via `Simd::Allocate` when needed), build a pyramid of `pyramid_level` levels with `ReduceGray2x2`
(`Simd::ShiftDetector::InitLevels`, `SetBackground` with texture = gray, `AbsGradientSaturatedSum` available for the gradient
texture), `SetCurrent(current, ROI)`, then `Estimate(…, ROI, maxShift = (search_range, search_range), hiddenAreaPenalty = 0,
regionAreaMin = 25)` — the coarse-to-fine exhaustive search of upstream Simd (sum of absolute differences per candidate
shift, `AbsDifferenceSums3x3`, `SquaredDifferenceSum` for the refinement). Result: **one byte** each —
`dstX->virt[0] = (s8)shift.x`, `dstY->virt[0] = (s8)shift.y`, or 0 / 0 when Estimate fails. `enMode` (single / multi) is not
read on this path [R: no use found; I].

### 5.15 NoiseRemoveHor / NoiseRemoveVer (in place; `_PureC` 0x360b0 / 0x3cab0) [R]

`(h, srcdst, ctrl {u8 thr, u8 thrMax})`, both 1…100 (else "Input parameter error.", 0x10000104), srcdst U8C1 (binary, 255 =
set). Hor scans each row (row start = y·stride, `width` pixels), Ver each column (step = stride, `height` pixels): every
maximal run of 255-pixels of length L is **kept if thr ≤ L ≤ thrMax, otherwise cleared to 0**; kept pixels are marked 1
during the scan and set back to 255 when the scan passes them, so the result is again 0 / 255. A run touching the end of the
row / column is judged the same way. ("Hor" removes short and over-long horizontal segments, e.g. noise and long lines.)

### 5.16 LineFilterHor / LineFilterVer (in place; `_PureC` 0x3acf8 / 0xc9d8) [R decompile, transcribed; the final meaning I]

Binary U8C1 image (255 = set); a line is split into alternating runs at every position where the byte value changes (the
first run's state is bit 7 of the first pixel). Run boundaries go into stack arrays of 1920 (Hor) / 1200 (Ver) u16 entries
— **a row longer than ~1918 pixels or a column taller than ~1198 overflows the stack array** [R sizes; I consequence]. In the
formulas a, b are the lengths of the set runs before / after a clear gap of length g.

**Hor** `(h, srcdst, ctrl {u8 gapMinLen 1…20, u8 densityThr 1…50, u8 horThr 1…50})` (out of range → "Input parameter
error.", 0x10000104). Rows are taken as contiguous `width`-byte lines (**stride ignored**). Per row:
1. *Grouping*: walk the set runs left to right; a gap with g < gapMinLen joins the next run to the current group (gap
   lengths summed into G, run lengths into S); a gap ≥ gapMinLen closes the group. A closed group is kept as **one segment
   from its first run's start to its last run's end** iff `S ≥ 2` and `densityThr · S ≥ 10 · (S + G)`; otherwise it is
   dropped (so with densityThr < 10 every group is dropped, and a single run survives only if densityThr ≥ 10). The last
   group of the row is judged the same way.
2. *Joining*: over the kept segments, a segment P followed by Q is joined into one (P.start … Q.end) iff the gap between
   them < horThr, `9 < len(P) < 3·horThr` and `len(Q) > 2·horThr`; otherwise both stay.
3. The row is cleared to 0 and every final segment filled with 255.

**Ver** `(h, srcdst, ctrl {u8 verThr 1…64})`. Columns use the stride as the step. For every clear gap between set runs a
(above) and b (below):
- g ≥ verThr → keep both runs separate (the gap stays 0);
- g < verThr → let `c = (b < 7 or b > 24) ? true : (a > 11)`; if not c → **fill the gap** (merge). If c: when `g < verThr −
  1`, with `m = (b < 11 ? a : b)`, merge if `m < 11 and a + b < 16`; otherwise let `m2 = (b < 8 ? a : b)`: merge if `m2 ≤ 7`,
  else keep.

Then the column is rewritten from the merged list: set segments 255, gaps 0 (pixels outside the list are untouched).
(Reading: Hor closes small gaps inside dense horizontal strokes and drops sparse fragments; Ver bridges short vertical
breaks between short strokes — a text / line clean-up pair [I].)

### 5.17 Otsu (`MI_MVE_Otsu(h, src, dst, u8 *thr)`, 0x19504, MVE-only) [R]

src U8C1 and dst the same width / height (else 0x10000104), NULL virt → "Buf is NULL" 0x1000010B. Histogram over `height`
lines of `width` bytes taken contiguously (stride ignored). In double precision, p_k = hist[k]/N; for k = 0…255:
ω = Σ_{i≤k} p_i, μ₀ = Σ_{i≤k}(i+1)p_i / ω, μ₁ = (Σ_all (i+1)p_i − Σ_{i≤k}(i+1)p_i)/(1 − ω), σ²_B = ω(1−ω)(μ₀ − μ₁)²; the
threshold is the first k with the largest σ²_B (strictly greater replaces). `*thr = k` (0 if k ≤ 0); `dst = src > k ? 255 :
0` over the same contiguous block. This is the standard Otsu method (the +1 offset cancels in μ₀ − μ₁) [I].

### 5.18 RadonHor / RadonVer (`MI_MVE_RadonHor/Ver(h, src, dst, ctrl {u8 ?, u8 thr})`, 0x34864 / 0x34ed0, MVE-only) [R]

A small-angle Radon (projection) transform of a gray image, used to find the skew of near-horizontal / near-vertical
structure [I]. Checks: src U8C1; NULL `virt[0]` → "Buf is NULL" 0x1000010B; `ctrl[0]` must be 1…16 (only range-checked,
never used [R]); `thr` (ctrl[1]) 1…255 ("RadonCtrl u8Thr out of [1, 255]"). Width and height are rounded down to even (W, H).
- **Hor**: 21 angles a = −10…+10°; src stride ≥ align4(W) ("Source stride is too small."); dst must have **height 21** and
  **stride = align4(H)**. Accumulator `acc[21][H]` (u16, `calloc`). For every pixel (x, y) with value v' = min(v, thr) and every
  angle: `d = (x · T[|a|] + 0x800) >> 12`, row `r = (y − sign(a)·d) mod H` (wrapped into 0…H−1), `acc[a][r] += v'`
  **saturating at 0xFFFF**. T = tan(n°)·4096 rounded, n = 0…63 (`.rodata` 0xa73e0: 0, 71, 143, 215, 286, 358, …).
- **Ver**: 23 angles a = −11…+11°; dst **height 23**, **stride = align4(W)**;
  `c = (x − sign(a)·((y · T[|a|] + 0x800) >> 12)) mod W`, `acc[a][c] += v'` **without saturation** (u16 wrap). The kernel
  reads source rows with the **dst stride** as the row step (the same value is passed for both) — correct only when the src
  stride = align4(W) [R: `MI_MVE_RadonVer` passes `dst->stride` as the kernel's source-row step; I: a vendor bug].
- Both: `m` = maximum of all accumulator cells (as double); `dst[a][i] = (u8)(u16)trunc(acc[a][i] · 255/m)` (m = 0 is not
  guarded: 255/0 = inf → trunc of inf·0 / inf [I]). The accumulator is freed; nothing is kept in the handle.

### 5.19 ImageHistMatch (`MI_MVE_ImageHistMatch(h, src1, src2, ctrl {u32 mode; MVE_MEM_INFO_S mem})`, 0x20b38, MVE-only) [R]

Produces **lookup tables** that match the grey-level distribution of one image to another; it does not modify the images.
src1 / src2 the same type, width, height ("Source format must be the same!"); type ∈ {U8C1 (1 channel), YUV420P, YUV422P,
U8C3_PLANAR (3 channels)} (bit mask 0x831; else "Input image format is not supported!!!"), all 0x10000104.
`mem` (physical: it is the output of HW histograms) holds per channel c a 0xA00-byte block at `mem.virt + c·0xA00`:
+0x000 `u32 hist1[256]`, +0x400 `u32 hist2[256]` (become cumulative), +0x800 `u8 lut1[256]`, +0x900 `u8 lut2[256]` (both
initialised to identity). Per channel: `MI_MVE_Hist(h, plane_c of src_i, {phys, virt} of hist_i)` (HW when eligible, §4.1),
flush / invalidate, prefix-sum to a CDF; for YUV420P the chroma planes are processed with width, height and stride halved,
for YUV422P width and stride halved.
Matching rule (per channel, per mapped image): for every level k whose CDF is non-zero, starting the search at the previous
result j₀, take the first j ≥ j₀ where `|T[j] − C[k]|` stops decreasing; `lut[k] = j` (255 if the search runs off the end);
levels with CDF 0 keep the identity value.
- mode 0: `lut2` maps **src2 → src1** (T = CDF of src1);
- mode 1: `lut1` maps **src1 → src2** (T = CDF of src2);
- mode 2: both LUTs map to the **mean CDF** `T = (C1 + C2) >> 1` (computed in the CPU work buffer, `256·channels·4` bytes).

At the end both LUT areas are flushed (cache) so that a HW `Map` can use them; the call returns the OR of the Hist results if
any failed.

### 5.20 OptimalSeam (`MI_MVE_OptimalSeam(h, src1, cost, src2, MVE_MEM_INFO_S *seam, ctrl)` → `_PureC` 0x3c714, MVE-only) [R calls; I meaning]

A stitching helper: finds the cheapest top-to-bottom seam through the overlap of two images. src1, cost and src2 must share
width, height and type ("Width/height of source/cost/destination images must be the same", "…format must be the same");
type U8C1 (1 channel) or U8C3_PLANAR (3 channels, the seam is searched on channel plane 2 — index 2), else
0x10000104. ctrl = {u16 wA @0, u16 wB @2, u8 addFlag @4, u16 seamEndX @6 (out), u16 seamEndY @8 (out)}.
1. **Cost image** (0x3bd60), built with MVE calls (HW path where eligible): per channel `Sub(src1, src2, ABS)` and
   `Sub(…, SHIFT)`, `Thresh` with {low 0x80, max 0xff} and {min 0, mid 0, max 0xff} variants, `And`, `Or` to merge the
   channels into one U8 difference map in `cost` [R call sequence; I: a per-pixel colour-difference magnitude].
2. If `addFlag` ≠ 0: `MI_MVE_Add(cost, {seam->phys/virt + w·h as a second U8 image}, cost, {mode 1 = CLIPPING, x = wA, y =
   wB})` — a weighted blend of the difference map with a caller-supplied prior map stored after the first w·h bytes of
   `seam`.
3. **Dynamic programming** (0x3c130, CPU work buffer 8·w bytes): flush the cost plane; `acc[0][x] = cost[0][x]`; for each
   next row y: `acc[y][x] = cost[y][x] + min(acc[y−1][x−1], acc[y−1][x], acc[y−1][x+1])` (edges use the two existing
   neighbours), then a left-to-right and a right-to-left relaxation within the row (`acc[y][x] = min(acc[y][x], acc[y][x∓1]
   + cost[y][x])`) so the seam may run sideways. Back-pointers are stored as u16 per pixel in `seam->virt` (first row
   0xFFFF): the predecessor column, or 0x8000 + x / 0x7FFF + x codes for a horizontal step [R constants 0x7fff, 0x8001,
   0x7ffe].
4. `ctrl.seamEndX` = the DP routine's return value [I: the column of the minimum in the last row], `ctrl.seamEndY` = height
   − 1 [R]; the caller walks the back-pointers.

### 5.21 MultibandBlending (`MI_MVE_MultibandBlending(h, src1, src2, alpha, dst, ctrl)`, 0x58228, 13.7 KB, MVE-only) [R calls; I: classic Burt–Adelson pyramid blending]

ctrl (bytes) = {u8 num_layers @0, u8 layer_enable[10] @1…10}. Checks: src1, src2, dst the same type, type YUV420SP or
U8C3_PLANAR ("Src1/src2/dst image format must be the same"); for src1, src2, alpha, dst **width = stride** ("…width and
stride must be the same"); `layer_enable[0]` ≠ 0 ("Layer 0 must be enabled"); the pyramid depth is `L = ceil(log2(max(w,
h)))` (log·1.4427); only the first L enable flags count, and the number of enabled ones must equal `num_layers` ("total
layers of layer_enable is not equal to num_layers"), all 0x10000104.
Pyramid geometry: each enabled level after level 0 is a further reduction by 2^k, where k = 1 + the number of disabled
levels skipped since the previous enabled one (a byte list of ratios built on the stack: 2, 4, 8 …). Buffers are
`Simd::View` images held in `std::vector`s (growth helper 0x723f8; the exported `std::vector<double>::_M_default_append`
0x731c0 belongs here too [I]).
1. **Gaussian pyramids** of src1, src2 (per plane: Y and UV for YUV420SP, three planes for U8C3_PLANAR) and of `alpha`: each
   level from the previous by `ReduceGray2x2` repeated per factor 2 (Simd, (a+b+c+d+2)/4).
2. **Laplacian pyramids** (0x57c80): for level i from the top down, `up = MI_MVE_Resize(G[i+1] → size of G[i])` (bilinear,
   §5.5), then two 8-bit planes `P = sat(G[i] − up)` and `N = sat(up − G[i])` (Simd `OperationBinary8u` type 5, saturated
   subtraction) — the signed difference is kept as a positive and a negative part.
3. **Blend** (0x57e88): for every level and plane `MI_MVE_AlphaBlending(P1, P2, alpha_i → P)` and the same for N (and for
   the top Gaussian level), i.e. an alpha-weighted mix as the AlphaBlending op defines it (§4.1; HW when eligible; the
   engine's rounding is not known, §4.5).
4. **Collapse** (0x57fd8): from the top, `up = Resize(R[i+1])`, `R[i] = sat(sat(up + P) − N)` (type 6 saturated add, then
   type 5); R[0] is written to dst [R: a last `MI_MVE_AlphaBlending` call follows the collapse; I: it writes the result
   planes into dst].

Errors of the inner MVE calls are ORed into the result; all views are freed before return.

## 6. Third-party code inside libmi_ive

### 6.1 Simd (github.com/ermig1979/Simd, MIT) — vendor copy v4.4.84 … v4.6.93
<!-- ev:ive-video-analytics.6.1 -->

**Version** [R string match; I: the copy itself is unmodified except as listed]: the 46 Simd assertion texts in `.rodata`
 were compared verbatim with the 24 upstream files the assert paths name, at every v4
tag. v4.4.84 … v4.6.93 contain 44 of the 46; earlier tags lack
`_background.Size() && … == _background[0].format` (SimdShift.hpp), later ones lack `result == 0` (SimdMemory.h `Allocate`).
The two texts never found are vendor additions:
- `Simd::Base/Neon::ReduceGray2x1` (`./../C/src/Function_tools/Simd/Simd{Base,Neon}ReduceGray2x1.cpp`: halve the width only,
  `(srcWidth + 1) / 2 == dstWidth`) — not in any upstream v4 tag [R: file absent];
- `Simd::Base::Reduce2DimSizeGray` (`SimdBaseReduce2DimSizeGray.cpp`: reduce by a power-of-two `ratio` ≤ 512 in both
  dimensions, `(srcWidth + ratio/2) / ratio == dstWidth`, message "reduce size ratio should be power of 2 and up to 512!!").

mi_ldc.ko carries `Base/Neon::ReduceGray2x1` and `Base::Reduce2DimSizeGray` with the same message [R], so the two copies are
one vendor `Function_tools/Simd` tree [I]; by its machine code alone mi_ldc.ko's copy is v4.3.76 … v4.6.95, likely ≤ v4.6.94
(`ldc-2-eptz-lib.md` §11.3 "Simd version"), and the bracket v4.4.84 … v4.6.93 above applies to it too [I].

**Functions present** [R: `__PRETTY_FUNCTION__` texts; template instances counted]: Base — AbsDifferenceSums3x3(+Masked),
AbsSecondDerivativeHistogram, AlphaBlending, AlphaFilling, HistogramConditional, OperationBinary8u/16i, ReduceGray2x1/2x2,
Reduce2DimSizeGray, ResizeBilinear, SquaredDifferenceSum(+Masked), StretchGray2x2; Neon — AbsDifferenceSum(+Masked),
AbsDifferenceSums3x3(+Masked), AbsSecondDerivativeHistogram, AlphaBlending, AlphaFilling, Deinterleave{Uv,Bgr,Bgra},
Interleave{Uv,Bgr,Bgra}, HistogramConditional, HistogramMasked, LbpEstimate, MedianFilter{Rhomb,Square}{3x3,5x5} (10 instances
each), OperationBinary8u (16) / 16i (6), ReduceGray2x1/2x2, ResizeBilinear (4) / ResizeBilinearGray, SquaredDifferenceSum
(+Masked), StretchGray2x2, VectorProduct; C++ layer — `Simd::View`, `Simd::Copy`, `Simd::ReduceGray`,
`AbsGradientSaturatedSum`, `ShiftDetector<Allocator>` (SetBackground, SetCurrent, InitLevels, Estimate). The entry points reached from no MVE export are in §7. Dispatch: §3.6.

### 6.2 KLT 1.3.4 (Stan Birchfield, public domain) [R strings against the upstream release `cecas.clemson.edu/~stb/klt/klt1.3.4.zip`]

All of KLT sits in one vendor file `./../C/src/Function_tools/MVE_LKOpticalFlow.c` (every KLT `assert` names it). Of the 173
string literals of the KLT C files, 85 are in the binary; the 88 missing are (a) the whole reader side —
`KLTReadFeature{List,History,Table}`, `_readFeatures`, `pgmRead*` / `ppmRead*`, `KLTExtract*` / `KLTStore*` (`storeFeatures.c`)
— so those functions were deleted; (b) the affine-consistency debug prints (`DEBUG_AFFINE_MAPPING` off, as upstream builds
it); (c) the "KLT Error: " / "KLT Warning: " prefixes and the progress prints of `KLTTrackFeatures` / `KLTReplaceLostFeatures`.
Present and identified by `__func__` / messages: `KLTCreateTrackingContext`, `KLTChangeTCPyramid`, `KLTUpdateTCBorder`,
`KLTPrintTrackingContext`, `_KLTCreateFloatImage`, `_KLTToFloatImage`, `_computeKernels`, `_convolveImageHoriz/Vert`,
`_KLTComputeGradients`, `_KLTComputeSmoothedImage`, `_KLTCreatePyramid`, `_KLTComputePyramid`, `_KLTSelectGoodFeatures` +
`_enforceMinimumDistance`, `KLTTrackFeatures` (+ `_interpolate`, `_am_getSubFloatImage`), `KLTWriteFeatureList(ToPPM)`,
`pgmWriteFile`, `ppmWriteFileRGB` (the writer side is kept).

Changed: **`KLTError` = `printf(message)` followed by a store to address 0 and an undefined instruction** — any KLT error
(e.g. out of memory) crashes the calling process instead of `exit(1)` [R 0x23ee0]. Warnings print without prefix.

Use by MVE (§5.12, §5.13): one tracking context per MVE handle (h+0x14 / +0x18), created on the first LK call;
`KLTChangeTCPyramid(tc, 15)` with the search range the MI wrapper hard-codes (15, upstream's default too); a feature list of
`u16CornerNum` (1…200) features of 0x34 bytes; tracking by `KLTTrackFeatures(tc, prev, cur, w, h, fl)`.

## 7. Unreached code in libmi_ive (unused) [R: call graph from the disassembly, rows with roots "-"]
<!-- ev:ive-video-analytics.7 -->

126 functions (≈90 KB) are reached from no export, no GOT / data pointer and no `.init_array` entry:
- **A vendor utility object linked twice** (copies at 0x0aa28–0x0b8d0 and 0x3e740–0x3f5e0, the same bodies): NEON
  float-vector multiply helpers (0xaa28, 0xaa38, 0xaa74 / 0x3e740…); a cache-line `memcpy` (0xb5dc / 0x3f2ec: plain `memcpy`
  below 256 B or for misaligned pointers, else 64-byte blocks with preload); the chip check (0xb6a8 / **0x3f3b8 — only this
  copy is used**, §3.2); a chip-id reader returning a 64-bit constant + the PMTOP id (0xb8d0 / 0x3f5e0); a **time code**
  (0xaaa8 / 0x3e7c0): `sprintf("%d", (int)(mon·10⁶ + (year−2000)·10⁸ + mday·10⁴ + hour·100 + min))` from
  `localtime(time())` into a static buffer, i.e. the decimal `YYMMDDhhmm` (it exceeds `INT_MAX` from 2022 on; the double →
  int64 → int conversion keeps the low 32 bits, so it wraps); and a **10-character licence-code cipher** in two variants
  per copy (0xab88 + 0xb3dc / 0x3e898 + 0x3f0ec): `cipher(out, in, dir)` with `dir` = +1 / −1 (other values → "The parameter:
  EN_OR_DE -- only support -1 or 1"); alphabet 0-9 A-Z a-z = values 0…61; encode maps value v → 3·v mod 62, decode inverts
  it (3 ⋅ 21 ≡ 1 mod 62); a character outside the alphabet → "The code is not permission..." (the second variant writes the
  result in a shifted output alphabet). The nine algorithm libraries carry the same chip-id reader pair, YYMMDDhhmm
  timestamp and 10-character base-62 codec with the same messages (`audio-algorithms.md` §2.2–§2.4). Nothing in the library or the
  SDK binaries calls them [R: no caller; I: a licence / expiry scheme that was disabled].
- **Simd entry points with no MVE caller** (≈40 functions, 35 KB): AbsDifferenceSum(s)Masked, AbsSecondDerivativeHistogram,
  AlphaFilling (Base, Neon), every Simd ResizeBilinear variant (the MVE Resize uses its own kernels, §5.5),
  SquaredDifferenceSumMasked, OperationBinary16i, Interleave / DeinterleaveBgra, ReduceGray2x1 (Base, Neon — the vendor
  addition is never called), VectorProduct, `Simd::Allocate` — plus their inner template instances.
- **KLT writers**: `pgmWriteFile` (0x26bbc, "P5\n%d %d\n255\n") and the `wb` file writer (0x2b9bc), and the debug-image dumps
  `kltimg_*.pgm` they serve (reached only when `tc->writeInternalImages` is set, which MVE never does [I]).
- `~MI_MVE_LBP_Neon` (0x35eac; the LBP dispatcher always takes `_PureC`) and `~MI_MVE_CSC_PureC` (0x100a8; CSC always takes
  `_Neon`).
- ≈84 small unnamed functions that are inner helpers of the above, crt (`_DT_INIT`, `entry`, `__do_global_dtors`,
  `frame_dummy`, 0x2cf0–0x2d74) and the Thumb libgcc soft-float routines (0xa66bc–0xa69c8).

## 8. The MVE copies inside libMD and libOD, and the weak back-end symbols

### 8.1 What the copies contain [R imports, strings, `__func__` names]

libMD_LINUX.so and libOD_LINUX.so carry their **own static copy of the MVE core** — Init / Uninit, buffers, the chip check
("The platform check is incorrect…", "Unexpected IC version"), `/dev/mstar_ive0`, `IVEWorkBuff`, the dispatchers (44 in MD,
46 in OD, recognisable by their `__func__` strings `MI_MVE_<op>` and `MI_MVE_<op>_Acc`) and the same 30 `*_Acc` HW builders
(an identical `_Acc` name set; libmi_ive adds only `MI_MVE_Acc`) — and drive the IVE engine themselves (`open` / `ioctl` /
`poll` / `mmap` imports; the "MVE" build tag, §1.2). They carry no Simd, no KLT and none of libmi_ive's split CPU back-ends
(`*_Neon` / `*_PureC`). libMD does carry a CPU `MI_MVE_CCL` (0x1453c, exported as `_md_sym_143`; no weak import, so it
works on HW-ineligible maps), used by `MI_MD_CCL`, `MI_MD_LightSwitchDetect` and `MI_MD_Preproc` (§9.9, §9.11, §9.12).

Their argument structures are libmi_ive's (§2.2, §4.1): the MVE control layouts are those of `mi_ive_datatype.h`, copied byte
for byte — ThreshCtrl {u32 mode; u8 low, high, min, mid, max}, AddCtrl {u32 mode; u16 x, y (U0Q16)}, Dilate mask[25],
OrdStatFilter mode {0 median, 1 max, 2 min} [S header]. Argument orders read from the dispatchers: Map(ctx, src, lut, dst),
Add / And / Or / Sub(ctx, src1, src2, dst[, ctrl]) [R].

### 8.2 The weak imports and what they resolve to [R]

The dispatchers' CPU back-ends are imported as **weak undefined** symbols: libMD 41 (`MI_MVE_*_Neon` / `*_PureC`), libOD 40.
Of these only the five `_PureC` that libmi_ive.so exports (§3.5) can be bound by the dynamic linker; the other **36 (MD) /
35 (OD)** — e.g. `MI_MVE_Filter_Neon`, `Sobel_Neon`, `Integ_PureC`, `Resize_Neon`, OD's `MI_MVE_Shift_Detector_Simd`, `CCL_PureC`, `EqualizeHist_PureC` (names that exist in libmi_ive only as
internal `__func__` strings, not as exports) — stay **0** at run time, whatever is loaded.

Every fallback is guarded: the GOT slot of the weak symbol is tested and, when it is 0, the dispatcher **returns
0x10000106** instead of calling it (checked for all 40 / 38 call sites; e.g. 0x1ab14 Sobel, GMM, CannyEdge, LKOpticalFlow).
So inside MD / OD every split MVE operation works only when its images are HW-eligible (§3.5) and otherwise returns
0x10000106 cleanly.

The operations the detectors actually use — MD: Add, And, Or, Sub, Thresh, Thresh_U16, SAD, Hist, MagAndAng (SAD in
ComputeImageSAD and LightSwitchDetect, Thresh_U16, Hist and MagAndAng in LightSwitchDetect; §9.8, §9.11) and its CPU CCL; OD: Add, And, Or, Sub, Thresh, Hist, Map, Dilate, OrdStatFilter — **all have a
hardware path** (CCL needs none). The operations without a HW path (GMM, CannyEdge, CannyHysEdge, NormGrad, LineFilter*,
NoiseRemove*, LKOpticalFlow, Resize, OptimalSeam; in OD also EqualizeHist and CCL) are never called by MD / OD code. MD and
OD therefore work on this firmware as long as the images they pass to the engine are 16-byte aligned with 16-multiple
strides and widths (MVE-allocated buffers are) [I].

### 8.3 libVG's MVE use [R]

libVG_LINUX.so has no MVE copy and imports 8 MVE exports by normal (non-weak) reference — `MI_MVE_Init`, `MI_MVE_Uninit`,
`MI_MVE_AllocateImage`, `MI_MVE_FreeImage`, `MI_MVE_AllocateBuffer`, `MI_MVE_FreeBuffer`, `MI_MVE_Integ`,
`MI_MVE_OrdStatFilter` — all of which libmi_ive exports; it needs libmi_ive.so loaded first (no NEEDED entry, §1.2).
Within libVG's use, `Integ` has a CPU fallback in libmi_ive (§4.1), `OrdStatFilter` does not (absent `_PureC` in padding
mode 2, §3.5).

### 8.4 The obfuscated exports `_md_sym_*` / `_td_sym_*` = the MVE dispatcher layer [R: their `__func__` strings]

| MVE function | libMD | libOD |
|---|---|---|
| Bernsen | `_md_sym_106` | `_td_sym_90` |
| Thresh_S16 | 107 | 91 |
| AdpThresh | 108 | 92 |
| NormGrad | 109 | 93 |
| FilterAndCSC | 110 | 94 |
| LineFilterVer | 111 | 95 |
| LBP | 112 | 96 |
| Filter | 113 | 97 |
| Hist | 114 | 98 |
| LineFilterHor | 115 | 99 |
| Or | 116 | 100 |
| Shift_Detector | — | 101 |
| AlphaBlending | 117 | 102 |
| BAT | 118 | 103 |
| Integ | 119 | 104 |
| NoiseRemoveHor | 120 | 105 |
| CannyHysEdge | 121 | 106 |
| Xor | 122 | 107 |
| Sub | 123 | 108 |
| 16BitTo8Bit | 124 | 109 |
| FreeBuffer | 126 | 111 |
| CannyEdge | 127 | 112 |
| NCC | 128 | 113 |
| EqualizeHist | — | 114 |
| ConfigPadding | 130 | 115 |
| Resize | 131 | 116 |
| Map | 133 | 118 |
| Uninit | 134 | 119 |
| AllocateImage | 135 | 120 |
| Sobel | 136 | 121 |
| Add | 137 | 122 |
| OptimalSeam | 138 | 123 |
| And | 139 | 124 |
| Erode | 140 | 125 |
| Matrix_Transform | 141 | 126 |
| SAD | 142 | 127 |
| CCL | 143 (CPU copy, §8.1) | 128 |
| MVE init | 144 | 129 |
| CSC | 145 | 130 |
| GMM | 146 | 131 |
| LKOpticalFlow | 147 | 132 |
| FreeImage | 148 | 133 |
| OrdStatFilter | 149 | 134 |
| Dilate | 150 | 135 |
| Thresh_U16 | 151 | 136 |
| Thresh | 152 | 137 |
| NoiseRemoveVer | 153 | 138 |
| Acc | 154 | 139 |
| MagAndAng | 155 | 140 |

The numbers not in the table (MD 125, 129, 132; OD 110, 117) are not identified.

## 9. libMD_LINUX.so — motion detection

### 9.1 Exports and API [R; header `mi_md.h` S]

The `MI_MD_*` exports at 0x7af8…0x7d94 are almost all one-instruction tail jumps into the core: GetLibVersion → 0x3fe4,
Preproc → 0x61b0, GetMBResult → 0x4cb8, SetTime → 0x4024, ComputeImageSAD → 0x4300, CCL → 0x4778, GetMotionCnt → 0x4e0c,
LightSwitchDetect → 0x4f38; Init 0x7afc, Uninit 0x7c58, Run 0x7cdc, SetParam 0x7cfc, GetParam 0x7d40 have small bodies. 50
more functions are exported under obfuscated names `_md_sym_106…155` — the MVE dispatcher layer (§8.4). Return codes
`MI_MD_RET` 0x10000401…405 (module byte 04, the same as libAEC's) [S header]. The internal name of Run is `IvaMD_Run`; a
variant `IvaMD_Run_Ex` takes an explicit {phys, virt} pair and marks it with 0xDEAD at +0xC2.

```c
typedef struct { uint16_t width, height; uint8_t color; uint32_t stride; MDMB_MODE_e mb_size /*4x4,8x8,16x16*/;
                 MDSAD_OUT_CTRL_e sad_out_ctrl /*16/8-bit*/; MDROI_t roi_md /*u8 num; {u16 x,y} pnt[8]*/;
                 MDALG_MODE_e md_alg_mode /*0 FG, 1 SAD, 2 FRAMEDIFF*/; } MI_MD_static_param_t;   /* 0x3C */
typedef struct { uint8_t sensitivity; uint16_t learn_rate; uint32_t md_thr; uint32_t obj_num_max; uint8_t LSD_open; } MI_MD_param_t;
uint32_t  MI_MD_GetLibVersion(void);                                       /* 20210127 */
MD_HANDLE MI_MD_Init(MI_MD_static_param_t *, MI_MD_param_t *);
void      MI_MD_Uninit(MD_HANDLE);
int32_t   MI_MD_Run(MD_HANDLE, const MI_MD_IMG_t *);                      /* {void *phys; uint8_t *virt} */
MI_MD_RET MI_MD_Preproc(MD_HANDLE, const MI_MD_IMG_t *img0, const MI_MD_IMG_t *img1, MDPreproc_ctrl_t *, MDSAD_DATA_t *, MDblock_info_t *);
MI_MD_RET MI_MD_SetParam / MI_MD_GetParam(MD_HANDLE, MI_MD_param_t *);
MI_MD_RET MI_MD_GetMBResult(MD_HANDLE, uint8_t *result_img, uint32_t *result_len);
MI_MD_RET MI_MD_ComputeImageSAD(MD_HANDLE, const MI_MD_IMG_t *, MDSAD_DATA_t *);
MI_MD_RET MI_MD_CCL(MD_HANDLE, MDCCL_ctrl_t *, MDOBJ_DATA_t *);
MI_MD_RET MI_MD_GetMotionCnt(MD_HANDLE, uint32_t *);
MI_MD_RET MI_MD_LightSwitchDetect(MD_HANDLE, const MI_MD_IMG_t *, MDOBJ_DATA_t *);
void      MI_MD_SetTime(MD_HANDLE, uint32_t time_diff);
```

### 9.2 Init, the creator, Uninit [R]

- `MI_MD_Init` prints `MI_MD_Init`; size = 0x3fb0(width, height, color, md_alg_mode == FG): colour ≠ 1 → 0; FG mode →
  **width·height·20 + 0x1C98** bytes, other modes 0x1C98. It `malloc`s that (failure → NULL), packs the static and dynamic
  parameters into a local block and calls the creator 0x36bc; failure → `free`, `[mi] MD initialize failed`, NULL.
- Creator 0x36bc: chip gate (`audio-algorithms.md` §2.2; gate 0x8a10, called from 0x36e4) fails → an `ALGO_MSG_WRN` message
  with 20210127 + 0x4D2, NULL. Else it prints the version, inits a pthread mutex at +0x1C0C, records the buffer base /
  length at +0x1C80 / +0x1C7C, header size 0x1C84 — a buffer smaller than that prints `MD_malloc exceed working buffer size
  now!!!` and executes an **undefined instruction** (the process dies with SIGILL) [R: `software_udf`]. Magic **0x0BEECAFE**
  at +0; width / height / colour / stride stored; mb_size, sad_out_ctrl, md_alg_mode at +0xDCC / +0xDD0 / +0xDD4; parameter
  defaults from `.rodata` pairs: +0x20 = 3, the u32 at +0x24 = 0x803100 (u16 +0x24 = 0x3100, u16 +0x26 = 0x80), +0x28 =
  0xF00 (§9.3). ROI: used only when `roi_md.num == 4`, and then only pnt[0] (start) and pnt[2] (end); end ≤ start on an axis
  → `Waring: … reset to full size!!!`; the macro-block grid size = ROI size >> (mb_size + 2). Work areas are carved from the
  same buffer with 4-byte alignment and the same overflow message. Then the dynamic parameters (0x3468, §9.4) and the MVE
  context (`_md_sym_144`).
- **Uninit**: NULL → `[mi] The handle is inconsistent.`; else core release (0x4100), `free`.
- `MI_MD_GetLibVersion` returns 20210127.

### 9.3 Handle fields [R offsets; names I]
<!-- ev:ive-video-analytics.9.3 -->

| off | meaning |
|---|---|
| +0x00 | magic 0x0BEECAFE |
| +0x04 | initialised (1) |
| +0x08 / +0x0c / +0x10 / +0x14 | W, H (u32 each), colour, stride |
| +0x18 | frame counter (Run) |
| +0x1c | 50 — frame cap of the learning rate and length of the time-average window |
| +0x20 | K = 3 modes per pixel |
| +0x24 (u16) | 0x3100 — read by neither FG path nor any other function of §9 [R] |
| +0x26 (u16) | T = 0x80 (background ratio 128/256) |
| +0x28 (u16) | 0x0F00 — read by neither FG path nor any other function of §9 [R] |
| +0x2c / +0x30 | 4·var0 = 230400, minV = 57600 (§9.6) |
| +0x34 / +0x38 | model buffer (K·n·6 bytes, n = ROI area) / fg mask (n bytes) — FG mode |
| +0x48 | SAD images state (−1 = their allocation failed, §9.11) |
| +0x4c | BG "ucBGImage" image descriptor (SAD / FRAMEDIFF, §9.7) |
| +0x5c (h[0x17]) / +0x18c (h[99]) | background image / its enable (0 in FG mode; 1 in SAD mode, image "ucBGImage") — +0x5c is `virt[0]` of the descriptor at +0x4c [I: +0x4c + 0x10] |
| +0x74 | SUB "ucSubImage" (§9.7) |
| +0x9c | CUR, the caller's frame |
| +0xc2 | 0xDEAD mark of `IvaMD_Run_Ex` |
| +0xc4 | LSD copy of the current frame (FG mode: its own W × H image, §9.11) |
| +0xec | UPD "ucUpBGbywf" (§9.7) |
| +0x190 (h[100]) | per-pixel count of used modes (n bytes; NEON path) |
| +0x194 | CCL blob: region count +0x197, region records from +0x198, 12 bytes each {u32 area at +0x198, then the u16 box from +0x19c} [R] (§9.9) |
| +0xd9c (h[0x367]) | MB result map (§9.10) |
| +0xdbc | row pitch p of the SAD result images (§9.8) |
| +0xdc0 | SadCtrl of ComputeImageSAD (§9.8) |
| +0xdc8 (u16) | SAD block threshold = b²·md_thr (§9.8) |
| +0xdcc / +0xdd0 / +0xdd4 | mb_size / sad_out_ctrl / md_alg_mode |
| +0xde8 | LSD images allocated (§9.11) |
| +0xdec / +0xdf0 | LSD warm-up done flag / call counter |
| +0xdf4 / +0xdf8 / +0xdfc / +0xe00 / +0xe04 | LSD floats m, m_prev, jump 3.5, 10.0, angle factor (§9.11) |
| +0xe08 | LSD_open |
| +0x1bc8 / +0x1be8 / +0x1bf8 / +0x1c04 | LSD MagAndAngCtrl / ThreshU16Ctrl / SadCtrl / AddCtrl (§9.11) |
| +0x1c0c | pthread mutex |
| +0x1c24 | ROI set (1) |
| +0x1c28 / +0x1c29 | MB grid width / height = ROI w / h >> (mb_size + 2) |
| +0x1c2a / +0x1c2c / +0x1c2e / +0x1c30 | ROI w, h, x0, y0 |
| +0x1c32 (u16) | averaged frame interval (from `SetTime`) |
| +0x1c38 (u8) | k = sensitivity step (§9.4) |
| +0x1c3a / +0x1c3c (u16) | SAD-mode weights: (learn_rate & 0xFF) << 8 and ((learn_rate & 0xFF)/20) << 8 |
| +0x1c3e (u16) | learn_rate (FG: clamped 1000…30000) |
| +0x1c40 / +0x1c44 | md_thr (SAD / FRAMEDIFF) / float md_thr·0.01 (FG) |
| +0x1c48 | obj_num_max |
| +0x1c50 → P1 | 12 bytes: {u32 n/50, u32 n, u16 α (Q8), u16 vt} |
| +0x1c54 → P2 | 8 bytes: {u8 motion flag, u32 fg pixel count} |
| +0x1c60 / +0x1c64 | watermark check state (§9.13) |
| +0x1c68 | vmax = 0xFFFF |
| +0x1c6c | last `SetTime` value |
| +0x1c74 (u16) | interval the current α was computed from |
| +0x1c76 / +0x1c77 (u8) | 1 / 0 — pixel step and row shift of the C path; +0x1c76 = 1 (set by the creator, no setter) also selects the NEON path |
| +0x1c78 … +0x1c80 | working buffer: length +0x1c7c, base +0x1c80; header size 0x1C84 |

### 9.4 SetParam, GetParam, SetTime, GetMotionCnt [R]

- `MI_MD_SetParam` → 0x3468 (also called by the creator): needs the ROI set (else 0x10000401 "Please set detect window of
  first!!!"); LSD_open → +0xe08; FG: +0x1c44 = (float)md_thr · 0.01, learn_rate clamped to 1000…30000 into +0x1c3e; other
  modes: +0x1c40 = md_thr, +0x1c3e = learn_rate (unclamped) and the two SAD weights +0x1c3a / +0x1c3c; obj_num_max →
  +0x1c48; sensitivity s → k = 50 for s < 10, 5 for s > 100, else ((100 − s) >> 1) + 5, stored at +0x1c38, and **vt =
  (u16)(k²·2.56)** (0x1900 for k = 50, 0x40 for k = 5) into P1+10; P1 = {n/50, n}; +0x1c74 = 0.
- `MI_MD_GetParam` → 0x3e08: learn_rate from +0x1c3e, md_thr = (int)(float·100) in FG or +0x1c40, obj_num_max, sensitivity =
  **110 − 2k** (so odd values and values outside 10…100 do not read back as written); LSD_open is not returned.
- `MI_MD_SetTime` (0x4024) stores its argument at +0x1c6c under the mutex.
- `MI_MD_GetMotionCnt` (0x4e0c) returns P2.count; in other modes than FG it warns "This function is use for only
  MD_ALG_MODE_FG." and still returns 0.

### 9.5 Run (0x7cdc → `IvaMD_Run`) [R unless tagged]

- The image must have phys = 0 or phys = virt (`*img == 0 || *img == img[1]`) [R]; the source reads this as a CPU-only
  path [I]. How the MVE steps of the SAD and FRAMEDIFF modes on such a frame can then be HW-eligible (§3.5 requires phys ≠
  virt) is not known.
- Takes the mutex; the magic must be 0x0BEECAFE (else "Error MD magic number(%p)", −1); not initialised or no ROI → −1.
- Per call the frame counter (+0x18) advances, and the `SetTime` deltas are summed in two process globals (`.bss` 0x344e8
  count, 0x344ec sum) and every 50 calls averaged into +0x1c32 — two MD handles in one process mix their deltas [I].
- By `md_alg_mode`:
  - **FG (0)**: NEON path (+0x1c76 = 1 and ROI width a multiple of 8): model initialisation on the first frame (VA
    0x18150, Ghidra 0x28150), update / detect afterwards (VA 0x17258, Ghidra 0x27258); otherwise the plain-C variant (VA
    0x23d8, Ghidra 0x123d8). CPU code, not MVE GMM. §9.6. Returns the motion flag.
  - **SAD (1)**: selective background update through MVE calls (§9.7); returns 0. The motion output comes from
    `MI_MD_ComputeImageSAD` (§9.8).
  - **FRAMEDIFF (2)**: the first frame is stored, every later call returns 0 **without processing** [R: no branch for mode
    2 after the first frame]; the reference is renewed by `MI_MD_ComputeImageSAD` (§9.8), so FRAMEDIFF = SAD against the
    previous frame [R: §9.8 last step; I: wording].
  - other values: `IvaMD_Run is not support!`, −1.

### 9.6 FG mode: fixed-point mixture of Gaussians (C path 0x23d8, NEON path 0x18150 + 0x17258) [R unless tagged]
<!-- ev:ive-video-analytics.9.6 -->

The model is the KaewTraKulPong–Bowden adaptive mixture (OpenCV legacy `BackgroundSubtractorMOG`, `process8uC1`; libmi_ive's
float copy of it is §5.7) re-done in integers (a fixed-point Stauffer–Grimson mixture) [I: the same per-mode scan, hit
test, replacement and background-ratio decision; the differences are listed below]. Per ROI pixel, K = 3 modes {w u8,
μ u16 = mean·256, v u16 = variance·256/4}; units: P = pixel·256, d = P − μ, d2 = d²/256 (= 256·Δ²), V = 4·v (= 256·σ²).

Per frame:
1. **Learning rate α (Q8)**: if the averaged interval t (+0x1c32) is 0: α = 255 / min(frame counter, 50); else if t differs
   from +0x1c74: α = (t·256) / learn_rate, clamped to 2…255 (so α = frame interval / time constant, learn_rate in the same
   unit as `SetTime`) [I: ms per the header]; else α unchanged. Stored in P1+8.
2. Per pixel, the modes are scanned in order while w ≠ 0:
   - every visited mode decays: w := ((256 − α)·w) >> 8;
   - **hit** if d2 < (V·vt) >> 8, i.e. |Δ| < λ·σ with **λ = k/10** (vt/256 = k²/100; λ = 0.5 … 5.0, sensitivity 100 → 0.5,
     ≤ 10 → 5.0): w += α; μ += (α·d) >> 8; V := max(V + ((α·(d2 − V)) >> 8), minV = 57600 → σ² ≥ 225); v := min(V >> 2,
     vmax 0xFFFF → σ² ≤ 1024); the mode bubbles up while its weight is **greater** than the one above (equal weights do not
     swap); the modes after it only decay; stop;
   - no hit: the first empty mode, or the last one, becomes {w = **α**, μ = P, v = 57600 (σ² = 900)} and bubbles up the same
     way;
   - weights are renormalised: w := (w·(65536 / Σw)) >> 8;
   - **foreground iff Σ_{j < hit} w_j ≥ T** (C path; T = 128 = half the total weight), i.e. the matched mode is not among the
     modes that make up the first half of the weight. fg mask byte 255 / 0.
3. count = number of fg pixels; P2 = {flag = (n/50 < count ≤ n), count}; Run returns the flag.

Differences from OpenCV's float version: weights are u8; a new mode starts with weight α instead of 0.05; the order key is
the weight alone (OpenCV: w/σ); the variance has an upper bound; the match threshold λ comes from the sensitivity instead of
a fixed 2.5 [R against §5.7].

**The two implementations.** Run takes the NEON path when +0x1c76 = 1 (always: set at creation, no setter) and the ROI width
is a multiple of 8, else the C path 0x23d8.
- NEON layout: planar, groups of 8 pixels: μ plane (K·n u16: per group K × 8 lanes), v plane (the same, after K·n·2 bytes),
  w plane (K·n bytes, after K·n·4), plus the per-pixel mode count (+0x190). C layout: interleaved, K × 6 bytes per pixel {u8
  w, pad, u16 μ, u16 v}. The two layouts are not interchangeable; the path is fixed by the ROI width.
- NEON first frame (frame counter < 2) = 0x18150: mode 0 = {w 255, μ = P, v = var0/4}, count 1, no mask or result written.
  The C path has no separate initialisation: on the first frame α = 255 and every pixel gets one mode of weight 255 — the
  same state, and an all-zero mask.
- Run side by side under unicorn (noisy static scene + a moving 6×6 object +
  a lasting scene change): identical masks and results for all 40 frames at sensitivity 50; at sensitivities 5 and 95 they
  agree until about frame 47 and then differ on a few pixels per frame [R: emulation]. Two causes [R: `fgprobe.py`,
  `fgsweep.py`]:
  - the NEON decision uses the complement of T (`mvn` of +0x26 → 127) and counts the hit as background only when
    Σ_{j<hit} w_j < 255 − T: for T = 128 the single cumulative weight 127 is background in C and foreground in NEON
    (two-mode sweep: weights after the update (127, 127), hit = mode 1 → C 0, NEON 255; every other split agrees);
  - μ and V drift apart by a few LSB after many updates (e.g. μ 17178 vs 17173 after 47 frames) [R]; from the direction of
    the drift the NEON path rounds the signed update terms down while C rounds them toward zero ((x + 255) >> 8 for negative
    x) [I]. The rounding of one update step itself was not probed; the cause is not settled.
- The optional background-image output of the NEON path (argument 5; the mean of mode 0 >> 8 written per pixel, 0x17a34) is
  fed from h[0x17] only when h[99] ≠ 0, which is 0 in FG mode — **unused** [R].

### 9.7 SAD mode (1): background maintenance in Run [R: MVE ctrl layouts §8.1]
<!-- ev:ive-video-analytics.9.7 -->

Images: CUR (+0x9c, the caller's frame), BG "ucBGImage" (+0x4c), SUB "ucSubImage" (+0x74), UPD "ucUpBGbywf" (+0xec), all
W (= stride) × H, U8C1. With a_fast = (learn_rate & 0xFF)·256 and a_slow = ((learn_rate & 0xFF) / 20)·256 (U0Q16, +0x1c3a /
+0x1c3c):
- first frame: BG = Thresh(CUR, TRUNC, low 255) = a copy of the frame; return 0;
- later frames: SUB = Sub(CUR, BG, ABS); SUB = Thresh(SUB, BINARY, low 15, min 0, max 255) — "moving" = |CUR − BG| > 15;
  UPD = Add(CUR, BG, ROUNDING, x = a_slow, y = 65536 − a_slow); UPD = And(SUB, UPD); SUB = Thresh(SUB, BINARY, low 0, min
  255, max 0) (= not moving); BG = Add(CUR, BG, ROUNDING, x = a_fast, y = 65536 − a_fast); BG = And(SUB, BG); BG = Or(UPD,
  BG). Net: **BG := moving ? BG + (a_slow/65536)·(CUR − BG) : BG + (a_fast/65536)·(CUR − BG)**; return 0. (learn_rate's low
  byte is the per-frame update weight in 1/256; the high byte is ignored in this mode.)
- The MVE return values in this branch are not checked.

### 9.8 `MI_MD_ComputeImageSAD` (0x4300) [R]

`(h, img, MDSAD_DATA_t *out)`; FG mode → 0x10000404 "is not support FG mode."; no MVE context → 0x10000403. With s = mb_size
+ 2 (block side b = 2^s = 4 / 8 / 16), blocks per row bw = stride >> s, rows bh = H >> s, row pitch of the result images
p = ceil(bw/16)·16 (+0xdbc):
- first call: images "ucSadResult" (type 9, 16-bit) and "ucThdResult" (type 0) p × bh; failure → 0x10000405;
- SadCtrl at +0xdc0: mode = mb_size, out ctrl = 0 (16BIT_BOTH) for sad_out_ctrl 0, 1 (8BIT_BOTH) for 1 (other values →
  0x10000404), **thr = b²·md_thr** (md_thr = mean absolute difference per pixel; stored at +0xdc8), min 0, max 255;
- SAD(CUR = img, BG) → {ucSadResult, ucThdResult};
- if `out` ≠ NULL: the ROI part of ucSadResult (rows y0>>s …, ROI_h>>s rows of (ROI_w>>s)·e bytes, e = 2 or 1) is copied
  **packed** to out->paddr; out->stride = bw (whole-image blocks per row, not the packed ROI width [I: a caller using it as
  the row pitch reads wrong rows when the ROI is narrower than the image]); out->enOutCtrl = sad_out_ctrl;
- FRAMEDIFF only: BG = Thresh(CUR, TRUNC, low 255) — the current frame becomes the next reference.

### 9.9 `MI_MD_CCL` (0x4778) [R]

`(h, MDCCL_ctrl_t *ctrl, MDOBJ_DATA_t *obj)`. The first call allocates "ucCCLResult" (U8C1, grid_w × grid_h, the MB grid of
§9.3). Then the MB map is built:
- FG: per MB, the fraction of 255 pixels of the fg mask inside its cell (cell = ROI_w/grid_w × ROI_h/grid_h pixels) is
  compared as float with md_thr·0.01: **MB = 255 iff fraction > md_thr/100** (equal or NaN → 0);
- SAD / FRAMEDIFF: the ROI part of ucThdResult (§9.8) is copied in (255 where the block SAD exceeds b²·md_thr).

Labelling: `_md_sym_143` = `MI_MVE_CCL`, the CPU CCL of §5.10 (8-connectivity on 255 pixels, ≤ 720 × 640, area-threshold
folding with ctrl {u16InitAreaThr, u16Step}), on the MB map in place, blob at +0x194. Output: obj->u8RegionNum = count; for
each region, into obj->astRegion (caller-owned, count × 12 bytes): u32Area = blob area · cell_w · cell_h (pixels), u16Left =
left·cell_w + x0, u16Right = (right + 1)·cell_w + x0 − 1, u16Top = top·cell_h + y0, u16Bottom = (bottom + 1)·cell_h + y0 − 1
(image coordinates). **`indexofmaxobj`, `areaofmaxobj`, `areaoftotalobj` are never written**, and `obj_num_max` (+0x1c48) is
read by no function (only set / get) — both unused. CCL failure → count 0, 0x10000404.

### 9.10 `MI_MD_GetMBResult` (0x4cb8) [R]

`(h, u8 *result, u32 *len)`: clears the MB map (+0xd9c, grid_w × grid_h), paints **255 over every CCL region's bounding box**
(MB units, from the blob of §9.9), copies the map to `result` and sets *len = grid_w · grid_h; always 0 with a valid magic. It
therefore reports the last CCL's boxes, not the raw per-MB motion, and overwrites the labelled map CCL left there. The map
pointer is set only by the first `MI_MD_CCL` call; GetMBResult before any CCL uses an unset pointer [I].

### 9.11 `MI_MD_LightSwitchDetect` (0x4f38) [R unless tagged]

`(h, img, MDOBJ_DATA_t *obj)` — a **filter over the objects of the preceding `MI_MD_CCL`** (it reads obj->u8RegionNum on
entry and rewrites obj), removing objects explained by an illumination change; with `LSD_open` ≠ 1 it only frees its images
(when allocated) and returns 0, leaving obj untouched. Grid: FRAMEDIFF recomputes grid_w / h = W / H >> (mb_size + 2) into
+0x1c28 / +0x1c29, the other modes use the ROI grid. Checks: no MVE context → 0x10000403; W must be a multiple of 8 and both
cell sides (cw = W/grid_w, ch = H/grid_h) larger than 7, else 0x10000404; SAD images failed (+0x48 = −1) → 0x10000401.

The first use (+0xde8 = 0) allocates "LSDbkg", "LSDSobelBkg", "LSDSobelCur", "LSD_angle_bkg", "LSD_angle_cur" (U8C1 W × H),
"LSDsobelu16" (type 9, W × H), "LSDSobelSad" (U8C1) and "LSD_angle_sad" (type 9) of W/8 × H/8, "LSD_Hist" (0x400 bytes),
"LSDgmmoutori" (U8C1 grid_w × grid_h) and in FG mode one more W × H image at +0xc4 (any failure → 0x10000405), and sets:
- SadCtrl +0x1bf8 {8×8 blocks, out 3 = 8BIT_SAD (later 2 = 16BIT_SAD), thr 960, min 0, max 255};
- MagAndAngCtrl +0x1bc8 {MAG_AND_ANG, thr 0, mask = 3×3 Sobel rows (−1 −2 −1 / 0 0 0 / 1 2 1) in the 5×5 grid};
- ThreshU16Ctrl +0x1be8 {MIN_ORI_MAX, low 0, high 255, min 0, max 255} (16-bit magnitude saturated to u8);
- AddCtrl +0x1c04 {CLIPPING, x = 0xFBF4, y = 0x040C} (≈ 0.984·bkg + 0.016·cur);
- floats +0xdf4 = 1.0 (m, the current mean edge SAD), +0xdf8 = 1.0 (m_prev), +0xdfc = 3.5 (jump), +0xe00 = 10.0, +0xe04 =
  **1.8 in FG mode, 1.5 otherwise**; +0xdec = 0 (warm-up done flag), +0xdf0 = 0 (call counter).

Per call (frame counter f = +0x18, advanced by Run):
1. CURc (+0xc4) = copy of the frame (Thresh TRUNC 255); f = 1 → LSDbkg = copy of CURc; f > 1 → LSDbkg = Add(LSDbkg, CURc)
   (the slow running average above).
2. MagAndAng(LSDbkg) → magnitude saturated into LSDSobelBkg, angle into LSD_angle_bkg; the same for CURc → LSDSobelCur,
   LSD_angle_cur. SAD 8×8 8-bit(LSDSobelCur, LSDSobelBkg) → LSDSobelSad; Hist → LSD_Hist; SAD 8×8 16-bit(LSD_angle_bkg,
   LSD_angle_cur) → LSD_angle_sad.
3. **m = Σ i·hist[i] / (W/8 · H/8)** (float) = the mean per-block edge-magnitude SAD.
4. Warm-up (+0xdec ≠ 1 and f < 3): m_prev = m; all objects dropped (FG / SAD: blob count 0; FRAMEDIFF: the CCL map cleared
   by Thresh low 255 → 0); obj->u8RegionNum = 0; at f = 2 +0xdec = 1; return 0.
5. No objects on entry → m_prev = m, return 0.
6. **Global change** — either m_prev ≠ 0 and |m_prev − m| > 3.5 (a jump of the mean edge difference), or, after the
   relabelling below, the blobs cover more than 0.9 of the grid: +0xdec = 0; FG mode: the whole mixture model is zeroed
   (memset K · grid_h·cw · grid_w·cw · 6 bytes — cw used for both axes); FRAMEDIFF: CCL map cleared; blob count and
   obj->u8RegionNum = 0; LSDbkg = copy of CURc; **f = 0** (the detector restarts learning); return 0.
7. Otherwise per object: LSDgmmoutori = Thresh(CCL map, BINARY low 0 → 255); `MI_MVE_CCL` on it with a fixed ctrl
   {u16InitAreaThr = 8, u16Step = 2} (blob at +0x194; total blob area > 0.9·grid → step 6). For blob i (box in MB units):
   x8 = (ch·left) >> 3 … (ch·right) >> 3, y8 = (cw·top) >> 3 (8-pixel block coordinates; ch for x and cw for y — equal for
   square MB cells); if the x range is empty the blob is erased; else over the **single block row y8** (the top row of the
   box only) and the blocks whose MB carries label i: n = count, E = Σ LSDSobelSad, A = Σ LSD_angle_sad. Decision:
   - n = 0 or E < 10·n → erase (no edge change: a brightness change);
   - E = 10·n exactly, or E > 60·n → keep;
   - 10·n < E ≤ 60·n → erase iff A ≤ (+0xe04)·64·n (edges changed in strength but kept their direction), else keep.
   Erasing = writing 0 over the blob's box in LSDgmmoutori.
8. CCL map = Thresh(LSDgmmoutori, BINARY low 0 → 255); `MI_MVE_CCL` again (same ctrl) → obj->u8RegionNum and
   obj->astRegion: FRAMEDIFF copies the blob records **raw (MB units, the MVE layout)**; FG / SAD convert as in §9.9 (x with
   ch, y with cw). Return 0.

On exit (every path through the unlock): **m_prev = m** and +0xdf0 += 1 — so the "m_prev = 1.0" written by the reset of step
6 is overwritten at once. Any failing MVE call → 0x10000404.

### 9.12 `MI_MD_Preproc` (0x61b0) [R unless tagged]

`(h, img0, img1, MDPreproc_ctrl_t *ctrl {u16 md_rgn_size, u16 align}, MDSAD_DATA_t *sad, MDblock_info_t *blk)` — FRAMEDIFF
only (other modes: "is not support FG and SAD mode.", 0x10000404). It finds the box of motion between two frames and fits it
to an alignment grid. With s = mb_size + 2, gw = W >> s, gh = H >> s (the whole image, not the ROI):
1. h[99] = 0; frame counter += 1; a 0xBF4-byte object buffer is `malloc`ed **and never freed** (a leak per call) [R: no
   `free` call in the function].
2. The BG image descriptor is pointed at img1 for one `ComputeImageSAD(h, img0, sad)` (§9.8: SAD of img0 against img1 →
   ucSadResult / ucThdResult and the caller's `sad`), then restored. That call's FRAMEDIFF step copies img0 into "BG" — at
   that moment img1's buffer — so **img1 is overwritten with img0** [R: descriptor swap around the call; I: effect on the
   caller].
3. Map M = ucThdResult, or, when md_rgn_size ≠ 0: `MI_MD_CCL(h, {8, 2}, local obj)` (ctrl from `.rodata` 0x21508), M = the CCL
   map binarised (Thresh low 0 → 255), and every object whose box satisfies ((right − left + 1) << s) ≤ md_rgn_size and
   ((bottom − top + 1) << s) ≤ md_rgn_size is erased from M over left…right × top…bottom. The boxes are the **pixel** boxes
   of §9.9 while the size test shifts them again and the erase indexes the MB map with them [R; I: a unit mix — small
   objects are erased at the wrong place, and boxes beyond the grid write past the map]. Then, if LSD_open = 1,
   `MI_MD_LightSwitchDetect(h, img0, local obj)` (§9.11).
4. Bounding box of the non-zero entries of M (gw × gh): first / last non-zero column and row, **widened by one block on each
   side** where possible; in pixels x0 = (c0 − 1)·b or 0, x1 = (c1 + 2)·b − 1 (≤ W − 1), the same for y. If nothing is
   non-zero the box is the whole frame and the call prints "if no motion, do nothing...", returns blk = {0, 0, x1, y1} (the
   whole frame) and 0; a box that happens to be the whole frame for other reasons takes the same path [I].
5. Alignment to a = ctrl->align (used as a power of two): for width w (and height likewise) up = (w + a − 1) & −a, down =
   a·⌊w/a⌋; pad = (up − w)/2 if down = 0; 0 if w is aligned; else (the nearer of up − w and down − w)/2 (negative = shrink).
   The box is then grown (or shrunk) by pad on both sides; if that crosses 0 it is shifted right (start 0, end = end + 2·pad −
   start, clamped to W − 1); if it crosses W − 1 it is shifted left (end W − 1, start = max(0, start − (2·pad − (W − 1 −
   end)))). blk = {st_x, st_y, end_x, end_y}; return 0.

### 9.13 Unused code and fields in libMD [R]

- **Frame watermark pair**: 0x7170 writes the 16-bit sum of the first N bytes of a buffer (N = its byte 0) into the low bits
  of the 16 bytes after them — no caller; 0x76c0 checks such a mark (−1 on mismatch) and is called only from Run when
  +0x1c60 = −1, a value no code writes (the creator sets +0x1c60 = +0x1c64 = 0) — both unused.
- Fields: `obj_num_max` and MDOBJ_DATA_t's three max / total fields (§9.9), the NEON background output (§9.6), +0x24 and
  +0x28 of the defaults (§9.3).
- The chip gate's second copy 0x8c38, the codec 0x7ef0 and the timestamp 0x7e18 have no caller; the used gate 0x8a10 is
  called from 0x36e4, 0x7178 and 0x9920 (`audio-algorithms.md` §2.4).

## 10. libOD_LINUX.so — occlusion (camera tamper) detection

The Run core is at 0x241c, detector creation at 0x3b88, the window set at 0x3798. The `MI_OD_*` exports (0x4694…0x4948) are thin; the core sits in unnamed functions
0x13798…0x145a0 and in the 51 exported but obfuscated `_td_sym_90…140` — the MVE dispatcher layer (§8.4; "td" = tamper
detection [I]). Return codes `MI_OD_RET` 0x10000501…506 (module byte 05, the same as libAPC's) [S header].

### 10.1 API (header `mi_od.h`) [S; R where the binary is described]

```c
typedef struct { uint16_t inImgW, inImgH; uint32_t inImgStride; ODColor_e nClrType /*1 = OD_Y*/; ODWindow_e div /*0 1x1,1 2x2,2 3x3*/;
                 ODROI_t roi_od /* u8 num; {u16 x,y} pnt[8] */; int32_t alpha, M, MotionSensitivity; } MI_OD_static_param_t;  /* 0x40 */
typedef struct { int32_t thd_tamper, tamper_blk_thd, min_duration; } MI_OD_param_t;
uint32_t  MI_OD_GetLibVersion(void);          /* → 20210525 */
OD_HANDLE MI_OD_Init(MI_OD_static_param_t *, MI_OD_param_t *);
int32_t   MI_OD_Run(OD_HANDLE, const MI_OD_IMG_t *yImage);
MI_OD_RET MI_OD_GetDetectWindowSize(OD_HANDLE, uint16_t *st_x, uint16_t *st_y, uint16_t *div_w, uint16_t *div_h);
MI_OD_WIN_STATE MI_OD_GetWindowResult(OD_HANDLE, uint8_t col, uint8_t row);
MI_OD_RET MI_OD_SetAttr(OD_HANDLE, MI_OD_param_t *); MI_OD_RET MI_OD_GetAttr(OD_HANDLE, MI_OD_param_t *);
void      MI_OD_Uninit(OD_HANDLE);
MI_OD_RET MI_OD_AlarmReset(OD_HANDLE, uint8_t enable, uint8_t reset_duration);
```
- `MI_OD_ExcludeSceneChange` and `MI_OD_sc_param_t` are declared in the header but **not exported** by this build [R].
- `GetLibVersion` (tail jump to 0x4560): sprintf("%04x%04x", 0x2021, 0x525) → strtol(…, 10) = 20210525 [R].

### 10.2 Init and detector creation [R]

- **Init** reads only the static parameters — the `MI_OD_param_t *` argument is not used [R: no use of r1]; the thresholds
  stay at the defaults until `SetAttr`. `div` 0 / 1 / 2 → 1×1 / 2×2 / 3×3 windows; any other value prints `Unsupported Windows
  larger than 3x3` and calls **`exit(1)`**. `nClrType` ≠ 1 → NULL without a message. The library `malloc`s W·H·3 + (n·n +
  0x3B)·16 bytes (0x453c; n = windows per side) and builds the detector in it (0x3b88); failure → `[mi] Init failed`, NULL.
  Then the detect window (0x3798) is set from `roi_od.pnt[0]` (start) and `roi_od.pnt[2]` (end) — `num` and the other six
  points are ignored; end ≤ start on an axis → `Waring: … reset to full size!!!` and the whole image on that axis; window
  cell size = (end − start + 1) / n.
- Detector creation (0x3b88): chip gate (`audio-algorithms.md` §2.2; gate 0x55c8, called from 0x3bbc and 0x64d8) fails → an
  `ALGO_MSG_WRN` message with 20210525 + 0x4D2, NULL; else it prints the version, `memset`s a 0x3B0-byte header, sets the
  defaults (`*(u32 *)(h + 0x398) = 0x050F0403` and `h[0x39c] = 1`: thd_tamper 3, tamper_blk_thd 4, min_duration 15, g10 5,
  p 1), creates an MVE context (`_td_sym_129`), stores W, H, stride, colour; alpha (clamped to ≤ 10), M (s16), windows per
  side; MotionSensitivity s ∈ 1…5 → internal value s' = (5 − s)·2, else 0; requires W, H, stride ≠ 0 and colour 1; the
  per-window records of 16 bytes follow the header in the same buffer; the two LUTs of §10.4 are written into +0x185…+0x384.
  `noop_init` (0x15a1c) (VA 0x5a1c), called during creation, returns 0 and does nothing.

### 10.3 Handle fields used by the core [R offsets from 0x3b88 / 0x3798 / 0x241c; names I]

| off | type | meaning |
|---|---|---|
| +0x00 | u8 | initialised (1 after a complete 0x3b88) |
| +0x02 / +0x04 / +0x08 | u16 / u16 / u32 | W, H, stride |
| +0x0c | u8 | colour (must be 1) |
| +0x10 | u32 | F — frames since (re)start of learning |
| +0x14 | u32 | L — frames in the current accumulation period |
| +0x18 | u8 | alpha (≤ 10) |
| +0x1a | u16 | M (frames per accumulation period) |
| +0x1c | ptr | MVE context (`_td_sym_129`) |
| +0x20 | MVE_IMAGE_S | CUR — the caller's frame (described on the first frame: type 0, width = stride = +0x08, height H; phys = the caller's phys, or virt when phys is 0 or equal to virt) |
| +0x48 | image | T1 "od-EdgeMa%p" (edge map; scratch) |
| +0x70 | image | EM "od-EgMaDi%p" (edge map, dilated) |
| +0x98 | MVE_MEM_INFO_S | HIST "od-HistMa%p", 0x400 bytes = 256 × u32 |
| +0xa4 | image | ACC "od-EgAccu%p" (per-pixel edge counter) |
| +0xcc | image | STG "od-StgEg%p" (blended edge counter) |
| +0xf4 | image | BGEM "od-BgEgMa%p" |
| +0x11c | image | BGB "od-BgBin%p" (background edge mask) |
| +0x144 | image | FNB "od-FgNBg%p" |
| +0x170 | u8 * | CPU mask, W·H bytes (from the working buffer) |
| +0x17c | u16 * | CPU accumulator ACC16, W·H·2 bytes (from the working buffer) |
| +0x180 | u8 | B — "background may be refreshed" (1 at creation) |
| +0x181 | s8 | C — alarm evidence counter |
| +0x182 | u8 | s' = (5 − MotionSensitivity)·2 for MotionSensitivity 1…5, else 0 |
| +0x183 / +0x184 | u8 / u8 | AlarmReset enable / reset_duration (`MI_OD_AlarmReset`) |
| +0x185…+0x284 | u8[256] | LUT_INC |
| +0x285…+0x384 | u8[256] | LUT_DBL |
| +0x388 | u32 | window set (1 after 0x3798) |
| +0x38c / +0x38d | u8 / u8 | windows per row / per column (n, n) |
| +0x38e / +0x390 | u16 / u16 | cell width / cell height |
| +0x392 / +0x394 | u16 / u16 | window origin x / y |
| +0x398 / +0x399 / +0x39a | u8 | thd_tamper (3), tamper_blk_thd (4), min_duration (15) — `MI_OD_SetAttr` |
| +0x39b | u8 | g10 = 5 (no API writes it) |
| +0x39c | u8 | p = 1 (no API writes it) |
| +0x3a0 | ptr | window records, n·n × 16 bytes: +0 u8 enabled (set to 1 for all by 0x3798), +4 s8 state, +8 u32 current count, +0xc u32 background count |
| +0x3a4 / +0x3a8 / +0x3ac | | working-buffer base / used / size |

### 10.4 The two LUTs and the constants [R: literal pool 0x40f8…0x4277 and the stores of 0x3b88]

- **LUT_INC** (+0x185): LUT_INC[i] = i + 1 for i = 0…254, LUT_INC[255] = 255 (the copy writes 0 at index 255, then
  `h[0x284] = 0xFF` overwrites it) — a saturating increment.
- **LUT_DBL** (+0x285): LUT_DBL[i] = (2·i) mod 256 — doubling with wrap-around (0, 2, …, 0xFE, 0, 2, …, 0xFE).
- The dilation mask (`.rodata` 0x173a8, 25 bytes) is a 3×3 square of 0xFF centred in the 5×5 grid; the state table at
  `.rodata` 0x173c4 is {2, 0, 1} (§10.5).

### 10.5 SetAttr, GetAttr, GetWindowResult, GetDetectWindowSize, AlarmReset, Uninit [R]

- **SetAttr**: needs the window set ("Please set detect window first!!!"); thd_tamper clamped to ≤ 10, tamper_blk_thd to
  ≤ n·n, min_duration stored as a byte (values > 255 wrap); any failure → 0x10000504. **GetAttr** returns the three bytes.
  Neither checks the handle for NULL.
- **GetWindowResult(h, col, row)**: NULL → 0x10000503 (returned as an enum value); col ≥ n or row ≥ n → −1; the window
  record's state s (+4) maps by the table 0x173C4 {2, 0, 1} on s + 1: s = −1 → NO_FEATURE (2), 0 → NON_TAMPER, 1 → TAMPER;
  anything else → FAIL (−1). A window never evaluated reads 0 = NON_TAMPER [R memset; I].
- **GetDetectWindowSize**: NULL → …503; window not set → …505; else start x, start y, cell width, cell height.
- **AlarmReset(h, enable, dur)** (0x4618): stores enable at +0x183 and, if enabled, dur at +0x184; prints `Alarm Reset is
  enable/disable.`; always 0; no NULL check.
- **Uninit**: NULL ignored; releases the MVE context (0x45a0), `free`.
- **Run** (tail jump to 0x241c): §10.6.

### 10.6 `MI_OD_Run` = 0x241c, step by step [R unless tagged]

Before anything else, every call stores the process-global s16 **G** (`.bss` 0x2a51c) = g10·256/10 = 128 (Q8 of 0.5) — a
global shared by all OD handles [R store at 0x250c; I: shared]. Then: not initialised or window not set → −1. All MVE calls
pass `bInstant = 1`. Any failing MVE call in the main chain returns −1 (not checked: the per-window `Hist` calls, every call
of step 5, the three `Map` calls, and the result of 0x2278). There is no reference frame.

1. **Frame counters**: F += 1; on F = 1 CUR's geometry is set (§10.3) and its phys / virt are taken from the argument on
   every call.
2. **Edge strength**: T1 = OrdStatFilter(CUR, MAX); EM = OrdStatFilter(CUR, MIN) (ctrl words 1 and 2); EM = Sub(T1, EM, ABS)
   — the 3×3 max − min, a morphological gradient. The first and last row and column of EM are zeroed by the CPU.
3. **Adaptive edge threshold**: Hist(EM) → HIST (cache invalidated); N = W·H; μ = ⌊Σ i·h[i] / N⌋, q = ⌊Σ i²·h[i] / N⌋ (64-bit
   integer sums, a NEON loop over bins 1…252, bins 253…255 added separately; the divisions by VA 0x17244 = the libgcc signed
   64-bit division, which raises SIGFPE (`raise(8)`) on a zero divisor before the divide routine 0x17280, and VA 0x171d8 = the
   int64 → double conversion [R code; I: libgcc `__aeabi_ldivmod` / `__aeabi_l2d` (0x271d8); 0x27244 / `__aeabi_l2d`]),
   σ = (u8)(int)sqrt((double)(q − μ²)). **t_e = μ + 4 if σ < 5, else μ + 2σ**, computed in u8 (wraps above 255).
4. **Edge map**: T1 = Thresh(EM, BINARY, low = t_e, min 0, max 255) — edge iff EM > t_e; EM = Dilate(T1, 3×3); the CPU zeroes
   a 2-pixel frame of EM (rows 0, 1, H−2, H−1; columns 0, 1, W−2, W−1).
5. **Edge counting**: T1 = Map(ACC, LUT_INC); T1 = And(T1, EM); FNB = Thresh(EM, BINARY, low 0, min 255, max 0) (= NOT
   edge); FNB = And(ACC, FNB); ACC = Or(T1, FNB). Net effect: **ACC[p] += 1 (saturating at 255) where p is an edge now,
   unchanged elsewhere**.
6. L += 1. Cell threshold **t_px = p·cell_h·cell_w / 100** (1 % of a window's area). Then either the learning branch (F ≤ M)
   or the running branch (F > M).

**Learning branch (F ≤ M)** — the first M frames after a (re)start:
- nf (`.bss` 0x2a520, global) = 0.
- M < 128 (MVE path): STG = Thresh(ACC, TRUNC, low 255, max 0) — with low = 255 a plain copy; BGEM = Map(ACC, LUT_DBL); BGB =
  Thresh(BGEM, BINARY, low = (u8)L, min 0, max 255): **background edge pixel iff 2·ACC > L**, i.e. an edge in more than half
  of the frames so far.
- M ≥ 128 (CPU path, interior pixels only): ACC16[p] = ACC[p] << 8; mask[p] = 255 if ACC16[p] > (u16)(G·L) else 0 (G·L =
  128·L: the same "more than half" rule in Q8); then **BGB[p] = 1** (not 255) where mask[p] ≠ 0, else 0, over the full
  W × H.
- Per window (§10.8 iteration): BGB's descriptor is pointed at the window (virt / phys + y·stride + x, width = cell_w, height =
  cell_h), Hist → h; record.bg = h[255]; if h[255] < t_px the window's state := −1 (no feature) and nf += 1. BGB's width /
  height are restored.
- If nf ≥ (n·n·2)/3: **restart** — F = 0, ACC cleared (memset stride·H), L = 0. Otherwise, when L = M (the last learning
  frame, F = M): L = 0, ACC cleared, 0x2278 (§10.7); the running branch starts with the next frame.

**Running branch (F > M)**:
- Only when L = M (once per period of M frames): nf = 0; then
  - if B = 0 (alarm evidence present): the background is **not** refreshed;
  - else if M < 128: STG = Add(ACC, STG, CLIPPING, x = 0x3333, y = 0xCCCD) — **STG = 0.2·ACC + 0.8·STG**; BGEM = Map(STG,
    LUT_DBL); BGB = Thresh(BGEM, BINARY, low = (u8)M, min 0, max 255) — background edge iff the blended per-period count
    exceeds M/2;
  - else (M ≥ 128, CPU, interior pixels): ACC16[p] = ((((10 − alpha)·256)/10)·ACC16[p] >> 8) + (ACC[p] ≠ 0 ?
    ACC[p]·((alpha·256)/10) : 0) (u16 arithmetic) — an exponential blend with weight alpha/10; mask[p] = 255 iff ACC16[p] >
    (u16)(G·M); BGB[p] = 1 where mask[p] ≠ 0, else 0;
  - in all three cases: per-window Hist of BGB → record.bg = h[255], state −1 and nf += 1 when h[255] < t_px (as in the
    learning branch); ACC cleared; L = 0; 0x2278.
- Otherwise nothing is learnt this frame.

**Common tail** (every frame): FNB = And(BGB, EM) — the background edges still visible now. Per window: Hist(FNB window) → h,
cur = h[255]; with bg = record.bg:
- bg < t_px → state −1 (no feature);
- else cur < bg·thd_tamper / 10 → state 1 (tamper), T += 1;
- else state 0;
- record.cur = cur.

Then the decision of §10.8. Return value: 1 = tamper alarm, 0 = no alarm, −1 = error.

Notes [R facts; I consequences]:
- `alpha` reaches the code only on the M ≥ 128 path; with M < 128 the blend weight is the fixed 0.2.
- On the M ≥ 128 path BGB holds 1, not 255, so every later `Hist` sees h[255] = 0: every window becomes "no feature"
  (record.bg = 0 < t_px) and, by the rule of §10.8, the alarm counter rises every frame [I: the detector reports tamper
  permanently after min_duration frames when M ≥ 128; a vendor bug].
- A per-window `Hist` is HW-eligible only when the window's x offset, address and cell width are multiples of 16 (§3.5,
  §8.2); otherwise the dispatcher returns 0x10000106, the return is ignored, and HIST still holds the previous histogram [I].
- G and nf are process globals: two OD handles in one process share them [I].

### 10.7 0x2278 — growing the background edge mask [R]

With s' = h[0x182] = (5 − MotionSensitivity)·2 ∈ {0, 2, 4, 6, 8}: repeat s'/2 times {T1 = Dilate(BGB, 3×3); BGB = Dilate(T1,
3×3)} — BGB dilated s' times by the 3×3 square; then the CPU zeroes an s'-pixel frame of BGB (top and bottom s' rows, left
and right s' columns) and flushes it. s' = 0 (MotionSensitivity 5 or out of range) does nothing. It runs at the end of every
accumulation period (learning and running branch alike), also when the background was not refreshed (B = 0): in that case
the old mask is dilated again each period [R; I: the mask then keeps growing until it is refreshed]. A larger dilation makes
more of the current edges fall inside the background mask, so FNB keeps more pixels and a window needs a larger loss of edges
to count as tampered [I] — a lower MotionSensitivity = less sensitive.

### 10.8 Window loop and alarm decision [R]

- Window (c, r), c, r = 0…n−1, record index r·n + c: x = origin_x + c·cell_w, y = origin_y + r·cell_h (§10.3; origin and cell
  size from `roi_od.pnt[0]` / `pnt[2]`, §10.2). Only records with `enabled` ≠ 0 are processed (all are).
- After the tail loop, with T = tampered windows of this frame, nf = no-feature windows counted at the last background
  computation, D = min_duration, R = reset_duration, C = the counter at +0x181:
  - C −= 1 if T < tamper_blk_thd **and** T + nf ≠ n·n; else C += 1 (enough tampered windows, or every window either tampered
    or featureless — e.g. a lens covered from the start);
  - C is clamped to 0…2·D;
  - if AlarmReset is enabled and (C = R + D or C = 2·D): ACC cleared, F = 1, L = 0 (the 8-byte constant 0x3518 = {1, 0}),
    B = 1, **return 1** — the alarm is reported and learning restarts from scratch, so a lasting change becomes the new
    background;
  - else if C ≥ D: B = 0, **return 1** (alarm; background refresh frozen);
  - else B = (C = 0), **return 0** (refresh allowed again only once the counter has fully decayed).
- What a caller sees, in words [I]: a tamper alarm is raised after min_duration more "tampered" frames than "normal" frames
  (net), where a frame is tampered when at least tamper_blk_thd windows have kept less than thd_tamper/10 of their learnt
  background edges. The libmi_vdf caller enables AlarmReset with duration 5 (`sdk-orphan-libs.md` §7), so there the alarm
  re-learns the scene at C = D + 5.

### 10.9 Parameters: where each one acts [R]

| parameter | used by | effect |
|---|---|---|
| `alpha` (≤ 10) | M ≥ 128 CPU blend only | weight of the new period in ACC16 |
| `M` (u16) | accumulation period, learning length, BGB threshold M/2; selects the MVE (< 128) or CPU (≥ 128) path | — |
| `MotionSensitivity` 1…5 | 0x2278 | s' = (5 − s)·2 dilations of BGB |
| `thd_tamper` (≤ 10, default 3) | window state | tampered when < thd_tamper/10 of the background edges remain |
| `tamper_blk_thd` (≤ n·n, default 4) | alarm counter | tampered windows needed per frame |
| `min_duration` (byte, default 15) | alarm counter | alarm at C ≥ D; C capped at 2·D |
| AlarmReset enable / duration | alarm counter | relearn at C = D + duration or C = 2·D |
| +0x39b g10 = 5, +0x39c p = 1 | not settable | G = 0.5 (CPU path), t_px = 1 % of a window |
| `roi_od.num`, `pnt[1]`, `pnt[3…7]` | — | **unused** (§10.2) |
| the `MI_OD_param_t *` of `MI_OD_Init` | — | **unused** (§10.2) |

The chip gate's second copy 0x57f0, the codec 0x4aa8 and the timestamp 0x49d0 have no caller (`audio-algorithms.md` §2.4).

## 11. libVG_LINUX.so — virtual gate / region intrusion

### 11.1 Layers [R]

`MI_VG_*` (public wrapper, 0x6344…0x75ec), `IvaVG_*` (core, 0x1b38…0x406c), `vg_cross_*` (line-crossing engine,
0x7a04…0x9720), a private "VG-MD" background / motion model (0x4190…0x625c, strings `VG_MdTrain%d`, `VG_MdMean%d`,
`VG_MdBg%d`, "Simple Method") and the chip gate pair 0xa320 (used, called from 0x25b8) / 0xa548 (`audio-algorithms.md`
§2.2, §2.4). libmi_ive is used for `MI_MVE_Init`, `AllocateImage` / `FreeImage`, `OrdStatFilter`, `Integ`,
`AllocateBuffer` / `FreeBuffer`, `Uninit` ("With HW" in the init messages; §8.3).

### 11.2 API (header `mi_vg.h`, VG 2.3) [S header; R behaviour]

Configuration lives in a caller-owned `MI_VgSet_t` (0xB8 bytes): +0x00 float `object_size_thd`; +0x04 u16 `line_number`;
+0x06 u8 `indoor`; +0x08 `fp[4]`, +0x28 `sp[4]`, +0x48 `fdp[4]`, +0x68 `sdp[4]` (`MI_VG_Point_t` {s32 x, y}); +0x88 u8
`function_state`; +0x8C…+0xAB region points p1…p4; +0xAC u8 `region_direction`; +0xB0 s32 `magic_number`; +0xB4 s32
`region_spdir_state`. The handle is allocated by the library. Return codes `MI_VG_RET` 0x10000301…0x10000319 (the same
module byte 03 as libBF's `ALGO_BF_RET`).

| function | checks → code | effect |
|---|---|---|
| `SetFunction(set, fs)` | set NULL → 303; fs ∉ {2 VIRTUAL_GATE, 3 REGION_INVASION} → 304 | set+0x88 = fs |
| `SetObjSizeThd(set, f)` | NULL → 303; f ∉ [0, 4] → 305 | set+0x00 = f |
| `GetObjSizeThd` | NULL set 303, NULL out 306; stored ∉ [0,4] → 305 (value still returned) | |
| `SetScene(set, s)` | NULL → 303; s > 1 → 307 (message says "lineno") | set+0x06 = s (indoor) |
| `GetScene` | NULL set 303, NULL out 308; stored > 1 → 307 | |
| `SetLineNumber(set, n)` | NULL → 303; n ∉ 1…4 → 309 | set+0x04 = n |
| `GetLineNumber` | NULL set 303, NULL out 30A; stored ∉ 1…4 → 309 | |
| `SetLineAndDir(set, line, n)` | NULL set 303, NULL line 30C; n ∉ 1…4 → 309; function_state ≠ 2 or magic = 0x5A5A5A5A → 30D | line n (1-based) → fp/sp/fdp/sdp[n−1]; magic := 0x55555555 |
| `GetLineAndDir(set, line, n)` | NULL set 303, NULL line 30B (not 30C); n ∉ 1…4 → 309 | copies back |
| `SetRegionAndDir(set, reg)` | NULL set 303, NULL reg 30E; function_state ≠ 3 or magic ∈ {0x5A5A5A5A, 0x55555555} → 30F; region_dir > 2 → 310 | magic := 0x5A5A5A5A **before** the direction check; p1…p4, region_direction stored; region_spdir_state = (dir = 2 ? (spec_dir_state == 1) : 0); then RegionInfoCheck (§11.4) ≠ 0 → 310 |
| `Init(set, w, h)` | set NULL → NULL | §11.3 |
| `Uninit(h)` | NULL → 311; magic ≠ 0x01010101 → 312 | frees +0x0C, core release, frees the two sub-blocks and the handle |
| `Reset(h)` | 311 / 312 | `IvaVG_Reset` |
| `SetDetectThd(h, p)` | 311 / 312; NULL → 317; switch > 1 → 318; thd ∉ 1…50 → 319 — both errors also **zero the caller's struct** | VG-MD threshold set (§11.7), prints "Set detection threshold" |
| `Run(h, img)` | 311 / 312; img NULL → 313; core error → 314 | §11.6 |
| `GetResult(h, res)` | 311 / 312; NULL → 315 | §11.6 |
| `GetDebugInfo(h, dbg)` | 311 / 312; NULL → 316; core error → 314 | fills `MI_VgDebug_t` |

Consequence [R]: once a region has been set, `SetRegionAndDir` on the same `MI_VgSet_t` always fails (magic is already
0x5A5A5A5A); a region and lines cannot both be configured in one struct; the caller has to clear `magic_number` itself.

### 11.3 Init and SetParams [R]

`calloc` a 0x534-byte handle (magic 0x01010101 at +0), a 0xADC-byte VG-MD block (+0x52C…, pointer at index 0x14B) and a
0x4D0-byte block (index 0x14C). `IvaVG_SetParams(h, w, h, set)` (0x1d78) validates and returns 1…9 (messages "Function state",
"Input source information", "Object threshold", "Virtual gate number", "Environment state", "Region direction", "Region
coordinate", "Virtual gate coordinate", "Direction point" error); any non-zero frees everything and returns NULL. Parameter
mapping in SetParams:
- function_state must be 2 or 3 (1); w, h > 0 (2); stored w (+0x28 s16), h (+0x2A), w·h (+0x34).
- object_size_thd: (int)f > 4 → 3; 0 < f < 1 → the caller's value is **rewritten to 4.0** and the size factor +0x30 = 0.5;
  otherwise by (int)f: 1 → 10.0, 2 → 5.0, 3 → 1.0, 4 → 0.5, 0 → 30.0 (percent of a line window, §11.5).
- line_number 1…4 (4); indoor ≤ 1 (5); indoor 1 → +0x40 = 50, +0x48 = 20; outdoor → 25, 5 — +0x40 is the L1 matching and
  de-duplication distance D of the crossing engine, +0x48 the object box margin m (§11.8, §11.9).
- region mode (3): region_direction ≤ 2 (6); p1.x < p2.x, p1.y < p4.y, p2.y < p3.y, p4.x < p3.x (7) — points clockwise from
  top-left.
- per line: coordinate and direction-point checks (8, 9; 0x197c) and the line geometry precomputed into 0x9C-byte line
  records (helpers 0x197c, 0x1020, 0x121c, 0x1314; §11.5).

Then `IvaVG_Init`: chip gate (fail → code 10, `Warning!! IC version check wrong, disable VG Lib ******0x20202241`); pass prints
`IC version check pass, VG Lib ver 20201007`; `MI_MVE_Init` — failure calls **`exit(-1)`** (kills the process); per line
three MVE images `VG_Label%d` (type 9), `VG_Temp%d` (type 9), `VG_Hist%d` (type 0xD) sized from the line record; failure →
11; VG-MD init (needs non-zero w, h; images `VG_MdTrain%d` / `VG_MdMean%d` / `VG_MdBg%d` per
region) failure → 12. Init codes 10 / 11 / 12 are printed as "IC check fail" / "VG init error"; codes 1…9 from Init are
silent. `IvaVG_Init` also sets up the crossing engine (`vg_cross_setparams`, §11.9).

### 11.4 Region → lines (`IvaVG_RegionInfoCheck`) [R values; I meaning]

Re-checks the clockwise order (fail → 0x10000310), forces `line_number` = 4 and writes the four edges as lines 1–4 (p1→p2,
p2→p3, p3→p4, p4→p1) into fp / sp. Direction points per edge: for region_direction 0 (ENTER) and 1 (LEAVING) the edge's
direction point pair is the edge offset by 10 pixels to one side or the other (coordinates clamped at ≥ 10 before
subtracting 10), giving "into the region" for ENTER and "out of it" for LEAVING; for 2 (CROSS) the direction points equal
the edge end points (no preferred direction).

### 11.5 Per-line window and minimum object size [R unless tagged]

- 0x1020: bounding box of a line's two end points (min / max x, y); 0x197c checks the end points (code 8, "first/second x|y
  point") and, in gate mode, the two direction points (code 9) against the image. 0x1314 builds the per-line window record
  (0x9c bytes from +0x298): the run window is the line box shrunk by 2 pixels on each side (x +0x2c0…+0x2c4, y
  +0x2d0…+0x2d4, width = box width + 1 at +0x2b8, frame offsets +0x2a8 (y) and +0x2b0 (x)), and the **minimum object area**
  +0x298 = (int)(window pixels · size_factor / 100), +0x29c = that / 10, where size_factor (+0x30) comes from
  `object_size_thd` (§11.3: 0 → 30, 1 → 10, 2 → 5, 3 → 1, 4 or (0, 1) → 0.5 percent of the window). Where the box record
  +0x22c… (the input of 0x1314) is filled is not known [I: the box of 0x1020].
- 0x121c: in region mode, the "inside" sign of edge i (+0x51c + 4i): edges 0 and 2 from ±10·Δx, edges 1 and 3 from ±10·Δy of
  the edge, sign → ±1 (0 counts as +1 or −1 by the other product).
- **Unused** (no caller): 0x12dc (box grown by 30 pixels), 0x17e4 (box overlap test), 0x18d4 (union of two boxes).

### 11.6 Run / GetResult [R]

**Run** (core state +0x08 must be 0, else 12): the image pointer (header name `_ucMask`) goes to VG-MD (0x5d34, w×h bytes;
§11.7) [I: a luma frame, since VG-MD builds its own background]; its return 2 = a new foreground mask is ready, < 0 = error
11. On 2, per line: median (`MI_MVE_OrdStatFilter`, cache flush around it) of the line's mask window, labelling (0x2a10),
measurement and collection (0x2f50, 0x3348) and the crossing engine (§11.8, §11.9); then the line's Label / Temp / Hist
images are cleared.

**GetResult**: `vg_cross_result` gives alarm[0…line_number−1] (copied, 4 bytes each, §11.9); `alarm_cnt` = the object count
of the crossing engine (+0x760), bounding boxes copied {up, down, left, right} from the 0x24-byte object records; the engine's
object list is then cleared. The count is not capped at 20 by this function, and the engine appends without an upper bound
(§11.9 step 5) [R; I: overflow risk if more than 20 objects are listed].

### 11.7 VG-MD background model: training 0x4750, run 0x5d34, decision and update 0x4d0c, reset 0x4598 [R unless tagged]

Per region r (count at +0x10; one region per configured line) the VG-MD block keeps a 0x68-byte record at +0x29c + r·0x68
(window x0 +0x29c, x1 +0x2a4, y0 +0x2ac, y1 +0x2b0 for training and +0x2bc / +0x2c0 / +0x2b4 / +0x2b8 for the run, width
+0x2d0, pixel count N +0x2c4, decided count +0x2c8, state +0x2cc, training frame t +0x2f8, ratio threshold +0x2fc, foreground
count +0x300), a 32-byte per-pixel record array (+0x48c[r]) and the images of §11.3 (background "VG_MdBg" virt at +0xcc +
r·0x28, texture map +0x2c + r·0x28, mask +0x20c + r·0x28). The frame row pitch is the u16 at +0x49c.

**Training (0x4750), one call per frame and region** — background by majority vote:
- per-pixel record: {u8 decided, u8 nbins, u8 cnt[15] (+2…+0x10), u8 val[15] (+0x11…+0x1f)};
- t = 0: val[0] = pixel, cnt[0] = 1, nbins = 1;
- t > 0, undecided pixels: among the bins with |pixel − val| ≤ 10 the one with the largest count is taken: cnt += 1, val =
  (val·(cnt − 1) + pixel) / cnt (integer running mean); no such bin → a new bin {pixel, 1}. nbins has no upper check: a 16th
  distinct value writes past the 15 slots into the next pixel's record [R; I: possible only for a pixel still undecided with
  15 bins];
- from t = 14 on, after the update: ratio = max(cnt) / Σcnt (float → Q16); **ratio ≥ thr → decided**, the background pixel =
  val of the largest bin, decided count += 1;
- thr starts at 0x9999 (0.6) and from t = 15 on drops by 0xA3D (0.04) per frame, so every pixel is decided by about t = 29
  [I: arithmetic];
- t += 1, return 1; once the decided count = N: the records are cleared and 0 is returned (region trained).

**Run (0x5d34)**: until every region is trained (+0x46c = 0): train each region, count the newly finished ones (+0x470[r],
+0x480); when all are trained print the "trained" message, +0x484 = 1, +0x46c = 1; return 1 (no foreground yet). Then per
frame and region: `MI_MVE_Integ`, the texture map 0x5208, the decision 0x4d0c; afterwards, if the foreground pixels of all
regions together exceed 2/3 of their pixels (Σ fg > (Σ N / 3)·2; one region: fg > (N/3)·2) → **reset (0x4598)** and return
2, else the fg counters are cleared; return 2 (mask ready).

**Texture map** (0x5208): from the MVE integral image (`MI_MVE_Integ`, COMBINE output: Σx in the low 28 bits and Σx² in the
upper 36 bits of each 64-bit entry, §4.4) the local 3×3 variance var = Σx²/9 − (Σx/9)² is computed per pixel of each region
(s16).

**Decision and update (0x4d0c)**, per pixel, d = |cur − bg| >> 2 (bg = the region's background image):
- `SetDetectThd` function_switch = 1: foreground iff d > detect_thd (1…50).
- switch = 0 (default): if d² > var(bg pixel) a 3×3 neighbourhood pattern is built for the current frame and for the
  background — bit j set when |centre − neighbour_j| ≤ 0.2·centre (0x3333/65536) — and the pixel is foreground iff the two
  patterns differ. The patterns are built with XOR into a u8 masked by 0xFF after each bit, so bit 8 (the bottom-right
  neighbour) is always lost; the centre bit is always set. The current-frame pattern steps rows by the frame pitch (+0x49c),
  the background pattern by the region width (+0x2d0).
- foreground → mask 1, the pixel's run counter (record byte +1) = 0, and (switch 0 only) fg count += 1;
- background → mask 0; if the run counter > 29: **bg = (7·bg + cur) >> 3**; run counter += 1 (u8, wraps after 255 → the
  update pauses for 30 frames every 256 [I]).

**Reset (0x4598)**: prints "Reset VG Background"; +0x46c = +0x480 = +0x484 = 0; per region: trained flag, decided count, t = 0,
thr = 0x9999, fg count = 0, and the per-pixel records, texture map, mask, background and the u64 map at +0x16c cleared — the
regions retrain from scratch (another 15…30 frames without output).

### 11.8 Object extraction per line: labelling 0x2a10, measurement 0x2f50, collection 0x3348 [R]

`IvaVG_Run`, after VG-MD returns 2, per line r: median (`MI_MVE_OrdStatFilter` MEDIAN) of the region's mask into the line
image at +0x9c + r·0x78 (failure → state 10, sticky, return 10), then:
- **0x2a10 — labelling**: two-pass connected components over the run window with **4-connectivity** (left and upper
  neighbours) and a u16 union-find table (the line's +0x84 image, entries initialised to i, union to the smaller root); the
  input mask (value 1 = foreground, the +0xac image) is then reused as a u32 area table. Labels with area > the minimum object
  area (§11.5) are kept (table entry = area); fewer than 11 kept → returns the label count + 1; 11 or more → a quickselect on
  the areas moves the **10 largest** to entries 0…9, clears the rest and returns 10.
- **0x3348 — collection**: for every kept entry 1 … (return − 1) (entry 0 is never read [R]; with more than ten objects one of
  the ten largest is therefore dropped [I]) the window is scanned for the first pixel of that label and **0x2f50** measures
  it: over the rows from there to the window end, n = pixel count, centroid (Σx/n, Σy/n), box = [min − m, max + m] clamped to
  the window (m = handle +0x48: **20 indoor, 5 outdoor**), record {cx, cy, top, bottom, left, right, n, 0, 0} (9 ints; at most
  20 per line, a stack array). Window offsets are added (x + 0x2b0, y + 0x2a8) and the list goes to
  `vg_cross_detection(engine, line, count, list)`. Afterwards Run clears the line's label, union-find and mask images.

Frame-to-frame association (tracking) is not in 0x2a10 but in `vg_cross_detection` (0x81c0, §11.9).

### 11.9 Crossing engine `vg_cross_*` [R unless tagged]
<!-- ev:ive-video-analytics.11.9 -->

**`vg_cross_setparams(engine, w, h, n, function_state, D, m, points…, region_flag)`** (from `IvaVG_Init`): n > 4 → 0xD; any
point outside [0, w) × [0, h) → 0xE. engine[0] = function_state (2 gate, 3 region), [1] = n, **[2] = D = handle +0x40 = 50
indoor / 25 outdoor** (the matching and de-duplication distance), [3] = m (+0x48), +0xac8 = region_flag. Per line i a
0x24-byte record at engine + 0x10 + 0x24·i: end points p1 (fp), p2 (sp); s1 = side(fdp), s2 = side(sdp) with side(q) = (p1.y −
p2.y)(q.x − p1.x) − (p1.x − p2.x)(q.y − p1.y); **mode 4** (no direction) when fdp = sdp or both direction points lie on the same
side (or both on the line), else mode 0 (directional); a direction point exactly on the line gets ∓100 (the opposite sign of
the other one). With region_flag = 1, 0x7944 sets +0xacc[i] per edge by the same rule as 0x121c (§11.5) on the engine's copy
of the edge: edges 0 / 2 from ±10·Δx, edges 1 / 3 from ±10·Δy, sign → ±1.

**`vg_cross_detection(engine, line, k, objs)`** (0x8c70) per frame and line:
1. 0x7d1c ages the line's crossing history (below): entries older than 9 frames are cleared [R: age at +0x63c > 9 → the entry is zeroed].
2. **Merging**: objects whose boxes overlap (non-negative intersection in x and y) get one group id (pairwise union by the
   smaller id over a 0…19 table); members of a group are folded into the first: centroid = n-weighted mean, box = union
   ("Input feature size error", return 0xF, when a pixel count is 0).
3. **Association** (0x81c0): each remaining object's centroid is matched to the nearest centroid of the previous frame on this
   line (engine + 0xa0 + 0x158·line: count, x[20], y[20]) by **L1 distance < D**; matches record {prev x, prev y, distance,
   prev index}; when several objects match the same previous point only the closest keeps it.
4. **Crossing test** (0x877c) per kept pair, prev → cur, with d_prev = side(prev), d_cur = side(cur):
   - mode 4, gate: the sides change (prev ≥ 0 ≥ cur or prev ≤ 0 ≤ cur, not both 0) and the segment prev–cur meets the line
     segment p1–p2 (0x8294: integer intersection point inside both coordinate ranges; parallel → no) → event class 1;
   - mode 4, region (+0xac8 = 1): the same test; the class depends on the direction relative to the edge's inside sign
     +0xacc[i]: with sign −1, prev ≤ 0 → cur ≥ 0 is class 1 and the reverse class 2; with any other sign the opposite;
   - mode 0 (directional): only a change from the fdp side to the sdp side (s1 > 0: prev ≥ 0 → cur ≤ 0; s1 < 0: prev ≤ 0 →
     cur ≥ 0) is **reported** (class 1); the reverse change is entered into the history without a report.
5. **Event record** (0x83f8): the crossing point (cur) is compared with the line's history (up to 10 points, x +0x614, y
   +0x628, age +0x63c, count +0x600, per-line stride 0x58); a point within D on both axes of a stored one is a repeat; a new
   point is stored, and if reported: +0x608 = class, +0x650 / +0x654 = point, +0x60c += 1, and engine +0x764 = 1 makes the
   detection call `vg_cross_bounding_box_info` for that object: the box is shrunk back by m on every side (undoing 0x2f50's
   margin) and compared with the engine's object list (0x24-byte records from +0x770: up, down, left, right; count +0x760),
   last to first: an overlapping stored box is replaced by the union (a second overlapped one is zeroed) and nothing is
   appended; otherwise the box is appended (count += 1, no upper bound). The exact branch order of the "new point" test of
   0x83f8 is not transcribed; only the rule above is known [I: summary of the branches].
6. The current centroids become the previous ones (x → +0xa4, y → +0xf4, count → +0xa0); every non-empty history entry ages
   by one.

**`vg_cross_result(engine, alarm[4])`**: gate mode — alarm[l] = +0x608 of line l (then cleared) and a process-global int[4]
(GOT slot 0x1d0b0, R_ARM_RELATIVE) [l] = the crossing count +0x60c; region mode — alarm[0] = the class of the last edge (in
edge order) that has class 1 or 2, the other entries 0, and the global counter [0] += 1 per edge whose count flag is 1 (that
flag cleared) [R; I: "count flag" = +0x60c reaching 1]. `IvaVG_GetResult` copies alarm[0…n−1] and the boxes
(`vg_cross_bounding_box_info`, §11.6).

### 11.10 Unused code in libVG [R: no direct call in the decompile; role I from the code]

0x4c48 (single-pixel 3×3 texture pattern — the same test 0x4d0c does inline), 0x5150 (box sum from the integral image),
0x5b58 (per-region 256-bin histograms), 0x625c (debug: allocates a frame-sized buffer and composes the regions into it),
0x76e0 and 0x7828 (point-in-image / equal-direction-point checks of an older `setparams`), 0x9720 (the YYMMDDhhmm timestamp
from `localtime`, `audio-algorithms.md` §2.4), 0xa254 (aligned `memcpy`), 0x12dc / 0x17e4 / 0x18d4 (§11.5), the base-62 codec
0x9800 (`audio-algorithms.md` §2.3) and the second chip-gate copy 0xa548. All unused.
