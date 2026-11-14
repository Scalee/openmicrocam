#!/bin/sh
# Write one flash region of the YIZHAN 4K camera back from your own dump, through U-Boot over the serial console: the
# region goes onto the SD card's raw sectors from the laptop, U-Boot reads it from there, checks its CRC32, erases and
# writes the flash range, reads it back and checks again. For a region checkparts.py reports as differing from the dump,
# or `customer` for /customer exactly as dumped (install.py uninstall restores files instead).
# usage: restore-region.sh REGION CARD   step 1: the region onto the card in the laptop's reader (CARD = /dev/sdX)
#        restore-region.sh REGION        step 2: the card in the camera, `tools/cam/uart.py bridge` running
# The dump is the unpacked one MICROCAM_SYSROOT names (docs/TOOLS.md; default work/dump).
# REGION: KERNEL, rootfs, miservice, customer, MISC. The loaders, the partition table, U-Boot and its environment are
# refused: a failed write there leaves a camera that does not start, and only an SPI flash programmer brings it back.
# docs/INSTALL.md «Over the serial console (UART)».
set -e
D=$(cd "$(dirname "$0")" && pwd)
eval "$(python3 "$D/../hostconf.py" --sh)"
DUMP=$MICROCAM_SYSROOT
REGION=${1:?usage: restore-region.sh REGION [CARD]}
CARD=$2
[ -f "$DUMP/nor.bin" ] || { echo "restore-region: no $DUMP/nor.bin (MICROCAM_SYSROOT): dump the camera first"; exit 1; }
case $REGION in
KERNEL) OFF=0x70000; SIZE=0x210000 ;;
rootfs) OFF=0x280000; SIZE=0x200000 ;;
miservice) OFF=0x480000; SIZE=0x400000 ;;
customer) OFF=0x880000; SIZE=0x610000 ;;
MISC) OFF=0xE90000; SIZE=0xA0000 ;;
*) echo "restore-region: $REGION is not one this tool writes"; exit 1 ;;
esac
LBA=0x100000                       # 512 MB into the card, where dump.sh puts the dump
BLOCKS=$(printf '0x%x' $((SIZE / 512)))
IMG=$(mktemp)
trap 'rm -f "$IMG"' EXIT
dd if="$DUMP/nor.bin" of="$IMG" bs=4096 skip=$((OFF / 4096)) count=$((SIZE / 4096)) status=none
CRC=$(python3 -c "import binascii,sys; print('%08x' % binascii.crc32(open(sys.argv[1],'rb').read()))" "$IMG")
if [ -n "$CARD" ]; then
  [ -b "$CARD" ] || { echo "restore-region: $CARD is not a block device"; exit 1; }
  [ -w "$CARD" ] || SUDO=sudo   # a card's block device is root's on most systems
  $SUDO dd if="$IMG" of="$CARD" bs=512 seek=$((LBA)) conv=fsync status=none
  echo "$REGION ($SIZE bytes, CRC32 $CRC) is on the card at 512 MB. Card into the camera, then: $0 $REGION"
  exit 0
fi
U="python3 $D/../cam/uart.py"
run() {
  out=$($U send "$1" -w "${2:-60}" --until 'SigmaStar #') || { echo "restore-region: no answer to «$1»"; exit 1; }
  echo "$out"
}
crc_of() { run "crc32 $1 $SIZE" 60 | grep -aoiE '==> [0-9a-f]{8}' | cut -c5-; }
echo "[$(date +%T)] power-cycle the camera now; sending Enter every 50 ms for up to 120 s"
$U send '' -w 120 --every 0.05 --until 'SigmaStar #' >/dev/null || { echo "restore-region: no U-Boot prompt"; exit 1; }
run 'mmc rescan' 15 >/dev/null
run 'sf probe 0' 15 >/dev/null
run "mmc read 0x21000000 $LBA $BLOCKS" 180 >/dev/null
GOT=$(crc_of 0x21000000)
[ "$GOT" = "$CRC" ] || { echo "restore-region: the card holds CRC32 $GOT, the dump's $REGION is $CRC: nothing written"; exit 1; }
echo "[$(date +%T)] the card's copy is right; erasing and writing $REGION at $OFF"
run "sf erase $OFF $SIZE" 300 >/dev/null
run "sf write 0x21000000 $OFF $SIZE" 300 >/dev/null
run "mw.b 0x22000000 0 $SIZE" 30 >/dev/null
run "sf read 0x22000000 $OFF $SIZE" 120 >/dev/null
GOT=$(crc_of 0x22000000)
if [ "$GOT" != "$CRC" ]; then
  echo "restore-region: the flash reads back CRC32 $GOT, not $CRC — do not power off; run step 2 again"; exit 1
fi
echo "[$(date +%T)] $REGION written and read back, CRC32 $CRC; booting"
$U send 'reset' -w 2 >/dev/null || true
