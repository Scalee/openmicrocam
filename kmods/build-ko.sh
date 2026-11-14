#!/bin/sh
# Build a kernel module directory against the prepared vanilla 4.9.227 tree ($KTREE; configured to the camera's module
# ABI: vermagic "4.9.227 SMP preempt mod_unload ARMv7 thumb2 p2v8", struct module 512 B with init @0xe8 / exit @0x1dc).
# Usage: build-ko.sh <module dir> ; the .ko is also copied to $MICROCAM_STAGE. Settings: docs/TOOLS.md «Settings».
set -e
eval "$(python3 "$(dirname "$0")/../tools/hostconf.py" --sh)"
K=${KTREE:?KTREE=<the prepared linux-4.9.227 tree>}
ST=${MICROCAM_STAGE:-$(cd "$(dirname "$0")/.." && pwd)/stage}
D=$(cd "$1" && pwd)
mkdir -p "$ST"
make -C "$K" M="$D" ARCH=arm CROSS_COMPILE=arm-linux-gnueabihf- modules 2>&1 | grep -v 'Symbol version dump'
for f in "$D"/*.ko; do
  cp "$f" "$ST/"
  echo "== $(basename "$f")"; modinfo "$f" | grep -E 'vermagic|depends'
  arm-linux-gnueabihf-nm -u "$f" | sort
done
