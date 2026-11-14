# OpenMicroCam: start microcamd in place of the vendor's application. microcam.try = this boot only, microcam.ok = always.
# Sourced by the boot (camera/boot-hook.sh) where the vendor's application would start; FASTDIR comes from there, USBTRY
# from udc-insmod.sh. Sets MICROCAM=1 when microcamd starts; otherwise the boot goes on to the vendor's application. The
# composite gadget is loaded here, before microcamd, which reads its format list, because the vendor's application, not
# the boot script, used to load it.
MICROCAM=
if [ -f /customer/microcam/microcamd.xz ] && { [ -f $FASTDIR/microcam.ok ] || [ -f $FASTDIR/microcam.try ]; }; then
  rm -f $FASTDIR/microcam.try; sync
  mkdir -p /tmp/microcam && cd /tmp/microcam && unxz -c /customer/microcam/microcamd.xz > microcamd && chmod +x microcamd && cp /customer/microcam/*.toml . && MICROCAM=1
  cd /
  # fb0's driver is the vendor app's: microcamd draws HDMI's menu into its frames and never loads it;
  # its reserve goes back to the media heap. Its window (U-Boot's logo) is hidden first: unloaded, fbdev leaves it on
  /customer/microcam/fbhide
  rmmod fbdev
  # the SD card is microcamd's: it mounts and unmounts it itself, so the vendor's mdev hook is made to do nothing
  printf '#!/bin/sh\n' > /tmp/microcam/sd_hotplug.sh && chmod +x /tmp/microcam/sd_hotplug.sh && mount -o bind /tmp/microcam/sd_hotplug.sh /customer/sd_hotplug.sh
  # microcamd's own gadget (MJPEG 4K45 the default; the card's mass-storage LUN). USBTRY (udc-insmod.sh): this boot
  # tries g_webcam.try.ko
  insmod /config/modules/4.9.227/usb_f_mass_storage.ko
  GW=/config/modules/4.9.227/g_webcam.ko; [ -f /customer/microcam/g_webcam.ko ] && GW=/customer/microcam/g_webcam.ko
  [ -n "$USBTRY" ] && [ -f /customer/microcam/g_webcam.try.ko ] && GW=/customer/microcam/g_webcam.try.ko
  insmod $GW streaming_maxpacket=3072 uvc_function_enable=1 uac_function_enable=0 rndis_function_enable=1
  ifconfig usb0 up; ifconfig usb0 192.168.1.100 netmask 255.255.0.0
fi
if [ -n "$MICROCAM" ]; then
  # microcamd's stderr lives in RAM; on flash only when asked for (log.flash), since every line there is a NOR write
  LOG=/tmp/microcamd.log; [ -f $FASTDIR/log.flash ] && LOG=/customer/microcamd-boot.log
  # one line into the kernel's log, which microcamd's log ring reads (`microcam log`)
  echo "<6>microcam-boot: microcamd starting, $(cd $FASTDIR && ls *.ok 2>/dev/null | tr '\n' ' ')" > /dev/kmsg
  # clocks that can only be set before microcamd creates its encoder device (venc: "no support set clk, pls set before createdev"):
  # /customer/fast/clocks, one line "<node under /sys/devices/virtual/mstar> <value>" (e.g. "venc/ven_clock 480000000"); absent = defaults
  [ -f $FASTDIR/clocks ] && while read CLKN CLKV; do [ -n "$CLKV" ] && echo $CLKV > /sys/devices/virtual/mstar/$CLKN; done < $FASTDIR/clocks
  /tmp/microcam/microcamd /tmp/microcam/config.toml >$LOG 2>&1 &
  # the menu, a client of microcamd's socket (it waits for it), started again whenever it ends — it ends itself when its
  # loop hangs (alarm) or microcamd speaks another protocol: the pause doubles up to 30 s while it keeps ending at once,
  # back to 1 s after a run of a minute; each start's uptime in its log
  if [ -f /customer/microcam/microcam-menu.xz ] && unxz -c /customer/microcam/microcam-menu.xz > /tmp/microcam/microcam-menu && chmod +x /tmp/microcam/microcam-menu; then
    ( P=1; while :; do
        T=$(cut -d. -f1 /proc/uptime); echo "menu start at $T s" >> /tmp/microcam-menu.log
        /tmp/microcam/microcam-menu /tmp/microcam.sock >> /tmp/microcam-menu.log 2>&1
        echo "menu ended: $?" >> /tmp/microcam-menu.log
        [ $(( $(cut -d. -f1 /proc/uptime) - T )) -ge 60 ] && P=1
        sleep $P; [ $P -lt 30 ] && P=$((P * 2)); [ $P -gt 30 ] && P=30
      done ) </dev/null >/dev/null 2>&1 &
  fi
fi
