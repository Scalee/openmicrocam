# ipl — IPL and IPL_CUST, the two boot stages before U-Boot
<!-- ev:ipl -->

The first two programs the SoC runs from the NOR flash. **IPL** (NOR 0x0, run from on-chip IMI SRAM by the boot ROM) sets
the early clocks, the CPU PLL and the console UART, initialises DRAM and the MIU, starts the GIC, turns the MMU on, finds
IPL_CUST through the MXPT partition table, copies and checks it and jumps to it; it also carries an OTP (eFuse) service
that U-Boot calls later. **IPL_CUST** (NOR 0x10000, run from DRAM) finds U-Boot through MXPT, copies it with the BDMA
engine while XZ-decoding it, checks its data CRC and jumps to it. This document owns the OTP / eFuse fields and the IPL →
U-Boot hand-over; the partition map is `firmware-image.md` §1.1, U-Boot's own side is `uboot.md`, RIU addressing, the
clock tree after boot, the MIU arbiter registers and the chip id are `kernel-platform.md` §3.1, §5.1, §12.3, §12.4, §3.2.

Tags: **[R]** read from the flash dump, the binary, its decompile, the emulation trace, the boot log or a cited source file;
**[S]** stated by a source; **[I]** inference. A heading tagged [R] means every statement under it is [R] unless marked
otherwise. Values called "emulated" are [R] for the code path they come from; where a value depends on a register an
emulator cannot know (eFuse trims, training results) this is said at the place.

**Notation.** `bank.reg` = one 16-bit RIU register (bank → CPU address: `kernel-platform.md` §3.1); `.h` or
`h` after it = its high byte. Banks < 0x1000 are PM-domain, ≥ 0x1000 non-PM. In the write sequences: registers in write
order; several values = successive writes to that register; byte writes show 2 hex digits; `*` = read-modify-write (the
value shown is what the emulator computed with untouched bits = 0 — on hardware those bits keep their value; the
operation is given in the text); `‖n‖` = `delay(n)`. **Bank names** used below, from the mercury6 sources
(`registers.h`, `ms_complex_clk.c`, U-Boot `hal_disp_reg.h`) [R]: 1004 mailbox / postcode, 1010 DDR PHY,
1011 / 1012 MIU controller and arbiter, 1013 MIU_E extension, 1018 OTP, 101E CHIPTOP, 101F OTP2 (eFuse words), 1022
MIU pipe, 1031 MIUPLL, 1032 CPU PLL, 1033 LPLL, 1035 IPU PLL (identified from mhal's IPU block, §3.8), 1037 DISP-LPLL, 1038 CLKGEN,
1108 UART0, 1122 hash / RSA engine, 111B XTAL_ATOP, 1420 UPLL0, 1421 UTMI0 (USB PHY), 145x HDMI-TX, 0016 FSP (PM flash
serial programmer), 001E PMTOP, 0030 PM timer / reset status, 0034 PM, 0038 PM "DIDKEY", 003F PMPADTOP, 1002 BDMA. Not
named in any source on hand: 1030, 1039, 1131, 002A, 100E, and most fields of 1010 / 1012 / 1013 / 1038 (§9).

## 1. The two images [R]

### 1.1 Place, load address, entry

| | IPL | IPL_CUST |
|---|---|---|
| NOR offset / slot | 0x00000 / 64 KiB | 0x10000 / 64 KiB |
| used size (header) | 0x7400 = 29 696 B | 0x5C90 = 23 696 B |
| magic (offset 4) | `IPL_` (0x5F4C5049) | `IPLN` (0x4E4C5049, `IPLK_HEADER_CHAR` in `ipl.h`) |
| runs at | 0xA0000000, IMI SRAM, loaded by the boot ROM [R: string / code pointers 0xA0006xxx] | 0x23C00000, DRAM (`IPL_CUST_LOAD_ADDRESS` = MIU0 + 0x03C00000 in U-Boot `arch-mercury6/mach/platform.h`) |
| entered | at +0 (ARM, `b +0x20`) by the ROM; further entry points §2.1 | at +0 (ARM) by IPL, `blx 0x23C00000` |
| build id | `MVX4##m6##ga0090c8IPL_###########XVM` at +0x72C0; prints `IPL a0090c8` | the same string at +0x5610; prints `IPL_CUST ga0090c8` |
| code | ARM stub 0x00–0x13F, then Thumb-2 | ARM stub 0x00–0x7F, then Thumb-2 |

Bytes after the used size up to 0xFFFF are 0xFF in both slots. The flash uses 64 KiB slots, not the U-Boot `platform.h`
layout "IPL 48K / IPL_CUST 60K / KEY_CUST 4K": IPL_CUST is at 0x10000, nothing is at 0x1F000 (erased), MXPT is at 0x20000.
IPL_CUST + 0x523F (NOR 0x1523F) holds the magic constants of IPL_CUST's embedded XZ decoder (`FD 37 7A 58 5A 00` header
magic, `59 5A` footer magic), not an XZ stream.

### 1.2 Header (16 bytes, both images)

Field names from `ipl.h` in `dl/u-boot-sigmastar-git/src/include/`.

| off | size | field | IPL | IPL_CUST | who checks |
|---|---|---|---|---|---|
| 0x00 | 4 | ARM branch to the code (`ea000006` = `b +0x20`) | ✓ | ✓ | — |
| 0x04 | 4 | magic | `IPL_` | `IPLN` | IPL checks `IPLN` before and after loading IPL_CUST |
| 0x08 | 2 | image size in bytes, header included | 0x7400 | 0x5C90 | IPL (copy length, CRC end) |
| 0x0A | 1 | `auth` | 0x00 | 0x00 | not read by IPL |
| 0x0B | 1 | `cid`; **0xFA = image is signed** | 0x00 | 0x00 | IPL and IPL_CUST read IPL's *own* byte 0x0B (0xA000000B) to decide authentication (§4.5, §5.2) |
| 0x0C | 2 | CRC-16 of bytes [0x10, size) | 0x56DB | 0xFE70 | IPL checks IPL_CUST's (§4.5); IPL's own is checked by the ROM [I] |
| 0x0E | 2 | — | 0x0003 | 0x0003 | not read by IPL; meaning not known [I: format / version word] |
| 0x10 | 2 | IPL_CUST only: `KEYN` length (`IPL_CUST_IMAGE_KEYN_OFFSET`) | (code) | 0x0000 | IPL_CUST (§5.2) and U-Boot `sigauth` |
| 0x12 | 2 | IPL_CUST only: `KEYAES` length | (code) | 0x0000 | IPL_CUST (§5.2) and U-Boot `sigauth` |

**CRC-16:** reflected polynomial 0xA001, init 0xFFFF, no final xor (CRC-16/MODBUS), over the bytes from 0x10 up to the
header size; recomputed for both images and equal to the header (`0x56DB`, `0xFE70`). U-Boot's `calc_checksum` (32-bit
word sum) in `cmd_sigauth.c` is a different check and does **not** match these headers (0x9BCBA392 / 0x4190C1CE).

IPL_CUST is an `IPLN` image with `KEYN = KEYAES = 0`: it carries no customer RSA / AES key; U-Boot `sigauth` would fall
back to the SigmaStar / OTP key, and on this board neither stage authenticates (byte 0x0B = 0) [R header; I consequence].

### 1.3 What the boot ROM does

The boot ROM is not in the flash and is not on hand. Known from IPL's side only [R]: the ROM loads IPL to 0xA0000000 and
enters it at +0 in ARM state; IPL never sets SP (inherited from the ROM); the ROM leaves a boot-source status in 0038.70
(§4.4). Not known: the SP value, the exact CPU mode, what the ROM checks in IPL's header (`auth`, `cid`, the CRC, the word
0x0003), and whether the ROM stays readable at 0x0 after boot (§9).

## 2. IPL: entry points, code layout, `main` [R]

### 2.1 Entry points

| entry | mode | what it is | caller |
|---|---|---|---|
| +0x00 | ARM | header branch `b +0x20` to the ARM stub, which calls `main` (§2.3 step 1) | boot ROM |
| +0x10 | ARM | OTP (eFuse) service: `push {r0–r12, lr}; blx 0xA00036E4; pop; mov pc, lr` (§7.1) | U-Boot `otpctrl` through its SVC handler (§7.1) |
| +0x98 | ARM | CP15 context restore: r0 = context; r5 = [r0]; from [r0 + 0x98]: VBAR, TTBR0, TTBR1, TTBCR, DACR, PAR, PRRR, NMRR, CONTEXTIDR, TPIDRURW, TPIDRURO, TPIDRPRW, then SCTLR; `bx r5` | none in IPL or IPL_CUST — unused here; it looks like a warm-boot / resume helper for a caller outside these images [I] |
| +0x11C | ARM | crt-style "call [GOT + 0x11F8C] if non-zero" stub (`frame_dummy`-like); the slot points outside the image | none — unused |

### 2.2 Code layout

| range | content |
|---|---|
| 0x0000–0x013F | ARM stub (header, entry points of §2.1) |
| 0x0140–0x0826 | RSA / big-number code on bank 0x1122 (authentication only, §4.5) |
| 0x08B6–0x0A24 | UART, timer and FSP (bank 0x0016) primitives; `delay` at 0x096C |
| 0x0B2C–0x3208 | SPI-NAND driver (CIS / GCIS / PNI parsing, ECC status, BDMA) — §6 |
| 0x0DE8 | DRAM size probe (§3.9) |
| 0x0FA0 | 5-bit signed add used by the DQ-deskew trims (§3.8) |
| 0x0FC0 / 0x1198 / 0x1704 | DRAM init variants (0x1198 = STR resume twin of 0x1704; §3.5) |
| 0x1C38 | software read-DQS training (bonding 0x27 only, §3.12) |
| 0x22E8–0x23F4 | MXPT check, strncpy, time stamp, halt (`0x23B8`), MIU-pipe flush (`0x23D8`) |
| 0x322C | loader (§4.4–§4.5) |
| 0x36E4 | OTP service body (§7.1) |
| 0x3878 | `main` |
| 0x641C–0x6620 | ARM-mode MMU / cache helpers (`0x641C` MMU on, `0x6588` MMU / D-cache off, `0x6624` D-cache clean+invalidate by set/way) |
| 0x66DC | memcpy |
| 0x6B14 | memset |
| 0x68A6, 0x6A92, 0x6BAC | library-looking routines with no caller anywhere in IPL — unused (SHA-256 is done by the 0x1122 engine, §4.5) |
| 0x6C74–0x73FF | strings and tables (OTP field table 0xA00072F8, deskew offsets 0xA00072F0, feature table 0xA0007398, MXPT candidates 0xA00073C8, build id 0x72C0) |

**Delays:** `delay(n)` (0xA000096C) waits until PM timer0 (0030.12–0030.15) has advanced n ticks. The timer runs at 12 MHz
[I: SigmaStar PM timer default], so 1200 = 100 µs, 3000 = 250 µs, 6000 = 0.5 ms, 12000 = 1 ms.

### 2.3 `main` (0xA0003878) in order

1. **ARM stub** (0x00–0x9F, before `main`): mailbox 1004.00 (32-bit) = 0xA001; `CPUECTLR` (`mrrc/mcrr p15,1,c15`) |= 0x40
   (SMPEN); `NSACR` |= 0xC00 (cp10/11 non-secure access); `CPACR` = 0x00F00000; `FPEXC` = 0x40000000 (VFP / NEON on);
   1004.00 = 0xA002; `blx main`; if `main` returns: 1004.00 = 0xA0FF and spin.
2. **Postcodes and boot-time record.** 1004.00 = 0xB001; the time-stamp record at **0xA000F000** is cleared (count = 0)
   and stamp #0 written; delays; postcodes 1004.00 = 0xB002 … 0xB012. The record format is U-Boot's `struct timecrecord`
   (`arch/arm/cpu/armv7/mercury6/prof.c`, `RECORD_IPL_ADDR 0xA000F000`): u32 count, then entries of 40 B {u32 time_us =
   CNTPCT/6, u32 mark, char name[32]}; at most 31 entries are written by IPL / IPL_CUST (the code checks `< 0x1F`); U-Boot's
   `btime` command prints it. Stamps: IPL `I` (0), `IPL+`, `MIU+`, `MIU-`, `LDR+`, `LDR-`, `JMP+` (`LoadBL++` / `LoadBL--`
   on SPI-NAND); IPL_CUST `CMN+`, `CMN ` (mark 2), `CLDR+`, `CLDR-`, `CDEP-`, `CJMP+`.
3. **Early clocks, CPU PLL, feature fuses, UART** (§3.1–§3.3). Prints `\r\nIPL a0090c8\r\n`, `D-<bonding hex>\n\r`, then
   the reset reason: `WDT Reset` if 0030.02 bit 0, else `SW Reset` if 0006.01 bit 0, else `HW Reset`.
4. **LPLL / DISP-LPLL / bank 0x1039 / UPLL0** (§3.4), stamp `MIU+`.
5. **DRAM + MIU** (§3.5–§3.10): branch on the bonding id → the `MCP2133_4X` init (0xA0001704) or its resume twin
   (0xA0001198), or the inline bonding-0x27 init, or halt; MIU-PLL choice (`miupll_<f>MHz`); arbiter; clocks; OTP trims;
   size probe (`256MB`); BIST (`BIST0_0001-OK`); software DQS training only for bonding 0x27 (else
   `No support sw autoK flow` ×2).
6. **GIC** distributor + redistributors (§3.11), print `GIC Init` (only if 001E.01h = 1).
7. **SPI-NAND probe** (always, §4.1), MMU + caches on (§4.2), power-down of fused-off blocks and the last CHIPTOP / 1131
   writes (§4.3).
8. **Loader** 0xA000322C (§4.4–§4.5): find IPL_CUST, copy it to 0x23C00000, optional RSA check, CRC-16, jump.

### 2.4 Board inputs on this camera

The values the code reads that reproduce this camera's boot log; everything else reads 0 in the emulation.

| register | value | why | where it is read |
|---|---|---|---|
| 101E.48 (CHIPTOP, bonding id) | 0x11 | the log prints `D-11` = this byte & 0xF7 | `main` 0xA000396A |
| 0038.70 (PM "DIDKEY" bank, boot-source status from the ROM) | 0x0020 | NOR path; 0x0002 takes the same path [R code]; which of the two the ROM sets is not known | loader, IPL_CUST |
| 001E.01 high byte (PMTOP) | 0x01 | the log prints `GIC Init`, printed only when this byte is 1 | 0xA0006194 |
| 0016.65 (FSP read data) | 0xFF | the SPI-NAND probe must fail on a NOR part (no `Found bad block` lines in the log) [R log; I that NOR answers 0xFF] | SPI-NAND probe |
| 0030.02 bit 0, 0006.01 bit 0 | 0 | the log prints `HW Reset` | reset-reason print |
| 0034.41 (PM, STR magic) | ≠ 0xBABE | the log prints `MCP2133_4X`, not `… Resume` | 0xA0003C8C |
| status polls 1032.5D b0, 1012.70 b15, 0016.6E b0, 1002.01 b3, 1022.10 b12 | "done" | completion bits | — |
| 101F.50–5B (OTP2 trim words), 101F.26 / 27 / 2C–2F / 71 (OTP2 feature / lock words) | 0 | **not known on this chip**; with 0 every trim / disable step is skipped (§3.3, §3.8, §7) | — |

## 3. IPL: clocks, DRAM and MIU

### 3.1 Early sequence (0xA0003878–0xA0003C1E) [R trace]
```
1004.00=B001 111B.09=00 ‖1200‖ 1030.01h=00 ‖12000‖ 1004.00=B002 1038.20=04 1038.17=00*,0C,1C* 1038.01=04,24
000E.20=80 1022.3Ah=80 1004.00=B012 0034.0F=0001* 003F.06=0008*,0008*,000A* 003F.04=0008*,0008*,000A*
003F.07=0008*,0008*,000A* 003F.05=0008*,0008*,000A* 1001.17=0001* 1032.12=88 1032.12h=00 1032.11h=01 1032.61=45
1032.60h=1E 1032.60=B8 1032.62=01 1032.11h=00 1032.50=1EB8 1032.51=0045 ‖1200‖ 1032.52=1EB9 1032.53=0045
1032.58=0001 1032.55=0006 1032.57=0008 1032.59=1000* 1032.54=0000,0001,0000 1032.50=1EB9 1032.51=0045 ‖3000‖
1020.7C=01 000E.30=10* 1038.31=00,00 000E.09=0000* 101E.53=3210 1108.1C=00*,01* 1108.02=00 1108.08=10* 1108.06=80*
1108.00=5E 1108.02=00 1108.06=00*,03 1108.08=00* 1108.04=07 101E.53=3012 000E.09=0800* 1038.34=0000* 101E.03=0020*
101E.53=3102*
```
Between `1004.00=B012` and `0034.0F` the feature-fuse check of §3.3 runs; between `1032.54=0001` and `=0000` the code
waits for 1032.5D bit 0; between `1038.31=00` and `=00` it polls 1108.0A bit 6 up to 256 times.

### 3.2 What the early writes are [R]

| step | registers | operation / meaning |
|---|---|---|
| analog | 111B.09 = 0 (XTAL_ATOP), 1030.01h = 0 | power-up bits [I] |
| CLKGEN | 1038.20 = 4; 1038.17: &= ~0x10, = 0x0C, \|= 0x10; 1038.01 = 0x04 then 0x24 | clock selects; the "clear bit 4 – set mux – set bit 4" pattern on 1038.17 is a glitch-free switch [I]; which clocks: not named in any source on hand [R] |
| PM | 000E.20 low = 0x80; 1022.3Ah = 0x80; 0034.0F \|= 1 | power / isolation enables [I]. 000E.20 **high** byte is the SPI-flash clock select (IPL_CUST pairs its values with the `SPI 54M/86M/104M/108M` prints: 0x14 / 0x08 / 0x04 / 0x0C, each \| 0x20; §5.3) [R] |
| pads | 003F.06, .04, .07, .05: each `\|= 8; &= ~4; \|= 2` | PMPADTOP pad configuration (drive / pull) [I]; which pads: not named [R] |
| — | 1001.17 \|= 1 | meaning not known |
| **CPU PLL** (bank 0x1032) | 1032.12 = 0x88, 1032.12h = 0, 1032.11h = 1; synth 1032.60 / .60h / .62 = **0x451EB8**; 1032.11h = 0; LPF low 1032.50 / 51 = 0x451EB8; ‖100 µs‖; LPF high 1032.52 / 53 = 0x451EB9; 1032.58 = 1 (LPF control); 1032.55 = 6; 1032.57 = 8 (update count); 1032.59 \|= 0x1000 (low → high); 1032.54 = 0, 1, wait 1032.5D bit 0, 0; LPF low = 0x451EB9 | register names from mercury6 `ms_complex_clk.c` (`0x1032A0..BA`) [R]. Rate = 432 MHz·2^19 / LPF · 2 / post_div · 16 → LPF 0x451EB8 gives 50 MHz·16·2 / post_div = **800 MHz with post_div 2** [R formula; I that 1032.19 (post_div − 1, never written by IPL) resets to 1] |
| — | 1020.7C = 1; 000E.30 \|= 0x10 | meaning not known |
| **UART0** (bank 0x1108; 8250-style registers at 8-byte stride [I]) | 1038.31 = 0 (UART clock select); wait LSR (1108.0A) bit 6 ≤ 256×; 1038.31 = 0; 000E.09 &= ~0x800; **101E.53 = 0x3210** (pin mux); 1108.1C bit 0 cleared then set; IER (1108.02) = 0; MCR (1108.08) \|= 0x10 (loopback); wait 1108.0E bit 0 clear (≤ 255×; on timeout mailbox 1004.00 = 0x0BF1 and the divisor is not written); LCR \|= 0x80; **DLL = 0x5E, DLH = 0**; LCR &= ~0x80; LCR = 3 (8N1); MCR &= ~0x10; FCR (1108.04) = 7; 101E.53 = 0x3012; 000E.09 \|= 0x800; 1038.34 &= 0xFFF0; 101E.03 = (x & ~0x30) \| 0x20; 101E.53 = (x & 0xF00F) \| 0x100 → 0x3102 | divisor 94 at 115 200 Bd ⇒ UART clock ≈ 172.8 MHz [I]; the console runs at 115 200 [R log]. 101E.53 nibbles = UART port → pad-group map [I from the values 0x3210 → 0x3012 → 0x3102] |
| print | 1108.00 | `putc` waits for LSR bit 5, writes THR |

### 3.3 Feature fuses and the lock check (before the CPU PLL) [R decompile]

- Table at 0xA0007398: 12 entries {u16 id, u16 bit}, ids 0–11 (the bits and what IPL does for each: §7.2) → the set
  of disable bits IPL knows = 0x0F1C.
- If OTP2 101F.2E & 3 = 3: if 101F.2F & 3 ∈ {1, 2}: mask = (101F.2D << 16) | 101F.2C, and if the mask has a bit
  outside 0x0F1C → print `halt on disabled-func rule-2` and hang; otherwise (2F & 3 ∈ {0, 3}) mask = 0.
  If 101F.2E & 3 ∈ {1, 2}: mask = (101F.27 << 16) | 101F.26; a bit outside 0x0F1C → `halt on disabled-func rule-1`.
  If 101F.2E & 3 = 0: mask = 0. The mask is stored at 0xA00120D0.
- If 101F.71 bits 12–13 are non-zero and ≠ 3 → print `halt!!!` and hang.
- The mask is applied after the MMU is on (§4.3). On this chip the fuse words are not known (§9); the boot reaches U-Boot,
  so no halt rule fires [R log]. The same words feed the capability flags mhal reads (§7.3).

### 3.4 LPLL, DISP-LPLL, bank 0x1039, UPLL0 (after the reset-reason print) [R trace]
```
1033.48=A3 1033.48h=8B 1033.49=2E 1033.49h=00 1033.40=01 1033.40h=22 1033.41=20 1033.41h=04 1033.42=43 1033.42h=00
1033.43=02 1033.43h=00 1033.44=00 1033.44h=05 1033.4A=01 1033.4Ah=00 1033.4B=00 1033.4Bh=00
1037.48 … 1037.4Bh = the same 18 bytes
[if 1420.00 & 0x18: 1420.00=C0, 1420.0E=11, ‖1200‖]      (UPLL0 power-up; not taken in the emulation, 1420.00 read 0)
1039.00=27 1039.00h=76 1039.01=42 1039.33=00 1039.34=02 1039.37=00 1039.36=00
```
0x1033 = `REG_LPLL_BASE`, 0x1037 = `REG_DISP_LPLL_BASE` (U-Boot `disp/hal/mercury6/hal_disp_reg.h`) [R]; synth word
0x002E8BA3 in regs 0x48 / 0x49 [R values; I meaning]. 0x1039 is not named in any mercury6 source on hand; IPL_CUST writes
the same 0x7627 / 0x42 to it for the 104 MHz SPI clock (§5.3), so it is a PLL feeding the SPI clock [I]. The UPLL0 step
depends on the hardware state of 1420.00 bits 3–4 [R]; whether it runs on this camera is not known (§9).

### 3.5 Which DRAM init runs [R decompile 0xA0003E26–0xA0003EB4, 0xA0005984]

b = 101E.48 & 0xF7 (printed as `D-xx`); STR = (0034.41 == 0xBABE). The kernel's `mstar_rtc_magic_set` (mercury6 `pm.c`)
writes 0xBABE to 0034.04 and latches it through the PM "ISO" sequence before suspend [R]; that 0034.41 is the latched
read-back is [I].

| b | init | print | MIU PLL | runs on this camera |
|---|---|---|---|---|
| 0x30, 0x31 | 0xA0001704(b) (STR: 0xA0001198(b)) | `MCP2133_4X` (`… Resume`) | fixed 300 MHz | no |
| 0x00, 0x01 | the same functions | the same | fixed 300 MHz | no |
| **0x11** | **0xA0001704(0x11)** (STR: 0xA0001198(0x11)) | **`MCP2133_4X`** | **computed → 266 MHz** | **yes** [R log `D-11`] |
| 0x27 | inline sequence in `main` (STR: an inline resume variant) | `MCP2133_4X` (`… Resume`) | computed (468 MHz with its PLL value) | no |
| other | — | `BondingID no support, halt !!!` | — | — |

Inside 0xA0001704 only two things depend on b: the PHY skew block (b = 0x11 → 1010.3A / 3B = 0x0404, 4A = 0x1515,
50–53 = 0x1220 / 0x2221 / 0x1000 / 0x0001; else 0x0606 / 0x1516 / 0x2223 / 0x1212 / 0x1222 / 0x2122) and the final
1010.00 value (b ∈ {0, 0x30} → 0x000A, else 0x000C). After the init `main` adds, for b < 2 only: 1010.44 &= ~0x10,
1010.45 |= 0x200. "MCP2133_4X" read as "multi-chip-package DDR3-2133, 4:1 PHY ratio" fits the numbers of §3.7 [I].

On this board the DRAM init is table-driven plus a BIST; no DRAM training runs — software DQS training exists only for
bonding 0x27 (§3.12; the log's `No support sw autoK flow`).

### 3.6 This board's DRAM init 0xA0001704(0x11) [R trace, all absolute writes]
```
1012.0F=0C00,0C00,0C00,0C01 1012.23=FFFE 1012.33=FFFF 1012.43=FFFF 1012.53=FFFF 1011.03=FFFF 1011.13=FFFF
1011.73=FFFE 1010.3C=0001 ‖12000‖ 1010.12=1000 ‖12000‖ 1010.12=0000 ‖12000‖ 1010.1B=0400 1010.1A=2004 1010.45=0001
1010.18=67D7 1010.19=001A ‖12000‖ 1010.11=0004 1010.16=0114 1010.44=0011 1012.01=02A3 1012.02=0055 1012.03=1680
1012.04=24EE 1012.05=3287 1012.06=E80A 1012.07=C0AB 1012.08=1124 1012.09=4004 1012.0A=8028 1012.0B=C000 1012.14=00C0
1013.11=0003 1013.20=0E0E 1013.21=0724 1013.22=3208 1013.23=100A 1013.24=0E08 1013.25=0604 1013.26=052A 1013.27=00AB
1013.28=E000 1013.29=0000 1013.2A=0E00 1013.40=0040 1013.43=0000 1013.7F=0000 1011.30=0000 1011.31=0000 1011.32=0000
1011.33=0030 1011.34=5000 1010.01=AAAA 1010.02=0008 1010.05=2200 1010.07=0099 1010.17=2222 1010.1C=0077 1010.1D=6063
1010.1E=9222 1010.1F=A022 1010.24=0077 1010.25=6060 1010.26=0022 1010.27=0022 1010.28=1111 1010.29=000B 1010.36=0808
1010.37=0808 1010.3A=0404 1010.3B=0404 1010.4A=1515 1010.50=1220 1010.51=2221 1010.52=1000 1010.53=0001 1010.54=1111
1010.55=1111 1010.56=1111 1010.57=1111 1010.5B=0000 1010.5C=2222 1010.5D=0222 1010.5E=0222 1010.5F=0222 1010.68=4444
1010.69=4444 1010.6A=4444 1010.6B=4444 1010.6C=0044 1010.70=4444 1010.71=4444 1010.72=4444 1010.73=4444 1010.74=0044
1010.31=007F 1010.32=F000 1010.30=00CB,00CF,00CB,00C3,00CB,00C3,00CB,00C2,00C0,33C8 1010.38=0000 1010.4C=0000
1010.4D=0000 1010.48=F0F1 1010.38=0800 1012.16=8021 1012.7E=951A 1012.29=FFFF 1012.39=FFFF 1012.49=FFFF 1012.59=0000
1011.09=FFFF 1011.19=FFFF 1012.20=8015 1012.30=8015 1012.40=8015 1012.50=8015 1011.00=8015 1011.10=8015 1010.45=0001
1010.38=0800 1010.2C=0A0A 1010.2D=AAAA 1010.2E=AAAA 1010.2F=AAAA 1010.0D=8000 1010.0E=0020 1010.04=003F
1010.0F=0005,000F,0005 1012.0F=8C01,8C00 1010.00=0008 1010.0C=2000 1010.3E=0000 1010.2A=C000 1010.0F=0005,000F,0005
1010.00=000C 1012.00=0000 ‖12000‖ 1012.00=0008,000C ‖12000‖ 1012.00=000E ‖12000‖ 1012.00=001F ‖12000‖ ‖12000‖
‖12000‖ 1010.0F=0005,000F,0005 1012.23=7FFE 1011.73=FFFA 1012.7F=A0E1,80E1 1012.78=0000
```
Phases [R order; I phase names]:
- (a) 1012.0F soft-reset pattern and **all arbiter request masks set** (1012.23 / 33 / 43 / 53, 1011.03 / 13 = 0xFFFF,
  1011.73 = 0xFFFE — no client can reach DRAM);
- (b) DDR PHY / PLL power-up: 1010.3C, reset pulse on 1010.12, PLL dividers 1010.1A / 1B, **DDR PLL synthesizer
  1010.18 / 19 = 0x1A67D7**, 1 ms;
- (c) DRAM type and timings in the controller 1012.01–0B, 1012.14, and the MIU_E extensions 1013.11, 1013.20–2A,
  1013.40 / 43 / 7F; 1011.30–34;
- (d) PHY drive / ODT / delay tables 1010.01–07, 17, 1C–29, 36 / 37, the board skew block 3A / 3B / 4A / 50–53, 54–57,
  5B–5F, 68–6C, 70–74;
- (e) 1010.30 command sequence (0xCB, CF, CB, C3, CB, C3, CB, C2, C0, 0x33C8 — a PHY DLL / calibration kick [I]) with
  1010.31 = 0x7F, 1010.32 = 0xF000;
- (f) arbiter preset for init: group "+9" registers 1012.29 / 39 / 49 = 0xFFFF, 1012.59 = 0, 1011.09 / 19 = 0xFFFF
  (written once here and never changed afterwards by IPL); group cfg 1012.20 / 30 / 40 / 50 and 1011.00 / 10 = 0x8015;
- (g) DQ deskew defaults 1010.2C = 0x0A0A, 2D–2F = 0xAAAA; pads 1010.0D / 0E / 04; three latch pulses on 1010.0F
  (5, F, 5); 1012.0F = 0x8C01, 0x8C00; 1010.00 = 8, 1010.0C = 0x2000, 1010.2A = 0xC000;
- (h) **JEDEC init through 1012.00 = 0 → 8 → 0xC → 0xE → 0x1F with 1 ms steps** (3 ms after the last), latch pulse,
  masks partly opened (1012.23 = 0x7FFE, 1011.73 = 0xFFFA), 1012.7F = 0xA0E1, 0x80E1, 1012.78 = 0.

Register fields a source names [R]: 1012.00 low byte = DRAM init / command control (the kernel's `sram.S` enters
self-refresh with 1012.00 = 0x202E and issues commands through the high byte); 1012.0C = 0x0400 "pre-charge all"
(`sram.S` comment); 1012.69 bits 15:12 = DRAM size code for MIU protect (`MIU_PROTECT_DDR_SIZE` 0x80 = 256 MB in i6c
`mercury6/regMIU.h`); 1012.70–78 BIST (§3.10; MIU BIST registers: `kernel-platform.md` §12.5); 1010.00 bits 3 / 4, 1010.2A
bits 4–6 and 1010.04 bits 0–5 = PHY analog power-down controls (`sram.S` "AN power down"). All other 1010 / 1012 / 1013
fields: no source on hand names them; the values above are what this board's DRAM needs.

### 3.7 DDR frequency and the MIU PLL [R decompile 0xA0004740–0xA0004830]

`main` reads back the DDR PLL synthesizer s = (1010.19 & 0xFF) << 16 | 1010.18 (= 0x1A67D7) and halves s if 1012.01
bits 3:2 = 01 (they are 00 here). With F = 432·2^20 (the constant 0x1B000000 compared against s·f) the DDR clock is F/s =
**261.8 MHz** [R arithmetic]. The first f in {100, 133, 150, 166, 200, 216, 233, 266, 300, 333, 400} with s·f ≥ F picks
the MIU PLL (bank 0x1031 = MIUPLL); s below all → 468:

| print `miupll_<f>MHz` | 1031.03 (loop N) | 1031.03h (post) | 24 MHz·N / (post + 2) [I] |
|---|---|---|---|
| 100 / 133 / 150 | 0x15 / 0x1C / 0x1F | 3 | 100.8 / 134.4 / 148.8 |
| 166 | 0x1C | 2 | 168 |
| 200 / 216 / 233 | 0x19 / 0x1B / 0x1E | 1 | 200 / 216 / 240 |
| **266** | **0x2C** | **2** | **264** |
| 300 (and forced for b ∈ {0, 1, 0x30, 0x31}) | 0x32 | 2 | 300 |
| 333 / 400 | 0x2A / 0x32 | 1 | 336 / 400 |
| 468 | 0x27 | 0 | 468 |

Then [R trace]: 1031.02 = 0x0000, 1031.03 = N | post << 8, 1031.04 = 0x0010, 1031.01h = 0 (power on), ‖250 µs‖.
DRAM data rate on this board = 4 × 261.8 MHz × 2 ≈ 2094 MT/s if "4X" is the PHY : controller clock ratio [I].

### 3.8 After the init, in order (`MIU-` … `256MB`) [R trace]
```
1031.02=00 1031.02h=00 1031.03=2C 1031.03h=02 1031.04=10 1031.04h=00 1031.01h=00 ‖3000‖ 1012.20=801D 1012.21=1008
1012.30=801D 1012.31=2010 1012.40=801D 1012.41=4020 1012.50=801D 1012.51=2010 1012.4C=FFFC 1013.32=0000 1013.33=0000
1013.36=0000 1013.37=0000 1013.3A=0000 1013.3B=0000 1013.3E=0000 1013.3F=0000 1013.4F=002C 1011.70=0301 1011.7A=8054
1011.71=007E 1011.75=8008 1011.76=0402 1011.77=0204 1011.7C=FFCD 1012.7F=9000 1035.12=88 1035.12h=00 1035.11h=01
1035.61=45 1035.60h=1E 1035.60=B8 1035.62=01 1035.11h=00 1038.17=0C*,08,18* 1022.01=84 ‖12000‖ 1038.31=00
1038.61=04,15 1038.60=10 1038.32=0C,1C* 000E.20h=14,34* 100E.72=01 1038.64=09 1038.71=03 1038.71h=00 002A.03=38
002A.0A=03 1038.61=14 1038.6A=00* 1012.23=0000 1012.33=0000 1012.43=0000 1012.53=0000 1011.13=0000 1011.73=0000
1012.0F=8C08 «print '256MB\n'» 1012.69=8000* 1012.6F=0001*,0000* 1012.70=0000 1012.71=0000 1012.72=FFFF 1012.73=00FE
1012.74=5AA5 «print 'BIST0_'» 1012.78=0000 «print '0001'» 1012.70=0001 «print '-OK\n'» 1012.78=0000 1012.70=0000
«print 'No support sw autoK flow\nNo support sw autoK flow\n'» 1012.7F=9800* «print 'GIC Init\n'»
```
- **Arbiter** 1012.20 … 1011.7C, 1012.7F = 0x9000: the IPL's arbiter policy (vendor table at NOR 0x4800–0x48B0). The DRAM
  init of §3.6 first sets all six group cfgs to 0x8015 and closes all request masks; the 0x801D values follow here, after
  the MIU PLL. The meaning of each arbiter register, the policy the kernel later loads over it and the change of groups
  2 / 3 by `/etc/init.d/rcS` are `kernel-platform.md` §12.3, §12.4.
- **IPU PLL** (bank 0x1035): the same 9-byte pattern as the CPU PLL start (1035.12 = 0x88, 1035.11h = 1, synth
  1035.61 / .60h / .60 = 0x451EB8, 1035.62 = 1, 1035.11h = 0). mhal's IPU block identifies 0x1035 as the IPU PLL; by its
  formula f = 432·2^23 / synth, 0x451EB8 = **800 MHz**. mi_ipu.ko later moves it to 900 MHz from the device-tree OPP list
  (`ipu-host.md` §2.3).
- **Clocks**: 1038.17 (&~0x10, = 8, |0x10); 1022.01 = 0x84; ‖1 ms‖; 1038.31 = 0; 1038.61 = 4, 0x15; 1038.60 = 0x10;
  1038.32 = 0x0C, |0x10; **000E.20h = 0x14, |0x20 = SPI flash clock 54 MHz** (IPL_CUST's table, §5.3); 100E.72 = 1;
  1038.64 = 9; 1038.71 = 3, 1038.71h = 0; 002A.03 = 0x38; 002A.0A = 3; 1038.61 = 0x14; 1038.6A &= 0x7F. The meanings of
  these CLKGEN fields are in no source on hand.
- **OTP2 trims** — each step runs only if its fuse "valid" bit is set; with all fuse words 0 (emulation) none ran, so on
  the camera any of them may run (§9) [R decompile 0xA000466C–0xA0004EE0]. The values come from each chip's own fuses.

| fuse word, valid bit | what is written |
|---|---|
| 101F.55 b7 | 1010.58 bits 14:9 = (101F.55 & 0x7F) >> 1 (before `MIU-`) |
| 101F.55 b11 | DQ deskew: 101F.55 bits 10:8 index the signed table at 0xA00072F0 {−2, −4, −6, 0, 2, 4, 6}; each 4-bit field of 1010.2C (bits 11:8), 1010.2F (all four nibbles) and 1010.2D (bits 11:8, 15:12), with a 5th bit kept in 1010.6F, gets the offset added by 0xA0000FA0 (5-bit add with ±6 correction across the 0x10 boundary, saturated to 0…31) (before `MIU-`) |
| 101F.55 b15 | the same for 1010.2C bits 3:0, 1010.2E (four nibbles), 1010.2D bits 3:0 and 7:4, index = 101F.55 bits 14:12 |
| 101F.50 b6 | 0034.23 bits 5:0 = 101F.50 & 0x3F (address from pointer arithmetic on 1038.61) |
| 101F.51 b15 | 000E.64 \|= 1; 0014.7F \|= 1; 0014.79 bits 2:0 = 101F.51 bits 14:12; 0014.7E bits 5:0 = 101F.51 bits 11:6; 000E.64 bits 5:1 = 101F.50 bits 12:7 (PM SAR / LDO trims [I]) |
| 101F.52 b8 | 1421 (UTMI0 USB PHY): 1421.16 bit 15 = 101F.52 bit 4, 1421.17 bits 2:0 = bits 7:5, 1421.14 bits 8:5 = bits 3:0 |
| 101F.53 b1 | 1429 (second UTMI [I]): v = 101F.52 >> 13 \| (101F.53 & 1) << 3; 1429.16 bit 15 = v bit 0, 1429.17 bits 2:0 = v >> 1, 1429.14 bits 8:5 = 101F.52 bits 12:9 |
| 101F.55 b0 | data w = 101F.54: 1523.46 \|= 0x2000; 1523.50 bits 15:10 = w bits 15:10; 1523.0E \|= 4; 1523.3C bits 8:4 = w bits 9:5; 1523.0B \|= 0x10; 1523.2A bits 11:7 = w bits 4:0; 143A.00 = 0; then the same five steps on bank 143B (SATA / PCIe-type PHY trims [I]) |
| 101F.57 b2 | w = 101F.56: 151A.30 \|= 4; 151A.34 \|= 0x8000; 151A.42 bits 4:0 = w bits 4:0, bits 12:8 = w bits 9:5; 151A.41 bits 7:4 = w bits 13:10; 151A.40 bits 3:0 = w bits 15:14 \| (101F.57 bits 1:0) << 2; 1518.2E \|= 7 (an analog block, unnamed) |
| 101F.57 b5 / b8 | 153E.10 / 1546.10: 2-bit fields from 101F.57 bits 4:3 / 7:6 replicated into bits 0–11 |
| 101F.57 b13 | 1535.14 bits 12:11 = 101F.57 bits 12:11; 1535.45–1535.49 bits 1:0 = 101F.57 bits 10:9 |
| 101F.58 b7 / b15, 101F.59 b7 / b15 | HDMI-TX ATOP 145B.1A / 1B / 1C / 2C low 7 bits from the fuse bytes |
| 101F.5B b9 | 145A.38 bits 4:0 = 101F.5A bits 4:0; 145A.19 bits 4:0 = 5A bits 9:5, bits 12:8 = 5A bits 14:10; 145A.1A bits 4:0 = 5A bit 15 \| 5B bits 3:0 << 1, bits 12:8 = 5B bits 8:4 (HDMI-TX trims) |

  Bank names 1421 = UTMI0 and 145x = HDMI-TX are from mercury6 `registers.h` and the mhal map [R]; the rest [I].
- **Open the arbiter**: 1012.23 / 33 / 43 / 53, 1011.13, 1011.73 = 0; 1012.0F = 0x8C08 (normal operation).

### 3.9 Size probe 0xA0000DE8 [R]

Writes 0x11111111 @0x20000000, 0x33333333 @0x22000000, 0x22222222 @0x24000000, 0x44444444 @0x28000000, 0x88888888
@0x30000000, 0x99999999 @0x40000000, then reads back in order and takes the first match:

| read | size printed | 1012.69 code |
|---|---|---|
| @0x20000000 still 0x11111111 | 1024 MB | 0xA |
| @0x30000000 = 0x88888888 | 512 MB | 9 |
| @0x28000000 = 0x44444444 | **256 MB** (this board) | **8** |
| @0x24000000 = 0x22222222 | 128 MB | 7 |
| @0x22000000 = 0x33333333 | 64 MB | 6 |
| @0x20000000 = 0x99999999 | 32 MB | 5 |

For each: print, 1012.69 = (1012.69 & 0x0FFF) | code << 12, then pulse 1012.6F bit 0 (set, clear), return the size. No
match: if b = 0x27 and 1012.01 bit 2: print `Single EXT-DDR3`, clear 1012.01 bit 2, set 1012.6B bit 0, wait 250 µs and
retry; else print `Access DRAM fail`, flag 0xFF, return 64 MB. With the flag `main` prints `ReInit MIU`, re-runs 1012.00 =
0, 8, 0xC, 0xE, 0xF (0.5 ms / 100 µs steps, 1 ms) and probes once more. The probe works by address wrap-around: on 256 MB
the writes to 0x30000000 / 0x40000000 land on 0x20000000 [I, consistent with the result]. It overwrites those six words.

### 3.10 BIST [R]

1012.70 = 0, 1012.71 = 0 (start 0), 1012.72 = low 16 and 1012.73 = high bits of ((size − 0x100001) · 0x1000 >> 16) and
(size − 0x100001) >> 20 (256 MB: 0xFFFF / 0x00FE), 1012.74 = 0x5AA5 (pattern), print `BIST0_`, 1012.78 = 0, print `0001`,
1012.70 = 0x0001 (start), wait 1012.70 bit 15; bits 14:13 ≠ 0 → `-FAIL`, `[HALT]`; else `-OK`, 1012.78 = 0, 1012.70 = 0.
The test covers the whole DRAM but the top 1 MiB and is destructive [I].

### 3.11 After BIST: GIC [R]

b ≠ 0x27: print `No support sw autoK flow` twice. 1012.7F |= 0x800 (the "TV tool / BIST" client switch; MIU:
`kernel-platform.md` §12.5). If 001E.01h = 1, GICv3 at 0x16000000: GICD_CTLR &= ~7, wait RWP (bit 31); GICD_CTLR |= 0x30
(ARE_S, ARE_NS), wait; GICD_IGROUPR[1 … ITLinesNumber] (0x16000084 …) = 0xFFFFFFFF; GICD_CTLR |= 7, wait; CPU0
redistributor: SGI-frame ICENABLER0 (0x16050180) = 0xFFFFFFFF, wait GICR_CTLR.RWP (0x16040000 bit 3), IGROUPR0
(0x16050080) = 0xFFFFFFFF; CPU1: ICENABLER0 (0x16070180) = 0xFFFFFFFF, wait 0x16060000 bit 3, IGROUPR0 (0x16070080) =
0xFFFFFFFF; CPU0 GICR_WAKER (0x16040014) &= ~2 (ProcessorSleep), wait ChildrenAsleep (bit 2) clear; print `GIC Init`. All
interrupts end up Group 1 (non-secure) [R values; I purpose: Linux runs non-secure].

### 3.12 Variant paths (unused on this camera) [R: emulation; full logs]
<!-- ev:ipl.3.12 -->

- **b = 0x00 / 0x30** (`var-bond00`, `var-bond30`): differs from this board's path only in the skew block (§3.5),
  1010.00 = 0x000A, MIU PLL N = 0x32 (300 MHz); b = 0x00 also gets 1010.44 &= ~0x10, 1010.45 |= 0x200.
- **STR resume, b = 0x11** (`var-resume`): 0xA0001198 = the same tables, but 1012.01 starts as 0xF2A3, 1010.04 = 0
  (pads held), 1010.00 / 0C / 2A are walked byte-wise (1010.00 → 0x08, 0x18, high 0x60; 1010.0C high 0x20, 0x60;
  1010.2A = 0xC070, 1010.00 = 0x601C), then the self-refresh exit: BIST off, masks closed (1011.73 = 0x00FE), ‖1 ms‖,
  1012.0C = 0x0400 (pre-charge all), 1012.00 low = 0x2F, high byte commands 04, 04, 05, 04, 02, 02, 03, 02, ‖1 ms‖,
  1012.00 = 0x202E, ‖1 ms‖, 1010.00 low 0x1C → 0x0C, high 0, 1010.0C high 0x60 → 0x20, ‖1 ms‖, 1012.01h = 0x02,
  1010.2A low = 0, 1010.04 = 0x3F, latch pulse 1010.0F, 1012.00h = 0, 1011.73 = 0x00FE, 1012.00h = 02, 02, 03, 02,
  1012.00 low = 0x0E, ‖1 ms‖, 0x1E, ‖1 ms‖, 1012.0F = 0x8C08, masks opened. **After it IPL continues exactly like a cold
  boot** — size probe and BIST (both overwrite DRAM), IPL_CUST, U-Boot; neither stage reads the kernel's
  `SIG_STR` / `resume_entry` block at 0x20000000 nor jumps to a resume vector [R: no reference to 0xBABE, 0x5A5A55AA or
  0x20000000 in IPL_CUST; IPL's only 0x20000000 use is the probe] — so suspend-to-RAM cannot resume with this IPL [I].
- **b = 0x27** (`var-bond27`): inline init in `main` (different PHY tables; the same DDR PLL value 0x1A67D7 but
  1012.01 = 0x02A7, whose bits 3:2 = 01 halve s before the PLL choice → `miupll_468MHz`; MIU_E 1013.27 = 0x0118 among
  others); the size probe may fall back to `Single EXT-DDR3`; then **software DQS training** instead of
  `No support sw autoK flow`: `SW ReadDQS Training` (0xA0001C38: per byte lane it sweeps the read-DQS delay over 0…31
  with BIST passes, keeps the window [min, max], programs the centre, prints `Err, skew %d over 15` if the window is too
  wide) and `SW WriteDQS Training` (inline, the same sweep on write DQS, prints `min:` / `max:`, `write autoK R … fail`).
  Its results come from the hardware; the emulation shows only the register walk [R strings / structure; I algorithm
  summary].

## 4. IPL: from `GIC Init` to the jump into IPL_CUST

### 4.1 SPI-NAND probe (always runs) [R decompile + trace]

The PM flash serial programmer (FSP, bank 0x0016) sends raw SPI commands: 0016.6C = 7, 0016.75 = 0016.76 = 0, command /
address bytes into 0016.60–0016.62 (byte-addressed write buffer), 0016.6A = bytes to write, 0016.6B = bytes to read,
0016.6D = 1 (go), wait 0016.6E bit 0 (≤ 10 000 polls), read data from 0016.65 (byte-addressed read buffer), 0016.6F = 1
(clear). Sequence: (1) `GET FEATURE 0xC0` (0016.60 = 0xC00F) repeated until bit 0 (OIP) clears, ≤ 10 000×; (2) mailbox
1004.02 = 0x20, 101E.50 = 0, 0017.7A = 0, 000E.35 = 8; (3) `RESET 0xFF`, then GET FEATURE 0xC0 until OIP clears,
≤ 10 001×. On a NOR flash the status byte reads 0xFF, both loops time out and the NAND branch is skipped [R code; I for the
NOR answer, consistent with the log]. What runs when a NAND answers is §6.

### 4.2 MMU and caches on (0xA000641C, ARM mode) [R]

L1 section table at **0x20004000** (in DRAM), TTBR0 = 0x20004000, DACR = 3, then SCTLR |= M | C | I:

| VA | PA | size | descriptor | memory type [I from the bits] |
|---|---|---|---|---|
| 0x00000000 | 0x00000000 | 1 MiB | 0x0C0E | normal, write-back (boot ROM) |
| 0x14000000 | 0x14000000 | 16 MiB | 0x0C0E | normal WB (SPI-NOR window) |
| 0x1F000000 | 0x1F000000 | 4 MiB | 0x0C02 | strongly ordered (RIU) |
| 0x20000000 | 0x20000000 | 256 MiB | 0x0C0E | normal WB (DRAM) |
| 0x40000000 | 0x20000000 | 128 MiB | 0x1C02 | normal non-cacheable alias of DRAM |
| 0xA0000000 | 0xA0000000 | 128 MiB | 0x0C0E | normal WB (IMI SRAM) |

The table stays in DRAM and the MMU stays on through IPL_CUST until just before U-Boot (§5.2 step 10): IPL_CUST is entered
with the MMU **on** (this identity map).

### 4.3 Fused-off blocks and the last CHIPTOP writes [R]

For each entry of the 0xA0007398 table (§3.3) whose bit is in the stored disable mask, IPL powers the block down with
the writes listed for its id in §7.2 (ids 3, 8, 9, 10; ids 2, 4, 11 are accepted but have no action). Then always: **101E.1F = 0, 101E.6C = 2, 101E.1F = 1, 1131.10 = 0x0010, 101E.6C = 3** [R
trace; meanings in no source on hand].

### 4.4 Finding IPL_CUST (0xA000322C) [R]

Stamp `LDR+`. Boot source s = 0038.70 & 0x14BE:

| s | path |
|---|---|
| 0x20 or 0x02 | SPI-NOR (this board) |
| 0x10 or 0x04 | SPI-NAND: block loader via the PNI table, `Load BL from SPINAND`, stamps `LoadBL++` / `LoadBL--`, copy to 0x23C00000 of IPL_CUST size + 0x210, or 0xF000 with `NO Header` (§6) |
| 0x1000 | prints `Load  from USB` / `Load IPL_CUST from USB` and goes straight to the checks of §4.5 on whatever is already at 0x23C00000 (the ROM's USB download) [I] |
| any other | the same as 0x1000 |

NOR path:
1. MXPT search: for each candidate offset in the table at 0xA00073C8 = {0xA000, 0xC000, 0xF000, **0x20000**, 0x40000,
   0x80000}: base = 0x14000000 + offset; the table is valid if record 0 or record 29 passes 0xA00022E8: word at +0 =
   `MXPT`, bytes +0x7D / +0x7E / +0x7F / +0x80 = `T`, `P`, `X`, `M`. Record count = index of the first record whose byte
   +5 is 0 (max 29). Print `MXP found at 0x%08x`.
2. Record lookup by name `IPL_CUST` (16-byte compare at record +0x20, record stride 0x88); offset = u32 at record +0x08
   (record +0x10 = size). No record → print `No mxp IPL_CUST record!!`, offset = 0. (MXPT as a whole: `firmware-image.md`
   §1.1.)
3. If there is no `IPLN` magic at the offset but there is one at (u16 IPL size at flash + 8) + 0x100, that offset is used
   (an IPL_CUST placed right after a signed IPL and its 256-byte key) [R code; I purpose].
4. Print `offset:%08x`; no `IPLN` there → ` -Verify  SnorOp Header failed!`, `[HALT]`.

A flash-ops table is set up at 0xA00120D4 = {0xA0003209 (memcpy from base + 0x20000), 0xA00022E5, 0xA00022E1 (both
return 0)} with 0xA00120E8 = 0x1000; nothing on the NOR path calls it.

### 4.5 Copy, authentication, CRC, jump [R]

- D-cache clean+invalidate by set / way (0xA0006624), MIU pipe flush (0xA00023D8: 1022.05 = 0, 1; wait 1022.10 bit 12 —
  the kernel's `Chip_Flush_MIU_Pipe` writes the same 0x1F204414 / polls 0x1F204440 [R U-Boot mercury6 `chip.c`]).
- **Copy.** If 001E.00 = 0xF4 and 001E.01 bit 8 = 0: CPU `memcpy(0x23C00000, 0x14000000 + offset, size + 0x210)`, print
  `CPU read`. **Else BDMA** — the path of this camera (the log has no `CPU read`): up to 5000 × (1 ms) wait for 1002.01
  bit 1 clear, else print `MDrv_BDMA_FlashToMem error!! device is busy!!`; 1002.01 = 0x1C (clear), 1002.02 = 0x4035
  (source SPI, destination MIU [I]), 1002.03 = 0, 1002.04 / 05 = source = CPU address & ~0xF (0x0000 / 0x1401),
  1002.06 / 07 = destination MIU offset (0x0000 / 0x03C0 = 0x23C00000), 1002.08 / 09 = length (size + 0x21F) & ~0xF
  (0x5EA0), 1002.00 = 1, wait 1002.01 bit 3 [R trace]. 001E.01 bit 8 is bit 0 of the "GIC byte" 001E.01h, which is 1 on
  this camera (§2.4), so the BDMA path follows from that bit alone and the log does not show whether the chip id 001E.00
  is 0xF4 [I] (chip id: `kernel-platform.md` §3.2).
- Stamp `LDR-`. **Authentication only if IPL's own header byte 0x0B (0xA000000B) = 0xFA or 0038.70 bit 0 = 1**; on this
  board the byte is 0x00 and the log shows no `KEYN_ADDRESS` line, so it is off. When on: print `KEYN_ADDRESS(0x%08x)` with
  KEYN = 0xA0000000 + IPL size − 0x100 (the last 256 bytes of the IPL image), clean caches, flush MIU, then
  `0xA0000826(data = 0x23C00000, len = IPL_CUST size, sig = 0x23C00000 + size, key = KEYN)`: SHA-256 of [data, data +
  len) on the hash / RSA engine (bank 0x1122: 0xA00006E2 = init, address, length, mode 1, start, wait bit 0, read 8
  words); RSA public operation on the 256-byte signature (0xA000074C: key length 0x800 bits, key source = memory N at
  `key`, or the internal / OTP key when `key` is 0 or its first word is 0; exponent loaded by 0xA0000310 without an
  argument); pass = the first 8 result words equal the digest (no PKCS#1 padding check). Fail →
  ` -Authenticate image 08 failed!`, `[HALT]`; pass → 0012.04 |= 4.
- **CRC-16/MODBUS** over [0x23C00010, 0x23C00000 + size) against the u16 at 0x23C0000C (§1.2) → `Checksum NG\r\n in
  hdr:%04x \r\n%04x` + halt, or `Checksum OK`, stamp `JMP+`, D-cache clean, MIU flush, **`blx 0x23C00000`** (ARM state,
  MMU and caches still on, IRQs as left by the ROM, no arguments).
- `[HALT]` (0xA00023B8): MMU / D-cache off (0xA0006588), print, spin.

## 5. IPL_CUST (runs at 0x23C00000)

### 5.1 Entry, memory and code layout [R]

ARM stub: `mov sp, #0x23800000` (the stack grows down below IPL_CUST); `blx main` (0x23C03AE4, Thumb); if `main` returns:
mailbox 1004.00 = 0xA0FF and spin. A second ARM routine at +0x44 switches to monitor mode (`cps #0x16`), sets SCR.NS,
writes **CNTVOFF = 0** (`mcrr p15, 4, r0, r0, c14`), restores SCR and returns in SVC mode — nothing in IPL_CUST calls it
(no literal 0x23C00044 / 45 anywhere); a "secure CNTVOFF init" helper for a later stage [I]. Unused here.

Data / bss above the image [R addresses; I field meaning]:

| address | content |
|---|---|
| 0x23C0F000 | NAND header buffer |
| 0x23C10000 | stamp-like counter |
| 0x23C10004 … | flash-ops / MXPT state |
| 0x23C10024, 0x23C16F50 | XZ decoder state |
| **0x23C17400** | backup flag (§5.2 steps 2, 9) |
| 0x23C17404 | CRC-32 table (built at run time, poly 0xEDB88320) |
| 0x23D00000 | scratch (default load address) |
| 0x21000000 | XZ dictionary (8 MiB, BDMA-XZ mode) |

Code layout:

| range | content |
|---|---|
| 0x0136–0x08A4 | the same SHA / RSA engine helpers as IPL (bank 0x1122) |
| 0x08F2–0x0E88 | UART / timer / FSP primitives (the same as IPL) |
| 0x0990 | BDMA FlashToMem |
| 0x0F3C | halt |
| 0x0F5C | MIU flush |
| 0x0F78–0x13FF, 0x1E7C | LZMA-type "MZ" decoder |
| 0x1410–0x1460 | CRC-32 |
| 0x14E4–0x1D2C | SPI-NAND block loader (PNI, backup blocks; §6) |
| 0x1E34 | be32 |
| 0x1E38 / 0x1E54 | uImage entry / load |
| 0x2BA8 / 0x2BE0 | MZ (CPU / BDMA-pipelined) |
| 0x2EC4–0x3AE0 | xz-embedded (`xz_dec_reset`, `xz_dec_run`, LZMA2) |
| 0x3AE4 | `main` |
| 0x4988–0x4A24 | ARM MMU / cache helpers |
| 0x4ADC | memcpy |
| 0x4F14 | memset |
| 0x5074–0x5C8F | strings, CRC table (0x23C05640), MXPT candidates (0x23C05A40), `MSTARSEMIUSFDCIS` |

### 5.2 `main` [R decompile + trace]

1. Stamp `CMN+`; print `\r\nIPL_CUST ` + the 8-char build id taken from the `MVX4…` string (`ga0090c8`) + `\r\n`; stamp
   `CMN ` (mark 2).
2. Backup flag init: only if the boot source is SPI-NAND (0x10 / 0x04) and mailbox 1004.00 ≠ 0xA003: 0x23C17400 = 0. The
   value at entry is kept as `first`. On NOR the flag is never initialised (whatever DRAM holds).
3. Stamp `CLDR+` (the reload loop restarts here). Boot source from 0038.70 & 0x14BE as in IPL (§4.4).
4. **NOR**: MXPT search with the same six candidates and checks as IPL (§4.4), print `MXP found at 0x%08x`; record named
   `UBOOT` → offset (no record: `No mxp UBOOT record!!`, offset 0); print `offset:%08x`; the word at 0x14000000 + offset
   must be the uImage magic 0x27051956 (big-endian), else ` -Verify  SnorOp Header failed!` and halt. Clean D-cache,
   flush MIU.
5. **Copy and unpack** — chosen by the chip id and `ih_comp` (header byte 0x1F):
   - 001E.00 = 0xF4 and 001E.01 bit 8 = 0 ("CPU read" chips): `memcpy(load, flash + offset, ih_size + 0x140)`, print
     `CPU read`; decompression later (step 8).
   - otherwise (**this camera**): `ih_comp` = 0 → BDMA of ((ih_size + 0x14F) & ~0xF) bytes from (flash + offset) & ~0xF
     to load = entry − 0x40 and wait; `ih_comp` = 9 ("MZ") → BDMA-pipelined MZ decode (0x23C02BE0), print
     `MZ decomp_size=0x%08x`; **`ih_comp` = 3 (this U-Boot: the header says lzma, the payload is XZ) → BDMA-pipelined
     XZ**: copy the 64-byte header to `load` (0x23F00000) by CPU, build the CRC-32 table, `xz_dec_reset`, decoder in
     multi-call mode with dictionary 0x21000000 / limit 0x800000; start BDMA channel 0 of 64 KiB from flash + offset +
     0x40 to load + 0x40 (`0x23C00990(0, src, dst, 0x10000)`: 16-byte alignment check, ≤ 5000 × 1 ms busy wait, the same
     register pattern as IPL §4.5 with destination = dst − 0x20000000), wait done; then loop: whenever 1002.01 bit 1 is
     clear and data remains, clean D-cache, flush MIU and start the next 64 KiB chunk; feed `xz_dec_run` in 0x4000-byte
     steps up to the bytes already copied, output to `entry` (0x23E00000) with limit 0x100000; stop on stream end or when
     all input is consumed; > 10 000 000 iterations → `BDMA_XZ timeout!`. Print `XZ decomp_size=0x%08x` (output size).
     On this board: 4 chunks (flash 0x30040, 0x40040, 0x50040, 0x60040 → 0x23F00040 … 0x23F30040), 0xAF8C8 bytes out
     [R emulation, equal to the log].
6. **Authentication** — only if IPL's header byte 0x0B (still readable at 0xA000000B) = 0xFA or 0038.70 bit 0; off on
   this board. KEYN = 0x23C00000 + IPL_CUST size − KEYAES length − KEYN length (header +0x12 / +0x10), printed as
   `KEYN_ADDRESS(0x%08x)`; clean, flush; `0x23C002C0(data = load + 0x40, len = ih_size, sig = data + ih_size, key)` — the
   same SHA-256 + raw-RSA check as IPL §4.5; fail → ` -Authenticate image 01 failed!` and halt, pass → 0012.04 |= 4.
   With this IPL_CUST (KEYN = KEYAES = 0) the key pointer would be the end of the image, i.e. no customer key [R; I].
7. Stamp `CLDR-`. **Data CRC**: magic again at `load`, CRC-32 (table at 0x23C05640, init 0xFFFFFFFF, final xor) over
   [load + 0x40, + ih_size) against `ih_dcrc` — on the BDMA path after the XZ decoding. **The header CRC `ih_hcrc` is
   never checked.** Mismatch → print ` -Verify CRC32 failed!`, `calc_crc32=0x…`, ` header dcrc=0x…` and go to step 9.
   Missing magic → ` -Verify CRC32 Header failed!` → step 9. Print `u32RunAddr = 0x%08x` (entry) and
   `laodaddr = 0x%08x` (load).
8. CPU-read chips only (already done on this camera): `ih_comp` 9 → MZ (0x23C02BA8: out = entry, limit 0x100000); 3 → XZ
   single-call (dictionary limit 0x4000, out = entry, limit 0x100000) with the error prints `ERR!! Memory allocation
   failed` / `Memory usage limit reached` / `Not a .xz file` / `Unsupported options in the .xz headers` / `File is
   corrupt` / `Bug!`; 0 → nothing (the image is already at entry).
9. **Reload backup**: if 0x23C17400 still equals `first`: 0x23C17400 = (first == 0), print `Reload backup file`, go back
   to step 3; else print `u-boot corrupted, halt` and halt. The flag is read only by the SPI-NAND block loader
   (0x23C01AE0, which then takes the backup copy); on NOR the retry re-reads the same `UBOOT` record, so a corrupt U-Boot
   fails twice and halts.
10. Stamps `CDEP-`, `CJMP+`; D-cache clean+invalidate by set / way (0x23C04A24); MIU flush (1022.05 = 0, 1; wait 1022.10
    bit 12); **MMU and D-cache off** (0x23C04988: SCTLR &= ~(C | M); the I-cache bit stays as IPL set it);
    `blx entry` (0x23E00000). If it returns: `u-boot corrupted, halt`.

uImage helpers: entry = be32 `ih_ep` (+0x14), 0 → 0x23E00000; load = `ih_comp` 0 → entry − 0x40, else be32 `ih_load`
(+0x10), 0 → 0x23D00000. (The U-Boot uImage header itself: `firmware-image.md` §1.3.)

### 5.3 Register writes of IPL_CUST; the SPI clock table [R trace; table R decompile]

On this board's path only the MIU-pipe flush (1022.05 = 0, 1 before every BDMA chunk and before the jump) and the five
BDMA programmings — no clock, pad or DRAM register is touched. The SPI clock stays at IPL's 54 MHz (000E.20h = 0x34).
The SNI-driven SPI clock table exists only on the SPI-NAND branch (SNI clock code, §6):

| SNI clock code | writes | print |
|---|---|---|
| 0x36–0x55 | 000E.20h = 0x14 \| 0x20 | `SPI 54M` |
| 0x56–0x67 | 1038.71 rewritten with its own value, 1038.6D &= ~0x20, 000E.20h = 0x08 \| 0x20, 003F.14 \|= 0x80 | `SPI 86M & SPI_CK driving` |
| 0x68–0x6B | 1030.01 = 0, 1039.00 = 0x7627, 1039.01 = 0x42, 1039.27 = 1, 1039.36 = 0, 000E.20h = 0x04 \| 0x20, 003F.14 \|= 0x80 | `SPI 104M & SPI_CK driving` |
| ≥ 0x6C | 003F.14 \|= 0x80, 000E.20h = 0x0C \| 0x20 | `SPI 108M & SPI_CK driving` |
| else | — | `Unknown clk 0x1b … halSPINANDCLK.h 00000031`, `[I]e4` |

## 6. SPI-NAND and USB boot, both stages (unused on this board) [R decompile + strings; structures from U-Boot sources]

The SPI-NAND code follows SigmaStar's UNFD / SPINAND structures in `dl/u-boot-sigmastar-git/src/drivers/mstar/spinand/
inc/common/{spinand.h, drvSPINAND.h}`; the offsets the IPL code uses match them.

- **CIS page** = 16-byte tag `MSTARSEMIUSFDCIS` + `SPINAND_FLASH_INFO_t` ("SNI"): +0x00 ID byte count, +0x01 ID[15],
  +0x10 spare bytes, +0x12 page bytes, +0x14 pages per block, +0x16 block count, +0x18 sector bytes, +0x1A plane count,
  +0x1B wrap config, +0x1C RIU-read flag, **+0x1D clock code**, +0x1E / 1F / 20 U-Boot / BL0 / BL1 PBA, +0x21 hash
  PBAs, +0x27 / 28 read / write mode. IPL keeps it at **0xA0010000** (tag) / 0xA0010010 (SNI) and appends derived shift
  fields at +0x29 … +0x34; IPL_CUST reads them from there (the IPL → IPL_CUST hand-over on NAND).
- **PNI** = `PARTITION_INFO_t` (u32 checksum, spare / page / block-page / block counts, u16 partition count, unit size,
  records[62] of {u16 start block, u16 block count, u16 type, u16 backup block count}) at **0xA0010A00**, records from
  0xA0010A10; the types looked up are `UNFD_PART_IPL_CUST` = 0x01 and `UNFD_PART_UBOOT` = 0x06.
- **IPL `main`** (probe answered): SNI pointers 0xA00120EC / F0 = 0xA0010010; CIS search over blocks 0, 2, 4, 6, 8
  (page 0 via the FSP page-read + BDMA routine 0xA00029BC; `Find CIS in block`, `Found bad block%02x`,
  `Not find CIS in block0~8` → 10); SNI tag check and ID print (`Found SNI `, `NAND ID: …`, `No SPINAND INFO`); PNI read;
  the 512-byte CIS copied to 0xA0010000 (and the SNI to 0x23D00000); SPI clock from SNI +0x1D (the table of §5.3, prints
  `SPI …`); PNI check `Not found PNI`.
- **IPL loader**, s = 0x10 / 0x04: the same probe; partition read `0xA0002D10(dst, len, type 1)`: header page to
  0x23C0F000; if `IPLN`: len = size + 0x210, else 0xF000 and `NO Header`; then the whole partition to 0x23C00000 skipping
  bad blocks (`Bad block: `, `reload next block: 0x`, `, backup count: 0x`, `jump to next partition, block: 0x`,
  `u8BackupBlkCnt = `, `load ERR`, `Can't find PniPart`, `unable to find PBA for part type:`); page reads check the
  GET FEATURE 0xC0 status (0xA0000BAC: bit 5 → 0x80 = uncorrectable ECC, bit 2 → 8, extra-byte bit 3 → 0x10, else bit
  3; `[I]e3`). Then the same checks and jump as §4.5.
- **IPL_CUST**, s = 0x10 / 0x04: re-probe; CIS / GCIS / SNI re-read when the SRAM copy is missing (sets FSP quad /
  continuous read with command 0xD8 and feature writes 0xC1–0xC5 for some vendor ids: SNI ID byte 1 = 0xC8 (GigaDevice)
  with byte 2 in {0xD1 / 0xD9, 0x31, 0x32, 0x95, 0x51 / 0x55} or 0xC2 (Macronix) with {0x12, 0x26, 0x37}, 0xEF / 0x2C
  special-cased — `0xB0` feature register bit 0 set, i.e. quad enable [I]); U-Boot partition (type 6) via 0x23C01A48,
  header to 0x23C0F000, len = `ih_size` + 0x140 (no header: 0x80000, `NO Header`), **the backup flag 0x23C17400 chooses
  the backup copy**; `Load BL from SPINAND`; then §5.2 from step 5 with the CPU-copy semantics.
- **USB** (s = 0x1000): both stages only print `Load  from USB` / `Load IPL_CUST from USB` and continue with the image
  already in DRAM (put there by the boot ROM's USB loader) [R prints; I mechanism].

## 7. OTP / eFuse

Two register banks carry the fuses: **0x1018 = OTP** (the programming / read interface of the OTP array, used by the IPL
service) and **101F = OTP2** (eFuse words readable as plain registers: the feature / lock words and the trim words).

### 7.1 The IPL OTP service (entry 0xA0000010) [R]

ARM `push {r0–r12, lr}; blx 0xA00036E4; pop; mov pc, lr` — a callable service: `r0` = operation (1 = program,
2 = read), `r1` = id << 24 | byte offset, `r2` = data (program). Status word at 0xA0010000: 0 ok, 0xFF01 unknown id,
0xFF03 operation not allowed, 0xFF02 offset beyond the field (program).

Field table at 0xA00072F8, 20 × {u16 allowed-ops mask, u16 id, u16 first row, u16 length in bytes}; all masks 3:

| id | first row | length |
|---|---|---|
| 0 | 0xBF … | 256 B |
| 1 | 0xFF | 4 |
| 2 | 0x8D | 4 |
| 3 | 0x94 | 4 |
| 4 | 0x95 | 4 |
| 5 | 0xAB | 16 |
| 6 | 0xB0 | 16 |
| 7 | 0xB5 | 16 |
| 8 | 0xBA | 16 |
| 9–16 | 0x96–0x9D | 4 each |
| 0x18 | 0x39 | 8 |
| 0x19 | 0x8E | 8 |
| 0x1A | 0xA8 | 8 |

U-Boot's `otpctrl` help names command 0 = RSA public N key and 1 = RSA public E key, which matches ids 0 (256 B) and 1
(4 B). The other ids are not named in any source.

- **Program:** 1018.51 = 3, ‖2 µs‖, 1018.06 = 3, ‖2 µs‖, 1018.00 = 6, ‖1 ms‖, 1018.03 = (row + offset/4) & 0xFF,
  ‖1 ms‖, 1018.04 / 05 = data low / high, 1018.01 = 0x8000, 0x8001, 0x8000 (1 ms apart).
- **Read:** 0xA0010002 = length; for each row: 1018.02 = row, 1018.00 = 0x0B, pulse 1018.01 0x8000 / 0x8001 / 0x8000,
  ‖5 µs‖, word = 1018.33 << 16 | 1018.32 stored to 0xA0010004 …

**Caller.** Nothing in IPL calls it. U-Boot's `otpctrl` command (0x23E06AD8, `common/cmd_otpctrl.c`) does:
`otpctrl -w <cmd> <off> <data>` → `svc 1` with r0 = cmd << 24 | off, r1 = data; `otpctrl -r <cmd>` → `svc 2` with r0 =
cmd << 24; U-Boot's SVC vector (0x23E000C0) passes the SVC number and r0 / r1 to the SVC handler 0x23E010FC, which prints
`[OTP Control] operation = %lx, command = %lx, data = %lx` and calls the function pointer at 0xA0000010 — this service.
The results are read back from 0xA0010000 (u16 status, for reads u16 size at +2 and data at +4). The rest of the command
(argument checks, printing) is `uboot.md` §9.5. The call works only while the IPL image is still in IMI SRAM [I]. `otpctrl`
is not run on this camera.

### 7.2 OTP2 words (bank 101F) [R]

| word | read by | use |
|---|---|---|
| 101F.2E bits 1:0 | IPL (§3.3), mhal JPE / VENC / VIF (§7.3) | class: 0 = no disable mask; 1 / 2 = mask from 101F.27:26 ("rule-1"); 3 = mask from 101F.2D:2C ("rule-2") if 101F.2F bits 1:0 ∈ {1, 2}, else no mask |
| 101F.2F bits 1:0 | IPL, mhal | qualifier of class 3 |
| 101F.26, 101F.27 | IPL, mhal | rule-1 disable mask (low / high 16 bits) |
| 101F.2C, 101F.2D | IPL, mhal | rule-2 disable mask (low / high 16 bits) |
| 101F.71 bits 13:12 | IPL | lock: non-zero and ≠ 3 → IPL prints `halt!!!` and hangs |
| 101F.50–101F.5B | IPL | trim words with their valid bits — DRAM PHY, PM SAR / LDO, USB PHYs, SATA / PCIe-type PHY, HDMI-TX and other analog trims (table in §3.8) |

Disable-mask bits (IPL's table at 0xA0007398, §3.3, and the blocks §4.3 powers down):

| id | bit | IPL action when set | other readers |
|---|---|---|---|
| 0, 1, 5, 6, 7 | 0 | — (no bit) | — |
| 2 | 0x4 | none | mhal JPE and VENC throughput flag (§7.3) [I: same bit of the same words] |
| 3 | 0x8 | 1020.11 = 0xF0DF, 1020.10 = 7 | — |
| 4 | 0x10 | none | — |
| 8 | 0x100 | 113A.10 \|= 0x40, 131B.10–131B.15 = 0, 131B.23 \|= 4 | — |
| 9 | 0x200 | 1459.04 = 0, 145A.15 \|= 1 (HDMI-TX off [I from the bank]) | — |
| 10 | 0x400 | 143A.00 &= ~0x1000, 143C.50 \|= 1, 143B.0C \|= 0x100 | — |
| 11 | 0x800 | none | mhal VIF pipe / port capability (§7.3) [I: same bit of the same words] |

IPL's halt rules for unknown bits are in §3.3. The names of the blocks behind ids 3, 8, 10 and of the remaining bits are
in no source on hand.

### 7.3 Capability flags mhal derives from the same words [R from the mhal decompiles]

- **JPE and VENC throughput flag** (`jpe` probe, `venc_probe` 0x400b14d0): s = 101F.2E & 3; if s = 3: t = 101F.2F & 3,
  flag = 0 if t is 3 or 0, else bit 2 of 101F.2C; else if s ≠ 0: flag = bit 2 of 101F.26; else 0. With the flag set the
  encoders refuse settings above 167 116 800 pixel/s — `mhal-scl-jpe-cmdq.md` §4.3 (JPE), `mhal-vcodec.md` §2.3 (VENC).
- **VIF pipe / port capability** (`HalVif_CheckOTPPipeCapability`, `CheckOTPPortCapability`): s = 101F.2E & 3,
  a = 101F.26 & 0x800, t = 101F.2F & 3, b = 101F.2C & 0x800; full chip if s = 0, or s = 3 and (t = 0 or t = 3 or b = 0),
  or s ∈ {1, 2} and a = 0; a cut-down chip loses pipes 6–7 and port 1. VIF's input throughput cap
  (`HalVif_OTP_VENC_Fps_Capability`) reads 101F.26 bit 2 only, with no class s / t test — `mhal-vif.md` §7.2.

The values of these words on this chip are not known (§9). 4K30 frames reach the ISP on this camera, but that path is
realtime and never checks the VIF cap, so this does not settle it [I].

## 8. Hand-over to U-Boot — what U-Boot gets and what it relies on
<!-- ev:ipl.8 -->

| item | state at `blx 0x23E00000` | origin |
|---|---|---|
| CPU | core 0, ARM state, SVC mode, IRQ / FIQ masked as left by the ROM [I for the mode: IPL never changes it]; core 1 not started | [R code] |
| registers | **no arguments**: r0–r3 hold leftovers (emulation: r0 = 0xC51078, r4 = entry, r5 = load) | [R] |
| MMU / caches | MMU off, D-cache off (cleaned), I-cache on; page table at 0x20004000 abandoned; VFP / NEON enabled (CPACR, FPEXC) | [R] |
| DRAM | 256 MiB at 0x20000000 initialised and BIST-tested; MIU arbiter in the IPL state (§3.8); 1012.69 = 256 MB code | [R] |
| U-Boot image | uImage header at 0x23F00000, compressed data at 0x23F00040, **unpacked U-Boot at 0x23E00000** (0xAF8C8 B), entry = load = `ih_ep` | [R] |
| clocks | CPU PLL 800 MHz [I post_div], DDR PLL 0x1A67D7, MIUPLL 264 MHz, IPU PLL 800 MHz, SPI 54 MHz, UART0 115 200 Bd 8N1 on the 101E.53 = 0x3102 pads | [R] |
| GIC | v3 distributor + both redistributors: all interrupts Group 1, distributor enabled, CPU0 awake | [R] |
| SRAM | boot-time record at 0xA000F000 (read by U-Boot `btime`, `prof.c`); mailbox 1004.00 = 0xB012 (last IPL postcode); IPL image still at 0xA0000000 (OTP service, §7.1; header byte 0xA000000B) | [R] |
| stack | sp ≈ 0x237FFEF8 (IPL_CUST stack, below 0x23800000) — U-Boot sets its own | [R emulation] |
| security | no authentication on this board; 0012.04 bit 2 not set | [R] |

U-Boot's side [R; S U-Boot mercury6 sources]: it is linked at 0x23E00000 and relocates itself; it
re-initialises the CPU / caches itself and depends on no IPL register except the DRAM / MIU state and, for `btime` only,
the 0xA000F000 record (and, for `otpctrl` only, the IPL OTP service in SRAM). `REG_ADDR_STATUS` (101E.21, "sync with
IPL") is read by the mercury6 board code for the AES re-encryption feature; neither IPL nor IPL_CUST writes 101E.21 [R: no
such store in the trace or code], so it reads its reset value.

What IPL_CUST takes from IPL [R]: entry at 0x23C00000 in ARM state (it runs with the MMU on — IPL's identity map — and
switches MMU / D-cache off itself, so it would equally run with the MMU off [I]); IPL's header byte 0xA000000B
(authentication switch); the boot-source status 0038.70; on SPI-NAND the SNI / PNI copies at 0xA0010000 / 0xA0010A00.
It programs no clock, pad or DRAM register (§5.3).

## 9. What is not known

Static analysis cannot settle these; each is a value on the running chip (read-only RIU reads, except the last row, which
needs the U-Boot prompt on the serial console) [R for what the code does with them]:

| value | what it decides |
|---|---|
| 101F.50–101F.5B (OTP2 trim words) | which trim steps of §3.8 run on this chip — **101F.55 bits 7, 11, 15 change DRAM PHY values (1010.58, DQ deskew 1010.2C–2F)**; the rest trim USB / SATA / HDMI / SAR analog |
| 101F.26, .27, .2C–.2F, .71 | the feature-disable mask and the lock word (§3.3, §4.3), and with them the JPE / VENC throughput flag and the VIF capability (§7.3) |
| 001E.00 (chip id; also the sysfs `CHIP_ID`, `/sys/devices/virtual/mstar/msys/CHIP_ID`) | 0xF4 or not; the BDMA load path does not decide it (§4.5) |
| 001E.01 | the GIC byte; = 1 per the log (`GIC Init`), the rest of the register not known |
| 0038.70 | the boot-source code the ROM leaves (0x20 or 0x02 for NOR) |
| 1420.00 bits 3–4 | whether IPL's UPLL0 power-up step ran (§3.4) |
| 1032.19 | CPU PLL post-divider ⇒ the actual CPU clock (800 MHz if 1) |
| 1010.xx, 1012.xx, 1013.xx after boot | which of the §3.6 values the kernel changed (the arbiter part: `kernel-platform.md` §12.4) |
| U-Boot `btime` at the prompt (SRAM 0xA000F000 …) | the real time stamps of every IPL / IPL_CUST phase (§2.3) |

Also not known: the names and meaning of most DDR PHY (1010) and controller (1012 / 1013) fields, the CLKGEN fields, and
the banks 0x1030, 0x1039, 0x1131, 0x002A, 0x100E — no M6 register reference is on hand; the boot ROM itself (not in the
flash; neither a ROM image nor a readout is on hand, and whether the ROM stays readable at 0x0 after boot — IPL maps VA 0
as normal memory, §4.2 — is not known), so the SP value, the exact CPU mode and what the ROM checks in IPL's
own header (`auth` / `cid` / the upper CRC word 0x0003) stay unknown; which of 0x20 / 0x02 the ROM sets in 0038.70; the
meaning of the header word at +0x0E.
