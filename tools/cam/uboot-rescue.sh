#!/bin/sh
# Rescue over the serial console when /customer/demo.sh cannot run: catch the U-Boot prompt (Enter every 50 ms from
# power-up), boot once with init=/bin/sh (RAM only, U-Boot's environment untouched), mount the customer jffs2, run the
# given fix command, sync. Needs `uart.py bridge` running. usage: uboot-rescue.sh "<fix command>"
U="python3 $(dirname "$0")/uart.py"
FIX=${1:?usage: uboot-rescue.sh "<fix command>"}
echo "[$(date +%T)] power-cycle the camera now; sending Enter every 50 ms for up to 120 s"
$U send '' -w 120 --every 0.05 --until 'SigmaStar #' >/dev/null || { echo "no U-Boot prompt"; exit 1; }
echo "[$(date +%T)] U-Boot prompt caught"
sleep 1
# the stock boot line with init=/bin/sh (the stock media heap; this boot runs no video)
$U send 'setenv bootargs console=ttyS0,115200 root=/dev/mtdblock2 rootfstype=squashfs ro init=/bin/sh LX_MEM=0xffe0000 mma_heap=mma_heap_name0,miu=0,sz=0x9000000 mma_memblock_remove=1 cma=2M@0x23800000 mmap_reserved=fb,miu=0,sz=0x800000,max_start_off=0xF700000,max_end_off=0xFF00000' -w 1 >/dev/null
$U send 'run bootcmd' -w 60 --until '(^/ # |sh: can.t access tty|# $)' >/dev/null || { echo "no shell after the boot"; exit 1; }
sleep 3
echo "[$(date +%T)] shell, mounting /customer"
$U send 'mount -t proc proc /proc; mount -t jffs2 mtd:customer /customer; mount | grep customer' -w 4
$U send "$FIX; sync; ls -la /customer/demo.sh; echo RESCUE_DONE" -w 30 --until '^RESCUE_DONE' | tail -12
