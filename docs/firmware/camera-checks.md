# CAMERA-CHECKS — every reading only the running camera can give

The open points of the specification that only the running camera can settle, deduplicated, with the read or command and
what its result decides. The component documents cite them as «CAMERA-CHECKS Kn / Pn». Static collection only — nothing
here has been run unless a document says so.

Conventions:
- The read or command is copied as the source writes it. Register notation varies between sources: `bank.reg`, `bank:reg`
  and `riu_r bank reg` all mean one 16-bit RIU register (`riu_r B R` prints it; `riu_r B` dumps the bank — `rootfs-customer.md`);
  `J:xx` = bank 1622 (NJPD, `mhal-jpd.md`); `S.xx` = IspScl unit slot (`isp-2-ispmid-mload-ispscl.md`); `W.xx` = ISP WDMA block (`isp-1-hal.md`).
  A byte address like `0x103CC4` is bank 0x103C byte offset 0xC4 = 16-bit reg 0x62 [I: conversion of this list].
- **Part 1 is read-only** (reads of registers, sysfs, procfs, `lsusb` on the host). **Part 2 changes state** (writes,
  boots without a module, replugs, provoked crashes, runs of prog_uvc with other options) — each is marked there.
- "expect / decides" keeps the origin tags of the document it comes from ([R]/[S]/[I]); a line without a tag restates that document.

## Part 1 — read-only

### 1.1 SoC identity, OTP / eFuse, boot

| # | read / command | expect / what it decides |
|---|---|---|
| K1 | `riu_r 001E 00`, `riu_r 001E 01` (chip id, GIC byte); `cat /sys/devices/virtual/mstar/msys/CHIP_ID` | chip id ≠ 0xF4 or bit 8 set ⇒ BDMA path (confirmed indirectly by the log); GIC byte = 1 per log. CHIP_ID 0xF4 or not decides whether the EPTZ library (and MVE) runs at all; any other id → `-L` works only with ready tables |
| K2 | `CamOsChipRevision()` value, or `riu_r 1214 60` (= 1 if the revision-2 branch ran) | revision 2 → SCL init writes `1214..1219:60 = 1`; other M6 boards report "soc-revision 2" → likely ours; also decides whether the VIF port-1 errata is live |
| K3 | `riu_r 101f 26`, `27`, `2c`, `2d`, `2e`, `2f`, `71` | feature-disable mask and lock word (`ipl.md`); the JPE OTP throughput cap flag (`mhal-scl-jpe-cmdq.md`) and the codec OTP throughput flag (`mhal-vcodec.md`) are the same words; `101f.26` bit 2 = VIF throughput cap [I: must be off, since 4K30 frames reach the ISP — but that path is realtime and never checks] |
| K4 | `riu_r 101F 50` … `5B` (OTP2 trim words) | which trim steps of `ipl.md` run on our chip — **101F.55 bits 7, 11, 15 change DRAM PHY values (1010.58, DQ deskew 1010.2C–2F)**; the rest trim USB/SATA/HDMI/SAR analog |
| K5 | `riu_r 0038 70` | boot-source code the ROM leaves (0x20 or 0x02 for NOR) |
| K6 | `riu_r 1420 00`, bits 3–4 | whether IPL's UPLL0 power-up step ran |

### 1.2 Clocks and power

| # | read / command | expect / what it decides |
|---|---|---|
| K7 | `riu_r 1032 19` | CPU PLL post-divider ⇒ the actual CPU clock (800 MHz if 1) |
| K8 | `cat /sys/devices/system/voltage/core_power/voltage_current` and `…/cpu_power/voltage_current` | expected 1000 and the demander list (`kernel-drivers.md`) [I: both rails at 1000 mV from probe, TEMPERATURE request] |
| K9 | `cat /sys/class/mstar/venc/ven_clock` (and `_2nd`, `_axi`) | the current codec clocks (384 MHz default per `mhal-vcodec.md`) |
| K10 | `/sys/.../jpdclk` read (prints the `CKG_jpd` rate actually set) | default request 432 MHz [R `mhal-jpd.md`] |

### 1.3 DRAM, MIU, memory heaps

| # | read / command | expect / what it decides |
|---|---|---|
| K11 | "1010.xx, 1012.xx, 1013.xx after boot" (= `riu_r 1010`, `riu_r 1012`, `riu_r 1013`) | compare with `ipl.md` to see what the kernel changed (`kernel-platform.md` lists the arbiter part) |
| K12 | `/sys/devices/system/miu/miu0/` | cross-check of the MIU client → group table read from the image (`kernel-platform.md`) |
| K13 | MMU page mode actually chosen; free-slice count of the MMU heap under load; `cat /proc/mi_modules/mi_sys_mma/mma_heap_name0` | mode 1 inferred from `lx_mem_size` 0xffe0000; the proc file shows the VA window and slice use directly |
| K14 | SRAM requests of the bound modules (`ConfigSramBuf`: mi_disp, mi_venc `dev+0x5f8/0x5fc`, mi_jpd on REALTIME binds) while the pipeline runs | which module requests SRAM and with what size |

### 1.4 Sensor interface: CSI, VIF

| # | read / command | expect / what it decides |
|---|---|---|
| K15 | "a dump of 1538/1539/153a on the running camera" (= `riu_r 1538`, `riu_r 1539`, `riu_r 153a`) | reset values of the CSI registers the vendor never writes: MAC `sw_rst`/`mac_en`/`vc_en`, ECC, SOT/EOT, error/report/PHY/FSM masks, 1539 except reset, 1538 skew/ULPS/LPCD/clock-lane override/BIST [I: unknown]; which VCs are passed is the reset default |
| K16 | `cat /sys/devices/virtual/mstar/csi0/csi_dbg_mask`, `…/csi_ints` | `dbg_mask` show prints "sensor 0 csi dbg mask = 0x…" — the enabled error-interrupt set is the reset default [I: unknown] |
| K17 | register dump of `VIFn.64[3:0]`, `VIFn.34`, `153e.0e[0]`, `153e.11[1:0]` "with and without a 2-lane sensor" | meaning of these bits (no header names them); **needs a 2-lane sensor we do not have** — with our 4-lane sensor only the "without" half is possible |

### 1.5 ISP and 3A

| # | read / command | expect / what it decides |
|---|---|---|
| K18 | read 1321.7E; 1331.00–.3d; 1312.01–.08 / .50–.58; 1322.06–.0c | 1321.7E expect 0; 1331.00–.3d expect 0 (`isp-4-iq-engine.md`); two-stage black level (§8.2); WDR box geometry for 3840 × 2160 (§8.2) |
| K19 | Get 0x1039 TemperatureInfo | expect 36 (confirms the constant temperature of 63b §2.4) |
| K20 | read back banks 1333/1334/1335/133A (YEE), 1328 (NR3D), 1316 (DPC), 1322 (WDR), 1330 (GNR) after boot | equal to our bin's Sharpness / Sharpness_EX / NR3D / DynamicDP / WDR / NRLuma_Adv manual sets through the 63b maps — confirms the bin is applied as specified and nothing (prog_uvc menu, Dummy_EX) overwrote them |
| K21 | registers 131B (AE/AWB/histogram windows) | whether prog_uvc's user 3A changes the windows or they stay at the open-time defaults (63b §4.3) |
| K22 | bank 1316.6a–6f + the SDC menuload table | whether a static defect-pixel calibration (0x2414) was ever applied or the imx307 default list is active |
| K23 | register dump under the vendor stack: `1342.40/.41/.45/.46`, IspScl `S.10` / `S.20` / `S.21–23` | whether the ISP HAL (`isp-1-hal.md`) rewrites any of them; meaning beyond "per-unit bit, pulsed" |
| K24 | content of ISP SRAMs 0, 1 and 4; SRAM 11 (WDR table) | SRAMs 0/1/4 have no writer in this build; whether the hardware uses the never-loaded WDR table in realtime |
| K25 | `ispscl_ints` of the class device `ispscl0` (msys sysfs class); `cat /sys/class/chardrv/mload/info` | WDMA-done / FIFO-full counters stay at 0 [I]; live menuload handle and its error flags. **Do not read `combie`** — reading it stops the combie log (`isp-2-ispmid-mload-ispscl.md`) |
| K26 | the AF bit of prog_uvc's ISP channel (`nCtrlFlag` bit 6); the path `MI_ISP_GetIspRoot` returns | bit 6 → whether the user AF runs at all; root path assumed `/config/iqfile` |

### 1.6 SCL, CMDQ, JPE (MJPEG encoder)

| # | read / command | expect / what it decides |
|---|---|---|
| K27 | `/proc/jpe/dev_info` "CmdQ: %p", or mi_venc's proc device table `CmdqHandle`, while UVC MJPEG streams | does the MJPEG channel run JPE in CMDQ mode (record + replay + Barrier)? — decides which JPE path a replacement must implement |
| K28 | read `1620:22/23/26` during a stream (and after a frame) | which of JPE regs 22/23/26 is CPU mask, IP mask, IP force |

### 1.7 Video encoder (VENC, H.26x)

| # | read / command | expect / what it decides |
|---|---|---|
| K29 | `cat /proc/mhe/ctx_info` while streaming H.264 | encode-time split: firmware time per frame vs host time — decides whether a faster clock can help |

### 1.8 Display, HDMI, GE

| # | read / command | expect / what it decides |
|---|---|---|
| K30 | 1172:0F / 1172:1F during streaming | are the CMDQ handshake flags in use? (1172:0F bit 8 "CMDQ in process", 1172:1F [15:8] wait count — `mhal-disp.md`) |
| K31 | whether `/sys/…/mdisp/pq` shows a loaded PQ bin | expected none |
| K32 | live values of 145A:22–25/32 under the vendor stack | confirms that DISP's table is what drives the HDMI PHY (drive current Tap1/Tap2, Pd fields) |
| K33 | 1459:1F (set to 1 by `InitSeq`, unnamed, new on M6) | its meaning |
| K34 | whether mi_hdmi's debug level 3 really puts the MDrv HDMI prints into the kernel log (`dmesg`) | mi_hdmi sets level 3 (`SetDebugLevel`, `mhal-hdmitx.md`) |
| K35 | 0x1F001720 bit 0 = bank 0x000B reg 0x48 (I2 GE clock gate the HAL still toggles), GE regs 0x24/0x25, reg 0x1e bits 5/6/7 — "a register read under the vendor stack" | their meaning on M6 (bank 0x000B reg 0x48 is not in the M6 CLKGEN map [R]) |
| K36 | the zoomed 4K picture on HDMI | whether DISP (input port fixed at 3840×2160) scales or shows the window smaller |
| K37 | whether the HDMI boot logo is visible; what MOP/GOP state U-Boot leaves | needs a monitor on HDMI during boot |

### 1.9 Decoders (JPD, VDEC)

| # | read / command | expect / what it decides |
|---|---|---|
| K38 | J:76 and `101e:42` values while the photo viewer decodes | `HAL_NJPD_PowerOn` (which would set `101e:42[7]` = 0, J:76 P0/P1 on, P2 off) is never executed in this build [R]; [I] J:76 / `101e:42` stay at their reset values — the first registers to compare if a replacement's decoder writes nothing (`mhal-jpd.md`) |
| K39 | 4K MP4 playback in the 1080p VDEC channel | prog_uvc never sends `SetDevConfig 0x80020005`, so port 0 would be scaled to 1920×1080 |

### 1.10 LDC

Both LDC checks run prog_uvc with `-L` — Part 2 (P14, P15).

### 1.11 Audio

| # | read / command | expect / what it decides |
|---|---|---|
| K40 | a live read of `0x103CC4` / `0x103CF2` (= `riu_r 103c 62` / `riu_r 103c 79` [I]) or of the amp pad's GPIO state after an `MHAL_AUDIO_Init` | whether the padmux driver probed at boot — decides padmux modes 0 and amp pad 21 (`audio.md`); the boot log has no padmux line, DT node and code make it near certain [I] |

### 1.12 USB device port, UVC gadget

| # | read / command | expect / what it decides |
|---|---|---|
| K41 | `lsusb -v` on the host (laptop) with the camera attached | g_webcam topology: XU source IDs (both 2), bNumFormats 5 vs the 3 formats |
| K42 | `UTMI 0x16` / `0x17` (phys 0x1F284258 / 0x1F28425C) now, as the "before" value of P7 | baseline for the "full speed after reconnect" comparison |

### 1.13 SD card

| # | read / command | expect / what it decides |
|---|---|---|
| K43 | `readlink /sys/block/mmcblk0/device` (contains `mmc0:` = slot 0 host or `mmc1:` = slot 1), or `cat` the two `get_sdmmc*_status` files with a card inserted, or the CDZ GPIO levels of pads 25 and 148 | which SD slot is the socket (`kernel-drivers.md`) |

### 1.14 Device nodes

| # | read / command | expect / what it decides |
|---|---|---|
| K44 | `ls -l /dev` and `cat /proc/misc` | expect no `sensor_earlyinit` and no `vcodec`; `mge`, `jpd`, `mdisp`, `mhdmitx`, `mpnl`, `mmipitx` nodes present but opening them fails (`mhal-core.md`); `/proc/divp/` present (§4.7) |

### 1.15 Front panel

| # | read / command | expect / what it decides |
|---|---|---|
| K45 | "SAR ladder values per button" (= `/dev/sar` channel 0 while each button is held [I: per `rootfs-customer.md`]) | real readings against the six windows of `rootfs-customer.md` (identical in kjd_lvgl and `tmp`) |
| K46 | the compare / split view picture (menu 0x19 value 3; values 1/4 resume — a menu action, reversible) | the path is specified (`prog_uvc.md`: two SCL output ports, DISP port 0 paused, split port half HDMI width × full height); only the picture is unverified |

## Part 2 — checks that change state

Each line says what it changes. Read the matching Part 1 values first where a "before" value is needed.

| # | action and read (as in the source) | changes | expect / what it decides |
|---|---|---|---|
| P1 | one queued masked / unmasked CMDQ write test: mask field 0 vs 0xFFFF | writes a register through the CMDQ | whether the M6 engine treats 0xFFFF as "mask off, plain write" [I] |
| P2 | scaler sweep with a test pattern | SCL filter tables / pattern generator | HVSP filter entry layout (5 bytes → taps); not needed to reproduce — the bytes are copied |
| P3 | `echo 480000000 > /sys/class/mstar/venc/ven_clock` written before the first H.26x stream, then the same stream | codec core clock | whether the faster clock helps (compare with K29); the write works only while no H.26x device exists (`mhal-vcodec.md`) |
| P4 | `echo dbg:3 > /sys/class/mstar/venc/log_ctrl` before an H.26x stream starts, then `dmesg` | host log level of mi_venc / mhal | the firmware's own product report; `VPU_RET_VPU_CONFIG1` bit 26 (dual core) — **hw** whether it is ever set |
| P5 | `forcecfg hpddetect 0` (sysfs of the HDMI-TX device) | HPD override `g_bHpdDetect` = 0 ("always connected") | whether the output comes up without a sink |
| P6 | boot with `ehci-hcd.ko` not loaded; observe the USB device port | boot sequence (module list) | ehci-1 programs the UVC port's PHY and switches it to host before the gadget loads (`kernel-drivers.md`) — does the device port behave differently (speed after reconnect)? |
| P7 | compare `UTMI 0x16` / `0x17` (phys 0x1F284258 / 0x1F28425C) before (K42) and after a re-enumeration | replug / re-enumeration | whether the missing `init_utmi` in `udc_start` and the missing UTMI 0x16 rewrite on bus reset (`kernel-drivers.md`) contribute to "full speed after reconnect"; run together with P6 |
| P8 | measure the core / cpu rails; writing 900 to `voltage_current` does not lower them (the TEMPERATURE request of 1000 wins) | voltage request (USER demander), physical measurement with a meter | whether PM_GPIO4/5 actually reach the regulators |
| P9 | `echo 0/1 > …/gpio46/value`, look at the LED | GPIO46 level | confirms 1 = orange, 0 = red, as the code reading gives it (`kernel-platform.md` §6.3) [R] |
| P10 | register dump of the camera before mhal loads | boot stopped before the modules load | reset defaults of every ISP register written at probe/init (which of `isp-1-hal.md` writes are no-ops) |
| P11 | read 1170:00 bit 0 and 1170:04/05 before demo.sh's `riu_w` | boot stopped before demo.sh | whether U-Boot left 0xF8/0xD0 |
| P12 | boot with the rcS `riu_w` table's intermediate writes left out | rcS / boot script | whether the intermediate writes matter (latching behaviour unknown [R]) |
| P13 | at the U-Boot prompt: `btime` (SRAM 0xA000F000…) | boot interrupted at U-Boot (serial) | real time stamps of every IPL / IPL_CUST phase |
| P14 | run prog_uvc with `-L` and a JSON; read `/proc/mi_modules/mi_ldc/mi_ldc0` (table info: NumRuns, block sizes) and mhal's sysfs `check_*` attributes (`check_clk`, `check_inst_table_info`, …) | prog_uvc options | the real LDC tile layout and clock |
| P15 | dump a stock table with proc `dumptablebin` (`echo dumptablebin <chn> <viewid\|all> <path> > /proc/mi_modules/mi_ldc/mi_ldc0`) and compare with `ldc-2-eptz-lib.md`; then feed a DBIN container (`ldc-2-eptz-lib.md`) through `prog_uvc -L` | writes files; prog_uvc options | verifies the whole LDC table format on hardware |
| P16 | run the HW IVE ops on test images and compare with the CPU versions of `ive-video-analytics.md` | runs a test program on the engine | engine-side arithmetic: Q8 CSC division, Filter shift of negative sums, Image_Dot / Matrix rounding, MagAndAng angle bins, SAD 8-bit scaling |
| P17 | disconnect the menu client from a running prog_uvc | provokes the crash | prog_uvc death on menu-client disconnect — the menu thread's `free()` of a non-heap address; the crash is [I] and the exact glibc behaviour is inference |
| P18 | IPU engine probes, each one op with known inputs read back from L2 / DDR: GREATER on in0 = [1, 2], in1 = [2, 1]; one 0x20 ADD and one 0xA8 / 0xB0 conversion with chosen scale / shift on a ramp; a LUT op with a ramp table; equal values in several lanes for the 0x02 lane max | runs test models on the IPU | the hardware definitions the firmware does not carry (`ipu-firmware-2-engines.md`): predicate polarity of w1 bit 24 (also 0x12 / 0x16 / 0x1A, CAST-to-BOOL, LOGICAL_AND); order and rounding of requantising operations 7 / 3 / 4 (also ROUND, RELU / Clip clamps); LUT interpolation; the 0x02 tie order (TOPK, ARG_MAX, NMS, PostProcess_Max); SR opcode 0x65 rounding; 0xAC resize and 0xDB / 0x5B pooling rounding; the operations of the private opcodes |

## Not camera checks (listed so they are not looked for here)

- Physical IR remote layout — needs the remote itself; the keymap is narrowed to 18 keys (`rootfs-customer.md`).
- `H161X.bin` — a file we do not have.
