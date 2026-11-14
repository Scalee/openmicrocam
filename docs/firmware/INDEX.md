# Firmware specification of the YIZHAN 4K microscope camera — index

One current-truth document per component of the stock firmware of the YIZHAN 4K HDMI/USB microscope camera (SigmaStar
SSC8836 "Mercury6" SoC, GalaxyCore GC8613 sensor): what each component is and how it works, as known, including the parts
the product does not use. Unknowns are stated as unknown where they belong. The documents are written from the binaries
of the camera's own firmware, public sources and readings on the running camera; no vendor source code is quoted.

## How to read the documents

- **Origin tags.** Every claim carries its origin: **[R]** read from the binary, a file or the running camera ("[R camera]"
  marks a reading on our unit), **[S]** stated by a source (vendor headers, public code, documentation, a vendor
  engineer's statement), **[I]** inference. An inference stays [I]. A heading tagged [R] covers every statement under it
  unless marked otherwise.
- **Names.** Function names are the binary's own symbols or `__func__` strings where it has them. A function without a
  symbol is given a descriptive name of ours, chosen for what it does [I], with its address.
- **Addresses.** Each document states its address convention in its header: `.text` offsets, file virtual addresses, or
  the addresses of an image rebased for analysis (kernel modules of `mhal.ko` at 0x40000000, so 0x400xxxxx = `.text`
  offset + 0x40000028; user libraries at base 0x10000).
- **Registers.** The SoC's peripheral registers sit on the RIU bus: bank B, 16-bit register r is at physical
  `0x1F000000 + B·0x200 + r·4` (CPU virtual `0xFD000000 + …`); documents write it `B:r` or `B.r`
  ([`kernel-platform.md`](components/kernel-platform.md) §3.1).
- **Layers.** Applications call the MI user libraries (`libmi_*.so`), which issue ioctls to the MI kernel modules
  (`mi_*.ko`); those drive the hardware blocks through `mhal.ko`, the hardware abstraction layer, on the vendor's Linux
  4.9.227 kernel ([`mi-user-abi.md`](components/mi-user-abi.md), [`mhal-core.md`](components/mhal-core.md)).
- **References** to another document name its file and section.
- [`camera-checks.md`](camera-checks.md) — what only the running camera can settle; the documents cite its items as
  «CAMERA-CHECKS Kn / Pn».
- [`blobs.md`](blobs.md) — every vendor binary the camera runs or loads: loader, its document, and whether OpenMicroCam
  uses it; their sizes and sha256 are in [`stock-parts.tsv`](../../tools/install/stock-parts.tsv).
- [`data/`](data/) — the reference tables too long for a document, as TSV, each described where its document links it.

## Flash image and boot

| document | covers |
|---|---|
| [`firmware-image.md`](components/firmware-image.md) | the NOR flash image as shipped; file inventory in its appendix |
| [`ipl.md`](components/ipl.md) | IPL and IPL_CUST, the two boot stages before U-Boot |
| [`uboot.md`](components/uboot.md) | vendor U-Boot 2015.01 "M6" |
| [`misc-firmwarefs.md`](components/misc-firmwarefs.md) | the MISC partition's firmwarefs and the `/bin/fwfs` FUSE daemon |
| [`config_tool.md`](components/config_tool.md) | `/config/config_tool` and the mmap.ini / config.ini / PQConfig.ini contract |
| [`rootfs-customer.md`](components/rootfs-customer.md) | the boot scripts of `/etc` and `/customer`, the boot-time `riu_w` writes and the small `/customer` tools |

## Kernel

| document | covers |
|---|---|
| [`kernel-platform.md`](components/kernel-platform.md) | the BSP core of the vendor kernel 4.9.227 (Mercury6) |
| [`kernel-drivers.md`](components/kernel-drivers.md) | the BSP peripheral drivers of the vendor kernel and `kdrv_sdmmc.ko` |
| [`usb-gadget.md`](components/usb-gadget.md) | the USB device port: `udc-msb250x.ko`, `usb_f_uvc.ko`, `g_webcam.ko` |
| [`sensor-gc8613.md`](components/sensor-gc8613.md) | the vendor GC8613 sensor driver `gc8613_MIPI.ko` |

## MI layer: kernel modules and their user libraries

| document | covers |
|---|---|
| [`mi-user-abi.md`](components/mi-user-abi.md) | the user-space ABI shared by the MI libraries; the calls prog_uvc makes in §10 |
| [`mi_sys-1-engine.md`](components/mi_sys-1-engine.md) | the mi_sys pipeline engine |
| [`mi_sys-2-interfaces.md`](components/mi_sys-2-interfaces.md) | mi_common and the interfaces of mi_sys |
| [`mi-capture.md`](components/mi-capture.md) | mi_sensor, mi_vif, mi_isp, mi_scl and their user libraries |
| [`mi_disp-mi_hdmi.md`](components/mi_disp-mi_hdmi.md) | mi_disp, mi_hdmi and their user libraries |
| [`mi_venc.md`](components/mi_venc.md) | mi_venc.ko and libmi_venc.so |
| [`mi_jpd-mi_vdec.md`](components/mi_jpd-mi_vdec.md) | the MI decoders (mi_jpd.ko, mi_vdec.ko) and their user libraries |
| [`rgn-gop.md`](components/rgn-gop.md) | mi_rgn, libmi_rgn, the mhal RGN block (GOP, OSD blender, cover) and fbdev.ko |
| [`gfx-ge.md`](components/gfx-ge.md) | mi_gfx, libmi_gfx and the mhal GE block |
| [`mi_vdisp-mi_shadow.md`](components/mi_vdisp-mi_shadow.md) | mi_vdisp, mi_shadow and their user libraries |
| [`ldc-1-stack.md`](components/ldc-1-stack.md) | mi_ldc.ko, libmi_ldc, mhal LDC, the LDC config and calibration file formats |
| [`ldc-2-eptz-lib.md`](components/ldc-2-eptz-lib.md) | the map-generation (eptz) library inside mi_ldc.ko: table layout, tiling, projection and lens models, text configuration, MVE and Simd |
| [`audio.md`](components/audio.md) | mi_ai, mi_ao, libmi_ai, libmi_ao and mhal AUDIO ("bach") |
| [`panel-mipitx.md`](components/panel-mipitx.md) | mi_panel, mi_mipitx, libmi_panel, libmi_mipitx, mhal PNL and MIPITX, board.ini / Customer.ini |
| [`ipu-host.md`](components/ipu-host.md) | mi_ipu.ko, libmi_ipu.so, mhal IPU, the CCIF mailbox (host side) |

## mhal.ko: hardware blocks

| document | covers |
|---|---|
| [`mhal-core.md`](components/mhal-core.md) | mhal.ko as a module |
| [`mhal-csi-snr.md`](components/mhal-csi-snr.md) | mhal sensor interface (SNR) and MIPI CSI-2 receiver (CSI) |
| [`mhal-vif.md`](components/mhal-vif.md) | the VIF (video input) block of mhal.ko |
| [`mhal-scl-jpe-cmdq.md`](components/mhal-scl-jpe-cmdq.md) | the SCL scaler, the JPE MJPEG encoder and the CMDQ command queues of mhal.ko |
| [`mhal-disp.md`](components/mhal-disp.md) | the DISP (display engine) block of mhal.ko |
| [`mhal-hdmitx.md`](components/mhal-hdmitx.md) | the HDMITX block of mhal.ko |
| [`mhal-jpd.md`](components/mhal-jpd.md) | the NJPD JPEG decoder block of mhal.ko |
| [`mhal-vcodec.md`](components/mhal-vcodec.md) | the Wave5 video codec in mhal.ko (VCODEC glue, Wave5 host library, VENC / VDEC wrappers) |

## ISP

| document | covers |
|---|---|
| [`isp-1-hal.md`](components/isp-1-hal.md) | the ISP hal/driver layer of mhal.ko |
| [`isp-2-ispmid-mload-ispscl.md`](components/isp-2-ispmid-mload-ispscl.md) | ispmid, ISP statistics, MLOAD and ISPSCL of mhal.ko |
| [`isp-3-iq-api.md`](components/isp-3-iq-api.md) | the ISP API ids of mhal.ko: routing, payloads, converters; the API bin |
| [`isp-4-iq-engine.md`](components/isp-4-iq-engine.md) | the ISP IQ engine of mhal.ko |
| [`isp-5-3a.md`](components/isp-5-3a.md) | the SigmaStar AE / AWB / AF library: in-kernel copy and libispalgo |
| [`isp-6-userspace.md`](components/isp-6-userspace.md) | libcus3a, libsigma3a, libmi_iqserver, libfbc_decode, isp_api.xml |

## Coprocessor firmware

| document | covers |
|---|---|
| [`chagall-fw.md`](components/chagall-fw.md) | `chagall.bin`, the Wave5 codec firmware: container, image map, VCPU instruction set, program map, firmware side of the host interface, VCORE engine segments |
| [`ipu-firmware-1-runtime-model.md`](components/ipu-firmware-1-runtime-model.md) | `ipu_firmware.bin` part 1: image, RISC-V runtime, CCIF server, host commands, SGSI model file, option tables, offline command block |
| [`ipu-firmware-2-engines.md`](components/ipu-firmware-2-engines.md) | `ipu_firmware.bin` part 2: interpreter and op resolver, IPU200M HAL and compute engines, operator kernels, the Anjoy build |

## Applications and user-space libraries

| document | covers |
|---|---|
| [`prog_uvc.md`](components/prog_uvc.md) | the vendor pipeline app (SigmaStar UVC demo with the ODM's additions) |
| [`kjd_lvgl.md`](components/kjd_lvgl.md) | the on-screen menu app (LVGL) |
| [`ive-video-analytics.md`](components/ive-video-analytics.md) | libmi_ive (MI_IVE API and the MVE core) and the video analytics libraries libMD, libOD, libVG |
| [`audio-algorithms.md`](components/audio-algorithms.md) | libSRC, libSSL, libAEC, libAPC, libAED, libBF and the mechanics of the nine algorithm libraries |
| [`sdk-orphan-libs.md`](components/sdk-orphan-libs.md) | the unused SDK libraries of `/config/lib` |
