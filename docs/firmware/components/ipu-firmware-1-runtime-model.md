# ipu-firmware-1-runtime-model — `ipu_firmware.bin` part 1: image, RISC-V runtime, CCIF server, host commands, SGSI model file, option tables, offline command block
<!-- ev:ipu-firmware-1-runtime-model -->

`ipu_firmware.bin` is the program of the RISC-V core inside the NPU ("IPU", "DLA" in the device tree and power names) of
the SigmaStar M6 SoC on this camera. The host side (mhal IPU, mi_ipu.ko, libmi_ipu.so, the host half of the CCIF mailbox)
is `ipu-host.md`; this document is the firmware's own side, part 1:

- **the image** — a raw RV32IMC memory image linked at 0x2000_0000, no container header; regions, globals, source files,
  function inventory, version strings, the other build on hand (§1, §2);
- **the runtime** — start-up, core-local registers, traps, the FreeRTOS port, the two heaps, the log ring (§3);
- **the CCIF server** — the firmware end of the mailbox: registers as the RISC-V sees them, interrupt bits, receive and
  send paths, boot message (§4);
- **the host commands** — the service task and its nine commands: channels, invoke, device parameters, statistics (§5);
- **the SGSI model file** — header, the TFLite-derived model flatbuffer as the firmware reads it (Model, SubGraph,
  Tensor, QuantizationParameters, Operator), DDR types, fast-run model (§6);
- **the option tables** — every operator option type field by field, the shared records (LUT block, requantisation
  records, tile and slot records) and the FlexBuffer custom options (§7);
- **the offline command block** — the precompiled command stream of a model: framing, decoder control words, delta form,
  engine instruction headers and the field layouts of the classes a convolution network uses, the decoder registers that
  run a block (§8).

Part 2, `ipu-firmware-2-engines.md`, holds the interpreter and op resolver, the IPU200M HAL and its compute engines (L2 /
VR buffers, SR register file, conv engine, vector unit, LUT unit), the operator kernels and the diff against the Anjoy
build; this document refers to it by topic where a fact of part 2 is needed.

Tags: **[R]** read from the binary (`ipu_firmware.bin`, its decompile, disassembly, strings), from a file named, or from
an emulation run of the binary's own code (below); **[I]** inference. A heading tagged [R] means every statement under
it is [R] unless marked otherwise. Function names taken from the `__func__` strings of the asserts and log calls inside
a function are [R]; names the binary does not carry (fields, structs, functions without an assert or log string) are
chosen in SigmaStar style and marked [I] where they first appear, or are descriptive names of ours [I] given with the address. Addresses are RISC-V
addresses of the firmware (file offset + 0x2000_0000) unless written "file 0x…".

Binary [R]: `/config/dla/ipu_firmware.bin` (602 632 B = 0x93208, md5 6e1d4c3f97a61bd4b95174f139202b71; on the camera
`/config/dla/ipu_firmware.bin`), build `Q_0.1.0_verified_21110517`. "cmd+0x…" is a byte offset into the command vector
of the Anjoy model's offline flatbuffer (file offset 0xBC040).

**[S]** stated by a source — the public TensorFlow Lite schema v2.1.0 (`schema.fbs`; used only for TFLite field
**names**; field numbers, types, defaults and storage are read from the firmware) and the public M6 headers
`mi_ipu{,_datatype}.h`. "[R: emulation]" means the firmware's own function was run under unicorn (RV32) on synthetic
flatbuffer tables, with a memory hook recording which vtable slot is read with which width and where the value is stored.

Second build and a model for it [R]: the Anjoy MC-H12S firmware (SSC359G, same Mercury6 family) carries an
`ipu_firmware.bin` of 602 280 B (`Q_0.9_verified_21091510`) and the model `sypcna2.4803013_V2.1_fixed.sim_sgsimg.img`
(1 273 856 B, md5 ca15bfefb40f69d90e39ec22497a3d88) — "Anjoy" below.

## 1. What the program is [R unless marked]
<!-- ev:ipu-firmware-1-runtime-model.1 -->

A bare-metal **RV32IMC** program: no F/D, no A, no custom opcodes — a capstone sweep of the 0x7D000-byte code region
decodes everything except two halfwords; floats are soft-float [R]. It is SigmaStar's
"IPU SDK" firmware for the **IPU200M** NPU core:

- FreeRTOS (tasks `IPU_service`, `IDLE`, `Tmr Svc`, queue `TmrQ`) (§3.4);
- a CCIF mailbox server (`drv/ccif/drv_ccif_server.c`) (§4);
- a host-command layer (`src/net_mt/ipu_api.c`) (§5);
- a TensorFlow-Lite-derived model parser and interpreter (`model_parser.c`, `interpreter.c`, `op_resolve.c`,
  flatbuffers / flexbuffers readers, schema `sgs_schema_reader.h`) (§6, §7; interpreter: `ipu-firmware-2-engines.md` §2);
- the operator kernels with their HAL (`SDK/OPS_IMPL/…`, `Driver/hal/IPU200M/…`: DMA, CMDQ, BDMA, convolution, SR, LUT,
  vector processor "vp1", L2 memory) (`ipu-firmware-2-engines.md`).

Origin: strings and `__FILE__` paths; build tree `…/ALL--IPU--SH--ReleaseBuild/ipu/Project/SRC/SDK/…`.
The NPU's compute engines are driven only through memory-mapped registers and command queues seen by the RISC-V at
0x0100_0000 — the RISC-V core itself only parses, schedules and runs the "invoke by cpu" parts of a network [I: from the
instruction mix and the HAL strings].

Status in the product [R: the host stack, `ipu-host.md` §1]: unused by prog_uvc and kjd_lvgl; loaded only by libmi_ipu on the first
`MI_IPU_CreateDevice` (`ipu-host.md` §5.2); on 2026-10-18 it ran a person / vehicle model on this camera
(`ipu-host.md` §6).

## 2. The image

### 2.1 No container [R]
The file is a raw memory image of the RISC-V program linked at **0x2000_0000**: file offset = link address − 0x2000_0000.
No magic, header, length field, checksum or signature — offset 0 is the first instruction; libmi_ipu reads the file
whole into memory and nothing on the host parses it (`ipu-host.md` §5.2). The firmware runs where it is loaded: there is
no `.data` copy, the entry code only zeroes `.bss` (§3.1).

The host hard-codes the same split (`ipu-host.md` §2.4): code region 0x2000_0000 + **0x7D000**, data region
0x2007_D000 + **0x70800** (ends 0x200E_D800 = the stack top), so the memory the firmware needs is **0xED800 B (972 800
B)**; the file must not be larger ("firmware runtime size is small than binary size", libmi_ipu).

### 2.2 Regions [R unless marked]

| RISC-V address | file offset | size | content |
|---|---|---|---|
| 0x2000_0000 | 0x00000 | | reset entry (§3.1) |
| 0x2000_0000–0x2007_9F9F | 0x00000–0x79F9F | 499 616 B | code (the code window is file 0x00000–0x7CFFF); 1184 functions found by Ghidra (429 KB inside function bodies) |
| 0x2007_9FA0–0x2007_CFFF | | | zero padding to the end of the 0x7D000-byte code window (`.text`) |
| 0x2007_D000–≈0x2009_19FB | 0x7D000– | ≈84 KB | read-only data, starting with the trap-dump strings "Core Status---…": trap-dump and log strings, assert strings, op-name / option-name tables, flatbuffers vtable offsets, LUT and constant tables [I boundary: first writable global found at 0x2009_19FC] |
| ≈0x2009_19FC–0x2009_3207 | –0x93207 | ≈6 KB | initialised writable data (heap bounds, pitch alignments, newlib `_impure_ptr` / reent, pointer literals near `gp`) [I: from what the code writes] |
| 0x2009_3208–0x200E_69FF | not in the file | 0x537F8 | `.bss`, zeroed at start |
| 0x200E_6A00–0x200E_D7FF | not in the file | 0x6E00 | start-up / interrupt stack (`sp` = 0x200E_D800 at entry); FreeRTOS tasks have their own stacks from the FreeRTOS heap |
| `gp` = 0x2009_3770 | | | small-data base; `gp`-relative globals span 0x2009_2F70–0x2009_3F6F |

### 2.3 Selected globals [R addresses, I names]

| address | size | object |
|---|---|---|
| 0x2009_3320 | 0xE3F8 (58 360 B) | FreeRTOS `heap_4` arena `ucHeap` (task stacks, queues, TCBs, CCIF server record) (§3.5) |
| 0x200A_1B3C | 48 × 100 B | channel table (§5.2) |
| 0x200A_1860 / 0x200A_1954 | 61 pointers / 61 × 8 B | invoke input-tensor address list (§5.4) |
| 0x200A_2DFC | 3 words | log ring: buffer, data start, size (§3.6) |
| 0x200A_2E08 | 0x3898 B | statistics record copied out by cmd 5 (§5.6) |
| 0x200A_7184 / 0x200A_7188 | words | SGS allocator peak / current bytes (§5.6) |
| 0x200A_916C | 13 B (+ valid flag 0x200A_9178) | DMA address map {start, end, offset, valid} (§5.5) |
| 0x2009_32B8 / 0x2009_32BC | words | "malloc region" end / start of cmd 3 (§3.5) |
| 0x2009_32C0 / 0x2009_32C4 | pointers | CCIF server record / CCIF hardware record (§4.1) |
| 0x2009_32CC | word | log mode (§3.6) |
| 0x2009_32D0 | pointer | op resolver (`ipu-firmware-2-engines.md` §2.1) |
| 0x2009_32E0 | table | external-interrupt handlers (§3.3) |
| 0x2009_324C | byte | interrupt-nesting enable (§3.3) |
| 0x2009_2F70 | word | critical-section nesting counter (§3.4) |
| 0x2009_19FC / 0x2009_1A00 | words | big-heap end / start (from the host, §3.1) |
| 0x2009_1A04 / 0x2009_1A08 / 0x2009_1A0C | words | pitch alignments XRGB W / YUV420 W / YUV420 H (§5.5) |
| 0x2000_06F0 / 0x2000_06F4 / 0x2000_06F8 | words in a padding gap of the code window, before the trap vector 0x2000_06FC | `IPU_service` stack base / stack top / TCB (§3.4) |

### 2.4 Source files [R: `__FILE__` strings, build-tree prefix dropped]
94 file names. Grouped:
- runtime / host: `drv/ccif/drv_ccif_server.c`, `src/net_mt/ipu_api.c` (FreeRTOS and newlib carry no file strings);
- model and interpreter: `model_parser.c`, `interpreter.c`, `op_resolve.c`, `sgs_firmware.c`,
  `sgs_firmware_domain_execute.c`, `utility_operator.c`, `ipu_vrbuf.c`, `dma_util.c`, headers
  `Project/Interface/sgs_schema_reader.h`, `SDK/FlatIO/include/{flatbuffers_common_reader.h,flexb.h}`,
  `SDK/common/include/sgs_utility.h`;
- drivers / HAL: `drv/{drv_bdma.c,drv_cmdq.c,drv_sr.c}`, `hal/IPU200M/hw/{hal_cmdq.c,hal_conv.c,hal_dma.c,hal_ipu_ctrl.c,
  hal_ipu_dma.c,hal_ipu_sr.c}`, headers `Driver/hal/IPU200M/inc/{hal_ipu_sr.h,vp1_hal.h}`;
- operator kernels (`SDK/OPS_IMPL/…`): `Activation/op_relu.c`; `CONV_POOL/{conv_pool_ipu.c,conv_priv_common.c,sdk_conv.c,
  sdk_pool.c,tile_partition_util.c}` + `CONV_POOL/IPU200M/{sdk_conv_hw.c,sdk_mean.c,tile_partition_util_hw.c}` + header
  `tile_partition_util.h`; `Custom/` 24 files (arg_min_max, CondGreat, JointMicroPhaseModify, algtof, atan2, clip,
  floormod, formatconvert, fourbytesflip, jointmicroCondLess, lstm, not_equal, psroi_pooling, mean, postprocess_max,
  postprocess_unpack, range, reducesum, topkv2; IPU200M: dmacoefmatrix, roipooling, fixed2float, ipu_implement, ssd_nms
  offline / online); `DMA/` 24 files (align, concat, dilation, gather, pack, pad, reshape, resizebilinear, resizenearest,
  slice, stridedslice, tile, transpose — `op_*` front ends and `sdk_*` bodies; `dma_ops_common.c`,
  `drv_dma_ops_common.c`; IPU200M: `drv_dma_ops_ipu200.c`, `drv_resizebilinear_ipu200.c`, `drv_tile_ipu200.c`);
  `Lut/{lut_ipu.c,lut_ops_comon.c}` + `Lut/IPU200M/drv_lut_ops_ipu200.c`; `Math/{elementwise_ipu100.c,op_cast.c}`;
  `Utility/{datatype_convert_util.c,misc.c}` + `Utility/IPU200M/drv_jump.c`.

### 2.5 Function inventory [R]
The firmware has 1184 functions; 393 of them get a name from the
`__func__` string of their asserts (`__assert_func(file, line, func, expr)` at 0x2007_6E40) or of their log calls
(`printf_log(fmt, func, line…)` at 0x2001_F9C8); more get a name given by reading the code (runtime, CCIF, host
commands; [I] names, [R] behaviour). Many `static` helpers are inlined: one function often holds the asserts of several
source functions. With all these names, 1151 functions are named.

The region **0x2007_05BC–0x2007_9F9F** is newlib + libgcc (lower bound [R]: `ceil` at 0x2007_05BC, named by emulation in
`ipu-firmware-2-engines.md` §9.1, is the first library routine; the functions below it, up to 0x2007_05B6, are the
firmware's own LUT-ops helpers): soft-float `__adddf3` / `__muldf3` / `__divdf3` / `__*sf3`,
conversions, 64-bit division, `memcpy` 0x2007_7870, `memset` 0x2007_7958, `sprintf` 0x2007_8804, the `vfprintf` core,
the `malloc` / `_sbrk` client, `__assert_func` with the newlib text `assertion "%s" failed: file "%s", line %d%s%s` at
0x2009_1930 [R for the named ones; I for the rest, from their shape]. The soft-float and libm routines the operator
kernels call are named by their behaviour in `ipu-firmware-2-engines.md` §9.1; the rest of the region is identified by shape
only — a name match needs a RISC-V newlib build, which is not on hand, so the remaining names are not known.

### 2.6 Version strings [R]
- `version=Q_0.1.0_verified_21110517` at file 0x7D544 (0x2007_D544) and the hash `579e2d5fa321b41c4f00ea5d76ec9fc8` at
  file 0x7D568. Command 6 answers with both (§5.1); libmi_ipu requires the hash to equal its own copy and the major
  number of the version (digits before the first '.') to equal its own major `0` (`ipu-host.md` §5.2).
- `ipu_ccif_riscv_server`, `ipu_ccif_riscv_server_queue`, `drv/ccif/drv_ccif_server.c`, `_hal_to_riscv_isr`,
  `_malloc_start:%#x, _malloc_end:%#x`, `mapping_offset=%x`, `IPU_StoreLog` — the strings of the CCIF server (§4), of
  command 3 (§5.1) and of the log ring (§3.6).

**The other build.** The Anjoy MC-H12S firmware (602 280 B) has the same layout (data at file 0x7D000, the same entry
sequence except the PC-relative constants), version `Q_0.9_verified_21091510` and the **same** hash string. So the hash
is a protocol / ABI stamp, not a build id [I], and libmi_ipu of this camera would accept that build too (major 0) [I] —
consistent with the camera run, where a model converted by `Q_0.9` ran on this `Q_0.1.0` firmware (`ipu-host.md` §6).
This firmware is a later build of the same code as the Anjoy `Q_0.9` image, 97 %+ identical by instruction structure;
the function-level diff of the two builds: `ipu-firmware-2-engines.md` §11.

## 3. Runtime

### 3.1 Start-up [R: disassembly at 0x2000_0000]
```
0x20000000 csrci mstatus,8                    ; MIE off
           gp = 0x20093770; sp = 0x200ED800; mtvec = 0x200006FC   (direct mode)
           *(0x20091A00) = *(u32*)0x014200C4  ; big-heap start
           *(0x200919FC) = *(u32*)0x014200C8  ; big-heap end
           zero [0x20093208, 0x200E6A00)      ; .bss
           atexit(0x200774A0) at 0x20076E90; __libc_init_array at 0x2007760C
           csrsi mstatus,8; main(0,0) at 0x2000051E; exit (0x20076F54) if it ever returns, then a halt loop
```
1. Interrupts off; `gp`, `sp`, `mtvec` set.
2. Big-heap bounds from the IPU control block, written by the host before the core starts (`ipu-host.md` §2.4):
   `*(0x2009_1A00)` = `*(u32*)0x0142_00C4` (heap start, already + 0x2000_0000), `*(0x2009_19FC)` = `*(u32*)0x0142_00C8`
   (heap end). The RISC-V sees the IPU register space at **0x0100_0000** where the CPU sees it at **0x1800_0000**: the
   firmware reads 0x0142_00xx, the host writes 0x1842_00xx [I: from the matching offsets 0x4200C4 / 0x4200C8].
3. `.bss` [0x2009_3208, 0x200E_6A00) zeroed.
4. The two newlib init calls [I: names `atexit` / `__libc_init_array` from the newlib pattern].
5. Interrupts on; `main(0,0)`.

`main` (0x2000_051E): clear `mie` MEIE + MTIE; `irq_platform_init` (MEIE on, PLIC threshold 0, `0x0142_0038` =
0xFFFF_FFFF — all RISC-V→CPU interrupt sources masked [I: register meaning from §4.2]); `ipu_service_setup` (prints
"service setting up", `drv_ccif_riscv_init`, §4); `xTaskCreate(IPU_service_task, "IPU_service", 0x2000 words, NULL,
prio 4, NULL)`; `vTaskStartScheduler()` (creates `IDLE`, stack 0x100 words, prio 0, and the timer task `Tmr Svc`, 0x1000
words, prio 3, queue `TmrQ` of 5 × 16 B); never returns.

### 3.2 Core-local registers of the RISC-V (address space of the IPU core) [R addresses and use; I names]
| address | use |
|---|---|
| 0x0001_0000 | MSIP: 1 = raise the machine software interrupt (FreeRTOS yield), 0 = clear (in the handler) |
| 0x0001_0004 / 0x0001_0008 | `mtimecmp` low / high |
| 0x0001_000C (64 bit) | `mtime` |
| 0x0001_0100 + 4·n | interrupt *n* priority, 0–7 |
| 0x0001_0210 | interrupt enable bitmap (bit *n*) |
| 0x0001_0220 | priority threshold |
| 0x0001_0230 | claim (read) / complete (write) |
| 0x0001_110C | cache command: 1 = write back (clean), 2 = invalidate, 3 = both [I meanings, from the use in §3.6] |
| 0x0001_112C | 1 = the command applies to the range below, 0 = back to whole-cache mode |
| 0x0001_1130 / 0x0001_1134 | range start / end |

`cache_op(clean, inval, all)` (0x2000_05A4) [R]: (1,1,1) = only `fence iorw,iorw`; (1,1,0) = range, command 3; (1,0,0)
= range, command 1; (0,1,0) = range, command 2; (1,0,1) = command 1 whole cache; (0,1,1) = command 2 whole cache. Every
write is followed by `fence io,io`. `cache_set_range(start, end)` (0x2000_0680) writes 0x1_1130 / 0x1_1134. Every host
command handler calls (1,1,1) before and after its work — only a fence; the firmware relies on the host's cache
maintenance and on the IPU bus being coherent for its own accesses [I].

### 3.3 Traps [R: vector 0x2000_06FC]
The vector saves 31 registers in a 0x88-byte frame on the current stack (+0x80 `mstatus`, +0x84 `mepc`), then:
- **exception** (`mcause` ≥ 0) → `trap_exception(mcause, sp, a2)`. For `ecall` (cause 8 or 11) a2 selects 0x14 = clear
  MPIE, 0x1E = set MPIE, 0x28 = context switch resuming at `mepc` + 4, 0x32 = context switch resuming at the frame's
  saved `ra`; no code in the image issues `ecall` [R: instruction sweep], so this path is **unused**. Any other
  exception: write "trap" to stdout, `trap_dump_core` ("Core Status----", "Trap Cause: 0x%x", "Trap pc : 0x%x",
  "Mstatus: 0x%x", "sp: %p", and if a task runs: "RTOS Status----", "Current Task: %s", "Task stack:[%p, %p]", "Task
  priority %d"), then spin forever. All of it goes to the log ring (§3.6); the host notices only by its 20 s timeout and
  reads the ring (`ipu-host.md` §3.4, §4.7).
- **interrupt cause 7** (machine timer) → save the context in the current TCB, tick handler (0x2000_0CF8): `mtimecmp` +=
  **0xD548** (54 600 `mtime` counts per tick), `xTaskIncrementTick`, switch if needed. The tick rate in Hz depends on
  the `mtime` clock, which no string or constant gives — not known; 1 kHz would mean a 54.6 MHz `mtime` [I].
- **cause 3** (machine software) → `trap_soft_irq`: clear MSIP, `vTaskSwitchContext`, resume the new task. Other
  software causes: "Unknow Soft Interrupt", spin.
- **any other cause** (external, 11) → `external_irq_dispatch` (0x2000_1264): claim *n* from 0x1_0230; if nesting is
  enabled (byte 0x2009_324C, 0 in this image, so off) raise the threshold to *n*'s priority and re-enable MIE; call
  `handler[n]` (table 0x2009_32E0, set by `irq_attach`); write *n* to complete.

`irq_attach(n, prepare, handler)` (0x2000_3B46): "!!!Error: interrupt%d handler is NULL" if no handler; runs
`prepare()` ("…fail to prepare interrupt%d" on non-zero); priority(n) = 1; stores the handler; enables *n*; `fence.i`.
Only one external interrupt is attached: **n = 1, the CCIF CPU→RISC-V interrupt** (§4.2).

### 3.4 FreeRTOS port [R behaviour; I: version]
A RISC-V FreeRTOS kernel (stock task / queue / list / timer code; no version string, so the version is not known).
Port specifics:
- critical sections: `vPortEnterCritical` = clear MIE + nesting counter (0x2009_2F70); exit sets MIE at count 0; a
  negative count spins forever;
- yield = write 1 to MSIP; from an ISR the same (`portYIELD_FROM_ISR`);
- scheduler start: first timer compare = `mtime` + 0xD548, MTIE and MSIE on, restore the first task's frame and `mret`;
- the context frame (0x88 B, §3.3) is stored at `pxTopOfStack`; TCB 0x6C B: +0x00 `pxTopOfStack`, +0x2C priority,
  +0x30 `pxStack`, +0x34 name[16], +0x44 `pxEndOfStack`;
- the stack depth argument of `xTaskCreate` is in 4-byte words; new stacks are filled with 0xA5;
- **project addition in `xTaskCreate` / `prvInitialiseNewTask`**: when the task name contains "IPU_service", the stack
  base, the TCB address and the stack top are stored in three words of the code window (0x2000_06F0 stack base,
  0x2000_06F4 top, 0x2000_06F8 TCB), and `ccif_notify_stack_info` sends them to the host (§4.3, message bit 1) — once,
  during `main`, before the scheduler runs;
- **project addition in `vTaskSwitchContext`**: if the current task's `pxTopOfStack` ≤ `pxStack`,
  `ccif_notify_stack_overflow` (message bit 2, word 2 = `pxTopOfStack`), then `vApplicationStackOverflowHook` ("Stack
  Overflow" to stdout, spin);
- `vApplicationMallocFailedHook`: "malloc failed", spin;
- `mcycle` / `mcycleh` (read at 0x2000_3006 / 0x2000_3028) give the 64-bit time stamps of the statistics (§5.6).

### 3.5 Memory allocators [R]
Two separate heaps:
1. **FreeRTOS `heap_4`** (`pvPortMalloc` 0x2000_13CE): the fixed 0xE3F8-byte array at 0x2009_3320, 8-byte alignment,
   first-fit free list with coalescing; holds the TCBs, task stacks (`IPU_service` 32 KiB, timer 16 KiB, idle 1 KiB),
   queues and the CCIF records.
2. **newlib `malloc`** over `_sbrk` (0x2000_1064): the break starts at the host-given heap start (§3.1 step 2) and may
   not pass the heap end (else −1). The host gives the 4 MiB "ipu_heap" buffer here (`ipu-host.md` §5.2: InitFirmware
   heapStart / heapEnd = MIU address of the buffer + 0x2000_0000 — the IPU sees DRAM at 0x2000_0000 + MIU outside its own
   image window). All model, interpreter and kernel allocations (`sgs_malloc` → `malloc`) come from it; cmd 8 reports its
   current and peak use (§5.6).

The "malloc region" of host cmd 3 (`_malloc_start` 0x2009_32BC / `_malloc_end` 0x2009_32B8) is stored and printed but
read by nothing [R: no other reference] — **unused**.

### 3.6 Console and the log ring [R]
`printf_log` (0x2001_F9C8, the firmware's own printf, ~1200 call sites) and newlib `printf` / `write` (trap dump, hooks)
both end in `IPU_StoreLog(char)` (0x2000_432C), one character at a time (`_write(fd, buf, len)` at 0x2000_1322 stores
only `buf[0]` and returns `len` — newlib's stdout is unbuffered, so `len` is 1 [I]). There is no UART.

`IPU_StoreLog(c)` does nothing unless the log mode (0x2009_32CC, set by cmd 7) ≠ 0 **and** a ring was given (cmd 4,
§5.1). The ring is the host's 0x2000-byte "IPU_log" buffer: word 0 = read index (the host advances it), word 1 = write
index (the firmware), data from +8, size *S* = 0xFFC (from cmd 4). The buffer must be 64-byte aligned (assert
`((unsigned int)stlogInst.buf_base & (CACHE_LINE_ALIGNMENT-1)) == 0`, `ipu_api.c` lines 0x33D / 0x34B / 0x356 / 0x35E).
Per character: invalidate the ring range (`cache_op(0,1,0)` over [buf, buf + *S* + 8)), then
- mode 1 (**overwrite**): store at the write index, write index + 1, wrapping to 0 after *S* − 1; the read index is
  ignored;
- mode 2 (**stop when full**): store only if the ring is not full (write − read < *S* − 1, or for write < read: write
  + 1 < read);
- then clean the range (`cache_op(1,0,0)`).

Mode 0 drops everything. The host's `/sys/dla/ipu_log` reader and its 0 / 1 / 2 writes (`ipu-host.md` §4.8) match this.

## 4. CCIF server — the firmware end of the mailbox [R unless marked]

Code: `drv_ccif_riscv_init` 0x2000_36E2, `hal_ccif_riscv_init` 0x2000_395C, ISR `_hal_to_riscv_isr` 0x2000_387E, its
prepare routine 0x2000_3864, `ccif_receive_wait` 0x2000_3832, `ccif_parse_request` 0x2000_3AC0, `ccif_build_reply`
0x2000_39C4, `ccif_send_packages` 0x2000_3778 (+ `hal_ccif_write_rx_and_ring` 0x2000_3980), notifications 0x2000_394C /
0x2000_39A0 [names of the unnamed ones I]. The package format both ways and the host's side are `ipu-host.md` §3
(CCIF); this section states what the firmware does.

### 4.1 Records
Server record (`pvPortMalloc(0x1C)`, pointer at 0x2009_32C0; also the HAL's record via 0x2009_32C4): +0x00 name
"ipu_ccif_riscv_server"; +0x04 CCIF base 0x0141_0000; +0x08 FreeRTOS queue (100 items × 0x28 B, registry name
"ipu_ccif_riscv_server_queue"); +0x0C 0; +0x10 control window 0x0141_0000; +0x14 request window 0x0141_0018; +0x18 reply
window 0x0141_0040. The same block is at CPU physical 0x1841_0000 (`ipu-host.md` §3.1).

### 4.2 Registers as the RISC-V uses them (0x0141_0000 + off) [R use; I names]
| off | RISC-V use | host's use of the same offset (`ipu-host.md` §3.1) |
|---|---|---|
| +0x04 | write `1<<n`: clear CPU→RISC-V bit *n* (0xFFFF_FFFF at prepare) | — |
| +0x08 | CPU→RISC-V pending bits (the ISR scans bits 0–31) | the host reads bit 0 = "previous doorbell still pending" |
| +0x0C | write `1<<n`: raise RISC-V→CPU bit *n* | — |
| +0x10 | 0xFFFF_FFFF written at prepare (clears stale RISC-V→CPU bits [I]) | the host writes `1<<n` = clear RISC-V→CPU bit *n* |
| +0x14 | RISC-V→CPU pending bits; the sender waits for bit 0 = 0 | the host reads it in its ISR |
| +0x18…+0x3F | request package (CPU→RISC-V), read by the ISR | the host writes it ("TX") |
| +0x40…+0x67 | reply package (RISC-V→CPU), written by the firmware | the host reads it ("RX") |
| `0x0142_0038` bit 0 | 1 = RISC-V→CPU interrupt masked; `main` sets the word to 0xFFFF_FFFF, the prepare routine clears bit 0 | — |

The CCIF interrupt is core interrupt **1** (§3.3). Prepare routine (run once by `irq_attach`): `+0x04` = 0xFFFF_FFFF,
`+0x10` = 0xFFFF_FFFF, `0x0142_0038` &= ~1.

### 4.3 Bits (both directions)
| direction | bit | meaning |
|---|---|---|
| CPU→RISC-V | 0 | a request package is in +0x18 |
| CPU→RISC-V | 1 | the host's "dump your stack" doorbell (`ipu-host.md` §4.7): the ISR clears it and prints "IPU didn't respond"; it sends nothing back — the host's stack data come from the bit-1 message sent once at task creation (below) |
| CPU→RISC-V | 2–31 | "_hal_to_riscv_isr invalid isr %d", bit cleared |
| RISC-V→CPU | 0 | a package is in +0x40 (boot message, replies) |
| RISC-V→CPU | 1 | stack info, unsolicited: reply words 2 = `IPU_service` TCB − 0x2000_0000, 3 = its stack top − 0x2000_0000, 4 = its stack base (raw RISC-V address, **not** translated); sent from `prvInitialiseNewTask` during `main`, before the scheduler runs and before the boot message (§3.4) |
| RISC-V→CPU | 2 | stack overflow: word 2 = the task's `pxTopOfStack` (raw); the firmware then halts (§3.4) |

No bit-1 or bit-2 message waits for the host's acknowledgement (only bit 0 does).

### 4.4 Receive path
ISR: lowest pending bit *n* of +0x08; none → "<%s>unknown irq" with "_hal_to_riscv_isr". *n* = 0: copy the 40-byte
request window to a stack buffer, `xQueueSendFromISR`, write 1 to +0x04, yield if a task was woken. The service task
blocks in `xQueueReceive(…, portMAX_DELAY)` ("drv_ccif_tx_receive timeout" if it ever returns ≠ 1). The queue holds 100
requests, so the host may post several before the firmware answers; mi_ipu.ko serialises device requests anyway.

`ccif_parse_request` reads from the 40-byte package (byte offsets): +0 channel; +1 marker (host: 1); +4 command; +5 type
(host: 0x0B — read, not checked); +6 u16 payload length; +8… payload (copied for that length). For several fragments it
concatenates the payloads; the service always passes one package.

### 4.5 Send path
`ccif_build_reply(chn, cmd | marker<<16, len, words, &count, pkts)` cuts the reply into 32-byte fragments: per fragment
byte +0 channel, +1 marker (echo of the request's), +2 u16 fragment index, +4 command (echo), +5 **0x0A** for a full
fragment or **0x0C** for the last partial one, +6 u16 fragment length (0x20, or `len & 0x1F`), +8 data. Replies are 4 or
12 bytes, so every real reply is one package with type 0x0C and length 4 or 12. Payload word 0 (package word 2) = status.

`ccif_send_packages(n, pkts)`: per package, wait while (`+0x14` bit 0 | `0x0142_0038` bit 0) is set; if the mask bit is
set, print "error: risc2cpu irq was masked" and give up (the reply is lost); after 10 000 polls it prints "wait rx
timeout" on every further poll but keeps waiting (no timeout); then copy 40 bytes to +0x40 and write 1 to +0x0C.

### 4.6 Boot message
The service task first creates the op resolver, then sends one package whose marker byte (+1) is **0**; the other 39
bytes are uninitialised stack. The host accepts any marker-0 package as the end of its 5 s handshake (`ipu-host.md`
§3.5). If the op resolver cannot be created, "Fail to create op_resolver." is printed and the boot message is still sent;
channel creation would then fail.

## 5. Host commands — `ipu_api.c` [R unless marked]

Service task `IPU_service_task` (0x2000_3BDC): `ipu_service_init` (zero the channel table, create the op resolver),
boot message (§4.6), then forever: receive a request, dispatch on the command byte, reply with {status} (4 bytes) or, for
cmd 8, 12 bytes. All addresses in payloads are **MIU addresses**; the firmware adds 0x2000_0000 to use them (the IPU sees
DRAM at 0x2000_0000 + MIU outside its own image, `ipu-host.md` §2.4) [R: every handler adds 0x2000_0000]. Payload words
are w0…w7 below. Status 0 = OK, −1 = failure; an unknown command gets **0xFFFF_FFE8** (−24).

### 5.1 Command table
| cmd | handler | payload (sent by the host, `ipu-host.md` §3.6) | firmware action | reply |
|---|---|---|---|---|
| 0 | `ipu_cmd0_create_channel` 0x2000_41B8 | w0 chn, w1 model MIU, w2 desc MIU, w3 scratch MIU, w4 scratch size | §5.2 | status |
| 1 | `ipu_cmd1_destroy_channel` 0x2000_4218 | w0 chn | destroy the interpreter, free the model record, clear the slot | status |
| 2 | `ipu_cmd2_invoke` 0x2000_424E | w0 invoke-record MIU | §5.4 | status |
| 3 | `ipu_cmd3_set_malloc_region` 0x2000_4280 | w0 start, w1 end, w2 mapping offset | stores start / end (+0x2000_0000; read by nothing), prints "_malloc_start:%#x, _malloc_end:%#x" and "mapping_offset=%x", sets the DMA address map to {0xA020_0000, 0xA0A0_0000, w2} (§5.5) | 0 — **unused**: libmi_ipu never sends it (`ipu-host.md` §4.4) |
| 4 | `ipu_cmd4_set_parameter` 0x2000_42D6 + `ipu_set_log_ring` 0x2000_430E | w0 YUV420 W pitch alignment, w1 YUV420 H pitch alignment, w2 XRGB W pitch alignment, w3 0x2007_D000, w4 0x200E_D800, w5 mapping offset (firmware MIU), w6 log ring MIU or 0, w7 ring size | DMA address map = {w3, w4, w5}; pitch alignments stored (§5.5); if w6 ≠ 0: ring = w6 + 0x2000_0000, data at +8, size w7 (§3.6) | 0 |
| 5 | `ipu_cmd5_statistics` 0x2000_4498 | w0 buffer MIU | copy the 0x3898-byte statistics record (0x200A_2E08) to the buffer, `cache_op(1,1,1)` | 0 |
| 6 | `ipu_cmd6_get_version` 0x2000_44E8 | w0 buffer MIU, w1 length | length < 0x43 → −1; else `sprintf(buf, "%s-%s", "579e2d5fa321b41c4f00ea5d76ec9fc8", "Q_0.1.0_verified_21110517")` (58 characters + NUL; the host buffer must be ≥ 0x43 bytes) | status |
| 7 | `ipu_cmd7_set_log_mode` 0x2000_4308 | w0 mode 0–2 (not range-checked) | log mode (§3.6) | 0 |
| 8 | `ipu_cmd8_heap_stat` 0x2000_4544 | — | — | {0, current bytes, peak bytes} of the SGS allocator counters (0x200A_7188 / 0x200A_7184, §5.6) |

Handler names are [I] (given by reading the code). The version reply is what libmi_ipu splits at '-' (`ipu-host.md` §5.2): the hash
(protocol stamp, §2.6), then the firmware version without the `version=` prefix of the string at 0x2007_D544.

### 5.2 Channel table and CreateChannel
Table at 0x200A_1B3C, 48 slots × 100 B: +0x00 u8 in use; +0x04…+0x5F the model record filled by the model loader (§6);
+0x60 interpreter pointer. `ipu_get_channel(chn)`: chn ≥ 48 → "Error: channel%d exceeds the max limit:%d"; unused →
"Invalid channel ID:%d"; both give −1 to the host.

`ipu_create_channel_impl(chn, model, desc, scratch, scratchSize, 0)`:
1. chn < 48 and the slot free ("Error: channel%d exist, please choose other unused channel"), else −1;
2. zero the slot, mark it used;
3. model loader `sgs_ipu_ModelInitWithNetMemoryBySgsImage` (0x2001_A57C, named `parseFastRunGraph` by its `__func__` string
   because the fast-run parser is inlined) with the model buffer, the slot's record, the scratch buffer and its size —
   "SGS_IPU_ModelInit fail" on error. The slot stays marked used: a failed create **leaks the slot** until the host
   destroys it; the host frees its side and does not send cmd 1 then, so the channel id is lost until the firmware
   restarts [I: host behaviour from `ipu-host.md` §4.5];
4. `SGS_TF_InterpreterCreateByModel(&slot.interp, resolver, &slot.record)` (0x2000_AFF2) — "SGS_TF_InterpreterCreateByModel
   fail" (the model record is freed, the slot stays used);
5. `ipu_fill_tensor_desc(chn, desc)` (§5.3); on failure the interpreter and the model are destroyed, "bRet:%d" printed.

Both `cmd0` and `cmd2` call `cache_op(1,1,1)` (a fence) before and after.

### 5.3 The description buffer the firmware writes (the host reads it, `ipu-host.md` §5.6)
`ipu_fill_tensor_desc` asks the interpreter for subgraph 0's input and output tensors (`SGS_TF_InterpreterGetInputTensors`
0x2000_B55E / `…GetOutputTensors` 0x2000_B5D8, at most 61 each — "Fail to get input tensors' description." / "Fail to
get output tensors' description.") and writes, for tensor *t* (the 0x15C-byte tensor record, §6.4):

| entry off | value |
|---|---|
| +0x00 | `t.shape->size` (dimension count) |
| +0x04 | `t[+0x30]` inner-most stride |
| +0x08 | `t[+0x38]` aligned buffer size; for an output with the float flag: ×2 |
| +0x0C | element format: `map1[t.type]` if type < 10, else 6 (UNKNOWN); for inputs only, when that gives 0 (U8): `map2[t.ddrType − 3]` if ddrType − 3 < 15, else 0 |
| +0x10 (outputs) | u8 = `t[+0x128] & 2` — non-zero = "convert to FP32 on the host" |
| +0x11 (inputs) | u8 = `t[+0x128] & 1` — the same for inputs |
| +0x14 / +0x18 | if the format is INT16 (2): f32 scale = `*t.quant.scale`, s64 zero point = `*t.quant.zeroPoint`; else 1.0f and 0 |
| +0x20 | shape: `size` u32 words copied from the shape data |
| +0x40 | name, `snprintf(…, 0x100, "%s", t.name)` |

`map1` (0x2007_D324, index = TFLite tensor type FLOAT32, FLOAT16, INT32, UINT8, INT64, STRING, BOOL, INT16, COMPLEX64,
INT8): {5 FP32, 6, 3 INT32, 0 U8, 6, 6, 6, 2 INT16, 6, 4 INT8} — the values are `MI_IPU_ELEMENT_FORMAT` [S: public
header]. `map2` (0x2007_D34C, index ddrType − 3): {8 ABGR8888, 7 ARGB8888, 1 NV12, 0, …, 0, 1 NV12} — DDR types 3, 4, 5
and 17 are the image input layouts (§6.5). Input entries at +0x08, output entries at +0x4E30, 0x148 B each; counts at +0
and +4.

### 5.4 Invoke
`ipu_invoke_impl(record)` (record = the host's 0x7C0-byte invoke record, `ipu-host.md` §5.5, address + 0x2000_0000):
1. channel from record +0x000 (`ipu_get_channel`);
2. for each input *i* < record +0x008 (the counter is 8-bit): the pair {`phy[0]` + 0x2000_0000, `phy[1]` + 0x2000_0000}
   into the table 0x200A_1954 and its pointer into the list 0x200A_1860 (only the low 32 bits of each u64 are used);
3. for each output *i* < record +0x3E0: `phy[0]` + 0x2000_0000 into a stack list (`phy[1]` of outputs is ignored);
4. `SGS_TF_InterpreterFlipInOutBuffer(interp, subgraph 0, inputs, nIn, outputs)` (0x2000_B68A) — rebinds the subgraph's
   input / output tensors to these buffers (no copy);
5. `_sgs_doWorkThread(interp, subgraph 0, record[0x7B8] (force-online flag), 0)` (0x2000_B8AE) — runs the network
   (`ipu-firmware-2-engines.md` §2.5). 0 → status 0; any failure → −1 (the host maps non-zero to `E_IPU_ERR_INVOKE`).

The second address of an input (`phy[1]`) is the UV plane of an NV12 input [I: from the YUV420 input type and the public
`phyTensorAddr[2]`].

### 5.5 Device parameters
- **DMA address map** (`dma_set_addr_map` 0x2002_96B8, 13 bytes at 0x200A_916C + valid flag 0x200A_9178): the DMA
  drivers convert a firmware pointer *p* into a bus address: if a map is valid and start ≤ *p* ≤ end: *p* + offset +
  0xE000_0000, else *p* + 0xE000_0000 (= *p* − 0x2000_0000, i.e. back to the MIU address); the top two bits of the result
  go to IPU register 0x12's high field and the low 30 bits into the DMA descriptor (`ipu_set_reg_hi_field(0x12, a & 0xC000_0000)` (0x2002eb94))
  [R: the 0x2000_7xxx / 0x2000_81A2 callers]. After cmd 4 the map is the firmware's own data region 0x2007_D000…0x200E_D800
  → MIU = *p* − 0x2000_0000 + firmware MIU: buffers inside the image (`.bss`) get the right MIU address, buffers on the big
  heap are already "MIU + 0x2000_0000".
- **Pitch alignments** (0x2009_1A08 YUV420 W, 0x2009_1A0C YUV420 H, 0x2009_1A04 XRGB W; read back by 0x2000_469C and used
  by `dma_load_utils` 0x2000_4C06 for the input-format DMA): set by cmd 4 and **overridden at channel creation** by the
  model's `configuration` vector (Model field 8, §6.2; Anjoy model {16, 2, 16}). They give the DDR pitch of image inputs;
  the DMA asserts `(u32YUV420_H_Pitch_Alignment % 16) == 0`, `(u32YUV420_V_Pitch_Alignment % 2) == 0`,
  `(alignment_up(pstInputTensor->w*4, u32XRGB_H_Pitch_Alignment) % 4) == 0`. So the input image rows lie in the host's
  buffer with these pitches: YUV420 Y pitch = align(w, W align), plane height = align(h, H align); XRGB pitch = align(4w,
  XRGB align) [I: from the asserts and the public header's pitch semantics; the exact formula is in the DMA code,
  `ipu-firmware-2-engines.md`].

### 5.6 Statistics and heap counters
Statistics record 0x200A_2E08 (0x3898 B, the same layout as the host's buffer, `ipu-host.md` §4.9): +0x00 u64 invoke
start, +0x08 u64 invoke end (`mcycle`), records from +0x48, 0x48 B each, at most 200
(`sgs_TimeStatistics_SetBlockTimeForward`, assert `interpreter.c` 0x1112): {u64 start, u64 end, u32 decoder id, u8 mode
(1 = offline / IPU), char name[0x32] at +0x15}; +0x3888 record count; +0x388C u8 = the invoke's force-online flag — when
1 no records are written and the host prints nothing; +0x3890 / +0x3894 DMA load / store counters (`dma_counter_inc(0/1)` (0x20031790)).
The record is cleared at the start of every invoke while the subgraph record's word +0x48 is 0 [R] (that word marks
fast-run graphs [I]; which subgraph record is meant is not settled — in the 0x1070-byte parse record of §6.3, +0x48 lies
inside the input-index array, so it is the interpreter's own record [I]), so cmd 5 returns the **last** invoke's timing.

Heap counters: `sgs_malloc` adds the size to 0x200A_7188 and raises the peak 0x200A_7184; `sgs_free` subtracts.

## 6. The SGSI model file

Loader: `sgs_ipu_ModelInitWithNetMemoryBySgsImage(img, rec, scratch, scratchSize, verbose, fastRunRec, fastRun, …)` at
0x2001_A57C; model parser `parseModel` / `parseOpCodeVector` / `parseSubGraphVector` / `parseTensorVector` /
`parseOperatorVector`, inlined into 0x2001_7610 (`parseSubGraphVector`), 0x2001_5024 (`parseTensorVector`) and
0x2001_5EE8 (`parseOperatorVector`) [R]; they allocate through the `sgs_malloc` wrapper 0x2001_4ED2. Checked against the
Anjoy model with a flatbuffer walker of ours [R both]. The firmware parses the model **in place** in the host's network buffer; it does not copy it. The host's reading of the
same file (libmi_ipu reads the header up to +0x13A3): `ipu-host.md` §5.4.

### 6.1 Header and blob layout [R: offsets from the loader's prints; values Anjoy]
Header 0x13AC bytes ("SGS_HEADER size %d" prints 0x13AC):

| off | type | field (print label) | Anjoy |
|---|---|---|---|
| 0x0000 | char[4] + u32 | magic "SGSI" (`SGS_MAGIC %s`) | "SGSI", 1 |
| 0x0004 + 0x28·i | struct ×61 | input *i*: u32 shape[8] (right-aligned, unused leading dims = 1), +0x20 u32 stride / alignment, +0x24 u8 TFLite tensor type, 3 pad | (1,1,1,1,1,480,800,3), 16, 3 = UINT8 |
| 0x098C | u32 | input count (`SGS_INPUT_SIZE`) | 1 |
| 0x0990 + 0x28·i | struct ×61 | output *i*, same layout | 3 × (…,3,60\|30\|15,100\|50\|25,11), 16, 7 = INT16 |
| 0x1318 | u32 | output count (`SGS_OUTPUT_SIZE`) | 3 |
| 0x131C | u32 | model flatbuffer size *A* (`SGS_MODEL_SIZE`) | 759 172 |
| 0x1320 | u32 | variable (scratch) buffer size (`SGS_BUFFER_SIZE`) | 2 304 000 |
| 0x1324 | u32 | offline command flatbuffer size *B* (`SGS_OFFLINE_CMD_SIZE`) | 501 180 |
| 0x1328 | u32 | not read by the firmware | 1 |
| 0x132C | u8[32] | hash (`SGS_HASH_VALUE:0x` + 32 × `%02x`) — printed only, not checked | |
| 0x134C–0x1361 | | zero, not read | |
| 0x1362 | u32 (unaligned) | page size / alignment *P* (`SGS_PAGE_SIZE`) | 4096 |
| 0x1366 | u64 (unaligned) | MAC count (`SGS_MACS %lld`) | 1 289 162 496 |
| 0x136E | char[0x32] | converter SDK version (`SGS_SDK_VERSION`) | "Q_0.9_verified_21091510" |
| 0x13A0 | u32 | fast-run model size *C* (`SGS_FASRRUN_MODEL_SIZE`) | 0 |
| 0x13A4–0x13AB | | zero, not read | |

The header is only printed (when the loader's verbose flag is 1; the host path passes 0) — the firmware takes nothing
from it but *A*, *B*, *C* and *P*. Blob layout, `pad(x) = align_up(x, P) − x`:

| blob | starts at | content |
|---|---|---|
| model | align_up(0x13AC, *P*) | flatbuffer, identifier `TFL3` (§6.2) |
| offline commands | model start + *A* + pad(*A*) | flatbuffer `TFL3` (§8) |
| fast-run model | offline start + *B* + pad(*B*) | flatbuffer `TFL3` `FastRunModel` — parsed only when the loader's fast-run argument is set; the host path (`ipu_create_channel_impl`) passes 0, so **unused** here (§6.7) |

File size = align_up(0x13AC, *P*) + align(*A*) + align(*B*) + align(*C*) (Anjoy: 8192 + 761 856 + 503 808 + 0 =
1 273 856 ✓).

### 6.2 Model flatbuffer — TFLite schema v3 with SigmaStar additions [R: parser field indices; Anjoy values]
Readers are FlatCC-style (`tflite_<Table>_<field>_get`, `flatbuffers_*_vec_at` with "index out of range" asserts in
`flatbuffers_common_reader.h` / `sgs_schema_reader.h`).

**Model**: 0 `version` (u32; Anjoy 3); 1 `operator_codes` [OperatorCode]; 2 `subgraphs` [SubGraph]; 3 `description`
(string) — must be one of "TOCO Converted." (→ kind 1), "SGS Converted." (→ kind 2) or the seven "Original Float",
"Sigmastar Float", "Sigmastar Cmodel Float", "Sigmastar Cmodel Without IPU CTRL", "Sigmastar Cmodel With IPU CTRL",
"Sigmastar Cmodel Convert Image", "Sigmastar Postprocess", else assert `parseModel` (`model_parser.c` 0x19DF); Anjoy:
"Sigmastar Cmodel Convert Image". 4 `buffers` [Buffer]; 5 `metadata_buffer` [int] (count printed only); 6 string: the
converter SDK version (Anjoy "Q_0.9_verified_21091510"; the string "Got model version: (%s), doesn't match this SDK
version: (%s)!!!" exists, which code prints it is not traced [I]); 7 u32 (Anjoy 0x44, use not traced); **8
`configuration` [u32]**: words 0–2 are passed to `ipu_set_pitch_alignment` — the model overrides the host's pitch
alignments (Anjoy {16, 2, 16}) (§5.5). "The model is not convert to SGS Model!please read ConvertTool doc!" is printed for
a model whose kind is not SGS [R string; I: condition]. The "SGS-converted" models of §6–§7 are those of kind 2 (`sgs` = 1
in §7.1).

**OperatorCode**: 0 `builtin_code` (i8, TFLite numbering, converted by 0x2000_D922 to the firmware's op type); 1
`custom_code` (string); 2 `version` (u32, default 1). Anjoy: ADD (0), CONCATENATION (2), CONV_2D (3), MAX_POOL_2D (17),
MUL (18), CUSTOM (32) "Reshape_Alias", TRANSPOSE (39), RESIZE_NEAREST_NEIGHBOR (97).

**Buffer**: 0 `data` [u8]; the firmware keeps {pointer, length, offset from the model start} per buffer (12 B). Anjoy:
241 buffers, 139 non-empty, 578 176 B.

### 6.3 SubGraph [R: `parseSubGraphVector` 0x2001_7610, `parseTensorVector`, `parseOperatorVector`, the SGS_LSTM option parse; Anjoy values; I where marked]
Fields:
- 0 `tensors`; 1 `inputs` [int]; 2 `outputs` [int]; 3 `operators`; 4 `name`;
- **5 `dma_total_size`** (u32 — the scratch the subgraph needs; must be ≤ the host's scratch size, assert
  `model_parser.c` 0x1732; Anjoy 2 304 000 = header +0x1320);
- **6 `aInputFormat`** [u8] — DDR type per input (§6.5); Anjoy [4] = BGRA, i.e. memory bytes B, G, R, A — the order the
  camera run found the model wants (`ipu-host.md` §6); libmi_ipu reports it as `MI_IPU_FORMAT_ARGB8888` (§5.3);
- **7 `aTrainingInputFormat`** [u8] (Anjoy [1]);
- **8** [bool] per tensor — "offline required Tensor"; **9** [bool] per operator — "offline required operator";
- **10 `TensorMatrix`** vector of tables {0 `as16ConvertMartix` [i16 × 12], 1 `idx`} — the input colour conversion done by
  the DMA (`DmaCoefMatrix`): nine coefficients *c*0…*c*8 of a 3×3 channel matrix and three row biases. The kernel packs
  the coefficients as 13-bit sign-magnitude and the biases as 9-bit fields into the DMA registers +0x40…+0x54 and sends
  them by decoder op 1 to address 0xC040 (§8.3.1; packing: `ipu-firmware-2-engines.md` §10.19, DmaCoefMatrix). Anjoy {0,0,1024,
  0,1024,0, 1024,0,0, 256,256,256}: a B ↔ R swap in Q10 with bias 256 in each row [I: Q10 and the bias role].

The record: the array is allocated once (`count × 0x1070`, zero-filled; model record +0x10 = count, +0x14 = array) and
every subgraph is parsed in order:

| off | content | from |
|---|---|---|
| +0x000 | tensor count | field 0 length |
| +0x004 | tensor records (0x15C each, §6.4) | `parseTensorVector` (its 3rd argument is the subgraph record itself) |
| +0x008 | quant store (§6.4) | `parseTensorVector` |
| +0x00C | input count | field 1 length |
| +0x010 … +0x40F | input tensor indices, int32 (room for 256) | field 1, copied (`flatbuffers_int32_vec_at_0f35c`) |
| +0x410 | output count | field 2 length |
| +0x414 … +0x813 | output tensor indices (256) | field 2 |
| +0x814 | operator count | field 3 length |
| +0x818 | op nodes (0x2A8 each, §6.6) | `parseOperatorVector` (reads them back as `*(sg + 0x818) + i·0x2A8`) |
| +0x81C | option block (§7.1) | `parseOperatorVector` (`pOptions`) [I: the third word of the {count, nodes, options} triple the parser fills] |
| +0x828 | TensorMatrix records (0x1C each) | field 10 |
| +0x82C | TensorMatrix count | field 10 length |
| +0x830 | `dma_total_size` (`s32DmaTotalSize`) | field 5; the first subgraph's value is also added into model record +0x40 |
| +0x834 | scratch ("variable") buffer = the host's buffer (`pVariableTop`) | loader argument; assert size ≤ the host's (`model_parser.c` 0x1732) |
| +0x83C … +0xC3B | `aInputFormat`: u32 per input, DDR type (0x2000_D754, identity 0–21, §6.5) | field 6 |
| +0xC3C … +0x103B | `aTrainingInputFormat`: u32 per input, DDR type | field 7 |
| +0x103C | number of tensors flagged "offline required" (value 1) | field 8 |
| +0x1040 | number of operators flagged "offline required" | field 9 |
| +0x1044 | field 8 length | |
| +0x1048 | copy of field 8 (bytes) | |
| +0x104C | field 8 length again | |
| +0x1050 | int32 per tensor: its index among the offline-required tensors, or −1 | built from field 8 |
| +0x1054 | field 9 length | |
| +0x1058 | copy of field 9 (bytes) | |
| +0x105C | byte: 1 = this subgraph is an LSTM sub-net (set by the SGS_LSTM option parse, §7.4); read by `parseTensorVector` / `parseOperatorVector` together with the model's fast-run byte (+0x4C): only when fast-run = 1 and this byte ≠ 1 are just the offline-required tensors / operators parsed (counts +0x103C / +0x1040) | |
| +0x1060 | name (field 4) or "" | field 4 |
| +0x1064 | LSTM meta-tensor buffer | §7.4 |
| +0x1068 | LSTM meta-tensor aligned size | §7.4 |
| +0x106C | back pointer to the model record | |

No store to +0x820, +0x824, +0x838 was found in the three parse functions [R]; whether they are padding or filled by
the interpreter is not known [I]. The domain records, run contexts and statistics of the interpreter live in its own
structures, not here (`ipu-firmware-2-engines.md` §2.4).

### 6.4 Tensor and QuantizationParameters [R: `parseTensorVector` 0x2001_5024, decompile and emulation]
Firmware tensor record: 0x15C B per tensor.

| field | type as read | → record | notes |
|---|---|---|---|
| 0 | [int] | +0x00 as given; right-aligned into int[10] padded with 1 at +0xA0; element count +0x0C | ≤ 10 dims (assert `flatbuffers_int32_vec_len(vTensorShape) <= TENSOR_DIM_MAX`); no shape → scalar {1} |
| 1 | i8 | +0x04 | TFLite TensorType → firmware type (0x2000_F628) |
| 2 | u32 | +0x08 | buffer index |
| 3 | string | +0x28 | name |
| 4 | table | +0x40…+0x88 | QuantizationParameters, **mandatory** (below) |
| 5 | u8 | +0x98 (byte) | TFLite `is_variable` [S]; printed "bVar: %d" in the dump |
| 6 | i8 | +0x18 | DDR type (`sgs_tf_ConvertTensorDDRTypeFromTf`, §6.5); read only for SGS-converted models |
| 7 | **table** `stAlloc` {0 s32 → +0x1C `stAlloc.s32Started`, 1 s32 → +0x20 `stAlloc.s32ended`} | see notes | read only for a tensor whose Buffer is empty (no constant data) **and** whose subgraph has `dma_total_size` (+0x830) ≥ 1. Then: assert `(s32ended − s32Started) <= s32DmaTotalSize` (`model_parser.c` 0x15E9); subgraph scratch +0x834 must be non-zero (`pVariableTop != NULL`, 0x15EA); buffer +0x3C = scratch + `s32Started`; +0x34 and +0x38 (aligned size) = `s32ended − s32Started`, which must be a multiple of 64 (`% CACHE_LINE_ALIGNMENT`, 0x15EF) — or of 16 (`% L2_SLOT_WIDTH_INBYTES`, 0x15F3) when field 11 ≠ 0; kind +0x12C = 0x11; bytes +0x99, +0x9B cleared. Started = ended (or ended = 0) → no placement; with `dma_total_size` = 0 the slot is read and nothing is stored |
| 8 | u32 | +0x9C | — |
| 9 | u32 | +0x128 | extension flags: bit 0 = input given as FP32 by the host, bit 1 = output converted to FP32; `E_TF_TENSOR_EXT_FLAG_U16_2_S16` is another bit |
| 10 | i8 | +0x10 | type in EMI (`eTensorTypeInEmi`): TFLite TensorType → firmware type (0x2000_F628), default = field 1; unknown → 0xC with a log line |
| 11 | u8 (`lbu`) | +0x130 | non-zero selects the 16-byte instead of the 64-byte alignment check of field 7 |
| 12, 13, 14 | u32 | +0x134, +0x138, +0x13C | stored only |
| 15 | [int] | +0x148 | a vector object made by `sgs_tf_CreateVector` (0x2000_B45A: {count, flags, data*}) holding a copy of the ints |
| 16 | u32 | +0x14C | — |

Anjoy tensors use fields 0–10 (130 of 240 without field 9); fields 11–16 are newer optional ones [R presence]. No reader
of fields 8, 12–14, 16 was traced and the binary names none of them — their meaning is not known. Computed record fields:
inner-most stride +0x30 and aligned buffer size +0x38 (asserts `s32AlignedBufSize % CACHE_LINE_ALIGNMENT == 0` and
`% L2_SLOT_WIDTH_INBYTES == 0`, via `_sgs_PatchTensorAlignBufferSize`; the placement check of field 7 in full reads
`(pstTensor->stAlloc.s32ended - pstTensor->stAlloc.s32Started) <= (SGS_S32)pstSubGraph->s32DmaTotalSize`); +0x34 is
read as the element size from the parser's computed fields, while the field-7 placement above writes `s32ended − s32Started` there — the two readings are
not reconciled [R both].

**QuantizationParameters** (Tensor field 4) [R: emulation]. The table is mandatory for every tensor (absent → null-table
assert `tflite_QuantizationParameters_scale_get`). Each vector is taken only when its length is exactly 1 (per-tensor
quantisation); any other length leaves count 0 and pointer 0. Values are copied into a per-subgraph quant store
(subgraph record +0x008; allocated after a sizing pass: 0x24 B per tensor without quantisation, else 4 per f32 field and
8 per i64 field present with length 1) and the record points at the copy:

| field | TFLite [S] | type as read | → record |
|---|---|---|---|
| 0 | `min` | [f32] | count +0x58, pointer +0x5C |
| 1 | `max` | [f32] | count +0x50, pointer +0x54 |
| 2 | `scale` | [f32] | count +0x40, pointer +0x44 |
| 3 | `zero_point` | [i64] | count +0x48, pointer +0x4C |
| **4** | (TFLite: `details_type`) | **[i64]** | count +0x70, pointer +0x74 |
| **5** | (TFLite: `details`) | **[i64]** | count +0x78, pointer +0x7C |
| 6 | (TFLite: `quantized_dimension`) | u8 — `details_type` of the union below | — (selects field 7) |
| 7 | — | table `CustomQuantization` when field 6 = 1 (its field 0 `custom` [u8], accessor string `tflite_CustomQuantization_custom_get`) | +0x80 pointer to the bytes **in the model**, +0x84 length; field 6 = 1 with no table → null-table assert; other field-6 values: field 7 not read |
| 8 | — | u32 | +0x88 (`quantized_dimension` [I: position after the union, as in TFLite]) |

So SigmaStar's QuantizationParameters inserts two `[long]` vectors as fields 4 and 5 ahead of TFLite's `details` union
and `quantized_dimension`, which move to 6 / 7 and 8 [R: read widths and storage; I: that fields 6 / 7 / 8 are the TFLite
trio]. The meaning of fields 4 and 5 is not named in the binary (no accessor string) and no reader was traced — not
known; +0x60…+0x6C of the record stay 0. The dump labels "Max/Min/Scaler/zeroP" print +0x54 / +0x5C / +0x44 / +0x4C [I:
from the order].

### 6.5 Tensor DDR types (`E_TF_TENSOR_DDR_TYPE_*`) [R: names from the dump's switch table 0x2008_0E7C]
The model value is used unchanged: `sgs_tf_ConvertTensorDDRTypeFromTf` is the identity on 0–21, anything else →
"Unsupport tensor ddr type".

| value | name | | value | name | | value | name |
|---|---|---|---|---|---|---|---|
| 0 | INVALID | | 8 | IPU_S8_NHWX | | 16 | ABGR |
| 1 | RGB [I: the name string is a substring] | | 9 | IPU_S8_NHWC | | 17 | GRAY |
| 2 | BGR | | 10 | IPU_S8_KB | | 18 | IPU_U4_NHWX |
| 3 | RGBA | | 11 | IPU_S16_NHWX | | 19 | IPU_U4_NHWC |
| 4 | BGRA | | 12 | IPU_S16_NHWC | | 20 | IPU_S4_KB |
| 5 | YUV420 | | 13 | IPU_S16_KB | | 21 | not resolved (`IPU_FLOAT32_NHWC` exists as a string) [I] |
| 6 | IPU_U8_NHWX | | 14 | IPU_S32_NHWC | | | |
| 7 | IPU_U8_NHWC | | 15 | ARGB | | | |

Image types (1–5, 15–17) are the network-input layouts the DMA converts from (`DmaCoefMatrix`, the colour matrix of
§6.3); `NHWC` = packed channels, `NHWX` = channels padded to the slot width, `KB` = kernel (weight) blocks for the conv
engine [I: from the asserts that pair them with feature maps and weights]. The host-visible format of an input (§5.3) is
RGBA → ABGR8888, BGRA → ARGB8888, YUV420 → NV12, GRAY → NV12, others U8 [R: `map2`].

### 6.6 Operator [R: `parseOperatorVector` 0x2001_5EE8, `_sgs_ipu_parseDataConvertPara` 0x2000_EDD0]
<!-- ev:ipu-firmware-1-runtime-model.6.6 -->
Firmware op node: 0x2A8 B per operator. Anjoy operators carry fields {0–4, 7–10, 12, 14} (one also 13) [R presence].

| field | type | meaning → node offset |
|---|---|---|
| 0 | u32 | `opcode_index` → +0x00; +0x04 = pointer to the OperatorCode record |
| 1 / 2 | [int] | `inputs` / `outputs` → +0x0C / +0x10 |
| 3 | u8 | `builtin_options_type` → +0x14 (mapped, below) |
| 4 | table | `builtin_options` → +0x18; the parsed option struct's address is stored at +0x1C (§7.1) |
| 5 | [u8] | `custom_options` (FlexBuffer, §7.4) → +0x24 pointer, +0x28 length |
| 6 | | not read |
| 7 | [int] | → +0x30 (vector pointer) [I: TFLite `mutating_variable_inputs` / `intermediates` position; use not traced] |
| 8 | i8 | **domain type** (`sgs_tf_ConvertDomainFromTf` 0x2000_D80A) → +0x34 |
| 9 | [DataConvertParameters] | input requantisation, ≤ 6 entries (assert `sizeof(pstConvertInputs->astPara)…`), 0x50 B per input from +0x40, count +0x220 |
| 10 | [DataConvertParameters] | output requantisation, ≤ 1 entry, at +0x224, count +0x274 |
| 11 | | not read |
| 12 | table RunMode | {0 i8 `eRunMode` (default 4), 1 u32 `u32Fence` (default −1), 2 s32 `s32DecodeId` (−1), 3 s32 `s32Offset` (−1), 4 s32 `s32Size` (−1)} → +0x280 mode, +0x28C fence, +0x290 decoder, +0x294 offset, +0x288 size, +0x284 `pCmdEntry` = offline command data (§8.1) + offset; read only for SGS-converted models |
| 13 | u8 | `bDepthRenderNextDomain` → +0x298 |
| 14 | u32 | → +0x2A0 (SGS-converted models only) |

Node +0x29C points back to the subgraph record (read by the SGS_LSTM option parse, §7.4).

**Run modes** (dump strings): 1 "invoke by cmdq" (the op's slice of the offline command block is run by a decoder, §8),
2 "invoke by cpu", 3 "skip", others "Invalid" [R: the print's selection; the mapping of 2 is from the string left as the
default — I]. How the interpreter acts on them: `ipu-firmware-2-engines.md` §2.5.

**DataConvertParameters** (one table per entry): field 0 `data_convert_type` (1, 2 or 3; else "Unsupport data convert
type: %d."), fields 1–4 four s32. Each 0x50-byte group holds three slots of {type, f1, f2, f3, f4}: type 1 at +0x14, type
2 at +0x28, type 3 at +0x3C. The dump prints f2 as ".scaler", f1 ".bias", f3 ".shift", f4 ".rcshift" and labels the
type-2 slot "u8_s16", the type-1 slot "s16_u8", type 3 "merge". The labels fit the use: the type-2 slot widens inputs
(u8 → s16), the type-1 slot narrows outputs (s16 → u8); the type-3 "merge" slot has no reader. By the assert-line rule
of §7.2 the table has 5 fields [I].

**Option type mapping.** The value in field 3 is used as is for 1–90 (TFLite numbering) and 200–219 (SigmaStar), except
211 and 216, which are rejected ("Unsupport Options type: %d(%s)!!!", type set to 0xDC), and 215, which becomes 216 [R:
`parseOperatorVector`]. Custom codes override it before the size pass: `SGS_LSTM` → 215 (0xD7), `PhaseGen` → 216 (0xD8),
`CustomFloorMod` → 219 (0xDB) (comparison by `strcmp` (0x200789a8) at 0x2001_60xx; others in the same chain not
traced). `SGS_IPU_TfParseOption` itself parses 211 and 216 normally (§7.3); the rejection of 211 / 216 and the 215 → 216
remap in `parseOperatorVector` were read once and not re-checked against that, so which option tables of types 211 / 216
reach the parser through field 3 is not settled. Names (0x2000_D1C2): 0 NONE, 1–90 the TFLite `BuiltinOptions` names
(`Conv2DOptions` … `MatrixSetDiagOptions`), 200 Logistic, 201 Relu, 202 PRelu, 203 NMS, 204 PostProcessUnpack, 205 Round,
206 Sqrt, 207 RSqrt, 208 Tanh, 209 Align, 210 RoiPooling, 211 PSRoiPooling, 212 CustomizedConcatenation, 213 Reciprocal,
214 DmaCoefMatrix, 215 JointMicroPhaseGen, 216 Atan, 217 WiggleErr, 218 Atan2, 219 CustomFloorMod, 220 CustomMulAdd, 221
CustomAlphaBlending, 222 Gelu, 223 LayerNorm, 224 Pool3D, 225 ImageConcatenation, 226 Conv3D, 227 MedianFilter (all
`…Options`). The union's name strings at 0x2008_5108…0x2008_5970 are the TFLite ones up
to `MatrixSetDiagOptions` plus SigmaStar `LogisticOptions`, `ReluOptions`, `PReluOptions`, `NMSOptions`,
`PostProcessUnpackOptions`, `RoundOptions`, `SqrtOptions`, `RSqrtOptions`, `TanhOptions`, `AlignOptions`,
`RoiPoolingOptions`, `PSRoiPoolingOptions`, `CustomizedConcatenationOptions`, `ReciprocalOptions`,
`DmaCoefMatrixOptions`, `JointMicroPhaseGenOptions`, `AtanOptions`, `WiggleErrOptions`, `Atan2Options`,
`CustomFloorModOptions`, `CustomMulAddOptions`, `CustomAlphaBlendingOptions`, `GeluOptions`, `LayerNormOptions`,
`Pool3DOptions`, `ImageConcatenationenationOptions` (sic, 225), `Conv3DOptions`, `MedianFilterOptions`. The option union
thus has 110+ table types, parsed by `SGS_IPU_TfParseOption` (0x2000_F6B4, 42 accessor asserts) (§7).

**Internal option struct size per type** (`sgs_utility_GetOptionsSize` 0x2001_5D2A; types not listed have none) [R: re-run under emulation for 0–255]: 1, 2 0x390; 5 0x160; 8 0x310; 9 0x300; 10, 11 0x20;
15 0x10; 17, 21 0x40; 23 0x10; 27, 28 0x20; 29 0x40; 30 0x10; 32 0x20; 33 0x320; 35 0x2A0; 37 0x30; 58 0x10; 59, 64 0x2A0;
74 0x10; 75 0x40; 77 0x10; 79 0x2A0; 80 0x10; 87 0x10; 200 0x310; 201 0x20; 202 0x30; 203 0x40; 204 0x70; 205 0x30; 206
0x320; 207 0x620; 208 0x2F0; 210 0x20; 211, 212 0x10; 213 0x2F0; 214 0x20; 215 0x10; 216 0x20; 217 0xF90; 218 0x310; 219
0x80. For types **76, 81, 83–86, 88–90** the function runs `assert(0)` (`model_parser.c` lines 4202, 4228, 4239, 4245,
4251, 4257, 4268, 4274, 4280), so a model whose operator carries one of these option types stops in the size pass of
`parseOperatorVector`, before the option parser is reached. Conv2D / DepthwiseConv2D options also reserve room for their
`kb_tile_info` records (11 **bytes** per entry, §7.2) and ConcatenationOptions (10) / CustomizedConcatenationOptions
(212) for their changed-input lists (§7.1).

### 6.7 Fast-run model (blob 3) — unused on the host path [R: loader 0x2001_A57C tail, `parseFastRunTensors` 0x2001_9160, `parseFastRunOperators` inlined in 0x2001_99D2]
Parsed only when the loader's fast-run argument is set (never by `ipu_create_channel_impl`, §5.2); then the blob after
the offline commands must carry identifier `TFL3`, else the loader asserts in `tflite_FastRunModel_operator_codes_get`.
The fast-run model is a slimmed graph that keeps only what replaying the command stream needs [I]. With the fast-run flag
set, Conv2D / DepthwiseConv2D options are not parsed (§7.1) and only the offline-required tensors / operators of a
non-LSTM subgraph are parsed (§6.3, +0x105C).

**FastRunModel**: 0 `operator_codes` [OperatorCode] (same table as §6.2; 12-byte records {version, op type, custom
name}); 1 `graphs` [FastRunGraph].

**FastRunGraph** (record 0x34 B): 0 `tensors` [FastRunTensor] (count +0x00); 1 `operators` [FastRunOperator] (count
+0x0C); 2 `inputs` [int] → +0x18; 3 `outputs` [int] → +0x1C; 4 `TensorMatrix` vector (as in SubGraph: {0
`as16ConvertMartix` [i16], 1 `idx`}, 0x1C B each) → +0x20 / count +0x24; 5 `dma_total_size` u32 → +0x28 (must be ≤ the
scratch size, assert `model_parser.c` 0x1CDB; "pVarBuf != NULL" if no scratch); scratch pointer → +0x2C.

**FastRunTensor** (the same 0x15C-byte tensor record as §6.4):

| field | type | meaning → record |
|---|---|---|
| 0 | string | name → +0x28 |
| 1 | [int] | shape (≤ 10) → +0x00, right-aligned int[10] at +0xA0 padded with 1 |
| 2 | i8 | TFLite type → +0x04 |
| 3 | s32 | data offset (below) |
| 4 | u32 | element count → +0x0C |
| 5 | u32 | aligned buffer size → +0x38 |
| 6 | i8 | DDR type → +0x18 |
| 7 | u8 | 1 = constant: record kind 0x10, data = model data base + offset; otherwise: graph inputs / outputs are left to be bound at invoke, every other tensor is kind 0x11 with data = scratch + offset |
| 8 | table | quantization (as Tensor field 4) |

Inner-most stride +0x30 = align_up(last dim × element size, 16).

**FastRunOperator** (the 0x2A8-byte op node of §6.6): 0 RunMode table (the same five fields as Operator field 12 →
+0x280…+0x294); 1 i8 domain type → +0x34; 2 `opcode_index` → +0x04 (OperatorCode record), registration looked up into
+0x14 by 0x2001_8E44; 3 `inputs` → +0x0C; 4 `outputs` → +0x10; 5 [u8] → +0x1C [I: custom options]; 6 vector → +0x24,
length +0x28 [I: not traced further]. No builtin options are parsed for fast-run operators [R: no option parse call in
this path].

## 7. Option tables [R: emulation and the parser code]
<!-- ev:ipu-firmware-1-runtime-model.7 -->

`SGS_IPU_TfParseOption` (0x2000_F6B4) is one 22.5 KB function (≈5 000 decompiled lines) that inlines every FlatCC
accessor; its behaviour below is from running it per option type (0–255) and reading the branches the runs flagged.

### 7.1 The call and the option memory [R: `parseOperatorVector` 0x2001_5EE8; emulation]
`SGS_IPU_TfParseOption(opt, node, sgs, kbCursor, fastRun, concatCursor, ccatCursor)` — called once per operator by
`parseOperatorVector`:

| arg | meaning |
|---|---|
| `opt` | this operator's option struct: a slice of one block allocated for the whole subgraph (`pstOperatorVector->pOptions`, assert `model_parser.c` 0x1273; subgraph record +0x81C, §6.3) |
| `node` | the 0x2A8-byte op node (§6.6): +0x14 option type, +0x18 the option table (flatbuffer); the function stores `opt` at +0x1C |
| `sgs` | 1 = SGS-converted model (model kind 2, §6.2); otherwise the result of 0x2000_EBDA. With `sgs` ≠ 1 only the TFLite fields are read (§7.3 marks the SGS-only fields) |
| `kbCursor` | where Conv2D / DepthwiseConv2D write their `Kb_Tile_Info` records (§7.2) |
| `fastRun` | byte +0x4C of the model record = the loader's fast-run argument (`parseFastRunGraph` 0x2001_A57C stores its 8th argument there). When 1, types 1 and 2 return at once — Conv2D / DepthwiseConv2D options are not parsed in a fast-run model; the host path passes 0 (§6.7) |
| `concatCursor` | the area that `ConcatenationOptions` field 2 sizes (§7.3 type 10) |
| `ccatCursor` | the area of `CustomizedConcatenationOptions` (type 212, §7.4) |

The block is sized by a first pass of `parseOperatorVector`: Σ `sgs_utility_GetOptionsSize(type)` (§6.6) + 11 × (number
of `kb_tile_info` entries of all Conv2D / DW ops, field 40 / 41) rounded up to 4, + the ConcatenationOptions area
(`get_width_inslots_2a9d0` per op) + the type-212 area (`get_width_inslots_2aa16`), then zero-filled (`memset`) before
the second pass calls the parser [R: 0x2001_6114–0x2001_6150]. So every struct field the parser does not write is 0.
Custom codes `SGS_LSTM`, `PhaseGen`, `CustomFloorMod` set the node's type to 0xD7, 0xD8, 0xDB before the size pass
(§6.6).

Return value: always 1 except where a nested check fails (asserts abort; the slot-group copy returns 0 from its helper
and the caller asserts).
- **Unsupported types** (the `default:` of the switch: 3, 4, 6, 7, 12–14, 16, 18–20, 24, 25, 31, 34, 36, 38, 41–47, 49,
  50, 52–57, 60–63, 65–73, 91–199, 220–255) print "Unsupport: builtin option type: %s(%d)." with the firmware's own
  option name (0x2000_D1C2 over `type & 0xFF`) and return 1 with nothing written [R: emulation of all 256 values].
- **Types handled with no fields read** (`break` at once): 0 NONE, 8 FullyConnected, 22 Pad, 26 Transpose, 39
  MaximumMinimum, 40 ArgMax, 48 Slice, 51 Tile, 76 SquaredDifference, 78 Abs, 81 ReverseV2, 82 AddN, 83 GatherNd, 84 Cos,
  85 Where, 86 Rank, 88 MatrixDiag, 89 Quantize, 90 MatrixSetDiag, 209 Align — their option struct (if
  `GetOptionsSize` gives one) stays zero; for 76, 81, 83–86, 88–90 the size pass asserts first (§6.6).
- Several of these ops are registered (e.g. LESS uses LessOptions = 41, which is "unsupported" here): the message is
  only a log line, the op still runs without options [R: return 1; I: that the kernels never read an options struct for
  them].

### 7.2 Shared pieces [R unless marked]
<!-- ev:ipu-firmware-1-runtime-model.7.2 -->

**Field names.** The binary names only fields whose FlatCC accessor is a real function or whose accessor's null-table
assert survived inlining (every named accessor was run on a probe table, so
name ↔ index is [R]). TFLite fields (indices that exist in the TFLite schema of that table) are named from the TFLite
schema [S]; the firmware's option union is TFLite's up to 90 `MatrixSetDiagOptions`, then SigmaStar's 200–227 (§6.6).
All other SigmaStar fields carry no name in the binary and are listed by index ("—"). Assert line numbers in
`sgs_schema_reader.h` follow a fixed rule [R on four tables: LutInfomations, OpLutConfig, OpLutParamters,
OpParameterConvert]: the accessor of field *i* of a table sits on line `vec_at + 3 + i`, and the next table's `vec_at` is
7 lines after the previous table's last field. From it [I]: `Kb_Tile_Info` field 1 is `tile_data_lenth` (line 0x994,
vec_at 0x990); `DataConvertParameters` has 5 fields (0x985…0x989, next vec_at 0x990); `ConcatenationOptions` field 2 is
`changed_input_count` (0xCD5, field 0 `axis` 0xCD3); Conv2DOptions field 40 and DepthwiseConv2DOptions field 41 are
`kb_tile_info` (0xB41 − 0xB19 = 40, 0xBFF − 0xBD6 = 41 — matches the read indices [R]); the sub-table in ReducerOptions
field 1 is `ReduceSumOptions` with 3 fields, field 1 `rcshift_int32toint16_convert` (assert line 0xE22, ReducerOptions
`keep_dims` = field 0 at 0xE2D).

**Enum converters** (each prints an error and returns the "unknown" value; the parse goes on):

| converter | input (flatbuffer i8) → stored u32 | unknown → |
|---|---|---|
| `sgs_tf_ConvertPadFromTf` 0x2000_DC56 | 0 → 0, 1 → 1, 2 → 2 (TFLite SAME = 0, VALID = 1 [S]) | 3, "Unknow padding type" |
| `sgs_tf_ConvertActivationFromTf` 0x2000_DCAA | 0–5 identity (NONE, RELU, RELU_N1_TO_1, RELU6, TANH, SIGN_BIT [S]) | 6, "Unknow activation type" |
| MirrorPad mode (inline, type 77) | 0 → 0, 1 → 1 (REFLECT, SYMMETRIC [S]) | 2 (also for 2), "Unknow Mirror padding type" |
| TensorType (inline, type 80 `idx_out_type`) | 0–11 identity (TFLite FLOAT32 … INT8 [S]) | 0xC, message reuses "Unknow activation type" |
| DetectionType (inline, types 203 / 204 field 0) | 0–6 identity | 8, "Unknow DetectionType" |
| PostProcessUnpackComponentType (inline, type 204 field 3.0) | 0–7 identity | 8, "Unknown PostProcessUnpackComponentType" |

(Ranges from a sweep of −2…15 per converter [R]; values are read as signed bytes.)

**OpParameterConvert** (requantisation record; table fields 0 `op_convert_type`, 1 `scale`, 2 `scale2`, 3 `shift`,
4 `offset`, 5 `rcshift`, all u32 [R: accessors]) → 0x18-byte struct entry {+0 type, +4 scale, +8 scale2, +0xC shift,
+0x10 offset, +0x14 rcshift}, copied as read (no conversion). Option fields that hold a vector of them copy at most a
fixed number of entries; further entries are ignored [R: emulation with 1–7 entries]:

| type | field | entries at | max |
|---|---|---|---|
| 11 Add, 28 Sub | 1 | +0x4 | 1 |
| 21 Mul, 29 Div, 75 LeakyRelu | 1 | +0x4, +0x1C | 2 |
| 27 Reducer | 1.1 (in ReduceSumOptions) | +0x8 | 1 — and ≥ 1 required ("index out of range" assert) |
| 37 Cast | 4 | +0xC | 1 |
| 201 Relu | 2 | +0x4 | 1 |
| 202 PRelu, 205 Round | 0 | +0x0, +0x18 | 2 |
| 207 RSqrt | 1 | +0x5E0, +0x5F8 | 2 |
| 216 Atan | 0 | +0x0 | 1 |
| 217 WiggleErr | 1 | +0xEB0 + 0x18·k | 6 |
| 217 WiggleErr | 3 | +0xF48 + 0x18·k | 3 |
| 219 CustomFloorMod | 1 | +0x4 + 0x18·k | 5; with ≥ 6 entries only entry 0 is copied [R: 4, 5 → all; 6, 7, 8 → only +0x4] |
| 33 Exp, 200 Logistic, 206 Sqrt | 1 | +0x2F0 | 1 (read when the table's vtable has the field) |

There is no count word: unused entries stay 0. `op_convert_type` values and their meaning are those of
`DataConvertParameters` (§6.6: 1, 2, 3) [I: same record shape; not checked in the kernels].

**LutInfomations** (`lut_info`, the LUT program of a LUT-driven op; fields 0 `table_first`, 1 `table_second`, 2 `op_lut_config`
(table `OpLutConfig`), 3 `op_lut_params` (table `OpLutParamters`) [R: accessors]) → a 0x2F0-byte block, the same layout
in every type that has one [R: emulation]:

| block off | size | from |
|---|---|---|
| +0x000 | 0x82 | `table_first` vector data, **copied as 0x82 bytes whatever the vector's length** (shorter vectors over-read the flatbuffer behind them; the length word is not read) — 65 × i16 [I: size] |
| +0x088 | 0x202 | `table_second` vector data, 0x202 bytes the same way — 257 × i16 [I] |
| +0x290 | 0x14 | `OpLutConfig` packed: +0x290 `intpx_ko` i16, +0x292 `intpy_ko` i16, +0x294 `intpx_ku` i16, +0x296 `intpy_ku` i16, +0x298 `table_hybrid_priority` u8, +0x299 `table_oflow_priority` u8, +0x29A `table_uflow_priority` u8, +0x29B `lut_x_function` u8, +0x29C `enable_lutSymm` u8 (fields 0–8 in this order) |
| +0x2A4 | 0x4C | `OpLutParamters` packed (field → off, type): 0 `lut_data_bias0` +0x2A4 i16; 1 `lut_data_bias1` +0x2A6; 2 `bias2` +0x2A8; 3 `bias3` +0x2AA; 4 `lut_scale` +0x2AC i16; 5 `lut_exp_start` +0x2B0 s32; 6 `table_id` +0x2B4 u8; 7 `lut_x_shift` +0x2B8 s32; 8 `lut_y_shift` +0x2BC s32; 9 `bias4` +0x2C0 i16; 10 `bias5` +0x2C2 i16; 11 `lut_x_max` +0x2C4 s32; 12 `lut_x_min` +0x2C8; 13 `lut_y_max` +0x2CC; 14 `lut_y_min` +0x2D0; 15 `lut_x_bias` +0x2D4; 16 `lut_y_bias` +0x2D8 (s32); 17 `intp_x_ko_shift` +0x2DC s8; 18 `intp_x_ku_shift` +0x2DD; 19 `intp_y_ko_shift` +0x2DE; 20 `intp_y_ku_shift` +0x2DF (s8); 21 `bias6` +0x2E0 i16; 22 `bias7` +0x2E2; 23 `ocvt_x_offset` +0x2E4 i16; 24 `ocvt_y_offset` +0x2E6; 25 `ocvt_x_scale` +0x2E8; 26 `ocvt_y_scale` +0x2EA (i16); 27 `ocvt_x_shift` +0x2EC u8; 28 `ocvt_y_shift` +0x2ED u8 |

The flatbuffer read widths are those of the accessors: i16 = `lh`, s8 = `lb`, u8 = `lbu`, s32 = `lw`; the
`pstOptLut->stLutParam.s32LutExpStart / s32LutXShift / s32LutYShift / s8IntpXKoShift … / u8OCvtXShift / u8OCvtYShift`
assert strings name the C fields [R]. The four sub-objects have no null checks of their own beyond the accessor asserts:
a LutInfomations without `op_lut_config` or `op_lut_params` asserts; without `table_first` / `table_second` the copy
reads address 0 [R: emulation stops at `memcpy` with source 0]. Where the LUT is a vector of LutInfomations, block *k* is
at base + 0x2F0·*k*, up to a fixed count (RSqrt 2, WiggleErr 5; more entries ignored). The LUT kernel loads
`table_first` / `table_second` into the decoder's LUT memory (§8.3.1); the use of the configuration and parameter fields
is `ipu-firmware-2-engines.md` §8 (LUT unit).

**Kb_Tile_Info** (Conv2D field 40, DepthwiseConv2D field 41: vector of tables, 6 fields) → 11-byte packed records written
at `kbCursor` (not inside the option struct), one per vector entry, in order: field 0 u32 → +0 (4 B), field 1
`tile_data_lenth` u32 → +4 (4 B), field 2 u16 → +8 (2 B), fields 3, 4, 5 (u8, only bit 0 used) → bits 0, 1, 2 of +0xA.
The option struct gets the entry count and the pointer (Conv2D +0x338 / +0x33C, DW +0x33C / +0x340); the caller advances
`kbCursor` by 11 × count (§7.1). These are tile descriptors of the conv kernel's weight data, not the tiled data itself:
+0 byte offset of the tile in the layer's weight data, +4 byte size, +8 KB buffer depth in lines, +0xA bit 0 = zero tile
(skipped), bit 1 = last non-zero tile of its output group, bit 2 = not first (accumulate) [R: the conv kernel's reads,
`ipu-firmware-2-engines.md` §6.9 (conv tile planning)]. When a model carries no `kb_tile_info`, the kernel builds the table
itself.

**Slot groups** (`sgs_tf_getSlotsData` 0x2000_F5A2): a `[u32]` vector of ≤ 32 entries → u32[32] + count at +0x80 of the
group (0x84 bytes per group); > 32 prints "Slot group len: %d max: %d, it is overflow !!!" and the caller asserts
`sgs_tf_getSlotsData(&pstOpt->stPartition…GroupsSlot, vSlot) == TRUE`. The groups are named by those asserts:
`stPartitionKBGroupsSlot`, `…IBGroupsSlot`, `…OBGroupsSlot`, `…ABGroupsSlot`, `…VBGroupsSlot` (Conv / DW),
`stPartitionInput0GroupsSlot`, `stPartitionOutput0GroupsSlot` (Pool2D). In Conv / DW they are the L2 slot lists of the
kernel, input, output and accumulator buffers, used unchanged; the counts are those buffers' depths in slots (§7.3).

**Fixed-length array copies.** Several `[u32]` fields are copied with a fixed length regardless of the vector length (the
rest over-reads whatever follows the vector in the model): Conv2D field 41 / DW field 42 — 16 words; Split field 2 /
Pack, Unpack field 3 — 4 words; Split field 3 / Pack, Unpack field 4 — 160 words; DmaCoefMatrix field 0 — 6 words (= 12 ×
i16, `as16ConvertMartix`) [R: guard bytes after the vector land in the struct]. A model is read correctly only when these
vectors are at full length [I].

### 7.3 Every parsed option type, field by field [R: emulation; notes on the use from the kernels' reads]
<!-- ev:ipu-firmware-1-runtime-model.7.3 -->

Columns: **f** flatbuffer field index; **name** — TFLite name [S] where the index exists in the TFLite table, else "—"
(no name in the binary); **read** — load width of the field (`i8` = `lb`, `u8` = `lbu`, `i16` = `lh`, `u32` = `lw`,
`u64` = two `lw`), the scalar reads are the FlatCC defaults-when-absent of the converter's schema; **→ struct** — offset
in the option struct (`opt`) and stored width; **default** — what the parser stores when the field is absent; **notes** —
the enum converter applied (§7.2), "SGS only" = read only when `sgs` = 1 (absent from the TFLite-model run), and the use
of the field where a kernel's reads show it. Values are stored unconverted unless a converter is named (floats such as
Softmax `beta`, FakeQuant `min` / `max`, LeakyRelu `alpha` are copied as raw IEEE words). Struct sizes are
`sgs_utility_GetOptionsSize` (§6.6).

The uses noted for Conv2D / DepthwiseConv2D come from `conv_exec_layer_callback_helper` (`CONV_POOL/conv_pool_ipu.c`,
asserts at lines 0x88 / 0xB9) and the conv engine's reads; those for NMSOptions from the NMS kernel's parameter record
(34 words, `TFLite_Detection_NMS`, §7.4) [R: positions; I: the names, by the FlexBuffer key that fills the same record
word]; how the kernels compute with them is `ipu-firmware-2-engines.md` §10. In Conv2D and DepthwiseConv2D the bytes
+0x4…+0x13 are never written by the parser; whether a kernel fills them is not traced. The same holds for the tails of
the LUT types' structs behind their last parsed field (Exp, Sqrt +0x308…+0x31F; Logistic +0x308…+0x30F; Atan2
+0x2F0…+0x30F) [R: parser stores]. Fields marked "—" without a use note have no name in the binary and no
traced reader: their meaning is not known.

**1 Conv2DOptions** — struct 0x390 B, fields read: 42

| f | name | read | → struct | default | notes |
|---|---|---|---|---|---|
| 0 | `padding` [S] | i8 | +0x0 (4 B) | 0 | pad conv. |
| 1 | `stride_w` [S] | u32 | +0x14 (4 B) | 0 |  |
| 2 | `stride_h` [S] | u32 | +0x18 (4 B) | 0 |  |
| 3 | `fused_activation_function` [S] | i8 | +0x1C (4 B) | 0 | activation conv. |
| 4 | `dilation_w_factor` [S] | u32 | +0x20 (4 B) | 1 |  |
| 5 | `dilation_h_factor` [S] | u32 | +0x24 (4 B) | 1 |  |
| 6 | — | u32 | +0x28 (4 B) | 0 | SGS only. |
| 7 | — | u32 | +0x2C (4 B) | 0 | SGS only. |
| 8 | — | u32 | +0x30 (4 B) | 0 | SGS only. |
| 9 | — | u32 | +0x34 (4 B) | 0 | SGS only. |
| 10 | — | u32 | +0x38 (4 B) | 0x10 | SGS only. |
| 11 | — | u32 | +0x3C (4 B) | 0x10 | SGS only. |
| 12 | — | u32 | +0x40 (4 B) | 0x10 | SGS only. |
| 13 | — | u32 | +0x44 (4 B) | 0x10 | SGS only. |
| 14 | — | u32 | +0x48 (4 B) | 0 | SGS only. `s32PartitionConvIpuMode` (conv mode); the first-layer path checks == 4 `EN_IPU_CONV_MODE_PIXELMODE`. |
| 15 | — | u32 | +0x4C (4 B) | 0 | SGS only. **loop order**: 1 → `Conv_FFDIDOHW`, 2 → `Conv_FFDIHWDO`, else "invalid parameter looporder %d" and assert. |
| 16 | — | u32 | +0x50 (4 B) | 0 | SGS only. **KB reuse mode** [I: "reuse"]. |
| 17 | — | u32 | +0x54 (4 B) | 1 | SGS only. |
| 18 | — | u32 | +0x58 (4 B) | 0 | SGS only. |
| 19 | — | u32 | +0x5C (4 B) | 0 | SGS only. |
| 20 | — | u32 | +0x60 (4 B) | 0 | SGS only. **IB reuse mode** [I: "reuse"]. |
| 21 | — | u32 | +0x64 (4 B) | 2 | SGS only. |
| 22 | — | u32 | +0x68 (4 B) | 0 | SGS only. |
| 23 | — | u32 | +0x6C (4 B) | 0 | SGS only. |
| 24 | — | u32 | +0x70 (4 B) | 2 | SGS only. |
| 25 | — | u32 | +0x74 (4 B) | 0 | SGS only. |
| 26 | — | u32 | +0x78 (4 B) | 0 | SGS only. |
| 27 | — | u32 | +0x7C (4 B) | 0 | SGS only. |
| 28 | — | u32 | +0x80 (4 B) | 0 | SGS only. |
| 29 | — | u32 | +0x84 (4 B) | 0 | SGS only. |
| 30 | — | vector | slot group `stPartitionKBGroupsSlot`: u32[32] at +0x88, count +0x108 | | SGS only. Count = KB buffer depth in slots. |
| 31 | — | vector | slot group `…IBGroupsSlot` +0x10C, count +0x18C | | SGS only. Count = IB buffer depth in slots. |
| 32 | — | vector | slot group `…OBGroupsSlot` +0x190, count +0x210 | | SGS only. Count = OB buffer depth in slots. |
| 33 | — | vector | slot group `…ABGroupsSlot` +0x214, count +0x294 | | SGS only. Count = AB (accumulator) buffer depth in slots. |
| 34 | — | vector | slot group `…VBGroupsSlot` +0x298, count +0x318 | | SGS only. |
| 35 | — | u32 | +0x31C (4 B) | 0 | SGS only. Signed accumulator shift. |
| 36 | — | u32 | +0x320 (4 B) | 0 | SGS only. Per-layer scale of the 0xB8 output form. |
| 37 | — | u32 | +0x324 (4 B) | 0 | SGS only. Output shift. |
| 38 | — | i16 | +0x328 (2 B) | 0 | SGS only. Loop / accumulation selector, compared with 2 and 4; passed as the last argument of `Conv_FFDIDOHW`. |
| 39 | — | u64 | +0x330 (8 B) | 0 | SGS only. |
| 40 | — | vector of tables | `kb_tile_info` [Kb_Tile_Info] → records at `kbCursor`; count +0x338, pointer +0x33C | | SGS only. |
| 41 | — | vector | [u32] → 16 words at +0x344 (fixed 0x40-byte copy) | | SGS only. |

**2 DepthwiseConv2DOptions** — struct 0x390 B, fields read: 43

| f | name | read | → struct | default | notes |
|---|---|---|---|---|---|
| 0 | `padding` [S] | i8 | +0x0 (4 B) | 0 | pad conv. |
| 1 | `stride_w` [S] | u32 | +0x14 (4 B) | 0 |  |
| 2 | `stride_h` [S] | u32 | +0x18 (4 B) | 0 |  |
| 3 | `depth_multiplier` [S] | u32 | +0x1C (4 B) | 0 |  |
| 4 | `fused_activation_function` [S] | i8 | +0x20 (4 B) | 0 | activation conv. |
| 5 | `dilation_w_factor` [S] | u32 | +0x24 (4 B) | 1 |  |
| 6 | `dilation_h_factor` [S] | u32 | +0x28 (4 B) | 1 |  |
| 7 | — | u32 | +0x2C (4 B) | 0 | SGS only. |
| 8 | — | u32 | +0x30 (4 B) | 0 | SGS only. |
| 9 | — | u32 | +0x34 (4 B) | 0 | SGS only. |
| 10 | — | u32 | +0x38 (4 B) | 0 | SGS only. |
| 11 | — | u32 | +0x3C (4 B) | 0x10 | SGS only. |
| 12 | — | u32 | +0x40 (4 B) | 0x10 | SGS only. |
| 13 | — | u32 | +0x44 (4 B) | 0x10 | SGS only. |
| 14 | — | u32 | +0x48 (4 B) | 0x10 | SGS only. |
| 15 | — | u32 | +0x4C (4 B) | 0 | SGS only. `s32PartitionConvIpuMode` (conv mode), checked == 2 `EN_IPU_CONV_MODE_DEPTHWISE`. |
| 16 | — | u32 | +0x50 (4 B) | 0 | SGS only. |
| 17 | — | u32 | +0x54 (4 B) | 0 | SGS only. KB reuse mode [I: from the Conv2D counterpart]. |
| 18 | — | u32 | +0x58 (4 B) | 1 | SGS only. |
| 19 | — | u32 | +0x5C (4 B) | 0 | SGS only. |
| 20 | — | u32 | +0x60 (4 B) | 0 | SGS only. |
| 21 | — | u32 | +0x64 (4 B) | 0 | SGS only. IB reuse mode [I: from the Conv2D counterpart]. |
| 22 | — | u32 | +0x68 (4 B) | 2 | SGS only. |
| 23 | — | u32 | +0x6C (4 B) | 0 | SGS only. |
| 24 | — | u32 | +0x70 (4 B) | 0 | SGS only. |
| 25 | — | u32 | +0x74 (4 B) | 2 | SGS only. |
| 26 | — | u32 | +0x78 (4 B) | 0 | SGS only. |
| 27 | — | u32 | +0x7C (4 B) | 0 | SGS only. |
| 28 | — | u32 | +0x80 (4 B) | 0 | SGS only. |
| 29 | — | u32 | +0x84 (4 B) | 0 | SGS only. |
| 30 | — | u32 | +0x88 (4 B) | 0 | SGS only. |
| 31 | — | vector | slot group `stPartitionKBGroupsSlot`: u32[32] at +0x8C, count +0x10C | | SGS only. Count = KB buffer depth in slots. |
| 32 | — | vector | slot group `…IBGroupsSlot` +0x110, count +0x190 | | SGS only. Count = IB buffer depth in slots. |
| 33 | — | vector | slot group `…OBGroupsSlot` +0x194, count +0x214 | | SGS only. Count = OB buffer depth in slots. |
| 34 | — | vector | slot group `…ABGroupsSlot` +0x218, count +0x298 | | SGS only. Count = AB buffer depth in slots. |
| 35 | — | vector | slot group `…VBGroupsSlot` +0x29C, count +0x31C | | SGS only. |
| 36 | — | u32 | +0x320 (4 B) | 0 | SGS only. Signed accumulator shift. |
| 37 | — | u32 | +0x324 (4 B) | 0 | SGS only. Per-layer scale of the 0xB8 output form. |
| 38 | — | u32 | +0x328 (4 B) | 0 | SGS only. Output shift. |
| 39 | — | u64 | +0x330 (8 B) | 0 | SGS only. |
| 40 | — | i16 | +0x338 (2 B) | 0 | SGS only. |
| 41 | — | vector of tables | `kb_tile_info` [Kb_Tile_Info] → records at `kbCursor`; count +0x33C, pointer +0x340 | | SGS only. |
| 42 | — | vector | [u32] → 16 words at +0x348 (fixed 0x40-byte copy) | | SGS only. |

**5 Pool2DOptions** — struct 0x160 B, fields read: 19

| f | name | read | → struct | default | notes |
|---|---|---|---|---|---|
| 0 | `padding` [S] | i8 | +0x0 (4 B) | 0 | pad conv. |
| 1 | `stride_w` [S] | u32 | +0x14 (4 B) | 0 |  |
| 2 | `stride_h` [S] | u32 | +0x18 (4 B) | 0 |  |
| 3 | `filter_width` [S] | u32 | +0x1C (4 B) | 0 |  |
| 4 | `filter_height` [S] | u32 | +0x20 (4 B) | 0 |  |
| 5 | `fused_activation_function` [S] | i8 | +0x24 (4 B) | 0 | activation conv. |
| 6 | — | u32 | +0x28 (4 B) | 0 | SGS only. |
| 7 | — | u32 | +0x2C (4 B) | 0 | SGS only. |
| 8 | — | u32 | +0x30 (4 B) | 0 | SGS only. |
| 9 | — | u32 | +0x34 (4 B) | 0 | SGS only. |
| 10 | — | u32 | +0x38 (4 B) | 0 | SGS only. |
| 11 | — | u32 | +0x3C (4 B) | 0 | SGS only. |
| 12 | — | u32 | +0x40 (4 B) | 0 | SGS only. |
| 13 | — | u32 | +0x44 (4 B) | 0 | SGS only. |
| 14 | — | u32 | +0x48 (4 B) | 2 | SGS only. |
| 15 | — | u32 | +0x4C (4 B) | 0 | SGS only. |
| 16 | — | u32 | +0x50 (4 B) | 2 | SGS only. |
| 17 | — | vector | slot group `stPartitionInput0GroupsSlot`: u32[32] at +0x54, count +0xD4 | | SGS only. |
| 18 | — | vector | slot group `stPartitionOutput0GroupsSlot` +0xD8, count +0x158 | | SGS only. |

**9 SoftmaxOptions** — struct 0x300 B, fields read: 2

| f | name | read | → struct | default | notes |
|---|---|---|---|---|---|
| 0 | `beta` [S] | u32 | +0x0 (4 B) | 0 |  |
| 1 | — | table | LutInfomations → LUT block at +0x4 | |  |

**10 ConcatenationOptions** — struct 0x20 B, fields read: 5

| f | name | read | → struct | default | notes |
|---|---|---|---|---|---|
| 0 | `axis` [S] | u32 | +0x0 (4 B) | 0 |  |
| 1 | `fused_activation_function` [S] | i8 | +0x4 (4 B) | 0 | activation conv. |
| 2 | `changed_input_count` [I: line rule] | u32 | *n* → +0x8 (≤ 0x100, else "sgs_getConcatChangedInfoSize" + `ebreak`); if *n* ≠ 0: +0xC = `concatCursor`, +0x10 = `concatCursor` + align4(*n*); *n* = 0 → +0xC = +0x10 = 0 | 0 | |
| 3 | — | [u8] | *n* bytes copied to the +0xC area (read only when *n* ≠ 0) [R] | | |
| 4 | — | [u8] | *n* × 0x28 bytes copied to the +0x10 area (read only when *n* ≠ 0) | | |

**11 AddOptions** — struct 0x20 B, fields read: 2

| f | name | read | → struct | default | notes |
|---|---|---|---|---|---|
| 0 | `fused_activation_function` [S] | i8 | +0x0 (4 B) | 0 | activation conv. |
| 1 | — | vector of tables | [OpParameterConvert] → +0x4 (max 1) | | SGS only. |

**15 ResizeBilinearOptions** — struct 0x10 B, fields read: 1

| f | name | read | → struct | default | notes |
|---|---|---|---|---|---|
| 2 | `align_corners` [S] | u8 | +0x0 (1 B) | 0 |  |

**17 ReshapeOptions** — struct 0x40 B, fields read: 1

| f | name | read | → struct | default | notes |
|---|---|---|---|---|---|
| 0 | `new_shape` [S] | vector | `new_shape` [int] → +0x0 pointer to the vector data (in the model), +0x4 its length; not copied | |  |

**21 MulOptions** — struct 0x40 B, fields read: 2

| f | name | read | → struct | default | notes |
|---|---|---|---|---|---|
| 0 | `fused_activation_function` [S] | i8 | +0x0 (4 B) | 0 | activation conv. |
| 1 | — | vector of tables | [OpParameterConvert] → +0x4, +0x1C (max 2) | | SGS only. |

**23 GatherOptions** — struct 0x10 B, fields read: 1

| f | name | read | → struct | default | notes |
|---|---|---|---|---|---|
| 0 | `axis` [S] | u32 | +0x0 (4 B) | 0 |  |

**27 ReducerOptions** — struct 0x20 B, fields read: 2

| f | name | read | → struct | default | notes |
|---|---|---|---|---|---|
| 0 | `keep_dims` [S] | u8 | +0x0 (1 B) | 0 |  |
| 1 | — | table | **required** sub-table (`ReduceSumOptions` [I]; absent → null-table assert even for TFLite models): field 0 u8 → +0x4; field 1 [OpParameterConvert] (`rcshift_int32toint16_convert` [I]) → +0x8, ≥ 1 entry required, only the first used | |  |

**28 SubOptions** — struct 0x20 B, fields read: 2

| f | name | read | → struct | default | notes |
|---|---|---|---|---|---|
| 0 | `fused_activation_function` [S] | i8 | +0x0 (4 B) | 0 | activation conv. |
| 1 | — | vector of tables | [OpParameterConvert] → +0x4 (max 1) | | SGS only. |

**29 DivOptions** — struct 0x40 B, fields read: 2

| f | name | read | → struct | default | notes |
|---|---|---|---|---|---|
| 0 | `fused_activation_function` [S] | i8 | +0x0 (4 B) | 0 | activation conv. |
| 1 | — | vector of tables | [OpParameterConvert] → +0x4, +0x1C (max 2) | |  |

**30 SqueezeOptions** — struct 0x10 B, fields read: 1

| f | name | read | → struct | default | notes |
|---|---|---|---|---|---|
| 0 | `squeeze_dims` [S] | vector | `squeeze_dims` [int] → +0x0 pointer, +0x4 length; not copied | |  |

**32 StridedSliceOptions** — struct 0x20 B, fields read: 5

| f | name | read | → struct | default | notes |
|---|---|---|---|---|---|
| 0 | `begin_mask` [S] | u32 | +0x0 (4 B) | 0 |  |
| 1 | `end_mask` [S] | u32 | +0x4 (4 B) | 0 |  |
| 2 | `ellipsis_mask` [S] | u32 | +0x8 (4 B) | 0 |  |
| 3 | `new_axis_mask` [S] | u32 | +0xC (4 B) | 0 |  |
| 4 | `shrink_axis_mask` [S] | u32 | +0x10 (4 B) | 0 |  |

**33 ExpOptions** — struct 0x320 B, fields read: 2

| f | name | read | → struct | default | notes |
|---|---|---|---|---|---|
| 0 | — | table | LutInfomations → LUT block at +0x0 | |  |
| 1 | — | vector of tables | [OpParameterConvert] → +0x2F0 (type +0x2F0, scale +0x2F4, scale2 +0x2F8, shift +0x2FC, offset +0x300, rcshift +0x304) (max 1), when the table's vtable has the field | | Used by the LUT kernel. |

**35 SplitOptions** — struct 0x2A0 B, fields read: 4

| f | name | read | → struct | default | notes |
|---|---|---|---|---|---|
| 0 | `num_splits` [S] | u32 | +0x0 (4 B) | 0 |  |
| 1 | — | u32 | +0x4 (4 B) | 0 |  |
| 2 | — | vector | [u32] → 4 words at +0x8 (fixed copy) | |  |
| 3 | — | vector | [u32] → 160 words at +0x18 (fixed copy) | |  |

**37 CastOptions** — struct 0x30 B, fields read: 5

| f | name | read | → struct | default | notes |
|---|---|---|---|---|---|
| 0 | `in_data_type` [S] | i8 | +0x0 (4 B) | 0 |  |
| 1 | `out_data_type` [S] | i8 | +0x4 (4 B) | 0 |  |
| 2 | — | i16 | +0x8 (2 B) | 0 |  |
| 3 | — | i16 | +0xA (2 B) | 0 |  |
| 4 | — | vector of tables | [OpParameterConvert] → +0xC (max 1) | | SGS only. |

**58 FakeQuantOptions** — struct 0x10 B, fields read: 4

| f | name | read | → struct | default | notes |
|---|---|---|---|---|---|
| 0 | `min` [S] | u32 | +0x0 (4 B) | 0 |  |
| 1 | `max` [S] | u32 | +0x4 (4 B) | 0 |  |
| 2 | `num_bits` [S] | u32 | +0x8 (4 B) | 0 |  |
| 3 | `narrow_range` [S] | u8 | +0xC (1 B) | 0 |  |

**59 PackOptions** — struct 0x2A0 B, fields read: 5

| f | name | read | → struct | default | notes |
|---|---|---|---|---|---|
| 0 | `values_count` [S] | u32 | +0x0 (4 B) | 0 |  |
| 1 | `axis` [S] | u32 | +0x4 (4 B) | 0 |  |
| 2 | — | u32 | +0x8 (4 B) | 0 |  |
| 3 | — | vector | [u32] → 4 words at +0xC (fixed) | |  |
| 4 | — | vector | [u32] → 160 words at +0x1C (fixed) | |  |

**64 UnpackOptions** — struct 0x2A0 B, fields read: 5

| f | name | read | → struct | default | notes |
|---|---|---|---|---|---|
| 0 | `num` [S] | u32 | +0x0 (4 B) | 0 |  |
| 1 | `axis` [S] | u32 | +0x4 (4 B) | 0 |  |
| 2 | — | u32 | +0x8 (4 B) | 0 |  |
| 3 | — | vector | [u32] → 4 words at +0xC (fixed) | |  |
| 4 | — | vector | [u32] → 160 words at +0x1C (fixed) | |  |

**74 ResizeNearestNeighborOptions** — struct 0x10 B, fields read: 1

| f | name | read | → struct | default | notes |
|---|---|---|---|---|---|
| 2 | — | u8 | +0x0 (1 B) | 0 |  |

**75 LeakyReluOptions** — struct 0x40 B, fields read: 2

| f | name | read | → struct | default | notes |
|---|---|---|---|---|---|
| 0 | `alpha` [S] | u32 | +0x0 (4 B) | 0 |  |
| 1 | — | vector of tables | [OpParameterConvert] → +0x4, +0x1C (max 2) | | SGS only. |

**77 MirrorPadOptions** — struct 0x10 B, fields read: 1

| f | name | read | → struct | default | notes |
|---|---|---|---|---|---|
| 0 | `mode` [S] | i8 | +0x0 (4 B) | 0 | mirror-pad conv. |

**79 SplitVOptions** — struct 0x2A0 B, fields read: 1

| f | name | read | → struct | default | notes |
|---|---|---|---|---|---|
| 0 | `num_splits` [S] | u32 | +0x0 (4 B) | 0 |  |

**80 UniqueOptions** — struct 0x10 B, fields read: 1

| f | name | read | → struct | default | notes |
|---|---|---|---|---|---|
| 0 | `idx_out_type` [S] | i8 | +0x0 (4 B) | 2 | TensorType conv. (0–11 kept, else 0xC; the log line says "activation") |

**87 ReverseSequenceOptions** — struct 0x10 B, fields read: 2

| f | name | read | → struct | default | notes |
|---|---|---|---|---|---|
| 0 | `seq_dim` [S] | u32 | +0x0 (4 B) | 0 |  |
| 1 | `batch_dim` [S] | u32 | +0x4 (4 B) | 0 |  |

**200 LogisticOptions** — struct 0x310 B, fields read: 2

| f | name | read | → struct | default | notes |
|---|---|---|---|---|---|
| 0 | — | table | LutInfomations → LUT block at +0x0 | |  |
| 1 | — | vector of tables | [OpParameterConvert] → +0x2F0 (type +0x2F0, scale +0x2F4, scale2 +0x2F8, shift +0x2FC, offset +0x300, rcshift +0x304) (max 1), when the table's vtable has the field | | Used by the LUT kernel. |

**201 ReluOptions** — struct 0x20 B, fields read: 3

| f | name | read | → struct | default | notes |
|---|---|---|---|---|---|
| 0 | — | i16 | +0x0 (2 B) | 0 | Lower clamp of the RELU instruction [I]. |
| 1 | — | i16 | +0x2 (2 B) | 0 | Upper clamp [I]. |
| 2 | — | vector of tables | [OpParameterConvert] → +0x4 (max 1) | | SGS only. |

**202 PReluOptions** — struct 0x30 B, fields read: 1

| f | name | read | → struct | default | notes |
|---|---|---|---|---|---|
| 0 | — | vector of tables | [OpParameterConvert] → +0x0, +0x18 (max 2) | | SGS only. |

**203 NMSOptions** — struct 0x40 B, fields read: 16

| f | name | read | → struct | default | notes |
|---|---|---|---|---|---|
| 0 | — | i8 | +0x0 (4 B) | 0 | DetectionType conv. Detection type (record word 0x1E when field 15 = 1). |
| 1 | — | u32 | +0x4 (4 B) | 0 | Read as **f32** *v*: coordinate scale — clip bound word 0x20 = `(unsigned)(1.0 / v)` for *v* > 0, else 0x7FFF. |
| 2 | — | u32 | +0x8 (4 B) | 0 | Read as f32 → `(int)` → word 0x19: the s16 score threshold of the CPU pre-filter. |
| 3 | — | u32 | +0xC (4 B) | 0 | Read as f32 → `(float)(int)v` → word 0x0E (`clip`). |
| 4 | — | u32 | +0x10 (4 B) | 0 | Read as f32 → `(float)(int)v` → word 0x0D (`nms`). |
| 5 | — | u32 | +0x14 (4 B) | 0 | `nms_score_threshold` (f32 bits, word 0x15) when field 15 = 1. |
| 6 | — | u32 | +0x18 (4 B) | 0 | `nms_iou_threshold` (f32 bits, word 0x16) when field 15 = 1. |
| 7 | — | u32 | +0x1C (4 B) | 0 | `max_detections` (word 0x0F) when field 15 = 1. |
| 8 | — | u32 | +0x20 (4 B) | 0 | `max_classes_per_detection` (word 0x10) when field 15 = 1. |
| 9 | — | u32 | +0x24 (4 B) | 0 | `detections_per_class` (word 0x11) when field 15 = 1. |
| 10 | — | u32 | +0x28 (4 B) | 0 | `num_classes` (word 0x12) when field 15 = 1. |
| 11 | — | u8 | +0x2C (1 B) | 0 | Not read by the NMS kernel. |
| 12 | — | u8 | +0x2D (1 B) | 0 | `bmax_score` (word 0x13) when field 15 = 1. |
| 13 | — | u32 | +0x30 (4 B) | 0 | → word 0x17. |
| 14 | — | u32 | +0x34 (4 B) | 0 | → word 0x18. |
| 15 | — | u8 | +0x38 (1 B) | 0 | ≠ 0: the table carries the parameters (byte 0x21); = 1: fields 0, 5–10, 12 replace the FlexBuffer keys (§7.4). |

**204 PostProcessUnpackOptions** — struct 0x70 B, fields read: 5

| f | name | read | → struct | default | notes |
|---|---|---|---|---|---|
| 0 | — | i8 | +0x0 (4 B) | 0 | DetectionType conv. |
| 1 | — | u8 | +0x68 (1 B) | 0 |  |
| 2 | — | u32 | +0x4 (4 B) | 0 |  |
| 3 | — | vector of tables | [table of 3 fields] → 0xC-byte entries at +0x8 + 0xC·k, max 8 (more ignored): {0: i8 PostProcessUnpackComponentType (0–7, else 8) → +0, 1: u32 → +4, 2: u32 → +8} | |  |
| 4 | — | u8 | +0x69 (1 B) | 0 |  |

**205 RoundOptions** — struct 0x30 B, fields read: 1

| f | name | read | → struct | default | notes |
|---|---|---|---|---|---|
| 0 | — | vector of tables | [OpParameterConvert] → +0x0, +0x18 (max 2) | | SGS only. |

**206 SqrtOptions** — struct 0x320 B, fields read: 2

| f | name | read | → struct | default | notes |
|---|---|---|---|---|---|
| 0 | — | table | LutInfomations → LUT block at +0x0 | |  |
| 1 | — | vector of tables | [OpParameterConvert] → +0x2F0 (type +0x2F0, scale +0x2F4, scale2 +0x2F8, shift +0x2FC, offset +0x300, rcshift +0x304) (max 1), when the table's vtable has the field | | Used by the LUT kernel. |

**207 RSqrtOptions** — struct 0x620 B, fields read: 3

| f | name | read | → struct | default | notes |
|---|---|---|---|---|---|
| 0 | — | vector of tables | [LutInfomations] → LUT blocks at +0x0, +0x2F0 (max 2) | |  |
| 1 | — | vector of tables | [OpParameterConvert] → +0x5E0, +0x5F8 (max 2) | |  |
| 2 | — | table | table of 2 u32 fields → +0x610, +0x614; +0x618 = 1 when the table is present [R] | |  |

**208 TanhOptions** — struct 0x2F0 B, fields read: 1

| f | name | read | → struct | default | notes |
|---|---|---|---|---|---|
| 0 | — | table | LutInfomations → LUT block at +0x0 | |  |

**211 PSRoiPoolingOptions** — struct 0x10 B, fields read: 4

| f | name | read | → struct | default | notes |
|---|---|---|---|---|---|
| 0 | — | u32 | +0x0 (4 B) | 0 |  |
| 1 | — | u32 | +0x4 (4 B) | 0 |  |
| 2 | — | u32 | +0x8 (4 B) | 0 |  |
| 3 | — | u32 | +0xC (4 B) | 0 |  |

**212 CustomizedConcatenationOptions** — struct 0x10 B, fields read: 1

| f | name | read | → struct | default | notes |
|---|---|---|---|---|---|
| 0 | — | u32 | u32 → +0x0 (axis [I]) | |  |

**213 ReciprocalOptions** — struct 0x2F0 B, fields read: 1

| f | name | read | → struct | default | notes |
|---|---|---|---|---|---|
| 0 | — | table | LutInfomations → LUT block at +0x0 | |  |

**214 DmaCoefMatrixOptions** — struct 0x20 B, fields read: 1

| f | name | read | → struct | default | notes |
|---|---|---|---|---|---|
| 0 | — | vector | `as16ConvertMartix` [i16] → 6 words = 12 × i16 at +0x0 (fixed copy) | |  |

**216 AtanOptions** — struct 0x20 B, fields read: 1

| f | name | read | → struct | default | notes |
|---|---|---|---|---|---|
| 0 | — | vector of tables | [OpParameterConvert] → +0x0 (max 1) | | SGS only. |

**217 WiggleErrOptions** — struct 0xF90 B, fields read: 4

| f | name | read | → struct | default | notes |
|---|---|---|---|---|---|
| 0 | — | vector of tables | [LutInfomations] → LUT blocks at +0x0 + 0x2F0·k (max 5) | |  |
| 1 | — | vector of tables | [OpParameterConvert] → +0xEB0 + 0x18·k (max 6) | |  |
| 2 | — | vector | [u8] → 6 bytes copied to +0xF40 (fixed length) | |  |
| 3 | — | vector of tables | [OpParameterConvert] → +0xF48 + 0x18·k (max 3) | |  |

**218 Atan2Options** — struct 0x310 B, fields read: 1

| f | name | read | → struct | default | notes |
|---|---|---|---|---|---|
| 0 | — | table | LutInfomations → LUT block at +0x0 | |  |

**219 CustomFloorModOptions** — struct 0x80 B, fields read: 2

| f | name | read | → struct | default | notes |
|---|---|---|---|---|---|
| 0 | — | u16 | +0x0 (2 B) | 0 | SGS only. |
| 1 | — | vector of tables | [OpParameterConvert] → +0x4 + 0x18·k (max 5; ≥ 6 → only entry 0) | | SGS only. |

**Notes on single fields** [R unless marked]: ResizeNearestNeighborOptions (74) reads field **2** (vtable slot 2), an
index the TFLite table does not have (TFLite: 0 `align_corners`, 1 `half_pixel_centers` [S]) — so either the
converter's schema added a field 2 or the reader is the ResizeBilinear one applied to this table [I]; fields 0 and 1 are
not read. ResizeBilinearOptions (15) reads only field 2 `align_corners`, not `half_pixel_centers` (3).

### 7.4 Options from FlexBuffers (custom options) [R: decompile of the readers and emulation with real FlexBuffer maps; I: the FlexBuffers names]
The op node's `custom_options` (Operator field 5 → node +0x24 data, +0x28 length, §6.6) are FlexBuffers. Every reader
checks the root the same way: length ≥ 3, last byte = root byte width ∈ {1, 2, 4, 8}, the byte before = packed type whose
type (>> 2) must be 9 = MAP; keys are found by binary search (0x2007_6EA4, key comparator `strcmp`-like: 0x2000_F4D4 /
0x2003_A394 / 0x2004_AC6E), so the map keys must be sorted. A missing key leaves its destination at its previous value
(0 after the block `memset`). Value conversion follows FlexBuffers `AsInt32` / `AsFloat` semantics (inline and indirect
ints / uints / floats of width 1, 2, 4, 8; doubles narrowed by 0x2007_6C5C). The four kernel readers below and
`SGS_IPU_TfParseOption` are the only callers of the key search 0x2007_6EA4; custom ops not listed read no FlexBuffer keys.

**210 RoiPoolingOptions** (custom code `RoiPooling`; struct 0x20; both a FlexBuffer and the option table) [R: emulation]:

| source | key / field | type | → struct |
|---|---|---|---|
| FlexBuffer (`_ROIPooing_GetOptionsFromFlexbuffer`; missing or not a map → assert `model_parser.c` 0xD35) | `spatial_scale` | float (FLOAT or INDIRECT_FLOAT only; an int value is ignored) | +0x0 (f32) |
| table | field 1 (`in_vbslot_count`, accessor string `tflite_RoiPoolingOptions_in_vbslot_count_get`, line 0x10C6) | u32 | +0x4 |
| table | field 2 | u32 | +0x8 |
| table | field 3 | u8 | +0xC |
| table | field 4 — RoiPoolingStrategy | i8, 0–4 kept, other → 4 + "Unknown RoiPoolingStrategy:%d(%s)" | +0x10 |

Field 0 of RoiPoolingOptions is not read. The table is mandatory (null-table assert on field 1). The kernel implements
only strategy 0 (BestPolicy; 1–3 assert), and BestPolicy scales the ROI coordinates by the **ROI tensor's quantisation
scale**, not by `spatial_scale`; PSRoiPooling uses its option +0x00 as the scale (`ipu-firmware-2-engines.md` §10.17, RoiPooling
/ PSRoiPooling).

**215 JointMicroPhaseGenOptions slot = `SGS_LSTM`** (custom code `SGS_LSTM` sets type 0xD7, §7.1; struct 0x10) [R:
emulation]: FlexBuffer map (missing / not a map → assert `_Lstm_GetOptionsFromFlexbuffer`,
line 0xDA1) with keys `C0` (STRING → +0x0 = pointer to the string inside the custom options) and `SubNet` (STRING →
+0x4). Then the parser walks the model's subgraphs (node +0x29C → a subgraph record, its +0x106C → the model record, whose
+0x10 = subgraph count and +0x14 = the array of 0x1070-byte subgraph records, §6.3) for the one whose name (+0x1060)
equals `SubNet`: it marks that subgraph's byte +0x105C = 1, stores its index at +0x8, and copies, from the tensor named
by the op's input 4 (`node->inputs` data[4] — "meta tensor", `_Lstm_GetMetaData` asserts `u32TensorSize >
u32MetaTensorIndex` and `pstTensor->s32AlignedBufSize != 0 && pstTensor->buffer != NULL`), the tensor record's +0x38
(aligned size) and +0x3C (buffer) into the subgraph record +0x1068 / +0x1064 [R decompile; I: "input 4" — the index read
is `*(inputs + 4) + 0x10` in the decompile, i.e. element 4 of the inputs vector]. No match → +0x8 stays 0 and nothing is
marked.

**212 CustomizedConcatenationOptions** (struct 0x10) [R: emulation with node inputs]: field 0 u32 → +0x0; +0xC = number
of the op's inputs (length of the node's `inputs` vector, ≤ 0x100 else "sgs_getCustomConcatInfoSize" + `ebreak`); +0x4 =
`ccatCursor`, +0x8 = `ccatCursor` + 4·*n*; field 1 [u32] → *n* words copied to +0x4's area; field 2 [u32] → *n* words
to +0x8's area.

**Custom-op FlexBuffer keys read by the op kernels** (not by `SGS_IPU_TfParseOption`) [R: decompile; keys from the
string table, destinations from the stores after each lookup]:

`TFLite_Detection_NMS` (registration fn 0x2003_C9D6 → parser 0x2003_A72A, run on every invoke with the custom options;
writes a **34-word (0x88-byte) record** at 0x200A_A4E4, which the kernel copies to its stack). The parser first fills
the record from the NMSOptions table (type 203, option struct = node +0x1C; the word each field fills is in the §7.3
notes): byte 0x21, words 0x1E (default 8, the detection type), 0x17 / 0x18, 0x20, 0x19, 0x0D / 0x0E always; when field 15
= 1 the table also gives words 0x1E, 0x0F–0x13, 0x15, 0x16 and the FlexBuffer is not read; otherwise these FlexBuffer
keys are read (missing root / not a map → return with the defaults):

| key | → word | read as |
|---|---|---|
| `input_coordinate_x1`, `input_coordinate_y1`, `input_coordinate_x2`, `input_coordinate_y2` | 0, 1, 2, 3 | int (`AsInt32`, 0x2003_A462 or the same inline) |
| `input_class_idx`, `input_score_idx`, `input_confidence_idx`, `input_facecoordinate_idx` | 4, 5, 6, 7 | int |
| `output_detection_boxes_idx`, `output_detection_classes_idx`, `output_detection_scores_idx`, `output_num_detection_idx`, `output_detection_boxes_index_idx` | 8, 9, 10, 11, 12 | int |
| `nms`, `clip` | 13, 14 | via the float path (0x2007_6C5C); flags by name [I] |
| `max_detections`, `max_classes_per_detection`, `detections_per_class`, `num_classes`, `bmax_score`, `offline` | 15, 16, 17, 18, 19, 20 | int |
| `nms_score_threshold`, `nms_iou_threshold` | 21, 22 | float |

The rest of the record and the variant choice: `ipu-firmware-2-engines.md` §10.16 (NMS).

`PostProcess_Unpack` / `TFLite_YoloV3_Unpack` (`_SGS_PostProcessUnpack_LoadTensorDataToL2` 0x2004_B004,
`Custom/op_postprocess_unpack.c`): keys `x_offset`, `x_lengh`, `y_offset`, `y_lengh`, `w_offset`, `w_lengh`, `h_offset`,
`h_lengh`, `confidence_offset`, `confidence_lengh`, `scores_offset`, `scores_lengh` (ints, reader 0x2004_AD3C) build up
to six component records {type, offset, length} with type x = 0, y = 1, w = 2, h = 3, confidence = 4, scores = 5; a
component is counted only when its length > 0; `max_score` (int) is a flag [I: by name]. The option table of type 204
(§7.3) carries the same kind of component list for the SGS-converted form.

`PostProcess_Max` (0x2004_9B84, `Custom/op_postprocess_max.c`): keys `scores_lengh` (int) and `skip` (the string "skip"
at 0x2008_41E4, shared with the run-mode names); their use is the kernel's (`ipu-firmware-2-engines.md` §10.18).

## 8. The offline command block

### 8.1 What the block is and who reads it [R unless marked]
<!-- ev:ipu-firmware-1-runtime-model.8.1 -->
- **The flatbuffer.** The offline command block is the second blob of an SGSI file (§6.1), a `TFL3` flatbuffer whose
  root table has fields 0 [u8] command data, 1 u32 valid size, 2 and 3. The loader stores {data pointer, vector length =
  "max_size", valid size} at model record +0x30 / +0x34 / +0x38 ("Cmd entry data:", "addr", "max_size", "valid_size")
  and does not read fields 2 and 3. Anjoy: the vector starts at file 0xBC040 and holds 501 104 B = 125 276
  little-endian u32 words, valid size 501 104 [R]; root fields 2 and 3 hold the
  u32 values 0x18 and 0x4 at their slots, which read as offsets point at zero words (empty vectors) [R values; I: their
  type]. It is the precompiled part of the network: the converter emits the command stream, the firmware only replays
  it.
- **Who reads it.** The stream is a sequence of **32-bit words for the IPU's command decoder** (decoder 0 or 1, §8.7).
  The RISC-V never parses it: for an op in run mode 1 ("invoke by cmdq", §6.6) it hands the decoder the bus address
  (+0xE000_0000) and the byte size of the op's slice `{pCmdEntry = block + s32Offset, s32Size}` (both 16-byte aligned,
  asserted) and waits for the decoder's done bit (§8.7).
- **The online path speaks the same language.** Every online op kernel builds the same kind of words into a small buffer
  `{u32 *words, u32 count, u8 skip}` and pushes them one by one into the decoder's **command FIFO port** with
  `dec_push_insn(decoder, buf)` (0x2000844c) (415 call sites) → `dec_fifo_push` (0x2000a186). So the firmware's own encoders are a direct source for
  the word layouts, and the offline block is a recorded stream of those words, packed by the delta form of §8.4 [I: from
  the identical word formats; the decoder hardware itself is not visible].

### 8.2 Framing [R: the rules below parse all 125 276 words of the Anjoy block into 29 635 commands, ending exactly at the last word, with every delta mask consistent —, output]
<!-- ev:ipu-firmware-1-runtime-model.8.2 -->
The stream is a flat sequence of little-endian u32 words; there is no outer header, length prefix or checksum. Each
command starts with a header word; **bit 31 of the header selects the kind**:

| header | kind | words in the command |
|---|---|---|
| bit 31 = 0 | **decoder control word** (§8.3), opcode = bits 31:28 | op 1: 1 + ceil(((bits 27:16) + 1) / 4); op 3: 1; op 4: 2; op 2, op 7 (and any other): 1, or 2 when bit 27 is set |
| bit 31 = 1 | **engine instruction** (§8.5), class = bits 6:0 | 1 + *L*, where *L* = bits 26:23 (format A) or bits 17:13 (format B: classes 0x28, 0x30, 0x38, 0x60, 0x78, §8.5.1); ***L* = 0 → delta form (§8.4)**: 2 + (bits 31:27 of the next word) |

Origin of each rule [R]:
- op 1 length: `dec_fifo_push`'s caller `dec_push_op1` (0x20008478) pushes `((w>>16 & 0xFFF) + 4 >> 2) + 1` words for a header `w`;
- op 2 / op 7 / "other" length: `dec_push_op2` (0x200084da) pushes 2 words when header bit 27 (0x0800_0000) is set, else 1; the
  firmware sends `0x7800_0001|…` + one zero word through it (§8.3.4). Op 2 in the Anjoy block is the single word
  0x2000_0000;
- op 3: pushed alone by `dec_push_op3` (0x2000852c) (one word). Op 4: the firmware pushes 0x4034_0000 followed by one word (§8.3.5);
- instruction lengths: every firmware encoder sets its count from the same field it writes into bits 26:23
  (`hal_dma_load` 4 / 6 / 8 → 5 / 7 / 9 words, `hal_conv` 8 / 9 → 9 / 10 words, vector encoders 3 / 4 → 4 / 5 words) or,
  for class 0x38, into bits 17:13 (`convolution_IPU`: 0x6000 → 3 → 4 words). Class 0x28 uses bits 17:13 in the stream (its
  only full command has bits 17:13 = 5 and 6 words; reading bits 26:23 instead breaks framing at cmd+0x1FD8) [R stream; I:
  field choice];
- the SR class 0x7F (§8.5.6) does not follow the bits 26:23 rule (its count is per opcode); it does not occur as a header
  in the Anjoy block.

Kinds in the Anjoy block [R: parsed]: control op 1 ×863, op 3 ×939, op 7 ×62, op 2 ×1, op 4 ×1; instructions by
class (full / delta): 0x04 43 / 12 909, 0x44 1 159 / 11 098, 0x0C 26 / 321, 0x2C 9 / 604, 0x38 1 / 560, 0x28 1 / 654,
0x5B 1 / 14, 0x20 0 / 249, 0x53 0 / 9, 0x60 0 / 111. 26 529 of the 27 769 instructions (95.5 %) are deltas.

### 8.3 Decoder control words (bit 31 = 0) [R unless marked]
Common: bits 31:28 = opcode. Five opcodes are used by the firmware and / or the Anjoy block; no other value occurs in the
block. Opcodes **0, 5, 6** never occur in the block and no firmware encoder for them was found [R: search of the
decompile for header constants]; under the framing rule they would be 1 or 2 words by bit 27 — **unused**, meaning not
known.

#### 8.3.1 Op 1 — write bytes into the decoder's address space

| bits | field |
|---|---|
| 31:28 | 1 |
| 27:16 | byte count − 1 (12 bits; up to 4096 bytes) |
| 15:0 | target address (16-bit, decoder address space below) |
| following words | the bytes, little-endian, packed from the first data word; the last word is zero-padded |

Encoders: `vrbuf_init_bufid` 0x2000_C102, `_vrbuf_init_bufid` 0x2000_C5E4, `set_customer_vr_tbl2` 0x2000_CF40,
`set_customer_mr_tbl` 0x2000_D048, `convolution_IPU` 0x2006_AA06, `lut_build_write_a` (0x2006fda4), `lut_build_write_b` (0x2006fe34), `dma_color_matrix` (0x2006b260); push
`dec_push_op1`. `dec_push_op1` classifies the range [address, address + count] with `dec_classify_range` (0x200083e2) and, when it touches
region 1, drops the firmware's cache of SR register values (`sr_cache_clear_all` (0x200088de)) before pushing — the firmware knows such a
write changes SR registers behind its back.

Decoder address space as the firmware uses it [R: `dec_classify_range` region codes; addresses from the encoders; Anjoy
counts from its decoded command block]:

| address | `dec_classify_range` code | what is written there | Anjoy |
|---|---|---|---|
| 0x0000 + 0x100·*b* (*b* = VR buffer id 0…0x10) | 4 (0x0000–0x3FFF) | VR buffer *b* conv mapping table: one u16 per depth slot, ≤ 32 entries (`depth_inslots < MAPPING_TABLE_C…`) | ids 8, 0xC–0x10 |
| 0x0080 + 0x100·*b* | 4 | VR buffer *b* DMA / vector mapping table (`_set_dma_vec_mapping_table`): width × depth u16, ≤ 32 entries (type-4 buffers: +16 entries) | ids 0–5, 8, 0xC–0xF |
| 0x1100 + 0x100·*m* (*m* = MR buffer id 0…7) | 4 | MR buffer *m* table, ≤ 16 u16 (`set_customer_mr_tbl`) | *m* = 0, 1 |
| 0x1900 | 4 | conv pad value = the input tensor's zero point, u16 (4-bit inputs: the nibble duplicated) (`convolution_IPU`, `s16ZeroPoint`) | — |
| 0x1908 | 4 | 0, written by the firmware after every DMA layout op (`dma_op_end` (0x2002fffe)); an unused path writes *id* \| 0x8000_0000; meaning not known | ×19, 4 bytes |
| 0x4000–0x7FFF | 1 | SR register file (no firmware encoder writes here directly; region known only from the cache-drop rule) | — |
| 0x8000 / 0x9000 | 2 (0x8000–0x9FFF) | LUT unit 0 / 1 (`table_id` 0 / 1): `table_first`, 0x84 B (65 entries + the last repeated) (`lut_build_write_a`) | — |
| 0x8800 / 0x9800 | 2 | LUT unit 0 / 1: `table_second`, 0x204 B (257 + 1) | — |
| 0xA000 / 0xB000 | 3 (0xA000–0xBFFF) | LUT unit 0 / 1 configuration, 20 bytes (`lut_build_write_b`) | — |
| 0xC040 | 5 (≥ 0xC000) | DMA colour matrix: DMA registers +0x40…+0x54, 24 bytes, header 0x1017_C040 (`dma_color_matrix`, from DmaCoefMatrix and from a first conv layer whose option +0x20 is set; §6.3) | ×1, 24 bytes, cmd+0x114 |

The LUT addresses are the LUT descriptor's words +0x0C…+0x20 (0x2009_2EC0: 0x8000, 0x8800, 0x9000, 0x9800, 0xA000,
0xB000), used as op-1 target addresses, not as offsets from the LUT register base 0x0102_0000; the same LUT memory is
CPU-visible at decoder address + 0x0101_8000. The VR / MR meaning of the 0x0000–0x1FFF tables is from `ipu_vrbuf.c` [R
names]; the buffers, the configuration layout and the colour-matrix packing are `ipu-firmware-2-engines.md` §4, §8.3, §10.19. The Anjoy
write at 0xC040 is the block's first data write, 6 words of 0x0400 / 0x1000 / 0x0100 halfwords — 1024, zero in
sign-magnitude form, and the bias 256 of the Anjoy colour matrix (§6.3) [I: decoded with the packing rule].

#### 8.3.2 Op 3 — wait for a resource (barrier)
One word, `0x3000_0000 | id`. Ids used: **0** (all) before rewriting a VR table and at the end of a block; **0x20 +
*b*** before rewriting VR buffer *b*'s tables (`_vrbuf_init_bufid`, `conv_barriers` (0x2000d118)); **0x40 + *m*** before rewriting MR
buffer *m* (`set_customer_mr_tbl`); **0x182** and **0x162** (bit 8 set) before rewriting the LUT tables (the LUT load);
bits 27:9 are zero in every use. The firmware always sends it immediately before an op-1 write to the same buffer's
tables, so it reads as "drain everything that still uses resource *id*" [I]. Anjoy: 0x2D ×186, 0x2F ×124, 0x28 ×124,
0x20–0x25 ×62 each, 0 ×131, 0x40 and 0x41 once.

#### 8.3.3 Op 2
One word 0x2000_0000, once in the Anjoy block (cmd+0x7A5A0, just before the closing op 3 / op 4). No firmware encoder
builds it. Meaning not known [I: an end-of-program marker for the engines].

#### 8.3.4 Op 7 — conv-layer start marker
Two words: `0x7800_0001 | (b & 0xFF) << 4`, then 0 [R: `conv_layer_marker` (0x2000bfda), pushed by `dec_push_op2`, which sends 2 words
because bit 27 is set]. *b* = the VR buffer id that `vrbuf_get_bufid(1)` returns (the conv input buffer). Sent once per
conv layer by `Conv_FFDIDOHW` 0x2006_B6D2, `DepthWise_Conv_FFDIHWDO` 0x2006_C220, `Conv_FFDIHWDO` 0x2006_CB22; the
caller's first argument (a u16 from the conv record +0x3E) is ignored. Anjoy: 0x7800_00D1 + 0 ×62 = once per CONV_2D of
the model (62), *b* = 0xD. Effect on the hardware not known [I: selects / resets the VR buffer the conv engine reads].

#### 8.3.5 Op 4 — write a decoder register (end of block)
Two words: `0x4034_0000`, then a u32 value. The firmware's own "wait for idle" sends `0x3000_0000, 0x4034_0000, 0`
through the command port and then spins on the decoder's done bit (decoder 0: ctrl +0x6C bit 0, cleared by setting +0x74
bit 0; decoder 1: +0xD8 / +0xE0) (`dec0_drain` (0x20008566), `dec1_drain` (0x2000a582)). The Anjoy block ends with `0x2000_0000, 0x3000_0000,
0x4034_0000, 0x0000_0001` — the value is the op's `u32Fence` (1, RunMode of operator 0) [R values]. So op 4 with bits
27:16 = 0x034 writes the following word into the decoder's fence / done register, whose write raises the done bit the
RISC-V polls [I: from the firmware's use; the register's name is not in the binary, so not known].

### 8.4 Delta form of an engine instruction (offline block only)
<!-- ev:ipu-firmware-1-runtime-model.8.4 -->
- An engine instruction whose length field is 0 is followed by a **mask word**: bits 31:27 = *n*, bits 26:0 = a mask with
  exactly *n* bits set; then *n* words. Mask bit *i* set ⇔ instruction body word *i* + 1 is replaced by the next value,
  in increasing *i* [R: all 26 529 deltas of the Anjoy block have popcount(mask) = *n*; *n* = 0 (header alone, body
  unchanged) occurs, e.g. class 0x53 / 0x5B]. Mask bits used: up to bit 8 (body word 9) [R].
- The delta's **header word is a complete header**: its register-id and flag fields differ from the previous
  instruction's (e.g. 0x8004_A0A8 full, then 0x8008_00A8 delta; 0x84BD_0D2C full, then 0x807D_0D2C / 0x803D_0D2C deltas)
  [R]. Only the body is inherited.
- **Base.** The body words not in the mask keep their values from an earlier instruction. Tested on the Anjoy block (expanded conv word 6 = `FxNum` / `FyNum`, §8.5.3, compared with the kernel size
  of the model layer the command belongs to; layer = number of op-7 markers seen, §8.3.4) [R test; I: conclusion]: base
  kept per class (0x0C and 0x2C separate) — 764 match / 196 mismatch; **one base shared by the conv classes 0x0C and
  0x2C** — 936 / 24; one base for all classes — 798 / 162. The 24 are all in layer 1 (3×3, stride 2, 16 → 24 channels)
  and are not mismatches: that layer runs in conv mode 3 (9 taps), whose `FxNum` / `FyNum` = (⌈3/3⌉ − 1)·dilation = 0
  (`ipu-firmware-2-engines.md` §6.2, conv engine). So the decoder keeps the last body **per engine**; at least the conv classes
  0x0C / 0x2C (and by the same encoder 0x8C) share one [I]. Whether DMA load (0x04) and store (0x44) share a base is not
  decided: the alignment test of the expanded EMI address gives the same result both ways [R test]. Whether the vector
  classes share bases across classes, and whether a full instruction shorter than the body store clears or keeps the
  remaining words, are not known.
- **Length.** The delta header has no length, and deltas patch words beyond the body length of the last full instruction
  of the same class (per-class bases: 104 DMA-load, 10 DMA-store and 134 class-0x2C deltas patch a word beyond it; every
  grouping of classes tried leaves such deltas) [R]. So the body store is
  fixed-size per engine and a full instruction's length only says how many words follow; the mode the engine runs in is
  coded in body bits (DMA: word 2 bits 20 / 21, §8.5.2), not in the length [I].
- **Initial state.** Classes 0x20, 0x53 and 0x60 occur only as deltas in the Anjoy block; their first delta leaves body
  words unset (0x20: word 3 unset by the first, word 4 never set; 0x60: word 3 unset by the first, words 2 and 4 never
  set) [R]. The base at the start of a block is therefore a defined reset state; that
  it is zero is not known [I: zero is the simplest reading; the line-mode trigger pulses the decoder control word +0x00 =
  1 → 0 before every block, §8.7, which may be that reset].
- The firmware's online encoders never produce the delta form (every encoder sets a non-zero length or, for SR class
  0x7F, an opcode-implied one) [R]; it exists only in converter output.

### 8.5 Engine instructions (bit 31 = 1)

#### 8.5.1 Header word and classes [R unless marked]
<!-- ev:ipu-firmware-1-runtime-model.8.5.1 -->

| bits | field |
|---|---|
| 31 | 1 = engine instruction (every encoder ORs 0x8000_0000) |
| 30:27 | class-specific (DMA: queue id + one tensor-index bit, §8.5.2; conv: the fifth-vector flag at 30, §8.5.3; vector classes: a 3-bit field at 29:27 and a flag at 30) |
| 26:23 | **length *L*** = number of body words that follow, for format-A classes; zero for format-B classes |
| 22:18 | register / buffer id C (5 bits; written as `idx & 0xF` at 21:18 and `bank & 1` (for conv buffers the ping-pong index) at bit 22) |
| 17:13 | register / buffer id B for format A; **length *L*** for format B |
| 12:8 | register / buffer id A (`idx & 0xF` at 11:8, bank / ping-pong bit 12) |
| 7 | variant flag of the class (conv: output target; 0x38: output form; 0x5B: average pooling; 0x78: the 0xF8 form; SR: operand form) |
| 6:0 | **class** (framing and engine) |

The byte 7:0 is the opcode byte; a class written with bit 7 set (0xAC, 0xB8, 0xDB, 0xF8 …) is the same class in its
variant form. Format B classes (length in 17:13, registers only A and C): **0x28, 0x30, 0x38, 0x60, 0x78** (their
encoders set 0xA000 / 0x6000 / 0x12000 there and keep 26:23 zero; stream framing). Every other class seen is format A,
except the SR class 0x7F whose length depends on the opcode in bits 31:23 (§8.5.6). A heuristic scan found 183 encoder sites and the classes 0x02 … 0x7E with their format, full lengths and encoder
functions [R: the scan misreads some sites; every mismatch was checked by hand and was a scanner error].

Classes and their firmware encoders (the ones the Anjoy block uses in **bold**):

| class | words | firmware encoder(s) | used by |
|---|---|---|---|
| **0x04** | 5 / 7 / 9 | `hal_dma_load` 0x2000_8FB4 | EMI → L2 load (§8.5.2) |
| **0x44** | 5 / 7 / 9 | `hal_dma_store` 0x2000_9916 | L2 → EMI store (§8.5.2) |
| **0x0C**, **0x2C**, 0x8C | 9 / 10 | `hal_conv` 0x2003_2684 | conv / pool engine (§8.5.3); the opcode byte gives the output target |
| 0xAC (**0x2C** with bit 7) | 9 | `resize_conv_form` (0x20033072) ← `_SGS_ConvBilinear` (`DMA/IPU200M/drv_resizebilinear_ipu200.c`) | resize through the conv engine (word 8 bit 13 = nearest mode: 1 from `SGS_OPS_ResizeNearest`, 0 from `SGS_OPS_ResizeBilinear`); 52 Anjoy instructions of class 0x2C are this form, its two RESIZE_NEAREST_NEIGHBOR |
| **0x38** (0xB8) | 4 | `convolution_IPU` 0x2006_AA06 | conv output stage (§8.5.4) |
| **0x5B** / 0xDB | 9 | `vp1_vPoolingAvg_hal` 0x2006_E022 (inlined `vp1_vPoolingMax_hal` → 0x5B, `vp1_vPoolingAvg_hal` / `vp1_vPoolingAvgMean_hal` → 0xDB) | 0x5B max pooling — the 15 Anjoy instructions sit between the op-7 markers of conv layers 28 and 29 and are its three MAX_POOL_2D (5×5, 15×25×68); 0xDB average / average-mean pooling |
| **0x28** | 6 / 14 | `SGS_IPU_HAL_Convert16BitTo8Bit(ForMathOp)`, `…8BitTo16BitForMathOp`, `SGS_Logistics_IPUExeTile`, `SGS_CustomFloorMod_IPUExeHeader`, `bcast_addsub_kernel` (0x2006d7fe); 14 words: broadcast ADD / SUB, EXP / LOGISTIC | data-type conversion with requantisation |
| 0x30 | 6 | `SGS_IPU_HAL_Convert8BitTo16Bit`, `relu_kernel` (0x200534a2) (RELU family) | u8 → s16 conversion; 0xB0 with operation 7 = the RELU family's clamp |
| **0x20** | 4 / 6 | `wiggleerr_kernel` (0x200673ca), `nms_mul_step` (0x200692a2), `addsub_kernel` (0x2006d6a4) (also ADD / SUB), `mul_kernel` (0x2006d942), `_sgs_PRelu_IPUExeTile` | vector element-wise |
| **0x53** (0xD3) | 4 | `reduce_d3_helper` (0x20063de4), `nms_d3_step` (0x20068ad6), argmax / reduce-max kernels | vector reduce; 0xD3 carries sub-op 1 in w1 22:16 |
| **0x60** (0xE0, and 0xA0) | 6 / 10 | `round_kernel` (0x2001d33c) (ROUND), `SGS_IPU_IPUExeTile` (CAST), `_sgs_PRelu_IPUExeTile`, `wiggleerr_neg_step` (0x20066c3c), `requant_mul_u16_kernel` (0x2006728a), `mul_kernel`, `prelu_kernel` (0x2006dc00), `div_kernel` (0x2006dd1a), FloorMod, PhaseGen; MUL and DIV (0xA0 / 0xE0 / 0x60) | vector element-wise with requantisation (10 words: parameter blocks in w6+) |
| 0x78 | 21 (*L* = 20) | `SGS_Lut_Impl` 0x2006_80AA (logistic, tanh, sqrt, rsqrt, exp, reciprocal, atan2, wiggle) | LUT unit |
| 0xF8 (0x78 with bit 7) | 5 | `fix2float_insn` (0x2003ca86) (Fix2Float only) | s16 → f32 conversion |
| 0xD8 | 6 | `leaky_relu_kernel` (LEAKY_RELU; op entry 0x2005_3616 → 0x2005_C876) | |
| 0x00 | 4 | softmax | normalising multiply by an SR register |
| 0x73 (0xF3) | | `SGS_OPS_CopyDataInL2` | L2 → L2 copy |
| 0x43 (0xC3) | | includes `SGS_DMA_FillMemory` (0x20030180)'s fill (0xC3 = fill) | |
| 0x82 | | | lane maximum with its index into SR 6 / 8 |
| 0x13 | | | with w1 bit 24: select between A and B |
| 0x7F | opcode-implied | `drv_ipu_sr_*` / `hal_ipu_sr_*` (§8.5.6) | SR scalar register file |
| others | 4 / 5 / 6 | 0x01, 0x02, 0x05, 0x09, 0x12, 0x16, 0x1A, 0x1E, 0x21, 0x23, 0x25, 0x26, 0x2A, 0x2E, 0x32, 0x36, 0x3A, 0x3E, 0x3F, 0x41, 0x46, 0x4A, 0x4E, 0x52, 0x5A, 0x66, 0x6D, 0x7E; further class bytes 0x10, 0x45 / 0xC5, 0x56 / 0xD6, 0x5E, 0xA1, 0xA3, 0xA5, 0xAA, 0xAE, 0xB2, 0xBA, 0xBE, 0xC1, 0xC6, 0xCA, 0xCE, 0xD2, 0xDA, 0xED — vector-unit ops of the custom / element-wise / reduce kernels | not in the Anjoy block |

The length and written fields of every vector-unit class, and the predicate semantics of 0x12 / 0x16 / 0x1A / 0xD2 /
0xDA, are `ipu-firmware-2-engines.md` §7.8, §7.4 (vector unit). The ops of the Anjoy block map to classes as its model leads one to
expect (62 CONV_2D, 3 MAX_POOL_2D, 7 ADD, 4 MUL, 13 CONCATENATION, 6 TRANSPOSE, 2 RESIZE_NEAREST_NEIGHBOR, 3
Reshape_Alias): conv + output stage + DMA for the convs, 0x5B for the pools, the 0xAC form for the resizes, 0x20 / 0x28 /
0x53 / 0x60 for the element-wise ops and requantisation, DMA alone for concatenation / transpose [I: by class counts and
the encoders' users; the per-op slices are not delimited inside the block, §8.6].

#### 8.5.2 DMA load 0x04 / store 0x44 [R: `hal_dma_load` 0x2000_8FB4, `hal_dma_store` 0x2000_9916; field names from their asserts]
The descriptor is an engine instruction of the decoder command stream: the encoder's last argument is the command
buffer `{words, count}`; the value 5 / 7 / 9 it sets is the **word count**, and the header's length field holds 4 / 6 /
8. Three forms by *L*: **4 → 5 words** (linear: one `total_length`), **6 → 7 words** (EMI window, no L2 window), **8 → 9
words** (EMI window and L2 window with strides). Load picks 8 when its argument 10 is 0, else 4 when argument 9
(`linear`, a `total_length` instead of a window) is set and DDR mode ≠ 2, else 6; store picks 8 when its argument 9 is 0,
else 6 when argument 8 is 0, else 4.

| word | bits | load (0x04) | store (0x44) |
|---|---|---|---|
| 0 | 6:0 | 0x04 | 0x44 |
| 0 | 12:8 | register id from the caller's 3rd argument (`dma_load_utils` passes tensor record +0x9C \| 0x10) | register id from the 2nd argument |
| 0 | 17:13 | tensor SR index *t* = arg4 − 0xF (< 64, `t < TENSOR_SR_SIZE`): bit 17 = *t* bit 0, bits 16:13 = *t* bits 5:2 | same |
| 0 | 22:18 | register / buffer id from the 2nd argument (`dma_load_utils`: a VR buffer id) | from the 3rd argument |
| 0 | 26:23 | *L* = 4 / 6 / 8 | same |
| 0 | 29:27 | `qid` (< 8; queue chosen by `hal_dma_getqid` from the DDR type, `ipu-firmware-2-engines.md` §3.2, DMA) | same |
| 0 | 30 | *t* bit 1 | same |
| 1 | 31:0 | `emi_addr` — bus address (+0xE000_0000 applied by the caller), 16-aligned; 4-aligned in DDR mode 3 | same (16-aligned) |
| 2 | 3:0 | data type (arg 6) | same |
| 2 | 7:4, 15:14, 16, 17 | extract-box parameters (bytes 2, 3, 4, 5 of the extbox record) | same (record bytes 2, 3, 5) |
| 2 | 11:8 | arg 21 (4 bits) | — |
| 2 | 13:12 | DDR mode: 0 linear, 2 windowed YUV (width/height even, channel 1, `emi_uv_addr_offset` used), 3 4-byte elements | — |
| 2 | 18 | 1 on the byte-granular paths (DDR mode 2 or 3, or bit 4 of the 3rd argument), else extbox record byte 1 | extbox byte 1 |
| 2 | 19 | — | extbox byte 0 |
| 2 | 20 | encoder flag: 1 on the windowed path whose EMI fields were converted to 16-byte units, 0 in the linear form [R paths; I: meaning] | store: flag from its own path |
| 2 | 21 | 1 in the 9-word form (the encoder writes `!arg10` / `!arg9`): L2 window and strides present | same |
| 2 | 22, 23, 24, 25 | four 1-bit options (args 13, 14, 12, 11) | — |
| 3 | 13:0 | L2 address (arg 7) | same |
| 3 | 31:14 | `emi_window_width` (< 2^18), or `total_length` (< 2^18) in linear form | same |
| 4 | 13:0 | `emi_window_height` (< 2^14) (or extbox +8 when extbox mode 1) | `emi_window_height` |
| 4 | 31:14 | `emi_w_stride` bits 17:0 | same |
| 5 | 13:0 | `emi_window_channel` (< 2^14); DDR mode 2: bits 19:0 = `emi_uv_addr_offset` >> 4 | `emi_window_channel` |
| 5 | 31:14 | `emi_wh_stride` bits 17:0 (mode 2: bits 31:20 only) | same |
| 6 | 8:0 | `emi_w_stride` bits 26:18 (signed, ±2^26) | same |
| 6 | 17:9 | `emi_wh_stride` bits 26:18 | same |
| 6 | 22:18 | `l2_w_stride` bits 18:14 | same |
| 6 | 23 | `l2_wh_stride` bit 18 | same |
| 7 | 17:0 | `l2_window_width` (< 2^18) | same |
| 7 | 31:18 | `l2_w_stride` bits 13:0 | same |
| 8 | 13:0 | `l2_window_height` (< 2^14) | same |
| 8 | 31:14 | `l2_wh_stride` bits 17:0 | same |

Units: in the 9-word form the L2 window width and strides are given to the encoder in bytes, must be 16-aligned, and are
stored in 16-byte L2 slots (>> 4); EMI window width, strides and total length are stored >> 4 unless DDR mode 2 / 3 or
the extract box is used [R: the `& 0xF` asserts and shifts]. Store words 2 bits 22–25 and 11:8 are left as the template
had them (zero). Which words each form carries is cut by *L*: the 5-word form sends words 0–4 and the 7-word form words
0–6; the rest of the encoder's buffer is not pushed.

#### 8.5.3 Convolution / pooling engine 0x0C / 0x2C / 0x8C [R: `hal_conv` 0x2003_2684 writes the words; `hal_Conv_CompareParameter` 0x2003_2196 checks word 1+*i* against named fields — the names below are its assert strings; `CMD_LENTH` = 10]
Header: the opcode byte gives the output target — **0x0C** accumulator (AB) only, **0x2C** a vector-processor buffer
(`convolution_IPU` argument 16 = 1; always followed by a 0x38 output-stage instruction, §8.5.4), **0x8C** a conv buffer
(= 2); 12:8 = input buffer (IB) id, 17:13 = kernel buffer (KB) id, 22:18 = output buffer (OB) id (each `bufid & 0xF`
plus the buffer's **ping-pong index** as the fifth bit, `combine_bufid_type(id, pp)`); 26:23 = *L*: 9 (10 words) except
plain 0x0C with flag byte 4 of `convolution_IPU`'s argument 5 clear, which is 8 (9 words, word 9 not sent); bit 30 = that
flag — the per-tile flag for the fifth (32-bit) per-channel vector, which also forces the 10-word form of 0x0C. Hence the
Anjoy forms 0x2C × 10 words, 0x0C × 9 words.

| word | bits | field (assert name ← hardware name) |
|---|---|---|
| 1 | 13:0 | `IbBufAddr` ← nbuf_begin_addr (input address in the conv address space, `vrbuf_get_convaddr`) |
| 1 | 16:14 | `nBufSel01` ← nbuf_sel_01 (always 0 from the firmware) |
| 1 | 29:17 | `KbBufAddr` ← sbuf_begin_addr (kernel address) |
| 1 | 30 / 31 | `IbIs4Bit` / `KbIs4Bit` |
| 2 | 2:0 | `ConvMode` ← conv_mode (the SIMD shape from `hal_set_simd`, `ipu-firmware-2-engines.md` §6.2, conv engine) |
| 2 | 16:3 | `AbBufAddr` ← cbuf_begin_addr (accumulator) |
| 2 | 17 / 18 | `AbBufId` ← cbuf_id (always 0 from the firmware) / `IsInt8` |
| 2 | 28:19 | `HwTileHeight` ← h |
| 2 | 31:29 | `left_padding` ← lp |
| 3 | 10:0 / 20:11 | `Dxnum` / `Dynum` |
| 3 | 25:21 / 30:26 | `W_stride` ← sx / `H_stride` ← sy |
| 3 | 31 | `ReadAb` ← need_first_rd_cbuf (accumulate onto the accumulator) |
| 4 | 19:0 | `full_di_reqnum` |
| 4 | 29:20 | `CNum` ← c_num |
| 4 | 30 / 31 | `ReadObShift` / `ReadRcShift` |
| 5 | 3:0, 7:4, 11:8, 15:12 | `Full_di_full_do`, `Frag_di_full_do`, `Frag_di_frag_do`, `Full_di_frag_do` (tile counts − 1) |
| 5 | 31:16 | `WH` |
| 6 | 10:0 / 20:11 | `FxNum` / `FyNum` = (kernel width/height − 1) × dilation |
| 6 | 25:21 / 30:26 | `W_dilation` ← fsx / `H_dilation` ← fsy |
| 6 | 31 | `padding_not_0` |
| 7 | 19:0 | `kbbuf_reqnum` ← sbuf_req_num |
| 7 | 30:20 | `HwTileWidth` ← w |
| 7 | 31 | `CompressKb` ← kb_iscompressed |
| 8 | 12:0 / 25:13 | `di_hw` ← dihw_sub1 / `do_hw` ← dohw_sub1 |
| 8 | 28:26 / 31:29 | `top_padding` ← tp / `bottom_padding` ← bp |
| 9 | 13:0 | `ObBufAddr` ← obuf_begin_addr |
| 9 | 19:14 | `CfgTruncate` |
| 9 | 20 | `ShiftDirection` |
| 9 | 28:21 | `BiasAddr` ← bias_offset |
| 9 | 29 / 30 / 31 | `NeedBias` / `NeedScale` / `ObDataFormat` ← ob_lane_num |

Anjoy check [R: decoded command block, cmd+0x1A4, the first conv instruction, layer 0 = 3×3, stride 2]: word 3 =
0x0842_F062 → `Dxnum` 0x62, `Dynum` 0x5E, `W_stride` 2, `H_stride` 2; word 6 = 0x0420_1002 → `FxNum` 2, `FyNum` 2,
dilation 1 / 1 — matching the model's first CONV_2D (filter 16×3×3×3, 480×800 → 240×400) [R values; I: `Dxnum` /
`Dynum` as output tile extents]. After packing, `hal_conv` passes words 1–9 and the very parameters it packed them from to
`conv_compare_params` (0x2003250a), which calls `hal_Conv_CompareParameter(i, word[i+1], …)` for each word and asserts that every field
unpacks to its parameter — an unconditional self-check of the encoding (this is why the field names are known), not a
compare with the previous instruction. How the fields are derived (conv modes, tile shapes, per-channel vectors, the
0xAC resize form): `ipu-firmware-2-engines.md` §6 (conv engine).

#### 8.5.4 Conv output stage 0x38 / 0xB8 (4 words, format B) [R: `convolution_IPU` 0x2006_AA06 tail]

| word | bits | content |
|---|---|---|
| 0 | 6:0, 7 | 0x38; bit 7 set (0xB8) when the conv's first flag byte (argument 5 byte 0) is 0 |
| 0 | 17:13 | *L* = 3 |
| 0 | 21:18, 22 | output VR buffer id (`& 0xF`) and its type bit |
| 1 | 13:0 | output address in the vector-processor space (`vrbuf_conv_ob_convert_convaddr2vpaddr`) |
| 1 | 15:14 | 3 |
| 1 | 16, 22, 23 | output format bits: 0xC1_C000 (two ops, data type 3), 0x40_C000 (two ops, type 1), 0x81_C000 (one op, type 3) — `get_op_format` |
| 1 | 21:17 | shift (requantisation record byte +8) |
| 2 | 15:0 / 31:16 | requantisation record u16 +0x0E / u16 +0x10 |
| 3 | 15:0 | requantisation record u16 +0x04 (0xB8 form only); bit 31 cleared |

The requantisation record is `convolution_IPU`'s last argument, the conv layer's post-quantisation record: +0x00
accumulator shift (signed; it goes into word 9 `CfgTruncate` / `ShiftDirection` of the conv instruction), +0x04 the
scale of the 0xB8 form, +0x08 the output shift, +0x0C the zero point — the **input** tensor's zero point, i.e. the pad
value (§8.3.1, 0x1900) — and +0x0E / +0x10 the upper / lower clamp, from the **output** tensor
(`ipu-firmware-2-engines.md` §6.4, conv engine). Anjoy cmd+0x1CC: 0x803C_6038 0x0040_E000 0x0000_00FF 0 → buffer 0xF, address
0x2000, format 0x40_C000, word 2 = 0x00FF.

#### 8.5.5 Vector-unit classes
The header follows §8.5.1 (format A or B as listed); the bodies are written by the encoders named in §8.5.1. Example, class 0x20 (6 words, `wiggleerr_kernel`, header `0x8288_0220 | n<<13`, n = 3…5): word 1 = u16
argument | 0x0207_0000 with bits 23:19 = a 5-bit register; word 2 cleared to bits 31:30; word 3 keeps bits 29:27; word 5
= two u16 from the op's parameter record [R]. The body fields of every vector-unit class (operand buffer addresses, lane
counts, requantisation), the LUT instruction 0x78 / 0xF8 and the pooling instruction 0x5B / 0xDB are
`ipu-firmware-2-engines.md` §7, §8 (vector unit, LUT unit); the wire format here (header, length, delta) is all the decoder
needs to frame them [I].

#### 8.5.6 SR class 0x7F (scalar register file; firmware encoders only, not in the Anjoy block) [R: encoders below]
Low 7 bits 0x7F; bit 7 is an operand-form bit for some opcodes (and an immediate bit for 0xB…); bits 31:23 hold the
opcode (only 31:28 for 0xB…, whose bits 27:23 carry immediate bits); destination register at 22:18 (`idx & 0xF`, bank bit
22), source register at 12:8 (bank bit 12). An SR instruction is this 32-bit header followed by 0–16 immediate words;
lengths are per opcode. The ten opcodes below are the complete set (scan of the decompile):

| header constant | words | encoder (callers) | operation [I: from names / cache use] |
|---|---|---|---|
| 0xA400_207F \| rd<<18 | 2 (word 1 = 32-bit immediate) | `sr_load_imm32` (0x200088fc) (Softmax, `softmax_recip_step` (0x2007031e), `nms_load_imm_step` (0x2006a19a)) | load 32-bit immediate (SR value cache type 6) |
| 0xA600_007F \| rd<<18 \| (4·n)<<13 | 1 + 4·n (n = 1, 2, 4 quads of 16 B) | `drv_ipu_sr_smove128_immed` 0x2000_8990 | `smove128` immediate, 128/256/512 bit (cache types 7/8/9) |
| 0xB000_007F \| rd<<18 \| imm[10:0]<<7 \| imm[15:11]<<23 | 1 | `cmp_load_imm16` (0x2005ff7e) | load 16-bit immediate (cache type 5) |
| 0x8300_007F / 0x8300_00FF \| rd<<18 \| rs<<8 | 1 | 0x20008b24 / `sr_move_b` (0x20008ba8) (ReduceMax, `softmax_sub_max` (0x200648ba)) | register move, two forms (bit 7); drops rd's 16-bit cache entry |
| 0x8480_007F \| rd<<18 \| rs<<8 | 1 | `nms_unary_step` (0x2006a26e) | register op, drops rd's 16-bit cache entry |
| 0xF280_00FF \| rd<<18 \| ra<<8 \| rb<<13 | 2 (word 1 = 0x0000_203E with bits 14–18 cleared) | `sr_three_reg_op` (0x20008cae) (Softmax, `softmax_recip_step`) | three-register op |
| 0xC080_007F \| rd<<18 \| rs<<8 | 2 (word 1 = immediate) | `NMS_IPU_sAlu_add` | scalar add immediate |
| 0xC480_007F \| rd<<18 \| rs<<8 | 2 (word 1 = immediate) | `NMS_IPU_sAlu_and` | scalar and immediate |
| 0xCA00_007F \| rd<<18 \| rs<<8 \| imm<<13 | 1 (imm < 16) | `hal_ipu_sr_leftShift` 0x2000_8E70 | shift left |
| 0xCC00_007F \| rd<<18 \| rs<<8 \| imm<<13 | 1 (imm < 16) | `hal_ipu_sr_rightShift` 0x2000_8F12 | shift right |

The 0xC08 / 0xC48 / 0xCA / 0xCC operations are named by their callers `NMS_IPU_sAlu_add` / `NMS_IPU_sAlu_and` /
`NMS_IPU_sAlu_leftShift` / `NMS_IPU_sAlu_rightShift` (wrappers in `op_custom_ipu_implement.c`); the 0xA6 operation is
`drv_ipu_sr_smove128_immed`. The firmware keeps a cache of SR register values (`sr_cache_lookup` (0x2000859a) /
`sr_cache_store` (0x200087fc) / `sr_cache_set_valid` (0x200086da)) in the `ipu_sr` descriptor 0x2009_1E30 (+0x14 … +0x304: valid-bit masks per width and
the values; 0x2009_2138 is the pointer cell that holds the descriptor's address; the descriptor's +0x0C / +0x10 hold the
two SR bank bases 0x0101_0000 / 0x0101_0200; `ipu_global_ctrl` +0x48 is written 1 / 0 around every CPU write to SR bank
0) and skips an immediate load whose value is already in the register by
setting the buffer's skip byte, which makes `dec_push_insn` push nothing. The register file itself (32 registers, 8 / 16 /
32-bit scalar and 128 / 256 / 512-bit views): `ipu-firmware-2-engines.md` §5.1 (SR).

### 8.6 The Anjoy block as a whole [R: decoded command block]
<!-- ev:ipu-firmware-1-runtime-model.8.6 -->
- **One slice for the whole network.** Operator 0 (the first CONV_2D) carries RunMode {mode 1 = invoke by cmdq, fence 1,
  decoder 1, offset 0, size 0x7A570 = the whole block}; all 99 other operators carry mode 3 (skip) with fence 0, decoder
  0, offset 0, size 0. So one line-mode trigger runs the entire network, and the per-op RunMode fields can describe either
  one slice or many [R values; I: "many" from the field design].
- **Layout.** Block prologue → 62 × (layer prologue, op-7 marker, layer body) → epilogue:
  - layer prologue (62×): op 3 id 0; for VR buffers 1, 0, 2, 4, 3, 5: op 3 (0x20 + *b*) + op 1 to the buffer's DMA /
    vector table (0x080 + 0x100·*b*); for VR buffers 0xD, 8, 0xF: op 3 + conv mapping table (0x100·*b*), op 3 + DMA /
    vector table; op 1 to 0x1000 (VR 0x10 mapping table, no barrier); op 3 id 0. VR 0xD / 8 / 0x10 / 0xF are the conv
    IB / KB / AB / OB; the AB table at 0x1000 is written without a barrier, as the firmware does. The first prologue also
    writes MR tables 0 and 1 (0x1100, 0x1200) and, after a second op 3 id 0, the 24-byte colour matrix at 0xC040. This
    is the same op 3 + op 1 pattern the firmware's `_vrbuf_init_bufid` emits online (§8.3.2).
  - layer body: tiles of {DMA loads (input, weights, bias) → conv 0x2C (or 0x0C) → output stage 0x38 → DMA store 0x44};
    the first instruction of each class full, the rest deltas (§8.4). Layer 0: 40 tiles.
  - epilogue: last layer prologue, op 1 0x1908 (4 bytes, zero), op 2, op 3 id 0, op 4 0x034 = 1 (fence) (§8.3.3–§8.3.5).
- The op-1 payloads in the prologue are small tables of u16: e.g. VR 1 DMA / vector table = 0x0804, VR 0 = {0x0801,
  0x0802}, VR 0xD mapping = {0x0240, 0x0280, 0x0408, 0x0410, 0x0420} — the per-layer L2 slot assignment of each buffer
  [R values; I: meaning; slot tables: `ipu-firmware-2-engines.md` §4.3, L2 / VR buffers].

### 8.7 Decoder registers used to run a block [R: `hal_ipu_ctrl_open` 0x2000_A042, `hal_ipu_ctrl_dec0_line_mode_trigger` 0x2000_A2AA, `dec1_trigger` (0x2000854e) (decoder 1), `dec_fifo_push` / `dec_fifo_push_b` (0x2000a1e0) / `dec_fifo_push_c` (0x2000a1f8) / `dec_push_wait` (0x2000a22e) / `dec_fifo_push_d` (push), `dec0_drain` / `dec1_drain` (wait)]
The control object `ipu_ctrl` (0x2009_1A10) has register base +0x28 = **0x0100_0000** (host CPU view 0x1800_0000) [R:
descriptor words]. Offsets used:

| offset | decoder 0 | decoder 1 | use |
|---|---|---|---|
| +0x00 | written 1 then 0 | same | pulse before every line-mode block (both decoders' triggers write it) |
| +0x04 | bit 9 = ring mode, bit 10 = ring size valid, bits 8:0 = ring size >> 4 | — | `hal_ipu_ctrl_open` (ring mode is never used: its trigger / wait asserts) |
| +0x08 | bit 0 enable, bits 1/2 ring-mode flags | — | `hal_ipu_ctrl_open` |
| +0x0C, +0x10 | 0; length >> 4 | — | line mode |
| byte +0x28 | 0x10 | same | written before every line-mode trigger |
| +0x2C | bit 0 = start | — | line mode |
| +0x30 | **command FIFO port** | — | every online command word, written one by one after waiting (≤ 90 000 polls) until +0x34 + n ≤ 0x400 |
| +0x34 | FIFO fill level (entries, ≤ 0x400) | — | flow control |
| +0x68 (u16) | == 1: reply ready | — | `dec_push_wait` (push one word, then wait) — used by `dec_push_wait_caller` (0x20008512), 2 callers |
| +0x6C / +0x74 | bit 0 done / write 1 to clear | — | wait for end of block |
| +0x8C, +0x90 | — | block bus address >> 4; length >> 4 | line mode |
| +0xAC | — | bit 0 = start | line mode |
| +0xB0 | — | **command FIFO port** | decoder 1 (no flow control in `dec_fifo_push_b`) |
| +0xD4 (u16) | — | == 1: reply ready | 0x2000a278 |
| +0xD8 / +0xE0 | — | bit 0 done / clear | wait |
| +0x12C | bit 0 = global run (decoder 0 trigger only) | — | |
| 0x01AA_6018 / 0x01AA_601C (absolute, u16 halves) | block bus address low / high | — | decoder 0 line mode |

Decoder 0 line mode also writes 0x0142_00B0 = 1 (the global "run" bit mhal associates with the IPU) and, while it waits,
drains the DLA IRQ status at 0x0142_0040 through the IPU interrupt status / clear register pairs 0x0142_0058–0x0142_00C0
(the pairs of the host's IPU interrupt table, `ipu-host.md` §2.6), so in normal operation the host never gets a DLA
interrupt while a block runs. All online command words go to decoder 0 (all 458 call sites of `dec_push_insn` /
`dec_push_op1` / `dec_push_op2` / `dec_push_op3` pass decoder 0); the offline block of the Anjoy model runs on decoder 1
[R: RunMode decoder 1]. In line mode **the fence is not waited on**: `_HAL_IPU_CommitOfflineCmdblock` (0x2003_1C72)
calls `_HAL_IPU_TriggerDecoderDMALineMode` (0x2003_1BF8 → `dec1_trigger`), which itself spins on the done bit, and
returns; `u32Fence` is passed only to the ring-mode wait, which asserts (0) [R: disassembly 0x2003_1C72–0x2003_1D92]. The
fence value instead ends the block as the operand of op 4 (§8.3.5). The rest of the decoder / CMDQ / BDMA register map:
`ipu-firmware-2-engines.md` §3 (IPU200M HAL).
