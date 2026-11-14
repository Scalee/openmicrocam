# chagall-fw — `chagall.bin`, the Wave5 codec firmware: container, image map, VCPU instruction set, program map, firmware side of the host interface, VCORE engine segments
<!-- ev:chagall-fw -->

`chagall.bin` is the firmware of the camera's H.264 / H.265 codec, a Chips&Media Wave5-class VPU. The file carries two
programs for two processors inside the codec: the **VCPU firmware** — the control processor's program (host-command
layer, scheduler, header writers and parsers, rate control, C runtime), 16-bit instructions of an EISC-family ISA — and
four 8 KiB **VCORE engine-microcode segments** in a second, undecoded instruction set, one per codec and direction. The
host (mhal.ko, VCODEC block) copies the file raw into the codec's code region and talks to the VCPU through the Wave5
host register window. This document covers the container and its version tag, the image map, the VCPU instruction set,
the program map, the firmware side of the host interface, the VCORE window and the engine segments.

Shared facts owned elsewhere: the host side of the host ↔ VCPU interface — firmware load path order, size limit and the
host's version check, the code region and the remap, the init-block values the host writes, the host interrupt handler
and the instance-slot decode, the product-id fake and the `GET_VPU_INFO` property reads, codec clocks and throughput —
`mhal-vcodec.md` §2.2, §2.3, §2.5, §2.7, §3, §4; the file's place in the firmware inventory — `firmware-image.md` §3.6;
mi_venc's `fw_path` — `mi_venc.md` §4.

Addresses are VCPU addresses; for this image they equal file offsets (§3.1). Tags: **[R]** read from the binary or a
file named, including runs of the image's own routines in the emulator; **[S]** stated by a source (cited); **[I]**
inference. A heading tagged [R] means every statement under it is [R] unless marked otherwise.

## 1. What the file is

- File: `/config/venc_fw/chagall.bin`, on the camera `/config/venc_fw/chagall.bin` (the only file in
  `venc_fw`), **394 112 bytes (0x60380)**, sha256 prefix f2dfc1ebd9594e46; last non-zero byte at 0x5fd18, zero padding
  after [R].
- **Two programs** [R layout]: the VCPU firmware (code from 0x1040 to 0x4e6cc, data after it) and four 8 KiB VCORE microcode
  segments (0x4e940–0x56940) in a different instruction set (§7).
- **Codec set** [R strings and code]: HEVC and AVC decode (`hevc_dec_*`, `avc_dec_*`), HEVC and AVC encode (the HEVC
  encoder through `hevc_enc_*` — `hevc_enc_one_pic_vcpu` / `_vcore`, rate control, RPS, slice header, VUI — and a
  separate AVC encoder module), a multi-instance scheduler (`sched_*`, including `sched_avail_core`, `sched_alloc_core`
  and `sched_release_resource_reencode`) and the host-command layer. An encoder + decoder ("codec") build of a
  Wave521C-class core trimmed to AVC + HEVC [I].
- **One VCORE**: the per-core pointer array at 0x5f0f4 has one entry (the next word, 0x5f0f8, begins a register-name
  pointer table) [R]; no segment or code path serves a second VCORE [R] (§7).
- **Processor family** [I, strong]: an **EISC-family core of the Advanced Digital Chips AE32000 class** — fixed 16-bit
  instructions, an immediate-extension prefix instruction (`LERI`) with a 14-bit payload chained for wider constants, 16
  general registers plus special registers pushed and popped by mask (§4). Outside sources [S]: EISC = 16-bit
  fixed-length instructions, operand extension through an extension register loaded by `LERI` in 14-bit steps on the
  32-bit cores; AE32000 has 16 GPRs and 7 special registers (ko.wikipedia "EISC"; US patent application 2014/0317381
  "Method of processing immediate value in EISC processor"); Chips&Media and ADChips are both Korean companies [S,
  general]. No public AE32000 opcode table or toolchain exists as far as searched (web search 2026-11-10); the closest
  public reference is MAME's `se3208` core, the 8-register 32-bit EISC predecessor [S]. The encoding in §4 is read from
  the image.
- **Toolchain** [R strings, I attribution]: a GCC-style C toolchain — the C library is newlib (strings "ISO-8859-1",
  "Heap and stack collision" = newlib `_sbrk` message) and libgcc integer, 64-bit and soft-float routines are linked in
  (§5.4). One source path is in the image: `../../ref_c/src_cq/common/lib/util.c` [R string].
- No source of either program is published; the VCORE instruction set is not public and is not decoded (§7.4).

## 2. Container and version tag

### 2.1 No header [R]

| offset | content |
|---|---|
| 0x0000–0x03ff | VCPU vector table, 256 little-endian words (§3.2); word 0 = **0x1040**, the first instruction after the tag |
| 0x0400–0x0fff | zero |
| 0x1000–0x101b | ASCII **`CODEC40.22031116.I5535c99.03`**, zero-padded to 0x1040 |
| 0x1040– | VCPU code, then data and the VCORE segments (image map §3.1); the instruction stream begins `05 73 0c a0 3f 40 ff 7f …` |

- There is **no header, length or checksum field**. The image is copied raw to the start of the 512 KiB code region,
  and the VPU remap maps VCPU address 0 to the region's base (`W5_VPU_REMAP_CTRL` (0x60) = 0x80000800 | 512K>>12,
  `REMAP_VADDR` (0x64) 0, `REMAP_PADDR` = code base; host side `mhal-vcodec.md` §2.7), so file offsets are VCPU addresses.
- The only content the host checks is the tag: bytes 0x1008–0x100f "22031116" must match (else the load is refused,
  "fw ver mismatch"), bytes 0x101a–0x101b "03" only warn (shared-memory interface version, the driver's `shm ver` 3)
  (`mhal-vcodec.md` §2.7).
- Reading of the tag fields [I]: "CODEC40" product / branch name, "22031116" build stamp (2022-03-11 16 h), "I5535c99"
  source revision, "03" host-interface version.

### 2.2 The StarFive and TI Wave5 images compared [R unless marked]
<!-- ev:chagall-fw.2.2 -->

Images: StarFive `wave511/firmware/chagall.bin` (949 120 B) and the mainline TI image
`cnm/wave521c_k3_codec_fw.bin` (999 104 B).

- **Container**: byte-wise the same layout — 256-entry vector table with the non-default handlers at the same 38
  indexes, reset code starting with the same instruction bytes — but their code starts at 0x1000 and they carry no tag.
  Only this image has the 0x40-byte tag in front of the code (reset vector 0x1040 instead of 0x1000), and it is less
  than half their size.
- **Engine segments**: all three carry four 8 KiB segments with identical leading bytes (StarFive 0x38870…0x3e870, TI
  0x3d550…0x43550, this image 0x4e940…), so the VCORE microcode container is shared across the family; the VCPU code
  around it differs by build.
- **Symbol strings**: this image is a full `__func__` trace build; StarFive carries 21 identifier strings, TI 26 (trace
  strings stripped). The names they share with this image — `get_instance`, `create_instance`, `destroy_instance`,
  `vcpu_get_codec_resource_size`, `vcore_init` / `vcore_reset`, `sched_queue_pic`, `*_parser_irq`, `hevc_vcore_irq` —
  show the common Chips&Media Wave5 code base; a name-level function diff is not possible.
- **Codec set**: StarFive wave511 is decode-only (its names are `*_dec_parser_irq`, `hevc_vcore_irq`); TI wave521c is a
  codec [S/R]. The reference images bundle more standards and more VCORE microcode, which accounts for their size [I].
- `sched_release_resource_reencode` and the re-encode path of the scheduler occur in this image and not among the stock
  names — a SigmaStar addition [R name, I "addition"].
- **Foreign images do not load as they are**: no tag → "fw ver mismatch" and refusal; the StarFive and TI images also
  exceed the host's 0x80000 size limit (`mhal-vcodec.md` §2.7). They would also not understand the SigmaStar
  init-block layout (§6.3), the packed instance-report registers (§6.4) or the ring-reference frame buffers [I].

### 2.3 Product identity

The host never checks the hardware against the image: it fakes the product code as 0x5210 (WAVE521) and hard-codes the
capabilities (`mhal-vcodec.md` §4.5, §4.6). The firmware reports its name, version and configuration through the QUERY
option `GET_VPU_INFO` (handler 0x5cf6, §6.2), which the host prints with `PrintVpuProductInfo` (log module `dbg`, level
≥ 3, set through `/sys/class/mstar/venc/log_ctrl`; `mhal-vcodec.md` §4.6). The values it reports on this camera have not
been read; nor have the silicon's `W5_PRODUCT_NUMBER` (0x1044) and `VPU_RET_VPU_CONFIG1` bit 26 (dual core). The
firmware itself reads W5 0x94 (`W5_VPU_RET_PRODUCT_VERSION`) and 0xa8 when it boots the VCORE (§7.2).

## 3. Image map and address space

### 3.1 Image map (whole file, 0x0–0x60380)
<!-- ev:chagall-fw.3.1 -->

| range | content | tag |
|---|---|---|
| 0x00000–0x003ff | VCPU vector table (§3.2) | [R] |
| 0x00400–0x00fff | zero | [R] |
| 0x01000–0x0103f | version tag, zero-padded (§2.1) | [R] |
| 0x01040–0x4e6a6 | VCPU code: 16-bit instructions; no halfword ≥ 0xF000 occurs in it; 1554 functions (§5) | [R] |
| 0x4e6a6–0x4e6bf | zero; our function table carries it as one unreferenced entry `tail__unused_4e6a6` (0x4e6a6–0x4e6c0) | [R] |
| 0x4e6c0–0x4e6cb | three tiny leaf routines after the zero gap; they lie outside the 0x1040–0x4e6c0 range the disassembly and the function table cover, and no caller of them is recorded | [R bytes] |
| 0x4e6cc–0x4e93f | read-only data of the first module: switch tables (32-bit code addresses, e.g. at 0x4e6cc…, 0x4e8b0…), a power-of-two mask table 0x4e710–0x4e74f, `__func__` strings `vcpu_init` … `vcpu_get_codec_resource_size` (`main` at 0x4e804) | [R] |
| 0x4e940–0x56940 | **four 8 KiB VCORE microcode segments** (§7.1) | [R] |
| 0x56940, 0x56944 | two words: 0x00004000 (no reader in the code, unused) and 0x0000002e (written to 0xFFFF1000 by the VCORE reset paths, §7.7) | [R] |
| 0x56948–~0x5e8c0 | read-only data: the `__func__` identifier strings of every module, printf format strings (trace, error, exception texts), syntax-element names of the HEVC / AVC header writers and parsers (VUI, HRD, SEI), the `GradIQP_*` strings 0x58657–0x586b1, register-name strings from 0x59ffc, exception texts 0x5a06c–0x5a1c0, switch tables, halfword-pair number tables (e.g. words 0x59120, 0x593ec, 0x59454, 0x59660) | [R] |
| ~0x5e8c0–0x5fd18 | initialised data: three copies of the register-name pointer table (0x5e900, 0x5e960, 0x5f0f8), the instance-record table 0x5ebec (20-byte records, §6.2), small globals (e.g. 0x5f0f0 = 0xff, 0x5f0f4 = 0xffff8000 = the VCORE window pointer, §7.2), newlib locale data (`ISO-8859-1` pointer at 0x5fd14) | [R] |
| 0x5fd18–0x6037f | zero (end of file) | [R] |
| 0x60380–0x6ec78 | not in the file: bss, cleared by the reset code on `INIT_VPU` (§5.3) | [R] |
| 0x6ec78–0x7ffff | rest of the 512 KiB code region the host reserves; the stack lies below the top the host gives in `W5_CODE_SIZE` (0x114) and is filled with 0x55aa55aa at reset [R]; a heap here [I] | |

- **Addresses = file offsets** [R]: every 32-bit pointer in the data part (register-name table → strings at 0x59ffc…,
  switch tables → code, the `ISO-8859-1` pointer) is a plain file offset, and code builds the same values as
  immediates (e.g. 0x4e940, the first segment, is formed at 0x6aa8 and 0x6b3a).
- The halfwords at 0x4e6c0 (`e0c8 02d5 c0e0 a0e0 c1e0 a0e0` as bytes) read by the table of §4.2 as `cmpq r8, 0; jz`,
  `swi 0; ret`, `swi 1; ret` [R bytes, I reading].

### 3.2 Vector table [R]

256 words at 0x000. 218 slots hold the default handler **0x1af8**; 38 slots hold other addresses, 34 distinct handler
addresses in all (reset and default included). The non-default slots (function names as in §5; `…__vecN` are
mechanical names, §5.1):

| slot | handler | | slot | handler |
|---|---|---|---|---|
| 0 | 0x1040 reset | | 24 | 0x4c72 (`swi 8`, §5.3) |
| 1 | 0x178c | | 25 | 0x1edc (`swi 9`) |
| 2 | 0x1798 `irq_entry` (§5.3) | | 26, 41 | 0x1f80 |
| 3 | 0x17c6 | | 27 | 0x1f44 (`swi 11`, DMA wait) |
| 4 | 0x17d2 | | 28–31 | 0x1270, 0x127e, 0x128c, 0x129a (atomic-add traps, §5.3) |
| 8 | 0x17de `exception_entry` (§5.3) | | 33 | 0x36c6 |
| 9–12 | 0x1958, 0x1968, 0x1978, 0x1988 | | 34 | 0x2d78 |
| 13 | 0x19b0 | | 35 | **0x4c24 host-command interrupt** (§6.2) |
| 16–19 | 0x4cae, 0x4cc2, 0x4cd6, 0x4ce2 | | 38 | 0x6d6e |
| 20 | 0xb1f6 (scheduler) | | 42 | 0x371a |
| 21 | 0x2f08 | | 43, 45 | 0x19f8, 0x1a4e |
| | | | 46, 47, 49, 50, 51 | **0x6d1e `vcore_irq`** (§7.7) |

The interrupt sources behind the slots without a named role are not known.

### 3.3 Strings carried by the image [R]

- **Identifier strings.** One extraction lists 364, another counts 349 distinct
  identifier strings and, in an image comparison, 335 — the counts are not reconciled. Nine `__func__` strings
  are missing from the 364: `main` (0x4e804), `_sched_run_pic`, `setEncCmdDrop`, `wtl_put_pic`, and the rate-control
  model `GradIQP_lookup_qp`, `GradIQP_init_rq_model`, `GradIQP_calc_qp_from_bpp`, `GradIQP_calc_bpp_from_qp`,
  `GradIQP_update_cond`. Name prefixes: `hevc_*`, `avc_*`, `dec_*`, `enc_*`, `parser_*`, `packer_*`, `sched_*`,
  `pool_*`, `resrc_*`, `report_*`, and the VCPU core handlers (`vcpu_init`, `exec_codec_cmd`, …). Some names occur in
  several modules (`create_instance`, `init_pic`, `init_seq`). Identifier-shaped strings that are arguments, not names:
  `timeout` (0x26230, used by `[FW] vcore %s, inst=%d`), `hrd_parameters_present_flag` and the other syntax-element
  names, `INF` / `inf` / `NAN` / `nan` / `NaN` / `Infinity`, `C`.
- **Trace and fail macros**: formats `%s:%d LR:%x a:%x,%x,%x`, `%s:%d LR:%x a:%x` and `%s(%d) - fail, LR: %08x`, with the
  function's own `__func__` string as first argument; this is how every identifier string is used, so each names the
  function that loads it. The trace output goes nowhere in this build (§5.3).
- **Register names** (pointer table 0x5e900): `GPR0` … `GPRF`, then `CR1`, `CR0`, `ML`, `MH`, `ER`, `LR`, `PC`, `SR`, printed
  by the register dump with `%p, %4s: %08x` (§5.3).
- **Exception texts** (0x5a06c–0x5a1c0): `CP0_EXCEPTION: unknown reason(%08x)`, `SYSTEM BROKEN`, `CP0_STATUS: %08x`, and the
  causes `INSTRUCTION_ADDR_ALIGNMENT_ERR`, `CP0_WRITE_PROTECTION_ERR`, `DATA_ACCESS_VIOLATION`, `DATA_ADDR_ALIGNMENT_ERR`,
  `CP0_PRIVILEDGE_VIOLATION`, `INSTRUCTION_ACCESS_VIOLATION` — the VCPU has a memory-protection unit and a privilege model
  reported through a "CP0" status word [R text, I interpretation].

### 3.4 The VCPU address space, as the code uses it [R]
<!-- ev:chagall-fw.3.4 -->

| range | what |
|---|---|
| 0x00000–0x7FFFF | the program / data region (the host's 512 KiB code region): code and read-only data to 0x5fd18, bss 0x60380–0x6ec78, stack |
| 0xFFFF0000–0xFFFF01xx | VCPU interrupt controller and timers (§5.3) |
| 0xFFFF1000–0xFFFF1018 | VCPU-local control (interrupt enable / disable masks) |
| 0xFFFF2000–0xFFFF2018 | DMA queue |
| 0xFFFF3000–0xFFFF30FF | the W5 host control block (§6.1) |
| 0xFFFF4000–0xFFFF400C, 0xFFFF4100 | UART |
| 0xFFFF5000–0xFFFF5C38 | stream unit |
| 0xFFFF6800–0xFFFF6B5C | encoder packer |
| 0xFFFF7000–0xFFFF70FF | the W5 host command window 0x100–0x1FF (§6.1) |
| 0xFFFF8000– | the VCORE window (§7.6) |
| 0xFFFFCE0C–0xFFFFCE18 | used around the encoder run with W5 0x332c (a handshake that waits for 0xdead) [R addresses, I role] |
| 0xFFFFFE10, 0xFFFFFE14 | VCORE reset handshake (§7.7) |

- Every branch and call target of the code lies inside 0x1040–0x4e6c0, and all **7 420 calls land on a function start**
  — 4 563 on a `push {lr}` prologue, 2 857 on the first word of a leaf right after a `ret` / `pop {pc}` / `jmp` / `nop`
  (full scan).
- Constants between 0x80000 and 0xFFFF0000 occur (sizes, masks, bit values such as 0x100000, 0x800000), but none is
  used as the base of a load or store within the next instructions (full scan of `ldi` / `add` constants); an address
  reached through a longer computation is not excluded by that scan.
- Peripheral roles are [I] where the table does not cite a use; the register roles inside the blocks 0xFFFF0004–0x0104,
  0xFFFF1000–0x1018, 0xFFFF2000, 0xFFFF4000–0xFFFF6Cxx are not known (addresses and access sites in).

## 4. VCPU instruction set

### 4.1 Shape and the LERI prefix [R unless marked]
<!-- ev:chagall-fw.4.1 -->

- Every instruction is one 16-bit little-endian halfword. The top bits select a class (§4.2); class `1111` never occurs
  in the code.
- **LERI** (`01ii iiii iiii iiii`, halfwords 0x4000–0x7FFF) is the immediate-extension prefix: it loads its 14-bit
  payload into the extension register ER. A LERI never stands alone — it always sits right before a non-LERI instruction
  that consumes ER, and the combined values land on known string and function addresses. The **first LERI
  sign-extends** its 14 bits; each further LERI shifts in 14 more (SE3208 rule [S]; the branch targets prove it [R]).
  The image uses at most two chained LERIs. A branch with a negative first LERI goes backward: `leri 0x3fe2; call` at
  0x4d44 calls 0x11e8 (ER = −0x1e). A data constant built with a negative first LERI is 0xFFFFxxxx — the VCPU's
  peripheral space 0xFFFF0000–0xFFFF7FFF and above (§3.4).
- **ER form** of the consuming instruction: its immediate / offset becomes `(ER << 4) | (low 4 bits of the scaled short
  field)`; branches and calls take `(ER << 8) | imm8`. A LERI-prefixed word access always has field bits 3:2 = 0 (8 797
  cases), as the rule predicts.
- The encoding is read from the image, anchored on ground-truth code read by hand — the reset routine 0x1040, the bss
  clear 0x10c2 and stack fill 0x10d0, the register dump 0x11e8 (loop i = 0…23 over the name table 0x5e900), the exception
  entry, the copy routine 0x13b0, the fixed-point helpers 0x7d6e–0x7f40 (a 4-byte `{int16 mantissa; int16 exponent}`
  type), the CP0 cache helpers 0x15b8–0x16ae — and on the opcode classes that match MAME's `se3208` core [S: `se3208.cpp`]: the LERI prefix (bits 15:14 = 01), the branch class 0xD with the
  same 16 condition codes, the ER-extension rule, and the ALU operation order add / adc / sub / sbc / and / or / xor / cmp.
  The register fields, the class numbers of everything else and the 16-register widths differ from SE3208.
- **Execution check** [R]: an emulator of the decoded semantics runs routines of
  the image on random inputs against C semantics — `min` 0x7d6e, `clamp(lo, hi, v)` 0x7d7e, the normaliser 0x7d94 (count
  of redundant sign bits + 1), the 32-bit divisions 0x46b14 / 0x46b98, the copy routine 0x13b0 (copy, then cache call
  0x16ae, returns nothing), `__ashldi3` 0x47c4a, `__udivdi3` 0x47c8e: all 1 820 runs give the expected result (the two
  64-bit routines were identified by trying five candidate semantics; only the named one fits). Later runs of the
  soft-double routines (§5.4) also execute `adc` / `sbc`. The runs cover word / half / byte loads and stores,
  add / adc / sub / sbc / and / or / xor / cmp in register and LERI-immediate form, shifts by immediate and by register,
  `addq` / `cmpq`, `clz`, `mul`, `neg`, push / pop, call / ret, `ldi`, and the conditions jp / jz / jnz / jnc / jc /
  jgt / jlt / jge / jle / jhi / jls. Not executed: `mfmh`, sign / zero extension, `lea`, the sp-relative byte / half
  forms — those rest on the code reading in the table.
- **Coverage** [R]: our disassembler decodes every halfword of 0x1040–0x4e6c0 (116 446 instructions).

### 4.2 Encoding table [R unless marked]

Fields: `d` destination / data register, `s` source register, `b` base register, `o` offset, `i` immediate, `Z` size bit
(0 byte, 1 halfword).

| halfword | form | meaning | evidence |
|---|---|---|---|
| `0000 dddd oooo bbbb` | `ld.w rd, [rb + o*4]` | word load | reset poll `0000`, pointer chases; emulator |
| `0001 dddd oooo bbbb` | `st.w rd, [rb + o*4]` | word store | bss clear `1200`, HW writes `1100` |
| `0010 dddd Zooo bbbb` | `ld.b` / `ld.h rd, [rb + o<<Z]` | **signed** byte / half load | 14 loads compared at once with −1; fixed-point mantissa |
| `0011 dddd Zooo bbbb` | `st.b` / `st.h` | byte / half store | fixed-point constructor 0x7dde stores `{mant, exp}` at +0 / +2 |
| `0100`–`0111` … | `leri i14` | immediate-extension prefix | §4.1 |
| `1000 dddd Zooo bbbb` | `ldu.b` / `ldu.h` | **unsigned** byte / half load | never followed by an extension or a compare with a negative value (1 429 uses) |
| `1001 dddd S ooooooo` | `ld.w` / `st.w rd, [sp + o*4]` (S = 1 store) | sp-relative word | register dump, varargs spill at 0x11a6, copy routine's third argument |
| `1010 dddd iiiiiiii` | `ldi rd, sext(i8)` / ER form `(ER<<4)\|i[3:0]` | load immediate | `a0ff` = −1, `a07c` = 0x7c, every built address |
| `1011 0000` / `1011 0001 mmmm mmmm` | `pop` / `push {r0–r7 by mask}` | | prologues / epilogues; copy routine `b15f` = r0–r4, r6 |
| `1011 0010` / `1011 0011` | `pop` / `push {r8–r15 by mask}` | | `b304` = r10 |
| `1011 0100` / `1011 0101` | `pop` / `push {cr1, cr0, ml, mh, er, lr, pc, sr}` (bits 0…7) | `b520` = push lr; `b53f` = push cr1 … lr; `b440` = pop pc = **return**; `b4ff` = exception return | prologue markers; `b440` sits last before the next function 420 times |
| `1011 0110 iiiiiiii` | `addsp sext(i8)*4` / ER form `(ER<<4)\|i[3:0]` bytes | stack adjust | prologues `b6fc` / `fd` / `fb` (negative = allocate), epilogues `b603` / `04`; `leri 0x7fdf; b600` = −0x210 at 0x11a6 |
| `1011 1ooo dddd ssss` | `add adc sub sbc and or xor cmp` (o = 0…7) `rd, rs` / ER form `rd, imm` | two-operand ALU | order [S: SE3208]; all eight executed; `add r8, r0; adc r9, r1` 64-bit adds at 0x1c3a |
| `1100 dddd 0aaaaa tt` | `asr` / `lsr` / `asl rd, a` (tt = 0 / 1 / 2) | shift by 0–31; **sets C to the last bit shifted out** (asr / lsr: bit a−1; asl: bit 32−a) | `lsr 29` bit-field extracts, clz / normalise; emulator — with the carry rule the soft-double add / sub / div are IEEE-exact on 40 of 40 random pairs each, without it 15 / 15 / 10 of 40; tt = 3 never occurs |
| `1100 dddd 10ssss tt` | `asr` / `lsr` / `asl rd, rs` | shift by register | emulator (64-bit shifts) |
| `1100 dddd 110iiiii` | `addq rd, sext(i5)` | add −16…15 | loop counters `c0c1`, `c0c4`, `c4d0` = −16 |
| `1100 dddd 111iiiii` | `cmpq rd, sext(i5)` | compare with −16…15 | `cmpq r8, 0` (`c8e0`) after calls, 3 797 uses |
| `1101 cccc iiiiiiii` | `jnv jv jp jm jnz jz jnc jc jgt jlt jge jle jhi jls jmp call` (c = 0…F) | PC-relative, target = PC + 2 + 2·offset | SE3208 [S]; loops, min / clamp |
| `1110 0000 0000 dddd` … `0001`, `0100`, `0101` | `sext.b`, `sext.h`, `zext.b`, `zext.h rd` | extension, sets Z | `sext.b` then `cmpq −1` at 0x9e6e; pairing with byte / half loads; zero / sign split of 4 / 5 vs 0 / 1 [I] |
| `1110 0000 0110 dddd` / `0111` | `neg rd` / `not rd` | | `neg`: signed division (emulated); `not` before a leading-ones count at 0x7dda [I] |
| `1110 0000 1000 ssss` / `1001` | `jr rs` / `callr rs` | jump / call through register | switch tables `ld.w r8, [r8]; jr r8` |
| `1110 0000 1010 0000` | `ret` (jump to lr) | | 527 uses, after a pop and before the next function's `b520`; leaves that did not push lr |
| `1110 0000 1011 0000` | `nop` | padding runs | |
| `1110 0000 1100 nnnn` | `swi n` | trap: **enters vector 16 + n** [I, strong] | the stubs `swi 12 / 13 / 14 / 15; ret` 0x1224–0x1230 pair with the vector-28…31 handlers (§5.3); `swi 11` 0x1d68 ↔ vector 27, `swi 8` 0x4c66 / 0x4c6c ↔ vector 24, `swi 9` 0x1d60 ↔ vector 25 |
| `1110 0000 1110 0001` | `halt?` | stops the core [I] | after every assert message, in `halt; halt; jmp .-4` loops; in the IRQ stubs 0x1960… with r0 = 1 / 2 / 3 |
| `1110 0001 0000 ssss` | `clz r0, rs` | count leading zeros into **r0** (implicit) | division routines (emulated) |
| `1110 0001 0010 nnnn` / `0011` | `clrf? n` / `setf? n` | clear / set status flag n [I]; n = 8 brackets every CP0 / HW read-modify-write (interrupt disable / enable) [R pattern] | |
| `1110 0001 0111 dddd` | `mfmh rd` | high 32 bits of the last `mul` | 64-bit products `mul r10, r8; mfmh r11` returned as r8:r9 at 0x2f1b8 |
| `1110 0001 1010 0000` | `sync?` | barrier after CP0 writes and flag changes [I]; 200 uses [R] | |
| `1110 0001 1100 0000` | `e1c0` | one site; meaning not known | |
| `1110 0001 1101 dddd` | `lea rd, [sp + ER]` (no LERI: `rd = sp`) | address of a stack slot | `leri 0x220; e1d8` = varargs start at 0x11c0 |
| `1110 0011 aaaa ssss` | `tst ra, rs` / ER form `tst ra, imm` | and without write-back | `tst r8, 0xff000000; jz` in soft-float |
| `1110 0100 dddd ssss` | `mov rd, rs` | **destination is bits 7:4** | reset code; argument set-up before calls; 13 210 uses, source never equal to destination |
| `1110 0110 dddd ssss` | `mul rd, rs` / ER form `mul rd, imm` | low 32 bits to rd, high to MH | `mul r0, 0x14` array index; emulated |
| `1110 1000 Zooo dddd` | `ld.b` / `ld.h rd, [sp + o<<Z]` | signed sp-relative byte / half | fixed-point helpers read struct-by-value halves |
| `1110 1001 Zooo dddd` | `ldu.b` / `ldu.h rd, [sp + …]` | unsigned sp-relative | by analogy with class 8, 41 uses |
| `1110 1010 Zooo dddd` | `st.b` / `st.h rd, [sp + …]` | sp-relative store | byte 3 of a volatile register copy at sp+7 (0x4734) |
| `1110 1100 0110 nnnn` / `0111` | `mvtc crn, r0` / `mvfc r0, crn` | CP0 register move (r0 implicit) | exception entry `mvfc r0, cr15` → cause `(x >> 2) & 0xf`; MPU set-up at 0x10e2 writes cr3 / cr4 pairs; cache ops write cr11 [R]; register roles [I] |
| `1111 …` | — | never occurs in code | |

Halfword groups that never occur in the code: `1011 0111 …` (0xB7), `1110 xxxx` with xxxx = 0010, 0101, 0111, 1011,
1101, 1110, 1111 (0xE2, 0xE5, 0xE7, 0xEB, 0xED, 0xEE, 0xEF), and the whole `1111` class. Their meaning is not known;
by the pattern of the neighbours 0xEB would be the missing sp-relative store variant and 0xE7 a second multiply [I].

### 4.3 Use statistics and condition codes [R]

- `call` (`DFxx`): of 7 420 calls, 4 563 land on a `push {lr}` prologue `b520`, the rest on a leaf's first word.
  `jmp` (`DExx`): 2 751 of 3 071 go backward (loop tails); a few go forward to function starts (tail calls). `jz`
  (`D5xx`) is the most common conditional after a compare, 247 of them going to function entries (tail calls); `jnz`
  (`D4xx`) the second.
- Conditions 0 and 1 (`jnv`, `jv`) do not occur as branches; `jp` (`D2xx`) has 16 uses and `jm` (`D3xx`) 2.
- `cmp` with a LERI immediate (`BFxx`) is followed by a conditional branch in 85% of uses.
- LERI-immediate operands of the ALU forms cluster on masks and powers of two — `0x00ffffff`, `0x001fffff`,
  `0xfffdffff`, `0x100000`, `0x800000`, `0x400000`.

### 4.4 Registers and calling convention [R unless marked]

- 16 general registers r0–r15 and 8 special registers, named by the image's table: `cr1`, `cr0` (control), `ml`, `mh`
  (multiply result low / high), `er` (extension), `lr`, `pc`, `sr` (status) [R names, I roles]. `sp` is not one of
  r0–r15: sp-relative forms name no register and push / pop / addsp act on it implicitly. CP0 registers are reached
  through `mvtc` / `mvfc`.
- **Arguments**: r8, r9, then the stack at caller_sp+12, +16, … (the copy routine 0x13b0 reads its third argument with
  `ld.w r4, [sp+0x38]` after a 0x2c-byte frame + pushes = caller_sp+12). Slots caller_sp+4 / +8 are the home slots of r8
  / r9: a variadic callee spills them there so the arguments are contiguous (0x11a6: `st.w r8, [sp+0x21c]`,
  `st.w r9, [sp+0x220]`, `va_list = sp+0x220`). caller_sp+0 is not seen used (no store there in the samples read).
- **Return value** in r8; 64-bit values in **r8:r9** (low : high), as `__udivdi3` returns them and `add r8, r0; adc r9, r1`
  uses them.
- A struct passed by value: its first word in the register, the rest in the stack slots after it; small structs are built
  in the outgoing area (0x2de68: a `{int16, int16}` copied into sp+0xc as the third argument).
- **Saving**: each function pushes exactly the registers it writes, r0–r7 included (the copy routine pushes r0–r4, r6,
  r10); r8 / r9 are not saved by callees that return a value in them [R for the functions read; I as a general rule].
- **r5 and r7** are almost never a base or data register of a register+offset access (r5 once and r7 once as a base
  among 22 781 such accesses, never as data). r7 serves as the scratch copy of sp (`lea r7, [sp]` 205 times, then
  `add rX, r7`); r5 is otherwise touched only in the libgcc float / multiply code (`mfmh r5` at 0x476b4). Whether the
  hardware reserves them is not known.

### 4.5 Semantics not known

- The exact hardware effect of `halt?`, `clrf? n` / `setf? n`, `sync?` and the single `e1c0` (use patterns are read).
- `mul` signed vs unsigned and the exact `mfmh` result: no model of `mul` / `mfmh` tried (unsigned, signed, swapped
  halves, flags) reproduces the image's `__mulsf3?` 0x471a0 and `__muldf3?` 0x47632 (§5.4). The shift-amount mask (5 or
  6 bits) is not known; neither it nor the `mul` signedness changes the soft-double routines' results.
- Condition 2 (`jp`): read as "N clear", it sends `__umodsi3` 0x46c50 for a divisor ≥ 2³¹ that is not a power of two
  into the `a & (b−1)` path — 4173883019 mod 2194286183 gives 2160600578 instead of 1979596836 in the emulator (the
  routine tests "power of two" with `tst r9, r9−1; jp`, where `__udivsi3` 0x46b14 uses `jz`). Either the routine is wrong
  for such divisors or condition 2 is not "N clear"; not decided.

## 5. Program map

### 5.1 Functions [R]
<!-- ev:chagall-fw.5.1 -->

- The code range 0x1040–0x4e6c0 holds **1554 functions**: 1329 called directly, 34 reached only through the vector
  table (the 34 distinct handler addresses of §3.2), 13 reached only through an address constant or a data word, and
  **178 unreferenced** (no call, no address, no branch from other code; 11 468 bytes) — code linked in and never used,
  named `…__unused_…` or by its library name.
- Entry rule: direct call targets ∪ `push {lr}` / `push {cr1 … lr}` prologues not followed by `pop` of one register ∪
  vector-table entries ∪ the first instruction after an unconditional exit (`ret`, `pop {pc}`, `jmp`, `jr`; `nop`
  padding skipped) when no branch of the current stretch and no switch-table entry reaches past it
 . A `push {lr}` followed by `pop {rX}` is the "rX = lr" idiom inside a function —
  the trace macro (`pop {r8}`) and the fail tail (`pop {r9}`, line number, `halt` loop) — and starts no function (470
  such places, none a call target).
- Every function has a name in our function table, [`chagall-fw-functions.tsv`](../data/chagall-fw-functions.tsv) (start, end,
  how it is reached, name, tag, naming source, module, direct callers, evidence), with its tag:

| source | functions | tag | how |
|---|---|---|---|
| `__func__` string | 291 | [R] | the function loads an identifier string for the trace / fail macros (§3.3); four functions load several (the HEVC VUI / HRD / SEI header writers pass every syntax-element name to the bit writer) and carry their `write_*` / `hevc_encode_vui` name |
| execution | 56 | [R] | run in the emulator against 71 C / libgcc semantics, 4 quick + 150 confirming random trials, all exact |
| read by hand | 96 | [R] or [I] per entry | runtime, drivers, bit writers, newlib entry points |
| mechanical | 1111 | [I] | `<root>__h_<addr>` (234): every call path up reaches the one named function `<root>`; `<module>__shared_<addr>` (697): paths reach several (listed in the evidence column); `<module>__unused_<addr>` (143); `<module>__vec<slots>` (25); `<module>__ptr_<addr>` (12, address taken) |

  A name that occurs in several modules carries its address (`create_instance@04eb0`, `init_pic@366e0`).

### 5.2 Modules [R counts, I boundaries]

Link order, from the names and strings each range carries:

| module | range | functions | `__func__` | exec | hand | mechanical |
|---|---|---|---|---|---|---|
| rt (start-up, traps, cache, IRQ, DMA, stream-unit and packer drivers, timers, UART) | 0x01040–0x03f54 | 182 | 0 | 2 | 66 | 114 |
| vcpu (host-command layer) | 0x03f54–0x06a8a | 75 | 13 | 0 | 0 | 62 |
| vcore (engine boot, reset, wait, shared engine helpers) | 0x06a8a–0x07022 | 14 | 2 | 0 | 4 | 8 |
| packer interrupt handlers | 0x07022–0x07360 | 3 | 2 | 0 | 0 | 1 |
| util (`util.c`, fixed-point `{int16, int16}` type, queues) | 0x07360–0x0a34c | 206 | 1 | 18 | 3 | 184 |
| sched | 0x0a34c–0x0e762 | 57 | 35 | 0 | 1 | 21 |
| resrc | 0x0e762–0x0ecae | 10 | 9 | 0 | 0 | 1 |
| report_queue (+ ramlog writer) | 0x0ecae–0x0f464 | 25 | 9 | 0 | 0 | 16 |
| trace stubs | 0x0f464–0x0f468 | 2 | 0 | 0 | 2 | 0 |
| HEVC decoder | 0x0f468–0x1cb80 | 177 | 108 | 0 | 0 | 69 |
| AVC decoder | 0x1cb80–0x26e1e | 205 | 8 | 0 | 0 | 197 |
| HEVC encoder (+ the shared header, RC and RPS code) | 0x26e1e–0x394e2 | 229 | 101 | 8 | 5 | 115 |
| AVC encoder | 0x394e2–0x46b14 | 203 | 3 | 2 | 2 | 196 |
| libgcc (integer, 64-bit, soft-float) | 0x46b14–0x48088 | 17 | 0 | 11 | 3 | 3 |
| newlib | 0x48088–0x4e0b0 | 140 | 0 | 8 | 10 | 122 |
| libgcc soft-double compare / convert | 0x4e0b0–0x4e6a6 | 8 | 0 | 7 | 0 | 1 |
| tail (the zero stretch, §3.1) | 0x4e6a6–0x4e6c0 | 1 | 0 | 0 | 0 | 1 |

The AVC decoder and encoder carry almost no `__func__` strings (their traces use format strings only), so most of their
names are mechanical [R counts]; so are 122 of newlib's 140.

### 5.3 Runtime layer (rt) [R unless marked]
<!-- ev:chagall-fw.5.3 -->

- **Reset** 0x1040: reads the command word 0xFFFF7000 and compares it with 1 / 2 — `INIT_VPU` (1) clears bss
  0x60380–0x6ec78 (0x10c2), `WAKEUP_VPU` (2) keeps it; loads the stack top from 0xFFFF7014 (`W5_CODE_SIZE`); fills the
  stack area with 0x55aa55aa (0x10d0); writes `VPU_CLK_MASK` (W5 0x05C) = 0xFFFFFF; calls `main` 0x4a3a. The MPU
  set-up at 0x10e2 writes CP0 cr3 / cr4 pairs.
- **Interrupts**: `irq_entry` 0x1798 (vector 2) reads the pending word 0xFFFF000C, finds the lowest set bit and writes
  its number as a byte to 0xFFFF0000 (acknowledge). 0x1760 read-modify-writes 0xFFFF1010 / 0xFFFF1018 with interrupts
  off (enable / disable masks [I]).
- **Exceptions**: `exception_entry` 0x17de (vector 8; its first instruction `push {cr1,cr0,ml,mh,er,lr}`, 0x17e0 being the second push of the same prologue —) [R] reads the cause
  with `mvfc r0, cr15` → `(x >> 2) & 0xf`, prints the CP0 texts (§3.3) and copies its cause word to host window 0x190
  and the 24 saved registers to 0x194… — a crash dump the host can read (§6.1). `dump_registers` 0x11e8 loops i = 0…23
  printing `%p, %4s: %08x` over the name table 0x5e900, through `trace_printf`, so it prints nothing in this build.
  Exception return is `pop` with mask 0xff (`b4ff`). A standard EISC CP0 trap frame [I].
- **Traps** (`swi n` → vector 16 + n [I, strong]): vectors 28–31 (0x1270, 0x127e, 0x128c, 0x129a) add r9 (r9:r10 for 31)
  to the byte / halfword / word / doubleword at [r8] and return with `pop {…, pc, sr}` — atomic read-modify-write by
  trap; their callers are the stubs `swi 12…15; ret` 0x1224–0x1230 (0x1230 loads r9:r10 from a 64-bit argument as
  handler 31 consumes them). `swi 11` (0x1d68) → vector 27 = 0x1f44, which polls the DMA busy bit 0xffff2014 like
  `dma_wait_idle` 0x1d6c; `swi 8` (0x4c66 / 0x4c6c with r8 = 1 / 0) → vector 24 = 0x4c72; `swi 9` (0x1d60) → vector 25
  = 0x1edc.
- **Logging**: `trace_printf` 0xf464 is a single `ret` (with `trace_stub2` 0xf466, unused): the 479 trace call sites
  produce no output. Output goes through `log_printf` 0x11a6 — a `vsnprintf`-style formatter with a 0x200-byte buffer and
  `va_list` = the caller's stack — → `log_out` 0x3af0 → `uart_puts` 0x3abe (port 0xffff4100), and through the ramlog
  writer (report_queue module; into the ramlog buffer the host gives, §6.3 [I]). `log_puts` 0x11dc is a one-call
  wrapper of 0x3abe. `uart_init` 0x3a68 sets a 16550-style UART (line control 0x83) with the divisor from host register 0x12c.
- **Timers** [R addresses, I roles]: blocks at 0xffff0000 + 16n serve both the interrupt entry (0x0000 ack byte, 0x000c
  pending word) and the timer helpers (`timer_setup` / `start` / `stop` / `read`: +4 control, +8 count); the two roles are
  not separated. `get_time` 0x3a20 reads the counter 0xffff0088 when W5 0x309c bit 27 is set. `delay` 0x155e takes a
  count (unit not known).
- **Memory helpers**: `copy_and_flush` 0x13b0 (byte / word copy, then the cache call 0x16ae; returns nothing),
  `zero_bytes_words` 0x139a, the unused copy loop 0x1372 before it; CP0 cache helpers 0x15b8–0x16ae (cache operations
  write cr11).
- **Drivers**, one per peripheral block [R addresses, I roles]: DMA queue
  0xffff2000–0x2018; UART 0xffff4000–0x400c and 0xffff4100; stream unit 0xffff5000–0x5c38 (strings
  `spp_check_sdma_empty`, `wraparound not support`; read registers at 0xffff5900 + 4·i); encoder packer
  0xffff6800–0x6b5c with the interrupt handlers `packer_int_hander` 0x7022 and `packer_low_lat_int_handler` 0x723a.

### 5.4 Library and helper functions identified [R by execution unless marked]

- libgcc integer: `__udivsi3` 0x46b14, `__divsi3` 0x46b98, `__umodsi3` 0x46c50, `__modsi3` 0x46ce8, `__lshrdi3` 0x47c06,
  `__ashldi3` 0x47c4a, `__udivdi3` 0x47c8e.
- libgcc soft-float: `__floatunsisf` 0x46f16 (falls into 0x46f1e), `__floatsisf` 0x46f1e, `__fixsfsi` 0x475c2 (also fits
  `__fixunssfsi` on the tested range), `__divdf3` 0x4791c; `__eqdf2`-style compares 0x4e0b0, 0x4e1d2 (0 iff equal),
  `__adddf3` 0x4e248, `__subdf3` 0x4e5c6, `__floatunsidf` 0x4e5d8 (unreferenced, unused), `__floatsidf` 0x4e5f6, `__fixdfsi`
  0x4e61e.
- A second single-float set inside the HEVC encoder's rate control: `__floatsisf` 0x32ea0, `__fixsfsi` 0x32fd6,
  `__eqsf2`-style 0x332e2 / 0x3346c, `__mulsf3` 0x334aa, `__divsf3` 0x3360e, `clz32` 0x32bec; one more `__eqsf2`-style
  compare at 0x267c in rt.
- newlib: `memcpy` 0x48afa and 0x4ce42 (the second also fits `memmove` callers' use [I]), `memset` 0x48b68, `strcpy`
  0x48f26, `strlen` 0x48f70, `strchr` 0x4de0a, `strcmp` 0x4de8c, `memcmp` 0x44608 (inside the AVC encoder range).
- util.c and the fixed-point helpers: `abs` 0x7360 / 0x7c00, `min` 0x736a / 0x7c0a / 0x7d6e, `max` 0x737a / 0x7c1a,
  `clip3(lo, hi, v)` 0x738a / 0x7462 / 0x7c2a / 0x7d7e, `sign` 0x7440, `align_up(x, a)` 0x753c / 0x7d54,
  `div_round_up(x, d)` 0x12d2 / 0x754a / 0x7cbe, `round_div(x, d)` 0x7c40 / 0x2ba12 / 0x3da7c, `ceil_log2` 0x7d30,
  `clz32` 0x4d08a. The duplicates are separate copies (util.c linked once, the same helpers again as static inlines in
  other files [I]). Fixed-point `{int16 mantissa; int16 exponent}` type: constructors 0x7dde and 0x7e1c (one of the most
  called functions), normaliser 0x7d94; 0x25a0 is another of the most called.
- By reading, not execution [I]: `__addsf3` 0x46f82 (exact in 138 of 200 runs, the rest a few units in the last place
  off — an emulator gap in its rounding path [I]), `__mulsf3?` 0x471a0 and `__muldf3?` 0x47632 (the `mul` / `mfmh`
  pattern; §4.5).

### 5.5 Host-command layer and VCORE module [R addresses and names]
<!-- ev:chagall-fw.5.5 -->

| address | name | role |
|---|---|---|
| 0x3f54 | `vcpu_init` | start-up read of the init block (§6.3) |
| 0x435c | — (`vcpu__shared_0435c`) | immediate-command dispatcher (§6.2) |
| 0x4412 | — | copies window words 5…n into a queue entry (§6.2) |
| 0x4580 | `get_instance` | instance index from window 0x110 (low 16 bits) → record `idx·0x14 + 0x5ebec` of the 20-byte instance-record table; its fail block 0x45e4 (line 0x3f1) is inside it |
| 0x4606 | — | the same record lookup from an index argument (read it as the body of `get_instance`) |
| 0x482c | `vpu_trigger_intrpt` | the interrupt raiser (§6.4); its fail block 0x4876 |
| 0x4a3a | `main` | calls `vcpu_init` with the window base, `vcore_init` (call at 0x4a70), clears the wake mask (0x4c1e) |
| 0x4c24 | vector 35 | host-command interrupt (§6.2) |
| 0x4cee | `exec_codec_cmd` | |
| 0x4e08 | `exec_queue_cmd` | |
| 0x4eb0 | `create_instance@04eb0` | CREATE_INSTANCE handler |
| 0x5648 | `destroy_instance@05648` | DESTROY_INSTANCE handler |
| 0x5804 | `enqueue_cmd` | queued-command entry (§6.2) |
| 0x5e96 | `dump_inst_and_cmd_status` | |
| 0x6026 | `query_vcpu_status` | QUERY option 0x60 |
| 0x644a | `query_flush_cache` | QUERY option 0x63 |
| 0x67ae | — | acknowledge: `RET_SUCCESS` / `RET_FAIL_REASON` |
| 0x67fc | `vcpu_get_codec_resource_size` | |
| 0x6a8a | `vcore_init` | VCORE boot (§7.2) |
| 0x6b9c | `vcore_reset` | §7.7 |
| 0x6c9c | `vcore_soft_reset` | §7.7 |
| 0x6d1e | `vcore_irq` | VCORE interrupt, vectors 46, 47, 49, 50, 51 (§7.7) |
| 0x6d6e | vector 38 | |
| 0x6e5c | `vcore_wait_idle` | poll VCORE +0x30 (§7.2) |
| 0x6e94 | `vcore_wait_idle_forever` | unused (never called) |

The other host-command handlers are listed with the dispatchers (§6.2); the queue helper `report_queue_count` 0x0ecf6
serves `enqueue_cmd`. The resource-pool functions: `resrc_alloc_seg_buf` 0xe762, `resrc_check_avail` 0xe83a,
`resrc_get_buf_end` 0xe926, `resrc_get_seg_start` 0xe9e4, `resrc_get_seg_end` 0xea38, `resrc_update_rd_ptr` 0xea8c,
`resrc_backup` 0xeb2c, `resrc_restore` 0xeb88, `resrc_update_seg_info` 0xebea. The report queue — the firmware → host
result channel: `report_queue_init` 0xecae, `_count` 0xecf6, `_is_full` 0xed6c, `_is_empty` 0xedd4, `_get_buffer`
0xee34, `_read` 0xeed2, `_get_start` 0xef48, `_get_start_idx` 0xefa8, `_write` 0xf016 (unused: 0xf3b2, 0xf414);
`notify_host_state` 0xda7a in the scheduler posts results to the host with it [R names, I role].

### 5.6 Encoder helpers [R unless marked]

- Bit writers of the HEVC encoder (also used by the shared header code): `enc_put_bits(value, n)` 0x2bfa0 (91 calls),
  `enc_put_ue` 0x2c01e (exp-Golomb: n = bit length of v+1, `put_bits(0, n−1)`, `put_bits(v+1, n)`), `enc_put_flag`
  0x2c8b4, `enc_put_u(n, v)` 0x2c8c2. The AVC encoder has its own pair of the same shape, `avc_put_bits` 0x3dc52 and
  `avc_put_ue` 0x3dcd0 [I].
- The rate-control model `GradIQP_*` (`lookup_qp`, `init_rq_model`, `calc_qp_from_bpp`, `calc_bpp_from_qp`,
  `update_cond`) uses the single-float set of §5.4.
- The encoder set-up routines write the VCORE window (§7.3, §7.6): HEVC `init_pic@366e0` and 0x36c86, AVC
  `init_pic@43ef8` and 0x446f2.

## 6. Host interface — firmware side [R unless marked]

The host's half — which host functions write which register with which value, the host interrupt handler and the
instance-slot decode — is `mhal-vcodec.md` §3, §4. Register names are the mainline `wave5-regdefine.h` ones [S];
command and option names the mainline enums [S]. Accesses were traced by following the window pointer through each handler (the tracer does not merge paths, so a register
reached only through a path it does not follow can be missing; every row below was also read in the disassembly).

### 6.1 The W5 host registers as the VCPU sees them

| VCPU address | host offset | what the firmware does there |
|---|---|---|
| 0xFFFF3000 + k, k < 0x100 | the W5 control block 0x000–0x0FF at the same k | 0x3070 written 0 when a command is finished (`W5_VPU_BUSY_STATUS`); 0x304C written with the reason (`VINT_REASON`, §6.4); 0x3048 read and ANDed with the reason first (`VINT_ENABLE`); 0x3040 written 1 on entry of the host-command interrupt (`HINT_CLEAR`); 0x305C written 0xFFFFFF by the reset code (`VPU_CLK_MASK`); 0x3050 / 0x3054 (`RESET_REQ` / `RESET_STATUS`, §7.7); W5 0x94 and 0xa8 read by `vcore_init` (§7.2) |
| **0xFFFF7000 + (off − 0x100)** | the command window 0x100–0x1FF | 0xFFFF7000 = `W5_COMMAND` (0x100), compared with 1 / 2 by the reset code (`INIT_VPU` → clear bss, `WAKEUP_VPU` → keep it, §5.3) and dispatched on; 0xFFFF7014 (`W5_CODE_SIZE` 0x114) loaded as the initial stack top; 0xFFFF7008 / 0xFFFF700C written with 1 / 0 and the fail code (`RET_SUCCESS` 0x108 / `RET_FAIL_REASON` 0x10C) by 0x67ae; 0xFFFF7010 low 16 bits = instance index (`CMD_INSTANCE_INFO` 0x110) in `get_instance` 0x4580 |
| 0xFFFF7090–0xFFFF70FC | window 0x190–0x1FC (`W5_RET_*` area and the instance-report registers) | the exception entry copies its cause word to 0x190 and the 24 saved registers to 0x194… (crash dump for the host); 0x1E4 / 0x1E8 / 0x1EC / 0x1F0 / 0x1FC cleared at start-up and filled by the interrupt raiser (§6.4) |

The command window is reached through a pointer: the host-command interrupt 0x4c24 calls the dispatcher 0x435c with r8 =
r9 = 0xFFFF7000, and handlers get it as an argument — so no constant 0xFFFF71xx occurs in the code.

### 6.2 Host-command interrupt and the two dispatchers

The host writes the command code to `W5_COMMAND` and the parameters to the command window, then rings
`W5_VPU_HOST_INT_REQ` [S: Wave5 protocol]; the VCPU takes it on vector 35 [I]. `0x4c24` (vector 35) writes
`HINT_CLEAR` = 1 and 0x23 to the VCPU-local 0xFFFF1000, then calls 0x435c. The immediate commands — `0x435c` switches on
the command word (window +0x00):

| code | name [S] | handler | registers read (host offsets) / written |
|---|---|---|---|
| 1, 2 | INIT_VPU, WAKEUP_VPU | 0x67ae only (acknowledge: `RET_FAIL_REASON` = 0, `RET_SUCCESS` = 1) | the parameters are read at start-up instead (§6.3) |
| 4 | SLEEP_VPU | 0x5bd0 | none read; idle: sets the wake mask to 0x00000000FFFFFFFF (§6.4), calls `delay` 0x155e with 100 000, acknowledges, raises reason 0x4, BUSY = 0, disables interrupts and halts; busy: fail code 0x1000, raises 0x4, BUSY = 0 |
| 8 | CREATE_INSTANCE | 0x4eb0 | 0x110, 0x114, 0x118 |
| 0x10 | FLUSH_INSTANCE | 0x55a0 | 0x100, 0x104, 0x110; writes 0x1E0 |
| 0x20 | DESTROY_INSTANCE | 0x5648 | 0x100, 0x110 |
| 0x80 | SET_FB | 0x51fe → 0x646e | 0x104, 0x110, 0x118–0x120, 0x128–0x150, 0x1A4–0x1B8, 0x1D4–0x1E0 (one base + stride per pool — the ring-reference frame buffers) |
| 0x800 | — (not in the mainline enum; meaning not known) | 0x5520 | 0x110, 0x11C, 0x120 |
| 0x4000 | QUERY | 0x5af8, switch on 0x104 | per option, below |
| 0x8000 | UPDATE_BS | 0x53d4 | 0x110, 0x118, 0x11C, 0x120 |
| other (0x40 INIT_SEQ, 0x100 DEC / ENC_PIC, 0x200 ENC_SET_PARAM, …) | queued | `enqueue_cmd` 0x5804 | 0x100, 0x104 (option bits 0x340 / 0x2000 tested), 0x110, 0x11C, 0x120, 0x144; writes 0x1E0 = `(instance word +0xe4 << 16) \| (report_queue_count(queue at instance + 0x29f0) & 0xffff)` — the layout of mainline `W5_RET_QUEUE_STATUS` [S], field roles [I] |

QUERY options (0x104) [names S: mainline `enum query_opt`]: 0 GET_VPU_INFO → 0x5cf6; 1 SET_WRITE_PROT → 0x5d8a (reads
0x118–0x120); 2 GET_RESULT → 0x60c2; 3 UPDATE_DISP_FLAG → 0x62d6 (reads 0x118, 0x11C; writes 0x17C); 4 GET_BW_REPORT →
0x6406; 5 GET_BS_RD_PTR → 0x6356; 6 GET_BS_WR_PTR → 0x638c; 7 GET_SRC_BUF_FLAG → 0x6268 (writes 0x12C, 0x18C,
0x1B0–0x1BC); 8 SET_BS_RD_PTR → 0x6372; options outside the mainline enum: **9** → 0x633c (an instance call 0xdeb8),
**0x40** → 0x5dc4 (a GET_RESULT variant), **0x41** → 0x5e7c (copies 0x150–0x178 into the instance object at +0x90),
**0x60** → `query_vcpu_status` 0x6026 (reads 0x10C, 0x1E8; prints `VINT_REASON` at 0x607c), **0x63** →
`query_flush_cache` 0x644a; anything else → fail code 0x8000. Every option ends with the acknowledge and reason 0x4000
(instance −1).

- **Queued commands**: `enqueue_cmd` copies the command into a queue entry (0x4412 copies window words 5…n), and the
  handlers read it later from memory. Which window offset each field of INIT_SEQ, ENC_SET_PARAM and DEC / ENC_PIC comes
  from is not known; mainline defines, at offsets this document meets elsewhere, 0x140 `W5_CMD_ENC_SEQ_INDEPENDENT_SLICE`,
  0x154 `W5_CMD_ENC_SEQ_RC_TARGET_RATE`, 0x164 `W5_CMD_ENC_SEQ_RC_BIT_RATIO_LAYER_4_7` (ENC_SET_PARAM), 0x154
  `W5_CMD_ENC_PIC_SRC_STRIDE`, 0x164 `W5_CMD_ENC_PIC_CODE_OPTION` (ENC_PIC), 0x140 `RET_QUERY_BW_PROC_AXI_READ` (DEC_QUERY)
  [S].
- `enqueue_cmd` (0x59fe), `dump_inst_and_cmd_status` 0x5e96 and the scheduler (0xace6) also read the wake mask (§6.4).
- The command-code set the dispatchers accept is the table above; **0x300 is accepted nowhere** — neither dispatcher
  takes it and the interrupt raiser rejects it as a reason. 0x300 is mhal's own `Wave5VpuEncGiveCommand` selector (a
  host-library `CodecCommand` value outside the Chips&Media enum), never written to `W5_COMMAND` (`mhal-vcodec.md` §4.3).

### 6.3 Start-up reads: the init block

`main` (0x4a3a) calls `vcpu_init` 0x3f54 with the window base; it reads the parameters the host's `Wave5VpuInit` wrote
(host values: `mhal-vcodec.md` §4.2) and stores them in globals:

| host reg | host value | what the firmware does with it |
|---|---|---|
| 0x12C | firmware-UART divisor in bits 31:16 | `>> 16`, passed with 38 400 (0x9600) baud to `uart_init` 0x3a68 |
| 0x11C, 0x120 | temp buffer base / size | globals 0x605c4 / 0x605c8; a size below 0x1d000 aborts with "temp buffer size: 0x%x is too small" |
| 0x124, 0x128 | SRAM base / size | globals 0x605cc / 0x605d0 |
| 0x134, 0x138 | VLC / CINFO sizes | carve-outs of the memory layout (rounded to 4 KiB, ×2) when their feature bits are set |
| 0x13C | feature bits (host: 0x10) | bit 0 → flag 0x608e5, bit 1 → 0x608e4, bit 2 → 0x60911, bit 3 → 0x60910, bit 4 → 0x60934; each selects a layout branch |
| 0x158, 0x15C | temp buffer | 0x158: base of two 32 KiB slots (loop 0x409a) and of 512 KiB-stepped slots after them (loop 0x40f8); 0x15C: top of a region from which the VLC / CINFO carve-outs are subtracted [R], roles [I] |
| **0x140** | 3 | **number of decoder task buffers**: must be ≤ 16, else `RET_FAIL_REASON` = 0x40000 and `RET_SUCCESS` = 0 |
| **0x144** | decoder task memory base | first task buffer; with 0x140 and 0x154 the firmware builds the table `base + i·size`, i < n (loop 0x420c), and hands n, the table and the size to 0xa136 |
| 0x148, 0x14C | ramlog base / size | log buffer set-up when the size is not 0 (0x42fc) |
| 0x150 | 0x1000-block base | global 0x6d744; the firmware writes its clock value (global 0x60340) into the block's first word |
| **0x154** | 0x18000 | **size of one decoder task buffer (96 KiB)** |
| 0x160 | VCPU clock in 100 kHz | global 0x60340; values ≤ 0x4d1 replaced by 0x4d2 |
| **0x164** | 3 | **mode word**, global 0x60344, compared only ever with **4**: when 4 the firmware also reads 0x168 (byte, flag 0x60950) and skips the tick timestamps and some report fields in the encoder task paths (0x35c04 in `hevc_enc_vcore_run_task`, 0x438a0, 0x390ba `enc_pic_in`), SET_FB (0x64f2) and `sched_done_pic` (0xbd3c). The host's 3 takes the ordinary path. What 4 stands for has no string or name in the image and is not known [I: a low-overhead / special-product mode] |
| 0x168 | not written by the host | read only in mode 4; meaning not known |

- 0x140–0x160 (with 0x148 / 0x14C / 0x150) are read only when the command word is `INIT_VPU` (the test at 0x417c); on
  `WAKEUP_VPU` that part is skipped and the task-buffer table is kept. `vcpu_init` ends with `RET_SUCCESS` = 1.
- In this firmware's INIT_VPU layout the mainline `W5_CMD_INIT_NUM_TASK_BUF` / `_ADDR_TASK_BUF0` / `_TASK_BUF_SIZE`
  (0x134 / 0x138 / 0x178 in mainline [S]) sit at 0x140 / 0x144 / 0x154 [R use, S mainline offsets].
- In other commands: SET_FB reads 0x140 as a pool register (with 0x134–0x150); QUERY 0x41 copies 0x154 and 0x164 (with
  0x150–0x178) into the instance object.
- `vcore_init`, called from `main`, also reads 0x110 and 0x118 (§7.2).

### 6.4 The interrupt raiser; reasons 11 and 12

`0x482c(instance, reason)` (`vpu_trigger_intrpt`) is the only writer of `VINT_REASON` (0xFFFF304C) in the code. It accepts
the reasons 1, 2, 4, 8, 0x10, 0x20, 0x40, 0x80, 0x100, 0x200, 0x400, 0x800, 0x1000, 0x2000, 0x4000, 0x8000 (others:
"unexpected irq 0x%x,0x%x" and halt) and, by reason:

| reason | instance register written (host) | then |
|---|---|---|
| 0x40, 0x100, 0x200, **0x1000** | 0x1E8 (`QUEUE_CMD_DONE_INST`) via 0x46b6 / 0x4704 — only if `VINT_ENABLE & reason` and no report is pending (global 0x5ebe4) | `VINT_REASON` = reason; for 0x40 also 0x47cc on the instance |
| **0x800** | 0x1F0 (index 0x3c) via 0x4704 | `VINT_REASON` = reason |
| 0x8000 / 0x400 / 0x2000 | 0x1E4 / 0x1EC / 0x1FC (indexes 0x39 / 0x3b / 0x3f) | `VINT_REASON` = reason |
| 1, 2, 4, 8, 0x10, 0x20, 0x80, 0x4000 | none | `VINT_REASON` = reason (if enabled) |

- 0x4704 packs the instance number into one of the three byte slots of the register and sets the slot's type bit
  (24 / 25 / 26) — the encoding the host decodes (`mhal-vcodec.md` §3.2).
- The reason bits are one per completed command: the stock `W5_VPU_VINT_REASON` enum (0x4C) assigns INIT_VPU = 0,
  WAKEUP = 1, SLEEP = 2, CREATE_INSTANCE = 3, FLUSH = 4, DESTROY = 5, INIT_SEQ = 6, SET_FRAMEBUF = 7, DEC_PIC / ENC_PIC = 8,
  ENC_SET_PARAM = 9, DEC_QUERY = 14, BSBUF_EMPTY / FULL = 15 [S, `wave5-vpuapi.h`]. **Bits 11 and 12 are not in it**
  [R]: they are SigmaStar / build additions.
- **Bit 11 (0x800)** is raised from three places: `sched_done_pic` 0xbaaa (at 0xbfa0, after a picture's per-slice
  counter `+0x6d5` is incremented; instance from the task's first halfword) and two twin routines 0x188b2 / 0x2663c (called from 0x1c092
  and 0x24dcc), which, when a flag at `+0x210` is set, store 1, a bitstream pointer (`+0x1ac` resp. `+0x1c8`) and a
  second word (`+0x10c`) into the report record (+0x5c / +0x68 / +0x6c) and call 0x4998 = `0x482c(inst, 0x800)`. A
  partial (slice-level) output report with its bitstream position [R mechanics, I name]; the host reads the instance from
  0x1F0.
- **Bit 12 (0x1000)** is raised from one place: 0x61c0 in the QUERY GET_RESULT handler 0x60c2. For the finished report of
  instance i it calls 0xdf9e, which returns the 64-bit **wake mask** (global 0x6d858; printed by the debug line
  `stage=0x%x, wake=0x%llx hangstage=0x%x`); if bit i is set it raises 0x1000 **instead of** the report's own reason
  (stored at report +4), else the own reason. The mask is set to 0x00000000FFFFFFFF by the SLEEP handler 0x5bd0 before
  the core halts and cleared to 0 at start-up (`main` 0x4c1e) and in 0xa332. So bit 12 marks the first completion
  reported to an instance that was pending across a SLEEP / WAKEUP cycle [R mechanics, I meaning]; the firmware gives it
  no name (the host driver's reading, "source buffer released", is `mhal-vcodec.md` §3.2).
- How much of an encoded frame's time the firmware takes on the live H.264 stream, against the host's share, is not known
  from the image; the running camera's `/proc/mhe/ctx_info` shows the split (`mhal-vcodec.md` §2.8).

## 7. The VCORE and its four microcode segments

The VCPU drives one VCORE, the codec's engine processor, through a register window of its own; the engine runs
microcode from the four segments in the image. One segment per codec and direction: the mode value each VCPU codec module
writes is [R]; the mode → segment pairing rests on two readings that agree — the window registers each segment's code
touches match the registers each VCPU module writes and reads [R facts, I conclusion], and the segments' common boot code
picks one of four 0x2000 offsets by the mode [R constants, I pairing] (§7.4).

### 7.1 Segments [R unless marked]

| segment | file range | code part (segment offset; file range) | data part | mode (VCORE +0x104) | serves |
|---|---|---|---|---|---|
| 0 | 0x4e940–0x50940 | 0x0000–0x144c; 0x4e940–0x4fd8c | 0x1800–0x1e00 (file 0x50140): scaler filter table (§7.5) | 0 | **HEVC decode** [I] |
| 1 | 0x50940–0x52940 | 0x0000–0x0ede; 0x50940–0x5181e | 0x1800–0x1e00 (file 0x52140): the same table, byte-identical | 2 | **AVC decode** [I] |
| 2 | 0x52940–0x54940 | 0x0000–0x1164; 0x52940–0x53aa4 | none | 1 | **HEVC encode** [I] |
| 3 | 0x54940–0x56940 | 0x0000–0x0cae; 0x54940–0x555ee | none | 3 | **AVC encode** [I] |

- The rest of each segment is the fill halfword **0xE190**, which never occurs in VCPU code — the segments are in a
  different instruction set from the VCPU's. Their vector slots are not VCPU prologues either.
- The two decoders (0 / 1) share long byte-identical stretches at equal offsets (e.g. 0x4ece6 / 0x50ce6), and so do the
  two encoders (2 / 3).
- There is no segment for a second VCORE (§1).

### 7.2 How the VCPU hands the segments to the VCORE [R]
<!-- ev:chagall-fw.7.2 -->

- The VCORE window is seen by the VCPU at **0xFFFF8000**, through the per-core pointer array at global 0x5f0f4 (initial
  value 0xffff8000; code indexes it `core*4 + 0x5f0f4`). It is laid out like the W5 host window: control words at
  +0x000–+0x03c, a parameter area +0x100–+0x1f8, a command word at +0x1fc (§7.6).
- `vcore_init` (0x6a8a) is called by `main` (call at 0x4a70) with r8 = 0xFFFF7000, the host command window, and by
  `vcore_reset` and `vcore_soft_reset` (calls at 0x6c6e, 0x6d10) with r8 = 0. With a window it stores two host registers in
  globals: `0x60988 = [window+0x10] + 0x4e940` and `0x6098c = [window+0x18]` — host offsets **0x110 `W5_ADDR_CODE_BASE`**
  and **0x118 `W5_CODE_PARAM`** [S: mainline names], which mhal's `Wave5VpuInit` writes as the code region's physical
  base (`dev_info+0x8c`, the same value as `REMAP_PADDR`) and 0 [R]. So 0x60988
  is **the bus address of segment 0 in DRAM** (code base + 0x4e940); the VCPU never copies the segments itself.
- It then reads W5 control registers 0x94 (`W5_VPU_RET_PRODUCT_VERSION` [S]) and 0xa8 (no name in the mainline or
  StarFive headers): flag r1 = 1 when 0x94 = 0x5150 and 0xa8 ≤ 0x23700, else 0. What the flag tells the engine is not
  known.
- **Boot**: if VCORE +0x18 is not 0, it prints `[FW]++%s this shouldn't happen PC=0x%x` with that value (+0x18 = the
  engine's program counter, non-zero = running) and returns. Else: +0x30 = 1, **+0x10c = 0x60988** (segment-area base),
  +0x110 = 0x6098c (`CODE_PARAM`), +0x114 = flag r1, +0x1c = 0x8002, +0x104 = 0, +0x1fc = 0, +0x0c = 1; then a loop writes
  `+0x04 = (i << 16) | halfword[0x4e940 + 2i]` for i = 0…0x1ff — **the first 0x400 bytes of segment 0 pushed halfword
  by halfword into the engine's instruction memory**; then +0x00 = 1 (start), and `vcore_wait_idle(0, −1)` waits for
  +0x30 to drop to 0 (`vcore_wait_idle(core, n)` 0x6e5c: polls +0x30 with a 1000-unit `delay`, up to n tries; returns 1
  when idle).
- Nothing else in the code reads 0x60988 or forms an address inside 0x4e940–0x56940: a full scan of LERI-built
  constants finds 0x4e940 at 0x6aa8 (`add r8, 0x4e940`) and 0x6b3a only, the other values in that numeric range being load / store offsets
  and compare operands (e.g. `cmp r2, 0x54600` at 0x21892), and the four data words in that range (0x59120, 0x593ec,
  0x59454, 0x59660) are entries of halfword-pair number tables. So the other 31 KiB reach the engine only through the
  base in +0x10c.

### 7.3 The mode word +0x104: who writes which value [R]

| VCPU module (address range, identified by its strings) | writer | +0x104 | with it |
|---|---|---|---|
| HEVC decoder (`hevc_vcore_*`, 0x1a4b0–0x1cb80) | `set_pic_param` 0x1b44c (store at 0x1b510); `set_core_param` 0x1b646 (0x1b728, value computed, 0 on that path) | **0** | +0x3c = 0, then +0x30 = 1 and command +0x1fc = 0x10 (0x1b60c) |
| HEVC encoder (`hevc_enc_*`) | `init_pic@366e0` (0x36a10) | **1** | +0x100 = `(halfword[r6] << 16) \| 1` (0x36a08), +0x1b8 = 0 |
| AVC decoder (strings `sps::`, `pps::`, `sh::`, `avc_dec_handle_t size`; 0x1cb80–0x26e1e) | 0x2592a (0x25950) | **2** | the slice / picture parameters +0x10c…+0x1a8, +0x30 = 1, command +0x1fc = 0x10 (0x25a8e) |
| AVC encoder (strings `avc_enc handle size`, `vlc buffer is not enough…`; 0x394e2–0x46b14) | `init_pic@43ef8` (0x4412c) | **3** | +0x100 = `(halfword[r11] << 16) \| 3` (0x44122), +0x1b8 = 0 |
| boot | `vcore_init` 0x6a8a | 0 | before the first start |

The low half of +0x100 repeats the mode for the two encoders (1, 3); the decoders do not write +0x100 (full scan).

### 7.4 The segments' boot code selects the segment [R bytes, I semantics]
<!-- ev:chagall-fw.7.4 -->

The segment instruction set is not decoded and not public; what follows uses only facts that need no instruction
semantics, plus one inferred pairing.

- Every segment starts with the same 16 two-halfword vector slots: slot 0 `e40e 0020`, slot 2 `e40e 0092`, slot 4
  `e40e 0160` (decoders) / `e40e 00fa` (encoders), the others `e470 e190` (as bytes: `0ee4 2000`, `70e4 90e1`, …); code
  begins at byte 0x40 = halfword 0x20, the slot-0 operand. Bytes 0x000–0x197 are identical in all four segments except
  three (0x2a–0x2b, the slot-4 operand; 0x7a and 0x108, branch operands); the two decoders then stay identical to 0x1a0,
  the two encoders to 0x240. So every segment carries the same boot / dispatch prologue, and the first 0x400 bytes of
  segment 0 that the VCPU pushes are enough to hold it [R sizes, I purpose].
- Halfword pairs `e0cN pppp` occur with `pppp` = window offset / 4 — at the boot code `e0c2 007f` (0x1fc, the command
  word) and `e0c0 0043` / `0044` / `0045` (0x10c / 0x110 / 0x114, exactly the three words `vcore_init` sets before the
  start). So `e0cN` is the engine's access to its own window and `pppp` a word index [I, strong: the indexes hit the
  registers the VCPU prepares, and nothing else]. `e0c2` covers 0x030 (the busy word the VCPU sets to 1 and polls for 0)
  and the result words the VCPU reads after a run, so it is the engine's write [I].
- At segment bytes 0x84–0xe1 (identical in all four) one block occurs four times: `e005 <X>`, `a2nn`, `3c16`, `e0c0 0041`
  (window 0x104 = the mode), `e008 ffff`, `a1nn`, `e40a 0071`, with (X, nn) = (0x0000, 0x00), (0x2000, 0x04), (0x4000,
  0x02), (0x6000, 0x06), and a final `e005 0000` — a four-way choice on the mode's low 16 bits between the four segment
  offsets [I]. Read as "load X, compare the mode with nn>>1, branch out if equal", with no branch delay slot, it gives
  mode 0 → 0x0000, 2 → 0x2000, 1 → 0x4000, 3 → 0x6000 — the table of §7.1 [I: the immediate field's scaling and the
  absence of a delay slot are assumptions; the result-word match below decides between the readings].
- Window words each segment's code addresses (`e0cN pppp` pairs, word index × 4) [R]:

| segment | writes (`e0c2`) | VCPU readers of those words [R] |
|---|---|---|
| 0 | 0x030, 0x1a0, 0x1ec, 0x1f0, 0x1f8, 0x1fc, 0x300–0x308 | 0x1ec: `hevc_vcore_irq` 0x1ac62 (HEVC decoder) and 0x261a8 (AVC decoder) |
| 1 | 0x030, 0x1a0, 0x1ec, 0x1f8, 0x1fc, 0x300–0x308 | the same |
| 2 | 0x030, 0x0c0–0x0c8, 0x1b0, 0x1b4, 0x1b8, 0x1bc, 0x1c0, 0x1c4, 0x1c8, 0x1d8, 0x1dc, 0x1e0, 0x1e4, 0x1e8–0x1fc, 0x300–0x308 | 0x37e1a (called by `hevc_enc_vcore_done_pic`) reads 0x1a0–0x1a8, 0x1b0–0x1c8, 0x1d8–0x1e4 |
| 3 | 0x030, 0x0c0–0x0c8, 0x1b8, 0x1d4, 0x1d8, 0x1dc, 0x1e0, 0x1e8–0x1fc, 0x300–0x308 | AVC encoder: 0x439be reads 0x1b8, 0x452ea reads 0x1d4–0x1dc |

- The encoder pairing is decided by the words only one segment writes [R]: the HEVC encoder's reader takes 0x1b0, 0x1b4,
  0x1bc, 0x1c4, 0x1c8 and 0x1e4, written by segment 2 and not by segment 3; the AVC encoder's reader takes 0x1d4, written
  by segment 3 and not by segment 2 (0x1a0–0x1a8 and 0x1c0 are also written by the VCPU's own encoder set-up 0x36c86 /
  0x446f2, so they do not discriminate). Segments 0 / 1 write no result words beyond 0x1a0 / 0x1ec / 0x1f8 and read the
  decoder parameter block 0x110–0x18c that `set_core_param` / `set_pic_param` (HEVC) and 0x2592a / 0x25aa0 / 0x25dee /
  0x25ed2 (AVC) fill. Of the two readings of the boot block, only "nn>>1, no delay slot" sends HEVC encode (mode 1) to
  0x4000 = segment 2 and AVC encode (mode 3) to 0x6000 = segment 3; the same reading sends HEVC decode (mode 0) to
  segment 0 and AVC decode (mode 2) to segment 1 [I, resting on the encoder match].
- Segment 0 reads 0x17c / 0x180 through `e0c0`, two words only the HEVC decoder's `set_pic_param` writes, while segment
  1 reads them only through `e0c1`, like 0x184 [R]; this does not separate the decoders on its own.

### 7.5 The decoder segments' data table: scaler filter coefficients [R layout, I purpose]
<!-- ev:chagall-fw.7.5 -->

The table exists twice — at 0x50140 (segment 0 + 0x1800) and 0x52140 (segment 1 + 0x1800), 0x600 bytes each,
byte-identical; segments 2 and 3 have none (their 0x1800–0x1e00 is fill). Read as signed bytes, it is a set of filter
kernels that each sum to 128 (every kernel checked):

- 24 rows of 64 bytes; bytes 54–63 of every row are 0.
- Rows 0–11: nine 6-tap kernels each (9 × 6 = 54 bytes). Rows 12–23: nine 4-tap kernels each (36 bytes); row 12 adds
  nine 2-tap kernels in bytes 36–53: (1, 127), (8, 120), (16, 112), … (64, 64) — linear-interpolation weights.
- The ninth kernel of every row is symmetric (row 0: `21 5 38 38 5 21`; row 11: `-3 0 67 67 0 -3`; row 19: `63 1 1 63`),
  so the nine kernels are phases 0/16 … 8/16 of a polyphase interpolator, the other half mirrored [I].
- Within each group of 12 rows the kernels go, with small steps back, from wide and smooth (row 0, phase 0:
  `30 15 30 38 0 15`) to nearly pass-through (row 11, phase 0: `17 -2 18 97 0 -2`): twelve filter strengths, the usual set
  for a resampler chosen by scale ratio [I]; 6-tap for one plane class and 4-tap for the other (luma / chroma) [I].
- Purpose [I]: coefficient sets for the decoder's output down-scaler. The Wave5 configuration word carries a scaler
  ("`conf_scaler_en` [07]" in `W5_VPU_RET_VPU_CONFIG0` [S: mainline `wave5-regdefine.h`]), mhal's decoder exposes an output
  scale setting (scale {enable, W, H}, mhal VDEC SetParam `0x8002000a`) and the decode command sends linear / scaled
  output descriptors (`Wave5VpuDecode`) (`mhal-vcodec.md` §5.3, §8.3); both decoders carry the same table because the scaler sits
  after reconstruction, independent of the standard. How the segment code moves the table into the hardware lies in the
  undecoded engine code and is not known.

### 7.6 The VCORE window at 0xFFFF8000

Writers and readers from the traced accesses (VCPU) and the segment ports [R]; the role column is [I] unless marked.
(The scan hit "0xffff8264 written by 0x4580" is not an access to this window: `add r0, 0xffff8180` at 0x45b0 subtracts
0x7e80 [R].)

| offset | VCPU | segments | role |
|---|---|---|---|
| +0x000 | W 1 by `vcore_init` after the push | — | start the engine |
| +0x004 | W `(i << 16) \| halfword` × 0x200 by `vcore_init` | — | instruction-memory write port, address in the high half [R form] |
| +0x008 | W = [+0x14] in `vcore_reset` 0x6b9c, `vcore_soft_reset` 0x6c9c and `vcore_irq` 0x6d1e | — | interrupt clear (write back the pending bits) |
| +0x00c | W 1 by `vcore_init`, 0x6b9c, 0x6c9c, 0x6d1e | — | interrupt enable / re-arm |
| +0x014 | R in the same three | read (`e0c1`, `e0c3`) | interrupt status (pending bits) |
| +0x018 | R by `vcore_init` before the push | — | engine program counter: non-zero = running [R: printed as `PC=0x%x`] |
| +0x01c | W 0x8002 by `vcore_init` | read (`e0c1`) | boot configuration word |
| +0x030 | W 1 before each command (`vcore_init`, 0x1b44c, 0x1b646, 0x2592a, 0x25ed2, 0x36c86, 0x446f2); R polled until 0 by 0x6e5c / 0x6e94 | written (`e0c2`) | busy flag: set by the VCPU, cleared by the engine |
| +0x03c | W 0 (decoders 0x1b44c, 0x25aa0), 1 (encoders 0x36c86, 0x446f2) | — | per-run flag set by direction; meaning not known |
| +0x0c0–+0x0c8, +0x0d0 | — | encoders only: write 0x0c0–0x0c8, read 0x0d0 | not seen by the VCPU; meaning not known |
| +0x100 | W `(id << 16) \| 1` / `\| 3` by the two encoders' `init_pic` | — | encoder job word, low half = mode |
| +0x104 | W 0 / 1 / 2 / 3 (§7.3) | read (`e0c0`), boot block | **mode = segment selector** |
| +0x10c | W segment-area base by `vcore_init`; later 2 with command 0x8000 (0x1b10a, 0x255e8), 6 with command 2 (0x1b646, 0x25ed2), a picture value by 0x1b44c / 0x2592a | read | first parameter word |
| +0x110–+0x19c | W `CODE_PARAM` (+0x110) and the product flag (+0x114) by `vcore_init` (§7.2); later by the decoders' parameter set-up and the encoders' 0x36c86 / 0x446f2 | read | command parameters |
| +0x1a0–+0x1e8 | encoders: W some (0x1a0, 0x1c0, 0x1c4, 0x1cc–0x1e8 by 0x36c86 / 0x446f2), R results (0x37e1a, 0x452ea, 0x439be; 0xd9c6 reads 0x1e0) | written (results) | encoder parameters and results |
| +0x1ec | W 0 by 0x1b44c / 0x25aa0; R by `hevc_vcore_irq` 0x1ac62 and 0x261a8 | written by all four | decoder result word |
| +0x1f0 | W 0 by 0x36c86 / 0x446f2; R by `hevc_enc_vcore_irq` 0x37ff4 and 0x44294 | written by 0, 2, 3 | encoder result word |
| +0x1f8 | W 1 with command 0x8000 (0x1b10a, 0x255e8) | written | command argument |
| +0x1fc | **command**: 0 at boot; 0x10 = decoder picture run (0x1b44c, 0x2592a); 2 (0x1b646, 0x25ed2, `hevc_enc_vcore_run_task` 0x359b6, 0x4378a); 0x8000 with +0x10c = 2, +0x1f8 = 1 (0x1b10a from `hevc_vcore_run_task`, 0x255e8 from the AVC decoder) | read at boot (`e0c2 007f` clears it) and later | command word |
| +0x300–+0x308 | — | all four: write and read | not seen by the VCPU; meaning not known |

What the engine does for commands 2 and 0x8000 (beyond the decoder picture run 0x10) is not known.

### 7.7 VCORE interrupt and reset [R]

- The VCORE interrupt reaches the VCPU on vectors 46, 47, 49, 50 and 51, all → `vcore_irq` 0x6d1e. The handler
  acknowledges (+0x08 = +0x14, +0x0c = 1), writes 0x2e to the VCPU-local 0xFFFF1000, stores a time stamp (`get_time`
  0x3a20) in the scheduler object at +0x84 and calls `sched_irq(2, 0)` (0xc526).
- `vcore_reset` 0x6b9c (called from 0x6e18, `hevc_enc_vcore_irq` and 0x44294): writes 7 to 0xFFFFFE10, polls 0xFFFFFE14
  for 0x3f (5000 tries; message `[FW]++%s base=0x%x, …` 0x5a69c on timeout), pulses W5 `RESET_REQ` 0x3050 = 0x151515
  until `RESET_STATUS` 0x3054 is 0 (message `… reset timeout, H_VPU_RESET_STATUS=0x%x` 0x5a65f), clears 0xFFFFFE10,
  acknowledges, writes the word at 0x56944 (0x2e) to 0xFFFF1000, re-boots with `vcore_init(0)` (the stored segment base
  is kept) and sets global 0x5ebe0 = −1.
- `vcore_soft_reset` 0x6c9c (from `sched_done_pic`, `hevc_enc_vcore_run_task`, 0x4378a) is the same without the
  0xFFFFFE10 step, followed by a delay.
- `vcore_wait_idle_forever` 0x6e94 (wait for +0x30 on core 0 without a limit) is never called — unused.
