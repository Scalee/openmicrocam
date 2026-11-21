#!/bin/sh
# OpenMicroCam install for the IMX678 camera (black YIZHAN 4K), from the SD card that tools/mkcard-imx678.py builds. Runs
# once as PID 1 of the one-time boot the card's H161X.bin starts: checks that the camera has the firmware this was made
# for, makes the patched copies (patches/) and installs /customer/omc with microcamd (mcd/), adds one line to
# /customer/set.sh, logs to OMC/install.log on the card and reboots. Nothing of the vendor's files is changed but that one
# line; the original set.sh is kept as /customer/omc/set.sh.orig.
C=/mnt/OMC
OMC=/customer/omc

same() { [ -f "$1" ] && [ "$(md5sum "$1" | cut -c1-32)" = "$2" ]; }

# omc_patch NAME: the patched copy of the file `# target` names, as $OMC/NAME, from patches/NAME.patch (lines `offset hex`)
omc_patch() {
  p=$C/patches/$1.patch
  target=$(sed -n 's/^# target //p' $p); from=$(sed -n 's/^# from //p' $p); to=$(sed -n 's/^# to //p' $p)
  same "$target" "$from" || { echo "FAIL: $target is not the file the patch was made for"; return 1; }
  cp "$target" $OMC/$1.part || return 1
  grep -v '^#' $p | while read off hex; do
    n=$((${#hex} / 2))
    printf "$(echo "$hex" | sed 's/\(..\)/\\x\1/g')" > /tmp/omc-bytes
    [ "$(wc -c < /tmp/omc-bytes)" -eq $n ] || { echo "FAIL: printf"; exit 1; }
    dd if=/tmp/omc-bytes of=$OMC/$1.part bs=1 seek=$off count=$n conv=notrunc 2>/dev/null || exit 1
  done || return 1
  same $OMC/$1.part "$to" || { echo "FAIL: patched $1 is not as expected"; rm -f $OMC/$1.part; return 1; }
  mv $OMC/$1.part $OMC/$1 && echo "patched copy $OMC/$1"
}

omc_install() {
  echo "=== OpenMicroCam install, uptime $(cut -d' ' -f1 /proc/uptime) s"
  # the card inert for the vendor's sd_version_recognition whatever U-Boot's fatwrite managed
  rm -f /mnt/sd_version.txt
  mount -t ubifs ubi0:customer /customer || { echo "FAIL: mount /customer"; return 1; }
  if ! grep -q /customer/omc/hook.sh /customer/set.sh; then
    same /customer/set.sh d5edba794177cc343683fd4bd8c234de || {
      echo "FAIL: /customer/set.sh is not the stock one; nothing changed"; return 1; }
  fi
  mkdir -p $OMC/mcd || { echo "FAIL: mkdir"; return 1; }
  omc_patch g_webcam.ko || return 1
  omc_patch libmain_uvc.so || return 1
  # a network-only card has no mcd/: microcamd stays as it is (its omc.conf says the vendor app)
  if [ -d $C/mcd ]; then
    cp $C/mcd/* $OMC/mcd/ && chmod 755 $OMC/mcd/microcamd $OMC/mcd/microcam $OMC/mcd/start.sh || { echo "FAIL: copy mcd"; return 1; }
    while read sum name; do
      same $OMC/mcd/$name $sum || { echo "FAIL: mcd/$name copied wrong"; return 1; }
    done < $OMC/mcd/md5.txt
    rm -f $OMC/mcd/starts
  fi
  cp $C/hook.sh $OMC/hook.sh && cp $C/omc.conf $OMC/omc.conf || { echo "FAIL: copy hook.sh / omc.conf"; return 1; }
  [ -f $OMC/set.sh.orig ] || cp /customer/set.sh $OMC/set.sh.orig
  if grep -q /customer/omc/hook.sh /customer/set.sh; then
    echo "set.sh already has the hook line"
  else
    printf '[ -f /customer/omc/hook.sh ] && . /customer/omc/hook.sh\n' >> /customer/set.sh
    echo "hook line added to set.sh"
  fi
  sync
  grep ^OMC $OMC/omc.conf
  echo OK
}

omc_install > $C/install.log 2>&1 || echo "install FAILED (reason above)" >> $C/install.log
sync
umount /customer 2>/dev/null
echo "rebooting, uptime $(cut -d' ' -f1 /proc/uptime) s" >> $C/install.log
sync
reboot -f
sleep 10
echo b > /proc/sysrq-trigger
while true; do sleep 60; done
