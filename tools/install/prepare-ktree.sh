#!/bin/sh
# Prepare a Linux 4.9.227 tree for building OpenMicroCam's kernel modules against the YIZHAN 4K camera's stock kernel:
# vanilla 4.9.227 from kernel.org (checked by sha256), kernel-4.9.227.defconfig beside this script, `make modules_prepare`,
# then a probe module checks what the stock modules show: the vermagic string and the layout of struct module.
# usage: prepare-ktree.sh [DIR]  → DIR/linux-4.9.227; DIR defaults to the directory of KTREE (work/build here), and a
#        DIR of your own is then named by KTREE=DIR/linux-4.9.227 (docs/TOOLS.md «Settings»)
# Needs the arm-linux-gnueabihf cross gcc, make, gcc, flex, bison, bc, curl, xz. Re-running it reuses the download.
set -e
V=4.9.227
SHA=38c485b411a14e825f8a66a5c2e64f1480397325a5f692b4929f68d228070c5f
URL=https://cdn.kernel.org/pub/linux/kernel/v4.x/linux-$V.tar.xz
HERE=$(cd "$(dirname "$0")" && pwd)
eval "$(python3 "$HERE/../hostconf.py" --sh)"
DIR=${1:-$(dirname "$KTREE")}
mkdir -p "$DIR"; DIR=$(cd "$DIR" && pwd)
T=$DIR/linux-$V.tar.xz
K=$DIR/linux-$V
M="make -s -C $K ARCH=arm CROSS_COMPILE=arm-linux-gnueabihf-"
if ! echo "$SHA  $T" | sha256sum -c --status 2>/dev/null; then
  echo "fetching $URL"
  curl -fL --progress-bar -o "$T.part" "$URL"
  echo "$SHA  $T.part" | sha256sum -c --status || { echo "prepare-ktree: the download's sha256 is not $SHA"; exit 1; }
  mv "$T.part" "$T"
fi
[ -d "$K" ] || { echo "unpacking"; tar -xf "$T" -C "$DIR"; }
cp "$HERE/kernel-$V.defconfig" "$K/.config"   # kconfig reads its own comment lines and skips the rest
$M olddefconfig >/dev/null 2>&1
for want in 'CONFIG_PREEMPT=y' 'CONFIG_THUMB2_KERNEL=y' 'CONFIG_ARM_MODULE_PLTS=y' '# CONFIG_MODVERSIONS is not set' \
            '# CONFIG_CACHE_L2X0 is not set' 'CONFIG_DMA_CMA=y' 'CONFIG_USB_GADGET=m' 'CONFIG_DEBUG_SPINLOCK=y'; do
  grep -qx "$want" "$K/.config" || { echo "prepare-ktree: the config lost «$want»"; exit 1; }
done
echo "modules_prepare"
$M modules_prepare 2>&1 | grep -E 'error|Error' && exit 1
# the probe: refuses to build unless struct module has the stock layout (512 bytes, init at 0xe8, exit at 0x1dc)
P=$(mktemp -d)
trap 'rm -rf "$P"' EXIT
cat > "$P/probe.c" <<'C'
#include <linux/module.h>
#include <linux/bug.h>
static int __init probe_init(void)
{
	BUILD_BUG_ON(sizeof(struct module) != 512);
	BUILD_BUG_ON(offsetof(struct module, init) != 0xe8);
	BUILD_BUG_ON(offsetof(struct module, exit) != 0x1dc);
	return 0;
}
module_init(probe_init);
MODULE_LICENSE("GPL");
C
echo 'obj-m := probe.o' > "$P/Kbuild"
$M M="$P" modules >"$P/log" 2>&1 || { tail -5 "$P/log"; echo "prepare-ktree: struct module differs from the stock kernel's"; exit 1; }
VM=$(modinfo -F vermagic "$P/probe.ko" | sed 's/ *$//')
[ "$VM" = "4.9.227 SMP preempt mod_unload ARMv7 thumb2 p2v8" ] || { echo "prepare-ktree: vermagic «$VM»"; exit 1; }
echo "ready: $K, vermagic «$VM», struct module 512 B, init @0xe8, exit @0x1dc"
