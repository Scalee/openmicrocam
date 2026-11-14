#!/bin/sh
# One-boot trial of a sensor driver on the camera: /customer/fast/drv.try names the driver file (consumed by that boot,
# camera/boot-hook.sh), reboot, wait for the USB network and microcamd (cam wait), report the mode and the trial's check.
# usage: boottry.sh DRIVER.ko     (a file in /customer/fast)
# A hang in the trial comes back on the usual driver after the watchdog reset; a trial without USB network reboots itself
# at +100 s. The host's USB stream is released before the reboot and restored after (docs/TOOLS.md «Hooks»).
cd "$(dirname "$0")/../.." || exit 1
eval "$(python3 tools/hostconf.py --sh)" || exit 1
S=tools/cam
DRV=${1:?usage: boottry.sh DRIVER.ko}
REL=; [ -n "$MICROCAM_HOOK_STREAM_RELEASE" ] && sh -c "$MICROCAM_HOOK_STREAM_RELEASE" >/dev/null 2>&1 && REL=1
back() { [ -z "$REL" ] || sh -c "$MICROCAM_HOOK_STREAM_RESTORE" >/dev/null 2>&1 || echo "stream-restore hook failed"; }
$S/cam sh -t 20 "cd /customer/fast && [ -f $DRV ] && echo $DRV > drv.try && sync && echo RB && { (sleep 1; reboot) >/dev/null 2>&1 & true; }" | grep -q RB || { echo "could not arm the trial (is /customer/fast/$DRV on the camera?)"; back; exit 1; }
for i in $(seq 1 40); do ping -c1 -W1 "$MICROCAM_CAMERA" >/dev/null || break; sleep 0.5; done   # until it is gone
$S/cam wait up 180 || { echo "NO NET after the trial boot"; back; exit 2; }
$S/cam wait microcamd 60
back
$S/cam sh -t 40 "dmesg | grep SetVideoRes | tail -1; cat /customer/fast/trial.log 2>/dev/null; timeout -t 8 grep -A2 SrcFrmrate /proc/mi_modules/mi_isp/mi_isp0 | sed -n 2p"
