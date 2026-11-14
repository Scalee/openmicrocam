# Our build of the SigmaStar MSB250x USB device controller driver for the YIZHAN 4K camera's 4.9.227 kernel (licence: LICENSE).
# Base: SigmaStar's Infinity6E 4.9 driver (github.com/tipoman9/sigmastar-infinity6e, drivers/sstar/usb/gadget/udc/usb20);
# the Mercury6 register header comes from the Infinity6C 5.10 tree (OpenIPC/linux, sigmastar-infinity6c), its register bases
# equal the Infinity6E ones (docs/firmware/components/usb-gadget.md §2.1). Build: kmods/build-ko.sh kmods/udc-msb250x.
# The tree must have CONFIG_CACHE_L2X0 off (so no OUTER_CACHE/ARM_HEAVY_MB): the camera kernel has no outer cache and does not
# export arm_heavy_mb, which readb/writeb barriers would otherwise import (the first load failed on it).
# It must have CONFIG_CMA + CONFIG_DMA_CMA on (the camera boots with cma=2M): without them struct device lacks cma_area and
# of_node sits 4 bytes earlier than in the camera kernel; the probe then read a null of_node and got irq 0 (second load).
# Check after a config change: the `ldr.w r0,[r0,#368]` before irq_of_parse_and_map must match the vendor module's 0x170.
# It must have CONFIG_USB_GADGET=m: without it linux/usb/gadget.h turns usb_ep_set_maxpacket_limit into an empty inline, every
# endpoint keeps maxpacket 0 and EP0 answers descriptor reads with empty packets (host: error -71; the third load).
# `nm -u udc-msb250x.ko` must list usb_ep_set_maxpacket_limit.
ccflags-y += -march=armv7-a -I$(src)/include/mercury6 -I$(src)/include -I$(src)/include/sstar -I$(srctree)/drivers/usb/gadget/udc
# SigmaStar sets this in every usbcam defconfig; the camera's vendor build lacks it and panics on RNDIS at MTU 1500.
ccflags-y += -DCONFIG_USB_AVOID_SHORT_PACKET_IN_BULK_OUT_WITH_DMA_FOR_ETHERNET
obj-m := udc-msb250x.o
udc-msb250x-y := src/msb250x_gadget.o src/msb250x_dma.o src/msb250x_ep.o src/msb250x_udc.o src/microcam_trace.o
