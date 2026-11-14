# kernel-drivers — the BSP peripheral drivers of the vendor kernel and `kdrv_sdmmc.ko`
<!-- ev:kernel-drivers -->

The camera's kernel carries, besides its platform core, a set of SigmaStar
peripheral drivers built in, and the firmware loads three more peripheral drivers as modules. This document covers them:
the USB3 PHY and the EHCI host glue, the GICv3 state save / restore, PM / suspend-to-RAM, voltage-ctrl, the RTC and power
controller, the watchdog, the SAR ADC, the IR receiver and its keymap, PWM, notify, EMAC with its internal PHY, SATA, IVE,
the AES / SHA / RSA / RNG engine, and the SD host module `kdrv_sdmmc.ko`.

Elsewhere: RIU addressing, the static I/O map, the clock tree, interrupt numbers and the device tree as a whole, padmux,
GPIO and the board wiring (voltage-rail GPIOs, SAR buttons, IR pad, SD pads), DMEM, the per-driver function counts and the
body-difference table of all built-in drivers — `kernel-platform.md`; the inventory rows of `ehci-hcd.ko`,
`mdrv-sata-host.ko`, `kdrv_sdmmc.ko` and the `demo.sh` load lines — `firmware-image.md` §3.4, `rootfs-customer.md` §3.2; the USB
device port (`udc-msb250x.ko`, `usb_f_uvc.ko`, `g_webcam.ko`) — `usb-gadget.md`; the SAR button windows and the actions
kjd_lvgl binds to keys — `kjd_lvgl.md` §6.1, §6.2; the user-space IVE library `libmi_ive.so` — `ive-video-analytics.md`; the codec's
voltage request — `mhal-vcodec.md` §2.2.

Tags: **[R]** read from the Image, kallsyms, a `.ko`, a decompile or disassembly, the DTB, the boot log, a photo or a
camera log; **[S]** stated by a source (a vendor source tree, header or comment, or the user); **[I]** inference. A
register meaning tagged [S] is what a vendor header or comment says, not a measurement. Register notation: `bank:reg` =
16-bit register `reg` of RIU bank `bank`, phys 0x1F000000 + bank·0x200 + reg·4, kernel virtual 0xFD000000 + the same
offset (`kernel-platform.md` §3.1). Source trees: "i6c" = OpenIPC/linux `sigmastar-infinity6c` (5.10.61 SDK, carries the
Mercury6 platform files), "i6e" = OpenIPC/linux `sigmastar-infinity6e` (4.9.84 SDK) (`kernel-platform.md` §1.1).

## 1. The drivers at a glance

| driver | where | source relation | on this camera |
|---|---|---|---|
| USB3 PHY front-end + HAL (§4.1) | built in | front-end: no source anywhere; HAL = i6c `usbphy/infinity6c/usb_phy.c`, another revision | probe only: the `dwc3` node has no driver |
| EHCI host glue (§4.2) | `ehci-hcd.ko` | stock 4.9 EHCI + i6e `ehci-mstar.c` glue + Mercury6 bases | loaded; ehci-1 programs the device port's UTMI / USBC before the gadget loads |
| GICv3 save / restore (§2.1) | built in | no source in any tree | no (suspend only) |
| PM / suspend-to-RAM (§2.2) | built in | i6c `mercury6/pm.c`, `sram.S` with binary differences | no (suspend never entered) |
| voltage-ctrl (§2.3) | built in | i6c probe with a mutex, i6e `set_core_voltage` | **yes**: `cpu_power`, `core_power` drive PM_GPIO4 / PM_GPIO5; `dla_power` fails |
| RTC + power controller (§2.4) | built in | own revision of i6c `drv_rtcpwc` (spinlock, no IRQ, no SW reset, alarm-init change) | **yes**: hctosys, `hwclock -w` from kjd_lvgl, `pm_power_off` |
| watchdog (§2.5) | built in | i6e `infinity_wdt` + an INTR_PERIOD write | probed, never armed |
| SAR ADC (§3.1) | built in | `sar/mdrv_sar.c` (ioctl set identical) | **yes**: kjd_lvgl front buttons |
| IR receiver (§3.2) | built in | `ir/ms_ir.c`; vendor 21-entry keymap | **yes**: kjd_lvgl remote |
| PWM (§6.1) | built in | i6e `mdrv_pwm.c` + i6c `pwm/mercury6/mhal_pwm.c` logic under old names | probed; no pad, no user |
| notify (§6.2) | built in | = `notify/ms_notify.c` | init only |
| EMAC + netphy (§7) | built in | = source + binary differences | two MACs probe, link down |
| SATA (§5.2) | kernel glue + `mdrv-sata-host.ko` | older platform revision + the `mercury6p` tuning HAL | the module brings the PHY up at every boot; no disk |
| IVE (§8.1) | built in | i6c bodies, **i6e 116-byte ioctl ABI** | probed; no user |
| AES / SHA / RSA / RNG (§8.2) | built in | i6e / i6c mix | algorithms registered; no user |
| SD host (§5.1) | `kdrv_sdmmc.ko` | i6c generic files + an M6 platform file in no public tree | **yes**: the SD card |

Function counts in kallsyms by directory [R]: `emac/` 198, `ive/` 56, `crypto/` 107. The Goodix gt911 touch driver
in the image is for hardware not on this camera and is outside this document.

## 2. Power and system

### 2.1 GICv3 state save / restore — unused (suspend only)

#### 2.1.1 What is in the image [R]
A vendor patch to `drivers/irqchip/irq-gic-v3.c` adds `gic_dist_save`, `gic_dist_restore`, `gic_redist_save` and
`gic_redist_restore`, extends `struct gic_chip_data`, and changes `gic_cpu_pm_notifier` and `gic_of_init`. GICv3 has no
functions of these names in vanilla 4.9.227, i6e 4.9.84 or i6c 5.10; no tree has this code. Our symbol classification (`kernel-platform.md` §1.2) classes
`gic_dist_save/restore` as vanilla ("V") only because GICv2's `irq-gic.c` has functions of the same names.  The GIC itself and its addresses: `kernel-platform.md` §4.1.

#### 2.1.2 Data: `struct gic_chip_data` extended [R offsets from the code; I field roles from use]
| offset | field |
|---|---|
| +0x04 | `dist_base` (stock) |
| +0x10 | `rdists.rdist` (per-CPU `{rd_base, …}`, stock; +0x10 because the struct is 8-aligned under LPAE) |
| +0x28 | `domain` (stock) |
| +0x3C | `irq_nr` (stock) |
| +0x80 | **`saved_spi_enable[32]`** (u32, GICD_ISENABLERn) |
| +0x100 | **`saved_spi_active[32]`** (GICD_ISACTIVERn) |
| +0x180 | **`saved_spi_conf[64]`** (GICD_ICFGRn) |
| +0x280 | **per-CPU u32 `saved_ppi_enable`** (`__alloc_percpu(4,4)`) |
| +0x284 | **per-CPU u32 `saved_ppi_active`** (`__alloc_percpu(4,4)`) |
| +0x288 | **per-CPU u32[2] `saved_ppi_conf`** (`__alloc_percpu(8,4)`) |

`gic_of_init` allocates the three per-CPU areas after `gic_init_bases` succeeds; a failed allocation gives `WARN_ON` at
irq-gic-v3.c lines 903 / 908 (0x387, 0x38C) and frees the earlier ones; the driver then continues [R], and the save code
would then dereference NULL on suspend [I]. The field names mirror GICv2's `gic_chip_data` (`saved_spi_*`, `saved_ppi_*`) [I].

#### 2.1.3 Behaviour [R]
`n = irq_nr`. Word counts: enable / active `(n+31)/32`, conf `(n+15)/16`, all starting at word 0 [R code]; the SGI / PPI words
of the distributor are RAZ/WI under affinity routing, so they save zeros [I ARE].

| function | does |
|---|---|
| `gic_dist_save()` | if `dist_base`: copy GICD_ICFGRn (0xC00) → conf[], GICD_ISENABLERn (0x100) → enable[], GICD_ISACTIVERn (0x300) → active[] |
| `gic_dist_restore()` | if `dist_base`: per word write ~0 to GICD_ICENABLERn (0x180) and wait RWP; write conf[] to ICFGRn (wait each); write ~0 to GICD_ICACTIVERn (0x380), then active[] to ISACTIVERn (wait each); ~0 to ICENABLERn, then enable[] to ISENABLERn (wait each) |
| `gic_redist_save()` | this CPU's `rd_base` (via the TPIDRPRW per-CPU offset); if `rd_base + 0x10000` (SGI frame) ≠ 0: GICR_ISENABLER0 (SGI+0x100) → ppi_enable, GICR_ISACTIVER0 (SGI+0x300) → ppi_active, GICR_ICFGR0/1 (SGI+0xC00 / 0xC04) → ppi_conf[0..1] |
| `gic_redist_restore()` | GICR_ICENABLER0 (SGI+0x180) = ~0, wait; ICFGR0, wait; ICFGR1, wait; GICR_ICACTIVER0 (SGI+0x380) = ~0; ISACTIVER0 = saved, wait; ICENABLER0 = ~0; ISENABLER0 = saved; wait (`gic_redist_wait_for_rwp`) |
| `gic_cpu_pm_notifier(cmd)` | **CPU_PM_EXIT (2)**: stock part (`gic_dist_security_disabled()` → `gic_enable_redist(1)`; `gic_cpu_sys_reg_init()`), then **added**: `gic_cpu_init()`, `gic_dist_init()`, `gic_dist_restore()`, `gic_redist_restore()`. **CPU_PM_ENTER (0)**: stock part (if security disabled: ICC_IGRPEN1 = 0, `gic_enable_redist(0)`), then **added, unconditionally**: `gic_dist_save()`, `gic_redist_save()`. ENTER_FAILED (1) and others: nothing. Returns NOTIFY_OK |

So every CPU-PM cycle re-initialises and restores the whole distributor from each CPU that exits: the vendor assumes the GIC
loses its state in suspend (power-gated with the SoC core) [I].

#### 2.1.4 Who triggers it [R]
Stock `kernel/cpu_pm.c`: its syscore op `cpu_pm_suspend` calls `cpu_pm_enter()` (and `cpu_pm_resume` → `cpu_pm_exit()`)
during system suspend, after the devices and before the platform `enter` (§2.2). No cpuidle driver is in the image (only the
`__cpuidle_text_start/end` markers, no `cpuidle_*` symbols) [R kallsyms], so system suspend is the only path. Suspend is
never entered on this camera (§2.2.5), so this code never runs [R / I].

### 2.2 PM / suspend-to-RAM — unused

Source: i6c `arch/arm/mach-sstar/mercury6/{pm.c, sram.S}` and `soc.c` (`mstar_pm_reserve`), with the binary differences
below (the body comparison of pm.c and the instruction-level comparison of `sram.S`: `kernel-platform.md` §2.3, §3.4).


#### 2.2.1 Data [R]
| what | where | content |
|---|---|---|
| STR info page | DRAM phys **0x20000000**, 4 KiB, taken out of the allocator by `mstar_pm_reserve` (`machine_desc.reserve` → `memblock_reserve(0x20000000, 0x1000)`) | `suspend_keep_info` {char magic[8] = "SIG_STR"; u32 resume_entry = phys(`cpu_resume`); u32 count; u32 status (1 sleep / 0 awake); u32 password (0x5A5A55AA while asleep, else 0)} — 0x18 bytes, cleared at init; the rest of the page holds timestamp records [S source comment] |
| IMI copy of `sram_suspend_imi` | IMI SRAM phys 0xA0010000 (offset 64 KiB), 4 KiB mapped executable (`__arm_ioremap_exec`); boot log `suspend_imi_vbase=0xD0061000` | 4 KiB copied from `sram_suspend_imi` at every suspend (`fncpy`; the routine itself is ≈0x1BC bytes) |
| `rtc_suspend_info[6]` (binary-only) + pm.c statics | `.bss` | 6 saved PM-RTC registers (u16), then +0x0C `pStr_info`, +0x10 `suspend_status`, +0x14 IMI vbase, +0x18 IMI entry (vbase \| Thumb bit) |

Boot log: `[mstar_pm_init] resume_pbase=0x20010CF5, suspend_imi_vbase=0xD0061000` [R]. The constants in the image for
`virt_to_phys` / `__va` are the pre-patch `__pv_stub` placeholders, so the decompile shows 0x9F000000 / −0x7F000000; the log
gives the run-time value [R].

#### 2.2.2 Entry path [R code; stock parts named from vanilla 4.9]
`echo mem > /sys/power/state` (`suspend_valid_only_mem`: only "mem") → stock freeze / device suspend → syscore
`cpu_pm_suspend` (GIC save, §2.1) → **`mstar_suspend_enter(PM_SUSPEND_MEM = 3)`**: prints "state = PM_SUSPEND_MEM",
`cpu_suspend(0, mstar_suspend_ready)` (stock ARM: saves the CPU context and the MMU state for `cpu_resume`), returns 0; other
states −EINVAL. **`mstar_suspend_ready`**:
1. `fncpy(IMI vbase, sram_suspend_imi, 0x1000)` (alignment `BUG` if either address is not 8-aligned), `suspend_status = 1`.
2. `mstar_rtc_magic_set(0xBABE)` — PM-RTC bank 0x0034 (§2.4): reg 0x00 bit 9 ("write sw2 cmd") set, reg 0x04 = magic, ISO
   handshake (reg 0x30 = 1, wait reg 0x31 bit 0 = 1 up to 40 × 100 µs — the source waits 20; reg 0x32 = 1, wait bit 0 = 0 up to
   20 × 100 µs; a timeout restarts the handshake), `udelay(2000)` (binary-only), reg 0x00 bit 9 cleared, then — binary-only —
   **saves regs 0x00, 0x01, 0x0F, 0x20, 0x21, 0x22** into `rtc_suspend_info`. The source's reg 0x35 = 1 "enable autogen"
   write (phys 0x1F0068D4) is absent.
3. STR info: count++, status = 1, password = 0x5A5A55AA.
4. `v7_flush_kern_cache_all`, `Chip_Flush_MIU_Pipe`, local TLB flush; jump to the IMI copy.

**`sram_suspend_imi`** (runs from IMI, MMU on, through the static RIU map 0xFD000000) [R listing; S source]:
1. LPDDR enters self-refresh: MIU 1012:70 = 0; request masks 1012:23 = 0xFFFE, 1012:33 / 43 / 53 = 0xFFFF; 1011:73 = 0x00FE;
   1 ms (6000 arch-timer ticks at 6 MHz).
2. Pre-charge all + one refresh: 1012:0C = 0x0400; 1012:00 low byte bits 5:4 = 2; high byte: command field (bits 4:1) = 2
   (0x04), pulse bit 0 (0→1→0), field = 1 (0x02), pulse bit 0 again; 1 ms ("DO NOT REMOVE"); 1012:00 = 0x202E; 1 ms.
3. Analog power-down: 1012:01 high byte |= 0xF0; bank 0x1010 reg 0x00 low byte |= 0x08, |= 0x10; reg 0x2A low byte |= 0x70;
   reg 0x04 low byte bits 5:0 = 0; **binary-only**: reg 0x00 high byte `&0x9F|0x60`, reg 0x0C high byte `&0xDF|0x20`, then
   `&0x9F|0x60` (CPU byte offsets 1 and 0x31 from the bank base; offset 0x31 = reg 0x0C high byte).
4. Writes to bank 0x1524 (regs 0x2A, 0x2B, 0x28): 1524:2A = 0, 1524:2B = 0x0C80, 1524:28 = 0x0437. The source writes
   them as 8-bit RIU offsets 0x54 / 0x56 / 0x50 (`[r2, #0x54 << 1]` …) and names them the PM sleep block's timer2 / timer3
   and "enter u3 mode", with "100 ms, 64 ms auto wakeup" for 0x0C80 [S comments]. Bank 0x1524 is also the USB3 / SATA combo
   PHY's A-PHY0 bank (`USB3_SATA_PHYA 0x152400` in the M6 SATA header, §4.1, §5.2) [S]; whether these writes reach a sleep
   timer is not known [I].
5. `start`: PM-RTC bank 0x0034 power-down — **binary**: reg 0x00 = 0, reg 0x0F bit 0 cleared (source: reg 0x00 = 0, reg 0x0F =
   0x0004, reg 0x20 = 0x0011).
6. `rtc_iso`: reg 0x30 = 1, poll reg 0x31 bit 0 = 1, reg 0x32 = 1, poll bit 0 = 0, four `nop` — then the core loses power.
   There is no return path: if power stays on, execution runs into the literal pool [R listing].

#### 2.2.3 Wake path [R code; I boot ROM / IPL part]
Power returns → the boot ROM / IPL finds the STR info at DRAM 0x20000000 (magic + password) and jumps to `resume_entry` = phys
`cpu_resume` [I: the IPL's resume check is not specified in these sources; `ipl.md` §2.4, §3.5] → stock ARM `cpu_resume` restores the MMU
and the `cpu_suspend` context → `cpu_suspend` returns 0 → `mstar_suspend_enter` returns 0 → stock resume of syscore
(`cpu_pm_resume`, GIC restore §2.1) and devices → **`mstar_suspend_wake`**: STR status = 0, password = 0, `suspend_status = 0`,
writes the 6 saved PM-RTC registers back (binary-only), then `mstar_rtc_magic_set(0)` (same handshake, magic 0).
`secure_cntvoff_init` is not called (commented out in the source too) [R]. The secondary CPU is taken down and up by the stock
`disable_nonboot_cpus` hot-plug path.

#### 2.2.4 Kernel interfaces [R]
`/sys/power/state` ("mem" only), `suspend_set_ops(&mstar_suspend_ops {enter, wake, valid = suspend_valid_only_mem})`; exported
global `suspend_status` (no kept module imports it [R module import tables]). Drivers of this document with suspend / resume
hooks: GIC (§2.1), RTC (§2.4), watchdog (§2.5), PWM (§6.1), EMAC (§7), SATA (§5.2), EHCI (§4.2), IVE (§8.1), SD (§5.1); the
MIU arbiter and I2C have them too (`kernel-platform.md` §12.4).

#### 2.2.5 Use on this camera [R]
No program on the camera writes `/sys/power/state` (only busybox carries the string; prog_uvc, kjd_lvgl and rcS do not), and
mi_common's wake-lock / suspend paths do not call `pm_suspend` (`mi_sys-2-interfaces.md` §1.5). Suspend is built but never entered.

### 2.3 voltage-ctrl (`sstar,voltage-ctrl`) — in use

Source: i6c `voltage/{voltage_ctrl.c, mercury6/voltage_request_init.c}` with a `struct mutex` and without `lt_voltage`;
`set_core_voltage` is the i6e body plus a `get_voltage_ctrl` lookup.

#### 2.3.1 Instances [R]
The DT has three `sstar,voltage-ctrl` nodes, each `vid_width = <1>`, `vid_voltages = <900 1000>` (mV), `init_voltage = <900>`,
`status = "ok"`: `cpu_power` (`vid_gpios` 10 = PAD_PM_GPIO4), `core_power` (11 = PAD_PM_GPIO5) and `dla_power` (**11 again**).
`cpu_power` and `core_power` probe; `dla_power`'s `gpio_request(11, "vid0")` is refused because `core_power` already owns the
GPIO → "Failed to get gpio for voltage control!! -1", "sstar,voltage-ctrl: probe of soc:dla_power failed with error -1" — the
only failure line in the boot log. The pads and the rail wiring: `kernel-platform.md` §5.4, §6.3. Whether PM_GPIO4 / PM_GPIO5
reach a regulator's VID / feedback input on this board is not known (a physical measurement of the rails, CAMERA-CHECKS P8)
[I: the vendor template assumes it].

#### 2.3.2 Data [R offsets from the decompile]
`struct voltage_ctrl` (0x214 B, kmalloc-512 cache): +0x00 `name` (= DT node name), +0x04 `gpio_vid_width`, +0x08
`gpio_vid_pins*`, +0x0C `gpio_vid_voltages*`, +0x10 `current_voltage`, +0x14 u8 `voltage_gpio_inited`, +0x18 `struct device
dev_core` (`kobj.name` = name, `bus = &voltage_subsys`), +0x1C8 u8 `enable_scaling_voltage`, +0x1CC `pdev`, +0x1D0 `struct mutex`
("&ctrl->core_voltage_mutex"; i6c: a CamOs spinlock), +0x1F4 `int request[6]` (one per demander), +0x20C `struct list_head`
(global `voltage_ctrl_list`). No `lt_voltage` field (i6c has one and reads DT `lt_voltage`; the binary does not).
Demanders (`VOLTAGE_DEMANDER_E`, i6c `include/mercury6/voltage_ctrl_demander.h`, bound `< 6` in the binary): 0 CPUFREQ,
1 TEMPERATURE, 2 VENC, 3 MIU, 4 IPUFREQ, 5 USER.

#### 2.3.3 Behaviour [R]
- **probe**: kmalloc; name / bus; mutex init and lock; read `vid_width` (missing or 0 → error), vzalloc pins and `1 << width`
  voltages, read `vid_gpios` (count must equal width) and `vid_voltages` (count must equal 2^width); `gpio_request(pin, "vid%d")`
  + `gpiod_export(desc, 0)` per bit (so `/sys/class/gpio/gpio10`, `gpio11` exist, direction fixed); `voltage_gpio_inited = 1`;
  unlock; register bus `voltage` once (`subsys_system_register` → `/sys/devices/system/voltage/`), `device_register(dev_core)`
  (→ `/sys/devices/system/voltage/<name>/`), drvdata, list add, four attributes, **`voltage_request_chip(name)`**,
  `enable_scaling_voltage = 1`, `sync_core_voltage`. Errors print "Failed to register voltage sub system!! %d" / "…core
  device!! %d" / the gpio line, free the pins and return −1.
- **`voltage_request_chip(name)`** (= i6c `mercury6/voltage_request_init.c`): if the node has `init_voltage`: TEMPERATURE = 1000
  for `cpu_power`, `core_power`, `dla_power` (only already-registered controllers accept it, so each controller gets its own at
  its probe), then USER = `init_voltage` (900) for this one; without `init_voltage`: USER = 1000.
- **`set_core_voltage(name, demander, mV)`** (exported; mhal imports it): demander ≥ 6 or unknown name → −EINVAL; under the mutex
  store `request[demander] = mV`; if inited, width ≠ 0 and scaling enabled: `max(request[])` → `check_voltage_valid` (exact level,
  else the next higher level with a KERN_ERR print "[Core Voltage] %s: Not support %dmV, use %dmV" — a debug print in i6c — else
  the highest level) → for the level index `i`, bit `b` of `i` drives `gpio_vid_pins[b]` (`gpiod_direction_output_raw`, 1 = bit
  set); `current_voltage` = that level. Returns 0. There is no "max == 0 → skip" check as in i6c, and no DUALOS path.
- **`get_core_voltage(name, *mV)`**: `current_voltage`; −EINVAL for an unknown name. `core_voltage_available` /
  `core_voltage_pin`: voltage / pin arrays and counts (exported, no kept-module user).
- `sync_core_voltage(ctrl)`: the same as `set_core_voltage` without storing a request.

With width 1 the mapping is level 0 = 900 mV = GPIO low, level 1 = 1000 mV = GPIO high [R DT + code].

#### 2.3.4 sysfs `/sys/devices/system/voltage/{cpu_power,core_power}/` [R]
Bodies = i6c.

| file | mode | read | write |
|---|---|---|---|
| `scaling_voltage` | 0644 | "%d\n" enable flag | "%d": nonzero enables, 0 disables, then `sync_core_voltage` |
| `voltage_available` | 0444 | "\tVcore(mV)\tvid_0(<pin>)…" header, then per level "[j]\t<mV>\t\t<bit>…" | — |
| `voltage_current` | 0644 | current mV (or "Unknown (not assigned GPIO)"), then per demander "    %-32s%d" or "    %-32s-" with the enum names | "%d": if scaling enabled, USER demander = value, else "[Core Voltage] voltage scaling not enable" |
| `vid_gpio_map` | 0444 | "vid_0=<pin> " | — |

#### 2.3.5 Who requests what on this camera [R code; I resulting level]
| demander | requester | value |
|---|---|---|
| TEMPERATURE | `voltage_request_chip` at probe | 1000 (both) |
| USER | `voltage_request_chip` (DT `init_voltage`) | 900 (both) |
| CPUFREQ (`cpu_power`) | `ms_cpufreq_target_index`: OPP voltage of the target (DT `cpu@0` `operating-points`: 1.2 / 1.1 GHz → 1000 mV, 1.0 / 0.9 GHz → 900, 0.8 / 0.6 / 0.4 GHz → 850), set before raising / after lowering the clock | 850 at the running 800 MHz |
| TEMPERATURE | cpufreq `monitor_temp` thread (1 s period, only when the chip's OSC reading ≥ `osc_threshold`; sysfs `/sys/devices/system/cpu/cpufreq/{temp_adjust_threshold_lo,_hi,osc_threshold,temp_out,osc_out,sidd_out}`): above the high threshold and > 900 → 900; below the low threshold and < 1000 → 1000 — for `cpu_power` and `core_power` | 900 / 1000 |
| VENC (`core_power`) | mhal VCODEC `change_voltage`: any codec clock > 384 MHz → 1000, else 900 (`mhal-vcodec.md` §2.2) | 900 or 1000 |
| IPUFREQ | mhal IPU imports only `get_core_voltage` | — |

Both rails sit at **1000 mV (GPIO high)** from probe on, because the TEMPERATURE request is 1000 and the maximum wins; they drop
to 900 only if the temperature thread lowers TEMPERATURE on fast silicon while no other demander asks for 1000 [I]. The codec's
1000 mV request is therefore honoured but normally changes nothing, and the ISP and codec run at 1.0 V [I: if the GPIO reaches
the regulator]. Writing 900 to `voltage_current` sets only the USER demander and does not lower the rails [I: TEMPERATURE's
1000 wins]. The live values of `/sys/devices/system/voltage/core_power/voltage_current` and `…/cpu_power/voltage_current`
(expected 1000 and the demander list) are not read on this camera (CAMERA-CHECKS K8).

### 2.4 RTC + power controller (`sstar,rtcpwc`) — in use

Source: i6c `drivers/sstar/rtc/{drv_rtcpwc.c, mercury6/hal_rtcpwc.[ch], mercury6/hal_rtcpwcreg.h}`, in a revision of its own;
the differences are marked where they occur below. 

#### 2.4.1 DT [R]
`rtcpwc { compatible = "sstar,rtcpwc"; reg = <0x1f006800 0x200>; interrupts = <GIC_SPI 0x27 4>; alarm-enable; io4-enable = <3>;
io5-enable = <2>; status = "ok"; }`. Properties the probe reads: `default_date` (u32 array → `struct rtc_time` → `default_base`
seconds; absent → 0), `io0-hiz` (bool), `io2-wos` (u32 comparator select) + `io2-wos-v` (2 × u32: low, high level select),
`io3-pu` (bool), `io4-enable` (u32: bit 0 PWR_EN, bit 1 ALARM_ON_EN), `io5-enable` (the same for PWR2), `alarm-enable`,
`alarm-init-on`, `iso-auto-regen` (bools). Not read (i6c reads them): `offset-count`, `offset-nagative`. The `interrupts` entry
is **not used**: the binary requests no IRQ (i6c: `request_irq(…, "RTCPWC")` + `enable_irq_wake`).

#### 2.4.2 Registers: PM bank 0x0034 (phys 0x1F006800), ioremapped [R binary offsets; S names = i6c `mercury6/hal_rtcpwcreg.h`]
| reg | name | bits used |
|---|---|---|
| 0x00 | DIG2RTC_BASE_WR (command bits) | 1 BASE_WR, 2 BASE_RD, 3 CNT_RST_WR, 4 ALARM_WR, 5 SW0_WR, 6 SW1_WR, 7 SW0_RD, 8 SW1_RD; **9 = SW2 write** (used only by the suspend code, §2.2) |
| 0x01 | DIG2RTC_CNT_RD | 0 CNT_RD, 1 ALARM_RD, 2 ALARM_EN, 3 INT_CLR |
| 0x03 | ISO_CTRL | — |
| 0x04 / 0x05 | WRDATA_L / _H | data to the RTC domain |
| 0x06 | DIG2RTC_SET | bit 0 cleared after a base write |
| 0x07 | RTC2DIG_VALID | — |
| 0x08 | RTC2DIG_ISO_ALARM_INT | bit 0 alarm enabled, bit 1 alarm interrupt pending |
| 0x09 / 0x0A | RDDATA_L / _H | data from the RTC domain (base, SW0 / SW1, alarm) |
| 0x0B | CNT_UPDATING | bit 0 = counter latch in progress |
| 0x0C / 0x0D | RDDATA_CNT_L / _H | running counter (seconds since the last base write) |
| 0x0E | CNT_RD_TRIG | bit 0 latch counter |
| 0x0F | DIG2PWC_PWR_EN_CTRL | 0 PWR_EN (main power hold), 1 ALARM_ON_EN (alarm powers on) |
| 0x10 | DIG2PWC_OPT | bit 8 SW_RST (not used by the binary) |
| 0x11 | PWC2DIG_FLAG | wake-up source: 1–4 io0–io3, 6 alarm |
| 0x12 | PWC2DIG_STATE | bit n = io n level |
| 0x15 | RTC_TESTBUS | bit 5 = 1 kHz clock present |
| 0x20 | DIG2PWC_PWR2_EN_CTRL | 0 PWR2_EN, 1 ALARM2_ON_EN, 2 IO0_HIZ, 3 IO3_PUPD_SEL, 4 IO3_POL |
| 0x21 | WOS_V_SEL | io2 window: bits 5:3 low, 2:0 high (each ≤ 7) |
| 0x22 | WOS_CTRL | 0 CLR, 1 HP_EN, 3:2 comparator out select |
| 0x30 / 0x31 / 0x32 | ISO_TRIG / ISO_EN_ACK / ISO_EN_CLR | the ISO handshake |
| 0x35 | ISO_AUTO_REGEN | bit 0 (only with `iso-auto-regen`) |

#### 2.4.3 Protocol and time model [R]
- **ISO transfer** (`hal_rtc_iso_ctrl`): wait up to 22 × 100 µs for TESTBUS bit 5 (1 kHz clock; missing → one-time print
  "[%s][%d] RTCPWC fail to enter correct state and possibly caused by no power supplied" and return); then up to 50 tries of
  `iso_ctrl_ex`: ISO_TRIG = 1, wait ACK bit 0 = 1 (≤ 50 × 100 µs), ISO_EN_CLR = 1, wait ACK = 0 (≤ 50 × 100 µs); after 50 failed
  tries `iso_fail = 1` (shown by sysfs `isoctl_check`). Every access to the RTC domain = set command bit(s) in reg 0x00 / 0x01
  (+ data in WRDATA), ISO transfer, clear the bit(s).
- **Time = base + counter**. `set_time(s)`: base write (BASE_WR, WRDATA = s, ISO), counter reset (CNT_RST_WR, ISO), clear SET
  bit 0; then SW0 = **0xBABE** ("RTC_PASSWORD"), SW1 = s & 0xFFFF. i6c also pulses DIG2PWC_OPT SW_RST around the base write; the
  binary does not [R] — `CONFIG_RTCPWC_SW_RST_OFF` or an older HAL [I].
- `read_time`: read SW0, base, SW1; if SW0 ≠ 0xBABE or SW1 ≠ base low 16 bits → "Please set rtc timer (hwclock -w)",
  re-initialise with `default_base` (base, SW0, SW1); then if SW0 = 0xBABE latch the counter (CNT_RD, ISO, CNT_RD_TRIG pulses
  with 5 ms waits while UPDATING, ≤ 5 tries, else "Check valid bit of RTC counter failed!" and return 0 with *s untouched) and
  return base + counter, saturated at 0xFFFFFFFF.
- **Alarm**: `set_alarm(delta)`: latch the counter, alarm = counter + delta, ALARM_WR with WRDATA = alarm, ISO; `get_alarm`:
  base + (ALARM_RD → RDDATA); `alarm_enable(e)`: reg 0x01 bit 2, ISO; `clear_alarm_int`: reg 0x01 bit 3 pulse with ISO.
- **Power off** (`pm_power_off` hook = `sstar_rtcpwc_poweroff` → HAL power-off): PWR2_EN + ALARM2_ON_EN set, ISO; ALARM_ON_EN
  set, ISO; INT_CLR set; PWR_EN cleared; ISO → the PWC drops the main power; the alarm (or an io wake-up) can power the board on
  again.

#### 2.4.4 Probe and init [R]
devm alloc (0x50 B: flag, pdev, rtc dev, `hal` {base, default_base, iso_fail, flags…, alarm_time}, spinlock
"&(&info->mutex)->rlock"), MEM resource ("[%s]: failed to get IORESOURCE_MEM" → −ENODEV), ioremap, DT properties (§2.4.1),
**`hal_rtc_init`**: `iso-auto-regen` → reg 0x35 bit 0; alarm init: `alarm-enable` → ALARM_EN = 1; with `alarm-init-on` also
`set_alarm(0)` and, if the alarm-int bit is not set, a 1 s sleep (`CamOsMsSleep`) and a recheck ("Alarm init on fail !!!") —
then, **unlike i6c** (which returns after `set_alarm(1)`), always the power init: io4 → reg 0x0F bits 1 / 0 from `io4-enable`,
ISO; io5 → reg 0x20 bits 1 / 0 from `io5-enable`, ISO; event init: io0-hiz → reg 0x20 bit 2; io2-wos → reg 0x22 + 0x21; io3-pu
→ reg 0x20 bit 3 set, bit 4 cleared; each with ISO. Then `devm_rtc_device_register(…, sstar_rtcpwc_ops)` (`/dev/rtc0`; error →
"[%s]: unable to register device (err=%d)."), a chrdev region "rtcpwc", class device **`/sys/class/mstar/rtcpwc`**, the sysfs
files (§2.4.5), `pm_power_off = sstar_rtcpwc_poweroff`. With this DT: ALARM_EN = 1; reg 0x0F = PWR_EN 1, ALARM_ON_EN 1; reg 0x20
= PWR2_EN 0, ALARM2_ON_EN 1 [R DT + code].
`rtc_class_ops`: `read_time`, `set_time`, `read_alarm` (→ `rtc_time64_to_tm`, `rtc_valid_tm`), `set_alarm` (enable flag from
`wkalrm.enabled`, delta = alarm − now); no `alarm_irq_enable` (i6c has one). All run under the spinlock
(`spin_lock_irqsave(&info->mutex)`; i6c: a `struct mutex`), so the HAL's 5 ms `CamOsMsDelay` busy-waits and the 1 s
`CamOsMsSleep` of alarm init run with interrupts off or sleep under a spinlock [R code] — a latency / might-sleep defect of this
revision, absent in i6c's mutex version [I].

#### 2.4.5 sysfs `/sys/class/mstar/rtcpwc/` [R]
| file | mode | content |
|---|---|---|
| `alarm_timer` (only with `alarm-enable`) | rw | read "countdown time = %d" (RDDATA, i.e. whatever the last RTC read left there); write decimal seconds → `set_alarm(n)` |
| `alarm_interrupt` (only with `alarm-enable`) | rw | read "alarm interrupt actived/inactived"; write: **any word except "clear"** clears the interrupt ("clear" itself → −EINVAL) — inverted `strcmp`, the same in i6c |
| `isoctl_check` | r | "ISO EN sequence: %d" (`iso_fail`) — binary-only |
| `wakeup_event` | r | "wakeup event: %s" (rtc_io0–io3 / rtc_alarm from reg 0x11) |
| `event_state` | r | "event state:" + per io "%s : ON/OFF" (reg 0x12) |

Exported `sstar_rtc_get_wakeup_source()` returns the wake-up name (no kept-module importer [R module import tables]).

#### 2.4.6 Use on this camera [R]
Boot: `rtc0` registered, `hctosys` "setting system clock to 1970-01-01 00:24:20 UTC (1460)" (boot log) — no "Please set rtc
timer" line, so SW0 / SW1 were valid; base ≈ 0 and the counter has run since the last base write [I: a base written as 0 by an
earlier `hwclock -w` with the clock at 1970, or never written since the RTC domain kept power]. kjd_lvgl: `stime(2025-06-29
12:12:12)` at start and, after a Date & Time edit, `date YYYY.MM.DD-hh:mm:00` + **`hwclock -w`** (`kjd_lvgl.md` §2.1, §5.4) →
`RTC_SET_TIME` → `set_time`. busybox `poweroff` → `pm_power_off` → the power-off of §2.4.3; whether the board's supply follows
PWR_EN is a hardware question [I].

### 2.5 Watchdog (`sstar,infinity-wdt`) — probed, never armed

Source: i6e `drivers/sstar/watchdog/mdrv_wdt.{c,h}` (`infinity_wdt_*`; i6c has a different `sstar_wdt`). 

#### 2.5.1 DT, registers, clock [R]
`watchdog { compatible = "sstar,infinity-wdt"; reg = <0x1f006000 0x40>; status = "ok"; }` — no `clocks`, no interrupt. Platform
driver "infinity-wdt". Registers: PM bank **0x0030** (phys 0x1F006000); the driver uses the fixed address `BASE_REG_WDT_PA`
through the static map, the ioremapped `reg` is unused:

| reg | name (i6e `mdrv_wdt.h`) [S] | use [R] |
|---|---|---|
| 0x00 | WDT_CLR | write 1 = kick (clear counter); 0 at stop |
| 0x01 | DUMMY_REG_1 | not written by the driver; msys `msys_get_rebootType` reads a WDT status bit (`kernel-platform.md` §10.3) |
| 0x02 | RST_RSTLEN | not written |
| 0x03 | INTR_PERIOD | **0xFFFF written at start** (binary-only, §2.5.3) |
| 0x04 / 0x05 | MAX_PRD_L / MAX_PRD_H | timeout in clock ticks, 32 bits; 0 = stopped |

Clock: `__infinity_wdt_get_clk_rate` looks up the node's first clock; there is none, so `g_u32Clock` = **12 MHz** (0xB71B00)
and `MAX_PRD = 12e6 · timeout_s` (fits 32 bits up to 357 s) [R].

#### 2.5.2 Behaviour [R]
Watchdog core device (`/dev/watchdog`, `/dev/watchdog0`), info "infinity Watchdog", options SETTIMEOUT | KEEPALIVEPING, default
`tmr_margin` 10 s.
- **probe**: devm alloc, MEM resource (missing → −ENODEV), ioremap, `check_osc_clk`, copy of `infinity_wdd`, drvdata; inlined
  `set_heartbeat(tmr_margin)`: WDT_CLR = 1 and `timeout = tmr_margin` (10 if it were 0, with "tmr_margin value out of range,
  default %d used"); `watchdog_register_device` (error → "cannot register watchdog (%d)"). The counter is **not started**.
- **start** (first open of `/dev/watchdog`): CLR = 0, MAX_PRD = 0, **INTR_PERIOD = 0xFFFF**, CLR = 1, MAX_PRD = clock ·
  tmr_margin; `next_heartbeat = jiffies + 500 · tmr_margin`, open flag = 1 (the kernel timer is initialised but never added).
- **ping**: if `jiffies < next_heartbeat`: CLR = 1, `next_heartbeat = jiffies + 500 · tmr_margin`; else "Heartbeat lost! Will not
  ping the watchdog" (KERN_WARNING) and no kick. With HZ = 100 the software window is 5 · timeout seconds [R constants], so a
  ping inside the hardware timeout is never refused [I].
- **set_timeout(t)**: t < 5 → 5; `next_heartbeat`, `wdd->timeout = tmr_margin = t`, MAX_PRD rewritten (counter not cleared).
- **stop / shutdown**: del_timer, CLR = 0, MAX_PRD = 0, open flag = 0 (the core calls stop on a clean close: no `nowayout`).
- **suspend / resume**: if open, stop and keep the flag / restart.

#### 2.5.3 Binary vs i6e [R]
The same except: (1) `start` writes INTR_PERIOD (reg 0x03) = 0xFFFF before arming — absent in the i6e source; its meaning
(a pre-reset interrupt period?) is not documented [I]; (2) the "default timer value is out of range, cannot start" branch of
the probe is gone (dead in the source too: `set_heartbeat(10)` always returns 0).

#### 2.5.4 Use on this camera [R]
No program on the camera opens `/dev/watchdog` (no string in prog_uvc, kjd_lvgl or the init scripts; only busybox carries the
applet), so the Linux side never arms it. Whether U-Boot or the IPL arm it before Linux is not known from these sources [I: the
boot is not reset after 10 s with nobody kicking, so at least it is not armed with a short period].

## 3. Human input

### 3.1 SAR ADC (`/dev/sar`) — in use [R]
<!-- ev:kernel-drivers.3.1 -->
Driver `sar/mdrv_sar.c`, bodies the source. Character device `/dev/sar`, ioctl magic 'a': **0x6100** INIT (hardware init),
**0x6101** read {u32 channel (& 3), u32 value} in / out — identical to `mdrv_sar.c`. Quirk kept from the source: a failed copy
returns +14, not −EFAULT [R decompile].
User: kjd_lvgl reads the six front buttons (OK, ON, UP, MENU, DOWN, AE) as a resistor ladder on one SAR channel every 100 ms;
the vendor factory-test tool `tmp` (`/customer/tmp`) reads the same buttons with the same value windows (the windows:
`kjd_lvgl.md` §6.1; the wiring: `kernel-platform.md` §6.3). The physical keys on the camera body [R: photo]: top row ☰ · ▲/🔍+ · ⏻, bottom row ◀/🎥 · ▼/🔍− · OK/📷/▶ — vendor roles MENU · UP ·
ON · AE · DOWN · OK. The real ADC reading per button is not read on this camera (CAMERA-CHECKS K45).

### 3.2 IR receiver (`ms_ir.c`) and the keymap `rc-mstar-dtv` — in use

#### 3.2.1 Decoder [R decompile vs source]
<!-- ev:kernel-drivers.3.2.1 -->
`ms_ir.c` runs in **IR_MODE_RAWDATA**: ctrl 0x01B3, header code register 0xFF00 → NEC customer bytes 0x00, 0xFF; a frame is
accepted when byte2 == ~byte3; a 100-entry key queue; a dispatch thread → `rc_keydown_notimeout` / `rc_keyup` with scancode =
the NEC command byte. rc-core sends `EV_MSC/MSC_SCAN` with the scancode for every press and `EV_KEY` with the keycode of the
keymap; holding a key repeats it (`EV_KEY` value 2) [R camera log]. The device is `/dev/input/event0`,
which kjd_lvgl reads. Pad (PAD_PM_IRIN), bank 0x003D and clock `ir`: `kernel-platform.md` §3.1, §5.1, §6.2.

#### 3.2.2 Keymap [R: Image]
<!-- ev:kernel-drivers.3.2.2 -->
`struct rc_map_list mdrv_rc_map` (0xC044D848) → map name "rc-mstar-dtv", `rc_type` 1 (`RC_TYPE_OTHER` in 4.9), **21 entries** at
0xC044D878 ({u32 scancode, u32 keycode}). It is not the table of `ir/include/IR_MSTAR_DTV.h` in either tree (that one is a
66-entry MStar TV remote with other codes, e.g. 0x46 POWER, 0x50 KEY_0): the vendor kept the map name and replaced the table.

| scancode | kernel key | kjd_lvgl action (`kjd_lvgl.md` §6.2) | key on the microscope's remote |
|---|---|---|---|
| 0x45 | KEY_POWER (116) | EV_KEY: zoom in (live screen) | 🔍+ |
| 0x46 | KEY_UP (103) | EV_KEY: menu cursor up | ▲ |
| 0x47 | KEY_0 (11) | EV_KEY: zoom out (live screen) | 🔍− |
| 0x44 | KEY_LEFT (105) | EV_KEY: AE | ◀ |
| 0x40 | KEY_HOME (102) | EV_MSC: MENU | MENU |
| 0x43 | KEY_RIGHT (106) | EV_KEY: OK | ▶ |
| 0x07 | KEY_1 (2) | EV_MSC: mirror normal ↔ horizontal | ⇔ |
| 0x15 | KEY_DOWN (108) | EV_KEY: menu cursor down | ▼ |
| 0x09 | KEY_2 (3) | EV_MSC: AE toggle (live) / Exposure (menu) | AE |
| 0x16 | KEY_3 (4) | EV_MSC: mirror → vertical / normal | ⇕ |
| 0x19 | KEY_4 (5) | EV_MSC: reticle on / off | + (crosshair) |
| 0x0D | KEY_5 (6) | EV_MSC: AWB toggle (live) / AWB (menu) | AWB |
| 0x0C | KEY_6 (7) | EV_MSC: colour / B&W | ◐ |
| 0x18 | KEY_7 (8) | EV_MSC: freeze cycle | ❄ |
| 0x5E | KEY_8 (9) | EV_MSC: pause / resume playback | ▶‖ |
| 0x08 | KEY_9 (10) | EV_MSC: photo | 📷 |
| 0x1C | KEY_C (46) | EV_MSC: start / stop recording | 🎥 |
| 0x5A | KEY_F1 (59) | EV_MSC: playback chooser | 🖼 |
| 0x42 | KEY_F2 (60) | — | — |
| 0x52 | KEY_F3 (61) | — | — |
| 0x4A | KEY_F4 (62) | — | — |

kjd_lvgl uses 18 of the 21 codes; it takes 0x15 and 0x43–0x47 through `EV_KEY` and ignores them on `EV_MSC` [R]. The 21
scancodes are exactly the code set of the common 21-key NEC "Car MP3" remote (address 0x00) [R comparison]; the vendor's factory
test `tmp` enumerates exactly 18 of them in that remote's order, ending before its 7 / 8 / 9 [R], and the three codes
0x42 / 0x52 / 0x4A (KEY_F2–F4) are leftovers no product code uses.

The microscope's remote is not that remote but an 18-key "Full HD" remote, 6 rows × 3, printed icons top to bottom [R: photo]: row 1 🔍+ · ▲ · 🔍−; row 2 ◀ · MENU · ▶; row 3 ⇔ (mirror horizontal) · ▼ · AE;
row 4 ⇕ (mirror vertical) · + (crosshair) · AWB; row 5 ◐ (colour / B&W) · ❄ (freeze) · ▶‖ (play / pause); row 6 📷 · 🎥 · 🖼.
It has no printed OK and no digits. The last column of the table — which key emits which code — was measured: all 18 keys
pressed, each producing the code in its row [R camera, by pressing the keys].

## 4. USB host side and PHYs

The USB device port that carries the UVC stream is driven by `udc-msb250x.ko` (`usb-gadget.md` §2). Two other drivers touch USB
PHYs: the built-in USB3 PHY driver (never reaches its HAL) and the EHCI host module (programs the device port's PHY at boot).

### 4.1 USB3 PHY (`sstar,sstar-usb-phy`) — probe only

#### 4.1.1 What is built and what runs [R]
Two layers in `.text` 0xC016A464–0xC016AC64:
- **Generic-PHY front-end, no source anywhere** (searched both OpenIPC trees and vanilla): `_get_tx_volt_idx`,
  `tx_voltage_show/store`, `usb_phy_{init,exit,power_on,reset,probe}`, `usb_phy_disable_{emphasis,slew_rate}`,
  `usb_phy_driver_init/exit`; data `drv_cur`, `dem_cur`, `vcm_cur`, `tx_volt_idx`, `tx_voltage_array`, `usb_phy_ops`,
  `usb_phy_of_match`, `usb_phy_driver`, `dev_attr_tx_voltage`. 
- **PHY HAL** `sstar_usb_phy_*` = i6c `drivers/sstar/usbphy/infinity6c/usb_phy.c` (despite the directory name the file prints
  "Mercury6 USB PHY init"), in a **different revision** (§4.1.4). 
- Boot: the platform driver binds to the DT node `phy` and prints `SStar USB3 PHY probe, base:fd2a4600` (boot log) — and nothing
  else. `phy_init()` is never called: the only consumer, the `dwc3` node (`snps,dwc3`, `phys = <&phy &phy>`, no `status`
  property, i.e. enabled), has **no driver in the kernel** (0 `dwc3` symbols in kallsyms). So the PHY HAL never runs.

#### 4.1.2 DT node and module parameters [R]
`phy { compatible = "sstar,sstar-usb-phy"; io_phy_addr = <0x1f000000>; banks = <0x1523 0x1524 0x1525>;
reg = <0x1f2a4600 0x300>; #phy-cells = <0>; phy_type = "utmi"; status = "ok"; }`.
- `reg` is only ioremapped by probe (the pointer is kept as the phy's drvdata); the HAL computes addresses from `io_phy_addr` +
  bank·0x200 and uses them through the static map (`addr − 0x22000000` = virt). Banks: [0] 0x1523 = D-PHY, [1] 0x1524 = A-PHY0,
  [2] 0x1525 = A-PHY1 (names from the source's print).
- Built-in module parameters (kernel command line), names from the `.rodata` `__param` strings: `usb3_phy.vcm_cur`,
  `usb3_phy.dem_cur`, `usb3_phy.drv_cur`, `usb3_phy.tx_volt_idx`. Defaults in `.data`: drv_cur 46 (0x2E), dem_cur 19 (0x13),
  vcm_cur 7, tx_volt_idx 3.

#### 4.1.3 Front-end behaviour [R]
| function | behaviour |
|---|---|
| `usb_phy_probe(pdev)` | `devm_kmalloc(4)` for the base pointer; `platform_get_resource(MEM, 0)` + `devm_ioremap_resource` (error → return it); print "SStar USB3 PHY probe, base:%x"; `devm_phy_create(dev, NULL, &usb_phy_ops)` (error → "failed to create PHY", return it); `phy->drvdata(+0x64)` = base pointer; `device_create_file(dev, &dev_attr_tx_voltage)`; then: if `tx_volt_idx` is 3 (the default) or > 16, `tx_volt_idx = _get_tx_volt_idx()` (find the preset that equals the current vcm / drv / dem, 17 if none); else load drv / dem / vcm from preset `tx_volt_idx`; `__devm_of_phy_provider_register(dev, of_phy_simple_xlate)`; return 0 / error |
| `usb_phy_init(phy)` (`phy_ops.init`) | `sstar_usb_phy_utmi_deinit` → `sstar_usb_phy_switch_owner(1)` (USB3 owns the UTMI) → `sstar_usb_phy_setting(phy, vcm_cur, drv_cur, dem_cur)`; return 0 |
| `usb_phy_exit` (`.exit`) | `switch_owner(0)` (back to USB2) → `utmi_deinit`; 0 |
| `usb_phy_power_on` (`.power_on`) | `sstar_usb_phy_power_on`; 0 |
| `usb_phy_reset` (`.reset`) | `sstar_usb_phy_reset`; 0. `.power_off`, `.set_mode`, `.calibrate` are NULL |
| `usb_phy_disable_emphasis/_slew_rate` | exported tail calls to the HAL functions (no caller in the image or the kept modules [R module import tables]) |
| sysfs `tx_voltage` (mode 0644, on the phy platform device) | **show**: recomputes `tx_volt_idx` from the current values, then 17 lines `"%2d:%s\n"`, the matching one as `"%2d:%s <= current\n"`. **store**: `simple_strtoul(buf, 10)`; 0…16 → `tx_volt_idx` and vcm / drv / dem from the preset (takes effect at the next `phy_init`); else `dev_err "invalid index for tx voltage %d"`; always returns `count` |

`tx_voltage_array` (0xC0299588, 17 × {u16 vcm; u16 drv; u16 dem; u16 pad; char *name}) [R]:

| idx | vcm | drv | dem | name | idx | vcm | drv | dem | name |
|---|---|---|---|---|---|---|---|---|---|
| 0 | 5 | 34 | 14 | Va0.80_Vb0.53_De_m3.50dB | 9 | 7 | 52 | 17 | Va1.20_Vb0.85_De_m3.00dB |
| 1 | 6 | 38 | 15 | Va0.90_Vb0.60_De_m3.50dB | 10 | 7 | 53 | 15 | Va1.20_Vb0.90_De_m2.50dB |
| 2 | 7 | 42 | 17 | Va1.00_Vb0.67_De_m3.50dB | 11 | 7 | 44 | 22 | Va1.10_Vb0.67_De_m4.30dB |
| **3** | 7 | 46 | 19 | Va1.10_Vb0.65_De_m3.50dB (default) | 12 | 7 | 43 | 24 | Va1.10_Vb0.63_De_m4.84dB |
| 4 | 7 | 43 | 19 | Va1.05_Vb0.67_De_m3.90dB | 13 | 7 | 42 | 26 | Va1.10_Vb0.60_De_m5.26dB |
| 5 | 7 | 44 | 21 | Va1.09_Vb0.67_De_m4.20dB | 14 | 7 | 49 | 22 | Va1.20_Vb0.75_De_m4.08dB |
| 6 | 7 | 41 | 19 | Va1.00_Vb0.63_De_m4.00dB | 15 | 7 | 48 | 25 | Va1.20_Vb0.70_De_m4.68dB |
| 7 | 7 | 40 | 21 | Va1.00_Vb0.59_De_m4.60dB | 16 | 7 | 46 | 28 | Va1.20_Vb0.65_De_m5.33dB |
| 8 | 7 | 50 | 20 | Va1.20_Vb0.60_De_m3.50dB | | | | | |

The field order matches the source's `struct tx_voltage_settings {reg_biasi, reg_drv, reg_dem, descript}` — vcm = biasi [R];
the i6c header has `MAX_TX_VOL_OPT 9`, the binary has 17 presets [R]. "Va / Vb / De" = swing / de-emphasised swing /
de-emphasis [I from the names].

#### 4.1.4 HAL: binary vs i6c `usb_phy.c` [R decompile vs source]
`utmi_reset`, `utmi_deinit`, `power_on`, `switch_owner`, `reset`, `disable_emphasis`, `disable_slew_rate` are the **same**: UTMI
bank 0x1421 = phys 0x1F284200; `utmi_deinit` writes 0x7F03 to UTMI reg 0 and waits 5 × 1 ms; `switch_owner(1)` clears UTMI reg
0x01 bit 7, `(0)` sets it; `reset` toggles 1525:00 bit 4 low→high; `power_on` sets D-PHY reg 0x12 bit 12 (TX polarity invert);
`disable_emphasis` clears UTMI reg 0x16 bit 9; `disable_slew_rate` sets UTMI reg 0x16 bits 3:2.

`sstar_usb_phy_setting` is another revision (**DIFF**):

| point | i6c source | binary |
|---|---|---|
| signature | `(phy, tx_voltage_array, idx)` | `(phy, vcm, drv, dem)` — the values, not the table |
| print | "-Tx cur[%d]: %s" | "-Tx cur vcm[x%X], drv[x%X], dem[x%X]" |
| MIU PLL check | — | reads 1031:03 (phys 0x1F20620C): `f = (reg & 0xFF) · 24 / (((reg & 0x7FF) >> 8) + 2)`, prints "-miupll %d"; if `f < 267`, sets 1038:6F (phys 0x1F2071BC) bits 15:12 = 0xC, bits 11:0 kept (`reg = (reg & 0x0FFF) \| 0xC000`) [R]. The meaning of 1038:6F is not stated anywhere; 1038 is clock-gen bank 2 of the DT clocks (`CLK_sata_pm` at 0x1F2071B8 = 1038:6E) [R]; so it is a USB3 / SATA PHY clock select switched when the MIU PLL is low [I] |
| trim source | bank 0x1018 regs 0x5D / 0x5E (6 + 5 + 5 bits) | **101F:54** (phys 0x1F203F50), one 16-bit eFuse word: bits 15:10 = A-PHY0 reg 0x50 bits 15:10, bits 9:5 = reg 0x3C bits 8:4, bits 4:0 = reg 0x2A bits 11:7; all three equal → "IC is Trim", else "IC is not Trim, apply default setting" and the same defaults as the source (0x2A bits 11:7 = 0x10, 0x3C bits 8:4 = 0x10, 0x50 bits 15:10 = 0x20). The binary compares; it never copies the eFuse word into the PHY [R] |
| biasi | `D-PHY 0x26 &= ~0x0E; \|= biasi` | `D-PHY 0x26 &= ~0x0E; \|= (vcm << 1) & 0x0E` (vcm in bits 3:1) |
| reg 0x40 / 0x41 | `(dem & 0x0F) << 12 \| drv << 6 \| 0x19` ; `(dem & 0x30) >> 4 \| 0x2188` | the same formula, with `drv & 0x3F` |
| everything else | | the same register sequence: UTMI 0x09 bit 15 set, 0x1F286684 bit 1 cleared (unmask the USB30 gp2top interrupt), D-PHY 0x0B / 0x0E / 0x46 force imp / IEXT, A-PHY0 synth 0x40–0x44 (0xB6A7, 0x1B, 0x4, 0x3EE, 0x110), LFPS power, TX / RX PLL on, PHY reset release, RX / TX PLL lock detection (0x54 / 0x73 / 0x77 / 0x56 = 24000 / 0x57 = 0x1E0 / 0x70 / 0x60), LFPS burst gap 0xC4E, UTMI calibration (CA_START / CA_END poll), the UTMI tuning bytes and 0x16 / 0x17 = 0x290 / 0x110, the 2020-08-10 CDR / LFPS tweaks in 1524:11 / 14, 1523:67 / 29 |

The same eFuse word 101F:54 feeds the SATA PHY trim (§5.2.3). The OTP layout itself: `ipl.md` §7.

### 4.2 EHCI host glue (`ehci-hcd.ko`) — loaded, no host device; programs the device port's PHY

Section addresses used here: `.text` 0x10028, `.text.unlikely` 0x16558, `.init.text` 0x16b84. Loaded by
`/customer/demo.sh` line 5 (after `usb-common.ko`, `usbcore.ko`), i.e. before
`udc-msb250x.ko` [R].

#### 4.2.1 What it is [R]
Stock `drivers/usb/host/ehci-hcd.c` (4.9, built as a module) with the i6e vendor patches **plus** the MStar platform glue
compiled into the same module: i6e `drivers/sstar/usb/host/ehci-mstar.c` names (`ehci_hcd_mstar_drv_probe/remove/suspend/resume`,
`Titania3_series_start_ehc`, `MIU_select_setting_ehc`, "Mstar_ehc_init version:20180309" in the boot log) with the Mercury6 bases
of i6c `usb/host/mercury6/ehci-sstar-chip.h`. Vendor additions in the EHCI core [R strings; S i6e `ehci-hcd.c`]: the
**`ehci_monitor` kernel thread** (every 100 ms: when the core flags `ERR_INT_SOF_RXACTIVE`, drive the VBUS GPIO high,
`usb_remove_hcd` + `usb_put_hcd` and re-run the probe — "ERR_INT_SOF_RXACTIVE occur"), "EHCI SW frame index", "device has gone
before bus reset", "unknow port1 usb device speed", `usb_bc_enable` (a vendor usbcore export). Module parameters: stock
`log2_irq_thresh`, `park`, `ignore_oc`.

#### 4.2.2 DT and ports [R]
`Sstar-ehci-1 { compatible = "Sstar-ehci-1"; clocks = <phandle 0x82>; interrupts = <GIC_SPI 0x3F 4>; dpdm_swap = <0>; status =
"ok"; }`, `Sstar-ehci-2` (SPI 0x61). The probe recognises the device by **name** (`strncmp(pdev->name, "soc:Sstar-ehci-1"/"-2",
16)`), not by `reg` (there is none):

| port | UTMI | BC | USBC | UHC (EHCI regs) |
|---|---|---|---|---|
| ehci-1 | **0x1421** (0xFD284200) | 0x1422 | 0x1423 | 0x1424 |
| ehci-2 | 0x1429 | 0x1430 | 0x1431 | 0x1432 |

ehci-1's UTMI and USBC (0x1421 / 0x1423) are the **same banks `udc-msb250x` uses** for the USB device port that carries the UVC
stream (`usb-gadget.md` §2.2) [R both decompiles].

#### 4.2.3 Probe and PHY bring-up [R]
`ehci_hcd_mstar_drv_probe`: `usb_disabled()` → −ENODEV; by name: print "Sstar-ehci-N H.W init", read `dpdm_swap`,
`Enable_USB_VBUS(…)` (kernel `usb_pad.c`: **returns 0, does nothing**; `Get_USB_VBUS_Pin` returns −1, so the monitor thread's
VBUS GPIO is −1), **`Titania3_series_start_ehc(UTMI, USBC, UHC, flag = dpdm_swap)`**; DMA masks (32-bit, "[USB][EHC] dma
coherent_mask …"), IRQ from the DT ("[USB] %s irq --> %d"), `usb_create_hcd(…, "mstar")`, bases stored in the hcd (+0x170 port
no., +0x174 UTMI, +0x178 UHC, +0x17C USBC, +0x180 BC), `rsrc_start` = UHC virt, `regs` = UHC, EHCI caps / regs offsets from
CAPLENGTH, `usb_bc_enable(hcd, 0)`, `usb_add_hcd`.

`Titania3_series_start_ehc` = the i6e source sequence with M6 values (8-bit RIU addresses as in the source, i.e. CPU byte
offset / 2; UTMI bank):
1. UTMI 0x00 = 0x0001 (override on); **MIU select** USBC 0x14 / 0x16 / 0x17 / 0x18 = 0x70 / 0xE8 / 0xEF / 0xEF, partition enable
   (USBC 0x19 b0), "enable miu lower bound address subtraction" (USBC 0x0F b0) — boot log "[USB] config miu select [70] [e8]
   [ef] [ef]" twice.
2. UTMI 0x08 = 0x0C2F; (test-package flag only: 0x02 = 0x2084, 0x20 = 0x8051); UTMI 0x00 |= 0xC2 (DP / DM pull-down, term
   override), 0x52 |= 0x3F; USBC 0x00 = 0x0A then 0x28 (reset UHC, enable UHC + OTG XIU); DDR options by flag; **squelch** UTMI
   0x2A = 2 ("init squelch level 0x2"); **calibration** UTMI 0x3C b0 pulse 1 ms, poll CA_END (b1), "WARNING: CA Fail !!" if
   0x3C[15:4] is all ones or zero.
3. `dpdm_swap` → UTMI 0x0B b5; **USBC 0x02 = (… & ~b1) | b0 — "UHC select enable": the port is switched to the host
   controller**; UHC 0x40 b4 = 0 (VBUS on), 1 µs, b3 = 1 (active high); override bits off (UTMI 0x00 &= 0x3D, 0x52 &= 0xC0);
   UHC 0x81 |= 0x8F; UTMI tuning (0x06 = (… & 0x9F) | 0x40 HS current, 0x03 disconnect window, 0x07 CDR off, 0x08 b3, 0x09 |=
   0x81, 0x0B b7 TX latch, 0x15 |= 0x20 / 0x40 chirp source, 0x40 b4 = 0), **disconnect level** UTMI 0x2A = 0x62, eye settings
   0x2C |= 0x10, 0x2D |= 0x02, 0x2E |= 0, 0x2F |= 0x81, then the M6 ECO bits (UTMI 0x04 |= 0x60, 0x3F b7, 0x03 b2; USBC 0x02 b6
   power-noise ECO, 0x11 b1, 0x0A b6, 0x0B &= 0xE1, 0x0F b3, 0x10 b0 + b1, 0x13 b0, 0x20 b0–b2, 0x03 = 5; UHC 0x34 b7, 0x83 b4,
   0x8C b0); test-package flag: UTMI 0x14 = 0x0600, 0x10 = 0x0078, 0x32 = 0x0BFE.

Suspend: `ehci_suspend`; resume: `Titania3_series_start_ehc(UTMI, USBC, UHC, 0)` again, `usb_bc_enable`, `ehci_resume`.

#### 4.2.4 Use on this camera [R / I]
No host device is ever attached: the only USB connector is the device port to the PC. Both EHCI controllers register root hubs
and run [I: no "new device" lines in the boot log]. **ehci-1's bring-up runs on the device port's UTMI before the gadget
loads**: it calibrates the PHY (CA), sets the squelch and disconnect levels and switches USBC 0x02 to the host side;
`udc-msb250x`'s own `init_utmi` later re-programs the same banks for device mode [R]. Whether the host controller left enabled
on the shared port contributes to the "full speed after reconnect" behaviour seen live (where `udc_start` skips `init_utmi`,
`usb-gadget.md` §2.1, §2.9) is not known [I: a plausible interaction — both drivers own USBC 0x1423]; a boot without `ehci-hcd.ko`
(CAMERA-CHECKS P6) and UTMI 0x16 / 0x17 (phys 0x1F284258 / 0x1F28425C) before and after a re-enumeration (CAMERA-CHECKS K42,
P7) are not read on this camera.

## 5. Storage

### 5.1 SD host `kdrv_sdmmc.ko` (SigmaStar FCIE / SDIO, `sstar,sdmmc`) — in use

Binary `/config/modules/4.9.227/kdrv_sdmmc.ko` (vermagic `4.9.227 SMP preempt mod_unload ARMv7 thumb2 p2v8`).
Source copies: OpenIPC/linux `sigmastar-infinity6c` (5.10.61) `drivers/sstar/sdmmc/` in
the OpenIPC i6c 5.10 tree (`src/mercury6/hal_card_platform.c`, `inc/mercury6/*`); the 4.9.84 I6E trees
(tipoman9, TAKOYAKI) hold an older revision without SD 3.0 tuning.

#### 5.1.1 Source relation [R]
108 functions (97 named + 11 without symbols; 7 matched by fingerprint: `_REG_ResetIP`, `_REG_WaitEvent`, `_REG_ClearMIEEvent`,
`_REG_ClearSDSTS`, `_BUF_CIFD_WaitEvent`, `_PassPrintCMD`, `ms_sdmmc_dts_init`; `sdmmc_slot_lock` (0x12798) / `sdmmc_slot_unlock` (0x127cc) = per-slot
`mutex_lock/unlock` of `sdmmc{1,2,3}_mutex` by IP order = `_CRIT_SECT_BEGIN/END`). Against the i6c 5.10 build with
`CONFIG_SUPPORT_SD30` and the `mercury6` platform file: **65 of 87 compared identical**. The generic layers (`ms_sdmmc_lnx.c`,
`hal_sdmmc_v5.c`, `hal_card_intr_v5.c`, `hal_card_regs.c`, `hal_card_timer.c`) are the i6c revision — every binary function
name exists in the 5.10 copy (`ms_sdmmc_exec_tuning`, `ms_sdmmc_switch_busvdd`, `Hal_SDMMC_FindFitPhaseSetting/SavePassPhase/
SetPhase/Dump_GoodPhases`, `Hal_CARD_SetPADVdd`, `Hal_CARD_DrvCtrlPin`, `Trig_MIE_INTR`, globals `ge_ClkDriving/CmdDriving/
DataDriving`, `gu32_TXClkPhase/RXClkPhase/EnClkPhase`…) except `Hal_Card_DrvSetSD30` and the sysfs naming. For the 65 identical
fingerprints only calls, strings, large constants and MMIO addresses are known equal (small constants not read) [I: same code].
The **platform file `hal_card_platform.c` is an M6 revision that exists in no public tree**: none of the i6c platform files,
`mercury6` included, nor the i6e ones has `Hal_Card_DrvSetSD30`; the binary's pad handling through `MDrv_GPIO_*` /
`mdrv_padmux_*`, `Hal_CARD_DrvCtrlPin`, `Hal_Check_ClkCmd_Interrelate` and `gu32_SlotEnPwrHighValid` follow the style of i6c's
`infinity6c` / `mercury6p` platform files, while i6c's `mercury6` file writes pad registers directly. Those other i6c platform
files do not build against 4.9 (5.10 `MDrv_GPIO_Pad_Read` signature, `MDRV_PUSE_SDIO2_*`) and were compared by name only.
§5.1.4 is the platform layer as read from the decompile. Other differences:

| function | binary |
|---|---|
| `ms_sdmmc_dts_init` | reads `slotnum`, `revcdz`, `slot-ip-orders`, `slot-pad-orders`, `slot-max-clks`, `slot-intcdzs`, `slot-support-sd30`, `slot-fakecdzs`, **`slot-cdzs-gpios`, `slot-pwr-gpios`** (i6c: `slot-cdzs-pad` / `slot-pwr-pad` / `slot-revcdz`), `slot-pwr-off-delay`, `slot-pwr-on-delay`, `slot-sdio-use`, `slot-removable`, `slot-sdio-use-1bit`, `slot-sdio-dis-intr`, `slot-en-clk-phase`, `slot-tx-clk-phase`, `slot-rx-clk-phase`, `slot-en-pwr-high-valid`, `slot-clk/cmd/data-driving`, IRQ names `mie<n>_irq`, `cdz_slot<n>_irq`; no `MDrv_GPIO_To_Irq` / `irq_modify_status` (card-detect IRQs come from the DT `interrupts-extended`) |
| `Hal_SDMMC_SetBusTiming` (shadow registers per IP, written at the next command) | Default / HS: DDR_MODE 0x6400, ForR2N = DDR_MODE \| 0x0E, EMMC_TEST 0x0400, SKEW_SUM 0x40, **TX_BPS_EN 0x3F** (i6c 0xFFFF), rest 0. SDR50 / SDR104 / HS200: DDR_MODE 0x1000, EMMC_TEST 0x19, ECO_EN 0x100, EMMC_EN 1, SKEW_SUM 0x40, DQS_PAGE_NO 0x413, AFIFO_SUM 0xD00, **TX_BPS_EN 0** (i6c 0x4000). DDR50: DDR_MODE 0x180, ECO_EN 0x200, EMMC_EN 1, SKEW_SUM 0x40, DQS_PAGE_NO 0x213, DDR_IO_MODE 1, TX_BPS_EN 0x3F. Timing stored per IP |
| `ms_sdmmc_request` | sets the clock (`Hal_CARD_SetClock`) itself and maps highmem pages (`page_address`) |
| `ms_sdmmc_hotplug` | re-arms the card-detect GPIO interrupt polarity through `Hal_CARD_SetGPIOIntAttr` after insert / remove (debounce: up to 30 polls) |
| per-slot sysfs (§5.1.6), `sdmmc_driving_control` | older layout than i6c (5.10 has single `get_sdmmc_clock/status`, `sdmmc_reset`, `set_sdmmc_driving_control` and test attributes) |
| `Hal_SDMMC_DumpMemTool`, `Hal_SDMMC_GetMIEEvent`, `Hal_SDMMC_OtherPreUse` | = tipoman9 I6E (identical fingerprints) |

#### 5.1.2 Board configuration (built-in DTB, `/soc/sdmmc`) [R]
<!-- ev:kernel-drivers.5.1.2 -->
| property | value |
|---|---|
| slotnum | 2 (3-entry arrays, the third unused) |
| slot-ip-orders / slot-pad-orders | <0 1 2> / <0 0 1> |
| slot-max-clks | 200 MHz, 48 MHz, 48 MHz |
| slot-support-sd30 | <1 0 0> (UHS tuning only on slot 0) |
| slot-cdzs-gpios / slot-pwr-gpios | <25 148 0> / <145 146 0>; card detect by GPIO interrupt (`slot-intcdzs` 1) |
| slot-clk/cmd/data-driving | <3 0 0> |
| slot-en-clk-phase | <0 0 0> |
| slot-sdio-use | 0 |
| pwr off / on delay | 30 / 1 ms |
| interrupts | mie0 SPI 0x32, mie1 SPI 0x33 (level high); cdz_slot0 / 1 / 2 via interrupt-parent phandle 2, lines 0x97, 0x98, 0x97 |
| clocks | phandles 0x6a / 0x6b (0x6b = `CLK_sdio`, composite at 0x1F207114) |

Both slots are fully pad-muxed and removable (pads: §5.1.4, `kernel-platform.md` §6.2).

#### 5.1.3 Interface offered [R names; I op mapping from source]
Standard `mmc_host_ops` to `mmc_core`: request, set_ios, get_ro, get_cd, enable_sdio_irq, start_signal_voltage_switch →
`ms_sdmmc_switch_busvdd`, execute_tuning → `ms_sdmmc_exec_tuning`. Consumers: `mmc_block` → `/dev/mmcblk0p1` (vfat); prog_uvc's
SD detection via `/tmp/sdcard_info` (`prog_uvc.md` §2.2, `rootfs-customer.md` §1). Module parameters `gu32_sd_irq_count`,
`gu32_sdio_int_count` (read-only counters, the same in the source).

#### 5.1.4 M6 platform layer `hal_card_platform.c` [R decompile + literal pools]
Banks hard-coded in the binary [R literal pools]: CHIPTOP 0x101E (0xFD203C00), MPLL 0x1030, CLKGEN 0x1038 (0xFD207100…),
bank 0x1039 (clock-phase block), bank 0x103C (SD 3.0 pad drive), SC_GP_CTRL 0x1133 (0xFD226600), SDPLL 0x1419 (0xFD283200); the
FCIE / SDIO IP banks are computed from `A_RIU_BASE`.

**IPs and banks.** Three IPs; per IP three register banks (`Hal_CREG_GET_REG_BANK(ip, n)`, `.rodata+0`) and three PLL banks
(`Hal_CREG_GET_PLL_BANK`, `.rodata+0x24`), the last-used IP cached in `.bss`:

| IP | register banks (n = 0, 1, 2) | PLL banks | clock register | used by |
|---|---|---|---|---|
| 0 | 0x1413, 0x1414, 0x1415 (phys 0x1F282600 / 2800 / 2A00) | **0x1419**, 0x141A, 0x141B (SDPLL, 0x1F283200…) | CLKGEN 0x1038:0x43 (0x1F20710C) | slot 0 |
| 1 | 0x1410, 0x1411, 0x1412 (0x1F282000…) | 0x141C, 0x141D, 0x141E | CLKGEN 0x1038:0x45 (0x1F207114 = DT `CLK_sdio`) | slot 1 |
| 2 | 0x1402, 0x1403, 0x1404 (0x1F280400…) | as IP 1 | — | unused (slotnum 2) |

**Pad table** (`.bss+0x150`, filled by `Hal_CARD_IPOnceSetting` from padmux when `mdrv_padmux_active()`, i.e. from the DT
`padmux` node; per slot): data-group mode, PWR mode, CDZ mode, D0–D3, CLK, CMD, PWR pad, CDZ pad, looked up by PUSE code (i6e
numbering: 0x200**s**0 PWR, …1 CDZ, …2 CLK, …3 CMD, …4–7 D0–D3, s = slot). On this camera [R `board.dts` padmux `schematic`
triples, the image's `gpio_table`]:

| slot | D0 / D1 / D2 / D3 | CLK / CMD | pad mode | PWR (GPIO) | CDZ (mode) |
|---|---|---|---|---|---|
| 0 | PAD_SD0_D0 27 / D1 26 / D2 31 / D3 30 | PAD_SD0_CLK 28 / CMD 29 | 0x7A | PAD_SD_GPIO0 145 | PAD_SD0_CDZ 25 (0x7C) |
| 1 | PAD_SD1_D0 150 / D1 149 / D2 154 / D3 153 | PAD_SD1_CLK 151 / CMD 152 | 0x7D | PAD_SD1_GPIO0 146 | PAD_SD1_CDZ 148 (0x82) |

The DT `slot-cdzs-gpios <25 148>` / `slot-pwr-gpios <145 146>` give the same pads.

Functions:
- **`Hal_CARD_IPOnceSetting(ip)`** — IP 0: SDPLL 0x1419:0x1D bit 15 = 0; 0x1419:0x37 low byte kept, |= 0x3000, pulse bit 5 (set,
  10 ms, clear); 0x1419:0x1A = 0 then |= 0x0400; SC_GP_CTRL 0x1133:0x25 |= 0x80; clock register 0x1038:0x43 = (… & 0xFF80) |
  0x40. IP 1: 0x141C:0x37 bit 5 cleared, 10 ms, set; 0x1133:0x25 |= 0x08; 0x1038:0x45 = (… & 0xFF80) | 0x40. Then (both) fill
  the pad table from padmux.
- **`Hal_CARD_SetClock(ip, hz)`** — when `gu32_EnClkPhase[ip]`: clear the phase block (0x1030:0x01 = 0; bank 0x1039 regs 0x00,
  0x01, 0x03, 0x04, 0x33, 0x35, 0x36, 0x37 = 0, 0x34 = 2) and clock register &= 0x807F. Clock register = (… & 0xFF80) | 0x40,
  SC_GP_CTRL 0x25 |= 0x80 (IP 0) / 0x08 (IP 1). **IP 0 with UHS timing** (`ge_BusTiming[0]` ∈ 5, 6, 7, 8): 0x1038:0x44 = (… &
  0x0FFF) | 0x4000, clock register |= 0x38 (SDPLL source), SDPLL 0x1419:0x06 bit 0 pulse, synthesizer by frequency (100 MHz:
  0x19 = 0x45, 0x18 = 0x1EB8; 200 MHz: 0x24 / 0x03D8; 48 MHz: 0x24 / 0xD944), 0x04 = 6, 0x05 = (… & ~7) | 1, 0x07 &= ~0x0400,
  100 µs. **Otherwise** the source field (bits 5:2) of the clock register by frequency: 48 MHz → 0 (left as is), 43.2 MHz → 1
  (|0x04), 40 → 2 (|0x08), 36 → 3 (|0x0C), 32 → 4 (|0x10), 20 → 5 (|0x14), 12 → 6 (|0x18), 300 kHz → 7 (|0x1C); other values
  leave it. IP 1 at 48 MHz with `gu32_EnClkPhase[1]`: phase block with 0x1039:0x01 = 0x48, 0x34 = 3 and clock register |= 0x2120
  | TXphase << 9 | RXphase << 11 (both 0 on this board).
- **`Hal_CARD_FindClockSetting(ip, hz)`** — walks the per-IP table (`.rodata+0xD8`, 16 entries) to the first entry ≤ hz: IP 0:
  200 M, 100 M, 48 M, 43.2 M, 40 M, 36 M, 32 M, 20 M, 12 M, 300 k; IP 1 / 2: 48 M … 300 k.
- **`Hal_CARD_devpm_GetClock(ip, &clk, &gp)` / `devpm_setClock(ip, clk, gp)`** — save / restore 0x1133:0x25 and the IP's clock
  register over suspend; restore runs `IPOnceSetting` first and, for IP 0, `SetPADVdd(0, 3.3 V, 1 ms)`.
- **`Hal_CARD_SetPADVdd(ip, pad, mode, delay_ms)`** — PLL bank n0 reg 0x37: 3.3 V (mode 0) clears bits 4 and 2, 1.8 V (mode 1)
  sets them; poll reg 0x10 bit 12 (3.3 V ready) / bit 13 (1.8 V ready) every 1 ms, 20 tries; timeout prints ">>>>> Over Time
  during 3.3v/1.8v setting" and returns 1; then sleeps `delay_ms`; 0 = ok.
- **`Hal_Card_DrvSetSD30(field, level)`** — PAD_SD0 drive strength for the SD 3.0 pads, register **0x103C:0x7E** (0xFD2079F8).
  `level` is clamped to 7 and its three bits are interleaved: field 0 (data) → bits 0, 2, 4 (mask 0x15); field 1 (clk + cmd) →
  bits 1, 3, 5 (mask 0x2A); read-modify-write of only those bits.
- **`Hal_CARD_DrvCtrlPin(ip, pad)`** — applies `ge_{Clk,Cmd,Data}Driving[ip]` (DT `slot-*-driving`, −1 = leave): IP 0 (pad order
  < 2): `DrvSetSD30(1, min(clk, cmd))` when not both −1, `DrvSetSD30(0, data)` when data ≠ −1. IP 1: per pad order the pins and
  maximum level — 0: CLK 151, CMD 152, data 150 / 149 / 154 / 153, max 3; 1: 32 / 33, 40 / 41 / 35 / 34, max 1; 2: 98 / 99,
  97 / 96 / 101 / 100, max 1; 3: 113 / 114, 112 / 111 / 116 / 115, max 1; 4: 46 / 47, 45 / 44 / 49 / 48, max 1 — each set with
  `MDrv_GPIO_Drv_Set(pin, min(level, max))`. On this board slot 0 gets clk / cmd 3, data 3; slot 1 level 0 (its 0 ≠ −1).
- **`Hal_Check_ClkCmd_Interrelate(ip, pad)`** — true for IP 0 with pad order < 2 (clk and cmd share one field).
- **`Hal_CARD_InitPADPin(slot)`** — CDZ pad pull-up; CLK and CMD drive 1, CMD pull-down; data pads pull-down + drive 1; then
  `PullPADPin(slot, 0)`. **`Hal_CARD_PullPADPin(slot, on)`** — off: CLK low / GPIO-out-low, CMD and data pull-down, output low,
  PadVal 0; on: CLK output-disable, CMD and data pull-up + output-disable, then the pad group mode back to SD.
- **`Hal_CARD_ConfigSdPad(slot)`** — without padmux: set the data-group and CDZ pad-group modes.
- **`Hal_CARD_ConfigPowerPad(slot)`** — without padmux: PWR pad to its mode; then `PowerOff(slot, 0)`.
- **`Hal_CARD_PowerOn/PowerOff(slot, delay_ms)`** — PWR GPIO **low = power on** (`slot-en-pwr-high-valid` 0; with 1 inverted),
  then sleep; slot ≥ 2 → "sdmmc error ! [func][line]".
- **`Hal_CARD_ConfigCdzPad(slot)`** — CDZ pad as GPIO input (`Pad_Set`, `Pad_Odn`), group mode without padmux.
  **`Hal_CARD_GetCdzState(slot)`** — true when the CDZ pad reads **0** (card present, active low); `_GetCardDetect` inverts it
  when a global flag is set and returns 1 when a per-slot flag is set [R code; I: the flags are `revcdz` and `slot-fakecdzs`].
- **`Hal_CARD_GetPadInfoCdzPad/PowerPad(slot)`** — padmux pad of PUSE CDZ / PWR of the slot.
- **`Hal_CARD_DumpPadMux(ip 0)`** — debug only: prints seven CHIPTOP words, 8-bit addresses 0x101EA0, 0x101E24, 0x101E18,
  0x101E06, 0x101E10, 0x101E0E, 0x101E1E (16-bit regs 0x50, 0x12, 0x0C, 0x03, 0x08, 0x07, 0x0F), with the conflict rules in its
  strings (`reg_allpad_in`, `reg_test_in/out_mode`, `reg_spi0_mode`, `reg_fuart_mode != 4,6`, `reg_sdio_mode == 0x100`,
  `reg_pwm0/2_mode`, `reg_i2s_mode`, `reg_ttl_mode` must be 0).
- **`Hal_CARD_TransMIUAddr(a)`** = a − 0x20000000 (CPU physical → MIU address).

#### 5.1.5 Tuning and bus voltage [R; generic i6c code]
`ms_sdmmc_switch_busvdd` → `SetPADVdd` (1.8 V with the clock stopped around it, 10 ms) only when a per-IP flag is set [I:
`slot-support-sd30`]; `ms_sdmmc_exec_tuning` — SDR50 / SDR104: 18 TX phases (`Hal_SDMMC_SetPhase(ip, 0, p)`: phase < 9 → skew
register = p << 12, SKEW_SUM 0x40; else (p−9) << 12, 0x840), CMD19 each, pass map, `FindFitPhaseSetting(ip, 17)`; DDR50: 7 DQS
phases (`SetPhase(ip, 1, p)` → DQS_SUM = (p+1) & 7); errors print "Fit Phase Finding (SDR/DDR)". The DDR50 path is not used with
a microSD card in SD / UHS mode [I].

#### 5.1.6 Per-slot sysfs [R]
Attribute group created on the platform device by `ms_sdmmc_probe` (`sysfs_create_group`), removed in `remove`:

| file | mode | behaviour |
|---|---|---|
| `get_sdmmc{0,1,2}_clock` (`sdmmcN_get_clk_show`) | r | `gu32_SdmmcClk[N] / 1000` as "%dKHz\n" |
| `get_sdmmc{0,1,2}_status` (`sdmmcN_get_status_show`) | r | last result of slot N (`_Get_Sdmmc_Status`): 0 "OK", 1 "Err_<Read CRC Error>", 2 "Err_<Write CRC Error>", 4 "Err_<Write Timeout>", 8 "Err_<CMD No Response>", 0x10 "Err_<Response CRC Error>", 0x20 "Err_<Read Timeout>", 0x80 "Err_<Card Busy>", 0x100 "Err_<Wait Event Timeout>", other "Err_<Card Not Recognized>"; printed as "SD Card Status : %s" when no command was recorded (−1), "SD Read Status : CMD_%d %s" for CMD17 / 18, else "SD Write Status : CMD_%d %s" |
| `sdmmc{0,1,2}_reset` (`sdmmcN_reset_store`) | w | number > 0 → `mmc_claim_host`, `mmc_hw_reset`, release; 0 → "Please enter a number greater than 0…"; slot status 0x8000 (no card) → "No card detected in current slot!" and `-ENODEV`; non-number `-EINVAL` |
| `sdmmc_driving_control` | w | "slot sig level", "slot level" or (one slot) "sig level" / "level"; sig = clk / CLK, cmd / CMD, data / DATA or all; clk and cmd are tied when `Interrelate`; updates `ge_*Driving` and calls `Hal_CARD_DrvCtrlPin` (§5.1.4); the help text is the 5.10 one |

Slot 2 files exist but slot 2 is not configured (slotnum 2).

#### 5.1.7 Which slot carries the microSD socket
Slot 0 → IP 0 → SD0 pads, PWR PAD_SD_GPIO0, CDZ PAD_SD0_CDZ, max 200 MHz, SD 3.0 (1.8 V switch + tuning), drive 3; slot 1 →
IP 1 → SD1 pads, PWR PAD_SD1_GPIO0, CDZ PAD_SD1_CDZ, max 48 MHz, no SD 3.0 [R]. Static data cannot tell which pads reach the
socket [R]; **likely slot 0** [I: the only UHS-capable slot, drive strength set only there, U-Boot uses a single SD device
`mmc 0`]. Not read on this camera (CAMERA-CHECKS K43: `readlink /sys/block/mmcblk0/device` — `mmc0:` = slot 0 host, `mmc1:` =
slot 1 — or the two `get_sdmmc*_status` files with a card inserted, or the CDZ GPIO levels of pads 25 and 148).

### 5.2 SATA (AHCI glue in the kernel + `mdrv-sata-host.ko`) — PHY brought up at every boot, no disk



#### 5.2.1 What is where [R]
| part | location | source |
|---|---|---|
| `ahci_port_apply_hd_link`, `ss_sata_ssc_enable`, `ahci_port_phy_fsm_reset` (all exported) | kernel, `drivers/sstar/sata_host/mdrv_sata_common.c` | i6c `mdrv_sata_common.c` (where `ahci_port_phy_fsm_reset` is under `#if 0`; i6e has it in `libata-core.c`) |
| libata hooks | kernel `sata_link_hardreset` calls `ss_sata_ssc_enable` and `ahci_port_apply_hd_link`; `ata_std_postreset` calls `ss_sata_ssc_enable` (call-site scan) | vendor-patched `drivers/ata/libata-core.c` (i6c `libata-sata.c` / `libata-core.c` have the same calls) |
| platform driver "ahci_sstar" (`sstar,sata`), PHY / HAL | `mdrv-sata-host.ko`, loaded by `/customer/demo.sh` line 26 | i6c `mdr_sata_host_ahci_platform.c` + `mercury6/mhal_sata_host{,_ahci}.c` — an **older platform revision** (DT names below) with signal-tuning functions that i6c has only in `mercury6p/` and `infinity7/` (`hal_sata_set_VppLevel`, `hal_sata_set_de_emphasis`, `hal_sata_set_db_level`, `hal_sata_set_cts_pattern`, `hal_sata_set_testGen`, `ss_sata_applyDmhs`); binary-only `sstar_ahci_suspend/resume`, `sstar_ahci_sata_bus_address`; absent: `sstar_ahci_qc_issue`, `sstar_ahci_fill_sg`, `sstar_ahci_sw_activity`, `sstar_sata_wait_reg` (inlined or older) |

#### 5.2.2 Kernel functions — binary vs i6c [R]
- `ahci_port_phy_fsm_reset(link)`: if `priv->phy_mode ≤ 1`: SATA PHY-A reg 0x50 bit 15 cleared, ~1 µs, set (address
  `hpriv->mmio − 0xBEEC0` = bank 0x1527 in the source comment's naming). The same as the i6c `#if 0` body. **No caller** in the
  image or the module (exported only) [R scan].
- `ahci_port_apply_hd_link(link)`: SControl = (SControl & 0xF0) | 0x300, 1 ms, flush, 2 ms; SStatus speed field (bits 7:4) =
  `priv->hd_link` → 0; else up to 5 × (DET = 1 pulse 1 ms, 2 ms, recheck) with "%s ,retry = %d success/fail"; −1 on failure. The
  same as i6c except that the `plat_data == NULL` ("i am pcie") guard is missing.
- `ss_sata_ssc_enable(link, enable)` — **older**: two arguments (i6c adds `spd`), runs when `priv->ssc_mode ≠ 0` (i6c: `== 1`),
  prints "%s, enable = %d", port 0 only, writes bank 0x143C (PHY_ANA): enable → reg 0x44 = 0, 0x40 = **0x1800**, 0x44 = 1, 0x43 =
  0x04C2, 0x44 = 0x0110, 0x42 = 2; disable → 0x44 = 0, 0x40 = 0x0A3D, 0x44 = 1, 0x43 = 0, 0x44 = 0, 0x42 = 0; 10 µs between
  writes, 20 ms at the end. i6c also writes 0x41 = 0x17 and uses 0x1906 for Gen1 / 2. DT `ssc_mode = <0>`, so never active here.

#### 5.2.3 Module: DT, init sequence, sysfs [R]
DT `sata { compatible = "sstar,sata", "sstar,sata-ahci"; reg-names = "ahci", "ahci_port0", "ahci_misc"; reg = <0x1f346800 0x100
0x1f346900 0x100 0x1f287400 0x200>; interrupts = <GIC_SPI 0x38 4>; clocks = <4 clocks>; phy_mode = <2>; phy_setting = <0x4200>;
ssc_mode = <0>; gen1_gen2_Vpp = <0xff 0xff>; gen3_de_emphasis = <0xff 0xff 0xff>; }` (no `status` = enabled). The `dwc3` node
uses the **same SPI 0x38**: the USB3 and SATA controllers share the combo PHY (bank 0x1524 is "USB3_SATA_PHYA" in the M6 header;
§4.1). The property names are the old ones; i6c reads `tx_ssc_mode`, `rx_eq_*`, `tx_gen1_gen2_Vpp`, `tx_gen3_de_emphasis`
instead.

`ahci_sstar_probe`: priv alloc (0x34 B), `of_match_node`, ioremap `ahci_port0` and `ahci_misc`, `__ss_sata_get_phy_mode` (reads
`phy_mode` (default 2), `phy_setting`, `ssc_mode`, prints "[SATA] phy_mode =%d , phy_setting =0x%X , ssc_mode =0x%X";
`gen1_gen2_Vpp`, `gen3_de_emphasis` printed only when not 0xFF), **`ss_sata_init`**:
1. misc init → `MHal_SATA_Clock_Config(misc, port, 1)` + `_MHal_SATA_Trim_init`: eFuse word **101F:54** (the word the USB3 PHY
   compares, §4.1.4) → PHY-D 143B:3C / 46 / 50 fields, prints "SATA_TX_R50 trim=0x%04X", "SATA_RX_R50 …", "SATA_TX_R50_IBIAS …"
   (boot log 0x0012 / 0x0012 / 0x0032).
2. PHY init = i6c `mercury6/mhal_sata_host_ahci.c ss_sata_phy_init` **write for write**: 1524:03 = 0; 143C:20 = 0x0100; 143C:30 =
   (… & ~0xF) | 0x1008; 143C:33 = 0x0500; 143C:60 = 2; 143C:70 = 0x62; 143C:3E = 0x8000; 143C:04 = 2; 143C:44 bit 0 pulse; 143B:0A
   = 0x440A, 143B:26 = 0x1905, 143B:3E = 0xB659, 143B:40 = 0xD819, 143B:61 = 0x4000, 143B:64 = 0x44, 143B:12 = `phy_setting` (0 →
   0x4200); 143C:54 |= 0x8000, 143C:58 |= 0x100, 143C:15 |= 4; 143C:05 = `phy_mode`.
3. AHCI: HOST_CTL = HOST_RESET, wait ≤ 500 ms ("SATA host reset fail!" → −1), AHCI_EN up to 5 tries,
   `MHal_SATA_Setup_Port_Implement`, "sstar sata HW settings done!!!".

Then `ahci_platform_get_resources`, `hpriv->plat_data = priv`, `hpriv->flags |= 0x4000`, `ss_sata_applySetting` (`bFirstOOB = 1`,
`hd_link = __ss_ahci_store_hd_link()`: 5 × (PxSCTL DET pulse 0x31 → 0x30, 500 ms, read PxSSTS; count speeds 1–3 when DET = 3 and
IPM = 1) → the most frequent speed), `ss_sata_applyDmhs` (Vpp / de-emphasis from the DT arrays when not 0xFF), enable
resources, `ahci_platform_init_host` (`sstar_ahci_port_start` allocates the command list / FIS / table with
`dmam_alloc_coherent` and programs the bus addresses through `Chip_Phys_to_MIU`; `sstar_ahci_qc_prep` builds the command
table), class device **`/sys/class/mstar/sstar_sata`** with:
- `sata_dump` — read: "link[%d] = %d ," × 4 from the hd-link vote; write: no-op;
- `de_emphasis` — write "%d %d %d" = enable, db 0–2 (−1.5 / −2.5 / −3.5 dB Gen3), level 0–7 (0.4 … 1.1 V); read shows the current
  Gen1 / 2 / 3 Vpp and Gen3 de-emphasis;
- `cts_pattern` — write a CTS pattern index (HFTP / MFTP / LFTP / LBP / SSOP …); read;
- `test_gen` — write 0 / 1 / 2 → pattern generator 1.5 / 3.0 / 6.0 Gbps through `hal_sata_set_testGen` = bits 3:2 of 0x1F2876A8
  (143B:2A).

"[%s] , SSTAR AHCI SATA registered". Suspend / resume = `ahci_platform_suspend/resume`; remove disables the resources and
destroys the class device.

#### 5.2.4 Use on this camera [R / I]
There is no SATA connector. The boot log ends the SATA part at "sstar sata HW settings done!!!" (the KERN_INFO "registered" line
and any ata port lines are below the console level) [R]. The module costs the PHY bring-up — `__ss_ahci_store_hd_link`'s
5 × 500 ms link votes, 2.5 s of `msleep` at boot, ≈3 s in all [I from the code] — and the shared PHY's power.

## 6. PWM and notify

### 6.1 PWM (`sstar,infinity-pwm`) — probed, no pad routed, no user

#### 6.1.1 Source [R]
The Linux part is i6e `drivers/sstar/pwm/mdrv_pwm.c` (names `ms_pwm_probe`, `group_*_in`, `mstar_pwm_*`); the HAL carries the old
`DrvPWM*` / `MDEV_PWM_*` names of i6e `infinity*/mhal_pwm.c`, but every HAL body reads as the logic of i6c
**`drivers/sstar/pwm/mercury6/mhal_pwm.{c,h}`** (newer `MHal_PWM*` API on `struct sstar_pwm_chip`) — the same register map,
channel addressing, divider table, group / sync / IRQ algorithm and quirks (§6.1.4), with the differences of §6.1.5. So the M6
PWM HAL is published, as that i6c file. 

#### 6.1.2 DT and resources [R]
`pwm { compatible = "sstar,infinity-pwm"; reg = <0x1f203200 0x400>; npwm = <11>; clocks = <&CLK_pwm>; interrupts = <GIC_SPI 0x54 4>;
pad-ctrl = <6 7 8 9 0x73 0x74 0x28 0x20 0x88 0x89 0x8a>; status = "ok"; }`. Platform driver "sstar-pwm". `reg` is **not
ioremapped**: the driver keeps the physical start (0x1F203200) and accesses `phys − 0x22000000` = the static RIU map (as i6e's
commented-out `devm_ioremap_resource` shows). `pm_group_base` (optional, absent) would replace `group_data`. Boot log:
`[NOTICE]pwm-isr(86) success. If not i6e or i6b0, pls confirm it on .dtsi` (virq 86 = SPI 0x54).

#### 6.1.3 Register map (RIU banks 0x1019–0x101A, base phys 0x1F203200) [R binary = S i6c `mercury6/mhal_pwm.h`]
Channel block offset (bytes from base): ch 0–3 → `ch·0x80` (bank 0x1019 regs 0x00 / 0x20 / 0x40 / 0x60); ch 4–10 → `0x200 +
(ch−4)·0x40` (bank 0x101A regs 0x00, 0x10, … 0x60). 16-bit registers, reg n at block + n·4:

| reg (in block) | name | content |
|---|---|---|
| 0x00 / 0x01 | SHIFT_L / SHIFT_H | begin (shift) of pulse 0, 18 bits (H: bits 1:0) |
| 0x02 / 0x03 | DUTY_L / DUTY_H | end (duty) of pulse 0, 18 bits |
| 0x04 / 0x05 | PERIOD_L / PERIOD_H | period − 1, 18 bits |
| 0x06 | DIV | clock divider − 1 |
| 0x07 | CTRL | bit 0 VDBEN_SW, bit 1 DBEN (double-buffer enable), bit 2 DIFF_P_EN, bit 3 SHIFT_GAT, bit 4 POLARITY |
| 0x08 / 0x09, 0x0A / 0x0B, 0x0C / 0x0D | SHIFT2 / DUTY2 … SHIFT4 / DUTY4 | begin / end of pulses 1–3 (16 bits) |
| 0x10 (in the blocks of ch 0, 1, 2 only) | ROUND of group 0, 1, 2 | round count (`DrvPWMGroupSetRound/GetRoundNum`) |

Group / global registers (bank 0x1019): 0x71 GROUP_HOLD (bit g), 0x72 GROUP_STOP (bit g), 0x73 GROUP_ENABLE (bit g), 0x74
GROUP_JOIN (bit = channel; "bit 0/4/8 must stay set for sync mode"), 0x75 GROUP_INT (bits 2:0 hold events, bits 5:3 round events,
per group), 0x76 DUTY_QE0 (bit 0), 0x77 HOLD_MODE1 (bit 0, "keep low"), 0x7E PWM_OUT (output levels), **0x7F SW_RESET** (bit =
channel held in reset = output off; bits 11–13 = group 0–2 reset). Groups: 0 = ch 0–3, 1 = ch 4–7, 2 = ch 8–10 (join mask 0xF,
0xF, 0x7).

Clock: the HAL assumes **12 MHz** (`pwmclk = 3, common = 250` → one tick = 250/3 ns, for any `clk_get_rate`); divider table
`clk_pwm_div` = {1, 2, 4, 8, 32, 64, 128}. The `pwm` clock select: `kernel-platform.md` §5.1.

#### 6.1.4 Behaviour [R]
- **probe**: devm alloc (0x3C B: `pwm_chip` + clk + base + pad_ctrl + group_data + irq); `DrvPWMGroupCap()` = 1 → class device
  **`/sys/class/mstar/motor`** (msys class) with the 14 attributes of §6.1.6; MEM resource (missing → "Can't get I/O resource
  regs for pwm", returns **2**, a positive value; the same in i6e); clk get + prepare + enable; `pwm_chip` ops `mstar_pwm_ops`,
  base −1, `npwm` (default 4); pad table from `pad-ctrl`; `pm_group_base` → `group_data` (which overwrites the motor device
  pointer stored there before — harmless, nothing reads it back; the same in i6e); `DrvPWMInit(ch)` for every channel (**no
  `DrvPWMPadSet` in the probe loop**, unlike i6e); `pwmchip_add` (→ `/sys/class/pwm/pwmchip0`, 11 channels); all three groups
  disabled; IRQ `request_threaded_irq(PWM_IRQ, "pwm-isr")`.
- **`DrvPWMInit(ch)`**: reads the SW_RESET bit (bit 0 for ch 10 — a vendor quirk, below) and the period through `GetConfig`;
  clears DUTY_QE0; if the channel is out of reset with a period set (left running by the boot loader) it is marked enabled, else
  disabled (reset bit set).
- **`pwm_ops`** (stock PWM framework, legacy `config/enable/disable/set_polarity` + `get_state`): `config(duty_ns, period_ns)` →
  `DrvPWMSetConfig`: pick the first divider with `period < div·2^18·250/3`, period ticks = round(period·3/(div·250)), duty ticks
  the same (min 1), cached ns values, if the channel is enabled set / clear its reset bit for duty 0 / ≠ 0; prints "clk=%d,
  u16Div=%d u32Duty=0x%x u32Period=0x%x" (KERN_ERR); writes DIV = div−1, PERIOD = ticks−1, DUTY = ticks−1. `enable` → DBEN +
  VDBEN_SW set, reset bit cleared if DUTY ≠ 0 (else set), then the pad is set only if padmux is inactive or the PWM pad is not
  muxed (`_pwm_is_pad_set`: `mdrv_padmux_getpad(0x80000 + ch·0x100)` for ch 0–9; 10 → false) — and `DrvPWMPadSet` is an **empty
  function** in the binary, so the PWM driver never muxes a pad. `disable` → DBEN / VDBEN_SW, reset bit set. `set_polarity` →
  CTRL bit 4. `get_state` → cached period / duty ns (recomputed from the registers when the cache is 0), enable flag, polarity
  bit.
- **Group ("motor") mode**, for stepper / motor waveforms: per channel up to 4 pulses (begin / end in ‰ of the period), period by
  frequency (`DrvPWMSetPeriodEx(ch, Hz)`: ticks = clk_rate / Hz / (DIV+1), clamped 2 … 2^18, prints "reg=0x%08X clk=%d,
  period=0x%x"), polarity, round count; the values are cached and written to the registers by `MDEV_PWM_MemToReg(group)` when a
  group is enabled while the cache is dirty (`isSync == 0`) and from the hold interrupt; `DrvPWMGroupEnable(g, 1)` sets
  GROUP_ENABLE bit g and releases the reset bits of the joined members + group bit 11+g, `isSync = 1`; `(g, 0)` the opposite.
- **`PWM_IRQ`**: reads GROUP_INT; per hold event of group g: if HOLD_MODE1 = 0 write the cache (MemToReg), clear the hold,
  isSync = 1, print "hold_int GrpId:%d(event:x%x)"; per round event: if new data (isSync = 0) re-enable the group (disable +
  enable), else disable it and set its round to 0; print "round_int GrpId:%d". **Returns IRQ_NONE** (0) always (the same in the
  i6c source) — the core would count it as spurious if it ever fired.
- Suspend / resume: disable / enable the IRQ, clock off / on, `DrvPWMInit` all channels.

Quirks kept from the source: the SW_RESET bit for channel 10 is bit 0 in `SetConfig` / `Init` (`BIT0 << ((id == 10) ? 0 : id)`)
but bit 10 in `Enable` / `disable`; `GroupHoldM1(0)` does `CLRREG16(reg, 0)` (clears nothing, only prints "hold mode1 dis!");
`SetConfig` with a period above the largest divider range leaves `div = 0` (division by zero) [R; I: never reached with sane
periods].

#### 6.1.5 Binary vs i6c `mercury6/mhal_pwm.c` (beyond the renaming) [R]
| function | difference |
|---|---|
| `DrvPWMSetEndToReg` / `SetBeginToReg` | write through the channel's real block offset (`GetGrpAddr`); the source uses `base + id·0x80` for every channel, wrong for ch 4–10. The loop runs `arg_no = 0 … ArgNum` **inclusive** (the stored value is the last index written by `SetEnd/SetBegin`); the source's `<` would skip the last pulse |
| `group_mode_in` (sysfs) | calls `DrvPWMGroupJoin(id, en)` **and `DrvPWMDiv(id, en)`** (DIV register = 0 / 1, "workaround for group hold/round malfunctional when div is zero"); i6c mdrv has that call commented out, i6e mdrv lacks it |
| `DrvPWMPadSet` | empty (the i6c `MHal_PWMPadSet` body is commented out too — the same effect) |
| `group_round_get` attribute | mode 0200, store only (`group_round_out`: parses "%d %hd", reads the round count into a local — no visible effect); i6e / i6c: 0644 with a show |
| everything else | the same as the i6c mercury6 logic (`DrvPWMGetGrpAddr`, `DrvPWMDiv`, `DrvPWMGroupStop`, `_pwm_is_pad_set` + `mdrv_padmux_active` in `mstar_pwm_enable` included) |

#### 6.1.6 sysfs [R]
`/sys/class/pwm/pwmchip0/` (stock: `export`, `pwmN/{period,duty_cycle,enable,polarity}`) and `/sys/class/mstar/motor/`:

| attribute | mode | input | action |
|---|---|---|---|
| `group_mode` | 0200 | "%d %d" ch, en | join / leave group (leaving ch 0 / 4 / 8 refused: "Always enable BIT_%2d for sync mode!"), DIV = en, DBEN, channel off |
| `group_period` | 0200 | "%d %d" ch, Hz | `SetPeriodEx` |
| `group_begin` / `group_end` | 0200 | "%d %d [%d %d %d %d]" ch, up to 5 values (‰) | `SetBegin/SetEnd(ch, i, v)` |
| `group_round` | 0200 | "%d %d" group, n | ROUND = n |
| `group_enable` | 0200 | "%d %d" group, en | refuses a no-op change; enable: stop 0, hold 0, enable; disable: enable 0, stop 0, hold 0 |
| `group_hold` / `group_stop` | 0200 | "%d %d" group, en | HOLD / STOP bit (stop also disables the group) |
| `group_polarity` | 0200 | "%d %d" ch, pol | cached polarity |
| `group_hold_mode1` | 0200 | "%d" | HOLD_MODE1 bit 0 set (clear is a no-op) |
| `group_duty_qe0` | 0200 | "%d %d" (group ignored), en | DUTY_QE0 bit 0 |
| `group_round_get` | 0200 | "%d %hd" | no effect |
| `group_info` | 0444 | — | HoldM1, per group members / enable / reset / hold / stop / round, per channel pad, sync, reset, polarity, period (Hz), begin / end (‰) |
| `group_output` | 0444 | — | returns 1 byte not written by the driver; prints "output:x%x" of PWM_OUT |

#### 6.1.7 Use on this camera [R]
No pad is muxed to PWM (the DT padmux schematic, `kernel-platform.md` §6.2; the driver's pad set is empty), and no program opens
`/sys/class/pwm` or the motor class (strings absent from prog_uvc, kjd_lvgl, rcS). The driver probes, takes its clock and IRQ,
and does nothing else.

### 6.2 notify (`ms_notify.c`) — init only [R]
Source `drivers/sstar/notify/ms_notify.c` (i6c = i6e); the binary = the source.
- `_notify_init` (initcall): `netlink_kernel_create(&init_net, NETLINK_USERSOCK (2), {.groups = 0, .input = _notify_reply})` →
  "mstar notify driver install successfully" (boot log); failure → −1. `_notify_exit`: release + "…remove successfully".
- `_notify_reply(skb)` — an **echo / broadcast server**: take a reference; if `skb->len < NLMSG_SPACE(0)` (16) drop it; else copy
  256 bytes of payload (always 256, whatever the message length — it reads past a short message inside the skb buffer [I]), free
  the skb, allocate `NLMSG_SPACE(256)` (0x110), `nlmsg_put(…, 0, 0, 0, 256, 0)`, flags 0, payload, `portid = 0`, `dst_group = 1`,
  `netlink_broadcast(sock, skb, 0, group 1, GFP_ATOMIC)`. So any process can send a 256-byte message on `NETLINK_USERSOCK` and
  every socket bound to group 1 receives it — a user-space notification bus. No ioctls, no sysfs, no DT node.
- Use on this camera: `/customer/demo.sh` line 25 runs `insmod /config/modules/4.9.227/ms_notify.ko`, which fails ("can't read …
  No such file or directory", boot log) because the driver is built in; no program opens a `NETLINK_USERSOCK` socket (no netlink
  use in prog_uvc, kjd_lvgl or their SDK part).

## 7. EMAC (`sstar-emac`) + internal PHY (`sstar_100_phy`) — two MACs probe, no cable, link down

Sources: `drivers/sstar/emac/mdrv_emac.c` (61 functions) + **i6c `emac/hal/mercury6/mhal_emac.c`** (135), `drivers/sstar/netphy/
sstar_100_phy.c` (6). The source (217 KB) matches the binary apart from these differences [R]: open / close / suspend test
`strncmp(dev->name, "eth0", 4)` around a per-port NAPI flag; the `turndrv` sysfs commands are `max` / `f10t` / `an` / `dir_on` /
`dir_off` / `swing_100` (i6c: `rx_imp` / `timing`); `phyStatusWR` = `phy_r` / `phy_w` through `mdiobus_*` (i6c `phyE_r` /
`phyE_w`); binary-only sysfs `txq_single_mode` (`txq_single_mode_show/store`). netphy = the source. 

- **DT** (two nodes, no `status` = enabled): `emac0` regs MAC 0x1F2A2000 (0x800), 0x1F343C00 (0x600), 0x1F2A3000 (0, "internal
  PHY ATOP" in the i6c binding [I]), IRQs SPI 0x3A / 0x95 / 0x79, 5 clocks, `bus-mode = <1>`, `pad = <0x1f2079b8 0xf 0x0>`,
  `pad-rmii = <0x1f2079b8 0xf 0x4>`, `phy-handle` → `mdio-bus@emac0/ethernet-phy@0 { phy-mode = "mii"; }`; `emac1` regs
  0x1F2A2800, 0x1F345A00, IRQs 0x78 / 0x95 / 0x7A, `pad = <0x1f2079b8 0xf0 0x40>`, `ethernet-phy@1 { phy-mode = "rmii"; }`.
  Properties the probe reads: `txd_num`, `txq_num_sw`, `led-orange`, `led-green`, `bus-mode`, `pad`, `pad-rmii`, `pad_led`,
  `phy-mode` (via `of_get_phy_mode`), `phy-handle`, `mdio_path`, child `mdio-bus` [R strings in `mstar_emac_drv_probe`]. The EMAC
  clocks and the ETH LED pads: `kernel-platform.md` §5.1, §6.2.
- **Probe** (per node): HAL alloc, pad / LED pad / PHY mode (register 0x1F2079B8 = bank 0x103C reg 0x6E, mask / value from `pad`),
  "EMACn PHY RESET", clocks on, TX descriptors + **DMEM buffer** `emac0_buff` 0x812 B / `emac1_buff` 0x60812 B (boot log), MIU
  protect of the RX area, MAC address from the HAL, locks, `request_threaded_irq(…, "ETH")`, NAPI weight 32, class device under
  `/sys/class/mstar/`, MDIO bus "mdio" `%s@%s` + `of_mdiobus_register` (scans 32 addresses: boot log "invalid PHY address" + scan
  lines), `register_netdev` (eth0, eth1). PHY at emac0: uid 0x11112222 "SStar 10/100 Ethernet Phy" (the netphy driver); emac1:
  "Generic PHY", uid 0 (nothing attached) [R boot log].
- **sysfs** (per MAC class device) [R `device_create_file` list]: `turndrv`, `phyStatusWR`, `info`, `dlist_info`, `reverse_led`,
  `sw_led_flick_speed`, `check_link_time`, `check_link_timedis`, `txq_single_mode`. Network side: the stock netdev, ethtool and MII
  ioctls of the source. Suspend / resume hooks as in the source.
- **netphy** `sstar_100_phy`: `probe` (link-state helper alloc), `config_init` / `config_aneg` / `reset` empty (return 0),
  `read_status`: `genphy_read_status`, then only at link up + 100 Mb/s: read vendor regs 0x21 / 0x22 and three times 0x2E; on a
  polarity / state mismatch (0x21 bit 8 vs 0x22 bits 9:8) or a stable 0x2E state 0x2xxx / 0x3xxx counted more than 10 times in a
  row, restart autonegotiation (BMCR = 0x1200) and reset the counter; `remove` frees the counter [R decompile].
- **Use on this camera**: no Ethernet connector; both links stay down ("EMAC Link Down" ×2, boot log); nothing configures eth0 /
  eth1 [I: no `ifconfig` / `udhcpc` in rcS / demo.sh]. Costs at boot: two DMEM buffers (≈0.4 MiB), the MDIO scans, two IRQs, the
  PHY powered. The DMEM list at boot: `kernel-platform.md` §10.2.

## 8. Engines

### 8.1 IVE (`sstar,infinity-ive`, intelligent video engine) — probed, no user

Sources: `drivers/sstar/ive/{mdrv_ive.c, drv_ive.c, hal_ive.c, hal_clk.c, hal_ive_reg.h}` + `include/mdrv_ive_io{,_st}.h`. The
binary bodies follow i6c (6 functions closer to i6c, 2 to i6e; `ive_hal_set_coeff_alpha_blending` exists only in i6c), **but the
ioctl ABI is the i6e one** (§8.1.2). Register field map = `hal_ive_reg.h` (the same in both trees for the fields the binary
touches [I: not diffed field by field]). 

#### 8.1.1 Device [R]
DT `ive { compatible = "sstar,infinity-ive"; reg = <0x1f232000 0x100 0x1f232200 0x100>; interrupts = <GIC_SPI 0x4D 4>; clocks =
<&CLK_ive>; status = "ok"; }` — banks **0x1119** and **0x111A**; `CLK_ive` (0x1F2071A8, 8 parents); the probe sets the clock to
**parent 0** and enables it only while a job runs. Module init: chrdev region "mstar_ive", class **`mstar_ive_class`**, platform
driver; probe: devm alloc 0x170 B, both MEM resources (missing → error), `ive_drv_init` (HAL bases), clocks
(`of_clk_get_parent_count` of the node), `request_threaded_irq(…, "ive isr")` (the hard handler acknowledges, the thread runs
the post-process), cdev with `ive_fops`, device **`/dev/mstar_ive0`**. No sysfs, no proc.

#### 8.1.2 ABI: one ioctl [R]
`IVE_IOC_PROCESS` = **0x40044901** (`_IOW('I', 1, ive_ioc_config*)` — the size field says 4, the driver copies 0x74 bytes).
`ive_ioc_config` as the binary copies it = **116 bytes = the i6e `mdrv_ive_io_st.h` layout** (i6c's is 144 B: 64-bit addresses):

| offset | field |
|---|---|
| 0x00 | `u32 op_type` (`IVE_IOC_OP_TYPE`) |
| 0x04 | `ive_ioc_image input` {u32 format; u16 width; u16 height; u32 address[3] (physical, bit 31 = MIU1); u16 stride[3]} (28 B) |
| 0x20 | `ive_ioc_image output` |
| 0x3C | union of the per-op coefficients (56 B; the i6e member set, `coeff_ncc` = 32-bit physical address of the NCC result buffer, `coeff_map` = 32-bit address of the 256-entry map) — except op 0x1F below |

For op 0x1F (MATRIX_TRANSFORM) the kernel reads the nine matrix coefficients as **32-bit** words (`{u32 chl_mode; u32 ctrl_mode;
s32 matrix[9]}`), not the i6e header's `__u16[9]`; C1 / C2 / C3 use 1 / 4 / 9 words. The CSC coefficient and offset fields are
12-bit two's complement in the engine (i6e `hal_ive_reg.h`) [S], and the engine numbers colour channels in reverse memory order.
`libmi_ive.so` fills the structure in this layout (116 B, i6e image struct, 32-bit addresses, op 0x1F as 32-bit words) and also
uses op 0x21 ALPHA_BLENDING, which this kernel accepts (`ive-video-analytics.md` §4.1).

Ops (`op_type`, the same numbering in both trees): 0x00 FILTER, 0x01 CSC, 0x02 FILTER_AND_CSC, 0x03 SOBEL, 0x04 MAG_AND_ANG,
0x05 ORD_STA_FILTER, 0x06 BERNSEN, 0x07 DILATE, 0x08 ERODE, 0x09 THRESH, 0x0A THRESH_S16, 0x0B THRESH_U16, 0x0C AND, 0x0D OR,
0x0E XOR, 0x0F ADD, 0x10 SUB, 0x11 16BIT_TO_8BIT, 0x12 MAP, 0x13 HISTOGRAM, 0x14 INTEGRAL, 0x15 SAD, 0x16 NCC, 0x18 LBP, 0x19 BAT,
0x1A ADP_THRESH, 0x1F MATRIX_TRANSFORM, 0x20 IMAGE_DOT, **0x21 ALPHA_BLENDING** (i6c numbering; the binary has its HAL function).
Return values (`IVE_IOC_ERROR`): 0 ok, 0x1000 in / out size different, 0x1001 image too small (width < 16 or height < 5), 0x1002
config copy failed, 0x1003 busy (a job of this file is pending), 0x1004 memory failure, 0x1005 clock failure; 3 for an unknown
ioctl.

#### 8.1.3 Behaviour [R]
`open`: per-file data (0x98 B: device, config copy, state, wait queue). `ioctl`: under the device mutex enable all clocks, copy
the config, keep the user pointer, `ive_drv_process`: size checks (skipped for MAP, HISTOGRAM, NCC, BAT; SAD skips only the in =
out check), state = 3 (queued), append to the device queue; if the engine is idle, state = 1, `ive_drv_start` (HAL: images, MIU
select from bit 31, per-op coefficients, operation, IRQ mask, start bit). ISR: check / clear the done IRQ, state 2; thread
`isr_post_proc`: dequeue, mark the finished file done (2) and wake it, start the next queued job or mark idle, clocks off. `poll`:
while state ≠ 0, register the wait queue; state 2 → return POLLIN | POLLRDNORM and reset state to 0. The engine writes results
straight into the output buffers (and the NCC / histogram buffers); nothing is copied back to user space. Suspend / resume follow
the i6e bodies. The engine-side arithmetic (Q8 CSC division, the Filter shift of negative sums, Image_Dot / Matrix rounding,
MagAndAng angle bins, SAD 8-bit scaling) is not verified on hardware (CAMERA-CHECKS P16).

#### 8.1.4 Use on this camera [R]
No program on the camera opens `/dev/mstar_ive0` (no MI_IVE user); `libmi_ive.so` (742 KB) is not linked by prog_uvc or
kjd_lvgl.

### 8.2 AES / SHA / RSA / RNG (`sstar,infinity-aes`, AESDMA engine) — algorithms registered, no user

Sources: `drivers/sstar/crypto/{mdrv_aes.c, mdrv_sha.c, mdrv_rsa.c, mdrv_cipher.c, halAESDMA.c}` (+ `hal/mercury6/halAESDMA.h` for
the register names). Revision mix [R]: the RSA ioctl path, the AES probe / ECB / CBC / CTR and most SHA glue follow **i6e**;
`HAL_AESDMA_CheckAesKey`, `HAL_AESDMA_UseAesKey`, `infinity_aes_crypt_pub`, `cipher_sha_init`, `infinity_sha256_import` follow
**i6c**. Driver range 0xC0205E60–0xC0207BF0. Four functions our symbol classification puts under `crypto/cryptodev/{examples,lib,tests}`
(`aes_encrypt`, `aes_decrypt`, `hash`, `remove_mapping`) are **not** this driver: they are the stock `crypto/aes_generic.c` statics
and `mm/vmscan.c remove_mapping`, matched by name [R: their addresses 0xC007D66A, 0xC00A92C2, 0xC01383C2, 0xC01390D4 lie in mm/,
fs/ and next to `crypto_aes_set_key`, outside the driver range].

#### 8.2.1 Hardware and DT [R]
One IP at RIU bank **0x1122** (phys 0x1F224400; `AESDMA_BASE_ADDR = SHARNG_BASE_ADDR = RSA_BASE_ADDR = 0x112200` in the M6 header
[S]): AESDMA (DMA AES-ECB / CBC / CTR, key registers + 4 hardware keys), SHA-256 engine, RNG, RSA exponentiation unit. DT `aes {
compatible = "sstar,infinity-aes"; interrupts = <GIC_SPI 0x37 4>; clocks = <&CLK_aes>; status = "ok"; }` — no `reg` (fixed
addresses); the IRQ is not requested by the binary (no `request_irq` in the crypto range); work is polled.

#### 8.2.2 Probe and resources [R]
`infinity_aes_probe`: mutex "&_mtcrypto_lock", `enableClock`, `allocMem(0x1000)` → **DMEM `AESDMA_ENG`** 4 KiB (boot log, phys
0x23847000); register the crypto algorithms; `infinity_sha_create` → `allocTempMem(0x1000)` → **DMEM `AESDMA_ENG1`** 4 KiB
(0x23848000) and the SHA shash; `misc_register(&rsadev)` → **`/dev/rsa`**.

| algorithm (`cra_name`) | driver name | type | priority | block |
|---|---|---|---|---|
| `aes` | `infinity-aes` | cipher (single block, with software fallback `fallback_*_cip`) | 300 | 16 |
| `ecb(aes)` | `ecb-aes-infinity` | blkcipher (fallback `fallback_*_blk`) | 400 | 16 |
| `cbc(aes)` | `cbc-aes-infinity` | blkcipher | 400 | 16 |
| `ctr(aes)` | `ctr-aes-infinity` | blkcipher | 400 | 1 |
| `sha256` | `sha256-infinity` | shash (import / export state) | 400 | — |

Data path [R code; S source]: data is copied into the 4 KiB DMEM bounce buffer (`Chip_Flush_MIU_Pipe` / cache maintenance), the
AESDMA FILE_IN / FILE_OUT addresses = the buffer's MIU address, XIU length, chain mode, key, IV, start, poll status; SHA-256
likewise (init value / manual mode / word count for multi-part updates, `HAL_SHA_ReadOut`). Key selection in
`infinity_aes_crypt_pub` (binary-only behaviour): key bytes "SStarU" / "SStarV" / "SStarW" / "SStarX" select hardware key 1 / 2 /
3 / 4; key length 1 with byte 2 selects key 2; otherwise the cipher key (id 0, `SetCipherKey` / `HAL_AESDMA_SetCipherKey2`);
`HAL_AESDMA_CheckAesKey(id)` failing prints "invalid key id" and returns −1; `HAL_AESDMA_UseAesKey(id)` sets bits 6:5 of the
key-select register (present in i6c `halAESDMA.c`, not called by its `mdrv_aes.c`); there is no ISR wait path.
`infinity_random` / `cipher_random_num` read `HAL_RNG_Read`. The `cipher_*` exports (AES / ECB / CTR / RSA / SHA / random
wrappers for other kernel code) have **no importer** among the kept modules [R module import tables].

#### 8.2.3 `/dev/rsa` ioctls (i6e `mdrv_rsa.h`, magic 0x66) [R binary = i6e]
| cmd | name | argument |
|---|---|---|
| 0x6692 | `MDrv_RSA_Reset` | — : `HAL_RSA_Reset` |
| 0x6693 | `MDrv_RSA_Setmode` | `struct rsa_config` (28 B): key length = `u32RSA_KeyNLen` ∈ {0x10, 0x20, 0x40} words (512 / 1024 / 2048 bit) → `SetKeyLength(len − 1)`, `SetKeyType(software key)`; else "[RSAERR] Mode error" / "KenNLen != 0x10 or 0x20 or 0x40" |
| 0x6694 | `MDrv_RSA_Calculate` | `struct rsa_config` {u32 *pu32RSA_Sig; u32 *pu32RSA_KeyN; u32 *pu32RSA_KeyE; u32 *pu32RSA_Output; u32 KeyNLen; u32 KeyELen; u32 SigLen} (lengths used as u8): checks (non-zero lengths, non-NULL pointers), load signature / N / E **inverted (word-reversed)** through the indirect 32-bit port, exponentiation start, poll status ≤ 100 jiffies ("rsa timeout!!!"), read 0x20 or 0x40 words back word-reversed into `pu32RSA_Output`, reset |
| other | | −ENOTTY |

The driver **dereferences the user pointers directly** (no `copy_from/to_user` for the key, signature and output buffers — only
the 28-byte struct is copied) [R; the same in the i6e source]. A kernel that enforces PAN / uaccess separation would fault; on this
4.9 ARMv7 kernel it works and lets any `/dev/rsa` user make the kernel read and write arbitrary user addresses of its own process
[I].

#### 8.2.4 Use on this camera [R]
No program on the camera uses AF_ALG, cryptodev or `/dev/rsa` (prog_uvc's XU1 AES is software, `prog_uvc.md` §4.5). In the kernel only
fscrypt (ext4 encryption, built in) could pick `ecb/cbc/ctr(aes)` up, and no encrypted ext4 is mounted (rootfs squashfs, SD FAT);
no dm-crypt or IPsec symbols [R kallsyms]. Cost: two DMEM pages, a clock.
