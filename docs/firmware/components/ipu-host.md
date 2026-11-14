# ipu-host — mi_ipu.ko, libmi_ipu.so, mhal IPU, the CCIF mailbox (host side)
<!-- ev:ipu-host -->

The host side of the NPU ("IPU", called "DLA" in the device tree, sysfs and power names) of the SigmaStar media software
on this camera: everything that runs on the ARM CPU to boot the RISC-V program `ipu_firmware.bin` on the IPU, talk to it
and hand it networks and tensors. Loaded in part, never used by the product (§1):

- **mhal IPU** (block of `mhal.ko`, 28 functions, `MDRV_IPU_*` / `MHAL_IPU_*`) — maps the IPU register space, switches
  the DLA power domain, sets the IPU PLL, opens the two address windows, sets the RISC-V reset vector and starts the core,
  stops it, clears the IPU interrupt status (§2).
- **CCIF** — the mailbox between the CPU (`ipu_ccif_arm_client`) and the RISC-V program (`ipu_ccif_riscv_server`):
  doorbells, two 40-byte package windows, nine commands (§3). Host code lives in mi_ipu.ko.
- **mi_ipu.ko** (`/dev/mi_ipu`, MI module id 0x1D) — 24-slot ioctl table: firmware start and stop, up to 48 channels
  (one loaded network each), invoke, device lock shared between processes; firmware log ring, statistics, hang dump,
  `/sys/dla/` (§4).
- **libmi_ipu.so** — the 11 public `MI_IPU_*` functions: reads the firmware file into a buffer and has the kernel boot
  it, checks the firmware's version, loads model files (`SGSI` offline models), allocates tensor buffers, converts
  FP32 ↔ INT16 for float-flagged tensors and invokes (§5).

What the RISC-V program does with the commands, the `ipu_firmware.bin` image layout and the SGSI model format are
specified in `ipu-firmware-1-runtime-model.md`; this document states only what the host does and what it relies on.

Tags: **[R]** read from the binary, a decompile, disassembly, a file, or on the running camera where said; **[S]**
stated by a source (the public M6 `mi_ipu.h` / `mi_ipu_datatype.h`); **[I]** inference. A heading tagged [R] means every
statement under it is [R] unless marked otherwise. Function and export names are the binaries' symbols [R]; MI struct,
field and error names are those of the public M6 headers [S]; names the binaries do not carry (struct fields, ioctl
names, register names) are chosen in SigmaStar style and marked [I] where they first appear. Register addresses are
written `BANK.RR` (16-bit register index in the RIU bank, CPU phys 0x1F00_0000 + BANK·0x200 + RR·4, `kernel-platform.md`
§3.1; `h` = high byte); "IPU+off" is a byte offset in the IPU register space at CPU phys **0x1800_0000**.

Binaries (dump of 2026-10-16, `D` =) [R]: `D/modules/4.9.227/mi_ipu.ko` (36.9 KB, local
symbols kept), `D/lib/libmi_ipu.so` (25.6 KB, stripped, 11 exports), `D/modules/4.9.227/mhal.ko` block IPU (28
functions, 1.6 KB, as delimited in), `D/dla/ipu_firmware.bin` (602 632 B, md5
6e1d4c3f97a61bd4b95174f139202b71; on the camera `/config/dla/ipu_firmware.bin`). Build strings: mi_ipu.ko "Sigmastar
Module mi_ipu version: project_commit.7251436 sdk_commit.80327e1 build_time.20220721223310", libmi_ipu
`sstar_mi_so_version_string` the same with `build_time.20220721223314`.

## 1. The stack on this camera [R]

| piece | on the camera | used by the product |
|---|---|---|
| `mi_ipu.ko` | loaded at boot (`firmware-image.md` §3.3); creates `/dev/mi_ipu`, `/proc/mi_modules/mi_ipu/`, `/sys/dla/`; does **not** start the firmware | no: no app opens `/dev/mi_ipu` |
| mhal IPU block | in `mhal.ko`; its 22 `MDRV_IPU_*` exports are imported by mi_ipu.ko only (§2.7) | no |
| `libmi_ipu.so` | in `/config/lib` (`firmware-image.md` §3.5) | no: prog_uvc and kjd_lvgl do not link it |
| `/config/dla/ipu_firmware.bin` | 602 KB file (`firmware-image.md` §3.6) | no: loaded only by libmi_ipu on the first `MI_IPU_CreateDevice` (§5.2), never by a kernel module or boot script |

The IPU PLL is programmed to 800 MHz by the IPL (`ipl.md` §3.8) and moved to 900 MHz when mi_ipu.ko loads [I, §2.3]. The
DT voltage-select node `dla_power` fails its probe at boot (`kernel-platform.md` §5.4); neither mhal nor mi_ipu refers
to it (§2.2), and the failure does not stop the NPU from working (§6).

The stock pieces run a compiled model unchanged when a program calls the public API: a run on 2026-10-18 with this
camera's own `ipu_firmware.bin`, `mi_ipu.ko` and `libmi_ipu.so` and the Anjoy model computed repeatable results that
change with the input, alongside the live 4K pipeline (§6).

## 2. mhal IPU block (28 functions, `MDRV_IPU_*` / `MHAL_IPU_*`) [R]
<!-- ev:ipu-host.2 -->

Hal version string `T_0.4_210224` ("hal version"). Registers the block touches: `101E` (power 0x6C, 0x1F), `1035` (IPU
PLL 0x11, 0x60–0x62), `1038.50` (IPU clock select / gate, `kernel-platform.md` §5.1), `1530` (0x00, 0x13, 0x17, 0x21,
0x28) and the IPU's own register space at CPU phys 0x1800_0000 (CCIF mailbox, address windows, reset vector, core start,
interrupt status). Bank `1520`, named for the IPU in the early mhal map, and bank `1020` do not occur in the block's code
[R: no 0x2A40xx constant]. The block holds power, clock, firmware boot, shutdown and interrupt clearing.

### 2.1 State (mhal `.bss`+0x16C98, 0x60 B) [R]
| off | name in binary / [I] | content |
|---|---|---|
| +0x00 | `g_VirtRiuBaseIPU` | `ioremap(0x1800_0000, 8 MiB)` — the IPU register space |
| +0x04 | mapping offset [I name] | firmware MIU address, copied from init block word 0 (`MDRV_IPU_Get_MappingOffset`) |
| +0x08 | `g_VirtRiuBaseChipTop` | `ioremap(0x1F00_0000, 8 MiB)` — the whole RIU |
| +0x0C / +0x10 | OPP table ptr / count | set by `MDRV_IPU_Set_OPP_TB` from mi_ipu (DT OPP list, §4.1) |
| +0x14 | current clock, MHz | |
| +0x18…+0x47 | copy of the 12-word init block (§2.4) | read back by `MDRV_IPU_Get_Full_Fw_Info` |
| +0x48 | mutex | clock changes |

### 2.2 Accessors and power [R]
| function | returns / does |
|---|---|
| `MDRV_IPU_MappingIO()` | the two ioremaps of §2.1 (called first from mi_ipu's `init_module`) |
| `MDRV_IPU_Get_CCIF_Base` / `_Get_CCIF_Ctrl` | IPU+0x410000 (both) |
| `MDRV_IPU_Get_CCIF_TxPkg` | IPU+0x410018 |
| `MDRV_IPU_Get_CCIF_TxWord` | IPU+0x410068 (exported, no importer — unused) |
| `MDRV_IPU_Get_FirmwareDataStart` / `…End` | constants 0x2007_D000 / 0x200E_D800 |
| `MDRV_IPU_Get_MappingOffset` | state +0x04 |
| `MDRV_IPU_Get_Firmware_Info(u32 out[3])` | {0x7D000 code size, 0x70800 data size, 0x400000 default heap size} |
| `MDRV_IPU_Get_Full_Fw_Info(u32 blk[12], u32 **pc, u32 **branch)` | blk = state +0x18…; `*pc` = VA of IPU+0x420054, `*branch` = VA of IPU+0x420050 (core debug registers read in the hang dump, §4.7) |
| `MDRV_IPU_Get_Hal_Ver(buf, n)` | `strncpy` of `T_0.4_210224` |
| `MDRV_IPU_GET_DLAPowerSts` → `MHAL_IPU_GET_DLAPowerSts` | `101E.6C == 0` (1 = powered) |
| `MDRV_IPU_DLA_PowerOn/Off` → `MHAL_…` | `101E.6C = 0` / `= 1` |
| `MDRV_IPU_clr_irq` → `MHAL_IPU_clear_dla_irq` | §2.6 |

The host hard-codes the firmware's memory split: code region 0x2000_0000 + 0x7D000, data region 0x2007_D000 + 0x70800,
ending at 0x200E_D800 (the firmware's stack top), so the firmware needs **0xED800 B (972 800 B)** of memory (image
layout: `ipu-firmware-1-runtime-model.md` §2.2).

`101E.6C` is the only power control: 0 = DLA power domain on, 1 = off [I: from the names; the register is written
whole]. No regulator or voltage is touched; `dla_power` (the DT voltage-select GPIO node, `kernel-platform.md` §5.4) is
not referenced by mhal or mi_ipu [R: no string, no import].

### 2.3 Clock (IPU PLL, bank 1035) [R]
Table `.rodata+0x7538`, 10 × 8 B `{u8 r61lo, u8 r60hi, u8 r60lo, u8 pad, u32 MHz}`:

| MHz | 1200 | 1100 | 1000 | 900 | 800 | 700 | 600 | 500 | 400 | 300 |
|---|---|---|---|---|---|---|---|---|---|---|
| synth (61lo:60hi:60lo) | 2E147B | 3244E3 | 374BC7 | 3D70A4 | 451EB8 | 4EFE8A | 5C28F6 | 6E978D | 8A3D71 | B851EB |

Every entry fits **f[MHz] = 432 · 2^23 / synth** (e.g. 0x451EB8 → 800.0) [R arithmetic]. A frequency is applied by
writing `1035.61` low byte, `1035.60` high byte, `1035.60` low byte, in that order, under the mutex, only if it differs
from the current one. The IPL leaves the synth at 0x451EB8 = 800 MHz (`ipl.md` §3.8). The PLL start writes `1035.11` /
`1035.62` are done by `MHAL_IPU_Init` (§2.4).
- `MDRV_IPU_ClkInit()`: target = 900 MHz if the DT OPP list contains 900, else the highest OPP; with no OPP list it
  prints "do not support scaling ipu frequency" and returns −1 (mi_ipu then calls `MDRV_IPU_Get_Clk_Rate`). It also
  calls `get_core_voltage("cpu_power", &v)` and ignores the result — no voltage scaling is done for the IPU.
- The DT node `ipu` lists operating points 1000 … 300 MHz in 100 MHz steps, all at 1 000 000 µV (`kernel-platform.md`
  §14), so loading mi_ipu.ko moves the IPU PLL from 800 MHz to **900 MHz** [I: from the code path; not read on the
  camera]; the table's 1200 and 1100 MHz rows are unreachable (not in the OPP list).
- `MDRV_IPU_Scaling_Freq(MHz)`: MHz must be in the DT OPP list ("do not support scaling to %uMHz") and in the table
  ("…, miss table"); then as above. Used by sysfs `clk_rate` (§4.10) and the mi_ipu export `MI_IPU_Scaling_Freq`.
- `MDRV_IPU_Get_Clk_Rate()`: matches `1035.61` low byte and `1035.60` high byte against the table, stores and returns
  MHz (−1 if none).
- `MDRV_IPU_Show_Freq(buf)`: "current ipu clock frequency: %uMHz\n" [+ "available frequency:\n" + "%uMHz\n" per OPP].
- `MDRV_IPU_ClkDeinit()`: destroys the mutex only.

### 2.4 `MDRV_IPU_Init(blk)` → `MHAL_IPU_Init(blk)` — power-up and boot [R]
`blk` is 12 words; the caller (mi_ipu, §4.6) fills words 0–5 = three u64 from the InitFirmware ioctl: **fw** (MIU
address of the firmware buffer), **heapStart**, **heapEnd** (MIU addresses, §5.2). `MDRV_IPU_Init` fills the rest: u64
code base 0x2000_0000 (+0x18), u64 data base 0x2000_0000 (+0x20), code size 0x7D000 (+0x28), data size 0x70800
(+0x2C); stores `fw` as the mapping offset and the whole block in the state, then calls `MHAL_IPU_Init`, which writes,
in order:

| # | register | value | note [I unless R] |
|---|---|---|---|
| 1 | `101E.1F` | 0x0100 | then sleep 1 ms |
| 2 | `1530.17` low byte | `&= 0xF0` | |
| 3 | `1530.13` | 0x1000 | same value `MHAL_IPU_clear_dla_irq` enforces (§2.6) [R] |
| 4 | `1035.62` | 0x0001 | IPU PLL enable / start, as in the IPL's PLL start (`ipl.md` §3.8) |
| 5 | `1035.11` | 0x0080 | |
| 6 | `1038.50` | 0x0000 | clock gate open (shutdown writes 1, §2.5) |
| 7 | `1530.00` byte | 0x00 | reset release (shutdown writes 0xFF) |
| 8 | `1530.21` | 0xFFFF | |
| 9 | `1530.28` byte | 0x10 | |
| 10 | IPU+0x420008 byte | 0xFF | shutdown writes 0 (word) [R] |
| 11 | IPU+0x000188 | 0xFFFF_FFFF | |
| 12 | IPU+0x00018C | 0x0015_5400 | |
| 13 | IPU+0x46005C | 0x0005_5555 | |
| 14 | IPU+0x460060 (16-bit) | 0x0002 | |
| 15 | IPU+0x614104 / +0x614108 | 0x2000_0000 / 0x2007_CFFF | address window 0: code range |
| 16 | IPU+0x61410C | fw (low 32 bits) | window 0 target |
| 17 | IPU+0x614100 | 1 | window 0 enable |
| 18 | IPU+0x614114 / +0x614118 | 0x2007_D000 / 0x200E_D7FF | window 1: data range |
| 19 | IPU+0x61411C / +0x614110 | fw / 1 | window 1 target, enable |
| 20 | IPU+0x420044 | 0x2000_0000 | RISC-V reset vector |
| 21 | IPU+0x4200C4 / +0x4200C8 | heapStart + 0x2000_0000 / heapEnd + 0x2000_0000 | heap range, read by the firmware's entry code at 0x0142_00C4 / 0x0142_00C8 [R] |
| 22 | IPU+0x42004C | 1 | core run |

Both windows get the same target `fw`, so the translation is "IPU address − 0x2000_0000 + fw" over [0x2000_0000,
0x200E_D7FF] [I: the only reading under which code and data do not collide; the firmware buffer holds the image at
offset 0, §5.2]. Heap addresses are passed as MIU address + 0x2000_0000: outside the windows the IPU sees DRAM at
0x2000_0000 + MIU address, like the CPU [I]. The RISC-V sees the IPU register space at 0x0100_0000 where the CPU sees it
at 0x1800_0000 [I: from the matching offsets 0x4200C4 / 0x4200C8 read by the firmware at 0x0142_00xx;
`ipu-firmware-1-runtime-model.md` §3.1]. Only 32 bits are used: the high halves of the u64 are dropped [R].

Not known: the meaning of `1530.00`, `.13`, `.17`, `.21`, `.28` and `101E.1F` (values are specified; no reference names
them), and names of the IPU-space offsets the host writes (0x188 / 0x18C, the 0x420000 block, 0x460000, 0x611020 /
0x611120, the 0x614100 windows): the meanings above are inferred from use; no source names them. The firmware-side use of
the IPU space is in `ipu-firmware-1-runtime-model.md` §3.1, §8.7.

### 2.5 `MDRV_IPU_Shutdown_RISCV_IPU()` [R]
IPU+0x611020 = 2, spin until bit 0 set; IPU+0x611120 = 2, spin until bit 0 set (two bus / DMA ports drained [I]; there
is no timeout — a dead IPU hangs the CPU here [R]); IPU+0x420008 = 0 (word); `1530.00` byte = 0xFF; `1038.50` byte = 1.

### 2.6 `MHAL_IPU_clear_dla_irq()` (via `MDRV_IPU_clr_irq`) [R]
Forces `1530.13` = 0x1000 if it differs. Reads the IPU top status IPU+0x420040; for every set bit *b* prints "dla irq
comes from IPU_INT_IRQ_%d" and, for each row of the table `.rodata+0x7588` with id = *b*, copies every set bit of the
row's status register into its clear register (read-modify-write OR). Table `{id, mask?, status, clear}` (offsets in the
IPU space; the second column is never read, "mask" [I]):

| id | (2nd) | status | clear | | id | (2nd) | status | clear |
|---|---|---|---|---|---|---|---|---|
| 0 | 0x0 | 0x410014 | 0x410010 | | 4 | 0x4200B0 | 0x4200B4 | 0x4200AC |
| 1 | 0x42005C | 0x420060 | 0x420058 | | 6 | 0x70 | 0x6C | 0x74 |
| 2 | 0x420068 | 0x42006C | 0x420064 | | 7 | 0x7C | 0x78 | 0x80 |
| 2 | 0x420074 | 0x420078 | 0x420070 | | 8 | 0xDC | 0xD8 | 0xE0 |
| 3 | 0x420080 | 0x420084 | 0x42007C | | 9 | 0xE8 | 0xE4 | 0xEC |
| 3 | 0x42008C | 0x420090 | 0x420088 | | 9 | 0x4200BC | 0x4200C0 | 0x4200B8 |
| 3 | 0x420098 | 0x42009C | 0x420094 | | 10 | 0xF4 | 0xF0 | 0xF8 |
| 3 | 0x4200A4 | 0x4200A8 | 0x4200A0 | | 10 | 0x40001C | 0x400018 | 0x400020 |

Rows in table order, left column first: ids 0, 1, 2, 2, 3, 3, 3, 3, 4, 6, 7, 8, 9, 9, 10, 10; id 5 has no row. Id 0 is
the CCIF RISC-V→CPU status / clear pair (§3.1). The CPU reaches this function only when the CCIF interrupt fires with no
CCIF bit set (§3.4). In normal operation that does not happen: the firmware's decoder-DMA line-mode trigger polls and
clears the same status / clear pairs itself (0x0142_0040 status, pairs 0x0142_0058–0x0142_00C0 in the RISC-V's view)
while a command block runs, so the host never gets a DLA interrupt [R firmware, `ipu-firmware-1-runtime-model.md` §8.7]. The
global "run" bit the block associates with the IPU is 0x0142_00B0 = 1 [R firmware] (IPU+0x4200B0 in the CPU's view [I]).

### 2.7 Unused and exported-only [R]
The 22 `MDRV_IPU_*` exports are imported by mi_ipu.ko and by no other module [R: `nm -u` of every module];
`MDRV_IPU_Get_CCIF_TxWord` has no importer at all (unused); the five `MHAL_IPU_*` are internal (called by the `MDRV_`
wrappers). The `MDRV_IPU_*` helpers lie in mhal's INIT address range (`mhal-core.md` §2.1).

## 3. CCIF — the host ↔ RISC-V mailbox (`ipu_ccif_arm_client` ↔ `ipu_ccif_riscv_server`) [R]
<!-- ev:ipu-host.3 -->

Host code: mi_ipu.ko `drv_ccif_*`, `hal_ccif_*`, `rpc_client_*`, `_hal_to_cpu_isr`. The client name `ipu_ccif_arm_client` is a string in mi_ipu.ko with no code reference. The server is the
firmware's (`ipu-firmware-1-runtime-model.md` §4, CCIF server); firmware-side facts below are marked [R firmware].

### 3.1 Registers (IPU+0x410000 = CPU phys 0x1841_0000) [R offsets and use; I names]
| off | use by the host |
|---|---|
| +0x00 | CPU→RISC-V doorbell: write `1<<n`. n = 0: a request package is in TX; n = 1: stack-dump request (`hal_notify_riscv_dump_stack`, §3.4) |
| +0x08 | bit 0 = 1 while the previous CPU→RISC-V doorbell is still pending; the host does not send then |
| +0x10 | RISC-V→CPU clear: write `1<<n` |
| +0x14 | RISC-V→CPU status, bits 0–31 |
| +0x18…+0x3F | TX package (CPU→RISC-V), 10 words |
| +0x40…+0x67 | RX package (RISC-V→CPU), 10 words |
| +0x68 | "TxWord" (`MDRV_IPU_Get_CCIF_TxWord`): not used by the host |

Interrupt: DT node `dla` (`sstar,dla`), interrupt index 0 = GIC SPI 0x55 (`kernel-platform.md` §14),
`request_threaded_irq(…, _hal_to_cpu_isr, NULL, IRQF 0x80 = IRQF_SHARED, "CCIF_RISC2CPU_IRQ", cookie 0x31415926)` —
handled in hard-IRQ context. The status / clear pair +0x14 / +0x10 is also row id 0 of the IPU interrupt table (§2.6).

`ccif_client_dev` (mi_ipu `.data`+0x164, 0x1B8 B): +0x04 CCIF base VA; +0x08 device semaphore; +0x28…+0xE7
`complete[48]` (semaphore pointers, made per channel); +0xEC ctrl VA (= base); +0xF0 TX VA (+0x18); +0xF4 RX VA (+0x40);
+0xF8…+0x1B7 `pkg[48]` (0x28-byte buffers, per channel). Initial word +0x00 = 0xB80 (no reader found).

### 3.2 Package (40 bytes = 10 words, same layout both ways) [R]
| word | bits | request (host → RISC-V) | reply (RISC-V → host) |
|---|---|---|---|
| 0 | 7:0 | channel (0 for device requests) | channel |
| 0 | 15:8 | 1 (request marker) | echo of the request's marker (1); 0 only in the boot message (§3.5) [R firmware] |
| 0 | 31:16 | 0 | fragment index [R firmware] |
| 1 | 7:0 | command (§3.6) | command echoed |
| 1 | 15:8 | 0x0B (constant on every request; the firmware reads it and does not check it [R firmware]) | 0x0A = full 32-byte fragment, 0x0C = last fragment [R firmware]; the host does not read it and writes 0x0C into its own copy |
| 1 | 31:16 | payload length in bytes = 4·n, n ≤ 8 | payload length |
| 2–9 | | payload: n words | payload; word 2 = status (0 = OK) for every command the host decodes |

Every real reply is a single 0x0C fragment of 4 or 12 bytes [R firmware], so the host's one-package receive covers all
replies.

### 3.3 Sending [R]
`rpc_client_invoke(ch, cmd, n·4, words, 0x0B)` builds the package on the stack, then makes up to **10 000** tries of
{spin-lock `ccif_irq_lock`; if `+0x08` bit 0 is clear: copy 10 words to TX, write 1 to `+0x00`; unlock}, with no delay
between tries; after 10 000 busy tries it prints "!!! IPU hang" and fails (callers map that to 0x1A,
`E_IPU_ERR_SEND_REQUEST` [S]).

### 3.4 Receiving [R]
- `rpc_client_get_response`: device commands (cmd 3…8, `IsDevReq`) wait on the shared semaphore `ccif_client_dev+8`,
  then copy the RX window **from the hardware**; channel commands wait on `complete[ch]` and copy the per-channel copy
  made by the ISR. Timeout **20 000 ms** for both; timeout → "wait risc-v response timeout", result 0x400; any other
  semaphore error → "unknown return value(%d)".
- `_hal_to_cpu_isr`: takes the lowest set bit *n* of `+0x14`; none → "WARNING: IPU irqs(exclude riscv2cpu) were
  masked…", `MDRV_IPU_clr_irq()` (§2.6), done. Otherwise it copies RX (10 words), writes `1<<n` to `+0x10`, then by *n*:
  - **0** (package): if word0[15:8] == 0 or the command is 3…8 → up `ccif_client_dev+8` (the boot message completes
    the handshake this way); else copy the package into `pkg[ch]` (ch = word0[7:0]) and up `complete[ch]` ("…package
    space isn't exist" / "…complete[%u] isn't exist" if the channel has none). Since replies echo marker 1, device
    replies are told apart by their command number 3…8 and channel replies by commands 0…2 [I from §3.2].
  - **1** (stack info): words 2, 3, 4 → `g_stFreeRTOS_Info` +0x3C (task TCB address), +0x40 (end of stack), +0x44 (top
    of stack), used by the hang dump (§4.7). The firmware sends this message once, unsolicited, while it creates its
    `IPU_service` task during boot, before the boot message; it is not an answer to doorbell 1, which only makes the
    firmware print "IPU didn't respond". Word 4 (stack base) is a raw RISC-V address, not −0x2000_0000 like words 2–3
    [R firmware].
  - **2**: "IPU stack overflow, sp:%x" with word 2.
  - other: "_hal_to_cpu_isr invalid isr %d".

Only one request is in flight per channel (channel callers are serialised, §4.5); device requests share one semaphore
and the RX window and carry no sequence number [R], so two concurrent device requests from different processes can take
each other's answers — mi_ipu serialises them only through the user-side device lock (§4.3) [I].

### 3.5 Boot handshake [R]
`drv_ccif_handshake`: after `MDRV_IPU_Init` the host waits **5000 ms** on `ccif_client_dev+8`. The firmware sends one
bit-0 package with marker (word0[15:8]) 0 and its other bytes uninitialised, after it has created its op resolver [R
firmware]. Timeout → InitFirmware fails with 0xE (`E_IPU_ERR_INIT_FIRMWARE` [S]).

### 3.6 Commands [R host side]
| cmd | kind | sender | payload words (host → RISC-V) | reply used |
|---|---|---|---|---|
| 0 | channel | CreateChannel ioctl | the ioctl words (n from the caller, §4.4) | status |
| 1 | channel | DestroyChannel ioctl; process exit | ioctl words / {channel} | status |
| 2 | channel | Invoke | {MIU address of the channel's 0x7C0-byte invoke record} | status (0x400 = timeout) |
| 3 | device | SetMallocRegion ioctl | caller words + mapping offset appended | status |
| 4 | device | SetIpuParameter ioctl | {3 pitch alignments (§5.2), 0x2007D000, 0x200ED800, mapping offset, log ring MIU address, 0xFFC}; if the log ring could not be allocated, the last two words are replaced by a single 0 (7 words) | status |
| 5 | device | proc `ipu_statistic` / `time_statistic` | {MIU address of a 0x3898-byte buffer} | status, buffer filled (§4.9) |
| 6 | device | GetIpuVersion ioctl | {version buffer MIU address, its length} (ioctl +0x08, +0x10) | status, buffer = "`<hash>-<version>`" |
| 7 | device | InitFirmware; sysfs `ipu_log` | {log mode 0–2} | status ("fail to set ipu mode=%u") |
| 8 | device | proc `heap_stat` | {} | status, then words: current use, peak use (needs ≥ 12 payload bytes) |

Firmware side of the commands as far as the host depends on it [R firmware, `ipu-firmware-1-runtime-model.md` §5 host
commands]:
- cmd 3: the firmware takes {start, end, mapping offset}, stores start and end where nothing reads them and points its
  DMA address map at the fixed range 0xA020_0000–0xA0A0_0000. The library never sends it (§4.4).
- cmd 4: the pitch alignments it carries are overridden at channel creation by the model's own `configuration` vector
  (Anjoy model: 16, 2, 16).
- cmd 6: the reply string is made by `sprintf("%s-%s", hash, version)`; the buffer must hold at least 0x43 bytes. The
  firmware of this camera answers `579e2d5fa321b41c4f00ea5d76ec9fc8-Q_0.1.0_verified_21110517` [R file: both strings in
  `ipu_firmware.bin` at 0x7D568 / 0x7D544]; the Anjoy build carries the same hash with version `Q_0.9_verified_21091510`,
  so the hash is a protocol / ABI stamp, not a build id [I].

## 4. mi_ipu.ko [R]
<!-- ev:ipu-host.4 -->

Driver version `T_0.0.1_210525`, `depends=mi_common`, license GPL. The device frame (char device, ioctl copy-in / out,
`/proc/mi_modules/mi_ipu/` common files) is the MI_DEVICE framework every MI module carries (`mi_sys-2-interfaces.md` §1,
§10).

### 4.1 Load and unload [R]
`init_module`: `/proc/mi_modules/mi_ipu/` with `debug_level`, `debug_file`, `debug_func`, `module_version_file` (the
common MI_DEVICE set, `mi_sys-2-interfaces.md` §10.1) → `MDRV_IPU_MappingIO` → `drv_ccif_cpu_init` (CCIF pointers into
`ccif_client_dev`, clears `complete[]` / `pkg[]`; `hal_ccif_cpu_init`: DT `sstar,dla`, IRQ request, §3.1) →
`MI_IPU_init` → `MI_IPU_SysFsInit` → `MI_DEVICE_Register` (user path `/dev/mi_ipu`).

`MI_IPU_init`: zeroes the channel table (48 pointers), inits the spinlock, the device-lock semaphore (count 1) and the
task-list mutex; DT `sstar,ipu` → `dev_pm_opp_of_add_table`, then for each OPP (ascending, `dev_pm_opp_find_freq_ceil`)
stores {Hz/1 000 000, µV/1000} in a `devm_kmalloc` table and hands it to `MDRV_IPU_Set_OPP_TB` (messages "fail to find
ipu device node", "fail to add ipu frequency table, err=%d", "error: ipu didn't have opp table"); `MDRV_IPU_ClkInit`
(on failure `MDRV_IPU_Get_Clk_Rate`); power flag = `MDRV_IPU_GET_DLAPowerSts()`. The firmware is **not** started at load
— only by the first user (§4.6).

`cleanup_module` removes the proc files and unregisters; it does not stop the IPU, free the IRQ or unmap.
`MI_IPU_deinit` (no caller in the module) only destroys the locks and the clock mutex.

Module parameters [R modinfo]: `debug_level` (uint), `debug_file`, `debug_func` (string) — the MI_DEVICE debug filters;
`fw_heap_size` (uint, `.bss`, default 0) — if non-zero it replaces the 4 MiB heap size returned by the firmware-info
ioctl (§4.4, nr 22).

### 4.2 The mi_sys device [R]
Registered on first open (`MI_IPU_TrackTaskInit`, §4.3) with `mi_sys_RegisterDev` (`mi_sys-1-engine.md` §2.1): module
id **0x1D**, 1 device, **0x30** channels, one input and one output port per channel (registration words 0x1D, 1, 1, 0x30,
1), no callbacks. It serves only for the mi_sys private heap (`mi_sys_PrivateDevChnHeapAlloc/Free`, channel −1 for
device-wide buffers, `mi_sys-2-interfaces.md` §7.5), `mi_sys_EnableChannel/DisableChannel` per IPU channel, the MMA
heap name and the proc commands.

MIU protection: `mi_sys_setMiuProtectMMa(<IPU MMA heap name>, {0x71, 0x72}, 2, on)` (`mi_sys-2-interfaces.md` §7.7) —
on at firmware start and around the hang dump, off at shutdown ("fail to %s ipu mma miu protection"). 0x71 / 0x72 are the
MIU clients DLA_HIWAY_R / DLA_HIWAY_W (`kernel-platform.md` §12.2).

Proc commands of the device file (`echo <cmd> > /proc/mi_modules/mi_ipu/mi_ipu0`, `mi_sys-2-interfaces.md` §10.3):
`ipu_statistic`, `time_statistic` (both `MI_IPU_GetIPUStatistic`), `dma_statistic` (sets the "show DMA" flag),
`heap_stat`, `version` (§4.9).

### 4.3 Processes, device record, device lock [R]
- open (`mi_ipu_process_init`) → `MI_IPU_TrackTaskInit(pid)`: a 0x3C-byte task node {list, pid, channel list, mutex}
  is linked into the task list; the first open registers the mi_sys device. "client [%d] connected, module:%s".
- release (`mi_ipu_process_exit`) → `MI_IPU_TrackTaskExit(pid)`: for each channel the process still holds ("collect
  channel%u's resource"): free its kernel record and invoke buffer, send cmd 1 {chn} to the firmware, free its CCIF
  slots, disable the mi_sys channel, clear the id; unlink the node. When the open count reaches 0: if the firmware runs,
  DestroyFirmware (§4.6) and reset the device-record count; unregister the mi_sys device. If the dying process held the
  device lock, release it.
- Device lock (ioctl 13 / 14): a counting semaphore of 1 plus the holder's pid; libmi_ipu takes it around creating and
  destroying the device, so only one process boots or stops the firmware.
- Device record (ioctl 16 / 17): a plain counter of processes using the IPU; the ioctl returns the new value (the
  library destroys the firmware when it reaches 0).
- Alive (ioctl 15): returns the "firmware running" flag.

### 4.4 ioctl table [R]
Dispatch through the MI_DEVICE framework: table at `.data`+0 indexed by the ioctl number, 24 slots
(`MI_DEVICE_WrapperFunction` rejects ≥ 0x18). The handler gets the copied-in argument (`_IOWR('i', nr, size)`; the user
side passes the 16-byte wrapper {u32 size; u32 pad; u64 user pointer}, `mi-user-abi.md` §3.1) and its return value is
the ioctl result. Sizes from libmi_ipu.

| nr | cmd | handler | arg | function |
|---|---|---|---|---|
| 0, 1, 4, 5 | — | none | | |
| 2 | 0xC0186902 | InitFirmware | {u64 fw MIU, u64 heapStart, u64 heapEnd} | §4.6 |
| 3 | 0x6903 | DestroyFirmware | — | §4.6 |
| 6 | 0xC7C06906 | Invoke | 0x7C0-byte invoke record (§5.5) | §4.5 |
| 7 | 0xC0046907 | GetChannelId | u32 out: first free id 0–47, 0x100 = none | marks it used, adds it to the caller's task node |
| 8 | 0xC0046908 | PutChannelId | u32 id | 1 if ≥ 48, 3 "channel%u is not exist"; else frees the id |
| 9 | 0xC0246909 | CreateChannel | {u32 n = 5; chn; network MIU; desc MIU; variable-buffer MIU; variable-buffer size; …} (9 words max) | §4.5 |
| 10 | 0xC024690A | DestroyChannel | {n; chn; …} | §4.5 |
| 11 | (0xC024690B) | SetMallocRegion | {n; words…} + mapping offset appended → cmd 3 | **unused**: libmi_ipu never issues it; 0x12 on failure |
| 12 | 0xC024690C | SetIpuParameter | {n = 3; 3 pitch alignments} | appends data start / end and the mapping offset; allocates the log ring (§4.8); cmd 4; 0x13 on failure |
| 13 / 14 | 0x690D / 0x690E | DeviceLock / DeviceUnlock | — | §4.3 |
| 15 | 0x690F | DeviceIsAlive | — | returns 0 / 1 |
| 16 / 17 | 0x6910 / 0x6911 | Create / DestroyDeviceRecord | — | returns the new count |
| 18 / 19 | 0xC1486912 / 0xC1486913 | Set / GetFirmwareMem | 0x148 bytes | memcpy to / from a kernel copy — the library's "who booted the firmware where" record, so a second process can attach (§5.2) |
| 20 | 0xC0186914 | GetIpuVersion | {u64 buffer user VA (not used by the kernel); u64 buffer MIU; u32 length} | cmd 6 {buffer MIU, length}; 0x16 on failure |
| 21 | 0xC0406915 | GetIpuMmaName | char[0x40] | `mi_sys_GetModuleMMaHeapName` for the device |
| 22 | 0xC00C6916 | GetIpuFirmwareInfo | 3 × u32 out | {0x7D000, 0x70800, `fw_heap_size` ?: 0x400000} |
| 23 | 0xC0506917 | SetLibVer | {char hash[0x30]; char ver[0x20]} | stored; printed by `version` ("lib version: %s-%s" = ver-hash) |

Handler names are [I]. Error codes are the `IPU_ErrCode_e` values of the public `mi_ipu_datatype.h` [S]; every value
the binaries use matches it [R]:

| value | name | value | name |
|---|---|---|---|
| 1 | `E_IPU_ERR_INVALID_CHNID` | 0xF | `E_IPU_ERR_CREATE_CHANNEL` |
| 2 | `E_IPU_ERR_CHNID_EXIST` | 0x10 | `E_IPU_ERR_DESTROY_CHANNEL` |
| 3 | `E_IPU_ERR_CHNID_UNEXIST` | 0x11 | `E_IPU_ERR_INVOKE` |
| 4 | `E_IPU_ERR_NOMEM` | 0x12 | `E_IPU_ERR_SET_MALLOC_REGION` |
| 5 | `E_IPU_ERR_NOBUF` | 0x13 | `E_IPU_ERR_SET_IPU_PARAMETER` |
| 6 | `E_IPU_ERR_BADADDR` | 0x14 | `E_IPU_ERR_INVALID_PITCH_ALIGNMENT` |
| 7 | `E_IPU_ERR_SYS_TIMEOUT` | 0x15 | `E_IPU_ERR_NO_CREATED_IPU_DEVICE` |
| 8 | `E_IPU_ERR_FILE_OPERATION` | 0x16 | `E_IPU_ERR_GET_IPU_VERSION` |
| 9 | `E_IPU_ERR_ILLEGAL_TENSOR_BUFFER_SIZE` | 0x17 | `E_IPU_ERR_MISMATCH_IPU_HEAD_FILE` |
| 0xA | `E_IPU_ERR_ILLEGAL_BUFFER_DEPTH` | 0x18 | `E_IPU_ERR_NO_SUPPORT_REQ` |
| 0xB | `E_IPU_ERR_ILLEGAL_INPUT_OUTPUT_DESC` | 0x19 | `E_IPU_ERR_FAILED` |
| 0xC | `E_IPU_ERR_ILLEGAL_INPUT_OUTPUT_PARAM` | 0x1A | `E_IPU_ERR_SEND_REQUEST` |
| 0xD | `E_IPU_ERR_MAP` | 0x1B | `E_IPU_ERR_GET_FIRMWARE_INFO` |
| 0xE | `E_IPU_ERR_INIT_FIRMWARE` | 0x100 | `E_IPU_ERR_NO_AVAILABLE_CHNID` |

### 4.5 Channels and invoke [R]
- **CreateChannel** (nr 9): chn < 48 ("exceed max channel limit:%d"), not existing ("error: channel%u exists"); kernel
  record 0x48 B {chn; CCIF package buffer ptr; mutex (+8); waiter list head (+0x30); u64 MIU (+0x38) and VA (+0x40) of a
  **0x7C0-byte** private buffer "IPU_Buffer_chn%u"}; CCIF `complete[chn]` semaphore (0x20 B) and `pkg[chn]` (0x28 B);
  then cmd 0 with the caller's words (channel = chn); `mi_sys_EnableChannel` follows always, even when the firmware
  refused (then everything is freed and 0xF returned).
- **DestroyChannel** (nr 10): frees the record and buffer, then cmd 1 with the caller's words, frees the CCIF slots,
  `mi_sys_DisableChannel`; 0x10 if the firmware call failed. It does not remove the id from the task node (PutChannelId
  does).
- **Invoke** (nr 6): a FIFO of callers per channel — a caller appends a stack node with its own semaphore and sleeps
  unless it is first; then it writes the module's `force_online_mode` byte (sysfs, §4.10) into record byte 0x7B8,
  copies the 0x7C0-byte record into the channel buffer, flushes it and runs the invoke loop: cmd 2 {buffer MIU} on
  channel chn, 20 s wait. On timeout (0x400): the first time only, the hang dump (§4.7); if the log mode is non-zero,
  `call_usermodehelper("/bin/sh", "-c", "cat /sys/dla/ipu_log >> /dev/console")`; "IPU invoke timeout, try again" — up
  to 3 retries, then "IPU invoking failure after try 3 times". Afterwards the caller unlinks itself and wakes the next
  waiter. Result: firmware status 0 → 0; 0x400 → 7 (`E_IPU_ERR_SYS_TIMEOUT`); other → 0x11 (`E_IPU_ERR_INVOKE`).
  The firmware writes its output directly into the user's tensor buffers (their MIU addresses are in the record); the
  kernel never copies tensor data.

### 4.6 Firmware start and stop [R]
**InitFirmware** (nr 2, from the library after it has loaded the image into the firmware buffer):
1. clear the 12-word block and `g_stFreeRTOS_Info`; MIU protection on (§4.2);
2. if auto power is on (sysfs `auto_power_on_off`, default 1): power flag = 1, `MDRV_IPU_DLA_PowerOn`;
3. block words 0–5 = the three u64 of the argument; `MDRV_IPU_Init` (§2.4, releases the core);
4. `MDRV_IPU_Get_Full_Fw_Info` into `g_stFreeRTOS_Info` (+0x00…+0x2F block copy, +0x30 pc-register VA, +0x34
   branch-register VA);
5. boot handshake, 5 s (§3.5). On failure: auto power off, MIU protection off, return 0xE;
6. on success: init the statistics mutex, clear the log state, alive = 1, cmd 7 {current log mode} (an error is only
   printed).

**DestroyFirmware** (nr 3; also at the last close): `MDRV_IPU_Shutdown_RISCV_IPU` (§2.5), auto power off, MIU
protection off, alive = 0, "--------shutdown IPU----------", free the log ring if allocated.

### 4.7 Hang dump (first invoke timeout only) [R]
Doorbell bit 1 (§3.1) is rung (the firmware only prints "IPU didn't respond", §3.4); sleep 50 ms; print "branch_address
: %x pc : %x" from IPU+0x420050 / +0x420054; MIU protection off; `mi_sys_Vmap` the firmware buffer (code + data size);
check that the TCB address and the end-of-stack address (stored from the boot-time stack-info message, §3.4) lie inside
the data region (else "wrong TCB address" / "wrong end stack address"), and that the TCB's first word (top of stack)
lies inside the image (else "wrong top stack address"); print "Base=%p, TaskTCB=%p, TopStack0=%p, TopStack1=%p,
EndStack=%p, TopStack2=%x", "latest_target_address", "Core Status----", "pc", "sp", "Current task stack :", then one
line per stack word (from top to end of stack) whose value lies inside the image (format `.rodata.str` `_LC52`, a
return-address candidate); MIU protection on; unmap. The address arithmetic assumes the FreeRTOS TCB's first word is
`pxTopOfStack` and that firmware addresses are image offsets + 0x2000_0000 [I]. The TCB and stack addresses are those of
the firmware's `IPU_service` task, sent once at boot (§3.4).

### 4.8 Firmware log ring [R]
Allocated by SetIpuParameter: a 0x2000 B private buffer "IPU_log", kernel VA kept. Layout: u32 read index (+0), u32
write index (+4), data from +8, ring size **0xFFC** (sent to the firmware as the last parameter word of cmd 4 with the
buffer's MIU address). `/sys/dla/ipu_log` read: invalidate, copy bytes from the read to the write index (wrapping at
0xFFC), advance the read index, flush the two indices; "IPU log is not inited" if there is no ring. Writing `0|1|2` sets
the log mode and, if the firmware runs, sends cmd 7 {mode}. The firmware writes the data and the write index
(`IPU_StoreLog`; `ipu-firmware-1-runtime-model.md` §3.6) [I].

### 4.9 Statistics [R]
`ipu_statistic` / `time_statistic`: the firmware must be alive ("IPU is not alive", 0x15); a 0x3898-byte private buffer
"IPU_Statistic" is zeroed, flushed and its MIU address sent with cmd 5. The buffer as read back (u32 index):
[0..1] u64 total start, [2..3] u64 total end; records from word 0x12, 0x12 words (72 B) each: [0..1] u64 start,
[2..3] u64 end, [4] decoder id, byte [5] = 1 "Offline mode" (IPU) / else "invoke by cpu", op name string at record byte
+0x15; [0xE22] record count, byte [0xE23] = 1 → print nothing, [0xE24] DMA load bytes, [0xE25] DMA store bytes.
Output: per record "[%d] Decoder%d Offline mode : start … duration … opName:%s" or "… invoke by cpu …", "Total execute
time", "IPU_execute_time : %lld(%lld%%), CPU_execute_time: …, IPU<=>CPU: …" (IPU = sum of the offline records, CPU = sum
of the others, the rest = total − both), and with `dma_statistic` on, "IPU DMA Statistic : load access = …".

`heap_stat`: cmd 8, prints "[IPU heap] Current Usage: 0x%x Peak Usage: 0x%x" (reply words 1 and 2).
`version`: "lib version: %s-%s" (SetLibVer), "driver version: T_0.0.1_210525", "hal version: T_0.4_210224".

### 4.10 `/sys/dla/` (kobject "dla" under `/sys`) [R]
| file | read | write |
|---|---|---|
| `version` | "lib version: …", "driver version: …", "hal version: …" | — |
| `auto_power_on_off` | "current dla auto power on/off status: open\|close …" | 0 / 1: whether firmware start / stop switch the DLA power domain (default 1) |
| `power` | "current dla power status: …" | 0 / 1: switch now (acts only when auto power is 1) |
| `ipu_log` | the log ring (§4.8) | 0–2: log mode → cmd 7 |
| `clk_rate` | `MDRV_IPU_Show_Freq` | MHz → `MDRV_IPU_Scaling_Freq` (−EINVAL if refused) |
| `task_channel` | "process %d channels:" + "channel%u" per held id | — |
| `force_online_mode` | 0 / 1 | 0 / 1: copied into every invoke record (byte 0x7B8) |

On the camera `/sys/dla/power` reads `off` between runs of a program (§6), consistent with auto power off at the last
close [I].

## 5. libmi_ipu.so [R]
<!-- ev:ipu-host.5 -->

Stripped; exports the 11 public `MI_IPU_*` functions of the M6 `mi_ipu.h` plus `sstar_mi_so_version_string`. The
library's own stamps are the hash `579e2d5fa321b41c4f00ea5d76ec9fc8` and the version `T_0.0.1_210525` (sent to the kernel
with SetLibVer, §5.2); the string it prints as "IPU SDK version" is the firmware's version (§3.6). Imports: libc,
CamOs* / CamFs* (from libcam_os_wrapper / libcam_fs_wrapper, `mi-user-abi.md` §7, which are **not** in DT_NEEDED — the
application must link them, §6.1),
`MI_SYS_{PrivateDevChnHeapAlloc,PrivateDevChnHeapFree,Mmap,Munmap,FlushInvCache,ConfigPrivateMMAPool}` and the shared MI
fd table `g_mi_fd_table` / `g_last_fd_index` / `g_mi_fd_semalock` (libmi_sys, `mi-user-abi.md` §2.1). All buffers come
from mi_sys private heaps of module **0x1D**, device 0.

### 5.1 Device file and globals [R]
`/dev/mi_ipu`, opened `O_RDWR` once per process (the pid is checked, so a forked child re-opens), `FD_CLOEXEC`,
registered in the libmi_sys fd table (assert if more than 0x22 entries). Every call passes {u32 size; u32 pad; u64
pointer}. Globals (`.bss`): firmware size (0x26208), u64 firmware MIU (0x26210), firmware path (0x2617C), u64
variable-buffer MIU (0x262A0), u64 heap MIU / heap end (0x262A8 / 0x262B0), IPU SDK version string (0x262B8), `DevAttr`
copy (0x26338), mutex (0x26348), per-process device reference count (0x26364), channel table [48] (0x26140).

### 5.2 `MI_IPU_CreateDevice(DevAttr *a, SerializedReadFunc rd, char *path, u32 fwSize)` [R]
`DevAttr` = `MI_IPU_DevAttr_t` {MaxVariableBufSize, YUV420_W_Pitch_Alignment, YUV420_H_Pitch_Alignment,
XRGB_W_Pitch_Alignment} [S]. Under the device lock (ioctl 13 … 14):
- process reference count > 0: count++ and return 0.
- else ask the kernel whether the firmware runs (ioctl 15):
  - **already running** (another process booted it): GetFirmwareMem (ioctl 19) restores firmware MIU, variable-buffer
    MIU, heap MIU, SDK version and MaxVariableBufSize from the shared 0x148-byte record; count++; mutex; device record
    +1 (ioctl 16). The pitch alignments of the second process are **not** checked or sent.
  - **not running**: boot (below); on success SetFirmwareMem (ioctl 18) publishes the record, count++, mutex, ioctl 16.

Boot sequence (internal function at 0x11714):
1. alignments: W_YUV and XRGB non-zero multiples of 16, H_YUV non-zero and even, else 0x14 (§6.1); MaxVariableBufSize
   ≠ 0, else 9;
2. `rd` defaults to a CamFs reader (open, seek `offset`, read until `size` or EOF); `path` defaults to
   `/config/dla/ipu_firmware.bin`; `fwSize` 0 → file size by seek-to-end ("can't access default firmeware image", 8);
3. ioctl 22 → {code 0x7D000, data 0x70800, heap 0x400000}; firmware size = code + data; a larger `fwSize` → 0x19
   ("firmware runtime size is small than binary size");
4. ioctl 21 → MMA heap name; the buffer name is `ipu_firmware#<heap>` when a name came back, else `ipu_firmware`;
5. allocate the firmware buffer (0xED800 B), mmap, `rd(va, 0, fwSize, path)` — the file is copied whole and unchanged
   to offset 0; nothing on the host parses it (it has no header, `ipu-firmware-1-runtime-model.md` §2.1); then the
   "ipu_variable" (MaxVariableBufSize) and "ipu_heap" (heap size) buffers; flush; unmap the firmware;
6. ioctl 2 InitFirmware {firmware MIU, heap MIU, heap MIU + size − 1};
7. a 0x80-byte buffer "ipu_version", ioctl 20 → string; ioctl 23 SetLibVer {"579e2d5fa321b41c4f00ea5d76ec9fc8",
   "T_0.0.1_210525"} (a failure is only printed);
8. split the version string at the first '-': the text after it is the "IPU SDK version" (printed, stored); the text
   before it must equal `579e2d5fa321b41c4f00ea5d76ec9fc8`, else "IPU version isn't matching" (0x17); the digits before
   the first '.' of the SDK version (`Q_0.1.0…` → "0") must equal "0", else "IPU fw major version(%s) and mi major
   version(%s) not matching!" (0x17). The Anjoy firmware (same hash, `Q_0.9…`) would pass this check [I];
9. ioctl 12 SetIpuParameter {3; W_YUV, H_YUV, W_XRGB} (failure → ioctl 3 DestroyFirmware, free all, 0x13).

Any failure after step 6 stops the firmware (ioctl 3) and frees the three buffers.

Record of ioctl 18 / 19 (0x148 B): +0x10 u64 firmware MIU, +0xA0 u64 variable-buffer MIU, +0xA8 u64 heap MIU, +0xB8
char[128] SDK version, +0x138 u32 MaxVariableBufSize; the other bytes are uninitialised stack.

### 5.3 `MI_IPU_DestroyDevice()` [R]
Under the device lock: count 0 → 0x15; count − 1; at 0: ioctl 17, and if the kernel's count is now 0, ioctl 3
(DestroyFirmware) and free the firmware, variable and heap buffers; destroy the mutex; close the fd.

### 5.4 Model file and channels [R]
A model file ("offline model") starts with a 0x13AC-byte header, magic `SGSI` [R on the Anjoy model]; the library
reads only its first 0x13A4 bytes (up to +0x13A3) and uses these fields (full format: `ipu-firmware-1-runtime-model.md` §6,
SGSI model file):

| off | field | use by the library |
|---|---|---|
| +0x131C | size A — the model flatbuffer (identifier `TFL3`) [R firmware] | model size |
| +0x1320 | variable (scratch) buffer size the network needs | must be ≤ MaxVariableBufSize (else 9) |
| +0x1324 | size B — the offline command flatbuffer (`TFL3`) [R firmware] | model size |
| +0x1362 | page size *al* (power of two; unaligned field) | alignment of the parts |
| +0x136E | model version string | printed ("Model version: %s") |
| +0x13A0 | size C — the fast-run model (`TFL3`) [R firmware] | model size |

Not read by the library: the hash at +0x132C and the MAC count at +0x1366 [R firmware]. Model size = align(0x13A4) +
align(A) + align(B) + align(C), aligned to *al*. Check on the Anjoy model: *al* 4096, A 759 172, B 501 180, C 0, total
1 273 856 = the file size; variable 2 304 000 (the size reported on the camera, §6).
`MI_IPU_GetOfflineModeStaticInfo(rd, path, out)` returns {variable size, model size} from this header alone (no device
needed).

**`MI_IPU_CreateCHN(&chn, ChnAttr *a, rd, path)`** — ChnAttr {SubNetId (unused), OutputBufDepth, InputBufDepth}, depths
≤ 3 (else 10); device created (else 0x15); path required (0x19):
1. read the header, compute the model size; ioctl 7 → channel id (0x100 → "no available IPU channel", returns 0x100);
2. a per-channel library record of 0x11B0 B; a "ipu_chn%u_network" buffer of the model size, mmap, read the **whole
   model** into it; a "ipu_chn%u_desc" buffer of **0x9C58 B**, zeroed; flush both;
3. ioctl 9 CreateChannel {5; chn; network MIU; desc MIU; variable-buffer MIU; MaxVariableBufSize} → the firmware parses
   the model and fills the description buffer (§5.6) [I: who fills it; the host only reads it];
4. the input and output counts in the description must be non-zero (0xB);
5. a channel private pool (`MI_SYS_ConfigPrivateMMAPool`, module 0x1D, channel chn) of Σ page-aligned tensor size ×
   depth over inputs and outputs, if non-zero;
6. per depth and tensor one buffer "ipu_chn%d_%s_port%d_depth%d" (`input` / `output`, size = the tensor's aligned size),
   mmapped and marked free; the network buffer is unmapped. "client [%d] Create IPU channel %u".

**`MI_IPU_DestroyCHN(chn)`**: frees the tensor buffers, deletes the private pool, ioctl 10 {1; chn}, unmaps and frees
the desc and network buffers, ioctl 8 (put id).

### 5.5 Tensors and invoke [R]
- `MI_IPU_GetInOutTensorDesc(chn, out)`: converts the firmware description (§5.6) to the public
  `MI_IPU_SubNet_InputOutputDesc_t` (entries 0x140 B) [S type]; counts must be < 62 (0xB). An INT16 tensor with the
  "float" flag is reported as FP32 (format 5), scale 1.0, zero point 0, and for outputs with the inner-most stride
  doubled.
- `MI_IPU_GetInputTensors` / `GetOutputTensors(chn, vec)`: take the first free depth slot, fill `vec` = {count; per
  tensor `ptTensorData[0]` = VA, `phyTensorAddr[0]` = MIU}, mark the slot busy; none free → 5 "no avaiable %s buffers".
  `Put…` gives the slot back by matching tensor 0's address ("the address is illegal", 6).
- `MI_IPU_Invoke(chn, in, out)`: the counts must equal the description's (0xC). It builds the **invoke record (0x7C0
  B)**:

  | byte | content |
  |---|---|
  | 0x000 | u32 channel |
  | 0x008 | u32 input count |
  | 0x010 + 16·i | input *i*: u64 `phyTensorAddr[0]`, u64 `phyTensorAddr[1]` (i < 61) |
  | 0x3E0 | u32 output count |
  | 0x3E8 + 16·i | output *i*: u64 `phyTensorAddr[0]`, u64 `phyTensorAddr[1]`; for a float-flagged output: `phy[0]` + aligned size/2 only |
  | 0x7B8 | u8 force-online flag (written by the kernel, §4.5) |

  Before the call: float-flagged INT16 inputs are converted in place from FP32 (`ptTensorData[0]`) to INT16 with
  1/scale (NEON helpers at 0x1118C / 0x111F8, saturating), then flushed; all output buffers are flushed. ioctl 6. After
  a successful call: every float-flagged output must be INT16 (else 0xC, "The output data isn't int16 type…") and is
  converted INT16 → FP32 × scale in place, row by row with the inner-most stride ("!!! Attention: c_align < 4…" when the
  channel count is below 4). User buffers that are not from the slots work too, since the record carries only MIU
  addresses [I].

### 5.6 Firmware tensor description (desc buffer, 0x9C58 B) [R: library reads]
{u32 inputCount (+0, read as a byte in places); u32 outputCount (+4); entry in[61] at +0x8; entry out[61] at +0x4E30};
entry 0x148 B:

| off | field | public field it becomes [S names] |
|---|---|---|
| +0x00 | u32 dimension count | `u32TensorDim` |
| +0x04 | u32 inner-most stride | `u32InnerMostStride` |
| +0x08 | u32 aligned buffer size (also the slot buffer size) | `s32AlignedBufSize` |
| +0x0C | u32 element format (`MI_IPU_ELEMENT_FORMAT`) | `eElmFormat` |
| +0x10 | u8 float-output flag (outputs; `bFP32Out`); +0x11 u8 the same for inputs | (format → FP32) |
| +0x14 | f32 scale | `fScalar` |
| +0x18 | s64 zero point | `s64ZeroPoint` |
| +0x20 | u32 shape[8] | `u32TensorShape` |
| +0x40 | char name[256] | `name` |
| +0x140 | 8 bytes, not read | — |

## 6. On the running camera (2026-10-18) [R camera]
<!-- ev:ipu-host.6 -->

### 6.1 A compiled model through the public API
- Program: a small test program of ours on the public M6 `mi_ipu.h` API: `MI_SYS_Init(0)` →
  `MI_IPU_GetOfflineModeStaticInfo(NULL, path)` → `MI_IPU_CreateDevice(attr, NULL, NULL, 0)` (default firmware path) →
  `MI_IPU_CreateCHN(NULL readfunc, path)` → get tensors → `MI_IPU_Invoke`. The camera's own `/config/dla/ipu_firmware.bin`,
  `mi_ipu.ko` and `libmi_ipu.so`; the stock app running in mode 29 (4K@45) with USB video, uptime 5 min.
- Model: `sypcna2.4803013_V2.1_fixed.sim_sgsimg.img` from the Anjoy firmware (md5 ca15bfefb40f69d90e39ec22497a3d88),
  header `SGSI`, converter string `Q_0.9_verified_21091510`, payload not encrypted (entropy ~7 bit/byte, zlib ratio
  0.55). The `Q_0.9` model was accepted by the `Q_0.1.0` firmware.
- Versions printed: "IPU SDK version" `Q_0.1.0_verified_21110517` (the firmware's, §3.6), driver `T_0.0.1_210525`, hal
  `T_0.4_210224`.
- Tensors: input `images` ARGB8888 1×480×800×3; outputs three INT16 heads 1×3×60×100×11, 1×3×30×50×11, 1×3×15×25×11
  (scale ≈ 0.00056); variable buffer 2 304 000 B. The shape reads as a YOLOv5-style detector, 3 anchors, 11 = 4 box + 1
  objectness + 6 classes [I]. The model asks for memory order B, G, R, x: its SubGraph `aInputFormat` = 4 = BGRA [R
  model, `ipu-firmware-1-runtime-model.md` §6.3].
- The same input twice gave identical output checksums, a different input different ones.
- 40.1 ms per invoke (average of 20) ≈ 25 inferences/s at 800×480, alongside the live 4K@45 pipeline; the camera stayed
  up.
- `/sys/dla/power` read `off` between runs; the failed `dla_power` probe at boot does not stop the NPU.
- `libmi_ipu.so` needs `libcam_fs_wrapper.so` (CamFs*) linked explicitly (§5). `MI_IPU_DevAttr_t` pitch alignments must
  be set (16 / 2 / 16, the header defaults); zero → `CreateDevice` returns 0x14 (`E_IPU_ERR_INVALID_PITCH_ALIGNMENT`).
- Decoding of the model's output (anchors, class meaning) is not established.

### 6.2 Detection on the camera's own video (2026-10-18 ~18:30–18:45)
A second test program of ours: frames from the app's SCL dev0 / chn0, the model on the IPU, boxes drawn into an
MI_RGN OSD region (handle 9, ARGB1555, full port-0 size) on SCL port 0, so they show on HDMI and USB. The model was
brought to `/tmp` as gzip parts (`ipu/push.sh`); it does not fit the 944 KB left free on `/customer`. Scene: `bus.jpg`
on a laptop screen in the microscope's field of view; camera in stock mode 4K30 (fast boot had
fallen back, `/customer/fast/fails` = 2), port 0 = 1920×1080 NV12.

| path | frames/s | per frame |
|---|---|---|
| CPU: SCL port 0, user depth 1, NV12 → 800×480 nearest + BT.601 on the CPU | 8–10 | convert 51–69 ms, IPU 37 ms, draw 11–14 ms |
| scaler: SCL port 1 switched on from the second process at 800×480 in an RGB format, the CPU only copies (`-scl 1`) | 17–18 | copy 3 ms, IPU 40 ms, draw 13 ms; camera CPU ~25 % busy |

- Byte order: SCL `ARGB8888` (pixel format 1) gives the B, G, R, x memory order the model wants; SCL `BGRA8888` (3) is
  the reverse and loses detections (bus.jpg: 2 people instead of 4). On the CPU path `bgr0` / `rgb0` / `0bgr` / `0rgb`
  scored alike.
- Model behaviour: people in full figure found (4 on bus.jpg, scores 0.34–0.83, class 0); the bus once as class 4 at
  0.33; close half-length portraits (zidane.jpg) score ≤ 0.36. Reads as a person / vehicle detector for surveillance
  views [I].
- Full-4K windows (`-tile 5x5`): SCL port 1's crop moved every processed frame over a 5×5 grid of 800×480 windows on the
  3840×2160 scaler input (1:1, overlapping), 2 frames dropped after each move; boxes merged across windows (same-class
  IoU > 0.45 or ≥ 70 % inside a kept box). One full scan 2.5 s (~10 windows/s).
- Small people (26×11 mosaic of bus.jpg rendered at the panel's physical 3072×1920; a person ≈ 110 px tall in 4K): whole
  frame scaled to 800×480 → 0 boxes; 5×5 windows → 19–32 boxes per scan. Big people (6×3 mosaic, ≈ 380 px tall): whole
  frame 25, windows 4–5 (people cut by the windows, and large figures score low anyway).
- With the port moved at once after a round of windows is read, so the IPU works while the scaler settles, and frames
  captured before the move dropped by PTS (`u64Pts` vs `MI_SYS_GetCurPts`, same clock; first fresh frame +35…90 ms after
  the move): full 5×5 scan 1.6 s instead of 2.5 s — wait 75–175 ms, IPU 1.1 s (≈ 44 ms × 25), draw ~0.1 s. The IPU is
  then the limit.
- Extra scaler ports on dev0: only ports 1 and 2 enable; port 3 → `0xA0222003` (ILLEGAL_PARAM, no hardware scaler mapped
  to it). With 2 ports the scan time stays the same (IPU-bound) and the box count rises to the 64 cap — port 2's windows
  likely do not land where expected [I].
