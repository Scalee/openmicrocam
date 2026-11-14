# uboot — vendor U-Boot 2015.01 "M6"
<!-- ev:uboot -->

The third boot stage on the NOR flash: SigmaStar's SDK U-Boot 2015.01 for Mercury6, built by the ODM, loaded by IPL_CUST
from the MXPT record `UBOOT` and run from DRAM at 0x23E00000. On every boot it brings up the console, the SPI NOR, the SD
host and the (unconnected) Ethernet MAC, runs the SD upgrade command `sdstar`, checks the USB-upgrade flag, shows the HDMI
boot logo from the MISC partition (`bootlogo`) and boots the kernel from the KERNEL partition with `bootm`. Besides that it
carries two upgrade paths (SD script, USB mass-storage download), a PQ loader for the display, a full copy of the DISP and
HDMI-TX drivers, and a set of commands nothing on this camera runs (network, crypto, sensor pad set-up, SD IP tests, INI
file tools, EEPROM, OTP). This document owns the U-Boot environment variables, the `PQ.bin` format and U-Boot's copies of
the display timings. Elsewhere: the partition map and the vendor file inventory — `firmware-image.md` §1; the IPL → U-Boot
hand-over and the OTP service — `ipl.md` §8, §7.1; the MISC firmwarefs on-flash format — `misc-firmwarefs.md` §2; the Linux
writers of the environment — `rootfs-customer.md` §2.7; the kernel PQ hand-over — `mhal-disp.md` §13.1; RIU addressing (bank →
CPU address) — `kernel-platform.md` §3.1.

**Tags.** [R] read from the binary, a decompile, the disassembly, the NOR dump, a boot log or a source file; [S] stated by
a source (help text, header comment, another document); [I] inference.

"mhal" = the kernel module `mhal.ko` of this firmware.

**Register notation.** `bank.reg` or `bank:reg` = one 16-bit RIU register, reg n at byte n·4 of the bank; `[b:a]` = bits.
A byte register `r` of bank `b` (the EMAC driver's and PQ writer's form) is at CPU address
`0x1F000000 + b·0x200 + (r & ~1)·2 + (r & 1)`.

**Reference trees.** "oi" = OpenIPC `u-boot-sigmastar` @ `bf77aff` (2026-07-23); "ssd" = `fifteenhex/ssd_lcd` `boot/`
(SigmaStar SDK U-Boot of the SSD20x generation). The SD image v1.1.2 `split-hc.bin` of the RunCam Split-H (M6, NOR) is a
vendor example of the SD upgrade script.

## 1. The image

### 1.1 Identity [R]

| item | value |
|---|---|
| version string | `U-Boot 2015.01 (Jun 03 2025 - 11:57:14)`, `Version: M6##g#######` (printed by `checkboard` in `chip.c`) |
| toolchain | `arm-linux-gnueabihf-sigmastar-9.1.0-gcc (GCC) 9.1.0`, `GNU ld (Linaro_Binutils-2020.07.23) 2.33` |
| code | ARM (not Thumb; the libgcc division helpers are Thumb-2), linked at 0x23E00000, 719 048 B unpacked (0xAF8C8 B); command table at 0x23EA0D48, 72 entries × 28 B |
| relocation table | 7266 `R_ARM_RELATIVE` words from 0x23EA15B8 to the end of the image (every absolute pointer) |
| flash | MXPT record `UBOOT`: 0x30000, size 0x3F000 (252 KiB); 247 180 B used (the image ends at 0x6C58C) → ≈10.6 KB free in the record |
| uImage header | legacy uImage; load 0x23F00000, entry 0x23E00000, os 17 (U-Boot), compression byte 3 ("lzma"), payload **XZ**; name `MVX4##M6##g#######CM_UBT1501#XVM` |
| packing | the same steps as oi `create_img.sh`: `xz -z`, then `mkimage -A arm -O u-boot -C lzma -a <load> -e <entry> -n <MVX string>` [R oi script; I that the vendor used it] |
| default environment (compiled in) | only `bootdelay=0`, `baudrate=115200` — no `bootcmd`, no `bootargs` |
| caches | D-cache stays off (`WARNING: Caches not enabled` in the log) |
| debug word | 0x23E9BB14 = 1: bit 0 (error prints of `cmd_bootlogo.c`) on, bit 1 (info prints) off — only error lines reach the log |

### 1.2 Load by IPL_CUST [R]

IPL_CUST finds the MXPT record `UBOOT`, checks the uImage magic and the **data** CRC32 only (`ih_hcrc` is never verified),
copies the image by BDMA in 64 KiB chunks to `ih_load` + 0x40 (0x23F00040) pipelined with the XZ decoder that writes to
`ih_ep` (0x23E00000), and jumps with `blx 0x23E00000` after turning its MMU and D-cache off. `ih_comp` 3 ("lzma" in the
header) is treated as XZ, 9 as "MZ", 0 as uncompressed. "Reload backup file" works only on SPI-NAND; on NOR a failed check
re-reads the same `UBOOT` record and the second failure halts with `u-boot corrupted, halt`. The strings `KEYN_ADDRESS` and
` -Authenticate image ` belong to an image authentication that does not run on this board. The full loader and the CPU,
memory and SRAM state U-Boot starts in: `ipl.md` §5.2, §8.

Consequences for the image [I]: the BDMA staging buffer at 0x23F00040 lies 1 MiB above the entry, so the unpacked image
(0x23E00000 + 0xAF8C8) must stay below 0x23F00000 for the decoder not to overwrite its own input; the image is booted only
in the uImage + XZ form with a valid data CRC.

### 1.3 Where the code comes from

Every one of the 72 commands and every `__func__` name kept in the binary exists in oi or in ssd, except that `srcfg`
exists there by name only (§9.4) [R]. Closest oi configuration: `mercury6_SZDEMO_BGA_defconfig` plus EMAC and `ufu` [I from
the command and feature set]. Strings: 2510 printable strings of ≥ 10 characters; 327 have no match in oi, 485 none in
ssd, 120 none in either — most of those 120 are artefacts of the matching (format strings split by escapes, `__FILE__`,
`jerror.h` messages spread over lines, data bytes); the real ones are the parts below [R].

| part | source | status in the vendor binary |
|---|---|---|
| core 2015.01, `cmd_mem`/`fat`/`mmc`/`sf`/`nvedit`/`net`/`i2c`/`eeprom`, `env_sf` (environment located through MXPT) | oi | same (§1.4) |
| `cmd_mstar.c` (`sdstar`, `dstar`, `estar`, `mstar`, `mxp`, `uart`, `debug`, `secauth`) | oi (ssd has an older copy) | same; `sdstar` patched (§4.1) |
| `main.c` (`sdstar` before autoboot), `autoboot.c` (`ota_upgrade_status` → `ufu`, `sdautoupgrade`) | oi (`CONFIG_SSTAR_AUTORUN_DSTAR`, `CONFIG_CMD_SSTAR_UFU`, `CONFIG_MS_SDMMC`) | same; the vendor also runs the `sdautoupgrade` block when `bootcmd` is empty |
| `ufu`: `cmd_usb_firmware_update.c`, `f_firmware_update.c`, `g_sstar_dnl.c` | oi | same; device descriptor VID 0x1B20 PID 0x0300 in the binary |
| `fs/firmwarefs` (MISC) | oi | core line for line the same as oi; the glue `firmwarefs.c` is an older revision (configuration struct without the `bbt` member, no NAND bad-block map); caches U-Boot 2 KiB (cache and file cache), pool 4 — Linux `fwfs` 32 KiB, pool 2; on-flash geometry the same as Linux: block 64 KiB, subblock 32 KiB, read/prog 2048, cache pool 4, 500 cycles, lookahead 8 |
| `cmd_bootlogo.c` | oi for the `LOGO`-partition path and `ini_*` | the MISC path is a vendor rewrite built against the old ssd DISP / HDMI-TX API (§5) |
| DISP driver + mercury6 HAL (`HalDispMop…`, `MHAL_DISP_*`) | oi `disp/hal/mercury6` | body logic = the oi mercury6 revision, debug-message layer = mhal's revision, older 0x6C-byte panel struct (§7) |
| HDMI-TX `MApi_/MDrv_HDMITx_*`, HDCP 1.4, M6 HAL `MHal_HDMITx_*` / `hal_HDMITx_*` | ssd `hdmitx/drv/hdmitx/src/api`, `…/drv`; the M6 HAL has no published source | drv / api / util / HDCP core and the M6 HAL = ssd bodies; the `*_if.c` wrappers = mhal's revision; the M6 HAL is an older revision of mhal's M6 HAL, itself the ssd i2m HAL ported (§8) |
| `do_reset` | oi `arch/arm/lib/reset.c` | ODM patch (§4.2) |
| `cmd_srcfg.c` | the 10-argument M6 variant and its driver are in neither tree | vendor (§9.4) |
| board / arch: `mercury6/board.c`, `chip.c`, `timer.c` | oi | same; `init_sequence_f` (34 entries) and `init_sequence_r` (26 entries) hold only stock U-Boot / SigmaStar steps — no board-specific init (no GPIO, pad or power set-up) [R] |

Feature set beyond `mercury6_defconfig` [R commands and strings → configuration; I the option names]: `MS_SDMMC` (`mmc`,
`fat` with `fatwrite`/`fatrm`, `mssdmmc`, `sdstar`), `SSTAR_AUTORUN_DSTAR`, `SSTAR_HDMITX`, an upgrade-UI variant
(`bootlogo 1` + `bootframebuffer bar`, no `dcache on/off`), `CMD_INI`; plus what `mercury6_defconfig` already has
(`CMD_SSTAR_UFU`, EMAC + net commands, `FS_FIRMWAREFS`, `SSTAR_DISP`, `SSTAR_JPD`, AES / `sigauth`, `otpctrl`). Options read
from the code: `CONFIG_MS_AESDMA` on, `CONFIG_SRAM_DUMMY_ACCESS_RSA` on, `CONFIG_MERCURY6P` off (§9.6); `CONFIG_SSTAR_RGN`
variant of the JPEG decoder (§5.2); `CONFIG_SSTAR_PNL` off (§5.5); `CONFIG_MS_DISPLAY` and `CONFIG_MS_SHOW_LOGO` off (§4.5);
EMAC options in §9.1.

The oi tree does not build for mercury6 as it is: GPIO driver / HAL API mismatch, `CONFIG_ENV_ROOTADDR` missing in
`cmd_sf.c`, DISP HAL headers out of step (`E_HAL_DISP_CLK_NUM`, `HAL_DISP_FPLL_CNT`, `E_HAL_DISP_IRQ_TYPE_DMA` undeclared),
no `hdmitx/hal/mercury6` (`regHDMITx.h`); oi's own `build.sh` builds no mercury6 target [R: build trial
with host gcc 15; the API mismatches do not depend on the compiler]. With `make -k` and the vendor feature set the host gcc
15 build gives 233 objects / 2221 functions; the DISP / PNL / GOP, HDMI-TX and GPIO HALs and firmwarefs (C99 for-loop
errors) give no object [R]. Whether the tree builds with the SDK gcc 9.1 (not on hand) is not known.

### 1.4 Body comparison with the sources [R]
<!-- ev:uboot.1.4 -->

Function split: a disassembler's automatic analysis finds 1878 functions (counts 1880) because functions reached only
through pointer tables (DISP / HDMI-TX query callbacks, e.g. `_HalDispIfSetPqSet` 0x23E25FDC inside 0x23E25B20…0x23E266C8)
were merged into their neighbours; our own split of the image gives **2344 functions**, seeded on
the relocation table, BL targets and push prologues. 737 functions carry at least one literal string, 1607 none. Ghidra's
decompile also drops arguments of variadic calls (`mstar`'s `tftp`, §4.5): `sprintf`/`printf` arguments are read in the
disassembly.

Pairing against the compiled oi objects on a shared distinctive string, then comparing string sets, resolved call names and
the constant multiset: **312 of the 737 string-bearing functions paired — 287 SAME (identical string set), 10
oi-superset (same body, some oi prints compiled out), 15 DIFFERS**. Of the 15: `do_sdstar` (ODM patch, §4.1), `do_sensor_cfg`
(`srcfg`, §9.4) and `MDrv_AESDMA_Run` (one vendor debug line, §9.6) are real; the rest are pairing artefacts — the strings
are in the oi source (`MDrv_AESDMA_Run`/`_SHA_Run`/`_RSA_Run` alignment / timeout strings, `mmc_berase` erase-group warning,
`HAL_HWI2C_XferWrite` read-path strings, `HAL_SERFLASH_WriteProtect` → `HAL_FSP_WriteProtect`, `set_default_env`
`bootdelay=0`, `fat_format_device`, `part_dos`, `IPV_*`). So **no function body differs from the oi / SigmaStar source
beyond the ODM patches (`sdstar`, `reset`, `autoboot.c`, `main.c`), the vendor `srcfg`, the rewritten `bootlogo` MISC path
and one AESDMA debug print**. Body-confirmed this way: `bootm`/image, `env_*`/`env_sf`/`saveenv`, `cmd_i2c`/`mstar_i2c`,
`cmd_mmc`/`mmc`/`mmc_write`, `cmd_sf`/SPI-NOR, `cmd_mem`, MXP (`part_mxp`, `env_sf`), AESDMA / SHA / RSA, `zlib`/`inflate`,
`eth`, SDMMC.

Unpaired string-bearing functions (425): 262 DISP / HDMI-TX / PNL / GOP / OSD / HDCP functions with no oi object (compared
against ssd and mhal in §7, §8); 12 firmwarefs (`fwfs.c`, `fwfs_tftl.c`); 8 `bootlogo` / `ini_*` / `srcfg` functions (read by
hand, §5, §9); 143 other core functions — 65 share strings with a built oi object but below the anchor threshold (short or
generic strings such as `%08lx`, `.constprop` clones), 78 have no oi string at all (banners `Net:`, `MMC:`, `U-Boot 2015.01`,
`bootdelay=0`, generic hexdump formats, `ufu` whose oi object was not built, the `bootlogo` JPEG helpers) — none a body change.

Per area, compared body against body:

| area | result | where |
|---|---|---|
| `cmd_bootlogo.c` (`do_bootlogo`, `_MiscPartiton`, `_BootLoadFile`, `_BootDispCtrl`, `_BootLogoDispPnlInit`, timing-id helpers, `_LogoPartiton`, `_BootDbTable`, `_GetLogoReservedAddr` (inlined), `ini_*`) | MISC path: vendor rewrite against the old ssd DISP / HDMI-TX enums and the u8-MHz panel struct, with a Hz DCLK table; `LOGO`-partition path and `ini_*` = oi | §5, §9.2 |
| DISP `DrvDispIfInitPanelConfig` | ssd revision (fps from a u8-MHz DCLK, period fields) | §5.4 |
| DISP PQ (`MDrv_PQ_Init`, `HalDispHpqLoadBin`, `_HalDispIfSetPqSet`, source-type table and matcher, `MDrv_PQ_LoadSettings`, `DumpTable`, CRD SRAM writers, bin parser and getters) | bin parser = oi; PQ glue = mhal's minus flag 4; no Y/C filter SRAM writer | §6 |
| DISP HAL | oi mercury6 logic + mhal coloured debug, old panel struct | §7 |
| HDMI-TX | ssd core + mhal-revision wrappers; M6 HAL older than mhal's | §8 |
| `fs/fat/fat_write.c` | newer SigmaStar revision without `fat_dirty = 1` in `set_fatent_value` | §10 |
| `fs/firmwarefs` | core = oi, glue older | §1.3 |
| `cmd_net.c`, `cmd_emac.c`, `cmd_mstar.c` (`mstar`, `estar`, `dstar`, `aes`, `secauth`), `cmd_sigauth.c`, `cmd_otpctrl.c`, `cmd_eeprom.c`, `cmd_sdmmc_ipv.c`, EMAC driver, net stack, AESDMA / SHA / RSA HAL, IPV tests | = oi (one AESDMA debug print) | §9 |
| `cmd_srcfg.c` | M6 variant, driver in neither tree | §9.4 |
| `cmd_mstar.c` `sdstar`, `arch/arm/lib/reset.c` `do_reset`, `autoboot.c`, `main.c` | ODM patches | §2.2, §4 |

The 1607 string-less functions (memcpy / printf internals, CRC, LZMA / XZ, hashtable, small getters) and the 311 string-less
DISP register functions are not compared one by one: a stripped image gives them no name and no string anchor, and an
instruction-stream comparison would need the vendor symbol map or the SDK gcc 9.1 (not on hand) [R]. They are reached only
from the paired core, so the SAME result across the string-bearing layer stands for them [I].

## 2. Boot flow

### 2.1 Steps as they run [R boot log + code]

| step | what happens | time (1-s log) |
|---|---|---|
| IPL → IPL_CUST | DRAM init 256 MB, MXPT, XZ unpack of U-Boot (`ipl.md` §3, §5.2) | ≈0 s |
| `init_sequence_f` / `_r` | timer, `Version:`, `I2C: ready`, `DRAM:`, MMC `MStar SD/MMC: 0`, SPI NOR probe (`0x1003, 0x5E, 0x40, 0x18`, 16 MiB), MXPT + environment (`env_offset=0x6F000 env_size=0x1000`), console | < 1 s |
| `Net:` | `eth_initialize` → `board_eth_init` → EMAC bring-up with PHY auto-negotiation, which fails (§9.1) | **≈4 s** (20:49:22 → 26) |
| `sdstar` | always, from `main_loop`: prints `20000ms`, `fatload mmc 0` of `$SdUpgradeImage` → no card: `Card Detect Fail! ** Bad device mmc 0 **` + usage text (§4.1) | < 1 s |
| autoboot hook | `ota_upgrade_status == "1"` → `ufu`; `sdautoupgrade` handling; then `bootcmd` (§2.2) | 0 s |
| `bootlogo 0 0 0 0` | HDMI logo from MISC (§5); prints `INI_GET_STR: Get LOGO:m_sPQFile0 error`, `can't get LOGO:m_sPQFile0`, then 4 × `raise: Signal # 8 caught` (§5.4) | ≈1 s |
| `sf probe 0; sf read 0x22000000 ${sf_kernel_start} ${sf_kernel_size}` | whole KERNEL partition, 2 162 688 B, `cost 424 ms` | 0.4 s |
| `bootm 0x22000000` | header says lzma, payload XZ (`[XZ] reserved 0x21000000 length=0x1000000`), 0x451000 B unpacked; ATAGs at 0x20000000 | < 1 s |

U-Boot takes ≈6 s of the boot, ≈4 s of it the PHY auto-negotiation on a camera with no Ethernet connector [R code + log
timestamps]. `board_eth_init` calls the EMAC init only when `getenv_yesno("autoestart") != 0`; unset reads as −1, i.e. yes
[R oi `mdrv_emac.c`, the same test at 0x23E1C918]; with `autoestart=n` in the environment this binary skips the bring-up
[R code; I the ≈4 s saving, measured only from the 1-s log].

### 2.2 `main_loop` and the autoboot hook [R]
<!-- ev:uboot.2.2 -->

`main_loop` (0x23E013E0) runs `run_command("sdstar", 0)` **always**, then parses `bootdelay`, then the autoboot hook
(0x23E017F4):
- no key abort and `ota_upgrade_status == "1"` → `run_command("ufu")` (§4.4);
- `sdautoupgrade == "1"` → `setenv sdautoupgrade 2; saveenv; sdstar`; `== "2"` → `setenv sdautoupgrade 1; saveenv`
  (`sdautoupgrade` is not set in this environment); the vendor runs this block also when `bootcmd` is empty;
- then `bootcmd`.
Both tests are `strcmp(getenv(…), "1")` with no NULL check, so `ota_upgrade_status` must exist in the environment; an
unset `sdautoupgrade` goes through the same unchecked `strcmp` [R]. The value `2` of `ota_upgrade_status` is compared
nowhere (the name string has one reference) [R].

Stopping autoboot: `bootdelay=0`, but Enter sent every 50 ms from power-up lands in the `sdstar` step and drops to the
`SigmaStar #` prompt (4 of 4 power-ups); Space does not [R]. Console: ttyS0 (UART0) at 115200 Bd.

### 2.3 Kernel hand-off [R]
<!-- ev:uboot.2.3 -->

`bootcmd` reads the whole KERNEL partition from NOR to 0x22000000 and runs `bootm` on it: legacy uImage, header type lzma,
payload XZ, unpacked by U-Boot's XZ decoder with a reserved buffer at 0x21000000 (0x1000000 B). The kernel gets ATAGs at
0x20000000 (`CONFIG_CMDLINE_TAG`, `SETUP_MEMORY_TAGS`, `INITRD_TAG`), no FDT: the DTB is appended to the kernel. The command
line is env `bootargs` (ATAG_CMDLINE); it carries the MI heap, CMA and framebuffer reservations (§3.2). U-Boot passes **no
`mtdparts`**; the kernel builds its six MTD partitions itself from MXPT (`firmware-image.md` §1.1).

RAM in U-Boot: U-Boot lives at 0x23E00000; an `sf read` of 16 MB into 0x23000000 overwrote it and hung the board; 0x21000000
and 0x22000000 are free for loads [R / I].

### 2.4 MXPT as U-Boot uses it [R]

U-Boot reads the partition table MXPT (flash 0x20000, entries 0x88 B; full map in `firmware-image.md` §1.1) through `mxp`
/ `part_mxp`: record `UBOOT_ENV` (0x6F000, 0x1000) locates the environment, `KERNEL` (0x70000, 0x210000) is read by
`bootcmd` through `sf_kernel_start/size`, `MISC` (0xE90000, 0xA0000) is mounted by `bootlogo` and `ini_*`, `LOGO` is looked up
by the `bootlogo` fallback (§5.5) and `KEY_CUST` would be the crypto key area (§9.6). This flash has **no `LOGO` and no
`KEY_CUST` record** (the RunCam image has `KEY_CUST`). The `mxp` command (`r.info <NAME>` sets `sf_part_start`,
`sf_part_size`, `cpu_part_start` = start + 0x14000000, and for `KERNEL` also `sf_kernel_start/size`; `t.init` / `t.update` /
`t.load` rewrite the table) is what upgrade scripts use (§4.1). `cpu_part_start` is the partition's address in the
memory-mapped NOR window at 0x14000000 [I: the `LOGO` path reads the flash at `start | 0x14000000`, §5.5].

### 2.5 Boot-time stamps [R]

`btime` prints the boot-time record at 0xA000F000 that IPL and IPL_CUST start (format: `ipl.md` §2.3). U-Boot adds stamps
through `bootstage_mark(mark, name)` (0x23e008d8) / `bootstage_mark_name`: 0xC0 `do_display++` / `do_display--` around `bootlogo` (then
`bootstage_mark_end(0xC0)` (0x23e013dc)), 0xA3 `tftp_start` / 0xA4 `tftp_done` around `tftpboot`.

## 3. Environment

### 3.1 Storage [R]

One copy, 4 KiB, CRC32 over the data, no redundant copy and no flags byte, at the MXPT record `UBOOT_ENV` (flash 0x6F000,
0x1000) found at run time (`env_sf` through MXPT); the CRC was valid in the dump of 2026-10-16. U-Boot's `saveenv` erases
only the 4 KiB sector (`CONFIG_ENV_SECT_SIZE 0x1000`). The compiled-in default environment holds only `bootdelay=0` and
`baudrate=115200`, so with a bad CRC U-Boot comes up with no `bootcmd` and stays at the prompt [I from the default
environment].

Linux side (writers: `rootfs-customer.md` §2.7): `/etc/fw_env.config` = `/dev/mtd0 0x6F000 0x1000 0x10000`, and mtd0 (`BOOT`)
has an erase size of 0x10000 [R dmesg]. `fw_setenv` therefore erases and rewrites the whole 64 KiB block 0x60000–0x6FFFF
[I: stock u-boot-tools `fw_env` behaviour when the environment is smaller than the erase block]; the U-Boot image occupies
0x30000–0x6C58C, so its last 50 572 B are in that block. demo.sh runs `fw_setenv` on every boot, prog_uvc and
sd_version_recognition on their events. A power cut inside that window leaves IPL_CUST with a bad U-Boot data CRC (`u-boot
corrupted, halt`) [I].

### 3.2 Variables [R environment dump + code]

| variable | value in the dump | read by | written by | if absent |
|---|---|---|---|---|
| `bootcmd` | `bootlogo 0 0 0 0; sf probe 0;sf read 0x22000000 ${sf_kernel_start} ${sf_kernel_size}; bootm 0x22000000` | autoboot | the upgrade script (§4.1) | not in the default environment: nothing boots [I] |
| `bootargs` | `console=ttyS0,115200 root=/dev/mtdblock2 rootfstype=squashfs ro init=/linuxrc LX_MEM=0xffe0000 mma_heap=mma_heap_name0,miu=0,sz=0x9000000 mma_memblock_remove=1 cma=2M@0x23800000 mmap_reserved=fb,miu=0,sz=0x800000,max_start_off=0xF700000,max_end_off=0xFF00000` | `bootm` (ATAG), `bootlogo` (`mmap_reserved=fb`, §5.2) | the upgrade script | not in the default environment; `bootlogo` then gets framebuffer 0 / size 0 |
| `sf_kernel_start`, `sf_kernel_size` | 70000, 210000 | `bootcmd` | `mxp r.info KERNEL` | — |
| `bootdelay` | 0 | autoboot | default environment; `mstar`/`estar`/`dstar` (§4.5) | — |
| `baudrate`, `stdin`, `stdout`, `stderr` | 115200, serial | console | default | — |
| `SdUpgradeImage` | `H161X.bin` | `sdstar` | demo.sh (every boot), `sdstar` (when unset: `SigmastarUpgradeSD.bin`), the `reset` patch (`SigmastarUpgrade.bin`) | set by `sdstar` itself |
| `ota_upgrade_status` | 0 | autoboot (`"1"` → `ufu`) | prog_uvc (1), sd_version_recognition (2, never read) | unchecked `strcmp` on NULL (§2.2) |
| `sdautoupgrade` | unset | autoboot (the same unchecked `strcmp`) | nobody on this camera | — |
| `autoestart` | unset (= yes) | `board_eth_init` | nobody | EMAC bring-up runs (§2.1) |
| `sf_part_start`, `sf_part_size`, `cpu_part_start`, `filesize` | e90000, a0000, 14e90000, 60000 | nobody at boot | `mxp r.info`, `fatload` | left over from the last upgrade-script run: `sf_part_*` = MISC [I] |
| `ethact`, `ethaddr` | `sstar_emac`, the unit's MAC | EMAC driver (§9.1) | EMAC registration | built-in MAC `00:00:23:34:45:66` |
| `ethaddr1`, `PHY_ID` | unset | EMAC driver (second-MAC address, PHY address) | — | PHY address 0 |
| `logo_rot` | unset | `bootlogo` (rotation) | — | no rotation |
| `dispout` | unset | `LOGO`-partition path (§5.5) | that path itself | — |
| `dbgLevel` | unset | `dbg` | — | — |
| `ipaddr`, `serverip` | unset | net commands, `mstar` | `mstar` (defaults `192.168.0.28`, `192.168.0.26`) | — |
| `ethprime`, `ethrotate`, `netretry`, `autoload`, `tftpblocksize`, `tftptimeout`, `tftpsrcp`, `tftpdstp`, `eth%daddr`, `rootfs_size` | unset | net stack (§9.1), `sigauth` (§9.6) | — | stock defaults |

`sf_kernel_*`, `bootcmd`, `bootargs` and the left-over `sf_part_*` / `filesize` are what an upgrade script of the RunCam form
leaves behind (§4.1) [I].

## 4. Upgrade paths

### 4.1 `sdstar` — SD script, every boot [R decompile 0x23E0F26C against oi `cmd_mstar.c`]
<!-- ev:uboot.4.1 -->

The oi code with these ODM changes: prints `20000ms`; when `SdUpgradeImage` is unset it sets `SigmastarUpgradeSD.bin`
**and runs `saveenv`** (oi has the `saveenv` commented out); no `-i <dev:part>` option and no `MMC_Dev` variable; the
progress UI runs without `dcache on/off`; the binary also carries the script command strings `sf erase`, `sf write` and
`Header check fail` that oi's `do_sdstar` lacks [R].

Flow: `fatload mmc 0 <buf> $SdUpgradeImage 0x4000 0x0` (the first 16 KiB of the file = the script); no card → `Card Detect
Fail! ** Bad device mmc 0 **`, usage text, normal boot (every UART log). With a file: count the lines, `bootlogo 1 0 0 0`
(`upgrade.jpg`), `bootframebuffer bar 0`, then per line `run_command`, print `CMD LINE: %s`, `bootframebuffer bar
<n·100/total>`; at the end `bootframebuffer bar 100`. The card must be present at power-up: a card inserted later is not
seen without a power cycle, and `mmc rescan` after a hot insert timed out [R].

Script format, from the vendor example (RunCam Split-H M6 NOR) [R]: plain U-Boot commands,
ending with the line `% <- this is end of script symbol`; per partition `mxp r.info <NAME>`, `sf probe 0`, `sf erase
${sf_part_start} ${sf_part_size}`, `fatload mmc 0 0x21000000 $(SdUpgradeImage) <len> <file offset>`, `sf write`; MXPT itself
through `mxp t.init` / `t.update` / `t.load`; finally `setenv bootargs …`, `setenv bootcmd …`, `saveenv`, `run bootcmd`.
The camera's own upgrade file `H161X.bin` is not on hand; its content and last line are not known.

Tried on the camera with a script of our own (2026-11-13) [R]: a file of 16 KiB + 24 bytes (the script padded with 0xFF, the
packer's tail) runs line by line with no header complaint — the tail check, if any, is not exercised by it [I]. A line is
cut after its `$` expansion: `setenv bootargs ${bootargs} …` kept 495 bytes of value after the 16-byte command, 511 in all
(`CONFIG_SYS_CBSIZE` 512 [I]), with no warning on the console. `setenv` there leaves the value in RAM only; a kernel
command line with `init=/bin/sh -- -c "…"` reaches the kernel whole (double quotes kept by the simple parser, split by
the kernel) and the quoted text runs as PID 1. `bootcmd`'s `bootlogo` takes the framebuffer from `mmap_reserved` in
`bootargs`: without it, a prefetch abort and a reset (§4.2).

File layout as the SDK's packer `make_sd_upgrade_sigmastar.sh` builds it [S: the script of the industio
PurPle-Pi-R1 tree and the SDK's `project/image/ota.mk`]: the script (`upgrade_script.txt`) padded with 0xFF to 0x4000
bytes, then the partition images concatenated, each 4 KiB-aligned, with a `12345678` magic and the file's first 16 bytes
copied to its tail; the script's `fatload … <len> <file offset>` lines address those images [I]. Whether the camera's `sdstar`
checks that tail (its `Header check fail` string, above) is not known [I].

### 4.2 The ODM patch in `reset` [R decompile 0x23E011E4]

```
do_reset: puts("XU_resetting ...\n"); run_command("setenv SdUpgradeImage SigmastarUpgrade.bin"); run_command("saveenv");
          then stock: PM_POR bit 0, WDT reg 2 bit 0, udelay 50 ms, reset_cpu
```
Any U-Boot `reset` (a script line, `ufu` after a download, the prompt, a crash — it saves whatever the RAM environment
holds then, a temporary `bootargs` too [R 2026-11-13]) renames the expected SD file, so the next boot's
`sdstar` finds nothing; Linux demo.sh puts `H161X.bin` back on every boot. A card holding `H161X.bin` therefore re-runs
the script on every second boot if the script ends in `reset`, on every boot if it ends in `run bootcmd` [I — the
`H161X.bin` ending is not known]. sd_version_recognition on the Linux side only adds a reboot when the versions differ
(`rootfs-customer.md` §4.3).

### 4.3 `sdautoupgrade` [R]

The autoboot-hook toggle of §2.2: `1` → set 2, save, run `sdstar` a second time; `2` → set 1, save. Nothing on this camera
sets the variable — unused.

### 4.4 `ufu` — USB download [R decompile 0x23E06BEC + oi source]

Entered when `ota_upgrade_status == "1"` (prog_uvc sets it from its UVC XU "OTA" request in `_UVC_Events_Process_Data` and
reboots — `prog_uvc.md` §4.7).
U-Boot becomes a USB mass-storage gadget, VID 0x1B20 PID 0x0300, on the device port (`usb_dnl_ufu`); it waits 60 s for the
cable (`Auto exit in: %.2d s.`), then serves the SCSI vendor command 0xE8 with sub-codes `0x05 LOAD_INFO` (address, size,
MD5), `0x01 DOWNLOAD_KEEP`, `0x04 DOWNLOAD_END` (MD5 check), `0x06 RUN_CMD` (any U-Boot command line), `0x02 GET_RESULT`;
on request it runs `reset` (→ §4.2). Nothing in U-Boot writes `ota_upgrade_status` back to 0 (the name string has one
reference), so the PC tool has to send that through `RUN_CMD` [I]; with the flag set and a PC attached the camera stays in
`ufu` until the PC tool finishes [I].

### 4.5 Other script runners: `mstar`, `estar`, `dstar` — unused [R = oi `common/cmd_mstar.c`]

Common to the three: a 4 KiB heap buffer; `bootdelay` is set (`setenv bootdelay <argv[4]>`, default `1`) when a 5th argument
is given **or** `bootdelay` is unset; the script is loaded **into that 4 KiB buffer with no size limit** (a longer script
overruns the heap block [I]); lines are split by `get_script_next_line` 0x23E0E8FC: CR / LF skipped, `#` starts a comment
line, `%` at a line start ends the script, a line > 8000 characters ends it with `Error: the max size of one line is
%d!!!`; each line is printed (`\n>> %s \n`) and passed to `run_command`.

| command | syntax | load step | extras |
|---|---|---|---|
| `mstar` 0x23E0EABC | `mstar [script] [ipaddr] [serverip] [bootdelay]` | env `ipaddr` := argv[2] if given, else `192.168.0.28` if unset; `serverip` := argv[3] if given, else `192.168.0.26` if unset; `tftp <buf> <argv[1] or auto_update.txt>` (argument read in the disassembly at 0x23E0EBF4) | runs every line regardless of errors |
| `estar` 0x23E0EC8C | `estar [script] … [bootdelay]` | `tftp <buf> <argv[1] or auto_update.txt>` | stops at the first failing line (returns −1) |
| `dstar` 0x23E0EDB8 | `dstar [script] … [bootdelay]` | once per boot `mmc rescan 0`; `fatload mmc 0 <buf> <argv[1] or auto_update.txt>` | lines equal to `dstar scripts/[[CIS`, `…/set_partition`, `[[misc`, `[[logo`, `[[recovery`, `[[boot`, `[[system`, `[[data`, `[[cache`, `[[custcon`, `[[pqbin`, `[[config` print `PROGRAMING <name>...` and set 5…100 %; after `[[config`: `PROGRAM Done` |

`dstar`'s progress texts go to the **serial console**: they are passed to `UPDPrintLineSize` 0x23E10090, the no-display
(`#else` of `CONFIG_MS_DISPLAY`) stub of oi `common/cmd_osd.c` (l. 556), a single `b puts`; the colour (0xFF00FF00 green),
size 2, x 5 (`POS_CENTER`) and row (9, 10, …) arguments are ignored and the text is written without a newline.
`DrawProgressBar` is not called (it sits under `CONFIG_MS_SHOW_LOGO`, off: `do_dstar` has no call to it). 0x23E0B4F4 is the console `puts` of
`common/console.c` (`gd->have_console` at gd+0x1C = 0 → return; `gd->flags & GD_FLG_DEVINIT` (gd+4 bit 1) →
`stdio_devices[stdout]->puts` (+0x24 of the stdio device, table 0x23EA17E0); else `serial_puts` through the current serial
device 0x23E58B8C); 0x23E0B4C0 is `putc` (slot +0x20, 0x23E58B70), 0x23E0B528 `printf`. The display branch of `cmd_osd.c`
(`BootTEXTStr`, `PaintTextTransfer`, `PaintLCDText`, a framebuffer progress bar at `GOP_DISP_ADDR`) is not in the binary:
this U-Boot has no text drawer, and the boot framebuffer holds only the `bootlogo` JPEG and the `bootframebuffer` bar (§5.7).

## 5. `bootlogo` — the HDMI boot logo

Functions [R decompile; names from the `__func__` strings of their prints]: `do_bootlogo` 0x23E09B44, `_MiscPartiton`
0x23E08AA8, `_BootLoadFile` 0x23E06DC0, `_BootDispCtrl` 0x23E08424, `_BootLogoDispPnlInit` 0x23E08368, `_BootJpdArgbCtrl`
0x23E075E4, DISP timing-id helper 0x23E07FC8, HDMI-TX timing-id helper 0x23E081D4, `_LogoPartiton` 0x23E09658,
`_BootDbTable` 0x23E0948C; `str_split` 0x23E562EC. Compared with oi `common/cmd_bootlogo.c`.

### 5.1 Command [R]

`bootlogo <logo_id> <aspect> <x> <y>`. With argc ≠ 5 it prints `usage: bootlogo [logo_id] [aspect ratio '0: zoom' '1:
center' '2: usr'] [x] [y]` **and carries on** reading argv[1..4] anyway (with fewer arguments it reads past argv — garbage
or a NULL dereference [I]). The work is bracketed by the `do_display++` / `do_display--` stamps (§2.5). Then:
- `logo_id` = strtoul(argv[1], 0); env `logo_rot` → rotation (unset → 0);
- aspect record {mode = argv[2], x = argv[3], y = argv[4]} (strtoul base 0); modes 0 ZOOM, 1 CENTER, 2 USER;
- `_MiscPartiton(logo_id, &aspect, rot)`; only if it returns −1, `_LogoPartiton(logo_id, &aspect, rot)` (§5.5).
oi differs: an optional 5th argument names the section; no section list [R oi `do_display`].

### 5.2 `_MiscPartiton` — the MISC path (vendor rewrite) [R]

1. `mxp_load_table`; record `"MISC"` — on failure `failed to get MXP record with name: MISC` + `get Partiton failed!`, −1.
2. `firmwarefs_mount("MISC")` (fails → `inifs_mount %s failed!`, −1); `iniparser_load("config.ini")` from the MISC root
   (not `/config/config.ini`) through the `inifs_open/close/fgets` callbacks; NULL → `iniparser_load config.ini failed`, −1.
3. **Section list**: `iniparser_getstring("ROOT:m_bootlogoList", NULL)` → `str_split`: the value must be `{a, b, …}` —
   leading blanks, `{`, entries separated by any run of space / `,` / tab, closing `}`; returns a malloc'd copy, an array of
   pointers and a count; errors print `Do not detect : '{'`, `Do not detect : '}'`, `No contain!`, `No value.` and give
   count 0. Count 0 and no array → one section, the default **`LOGO`** (info print `use default section %s`).
4. **PQ, once for the whole list** (`_BootLogoLoadPQ` / `_BootLogoSetPq`): for each section in list order
   `_BootLoadFile(dict, section, "m_sPQFile0")`; on success build `MHAL_DISP_PqConfig_t {u32PqFlags = 1 (LOAD_BIN),
   u32DataSize, pData}` and call `MHAL_DISP_DeviceSetPqConfig(NULL, &cfg)` (0x23E22238, §6.1); success → info `set pq
   success!` and **stop the PQ loop**, keeping the buffer until the end of `_MiscPartiton`; failure → free the buffer, try
   the next section. Size or pointer 0 → `illegal param in _BootLogoSetPq 4462`; a NULL list entry → `illegal param in
   _BootLogoLoadPQ 4452`. Only `m_sPQFile0` is ever tried (no index loop).
5. **Per section** (`_LogoloadConfigIni`), in list order, with a running framebuffer offset `shift` that starts at 0 **on
   each call** (oi keeps it in a `static` across calls):
   - `DispOutCfg` (0x38 bytes, vendor layout): +0x00 `enDev` ← `<sec>:m_eDeviceType` (missing → error print `INI_GET_INT: Get
     %s error`, value 0; values > 4 → 0); +0x04 `u32DevId` ← `<sec>:m_wDevId` (missing → 0, info only); +0x08/+0x0C 64-bit
     `phyAddr` and +0x10 `u32Size` ← `bootargs` (`strstr "mmap_reserved=fb"`, then `sscanf
     "mmap_reserved=fb,miu=%d,sz=%[^,],max_start_off=%[^,],max_end_off=%[^ ]"`, sz and start by strtoul base 16; no
     `bootargs` or no token → 0 / 0); +0x14 `u32Width` ← `m_wDispWidth`; +0x18 `u32Height` ← `m_wDispHeight`; +0x1C
     `u32Clock` (= fps) ← `m_wDispFps` (each missing → error print, 0); +0x20 / +0x24 image width / height (written by the
     JPEG decoder); +0x28 `bColorMetrixIdValid`, +0x2C `u32ColorMetrixId` ← `MISC_DISP<devid>:m_wColorMetrixId`; +0x30
     `bGopDstIdValid`, +0x34 `u32GopDstId` ← `MISC_DISP<devid>:m_wGopDstId` (each: present → valid 1, absent → 0 / 0, info
     print only). Unlike oi, a missing key **does not abort** the section. oi's LCD panel branch (`m_sParaTarget` →
     `MhalPnlUnifiedParamConfig_t`, SPI / MIPI-DSI keys) is **not compiled in**: device type 1 (LCD) gets no panel set-up
     [R: no `m_sParaTarget` reference, no PNL calls].
   - `_BootLoadFile(dict, section, "m_sLogoFile<logo_id>")`; failure → the section is skipped (only the error prints).
   - JPEG decode `_BootJpdArgbCtrl` (software libjpeg; ARGB8888 output, the `CONFIG_SSTAR_RGN` variant): input buffer and
     size, output area `phyAddr + shift`, size `u32Size − shift`, returns image width / height, rotation `logo_rot`; then
     `flush_dcache_all`, free.
   - `_BootDispCtrl(&cfg, &aspect, shift)` (§5.3).
   - `shift += ALIGN(imgW × imgH × 4, 32)` (oi: × 3/2, the YUV420 size); if `u32Size < shift` → `bootlogo shift:0x%x out of
     size:0x%x` (a next section would then write past the reserved area [I]).
6. Free list and dictionary, `firmwarefs_unmount`, free the PQ buffer; return 0 (info `bootlogo exit. _MiscPartiton 4954`).

`_BootLoadFile(dict, section, key, &buf, &size)`: `"<section>:<key>"` → `iniparser_getstring`; NULL → `INI_GET_STR: Get %s
error` + `can't get %s` (the two lines in every boot log, key `LOGO:m_sPQFile0`); else `firmwarefs_open(name, O_RDONLY)`
(fail → `can't open %s!`), size = `firmwarefs_lseek(fd, 0, SEEK_END)` (0 → `%s size is 0!`), malloc (fail → `malloc memory
failed for PQ!` — the same text for logo files), `firmwarefs_read(fd, buf, size)` (0 → `read %s failed!`, free), close. File
names are relative to the MISC root. `firmwarefs_lseek(SEEK_END)` returns the size **without moving the file position**
(0x23E693C0 = oi `firmwarefs.c`), so the read starts at 0.

### 5.3 `_BootDispCtrl(cfg, aspect, shift)` [R]

1. `MHAL_DISP_DbgLevel(&0)`; `_BootLogoDispPnlInit`: 17 timing rows from .rodata 0x23E9B938 (28 B each: u32 timing id,
   10 × u16 HSyncWidth, HSyncBackPorch, VSyncWidth, VSyncBackPorch, HStart, VStart, HActive, VActive, HTotal, VTotal, u32
   DCLK **in Hz**) → 17 × `MHAL_DISP_PanelConfig_t` (old 0x6C-byte layout, §7) → `MHAL_DISP_InitPanelConfig(cfg, 17)`.
   Runs on **every** `_BootDispCtrl` call; this is where the four division traps come from (§5.4).
2. Interface from `enDev` (raw config.ini value, no oi-style translation): 1 → 0x10 LCD, 2 → 0x01 HDMI, 3 → 0x04 VGA,
   4 → 0x02 CVBS, else 0.
3. Vendor addition: if `bColorMetrixIdValid` or `bGopDstIdValid`: `MHAL_DISP_DeviceSetConfig(u32DevId, {u32 eType =
   2·colorValid | 4·gopValid, u8 ColorId @+5, u8 GopBlendId @+6})` (8-byte record; the first argument is the device **id**,
   not a context); failure → `%s %d, MHAL_DISP_DeviceSetConfig`. Not called when both are absent.
4. `MHAL_DISP_DeviceCreateInstance(&{alloc, free}, devId)` (fail → `CreateDevice Fail`), `VideoLayerCreateInstance(…,
   devId·2)` + `VideoLayerBind`, `InputPortCreateInstance(…, layer, 0)`; `SetBackGroundColor(dev, 0x800080)`,
   `DeviceEnable(dev, 0)`, `AddOutInterface(dev, intf)`, `SetOutputTiming(dev, intf, {GetTimingId(w, h, fps), NULL})`,
   `DeviceEnable(dev, 1)` — the oi order. The video layer and input port are created but never fed: the picture goes
   through the GOP (step 6), as in oi's `CONFIG_SSTAR_RGN` build.
5. HDMI-TX, only when intf = HDMI and the static HDMI-TX context is still NULL: `MhalHdmitxCreateInstance(&ctx, 0)` (INIT →
   `MApi_HDMITx_Init` → `InitSeq`: ATOP + DTOP common tables, power on, §8.2) → `SetHpdConfig({0x1A})` (GPIO 26, an i2m-style
   number with no effect: the U-Boot GPIO layer is a stub) → `SetMute({1, 7})` (video + audio + AV-mute) → `SetSignal({0})`
   → `SetAttr({video 1, in RGB444 (1), out RGB444 (1), 24 bit (1), HDMI (2), GetHdmitxTimingId(w, h, fps), audio 1, 48 kHz
   (3), 2 ch (0), PCM (0), PCM})` → `SetSignal({1})` (TMDS on) → `SetMute({0, 7})`. Identical to oi. No `SetAttrBegin/End`,
   so no `Exhibit`, no state machine, no EDID read (the task and timer are stubs, the DDC GPIO writes do nothing): the logo
   is sent blind at the configured timing.
6. GOP (RGN) picture, **GOP id 9 for device 0, 11 for device 1** (oi: fixed `E_MHAL_GOP_DISP0_UI`), gwin 0: `GopInit` once
   (static flag; oi re-inits on each call because its flag is a local), `SetBaseWindow(src, dst)`, `GwinSetPixelFormat(6 =
   ARGB8888)`, `GwinSetWindow(w', h', w'·4, 0, 0)`, `GwinSetBuffer(phyAddr + shift)` (oi: `phyAddr` without the shift, so oi
   shows the first picture on every display), `SetAlphaZeroOpaque(0, 0, ARGB8888)`, `SetAlphaType(pixel alpha = 1, 0xFF)`,
   `GwinEnable`. w' = min(dispW, imgW), h' = min(dispH, imgH); windows by aspect mode:

   | mode | src window | dst window |
   |---|---|---|
   | 0 ZOOM | 0, 0, w', h' | 0, 0, dispW, dispH |
   | 1 CENTER | 0, 0, w', h' | (dispW − w')/2, (dispH − h')/2, w', h' |
   | 2 USER | 0, 0, w', h' | x, y (argv), w', h' |
   | other | — | return without enabling the GOP |
7. Vendor addition: `MHAL_DISP_SetRegWaitDoneConfig(dev, {0, NULL})` (8 + 4 zero bytes) [R string at 0x23E22400].
8. Globals for `bootframebuffer`: `gu32FrameBuffer = phyAddr + 0x20000000` (**without** the shift), `gu32DispWidth/Height =
   imgW/imgH` — the progress bar always targets the first section's picture.

**Timing-id helpers** — they return the **old (ssd) enum values**, not oi's [R: compared value by value with ssd
`mhal_disp_datatype.h` / `mhal_hdmitx_datatype.h`; every arm of 0x23E081D4 checked in the disassembly]:

| w × h @ fps | DISP id (0x23E07FC8) | HDMI-TX id (0x23E081D4) |
|---|---|---|
| 720×576@25 / 720×480@30 | 0 PAL / 1 NTSC | 0x16 (none) |
| 1920×1080 @24/25/30/50/60 | 4 / 5 / 6 / 0xB / 0xC | 4 / 5 / 6 / 7 / 8 |
| 1280×720 @50/60 | 7 / 8 | 3 / 2 |
| 720×576@50, 720×480@60 | 0xD, 0xE | 1, 0 |
| 640×480@60, 800×600@60 | 0xF, 0x10 | 0x16, 0x16 (any fps) |
| 1280×1024, 1366×768, 1280×800 @60 | 0x12, 0x13, 0x15 | 0x12, 0xF, 0x11 |
| 1440×**800**@60 (DISP) / 1440×900@60 (HDMI-TX) | 0x14 | 0x10 |
| 3840×2160@30 | 0x1C | 0x16 |
| anything else (incl. the listed sizes at another fps) | 0x1F (MAX) | 0x16 (MAX) |

The DISP enum is ssd's exactly (MAX 0x1F). The HDMI-TX enum has MAX 0x16, one entry more than ssd's (MAX 0x15): the added
entry 0x15 is 2560x1440p30 (§8.3); the mapper never returns it. The `1440×800` test for DISP id 0x14 (1440×900) is a typo
carried from the source [R both]. In mhal's own HDMI-TX enum 0x16 would be 1920x1200p60. This camera's config (HDMI,
1920×1080@60) gives DISP 0xC and HDMI-TX 8 (→ Utopia 13 → ATOP / DTOP column 8, §8.2); `bootcmd`'s `bootlogo 0 0 0 0` is ZOOM.

### 5.4 The four `raise: Signal # 8 caught` [R]
<!-- ev:uboot.5.4 -->

Division helpers (libgcc, Thumb-2) [R disassembly]: `__udivsi3` 0x23E767A0 (64 ARM call sites), `__aeabi_uidivmod`
0x23E769FC (60), `__divsi3` 0x23E76A18 (55), `__aeabi_idivmod` 0x23E76CAC (6); every divide-by-zero path ends in `__div0`
0x23E77348 = `raise(8)` 0x23E763C4, which only prints `raise: Signal # %d caught` and returns 0 — the quotient is **0** and
execution continues.

The trap is in `DrvDispIfInitPanelConfig` (0x23E21830, ssd revision of `drv_disp_if.c`), called through
`MHAL_DISP_InitPanelConfig(table, 17)` from `_BootLogoDispPnlInit` (§5.3 step 1). Per entry [R 0x23E21960–0x23E219C8]:
```
fps       = round(m_dwPanelDCLK[u8 @+0x35] * 1000000 / (HTotal * VTotal))   ; __aeabi_uidivmod + __udivsi3
period_us = 1000000 / fps                                                    ; __udivsi3 at 0x23E219C8  <- traps when fps == 0
VSyncPeriod, VBackPorchPeriod, VActivePeriod, VFrontPorchPeriod = period_us * lines / VTotal
```
The ssd DISP API takes `m_dwPanelDCLK` as a **u8 in MHz**; the vendor `cmd_bootlogo` table carries DCLK as a u32 **in Hz**
(27 000 000, 74 000 000, 148 000 000 …; oi's table has 27 / 74 / 148 MHz), and `_BootLogoDispPnlInit` stores its low byte
(`strb` of the Hz word at +0x35). Per row [R: table dump + replay of the arithmetic]:

| row | timing (DISP id) | DCLK Hz in the table | low byte | fps computed |
|---|---|---|---|---|
| 0, 1 | PAL 0, NTSC 1 | 27 000 000 | 0xC0 | 356, 426 |
| **2** | **1080P60 0xC** | 148 000 000 | **0x00** | **0 → trap** |
| 3 | 1080P30 6 | 74 000 000 | 0x80 | 52 |
| **4** | **1080P50 0xB** | 148 000 000 | **0x00** | **0 → trap** |
| 5, 6 | 720P50 7, 720P60 8 | 74 000 000 | 0x80 | 86, 103 |
| 7, 8 | 480P60 0xE, 576P50 0xD | 27 000 000 | 0xC0 | 426, 356 |
| 9 | 1024×768 0x11 | 65 000 000 | 0x40 | 59 |
| 10, 11 | 1366×768 0x13, 1440×900 0x14 | 86 000 000, 106 000 000 | 0x80 | 90, 72 |
| **12** | **1280×800 0x15** | 84 000 000 | **0x00** | **0 → trap** |
| **13** | **1280×1024 0x12** | 108 000 000 | **0x00** | **0 → trap** |
| 14, 15 | 1680×1050 0x16, 1600×1200 0x18 | 146 000 000, 162 000 000 | 0x80 | 52, 47 |
| 16 | 800×480 as USER 0x1E | 43 000 000 | 0xC0 | 394 |

Four rows → four traps per `_BootDispCtrl` call (one displayed section → four lines per boot, as in every log). The rows hold
DISP ids only; 1080P24 / 25 (4 / 5), 640×480 (0xF), 800×600 (0x10) and 3840×2160 (0x1C) have no row, so `SetOutputTiming`
with those ids finds no timing entry (`disp_find_timing` (0x23e20af0) returns NULL).

Effect [I]: the poisoned values are `u16Fps` and the four `V*Period` fields (0 for the four rows, nonsense for the rest);
the porch, active and total values are copied correctly. In the mercury6 DISP HAL those fields feed only the time-zone
query (`__HalDispIfGetTimeZoneType`, frame timeouts) used by input-port flips [R oi `hal_disp_if.c` / `hal_disp_mop_if.c`];
the logo goes through the GOP and flips no input port, and the HDMI / VGA clocks come from the HAL's per-timing tables (§7)
— so nothing visible changes. The same MHz → Hz change shows in the SDK's MISC `config.ini` (an older copy has
`m_wPanelDCLK = 148`, the live file 148000000) [R]; the `bootlogo` table followed the new unit while the DISP API compiled
into U-Boot kept the old one [I].

### 5.5 Fallback `_LogoPartiton` + `_BootDbTable` — unused (no `LOGO` record on this flash) [R]

The oi `_LogoPartiton` flow built without `CONFIG_SSTAR_PNL` (compared step by step):
1. MXPT record `"LOGO"` (missing → `failed to get MXP record with name: LOGO`, −1); malloc(size); `memcpy` from the
   memory-mapped NOR at `start | 0x14000000`; cache flush.
2. Image layout (little-endian): `SS_HEADER_Desc_t` {char title[8] = `"SSTAR"` (5 bytes compared), u32 count @+8}; then
   `count` blocks, each `SS_SHEADER_DataInfo_t` {char name[32]; u32 TotalSize @+0x20; u32 SubHeadSize @+0x24; u32
   SubNodeCount @+0x28}, next block = this + SubHeadSize + TotalSize. Block `"DISP"` extends the header with {u32
   FirstUseOffset @+0x2C (from the image start); u32 DispBufSize @+0x30; u32 DispBufStart @+0x34}; its entries (0x30 B
   each, SubNodeCount of them, from block + SubHeadSize) are `SS_SHEADER_DispConfig_t` {u32 device (1 LCD, 2 HDMI, 3 VGA);
   char resName[32] @+4; u32 width @+0x24, height @+0x28, clock (fps) @+0x2C}. Blocks `"LOGO"` = one JPEG each, data at
   block + SubHeadSize.
3. `_BootDbTable`: device must be 2 or 3 (else `Device fail`). Env `dispout` unset → `dcache off`, `setenv dispout <first
   entry's resName>`, `saveenv`, `dcache on`, use the first-use entry. Equal to the first-use entry's name → `DB Table and
   setting match.` Else search the entries by name: found → `FirstUseOffset = entry − block + 0xC` and the **partition is
   rewritten** (`dcache off`, `sf probe 0; sf erase <start> <size>`, `sf write <buf> <start> <size>`, `dcache on`) so the
   choice persists; not found → `Not found dispout: %s`, −1.
4. The `logo_id`-th `"LOGO"` block (counted among LOGO blocks only; missing → `Not found LOGO header`) is decoded into
   `DispBufStart` (size `DispBufSize`) and shown by `_BootDispCtrl(cfg, aspect, 0)` with devId 0, no `MISC_DISP` options,
   device / size / fps from the chosen entry (LCD keeps w / h / fps 0 — no panel code in this build).
The tool that builds such a partition is not on hand [I: the SigmaStar SDK `logo` packer].

### 5.6 What runs on this camera [R MISC `config.ini` + the code above]

- MISC `config.ini` (`/misc/config.ini` on the Linux side; keys owned by `config_tool.md` §4.1; read through firmwarefs by
  §5.2) has no `ROOT:m_bootlogoList`, so the default section `LOGO` is the only one: `m_eDeviceType = 2` (HDMI), `m_wDispWidth/Height/Fps` 1920×1080@60, `m_wDevId = 0`,
  no `MISC_DISP0` section → no `DeviceSetConfig`; `m_sLogoFile0 = sigmastar1024_600.jpg` (id 0, `bootcmd`), `m_sLogoFile1 =
  upgrade.jpg` (id 1, `sdstar`); ZOOM; GOP 9; framebuffer at DRAM offset 0xF700000, 8 MiB (from `bootargs`, so one string
  serves the kernel and the logo). `[LOGO]`'s `m_sParaTarget = DACOUT_1080P_60` is not read (no reference to the string in the
  binary); the timing comes from width / height / fps. kjd_lvgl replaces `/misc/sigmastar1024_600.jpg` from the SD card
  (`kjd_lvgl.md` §3.1).
- No PQ: `[LOGO]` has no `m_sPQFile0` → the two error lines, nothing sent to DISP.
- Unused, specified above: the section list (`ROOT:m_bootlogoList = {LOGO, LOGO1}` would also drive `[LOGO1]`: `m_eDeviceType
  = 2` HDMI on devId 1, layer 2, GOP 11, 1920×1080@30 = DISP id 6; its `m_sParaTarget = SAT070AT50H18BH` is not read; the
  HDMI-TX would not be set up a second time because its context is static, so device 1 would drive no HDMI-TX of its own
  [I]); the PQ load from the first listed section that names a file (`[LOGO1] m_sPQFile0 = PQ.bin`, §6); `[PQ_PATH]`
  `default = "/misc/PQ.bin"` is not read by U-Boot; the `MISC_DISP<n>` colour-matrix / GOP-blend options; aspect modes 1 / 2;
  `logo_rot`; the `LOGO`-partition fallback with `dispout`.
- Whether the logo is visible on a monitor is not known: no record exists, and nothing found statically stops it (the four
  traps change nothing visible, §5.4). Neither is it known what MOP / GOP state U-Boot leaves for Linux beyond the MOP DMA
  thresholds of §7.

### 5.7 `bootframebuffer`, `progressbar`, `blank` [R]

`bootframebuffer bar <pct>`, `progressbar <pct> [msg]` and `blank` draw over the logo framebuffer `gu32FrameBuffer` with the
size `gu32DispWidth/Height` of the first section's picture (§5.3 step 8); `sdstar` uses `bootframebuffer bar` (§4.1).

## 6. PQ in U-Boot and the `PQ.bin` format

U-Boot carries a loader for the display picture-quality bin; on this camera **no code path loads `PQ.bin`**: U-Boot would
load it only if a `bootlogo` section named it in `m_sPQFile0` (only `[LOGO1]` does, and `[LOGO1]` is not listed, §5.6), and
mi_disp only through `PQConfig.ini`, which is absent (`mhal-disp.md` §13.1, `config_tool.md` §6) [R config + code]. This section owns
the file format; the kernel-side hand-over is `mhal-disp.md` §13.1.

### 6.1 Code path [R addresses; disassembly where the decompile has no function]

The DISP query callbacks below have no function of their own in the decompile (§1.4) and are read in the disassembly.
Path of `bootlogo`'s PQ step (§5.2 step 4):
1. `MHAL_DISP_DeviceSetPqConfig(NULL, {flags, size, data})` 0x23E22238 → `DrvDispIfDeviceSetPqConfig` 0x23E21D84: query
   type 0x28 (PQ_SET), 12-byte config; ctx NULL and flags 1 or 2 → `HalDispIfInit` 0x23E266C8 + query without context, else
   with the context. Return 0 → `Device DisplayInfo Fail` print (`MHAL_DISP_DeviceSetPqConfig 403`).
2. `_HalDispIfSetPqSet` **0x23E25FDC** (callback): `flags == 1` → `HalDispHpqLoadBin` 0x23E2AAB0; `flags == 2` and `size == 4`
   and data ≠ NULL → u32 at data: low 16 = source id → `HalDispHpqSetSrcId` 0x23E2AB70, high 16 = panel id →
   `HalDispHpqSetPnlId` 0x23E2AB80 (kept only if < the SourceLUT panel count), picture data from the bin
   (`__HalDispIfGetPictureDataFromPq` 0x23E25A60) → `HalDispPictureIfSetPqConfig(panel id, …)` 0x23E2D4E0; `flags == 3` and
   *data ≠ 0 → `MDrv_PQ_LoadSettings` 0x23E2AB68 + 0x23E2FBE4 (applies the picture config [I: = mhal
   `HalDispPictureIfApplyPqConfig`]); **any other flag is ignored** (no flag 4 / manual load type, which mhal has).
3. `HalDispHpqLoadBin(ctx, cfg)`: builds a 0xDC-byte `MS_PQ_Init_Info` (zeroed; +0x1A u16 = 1 (bin count), +0x1C u8 bin id
   = 0, +0x20 and +0x24 = data pointer, +0x28 = size) → `MDrv_PQ_Init(info, ctx)` 0x23E2A610 →
   `MDrv_PQ_InitSourceTypeTable(0)` 0x23E2A78C.
4. `MDrv_PQ_Init`: stores ctx (0x23EE8188); clears the SRAM-table cache (0x23E29BC4: ten u16 "last loaded group" slots =
   0xFFFF); bin count 0 → only the "initialised" flag (0x23EE7EB2 := 1); else prints **`Start PQ Bin Init`** (plain printf,
   would show in the boot log), panel index := 0, size 0 → `MDrv_PQ_Init()  Error: idx:%d PQ_Bin_BufSize < 0 !!`; else prints
   `u8ID=%d`, header record = 0x23EE81A0 + id × 0xD0, `MDrv_PQBin_Parsing` 0x23E2A384 (prints `BinStartaDDR ID%u=%lx`),
   `MDrv_PQBin_LoadCommTable(panel, hdr)` 0x23E2A31C. **No check of the header text, version, offsets or size** — a wrong file
   is parsed blindly.
5. `MDrv_PQ_InitSourceTypeTable`: the SourceLUT panel count must be exactly 1 (else `PnlNum:%d more than 1`, printed for 0
   too); reads panel 0's SourceNum, (re)allocates SourceNum × 6 bytes (0x23EE817C {u16 count; ptr}); for each source s:
   current source := s, table := `GetTableIndex(s, IP 0 = INFO, panel)`, `LoadTableData(panel, table, IP 0, hdr, buf, 0x30)`
   → the 6 value bytes of INFO's group become entry s = **{u8 device flag, u8 interface code, u16 width, u16 height}**.
6. Automatic source selection, at the end of every `_HalDispIfSetDeviceOutputTiming` (0x23E25E2C, i.e. inside `bootlogo`'s
   `SetOutputTiming`): key = {u8 (device field == 1), u8 interface code (`_HalDispHpqTransInterface` 0x23E2AA10: HDMI bit 0 →
   1, VGA 4 → 2, CVBS 2 → 3, 0x100000 → 4, BT656 0x40000 → 5, BT601 0x80000 → 6, TTL 0x10000 → 7, MIPIDSI 0x20000 → 8,
   0x200000 → 9, 0x400000 → 10, else 0), u16 device +0x1E, u16 device +0x28 [I: active width / height]} →
   `MDrv_PQ_GetMatchSoruceType` 0x23E299D4 over the 6-byte table: an exact 6-byte match wins; else, if key byte 0 is 7…10
   (never true — byte 0 is the 0 / 1 device flag), the first entry whose first two bytes match; else the first entry with the
   same device flag, same width / height and interface byte **0**; else the first entry with the same device flag; else
   0xFFFF. Always "found" when the table exists → print `SrcType:%d` (debug), set the source, picture data,
   `HalDispPictureIfSetPqConfig` (0x23E2D4E0), 0x23E2FBE4, `MDrv_PQ_LoadSettings`. (mhal runs the same code, likely with the
   same byte order [I] — `mhal-disp.md` §13.1.)
7. `MDrv_PQ_LoadSettings` (body 0x23E2A9B8): if initialised, for each IP 0…IPNum−1: skip if `GetSkipRule(ip)` ≠ 0; table :=
   `GetTableIndex(source, ip, panel)`; `GetTable`; `DumpTable` (0x23E29E00).
8. `DumpTable` by table type: NULL table (RegNum 0 or offset 0xFFFFFFFF) → nothing; **50 GENERAL** and **52 SCALER**: per row
   `{bank, addr, mask, value[group]}` → byte read-modify-write at RIU bank 0x11`bank`, 16-bit register `addr>>1`, low / high
   byte `addr&1` (CPU address 0x1F000000 + (0x11`bank``addr` & ~1)·2 + (addr&1)); SCALER rows are skipped while the MLoad
   flag (0x23EE04AA) is set; GroupIdx ≥ GroupNum → `[PQBin]IP_Info error: Scaler Reg Table`; **53–63** (SRAM1–4, C_SRAM1–4,
   colour index / gain): the per-type "last group" cache is updated, **nothing is written** (U-Boot has no Y/C filter SRAM
   writer); **64 VIP_IHC_CRD_SRAM**: only when the group differs from the cached one; data = table + GroupIdx × RegNum =
   0x441 u16 entries, split 0x121 / 0x110 / 0x110 / 0x100 into four sub-SRAMs, each written by
   `_Hal_PQ_set_sram_ihc_crd_table` 0x23E297F0 (window must be 0, else `PQWin:%d is not correct`): 1169:7C bit 0 := 1,
   7C[2:1] := sub-SRAM id; per entry 1169:7D[8:0] := index, 1169:7E[8:0] := value (9 bits), 7E bit 15 := 1 (strobe); finally
   7C bit 0 := 0; **65 VIP_ICC_CRD_SRAM** (`Hal_PQ_set_sram_icc_crd_table`, window 0 only): 1169:78 bit 0 := 1; for i =
   0…0x3FF: 1169:79[9:0] := i, 1169:7A[8:0] := byte0 + (byte1 & 1)·256 of u16 entry i, 7A bit 15 := 1; then 78 bit 0 := 0 —
   only the first 1024 of the 1089 entries are written; any other type → `[PQBin] Unknown sram type %u`. COMB (51) does
   nothing (compiled out in the source too).
9. `MDrv_PQBin_LoadCommTable(panel)`: for every IP, table = COMM (0xFFFE) → type, CommRegNum rows at CommOffset, one group;
   SRAM-type IPs (53…57, 64, 65) are skipped; the others go through `DumpTable`.

The bin functions (`Parsing`, `GetTable`, `GetTableIndex`, `GetSkipRule`, `LoadTableData`, `LoadTable`, `LoadTableBySrcType`
(`[PQ]QMIPtype_size=%d`), `TransToPQWin`, the XRule / GRule getters) follow oi `drv_disp_hpq_bin.c` statement for statement
in the parts read (Parsing and GetTableIndex line by line, the rest by structure and strings); the `MDrv_PQ_*` around them
are the mhal versions minus flag 4.

### 6.2 `PQ.bin` file format [R oi `drv_disp_hpq_bin.c` / `drv_disp_pq_bin.h` = U-Boot `MDrv_PQBin_Parsing` 0x23E2A384, checked against this camera's file byte for byte]

All integers little-endian, read byte-wise (no alignment requirement). "Label" = 16 bytes of text padded with spaces. Every
offset stored in the index points at a label and the data follows it (the reader adds 16), **except the XRule group table
and the GRule rule / level offsets as noted**. Offsets are from the start of the file.

| offset | size | field |
|---|---|---|
| 0x00 | 16 | header text (here `M6_PQ`) — not checked |
| 0x10 | 16 | version text (here `39630.43853` [I: an Excel date serial, 2008-07-01 10:31]) — not checked |
| 0x20 | 4 | StartOffset (here 0x5F0 = first table label) — read, not used |
| 0x24 | 4 | EndOffset (here 0x6520 = end of the last table) — read, not used |
| 0x28 | 7 | dummy (here `a5 59 3e 5f 3c a5 59`) |
| 0x2F | 1 | PQID (here 0xFF) |
| 0x30 | 16+2+2 | `GRULE_HEADER` label, u16 RuleNum, u16 PnlNum |
| … | 14 × RuleNum | GRule info {u16 IPNum, u16 SourceNum, u16 GroupNum, u32 IPOffset (label), u32 RuleOffset (label)} |
| … | 6 × RuleNum × PnlNum | GRule level {u16 LvlNum, u32 Offset (label; data = u16 level → group index)}, index = panel + PnlNum × rule |
| … | 16+2 | `XRULE_HEADER` label, u16 RuleNum |
| … | 12 × RuleNum | XRule info {u16 IPNum, u16 GroupNum, u32 IPOffset (label; data = IPNum × u16 IP index), u32 GroupOffset (**no label**; data = GroupNum × IPNum × u16 table index)} |
| … | 16+2+4 | `SKIPRULE_HEADER` label, u16 IPNum, u32 Offset (label; data = IPNum × u16, ≠ 0 → IP skipped by LoadSettings) |
| … | 16+2 | `IP_COMM_HEADER` label, u16 IPNum |
| … | 16 × IPNum | IP info {u16 TableType, u16 CommRegNum, u16 IPRegNum, u16 IPGroupNum, u32 CommOffset (label), u32 IPOffset (label)} |
| … | 16+2 | `SRCLUT_HEADER` label, u16 PnlNum |
| … | 8 × PnlNum | SourceLUT info {u16 SourceNum, u16 IPNum, u32 Offset (label; data = SourceNum × IPNum × u16 table index, row = source)} |

Table index values: 0…GroupNum−1 = group of the IP table; **0xFFFE** = the common table (CommOffset, CommRegNum rows, one
group); **0xFFFF** = no table. Table bodies by `TableType` (enum `PQ_TABTYPE_*`, `drv_scl_pq_define.h`):
- 50 GENERAL / 51 COMB / 52 SCALER: RegNum rows of `3 + GroupNum` bytes {u8 bank, u8 addr, u8 mask, u8 value[GroupNum]}
  (common table: GroupNum = 1); write = §6.1 step 8.
- 53…65 SRAM types: GroupNum consecutive blocks of RegNum bytes each (IHC / ICC CRD: RegNum 2178 = 1089 u16).
- 69 PICTURE_1 / 70 PICTURE_2: GroupNum blocks of RegNum u8 / u16 (oi `MDrv_PQBin_LoadPictureSetting`; not used by U-Boot).

The text bin (`MDrv_PQTextBin_*`: SrcType / IPName / GroupName sections) is a separate file format; it is not in MISC and
not linked into U-Boot [R: none of its strings in the image; I: not linked]. The labels carry the names instead: IP table
labels are `<IP>` and `<IP>_COM` (truncated to 16 characters). Layout as the vendor tool writes it [R this file]: index area
0x00–0x5EF, then the tables in IP order, each label + body padded with 0xFF to a 16-byte boundary; the SourceLUT body last;
4 zero bytes after EndOffset (read by no parser). No other unexplained bytes (639 bytes outside the structures, all 0xFF
padding + the 4-byte trailer).

### 6.3 This camera's `/misc/PQ.bin` (25 892 B) [R]

- GRule: 0 rules. XRule: 2 rules `XRUL_HSPRule` / `XRUL_VSPRule`, each 1 IP (82 `SWDriver`), 1 group, table index 0xFFFF (no
  effect). SkipRule: IPs 0 `INFO`, 1 `PICTURE`, 83 `SC_End` skipped. 84 IPs, **all CommRegNum = 0** (LoadCommTable writes
  nothing), all type 52 SCALER except 68 `VIP_IHC_CRD_SRAM` (64) and 73 `VIP_ICC_CRD_SRAM` (65), 2 groups each.
- One panel, 30 sources. `INFO` (6 rows, mask 0 → never written) holds per source {device flag, interface, width, height}:
  sources 0–14 device 0, 15–29 device 1, each 15 = {0×0, 720×480, 720×576, 1280×720, 1920×1080, 2560×1440, 3840×2160,
  1024×768, 1280×800, 1280×1024, 1366×768, 1440×900, 1600×1200, 1680×1050, 1024×600 (interface 7 = TTL)}; all others
  interface 0. An HDMI 1080p60 key {0, 1, 1920, 1080} has no exact match and selects **source 4** by the "interface 0, same
  size" rule (§6.1 step 6).
- Per source the LUT selects (other IPs: no table for any source): `PICTURE`, `BW2SBRI`, `WLE` group 0; device-0 sources
  `PEAKING` + `GAMMA_0{,_R,_G,_B}` group 0; device-1 sources `GAMMA_1{…}`, `VIP_Peaking`, `VIP_Peaking_band` group 0;
  `VIP_LineBuffer` group by size (0×0 none, then 1, 2, 3, 0, 4…13); IHC / ICC CRD SRAM group 1. So 65 of the 84 IP tables
  are carried in the file but never selected.
- Register banks touched (rows with mask ≠ 0): 1150 (DRC, DLC), 1152 MACE (PICTURE, BW2SBRI, WLE, BLE, LPF, HCORING,
  PEAKING, FCC_T1…9), 1153 GAMMA0, 1162 (GAMMA_1), 1169–116F VIP blocks (IHC 1169, ICC 116A, FCC / ACK / clip 116B, DLC / UVC
  116C, LCE / VIP 116D, DLC histogram / FCC 116E, peaking / LPF 116F); `VIP_ICC_Ymode` also has rows in bank 1125 [R; I: a
  generator slip or a real cross-bank write — not resolvable statically].
- `PICTURE` (29 one-byte "rows", read as data by `HalDispHpqLoadData(…, 0x20)`, never written as registers because SkipRule
  skips IP 1): group 0 = video gain R/G/B 0x80, brightness R/G/B 0x80, hue 0x32, saturation 0x80, contrast 0x80, peaking band
  1 0x30, band 2 0x14, then 9 × u16 YUV → RGB coefficients 0x0730 0x04AC 0x0000 / 0x1223 0x04AC 0x10DA / 0x0000 0x04AC 0x0878
  — sign-magnitude (bit 12) BT.709 limited-range values [I from the numbers: 1.797, 1.168, −0.534, −0.213, 2.117]; group 1
  (never selected) has BT.601-like values in two's complement (0x0662 0x04A8 0 / 0xFCBF 0x04A8 0xFF26 / 0 0x04A8 0x0812) —
  the two groups use different encodings [R values; I encodings].

## 7. The DISP HAL copy (0x23E24974–0x23E34080) [R: decompile and register-table dump of `uboot.bin`]
<!-- ev:uboot.7 -->

The block splits into **383 functions** (bfuncs). 85 of them name themselves through the `__func__` strings of their error
/ debug prints (compared through their `__LINE__` numbers with mhal's); by register constants and strings
74 of the 85 agree with mhal and 71 with oi — too few prints survive for that to place the copy. Placed by code read
directly:

| point | U-Boot | oi | mhal |
|---|---|---|---|
| `HalDispMopgInit` (0x23e2fe6c) DMA thresholds (1170:04 / 05) | **0xF8 / 0xD0**, no FF0 register (`HalDispMopgInit`) | 0xF8 / 0xD0 | 0x30 / 0x30 + FF0 0x4040 |
| MOPG init trigger | static once-flag (call at 0x23e33d1c) | once-flag | hardware reset bit |
| VGA / HDMI timing table | **18 ids**, IDAC per-timing table 2 rows (145B:33, 145B:37) — identical to the source tables | 18 ids | 27 ids, 145B:33 moved to the common table, + HDMI-TX ATOP table |
| `__HalDispIfDumpSynthSetTbl` | `HalDispSetHdmiSynthSet(0)` for HDMI + VGA, VGA table when intf & 4 | same | no SynthSet call, VGA table only when intf == 4 |
| `__HalDispIfSetTgenRest` | source logic + a debug print (line 0x5b) | no print | DAC FPLL lock sequence |
| `_HalDispIfGetInfoDeviceInit` | `HalDispUtilityInit(dev id)` (lines 0x32c / 0x332) | same | per-device CMDQ id (lines 0x40a…0x417) |
| `MHAL_DISP_PanelConfig_t` | **0x6C bytes** (eTiming +0x60, bValid +0x68; fps from a byte at +0x35); `InitPanelConfig` limit 0x1f | 0x74 | 0x74, limit 0x27 |
| PQ query flags | prints "SET SRC ID" (flag names like mhal's) | — | same names |

String diff of all 383 functions against the oi mercury6 DISP source (`disp/hal/mercury6`, `disp/drv`, `hal/infinity2m` as
the shared-code fallback) with the mhal cross-check [R]:

| group | functions |
|---|---|
| string-less (register getters / setters, CMDQ / RIU access) | 311 |
| string-bearing | 72 |
| — all literal strings covered by the oi mercury6 source | 25 |
| — logic strings oi-covered, plus ANSI-coloured debug lines that are in mhal | 46 |
| — one coloured debug line matched in neither (`NULL CmdqInf`, a wording variant) | 1 |

Of the 530 strings in the block, 439 are in the oi source and 91 are not; 90 of the 91 are ANSI-coloured debug lines
(`\x1b[1;31m%s %d, …`) present verbatim in mhal. So **each function's body logic is the oi mercury6 revision and its
debug-message layer the vendor's (mhal) revision**; nothing in the logic strings is newer than oi, and the register
evidence (0x6C panel struct, 18 timing ids, MOP 0xF8 / 0xD0) is the older oi / ssd shape, not mhal's. The 311 string-less
functions stand on the register-level comparison above (the VGA / HDMI register tables dumped from `uboot.bin` are
byte-identical to the source's, the MOP init thresholds, the IDAC table, the SynthSet / TGEN differences) [R; I that these
are the same functions].

**U-Boot's copies of the display timings** (owned here): the DISP VGA / HDMI timing table with its 18 ids and the 2-row
IDAC table — byte-identical to oi; the 17 `bootlogo` panel-timing rows with a
Hz DCLK (§5.4); the HDMI-TX ATOP / DTOP tables (§8.2). The MI / mhal timing enums and tables are `mi_disp-mi_hdmi.md` §4.6.4 and
`mhal-disp.md` §2.2, §7.1, §7.4.

**State left for Linux:** U-Boot (like oi) initialises the MOPG of display device 0 (bank 0x1170) with 1170:04[7:0] = 0xF8
(luma DMA request threshold) and 1170:05[7:0] = 0xD0 (chroma DMA request threshold) — the registers of mhal's
`HalDispMopgSet{DmaY,DmaC}FifoThrd`; mhal writes 0x30 / 0x30 only while the MOP is
still in reset, so the U-Boot values stay until the rcS `riu_w` list rewrites the two registers [I] (register meaning
`mhal-disp.md` §11.4, the write list `rootfs-customer.md` §5.2). The live values before `riu_w` are not recorded.

## 8. The HDMI-TX copy (0x23E34080–0x23E3E7A0, ≈42 KB) [R: decompile and register-table dump]
<!-- ev:uboot.8 -->

The HDMI-TX code is 0x23E34080–0x23E3E7A0; the M6 HAL files are 0x23E3A288–0x23E3E7A0 (≈18 KB, including the unused i2m
HDCP HAL). Below it lies DISP: the DISP function split covers 0x23E24974–0x23E34080 (§7) [R]; by string references
the DISP picture / MOP code ends at 0x23E32150 and 0x23E32150–0x23E34080 holds code unrelated to HDMI-TX [R]. The two
placements of that stretch are not reconciled. A placement by string references alone (0x23E24974–0x23E3E76C, ≈80 functions /
≈30 KB) takes the DISP code in with the HDMI-TX code.

### 8.1 Layout and origin
<!-- ev:uboot.8.1 -->

The `bootlogo` side of the `MhalHdmitx*` calls is §5.3 step 5.

Register access functions: `Mask_Write` 0x23E3A830, `Mask_WriteByte` 0x23E3A870, `Read` 0x23E3A7BC, `Write` 0x23E3A800.
Function names come from aligning them in address order with the mhal functions (reordered to the U-Boot link order) on
strings, literal register calls, constants and size: 56 alignments anchored by a `__func__` string, 76 more scoring ≥ 0.6,
86 weak (layout-consistent, not individually checked), 25 U-Boot functions with no mhal counterpart; bfuncs splits the block
into 296 functions (the extra ones are callback-table entries). The block was then string-compared against the ssd source
it comes from, with the mhal cross-check [R]: of 541 literal strings **503 are in the
ssd source verbatim**, and 37 of the remaining 38 are in mhal.

| range | source file (ssd tree) | fn | strings | not in ssd | weak vs mhal | notes |
|---|---|---|---|---|---|---|
| 0x23E34080–0x23E341FC | `drv/hdmitx/src/uboot/drv_hdmitx_os.c` | 35 | 0 | 0 | 26 | U-Boot OS layer: tasks, timers, events, IRQ, I²C, clock calls are empty `return TRUE/FALSE`; **GPIO read returns 1**, GPIO write / request do nothing; `GetMmioBase` = 0x1F000000 (size 0x400000 / 0x200000); delays = `udelay` loops |
| 0x23E34200–0x23E38170 | `drvHDMITx.c` | 68 | 173 | 3 | 20 | full ssd revision **including the HDCP 1.4 transmitter** (`HDCPTx_Handler` 0x23E367FC, 428 lines: "Check Ri failed", "Exchange KSV fail", repeater / KSV list, "HDMI Advanced Cipher"); `MapVideoTimingToVIC` prints ssd's "Error video timing"; the 3 strings not in ssd are the profiling lines of `HDMITx_Handler` 0x23E3772C (`HpdTIme=%d, EdidTime:…TransmitTime…DoneTime`, `TMDS=%x`), all in mhal |
| 0x23E3817C–0x23E397A0 | `apiHDMITx.c` | 59 | 75 | 0 | 1 | ssd revision with the range checks ("Error output mode / deep color / video color / audio frequency") that the mhal copy no longer has |
| 0x23E397A0–0x23E3A288 | `drv_hdmitx_if.c`, `drv_hdmitx_ctx.c`, `mhal_hdmitx.c` | 17 | 69 | 19 | 9 | newer revision = mhal's (ANSI-coloured debug strings); only 5 `Mhal*` entry points are referenced by `bootlogo` |
| 0x23E3A288–0x23E3AB84 | `halHDMIUtilTx.c` | 27 | 23 | 0 | 9 | SW I²C, register access, EDID read, `AdjustDDCFreq` (identical constants) |
| 0x23E3AB84–0x23E3C584 | `halHDMITx.c` — the **M6 HAL, older revision** (§8.2) | 36 | 42 | 1 | 9 | the string not in ssd is `2560x1440_30P` (§8.3) |
| 0x23E3C588–0x23E3CFF8 | `halHDCPTx.c` (i2m) | 11 | 19 | 0 | 2 | HDCP HAL absent from mhal: BKSV / An / Aksv / Km / Ri over the i2m HDCP bank **0x172B** ("Reading BKSV failed", "Writing An failed", "Cipher not done") — unreached: `bootlogo` never selects an HDCP mode [R strings, bank literal; I unreached] |
| 0x23E3C584–0x23E3E7A0 | `src/hal_hdmitx_if.c` | 54 | 159 | 15 | 12 | newer revision = mhal's; query table as ssd: **14 queries** (no FORCE_ATTR / DRV_CUR_GET — no ForceAttr / DrvCurGet functions in the aligned set) |

So the drv / api / util / HDCP core and the M6 HAL are the ssd bodies (a few lines newer than the ssd snapshot), the two
`*_if.c` wrapper layers are the later revision mi_hdmi / mhal use: 34 ANSI-coloured debug strings (`\x1b[1;31m%s %d, …`)
absent from ssd, all but one (`CreateInstance Fail`, a prefix variant of mhal's) in mhal. The 86 weak matches resolve: 26
are the OS stubs (no strings, no mhal Linux-OS counterpart to score against), the other ≈60 are small getters / setters /
flag accessors of the ssd-era layers whose weak score reflects mhal being another revision; the 25 U-Boot-only rows are
callback-table splits and OS stubs [I]. No function has a string that is in neither ssd nor mhal.

### 8.2 U-Boot M6 HAL against the mhal M6 HAL [R decompile of the listed functions]

Same: bank bases 1459 / 145A, CLKGEN 1038:48 single 432 MHz clock (`SetClk` / `GetClk` identical), `GetRXStatus` logic (but
without the `g_bHpdDetect` override — and with the OS GPIO read returning 1, HPD always reads high), `Power_OnOff`,
`SetTMDSOnOff`, the video / audio / colour / mode functions, `SendPacket`, the DTOP timing-generator rows.

| item | U-Boot copy | mhal copy |
|---|---|---|
| `DumpAtopTable` tables | **4 descriptors** at 0x23EA0510: [0] **ATOP per-timing 14 rows × 18 columns**, [1] ATOP common 23 rows, [2] DTOP per-timing 13 × 18, [3] DTOP common 7 | 2 descriptors (DTOP only, 27 columns); the ATOP tables moved to DISP |
| timing → column | 18 columns: Utopia 3→0 … 13→8 (480p … 1080p60), 37→9 (1440p30), 19→10 (2160p30), 30→11, 29→12, 34→13, 28→14, 35→15, 36→16, 27→17 | 27 columns |
| Utopia timing ids | up to 37 (2560x1440p30), MAX 38 | up to 45 |
| ATOP values | the 18 columns are byte-equal to the corresponding columns of mhal's DISP-side ATOP table (incl. 145A:16H 0x07, 22–25 0x2C, 30H / 31H / 32L 0x08, 39H / 3BL 0x33, 3BH 0x23, 1DL 0x00 for 2160p30) | — |
| `InitSeq` | common tables (ATOP common → 145A:16L = 0xF0 TMDS data enable, 145A:33 / 34 / 35 = 0 power, 145A:1C = 0x01 …; DTOP common); 145A:19 / 1A / 38 defaults | DTOP common only, **plus 1459:1F bits 0 and 1** |
| drive current | 8 bytes (Tap1 / Tap2 × 4), inlined into `SetVideoOutputMode`, skipped when both words are 0; no 145A:32 write | 11 bytes, + PdRtermCh / PdLdoPreDrvCh / PdLdoMuxCh → 145A:32 |
| HDMI 2.0 set | 2160p50/60, 4096p50/60; deep colour: 1280x1470p60, 2160p24–30, 4096p24–30 | + 2160p29.97 |
| HPD-detect override, `DebugEnable`, `GetAnalogDrvCur` | absent | present |

The ATOP per-timing table writes 145A:32L = 0x0F (0x08 for 1440p50/60 and 2160p), the bits mhal names PdRtermCh (bits 3:0)
and PdLdoPreDrvCh (bits 7:4); so those fields are normally 0xF / 0x0 and 0x8 / 0x0 at the 297 MHz class [R values; I the
field overlap]. With this camera's logo timing (MHAL 8 → Utopia 13) the ATOP / DTOP column is 8.

### 8.3 Timing translators and the added enum entry 0x15 [R disassembly]

| layer | U-Boot function | cases | default |
|---|---|---|---|
| MHAL → HAL (`drv_hdmitx_if.c`) | `_DrvHdmitxIfTransTimingResypeToHal` 0x23E39804 | 0 … **0x15** returned unchanged (22 values) | 0x16 |
| HAL → Utopia (`hal_hdmitx_if.c`) | `__HalHdmitxIfTransTimingRes` 0x23E3DFB8 | 0 … 0x14 → 3, 4, 6, 5, 9–13, 17–21, 30, 34, 28, 35, 29, 36, 27 (= ssd's i2m HAL and the first 21 arms of mhal's) | **38** |

ssd's `_DrvHdmitxIfTransTimingResypeToHal` ends at `1600X1200P_60HZ` = 0x14 with MAX 0x15 (`…/hdmitx/drv/hdmitx/src/drv_hdmitx_if.c`
l. 361); the U-Boot copy has one more identity case, 0x15, and MAX 0x16. The entry is **2560x1440p30**, named in the
binary: the HAL debug name table at 0x23E3D8C0 (used by `_HalHdmitxIfGetInfoAttrConfig`, whose `__func__` is its last
entry) concatenates the attribute-enum names; its timing block runs index 0xF…0x24 = enum 0…0x15, listing `720x480p60 …
1600x1200p60` (enum 0x14) and then exactly one more, `2560x1440p30` (index 0x24 = enum 0x15), before the audio-frequency names;
a second copy `2560x1440_30P` sits in the M6-HAL mode-name table at 0x23E3B638. Neither string is in ssd, both are in mhal,
where 0x15 is `2560x1440p30` mapping to Utopia 37. In U-Boot the entry is half wired: the HAL → Utopia translator has no arm
for 0x15, so a request for it becomes Utopia 38 (MAX, unsupported); the only producer of MHAL timing ids, the `bootlogo`
mapper, returns 0–8, 0xF, 0x10, 0x11, 0x12 and 0x16 — the entry is **unused**.

## 9. Commands outside the boot path, and the engines under them
<!-- ev:uboot.9 -->

Usage / help texts from the command table [R]; handlers read in the decompile and
compared with the oi source of the same command. "= oi" means the handler matches the source statement for statement in
calls, strings and constants [R].

Used on the boot path or plain debugging tools (behaviour as in the oi source except where §2–§5, §10 say otherwise):
`base cmp cp crc32 md mm mw nm loop go help/? version printenv setenv saveenv env run reset bootm sf mmc mmcinfo fatinfo
fatload fatls fatsize fatwrite fatrm i2c icache dcache gpio riu uart debug dbg initDbgLevel btime mxp ufu sdstar bootlogo
bootframebuffer`. Everything below is **unused** on this camera — no caller on the boot path, in `bootcmd` or in any
script on hand — except the EMAC bring-up, which runs at every boot (§2.1).

### 9.1 Network: commands, EMAC driver, net stack
<!-- ev:uboot.9.1 -->

The SoC has an EMAC; the camera has no Ethernet connector [R].

**Commands** [R]:

| command (handler) | syntax as built | behaviour as built | vs source |
|---|---|---|---|
| `bootp` 0x23E060AC / `dhcp` 0x23E060B8 | `[loadAddress] [[hostIPaddr:]bootfilename]` | `netboot_common(BOOTP 0 / DHCP 4, …)` 0x23E05C04 | = `common/cmd_net.c` |
| `tftpboot` 0x23E060C4 | same | stamps 0xA3 / 0xA4 (§2.5) around `netboot_common(TFTPGET 3)` | = oi |
| `ping` 0x23E06124 | `pingAddress` | `string_to_ip`, `NetLoop(PING 5)`; prints `host %s is alive` / `ping failed; host %s is not alive` | = oi |
| `sfbin` 0x23E05F00 (not repeatable) | `Address offset [[hostIPaddr:]filename]` | SigmaStar addition: `sf probe 0`; length = `get_sf_size(offset)` (fails → `Get sf len(0x%x) from offset err!`); `sf read <addr> <offset> <len>`; then `netboot_common(TFTPPUT 0xC, argv' = {"", addr, len, filename})` — uploads that flash range to a TFTP server | = oi (incl. an unused `malloc(16)`) |
| `estart` 0x23E1C940 | `reset` / `start` (help; arguments ignored) | `MDrv_EMAC_initialize(NULL)` + `(Re)start EMAC...` | = `drivers/mstar/emac/cmd_emac.c` |
| `macaddr` 0x23E1CA70 | `XX XX XX XX XX XX` (exactly 6) | parse 6 hex bytes, `setenv -f ethaddr %02X:…`, print `MAC Address …`, write the SA1–SA4 address registers | = oi |
| `phy_r` 0x23E1C960 / `phy_w` 0x23E1C9DC | `phyAddress` / `phyAddress phyValue` (hex) | refuse with `Please start emac first!` unless the EMAC is initialised; PHY id 0; `read_phy` / `write_phy` | = oi |

`get_sf_size(offset)` 0x23E06984 (= oi `common/cmd_sf.c`, paired SAME): reads the `flash` pointer of `cmd_sf.c` (0x23EA1738,
set by `sf probe`): NULL → `please do command:"sf probe 0" first!`, −1; offset > `flash->size` (+0x0C of `struct spi_flash`,
unsigned compare) → `The offset(%d) is beyond spi flash size!`, −1; else `size − offset`. Its only caller is `sfbin`, which
probes first, so the NULL branch is reachable only if that probe fails. On the 16 MiB NOR, `sfbin <addr> 0 <file>` would read
and upload the whole flash [I].

**EMAC driver** (0x23E1B28C … 0x23E1CB00) [R; source oi `drivers/mstar/emac/mdrv_emac.c` + `mercury6/mhal_emac.c` + `cmd_emac.c`].
The string-bearing functions pair SAME (`MDrv_EMAC_initialize`, `_VarInit`, `_PhyAddrScan`, `_DumpMem`, `board_eth_init`,
`MHal_EMAC_CableConnection`, `_CheckTSR`, `_NegotiationPHY`, `_HW_init`, the four `_Ewavetest_*`, the four commands); the
string-less ones match the source statement by statement in the points below. Build
options visible in the code: **MII bus mode only** (`MHal_EMAC_ClkGen` and `_Power_On_Clk` contain only the `m_bus_mode ==
EMAC_MII_MODE` arm — the static is never changed, so the RMII arms were folded away [I]); `CONFIG_MS_EMAC1` off (MAC bank
1510); `SOFTWARE_DESCRIPTOR` on; `CONFIG_ETHERNET_ALBANY` on (the Realtek-PHY patch is compiled, `PatchPHY`);
`SUPPORT_ALBANY`'s `albany_patch` / `albany_write_reg` absent; `CONFIG_ETHERNET_FIXLINK` off (auto-negotiation);
`RX_OFFSET_4_BYTE_PATCH`, `EMAC_DBG_TFTP`, `CONFIG_ETHERNET_ICPLUS` off.

Banks [R literals]: MAC **1510** (0x1F2A2000) and its upper half **1511** (0x1F2A2200, the `JULIAN_01xx` registers and the
clock bits); second-MAC bank **1514** (0x1F2A2800, only its SA1 address is written); internal PHY "Albany" **1518 / 1519 /
151A** (0x1F2A3000 / 3200 / 3400); CLKGEN 1038 (byte `.84`), SCGPCTRL 1133 (bytes `.44`, `.46`), CHIPTOP 101E (byte `.a1`),
PADTOP 103C (byte `.c6`), PM_SLEEP 000E (byte `.50`). Byte registers as `MHal_EMAC_WritReg8` addresses them (notation above).

MAC register map (32-bit values split into low / high 16-bit RIU words; RIU reg of bank 1510; names from the source
`TITANIA_EMAC` struct) [R source; offsets confirmed by the binary accessors]:

| reg | name | use in this driver |
|---|---|---|
| 00–01 | CTL | [2] RE, [3] TE, [4] MPE (management port, MDIO), [5] CSR (clear statistics), [7] WES (write-enable statistics) |
| 02–03 | CFG | [0] SPD 100M, [1] FD, [11:10] MDC divider (written 0xC00 = slowest, "MDC ≤ 2.5 MHz") |
| 04–05 | SR | [2] PHY management idle (polled after each MDIO access) |
| 06–07 / 08–09 | TAR / TCR | TX frame bus address / length — writing TCR starts the send |
| 0A–0B | TSR | TX status (`CheckTSR` waits for it before a send) |
| 0C–0D | RBQP | RX descriptor queue base |
| 0E–0F | RBCFG (BUFF) | RX buffer base \| size select (`RX_BUFFER_BASE \| 1`) |
| 10–11 / 12–13 / 14–15 / 16–17 / 18–19 | RSR / ISR / IER / IDR / IMR | status and interrupt registers (U-Boot polls; `close` writes IDR) |
| 1A–1B | MAN | MDIO frame: `phy << 23 \| reg << 18 \| 2 << 16 \| data`, write op `0x50020000`, read op `0x60020000` |
| 1C–1D / 1E–1F | RBRP (RDPTR) / RBWP (WRPTR) | RX ring read / write pointer ([14] wrap) |
| 20–43 | FRA, SCOL, MCOL, OK, SEQE, ALE, DTE, LCOL, ECOL, TUE, CSE, RE, ROVR, SE, ELR, RJB, USF, SQEE | statistics counters (read only by `MDev_EMAC_stats`, not linked) |
| 48–4B | HSH / HSL | multicast hash |
| 4C–5B | SA1L/H … SA4L/H | four station addresses (`m3 m2 m1 m0`, `m5 m4`) |
| 5C–5D | TAG_TYPE | — |
| 1511.00–05 | JULIAN_0100 / 0104 / 0108 | 0100 := **0xF011** (MII); 0104 := 1 (software-descriptor enable); `MHal_EMAC_HW_init` sets MIU priority, TX "JULIAN_T" 4, burst-16, RX FIFO enlarge 2, TX-hang ECO, MIU out-of-range fix, and bit 0 of the word at 0x138 |

Functions as built (source names by match) [R]: SA1–SA4 writers 0x23E1B55C / 59C / 5DC / 61C; `Read_CTL` 0x23E1B65C,
`Write_CTL` 0x23E1B678, `Write_CFG` 0x23E1B694, `Write_RBQP` 0x23E1B6B0, `Read_MAN` 0x23E1B73C, `Write_MAN` 0x23E1B758,
`Write_BUFF` 0x23E1B788, `Write_RDPTR` 0x23E1B7DC, `Write_WRPTR` 0x23E1B7F8, `Write_JULIAN_0100` 0x23E1B814,
`Write_JULIAN_0104` 0x23E1B830, `enable_mdi` 0x23E1B84C, `MHal_EMAC_write_phy` 0x23E1B86C, `MHal_EMAC_read_phy` 0x23E1B8E0,
`enable/disable_phyirq` 0x23E1B95C / 0x23E1B960 (empty), `CableConnection` 0x23E1B964, `CheckTSR` 0x23E1BA94,
`NegotiationPHY` 0x23E1BAC8, `ClkGen` 0x23E1BC04, `eWave_Table_Init` 0x23E1BDB8, `Power_On_Clk` 0x23E1BDF4,
`MHal_EMAC1_Write_SA1_MAC_Address` 0x23E1BE1C, `MHal_EMAC_HW_init` 0x23E1BE5C, `MDrv_EMAC_close` 0x23E1BF38, `MDrv_EMAC_open`
(with `start` inlined) 0x23E1BF58, `PhyAddrScan` 0x23E1C044, `VarInit` 0x23E1C10C, `DumpMem` 0x23E1C294, `tx` 0x23E1C324, `rx`
0x23E1C3CC, `MdcStandardization` 0x23E1C5A4, `PatchPHY` 0x23E1C5AC, `MDrv_EMAC_initialize` 0x23E1C614, `board_eth_init`
0x23E1C918; the eWave pattern generators 0x23E1B28C (100M), 0x23E1B304 (10M LTP), 0x23E1B37C (10M all-ones), 0x23E1B434 (10M
TP-idle). No body in the image [R: no code with their constants / strings]: `MHal_EMAC_trim_phy` (eFuse PHY trim),
`MDrv_EMAC_WaveTest`, `_Bist`, `_DumpReg`, `MDev_EMAC_stats`, `_PowerMode`, `_SwReset`. The eWave table (0x23EA5188) is
filled by `Power_On_Clk` but **nothing reads it** (the only reference to its address is the filler's literal [R: word search
over the image]), so the four eWave generators are dead code.

Bring-up (`estart` → `MDrv_EMAC_initialize`, or `board_eth_init` from the net core when `autoestart` is yes) [R]:
1. env `PHY_ID` (decimal) → PHY address (default 0, global 0x23EDDE58).
2. `VarInit`: static `emac_var` area (RX buffer 128 × 0x600 = 0x30000 B + 128 × 8 B descriptors + 0x8000), base aligned up to
   16 KiB; env `ethaddr` → SA1, else the built-in `00:00:23:34:45:66` (`MY_MAC`); speed 100 / full duplex defaults.
3. `Power_On_Clk` → `ClkGen` (byte registers as numbered in the source): 1511 b3A = 0, b3B = 1, b00 |= 0x10, b39 |= 1; CLKGEN
   `1038.84 = 0` (EMAC AHB clock 123 MHz), SCGPCTRL `1133.44 = 0`, `.46 = 0` (EMAC RX / TX clocks from the PHY, 25 MHz); then
   the Albany PHY power-up / tuning list — the source's Albany byte writes of the MII arm (`eth_link_sar`, LDO, SADC, ADCPL,
   REF, TX power-on, ADC clock select, 100T phase, DAC / LP mode, inverted-waveform fix, EEE off, the shadow-banked
   `tin17…19_s2` values, …), **in the same order and with the same values** as oi `mercury6/mhal_emac.c` `MHal_EMAC_ClkGen`
   [R, write by write; the meaning of the tuning bytes and of the `JULIAN_01xx` bits beyond the source comments is not known —
   there is no M6 register reference]; CHIPTOP `101E.a1 &= 0x7F`; PM_SLEEP `000E.50 = (v & ~0x30) | 0x10`. Then `eWave_Table_Init` and PADTOP
   `103C.c6 = (v & 0x77) | 0x22`.
4. `MdcStandardization`: CFG = 0xC00. `PhyAddrScan`: JULIAN_0100 = 0xF011; read BMSR of the current PHY address; if it is 0
   or 0xFFFF scan addresses 0…31 (`phy [%d]=%x`); none found → `Can't get correct PHY Addr and reset to 0`.
5. `PatchPHY`: PHY reg 2 = 0x001C (Realtek OUI) → reg 25 := 0x0400 (read before and after).
6. `MDrv_EMAC_HW_init`: CTL = 0; RX buffer cleared; BUFF / RDPTR / WRPTR / RBQP; print `MAC Address …`; SA1–SA4; JULIAN_0104 =
   1; `NegotiationPHY` (prints `Auto-Negotiation...`; BMCR := 0x1000; `CableConnection` waits ≤ 20 × 200 ms for BMSR[5]
   AN-complete (`AN fail`), then ≤ 20 × 200 ms for BMSR[2] link (`Link up fail`), and resolves ANLPAR >> 5 & BMSR >> 11 to
   4 = 100 FD, 3 = 100 HD, 2 = 10 FD, 1 = 10 HD; prints `Link Status Speed:%u Full-duplex:%u`) → CFG = 0xC00 | SPD / FD; CTL
   = MPE (+ WES if set); JULIAN_0100 = 0xF011; `MHal_EMAC_HW_init`, which also takes env **`ethaddr1`** and writes it to SA1 of
   bank 1514.
7. First call only: `nic` "sstar_emac" with init = `open`, send = `tx`, recv = `rx`, halt = `close`, MAC = SA1,
   `eth_register`; a later call only re-scans the PHY and returns −1.

Data path [R = source]: `open` sets CTL.CSR, rebuilds the 128 RX descriptors (word 0 = buffer address `RX_BUFFER_BASE +
0x600·i`, the last one with the wrap bit 2; DONE = bit 0), re-programs BUFF / RDPTR / WRPTR / RBQP and sets RE | TE. `rx`
polls the descriptor at the software index: DONE → length = size & 0x7FF, copy out (with the ring-wrap split), advance RDPTR
by the length rounded to 4, `NetReceive`, clear DONE. `tx` pads to 60 bytes, waits on TSR (`CheckTSR`), copies into a static
2096-byte buffer, flushes, TAR = its bus address, TCR = length. `close` clears RE | TE and disables the interrupts in IDR. PHY
loopback test hooks (`Rx Data` / `Tx Data` dumps) are present but `ThisBCE.loopback` is always 0 (`_Bist` not linked).

What runs on this camera [R: boot log]: steps 1–7 on **every boot** — `board_eth_init`
is called by the net core at `Net:` and `autoestart` is unset; the log shows `MAC Address …`, `Auto-Negotiation...`, `AN fail`,
`Link Status Speed:10 Full-duplex:0`, then `Status Error!` (negotiation result 0 → CFG stays 10 Mbit half duplex); `sstar_emac`
is registered with the MAC from env `ethaddr` (set per unit) and left as `ethact`; ≈4 s. The data path, the net
commands and `estart` / `macaddr` / `phy_r` / `phy_w` never run.

**Net stack** (0x23E72E70 … 0x23E763C0, + `cmd_net.c` 0x23E05C04 … 0x23E06160) [R]: the U-Boot 2015.01 `net/` of oi
(`net/arp.c`, `bootp.c`, `eth.c`, `net.c`, `ping.c`, `tftp.c`) and `common/cmd_net.c` (with SigmaStar's `sfbin`); 25
functions pair SAME, none differs: `netboot_common` 0x23E05C04, `do_sfbin` 0x23E05F00, `do_tftpb` 0x23E060C4, `do_ping`
0x23E06124; arp — `ArpRequest` 0x23E72E70, `ArpTimeoutCheck` 0x23E72EF4, `ArpReceive` 0x23E72F74; bootp — `DhcpOptionsProcess`
0x23E731AC, `DhcpHandler` 0x23E73430, `BootpRequest` 0x23E73988, `BootpTimeout` 0x23E73C90; eth — `eth_write_hwaddr` 0x23E73EAC,
`eth_register` 0x23E740D8, `eth_initialize` 0x23E74154 / 0x23E74420 (two halves), `eth_try_another` 0x23E7439C; net —
`net_auto_load` 0x23E745F4, `NetStartAgain` 0x23E7462C, `NetReceive` 0x23E74868, `NetInitLoop` 0x23E750A4, `NetLoop`
0x23E75204; tftp — `icmp_handler` 0x23E75870, `TftpSend` 0x23E7589C, `TftpHandler` 0x23E75BBC, `TftpTimeout` 0x23E75EF8,
`TftpStart` 0x23E75F84. The string-less helpers between them (environment MAC / IP getters, checksum, packet builders,
`ip_to_string` 0x23E75008, ping's handlers, the TFTP progress marker 0x23E75B08) sit inside the same object ranges and have no
anchor of their own [R ranges; I that they are the source's static helpers]. Behaviour = the U-Boot 2015.01 source.

Built-in protocol set [R: strings present / absent in the image]:

| present | evidence |
|---|---|
| Ethernet core, `ethact`, `ethprime`, `ethrotate`, `ethaddr` / `eth%daddr`, `netretry`, `autoload` | `eth.c` / `net.c` strings |
| ARP (gateway use, `ARP Retry count exceeded`) | `arp.c` |
| BOOTP and DHCP client (`BOOTP broadcast %d`, `DHCP client bound to address %pI4 (%lu ms)`, options host name, root path, domain, "Opt Boot File" vendor boot file; unknown options reported `*** Unhandled DHCP Option in OFFER/ACK: %d`) | `bootp.c` |
| ICMP echo (`ping`) and ICMP redirect report | `ping.c`, `net.c` |
| TFTP get **and put** (`Loading: *`, `Saving: *`, `Save address/size`), `blksize` option (env `tftpblocksize`), env `tftptimeout` (min 1000 ms), env `tftpsrcp` / `tftpdstp` ports, default file `%02X%02X%02X%02X.img` (own IP in hex), put errors `File too large` / `File has bad magic` | `tftp.c` |
| **absent:** CDP, DNS, NFS, SNTP, RARP, link-local, VLAN (`vlan` / `nvlan`), TFTP server mode, SigmaStar `net_upgrade.c` | none of their strings is in the image |

On this camera the stack runs only `eth_initialize` → `board_eth_init` → `eth_register` at `Net:`.

### 9.2 `ini_*` — INI / file tools of `cmd_bootlogo.c` [R handlers 0x23E070CC…0x23E07BD8 = oi]

All take `<partition>` = an MXPT record name (mounted with `firmwarefs_mount`, unmounted at the end) and `<file>` relative
to its root; a wrong argument count → `arg error!` (return 0). Errors go to stderr: `inifs_mount return error %d`, `inifs_open
return NULL`, `inifs_close return error %d`. `get_int` / `get_str` / `get_str_array` ignore a failed mount or load
(dictionary NULL → default value).

| command | syntax | effect |
|---|---|---|
| `ini_get_int` 0x23E079CC | `<part> <file> <section> <key>` | `iniparser_getint("section:key", -1)` → `Partition %s file %s, section %s, key %s=%d` |
| `ini_get_str` 0x23E07908 | same | `iniparser_getstring(…, "")` → `… key %s=%s` |
| `ini_get_str_array` 0x23E07494 | same | value split as `{a, b, …}` (§5.2 step 3) → `Partition %s file %s, section %s count %d` + per item `idx[%d]=%s(%d)` (the second number = the item as hex via strtoul) |
| `ini_get_filesize` 0x23E070CC | `<part> <file>` | `file len is %d` (lseek SEEK_END) |
| `ini_get_buf` 0x23E07A9C | `<part> <file> <pos> <len>` (decimal) | reads min(len, 32) bytes at pos → `seek is %d`, `value is` + ` 0x%2x` per byte |
| `ini_put_buf` 0x23E07264 | `<part> <file> <pos> <hex>` | `<hex>` must have even length (else `ini_put_buf:invalid length %d ((%d %% 2) != 0)`), digits outside the accepted range → `ini_put_buf:invalid value 0x%x at %d`, at most 64 digits = 32 bytes used; opens O_RDWR, seeks, **writes into the flash file** |
| `ini_create_file` 0x23E07190 | `<part> <file> <text>` | open O_CREAT\|O_RDWR (0x42), write the text (no NUL) → `write len is %d` |
| `ini_write_file` 0x23E07BD8 | `<part> <file> <flags> <text> <pos>` | open with numeric `flags` (decimal), seek to pos if > 0, write the text → `write return %d` |

So this U-Boot can **write files in MISC (or any firmwarefs partition)** through `ini_put_buf` / `ini_create_file` /
`ini_write_file` (the firmwarefs write path is linked). Hex digits: `A`…`Z` → value − 0x37 and `a`…`z` → value − 0x57 are
accepted with no range check to F, so `G`…`Z` give 16…35 and corrupt the byte (high-nibble overflow) [R = oi].

### 9.3 `eeprom` [R 0x23E03E7C = `common/cmd_eeprom.c`]

`eeprom read|write <addr> <off> <cnt>` (hex; exactly 4 arguments), I²C device address fixed 0x54
(`CONFIG_SYS_I2C_EEPROM_ADDR`); prints `EEPROM @0x%lX %s: addr %08lx off %04lx count %ld ... `, calls `eeprom_read` 0x23E03D80 /
`eeprom_write` 0x23E03DF8, then `done`. No EEPROM on this board [I].

### 9.4 `srcfg` — sensor pad, MCLK and pin set-up (vendor, no source) [R: 0x23E1DD54…0x23E1E5F4]
<!-- ev:uboot.9.4 -->

oi's mercury6 build would take the 5-argument `#else` variant (`PdnPad RstPad MclkIdx PwdPin RstPin`); this binary has the
10-argument variant of the infinity6e branch with an M6 driver that is in neither tree (`snr_cfg_mercury6` [I name]).
Nearest source: oi `drivers/mstar/isp/snr_cfg_infinity6e.c` (the same function names and message strings, but three sensors,
five bus types and other register fields). The driver's literal bases are 0x1F207900 = PADTOP bank 103C + 0x100 (reg 0x40)
and 0x1F207100 = CLKGEN bank 1038 + 0x100; every pad field below is in PADTOP bank **0x103C**.

Call chain (`do_srcfg` 0x23E11028): `srcfg SnrIdx BusType PadSel CtrlPad MclkPad RstPad PwnPad MclkIdx PwdPin RstPin` — ten
decimal `simple_strtoul` values (`argc < 10` → `missing parameters`, so 9 arguments pass the check and argv[10] = NULL is
parsed), then:
1. `Set_SNR_BusType(id, bus)` 0x23E1DD54: `bus_tbl[id] = bus`, four words at 0x23EDE6A8 (`.bss`); id is not range-checked
   (id ≥ 4 writes past the table).
2. `Set_SNR_IOPad(id, PadSel, CtrlPad, MclkPad, RstPad, PwnPad)` 0x23E1DD64 — the tables below.
3. `Set_SNR_MCLK(id, MclkIdx < 999, MclkIdx)` 0x23E1E4D4.
4. `SNR_PowerDown(id, PwdPin)` 0x23E1E59C, `SNR_Reset(id, RstPin)` 0x23E1E5C8.
5. Print `Sensor configuration, ID=%u, Bus=%u, Pad=%u, CtrlPad=%u, MclkPad=%u, RstPad=%u, PwnPad=%u, MCLK=%u, PWD_PIN=%u,
   RST_PIN=%u`; return 0.

Bus types: the bus value indexes six-entry jump tables (`cmp r2,#5; ldrls pc,[pc,r2,lsl#2]`), so the build knows bus 0…5.
The help text names them **0 Parallel, 1 MIPI_1Lane, 2 MIPI_2Lane, 3 MIPI_4Lane, 4 BT656, 5 BT1120** [S help text]; the
fields written match mhal's pad functions of the same bus (`mhal-vif.md` §6.1) [R]. It is the i6e enum (`VIF_SNR_PAD_MODE`,
five values) plus BT1120 [R i6e header]. Bus 1 and 2 always take the same arm. The help also says `SensorId 0~3`, `PadSel
1~10`, `PwdPin` / `RstPin` 0 low / 1 high [S].

`Set_SNR_IOPad`: "≤ n" = the field is written only when the value is in 0…n (unsigned compare); a larger value is skipped
**silently**. "no" = prints `[U-Boot] SNR_<id> don't support <IO|Ctrl|Mclk|Rst|Pdn> mode_<bus>.` (the `%d` after `mode_`
receives the **bus type**, not the pad value — r2 holds the bus at every call) and skips that field. Steps run in the order
IO, Ctrl, Mclk, Rst, Pdn; each re-reads `bus_tbl[id]`.

Sensor 0 (table 0x23E1DD94 for IO; the other four fields are bus-independent; sensor 0 never prints "don't support"):

| bus | IO (PadSel) | Ctrl | Mclk | Rst | Pdn |
|---|---|---|---|---|---|
| 0 | ≤2 → `103C.6A[5:4]` | ≤2 → `.69[5:4]` | ≤1 → `.69[6]` | ≤1 → `.69[7]` | ≤2 → `.69[9:8]` |
| 1, 2 | ≤4 → `.69[2:0]` | same | same | same | same |
| 3 | ≤2 → `.69[2:0]` | same | same | same | same |
| 4 | ≤8 → `.6A[3:0]` | same | same | same | same |
| 5 | ≤2 → `.6A[9:8]` | same | same | same | same |
| > 5 | nothing; the function returns before Ctrl (no message) | — | — | — | — |

Sensor 1 (table 0x23E1DEF8 for IO; Ctrl / Mclk / Rst / Pdn by compare chains):

| bus | IO | Ctrl | Mclk | Rst | Pdn |
|---|---|---|---|---|---|
| 0 | ≤4 → `.6B[6:4]` | ≤2 → `.7A[1:0]` | ≤2 → `.7A[5:4]` | ≤3 → `.69[14:13]` | ≤1 → `.69[15]` |
| 1, 2 | ≤1 → `.69[10]` | same | same | same | same |
| 3, 4 | no | no | no | no | no |
| 5 | ≤2 → `.6A[9:8]` | same as bus 0 | same | same | same |
| > 5 | skipped silently | skipped | skipped | skipped | skipped |

Sensor 2 (tables 0x23E1E108 IO, 0x23E1E154 Ctrl, 0x23E1E1D0 Mclk, 0x23E1E22C Rst, 0x23E1E288 Pdn):

| bus | IO | Ctrl | Mclk | Rst | Pdn |
|---|---|---|---|---|---|
| 0 | no | no | no | no | no |
| 1, 2 | ≤4 → `.6B[10:8]` | ≤2 → `.75[1:0]` | ≤1 → `.75[4]` | ≤1 → `.75[8]` | ≤2 → `.75[13:12]` |
| 3 | no | no | no | no | no |
| 4 | ≤10 → `.6B[3:0]` | no | no | no | no |
| 5 | no | ≤2 → `.75[1:0]` | ≤1 → `.75[4]` | ≤1 → `.75[8]` | ≤2 → `.75[13:12]` |
| > 5 | skipped silently | skipped | skipped | skipped | skipped |

Sensor 3 (no tables; signed compare chains — a negative bus is skipped silently; sensor id ≥ 4: nothing):

| bus | IO | Ctrl | Mclk | Rst | Pdn |
|---|---|---|---|---|---|
| 0 | no | no | no | no | no |
| 1, 2 | ≤1 → `.6B[12]` | ≤2 → `.76[1:0]` | ≤2 → `.76[5:4]` | ≤3 → `.76[9:8]` | ≤1 → `.76[12]` |
| 3, 4, 5 | no | no | no | no | no |
| > 5 | skipped silently | skipped | skipped | skipped | skipped |

`Set_SNR_MCLK(id, en, idx)`: field = `idx << 2 | (en ? 0 : 1)` when idx ≤ 14, else field = **1** (gated, select 0)
regardless of `en`; written to CLKGEN sensor 0 `1038.62[13:8]`, sensor 1 `1038.65[5:0]`, sensor 2 `1038.65[13:8]`, sensor 3
`1038.66[13:8]`; id ≥ 4 → nothing. `en` = (MclkIdx < 999); the help lists **0 27 MHz, 1 72, 2 61.7, 3 54, 4 48, 5 43.2, 6 36,
7 24, 8 21.6, 9 12, 10 5.4, 999 MCLK off** [S help] — the select order of mhal's `HalVif_ConvertMCLK` (`mhal-vif.md` §6.3)
[R]. Any idx 15…998 also ends gated. 11…14 pass the range check but are not in the help; mhal's table has 11 = 37.125 MHz,
the i6e header names 11…14 LPLL, ÷2, ÷4, ÷8 [S i6e header] — not verified for M6. Unlike mhal's `HalVif_SetMCLK`, U-Boot does
not clear `1038.62[3:0]` first.

`SNR_PowerDown(id, v)` / `SNR_Reset(id, v)`: address = `(id + 0xF9304) << 9` = bank 0x1304 + id, reg 0x00: PowerDown writes
bit 3 := v & 1, Reset bit 2 := v & 1 (16-bit read-modify-write); no range check on id. The value is written **as given** —
mhal's `DrvVif_SensorPdwn/Reset` write the inverse (`= (v == 0)`, `mhal-csi-snr.md` §3) [R both].

Sensor numbering against mhal [I from the fields]: every field U-Boot's sensor n writes is the field mhal writes for VIF
**pad {0, 2, 1, 3}[n]** (`mhal-vif.md` §6.1, §6.3; channel banks 1304 / 1305 / 1306 / 1307 = mhal pads 0 / 2 / 1 / 3 by
`HalVif_GetChnBySNRPadID` 0 → ch 0, 1 → ch 8, 2 → ch 4, 3 → ch 12), i.e. U-Boot numbers sensors 1 and 2 the other way round
from mhal's pad index. One exception: U-Boot sensor 1's **parallel** IO field `.6B[6:4]` is mhal's parallel field of pad 1
(= U-Boot sensor 2), and U-Boot sensor 2 rejects parallel [R both] — the parallel arm does not follow the swap [I: likely a
copy slip in the vendor file]. This camera's MIPI sensor is on mhal pad 0, which `srcfg 0 3 …` would address; the sensor is
brought up by Linux / mhal, which program the same fields (`mhal-vif.md` §6.1).

### 9.5 `otpctrl` [R 0x23E06AD8 = `common/cmd_otpctrl.c`]

`otpctrl -w <cmd> <off> <data>` → `svc 1` with r0 = cmd << 24 | off, r1 = data; `otpctrl -r <cmd>` → `svc 2` with r0 = cmd <<
24; cmd ≥ 0x100 → `[ERR] otp cmd 0x%lx out of range`; then the results from the IMI log area 0xA0010000 (u16 error code →
`[ERR] otp control failure, error code:%x`; for reads a u16 size at +2 and the data at +4, printed 16 per line, size cleared).
U-Boot's SVC vector (0x23E000C0) passes the SVC number and r0 / r1 to 0x23E010FC, which prints `[OTP Control] operation =
%lx, command = %lx, data = %lx` and calls the function pointer at **0xA0000010 = the IPL's OTP service** (operations, ids,
status words: `ipl.md` §7.1). It works only while the IPL image is still in IMI SRAM [I]. The help lists command 0 = RSA
public N key (256 B), 1 = RSA public E key (4 B).

### 9.6 Crypto: `aes`, `secauth`, `sigauth`, `test_sig` and the AESDMA / SHA / RSA block
<!-- ev:uboot.9.6 -->

There is no `KEY_CUST` record on this flash. Key area `KEY_CUST_LOAD_ADDRESS` = 0x23FFF000 (where IPL_CUST would place the
`KEY_CUST` partition; nothing is loaded there on this camera [I from `ipl.md` §1.1]). Image = legacy uImage: the first word read
little-endian must be 0x56190527 (`IH_MAGIC` 0x27051956 in big-endian order), data size = big-endian `ih_size`, data at +0x40
[R].

**Commands** [R]:
- **`aes`** 0x23E0F8AC (= `common/cmd_mstar.c` `do_aesdma`): `aes enc|dec <image_addr> ECB|CTR|CBC CIPHER|EFUSE|HW
  [partition]` (≥ 4 arguments, else `missing parameters`); bad magic → `image header check failed, can't …`; chain mode
  default CBC (`use default chainmode: CBC`); key type CIPHER → key at 0x23FFF040 (key area + header size), EFUSE → OTP key 1
  (the default, `use default keytype: EFUSE`), `HW` → OTP key 2; in-place AESDMA over the data (`MDrv_AESDMA_Run`, below);
  encryption also sets header byte 0x3F := `E` (the "encrypted" mark that `secauth` / `image_get_encryption` test); with the
  5th argument: `mxp r.info <partition>`, `sf probe`, `sf erase ${sf_part_start} ${sf_part_size}`, `sf write <image>
  ${sf_part_start} <size + 0x40>` — the processed image is written to that MXPT partition.
- **`secauth`** 0x23E0FD14 (= `do_auth`): `secauth enc|dec <image_addr> <chain> <keytype> <partition> AUTHON|AUTHOFF
  AESON|AESOFF` (≥ 7 arguments); prints `[U-Boot] ChainMode: …`, `KeyType: …` (CIPHER key here 0x23FFF210), `Encrypt/Decrypt
  setting done`; only with exactly 7 arguments it acts: AUTHON → `runAuthenticate(image, size + 0x40, key area 0x23FFF000)` —
  fail → `[U-Boot] Authenticate KERNEL failed!` and **halt** (0x23E0FD00, endless loop); AESON → AESDMA decrypt of the data in
  place (`[U-Boot] Decrypt Kernel`, `Decrypt AES done`). The partition argument is not used.
- **`sigauth`** 0x23E1AFEC (= `drivers/mstar/aesdma/cmd_sigauth.c`): `sigauth <image> <key|0> [--aes]`; key 0 → `[U-Boot]
  SIGMASTAR Key` (built-in), a key with the `IPLN` tag (0x4E4C5049 at +4) → `CUST RSA Key(%08x)` at key + u16@+8 − u16@+0x10,
  other → `OTP RSA Key`; size from an `RTK_` image (word 8 = 0x5F4B5452, size word 9, payload +0x30 for AES) or a uImage, else
  env `rootfs_size` − 0x100; `--aes` rounds the size up to 16; RSA check `runAuthenticate` (fail → `Authenticate failed!` +
  halt); with `--aes`: `runDecrypt`, then CRC32 (`image_check_dcrc`, uImage), additive checksum == word 10 (`RTK_`), or additive
  checksum of size − 16 bytes == word 3 (other) — mismatch → `… Checksum error!` + halt, else `Checksum ok!`.
- **`test_sig`** 0x23E1AF24 (= oi): `test_sig img_addr img_sz key_addr key_len sig_addr sig_len` (hex, ≥ 6) →
  `runAuthenticate2` → `runAuthenticate2 passed!` / `failed!`.

**Engine source** (0x23E19968 … 0x23E1AF24) [R]: oi `drivers/mstar/aesdma/drvAESDMA.c` + `mercury6/halAESDMA.c` (built
because `CONFIG_MS_AESDMA`; `cmd_sigauth.c` for `sigauth` / `test_sig`; `cmd_verifyaes.c` is not built — none of its commands
is in the command table). `MDrv_AESDMA_Run` / `_SHA_Run` / `_RSA_Run`, `do_test_sig2`, `do_sig_auth` pair with the oi
objects; every HAL function was read in the decompile and matches the mercury6 HAL
statement by statement — **same registers, same bits, same order**. The one difference from oi: the debug line
`MDrv_AESDMA_Run 2222222` printed by the first statement of every `MDrv_AESDMA_Run` call (the name confirmed by the function's
own `__FUNCTION__` strings). `CONFIG_SRAM_DUMMY_ACCESS_RSA` is on: `HAL_RSA_DummyRead(reg, n)` 0x23E19968 (compiled at -O0)
reads the same register n + 5 times after most RSA / SHA / key writes. `CONFIG_MERCURY6P` is off: `MDrv_RSA_Run` is the
classic-RSA branch, not the PKA / CLUE one.

**Registers — one RIU bank, 0x1122** (byte base 0x1F224400; source `AESDMA_BASE_ADDR` = `SHARNG_BASE_ADDR` = `RSA_BASE_ADDR`
= 0x112200) [R]:

| regs | block | use (bits as the HAL writes them) |
|---|---|---|
| 08 | SHA ctrl | [0] fire (pulse), [6] clear (0 → 1 after done), [7] reset (pulse), [9] 1 = SHA-256 / 0 = SHA-1, [11] bypass scatter-gather |
| 0A–0B | SHA | source address [27:0] (top nibble masked "to avoid MIU/IMI wrap") |
| 0C–0D | SHA | message length, bytes |
| 0F | SHA status | [0] ready |
| 10–1F | SHA | digest out, 16 × u16 (reversed by software afterwards) |
| 20 | RSA ind32 | [0] ind32 start (open / close an indirect access window) |
| 21 | RSA ind32 ctrl | [1] direction 1 write / 0 read, [2] address auto-increment, [3] auto-start after the high half |
| 22 | RSA ind32 addr | operand base: E 0x00, N 0x40, A (signature) 0x80, Z (result) 0xC0 (+ word index) |
| 23 / 24 | RSA ind32 data in | high / low half of one 32-bit word, each byte-swapped |
| 25 / 26 | RSA ind32 data out | high / low half, byte-swapped back by `HAL_RSA_FileOut` |
| 27 | RSA | [0] exponentiation start, [1] interrupt clear |
| 28 | RSA ctrl | [0] reset (pulse), [1] hardware key, [2] public key, [13:8] key length (written 0x3F = 64 words) |
| 29 | RSA status | [0] busy, [1] done |
| 50 | AESDMA ctrl0 | [0] file start (pulse), [7] software reset, [8] file-out enable |
| 51 | AESDMA ctrl1 | [8] AES enable, [9] decrypt, [13:12] chain 0 ECB / 1 CTR / 2 CBC |
| 52–53 | AESDMA | file-in address [27:0] |
| 54–55 | AESDMA | length (XIU length), bytes |
| 56–57 / 58–59 | AESDMA | file-out start / end address (start + size − 1) [27:0] |
| 5D | — | cleared by `HAL_SHA_Reset` (the source does the same) |
| 60–67 | AESDMA | 128-bit cipher key, written 0x67 down to 0x60, each u16 byte-swapped |
| 68–6F | AESDMA | 128-bit IV, written 0x6F down to 0x68, byte-swapped |
| 79 | AESDMA | [6:5] key select: 0 cipher key (regs 60–67), 1…4 OTP key 1…4 (`AESDMA_OTP_KEY_MAX` 4; the check accepts 0…4) |
| 7F | AESDMA status | [0] DMA done |

**HAL functions as built** (names from the source by statement match) [R]: `DummyRead` 0x23E19968; AESDMA — `SetXIULength`
0x23E199F0, `CheckAesKey` 0x23E19A0C, `UseAesKey` 0x23E19A1C, `CipherEncrypt` 0x23E19A40, `CipherDecrypt` 0x23E19A5C, `Enable`
0x23E19A74, `FileOutEnable` 0x23E19A8C, `SetFileinAddr` 0x23E19AB4, `SetFileoutAddr` 0x23E19AD0, `SetCipherKey` 0x23E19B0C,
`SetIV` 0x23E19B5C, `SetChainModeECB/CTR/CBC` 0x23E19B90 / 0x23E19BAC / 0x23E19BCC, `Reset` 0x23E19BEC (sw-reset pulse, then 50,
79, 51–59 and the key / IV registers to 0), `Start` 0x23E19C80, `GetStatus` 0x23E19CB4; RSA — `Reset` 0x23E19CC4, `Ind32Ctrl`
0x23E19D34, `LoadSignInverse_2byte` 0x23E19D9C, `LoadKeyE` (fixed e = 65537 as words 0x0001 / 0x0001) 0x23E19F34,
`LoadKeyEInverse` 0x23E19FEC, `LoadKeyNInverse` 0x23E1A074, `SetKeyLength` 0x23E1A20C, `SetKeyType` 0x23E1A230,
`ExponetialStart` 0x23E1A294, `GetStatus` 0x23E1A2AC, `FileOutStart/End` 0x23E1A2BC / 0x23E1A2DC, `SetFileOutAddr` 0x23E1A300,
`FileOut` 0x23E1A320; SHA — `Reset` 0x23E1A3D8, `SetAddress` 0x23E1A40C, `SetLength` 0x23E1A454, `SelMode` 0x23E1A490,
`GetStatus` 0x23E1A4C0, `Clear` 0x23E1A4D0, `Start` 0x23E1A4F8, `Out` 0x23E1A528. Source HAL functions with no body in the
image (not referenced, dropped at link): `HAL_AESDMA_Disable`, `_DisableXIUSelectCA9`, `_WB2DMADisable`, `_ShaFromInput/Output`,
`HAL_RSA_ClearInt`, `_LoadSignInverse` (fixed 64 words), `_LoadKeyN` (non-inverse) [R: absent from the function list of the
range].

**Driver level** (= oi `drvAESDMA.c`) [R]:
- `MDrv_AESDMA_Run(cfg)` 0x23E1A5B8, `cfg` = {+0 src, +4 size, +8 dst, +0xC key type, +0x10 key ptr, +0x14 set-IV flag, +0x18
  decrypt flag, +0x1C IV ptr, +0x20 chain 0 ECB / 1 CTR / 2 CBC}. Size 0 → `ERROR!Size is zero!`; odd key pointer → `have to
  align to half-word`; key type > 4 → `ERROR!eKeyType(%d)`. Sequence: Reset; file-in = src; length = size; D-cache invalidate
  src … dst+size; file-out = dst … dst+size−1; key select; cipher key loaded only for type 0; decrypt / encrypt; IV if set; AES
  enable; chain mode — ECB also rounds the length up to 16, **CTR forces encrypt** (hardware CTR needs the encrypt bit), another
  value → return without starting; file-out enable; start pulse; poll `7F[0]` up to 1000 ms (timeout prints `timeout!` and
  continues); invalidate again; Reset. In-place operation (src = dst) is the normal use.
- `MDrv_SHA_Run(src, size, mode, out)` 0x23E1A858: src and out must be 16-byte aligned (`Must align to 16-bytes!`); Reset;
  D-cache flush src … src+size + `Chip_Flush_Memory` (0x23E006A8); address, length, mode (0 SHA-1, 1 SHA-256, else return);
  fire; poll `0F[0]` ≤ 1000 ms; read 16 u16, **reverse the 32 bytes** (`Reverse_Data` 0x23E1AEE0); Clear; Reset.
- `MDrv_RSA_Run(cfg)` 0x23E1A9B0, `cfg` = {+0 sig, +4 N, +8 E, +0xC out (each 4-byte aligned if non-NULL), +0x10 hw-key flag,
  +0x14 public-key flag, +0x18 key length (unused in this branch), +0x1C sig length bytes, +0x20 out length bytes}: Reset; key
  length 0x3F; key type; ind32 write mode; load the signature as `sig_len/4` words, last word first, into operand A; with a
  software key: N (64 words, reversed) when given, E (64 words, reversed) when given, else e = 65537; start; poll `29[1]` ≤
  **3000 ms**; ind32 read mode; read `out_len/4` words (64 when out_len = 0) from Z + i, stored big-endian (word i →
  out[n−1−i]); close; Reset.
- `runDecrypt(addr, size, keysel, key)` 0x23E1AC5C: in-place **AES-ECB decrypt** of addr…addr+size with key select `keysel`
  (the key pointer is used only for keysel 0); used by `sigauth --aes`.
- `runAuthenticate2(img, img_size, key, key_size, sig, sig_size)` 0x23E1ADA4: SHA-256 of the image; RSA with the public-key
  flag, the **hardware (OTP) key when `key` = 0**, else N = `key` (64 words) and e = 65537 (the E pointer is never set); output
  length 32 → only the **lowest 8 words (256 bits) of the 2048-bit result** are read and compared with the digest
  (`Compare_data` 0x23E1ACD4). Mismatch → `Digest check failed.` + `Dump_data` (0x23E1AD24, `%p: ` + `%02x ` rows) of both
  buffers, return FALSE. The PKCS#1 padding bytes above the digest are not checked [R code; I: a signature is accepted when its
  low 32 bytes after exponentiation equal the digest].
- `runAuthenticate(img, size, key)` 0x23E1AE98 = `runAuthenticate2(img, size, key, 256, img + size, 256)` — the 256-byte
  signature sits right after the image.
- `_Hal_GetMsTime(t0, ms)` 0x23E1AEB8 = `get_timer(0) − t0 > ms`.

Keys: key selects 1 / 2 are the OTP AES keys used by `aes … EFUSE|HW`; the RSA hardware key is the OTP RSA N / E (`otpctrl`
ids 0 / 1, `ipl.md` §7.1) [R; I that the RSA "hardware key" bit reads those OTP fields]. Which OTP fields `1122.28[1]` and
`1122.79[6:5]` select is done by the hardware and is not readable in U-Boot; it is not known.

### 9.7 `mssdmmc` — FCIE / SDIO IP-verification tests (0x23E10094 … 0x23E17200) [R = oi `common/cmd_sdmmc_ipv.c` + `drivers/mstar/sdmmc/ms_sdmmc_verify.c`]

`mssdmmc <op> <slot> [args]` — SigmaStar's FCIE / SDIO IP verification menu: `timer <s>`, `cdz`, `init`, `wibus`, `hibus`,
`cifd`, `dma`, `bcifd`, `bdma`, `adma`, `badma` (FCIE5), `at1d` / `at1h` / `at4d` / `at4h` (auto tests: CIFD + DMA (+ ADMA)
read / write in 1 / 4-bit default / high speed, then burn tests 30000…30050), `pat <slot> <pattern>`, `clk <slot> <ref clk>`,
`btm <slot> <bus timing>`, `pad <slot> <ip> <port> <pad>`, `while` (endless loop for a TV tool), `pwrsave <slot>`, `sdiointr
<slot>` (the help text spells it `sdioitr`, the parser wants `sdiointr` — both in the source). Handler `do_sdmmc_test`
0x23E1015C; the test layer is built for `D_PROJECT__M6`, FCIE version 5 (the ADMA arms are compiled). `_PRT_MSG`,
`_PRT_VALUE`, `_PRT_LINE`, `_DUMP_MEM_ARR`, `IPV_SDMMC_DMA_RW`, `_CIFD_RW`, `_ADMA_RW`, `_BurnRW`, `_PowerSavingModeVerify`
pair SAME; `IPV_SDMMC_TimerTest` equals the source line for line (separator, `(SDMMC TimerTest)`, `-->`, up to min(sec, 11) ×
(`(@)` + 1000 ms delay), separator); the unpaired functions carry exactly the source's strings and calls:

| addr | function | does |
|---|---|---|
| 0x23E10094 | `do_io_std_test(slot)` | sectors 6…15: CIFD R/W, DMA R/W (2 sectors), ADMA R/W each |
| 0x23E100E4 | `do_io_std_burntest(slot)` | sector 6 CIFD / DMA / ADMA once, then `BurnRW` CIFD, DMA, ADMA over sectors 30000…30049 |
| 0x23E163EC / 0x23E164E4 | `_PRT_MSG` (pass / fail halves) | `[sdmmc_<slot>]  <test> ........ (PASS)` or `(FAIL)= <err>` |
| 0x23E16450 / 0x23E164BC / 0x23E164D0 | `_PRT_VALUE`, `_PRT_LINE`, `_PRT_ENTER` | value line, `====` separator, CR LF |
| 0x23E16570 | `_DUMP_MEM_ARR` | hex dump `0x%02X`, 16 per line, between `----` rules |
| 0x23E16608 | `_MEM_COMPARE` | 0 equal / 1 differ |
| 0x23E16654 | `_SET_MEM_PATTERN(buf, n, p)` | p 0: value +1 every 16 bytes; 1: 0, 1, 2…; 2: 0xFF, 0xFE…; 3: alternating 256-byte up / down ramps; 4: FF / 00 alternating; 5: 55 / AA alternating — **p 4 and 5 write n + 1 bytes** (loop `<= n`, the same in the source) |
| 0x23E167CC | `IPV_SDMMC_DMA_RW` (body; 0x23E16C8C is its ≤ 8-sector guard) | pattern → write `n` sectors by DMA, clear, read back, compare, dump on mismatch |
| 0x23E16908 | `IPV_SDMMC_TimerTest` | above |
| 0x23E16988 / 69DC / 6A2C / 6A7C | `CardDetect`, `Init`, `SetWideBus`, `SetHighBus` | call the driver step, print PASS / FAIL |
| 0x23E16ACC / 6B1C | `SetClock(slot, ref, down)`, `SetBusTiming` | driver step, print the resulting value |
| 0x23E16B64 | `IPV_SDMMC_CIFD_RW` | one sector through the CIFD (CPU FIFO) path |
| 0x23E16C98 | `IPV_SDMMC_ADMA_RW` | two 1-sector ADMA descriptors (buffers +0, +0x200) |
| 0x23E16E2C | `IPV_SDMMC_TestPattern` | set the global pattern (0x23EA5048) |
| 0x23E16E74 | `IPV_SDMMC_BurnRW(slot, type 0 CIFD / 1 DMA / 2 ADMA, start, end)` | start = end = 0 → 30000…70000; per sector R/W/compare (stop at the first error); every 120 sectors clock down-level +1 (0…3, wraps), every 20 sectors pattern +1 (0…5, wraps) |
| 0x23E1704C | `IPV_SDMMC_SetPAD(slot, ip, port, pad)` | `SDMMC_SetPAD` + `SwitchPAD` |
| 0x23E170A8 | `IPV_SDMMC_PowerSavingModeVerify(slot)` | programs the FCIE power-save command FIFO (bank 2 of the slot, regs 0x00–0x1F: 16 entries {value, class BAT\|RST, op WREG/WINT/STOP, bank, reg} — clear enable, clear interrupts, clear SD mode / ctl, reset pulse, SD_MOD 0x21, `reg_sd_en`, command IDLE / STOP content, size 0x500, interrupt SD_CMD_END, start, wait interrupt, stop), then reg 0x35 of bank 0: power-save reset pulse, mode on, RIU-trigger + interrupt enable; polls bit 15 up to 10⁹ × 1 ms and prints `eMMC_PWRSAVE_CTL: 0x%04X` |

Global results `gu16Ret1..3` at 0x23EA504A / 4C / 4E. Hardware touched [R]: DMA write buffer **0x20006000**, read buffer
**0x20008000**, ADMA descriptor table **0x2000A000** (physical DRAM, the `D_PROJECT__M6` arm of `ms_sdmmc_verify.h`; no
reservation — whatever lives there is overwritten [I]); FCIE register banks from the driver's bank table at 0x23E77424:
**1413 / 1414 / 1415** and **1410 / 1411 / 1412** (the M6 arm of `hal_card_regs.h`: FCIE1 = 1413–1415, FCIE2 = 1410–1412). The
R/W tests **write to the card**: sectors 6…15 (`at*`), 30000…30049 (the burn inside `at*`), or the caller's range [R sector
numbers; I: destructive to what the card holds there]. The tests sit on the SDMMC driver (`ms_sdmmc_drv.c`,
`hal_sdmmc_v5.c`) that the used `mmc` / `fatload` path runs; only the IPV layer is unused.

## 10. FAT writing: `fatwrite` and `fatrm` do not update the FAT [R]
<!-- ev:uboot.10 -->

Symptom: `fatwrite` reports `N bytes written`, the data
clusters and the directory entry reach the card, the FAT does not; every new file starts at cluster 3 and overwrites the
previous one; a 16 MiB file came back with a broken chain.

Cause: the vendor `fat_write.c` is the newer SigmaStar revision with a dirty-flag FAT cache (`fsdata.fat_dirty` at +0x0E,
`flush_dirty_fat_buffer`), but its `set_fatent_value` (**0x23E5DDF4**) never sets the flag — it ends with the bare `strh r6,
[r3, r4]` / `str` of the entry into `fatbuf` [R disassembly]. The only stores to +0x0E in the FAT code are the two clears
(0x23E5E630 at mount, 0x23E5DF70 after a flush) [R: all `strb …, #14]` in 0x23E5D000–0x23E62000]. The oi tree has the missing
line `mydata->fat_dirty = 1;` at the end of `set_fatent_value` (fat_write.c:633, indented differently from the rest — a later
fix [I]). Consequences as built:
- At the end of `file_fat_write` (function 0x23E60360, flush call at 0x23E60684) and of `fat_unlink` (function 0x23E61328,
  reached by `fatrm`), `flush_dirty_fat_buffer` (0x23E5DF00) returns 0 without writing because the flag is 0 → the current FAT
  window is never written.
- Only a window **eviction** writes FAT: when `get_fatent_value` (0x23E5DFA0) or `set_fatent_value` moves to another window
  (6 sectors: 768 FAT32 / 1536 FAT16 / 2048 FAT12 entries), the old window is written by `flush_fat_buffer` (0x23E5DDC4) — the
  first FAT copy only, no second copy [R: one `disk_write`].
- A small file's cluster chain therefore never reaches the card, and the next `find_empty_cluster` reads the unchanged FAT and
  picks cluster 3 again; a large file gets its earlier windows written by eviction and loses the window holding the end of the
  chain (EOC) → broken chain. Both match the symptom.
- `fatrm` has the same defect: the freed chain is not written either. There is no `fatmkdir` command in this build.
Overwriting an existing file of the same size is not affected either: `fatwrite` of 16 MiB over a 16 MiB file a PC had
written (2048 contiguous 8 KiB clusters near the card's end, the card's lowest free cluster 3) put the data into that
file's own clusters, and the FAT stayed consistent (`fsck.fat -n` found nothing new; one card, 2026-11-13) [R]. Which
code path keeps the chain is not read [I].
Raw block writes (`mmc write`) are not affected: a whole-NOR copy by `sf read` + `mmc write` to whole-card LBAs, read back
with `mmc read` + `cmp.b` + `crc32`, worked (§«How it was taken») [R].

## 11. What is not known — index

Each item is stated in its section; listed here together:
- whether the HDMI logo appears on a monitor, and the MOP / GOP state U-Boot leaves (§5.6, §7; camera check
  `docs/firmware/camera-checks.md` §1.8 K37);
- content and last line of `H161X.bin` (§4.1, §4.2);
- who clears `ota_upgrade_status` after `ufu` (§4.4);
- a per-instruction comparison of the string-less functions, and whether the oi tree builds with the SDK gcc 9.1 (§1.3, §1.4);
- `srcfg` MCLK selects 12…14 on M6 (§9.4);
- meaning of the Albany PHY tuning bytes and the `JULIAN_01xx` bits (§9.1);
- which OTP fields the RSA hardware key and the AES OTP key selects read (§9.6);
- the `VIP_ICC_Ymode` rows in bank 1125 of `PQ.bin` (§6.3).
