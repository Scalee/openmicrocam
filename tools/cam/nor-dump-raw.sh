#!/bin/sh
# Dump the camera's 16 MB NOR flash from the U-Boot prompt as raw sectors on the SD card (this U-Boot's fatwrite breaks
# the card's FAT). The image goes to the card at LBA 0x100000 (512 MB in), 0x8000 blocks; U-Boot reads it back and
# compares, and prints the CRC32 of both copies. Read it on the laptop with
#   dd if=/dev/<card> of=nor.bin bs=512 skip=1048576 count=32768
# Buffers 0x21000000 and 0x22000000 (U-Boot lives at 0x23E00000). Needs `uart.py bridge` and the U-Boot prompt
# (uboot-rescue.sh shows how to catch it); the console's answers go to $MICROCAM_LOGS/uboot-nor-raw-<time>.txt.
D=$(dirname "$0")
eval "$(python3 "$D/../hostconf.py" --sh)" || exit 1
OUT=$MICROCAM_LOGS/uboot-nor-raw-$(date +%Y-%m-%d-%H%M).txt
run() {
  python3 "$D/uart.py" send "$1" -w "${2:-60}" --until 'SigmaStar #' >>"$OUT" || { echo "TIMEOUT: $1" | tee -a "$OUT"; return 1; }
}
run 'mmc rescan' 15 && run 'mmcinfo' 15 &&
run 'sf probe 0' 15 &&
run 'sf read 0x21000000 0 0x1000000' 60 &&
run 'crc32 0x21000000 0x1000000' 30 &&
run 'mmc write 0x21000000 0x100000 0x8000' 180 &&
run 'mw.b 0x22000000 0 0x1000000' 30 &&
run 'mmc read 0x22000000 0x100000 0x8000' 180 &&
run 'cmp.b 0x21000000 0x22000000 0x1000000' 60 &&
run 'crc32 0x22000000 0x1000000' 30
echo "saved $OUT"; grep -aE 'CRC32|crc32|blocks|same|differ|TIMEOUT|rror' "$OUT"
