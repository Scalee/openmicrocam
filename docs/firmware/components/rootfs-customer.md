# rootfs-customer — the boot scripts of `/etc` and `/customer`, the boot-time `riu_w` writes and the small `/customer` tools
<!-- ev:rootfs-customer -->

Between the kernel and the two apps the camera runs a few shell scripts and small programs from the rootfs (`/etc`,
squashfs, read-only) and from the ODM partition `/customer` (jffs2, read-write) [R]. busybox `init` runs `/etc/init.d/rcS`;
rcS mounts the filesystems, starts mdev, the MISC daemon `fwfs` and `telnetd`, runs `/customer/demo.sh` and then writes 22
RIU registers with `/customer/riu_w`; demo.sh loads the kernel modules, configures the network, rewrites one U-Boot
environment variable and starts `prog_uvc` and `kjd_lvgl` in the background. mdev runs `/customer/sd_hotplug.sh` when an SD
card comes or goes; that script mounts the card, signals kjd_lvgl and runs `/customer/sd_version_recognition`, which can set
an upgrade marker in the U-Boot environment and reboot. `/customer/usb_otg.sh` is kjd_lvgl's helper for the switch to USB
mass storage. The other small tools in `/customer` — `riu_r`, `riux32_r`, `riux32_w` (the same file as `riu_w`),
`i2c_read_write` and the factory test `tmp` — are started by nothing on the camera (unused) [R].

This document owns the content of the `/etc` configuration files, rcS, demo.sh and its module list, the hotplug and USB
scripts, the boot-time `riu_w` write list, the Linux-side writers of the U-Boot environment, the files and markers the apps
expect, and the three tools. Elsewhere: the partition map and the file inventory (sizes, versions, use of every file) —
`firmware-image.md` §1, §3; the U-Boot side of the upgrade flow (`sdstar`, the autoboot hook, `ufu`, the ODM patch in `reset`) and
the environment's storage and variables — `uboot.md` §2.2, §3, §4; the MISC firmwarefs and the `fwfs` daemon with its
command line — `misc-firmwarefs.md` §3, §4; `config_tool`, which mi_sys runs at insmod — `config_tool.md` §1.3; RIU addressing
and the `/dev/msys` ioctl set — `kernel-platform.md` §3.1, §10.1; the meaning of the MIU registers `riu_w` writes —
`kernel-platform.md` §12.3, §12.4; the meaning of the MOP registers — `mhal-disp.md` §11.4; GPIO46, the SAR button ladder and
the board wiring — `kernel-platform.md` §6.3; the SAR driver and the IR receiver with its keymap — `kernel-drivers.md` §3.1,
§3.2; the SD host and which slot carries the socket — `kernel-drivers.md` §5.1; the USB gadget modules and `g_webcam` —
`usb-gadget.md`; the sensor driver `gc8613_MIPI.ko` and its parameters — `sensor-gc8613.md`; what prog_uvc and kjd_lvgl do
with the files below — `prog_uvc.md`, `kjd_lvgl.md` §3.

**Tags.** [R] read from a file of the dump, a binary or its disassembly, a boot log, a public source or a
photo; [S] stated by a source (a vendor FAQ); [I] inference. A heading tagged [R] covers every statement under it
unless marked otherwise.

## 1. Boot order [R]

| # | who | what |
|---|---|---|
| 1 | U-Boot `main_loop` | `run_command("sdstar")` unconditionally (SD script upgrade), the autoboot hook, then `bootcmd` (`uboot.md` §2.2, §4) |
| 2 | kernel | `root=/dev/mtdblock2 rootfstype=squashfs ro init=/linuxrc` (busybox); `devtmpfs_mount` is in kallsyms and the squashfs `/dev` is empty → `/dev` comes from the devtmpfs automount [I] |
| 3 | busybox init | `/etc/inittab` (§2.1) |
| 4 | `/etc/init.d/rcS` | mounts, mdev, fwfs, `telnetd &`, `/customer/demo.sh` (synchronous), 22 `riu_w` writes (§2.3, §5) |
| 5 | `demo.sh` | modules, network, one environment write, `prog_uvc … &`, `sleep 3`, `kjd_lvgl &` (§3) |
| 6 | prog_uvc | `insmod g_webcam.ko …` (only when the VIF counts more than 5 interrupts and `/tmp/sdcard_info` is absent, `prog_uvc.md` §2.2), `ifconfig usb0`, route (§3.3) |
| 7 | mdev hotplug | `mmcblk[0-9]` → `/customer/sd_hotplug.sh` → mount `/mnt`, signal kjd_lvgl, `sd_version_recognition` (§4) |

With an SD card in the slot at power-on, mdev mounts the card and writes `/tmp/sdcard_info` before prog_uvc reaches its
gadget check, so the camera comes up without the USB camera / RNDIS function [I from the boot order].

## 2. rootfs `/etc` [R]

### 2.1 inittab
Two entries: at `sysinit` it runs `/etc/init.d/rcS`; with `respawn` it runs the login program, written with a `~/` prefix
(`~/bin/login`). `~` is a shell metacharacter for busybox init, so that entry runs through `/bin/sh -c "exec …"` with
`HOME=/` → `/bin/login` on the console (`(none) login:` in every UART log) [R log; mechanism I]. There is no `ttyS0` getty
entry and no `ctrlaltdel` / `shutdown` action.

### 2.2 fstab
Used by `mount -a`: `proc /proc proc`, `sysfs /sys sysfs`, `tmpfs /tmp tmpfs`, `var /var tmpfs`, `vendor /vendor tmpfs`,
`none /sys/kernel/debug debugfs`; the `devpts` line is commented out.

### 2.3 rcS
A `/bin/sh` script, run by init with working directory `/`. Its steps in order, with their effect:

| step | effect |
|---|---|
| mount the fstab | §2.2 |
| create `/dev/shm`, `/dev/pts`; mount devpts by fstab | **fails** at every boot (`mount: can't find devpts in /etc/fstab`) |
| make `/sbin/mdev` the kernel's hotplug agent | the SD hook of §2.4 depends on it |
| `mdev -s` | |
| `sysctl -p` | `/etc/sysctl.conf` (§2.5) |
| mount sysfs on `/sys` | **fails** (already mounted) |
| mount a tmpfs on `/dev` | replaces the devtmpfs view with an empty tmpfs |
| mount debugfs on `/sys/kernel/debug` | **fails** (busy, already mounted by fstab) |
| create `/var/lock`; `mdev -s` | repopulates `/dev` on the tmpfs |
| mount `/dev/mtdblock3` (squashfs) on `/config` | the miservice partition, read-only |
| mount the `customer` MTD (jffs2) on `/customer` | read-write |
| `mdev -s` | |
| start `fwfs` on `/dev/mtd5` → `/misc` with block size 65536, sub-block 32768, block cycles 500, read and program size 2048, cache 32768, file cache 32768, cache pool 2, block count 10, lookahead 8 | the firmwarefs FUSE daemon `/bin/fwfs` (libfuse 2.9.9) mounts MISC (640 KiB = 10 × 64 KiB blocks); it daemonizes after mounting and serves `/misc` for the life of the system. The MISC superblock records `block_count 6` while the daemon is told 10 and uses 10, and the daemon crashes without the file-cache option (`misc-firmwarefs.md` §3.3, §4.2) |
| create `/dev/pts` and mount devpts there | works (telnetd needs it) |
| start `telnetd` in the background | busybox telnetd with defaults: port 23, runs `/bin/login`, no `-l` / `-p` |
| run `/customer/demo.sh` if it exists | synchronous; demo.sh puts the apps in the background itself (§3) |
| 22 × `riu_w`, called by the relative path `./customer/riu_w` | §5; the relative path works because init's working directory is `/` |

The shell script carries three failing mounts, three `mdev -s` and a tmpfs over `/dev`. `/config` and `/misc` are mounted
before demo.sh loads `mi_sys.ko`, whose insmod runs `/config/config_tool` twice — as `load_mmap /config/mmap.ini` and as
`load_config /misc/config.ini /misc/PQConfig.ini` — to fill `/proc/mi_modules/common/*` (`config_tool.md` §1.3) [R boot log].

### 2.4 `/etc/mdev.conf`
Input `mice|mouse.*|event.*` → `/dev/input/`; ALSA `pcm.*|control.*|timer` → `/dev/snd/`;
`mmcblk[0-9] 0:0 0660 * /customer/sd_hotplug.sh` (whole-disk node only; `*` = run on add and on remove);
`$DEVNAME=bus/usb/([0-9]+)/([0-9]+)` → `bus/usb/%1/%2`. Of these only the `mmcblk` line and the `event*` line (the IR
receiver, `/dev/input/event0`, read by kjd_lvgl) serve the product [I].

### 2.5 sysctl.conf
`net.core.rmem_default=163840`, `net.core.rmem_max=163840`, `net.core.wmem_default=524288`, `net.core.wmem_max=1048576`,
`net.ipv4.tcp_mem=924 1232 1848`, `net.ipv4.tcp_rmem=4096 87380 325120`, `net.ipv4.tcp_wmem=4096 131072 393216` — network
buffer sizes; nothing else on the camera depends on them [I].

### 2.6 profile
Read by login shells only — **not** by rcS, demo.sh or the apps: `PATH=/bin:/sbin:/usr/bin:/usr/sbin:/config`,
`LD_LIBRARY_PATH=/lib:/config/lib:/customer`, `TERMINFO=/config/terminfo`, `ulimit -c unlimited`.

### 2.7 fw_env.config and the environment writers
`/etc/fw_env.config` = `/dev/mtd0 0x6F000 0x1000 0x10000`: the U-Boot environment inside mtd0 (the whole-flash view), 4 KiB
of environment, 64 KiB erase sector. `/etc/fw_printenv` and `/etc/fw_setenv` (a link to it) are u-boot-tools. Three
writers on the Linux side, each through `/etc/fw_setenv`:

| writer | variable and value | when |
|---|---|---|
| demo.sh (§3.4) | `SdUpgradeImage H161X.bin` | every boot |
| sd_version_recognition (§4.3) | `ota_upgrade_status 2`, then `reboot` | an SD card with `H161X.bin` whose version differs from the camera's |
| prog_uvc (UVC XU "OTA" branch, `_UVC_Events_Process_Data`; `prog_uvc.md` §4.7) | `ota_upgrade_status 1`, then `reboot` → U-Boot runs the USB download upgrade `ufu` | on the host's request |

What each value does in U-Boot, the cost of a `fw_setenv` (it erases and rewrites the 64 KiB block that also holds the
end of the U-Boot image) and the `reset` patch that renames `SdUpgradeImage` — `uboot.md` §3.1, §4.

### 2.8 passwd, group
Only `root` (uid 0, shell `/bin/sh`).

### 2.9 Other files of the rootfs
`/etc/init.d/udhcpc.script` exists; nothing starts a DHCP client. `/bin/scp` is the OpenSSH scp client (it carries OpenSSH
`log.c`'s level and facility tables QUIET…DEBUG3, AUTH, LOCAL0–7, which dropbear's scp lacks [R 2026-10-27, strings]); its
ssh program is `/customer/ssh/bin/ssh`, which the image does not contain, and the device has no ssh at all [R]. The stock
third-party software of the rootfs and `/customer` (busybox 1.20.2, glibc, libfuse 2.9.9, libmp4v2, u-boot-tools, htop
1.0.2, iperf3 3.1.3, terminfo) — `firmware-image.md` §3.7, §3.8.

## 3. `/customer/demo.sh` [R]

No `#!` line, so rcS's `/bin/sh` runs it as a child, working directory `/`. Its first line is `echo 4 > /proc/sys/kernel/printk`.

### 3.1 Environment of the apps
After the module lines demo.sh **sources** `/customer/set.sh`: a `#!/bin/sh` line ending in CRLF (a comment when sourced)
and `export LD_LIBRARY_PATH=/customer/lib:$LD_LIBRARY_PATH;` without a line end [R file]. Nothing else is exported and
`/etc/profile` is not read, so the apps run with `LD_LIBRARY_PATH=/customer/lib:` — needed only for `libmp4v2.so.2`
(prog_uvc NEEDED; kjd_lvgl needs only libc / libm / libpthread) [R readelf].

### 3.2 Modules
All from `/config/modules/4.9.227/`, in file order. Use: **used** by the product; **dependency** = needed only as a symbol
dependency of a used module (`depends=` in the .ko [R]); **unused** = loaded, nothing on the camera uses it
(`firmware-image.md` §3); **fails** = the insmod errors at every boot [R boot log].

| # | module | parameters | depends= [R] | use |
|---|---|---|---|---|
| 1 | usb-common | | | dependency (udc-core, libcomposite) |
| 2–6 | usbcore, ehci-hcd, scsi_mod, usb-storage, usbhid | | usb-common / usbcore | host stack, unused as such; `scsi_mod` **fails** (absent). ehci-hcd has no host device but programs the UVC device port's PHY and switches it to host before the gadget loads (`kernel-drivers.md` §4.2) |
| 7–14 | cifs, nls_utf8, grace, sunrpc, lockd, nfs, nfsv2, nfsv3 | | | unused. The kernel has cp437 and iso8859-1 built in (kallsyms `init_nls_*`); `mount -t vfat` without `iocharset=` does not need nls_utf8 [I] |
| 15 | mmc_core | | | used (SD) |
| 16 | mmc_block | | mmc_core | used |
| 17 | kdrv_sdmmc | | mmc_core | used (SD host, `kernel-drivers.md` §5.1) |
| 18 | fat | | | used |
| 19 | msdos | | fat | unused |
| 20 | vfat | | fat | used |
| 21 | ntfs | | | unused |
| 22–23 | sd_mod, ms_notify | | | **fail** (absent; built into the kernel) |
| 24 | mdrv-sata-host | | | unused (no SATA disk; `kernel-drivers.md` §5.2) |
| 25 | mhal | (none) | | used, first of the media stack (its parameters keep their defaults, `mhal-core.md` §5) |
| 26 | mi_common | | | used |
| 27 | mi_sys | `cmdQBufSize=3072 logBufSize=256` | mi_common | used |
| 28 | mi_sensor | | mi_common | used |
| 29 | mi_mipitx | | mi_common | unused |
| 30 | mi_gfx | | mi_common | dependency (mi_disp) |
| 31 | mi_rgn | | mi_common | dependency (mi_scl) |
| 32 | mi_scl | | mi_common, mi_rgn | used |
| 33 | mi_vdec | | mi_common | used by the MP4 player only |
| 34 | mi_hdmi | | mi_common | used |
| 35 | mi_isp | | mi_common | used |
| 36, 37 | mi_ipu, mi_shadow | | mi_common | unused |
| 38 | mi_jpd | | mi_common | used by the photo viewer only |
| 39 | mi_ao | | mi_common | unused |
| 40 | mi_ldc | (none) | mi_common | unused |
| 41 | mi_vif | | mi_common | used |
| 42 | mi_venc | | mi_sys, mi_common | used |
| 43 | mi_panel | | mi_common | unused |
| 44 | mi_disp | | mi_common, mi_gfx | used |
| 45 | mi_ai | | mi_common | unused |
| 46 | mi_vdisp | `vdisp_bufq_depth=4` | mi_common | unused |
| — | creates `/dev/mi_poll` by hand: looks up the major of `mi_poll` in `/proc/devices`, then `mknod … c <major> 0` | | | used — mi_poll registers no class device, so mdev cannot create the node (`mi_sys-2-interfaces.md` §4) |
| 47 | fbdev | `default_config_path_file="/config/fbdev.ini"` | | used (kjd_lvgl's `/dev/fb0`) |
| 48 | gc8613_MIPI | `chmap=1 lane_num=4` | | used: `chmap=1` registers one sensor instance on pad 0; `lane_num` is read by no code, so it has no effect (`sensor-gc8613.md` §2) |
| — | `mdev -s` | | | used (creates `/dev/mi_*`, `/dev/fb0`, …) |
| 49–51 | media, videodev, v4l2-common | | | **fail** (duplicate symbols, built into the kernel) |
| 52 | usb-common (again) | | | **fails** ("File exists") |
| 53–56 | videobuf2-core, videobuf2-v4l2, videobuf2-memops, videobuf2-vmalloc | | videodev (built in) | dependency (usb_f_uvc) |
| 57 | udc-core | | usb-common | used |
| 58 | libcomposite | | udc-core, usb-common | used |
| 59 | usb_f_uvc | | libcomposite, udc-core, videobuf2-* | used |
| 60 | u_ether | | udc-core | used (RNDIS) |
| 61 | usb_f_rndis | | libcomposite, udc-core, u_ether | used |
| 62 | udc-msb250x | | udc-core | used (USB device controller) |
| — | g_webcam | inserted later by prog_uvc: `streaming_maxpacket=3072 uvc_function_enable=1 uac_function_enable=0 rndis_function_enable=1` | libcomposite, u_ether, usb_f_uvc | used (`usb-gadget.md` §4.2) |

The `depends=` field of the mi_* modules names no mhal, yet mhal must be loaded first: they call its exports, and the
kernel resolves them because mhal is already in [I: the vendor build did not record mhal in modinfo]. In USB-disk mode
kjd_lvgl loads two more modules on demand, `usb_f_mass_storage` and `g_mass_storage` (§4.2, `kjd_lvgl.md` §9.1).

### 3.3 Network
demo.sh brings `usb0` up as 192.168.1.100 / 255.255.0.0 with a default route through 192.168.1.1, and sets `lo` to 127.0.0.1.
At that moment usb0 does not exist (g_webcam is loaded later by prog_uvc), so the boot log shows
`SIOCGIFFLAGS/SIOCSIFADDR: No such device` and `SIOCADDRT: Network is unreachable`; only `lo` is configured here. prog_uvc
repeats the same three usb0 / route commands after its own `insmod g_webcam.ko` [R strings], which is what brings usb0 up
(192.168.1.100/16, after the RNDIS function exists). The menu link (TCP 8888, prog_uvc server ↔ kjd_lvgl client) runs over
`lo` (`prog_uvc.md` §6.1). No DHCP client runs (§2.9).

### 3.4 Environment write and the apps
Setting `SdUpgradeImage` to `H161X.bin` with `fw_setenv` rewrites the environment sector at **every** boot, although the
value is already there [R value in the dump environment] — one NOR erase / write per boot (§2.7). Then prog_uvc starts in
the background with the options `-H -A 0 -a 0 -q` and the IQ file `/customer/iq_file_gc8613.bin`, demo.sh waits 3 s and
starts kjd_lvgl in the background; kjd_lvgl exits
when `/customer/ws.bin` is missing and prog_uvc creates it, so the `sleep 3` orders the two [I] (§9). demo.sh then exits and
rcS goes on with the register writes (§5) while the apps start.

## 4. Hotplug and USB scripts [R]

### 4.1 `/customer/sd_hotplug.sh`
Run by mdev (§2.4) with the environment variables `ACTION` and `MDEV`. `SD_DEV` = the `/dev/mmcblk[0-9]` node that exists,
`SD_DEV_NAME` = the existing `/dev/mmcblk[0-9]p[0-9]` nodes; mount point `/mnt`.
- `ACTION` = `add` or empty → mount:
  - no partition node: mount the whole disk as vfat on `/mnt`; send SIGUSR1 to kjd_lvgl; run
    `/customer/sd_version_recognition`; append a line «mount <device> on /mnt» to `/tmp/sdcard_info`.
  - partition nodes: take the first of them; mount it as vfat on `/mnt`; send SIGUSR1 to kjd_lvgl; append the same kind of
    line to `/tmp/sdcard_info`; run `/customer/sd_version_recognition`.
- `ACTION=remove` → send SIGUSR2 to kjd_lvgl; unmount `/mnt`.
- Two more writes to `/tmp/sdcard_info` are commented out.

In both branches SIGUSR1 goes out **before** `/tmp/sdcard_info` is written, and kjd_lvgl reads that file in its handler
(`kjd_lvgl.md` §9.2) — a race; in the no-partition branch the marker is written only after the upgrade check, so kjd_lvgl can read
an empty file. Only the whole-disk node triggers the script (§2.4), so the partition node must already exist when it runs [I:
the kernel creates the partitions before the uevent is processed — not verified]. Which of the two SD hosts carries the
socket, and so whether the card is always `mmcblk0`, is not read on this camera (`kernel-drivers.md` §5.1.7; CAMERA-CHECKS
K43: `readlink /sys/block/mmcblk0/device`, or the two `get_sdmmc*_status` files with a card inserted).

### 4.2 `/customer/usb_otg.sh <device|mass_storage|rmmod_device>`
A bash script, but the camera has no bash; the callers run it with `sh` [I]. Without an argument it prints the three
choices and exits with 1.
- `device`: loads the 14 gadget-side modules demo.sh loads (media, videodev, v4l2-common, usb-common, videobuf2-core,
  -v4l2, -memops, -vmalloc, udc-core, libcomposite, usb_f_uvc, u_ether, usb_f_rndis, udc-msb250x), then `g_webcam.ko` with
  `streaming_maxpacket=3072 streaming_maxburst=13 uac_function_enable=0 rndis_function_enable=0` — no caller uses this
  branch (`prog_uvc.md` §2.2, `kjd_lvgl.md` §9.1).
- `mass_storage`: loads `usb_f_mass_storage.ko`.
- `rmmod_device`: unloads `g_webcam` and prints `/proc/interrupts` (an unload of `udc-msb250x` is commented out).

kjd_lvgl sources `usb_otg.sh` for the switch to USB mass storage: on the first SD insert after start `rmmod_device` (the UVC camera
stops), then `insmod usb_f_mass_storage.ko` and `insmod g_mass_storage.ko file=/dev/<dev> removable=1`; with a card present
at boot only the two insmods; never reverted on removal (`kjd_lvgl.md` §9.1).

### 4.3 `/customer/sd_version_recognition` [R disassembly]
936 B of code, with debug info. What it does, completely:
1. It lists `/mnt` (exit 1 if the directory cannot be opened) and looks for an entry named exactly `H161X.bin`
   (case-sensitive). None → it prints that the file is not found and exits with 0.
2. It reads a version from `/mnt/sd_version.txt`, then one from `/customer/version.txt`; either failing → a message and
   exit 1. A version is read from the first line that contains `version:`: the text after it, with leading spaces and tabs
   skipped, cut at the first line feed and then at the first carriage return, at most 127 characters (lines are read in
   128-byte pieces).
3. It prints both versions. Equal strings → it prints «result: same» (in Chinese) and exits. Different → it prints
   «result: different», sets the U-Boot variable `ota_upgrade_status` to 2 with `fw_setenv` and reboots.
`/customer/version.txt` = `mirror:1\r\nversion:W11861322604301\r\n`; the `mirror:` line has no `version:`, so the second
line is taken. Without `/mnt/H161X.bin` the program does nothing. **Any** difference of the two strings (older or newer)
sets the environment and reboots; the content of `H161X.bin` is not checked here. U-Boot compares `ota_upgrade_status` with `"1"` only, so the value 2 is a marker that
nothing reads; the reboot is what matters. Independently of this program, U-Boot's `sdstar` runs `H161X.bin` from the card
as a U-Boot command script whenever `SdUpgradeImage` names it; how often that recurs depends on the last line of
`H161X.bin` and on the ODM patch in U-Boot's `reset`, which sets `SdUpgradeImage` to `SigmastarUpgrade.bin`
(`uboot.md` §4.1, §4.2). `H161X.bin` is not on hand; its content is not known.

## 5. The 22 boot-time register writes

### 5.1 How they are written [R]
The last 22 steps of rcS run `riu_w` with three hex arguments: bank, register, value. `riu_w` (§6) does a 16-bit store to the RIU
register `reg` of `bank`: **phys = 0x1F000000 + bank·0x200 + reg·4** (RIU addressing: `kernel-platform.md` §3.1), the same
store as busybox `devmem <phys> 16 <val>` (§6.3). The writes run after
demo.sh has put prog_uvc and kjd_lvgl in the background, so they land while the apps start: before 4K streaming begins,
not before the MI pipeline is created [R order; I timing].

### 5.2 The list
Register names [R]: bank 0x1012 holds group g of the MIU arbiter at reg `0x20 + 0x10·g`: +0 cfg, +1 burst (bits 15:8 group
burst, 7:0 member burst), +4 high-priority mask (regs 0x44 / 0x54 for groups 2 / 3), +0xA flowctrl0 (bits 15:8 period, 7:4
id1, 3:0 id0), +0xC member priority, +0xE member no-limit — the offsets of `mercury6/miu_arb.c` (gitee marcio/sigmastar),
which lacks only +4. Those, bank 0x1013 reg 0x3F and bank 0x1011 reg 0x75 — `kernel-platform.md` §12.3; bank 0x1170 = MOPG
of display device 0 — `mhal-disp.md` §11.4.

| # | bank, register, value | phys | register |
|---|---|---|---|
| 1 | `0x1012 0x5c 0xFEFF` | 0x1F202570 | group 3 member priority |
| 2 | `0x1012 0x41 0x3818` | 0x1F202504 | group 2 burst (group 0x38, member 0x18) |
| 3 | `0x1012 0x51 0x5838` | 0x1F202544 | group 3 burst (0x58, 0x38) |
| 4 | `0x1011 0x75 0xA008` | 0x1F2023D4 | normal-way cut length |
| 5 | `0x1012 0x51 0x4020` | 0x1F202544 | group 3 burst (0x40, 0x20) |
| 6 | `0x1170 0x4 0x56` | 0x1F22E010 | MOPG device 0, bits 7:0 = luma (Y) DMA request threshold |
| 7 | `0x1170 0x5 0x56` | 0x1F22E014 | MOPG device 0, bits 7:0 = chroma (C) DMA request threshold |
| 8 | `0x1012 0x5E 0xF000` | 0x1F202578 | group 3 member no-limit = members 12–15 |
| 9 | `0x1013 0x3F 0xF000` | 0x1F2026FC | group 3 member burst select `mbr_sel1` (members 8–15): members 14, 15 take burst #3 |
| 10 | `0x1012 0x5C 0x0FFF` | 0x1F202570 | group 3 member priority = members 0–11 |
| 11 | `0x1012 0x54 0x0FFF` | 0x1F202550 | group 3 high-priority mask |
| 12 | `0x1012 0x40 0x801D` | 0x1F202500 | group 2 cfg |
| 13 | `0x1012 0x41 0x4020` | 0x1F202504 | group 2 burst (0x40, 0x20) |
| 14 | `0x1012 0x50 0x801D` | 0x1F202540 | group 3 cfg |
| 15 | `0x1012 0x51 0x1008` | 0x1F202544 | group 3 burst (0x10, 0x08) |
| 16 | `0x1012 0x44 0x03FF` | 0x1F202510 | group 2 high-priority mask |
| 17 | `0x1012 0x54 0xFFFD` | 0x1F202550 | group 3 high-priority mask |
| 18 | `0x1012 0x5a 0x8022` | 0x1F202568 | group 3 flowctrl0: period 0x80, id1 2, id0 2 |
| 19 | `0x1012 0x50 0x811D` | 0x1F202540 | group 3 cfg, flowctrl0 enabled |
| 20 | `0x1012 0x4a 0x4010` | 0x1F202528 | group 2 flowctrl0: period 0x40, id1 1, id0 0 |
| 21 | `0x1012 0x40 0x811D` | 0x1F202500 | group 2 cfg, flowctrl0 enabled |
| 22 | `0x1012 0x51 0x3018` | 0x1F202544 | group 3 burst (0x30, 0x18) |

State after the list (14 registers) [R arithmetic]: 0x1F202500 = 0x811D, 0x1F202504 = 0x4020, 0x1F202510 = 0x03FF,
0x1F202528 = 0x4010, 0x1F202540 = 0x811D, 0x1F202544 = 0x3018, 0x1F202550 = 0xFFFD, 0x1F202568 = 0x8022,
0x1F202570 = 0x0FFF, 0x1F202578 = 0xF000, 0x1F2023D4 = 0xA008, 0x1F22E010 = 0x0056, 0x1F22E014 = 0x0056,
0x1F2026FC = 0xF000. The state the list starts from is the kernel's arbiter policy (groups 0 and 2 already at cfg 0x811D
with flow control); cfg 0x801D and burst 0x1008 are the IPL's values. The net change against the kernel state, with the
DRAM clients of groups 2 / 3 (VCODEC, ISP DMA, the MOP read channels) and the reading of the whole block as a display-bandwidth
change — `kernel-platform.md` §12.2, §12.4.

### 5.3 Where the list comes from [S, R comparison]
<!-- ev:rootfs-customer.5.3 -->
A SigmaStar FAQ answer gives these writes as the
fix for frame-buffer flicker at HDMI 4K30 with a 4K ARGB UI and VENC running: "VENC outranks DISP GOP for DDR bandwidth;
adjust MOP burst length" [S]. Writes 5–11 are exactly one variant of it quoted by SigmaStar FAEs on the Comake developer chat [S, R comparison];
another variant quoted there also writes bank 0x1175 regs 4 / 5 (1175:04 / 1175:05) = 0x56 (the same two MOP thresholds of display device 1) and 1012:4C = 0xFFBF —
this firmware writes neither. The whole list reads like several FAQ variants pasted one after another [I]. For the MOP
thresholds U-Boot and the OpenIPC source initialise 0xF8 / 0xD0; this mhal (`HalDispMopgSet{DmaY,DmaC}FifoThrd`) writes 0x30 / 0x30 only while
the MOP is still in reset (`mhal-disp.md` §11.4); mhal.ko holds 14 literal constants in 0x1170xx and 15 in 0x1175xx (bank·0x100 + reg ids)
[R count].

### 5.4 Not known
Whether the intermediate values matter (an arbiter that latches on write) or only the final state — a camera check
(CAMERA-CHECKS P12: boot with the intermediate writes left out). What 1170:04 / 1170:05 hold before `riu_w` — whether U-Boot left
0xF8 / 0xD0 (CAMERA-CHECKS P11: read 1170:00 bit 0 and 1170:04 / 05 with the boot stopped before demo.sh). Whether a 1 in
the priority / high-priority-mask registers raises or lowers a member, and the meaning of cfg bits 4 and 15 —
`kernel-platform.md` §12.4. The arbiter's live state is readable through `/sys/devices/system/miu/miu0/`
(`monitor_client_enable`, `measure_all`, the arbiter dump; `kernel-platform.md` §12.2).

## 6. `riu_r` / `riu_w` / `riux32_r` / `riux32_w` — one binary, four names

### 6.1 Identity [R]
- The four files `/customer/riu_w`, `/customer/riu_r`, `/customer/riux32_r`, `/customer/riux32_w` are **byte-identical** (sha256 `15016388aa96a37b2973cca16807f460ef56e89263feee3c9f6b636391189eef`,
  5.5 KB, ARM EABI5 hard-float, dynamically linked against glibc, stripped, `GCC: (GNU) 9.1.0`). In the dump they are four
  regular files, not symlinks [R extracted jffs2 listing]; the vendor build script makes one binary and three symlinks
  (`drivers/sstar/samples/build_riu.sh` compiles `riu_r`, strips it and links the other three names to it)
  [R source] — the symlinks were dereferenced when the ODM copied them into the image [I].
- Source: the public SigmaStar GPL sample `drivers/sstar/samples/riu.c`, **i6e** variant (2018), not the i6c
  one: the i6c variant calls `personality()` to clear `READ_IMPLIES_EXEC`, and the binary imports no `personality` (imports:
  strcmp, strtol, printf, ioctl, puts, open, mmap, putchar, `__xpg_basename`, munmap, abort, close). Control flow, strings,
  constants and argument checks of `main` @0x1061c (Thumb, built -O0) are those of the i6e source line for line [R disassembly
  vs source].
- Use: `riu_w` is run by rcS (§5); `riu_r`, `riux32_r`, `riux32_w` are **unused** — no script and no binary in the dump refers to
  them (the names occur only as jffs2 directory entries of the customer partition in `nor.bin`) [R].

### 6.2 Behaviour, complete [R]
1. `open("/dev/msys", O_RDWR|O_SYNC)` (flags 0x101002); failure → `can't open /dev/msys`, return −1.
2. `open("/dev/mem", O_RDWR|O_SYNC)`; failure → `can't open /dev/mem`, close msys, return −1.
3. `ioctl(msys, 0x5315, &info)` = `IOCTL_MSYS_GET_RIU_MAP` = `_IO('S', 0x15)` with the packed 20-byte `MSYS_MMIO_INFO`:

   | off | field | in | out |
   |---|---|---|---|
   | 0 | `VerChk_Version` u32 | `0x4D530100` (`VERCHK_HEADER 0x4D530000` \| `IOCTL_MSYS_VERSION 0x0100`) | unchanged |
   | 4 | `addr` u64 | — | RIU physical base |
   | 12 | `size` u32 | — | RIU window length |
   | 16 | `VerChk_Size` u32 | `0x14` = sizeof | unchanged |

   Kernel side (`ms_msys.c` `msys_get_riu_map_verchk`): rejects a missing `0x4D53xxxx` header (−EFAULT), a version below
   0x0100 or a size other than 20 (−EINVAL); fills `addr = Chip_Get_RIU_Phys()`, `size = Chip_Get_RIU_Size()`
   (`mstar_chip_get_riu_phys/size`), which on Mercury6 are `IO_PHYS 0x1F000000` and `IO_SIZE 0x00400000`
   (`arch/arm/mach-sstar/mercury6/soc.c`, `include/mercury6/registers.h`) [R source]; the DTB `io_phy_addr` agrees on the base
   [R]. The rest of the `/dev/msys` ioctl set — `kernel-platform.md` §10.1. Any non-zero return → `DMEM request failed!!`
   (the sample's wording; no DMEM is involved).
4. `mmap(NULL, size, PROT_READ|PROT_WRITE, MAP_SHARED, memfd, (off_t)addr)` — only the low 32 bits of `addr` are passed
   (32-bit `off_t`). The result is compared with **0**, not `MAP_FAILED`: a failed mmap (−1) is not caught and the access
   that follows faults (SIGSEGV) [R compare, I outcome]. `NULL pointer!` is effectively unreachable [I].
5. Dispatch on `basename(argv[0])` (`__xpg_basename`, so a path prefix is allowed; any other name → `argument error`):

   | name | argc | action | output (printf formats) |
   |---|---|---|---|
   | `riu_w` | 4: `bank reg val` | 16-bit store `strh` to `map + bank·0x200 + reg·4`, then 16-bit read back | `BANK:0x%04X 16bit-offset 0x%02X\n`, `0x%04X\n` (read-back) |
   | `riu_r` | 2: `bank` | 128 × 16-bit reads, reg 0x00…0x7F | `BANK:0x%04X\n`, then rows: `%02X: ` at every reg ≡ 0 mod 8, `0x%04X ` per reg, `\n` after reg ≡ 7 mod 8 |
   | `riu_r` | 3: `bank reg` | one 16-bit read | `BANK:0x%04X 16bit-offset 0x%02X\n`, `0x%04X\n` |
   | `riux32_w` | 4: `bank reg val` | 32-bit store `str` to the same address, 32-bit read back | `BANK:…16bit-offset…\n`, `0x%08X\n` |
   | `riux32_r` | 2: `bank` | 128 × 32-bit reads | as `riu_r`, values `0x%08X ` |
   | `riux32_r` | 3: `bank reg` | one 32-bit read | `BANK:…\n`, `0x%08X\n` |

   Every number is parsed with `strtol(s, NULL, 16)` (hex, optional `0x`, no error check; garbage → 0). A known name with
   the wrong argc does nothing and exits 0 silently. The `val` of `riu_w` is truncated to 16 bits. The output goes to the
   console; rcS ignores it.
6. `munmap(map, 0xff)` (not the mapped length — only the first page is unmapped; harmless, the process exits), close both
   fds, return 0.

### 6.3 Address range [R formula, I bus layout]
Byte offset = `bank·0x200 + reg·4` (`(reg + bank·0x80)·4` in the binary). With the 4 MiB window the valid range is bank
0x0000…0x1FFF; nothing checks `bank` / `reg`, so bank ≥ 0x2000, or a `reg` large enough to leave the window, touches
unmapped memory → SIGSEGV [I]. "16bit-offset": `reg` counts 16-bit registers, each occupying the low half of one 32-bit slot
of the RIU bus (standard SigmaStar RIU). A 32-bit access (`riux32_*`) on a 16-bit bank returns the register in bits 15:0;
bits 31:16 read as 0 or bus garbage, and only banks built as 32-bit registers take a full 32-bit value [I]. Every access is
also a busybox `devmem` access (busybox 1.20.2's `devmem` supports widths 8 / 16 / 32): `riu_w B R V` ≡ `devmem $((0x1F000000 + 0xB*0x200 + 0xR*4)) 16 0xV` without the two
printed lines, `riu_r B` ≡ 128 × `devmem … 16`, `riux32_*` ≡ width 32; the base is fixed for this SoC, so the `/dev/msys`
request adds nothing to the address [I].

## 7. `i2c_read_write` — user-space I2C register tool (unused)

### 7.1 Identity [R]
<!-- ev:rootfs-customer.7.1 -->
21.7 KB, ARM EABI5 hard-float, **ARM (not Thumb) code**, dynamically linked (glibc), **not stripped** (function and global
names kept; DWARF only for the glibc start files). Source file name `i2c_read_write.c` (symtab FILE). Built with
**`gcc-arm-8.2-2018.08`** (the ARM / Linaro A-profile toolchain, `GCC 8.2.1 20180802`), not the SigmaStar 9.1.0 toolchain
of the other `/customer` binaries — a tool brought in from elsewhere [I]. No public source is known (and the
kernel trees hold `doFileParser` / `ReadRegisterPair` only as unrelated sensor-driver enums). Nothing in the dump starts it
(the name appears only as a jffs2 directory entry of the customer partition in `nor.bin`) → **unused**.

Functions: `freeMem` 0x10b20, `i2cOpen` 0x10c48, `i2cClose` 0x10ccc, `i2cSetAddress` 0x10cf0, `WriteRegisterPair` 0x10d4c,
`ReadRegisterPair` 0x10f80, `getRegAddr` 0x11234, `getRegValue` 0x1132c, `getSlaveAddr` 0x11404, `getI2CFMT` 0x11434,
`doCMD` 0x114e4, `doFileParser` 0x116bc, `option` 0x11a30, `main` 0x12218.
Globals: string copies `i2cBus slaveAddr i2cFMT readReg writeReg dumpReg scriptFile` (malloc'd), flags `bus_flag
slaveAddr_flag i2cFMT_flag read_flag write_flag dump_flag script_flag` (static int), `devRegAddr` u16, `devRegValue` u16
(0x23178), `fmt` int, `devSlaveAddr` int, `g_i2cFile` int.

### 7.2 Command line [R]
`getopt_long(argc, argv, "b:a:f:r:w:d:s:", long_options)`. Short options store a malloc'd copy of the argument and set the flag:

| opt | meaning | parsed when given |
|---|---|---|
| `-b N` | bus: device `/dev/i2c-N` (the string pasted in by `sprintf("/dev/i2c-%s")` into a 64-byte stack buffer — no length check) | — |
| `-a XX` | **8-bit** slave address, hex (`strtol(s,0,16)`); every transaction uses `XX >> 1` as the 7-bit address | `devSlaveAddr` |
| `-f F` | format `A8D8` → 0, `A16D8` → 1, `A8D16` → 2, `A16D16` → 3, anything else → 4 | `fmt` |
| `-r REG` | read one register | `devRegAddr = getRegAddr(REG)` |
| `-w "REG VAL"` | write; one argument, split on spaces by `strtok` (1st token register, 2nd value, more ignored) | `getRegAddr`, `getRegValue` |
| `-d "START [STOP]"` | dump range: prints `dumpReg: %s`, sets `dump_flag` — **never acted on** (no dump code exists) | — |
| `-s FILE` | run a script file (§7.4); prints `scriptFile: %s` | — |

The format checks (`getRegAddr` / `getRegValue`) use the **current** `fmt`, so `-f` must come before `-r` / `-w` (default
`fmt` 0 = A8D8): the address string must be exactly 4 characters long for the A8 formats (0, 2) and 6 for A16 (1, 3), the
value 4 for D8 (0, 1) and 6 for D16 (2, 3) — i.e. `0x12` / `0x1234` with the prefix; on a mismatch `I2C addr FMT error: %d`
(two variants, one with a trailing space) or `I2C value FMT error`, `exit(-1)`. Format 4 skips the checks. Numbers:
`strtol(…,16)`.
Long options (`--bus --slaveAddr --fmt --read --write --dump --script`, all `has_arg=1`) are declared with `flag=&<x>_flag,
val=1`: getopt_long sets the flag and returns 0, and the code only prints `option %s[ with arg %s]\n` — **the argument is not
stored**, so e.g. `--bus 0` leaves `i2cBus` NULL (the open of `/dev/i2c-(null)` fails) and `--read X` reads register 0 [R
code; I outcome]. An unknown option → getopt's own message, ignored; the `abort()` default arm is unreachable [I].
Usage check after parsing: if `(argc < 2 || !bus || !addr || !fmt) && !script` → print the usage block (strings at
0x12594–0x12714, verbatim in the binary) and `exit(1)`. With `-s` the check passes without `-b`, but `i2cOpen` still needs
it (`/dev/i2c-(null)` → `perror("i2cOpen")`, `exit(1)`) [R, I glibc `%s` of NULL].

### 7.3 Transactions [R]
`main`: `option()` → `i2cOpen()` (`open(path, O_RDWR)`) → with a script: clear the read / write flags, `doFileParser()` → read
flag: `ReadRegisterPair`, print `Reg value: %#x \n` → write flag: `WriteRegisterPair` (return value ignored) → close, free,
return 0. Each register access first calls `ioctl(fd, 0x0706 = I2C_SLAVE, addr8 >> 1)` (failure → `perror("i2cSetAddress")`,
exit 1). `I2C_SLAVE`, not `I2C_SLAVE_FORCE`: an address claimed by a kernel driver gets EBUSY [I, standard i2c-dev].

| fmt | write: one `write()` of | read: `write()` of, then a separate `read()` of |
|---|---|---|
| 0 A8D8 (and 4) | 2 B `{reg[7:0], val[7:0]}` | 1 B `{reg[7:0]}`, 1 B |
| 1 A16D8 | 3 B `{reg[15:8], reg[7:0], val[7:0]}` | 2 B `{reg[15:8], reg[7:0]}`, 1 B |
| 2 A8D16 | 3 B `{reg[7:0], val[15:8], val[7:0]}` | 1 B, 2 B |
| 3 A16D16 | 4 B `{reg[15:8], reg[7:0], val[15:8], val[7:0]}` | 2 B, 2 B |

A short write → `perror("Write Register")`, return −1; for A16D16 the message is `SetRegisterPair` and the function still
returns 0. A short read → `perror("Read RegisterPair set register" | "… read value")`, −1. A read is two separate i2c-dev
transfers (a STOP between the address phase and the data phase, no repeated start) [R two syscalls, I bus behaviour]; i2c-tools
(`i2cget` / `i2cset` / `i2ctransfer`, e.g. `i2ctransfer -y N w2@0x37 0x03 0xf0 r1` for A16D8 at the 8-bit address 0x6e) cover
the same accesses, with a 7-bit address and a combined `I2C_RDWR` transfer [I].
Defects [R]:
- The read lands raw in a `u16` on the stack (`read(fd, &u16, n)`), without a byte swap: for the D16 formats the printed value
  is `byte0 | byte1<<8`, i.e. **byte-swapped** for a device that sends MSB first (writes are MSB first) [I for "device sends
  MSB first"].
- For the D8 formats one byte is read into that `u16`, whose upper byte is **uninitialised** (`main` fp−6, `doCMD` local), so
  `Reg value:` may show stack garbage in bits 15:8.

### 7.4 Script file (`-s`) [R]
Read line by line with `fgets` into a 4 KiB buffer (the file size is taken with fseek / ftell only for the final message
`do file: %s done, size: %#lx\n`). Per line, in this order:
- an empty line (`'\n'` first) or one starting with `//` → skipped;
- `fmt <F>` (prefix `fmt`) → `fmt = getI2CFMT(F)`;
- `slaveAddr <XX>` (prefix `slaveAddr`) → `devSlaveAddr = strtol(XX,16)` (a `getI2CFMT` call on the same string is discarded);
- first character `w` → `sscanf("%s %s %s")` → command, register, value;
- first character `r` → `sscanf("%s %s %s %s")` with **only two** destination pointers (command, register) — a line with more
  than two tokens writes through stray stack words (undefined behaviour);
- prefix `dela` (strncmp 4 of `delay`) → `sscanf("%s %s")` → command, ms;
- the command runs (`doCMD`) **only if `feof` is not yet set** after reading the line: a last line without a trailing newline
  is parsed but **not executed** [R code, I consequence].

`doCMD`: `w` → `getRegAddr` / `getRegValue` (the same length checks, exit on error), `WriteRegisterPair`, `write: %#x %#x \n`;
`r` → `ReadRegisterPair`, `read: %#x \n` (the same uninitialised-byte / swap defects); `dela…` → `delay %d\n`,
`usleep(strtol(ms,10)·1000)`. Tokens go into 16-byte stack buffers via `%s` without a width — a token of 16 characters or more
overflows. A script in this grammar [I, from the grammar; device and registers arbitrary]:
```
// example
fmt A16D8
slaveAddr 0x6e
w 0x03f0 0x00
delay 10
r 0x03f0
```

## 8. `tmp` — the factory test program (unused)

### 8.1 Identity [R]
16.4 KB, ARM EABI5 hard-float, **Thumb-2**, dynamically linked (glibc + libpthread), not stripped; source `tmp.c`; built with
the SigmaStar toolchain `gcc-sigmastar-9.1.0-2020.07` (the one of kjd_lvgl and prog_uvc) — an ODM-written tool [I]. DWARF
covers only the glibc start files. Functions: `ST_Flush` 0x108e4, `thread1` 0x10904 (1178 B), `thread2` 0x10d9e, `thread3`
0x10eb4, `main` 0x1108c. File-static globals: `acl` int @0x22074 (last front-panel key), `telecontrol` int @0x22078 (last
remote key), `is_flag` char @0x2207c (**never referenced**). Nothing in the dump starts it (only the jffs2 directory entry;
the `pidof tmp` inside the binary is its own) → **unused**. The Chinese UI strings are UTF-8; the translations below are
not in the binary.

### 8.2 Structure [R]
`main` creates three pthreads (default attributes, arg NULL) — `thread1` (console menu), `thread2` (SAR buttons), `thread3`
(IR remote) — and joins them in that order; on a `pthread_create` failure it prints `Failed to create threadN.` and returns
−1; after all three joins `Threads are terminated.`, return 0. The two input threads never end, so the process ends only by
being killed (menu item 7). The threads share `acl` / `telecontrol` through plain loads and stores, without locking
(int-sized, harmless) [R, I].

### 8.3 thread1 — console menu [R]
Prints the menu once at start (not again after each command):

| line (rodata) | text | translation |
|---|---|---|
| 0x111a0 | `select 1: SD卡测试` | SD card test |
| 0x111b8 | `select 2: LED灯变红` | LED turns red |
| 0x111d0 | `select 3: LED灯变橙  ` (two trailing spaces) | LED turns orange |
| 0x111ec | `select 4: 按键测试` | key (button) test |
| 0x11204 | `select 5: 遥控按键测试` | remote-control key test |
| 0x11224 | `select 6: 获取版本号` | get version number |
| 0x11240 | `select 7: 退出` | exit |

Loop: `scanf("%c", &c)`, then `ST_Flush()` (getchar until `'\n'`), then by `c`:

| c | action | output |
|---|---|---|
| `1` | `popen("ls -l /dev \| grep -i mmcblk0", "r")`, `fread` up to 1 KiB, `pclose`; looks only at the **first character**, i.e. whether `/dev/mmcblk0` is a block device | `b` (a block-device line) → `SD卡已插入` "SD card inserted", else `SD卡已拔出` "SD card removed" |
| `2` | `system("echo 0 > /sys/class/gpio/gpio46/value")` | `LED red sucess` (sic) — printed whatever `system()` returned |
| `3` | `system("echo 1 > /sys/class/gpio/gpio46/value")` | `LED orange sucess` |
| `4` | reads `acl` | `KEY<acl> PRESSED` for acl 1…6; nothing while acl = 0 (no key seen yet) |
| `5` | reads `telecontrol` | `TELECONTROL_KEY<n> PRESSED` for n 1…18; nothing while 0 |
| `6` | `fopen("/customer/version.txt","r")`; `fgets` lines of ≤ 100 B until one contains `version:`; prints the text after it with `%s` (no newline added; the file's own CRLF ends the line) | with the dump's file → `W11861322604301\r\n` (§4.3); no match → nothing |
| `7` | `system("kill -9 \`pidof tmp\`")` — SIGKILL to every process named `tmp`, itself included | — |
| other | ignored | — |

Items 4 and 5 are not live views: the tester presses a key on the device or the remote, then types 4 or 5 to see the last key
latched by the input threads; the latch is never cleared [R code, I intended use]. `tmp` does **not** export gpio46 or set its
direction: items 2 / 3 work only after `echo 46 > export` and `direction out`, which kjd_lvgl does at start, so the test
assumes the normal firmware has booted [R, I].
Defects:
- If `/customer/version.txt` cannot be opened, item 6 prints `无法打开文件n` ("cannot open file" followed by a literal `n` — the
  `\` of `\n` is missing in the source) and **returns from thread1**: the menu is gone, `main` stays blocked in
  `pthread_join(thread2)` forever and the program can only be killed from outside [R].
- `ST_Flush` loops `while ((char)getchar() != '\n')`; at end of input getchar returns EOF, `(char)EOF` is never `'\n'`, so with
  stdin closed or redirected from a finished file (started from a script, in the background, over a non-interactive shell)
  thread1 spins at full CPU forever; `tmp` needs an interactive console [R code, I outcome].

### 8.4 thread2 — front-panel buttons on the SAR ADC [R]
`open("/dev/sar", O_RDWR)` (failure: `perror("/dev/sar open error\n")`, continues with fd −1); `ioctl(fd, 0x6100, 0)` (INIT);
then forever: `ioctl(fd, 0x6101, &{u32 ch = 0, s32 value})` (an 8-byte struct zeroed once, channel 0; the ioctls:
`kernel-drivers.md` §3.1), classify `value`, `system("sync")`. There is no sleep: the loop runs as fast as `system()` can fork
a shell and run `sync`, so the thread keeps a shell busy all the time [R, I load]. Windows (inclusive, signed compares):

| value | acl | KEY | kjd_lvgl's name for the same window |
|---|---|---|---|
| 165…200 | 1 | KEY1 | ON |
| 280…315 | 2 | KEY2 | UP |
| 0…30 | 3 | KEY3 | OK |
| 435…470 | 4 | KEY4 | MENU |
| 745…780 | 5 | KEY5 | DOWN |
| 810…850 | 6 | KEY6 | AE |
| anything else (incl. released ≥ 1000) | unchanged | — | — |

The six windows are **identical** to kjd_lvgl's [R both binaries]: the same six-button resistor ladder on SAR channel 0
(`kernel-platform.md` §6.3). The KEY1…KEY6 order is not the ladder order; it may be the physical order of the buttons on the
case [I, unverified]. The real ADC reading of each button is not read on this camera (CAMERA-CHECKS K45: `/dev/sar` channel 0
while each button is held).

### 8.5 thread3 — IR remote [R]
<!-- ev:rootfs-customer.8.5 -->
`open("/dev/input/event0", O_RDWR)` (failure: `file to get ir dev` (sic), continues; `read` on −1 then fails at once and the
loop spins at full CPU [I]). Forever: zero a 16-byte `struct input_event` (32-bit layout: timeval 8 B, `type` u16 @8, `code`
u16 @10, `value` s32 @12), `read(fd, ev, 16)` (return not checked), `switch (type)`: 0 (EV_SYN) and 1 (EV_KEY) ignored, **4
(EV_MSC)** → map `value` (the NEC command byte rc-core sends as `MSC_SCAN`; `code` is not checked):

| value (scancode) | n | kernel key |
|---|---|---|
| 0x45 | 1 | KEY_POWER |
| 0x46 | 2 | KEY_UP |
| 0x47 | 3 | KEY_0 |
| 0x44 | 4 | KEY_LEFT |
| 0x40 | 5 | KEY_HOME |
| 0x43 | 6 | KEY_RIGHT |
| 0x07 | 7 | KEY_1 |
| 0x15 | 8 | KEY_DOWN |
| 0x09 | 9 | KEY_2 |
| 0x16 | 10 | KEY_3 |
| 0x19 | 11 | KEY_4 |
| 0x0D | 12 | KEY_5 |
| 0x0C | 13 | KEY_6 |
| 0x18 | 14 | KEY_7 |
| 0x5E | 15 | KEY_8 |
| 0x08 | 16 | KEY_9 |
| 0x1C | 17 | KEY_C |
| 0x5A | 18 | KEY_F1 |

Any other scancode leaves `telecontrol` unchanged. The test checks **exactly these 18 codes** — the 18 kjd_lvgl reacts to —
and not 0x42 / 0x52 / 0x4A (KEY_F2–F4), the other three entries of the kernel's 21-entry `rc-mstar-dtv` map (the keymap, kjd_lvgl's
action and the key of the microscope's remote for each code: `kernel-drivers.md` §3.2.2). TELECONTROL_KEY1…18 follow the
printed order of the 21-key generic NEC "Car MP3" remote row by row (CH−, CH, CH+, ⏮, ⏭, ⏯, −, +, EQ, 0, 100+, 200+, 1…6),
stopping before its 7 / 8 / 9 [R order against the keymap]. The microscope's own remote is a different, 18-key "Full HD"
remote that emits the same 18 codes [R: photo].

### 8.6 What items 2 and 3 show [R strings]
Item 2 writes `0` to GPIO46 (pad `PAD_GPIO2`) and reports "LED red", item 3 writes `1` and reports "LED orange": GPIO46 selects the colour of
the front status LED, 1 = orange, 0 = red — orange while kjd_lvgl runs, red in its standby (the pad, the export and the
probable two-colour circuit: `kernel-platform.md` §6.3). The colours by eye are not checked on this camera (CAMERA-CHECKS P9:
`echo 0/1 > …/gpio46/value`, look at the LED).

## 9. Files and markers the apps expect [R strings / disassembly]

What each app does with its files — `prog_uvc.md`, `kjd_lvgl.md` §3.

| path | expected by | created by | must exist at start? |
|---|---|---|---|
| `/customer/iq_file_gc8613.bin` | prog_uvc `-i` | shipped | yes |
| `/customer/ws.bin`, `/customer/default_ws.bin` (0x58 B) | prog_uvc, kjd_lvgl | prog_uvc creates both if ws.bin is missing; kjd_lvgl exits if ws.bin is missing (hence demo.sh's `sleep 3`) | ws.bin: yes for kjd_lvgl |
| `/customer/camp.txt` | prog_uvc (UVC controls) | shipped; opened `"r+"`, so not created | yes |
| `/customer/xu.txt` | prog_uvc (XU) | prog_uvc, `"w+"` | no |
| `/customer/ws.ini`, `/customer/Line.ini` | kjd_lvgl (reticle lines, photo / video size, language) | shipped; kjd_lvgl rewrites them | yes |
| `/customer/1600x900.png` | kjd_lvgl (reticle) | shipped | yes |
| `/customer/version.txt` | sd_version_recognition, kjd_lvgl, `tmp` | shipped; kjd_lvgl copies `/mnt/log/version.txt` from the card over it | yes for the SD check |
| `/customer/usb_otg.sh` | kjd_lvgl | shipped | for USB-disk mode |
| `/customer/lib/libmp4v2.so.2` | prog_uvc (NEEDED) | shipped | yes |
| `/misc/ws.txt` | prog_uvc crop mode (first character) | prog_uvc, `"ab+"` (creates) | no (needs `/misc` mounted) |
| `/misc/sigmastar1024_600.jpg` | kjd_lvgl, U-Boot `bootlogo` | MISC image; kjd_lvgl copies the card's copy over it | yes for its use |
| `/misc/config.ini` | mi_sys → config_tool (also U-Boot `bootlogo`) | MISC image | yes |
| `/config/mmap.ini`, `/config/fbdev.ini`, `/config/config_tool`, `/config/venc_fw/chagall.bin`, `/config/iqfile/iqfile0.bin` | mi_sys, fbdev, mhal, the 3A init | miservice image | yes |
| `/tmp/sdcard_info` | prog_uvc (exists → no USB gadget), kjd_lvgl (SD device name; truncates the file after reading) | sd_hotplug.sh (append, §4.1) | no |
| `/tmp/isp_uart` | prog_uvc (`getIspUartDebugEnable`: IQTool over the serial console) | nobody on the camera | no |
| `/tmp/raw_data.raw` | prog_uvc (raw-frame injection from IQTool) | prog_uvc itself, from the received data; removed after use | no |
| `/dev/mi_poll` | MI libraries | demo.sh `mknod` (§3.2) | yes |
| `/dev/mi_<mod>`, `/dev/fb0`, `/dev/msys`, `/dev/mem`, `/dev/sar`, `/dev/isp_fe`, `/dev/input/event0` | apps, MI libraries, `riu_w` | mdev (from sysfs) | yes |
| `/sys/class/gpio/gpio46/*` | kjd_lvgl, `tmp` | kjd_lvgl exports it itself | no |
| `/mnt/PHOTO`, `/mnt/VIDEO` | apps | apps, on the SD card [I] | no |
| `/etc/fw_setenv` | prog_uvc, sd_version_recognition, demo.sh | rootfs | yes |
| `/proc/mi_modules/mi_sensor/mi_sensor0` | prog_uvc (`echo setmirrorflip 0 1 1 >`) | mi_sensor | yes |
| `/sys/devices/system/miu/miu0/dram_info`, `/sys/devices/virtual/mstar/msys/CHIP_ID` | strings in prog_uvc, from the statically linked cam_os_wrapper object; no code references them (`kernel-platform.md` §10.1) | kernel | — |

prog_uvc also holds `/dev/mi/<mod>` path strings; which node each MI library actually opens — `mi-user-abi.md` §2.1.
