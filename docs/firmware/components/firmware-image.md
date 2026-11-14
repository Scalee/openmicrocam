# firmware-image — the NOR flash image as shipped
<!-- ev:firmware-image -->

The camera's 16 MiB SPI-NOR flash as it left the factory: partition map, boot chain, and every vendor file by layer, with
what uses it. Each component's internals live in its own document; this one owns the partition map (MXPT) and the vendor
file inventory, and the component documents refer to its rows.

Tags: **[R]** read from the dump, a binary, its disassembly, a log or a file; **[S]** stated by a source; **[I]** inference.
A section heading tagged [R] means every statement under it is [R] unless marked otherwise.

The vendor file inventory is the appendix: one row per file, flash region or kernel driver group, with its use. Counts:
used 139, unused 130, static-copy 17 [R].

## 1. Partition map [R]

### 1.1 MXPT and the MTD partitions

The partition table `MXPT` sits at flash offset 0x20000 (a 64 KiB region holding a 4 KiB table) with entries of 0x88
bytes, in this order: IPL, IPL_CUST, MXPT, UBOOT, UBOOT_ENV, BOOT, KERNEL, rootfs, miservice, customer, MISC. IPL, U-Boot
and the kernel all read it; the kernel's MXP partition parser (`MXP_PARTS`) builds its 6 MTD partitions from the same table.
The extent of the `BOOT` entry is not given in the sources; `fw_env.config` addresses the environment as
`/dev/mtd0 0x6F000 0x1000 0x10000`, so mtd0 starts at flash offset 0 and covers the boot regions [I].

### 1.2 Regions

| region | offset | size | format | what / mounted |
|---|---|---|---|---|
| IPL | 0x000000 | 64 KiB (0x7400 = 29 KiB used) | SigmaStar boot blob | first loader, build `a0090c8` (ipl.md §1.1) |
| IPL_CUST | 0x010000 | 64 KiB (0x5c90 = 23 KiB used) | boot blob | second loader; unpacks U-Boot (XZ) to 0x23E00000 (ipl.md §5.2) |
| MXPT | 0x020000 | 64 KiB (4 KiB table) | table | partition table (§1.1) |
| UBOOT | 0x030000 | 252 KiB (247 116 B image) | uImage, header says lzma, payload xz | U-Boot 2015.01 "M6", built 2025-06-03, 719 KB unpacked (uboot.md §1.1) |
| UBOOT_ENV | 0x06F000 | 4 KiB | U-Boot env | `fw_env.config`: `/dev/mtd0 0x6F000 0x1000 0x10000`; `env_offset=0x6F000` |
| KERNEL (mtd1) | 0x070000 | 2112 KiB (2 150 812 B used) | uImage, header says lzma, payload xz | Linux 4.9.227, 4.5 MB unpacked, DTB built in (§3.2) |
| rootfs (mtd2) | 0x280000 | 2 MiB (1 944 828 B image) | squashfs xz, 421 inodes, built 2026-05-22 | `/` (ro) |
| miservice (mtd3) | 0x480000 | 4 MiB (4 129 130 B image, 98.5 %) | squashfs xz, 170 inodes, built 2026-05-22 | `/config` — SigmaStar SDK release |
| customer (mtd4) | 0x880000 | 6208 KiB (0x880000–0xE90000) | jffs2 rw | `/customer` — ODM (KJD/OSEECAM) apps; no fully erased block left in the dump |
| MISC (mtd5) | 0xE90000 | 640 KiB (`sf_part_size=a0000`) | "firmwarefs" (§1.4) | `/misc` via the FUSE daemon `fwfs`; also read by U-Boot |
| — | 0xF30000 | 832 KiB (0xF30000–0xFFFFFF) | erased (all 0xFF) | not in MXPT; outside every partition |

The U-Boot environment in the dump also carries `sf_part_start=e90000` (= the MISC offset), `cpu_part_start=14e90000`
(its meaning is not known; 0x14000000 + the MISC offset [I]), `SdUpgradeImage=H161X.bin` and `ota_upgrade_status=0`; the
variables themselves are uboot.md §3.2's.

### 1.3 uImage headers

| image | load | entry | name | date | compression field |
|---|---|---|---|---|---|
| U-Boot | 0x23F00000 | 0x23E00000 | `MVX4##M6##g#######CM_UBT1501#XVM` | 2025-06-03 | 3 = "lzma" |
| kernel | 0x20008000 | 0x20008000 | `MVX4##M6##gc8e0e527dKL_LX409##[B` | 2026-03-05 | "lzma" |

Both payloads are xz [R]. IPL_CUST treats `ih_comp` 3 as XZ (9 as "MZ", 0 as uncompressed); how it loads and checks the
U-Boot image is ipl.md §5.2's (IPL → U-Boot hand-over). The kernel image is 2 150 748 B of data behind the 64-byte header.
IPL_CUST + 0x523F (NOR 0x1523F) holds the magic constants of IPL_CUST's embedded XZ decoder (`FD 37 7A 58 5A 00` header
magic, `59 5A` footer magic), not an XZ stream.

### 1.4 MISC contents

Format: littlefs v2.2.1 with SigmaStar "TFTL" changes (tagged block numbers `0xA…` / `0xB…`, 10-byte name tag, 12-byte CTZ structs, file
data in 32 KiB subblocks with an 8-byte header) — the format, its readers (`/bin/fwfs`, U-Boot) and their settings are
misc-firmwarefs.md's. Stock `littlefs-python` mounts the image after the magic is renamed but cannot read the directory
(LFS_ERR_CORRUPT) because of these changes. 4 of the 10 blocks are erased.

| file | bytes | content / readers |
|---|---|---|
| `config.ini` | 76005 | identical to `/config/config.ini` (all 76005 bytes, in three in-order 32 KiB subblocks); U-Boot `bootlogo` reads `[LOGO]`, mi_sys → `config_tool load_config` reads the panel / HDMI timing tables (config_tool.md §4.1) |
| `PQ.bin` (`/misc/PQ.bin`) | 25892 | named only in `[LOGO1]` (LCD dev 1) and `[PQ_PATH]`; U-Boot logs `can't get LOGO:m_sPQFile0`; unused; format: uboot.md §6.2 |
| `sigmastar1024_600.jpg` | 49873 | boot splash (`m_sLogoFile0`); same bytes as `/customer/misc/sigmastar1024_600.jpg`; read by U-Boot `bootlogo` and kjd_lvgl |
| `upgrade.jpg` | 17833 | shown by U-Boot during the SD upgrade (`m_sLogoFile1`); same bytes as `/customer/misc/upgrade.jpg` |
| `ws.txt` | 0 (inline) | prog_uvc crop modes (prog_uvc.md §5.2) |

### 1.5 Fill

`/config` (miservice) is 98.5 % full; `/customer` has no fully erased block in the dump; the 832 KiB at the end of the flash
are outside every partition. The three full copies of libmp4v2 in `/customer/lib` (§3.7) take ≈ 2.5 MB uncompressed.

## 2. Boot chain — who loads what [R]

1. **Boot ROM → IPL → IPL_CUST → U-Boot.** IPL runs the DRAM init (256 MB): on this board it is table-driven (bonding 0x11)
   plus a BIST, with no DRAM training — software DQS training exists only for bonding 0x27 (the boot log's
   `No support sw autoK flow`). IPL finds MXPT and loads IPL_CUST; IPL_CUST XZ-unpacks U-Boot to 0x23E00000 and jumps there
   (ipl.md §4.4, §5.2). U-Boot `bootcmd`: `bootlogo 0 0 0 0` (reads `config.ini` `[LOGO]` from MISC: HDMI `DACOUT_1080P_60`,
   `sigmastar1024_600.jpg`), then `sf read` + `bootm`. Whether the HDMI boot logo actually shows on screen is not known
   (the code path is uboot.md §5's). U-Boot also carries the `sdstar` SD-card upgrade (`SdUpgradeImage=H161X.bin`,
   `ota_upgrade_status`) and the `dstar` / `estar` / `usbstar` scripts. `H161X.bin`, the SD upgrade file, is not at hand;
   its content is not known.
2. **Kernel** command line: `root=/dev/mtdblock2 init=/linuxrc`, `mma_heap … sz=0x9000000`, `cma=2M`,
   `mmap_reserved=fb … 8 MB` (memory map: kernel-platform.md §11.1).
3. **`/etc/inittab` → `/etc/init.d/rcS`:** mount /proc /sys /dev, `mount -t squashfs /dev/mtdblock3 /config`,
   `mount -t jffs2 mtd:customer /customer`, `fwfs … /dev/mtd5 /misc`, `telnetd &`, `/customer/demo.sh`, then **22 `riu_w`
   register writes** to banks 0x1012 / 0x1013 / 0x1011 (MIU / DDR controller) and 0x1170 (not identified in the inventory) [I: banks
   0x1012 / 0x1011 are the MIU registers written by `mach-sstar/mercury6/sram.S` and `miu_arb.c` in gitee marcio/sigmastar].
   The write list, and the open question whether its intermediate writes matter, are rootfs-customer.md §5.2, §5.4's; the meaning of
   each register is mhal-disp.md §11.4's (MOP) and kernel-platform.md §12.3's (MIU).
4. **`/customer/demo.sh`:** insmod of the USB host stack, CIFS/NFS, MMC, FAT/NTFS, SATA; `mhal.ko`; 19 `mi_*.ko` (mi_sys
   `cmdQBufSize=3072 logBufSize=256`, mi_vdisp `vdisp_bufq_depth=4`, mi_ldc without parameters); `mknod /dev/mi_poll`;
   `fbdev.ko default_config_path_file=/config/fbdev.ini`; `gc8613_MIPI.ko chmap=1 lane_num=4`; the USB gadget stack;
   `usb0 192.168.1.100/16`; `fw_setenv SdUpgradeImage H161X.bin`; `prog_uvc -H -A 0 -a 0 -q -i /customer/iq_file_gc8613.bin &`;
   `sleep 3`; `kjd_lvgl &`. Of its 62 `insmod` lines about half load something never used (§3.3–§3.4) [R count, I share],
   and seven fail on every boot: `scsi_mod.ko`, `sd_mod.ko`, `ms_notify.ko` do not exist (built into the kernel);
   `media.ko`, `videodev.ko`, `v4l2-common.ko` are rejected (duplicate symbols, built in); `usb-common.ko` is loaded twice
   ("File exists").
5. **Inside the kernel:** `mi_sys` runs `/config/config_tool` as a usermode helper with `load_mmap /config/mmap.ini` and
   `load_config /misc/config.ini /misc/PQConfig.ini` (PQConfig.ini does not exist) (config_tool.md §1.3); `mhal` loads
   `/config/venc_fw/chagall.bin` for the encoder; the AE/AWB/AF init loads `/config/iqfile/iqfile0.bin` (boot log:
   `Load iq file /config/iqfile/iqfile0.bin`; mhal format `%s/iqfile%d.bin`), a symlink to **`imx307_iqfile.bin`** — the
   file mhal holds as its built-in `iq_bin`. The user-space 3A inside prog_uvc (libcus3a) loads the same file from its
   fallback directory `/config/iqfile`, because `MI_ISP_GetIspRoot` (id 0x66) always fails on this firmware [I]. So AE/AWB
   start from the IMX307 calibration on a GC8613 camera; prog_uvc then applies the GC8613-specific API bin `/customer/iq_file_gc8613.bin` on top (isp-5-3a.md §4.9, isp-6-userspace.md §2.8).
6. **Apps:** prog_uvc insmods `g_webcam.ko … uvc_function_enable=1 rndis_function_enable=1` itself; kjd_lvgl swaps it for
   `g_mass_storage.ko` in USB-disk mode (`usb_otg.sh`); mdev runs `/customer/sd_hotplug.sh` on `mmcblk*`, which mounts the
   card, signals kjd_lvgl and runs `sd_version_recognition` (rootfs-customer.md §4.1).

Nothing on the camera loads a `/config/lib/*.so` at run time: prog_uvc carries the MI library code statically, kjd_lvgl
needs only libc / libm / libpthread.

## 3. Inventory by layer

Per-file sizes, types and notes: the appendix. The tables below group the rows; "use" has the values
defined in the header.

### 3.1 Boot regions [R]

| item | use | used by | notes |
|---|---|---|---|
| IPL, IPL_CUST | used | boot ROM, IPL | DRAM init and SPI loader in the format the boot ROM expects; no public source known [I]; OpenIPC ships the vendor IPL too [I] (ipl.md) |
| MXPT | used | IPL, U-Boot, kernel | §1.1 |
| U-Boot | used | IPL_CUST | stock build drives HDMI-TX for the logo (`MHal_HDMITx_*` strings); OpenIPC `u-boot-sigmastar` (2015.01, `mercury6_*_defconfig` with `FS_FIRMWAREFS`, `CMD_BOOTLOGO`, `SSTAR_DISP`) is the public tree of the same U-Boot, with that HDMI-TX option off in its defconfig (uboot.md §1.3) |
| U-Boot env | used | U-Boot, `fw_printenv` / `fw_setenv` | vendor values only (uboot.md §3.2) |

### 3.2 Kernel image (KERNEL partition) [R]
<!-- ev:firmware-image.3.2 -->

Core: Linux 4.9.227 `#44 SMP PREEMPT Thu Mar 5 2026`, gcc 9.1.0, built by a developer account on the ODM's build host `kjd-AIOT-Server`, release string
`MVX4##M6##gc8e0e527dKL_LX409##[BR:WS_H1605_NOR_8836_IMX662]` — an IMX662 product branch of the ODM. No `/proc/config.gz`;
kallsyms present (41 651 symbols extracted). DTB built in: 49 471 B at Image + 0x421fa0, model `MERCURY6 SSC019A-S01A-S`,
decompiles with `dtc` (kernel-platform.md §14). Vendor code in the kernel ≈ 124 KB in ≈ 1350 functions of ≈ 2.6 MB text
[R, name-pattern estimate from kallsyms]. Kernel-side details: kernel-platform.md (SoC core, DTB, msys, MIU, CamOs) and
kernel-drivers.md (peripheral drivers).

| built-in vendor driver | code ≈ bytes / funcs | interface → used by | use |
|---|---|---|---|
| DTB | 49 KB data | probe of every platform driver | used |
| mach-sstar/mercury6 (SMP, PM, MIU arb) | ~1 KB / 8 + PM | boot; source in gitee marcio/sigmastar | used |
| intc (`ms_main_intc` / `ms_pm_intc` / `ms_gpi_intc`) + timer (`ms_timer`) | 2.3 KB / 24 | IRQs | used |
| clocks (`ms_clk_composite` / complex), cpufreq (`ms_cpufreq`, 800 MHz), sstar voltage-ctrl | 3.3 KB / 34 | everything | used |
| UART `ms_uart` | 5 KB / 33 | console ttyS0; 5 UARTs in the DT | used |
| GPIO + padmux + camdriver_gpio (`MDrv_GPIO` / `MHal_GPIO`, `sstar-padmux`) | 3.2 KB / 79 | kjd_lvgl gpio46, mhal | used |
| I2C (HWI2C + gpio-i2c) | 8.3 KB / 113 | sensor I2C via mhal, `/dev/i2c-*`; 6 buses in the DT | used |
| SPI-NOR (FSP / serflash / BDMA, `mtd-flashisp`), MXP parser | 14.5 KB / 136 | all MTD partitions | used |
| MSYS / MMA / DMEM | 13.7 KB / 101 | `/dev/msys` (`msys_ioctl`), `/sys/devices/virtual/mstar/msys/*` (`CHIP_ID`, `CHIP_VERSION`), MMA heap from bootargs → mhal, MI; the only user-space opener of `/dev/msys` is `riu_w` | used |
| MIU / MMU / movdma | 6.2 KB / 68 | mhal, MI; `/sys/devices/system/miu/miu0/dram_info` | used |
| SAR ADC | 0.9 KB / 13 | `/dev/sar` ioctl 0x6100 (init) / 0x6101 (read) → kjd_lvgl buttons | used |
| IR receiver (`mdrv_ir`) | 1 KB / 7 | `/dev/input/event0` → kjd_lvgl remote | used |
| RTC (`sstar_rtcpwc`) | 0.7–7.8 KB (each of RTC, PWM, watchdog, notify) | kernel hctosys at boot, kjd_lvgl `hwclock -w` after a Date & Time edit, `pm_power_off` (busybox `poweroff`); the clock starts at 1970 | used |
| PWM (`ms_pwm`), watchdog (`infinity_wdt`, `/dev/watchdog`), notify / chip (`mstar_chip`) | 0.7–7.8 KB each | no consumer | unused |
| USB PHY (`sstar-usb-phy`) | 1.9 KB / 14 | udc-msb250x | used |
| CamOs (kernel OS shim) | 8.8 KB / ~150 | mhal, sensor driver | used |
| EMAC, SATA/AHCI glue, IVE (`mdrv_ive`), AES/SHA/RSA (infinity aes/sha), goodix gt911 | — | EMAC link down in every boot log (network is RNDIS usb0) / no SATA on the board / no app uses MI_IVE / prog_uvc's XU challenge uses software AES / gt911: the DT node is not disabled, so it is probed and gives three I2C timeouts on every boot (no touch panel on this camera) | unused |

prog_uvc contains the strings `/dev/msys`, `dram_info` and `CHIP_ID` only as part of its statically linked CamOs library;
no code references them.

The branch tag read against the board maker's catalogue: `WS` matches the board silkscreen "oseecam WS03"
(`kernel-platform.md` §6.4), `NOR_8836` the SoC and flash, `H1605` OSEECAM's camera family — on oseecam.com H1605 / H1605-B
are 2 MP Sony 1/2.8" 2.9 µm boards with HDMI 1080P60, and H1605-C ("HDMI+Type-C") is an 8.29 MP Sony 1/2.8" 1.45 µm board
with HDMI 4K30 / 1080P60 and a Type-C USB list identical to this camera's (3840×2160, 2560×1440, 1920×1080, 1280×720, 640×480,
424×240, 320×240) [S: oseecam.com Product/show/15, 63, 65]; `IMX662` names the 2 MP sensor of that
branch while this camera loads the GC8613 driver and IQ file [R], i.e. one H1605 build serves several sensor boards [I].
`kjd` of the build host is not identified [R: no public hits]. Another Mercury6 microscope, the Yizhan XHU4006, runs the
same MI build (`project_commit.7251436 sdk_commit.80327e1 build_time.20220721`) and an app whose log names functions that
prog_uvc also contains (`ST_VideoModuleInit`, `St_HdmiModuleInit`, `change_scl_resolution`; plus `do_weishen`, which prog_uvc
lacks) [R: its boot logs, R `strings prog_uvc`] — the same ODM app family [I]. That unit has an IMX678 on I2C
0x34, SPI-NAND, an `RJGT102` security chip checked before the app starts, a sensor driver listing 1920×1080@60 and
3840×2160@30 / @45, app build dates 2024-12-21 / 24 [R its boot logs].

### 3.3 SigmaStar media stack — `/config/modules/4.9.227` [R]
<!-- ev:firmware-image.3.3 -->

MI build 2022-07-21, `project_commit.7251436`, `sdk_commit.80327e1`; mhal build 2022-07-21 "alkaid". Which SDK release the
build belongs to is not written in the image. [S] SigmaStar FAEs on the Comake developer chat: SSC8836 /
8838 / 8826 have their own SDK line `SSC88XX_TIRAMISU_CLS00V00x` (the car-recorder line: V001 2021-06-09, V003 2021-12, V005
2022-06-06, V004 named for 2022-08), and the `SSD26X_DLS00V012` package matches only SSD261 / SSD268G. [I] A 2022-07-21 MI build
fits CLS00V005 or a later drop of that line; the sensor module alone names its tree (`Tiramisu_DLS00V012`, sensor-gc8613.md).
All modules have full ELF symbol tables (function names, statics included) and no debug info. Code = `.text` bytes. Each MI module's device is `/dev/mi_<mod>`; "app ioctls" = the number of distinct ioctls prog_uvc
issues on that device (magic `'i'` 0x69; the ABI is mi-user-abi.md's).

| module | code / funcs | exports | interface → consumer | use |
|---|---|---|---|---|
| mhal.ko | 993 KB / 7746 | 988 | HAL API (`MHAL_DISP` ~224, `MHAL_AUDIO`, `MHAL_CMDQ`, `MHal_HDMITx`, `MHAL_VDEC`, `MHAL_RGN`, `MHAL_JPD`, `MHal_VIF`, `MHal_SNR`, `MHAL_MHE/MFE/VENC/JPE`, `MHAL_LDC`, `MHAL_DIVP`…) for VIF/CSI/ISP/SCL/DISP/HDMI-TX/GOP/VENC/JPE/JPD/LDC/CMDQ/RGN/MIPI-TX/PNL/audio → mi_*.ko, fbdev, sensor driver; char devices `isp`, `isp_fe` (opened by prog_uvc), `ispmid`, `ispscl`, `csi`, `sensorif`, `venc`, `jpe`; loads `chagall.bin` (mhal-core.md) | used |
| mi_common.ko | 2.3 KB / 23 | 8 | helpers → all mi_* | used |
| mi_sys.ko | 294 KB / 740 | 154 | `/dev/mi_sys` (22 app ioctls), `/dev/mi_poll`, `/proc/mi_modules/*`, usermode helper `config_tool`; bind / buffer / MMA core → prog_uvc, all mi_* | used |
| mi_sensor.ko | 12.6 KB / 51 | — | `/dev/mi_sensor` (8); drives the sensor driver through mhal SNR | used |
| mi_vif.ko | 46 KB / 97 | 16 | `/dev/mi_vif` (12) | used |
| mi_isp.ko | 58 KB / 139 | 25 | `/dev/mi_isp` (14 + IQ API) → prog_uvc, libispalgo | used |
| mi_scl.ko | 61 KB / 121 | 20 | `/dev/mi_scl` (11); imports two functions of mi_rgn | used |
| mi_rgn.ko | 50 KB / 96 | 9 | OSD regions; loaded only as a symbol dependency of mi_scl, the app uses no RGN | used |
| mi_gfx.ko | 11.9 KB / 66 | 14 | 2D blitter (GE); dependency of mi_disp, which calls it only for rotation (not used) | used |
| mi_venc.ko | 184 KB / 308 | — | `/dev/mi_venc` (20); H.264/H.265 on the VPU (`chagall.bin`) + MJPEG on JPE | used |
| mi_disp.ko | 80 KB / 184 | — | `/dev/mi_disp` (16) → prog_uvc, fbdev | used |
| mi_hdmi.ko | 15 KB / 48 | — | `/dev/mi_hdmi` (10); EDID / hot-plug, timing | used |
| mi_jpd.ko | 39 KB / 81 | — | `/dev/mi_jpd` (9: 2, 3, 5, 6, 9, 0xa, 0xb, 0xe, 0xf — `mi_jpd-mi_vdec.md` §1.5) → photo viewer only (JPEGs from SD) | used |
| mi_vdec.ko | 95 KB / 119 | — | `/dev/mi_vdec` (7) → MP4 player only (playback from SD) | used |
| mi_ldc.ko | 276 KB / 587 | 22 | loaded at every boot by `demo.sh` (no parameters); prog_uvc uses LDC only with `-L`, which the boot line does not pass. ≈ 220 KB of the 276 KB are the public Simd library, 7 KB cJSON 1.7.12; the 22 exports are all `MI_LDC_IMPL_*` and nothing imports them (ldc-1-stack.md, ldc-2-eptz-lib.md) | unused |
| mi_ai.ko, mi_ao.ko | 53 / 46 KB | 50 / 38 | audio only with prog_uvc `-d` / `-D`, or `-f <path>` (an audio-file test mode that skips video init); none is passed (audio.md) | unused |
| mi_vdisp.ko, mi_shadow.ko | `.text` 0x6B60 / 0x4C44 (files 77 / 62 KB) | — | loaded; prog_uvc never opens `/dev/mi_vdisp` (no opener copy, no library code); mi_shadow has no user, holders 0; no module depends on either (mi_vdisp-mi_shadow.md) | unused |
| mi_ipu, mi_mipitx, mi_panel | 10–27 KB each | — | loaded, but no app opens their devices and no module depends on them (the IPU firmware `ipu_firmware.bin`: §3.6); no MIPI-TX or LCD panel (HDMI only) | unused |
| fbdev.ko | 12.8 KB / 85 | — | `/dev/fb0` (1600×900 ARGB on GOP over the 1920×1080 timing, 8 MB, `/config/fbdev.ini`) → kjd_lvgl | used |
| gc8613_MIPI.ko | 2.1 KB / 26 | — | vendor GC8613 sensor driver, `chmap=1 lane_num=4` (sensor-gc8613.md §2) | used |
| imx307_MIPI.ko | 6.1 KB / 82 | — | never loaded; a second instance of the sensor-driver ABI; IMX307 is not on this camera | unused |

### 3.4 Other kernel modules [R]

| module | use | notes |
|---|---|---|
| g_webcam.ko | used | vendor-patched GPL `webcam.c`: params `uvc_function_enable`, `rndis_function_enable`, `bulk_streaming_ep`; UVC descriptor tables + RNDIS (code 192 B, the rest is data); insmodded by prog_uvc (usb-gadget.md) |
| usb_f_uvc.ko | used | vendor-patched GPL f_uvc (frame-based H.264/H.265, bulk endpoint) (usb-gadget.md) |
| udc-msb250x.ko | used | SigmaStar USB 2.0 device controller, GPL, version "5 June 2019", 9 KB / 70 funcs, names (usb-gadget.md) |
| kdrv_sdmmc.ko | used | SigmaStar SD host, GPL, 14.4 KB / 97 funcs, names; the SD card works through it (kernel-drivers.md §5.1) |
| udc-core, libcomposite, u_ether, usb_f_rndis (RNDIS `usb0` 192.168.1.100), usb-common, videobuf2-core / -memops / -v4l2 / -vmalloc | used | stock 4.9 gadget / V4L2 pieces |
| g_mass_storage, usb_f_mass_storage | used | kjd_lvgl USB-disk mode |
| mmc_core, mmc_block, fat, vfat | used | SD card (`sd_hotplug.sh` mounts `-t vfat`) |
| media, videodev, v4l2-common | unused | rejected at insmod — built into the kernel |
| usbcore, ehci-hcd (SigmaStar EHCI glue), usb-storage, usbhid | unused | host stack; no USB host port in use [I] |
| cifs, nfs, nfsv2, nfsv3, lockd, sunrpc, grace, ntfs, msdos, nls_utf8, mdrv-sata-host | unused | loaded, nothing mounts them (vfat is mounted without iocharset); no SATA on the board |

### 3.5 `/config/lib` (SDK user libraries) [R]

All stripped of `.symtab` but with dynamic exports; built with GCC 9.1.0, armhf. No `.so` here is loaded by anything on the
camera (§2); "static-copy" marks the libraries whose code prog_uvc carries statically linked. The user-side ABI of the
`libmi_*` wrappers is mi-user-abi.md's.

| library | code / exports | use | notes |
|---|---|---|---|
| libmi_sys 6.6 KB/47, libmi_common 2.3/40, libmi_sensor 3/19, libmi_vif 2.5/16, libmi_scl 2.5/15, libmi_venc 34/78, libmi_disp 6.7/58, libmi_hdmi 17/16, libmi_jpd 6/14, libmi_vdec 5.6/33, libmi_isp 36/402 | see col. 1 | static-copy | thin ioctl wrappers (libmi_isp: struct marshalling of the IQ API); no inter-.so NEEDED, `g_mi_fd_table` (libmi_common) shared |
| libcam_os_wrapper 13.6 KB/160, libcam_fs_wrapper | | static-copy | user-space CamOs shim (pthread / mmap wrappers) |
| libispalgo 108 KB/345, libcus3a 15 KB/46 | | static-copy | SigmaStar AE/AWB/AF algorithms and their glue, with the custom-3A hook API in libcus3a; tied to the IQ data (isp-5-3a.md, isp-6-userspace.md) |
| libmi_iqserver 28 KB/48 + libfbc_decode 105 KB/322 | | static-copy | IQTool server on TCP 9876 (prog_uvc `-q`), serving `isp_api.xml` (§3.6); 48 = exported functions of libmi_iqserver; libfbc_decode has 342 `.dynsym` rows / 322 distinct entries and holds an FBC decoder, the full FBC encoder model and three INI layers; the fbc_decode code (`FBD_Execute`) is reached only from the server's picture paths. Neither `.so` is NEEDED by any binary, no file contains the name `libfbc_decode` (no `dlopen`), and `libmi_iqserver.so` does not need `libfbc_decode.so`; tuning only (isp-6-userspace.md) |
| libmi_ldc, libmi_ai, libmi_ao | | unused | prog_uvc runs without `-L` / `-d` / `-D` / `-f` (§3.3) |
| AEC / AED / APC / BF / SRC / SSL `_LINUX` (audio), MD / OD / VG `_LINUX` (video analytics: motion detection, occlusion detection, virtual gate) | 845 928 B together | unused | stock SigmaStar binaries, byte-identical to the public Tiramisu SDK V012 drop (GitHub `qanthink/skyipc_tiramisu`); the SDK's own libmi_vdf.so references `MI_MD_*` / `MI_OD_*` / `MI_VG_*`; no static copy in prog_uvc (audio-algorithms.md, ive-video-analytics.md) |
| `AEC_LICENSE_*`, `APC_LICENSE_*` (`_1`, `_2`) | | unused | `_1` = Ooura FFT notice, `_2` = WebRTC BSD-3. Third-party code with no licence file on the device: speexdsp (libAEC's linear filter), SILK VAD (`psSilk_VAD`) and Ne10 (both in libBF) [R strings; origin of SILK / Ne10 I] |
| libcjson | | unused | cJSON 1.7.7 unmodified; imported by libmi_wlan (also unused) |
| libg711, libg726 | | unused | libg711 = Sun g711.c (its `alaw2linear` decodes A-law segment 0 to 0); libg726 = Sun G.72x plus 16 kbit/s tables |
| libsigma3a | | unused | ABI-incompatible with this firmware's libispalgo (packed 81-byte `AeInput_t`, M5 `AeOutput_t`, two-argument CUS3A calls), 4 channels, no device argument |
| libmi_divp (`/config/lib/libmi_divp.so`), libmi_gyro, libmi_vpe | | unused | the 2020 wrapper generation: private fd / pid pair, no lock, no shared table; no `mi_divp.ko` provides `/dev/mi_divp`; libmi_vpe's `MI_ISP_EnableUserspace3A(chn, &pair)` call does not match the camera libmi_isp's `(dev, chn)` signature (sdk-orphan-libs.md) |
| libmi_ive | | unused | imports five `MI_SYS_*` functions without a NEEDED entry for libmi_sys (ive-video-analytics.md §1.1) |
| libmi_sed, libmi_vdf, libmi_shadow | | unused | libmi_sed needs libmi_ive and libmi_vdf; libmi_vdf needs libmi_shadow (14 `MI_SHADOW_*` imports) and the MD / OD / VG libraries; none of it through NEEDED; libmi_vdf has no user |
| libmi_gfx, libmi_ipu, libmi_mipitx, libmi_panel, libmi_rgn, libmi_vdisp, libmi_wlan | | unused | no static copy in prog_uvc, no other user |

### 3.6 `/config` data and tools [R]

| file | use | used by / notes |
|---|---|---|
| `/config/config_tool` (+ `dump_config`, `dump_mmap` symlinks) | used | mi_sys usermode helper: parses `mmap.ini`, `/misc/config.ini` and PQConfig into `/proc/mi_modules/common/*`; 48.5 KB file, 25.6 KB `.text`, stripped (config_tool.md) |
| `/config/mmap.ini` | used | MMA memory map read by mi_sys (through config_tool), fbdev, mi_venc (config_tool.md §2.5) |
| `/config/fbdev.ini` | used | fbdev.ko (`default_config_path_file`) |
| `/config/venc_fw/chagall.bin` | used | the only file in `venc_fw`; encoder-core firmware, 385 KB, string `CODEC40.22031116`, loaded via `VPU_InitWithBitcode` (DT reg-name `vpu-bit`); runs on the codec's own processor. It carries two programs: the VCPU firmware (EISC / AE32000-class ISA [I], 16-bit instructions, newlib C) and the VCORE engine microcode (4 × 8 KiB segments, another ISA); no source (chagall-fw.md) |
| `/config/iqfile/imx307_iqfile.bin` via `iqfile0.bin` | used | **loaded at every start for the GC8613 by the 3A init** (§2 step 5); IMX307 calibration from SigmaStar IQTool (isp-4-iq-engine.md §13.4) |
| `/config/iqfile/isp_api.xml` | used | IQTool description, served by prog_uvc's static iqserver (`MI_IQSERVER_DownloadFile` ID 2) from this fixed path; tuning only |
| `/config/iqfile/iqfile1..3.bin` (symlinks to `imx307_iqfile.bin`), `imx307_api.bin` | unused | channels 1–3 unused; the app loads its own API bin |
| `/config/dla/ipu_firmware.bin` | unused | RISC-V firmware of the NPU (`ipu_ccif_riscv_server`), 602 KB; no app uses the IPU (ipu-firmware-1-runtime-model.md) |
| `/config/config.ini` | unused | identical to `/misc/config.ini`; config_tool's first fallback path (then `/config/model/config.ini`); the boot log shows `/misc/config.ini` being loaded |
| `/config/board.ini`, `/config/model/Customer.ini` | unused | LVDS panel board settings; no binary reads either; board.ini is named only in `model/Customer.ini`; board.ini's `#@CRC=0x725D` is the 16-bit byte sum of the preceding 460 bytes |
| `/config/model/LCM.ini` | unused | external LCD panel config, no reader; LCD panels are not on this camera |
| `/config/terminfo/` (29 vt100 / vt102 entries) | unused | only for htop (`TERMINFO` in `/etc/profile`) |

### 3.7 `/customer` (ODM partition) [R]

The boot scripts and small tools are rootfs-customer.md's; the two apps are prog_uvc.md's and kjd_lvgl.md's.

| file | use | used by / notes |
|---|---|---|
| `prog_uvc`, `kjd_lvgl` | used | the camera app (UVC, video pipeline, menu link) and the on-screen UI; started by demo.sh |
| `ws.ini`, `Line.ini`, `ws.bin`, `default_ws.bin`, `camp.txt` (UVC PU defaults), `version.txt` (`version:W11861322604301`), `1600x900.png`, `/misc/ws.txt` | used | app state and assets (prog_uvc.md, kjd_lvgl.md) |
| `iq_file_gc8613.bin` | used | ISP API bin, `prog_uvc -i` (isp-3-iq-api.md §6) |
| `demo.sh`, `set.sh` (`LD_LIBRARY_PATH` for libmp4v2), `/etc/init.d/rcS` | used | boot glue (§2) |
| `sd_hotplug.sh`, `usb_otg.sh` | used | SD mount (vfat on /mnt) + SIGUSR1/2 to the UI; USB-disk switch |
| `sd_version_recognition` | used | SD upgrade trigger: `H161X.bin` on the card and a `sd_version.txt` whose version differs from the camera's (any difference, `strcmp`) → `fw_setenv ota_upgrade_status 2; reboot`; 936 B, with debug info (file 12 632 B) |
| `riu_w` | used | rcS (22 register writes); the stock SigmaStar sample `drivers/sstar/samples/riu.c` (i6e variant), gets the RIU window from `/dev/msys` (`IOCTL_MSYS_GET_RIU_MAP`) and pokes it through `/dev/mem` (rootfs-customer.md §6) |
| `/customer/riu_r`, `riux32_r`, `riux32_w` | unused | the same file as `riu_w` (sha256 `15016388…`), dispatched by name; debug tools nothing starts |
| `i2c_read_write` | unused | debug tool built with `gcc-arm-8.2-2018.08` (a third-party toolchain, not SigmaStar's); nothing starts it |
| `htop` (1.0.2) | unused | not started |
| `lib/libmp4v2.so{,.2,.2.0.0}` | used | MP4v2 2.0.0 for recording / playback (`prog_uvc` NEEDED `libmp4v2.so.2`); three full 1.2 MB copies instead of symlinks |
| `iperf3` (3.1.3) | unused | linked against uClibc (`/lib/ld-uClibc.so.0`), which is not on the device — cannot run |
| `tmp` | unused | factory test: SAR buttons, gpio46 (front LED colour: 1 orange, 0 red — kernel-platform.md §6.1), the IR remote (18 keys), `mmcblk0`; prints the firmware version from `/customer/version.txt`; nothing starts it |
| `/customer/LCM/*.ini` (FPGA_800x480_TTL, GM8775C_1920x1080_LVDS, RM68200_720x1280_MIPI, SAT070AT50H18BH_1024x600_TTL, SAT070JHH_1024x600_TTL) | unused | external LCD panel configs, no reader; HDMI-only device |
| `/customer/misc/*` (`/customer/misc/config.ini`, `config_cardv.ini`, `boot.ini`, `sigmastar.jpg`, `sigmastar1024_600.jpg`, `sigmastar1024_600_bak.jpg`, `upgrade.jpg`) | unused | build-time sources of the MISC image; the runtime reads `/misc`; `boot.ini` (boot music / logo) has no reader at all; the jpgs are data only |

### 3.8 rootfs [R]

| item | use | notes |
|---|---|---|
| busybox 1.20.2 (2020-08-05) + 352 applet links | used | init, sh, mdev, telnetd, insmod, devmem, watchdog… |
| glibc 2.30 armhf set, libstdc++ 6.0.26, libgcc_s, zlib, libfuse 2.9.9 | used | |
| `/etc/fw_printenv` (+ `fw_setenv` symlink), `fw_env.config` | used | u-boot-tools; used by demo.sh, prog_uvc, sd_version_recognition |
| `/bin/fwfs` | used | firmwarefs FUSE daemon for MISC: 68.8 KB file, 25.6 KB `.text`, 214 named functions; symbols but no DWARF for the fwfs code (the debug info covers only crt / libgcc objects); its core, like the stock U-Boot's, is the OpenIPC `fs/firmwarefs` text (misc-firmwarefs.md §1.3, §1.2) |
| inittab, fstab, profile, mdev.conf, sysctl.conf, passwd / group / shadow | used | stock-style busybox configs with small vendor content (mdev hook for `mmcblk*`, `LD_LIBRARY_PATH`, `TERMINFO`) |
| `/lib/modules/4.9.227` | used | symlink → `/config/modules/4.9.227` |
| `/bin/scp`, `/etc/init.d/udhcpc.script` | unused | OpenSSH scp client with no ssh on the device; no DHCP client started |

## 4. Public counterparts of the vendor code
<!-- ev:firmware-image.4 -->

Sizes and interfaces are in §3; this table gives what public code corresponds to each vendor part and what the part
mostly consists of. Both columns are [I] unless tagged. "Vendor 4.9.84 trees" = linux-chenxing-vendor-slop (e.g.
`ikayaki_dlm00v014`) and fifteenhex/ssd_lcd: SigmaStar kernels of sibling chips that carry
`drivers/sstar`; that each driver below is in them is expected, not checked file by file [I]. Mainline Linux has
MStar/SigmaStar ARMv7 support from linux-chenxing (intc, GPIO, watchdog, RTC, some clocks for MSC313/SSD20x) [I, from
memory].

| item | public counterpart | character |
|---|---|---|
| mhal.ko | none public; HAL headers only in the vendor trees | undocumented register programming of a dozen IP blocks, the ISP about a third of the functions |
| mi_sys.ko | user API: Xsyt `m6/MI/include/mi_sys*.h`, sgs_docs | the buffer / bind / MMA engine the other modules sit on |
| mi_isp.ko | skyipc_tiramisu ISP headers, divinus `m6_isp.h` | talks to mhal ISP and libispalgo; coupled to the 3A library and the IQ bin formats |
| mi_venc.ko | Xsyt `mi_venc*.h` | rate control and VPU / JPE stream handling |
| mi_vdec.ko | — | playback only |
| mi_vif, mi_scl, mi_disp, mi_jpd, mi_rgn | public SigmaStar headers of other SDK trees | mostly parameter checking and job queueing over mhal |
| mi_hdmi, mi_sensor, mi_gfx, mi_common | public headers for mi_sensor and mi_common only; none for mi_gfx or mi_hdmi [R: 2026-10-27] | thin layers over mhal |
| libmi_* (the 11 with static copies) + libcam_os / fs_wrapper (0.3–36 KB each, dynamic exports only [R]) | the camera's own `.so`; the ioctl rules in `mi-user-abi.md` | ioctl number + struct size per call; libmi_isp: 402 exports of struct marshalling |
| fbdev.ko (`/dev/fb0` + SigmaStar ioctls) | vendor 4.9.84 trees? (fbdev is shipped as `.ko` there too) | GOP set-up through mhal plus fbdev boilerplate |
| g_webcam.ko | upstream `webcam.c` | descriptor tables |
| udc-msb250x.ko, kdrv_sdmmc.ko | vendor 4.9.84 trees | |
| kernel built-ins (§3.2) | vendor 4.9.84 trees; mach-sstar/mercury6 in gitee marcio/sigmastar; mainline for a few | msys: its kernel API is used by mhal and the MI modules |
| DTB | the decompiled board DTS [R] | |
| config_tool | — (the ini formats are plain text) | |
| fwfs | littlefs v2.2.1; OpenIPC u-boot-sigmastar `fs/firmwarefs` (the same core text) [R] | |
| rcS, demo.sh, set.sh, sd_hotplug.sh, usb_otg.sh, sd_version_recognition, the riu_w table | — (shell and one small C tool, < 4 KB each [R]) | |
| riu_r / riu_w / riux32_r / riux32_w (one 5612 B file [R]) | SigmaStar sample `drivers/sstar/samples/riu.c` [R] | |

## Appendix — the file inventory [R unless a note says otherwise]

One row per file, flash region or kernel driver group (busybox applet links and the glibc set count once per file).
`use`: **used** (something on the running camera reads, loads or executes it), **unused** (nothing does; the evidence is
in the note), **static-copy** (a `/config/lib` `.so` loaded by nothing, whose code runs statically linked inside `prog_uvc`).

| path | partition | type | size (B) | use | used by | notes |
|---|---|---|---|---|---|---|
| flash:IPL | BOOT | boot blob | 65536 (0x7400 used) | used | SoC boot ROM | first loader: DRAM init, table-driven for bonding 0x11 plus a BIST (no DRAM training on this board; software DQS training only for bonding 0x27), 256 MB, miupll 266 MHz; finds MXPT, loads IPL_CUST; build a0090c8; no public source known |
| flash:IPL_CUST | BOOT | boot blob | 65536 (0x5c90 used) | used | IPL | second loader: XZ-unpacks U-Boot to 0x23E00000; also has a load-from-USB path; offset 0x523F holds the XZ decoder magic constants |
| flash:MXPT | BOOT | partition table | 65536 (4 KB table) | used | IPL, U-Boot, kernel (MXP_PARTS) | MXPT entries of 0x88 B: IPL,IPL_CUST,MXPT,UBOOT,UBOOT_ENV,BOOT,KERNEL,rootfs,miservice,customer,MISC |
| flash:UBOOT | BOOT | U-Boot uImage (xz) | 258048 (247116 image) | used | IPL_CUST | U-Boot 2015.01 SigmaStar M6, built 2025-06-03; header compression lzma (ih_comp 3), payload xz; bootlogo over HDMI, firmwarefs, sdstar/dstar/estar upgrade |
| flash:UBOOT_ENV | BOOT | U-Boot env | 4096 | used | U-Boot, fw_printenv | bootcmd, bootargs (mma_heap 0x9000000, cma 2M, mmap_reserved fb 8M), SdUpgradeImage=H161X.bin, ota_upgrade_status |
| flash:KERNEL | KERNEL | uImage xz | 2162688 (2150812 used) | used | U-Boot bootm | see kernel:* rows; Image 4.5 MB with built-in DTB |
| flash:rootfs | rootfs | squashfs xz | 2097152 (1945600 used) | used | kernel root=/dev/mtdblock2 | glibc 2.30 + busybox 1.20.2 + a few vendor tools |
| flash:miservice | miservice | squashfs xz | 4194304 (4132864 used) | used | rcS mount /config | SigmaStar SDK release (MI modules/libs, mhal, firmware, IQ, ini); 98.5% full |
| flash:customer | customer | jffs2 rw | 6356992 | used | rcS mount /customer | ODM apps + scripts + data; no fully erased block left in the dump |
| flash:MISC | MISC | firmwarefs (littlefs fork) | 655360 | used | U-Boot bootlogo; rcS fwfs -> /misc | firmwarefs = littlefs v2.2.1 with SigmaStar TFTL changes; files: config.ini 76005, PQ.bin 25892, sigmastar1024_600.jpg 49873, upgrade.jpg 17833, ws.txt; 4 of 10 blocks erased |
| flash:unpartitioned | - | erased | 851968 | unused | - | 0xF30000-0xFFFFFF all 0xFF, not in MXPT |
| /misc/config.ini | MISC | text config | 76005 | used | U-Boot bootlogo ([LOGO] HDMI DACOUT_1080P_60); mi_sys -> config_tool load_config | identical to /config/config.ini (all 76005 B); panel/HDMI timing tables |
| /misc/PQ.bin | MISC | data | 25892 | unused | - | only named in [LOGO1] (LCD dev 1) and [PQ_PATH]; U-Boot logs "can't get LOGO:m_sPQFile0" [R] |
| /misc/sigmastar1024_600.jpg | MISC | JPEG | 49873 | used | U-Boot bootlogo m_sLogoFile0; kjd_lvgl | boot splash; same bytes as /customer/misc copy |
| /misc/upgrade.jpg | MISC | JPEG | 17833 | used | U-Boot during SD upgrade (m_sLogoFile1) |  |
| /misc/ws.txt | MISC | text | 0 | used | prog_uvc crop modes | inline file; prog_uvc crop modes (prog_uvc.md) |
| kernel:core | KERNEL | built-in | - | used | boot | Linux 4.9.227 #44 SMP PREEMPT, gcc 9.1.0, kjd-AIOT-Server 2026-03-05, branch BR:WS_H1605_NOR_8836_IMX662; kallsyms 41651 syms |
| kernel:dtb | KERNEL | built-in | - | used | kernel probe | built-in FDT 49471 B at Image+0x421fa0, model MERCURY6 SSC019A-S01A-S; decompiles with dtc |
| kernel:mach-sstar/mercury6 | KERNEL | built-in | - | used | boot | SMP boot, PM, MIU arbitration; source in gitee marcio/sigmastar |
| kernel:intc+timer | KERNEL | built-in | - | used | all | ms_main_intc/ms_pm_intc/ms_gpi_intc, ms_timer |
| kernel:clk/cpufreq/voltage | KERNEL | built-in | - | used | all | ms_clk_composite/complex, ms_cpufreq (800 MHz), sstar voltage-ctrl |
| kernel:uart | KERNEL | built-in | - | used | console ttyS0 | ms_uart, 5 UARTs in DT |
| kernel:gpio/padmux | KERNEL | built-in | - | used | kjd_lvgl gpio46, mhal | MDrv_GPIO/MHal_GPIO, sstar-padmux, camdriver_gpio |
| kernel:i2c | KERNEL | built-in | - | used | sensor (via mhal), /dev/i2c-* | HWI2C + gpio-i2c, 6 buses in DT |
| kernel:spi-nor (FSP/serflash/BDMA) | KERNEL | built-in | - | used | MTD for all partitions | mtd-flashisp, MXP partition parser |
| kernel:msys/MMA/DMEM | KERNEL | built-in | - | used | mhal, MI modules, riu_w | /dev/msys (msys_ioctl), only user-space opener riu_w; /sys/devices/virtual/mstar/msys/{CHIP_ID,CHIP_VERSION}; mma heap from bootargs; prog_uvc carries the /dev/msys and CHIP_ID strings in its static CamOs copy with no code reference |
| kernel:miu/mmu/movdma | KERNEL | built-in | - | used | mhal, MI | /sys/devices/system/miu/miu0/dram_info: string in prog_uvc's static CamOs copy, no code reference |
| kernel:sar-adc | KERNEL | built-in | - | used | kjd_lvgl buttons | /dev/sar ioctl 0x6100 init, 0x6101 read (kjd_lvgl.md) |
| kernel:ir | KERNEL | built-in | - | used | kjd_lvgl IR remote | mdrv_ir -> /dev/input/event0 |
| kernel:pwm | KERNEL | built-in | - | unused | none | ms_pwm; no consumer |
| kernel:watchdog | KERNEL | built-in | - | unused | none | infinity_wdt /dev/watchdog; no consumer in the vendor software |
| kernel:rtc | KERNEL | built-in | - | used | kernel hctosys; kjd_lvgl hwclock -w; busybox poweroff (pm_power_off) | sstar_rtcpwc; clock starts at 1970 |
| kernel:usb-phy | KERNEL | built-in | - | used | udc-msb250x | sstar-usb-phy |
| kernel:CamOs | KERNEL | built-in | - | used | mhal, sensor driver | about 150 CamOs* functions built in (OS shim used by vendor modules) |
| kernel:notify/chip | KERNEL | built-in | - | unused | none | mstar notify driver, mstar_chip; no consumer |
| kernel:emac | KERNEL | built-in | - | unused | - | EMAC link down in every boot log; network is RNDIS usb0 |
| kernel:sata/ahci | KERNEL | built-in | - | unused | - | no SATA on the board |
| kernel:ive | KERNEL | built-in | - | unused | - | mdrv_ive built in; no app uses MI_IVE |
| kernel:aes/sha/rsa | KERNEL | built-in | - | unused | - | infinity aes/sha; prog_uvc's XU challenge uses software AES |
| kernel:goodix gt911 | KERNEL | built-in | - | unused | - | DT node not disabled: probed, three I2C timeouts every boot; no touch panel (hardware not on this camera) |
| /bin/busybox | rootfs | ELF exec | 679184 | used | init, shell, mdev, telnetd, insmod... | BusyBox 1.20.2 (2020-08-05), 352 applet links |
| /bin/fwfs | rootfs | ELF exec | 70488 | used | rcS: fwfs ... /dev/mtd5 /misc | SigmaStar firmwarefs FUSE daemon; 25.6 KB .text, 214 named functions; symbols but no DWARF for the fwfs code (debug info only for crt/libgcc objects); core = OpenIPC fs/firmwarefs text |
| /bin/scp | rootfs | ELF exec | 42852 | unused | - | OpenSSH scp client; no ssh binary on the device |
| /etc/fstab | rootfs | data/text | 254 | used | busybox init/login/mdev | stock-style busybox configs |
| /etc/fw_env.config | rootfs | data/text | 33 | used | fw_printenv | /dev/mtd0 0x6F000 0x1000 0x10000 |
| /etc/fw_printenv | rootfs | ELF exec | 26432 | used | demo.sh, prog_uvc, sd_version_recognition (as fw_setenv) | u-boot-tools |
| /etc/fw_setenv | rootfs | symlink -> fw_printenv | 11 | used | symlink -> fw_printenv |  |
| /etc/group | rootfs | data/text | 13 | used | busybox init/login/mdev | stock-style busybox configs |
| /etc/inittab | rootfs | data/text | 48 | used | busybox init/login/mdev | stock-style busybox configs |
| /etc/mdev.conf | rootfs | data/text | 245 | used | busybox init/login/mdev | stock-style busybox configs |
| /etc/passwd | rootfs | data/text | 26 | used | busybox init/login/mdev | stock-style busybox configs |
| /etc/passwd- | rootfs | data/text | 44 | used | busybox init/login/mdev | stock-style busybox configs |
| /etc/profile | rootfs | data/text | 279 | used | busybox init/login/mdev | stock-style busybox configs |
| /etc/shadow | rootfs | data/text | 54 | used | busybox init/login/mdev | stock-style busybox configs |
| /etc/sysctl.conf | rootfs | data/text | 239 | used | busybox init/login/mdev | stock-style busybox configs |
| /etc/init.d/rcS | rootfs | data/text | 1536 | used | inittab sysinit | mounts /config /customer, fwfs /misc, telnetd, demo.sh, 22 riu_w writes |
| /etc/init.d/udhcpc.script | rootfs | data/text | 1295 | unused | - | no DHCP client started |
| /lib/ld-2.30.so | rootfs | ELF .so | 105792 | used | all dynamic binaries | glibc 2.30 armhf |
| /lib/ld-linux-armhf.so.3 | rootfs | symlink -> ld-2.30.so | 10 | used | all dynamic binaries | glibc 2.30 armhf |
| /lib/libc-2.30.so | rootfs | ELF .so | 926636 | used | apps | glibc 2.30 / libstdc++ 6.0.26 / libgcc_s / zlib / libfuse 2.9.9 |
| /lib/libc.so.6 | rootfs | symlink -> libc-2.30.so | 12 | used | apps | glibc 2.30 / libstdc++ 6.0.26 / libgcc_s / zlib / libfuse 2.9.9 |
| /lib/libcrypt.so | rootfs | ELF .so | 30348 | used | apps | glibc 2.30 / libstdc++ 6.0.26 / libgcc_s / zlib / libfuse 2.9.9 |
| /lib/libcrypt.so.1 | rootfs | ELF .so | 30348 | used | apps | glibc 2.30 / libstdc++ 6.0.26 / libgcc_s / zlib / libfuse 2.9.9 |
| /lib/libdl-2.30.so | rootfs | ELF .so | 9724 | used | apps | glibc 2.30 / libstdc++ 6.0.26 / libgcc_s / zlib / libfuse 2.9.9 |
| /lib/libdl.so.2 | rootfs | symlink -> libdl-2.30.so | 13 | used | apps | glibc 2.30 / libstdc++ 6.0.26 / libgcc_s / zlib / libfuse 2.9.9 |
| /lib/libfuse.so | rootfs | symlink -> libfuse.so.2.9.9 | 16 | used | apps | glibc 2.30 / libstdc++ 6.0.26 / libgcc_s / zlib / libfuse 2.9.9 |
| /lib/libfuse.so.2 | rootfs | symlink -> libfuse.so.2.9.9 | 16 | used | apps | glibc 2.30 / libstdc++ 6.0.26 / libgcc_s / zlib / libfuse 2.9.9 |
| /lib/libfuse.so.2.9.9 | rootfs | ELF .so | 137684 | used | apps | glibc 2.30 / libstdc++ 6.0.26 / libgcc_s / zlib / libfuse 2.9.9 |
| /lib/libgcc_s.so.1 | rootfs | ELF .so | 112280 | used | apps | glibc 2.30 / libstdc++ 6.0.26 / libgcc_s / zlib / libfuse 2.9.9 |
| /lib/libm-2.30.so | rootfs | ELF .so | 353752 | used | apps | glibc 2.30 / libstdc++ 6.0.26 / libgcc_s / zlib / libfuse 2.9.9 |
| /lib/libm.so.6 | rootfs | symlink -> libm-2.30.so | 12 | used | apps | glibc 2.30 / libstdc++ 6.0.26 / libgcc_s / zlib / libfuse 2.9.9 |
| /lib/libnss_compat.so.2 | rootfs | ELF .so | 22272 | used | apps | glibc 2.30 / libstdc++ 6.0.26 / libgcc_s / zlib / libfuse 2.9.9 |
| /lib/libnss_files.so.2 | rootfs | ELF .so | 30296 | used | apps | glibc 2.30 / libstdc++ 6.0.26 / libgcc_s / zlib / libfuse 2.9.9 |
| /lib/libpcprofile.so | rootfs | ELF .so | 5492 | used | apps | glibc 2.30 / libstdc++ 6.0.26 / libgcc_s / zlib / libfuse 2.9.9 |
| /lib/libpthread-2.30.so | rootfs | ELF .so | 76096 | used | apps | glibc 2.30 / libstdc++ 6.0.26 / libgcc_s / zlib / libfuse 2.9.9 |
| /lib/libpthread.so.0 | rootfs | symlink -> libpthread-2.30.so | 18 | used | apps | glibc 2.30 / libstdc++ 6.0.26 / libgcc_s / zlib / libfuse 2.9.9 |
| /lib/libresolv-2.30.so | rootfs | ELF .so | 50952 | used | apps | glibc 2.30 / libstdc++ 6.0.26 / libgcc_s / zlib / libfuse 2.9.9 |
| /lib/libresolv.so.2 | rootfs | symlink -> libresolv-2.30.so | 17 | used | apps | glibc 2.30 / libstdc++ 6.0.26 / libgcc_s / zlib / libfuse 2.9.9 |
| /lib/librt-2.30.so | rootfs | ELF .so | 22456 | used | apps | glibc 2.30 / libstdc++ 6.0.26 / libgcc_s / zlib / libfuse 2.9.9 |
| /lib/librt.so.1 | rootfs | symlink -> librt-2.30.so | 13 | used | apps | glibc 2.30 / libstdc++ 6.0.26 / libgcc_s / zlib / libfuse 2.9.9 |
| /lib/libstdc++.so | rootfs | symlink -> libstdc++.so.6.0.26 | 19 | used | apps | glibc 2.30 / libstdc++ 6.0.26 / libgcc_s / zlib / libfuse 2.9.9 |
| /lib/libstdc++.so.6 | rootfs | symlink -> libstdc++.so.6.0.26 | 19 | used | apps | glibc 2.30 / libstdc++ 6.0.26 / libgcc_s / zlib / libfuse 2.9.9 |
| /lib/libstdc++.so.6.0.26 | rootfs | ELF .so | 1270200 | used | apps | glibc 2.30 / libstdc++ 6.0.26 / libgcc_s / zlib / libfuse 2.9.9 |
| /lib/libutil.so | rootfs | ELF .so | 9752 | used | apps | glibc 2.30 / libstdc++ 6.0.26 / libgcc_s / zlib / libfuse 2.9.9 |
| /lib/libutil.so.1 | rootfs | ELF .so | 9752 | used | apps | glibc 2.30 / libstdc++ 6.0.26 / libgcc_s / zlib / libfuse 2.9.9 |
| /lib/libz.so | rootfs | ELF .so | 75388 | used | apps | glibc 2.30 / libstdc++ 6.0.26 / libgcc_s / zlib / libfuse 2.9.9 |
| /lib/libz.so.1 | rootfs | ELF .so | 75388 | used | apps | glibc 2.30 / libstdc++ 6.0.26 / libgcc_s / zlib / libfuse 2.9.9 |
| /lib/modules/4.9.227 | rootfs | symlink -> /config/modules/4.9.227 | 23 | used | insmod/modprobe path | symlink -> /config/modules/4.9.227 |
| /config/board.ini | miservice | data/text | 472 | unused | - | LVDS panel board settings; named only in model/Customer.ini; no binary reads it; #@CRC=0x725D = 16-bit byte sum of the preceding 460 bytes |
| /config/config.ini | miservice | data/text | 76005 | unused | config_tool (first fallback path) | identical to /misc/config.ini; config_tool's first fallback (then /config/model/config.ini); the boot log shows /misc/config.ini loaded |
| /config/config_tool | miservice | ELF exec | 49616 | used | mi_sys usermode helper (load_mmap, load_config) | parses mmap.ini, /misc/config.ini, /misc/PQConfig.ini (absent) into /proc/mi_modules/common/*; 48.5 KB, 25.6 KB .text; stripped |
| /config/dump_config | miservice | symlink -> config_tool | 11 | used | symlink -> config_tool |  |
| /config/dump_mmap | miservice | symlink -> config_tool | 11 | used | symlink -> config_tool |  |
| /config/fbdev.ini | miservice | data/text | 558 | used | fbdev.ko default_config_path_file | FB 1600x900 ARGB on 1920x1080 timing, 8 MB |
| /config/mmap.ini | miservice | data/text | 5487 | used | mi_sys (config_tool), fbdev, mi_venc | MMA memory map |
| /config/dla/ipu_firmware.bin | miservice | data | 602632 | unused | - | RISC-V firmware of the IPU (NPU), ipu_ccif_riscv_server; no app uses the IPU |
| /config/iqfile/imx307_api.bin | miservice | data | 93961 | unused | - | ISP API bin for IMX307; app loads /customer/iq_file_gc8613.bin |
| /config/iqfile/imx307_iqfile.bin | miservice | data | 39135 | used | AE/AWB/AF init via iqfile0.bin (mhal and the user 3A in prog_uvc) | IMX307 calibration, loaded for the GC8613 sensor at every start ("Load iq file /config/iqfile/iqfile0.bin") |
| /config/iqfile/iqfile0.bin | miservice | symlink -> imx307_iqfile.bin | 17 | used | 3A init (mhal "%s/iqfile%d.bin") | symlink -> imx307_iqfile.bin |
| /config/iqfile/iqfile1.bin | miservice | symlink -> imx307_iqfile.bin | 17 | unused | - | symlink -> imx307_iqfile.bin; channel 1..3 not used |
| /config/iqfile/iqfile2.bin | miservice | symlink -> imx307_iqfile.bin | 17 | unused | - | symlink |
| /config/iqfile/iqfile3.bin | miservice | symlink -> imx307_iqfile.bin | 17 | unused | - | symlink |
| /config/iqfile/isp_api.xml | miservice | data/text | 164700 | used | prog_uvc's static iqserver (-q) | IQTool description; served by MI_IQSERVER_DownloadFile ID 2 from this fixed path; tuning only |
| /config/lib/AEC_LICENSE_1.txt | miservice | data/text | 391 | unused | - | Ooura FFT notice |
| /config/lib/AEC_LICENSE_2.txt | miservice | data/text | 1510 | unused | - | WebRTC BSD-3 |
| /config/lib/APC_LICENSE_1.txt | miservice | data/text | 391 | unused | - | Ooura FFT notice |
| /config/lib/APC_LICENSE_2.txt | miservice | data/text | 1510 | unused | - | WebRTC BSD-3 |
| /config/lib/libAEC_LINUX.so | miservice | ELF .so | 79636 | unused | - | audio algorithm; linear filter is speexdsp code (no licence file); stock SigmaStar binary, byte-identical to the Tiramisu SDK V012 drop (qanthink/skyipc_tiramisu); referenced by libmi_vdf.so; no static copy in prog_uvc |
| /config/lib/libAED_LINUX.so | miservice | ELF .so | 115480 | unused | - | audio algorithm; stock SigmaStar binary, byte-identical to the Tiramisu SDK V012 drop (qanthink/skyipc_tiramisu); referenced by libmi_vdf.so; no static copy in prog_uvc |
| /config/lib/libAPC_LINUX.so | miservice | ELF .so | 125772 | unused | - | audio algorithm; stock SigmaStar binary, byte-identical to the Tiramisu SDK V012 drop (qanthink/skyipc_tiramisu); referenced by libmi_vdf.so; no static copy in prog_uvc |
| /config/lib/libBF_LINUX.so | miservice | ELF .so | 142680 | unused | - | audio algorithm; VAD is SILK's (psSilk_VAD), links Ne10, no licence files for either; stock SigmaStar binary, byte-identical to the Tiramisu SDK V012 drop (qanthink/skyipc_tiramisu); referenced by libmi_vdf.so; no static copy in prog_uvc |
| /config/lib/libMD_LINUX.so | miservice | ELF .so | 150148 | unused | - | video analytics (motion detection); stock SigmaStar binary, byte-identical to the Tiramisu SDK V012 drop (qanthink/skyipc_tiramisu); referenced by libmi_vdf.so; no static copy in prog_uvc |
| /config/lib/libOD_LINUX.so | miservice | ELF .so | 109240 | unused | - | video analytics (occlusion detection); stock SigmaStar binary, byte-identical to the Tiramisu SDK V012 drop (qanthink/skyipc_tiramisu); referenced by libmi_vdf.so; no static copy in prog_uvc |
| /config/lib/libSRC_LINUX.so | miservice | ELF .so | 37152 | unused | - | audio algorithm; stock SigmaStar binary, byte-identical to the Tiramisu SDK V012 drop (qanthink/skyipc_tiramisu); referenced by libmi_vdf.so; no static copy in prog_uvc |
| /config/lib/libSSL_LINUX.so | miservice | ELF .so | 30900 | unused | - | audio algorithm; stock SigmaStar binary, byte-identical to the Tiramisu SDK V012 drop (qanthink/skyipc_tiramisu); referenced by libmi_vdf.so; no static copy in prog_uvc |
| /config/lib/libVG_LINUX.so | miservice | ELF .so | 54920 | unused | - | video analytics (virtual gate); stock SigmaStar binary, byte-identical to the Tiramisu SDK V012 drop (qanthink/skyipc_tiramisu); referenced by libmi_vdf.so; no static copy in prog_uvc |
| /config/lib/libcam_fs_wrapper.so | miservice | ELF .so | 5448 | static-copy | prog_uvc (static copy) |  |
| /config/lib/libcam_os_wrapper.so | miservice | ELF .so | 34752 | static-copy | prog_uvc (static copy) | CamOs user-space shim (pthread/mmap wrappers) |
| /config/lib/libcjson.so | miservice | ELF .so | 26048 | unused | - | cJSON 1.7.7 unmodified; imported by libmi_wlan (unused); no static copy in prog_uvc |
| /config/lib/libcus3a.so | miservice | ELF .so | 26592 | static-copy | prog_uvc (static copy) | 3A glue + custom-3A hook API |
| /config/lib/libfbc_decode.so | miservice | ELF .so | 166608 | static-copy | prog_uvc's static libmi_iqserver copy (FBD_Execute) | FBC decoder plus the full FBC encoder model and three INI layers; 342 rows / 322 distinct .dynsym entries; loaded by nothing (no NEEDED, no dlopen name) |
| /config/lib/libg711.so | miservice | ELF .so | 5648 | unused | - | Sun g711.c (alaw2linear decodes A-law segment 0 to 0); no static copy in prog_uvc |
| /config/lib/libg726.so | miservice | ELF .so | 9512 | unused | - | Sun G.72x plus 16 kbit/s tables; no static copy in prog_uvc |
| /config/lib/libispalgo.so | miservice | ELF .so | 173160 | static-copy | prog_uvc (static copy) | SigmaStar AE/AWB/AF algorithms |
| /config/lib/libmi_ai.so | miservice | ELF .so | 30256 | unused | - | prog_uvc runs without -d/-D/-f |
| /config/lib/libmi_ao.so | miservice | ELF .so | 34364 | unused | - | prog_uvc runs without -d/-D/-f |
| /config/lib/libmi_common.so | miservice | ELF .so | 14004 | static-copy | prog_uvc (static copy) | g_mi_fd_table, device open |
| /config/lib/libmi_disp.so | miservice | ELF .so | 13792 | static-copy | prog_uvc (static copy) |  |
| /config/lib/libmi_divp.so | miservice | ELF .so | 9664 | unused | - | 2020 wrapper generation (private fd/pid pair, no lock, no shared table); no mi_divp.ko provides /dev/mi_divp |
| /config/lib/libmi_gfx.so | miservice | ELF .so | 9692 | unused | - | loaded by nothing (no NEEDED, no dlopen name); no static copy in prog_uvc |
| /config/lib/libmi_gyro.so | miservice | ELF .so | 9664 | unused | - | 2020 wrapper generation (private fd/pid pair, no lock, no shared table); no static copy in prog_uvc |
| /config/lib/libmi_hdmi.so | miservice | ELF .so | 30260 | static-copy | prog_uvc (static copy) |  |
| /config/lib/libmi_ipu.so | miservice | ELF .so | 26168 | unused | - | loaded by nothing (no NEEDED, no dlopen name); no static copy in prog_uvc |
| /config/lib/libmi_iqserver.so | miservice | ELF .so | 51720 | static-copy | prog_uvc -q (TCP 9876, static copy) | IQTool server; 48 exported functions; the .so does not need libfbc_decode.so; loaded by nothing (no NEEDED, no dlopen name) |
| /config/lib/libmi_isp.so | miservice | ELF .so | 73632 | static-copy | prog_uvc (static copy) | 402 exports, IQ API struct marshalling |
| /config/lib/libmi_ive.so | miservice | ELF .so | 742208 | unused | - | imports five MI_SYS_* functions without a NEEDED entry for libmi_sys; no static copy in prog_uvc |
| /config/lib/libmi_jpd.so | miservice | ELF .so | 13808 | static-copy | prog_uvc (static copy) | photo viewer |
| /config/lib/libmi_ldc.so | miservice | ELF .so | 9692 | unused | - | prog_uvc runs without -L |
| /config/lib/libmi_mipitx.so | miservice | ELF .so | 9680 | unused | - | loaded by nothing (no NEEDED, no dlopen name); no static copy in prog_uvc |
| /config/lib/libmi_panel.so | miservice | ELF .so | 9688 | unused | - | loaded by nothing (no NEEDED, no dlopen name); no static copy in prog_uvc |
| /config/lib/libmi_rgn.so | miservice | ELF .so | 9704 | unused | - | loaded by nothing (no NEEDED, no dlopen name); no static copy in prog_uvc |
| /config/lib/libmi_scl.so | miservice | ELF .so | 9680 | static-copy | prog_uvc (static copy) |  |
| /config/lib/libmi_sed.so | miservice | ELF .so | 22180 | unused | - | needs libmi_ive and libmi_vdf, not through NEEDED; no static copy in prog_uvc |
| /config/lib/libmi_sensor.so | miservice | ELF .so | 9684 | static-copy | prog_uvc (static copy) |  |
| /config/lib/libmi_shadow.so | miservice | ELF .so | 13812 | unused | libmi_vdf.so (14 MI_SHADOW_* imports) | its only importer libmi_vdf has no user |
| /config/lib/libmi_sys.so | miservice | ELF .so | 13928 | static-copy | prog_uvc (static copy) | ioctl wrapper to /dev/mi_sys |
| /config/lib/libmi_vdec.so | miservice | ELF .so | 13804 | static-copy | prog_uvc (static copy) | player |
| /config/lib/libmi_vdf.so | miservice | ELF .so | 67464 | unused | - | needs libmi_shadow and the MD/OD/VG libraries, not through NEEDED; no user |
| /config/lib/libmi_vdisp.so | miservice | ELF .so | 9680 | unused | - | loaded by nothing (no NEEDED, no dlopen name); no static copy in prog_uvc |
| /config/lib/libmi_venc.so | miservice | ELF .so | 46608 | static-copy | prog_uvc (static copy) |  |
| /config/lib/libmi_vif.so | miservice | ELF .so | 9680 | static-copy | prog_uvc (static copy) |  |
| /config/lib/libmi_vpe.so | miservice | ELF .so | 13776 | unused | - | 2020 wrapper generation; its MI_ISP_EnableUserspace3A(chn, &pair) call does not match libmi_isp's (dev, chn); no static copy in prog_uvc |
| /config/lib/libmi_wlan.so | miservice | ELF .so | 39112 | unused | - | imports cJSON; no static copy in prog_uvc |
| /config/lib/libsigma3a.so | miservice | ELF .so | 10048 | unused | - | ABI-incompatible with this libispalgo (packed 81-byte AeInput_t, M5 AeOutput_t, two-argument CUS3A calls), 4 channels, no device argument; no static copy in prog_uvc |
| /config/model/Customer.ini | miservice | data/text | 2808 | unused | - | no binary reads it |
| /config/model/LCM.ini | miservice | data/text | 475 | unused | - | external LCD panel config (hardware not on this camera); no binary reads it |
| /config/modules/4.9.227/cifs.ko | miservice | ELF .ko | 210788 | unused | - |  |
| /config/modules/4.9.227/ehci-hcd.ko | miservice | ELF .ko | 49260 | unused | - | loaded, SigmaStar EHCI glue; no USB host port in use [I] |
| /config/modules/4.9.227/fat.ko | miservice | ELF .ko | 55320 | used | SD card vfat |  |
| /config/modules/4.9.227/fbdev.ko | miservice | ELF .ko | 47932 | used | kjd_lvgl /dev/fb0 | SigmaStar GOP framebuffer, 1600x900 on 1080p, reads /config/fbdev.ini |
| /config/modules/4.9.227/g_mass_storage.ko | miservice | ELF .ko | 7972 | used | kjd_lvgl USB-disk mode |  |
| /config/modules/4.9.227/g_webcam.ko | miservice | ELF .ko | 12960 | used | prog_uvc insmod (uvc+rndis) | vendor-patched GPL webcam.c: UVC descriptors + RNDIS (usb-gadget.md) |
| /config/modules/4.9.227/gc8613_MIPI.ko | miservice | ELF .ko | 11240 | used | demo.sh insmod chmap=1 lane_num=4 | vendor GC8613 sensor driver |
| /config/modules/4.9.227/grace.ko | miservice | ELF .ko | 5592 | unused | - |  |
| /config/modules/4.9.227/imx307_MIPI.ko | miservice | ELF .ko | 21180 | unused | - | never loaded; second instance of the sensor-driver ABI (hardware not on this camera) |
| /config/modules/4.9.227/kdrv_sdmmc.ko | miservice | ELF .ko | 49560 | used | SD card slot | SigmaStar SD host driver |
| /config/modules/4.9.227/libcomposite.ko | miservice | ELF .ko | 40424 | used | gadget stack |  |
| /config/modules/4.9.227/lockd.ko | miservice | ELF .ko | 62284 | unused | - |  |
| /config/modules/4.9.227/mdrv-sata-host.ko | miservice | ELF .ko | 23544 | unused | - | loaded, no SATA on the board |
| /config/modules/4.9.227/media.ko | miservice | ELF .ko | 21932 | unused | - | insmod fails: duplicate symbols, built into the kernel |
| /config/modules/4.9.227/mhal.ko | miservice | ELF .ko | 2531752 | used | all mi_*.ko, fbdev, sensor driver | HAL for VIF/CSI/ISP/SCL/DISP/HDMI-TX/GOP/VENC/JPE/JPD/LDC/CMDQ/RGN/MIPI-TX/PNL/audio; 988 exported syms; full symtab, no debug info; build 2022-07-21 alkaid |
| /config/modules/4.9.227/mi_ai.ko | miservice | ELF .ko | 146256 | unused | - | loaded; audio only with prog_uvc -d/-D or the -f audio-file test mode; none passed |
| /config/modules/4.9.227/mi_ao.ko | miservice | ELF .ko | 129104 | unused | - | same as mi_ai |
| /config/modules/4.9.227/mi_common.ko | miservice | ELF .ko | 13888 | used | all mi_*.ko | shared helpers |
| /config/modules/4.9.227/mi_disp.ko | miservice | ELF .ko | 211936 | used | prog_uvc /dev/mi_disp, fbdev |  |
| /config/modules/4.9.227/mi_gfx.ko | miservice | ELF .ko | 40584 | used | mi_disp (module dependency) | 2D blitter (GE); mi_disp calls it only for rotation, which is not used |
| /config/modules/4.9.227/mi_hdmi.ko | miservice | ELF .ko | 51700 | used | prog_uvc /dev/mi_hdmi | EDID/hot-plug, timing |
| /config/modules/4.9.227/mi_ipu.ko | miservice | ELF .ko | 37824 | unused | - | NPU; no app opens /dev/mi_ipu |
| /config/modules/4.9.227/mi_isp.ko | miservice | ELF .ko | 150776 | used | prog_uvc /dev/mi_isp, libispalgo | plus /dev/isp_fe (mhal) opened by prog_uvc |
| /config/modules/4.9.227/mi_jpd.ko | miservice | ELF .ko | 103232 | used | prog_uvc photo viewer | only for viewing JPEGs from SD |
| /config/modules/4.9.227/mi_ldc.ko | miservice | ELF .ko | 733220 | unused | - | loaded at every boot by demo.sh (no parameters); prog_uvc runs without -L; about 220 KB public Simd library, 7 KB cJSON 1.7.12; 22 MI_LDC_IMPL_* exports, no importer |
| /config/modules/4.9.227/mi_mipitx.ko | miservice | ELF .ko | 31036 | unused | - | no MIPI-TX panel |
| /config/modules/4.9.227/mi_panel.ko | miservice | ELF .ko | 49488 | unused | - | no LCD panel; HDMI only |
| /config/modules/4.9.227/mi_rgn.ko | miservice | ELF .ko | 134332 | used | mi_scl (module dependency) | OSD regions; mi_scl uses two of its functions; app does not use RGN |
| /config/modules/4.9.227/mi_scl.ko | miservice | ELF .ko | 155540 | used | prog_uvc /dev/mi_scl | needs mi_rgn symbols |
| /config/modules/4.9.227/mi_sensor.ko | miservice | ELF .ko | 44192 | used | prog_uvc /dev/mi_sensor | drives sensor driver through mhal SNR |
| /config/modules/4.9.227/mi_shadow.ko | miservice | ELF .ko | 63296 | unused | - | no user, holders 0; .text 0x4C44 |
| /config/modules/4.9.227/mi_sys.ko | miservice | ELF .ko | 778348 | used | prog_uvc via /dev/mi_sys, all mi_*.ko | bind/buffer/MMA core, /dev/mi_poll, /proc/mi_modules; runs /config/config_tool; params cmdQBufSize logBufSize |
| /config/modules/4.9.227/mi_vdec.ko | miservice | ELF .ko | 246324 | used | prog_uvc MP4 player | only for playback from SD |
| /config/modules/4.9.227/mi_vdisp.ko | miservice | ELF .ko | 79300 | unused | - | loaded (vdisp_bufq_depth=4); prog_uvc never opens /dev/mi_vdisp; .text 0x6B60 |
| /config/modules/4.9.227/mi_venc.ko | miservice | ELF .ko | 442672 | used | prog_uvc /dev/mi_venc | H.264/H.265 on VPU (chagall.bin) + MJPEG on JPE |
| /config/modules/4.9.227/mi_vif.ko | miservice | ELF .ko | 118160 | used | prog_uvc /dev/mi_vif |  |
| /config/modules/4.9.227/mmc_block.ko | miservice | ELF .ko | 30896 | used | SD card |  |
| /config/modules/4.9.227/mmc_core.ko | miservice | ELF .ko | 99760 | used | SD card |  |
| /config/modules/4.9.227/msdos.ko | miservice | ELF .ko | 10364 | unused | - |  |
| /config/modules/4.9.227/nfs.ko | miservice | ELF .ko | 126852 | unused | - |  |
| /config/modules/4.9.227/nfsv2.ko | miservice | ELF .ko | 17116 | unused | - |  |
| /config/modules/4.9.227/nfsv3.ko | miservice | ELF .ko | 23124 | unused | - |  |
| /config/modules/4.9.227/nls_utf8.ko | miservice | ELF .ko | 3784 | unused | - | vfat mounted without iocharset |
| /config/modules/4.9.227/ntfs.ko | miservice | ELF .ko | 89736 | unused | - |  |
| /config/modules/4.9.227/sunrpc.ko | miservice | ELF .ko | 181072 | unused | - |  |
| /config/modules/4.9.227/u_ether.ko | miservice | ELF .ko | 13296 | used | RNDIS usb0 |  |
| /config/modules/4.9.227/udc-core.ko | miservice | ELF .ko | 16800 | used | gadget stack |  |
| /config/modules/4.9.227/udc-msb250x.ko | miservice | ELF .ko | 32688 | used | USB device port | SigmaStar UDC driver, GPL, version 5 June 2019 |
| /config/modules/4.9.227/usb-common.ko | miservice | ELF .ko | 5856 | used | udc-core, libcomposite |  |
| /config/modules/4.9.227/usb-storage.ko | miservice | ELF .ko | 71428 | unused | - |  |
| /config/modules/4.9.227/usb_f_mass_storage.ko | miservice | ELF .ko | 33696 | used | kjd_lvgl USB-disk mode |  |
| /config/modules/4.9.227/usb_f_rndis.ko | miservice | ELF .ko | 17316 | used | RNDIS usb0 192.168.1.100 |  |
| /config/modules/4.9.227/usb_f_uvc.ko | miservice | ELF .ko | 62120 | used | g_webcam | vendor-patched GPL f_uvc (bulk ep, frame-based H.264/H.265) |
| /config/modules/4.9.227/usbcore.ko | miservice | ELF .ko | 171728 | unused | - | host stack; needed only by ehci/usb-storage/usbhid |
| /config/modules/4.9.227/usbhid.ko | miservice | ELF .ko | 23484 | unused | - |  |
| /config/modules/4.9.227/v4l2-common.ko | miservice | ELF .ko | 6236 | unused | - | insmod fails: built into the kernel |
| /config/modules/4.9.227/vfat.ko | miservice | ELF .ko | 11916 | used | sd_hotplug.sh mount -t vfat |  |
| /config/modules/4.9.227/videobuf2-core.ko | miservice | ELF .ko | 24556 | used | usb_f_uvc |  |
| /config/modules/4.9.227/videobuf2-memops.ko | miservice | ELF .ko | 3824 | used | usb_f_uvc |  |
| /config/modules/4.9.227/videobuf2-v4l2.ko | miservice | ELF .ko | 13512 | used | usb_f_uvc |  |
| /config/modules/4.9.227/videobuf2-vmalloc.ko | miservice | ELF .ko | 7924 | used | usb_f_uvc |  |
| /config/modules/4.9.227/videodev.ko | miservice | ELF .ko | 119996 | unused | - | insmod fails: built into the kernel |
| /config/terminfo/v/vt100 | miservice | data/text | 1194 | unused | - | only for htop (TERMINFO in /etc/profile) |
| /config/terminfo/v/vt100+ | miservice | data/text | 1657 | unused | - | only for htop (TERMINFO in /etc/profile) |
| /config/terminfo/v/vt100+enq | miservice | data/text | 682 | unused | - | only for htop (TERMINFO in /etc/profile) |
| /config/terminfo/v/vt100+fnkeys | miservice | data/text | 450 | unused | - | only for htop (TERMINFO in /etc/profile) |
| /config/terminfo/v/vt100+keypad | miservice | data/text | 368 | unused | - | only for htop (TERMINFO in /etc/profile) |
| /config/terminfo/v/vt100+pfkeys | miservice | data/text | 422 | unused | - | only for htop (TERMINFO in /etc/profile) |
| /config/terminfo/v/vt100-am | miservice | data/text | 1194 | unused | - | only for htop (TERMINFO in /etc/profile) |
| /config/terminfo/v/vt100-bm | miservice | data/text | 1215 | unused | - | only for htop (TERMINFO in /etc/profile) |
| /config/terminfo/v/vt100-bm-o | miservice | data/text | 1221 | unused | - | only for htop (TERMINFO in /etc/profile) |
| /config/terminfo/v/vt100-bot-s | miservice | data/text | 1256 | unused | - | only for htop (TERMINFO in /etc/profile) |
| /config/terminfo/v/vt100-nam | miservice | data/text | 1190 | unused | - | only for htop (TERMINFO in /etc/profile) |
| /config/terminfo/v/vt100-nam-w | miservice | data/text | 1221 | unused | - | only for htop (TERMINFO in /etc/profile) |
| /config/terminfo/v/vt100-nav | miservice | data/text | 1059 | unused | - | only for htop (TERMINFO in /etc/profile) |
| /config/terminfo/v/vt100-nav-w | miservice | data/text | 1093 | unused | - | only for htop (TERMINFO in /etc/profile) |
| /config/terminfo/v/vt100-putty | miservice | data/text | 1199 | unused | - | only for htop (TERMINFO in /etc/profile) |
| /config/terminfo/v/vt100-s | miservice | data/text | 1272 | unused | - | only for htop (TERMINFO in /etc/profile) |
| /config/terminfo/v/vt100-s-bot | miservice | data/text | 1256 | unused | - | only for htop (TERMINFO in /etc/profile) |
| /config/terminfo/v/vt100-s-top | miservice | data/text | 1272 | unused | - | only for htop (TERMINFO in /etc/profile) |
| /config/terminfo/v/vt100-top-s | miservice | data/text | 1272 | unused | - | only for htop (TERMINFO in /etc/profile) |
| /config/terminfo/v/vt100-vb | miservice | data/text | 1207 | unused | - | only for htop (TERMINFO in /etc/profile) |
| /config/terminfo/v/vt100-w | miservice | data/text | 1203 | unused | - | only for htop (TERMINFO in /etc/profile) |
| /config/terminfo/v/vt100-w-am | miservice | data/text | 1203 | unused | - | only for htop (TERMINFO in /etc/profile) |
| /config/terminfo/v/vt100-w-nam | miservice | data/text | 1221 | unused | - | only for htop (TERMINFO in /etc/profile) |
| /config/terminfo/v/vt100-w-nav | miservice | data/text | 1093 | unused | - | only for htop (TERMINFO in /etc/profile) |
| /config/terminfo/v/vt100nam | miservice | data/text | 1190 | unused | - | only for htop (TERMINFO in /etc/profile) |
| /config/terminfo/v/vt102 | miservice | data/text | 1188 | unused | - | only for htop (TERMINFO in /etc/profile) |
| /config/terminfo/v/vt102+enq | miservice | data/text | 680 | unused | - | only for htop (TERMINFO in /etc/profile) |
| /config/terminfo/v/vt102-nsgr | miservice | data/text | 1167 | unused | - | only for htop (TERMINFO in /etc/profile) |
| /config/terminfo/v/vt102-w | miservice | data/text | 1210 | unused | - | only for htop (TERMINFO in /etc/profile) |
| /config/venc_fw/chagall.bin | miservice | data | 394112 | used | mhal VENC (/config/venc_fw/chagall.bin) | VPU bitcode "CODEC40.22031116": VCPU firmware plus VCORE engine microcode; runs on the codec core (chagall-fw.md) |
| /customer/1600x900.png | customer | data | 21742 | used | kjd_lvgl | UI asset |
| /customer/Line.ini | customer | data/text | 353 | used | kjd_lvgl | kjd_lvgl.md |
| /customer/camp.txt | customer | data/text | 198 | used | prog_uvc | UVC PU defaults (prog_uvc.md) |
| /customer/default_ws.bin | customer | data | 88 | used | prog_uvc, kjd_lvgl | prog_uvc.md |
| /customer/demo.sh | customer | data/text | 3268 | used | rcS | module load order, usb0 IP, fw_setenv, app start |
| /customer/htop | customer | ELF exec | 743240 | unused | - | htop 1.0.2, not started |
| /customer/i2c_read_write | customer | ELF exec | 22260 | unused | - | debug tool built with gcc-arm-8.2-2018.08 (third-party toolchain); nothing starts it |
| /customer/iperf3 | customer | ELF exec | 134268 | unused | - | iperf 3.1.3 linked to uClibc (/lib/ld-uClibc.so.0 absent): cannot run |
| /customer/iq_file_gc8613.bin | customer | data | 93961 | used | prog_uvc -i | ISP API bin (isp-3-iq-api.md) |
| /customer/kjd_lvgl | customer | ELF exec | 2345232 | used | demo.sh | kjd_lvgl.md |
| /customer/prog_uvc | customer | ELF exec | 579968 | used | demo.sh | prog_uvc.md |
| /customer/riu_r | customer | ELF exec | 5612 | unused | - | same file as riu_w; debug; nothing starts it |
| /customer/riu_w | customer | ELF exec | 5612 | used | rcS (22 register writes after demo.sh) | stock SigmaStar sample drivers/sstar/samples/riu.c (i6e variant), one binary with riu_r/riux32_r/riux32_w (sha256 15016388…), dispatched by name; RIU window from /dev/msys, poked through /dev/mem; the 22-write table: rootfs-customer.md |
| /customer/riux32_r | customer | ELF exec | 5612 | unused | - | same file as riu_w; debug; nothing starts it |
| /customer/riux32_w | customer | ELF exec | 5612 | unused | - | same file as riu_w; debug; nothing starts it |
| /customer/sd_hotplug.sh | customer | data/text | 1666 | used | mdev (mmcblk*) | mount vfat /mnt, SIGUSR1/2 to kjd_lvgl, runs sd_version_recognition |
| /customer/sd_version_recognition | customer | ELF exec | 12632 | used | sd_hotplug.sh | H161X.bin + newer sd_version.txt on SD -> fw_setenv ota_upgrade_status 2; reboot (U-Boot SD upgrade) |
| /customer/set.sh | customer | data/text | 65 | used | demo.sh | LD_LIBRARY_PATH for libmp4v2 |
| /customer/tmp | customer | ELF exec | 16836 | unused | - | factory test: SAR keys, gpio46 (front LED colour), IR remote (18 keys), mmcblk0; prints /customer/version.txt; nothing starts it |
| /customer/usb_otg.sh | customer | data/text | 1158 | used | kjd_lvgl | USB mass-storage switch (kjd_lvgl.md) |
| /customer/version.txt | customer | data/text | 35 | used | kjd_lvgl, sd_version_recognition | version:W11861322604301 |
| /customer/ws.bin | customer | data | 88 | used | prog_uvc, kjd_lvgl | prog_uvc.md |
| /customer/ws.ini | customer | data/text | 591 | used | kjd_lvgl | kjd_lvgl.md |
| /customer/LCM/FPGA_800x480_TTL.ini | customer | data/text | 657 | unused | - | external LCD panel config (hardware not on this camera); no reader |
| /customer/LCM/GM8775C_1920x1080_LVDS.ini | customer | data/text | 2275 | unused | - | external LCD panel config (hardware not on this camera); no reader |
| /customer/LCM/RM68200_720x1280_MIPI.ini | customer | data/text | 7966 | unused | - | external LCD panel config (hardware not on this camera); no reader |
| /customer/LCM/SAT070AT50H18BH_1024x600_TTL.ini | customer | data/text | 660 | unused | - | external LCD panel config (hardware not on this camera); no reader |
| /customer/LCM/SAT070JHH_1024x600_TTL.ini | customer | data/text | 663 | unused | - | external LCD panel config (hardware not on this camera); no reader |
| /customer/lib/libmp4v2.so | customer | ELF .so | 1246476 | used | prog_uvc NEEDED libmp4v2.so.2 | MP4v2 2.0.0; three full copies instead of symlinks |
| /customer/lib/libmp4v2.so.2 | customer | ELF .so | 1246476 | used | prog_uvc | copy |
| /customer/lib/libmp4v2.so.2.0.0 | customer | ELF .so | 1246476 | used | prog_uvc | copy |
| /customer/misc/boot.ini | customer | data/text | 232 | unused | - | build-time sources of the MISC image; runtime reads /misc (no reader of /customer/misc) |
| /customer/misc/config.ini | customer | data/text | 27641 | unused | - | build-time sources of the MISC image; runtime reads /misc (no reader of /customer/misc) |
| /customer/misc/config_cardv.ini | customer | data/text | 32061 | unused | - | build-time sources of the MISC image; runtime reads /misc (no reader of /customer/misc) |
| /customer/misc/sigmastar.jpg | customer | data | 107458 | unused | - | build-time sources of the MISC image; runtime reads /misc (no reader of /customer/misc) |
| /customer/misc/sigmastar1024_600.jpg | customer | data | 49873 | unused | - | build-time sources of the MISC image; runtime reads /misc (no reader of /customer/misc) |
| /customer/misc/sigmastar1024_600_bak.jpg | customer | data | 25797 | unused | - | build-time sources of the MISC image; runtime reads /misc (no reader of /customer/misc) |
| /customer/misc/upgrade.jpg | customer | data | 17833 | unused | - | build-time sources of the MISC image; runtime reads /misc (no reader of /customer/misc) |
