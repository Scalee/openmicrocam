#!/bin/sh
# OpenMicroCam from a microSD card (`install.py install|uninstall --card`): the card's U-Boot script boots Linux once with
# this as PID 1, from the card's OPENMICROCAM/ folder (/mnt, mounted by the boot line; /config mounted, /dev populated),
# where the bundle and camera/install.sh lie. It mounts the rest a normal boot gives install.sh, runs install.sh JOB on
# the bundle, logs everything to OPENMICROCAM/install.log and always ends in a reboot; the boot's bootargs were set in
# U-Boot's RAM only, so the next boot is the camera's usual one.
# usage (the card's boot line): sh /mnt/OPENMICROCAM/run.sh install|uninstall
D=/mnt/OPENMICROCAM
JOB=$1
export PATH=/sbin:/usr/sbin:/bin:/usr/bin
# a redirection that fails would end PID 1 (a kernel panic, no reboot): the log is tried first
touch $D/install.log || reboot -f
exec >$D/install.log 2>&1
echo "OpenMicroCam card job: $JOB"
mount -t proc proc /proc
mount -t sysfs sys /sys
mount -t tmpfs tmp /tmp
mount -t tmpfs var /var && mkdir -p /var/lock   # fw_printenv's lock file, as the stock rcS makes it
mount -t jffs2 mtd:customer /customer
case $JOB in
install|uninstall) sh $D/install.sh $JOB $D; rc=$? ;;
*) echo "FAIL run.sh: no job «$JOB»"; rc=2 ;;
esac
if [ $rc = 0 ]; then echo "RESULT ok: $JOB done; rebooting"; else echo "RESULT FAIL: $JOB stopped at the step above (exit $rc); rebooting"; fi
sync
# the open log would keep the card busy
exec >/dev/console 2>&1
umount /customer
umount /mnt
reboot -f
