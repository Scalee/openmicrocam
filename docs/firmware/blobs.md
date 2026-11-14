# Vendor binaries — what the camera runs or loads

Every vendor binary of the YIZHAN 4K camera's stock firmware — the NOR image `nor.bin` and its unpacked partitions
`rootfs/`, `miservice/` = `/config`, `customer/` = `/customer`: boot stages, kernel,
kernel modules, user libraries, coprocessor firmware, IQ and calibration data, the configuration tables vendor code loads,
and the apps. For each: what loads it and when, the component document that specifies it, and whether OpenMicroCam (our
firmware) uses it. A map, not a checksum list: every file and flash region here, with its size and sha256 [R], is a row of
[`tools/install/stock-parts.tsv`](../../tools/install/stock-parts.tsv), the one list of the stock firmware's parts.
`tools/install/checkparts.py` compares a camera's dump with that list before the build and the install: a difference in a
part OpenMicroCam depends on stops them, a difference anywhere else is reported quietly, as a sign of another firmware
build.

- Files that are byte-identical under several names are one row here and a row per name in the list. A flash partition's
  row is followed by the image inside it; the list names them `NAME@offset` and `NAME:what` (`IPL@0x0`, `IPL:used`).
- **Loaded by, when** is [S: `components/firmware-image.md` §2 (boot chain) and §3 (inventory by layer)] unless a row
  names another source; "nothing" = nothing on the stock camera loads or runs it.
- **Specified in** names the document in `components/`; "firmware-image.md §x" alone means no document of its own.
- **OpenMicroCam today**, read from the code on 2026-11-12 (`crates/`, `kmods/`, `camera/`, `tools/install/`; each value
  carries its origin mark):
  - **used** — microcamd, our kernel modules, the boot hook, the install or the build needs it: reads, links, loads or
    calls it;
  - **replaced by ours** — ours is loaded or run in its place;
  - **kept as the fallback path** — runs when ours does not: OpenMicroCam not installed or not switched on, a
    `MICROCAM-STOCK` file on the SD card, or our module failing to load (`camera/boot-hook.sh`);
  - **not used** — nothing of ours needs it; the stock boot may still load it;
  - **not established** — the code does not settle it.
- **Left out:** shell scripts and the small text files of `/etc` and `/customer` (`components/rootfs-customer.md`; the
  list carries the few the install depends on), images and fonts (`1600x900.png`, the `customer/misc/*.jpg` splash
  sources), the external LCD panel configs `customer/LCM/*.ini` and `customer/misc/*.ini` (no reader on the camera,
  firmware-image.md §3.7), `terminfo/`, licence texts, the busybox applet symlinks.

## Flash image — `nor.bin`

`nor.bin` is 16 MiB. Region map: `components/firmware-image.md` §1 [S]. A "region" row is the whole partition, the row
below it the image inside it ("used" for a loader: the bytes it fills).

| region (offset) | loaded by, when | specified in | OpenMicroCam today |
|---|---|---|---|
| IPL, region (0x000000) | boot ROM, power-on | [`ipl.md`](components/ipl.md) | used [R]: OpenMicroCam boots through the stock loaders; nothing of ours writes them (`restore-region.sh` refuses to) |
| IPL, used (0x000000) | | | |
| IPL_CUST, region (0x010000) | IPL, after DRAM init | [`ipl.md`](components/ipl.md) | used [R]: as IPL |
| IPL_CUST, used (0x010000) | | | |
| MXPT, region (0x020000) | IPL, U-Boot, kernel (partition table) | firmware-image.md §1.1 | used [R]: the dump tools take the partition offsets from it (`unpack-dump.py`) |
| MXPT, table (0x020000) | | | |
| UBOOT, region (0x030000) | IPL_CUST XZ-unpacks it to 0x23E00000 | [`uboot.md`](components/uboot.md) | used [R]: the boot loader; `dump.sh` and `restore-region.sh` run its commands |
| UBOOT, uImage (0x030000) | | | |
| UBOOT_ENV (0x06F000) | U-Boot; `fw_printenv` / `fw_setenv` | [`uboot.md`](components/uboot.md) §3.2 | used [R]: `install.sh` sets `mma_heap … sz=0xB000000` in `bootargs`, uninstall puts the dump's back |
| KERNEL, region (0x070000) | U-Boot `sf read` + `bootm` | [`kernel-platform.md`](components/kernel-platform.md), [`kernel-drivers.md`](components/kernel-drivers.md) | used [R]: microcamd and our modules run on it; our modules are built for its 4.9.227 module ABI (`prepare-ktree.sh`) |
| KERNEL, uImage (0x070000) | | | |
| rootfs, region (0x280000) | kernel, `root=/dev/mtdblock2` | [`rootfs-customer.md`](components/rootfs-customer.md) | used [R]: its glibc, busybox and `rcS` (rows below) |
| rootfs, squashfs (0x280000) | | | |
| miservice, region (0x480000) | `rcS` mounts it on `/config` | firmware-image.md §3.3–§3.6 | used [R]: its MI modules and libraries (rows below) |
| miservice, squashfs (0x480000) | | | |
| customer, jffs2 (0x880000) | `rcS` mounts it on `/customer` | [`rootfs-customer.md`](components/rootfs-customer.md) | used [R]: the install writes `/customer/microcam/`, `/customer/fast/` and two hook lines into `demo.sh` |
| MISC, firmwarefs (0xE90000) | U-Boot `bootlogo`; `fwfs` mounts it on `/misc` at boot | [`misc-firmwarefs.md`](components/misc-firmwarefs.md) | used [R]: `app-start.sh` hides the boot logo U-Boot shows from it (`fbhide`); [S] `config_tool` reads its `config.ini` (config_tool.md §1) |
| (outside every partition, 0xF30000) | — (all 0xFF) | firmware-image.md §1.2 | not used [R] |

MISC holds `config.ini` (read by U-Boot `bootlogo` and by `config_tool`), `PQ.bin` (unused, format `uboot.md` §6.2), the
splash `sigmastar1024_600.jpg`, `upgrade.jpg` and `ws.txt` (prog_uvc crop modes) [S: firmware-image.md §1.4]; they are not
unpacked in the dump, so the list carries only the region.

The unpacked kernel (the xz payload of the KERNEL uImage, `KERNEL:Image` in the list) is Linux 4.9.227 with its DTB built
in at + 0x421fa0 [R; S: firmware-image.md §3.2].

## Kernel modules — `miservice/modules/4.9.227/` (`/config/modules/4.9.227`)

All are 4.9.227 modules (vermagic `4.9.227 SMP preempt mod_unload ARMv7 thumb2 p2v8` for the ones checked [S:
`usb-gadget.md` intro, `kernel-drivers.md` §5.1]). `demo.sh` insmods them at boot in the order of firmware-image.md §2 step 4.

### Media stack: mhal, MI modules, framebuffer, sensor drivers

MI and mhal build 2022-07-21 (`project_commit.7251436 sdk_commit.80327e1`) [S: firmware-image.md §3.3].

| file | loaded by, when | specified in | OpenMicroCam today |
|---|---|---|---|
| `mhal.ko` | `demo.sh`, boot; the HAL under every MI module | [`mhal-core.md`](components/mhal-core.md); blocks: the `mhal-*`, `isp-1…4`, `ldc-1-stack`, `rgn-gop`, `gfx-ge`, `audio`, `panel-mipitx`, `ipu-host` documents | used [S]: the HAL under every MI module microcamd calls |
| `mi_common.ko` | `demo.sh`, boot | [`mi_sys-2-interfaces.md`](components/mi_sys-2-interfaces.md) | used [R]: microcamd links `libmi_common.so` |
| `mi_sys.ko` | `demo.sh`, boot (`cmdQBufSize=3072 logBufSize=256`); runs `config_tool` | [`mi_sys-1-engine.md`](components/mi_sys-1-engine.md), [`mi_sys-2-interfaces.md`](components/mi_sys-2-interfaces.md) | used [R]: microcamd links `libmi_sys.so` (`crates/mi/src/sys.rs`); our `microcam_canvas` and `microcam_sys` modules call into it |
| `mi_sensor.ko` | `demo.sh`, boot | [`mi-capture.md`](components/mi-capture.md) | used [R]: microcamd links `libmi_sensor.so` and calls it (`crates/mi/src/sensor.rs`) |
| `mi_vif.ko` | `demo.sh`, boot | [`mi-capture.md`](components/mi-capture.md) | used [R]: microcamd links `libmi_vif.so` and calls it (`crates/mi/src/vif.rs`) |
| `mi_isp.ko` | `demo.sh`, boot | [`mi-capture.md`](components/mi-capture.md) | used [R]: microcamd links `libmi_isp.so` and calls it (`crates/mi/src/isp.rs`) |
| `mi_scl.ko` | `demo.sh`, boot | [`mi-capture.md`](components/mi-capture.md) | used [R]: microcamd links `libmi_scl.so` and calls it (`crates/mi/src/scl.rs`) |
| `mi_rgn.ko` | `demo.sh`, boot; symbol dependency of mi_scl | [`rgn-gop.md`](components/rgn-gop.md) | replaced by ours [R]: `microcam_rgn.ko` loads in its place when `rgn.ok` is set (`camera/boot-hook.sh`); kept as the fallback path |
| `mi_gfx.ko` | `demo.sh`, boot; dependency of mi_disp | [`gfx-ge.md`](components/gfx-ge.md) | used [R]: microcamd links `libmi_gfx.so` and calls it (`crates/mi/src/gfx.rs`) |
| `mi_venc.ko` | `demo.sh`, boot | [`mi_venc.md`](components/mi_venc.md) | used [R]: microcamd links `libmi_venc.so` and calls it (`crates/mi/src/venc.rs`) |
| `mi_disp.ko` | `demo.sh`, boot | [`mi_disp-mi_hdmi.md`](components/mi_disp-mi_hdmi.md) | used [R]: microcamd links `libmi_disp.so` and calls it (`crates/mi/src/disp.rs`) |
| `mi_hdmi.ko` | `demo.sh`, boot | [`mi_disp-mi_hdmi.md`](components/mi_disp-mi_hdmi.md) | used [R]: microcamd links `libmi_hdmi.so` and calls it (`crates/mi/src/hdmi.rs`) |
| `mi_jpd.ko` | `demo.sh`, boot; photo viewer | [`mi_jpd-mi_vdec.md`](components/mi_jpd-mi_vdec.md) | used [R]: microcamd links `libmi_jpd.so` and decodes JPEG pictures with it (`crates/mi/src/jpd.rs`) |
| `mi_vdec.ko` | `demo.sh`, boot; MP4 player | [`mi_jpd-mi_vdec.md`](components/mi_jpd-mi_vdec.md) | used [R]: microcamd links `libmi_vdec.so`; its player decodes recordings with it (`crates/mi/src/vdec.rs`) |
| `mi_ldc.ko` | `demo.sh`, boot; nothing uses it (prog_uvc runs without `-L`) | [`ldc-1-stack.md`](components/ldc-1-stack.md), [`ldc-2-eptz-lib.md`](components/ldc-2-eptz-lib.md) | not used [R] |
| `mi_ai.ko` | `demo.sh`, boot; nothing uses it | [`audio.md`](components/audio.md) | not used [R] |
| `mi_ao.ko` | `demo.sh`, boot; nothing uses it | [`audio.md`](components/audio.md) | not used [R] |
| `mi_vdisp.ko` | `demo.sh`, boot; nothing uses it | [`mi_vdisp-mi_shadow.md`](components/mi_vdisp-mi_shadow.md) | not used [R]: the boot hook loads our `microcam_sys` and `microcam_canvas` right after it, nothing calls it |
| `mi_shadow.ko` | `demo.sh`, boot; nothing uses it | [`mi_vdisp-mi_shadow.md`](components/mi_vdisp-mi_shadow.md) | not used [R] |
| `mi_ipu.ko` | `demo.sh`, boot; nothing uses it | [`ipu-host.md`](components/ipu-host.md) | not used [R] |
| `mi_mipitx.ko` | `demo.sh`, boot; nothing uses it | [`panel-mipitx.md`](components/panel-mipitx.md) | not used [R] |
| `mi_panel.ko` | `demo.sh`, boot; nothing uses it | [`panel-mipitx.md`](components/panel-mipitx.md) | not used [R] |
| `fbdev.ko` | `demo.sh`, boot (`default_config_path_file=/config/fbdev.ini`); `/dev/fb0` for kjd_lvgl | [`rgn-gop.md`](components/rgn-gop.md) | used [R]: `fbhide` hides the logo window through `/dev/fb0`, then `app-start.sh` unloads it |
| `gc8613_MIPI.ko` | `demo.sh`, boot (`chmap=1 lane_num=4`) | [`sensor-gc8613.md`](components/sensor-gc8613.md) | replaced by ours [R]: `drv_ms_cus_gc8613_MIPI.ko` when `/customer/fast/enabled` is set (`boot-hook.sh`); kept as the fallback path, also when ours does not load; `kmods/gc8613/tables/stock.txt` holds the register values it programs |
| `imx307_MIPI.ko` | never loaded | out of scope (sensor not on this camera); named in [`mhal-csi-snr.md`](components/mhal-csi-snr.md) | not used [R] |

### USB device stack

| file | loaded by, when | specified in | OpenMicroCam today |
|---|---|---|---|
| `g_webcam.ko` | `prog_uvc` insmods it at its start | [`usb-gadget.md`](components/usb-gadget.md) §4 | replaced by ours [R] when our build is installed (`app-start.sh`); otherwise used [R]: microcamd recognises it by its `srcversion` (`crates/microcam-core/src/formats.rs`) |
| `usb_f_uvc.ko` | `demo.sh`, boot | [`usb-gadget.md`](components/usb-gadget.md) §3 | used [R]: the UVC function of our gadget (`g_webcam` depends on it) |
| `udc-msb250x.ko` | `demo.sh`, boot | [`usb-gadget.md`](components/usb-gadget.md) §2 | replaced by ours [R]: our build when `udc.ok` is set (`camera/udc-insmod.sh`); kept as the fallback path, also when ours does not load |
| `udc-core.ko` | `demo.sh`, boot | stock Linux 4.9; firmware-image.md §3.4 | used [R]: under both USB controller drivers and our gadget |
| `libcomposite.ko` | `demo.sh`, boot | stock Linux 4.9; firmware-image.md §3.4 | used [R]: our gadget is a composite one (`g_webcam` depends on it) |
| `u_ether.ko` | `demo.sh`, boot | stock Linux 4.9; firmware-image.md §3.4 | used [R]: the USB network of our gadget (`rndis_function_enable=1`, `app-start.sh`) |
| `usb_f_rndis.ko` | `demo.sh`, boot; RNDIS `usb0` | stock Linux 4.9; firmware-image.md §3.4 | used [R]: the USB network of our gadget (`rndis_function_enable=1`, `app-start.sh`) |
| `usb-common.ko` | `demo.sh`, boot (twice, the second fails) | stock Linux 4.9; firmware-image.md §3.4 | not established |
| `videobuf2-core.ko` | `demo.sh`, boot | stock Linux 4.9; firmware-image.md §3.4 | used [R]: `usb_f_uvc` depends on it |
| `videobuf2-memops.ko` | `demo.sh`, boot | stock Linux 4.9; firmware-image.md §3.4 | used [I]: under `videobuf2-vmalloc` |
| `videobuf2-v4l2.ko` | `demo.sh`, boot | stock Linux 4.9; firmware-image.md §3.4 | used [R]: `usb_f_uvc` depends on it |
| `videobuf2-vmalloc.ko` | `demo.sh`, boot | stock Linux 4.9; firmware-image.md §3.4 | used [R]: `usb_f_uvc` depends on it |
| `g_mass_storage.ko` | `kjd_lvgl` through `usb_otg.sh`, USB-disk mode | stock Linux 4.9; firmware-image.md §3.4, [`usb-gadget.md`](components/usb-gadget.md) | not used [R]: the card's disk is a function of our own gadget |
| `usb_f_mass_storage.ko` | `kjd_lvgl` through `usb_otg.sh`, USB-disk mode | stock Linux 4.9; firmware-image.md §3.4 | used [R]: `app-start.sh` loads it for the card's disk in our gadget |

### SD card, file systems and the rest

| file | loaded by, when | specified in | OpenMicroCam today |
|---|---|---|---|
| `kdrv_sdmmc.ko` | `demo.sh`, boot; the SD host | [`kernel-drivers.md`](components/kernel-drivers.md) §5.1 | used [R]: microcamd mounts the SD card (`crates/microcamd/src/card.rs`) |
| `mmc_core.ko` | `demo.sh`, boot | stock Linux 4.9; firmware-image.md §3.4 | used [R]: microcamd mounts the SD card (`card.rs`) |
| `mmc_block.ko` | `demo.sh`, boot | stock Linux 4.9; firmware-image.md §3.4 | used [R]: microcamd mounts the SD card (`card.rs`) |
| `fat.ko` | `demo.sh`, boot | stock Linux 4.9; firmware-image.md §3.4 | used [R]: microcamd mounts the card as vfat (`card.rs`) |
| `vfat.ko` | `demo.sh`, boot; `sd_hotplug.sh` mounts the card `-t vfat` | stock Linux 4.9; firmware-image.md §3.4 | used [R]: microcamd mounts the card as vfat (`card.rs`) |
| `msdos.ko` | `demo.sh`, boot; nothing uses it | stock Linux 4.9; firmware-image.md §3.4 | not used [R] |
| `ntfs.ko` | `demo.sh`, boot; nothing uses it | stock Linux 4.9; firmware-image.md §3.4 | not used [R] |
| `nls_utf8.ko` | `demo.sh`, boot; nothing uses it | stock Linux 4.9; firmware-image.md §3.4 | not established: whether the vfat mount's default character set needs it is not read |
| `cifs.ko` | `demo.sh`, boot; nothing uses it | stock Linux 4.9; firmware-image.md §3.4 | not used [R] |
| `nfs.ko` | `demo.sh`, boot; nothing uses it | stock Linux 4.9; firmware-image.md §3.4 | not used [R] |
| `nfsv2.ko` | `demo.sh`, boot; nothing uses it | stock Linux 4.9; firmware-image.md §3.4 | not used [R] |
| `nfsv3.ko` | `demo.sh`, boot; nothing uses it | stock Linux 4.9; firmware-image.md §3.4 | not used [R] |
| `lockd.ko` | `demo.sh`, boot; nothing uses it | stock Linux 4.9; firmware-image.md §3.4 | not used [R] |
| `sunrpc.ko` | `demo.sh`, boot; nothing uses it | stock Linux 4.9; firmware-image.md §3.4 | not used [R] |
| `grace.ko` | `demo.sh`, boot; nothing uses it | stock Linux 4.9; firmware-image.md §3.4 | not used [R] |
| `usbcore.ko` | `demo.sh`, boot; nothing uses it (USB host stack) | stock Linux 4.9; firmware-image.md §3.4 | not used [R] |
| `ehci-hcd.ko` | `demo.sh`, boot; nothing uses it (SigmaStar EHCI glue) | firmware-image.md §3.4 | not used [R] |
| `usb-storage.ko` | `demo.sh`, boot; nothing uses it | stock Linux 4.9; firmware-image.md §3.4 | not used [R] |
| `usbhid.ko` | `demo.sh`, boot; nothing uses it | stock Linux 4.9; firmware-image.md §3.4 | not used [R] |
| `mdrv-sata-host.ko` | `demo.sh`, boot; nothing uses it (no SATA on the board) | firmware-image.md §3.4 | not used [R] |
| `media.ko` | `demo.sh`, boot; rejected (built into the kernel) | stock Linux 4.9; firmware-image.md §3.4 | not used [R] |
| `videodev.ko` | `demo.sh`, boot; rejected (built into the kernel) | stock Linux 4.9; firmware-image.md §3.4 | not used [R] |
| `v4l2-common.ko` | `demo.sh`, boot; rejected (built into the kernel) | stock Linux 4.9; firmware-image.md §3.4 | not used [R] |

## User libraries — `miservice/lib/` (`/config/lib`)

Nothing on the stock camera loads a `/config/lib` library at run time: `prog_uvc` carries the code of the "static copy" ones
statically linked, `kjd_lvgl` needs only libc / libm / libpthread [S: firmware-image.md §2, §3.5]. All armhf, GCC 9.1.0,
`.symtab` stripped, dynamic exports kept [S: firmware-image.md §3.5]. microcamd links 16 of them dynamically and loads them from
`/config/lib` at its start (the list is in [`crates/microcamd/build.rs`](../../crates/microcamd/build.rs) [R]).

| file | loaded by, when | specified in | OpenMicroCam today |
|---|---|---|---|
| `libmi_common.so` | nothing; prog_uvc carries a static copy | [`mi-user-abi.md`](components/mi-user-abi.md), [`mi_sys-2-interfaces.md`](components/mi_sys-2-interfaces.md) | used [R]: microcamd links it (`crates/microcamd/build.rs`) |
| `libmi_sys.so` | nothing; prog_uvc carries a static copy | [`mi-user-abi.md`](components/mi-user-abi.md), [`mi_sys-2-interfaces.md`](components/mi_sys-2-interfaces.md) | used [R]: microcamd links it (`crates/microcamd/build.rs`) |
| `libmi_sensor.so` | nothing; prog_uvc carries a static copy | [`mi-user-abi.md`](components/mi-user-abi.md), [`mi-capture.md`](components/mi-capture.md) | used [R]: microcamd links it (`crates/microcamd/build.rs`) |
| `libmi_vif.so` | nothing; prog_uvc carries a static copy | [`mi-user-abi.md`](components/mi-user-abi.md), [`mi-capture.md`](components/mi-capture.md) | used [R]: microcamd links it (`crates/microcamd/build.rs`) |
| `libmi_isp.so` | nothing; prog_uvc carries a static copy | [`mi-user-abi.md`](components/mi-user-abi.md), [`mi-capture.md`](components/mi-capture.md) | used [R]: microcamd links it (`crates/microcamd/build.rs`) |
| `libmi_scl.so` | nothing; prog_uvc carries a static copy | [`mi-user-abi.md`](components/mi-user-abi.md), [`mi-capture.md`](components/mi-capture.md) | used [R]: microcamd links it (`crates/microcamd/build.rs`) |
| `libmi_venc.so` | nothing; prog_uvc carries a static copy | [`mi-user-abi.md`](components/mi-user-abi.md), [`mi_venc.md`](components/mi_venc.md) | used [R]: microcamd links it (`crates/microcamd/build.rs`) |
| `libmi_disp.so` | nothing; prog_uvc carries a static copy | [`mi-user-abi.md`](components/mi-user-abi.md), [`mi_disp-mi_hdmi.md`](components/mi_disp-mi_hdmi.md) | used [R]: microcamd links it (`crates/microcamd/build.rs`) |
| `libmi_hdmi.so` | nothing; prog_uvc carries a static copy | [`mi-user-abi.md`](components/mi-user-abi.md), [`mi_disp-mi_hdmi.md`](components/mi_disp-mi_hdmi.md) | used [R]: microcamd links it (`crates/microcamd/build.rs`) |
| `libmi_rgn.so` | nothing | [`mi-user-abi.md`](components/mi-user-abi.md), [`rgn-gop.md`](components/rgn-gop.md) | used [R]: microcamd links it (`crates/microcamd/build.rs`) |
| `libmi_gfx.so` | nothing | [`mi-user-abi.md`](components/mi-user-abi.md), [`gfx-ge.md`](components/gfx-ge.md) | used [R]: microcamd links it (`crates/microcamd/build.rs`) |
| `libcam_os_wrapper.so` | nothing; prog_uvc carries a static copy | [`mi-user-abi.md`](components/mi-user-abi.md) §7 | used [R]: microcamd links it (`crates/microcamd/build.rs`) |
| `libcam_fs_wrapper.so` | nothing; prog_uvc carries a static copy | [`mi-user-abi.md`](components/mi-user-abi.md) | not used [R] |
| `libispalgo.so` | nothing; prog_uvc carries a static copy | [`isp-5-3a.md`](components/isp-5-3a.md) | used [R]: microcamd links it (`crates/microcamd/build.rs`) |
| `libcus3a.so` | nothing; prog_uvc carries a static copy | [`isp-6-userspace.md`](components/isp-6-userspace.md) | used [R]: microcamd links it and waits for `CUS3A_GetAlgoStatus` (`crates/mi/src/isp.rs`) |
| `libmi_jpd.so` | nothing; prog_uvc carries a static copy | [`mi-user-abi.md`](components/mi-user-abi.md), [`mi_jpd-mi_vdec.md`](components/mi_jpd-mi_vdec.md) | used [R]: microcamd links it (`crates/microcamd/build.rs`) |
| `libmi_vdec.so` | nothing; prog_uvc carries a static copy | [`mi-user-abi.md`](components/mi-user-abi.md), [`mi_jpd-mi_vdec.md`](components/mi_jpd-mi_vdec.md) | used [R]: microcamd links it (`crates/microcamd/build.rs`) |
| `libmi_iqserver.so` | nothing; prog_uvc carries a static copy (IQTool server, `-q`) | [`isp-6-userspace.md`](components/isp-6-userspace.md) | not used [R] |
| `libfbc_decode.so` | nothing; prog_uvc carries a static copy (reached from the IQ server only) | [`isp-6-userspace.md`](components/isp-6-userspace.md) | not used [R] |
| `libsigma3a.so` | nothing | [`isp-6-userspace.md`](components/isp-6-userspace.md), [`isp-5-3a.md`](components/isp-5-3a.md) | not used [R] |
| `libmi_ldc.so` | nothing | [`mi-user-abi.md`](components/mi-user-abi.md), [`ldc-1-stack.md`](components/ldc-1-stack.md) | used [R]: only the C testers link it (`tools/cam/ctester.sh`) |
| `libmi_ai.so` | nothing | [`mi-user-abi.md`](components/mi-user-abi.md), [`audio.md`](components/audio.md) | not used [R] |
| `libmi_ao.so` | nothing | [`mi-user-abi.md`](components/mi-user-abi.md), [`audio.md`](components/audio.md) | not used [R] |
| `libmi_vdisp.so` | nothing | [`mi-user-abi.md`](components/mi-user-abi.md), [`mi_vdisp-mi_shadow.md`](components/mi_vdisp-mi_shadow.md) | used [R]: only the C testers link it (`tools/cam/ctester.sh`) |
| `libmi_shadow.so` | nothing | [`mi-user-abi.md`](components/mi-user-abi.md), [`mi_vdisp-mi_shadow.md`](components/mi_vdisp-mi_shadow.md) | not used [R] |
| `libmi_ipu.so` | nothing | [`mi-user-abi.md`](components/mi-user-abi.md), [`ipu-host.md`](components/ipu-host.md) | not used [R] |
| `libmi_mipitx.so` | nothing | [`mi-user-abi.md`](components/mi-user-abi.md), [`panel-mipitx.md`](components/panel-mipitx.md) | not used [R] |
| `libmi_panel.so` | nothing | [`mi-user-abi.md`](components/mi-user-abi.md), [`panel-mipitx.md`](components/panel-mipitx.md) | not used [R] |
| `libmi_ive.so` | nothing | [`ive-video-analytics.md`](components/ive-video-analytics.md) §1 | not used [R] |
| `libMD_LINUX.so` | nothing | [`ive-video-analytics.md`](components/ive-video-analytics.md) | not used [R] |
| `libOD_LINUX.so` | nothing | [`ive-video-analytics.md`](components/ive-video-analytics.md) | not used [R] |
| `libVG_LINUX.so` | nothing | [`ive-video-analytics.md`](components/ive-video-analytics.md) | not used [R] |
| `libAEC_LINUX.so` | nothing | [`audio-algorithms.md`](components/audio-algorithms.md) | not used [R] |
| `libAED_LINUX.so` | nothing | [`audio-algorithms.md`](components/audio-algorithms.md) | not used [R] |
| `libAPC_LINUX.so` | nothing | [`audio-algorithms.md`](components/audio-algorithms.md) | not used [R] |
| `libBF_LINUX.so` | nothing | [`audio-algorithms.md`](components/audio-algorithms.md) | not used [R] |
| `libSRC_LINUX.so` | nothing | [`audio-algorithms.md`](components/audio-algorithms.md) | not used [R] |
| `libSSL_LINUX.so` | nothing | [`audio-algorithms.md`](components/audio-algorithms.md) | not used [R] |
| `libmi_divp.so` | nothing | [`sdk-orphan-libs.md`](components/sdk-orphan-libs.md) §3 | not used [R] |
| `libmi_gyro.so` | nothing | [`sdk-orphan-libs.md`](components/sdk-orphan-libs.md) §4 | not used [R] |
| `libmi_vpe.so` | nothing | [`sdk-orphan-libs.md`](components/sdk-orphan-libs.md) §5 | not used [R] |
| `libmi_sed.so` | nothing | [`sdk-orphan-libs.md`](components/sdk-orphan-libs.md) §6 | not used [R] |
| `libmi_vdf.so` | nothing | [`sdk-orphan-libs.md`](components/sdk-orphan-libs.md) §7 | not used [R] |
| `libmi_wlan.so` | nothing | [`sdk-orphan-libs.md`](components/sdk-orphan-libs.md) §8 | not used [R] |
| `libcjson.so` | nothing | [`sdk-orphan-libs.md`](components/sdk-orphan-libs.md) §9 | not used [R] |
| `libg711.so` | nothing | [`sdk-orphan-libs.md`](components/sdk-orphan-libs.md) §9 | not used [R] |
| `libg726.so` | nothing | [`sdk-orphan-libs.md`](components/sdk-orphan-libs.md) §9 | not used [R] |

`AEC_LICENSE_{1,2}.txt`, `APC_LICENSE_{1,2}.txt` beside them are licence texts (Ooura FFT, WebRTC BSD-3) [S: firmware-image.md §3.5].

## Coprocessor firmware, IQ data, app data and configuration tables

| file | loaded by, when | specified in | OpenMicroCam today |
|---|---|---|---|
| `miservice/venc_fw/chagall.bin` | `mhal.ko`, for the encoder (`VPU_InitWithBitcode`) | [`chagall-fw.md`](components/chagall-fw.md), [`mhal-vcodec.md`](components/mhal-vcodec.md) | used [I]: microcamd encodes H.264 / H.265 [R: `crates/mi/src/venc.rs`], and mhal loads this firmware for those codecs [S] |
| `miservice/dla/ipu_firmware.bin` | nothing (mi_ipu is loaded, no app opens it) | [`ipu-firmware-1-runtime-model.md`](components/ipu-firmware-1-runtime-model.md), [`ipu-firmware-2-engines.md`](components/ipu-firmware-2-engines.md), [`ipu-host.md`](components/ipu-host.md) | not used [R] |
| `miservice/iqfile/imx307_iqfile.bin` | mhal's 3A init at ISP start, as `iqfile0.bin` (symlinks `iqfile0…3.bin` point to it); libcus3a from `/config/iqfile` | [`isp-4-iq-engine.md`](components/isp-4-iq-engine.md) §13 | used [I]: microcamd starts the ISP [R], and mhal loads this file at every ISP start [S] |
| `miservice/iqfile/imx307_api.bin` | nothing | [`isp-3-iq-api.md`](components/isp-3-iq-api.md) | not used [R] |
| `customer/iq_file_gc8613.bin` | `prog_uvc -i`, at app start | [`isp-3-iq-api.md`](components/isp-3-iq-api.md) §6 | used [R]: `install.py` makes microcamd's copy from it with the defect-pixel blending off (`iq-dpc-noblend.py`); kept as the fallback path as it is |
| `customer/ws.bin` | prog_uvc and kjd_lvgl: saved menu settings, written by the menu | [`prog_uvc.md`](components/prog_uvc.md), [`kjd_lvgl.md`](components/kjd_lvgl.md) | not used [R]: microcamd keeps its settings in `/customer/microcam/settings.json` and neither reads nor writes this file; the installer leaves it on the camera (`tools/install/install.py`) |
| `customer/default_ws.bin` | prog_uvc and kjd_lvgl: factory settings | [`prog_uvc.md`](components/prog_uvc.md), [`kjd_lvgl.md`](components/kjd_lvgl.md) | not used [R]: microcamd's defaults are its own (`crates/microcam-core/src/controls.rs`) |
| `miservice/iqfile/isp_api.xml` | prog_uvc's static IQ server (`-q`), on an IQTool request | [`isp-6-userspace.md`](components/isp-6-userspace.md) | not used [R] |
| `miservice/mmap.ini` | `config_tool` (mi_sys helper, at mi_sys load); fbdev, mi_venc | [`config_tool.md`](components/config_tool.md) §2 | used [I]: mi_sys loads under microcamd too [R: `demo.sh`], and its helper reads this file [S] |
| `miservice/config.ini` | nothing (identical to MISC `config.ini`, which is read) | [`config_tool.md`](components/config_tool.md) | not used [S]: `config_tool` reads the MISC copy |
| `miservice/fbdev.ini` | `fbdev.ko`, at load | [`rgn-gop.md`](components/rgn-gop.md) | used [R]: fbdev loads with it (`demo.sh`) until `app-start.sh` unloads fbdev |
| `miservice/board.ini` | nothing | [`panel-mipitx.md`](components/panel-mipitx.md) | not used [R] |
| `miservice/model/Customer.ini` | nothing | [`panel-mipitx.md`](components/panel-mipitx.md) | not used [R] |
| `miservice/model/LCM.ini` | nothing | [`panel-mipitx.md`](components/panel-mipitx.md) | not used [R] |

## Applications and tools

| file | loaded by, when | specified in | OpenMicroCam today |
|---|---|---|---|
| `customer/prog_uvc` | `demo.sh`, boot (`-H -A 0 -a 0 -q -i /customer/iq_file_gc8613.bin`) | [`prog_uvc.md`](components/prog_uvc.md) | replaced by ours [R]: microcamd starts where it would (`boot-hook.sh`); kept as the fallback path |
| `customer/kjd_lvgl` | `demo.sh`, boot, 3 s after prog_uvc | [`kjd_lvgl.md`](components/kjd_lvgl.md) | replaced by ours [R]: microcamd draws its own menu and `demo.sh` ends before the line that starts kjd_lvgl; kept as the fallback path |
| `customer/lib/libmp4v2.so`, `customer/lib/libmp4v2.so.2`, `customer/lib/libmp4v2.so.2.0.0` | prog_uvc (NEEDED, `LD_LIBRARY_PATH` from `set.sh`) | [`prog_uvc.md`](components/prog_uvc.md) §1.4 | kept as the fallback path [R]: only prog_uvc needs it |
| `customer/sd_version_recognition` | `sd_hotplug.sh`, on SD insert | [`rootfs-customer.md`](components/rootfs-customer.md) | kept as the fallback path [R]: under microcamd `app-start.sh` mounts an empty script over `sd_hotplug.sh` |
| `customer/riu_w`, `customer/riu_r`, `customer/riux32_r`, `customer/riux32_w` | `rcS`, boot (22 writes as `riu_w`); the other names nothing | [`rootfs-customer.md`](components/rootfs-customer.md) §5, §6 | not established: `rcS` still runs `riu_w` at every boot; whether microcamd needs those writes is not known |
| `customer/i2c_read_write` | nothing | [`rootfs-customer.md`](components/rootfs-customer.md) §7 | not used [R] |
| `customer/tmp` | nothing (factory test) | [`rootfs-customer.md`](components/rootfs-customer.md) §8 | not used [R] |
| `customer/htop` | nothing | firmware-image.md §3.7 | not used [R] |
| `customer/iperf3` | nothing (needs uClibc, absent) | firmware-image.md §3.7 | not used [R] |
| `miservice/config_tool` | mi_sys, as a usermode helper at its load (also `dump_config`, `dump_mmap`) | [`config_tool.md`](components/config_tool.md) | used [I]: mi_sys loads under microcamd too [R: `demo.sh`] and runs it [S] |
| `rootfs/bin/busybox` | the kernel (`init=/linuxrc`), every script | [`rootfs-customer.md`](components/rootfs-customer.md); firmware-image.md §3.8 | used [R]: the boot hook and the install scripts run in its shell |
| `rootfs/bin/fwfs` | `rcS`, boot: mounts MISC on `/misc` | [`misc-firmwarefs.md`](components/misc-firmwarefs.md) | not used [R]: nothing of ours reads `/misc` |
| `rootfs/etc/fw_printenv` | `demo.sh`, prog_uvc, `sd_version_recognition` (also as `fw_setenv`) | [`rootfs-customer.md`](components/rootfs-customer.md) §2.7 | used [R]: `install.sh` reads and writes `bootargs` with it (as `fw_printenv`, `fw_setenv`) |
| `rootfs/bin/scp` | nothing | firmware-image.md §3.8 | not used [R] |

## rootfs libraries — `rootfs/lib/`

glibc 2.30 armhf, libstdc++ 6.0.26, libgcc_s, zlib, libfuse 2.9.9 [S: firmware-image.md §3.8]; the `*.so.N` names are
symlinks to these files [R].

| file | loaded by, when | specified in | OpenMicroCam today |
|---|---|---|---|
| `ld-2.30.so` | the kernel, for every dynamic program | firmware-image.md §3.8 | used [R]: microcamd needs it (the built binary's NEEDED); the camera-side C tools link it |
| `libc-2.30.so` | the dynamic linker, for every program that needs it | firmware-image.md §3.8 | used [R]: microcamd needs it (the built binary's NEEDED); the camera-side C tools link it |
| `libm-2.30.so` | the dynamic linker, for every program that needs it | firmware-image.md §3.8 | used [R]: microcamd needs it (the built binary's NEEDED) |
| `libpthread-2.30.so` | the dynamic linker, for every program that needs it | firmware-image.md §3.8 | used [R]: microcamd needs it (the built binary's NEEDED) |
| `libdl-2.30.so` | the dynamic linker, for every program that needs it | firmware-image.md §3.8 | used [R]: microcamd needs it (the built binary's NEEDED) |
| `librt-2.30.so` | the dynamic linker, for every program that needs it | firmware-image.md §3.8 | not used [R]: the built microcamd does not need it, though `build.rs` passes `-lrt` |
| `libresolv-2.30.so` | the dynamic linker, for every program that needs it | firmware-image.md §3.8 | not used [R] |
| `libutil.so`, `libutil.so.1` | the dynamic linker, for every program that needs it | firmware-image.md §3.8 | not used [R] |
| `libcrypt.so`, `libcrypt.so.1` | the dynamic linker, for every program that needs it | firmware-image.md §3.8 | not used [R] |
| `libnss_compat.so.2` | the dynamic linker, for every program that needs it | firmware-image.md §3.8 | not established: glibc may load it for the login's user lookup; not read |
| `libnss_files.so.2` | the dynamic linker, for every program that needs it | firmware-image.md §3.8 | not established: glibc may load it for the login's user lookup; not read |
| `libpcprofile.so` | the dynamic linker, for every program that needs it | firmware-image.md §3.8 | not used [R] |
| `libstdc++.so.6.0.26` | the dynamic linker, for every program that needs it | firmware-image.md §3.8 | not used [R] |
| `libgcc_s.so.1` | the dynamic linker, for every program that needs it | firmware-image.md §3.8 | not established: the built microcamd does not need it; glibc may load it at run time |
| `libz.so`, `libz.so.1` | the dynamic linker, for every program that needs it | firmware-image.md §3.8 | not used [R] |
| `libfuse.so.2.9.9` | `fwfs` | [`misc-firmwarefs.md`](components/misc-firmwarefs.md); firmware-image.md §3.8 | not used [R]: only `fwfs` needs it |
