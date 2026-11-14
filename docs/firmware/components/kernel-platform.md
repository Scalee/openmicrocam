# kernel-platform — the BSP core of the vendor kernel 4.9.227 (Mercury6)
<!-- ev:kernel-platform -->

The camera's Linux kernel (ARMv7 Thumb-2 on two Cortex-A53 cores in AArch32)
carries the SigmaStar board-support package of the Mercury6 (SSC019A) SoC built in. This document covers its core: the
mach-sstar platform (static I/O map, chip ops, SMP start, the A53 processor entry), the interrupt controllers and timer,
clocks, cpufreq and the voltage rails, GPIO and padmux with the board wiring, the UARTs, the hardware I2C driver, the
SPI-NOR / BDMA / MXP flash stack, MSYS and DMEM, the MMA heap and the physical memory map, the MIU driver and arbiter, the
kernel side of CamOs, the device tree, the kernel configuration and the vendor patches to generic kernel files.

It owns these facts shared by several components: RIU addressing; the clock tree after boot; interrupt numbers and the
device tree as a whole; padmux, GPIO and board wiring; the physical memory map; the MIU client → group table, the arbiter
registers and the MIU protect hardware; the chip id and revision; the kernel side of CamOs. Elsewhere: the peripheral
drivers (USB PHYs, EHCI glue, GIC save/restore, PM / suspend, voltage-ctrl internals, PWM, watchdog, RTC, notify, EMAC,
SATA, IVE, AES/SHA/RSA, SAR ADC, IR, `kdrv_sdmmc.ko`) — `kernel-drivers.md`; the partition map and the vendor file
inventory — `firmware-image.md`; early clocks, DRAM / MIU init and OTP before the kernel — `ipl.md`; the boot-time `riu_w`
write list — `rootfs-customer.md` §5; the mmap.ini grammar — `config_tool.md` §2.1; mi_sys's use of MIU protect and the MMA heaps —
`mi_sys-2-interfaces.md` §7.7, §7.2; the DT properties mhal reads per node — `mhal-core.md` §7; user-space `libcam_os_wrapper` —
`mi-user-abi.md` §7; the fwfs daemon — `misc-firmwarefs.md` §4.

Tags: **[R]** read from the Image, kallsyms, a decompile, a disassembly, the boot log or a data file; **[S]** stated by a
source (a vendor source tree, header or comment); **[I]** inference. A register meaning tagged [S] is what a vendor header
or comment says, not a measurement.

## 1. The image, the reference trees and how "same as source" is meant

### 1.1 Inputs [R]
<!-- ev:kernel-platform.1.1 -->
The vendor kernel image (4.9.227 `#44`) with its kallsyms, the built-in DTB (decompiled with `dtc`), the kernel part of
the stock boot log (UART), and the vendor `.ko` files in `/config/modules/4.9.227/`.

No public tree is the M6 4.9.227 kernel itself. Two OpenIPC SigmaStar branches and vanilla kernels
are the references [R]:

| tree | kernel | what it holds for this kernel |
|---|---|---|
| OpenIPC/linux `sigmastar-infinity6c` @ d17f67f (2025-09-01), "i6c" | 5.10.61 SDK | the Mercury6 platform files (`mach-sstar/mercury6`, `drivers/sstar/*/mercury6`, M6 `proc-v7.S` / PMU entries) |
| OpenIPC/linux `sigmastar-infinity6e` @ 97b7fc4 (2026-09-27), "i6e" | 4.9.84 SDK | 4.9-era Linux glue of the same drivers; older revisions of i2c, pwm, watchdog, mma_heap |
| vanilla 4.9.227 / 4.9.84 (kernel.org) | | what is stock; i6e minus vanilla 4.9.84 = the vendor core patches |

"Same as source" below means the binary's functions, tables or ioctl set match that file — not a byte-identical build.

### 1.2 Method [R]
<!-- ev:kernel-platform.1.2 -->
- Symbol classes: every kallsyms text symbol classed as vanilla ("V"), vendor with
  source ("S": which tree, which file; Mercury6 files preferred), vendor without source ("B") or not matched to any
  definition ("N": asm labels, syscall wrappers, macro-generated names and the like); counts V 17815, S 1591, B 24, N 1477.
  Four "S" rows are wrong: `aes_encrypt`, `aes_decrypt` (`crypto/aes_generic.c`), `hash`
  and `remove_mapping` (`mm/vmscan.c`) are stock kernel functions, not `drivers/sstar/crypto/cryptodev/…` ones; and
  `gic_dist_save/restore` are classed "V" only because GICv2 has functions of those names (§4.1).
- Missing functions: functions of each matched source file the binary lacks (inlined statics show up too, so an
  absence is a hint, not proof). Large lacks are config-dependent code (msys perf tests, PROC / SYSP ioctls) or inlined
  statics (IR key helpers).
- String match: source string literals found in the image. Presence is low
  everywhere (13–75 %) because both trees carry debug prints that are compiled out; it does not separate the trees.
- All 1614 vendor functions of the raw Image were read in the disassembly with kallsyms applied and literal pools resolved
  to symbols / strings.
- Module imports: every kernel symbol the vendor modules import, and who provides it.
- The image's `gpio_table` was compared with the Mercury6 source.

**Body check**: two fingerprints per function,
compared item by item.
- Binary: Thumb-2 linear sweep with Capstone of every class S / B kallsyms function (1613 functions) over [address, next symbol); literal-pool words are skipped once an LDR-literal has
  pointed at them. Collected: direct calls and tail branches (→ kallsyms names), pool words that are symbol addresses
  (function pointers, data), pool words that point to C strings in `.rodata`, immediates.
- Source: the body of the same-named function in i6c and in i6e (Universal Ctags line ranges; Mercury6
  files preferred, else the binary's driver directory, else every definition, best kept). Identifiers and string literals
  of the body; macros, header inline functions (tree headers + vanilla 4.9.227 headers) and same-file functions expanded
  transitively, so a binary item counts as explained when any expansion reaches it. Tokens under `#if` inside the body
  only explain, they are never required.
- Findings per function and tree: **bx** binary calls / function pointers / external data no source token explains;
  **bs** binary strings no source string explains; **sx** functions the body calls unconditionally that the binary neither
  calls nor references (after allowing inlining: callee ≤ 48 B or all its callees present); **ss** source strings missing in
  the binary, split by the macro they are passed to (`soft` = a debug macro with an empty definition or `pr_debug`, i.e.
  compiled out; `hard` = printk family / string functions).
- Noise removed by rule: static data reached through section anchors (GCC addresses all statics of a file from one
  anchor), names made by token pasting in the ARM headers (`v7_*` cache ops, `_set_bit` family), compiler runtime
  (`__aeabi_*`, `memcpy` / `memset` for struct copies), lockdep `__key.*`, `__func__.*`.
- Not visible to the method [I]: changed constants, register offsets, conditions and arithmetic with the same calls and
  strings. Immediates are collected but not compared — macro-built register addresses make source constants incomparable
  without preprocessing the vendor tree under a config that does not exist publicly.

Coverage [R]: **1589 name-matched functions** checked (class S; per-tree scores, raw findings, per-function evidence; rerun). 4 have no C definition (asm labels `pen`, `start`, `rtc_iso` of `smp_head.S` / `sram.S`, and
`_load_policy.part.0.constprop.0`, whose C body ctags misses) and were checked by hand (§3.4, §12). **1346: no finding**
against at least one tree. **239 with a finding**: 99 only miss strings of compiled-out debug macros (`soft`, accepted); the
other **140** were each read in decompile and source. Tool artefacts found that way (inlining the rule missed, ICF thunks,
constprop clones, GCC builtin rewrites `strstr(",")`→`strchr`, `sprintf(d,"lit")`→`strcpy`, `WARN_ON(__FILE__)` strings of the
4.9 headers, dead branches the compiler removed, compiled-out debug macros) and 4.9-vs-5.10 API glue count as "same"; real
differences are listed as **DIFF** in §2.3.

## 2. Vendor code in the image

### 2.1 Overview [R]
- Of ≈1600 vendor functions in the image, all but ≈25 exist by name in the two OpenIPC trees. Hardware
  tables checked byte-wise equal the i6c Mercury6 files: GPIO (163 pads, all registers), padmux (2257 + 64 + 448 entries),
  SPI-NOR chip table (70 entries, this camera's chip included), static I/O map, msys / SAR ioctl sets, IR decoder setup.
- Without source: the 11 functions of the newer HW I2C code (§8), the USB3 PHY front-end (unused), the GICv3
  save / restore patch (suspend only), a Goodix gt911 variant (out of scope: hardware not on this camera); the full class-B
  list is §2.4.
- Every one of the 201 BSP functions the vendor modules mhal, mi_*, fbdev, sdmmc and udc call has source (§16.2).
- Where the trees disagree, the binary sits **between** them: CamOs has functions only in i6e and functions only in i6c
  (§13.2); msys and mma_heap mix both; serial is an i6e base with additions; watchdog and pwm carry the i6e names; gpio,
  irqchip and clk are the i6c ones; I2C is a CamOs-based revision under i6e names (§8); RTC is a revision of its own
  (spinlock, extra sysfs; `kernel-drivers.md` §2.4.4).

### 2.2 Per-driver table [R]
Funcs = vendor functions in kallsyms for that group / with a name match in i6c / in i6e. "Closest source" is the tree file
set whose functions the binary matches.

| group | funcs (i6c / i6e) | closest source | binary-only / differences | consumers on this camera |
|---|---|---|---|---|
| DTB | — | the built-in DTB (decompiled) | — | every probe; mhal reads its nodes (§14) |
| mach-sstar/mercury6 (SMP, PM, MIU arbiter, chip) | 134 (122 / 118) | i6c `arch/arm/mach-sstar/{ms_chip.c, mercury6/*}`, `msys/prof.c`; the MIU arbiter is built into mach-sstar with i6c `mdrv_sysfs_arb.c` logic and i6e `miu_bw.c` names (§12.1) | the 7 "no-name" symbols are asm labels of `mercury6/sram.S`, `smp_head.S`; the arbiter's client table, `_get_client_burst` and default policy differ from i6c (§12) | boot; `Chip_*` exports → mi_*, udc, sdmmc, mhal (§16.2) |
| intc (main / pm / gpi) + timer | 36 (36 / 35) | i6c `drivers/sstar/irqchip/mercury6/{irqchip,irq-pmsleep,irq-gpi}.c`; timer = stock `arm_arch_timer` (6 MHz) | GICv3 save / restore patch in `irq-gic-v3.c` (PM only, §4.1) | all IRQs |
| clocks, cpufreq, voltage-ctrl | 61 (59 / 58) | i6c `clk/{ms_composite_clk.c, mercury6/ms_complex_clk.c}`, `cpufreq/mercury6/cpufreq.c`, `voltage/{voltage_ctrl.c, mercury6/voltage_request_init.c}`, `cam_clkgen/` | voltage-ctrl: `cpu_power` and `core_power` probe, `dla_power` fails (§5.4) | everything; `CamClk*` → mhal; `get/set_core_voltage` → mhal |
| UART `ms_uart` | 52 (49 / 52) | `drivers/sstar/serial/ms_uart.c` (both trees) | URDMA present but off; RS485 absent (§7) | console ttyS0 (5 UARTs probe) |
| GPIO + padmux + camdriver_gpio | 115 (113 / 87) | i6c `gpio/{mdrv_gpio.c, mdrv_gpio_io.c, mercury6/mhal_gpio.c, mercury6/mhal_pinmux.c, ms_gpioi2c.c, mdrv_sw_iic.c}`, `padmux/mdrv_padmux.c` | — (tables identical, §6) | kjd_lvgl `gpio46` via sysfs; `MDrv_GPIO_*`, `mdrv_padmux_*` → sdmmc, mhal; `camdriver_gpio_*` → mhal |
| I2C (HW + gpio-i2c) | 114 (4 / 99) | i6e `drivers/sstar/i2c/{ms_iic.c, infinity6e/mhal_iic.c}` names + i6c `cam_drv_i2c.c` | the whole driver is a CamOs-based revision; 11 functions without source (§8) | sensor I2C via mhal `CamI2c*`; `/dev/i2c-*` |
| SPI-NOR (FSP / serflash / BDMA), MXP parser | 176 (176 / 174) | `drivers/sstar/flash_isp/{drvSERFLASH.c, mercury6/halSERFLASH.c, mtd_serflash.c, part_mxp.c, mxp_flash.c, drvDeviceInfo.c}`, `bdma/mercury6/hal_bdma.c` | `serflash_erase` is a revision mix (§9) | all MTD partitions; fwfs on mtd5 |
| MSYS / MMA / DMEM | 79 (54 / 67) | `msys/{ms_msys.c, ms_msys_dma_wrapper.c, mercury6/platform_msys.c}`, `mma_heap/mdrv_mma_heap.c` (i6e struct, §11.3) + `region_substract.c` (i6e), `cpu/memory.c` | mix of i6c and i6e revisions (§10, §11) | `/dev/msys` → riu_w only; exports → mi_sys, mhal |
| MIU / MMU / movdma | 70 (68 / 59) | i6c `miu/{mdrv_miu.c, mercury6/mhal_miu.c}`, `movedma/hal_movedma.c` | — | `MDrv_MIU_*`, `MDrv_MMU_*` → mi_sys; `/sys/devices/system/miu/miu0/dram_info` (no reader found) |
| SAR ADC | 14 (14 / 14) | `sar/mdrv_sar.c` | — (ioctl set identical) | kjd_lvgl buttons `/dev/sar` |
| IR receiver | 11 (11 / 11) | `ir/ms_ir.c`; keymap name `rc-mstar-dtv` | the keymap table is a vendor 21-entry table, not `ir/include/IR_MSTAR_DTV.h` (`kernel-drivers.md` §3.2.2) | kjd_lvgl `/dev/input/event0` |
| PWM, watchdog, RTC, notify / chip | 114 | pwm: i6e `pwm/mdrv_pwm.c` names over the logic of i6c `pwm/mercury6/mhal_pwm.{c,h}`; wdt: i6e `watchdog/mdrv_wdt.c` (`infinity_wdt_*`); rtc: i6c `rtc/{drv_rtcpwc.c, mercury6/hal_rtcpwc.c}` names, own revision; notify: `notify/ms_notify.c` | `kernel-drivers.md` §2.4, §2.5, §6.1, §6.2 | RTC: kernel hctosys at boot, kjd_lvgl `hwclock -w`, `pm_power_off` (busybox `poweroff`); PWM, watchdog, notify: none |
| USB PHY | 8 + 9 | `sstar_usb_phy_*` = i6c `usbphy/infinity6c/usb_phy.c` names; front-end `usb_phy_probe/init/…`, `tx_voltage_*` | front-end without source | none: the `dwc3` node is enabled but the kernel has no dwc3 driver; the device port's UTMI is set by udc-msb250x itself (`kernel-drivers.md` §4.1.1, `usb-gadget.md` §2.4) |
| CamOs (kernel OS shim) | 226 (213 / 184) | `cam_os_wrapper/src/cam_os_wrapper.c` (one revision with both trees' additions, §13), `cam_sysfs/`, `cam_fs_wrapper/`, `cam_proc_wrapper/` (i6c), `cam_drv_buffer/`, `cam_drv_poll/` | `CamOsMemPhysToVirt` without source; delayed-work DIFF (§13.3) | mhal (113 of its 131 BSP imports), fbdev / mi_common / mi_sys a few |
| unused (EMAC, SATA, IVE, AES/SHA/RSA, gt911) | | | | `kernel-drivers.md` §5.2, §7, §8; DT nodes §14 |


### 2.3 Body differences against the source trees [R decompile vs source]
Driver directories with no DIFF and no revision mix ("same as source" by the body check, §1.2): cam_clkgen, cam_drv_buffer,
cam_drv_poll, cam_fs_wrapper, cam_sysfs (glue only), bdma, clk, cpu, cpufreq, flash_isp (bar `serflash_erase`), gpio
(including the `camdriver_gpio_*` family, whose prints are a commented-out macro), ir, irqchip, ive, netphy, notify, padmux,
sar, movedma (the "missing" MSPI-channel error is a dead branch: the field is 1 bit wide). The rest, binary against the
closer tree:

| driver | function(s) | what differs in the binary | used here? |
|---|---|---|---|
| mach-sstar `mercury6/pm.c` | `mstar_rtc_magic_set`, `mstar_suspend_ready`, `mstar_suspend_wake`, `mstar_pm_init` | **DIFF**: the RTC "magic" write also saves 6 PM-RTC registers (bank 0x0034 regs 0x00, 0x01, 0x0F, 0x20, 0x21, 0x22; phys 0x1F006800 + 4·reg) into a new static `rtc_suspend_info[6]` after the ISO handshake when magic = 0xBABE, and writes them back before the handshake when magic = 0 (wake); no write to reg 0x35 = 0x1F0068D4 ("enable autogen" in the source); first ISO wait 40 tries (source 20); extra `udelay(2000)` after the ISO end. pm.c statics follow `rtc_suspend_info` (+0x0C `pStr_info`, +0x10 `suspend_status`, +0x14 IMI vbase, +0x18 IMI entry) | no (no suspend) |
| mach-sstar asm `sram.S`, `smp_head.S` | `pen`, `start`, `rtc_iso`, `sram_suspend_imi`, `mercury6_secondary_startup` | §3.4 | secondary start: yes |
| miu `mdrv_sysfs_arb.c`, `miu_bw`, `mhal_miu.c` | `client_nolimit_show`, `dump_show`, `create_miu_arb_node`, `miu_arb_resume`, `miu_client_id_to_name`, `HAL_MIU_ClientIdToName` | **DIFF**: binary-only helper `_get_client_burst`; an M6 client-name table; a `Chip_Get_Package_Type` call whose result is unused; a default policy that differs from i6c (§12) | sysfs only |
| cam_os_wrapper | `CamOsVersion` | **DIFF**: returns `"v1.0.42"` (i6e tree "v1.0.35", i6c "v2.0.43") | — |
| cam_os_wrapper | `_CmaOsWorkQurueDelayCb` (+ `CamOsWorkQueueAdd`) | **DIFF**: delayed work runs inside the timer callback (§13.3) | **yes (mi_sys)** |
| cam_os_wrapper | `CamOsGetTimeInMs` | glue: `getrawmonotonic64` (4.9) for `ktime_get_raw_ts64` | — |
| crypto `mdrv_aes.c` | `infinity_aes_crypt_pub` | **DIFF**: key selection — key bytes "SStarU"/"SStarV"/"SStarW"/"SStarX" select HW key 1/2/3/4; key length 1 with byte 2 selects key 2; otherwise cipher key (id 0, `HAL_AESDMA_SetCipherKey2`); `HAL_AESDMA_CheckAesKey(id)` failing prints "invalid key id" and returns −1; `HAL_AESDMA_UseAesKey(id)` sets bits 6:5 of one AESDMA register (function present in i6c `halAESDMA.c`, not called by its `mdrv_aes.c`); no ISR wait path | no |
| emac `mdrv_emac.c` | open / close / suspend, `turndrv_store`, `phyStatusWR_*`, probe | **DIFF**: open / close / suspend test `strncmp(dev->name,"eth0",4)` around a per-port NAPI flag; `turndrv` commands `max`/`f10t`/`an`/`dir_on`/`dir_off`/`swing_100` (i6c: `rx_imp`/`timing`); `phyStatusWR` = `phy_r`/`phy_w` through `mdiobus_*` (i6c `phyE_r/phyE_w`); binary-only sysfs `txq_single_mode` (show / store) | no |
| flash_isp `mtd_serflash.c` | `serflash_erase` | revision mix: i6c logic (whole-chip erase, else one `MDrv_SERFLASH_AddressErase`) with 4.9 MTD glue (`instr->state`, `mtd_erase_callback`); i6e's sector-head handling is absent | yes (mtd writes) |
| mma_heap `mdrv_mma_heap.c` | `parse_mmaheap_config` | **i6e revision**: `max_start_off=%lx` / `max_end_off=%lx` → `struct MMA_BootArgs_Config` is the i6e layout (§11.3) | **yes (mi_sys data)** |
| mma_heap | `deal_with_reserved_mmap` | **DIFF**: exact-start reservation (§11.2) | yes (fb) |
| msys `ms_msys.c`, `platform_msys.c` | `msys_get_rebootType`, `ms_dump_reboot_type` | **DIFF** (older API): returns a number (§10.3); i6c returns a name string | sysfs only |
| msys | `set_bist`, `alloc_dmem` | **DIFF** `set_bist`: a reduced, parameterised form of `mercury6/platform_msys.c msys_miu_set_bist` (§12.5); `alloc_dmem` calls `msys_miu_client_switch` (the "MIU_TEST" path of `msys_miu_ioctl`) — same as the source after inlining | no |
| msys `ms_msys_perf_test.c` | `print_test_func`, `loading_test_func`, `test_print_store`, `cpu_loading_store`, `test_para_in` | **DIFF**: older perf-test revision: the thread bodies are separate binary-only functions `print_cycle_func` ("Testing printk"), `loading_cycle_func`, `memcpy_cycle_func` (random-bytes memcpy); parameter line `"%s %d %d %d"`; switch 4 both / 3 memcpy / 2 loading | no |
| rtc `drv_rtcpwc.c`, `mercury6/hal_rtcpwc.c` | sysfs shows / stores, `sstar_rtc_{read,set}_{time,alarm}`, probe, `hal_rtc_alarm_init` | **DIFF**: the driver lock is a **spinlock** (`spin_lock_irqsave(&info->mutex)`; i6c: `struct mutex`); extra sysfs `isoctl_check` ("ISO EN sequence: %d"); probe also reads a DT property `default_date`; `hal_rtc_alarm_init` re-checks the alarm interrupt with `CamOsMsSleep` and prints "Alarm init on fail !!!" | sets the clock at boot |
| serial `ms_uart.c` | `ms_uart_probe`, `_remove`, `_set_divisor`, `_suspend`, `_set_termios` | revision mix (i6e base): URDMA present but off, no uart sysfs file, FIFO clean in `set_divisor` (§7) | console |
| i2c `ms_iic.c`, `mhal_iic.c` | all 99 i6e-named functions | **DIFF** (whole driver): CamOs-based revision (§8) | **yes (sensor I2C)** |
| pwm | 13 HAL functions | the binary's `DrvPWM*` functions (`DrvPWMGetGrpAddr`, `DrvPWMDiv`, `DrvPWMGroupStop`, `DrvPWMPadSet` / `_pwm_is_pad_set` + `mdrv_padmux_active` in `mstar_pwm_enable`) are the logic of i6c `pwm/mercury6/mhal_pwm.{c,h}` under the old i6e names (`kernel-drivers.md` §6.1.5) | no |
| voltage | `set_core_voltage`, `sstar_voltage_ctrl_probe`, `show/store_scaling_voltage` | revision mix: the probe is i6c's but takes a `struct mutex` (lockdep name `&ctrl->core_voltage_mutex`) where i6c uses a CamOs spinlock; `set_core_voltage` is i6e's plus a `get_voltage_ctrl` lookup (`kernel-drivers.md` §2.3) | yes (§5.4) |
| watchdog | `infinity_wdt_probe` | the "default timer value is out of range" branch is absent (`kernel-drivers.md` §2.5.3) | no |
| usbphy | `sstar_usb_phy_setting` | prints TX current as "vcm/drv/dem" and the MIU PLL; i6c prints "Tx cur[%d]" (`kernel-drivers.md` §4.1.4) | no |
| sata_host | `ss_sata_ssc_enable` | extra print "%s, enable = %d" | no |

So the binary bodies are the source for irqchip, clk, cam_clkgen, gpio / padmux, sar, ir, flash_isp (bar `serflash_erase`
glue), bdma, cam_drv_*, cam_sysfs / cam_fs and most of CamOs and msys; the binary's own behaviour, not a tree's, holds for
the I2C driver, the CamOs delayed work, mma_heap (i6e struct layout and exact-start reservation), the present-but-off
URDMA and the RTC [R, from the rows above].

### 2.4 Functions without a name match (class B, 24) [R]
`sram_suspend_imi`, `delay_retry210/257/264`, `sram_use_size`, `mercury6_secondary_startup` (asm, §3.4 — they do have
source; ctags does not index `.S`); `_get_client_burst` (MIU arbiter, §12); `print_cycle_func`, `loading_cycle_func`,
`memcpy_cycle_func` (msys perf test, §2.3); `CamOsMemPhysToVirt` (§13.2); `txq_single_mode_show/store` (EMAC); the 11 I2C
functions of §8.

Vendor functions in generic kernel files are §15.2.

## 3. mach-sstar: the Mercury6 platform

Source match: i6c `arch/arm/mach-sstar/mercury6/{soc.c, smp_platform.c, smp_head.S, pm.c, sram.S}` + common `ms_chip.c`
(46 / 49 functions; the 3 absent are the all-CPU cache flush IPI) [R]. Machine compatible `sstar,mercury6`; DT model
`"MERCURY6 SSC019A-S01A-S"`. Boot messages from these files [R log]: `[mercury6_smp_init_cpus]`,
`[mercury6_smp_prepare_cpus]`, `Version : MVX4##M6##gc8e0e527dKL_LX409##[BR:WS_H1605_NOR_8836_IMX662]#XVM`,
`[mstar_pm_init] resume_pbase=0x20010CF5, suspend_imi_vbase=0xD0061000`, `!!!!! Machine name [MERCURY6 SSC019A-S01A-S]`.
PM / suspend code (`sram_suspend_imi`, IMI copy) is present; no suspend is used on this camera [I] (PM path:
`kernel-drivers.md` §2.2).

### 3.1 Static I/O map and RIU addressing
<!-- ev:kernel-platform.3.1 -->
Static I/O map in the image (`mstar_io_desc` at 0xC04160D0) = source `mercury6/registers.h` [R]:

| window | phys | virt | size |
|---|---|---|---|
| RIU (all IP registers) | 0x1F000000 | **0xFD000000** | 4 MiB |
| SPI flash XIP | 0x14000000 | 0xF2000000 | 32 MiB |
| GIC | 0x16000000 | 0xF4000000 | 512 KiB |
| IMI SRAM | 0xA0000000 | 0xF9000000 | 0x49800 |

The vendor modules use these virtual addresses directly (udc UTMI 0xFD284200; the i2c probe computes `res − 0x22000000`),
so the map is part of the module ABI [R]. The chip ops `mstar_chip_get_riu_phys` / `mstar_chip_get_riu_size` report the RIU
window (§3.2).

**RIU addressing.** The RIU is an array of banks of 0x100 16-bit registers. Register `rr` of bank `BANK` sits in the low
half of a 32-bit word at CPU address `0x1F000000 + BANK·0x200 + rr·4` (kernel virtual `0xFD000000 + …`); the notation
`BANK.rr` (also `BANK:rr`, `riu_r BANK rr`) names it [R: the addresses of this document, e.g. 0x1F006800 = bank 0x0034,
0x1F2026C0 = 0x1013.30]. Vendor sources and tables also use a byte address `BANK·0x100 + 2·rr` (e.g. `BASE_REG_MIU_E_PA` =
0x101300, gpio register 0x103E2A, the `[r2, #0x54 << 1]` form of `sram.S`): byte offset `b` is register `b >> 1`, the high
byte when `b` is odd [S: i6c `sram.S`; I: general rule]. A complete bank → block index of the SoC has not
been assembled; the banks this document names are [R: addresses from the DT, decompile and sources; bank
numbers computed]:

| bank | phys base | block (as named by the source) |
|---|---|---|
| 0x0006 | 0x1F000C00 | reset status (reg 0x01 bit 0 read by `msys_get_rebootType`, §10.3) |
| 0x000E | 0x1F001C00 | PM clock selects (`CLK_mcu_pm`, `CLK_spi_pm`, `CLK_pm_sleep`, `CLK_sar`, `CLK_ir`, `CLK_live_pm`, §5.1) |
| 0x0014 | 0x1F002800 | SAR ADC (DT `sar`) |
| 0x001E | 0x1F003C00 | PMTOP: chip id (reg 0x00), revision (reg 0x67) (§3.2) |
| 0x0030 | 0x1F006000 | watchdog (0x1F006000), timers (0x1F006040 / 6080 / 60C0) |
| 0x0034 | 0x1F006800 | PM-RTC (DT `rtcpwc`) |
| 0x003D | 0x1F007A00 | IR receiver (DT `ir`, 0x1000 bytes) |
| 0x003F | 0x1F007E00 | PM pad top (PAD_PM_GPIO4 / 5 registers, §6.3) |
| 0x1004 | 0x1F200800 | regs 0x08 / 0x09 receive the kernel log-buffer address at machine init (§3.2) |
| 0x1010 | 0x1F202000 | MIU ATOP (DDR PHY) (§12.3) |
| 0x1011 | 0x1F202200 | MIU_ARB (§12.3) |
| 0x1012 | 0x1F202400 | MIU0 digital (§12.3) |
| 0x1013 | 0x1F202600 | MIU_E (§12.3) |
| 0x1022 | 0x1F204400 | MIU pipe flush (regs 0x05, 0x10; §3.2) |
| 0x1032 | 0x1F206400 | CPU PLL (reg 0x19 post-divider, §5.2) |
| 0x1038 | 0x1F207000 | CLKGEN (the composite clocks, §5.1) |
| 0x103C | 0x1F207800 | not named in these sources; second `reg` window of every I2C node and of `vif`, `venc` |
| 0x103C (reg 0x6E) | 0x1F2079B8 | EMAC pad select (DT `pad` / `pad-rmii` of `emac0/1`) |
| 0x103D | 0x1F207A00 | GPIO interrupt (gpic0 / gpic1 at 0x1F207B80 / 0x1F207B90) |
| 0x103E | 0x1F207C00 | GPIO pad registers (`gpio_table`, §6.1) |
| 0x1102, 0x1103 | 0x1F220400 | fuart (two windows) |
| 0x1108–0x110B | 0x1F221000 | uart0 … uart3 (0x1F221000 / 1200 / 1400 / 1600) |
| 0x1110–0x1112 | 0x1F222000 | MSPI 0–2 |
| 0x1118–0x111E | 0x1F223000 | I2C ports 0–5 (0x1F223000 / 3200 / 3400 / 3800 / 3A00 / 3C00) |
| 0x1523–0x1525 | 0x1F2A4600 | USB3 PHY (`phy` node); 0x1524 is also the USB3 / SATA combo PHY A-PHY0 bank (`USB3_SATA_PHYA 0x152400`) [S] |

Banks of the media blocks (VIF, ISP, SCL, JPE, codec, DISP, HDMI-TX, GE, LDC, IPU …) are named in the documents of those
blocks and in the `reg` windows of §14.

### 3.2 Chip ops, chip id and revision [R decompile + Image]
`mstar_init_early` calls `ms_chip_init_default`, writes 0x10 to 0x1022.05 and fills the chip-ops table: slot 0
`mstar_chip_flush_STB_and_miu_pipe`, 1 `mstar_chip_flush_miu_pipe`, 9 `mstar_phys_to_MIU`, 10 `mstar_MIU_to_phys`, 11
`mstar_get_device_id`, 12 `mstar_get_platform_name`, 13 `mstar_get_revision`, 16 `mstar_chip_get_riu_phys`, 17
`mstar_chip_get_riu_size`, 18 `mstar_get_storage_type`, 19 `mstar_get_package_type`, 20 `mstar_chip_function_set`, 21
`mstar_chip_get_us_ticks`. The exported `Chip_*` functions jump through this table (e.g. `Chip_Get_Device_ID` slot 11 at
+0x2C, `Chip_Get_Revision` slot 13 at +0x34); `CamOsChipRevision` jumps through the same slot 13.

- **Chip id** = `mstar_get_device_id` = low byte of the 16-bit read of 0xFD003C00 = **001E.00 bits 7:0**. The value
  expected for this SoC is 0xF4 (also the sysfs `CHIP_ID`, `/sys/devices/virtual/mstar/msys/CHIP_ID`) [I: from the IPL's
  chip-id branch, `ipl.md` §4.5]; not read on this camera (CAMERA-CHECKS K1).
- **Revision** = `mstar_get_revision` = `(16-bit read of 0xFD003D9C >> 8) + 1` = **(001E.67 bits 15:8) + 1** [R disassembly:
  `ldrh`, `lsrs #8`, `adds #1`]. Other M6 boards report "soc-revision 2" [S], i.e. 001E.67 high byte = 1 there [I
  arithmetic]; the value on this camera is not known (CAMERA-CHECKS K2). It decides, among others, the SCL revision-2 init branch
  (`1214..1219:60 = 1` when revision 2) and whether the VIF port-1 errata applies (`mhal-scl-jpe-cmdq.md` §3.13, `mhal-vif.md` §4.5).
- `mstar_init_machine` registers a SoC device (`soc_device_register`): machine = the DT machine name, revision = `"%d"` of
  `mstar_get_revision`, soc id = `"%u"` of the byte at 0xFD003C00, a fourth string from chip-ops slot 14; it prints
  `Version : %s`, then `of_platform_populate`, then writes the kernel log-buffer address (`log_buf_addr_get`) to
  0xFD200820 / 0xFD200824 (1004.08 / 1004.09) [R]; what reads those registers is not known.
- `mstar_chip_flush_miu_pipe` (under `mstar_l2prefetch_lock`): 1022.05 = 0x10, then 0x11, then wait until 1022.10 bit 12
  is set [R].

### 3.3 Secondary CPU start and the A53 processor entry [R unless marked]
<!-- ev:kernel-platform.3.3 -->
- `mercury6_secondary_startup` is written to the boot-address register (`SECOND_START_ADDR_LO/HI`). It sets the secondary's
  CPUECTLR.SMPEN and VFP access itself, then goes through `secondary_startup` → the same processor entry, so every CPU runs
  `__v7_ca53_setup` (§3.4).
- `__v7_ca53_proc_info` matches MIDR **0x410FD034 exactly** (mask 0xFFFFFFFF) and points at `__v7_ca53_setup`; the generic
  `__v7_proc_info` (match 0x000F0000 / 0x000F0000) follows it and would take any other ARMv7-class CPU. Binary = i6c
  `arch/arm/mm/proc-v7.S`, instruction for instruction.
- `__v7_ca53_setup`: switch to Monitor mode; `ICC_MSRE` (`p15,6,c12,c12,5`) = 0xF (GICv3 system-register interface enabled
  for EL3 and allowed below); `SCR.NS` = 1; `CNTVOFF` = 0; `ICC_HSRE` (`p15,4,c12,c9,5`) = 9 (SRE + Enable); back to SVC;
  continue in the generic `__v7_setup`. It does **not** set SMPEN.
- Before the kernel: IPL (NOR 0x0, reset path) sets CPUECTLR.SMPEN and VFP access on the boot CPU; IPL_CUST (0x10044)
  enters Monitor, sets NS only long enough to write `CNTVOFF = 0`, and **restores SCR** (Secure again); it also installs a
  monitor vector that clears NS, handles a call with r0 = 0x808 (stores r1|r2|r3 to a fixed address) and sets NS before
  return (`ipl.md` §5.1). U-Boot has no write to SCR, ICC_MSRE / ICC_HSRE or CPUECTLR (scan of, `cpscan.py`).
- `secure_cntvoff_init` (Monitor-mode CNTVOFF write, `arch/arm/common/secure_cntvoff.S` + `mach-sstar/mercury6/pm.c`) is
  in the image but nothing calls it; secondaries get CNTVOFF = 0 from `__v7_ca53_setup`.
- Boot log: GICv3 redistributors found for CPU0 (0x16040000) and CPU1 (0x16060000); `arm_arch_timer: Architected cp15
  timer(s) running at 6.00MHz (virt)`; `SMP: Total of 2 processors activated (24.00 BogoMIPS)`.

Consequence [I, strong]: the kernel is entered in the Secure state (IPL_CUST restores SCR, U-Boot never sets NS), and only
`__v7_ca53_setup` moves every CPU to Non-secure and enables the GICv3 system-register interface from EL3. Under the generic
entry Linux would run in Secure SVC with `ICC_MSRE` at its reset value; the GICv3 driver needs `ICC_SRE.SRE` to stick and
configures interrupts as Non-secure Group 1, which a Secure CPU does not take as IRQ — interrupts would not arrive. So the
A53 entry (15 instructions plus the proc_info record), or the same four steps done elsewhere before the GIC is set up, is
required for this kernel to run; SMPEN and CNTVOFF are covered by IPL / IPL_CUST and `smp_head.S`. Once Non-secure,
Secure-only registers (e.g. CPUECTLR, unless ACTLR(EL3) grants access) cannot be written by the kernel [I].

### 3.4 Assembly bodies [R: Capstone disassembly, against i6c `mercury6/*.S`]
<!-- ev:kernel-platform.3.4 -->

| file | binary symbols | result |
|---|---|---|
| `smp_head.S` | `mercury6_secondary_startup`, `pen` | **identical** instruction for instruction: set CPUECTLR.SMPEN (`mrrc/mcrr p15,1,c15` bit 6), NSACR CP10/11, CPACR, FPEXC.EN, `v7_invalidate_l1`, holding pen on `pen_release`, then `secondary_startup` |
| `proc-v7.S` | `__v7_ca53_setup`, `__v7_ca53_proc_info` | **identical** to i6c (§3.3) |
| `sram.S` | `sram_suspend_imi`, `delay_retry210/257/264` (the `arch_usdelay` macro labels, `__LINE__` = 210/257/264 of the binary's file), `start`, `rtc_iso`, `sram_use_size` | **DIFF** in two places: (1) after the bank-0x1010 byte updates the binary adds three read-modify-writes: byte 1 `&0x9F|0x60`, byte 0x31 `&0xDF|0x20`, byte 0x31 `&0x9F|0x60`; (2) at `start` (PM-RTC bank 0x0034) the binary writes 0 to reg 0x00 and clears bit 0 of reg 0x0F, where the source writes 0x0004 to reg 0x0F and 0x0011 to reg 0x20. The same in both: the MIU self-refresh writes to banks 0x1012 / 0x1011, 6000-tick delays (1 ms at 6 MHz), writes to bank 0x1524 at the source's 8-bit offsets 0x50 / 0x54 / 0x56 = 16-bit registers **1524.28 = 0x0437, 1524.2A = 0, 1524.2B = 0x0C80**, the ISO handshake. Unused (no suspend) |

Bank 0x1524 is named a "PM sleep timer" by the source's use here but is also the USB3 / SATA combo PHY A-PHY0 bank
(§3.1); whether those writes reach a sleep timer is not known [I].

### 3.5 The SoC part and the die, as outside sources describe them
<!-- ev:kernel-platform.3.5 -->
- **Part.** The release string carries `NOR_8836` and the DT model `MERCURY6 SSC019A-S01A-S` (§3) [R]; SSC019A is a SigmaStar
  board number, not the chip's part number [I]. The package marking is under the heatsink and has not been read; the exact
  ordering code (SSC8836 suffix) is not known [R: not read].
- **Family.** Mercury6 = SDK codename **Tiramisu**; parts of that family named by linux-chenxing: SSD268G (2 × Cortex-A53),
  SSC359G, SSC8836Q ("Cortex A53, LQFP"), SSC8838G ("Cortex A53, BGA"), SSD261Q [S: linux-chenxing.org `index.md`]; SigmaStar's MI documentation has no Mercury6 section, its per-chip tables carry a "Tiramisu" column [S].
  Other Mercury6 products: RunCam Split-H (SSC8836Q-H + IMX335; `/sys/devices/soc0/machine` = the same `MERCURY6
  SSC019A-S01A-S`, ipctool chip-id 0xF4, soc-revision 2, kernel 4.9.227 built 2024, NOR 16 MB with another partition map) and
  the SIYI A8 mini (SSC8836 + IMX678) [S].
- **Die blocks**, from SigmaStar's SSD268G demo-board hardware guide V1.0 (2021-07-23, same family) [S]: ISP;
  scaler / GOP / DIP; H.264 / H.265 encoder and decoder; JPEG / MJPEG codec; LDC / fisheye; IVE; graphic engine; IPU (neural
  network accelerator); dual Cortex-A53 "over 1.2 GHz", NEON + FPU, L1 32 + 32 KB, L2 256 KB; MIPI-TX CSI / DSI; TTL;
  BT.656 / BT.1120; HDMI; USB 2.0 port 1 host; USB 2.0 port 0 host / device; USB 3.0 device; SD / SDIO; SPI-NOR / NAND / eMMC;
  external DRAM controller; GbE MAC / PHY. The guide's HDMI connector line: "HDMI V1.4 接口，支持 3840x2160 30fps 输出" (HDMI 1.4,
  3840×2160 30 fps output). MIPI sensor inputs: a 4-lane group (S1) and two 2-lane groups (S01, S00), "4Lane+2Lane+2Lane"
  [S]. On the demo board the USB 2.0-A and USB 3.0-C sockets share one USB 2.0 signal group [S]. A boot-strap "升级" (upgrade)
  mode boots over USB after the flash is blanked [S guide §1.2.3].
- **Throughput claims disagree.** SigmaStar's IPO prospectus lists the SSC8836Q as "4K60fps (H.264/H.265/M-JPEG)" [S]; the MI documentation's Tiramisu columns give 4K30 for the ISP and SCL pixel rate and for HDMI
  (`mi-capture.md` §1.1, `mi_disp-mi_hdmi.md` §1).
- On this camera: CPU part 0xd03 (`CPU: ARMv7 Processor [410fd034]`, `armv7_cortex_a53` PMU), 2 CPUs, 800 MHz (§5.2); 256 MB
  DRAM (`ipl.md` §3.8) [R boot log].

## 4. Interrupts and timer

### 4.1 Controllers [R]
`ms_main_intc` (parent GICv3), `ms_pm_intc` and `ms_gpi_intc` = i6c `drivers/sstar/irqchip/mercury6/{irqchip,irq-pmsleep,
irq-gpi}.c` (36 / 36 functions, bodies the source). In the DT (§14) the `soc` bus's default `interrupt-parent` is
`ms_main_intc` (phandle 2, three cells `<0 n 4>` = SPI-type, number n, level-high); `ms_pm_intc` (phandle 0x5B, one cell) hangs
on main-intc number 0x22 and `ms_gpi_intc` (phandle 0x7E, one cell) on 0x58; the arch timer and the PMU name the GIC
(phandle 3) directly. How `ms_main_intc` maps its number n to the GIC's SPI is not stated in these sources.

GICv3: distributor 0x16000000 (0x20000), one redistributor region 0x16040000 (0x20000, stride 0x20000; CPU0 at
0x16040000, CPU1 at 0x16060000 per the log). The vendor patch to `irq-gic-v3.c` adds `gic_dist_save/restore` and
`gic_redist_save/restore` (no source in either tree), extends `struct gic_chip_data` (saved register arrays + three per-CPU
areas allocated in `gic_of_init`) and changes `gic_cpu_pm_notifier`; it serves suspend only (specification:
`kernel-drivers.md` §2.1).

### 4.2 IRQ priority API [R]
`__irq_set_priority` and `irq_chip_set_priority_parent` (with `irq_set_priority`) are vendor additions in i6c
`kernel/irq/{chip,manage}.c`; the sstar irqchip file only uses them.

### 4.3 Timer [R]
Clock events and source: the stock `arm,armv8-timer` (`arm,cortex-a53-timer`), PPIs 13, 14, 11, 10 (flags 0x308),
`always-on`, DT `clock-frequency` 0x5B8D80 = 6 MHz; the log shows only this timer in use ("running at 6.00MHz (virt)").
The DT also has `ms_timer` (`sstar,infinity-timer`, no resources) and `timer` (`sstar,timer`, windows 0x1F006040 /
0x1F006080 / 0x1F0060C0 of 0x40, interrupts 0x80 / 0x81 / 0x83).

## 5. Clocks, cpufreq, voltage

### 5.1 The DT clock tree [R `board.dts`]
Clock drivers: DT-driven `ms_composite_clk` (`sstar,composite-clock`) + Mercury6 `ms_complex_clk` (`sstar,complex-clock`:
CPU PLL `ms_cpuclk_ops`, VEN PLL `ms_venpll_ops` hooks, named in the log). Block documents keep their own clock selects; this
is the whole tree as the DT declares it. Frequencies of fixed clocks are the DT's nominal values; the PLLs themselves are
set up before the kernel (`ipl.md` §3).

Fixed clocks (MHz): `VOID` 1 Hz (placeholder parent), `utmi_480m` 480, `mpll_432m` 432, `upll_384m` 384, `mpll_345m` 345,
`upll_320m` 320, `mpll_288m` 288, `utmi_240m` 240, `mpll_216m` 216, `utmi_192m` 192, `mpll_172m` 172.8, `utmi_160m` 160,
`mpll_123m` 123.4, `mpll_86m` 86.4, `xtali_12m` 12, `xtali_24m` 24, `miupll_clk` 400, `ddrpll_clk` 2133, `lpll_clk` 288,
`disppll_clk` 288, `armpll_37p125m` 37.5 (sic, the name says 37.125), `utmi` 60, `upll` 384, `spi_synth_pll` 104,
`fuart0_synth_out` 432, `miu_in` 400, `sd_1x_p` 110, `sd_2x_p` 220, `eth_buf` 125, `rmii_buf` 125,
`emac_testrx125_in_lan` 125, `mpll_144m` 144; the root `oscillator` node 24 MHz.

Fixed-factor clocks (mult 1): `mpll_288m_div2/4/8`, `mpll_216m_div2/4/8`, `mpll_123m_div2`, `mpll_86m_div2/4/16`,
`utmi_192m_div4`, `utmi_160m_div4/5/8`, `xtali_12m_div2/4/8/16/40/64/128`, `xtali_12m_div12`, `lpll_clk_div2/4/8`,
`disppll_clk_div2/4/8`, `mpll_144m_div2/4` (the suffix is the divider); pass-through (÷1) `miu_p` = `miu_in`, `mspi0_p` =
`mspi0`, `mspi1_p` = `mspi1`, `mcu_p` = `mcu`, `fclk1_p` = `fclk1`, `fclk2_p` = `fclk2`.

Complex clocks: `cpupll_clk` (parent `mpll_432m`; the CPUs' clock), `ven_pll` (parent `xtali_24m`; a parent of the codec
clocks).

Composite clocks: register (`bank.reg`), parents in mux-index order, mux bit field, gate bit, flags (`glitch n` = DT
`glitch-shift = n`, `auto` = `auto-enable = 1`; `ir` has `auto-enable = 0`):

| clock | reg | parents (mux 0, 1, …) | mux bits | gate | flags |
|---|---|---|---|---|---|
| `miu` | 1038.17 | VOID, VOID, miupll_clk, mpll_216m | 2–3 | 0 | glitch 4, auto |
| `ddr_syn` | 1038.19 | mpll_432m, mpll_216m, xtali_12m, VOID | 2–3 | 0 | auto |
| `miu_rec` | 1038.18 | xtali_12m_div8, xtali_12m_div16, xtali_12m_div64, xtali_12m_div128 | 2–3 | 0 | auto |
| `mcu` | 1038.01 | mpll_216m, mpll_432m, upll_320m, mpll_345m, upll_384m, mpll_288m, mpll_172m, mpll_216m_div2 | 2–3 | 0 | glitch 5, auto |
| `riubrdg` | 1038.01 | mcu_p, VOID, VOID, VOID | 10–11 | 8 | auto |
| `bdma` | 1038.60 | miu_p, VOID, VOID, VOID | 2–3 | 0 |  |
| `spi_arb` | 1038.32 | mpll_216m, mpll_216m_div2, mpll_86m, miu_p | 2–3 | 0 | glitch 4, auto |
| `pwm` | 1038.38 | xtali_12m, xtali_12m_div2, xtali_12m_div4, xtali_12m_div8, xtali_24m, mpll_86m, VOID, VOID | 10–12 | 8 |  |
| `uart0` | 1038.31 | mpll_172m, mpll_288m_div2, xtali_12m, VOID | 2–3 | 0 | auto |
| `uart1` | 1038.31 | mpll_172m, mpll_288m_div2, xtali_12m, VOID | 6–7 | 4 |  |
| `uart2` | 1038.31 | mpll_172m, mpll_288m_div2, xtali_12m, VOID | 10–11 | 8 |  |
| `uart3` | 1038.31 | mpll_172m, mpll_288m_div2, xtali_12m, VOID | 14–15 | 12 |  |
| `fuart0_synth_in` | 1038.34 | mpll_432m, mpll_216m, VOID, VOID | 6–7 | 4 |  |
| `fuart` | 1038.34 | mpll_172m, mpll_288m_div2, xtali_12m, fuart0_synth_out | 2–3 | 0 |  |
| `mspi0` | 1038.33 | mpll_216m_div2, mpll_216m_div4, xtali_12m, mpll_288m_div2 | 2–3 | 0 |  |
| `mspi1` | 1038.33 | mpll_216m_div2, mpll_216m_div4, xtali_12m, mpll_288m_div2 | 10–11 | 8 |  |
| `mspi` | 1038.33 | mspi0_p, mspi1_p | 14 | 12 |  |
| `mspi2` | 1038.32 | mpll_216m_div2, mpll_216m_div4, xtali_12m, mpll_288m_div2 | 10–11 | 8 |  |
| `mspi3` | 1038.32 | mpll_216m_div2, mpll_216m_div4, xtali_12m, mpll_288m_div2 | 14–15 | 12 |  |
| `miic0` | 1038.37 | mpll_288m_div4, mpll_216m_div4, xtali_12m, VOID | 2–3 | 0 |  |
| `miic1` | 1038.37 | mpll_288m_div4, mpll_216m_div4, xtali_12m, VOID | 10–11 | 8 |  |
| `miic2` | 1038.37 | mpll_288m_div4, mpll_216m_div4, xtali_12m, VOID | 14–15 | 12 |  |
| `miic3` | 1038.39 | mpll_288m_div4, mpll_216m_div4, xtali_12m, VOID | 2–3 | 0 |  |
| `miic4` | 1038.39 | mpll_288m_div4, mpll_216m_div4, xtali_12m, VOID | 10–11 | 8 |  |
| `miic5` | 1038.39 | mpll_288m_div4, mpll_216m_div4, xtali_12m, VOID | 14–15 | 12 |  |
| `bist` | 1038.02 | mpll_172m, mpll_123m, mpll_216m_div2, mpll_86m, mpll_288m_div4, mpll_216m_div4, mpll_288m_div2, xtali_12m | 2–4 | 0 | auto |
| `bist_mcu` | 1038.09 | mpll_288m, mpll_216m, mpll_172m, mpll_216m_div2, mpll_86m, mpll_216m_div4, mpll_216m_div8, xtali_12m | 2–4 | 0 | auto |
| `pwr_ctl` | 1038.04 | xtali_12m_div16, xtali_12m_div8, xtali_12m, VOID | 2–3 | 0 | auto |
| `xtali` | 1038.00 | xtali_12m, VOID, VOID, VOID | 2–3 | 0 | auto |
| `live_c` | 1038.00 | xtali_12m, VOID, VOID, VOID | 10–11 | 8 | auto |
| `live` | 1038.00 | xtali_12m, VOID, VOID, VOID | 10–11 | 8 | auto |
| `sr00_mclk` | 1038.62 | mpll_216m_div8, mpll_288m_div4, mpll_123m_div2, mpll_216m_div4, utmi_192m_div4, mpll_86m_div2, mpll_288m_div8, xtali_24m, mpll_86m_div4, xtali_12m, mpll_86m_div16, lpll_clk, lpll_clk_div2, lpll_clk_div4, lpll_clk_div8, armpll_37p125m | 10–13 | 8 | auto |
| `sr01_mclk` | 1038.65 | mpll_216m_div8, mpll_288m_div4, mpll_123m_div2, mpll_216m_div4, utmi_192m_div4, mpll_86m_div2, mpll_288m_div8, xtali_24m, mpll_86m_div4, xtali_12m, mpll_86m_div16, lpll_clk, lpll_clk_div2, lpll_clk_div4, lpll_clk_div8, armpll_37p125m | 2–5 | 0 | auto |
| `sr1_mclk` | 1038.65 | mpll_216m_div8, mpll_288m_div4, mpll_123m_div2, mpll_216m_div4, utmi_192m_div4, mpll_86m_div2, mpll_288m_div8, xtali_24m, mpll_86m_div4, xtali_12m, mpll_86m_div16, lpll_clk, lpll_clk_div2, lpll_clk_div4, lpll_clk_div8, armpll_37p125m | 10–13 | 8 | auto |
| `sr2_mclk` | 1038.66 | mpll_216m_div8, mpll_288m_div4, mpll_123m_div2, mpll_216m_div4, utmi_192m_div4, mpll_86m_div2, mpll_288m_div8, xtali_24m, mpll_86m_div4, xtali_12m, mpll_86m_div16, lpll_clk, lpll_clk_div2, lpll_clk_div4, lpll_clk_div8, armpll_37p125m | 10–13 | 8 | auto |
| `bist_ipu_gp` | 1038.05 | mpll_288m, mpll_216m, mpll_172m, mpll_216m_div2, mpll_86m, mpll_216m_div4, mpll_216m_div8, xtali_12m | 2–4 | 0 |  |
| `ipu` | 1038.50 | xtali_12m, utmi_480m, mpll_432m, upll_384m, upll_320m, utmi_240m, mpll_216m, mpll_123m | 2–4 | 0 |  |
| `ipuff` | 1038.50 | mpll_432m, upll_384m, upll_320m, utmi_240m | 10–11 | 8 |  |
| `bist_usb30_gp` | 1038.05 | mpll_172m, mpll_123m, mpll_216m_div2, mpll_86m, mpll_288m_div4, mpll_216m_div4, mpll_288m_div2, xtali_12m | 10–12 | 8 |  |
| `sata_pm` | 1038.6E | mpll_216m_div2, xtali_12m, xtali_24m, mpll_123m | 10–11 | 8 |  |
| `sata_phy_108` | 1038.6E | mpll_216m_div2, VOID, VOID, VOID | 2–3 | 0 |  |
| `sata_phy_432` | 1038.6E | mpll_432m, VOID, VOID, VOID | 6–7 | 4 |  |
| `sata_axi` | 1038.6E | mpll_216m_div2, mpll_216m, mpll_288m, mpll_216m_div4 | 14–15 | 12 |  |
| `ssusb_axi` | 1038.6F | mpll_216m_div2, mpll_216m, mpll_288m, upll_384m | 14–15 | 12 |  |
| `ssusb_phy_108` | 1038.6F | mpll_216m_div2, VOID, VOID, VOID | 2–3 | 0 |  |
| `ssusb_phy_432` | 1038.6F | mpll_432m, VOID, VOID, VOID | 6–7 | 4 |  |
| `csi_mac_lptx_top_i_m00` | 1038.58 | mpll_216m, utmi_240m, mpll_172m, mpll_288m_div2, mpll_86m, utmi_160m_div4, VOID, VOID | 2–4 | 0 |  |
| `csi_mac_top_i_m00` | 1038.58 | mpll_216m, utmi_240m, mpll_172m, mpll_288m_div2, mpll_86m, utmi_160m_div4, VOID, VOID | 10–12 | 8 |  |
| `ns_top_i_m00` | 1038.59 | mpll_216m, utmi_240m, mpll_172m, mpll_288m_div2, mpll_86m, utmi_160m_div4, VOID, VOID | 2–4 | 0 |  |
| `csi_mac_lptx_top_i_m01` | 1038.59 | mpll_216m, utmi_240m, mpll_172m, mpll_288m_div2, mpll_86m, utmi_160m_div4, VOID, VOID | 10–12 | 8 |  |
| `csi_mac_top_i_m01` | 1038.5A | mpll_216m, utmi_240m, mpll_172m, mpll_288m_div2, mpll_86m, utmi_160m_div4, VOID, VOID | 2–4 | 0 |  |
| `ns_top_i_m01` | 1038.5A | mpll_216m, utmi_240m, mpll_172m, mpll_288m_div2, mpll_86m, utmi_160m_div4, VOID, VOID | 10–12 | 8 |  |
| `csi_mac_lptx_top_i_m10` | 1038.5B | mpll_216m, utmi_240m, mpll_172m, mpll_288m_div2, mpll_86m, utmi_160m_div4, VOID, VOID | 2–4 | 0 |  |
| `csi_mac_top_i_m10` | 1038.5B | mpll_216m, utmi_240m, mpll_172m, mpll_288m_div2, mpll_86m, utmi_160m_div4, VOID, VOID | 10–12 | 8 |  |
| `ns_top_i_m10` | 1038.5C | mpll_216m, utmi_240m, mpll_172m, mpll_288m_div2, mpll_86m, utmi_160m_div4, VOID, VOID | 2–4 | 0 |  |
| `csi_mac_lptx_top_i_m11` | 1038.5D | mpll_216m, utmi_240m, mpll_172m, mpll_288m_div2, mpll_86m, utmi_160m_div4, VOID, VOID | 2–4 | 0 |  |
| `csi_mac_top_i_m11` | 1038.5D | mpll_216m, utmi_240m, mpll_172m, mpll_288m_div2, mpll_86m, utmi_160m_div4, VOID, VOID | 10–12 | 8 |  |
| `ns_top_i_m11` | 1038.5E | mpll_216m, utmi_240m, mpll_172m, mpll_288m_div2, mpll_86m, utmi_160m_div4, VOID, VOID | 2–4 | 0 |  |
| `mipi_tx_csi` | 1038.5C | lpll_clk, utmi_160m, mpll_288m_div2, mpll_216m_div2, mpll_216m, utmi_240m, mpll_288m, upll_320m | 10–12 | 8 | auto |
| `mipi_tx_dsi` | 1038.5E | lpll_clk, utmi_160m, mpll_288m_div2, mpll_216m_div2, mpll_216m, utmi_240m, VOID, VOID | 10–12 | 8 | auto |
| `bist_vhe_gp` | 1038.03 | mpll_288m, mpll_216m, mpll_172m, mpll_216m_div2, mpll_86m, mpll_216m_div4, mpll_216m_div8, xtali_12m | 10–12 | 8 |  |
| `vhe` | 1038.68 | ven_pll, utmi_480m, mpll_432m, upll_384m, upll_320m, mpll_288m, utmi_192m, mpll_123m | 2–4 | 7 |  |
| `mfe` | 1038.69 | mpll_216m, upll_384m, upll_320m, ven_pll | 2–3 | 0 |  |
| `ven_axi` | 1038.69 | upll_384m, upll_320m, mpll_216m, ven_pll, VOID, VOID, VOID, VOID | 10–12 | 8 |  |
| `xtali_sc_gp` | 1038.00 | xtali_12m, VOID, VOID, VOID | 6–7 | 4 |  |
| `bist_sc_gp` | 1038.03 | mpll_288m, mpll_216m, mpll_172m, mpll_216m_div2, mpll_86m, mpll_216m_div4, mpll_216m_div8, xtali_12m | 2–4 | 0 |  |
| `emac_ahb` | 1038.42 | mpll_288m_div2, mpll_123m, mpll_86m, emac_testrx125_in_lan | 2–3 | 0 |  |
| `jpe` | 1038.6A | utmi_480m, mpll_432m, upll_384m, upll_320m, mpll_288m, mpll_216m, VOID, VOID | 2–4 | 0 |  |
| `aesdma` | 1038.61 | mpll_86m, mpll_172m, VOID, VOID | 2–3 | 0 | glitch 4 |
| `sdio` | 1038.45 | utmi_192m_div4, mpll_86m_div2, utmi_160m_div4, mpll_288m_div8, utmi_160m_div5, utmi_160m_div8, xtali_12m, xtali_12m_div40, spi_synth_pll, mpll_432m, upll_384m, upll_320m, mpll_288m, mpll_216m, mpll_216m_div2, mpll_86m | 2–5 | 0 |  |
| `sd` | 1038.43 | utmi_192m_div4, mpll_86m_div2, utmi_160m_div4, mpll_288m_div8, utmi_160m_div5, utmi_160m_div8, xtali_12m, xtali_12m_div40, spi_synth_pll, mpll_432m, upll_384m, upll_320m, mpll_288m, mpll_216m, sd_1x_p, sd_2x_p | 2–5 | 0 |  |
| `sd_syn` | 1038.44 | mpll_216m, mpll_432m, xtali_12m, VOID | 14–15 | 12 |  |
| `isp` | 1038.61 | utmi_480m, mpll_432m, upll_384m, upll_320m, utmi_240m, mpll_216m, mpll_172m, mpll_288m_div4 | 10–12 | 8 |  |
| `fclk1` | 1038.64 | utmi_480m, mpll_432m, upll_384m, upll_320m, mpll_288m, utmi_240m, mpll_172m, mpll_123m | 2–4 | 0 | auto |
| `fclk2` | 1038.64 | utmi_480m, mpll_432m, upll_384m, upll_320m, mpll_288m, utmi_240m, mpll_172m, mpll_123m | 10–12 | 8 | auto |
| `odclk` | 1038.66 | mpll_86m, mpll_86m_div2, mpll_86m_div4, lpll_clk | 2–3 | 0 | auto |
| `dip` | 1038.52 | utmi_480m, mpll_432m, upll_384m, upll_320m, mpll_288m, utmi_240m, mpll_172m, mpll_123m | 2–4 | 0 |  |
| `mop0` | 1038.47 | upll_384m, mpll_432m, utmi_480m, upll_320m, mpll_288m, utmi_240m, mpll_216m, utmi_192m | 2–4 | 0 | auto |
| `mop1` | 1038.47 | upll_384m, mpll_432m, utmi_480m, upll_320m, mpll_288m, utmi_240m, mpll_216m, utmi_192m | 10–12 | 8 | auto |
| `disp_432` | 1038.48 | mpll_432m, VOID, VOID, VOID | 10–11 | 8 | auto |
| `disp_216` | 1038.48 | mpll_216m, VOID, VOID, VOID | 14–15 | 12 | auto |
| `disp_pixel_0` | 1038.49 | mpll_288m, upll_320m, utmi_192m, utmi_240m, mpll_216m, mpll_172m, mpll_123m, mpll_288m_div2, mpll_216m_div2, lpll_clk, disppll_clk, VOID, VOID, VOID, VOID, VOID | 2–5 | 0 | auto |
| `disp_pixel_1` | 1038.49 | mpll_288m, upll_320m, utmi_192m, utmi_240m, mpll_216m, mpll_172m, mpll_123m, mpll_288m_div2, mpll_216m_div2, lpll_clk, disppll_clk, VOID, VOID, VOID, VOID, VOID | 10–13 | 8 | auto |
| `njpd` | 1038.4A | mpll_432m, upll_384m, upll_320m, mpll_288m, mpll_172m, utmi_240m, mpll_345m, VOID | 2–4 | 0 |  |
| `bist_disp_gp` | 1038.06 | mpll_288m, mpll_216m, mpll_172m, mpll_216m_div2, mpll_86m, mpll_216m_div4, mpll_216m_div8, xtali_12m | 2–4 | 0 | auto |
| `ive` | 1038.6A | mpll_216m, mpll_172m, mpll_123m, mpll_86m, mpll_288m, upll_320m, upll_384m, utmi_240m | 10–12 | 8 | auto |
| `ge` | 1038.51 | miu_p, mpll_216m, mpll_172m, mpll_288m_div2, upll_320m, upll_384m, mpll_432m, utmi_240m | 2–4 | 0 |  |
| `ldcfeye` | 1038.53 | utmi_480m, mpll_432m, upll_384m, upll_320m, mpll_288m, mpll_216m, mpll_86m, VOID | 2–4 | 0 |  |
| `bist_isp_gp` | 1038.02 | mpll_288m, mpll_216m, mpll_172m, mpll_216m_div2, mpll_86m, mpll_216m_div4, mpll_216m_div8, xtali_12m | 7–9 | 5 | auto |
| `emac_tx` | 1133.23 | eth_buf, rmii_buf | 2 | 0 |  |
| `emac_rx` | 1133.22 | eth_buf, rmii_buf | 2 | 0 |  |
| `emac_tx_ref` | 1133.23 | rmii_buf, VOID | 10 | 8 |  |
| `emac_rx_ref` | 1133.22 | rmii_buf, VOID | 10 | 8 |  |
| `emac1_tx` | 1133.2B | eth_buf, rmii_buf | 2 | 0 |  |
| `emac1_rx` | 1133.2A | eth_buf, rmii_buf | 2 | 0 |  |
| `emac1_tx_ref` | 1133.2B | rmii_buf, VOID | 10 | 8 |  |
| `emac1_rx_ref` | 1133.2A | rmii_buf, VOID | 10 | 8 |  |
| `live_pm` | 000E.34 | xtali_12m, VOID, VOID, VOID | 0–1 | — | auto |
| `mcu_pm_p1` | 000E.20 | xtali_12m, mpll_86m, mpll_216m, mpll_216m_div2 | 2–3 | 0 | auto |
| `mcu_pm` | 000E.20 | xtali_24m, mcu_pm_p1 | 7 | — | auto |
| `spi_pm` | 000E.20 | xtali_24m, spi_synth_pll, mpll_86m, mpll_216m_div2, mpll_86m_div2, mpll_216m_div4, VOID, VOID | 10–12 | 8 | glitch 13, auto |
| `pm_sleep` | 000E.22 | xtali_12m, VOID, VOID, VOID | 10–11 | — | auto |
| `sar` | 000E.22 | xtali_12m, VOID, VOID, VOID | 7–8 | 5 | auto |
| `bach` | 1038.6D | upll_384m | — | 7 | auto |
| `ir` | 000E.21 | xtali_24m, xtali_12m, xtali_12m_div2, xtali_12m_div4, xtali_12m_div8, VOID, VOID, VOID | 7–9 | 5 |  |

### 5.2 CPU clock and cpufreq [R]
Both CPU nodes: `clocks = <cpupll_clk>`, DT `clock-frequency` 1 GHz. cpu@0 `operating-points` (kHz / µV): 1 200 000 /
1 000 000, 1 100 000 / 1 000 000, 1 000 000 / 900 000, 900 000 / 900 000, 800 000 / 850 000, 600 000 / 850 000, 400 000 /
850 000. cpufreq (`sstar,infinity-cpufreq`, i6c `cpufreq/mercury6/cpufreq.c`) runs the CPUs at **800 MHz** (log `Current
clk=799999872`). The CPU PLL post-divider is 1032.19 (800 MHz if it reads 1); not read on this camera (CAMERA-CHECKS K7).

### 5.3 cam_clkgen [R]
`cam_clkgen` (`CamClk*`, 25 functions, bodies the source) is the clock API mhal uses (12 imports).

### 5.4 Voltage rails (voltage-ctrl) [R]
Three `sstar,voltage-ctrl` nodes, each `vid_width = 1`, `vid_voltages = <900 1000>` (mV), `init_voltage = 900`:

| node | `vid_gpios` | pad | probe |
|---|---|---|---|
| `cpu_power` | 10 | PAD_PM_GPIO4 (PM byte address 0x3F08) | succeeds |
| `core_power` | 11 | PAD_PM_GPIO5 (0x3F0A) | succeeds |
| `dla_power` | **11 again** | PAD_PM_GPIO5 | fails: `gpio_request(11, "vid0")` is refused (already owned by `core_power`) → "Failed to get gpio for voltage control!! -1", "sstar,voltage-ctrl: probe of soc:dla_power failed with error -1" |

`cpu_power` and `core_power` set their GPIO high = 1000 mV at probe: the TEMPERATURE demander holds the rail at 1000 mV
from probe on, so the codec's VENC request of 1000 mV is honoured but changes nothing; the ISP and the codec therefore run at
1.0 V [I: if the GPIO reaches the regulator]. mhal calls `get_core_voltage` / `set_core_voltage` (2 imports). sysfs:
`/sys/devices/system/voltage/{cpu_power,core_power}/` (`voltage_current` and the demander list). Expected 1000 mV in both
`/sys/devices/system/voltage/core_power/voltage_current` and `…/cpu_power/voltage_current` [I]; not read on this camera
(CAMERA-CHECKS K8). Driver behaviour, data and sysfs in full: `kernel-drivers.md` §2.3.

## 6. GPIO, padmux and board wiring

### 6.1 GPIO [R]
Driver: i6c `gpio/{mdrv_gpio.c, mdrv_gpio_io.c, mercury6/mhal_gpio.c}` + `camdriver_gpio_*` (bodies the source; DT node
`gpio`, `sstar,gpio`). The image's `gpio_table` (0xC02AC7A4, 163 × 80 B) equals i6c `gpio/mercury6/mhal_gpio.c` in every
name and every OEN / OUT / IN / DRV / PE / PS register + mask (0 differences). Example:
**gpio46 = pad `PAD_GPIO2`, register 0x103E2A** (byte address; OEN bit 2, OUT bit 1, IN bit 0). The gpiolib sysfs export is
built in (`/sys/class/gpio/export`). GPIO interrupts: `gpic0` / `gpic1` (`sstar,gpic`, 0x1F207B80 / 0x1F207B90, 0x10 each,
`group` 0 / 1) on `ms_gpi_intc` inputs 0x82 / 0x83. Exports used by vendor modules: `MDrv_GPIO_*` (kdrv_sdmmc, mhal),
`camdriver_gpio_*` (mhal).

### 6.2 Padmux [R]
The three tables of i6c `drivers/sstar/gpio/mercury6/mhal_pinmux.c`, compiled on the host from the source (macros of the
file + `include/mercury6/{gpio,padmux}.h` + `mdrv_types.h` BIT values; `padmux-gen.sh`) and compared byte for byte with the
Image:

| table | Image address | entries × size | result |
|---|---|---|---|
| `m_stPadMuxTbl` {u16 padID; u32 base; u16 offset, mask, val, mode} | 0xC02B4494 | 2257 × 16 B | **identical, every byte** (padding included) |
| `m_stPadMuxTblMisc` | 0xC02AFA94 | 64 × 16 B | **identical** |
| `m_stPadModeInfoTbl` {char name[32]; u32 RIU address; u16 mask, val} | 0xC02AFE94 | 448 × 40 B | **identical** |

The DT `padmux` node (`sstar-padmux`) carries `schematic = <pad mode puse …>` triples; the padmux driver applies them at
probe (`_mdrv_padmux_dts` → `MDrv_GPIO_PadVal_Set`). Decoded with the tables (pad names from mercury6 `gpio.h`, mode names
from `m_stPadModeInfoTbl`, puse ids from the **i6e** `mdrv_puse.h`, whose numbering the DT uses — the i6c header numbers
them differently); pad names below without their `PAD_` prefix:

| mode (id) | pads (pad id) | puse |
|---|---|---|
| I2C1_MODE_3 (28) | SR0_IO18 (70) SCL, SR0_IO19 (71) SDA | 0x60100 / 0x60101 `I2C1_SCL/SDA` |
| I2C2_MODE_5 (38) | SR1_IO18 (90) SCL, SR1_IO19 (91) SDA | 0x60200 / 0x60201 |
| I2C3_MODE_7 (46) | PM_I2CM_SCL (4), PM_I2CM_SDA (5) | 0x60300 / 0x60301 |
| I2C4_MODE_7 (53) | GPIO15 (143) SCL, GPIO16 (144) SDA | 0x60400 / 0x60401 |
| PM_IR_IN_MODE_1 (444) | PM_IRIN (20) — IR receiver | 0x40000 `IR` |
| SD0_MODE_1 (122) + SD0_CDZ_MODE_1 (124) | SD0_D1 (26), D0 (27), CLK (28), CMD (29), D3 (30), D2 (31), CDZ (25) | 0x200xx `SDIO0_*` |
| GPIO_MODE (0) | SD_GPIO0 (145) — SDIO0 power | 0x20000 `SDIO0_PWR` |
| SD1_MODE_1 (125) + SD1_CDZ_MODE_1 (130) | SD1_D1 (149), D0 (150), CLK (151), CMD (152), D3 (153), D2 (154), CDZ (148) | 0x201xx `SDIO1_*` |
| GPIO_MODE (0) | SD1_GPIO0 (146) — SDIO1 power | 0x20100 `SDIO1_PWR` |
| LED0_MODE_2 (236) / LED1_MODE_2 (240) | ETH_LED0 (38), ETH_LED1 (39) | 0x10001 `EMAC0_LED_GPIO_GREEN` / 0x10002 `…_ORANGE` |
| VGA_HSYNC_MODE_1 (396) / VGA_VSYNC_MODE_1 (397) | HSYNC_OUT (135), VSYNC_OUT (134) | 0x120000 / 0x120001 (no name in the header) |
| TTL24_MODE_4 (368) | GPIO0–GPIO7 (44–51), TX0_IO00–IO09 (92–101), TTL18–TTL27 (120–129) — 28 pads | 0x140200–0x14021B (no name) |
| GPIO_MODE (0) | SAR_GPIO0 (21), SAR_GPIO1 (22), SAR_GPIO2 (23) | 0xF0000 `AIO_AMP_PWR` |
| — | pad 163, mode 0xFFFF | 0 `MDRV_PUSE_NA` (end marker) |

PAD_GPIO2 — the gpio46 that kjd_lvgl drives — is in the TTL24 list; exporting gpio46 goes through the gpio chip's request
op `camdriver_gpio_request` → `MDrv_GPIO_Pad_Set`, which switches the pad back to GPIO mode [R code path; I that the sysfs
export reaches this op, standard gpiolib]. The TTL24 / VGA-sync entries look like the vendor board template (this camera
outputs HDMI) [I]. `mdrv_padmux_*` exports are used by kdrv_sdmmc and mhal (3 imports); `mdrv_padmux_getpad(puse)` returns
the first pad of the schematic with that use.

### 6.3 Board wiring
<!-- ev:kernel-platform.6.3 -->
| signal | pad / GPIO | what it does | origin |
|---|---|---|---|
| front status LED colour | gpio46 = PAD_GPIO2 | 1 = orange, 0 = red; kjd_lvgl exports it, sets output, writes 1 at start, 0 on standby, 1 on wake (orange while running, red in standby); before the export the pad is in the TTL24 group | [R: factory-test strings of the vendor tool `tmp` "LED red" / "LED orange", I physical form: a red / green two-colour LED with GPIO46 switching the green die] |
| CPU rail VID | GPIO 10 = PAD_PM_GPIO4 | `cpu_power`, high = 1000 mV (§5.4) | [R] |
| core rail VID | GPIO 11 = PAD_PM_GPIO5 | `core_power`, high = 1000 mV; `dla_power` names the same GPIO and fails | [R] |
| audio amplifier power | PAD_SAR_GPIO0 (pad 21) | mhal audio's `HalAudOsApiChangeAmpGpio` takes `mdrv_padmux_getpad(AIO_AMP_PWR 0xF0000)` = the first pad with that use (`audio.md` §5.7); SAR_GPIO1 / 2 carry the same use | [R] |
| front buttons | one SAR ADC channel | a resistor ladder of six buttons (OK, ON, UP, MENU, DOWN, AE) read by kjd_lvgl through `/dev/sar` every 100 ms; value windows: `kjd_lvgl.md` §6.1 | [R] |
| IR remote | PAD_PM_IRIN | NEC receiver, `/dev/input/event0` (`kernel-drivers.md` §3.2) | [R] |
| SD / SDIO | SD0 and SD1 pad groups, power on PAD_SD_GPIO0 / PAD_SD1_GPIO0 | `kdrv_sdmmc.ko`; which port carries the card slot: `kernel-drivers.md` §5.1.7 | [R DT] |
| console | uart0 (0x1F221000), ttyS0 115200 | §7 | [R] |

### 6.4 The board [R: board photos, boot log]
<!-- ev:kernel-platform.6.4 -->
- Mainboard silkscreen **"oseecam WS03 V5.0 / 2023.09.16"** (photo 05). Board maker OSEECAM = 深圳市微申时代科技有限公司 (Shenzhen
  Oseecam Technology), "WS" = 微申 (WeiShen) [S: oseecam.com, I for "WS"].
- SoC under a heatsink (marking not read); a **24.000 MHz crystal** next to it (photo 05); SPI-NOR **Zbit 25VQ128DSJG**, 16 MiB
  (photo 01; JEDEC id §9).
- FPC connectors: **HDMI_OUT** (J2), **SD** (J3), a third FPC **J1**, and on the back of the mainboard the sensor FPC labelled
  **"MIPI 4Lane"** (photos 01, 05). The sensor sits on its own board at the end of that FPC; its pinout is not known.
- **UART pads**: a 7-pad group near J1 with silkscreen RX / GND / 3.3V; the console TX is the middle pad — the only one that
  swings (to ~2.1 V) during boot, the others hold 3.268 V; 115200 8N1 (§7) (photos 03, 10; measured with a meter).
- Unlabelled 3-pad group near the crystal; GND / 5V and GND / 3.3V pad pairs.
- Power comes from a separate 12 V input, not from USB bus power; the USB descriptor declares `MaxPower 2mA`
  (`usb-gadget.md` §4.3).

## 7. UART [R]
`drivers/sstar/serial/ms_uart.c` (name matches 49 / 61 in i6c, 50 / 60 in i6e; an i6e base with additions). Five ports
probe, `base_baud` 10 800 000 each (log):

| tty | DT node | windows | DT interrupt | clock |
|---|---|---|---|---|
| ttyS0 (console, alias `serial0`) | `uart@1F221000` | 0x1F221000 | 0x42 | `uart0` |
| ttyS1 | `uart1@1F221200` | 0x1F221200 | 0x43 | `uart1` |
| ttyS2 | `fuart@1F220400` | 0x1F220400 + 0x1F220600 | 0x4F, 0x50 | `fuart`; `sctp_enable = 0`, `dma = 0` |
| ttyS3 | `uart2@1F221400` | 0x1F221400 | 0x72 | `uart2` |
| ttyS4 | `uart3@1F221600` | 0x1F221600 | 0x73 | `uart3` |

URDMA code is present (11 `URDMA_*` functions, `_urdma_tx`, `urdma_tx_thread`; the DMA buffer address goes through
`Chip_Phys_to_MIU`) but off: the only port with a `dma` property (fuart) has `dma = <0>`. RS485 is absent. No uart sysfs
file; `set_divisor` cleans the FIFO under the port spinlock. Console: `console=ttyS0,115200` (§11.1).

## 8. I2C — the driver with binary-only code [R decompile vs i6e]
The function names are i6e's `ms_iic.c` + `infinity6e/mhal_iic.c` (99 of 114), but the code is a CamOs-based revision:
prints via `CamOsPrintf`, delays via `CamOsUsDelay`, port mutex via `CamOsMutexInit/Destroy`, DMA buffer via
`MDrv_HWI2C_DMA_Alloc/DeAlloc`, every register access offset by `MDrv_HWI2C_GetPortRegOffset`, clock via `i2c_set_srcclk` /
`_MDrv_HWI2C_SetDelayFactor` / `HAL_HWI2C_SetClkCnt`, `ms_i2c_xfer` a 30-byte wrapper of `ms_i2c_xfer_common`, DMA MIU
address via `CamOsMemPhysToMiu`. The probe reads DT `i2c-group`, `i2c-padmux`, `i2c-speed`, `i2c-en-dma` and three `reg`
windows (port bank, 0x1F207800, 0x1F207000); adapter name "Sstar I2C adapter %d"; `i2c_add_numbered_adapter`. The 11
functions without source:

| function | what it does |
|---|---|
| `MDrv_HWI2C_GetPortRegOffset` | **6 ports**, bank offsets 0, 0x100, 0x200, 0x400, 0x500, 0x600 (i6e: via HAL, fewer ports) |
| `_MDrv_HWI2C_SetClk` | speed index 0–9 → SCL high / low counts + source clock index (table below), then `i2c_set_srcclk` and `HAL_HWI2C_SetClkCnt` |
| `HAL_HWI2C_SetClkCnt` | writes 6 count registers at port +0x12, +0x14, +0x10, +0x16, +0x18, +0x1A from `g_HWI2CClkCnt[port]` (6 × u16) |
| `i2c_set_srcclk` / `i2c_remove_srcclk` | takes the DT `clocks` of the port, prepares / enables every parent, sets parent 0 to 12 / 54 / 72 MHz |
| `_MDrv_HWI2C_SetDelayFactor` | DMA delay factor per speed (0x20 … 1) |
| `ms_i2c_xfer_common` | per-port mutex, controller reset pulse, then per message `ms_i2c_xfer_write/read`; an alternative "ability" callback path exists but `HAL_HWI2C_CheckAbility` returns 0, so it is never taken |
| `MDrv_HWI2C_DMA_Alloc/DeAlloc` | one 4 KiB coherent buffer per port (`dma_alloc_from_coherent`) |
| `HAL_HWI2C_{Read,Write}Clk{Byte,2Byte,2ByteMask}` | accessors for the clock bank |

| speed idx | H / L count | src clock | SCL ≈ [I: src / (H + L + ~2)] |
|---|---|---|---|
| 0 / 1 / 2 / 3 | 235/237, 115/117, 55/57, 25/27 | 12 MHz | 25 / 50 / 100 / 200 kHz (same counts as i6e's 25…200 kHz rows) |
| 4 / 5 / 6 / 7 | 87/89, 64/66, 42/44, 30/32 | 54 MHz | ≈300 / 400 / 600 / 850 kHz |
| 8 / 9 | 33/35, 21/23 | 72 MHz | ≈1 / 1.5 MHz |

No node in `board.dts` sets `i2c-speed`, so the probe default **5 (≈400 kHz)** applies to every bus [R default; I rate].
DT ports (`sstar,i2c`, all three windows as above, `i2c-group` = port number):

| node | bank window | DT interrupt | clock | pads (§6.2) | children |
|---|---|---|---|---|---|
| `i2c0@0` | 0x1F223000 | 0x44 | `miic0` | none in the schematic | — |
| `i2c1@1` | 0x1F223200 | 0x45 | `miic1` | SR0_IO18 / 19 | — |
| `i2c2@2` | 0x1F223400 | 0x2C | `miic2` | SR1_IO18 / 19 | — |
| `i2c3@3` | 0x1F223800 | 0x6D | `miic3` | PM_I2CM_SCL / SDA | `goodix_gt911@5D` (reg 0x5D, `goodix_rst` 0xD, `goodix_int` 0xC, `interrupts-extended = <ms_pm_intc 8>`; enabled and failing, §14; hardware not on this camera) |
| `i2c4@4` | 0x1F223A00 | 0x6E | `miic4` | GPIO15 / 16 | `gyro@d0` (`sstar,gyro`, reg 0x69, `gyro_int` / `gyro_fsync` 0xFFFF) |
| `i2c5@5` | 0x1F223C00 | 0x6F | `miic5` | none in the schematic | — |

Users: mhal through `CamI2cOpen/Close/Transfer` (i6c `cam_drv_i2c.c`, 3 imports; the sensor bus) and `/dev/i2c-*`
(i2c-dev built in). Which port the sensor sits on is `mhal-csi-snr.md` §2.4's. The i6c tree's own I2C driver is a different,
newer `drv_iic.c` with a `mercury6p` HAL (another register layer); the binary is not that driver.

## 9. SPI-NOR, BDMA, MXP partitions, MTD [R]
<!-- ev:kernel-platform.9 -->
`flash_isp` = source in both trees (166 / 166 by name): `drivers/sstar/flash_isp/{drvSERFLASH.c, mercury6/halSERFLASH.c,
mtd_serflash.c, part_mxp.c, mxp_flash.c, drvDeviceInfo.c}`; DT `flashisp` (`mtd-flashisp`, clock `bdma`, `quadread = 0`).
The chip table `_hal_SERFLASH_table` (0xC044DE14, 70 × 44 B) matches i6c `drvDeviceInfo.c` (entry count); this camera's
chip is the entry **`FLASH_IC_ST25Q128` (0x0303) = MID 0x5E (Zbit), 0x40, 0x18**, 16 MiB (log `[FSP] Flash is detected
(0x0303, 0x5E, 0x40, 0x18) ver1.1`, `[FSP] 1-1-1 FAST_READ MODE`, `mtd .name = NOR_FLASH, .size = 0x01000000 (16MiB)`).
- Partitions come from the MXP table at 0x20000 (`part_mxp.c`; log `MXP_PARTS!!`, `MXP found at mxp_offset[3]=0x00020000,
  size=0x1000`, `Creating 6 MTD partitions on "NOR_FLASH"`: BOOT 0x0–0x70000, KERNEL 0x70000–0x280000, rootfs
  0x280000–0x480000, miservice 0x480000–0x880000, customer 0x880000–0xE90000, MISC 0xE90000–0xF30000; the map itself:
  `firmware-image.md` §1.1).
- The vendor patches `drivers/mtd/mtdpart.c`: `ajust_offset` and `part_del_partition_mapping` with its `part_mapping[]`
  table — an MTD partition remap (both trees have it).
- `serflash_erase`: i6c logic (whole-chip erase, else one `MDrv_SERFLASH_AddressErase`) with 4.9 MTD glue (`instr->state`,
  `mtd_erase_callback`); i6e's sector-head handling is absent.
- Write path: the DMEM buffer `BDMA_FSP_WBUFF` (§10.2) through BDMA
  (`bdma/mercury6/hal_bdma.c`, 8 / 8 functions). DT `bdma0`…`bdma3` (`sstar,bdma0..3`, interrupts 0x48, 0x49, 0x5D, 0x5E,
  clock `bdma`); `movdma` (`sstar,movdma`, interrupt 0x5C, clock `miu`; `movedma/hal_movedma.c`).
- DT `spinandflash` (`ms-spinand`) is enabled but no NAND / SPINAND driver symbols are in kallsyms.

**MTD ioctl `MEMGETBBTBLKINFO` (`0x80084D33`).** `_IOR('M', 51, struct sstar_bbt_info_user)`, "Get an block info in bbt:
good block, factory bad block or running time bad block" [S: i6c `include/uapi/mtd/mtd-abi.h` l.215]; argument 8 bytes [S:
l.49]:

| offset | field | direction | meaning |
|---|---|---|---|
| +0 | `__u32 u32_offset` | in | byte offset of the erase block inside the MTD partition (fwfs passes `block × block_size`) |
| +4 | `__u8 u8_type` | out | `0x0F` good block, `0x00` factory bad block |
| +5 | `__u8 padding[3]` | — | untouched by the kernel; fwfs ignores it (reads `word1 & 0xFF`) |

- In the i6c (5.10.61) tree [S: `drivers/mtd/mtdchar.c` l.898–921]: copy the struct in, call `mtd_block_isbad(mtd,
  u32_offset)`; result `> 0` → `u8_type = 0x00` (`FACTORY_BAD_BLOCK`), `<= 0` → `0x0F` (`GOOD_BLOCK`) — an error from
  `mtd_block_isbad` (offset out of range → `-EINVAL`) is reported as a good block; copy back; return 0. Listed among the
  commands allowed on a read-only open [S: l.657]. The third state ("running time bad block") has no value and no code; the
  companion `MEMSETBBTBLKINFO` = `_IOWR('M', 52, …)` = `0xC0084D34` is defined [S: l.217] but has no `case` (→ `-ENOTTY`). Both
  are SigmaStar additions, absent from vanilla 4.9 and from the i6e tree [S: grep].
- **The camera kernel does not implement either.** Its `mtdchar_ioctl` (`ksyms` 0xC01B661C), run under unicorn from the
  first compare (0xC01B6684): `0x80084D33` and `0xC0084D34` both end at
  0xC01B66F8 = `mvn r4, #24` → **`-ENOTTY`**; `MEMGETINFO` (0x80204D01), `MEMERASE` (0x40084D02), `MEMGETBADBLOCK` (0x40084D0B)
  reach their case bodies. The literal pool holds no `0x…4D33` / `0x…4D34` word, and the bytes of `0x80084D33` occur nowhere in
  `Image` or the stock `uboot.bin` [R]. The kernel follows the i6e / vanilla command set here.
- Users: in the whole dump only `/bin/fwfs` holds the constant [R byte search; a `movw/movt`-built value would not be found
  this way — fwfs loads it from a literal]. fwfs issues it only for NAND MTDs (`MEMGETINFO.type` 4 `MTD_NANDFLASH` or 8 `MTD_MLCNANDFLASH`), so never on this NOR
  camera; on this kernel a NAND build would get `-ENOTTY` at its probe and run without a bad-block map [I]
  (`misc-firmwarefs.md` §4.4).

## 10. MSYS, DMEM and CPU helpers

### 10.1 `/dev/msys` [R decompile vs source]
<!-- ev:kernel-platform.10.1 -->
The ioctl set in the binary = source `ms_msys.c` built with DMEM, ADDR_TRANS, USER_TO_PHYSICAL, GET_UDID, FLUSH_CACHE and
PM_TSENSOR, and **without** MIU_PROTECT, SYSP strings, the tick API and PROC requests. Magic `'S'` (0x53), `_IO` numbers;
arguments are `MSYS_*_INFO` structs with a version header:

| cmd | name | cmd | name |
|---|---|---|---|
| 0x530F | FIND_DMEM_BY_NAME | 0x5315 | **GET_RIU_MAP** (riu_w) |
| 0x5310 / 0x5311 | REQUEST / RELEASE_DMEM | 0x5316 / 0x5317 | FIX / UNFIX_DMEM |
| 0x5312 | FLUSH_CACHE | 0x5319 | USER_TO_PHYSICAL (u32 in / out) |
| 0x5313 / 0x5314 | PHYS_TO_MIU / MIU_TO_PHYS | 0x5332 / 0x5378 | GET_UDID / READ_PM_TSENSOR (u32 out) |

User-space consumer on this camera: **only `riu_w` (0x5315)**. prog_uvc contains the strings `/dev/msys`,
`/sys/devices/system/miu/miu0/dram_info`, `/sys/devices/virtual/mstar/msys/CHIP_ID|CHIP_VERSION`, but no code references
them (has no hit) — they come with the statically linked cam_os_wrapper object [R strings;
I not called]. Kernel consumers: mi_sys (`msys_dma_copy/fill/blit`, `msys_read_uuid`), mhal (`msys_get_sysfs_class`) — 5
imports. sysfs class `mstar/msys` with `CHIP_ID` and `CHIP_VERSION` (§3.2).

### 10.2 DMEM [R log]
DMEM requests at boot (`MSYS: DMEM request: [name]:size … success, CPU phy:@…, virt:@…`):

| name | size | phys | requester |
|---|---|---|---|
| `emac0_buff` | 0x812 | 0x23846000 | EMAC 0 |
| `emac1_buff` | 0x60812 | 0x23850000 | EMAC 1 |
| `BDMA_FSP_WBUFF` | 0x10040 | 0x238C0000 | flash write path (§9) |
| `AESDMA_ENG` | 0x1000 | 0x23847000 | AES/SHA/RSA |
| `AESDMA_ENG1` | 0x1000 | 0x23848000 | AES/SHA/RSA |

The requesters are EMAC, BDMA_FSP and AESDMA. All lie in the 2 MiB CMA area at 0x23800000 (§11.1) [I: from the addresses]. `alloc_dmem` calls `msys_miu_client_switch` (the
"MIU_TEST" path of `msys_miu_ioctl`, §12.5). `CamOsMemPhysToVirt` looks DMEM entries up by physical address (§13.2).

### 10.3 Reboot type [R]
`msys_get_rebootType` (older API than i6c, which returns a name string) returns a number — 1 when the watchdog register's
bit 0 is set, else 3 − (bit 0 of phys 0x1F000C04 = 0006.01); the sysfs show `ms_dump_reboot_type` maps 3 "hardware reset",
2 "software reset", any other "watchdog reset".

### 10.4 Other [R]
- msys perf-test sysfs (`ms_msys_perf_test.c`, older revision with binary-only thread bodies): §2.3; MIU BIST `set_bist`:
  §12.5.
- `cpu/memory.c` exports `__aeabi_uldivmod` / `__aeabi_ldivmod` (64-bit division for mhal and mi_*; not in vanilla ARM;
  2 imports) and holds the boot-argument parsing of §11.

## 11. Physical memory map and the MMA heap

### 11.1 Layout on this camera [R log unless marked]
Command line from U-Boot (the kernel uses it; the DTB's `/chosen/bootargs` is stale):
`console=ttyS0,115200 root=/dev/mtdblock2 rootfstype=squashfs ro init=/linuxrc LX_MEM=0xffe0000
mma_heap=mma_heap_name0,miu=0,sz=0x9000000 mma_memblock_remove=1 cma=2M@0x23800000
mmap_reserved=fb,miu=0,sz=0x800000,max_start_off=0xF700000,max_end_off=0xFF00000` (the U-Boot side: `uboot.md` §2.3, §3.2). The stale
DTB line: `console=ttyS0,115200n8r androidboot.console=ttyS0 user_debug=31 root=/dev/mtdblock0 init=/linuxrc LX_MEM=0xfee0000
mma_heap=mma_heap_name0,miu=0,sz=0x5000000,max_start_off=0x28000000 mma_memblock_remove=1 cma=2M` [R DT]. The DT `memory`
node says 0x20000000 + 0x6000000 (96 MiB); `LX_MEM` overrides it. The DRAM is 256 MiB at 0x20000000 (MIU0 bus base;
`ipl.md` §3.9); MIU1 would be at bus base 0x2_0000_0000.

| physical range | size | what |
|---|---|---|
| 0x20000000 – 0x2FFE0000 | 0xFFE0000 (`LX_MEM`) | memory given to Linux (lowmem 0xC0000000 – 0xCFFE0000, 255 MB) |
| 0x23800000 – 0x23A00000 | 2 MiB | CMA (`cma=2M@0x23800000`; DMEM buffers of §10.2 come from it) |
| 0x26700000 – 0x2F700000 | 0x9000000 (144 MiB) | MMA heap `mma_heap_name0` (log `deal_with_reserve_mma_heap memblock_reserve success mma_config[0].reserved_start=0x26700000`); removed from the kernel's memory (`mma_memblock_remove=1`) |
| 0x2F700000 – 0x2FF00000 | 0x800000 (8 MiB) | `mmap_reserved` `fb` (log `deal_with_reserved_mmap memblock_reserve success mmap_reserved_config[0].reserved_start=0x2f700000`) |
| 0x2FFE0000 – 0x30000000 | 128 KiB | outside `LX_MEM`; its use is not known |

Log totals: `Memory: 98404K/114560K available (2504K kernel code, 214K rwdata, 1380K rodata, 172K init, 158K bss, 14108K
reserved, 2048K cma-reserved, 0K highmem)`; 114560K = `LX_MEM` − the heap [I arithmetic]. Virtual layout: vector
0xFFFF0000–0xFFFF1000, fixmap 0xFFC00000–0xFFF00000, vmalloc 0xD0000000–0xFF800000 (760 MB), lowmem 0xC0000000–0xCFFE0000,
pkmap 0xBFE00000–0xC0000000 (2 MB), modules 0xBF000000–0xBFE00000 (14 MB), `.text` 0xC0008000–0xC027A28C, `.init`
0xC03F7000–0xC0422000, `.data` 0xC0422000–0xC0457808, `.bss` 0xC0459000–0xC0480BF4. The first kernel print is
`DRAM_LEN= 0x0`. The mmap.ini items that carve the heap: `config_tool.md` §2.5; mi_sys's heap objects: `mi_sys-2-interfaces.md` §7.2.

### 11.2 Boot parameters and their parsers [R]
`mma_heap=`, `mma_memblock_remove=`, `mmap_reserved=` and `LX_MEM=` are vendor `early_param`s (i6c `mma_heap/
mdrv_mma_heap.c` + `cpu/memory.c`; called from the vendor-patched `arch/arm/mm/init.c`, with hooks in `mm/page_alloc.c`).
mi_sys reads the results directly as data: `mma_config[30]`, `mmap_reserved_config[30]`, `mstar_driver_boot_mma_buffer_num`,
`mstar_driver_boot_mmap_reserved_buffer_num`, `lx_mem{,2,3}_{addr,size}`.
- `parse_mmaheap_config` is the i6e revision (`max_start_off=%lx` / `max_end_off=%lx`).
- `deal_with_reserved_mmap` (DIFF against both trees): no `memblock_find_in_range`; the reservation starts exactly at bus
  base + `max_start_off` (`arm_add_memory` + `memblock_reserve`), the end offset is only checked for −1. With this camera's
  `mmap_reserved=fb,…` both the source and the binary form give 0x2F700000 [I arithmetic], as the log shows.

### 11.3 `struct MMA_BootArgs_Config` [R]
The **i6e layout** (two `unsigned long` offsets, one `phys_addr_t`), 56 B with LPAE: miu +0, size +4, name[32] +8,
max_start_off +40 (u32), max_end_off +44 (u32), reserved_start +48 (u64). Each array is 30 × 56 = 0x690 bytes [R sizes
from kallsyms gaps]; the i6c struct (`phys_addr_t` offsets) would be 64 B. Because mi_sys reads `mma_config[]` /
`mmap_reserved_config[]` directly, this layout is module ABI, and the 64-bit `reserved_start` is one of the proofs that the
kernel is built with LPAE (§15.1).

## 12. MIU: driver, arbiter, BIST, protect

### 12.1 Drivers [R]
<!-- ev:kernel-platform.12.1 -->
- MIU / MMU: i6c `miu/mdrv_miu.c` + `mercury6/mhal_miu.c` (27 / 40 hal functions; the absent ones are client-name helpers
  and BW / reset). mi_sys imports 16 of them (`MDrv_MIU_Init/Protect/…`, `MDrv_MMU_*`) plus the data `u8_MiuWhiteListNum`
  (in `mdrv_miu.c`). DT nodes `miu` (`sstar,miu`, interrupt 0x51) and `mmu` (`sstar,mmu`, interrupt 0x2E). The sysfs node
  `dram_info` (`/sys/devices/system/miu/miu0/dram_info`) has no reader found on the camera.
- Arbiter: built into mach-sstar, not the separate i6c driver files `drivers/sstar/miu/mdrv_sysfs_{arb,bw}.c`
  (`mdrv_sysfs_bw.c` has no counterpart in the image). Its sysfs handlers follow i6c `mdrv_sysfs_arb.c` + `mercury6/mhal_arb.c` (inlined;
  `group_member_burst`, `client_max_mbr_serv_idx`); its client table and helpers carry the i6e `miu_bw.c` names
  (`miu_clients`, `miu_client_id_to_name`, `set_miu_client_enable`); data `miu_clients`, `arb_policy_def`, code
  `_load_policy` (C body `_load_policy.part.0.constprop.0`, checked by hand), `_get_client_burst`, `HAL_MIU_ClientIdToName`.
  References read with it: i6c `drivers/sstar/miu/mercury6/{mhal_arb.[ch], mhal_miu.c, regMIU.h}`, `msys/mercury6/
  platform_msys.c` (BIST setup with register comments), i6e `include/infinity2/regMIU.h` ("i2"), the IPL DRAM-init code and
  the kernel's `sram.S` (scripts `miuclients.py` →, `ipldis.py`, `iplwrites.py`).
- Functions without source in either tree: `_get_client_burst`; `set_miu_client_enable`, `miu_client_reserved`,
  `miu_client_id_to_name` (i6e `miu_bw.c` style, M6 table); `msys_miu_client_switch` (§12.5); `set_bist` (§12.5).

### 12.2 Client → group table [R, image `miu_clients` @0xC027C718, 68 × {name, id, rsvd}]
Client id = group × 16 + member. Identical to i6c `mercury6/mhal_miu.c` for ids 0x00–0x3F; the image adds the four
high-way ids. `HAL_MIU_ClientIdToName` (MIU-protect messages) uses the same ids with `_RW` / `_R` names (e.g. 0x19
"LDCFeye_RW", 0x25 "ISP_MLOAD_STA", 0x23 / 0x24 "MOT0_W/R", 0x26 "DISP3_R", reserved slots "DUMMY_GxCy").

| member | group 0 | group 1 | group 2 | group 3 |
|---|---|---|---|---|
| 0 | OVERALL | USB30 | VCODEC_R | RSVD (rsvd) |
| 1 | GE | USB20 | VCODEC_W | ISP_DMA_W |
| 2 | BDMA | USB20_H | ISP_ROT_R | ISP_DMA_R |
| 3 | MOVDMA | SD30 | ISP_MOT0W | 3DNR0_W |
| 4 | AESDMA | SDIO30 | ISP_MOT0R | 3DNR0_R |
| 5 | CMDQ0_R | RSVD (rsvd) | ISP_MLOAD | RSVD (rsvd) |
| 6 | CMDQ1_R | SATA | GOP_DISP3 | PQ_VIP_DISP_W (rsvd) |
| 7 | URDMA | EMAC | RSVD (rsvd) | DISP_CVBS_W (rsvd) |
| 8 | CSI_TX_R | EMAC1 | RDMA_DIP | ISP_DMAG |
| 9 | JPE_R | LDC | SC0_FRM_R | GOP_DISP0 |
| 10 | JPE_W | GOP | SC0_FRM_W | GOP_DISP1 |
| 11 | JPD | GOP1_R | SC1_FRM_W | GOP_DISP2 |
| 12 | BACH | GOP2_R | SC2_FRM_W | MOPROT0_Y |
| 13 | BACH1 | GOP3_R | SC3_FRM_RW | MOPROT0_C |
| 14 | IVE | GOP4_R | SC4_FRM_RW | MOPROT1_Y |
| 15 | MCU51 | GOP5_R | SC5_FRM_RW | MOPROT1_C |

High way (ids 0x70–0x73): 0x70 CPU_W, 0x71 DLA_HIWAY_R, 0x72 DLA_HIWAY_W, 0x73 CPU_R.

So group 2 = video encoder (VCODEC_R / W), ISP rotate / motion / MLOAD, GOP_DISP3, DIP, scaler frame DMA SC0–SC5; group 3 =
ISP DMA, 3DNR, ISP_DMAG, GOP_DISP0–2 and the display MOP channels MOPROT0 / 1 Y / C. The table is read from the image only;
a cross-check on the running camera (`/sys/devices/system/miu/miu0/`, CAMERA-CHECKS K12) has not been made.

### 12.3 Banks and the per-group register map
| bank | role | evidence |
|---|---|---|
| 0x1010 | MIU ATOP (DDR PHY) | IPL training writes; i6c `BASE_REG_ATOP_PA` [R / S] |
| 0x1011 | MIU_ARB: arbiter groups 4 / 5 at regs 0x00 / 0x10 (same layout as below) and the "normal way / high way" cut-length and way-priority registers 0x70–0x7C | IPL writes cfg 0x8015 to 1011.00 / .10 and mask 0xFFFF to 1011.03 [R]; i2 `MIU_RQX_*` put groups ≥ 4 in the ARB bank [S]; BIST comments for 0x71 / 0x75 / 0x76 / 0x77 / 0x7C [S] |
| 0x1012 | MIU0 digital: DRAM controller 0x00–0x1F, **arbiter groups 0–3 at 0x20 + 0x10·g**, BIST / test 0x70–0x78, protect, outer-group cfg 0x7F | i6c `mhal_arb.h` [S], IPL [R] |
| 0x1013 | MIU_E: **extended arbiter regs 0x30 + 4·g** (+ MMU protect regs elsewhere in the bank) | i6c `BASE_REG_MIU_E_PA` = 0x101300, `REG_IGRP_BURST_EX23(g)` = 0x30 + 4g [S]; the image's `_load_policy` writes 0x1F2026C0 + 0x10·g [R] |

Group g register (bank 0x1012 for g = 0–3, reg 0x20 + 0x10·g + offset; groups 4 / 5 in bank 0x1011 at 0x00 / 0x10):

| off | name | meaning | source |
|---|---|---|---|
| +0x0 | cfg | bit 0 round-robin, bit 1 fixed priority, bit 2 member limit en, bit 3 group limit en, bit 8 flowctrl0 en, bit 9 flowctrl1 en; **bits 4 and 15: named in no available source** (i6c / i6e `IGCFG_*`, i2 / i7 / M6P headers) | [S] |
| +0x1 | burst | 7:0 member burst, 15:8 group burst | [S] |
| +0x3 | request mask | bit per member, 1 = requests blocked | i2 `MIU_RQX_MASK` (byte 0x46 + 0x20·g) [S]; `sram.S` writes 0xFFFE / 0xFFFF here before self-refresh, IPL masks during DRAM init [R] |
| **+0x4** | **high-priority mask** | bit per member | **i2 `MIU_RQX_HPMASK` (byte 0x48 + 0x20·g)** [S]; no driver in any tree reads or writes it, so the bit polarity is not known |
| +0x9 | — | IPL writes 1012.29 = 1012.39 = 1012.49 = 0xFFFF, 1012.59 = 0, 1011.09 = 1011.19 = 0xFFFF once during DRAM init and never changes them; meaning not named | [R emulation, `ipl.md` §3.6] |
| +0xA / +0xB | flowctrl0 / 1 | 3:0 id0, 7:4 id1 (members), 15:8 period | [S] |
| +0xC | member priority | bit per member (used when cfg bit 0) | [S] |
| +0xE | member no-limit | bit per member, exempt from the member burst limit | [S] |

Extended (bank 0x1013, reg 0x30 + 4·g): +0 `burst_23` (7:0 burst #1, 15:8 burst #2), +1 `burst_4` (7:0 burst #3), +2
`mbr_sel0` (members 0–7), +3 `mbr_sel1` (members 8–15): 2 bits per member choosing the burst length — 0 = the group's member
burst (+0x1 bits 7:0), 1 = `burst_23` low byte, 2 = `burst_23` high byte, 3 = `burst_4` low byte [R image `_get_client_burst`;
same as i6c `halARBGetClientBurst`]. 1012.7F: outer-group cfg (bit 8 fixed priority, bit 12 round robin, 2-bit group
priorities; bit 11 written by the driver, "enable tv-tool read/write" in i6c) [S / R].

Way registers of bank 0x1011 [S: comments in i6c `mercury6/platform_msys.c`]: 0x71 "enable high way, normal way cutted
length", **0x75 "set normal way cutted length"**, 0x76 / 0x77 "high way cutted length", 0x7C "normal way high priority than
high way". "Normal way" = the group arbiter (groups 0–5); "high way" = the CPU / DLA path, ids 0x70–0x73 [R names; I mapping,
as in i7's "normal way group + high way group"]. The comments' numbers fit one byte = length / 4 per byte (0x80 → 512,
0x40 → 256, 0x08 → 32) [I]: IPL's 0x8008 = 512 | 32, riu_w's 0xA008 = 640 | 32 — the riu_w write raises the high-byte
length by a quarter; which byte is read and which written is not stated [I].

### 12.4 Arbiter state at boot [R]
<!-- ev:kernel-platform.12.4 -->
1. **IPL** (DRAM init, NOR 0x4800–0x48B0; `ipl.md` §3.6, §3.8): first all six group cfgs = 0x8015 and all request masks closed; after
   the MIU PLL: groups 0–3 cfg 0x801D; bursts 0x1008 / 0x2010 / 0x4020 / 0x2010; 1012.4C (group 2 priority) 0xFFFC; 0x1013
   `mbr_sel` of every group 0; 1013.4F 0x002C; 1011.70 0x0301, .71 0x007E, .75 **0x8008**, .76 0x0402, .77 0x0204, .7A 0x8054,
   .7C 0xFFCD; 1012.7F 0x9000. No write to any +0x4 (high-priority mask) register.
2. **Kernel** `create_miu_arb_node` (always, at boot; the `Chip_Get_Package_Type` result is ignored) registers `miu_arb0`
   (`/sys/devices/system/miu/miu_arb0`) with the sysfs files `client`, `group`, `group_burst`, `group_member_burst`,
   `client_nolimit`, `client_max_mbr_serv_idx`, `client_priority`, `client_flowctrl`, `dump`, then loads `arb_policy_def`
   (0xC043A364) — **which differs from the i6c source policy** (differences in bold):

| group | cfg | burst | flowctrl0 | prio | no-limit | 0x1013 burst_23 / burst_4 / sel0 / sel1 |
|---|---|---|---|---|---|---|
| 0 | **0x811D** | 0x1008 | **0xFF22** (BDMA, period 0xFF) | 0x1000 | 0 | 0x2010 / 0x0040 / 0 / 0 |
| 1 | 0x801D | 0x2010 | 0 | 0 | 0 | same |
| 2 | **0x811D** | 0x4020 | **0xFF88** (RDMA_DIP, period 0xFF) | 0xFFFC | 0 | same |
| 3 | 0x801D | 0x2010 | 0 | **0xFFFD** | 0 | same |

   plus 1012.7F = 0x9000 | 0x800. The same loader runs on resume (`miu_arb_resume`).
3. **riu_w** (`/etc/init.d/rcS`; the write list: `rootfs-customer.md` §5.2) then changes groups 2 / 3 — net effect against the
   kernel state, with client names [R values + table]:

| register | kernel → riu_w | meaning |
|---|---|---|
| 1012.44 group 2 HP mask | reset → **0x03FF** | members 0–9: VCODEC_R, VCODEC_W, ISP_ROT_R, ISP_MOT0W, ISP_MOT0R, ISP_MLOAD, GOP_DISP3, (rsvd), RDMA_DIP, SC0_FRM_R |
| 1012.4A group 2 flowctrl0 | 0xFF88 → **0x4010** | flow control on VCODEC_R (id0 0) and VCODEC_W (id1 1), period 0x40 (was RDMA_DIP, 0xFF) |
| 1012.40 / .41 | unchanged in net (0x811D, 0x4020) | — |
| 1012.50 group 3 cfg | 0x801D → **0x811D** | flowctrl0 enabled |
| 1012.51 group 3 burst | 0x2010 → **0x3018** | group burst 0x30, member burst 0x18 |
| 1012.54 group 3 HP mask | reset → **0xFFFD** | all members except 1 (ISP_DMA_W) |
| 1012.5A group 3 flowctrl0 | 0 → **0x8022** | flow control on ISP_DMA_R (id0 = id1 = 2), period 0x80 |
| 1012.5C group 3 priority | 0xFFFD → **0x0FFF** | members 0–11 set, MOPROT0 / 1 Y / C (12–15) cleared |
| 1012.5E group 3 no-limit | 0 → **0xF000** | MOPROT0_Y, MOPROT0_C, MOPROT1_Y, MOPROT1_C exempt from the member burst limit |
| **1013.3F** | 0 → **0xF000** | **group 3 `mbr_sel1`: members 14, 15 (MOPROT1_Y, MOPROT1_C) take burst #3 = 1013.3D bits 7:0 (0x40 after the kernel policy)** |
| **1011.75** | 0x8008 → **0xA008** | **normal-way cut length** (§12.3) |

   So the riu_w block is a display-bandwidth change: it gives the display MOP read channels unlimited / longer bursts and
   throttles VENC and ISP DMA reads with flow control — the FAQ's "VENC outranks DISP; adjust MOP burst" [R values + table;
   I reading of intent]. Whether a 1 in the priority / HP-mask registers raises or lowers a member is stated by no source
   (not closable statically; every vendor write keeps cfg bits 4 and 15 set: IPL 0x801D / 0x8015, kernel 0x801D / 0x811D,
   riu_w 0x801D / 0x811D [R values; I that they are hardware-mode bits]). Whether the intermediate riu_w writes matter is not
   known (a camera reading).
4. **Measured on the running camera** [R]: the group 0 HP mask (1012.24, 0x1F202490) reads **0xFFFF after
   boot** — although neither IPL, the kernel policy nor riu_w writes it; writing 0 there brings back ≈45 green frames per
   10 s in the 4K pipeline (restored by a reboot). Changes of SC0_FRM_W priority, JPE_R flow control, the group 2 / 3 HP
   masks and USB20 priority / burst no-limit through `miu_arb0` had no measurable effect on the scaler / USB limits of that
   pipeline. A full read of banks 1010 / 1012 / 1013 after boot, to compare with the IPL values, has not been made
   (CAMERA-CHECKS K11).

### 12.5 BIST and the client switch [R decompile vs source]
- `msys_miu_client_switch`: 1012.7F bit 11 — clear = BIST, set = "TV tool".
- `set_bist` (sysfs, `"%d %d"` = enable, percent < 99): enable ≠ 0 → 1012.69 &= 0xF000, 1011.71 = 0x001E, 1011.75 = 0x8080,
  1011.76 = 0x4040, 1011.77 = 0x0808, 1011.7C = 0xFFC0, 1012.5F = 0x4040, 1012.73 = 0, 1012.72 = percent × 5, 1012.71 = 0,
  1012.20 bit 0 cleared (group 0 round robin off); enable = 0 → only 1011.76 = 0x0040; then `msys_miu_client_switch(enable
  == 0)` and a print. Against i6c `msys_miu_set_bist` the image **omits** the test-mode enable 1012.70 = 0x11, 1012.20
  bit 1, 1012.25 = 0x301F, 1012.28 = 0x2EDC and the 1012.24 clear, and uses 0 / percent × 5 / 0 instead of 0x7E00 / 0x0100 /
  0x0080 for 0x71–0x73. Nothing on the camera uses it.

### 12.6 MIU protect
The kernel side offered to mi_sys: `MDrv_MIU_Protect(block, whitelist, start, end, enable)`,
`MDrv_MIU_GetDefaultClientID_KernelProtect()`, `u8_MiuWhiteListNum` (i6c `mdrv_miu.c` / `mercury6/mhal_miu.c`, bodies the
source) [R]; violation messages name the client through `HAL_MIU_ClientIdToName` (§12.2). The protect registers lie in bank
0x1012 and the MMU protect registers in bank 0x1013 [S: i6c headers]; their field-level layout is not specified. How mi_sys uses protect (kernel ranges, module windows, the `miuprotect.txt` switch): `mi_sys-2-interfaces.md` §7.7.

## 13. CamOs, kernel side

### 13.1 Files and users [R]
`cam_os_wrapper/src/cam_os_wrapper.c` and its helpers `cam_sysfs/`, `cam_fs_wrapper/`, `cam_proc_wrapper/` (i6c only),
`cam_clkgen/`, `cam_drv_buffer/`, `cam_drv_poll/`, plus `CamI2cOpen/Close/Transfer` (i6c `cam_drv_i2c.c`). mhal imports 113
symbols from these (CamOs 74, cam_sysfs 22, cam_clkgen 12, cam_fs 5; 131 BSP imports in all); all vendor modules together
145; fbdev / mi_common / mi_sys use a few `CamClass/Device*`. All imported functions have source. `CamOsVersion` returns
**`"v1.0.42"`** (i6e tree "v1.0.35", i6c "v2.0.43"). The user-space `libcam_os_wrapper` is `mi-user-abi.md` §7's.

### 13.2 Revision per function [R: tool + reading; table: function, file, revision, importing modules]
<!-- ev:kernel-platform.13.2 -->
"Same in both" = the two trees' bodies are token-identical.

| file (functions) | same in both | i6e | i6c | indistinguishable |
|---|---|---|---|---|
| `cam_os_wrapper.c` (153 + `CamOsMemPhysToVirt` without source) | 99 | 9 + 12 only-in-i6e | 21 only-in-i6c | 12 |
| `cam_sysfs.c` (37) | 25 | 1 | 8 only-in-i6c | 3 |
| `cam_fs_wrapper.c` (7) | 5 | — | 2 only-in-i6c | — |
| `cam_proc_wrapper.c` (10) | — | — | 10 (file only in i6c) | — |
| `cam_clkgen` (25), `cam_drv_buffer` (13), `cam_drv_poll` (5) | all | — | — | — |

- Follows **i6e** where the trees differ: `CamOsAtomicAndFetch`, `…FetchAnd`, `…FetchNand`, `…FetchOr`, `…FetchXor`,
  `…NandFetch`, `…OrFetch`, `…XorFetch` (they print "%s null atomic handle, LR:0x%08X" on a NULL handle — i6c returns
  silently), `CamOsHexdump` (i6c's extra "%08X  " address column absent), `CamDeviceCreate` (`device_create_vargs`).
- Only-in-i6e functions present: `CamOsDirectMemAlloc/Release/Stat/PhysToMiu/MiuToPhys/PhysToVirt/VirtToPhys`,
  `_CheckDmemInfoListInited`, `CamOsMemRealloc`, `CamOsMemReallocAtomic`, `CamOsPropertyGet/Set`.
- Only-in-i6c functions present: `CamOsCallStack`, `CamOsGetCpuOnline`, `CamOsGetHz`, `CamOsGetJiffies`, `CamOsGetTimeInMs`
  (4.9 glue: `getrawmonotonic64` for `ktime_get_raw_ts64`), `CamOsIrqSetAffinityHint`, `CamOsJiffiesDiff`,
  `CamOsJiffiesToMs`, `CamOsMsToJiffies`, `CamOsMemFromUserModeMap/Unmap`, `CamOsMemIsUserSpace`, `CamOsMemMiuToPhys`,
  `CamOsMemPhysToMiu`, `CamOsMemVirtToPhys`, `CamOsWorkQueueCreate/Destroy/Add/Cancel`, `_CamOsWorkQueueProcThread`,
  `_CmaOsWorkQurueDelayCb` (DIFF, §13.3); `CamDevicePropertyReadString`, `CamDeviceRegister`, `CamGetUserPagesFast`,
  `CamPlatformDeviceAdd/Alloc/Put/Register/Unregister`; `CamFsMount/Unmount`; the whole `cam_proc_wrapper.c` (`CamProcCreate`,
  `CamProcMkdir`, `CamProcRemoveEntry`, `_CamProcCreateEntryNode`, `_CamProcFindEntry`, `_CamProcFindEntryByNameEx`,
  `_CamProcOpenLinux`, `_CamProcShowLinux`, `_CamProcWriteLinux`, `_CamProcRemoveAllSubNode`); `CamI2cOpen/Close/Transfer`.
- Indistinguishable (inline atomics or wrappers whose calls match both): `CamOsAtomicRead/Set/AddReturn/SubReturn/
  IncReturn/DecReturn/IncAndTest/DecAndTest/SubAndTest/AddNegative/CompareAndSwap`, `CamOsIrqCommonHandler`,
  `CamDeviceDestroy`, `CamDeviceRemoveFile`, `CamDeviceUnregister`.
- Without source: `CamOsMemPhysToVirt` — finds the DMEM entry with `msys_find_dmem_by_phys` and returns phys + (virt − phys)
  of that entry, else prints "CamOs WARNING: PhysToVirt not found 0x%08X in msys" and returns 0; no vendor module imports it.

So the binary's CamOs is one file revision that has both trees' additions: the i6e-era DirectMem / Property API and atomic
NULL prints, plus the i6c-era work queue, jiffies / time helpers, user-mode mapping and proc wrapper. Imports of the
non-"same" functions by the vendor modules [R module import tables]: mhal uses the DirectMem set, `CamOsHexdump`,
`CamOsIrqSetAffinityHint`, `CamOsMemPhysToMiu`, `CamI2c*`; mi_sys uses the work queue, the jiffies helpers,
`CamOsCallStack`, `CamOsMemFromUserModeMap/Unmap`.

### 13.3 Delayed work [R decompile vs source]
`CamOsWorkQueueAdd(…, delay ≠ 0)` queues a delayed item (status 2); when its timer fires, `_CmaOsWorkQurueDelayCb` runs
the item's function **inside the timer callback** (status 3, then cleared under the queue lock). In the i6c source the
callback only marks the item ready and wakes the queue thread, which runs it in thread context. Immediate items (delay 0)
go through the thread in both. mi_sys imports `CamOsWorkQueueAdd/Create/Destroy`; whether mi_sys passes a non-zero delay is
not known from these sources.

## 14. Device tree

The DTB is built into the vendor image (U-Boot `bootm` of a uImage); it sits at Image+0x421fa0 [R]. Root: `model = "MERCURY6 SSC019A-S01A-S"`,
`compatible = "sstar,mercury6"`; aliases `console` / `serial0` = `uart@1F221000`, `serial1` = `uart1@1F221200`, `serial2` =
`fuart@1F220400`, `serial3` = `uart2@1F221400`, `serial4` = `uart3@1F221600`. `/chosen/bootargs` is stale and `memory` says
96 MiB (§11.1). Vendor platform drivers and the vendor modules bind by `compatible` and use `reg`, `interrupts`, `clocks`
and vendor properties as they stand; mhal registers its platform drivers through `CamPlatformDriverRegister` and reads
properties with `CamOfPropertyRead*` (the per-node property list: `mhal-core.md` §7).

Enabled but unused on this camera [R]: `emac0/1` (two MACs probe and scan 32 PHY addresses each at every boot; link always
down), `sata` and `dwc3` (no `status` property = enabled; no SATA disk; no dwc3 driver in the kernel), `ive`, `aesdma`,
`i2c3/goodix_gt911` (no `status`; probes and fails with 3 I2C timeouts per boot), `emmc`, `spinandflash`, `i2c4/gyro`.

Nodes (clock nodes: §5.1). Interrupt column: the DT number n of `<0 n 4>` to `ms_main_intc` unless marked (PPI / GIC =
direct to the GIC; pm-intc / gpi-intc = the one-cell PM / GPI controllers). Status: `ok` = `status = "ok"`, (none) = no
`status` property, which also means enabled:

| node | compatible | reg (base+size) | interrupts | status |
|---|---|---|---|---|
| `memory` | (`device_type = "memory"`) | 0x20000000+0x6000000 | — | (none) |
| `cpus/cpu@0` | `arm,cortex-a53` | 0x0 | — | (none) |
| `cpus/cpu@1` | `arm,cortex-a53` | 0x1 | — | (none) |
| `phy` | `sstar,sstar-usb-phy` | 0x1F2A4600+0x300 | — | ok |
| `gic@16000000` | `arm,gic-v3` | 0x16000000+0x20000, 0x16040000+0x20000 | — | (none) |
| `ms_main_intc` | `sstar,main-intc` | — | — | (none) |
| `ms_pm_intc` | `sstar,pm-intc` | — | 0x22 | (none) |
| `ms_gpi_intc` | `sstar,gpi-intc` | — | 0x58 | (none) |
| `ms_timer` | `sstar,infinity-timer` | — | — | (none) |
| `arch_timer` | `arm,cortex-a53-timer, arm,armv8-timer` | — | PPI 0xD, PPI 0xE, PPI 0xB, PPI 0xA (GIC) | (none) |
| `pmu` | `arm,cortex-a53-pmu` | — | 0xA, 0x10, 0x16, 0x1C (GIC) | (none) |
| `venc` | `sstar,venc` | 0x1F345200+0x800, 0x1F2C5200+0x100, 0x1F203C00+0x100, 0x1F207800+0x100, 0x1F2C5200+0x100 | 0x35 | ok |
| `isp` | `isp` | — | 0x39 | ok |
| `csi` | `sstar,csi` | — | 0x41 | ok |
| `sound` | `sstar,audio` | — | 0x4A | ok |
| `vif` | `sstar,vif` | 0x1F260800+0x800, 0x0+0x0, 0x0+0x0, 0x0+0x0, 0x0+0x0, 0x1F207C+0x200, 0x1F207800+0x200, 0x1F227400+0x200, 0x1F207000+0x200, 0x1F000000+0x400000, 0x1F203C00+0x200, 0x1F260600+0x200 | 0x59 | ok |
| `ispalgo` | `sstar,ispalgo` | — | — | ok |
| `ispscl` | `sstar,ispscl` | — | 0x71 | ok |
| `ispmid` | `sstar,ispmid` | — | — | ok |
| `sensorif` | `sstar,sensorif` | — | — | ok |
| `jpe@0x1F2c4000` | `sstar,cedric-jpe` | 0x1F2C4000+0x100 | 0x3D | ok |
| `jpd@0x1F2c4400` | `sstar,njpd` | 0x1F2C4400+0x100 | 0x4C | ok |
| `ge0` | `sstar,ge0` | — | 0x76 | ok |
| `spi0@0` | `sstar,mspi` | 0x1F222000+0x200 | 0x46 | ok |
| `spi1@1` | `sstar,mspi` | 0x1F222200+0x200 | 0x47 | ok |
| `spi2@2` | `sstar,mspi` | 0x1F222400+0x200 | 0x70 | ok |
| `i2c0@0` | `sstar,i2c` | 0x1F223000+0x200, 0x1F207800+0x200, 0x1F207000+0x200 | 0x44 | ok |
| `i2c1@1` | `sstar,i2c` | 0x1F223200+0x200, 0x1F207800+0x200, 0x1F207000+0x200 | 0x45 | ok |
| `i2c2@2` | `sstar,i2c` | 0x1F223400+0x200, 0x1F207800+0x200, 0x1F207000+0x200 | 0x2C | ok |
| `i2c3@3/goodix_gt911@5D` | `goodix,gt911` | 0x5D | pm-intc 8 | (none) |
| `i2c3@3` | `sstar,i2c` | 0x1F223800+0x200, 0x1F207800+0x200, 0x1F207000+0x200 | 0x6D | ok |
| `i2c4@4/gyro@d0` | `sstar,gyro` | 0x69 | — | (none) |
| `i2c4@4` | `sstar,i2c` | 0x1F223A00+0x200, 0x1F207800+0x200, 0x1F207000+0x200 | 0x6E | ok |
| `i2c5@5` | `sstar,i2c` | 0x1F223C00+0x200, 0x1F207800+0x200, 0x1F207000+0x200 | 0x6F | ok |
| `cmdq0` | `sstar,cmdq0` | — | 0x31 | ok |
| `cmdq1` | `sstar,cmdq1` | — | 0x31 | ok |
| `cmdq2` | `sstar,cmdq2` | — | 0x31 | ok |
| `cmdq3` | `sstar,cmdq3` | — | 0x31 | ok |
| `cmdq4` | `sstar,cmdq4` | — | 0x77 | ok |
| `cmdq5` | `sstar,cmdq5` | — | 0x77 | ok |
| `cmdq6` | `sstar,cmdq6` | — | 0x77 | ok |
| `cmdq7` | `sstar,cmdq7` | — | 0x77 | ok |
| `ldc` | `sstar,ldc` | 0x1F25E000+0x100, 0x1F25E200+0x100 | 0x4B | ok |
| `scl` | `sstar,scl` | — | — | ok |
| `vpe` | `sigmastar,vpe` | — | 0x34, 0x5A, 0x5B, 0x5F | ok |
| `dip` | `sstar,dip` | — | 0x5F | ok |
| `dla` | `sstar,dla` | — | 0x55 | ok |
| `pnl` | `sstar,pnl` | — | — | ok |
| `disp` | `sstar,disp` | — | 0x63, 0x67, 0x64, 0x68, 0x6A, 0x6C | ok |
| `rgn` | `sstar,rgn` | — | — | ok |
| `hdmitx` | `sstar,hdmitx` | — | — | ok |
| `mipitx_csi` | `sstar,mipitx` | — | 0x56 | ok |
| `uart@1F221000` | `sstar,uart` | 0x1F221000+0x100 | 0x42 | ok |
| `uart1@1F221200` | `sstar,uart` | 0x1F221200+0x100 | 0x43 | ok |
| `fuart@1F220400` | `sstar,uart` | 0x1F220400+0x100, 0x1F220600+0x100 | 0x4F, 0x50 | ok |
| `uart2@1F221400` | `sstar,uart` | 0x1F221400+0x100 | 0x72 | ok |
| `uart3@1F221600` | `sstar,uart` | 0x1F221600+0x100 | 0x73 | ok |
| `emmc` | `sstar_mci` | — | 0x32 | ok |
| `flashisp` | `mtd-flashisp` | — | — | ok |
| `spinandflash` | `ms-spinand` | — | — | ok |
| `sdmmc` | `sstar,sdmmc` | — | 0x32, 0x33, 0x97, 0x98, 0x97 (interrupts-extended, main intc) | ok |
| `bdma0` | `sstar,bdma0` | — | 0x48 | ok |
| `bdma1` | `sstar,bdma1` | — | 0x49 | ok |
| `bdma2` | `sstar,bdma2` | — | 0x5D | ok |
| `bdma3` | `sstar,bdma3` | — | 0x5E | ok |
| `movdma` | `sstar,movdma` | — | 0x5C | ok |
| `sata` | `sstar,sata, sstar,sata-ahci` | 0x1F346800+0x100, 0x1F346900+0x100, 0x1F287400+0x200 | 0x38 | (none) |
| `cpufreq` | `sstar,infinity-cpufreq` | — | — | ok |
| `watchdog` | `sstar,infinity-wdt` | 0x1F006000+0x40 | — | ok |
| `rtcpwc` | `sstar,rtcpwc` | 0x1F006800+0x200 | 0x27 | ok |
| `sar` | `sstar,infinity-sar` | 0x1F002800+0x200 | — | ok |
| `gop` | `sigmastar,gop` | — | — | ok |
| `gop1` | `sstar,gop1` | — | — | ok |
| `gop2` | `sstar,gop2` | — | — | ok |
| `ive@0x1F232000` | `sstar,infinity-ive` | 0x1F232000+0x100, 0x1F232200+0x100 | 0x4D | ok |
| `emac0` | `sstar-emac` | 0x1F2A2000+0x800, 0x1F343C00+0x600, 0x1F2A3000+0x0 | 0x3A, 0x95, 0x79 | ok |
| `emac1` | `sstar-emac` | 0x1F2A2800+0x800, 0x1F345A00+0x600, 0x0+0x0 | 0x78, 0x95, 0x7A | ok |
| `gpio` | `sstar,gpio` | — | — | ok |
| `gpic0` | `sstar,gpic` | 0x1F207B80+0x10 | gpi-intc 0x82 | (none) |
| `gpic1` | `sstar,gpic` | 0x1F207B90+0x10 | gpi-intc 0x83 | (none) |
| `ir@1F007A00` | `sstar,infinity-ir` | 0x1F007A00+0x1000 | 0x8B, 0x84 | ok |
| `pwm` | `sstar,infinity-pwm` | 0x1F203200+0x400 | 0x54 | ok |
| `aesdma` | `sstar,infinity-aes` | — | 0x37 | ok |
| `miu` | `sstar,miu` | — | 0x51 | ok |
| `mmu` | `sstar,mmu` | — | 0x2E | ok |
| `timer` | `sstar,timer` | 0x1F006040+0x40, 0x1F006080+0x40, 0x1F0060C0+0x40 | 0x80, 0x81, 0x83 | ok |
| `cpu_power` | `sstar,voltage-ctrl` | — | — | ok |
| `core_power` | `sstar,voltage-ctrl` | — | — | ok |
| `dla_power` | `sstar,voltage-ctrl` | — | — | ok |
| `Sstar-ehci-1` | `Sstar-ehci-1` | — | 0x3F | ok |
| `Sstar-ehci-2` | `Sstar-ehci-2` | — | 0x61 | ok |
| `Sstar-udc` | `sstar,infinity-udc` | — | 0x40 | ok |
| `dwc3` | `snps,dwc3` | 0x1F344200+0xCFFF | 0x38 | (none) |
| `padmux` | `sstar-padmux` | — | — | ok |
| `ipu` | `sstar,ipu` | — | — | ok |

Further properties of the platform nodes [R]: `phy` — `io_phy_addr = 0x1F000000`, `banks = <0x1523 0x1524 0x1525>`,
`phy_type = "utmi"`; `rtcpwc` — `alarm-enable`, `io4-enable = 3`, `io5-enable = 2`; `pwm` — `npwm = 11`, `pad-ctrl = <6 7 8 9
0x73 0x74 0x28 0x20 0x88 0x89 0x8A>`; `emac0` — `bus-mode = 1`, `pad = <0x1F2079B8 0xF 0>`, `pad-rmii = <0x1F2079B8 0xF 4>`,
PHY `mii`; `emac1` — `pad = <0x1F2079B8 0xF0 0x40>`, PHY `rmii`; `sata` — `phy_mode = 2`, `phy_setting = 0x4200`,
`ssc_mode = 0`; `dwc3` — `maximum-speed = "super-speed"`, `phy_type = "utmi_wide"`, `phys = <phy phy>`; `Sstar-ehci-1/2` —
`dpdm_swap = 0`; `emmc` — `bus-width = 4`; `flashisp` — `quadread = 0`; `ipu` — `operating-points` (kHz / µV) 1 000 000 /
1 000 000, 900 000 / 1 000 000, 800 000 / 1 000 000, 700 000 / 1 000 000, 600 000 / 1 000 000, 500 000 / 1 000 000,
400 000 / 1 000 000, 300 000 / 1 000 000; voltage nodes §5.4; `padmux` §6.2; I2C §8; UARTs §7. The meaning of these
properties belongs to the documents of the drivers that read them.

## 15. Kernel configuration and vendor changes to generic kernel files

### 15.1 Configuration (as far as it shapes the module ABI) [R]
| item | vendor kernel | evidence / effect |
|---|---|---|
| **ARM_LPAE** | **on** | `__pmd_alloc` present, `remap_area_sections` absent, `MMA_BootArgs_Config` entry 56 B (§11.3); `phys_addr_t` and `resource_size_t` are 64-bit, so `struct resource` and phys arguments shared with the modules have the LPAE layout; **`dma_addr_t` stays 32-bit** (4.9 ARM selects `ARCH_DMA_ADDR_T_64BIT` only for XEN; `usb_f_uvc.ko` has a 4-byte `usb_request.dma`, §15.3) |
| **CC_STACKPROTECTOR** | **on** | `__stack_chk_guard` exported; 23 vendor modules import it (they do not load on a kernel without it: "Unknown symbol __stack_chk_guard"); REGULAR or STRONG not determined (both export the symbol) |
| HIGHMEM | on | `pkmap` in the boot memory layout, `kmap_high` in kallsyms (0K highmem on this board); no vendor module imports `kmap` |
| HZ | 100 | `jiffies_to_msecs` = ×10 |
| SMP, PREEMPT, THUMB2, AEABI, NR_CPUS 4, VMSPLIT_3G, MODULE_UNLOAD; no MODVERSIONS | | vermagic, boot log; without MODVERSIONS the modules bind to kernel exports by name only |
| built in, used | devtmpfs, squashfs (xz), jffs2, fuse, MTD + mtdblock, SCSI + sd (insmod of `scsi_mod` / `sd_mod` fails because they are built in), media + videodev + v4l2-common (built in; their `.ko` files are rejected), RC core + evdev, i2c-dev, gpiolib sysfs, RTC class, CMA (2 MiB), cpufreq, watchdog core, configfs | kallsyms, boot log; the vendor gadget / vb2 `.ko` files were built against the built-in videodev |

### 15.2 Vendor changes to generic files present in the image [R symbol / string in the image + file in a tree]
| change | file (tree) | in this image |
|---|---|---|
| Cortex-A53 in AArch32: `__v7_ca53_setup`, `__v7_ca53_proc_info` | `arch/arm/mm/proc-v7.S` (i6c) | identical to i6c; every CPU runs it (§3.3) |
| A53 PMU `armv7_a53_pmu_init` | `arch/arm/kernel/perf_event_v7.c` (i6c) | perf only |
| `secure_cntvoff_init` (CNTVOFF for the arch timer) | `arch/arm/common/secure_cntvoff.S` + `mach-sstar/mercury6/pm.c` | present, no caller (§3.3) |
| MMA / LX_MEM reservation hooks | `arch/arm/mm/init.c`, `mm/page_alloc.c`, `drivers/sstar/{mma_heap,cpu}` | §11 |
| `vmap_page_range` made global | `mm/vmalloc.c` (i6e) | no vendor module imports it |
| `do_page_cache_readahead` (exported wrapper of `__do_page_cache_readahead(…, 0)`) | `mm/readahead.c` (i6e) | no vendor module imports it |
| MTD `ajust_offset`, `part_del_partition_mapping` + `part_mapping[]` | `drivers/mtd/mtdpart.c` (both) | §9 |
| `__iget_wrap` (`atomic_inc(&inode->i_count)`, exported; at 0xC00AA134) | `fs/inode.c` (both) | **no caller**: no BL / B.W in the kernel text targets it (full Thumb-2 sweep of `.text`), the only word holding its address is its `__ksymtab` entry (0xC03AA130), no file of the camera dump (all 62 `.ko` of `miservice/modules`, rootfs, customer) contains the name, and the trees have only the definition — an export for an out-of-tree module this firmware does not ship [I] |
| deferred initcalls (`do_deferred_initcalls`, `/proc/deferred_initcalls`) | `init/main.c`, `fs/proc/root.c` | the section is empty (`__deferred_initcall_start == _end`) |
| IRQ priority API `__irq_set_priority`, `irq_chip_set_priority_parent` | `kernel/irq/{chip,manage}.c` (i6c) | §4.2 |
| GICv3 `gic_dist_save/restore`, `gic_redist_save/restore`, extended `gic_chip_data`, changed `gic_cpu_pm_notifier` | no source | suspend only (§4.1) |
| `struct usb_request.last_req`, `USB_COMP_EP0_BUFSIZ` 8 KiB, `config_ep_by_endp_desc` | `include/linux/usb/{gadget,composite}.h` (i6e) | `sgt` absent; `last_req` not proven either way (§15.3) |
| `get_ata_xfer_names` (libata), `_get_tx_volt_idx` (USB PHY front-end) | no source in either tree | unused paths (`kernel-drivers.md` §4.1.1, §5.2.4) |

Everything else of the class-N names is asm, syscall wrappers or macro-generated names. The full list of vendor-touched
generic files of the 4.9.84 tree counts 360 modified and 272 added files; many are upstream stable fixes already in 4.9.227.

### 15.3 Vendor struct changes [R]
<!-- ev:kernel-platform.15.3 -->
Method: (1) three-way line diff of every generic header the i6e SDK touched against vanilla 4.9.84 and 4.9.227, keeping
lines inside a struct body that neither vanilla has; (2) each candidate tested
against the image (symbols, strings, imports) and, where a vendor module uses the struct, against that module's field
offsets; (3) every class-N kallsyms name whose text appears nowhere in vanilla 4.9.227 (macro-generated names such as
`*_show`, `*_driver_init` filtered out).

| struct (header) | vendor change in i6e | in the binary? |
|---|---|---|
| `outer_cache_fns` (`asm/outercache.h`) | `flush_MIU_pipe` hook under `CONFIG_MS_L2X0_PATCH` | **no** — no `outer_cache` symbol; M6 has no L2X0 |
| `soc_device_attribute` (`sys_soc.h`) | `api_version` under `CONFIG_ARCH_SSTAR` | not visible: `soc_device_register` exists, no "api_version" string; layout effect not known; no vendor module imports `soc_device_register` |
| `sock` (`net/sock.h`) | `ss_swtoe` bitfields under `CONFIG_SS_SWTOE_TCP` | **no** (no swtoe symbol or string) |
| `usb_request` (`usb/gadget.h`) | `struct sg_table sgt` after `dma` under `CONFIG_USB_WEBCAM_UVC_SUPPORT_SG_TABLE`; `last_req:1` at the end | **`sgt` absent**: `usb_f_uvc.ko uvc_video_complete` reads `req->context` at +32, `req->list` at +36, `req->status` at +44 → buf 0, length 4, **`dma` 4 bytes at 8**, sg 12 … actual 48 = vanilla layout; `last_req` (after `actual`) not proven either way |
| `nand_chip` (`mtd/nand.h`) | `mtd_param_init` hook under `CONFIG_MS_NAND/SPINAND` | no NAND / SPINAND driver symbols in kallsyms; not applicable |
| `ehci_regs`, `usb_hcd`, `usb_device`, `ahci_platform_data` | MStar USB host / SATA fields | host USB and SATA unused here; `ehci-hcd.ko` / `mdrv-sata-host.ko` carry their own copies (`kernel-drivers.md` §4.2, §5.2) |
| `hidg_func_descriptor` (`usb/g_hid.h`) | `report_desc[64]` (fixed size) | no g_hid on the camera |
| `uvc_*` descriptors (`uapi/linux/usb/video.h`) | H.264 format / frame descriptor structs added | user-visible descriptor definitions for `usb_f_uvc`, not a kernel-object layout |
| **`MMA_BootArgs_Config`** (vendor `mdrv_mma_heap.h`) | — | **i6e layout**, 56 B with LPAE (§11.3) |

Not known statically: struct changes the M6 4.9.227 tree may have made outside the files the i6e tree touched; only the
i6e tree could be diffed.

## 16. Interfaces the kernel offers

### 16.1 To user space [R]
| interface | consumer | provided by |
|---|---|---|
| `/dev/sar` ioctls 0x6100 / 0x6101 | kjd_lvgl | sar (`kernel-drivers.md` §3.1) |
| `/dev/input/event0` (NEC raw, scancode = command byte) | kjd_lvgl | ir + rc core + evdev (`kernel-drivers.md` §3.2) |
| `/sys/class/gpio/gpio46` (PAD_GPIO2) | kjd_lvgl, factory test `tmp` | gpio + gpiolib sysfs (§6) |
| `/dev/msys` 0x5315 | riu_w | msys (§10.1) |
| `/dev/mtd*`, `/dev/mtdblock2..4` | rootfs, mounts, fwfs | flash_isp + MXP (§9) |
| ttyS0 115200 | console | serial (§7) |

### 16.2 Kernel symbols the vendor modules import [R]
Set examined: mhal, mi_common / sys / sensor / vif / isp / scl / rgn / gfx / venc / disp / hdmi / jpd / vdec, fbdev,
gc8613_MIPI, g_webcam, usb_f_uvc, udc-msb250x, kdrv_sdmmc, the gadget / vb2 / mmc / fat modules. 1317 imports: 556 resolved
between modules, 519 stock kernel, **201 vendor BSP** (all with source), 41 data (mostly stock; vendor data: `lx_mem*`,
`mma_config`, `mmap_reserved_config`, `mstar_driver_boot_*_num`, `u8_MiuWhiteListNum`; plus `__stack_chk_guard`).

| BSP group | imports | used by |
|---|---|---|
| cam_os_wrapper / cam_sysfs / cam_clkgen / cam_fs | 106 / 22 / 12 / 5 | mhal (all), fbdev, mi_common, mi_sys |
| miu | 16 | mi_sys |
| gpio / padmux | 15 / 3 | kdrv_sdmmc, mhal |
| mach-sstar `Chip_*`, `recode_timestamp` | 10 | mi_disp / isp / scl / sys / venc / vif, udc-msb250x, kdrv_sdmmc, mhal |
| msys | 5 | mi_sys, mhal |
| i2c `CamI2cOpen/Close/Transfer` | 3 | mhal |
| cpu `__aeabi_(u)ldivmod` | 2 | mhal, mi_* |
| voltage `get/set_core_voltage` | 2 | mhal |

The static I/O map's virtual addresses (§3.1), the `MMA_BootArgs_Config` layout (§11.3) and the LPAE / stack-protector
configuration (§15.1) are part of the same module ABI.
