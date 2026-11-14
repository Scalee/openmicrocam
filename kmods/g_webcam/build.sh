#!/bin/sh
# Build g_webcam.ko from source (SigmaStar's legacy/webcam.c, LICENSE) with the UVC descriptors gen_desc.py takes from the
# stock module in your camera's dump ($MICROCAM_SYSROOT/miservice/modules/4.9.227/g_webcam.ko), against $KTREE.
# The module shows microcamd its format list (/sys/module/g_webcam/parameters/formats): MJPEG 4K45 by default, the SD
# card as a mass-storage LUN. It goes to /customer/microcam/g_webcam.ko on the camera, from $MICROCAM_STAGE.
# usage: build.sh   (settings: docs/TOOLS.md)
set -e
cd "$(dirname "$0")"
eval "$(python3 ../../tools/hostconf.py --sh)"
K=${KTREE:?KTREE=<the prepared linux-4.9.227 tree>}
STOCK=${MICROCAM_SYSROOT:?MICROCAM_SYSROOT=<your camera dump, unpacked>}/miservice/modules/4.9.227/g_webcam.ko
ST=${MICROCAM_STAGE:-$(cd ../.. && pwd)/stage}
python3 gen_desc.py "$STOCK" yz_desc.h
LOG=$(make -C "$K" M="$PWD" ARCH=arm CROSS_COMPILE=arm-linux-gnueabihf- modules 2>&1) || { echo "$LOG"; exit 1; }
echo "$LOG" | grep -E 'warning: .*yz' || true
mkdir -p "$ST" && cp g_webcam.ko "$ST/"
sha256sum g_webcam.ko
