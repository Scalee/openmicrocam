#!/bin/sh
# Write the firmware of the YIZHAN 4K camera back from your own dump, through U-Boot over the serial console. Step 1
# puts everything from the kernel to MISC (the five regions below, which lie end to end) onto the SD card's raw sectors
# from the laptop. Step 2: U-Boot reads it from the card, checks its CRC32, erases and writes the flash range, reads it
# back and checks again — all of it, or only the REGION named: one checkparts.py reports as differing from the dump, or
# `customer` for /customer exactly as dumped (install.py uninstall restores files instead).
# usage: restore-region.sh CARD       step 1: the card in the laptop's reader (CARD = /dev/sdX)
#        restore-region.sh [REGION]   step 2: the card in the camera, `tools/cam/uart.py bridge` running
# The dump is the unpacked one MICROCAM_SYSROOT names (docs/TOOLS.md; default work/dump).
# REGION: KERNEL, rootfs, miservice, customer, MISC. The loaders, the partition table, U-Boot and its environment are
# never written: a failed write there leaves a camera that does not start, and only an SPI flash programmer brings it
# back.
# docs/INSTALL.md «Over the serial console (UART)».
set -e
D=$(cd "$(dirname "$0")" && pwd)
eval "$(python3 "$D/../hostconf.py" --sh)"
DUMP=$MICROCAM_SYSROOT
[ -f "$DUMP/nor.bin" ] || { echo "restore-region: no $DUMP/nor.bin (MICROCAM_SYSROOT): dump the camera first"; exit 1; }
ALL_OFF=0x70000; ALL_SIZE=0xEC0000   # from the kernel to the end of MISC
CARD_LBA=0x100000                    # 512 MB into the card, where dump.sh puts the dump
case $1 in /dev/*) CARD=$1; REGION=$2 ;; *) CARD=$2; REGION=$1 ;; esac
if [ -n "$CARD" ]; then
  [ -z "$REGION" ] || { echo "restore-region: the card always gets the whole firmware; name $REGION in the second step, with the card in the camera"; exit 1; }
  [ -b "$CARD" ] || { echo "restore-region: $CARD is not a block device"; exit 1; }
  [ -w "$CARD" ] || SUDO=sudo   # a card's block device is root's on most systems
  dd if="$DUMP/nor.bin" bs=4096 skip=$((ALL_OFF / 4096)) count=$((ALL_SIZE / 4096)) status=none |
    $SUDO dd of="$CARD" bs=512 seek=$((CARD_LBA)) conv=fsync status=none
  echo "the firmware is on the card. Next: put the card into the camera and run: $0"
  exit 0
fi
case $REGION in
'') OFF=$ALL_OFF; SIZE=$ALL_SIZE ;;
KERNEL) OFF=0x70000; SIZE=0x210000 ;;
rootfs) OFF=0x280000; SIZE=0x200000 ;;
miservice) OFF=0x480000; SIZE=0x400000 ;;
customer) OFF=0x880000; SIZE=0x610000 ;;
MISC) OFF=0xE90000; SIZE=0xA0000 ;;
*) echo "restore-region: $REGION is not one this tool writes (KERNEL, rootfs, miservice, customer, MISC)"; exit 1 ;;
esac
WHAT=${REGION:-the firmware}
LBA=$(printf '0x%x' $((CARD_LBA + (OFF - ALL_OFF) / 512)))   # the region's place inside the card's copy
BLOCKS=$(printf '0x%x' $((SIZE / 512)))
CRC=$(dd if="$DUMP/nor.bin" bs=4096 skip=$((OFF / 4096)) count=$((SIZE / 4096)) status=none |
  python3 -c "import binascii,sys; print('%08x' % binascii.crc32(sys.stdin.buffer.read()))")
U="python3 $D/../cam/uart.py"
run() {
  out=$($U send "$1" -w "${2:-60}" --until 'SigmaStar #') || { echo "restore-region: no answer to «$1»"; exit 1; }
  echo "$out"
}
crc_of() { run "crc32 $1 $SIZE" 60 | grep -aoiE '==> [0-9a-f]{8}' | cut -c5-; }
$U uboot || { echo "restore-region: no U-Boot prompt"; exit 1; }
run 'mmc rescan' 15 >/dev/null
run 'sf probe 0' 15 >/dev/null
run "mmc read 0x21000000 $LBA $BLOCKS" 180 >/dev/null
GOT=$(crc_of 0x21000000)
[ "$GOT" = "$CRC" ] || { echo "restore-region: the card's copy has CRC32 $GOT, the dump's $CRC: nothing written. Put the firmware onto the card again (step 1)"; exit 1; }
echo "[$(date +%T)] the card's copy is right; erasing and writing $WHAT at $OFF: do not touch the camera's power"
run "sf erase $OFF $SIZE" 300 >/dev/null
run "sf write 0x21000000 $OFF $SIZE" 300 >/dev/null
run "mw.b 0x22000000 0 $SIZE" 30 >/dev/null
run "sf read 0x22000000 $OFF $SIZE" 120 >/dev/null
GOT=$(crc_of 0x22000000)
if [ "$GOT" != "$CRC" ]; then
  echo "restore-region: the flash reads back CRC32 $GOT, not $CRC — do not power off; run step 2 again"; exit 1
fi
echo "[$(date +%T)] $WHAT written and read back, CRC32 $CRC; booting"
$U send 'reset' -w 2 >/dev/null || true
