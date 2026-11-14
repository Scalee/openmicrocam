#!/bin/sh
# The serial way of `install.py dump --serial`: the camera's whole 16 MB NOR flash into OUTDIR/nor.bin. U-Boot over the
# serial console copies the flash to raw sectors of the SD card (tools/cam/nor-dump-raw.sh), the laptop reads them back
# from the card and checks them against U-Boot's CRC32; install.py then unpacks and compares the dump.
# usage: dump.sh OUTDIR [READER]
#   step 1 (no READER): the U-Boot part. Needs `tools/cam/uart.py bridge` running and a microSD card in the camera whose
#          contents from 512 MB on may be overwritten. Power-cycle the camera when asked.
#   step 2 (READER = the card's whole block device on the laptop, e.g. /dev/sdb): read and check.
# docs/INSTALL.md «Over the serial console (UART)».
set -e
D=$(cd "$(dirname "$0")" && pwd)
OUT=${1:?usage: dump.sh OUTDIR [READER]}
READER=$2
mkdir -p "$OUT"
U="python3 $D/../cam/uart.py"
LBA=1048576      # the image's place on the card: 512 MB in, as nor-dump-raw.sh writes it
BLOCKS=32768     # 16 MB in 512-byte blocks
if [ -z "$READER" ]; then
  echo "[$(date +%T)] power-cycle the camera now; sending Enter every 50 ms for up to 120 s"
  $U send '' -w 120 --every 0.05 --until 'SigmaStar #' >/dev/null || { echo "dump: no U-Boot prompt (is uart.py bridge running?)"; exit 1; }
  echo "[$(date +%T)] U-Boot prompt caught; copying the flash to the card (a few minutes)"
  LOG=$("$D/../cam/nor-dump-raw.sh" | sed -n 's/^saved //p')
  [ -f "$LOG" ] || { echo "dump: nor-dump-raw.sh left no log"; exit 1; }
  CRCS=$(grep -aoiE 'crc32 for [0-9a-fx]+ \.\.\. [0-9a-fx]+ ==> [0-9a-f]{8}' "$LOG" | awk '{print $NF}' | sort -u)
  N=$(echo "$CRCS" | grep -c . || true)
  if [ "$N" != 1 ] || grep -aq 'TIMEOUT\|differ' "$LOG"; then echo "dump: U-Boot's copy is not clean, see $LOG"; exit 1; fi
  echo "$CRCS" > "$OUT/uboot-crc32.txt"
  echo "[$(date +%T)] on the card, CRC32 $CRCS (both copies). Power the camera off, move the card to the laptop, run:"
  echo "  $D/install.py dump $OUT --serial --reader /dev/<the card>"
  exit 0
fi
[ -f "$OUT/uboot-crc32.txt" ] || { echo "dump: run step 1 first (no $OUT/uboot-crc32.txt)"; exit 1; }
[ -b "$READER" ] || { echo "dump: $READER is not a block device"; exit 1; }
echo "[$(date +%T)] reading 16 MB from $READER at 512 MB"
[ -r "$READER" ] || SUDO=sudo   # a card's block device is root's on most systems
$SUDO dd if="$READER" bs=512 skip=$LBA count=$BLOCKS status=none > "$OUT/nor.bin"
WANT=$(cat "$OUT/uboot-crc32.txt")
GOT=$(python3 -c "import binascii,sys; print('%08x' % binascii.crc32(open(sys.argv[1],'rb').read()))" "$OUT/nor.bin")
[ "$GOT" = "$WANT" ] || { echo "dump: CRC32 $GOT on the laptop, $WANT in U-Boot: read it again"; exit 1; }
echo "[$(date +%T)] CRC32 $GOT matches U-Boot's; sha256 $(sha256sum "$OUT/nor.bin" | cut -c1-64)"
