# OpenMicroCam hook for the IMX678 camera (black YIZHAN 4K), sourced by the last line of /customer/set.sh, which the
# stock boot script sources just before it starts the vendor app (./customer/WS_H &). Every change is a copy in
# /customer/omc bind-mounted over the vendor's file, which stays as it is on the flash; a copy is used only when both it
# and the vendor's file have the checksums they were made for. Settings: /customer/omc/omc.conf. What it did: /tmp/omc.log.
#  OMC_MICROCAMD  microcamd in place of the vendor app: mcd/start.sh bind-mounted over /customer/WS_H, every file of mcd/
#                 checked against mcd/md5.txt; `try` = the next boot only. Three boots in a row that never reach a minute
#                 of microcamd (counted in mcd/starts) set it back to 0.
#  OMC_RNDIS      USB network: usb0 192.168.1.100/24, MTU 400, and telnetd. With the vendor app: its USB plugin patched
#                 to load the gadget with RNDIS; with microcamd: start.sh loads it so.
# Off for one boot: a file OMC-STOCK in the SD card's root. Removed for good: delete the hook's line from /customer/set.sh.
OMC=/customer/omc
OMC_RNDIS=1
OMC_MICROCAMD=0
[ -f $OMC/omc.conf ] && . $OMC/omc.conf
# `try`: set back to 0 before anything is applied, so a power cycle undoes a boot that fails
if [ "$OMC_MICROCAMD" = try ]; then
  sed -i 's/^OMC_MICROCAMD=try/OMC_MICROCAMD=0/' $OMC/omc.conf && sync && OMC_MICROCAMD=1
fi
if [ -e /dev/mmcblk0 ]; then
  for omc_i in 1 2 3 4 5 6 7 8 9 10; do grep -q ' /mnt ' /proc/mounts && break; usleep 200000; done
  [ -f /mnt/OMC-STOCK ] && OMC_RNDIS=0 && OMC_MICROCAMD=0
fi
echo "omc: RNDIS=$OMC_RNDIS MICROCAMD=$OMC_MICROCAMD" > /tmp/omc.log

omc_same() { [ -f "$1" ] && [ "$(md5sum "$1" | cut -c1-32)" = "$2" ]; }

if [ "$OMC_MICROCAMD" = 1 ]; then
  omc_n=$(cat $OMC/mcd/starts 2>/dev/null)
  case "$omc_n" in [0-9]) ;; *) omc_n=0 ;; esac
  omc_ok=1
  if [ -f $OMC/mcd/md5.txt ]; then
    while read omc_sum omc_name; do
      omc_same "$OMC/mcd/$omc_name" "$omc_sum" || { omc_ok=0; echo "omc: mcd/$omc_name differs" >> /tmp/omc.log; }
    done < $OMC/mcd/md5.txt
  else
    omc_ok=0
  fi
  omc_same $OMC/g_webcam.ko bc46b4246feeb2fd183220c942dec128 || omc_ok=0
  [ -x $OMC/mcd/start.sh ] && [ -x $OMC/mcd/microcamd ] || omc_ok=0
  if [ $omc_ok = 0 ]; then
    echo "omc: microcamd skipped: a checksum differs or a file is not executable" >> /tmp/omc.log; OMC_MICROCAMD=0
  elif [ $omc_n -ge 3 ]; then
    echo "omc: microcamd off: three boots without a minute of it" >> /tmp/omc.log
    sed -i 's/^OMC_MICROCAMD=.*/OMC_MICROCAMD=0/' $OMC/omc.conf; rm -f $OMC/mcd/starts; sync; OMC_MICROCAMD=0
  else
    echo $((omc_n + 1)) > $OMC/mcd/starts; sync
    echo "OMC_RNDIS=$OMC_RNDIS" > /tmp/omc.env
    mount -o bind $OMC/mcd/start.sh /customer/WS_H && echo "omc: microcamd in place of the vendor app" >> /tmp/omc.log ||
      OMC_MICROCAMD=0
  fi
fi

if [ "$OMC_RNDIS" = 1 ] && [ "$OMC_MICROCAMD" != 1 ]; then
  if omc_same /customer/lib/plugins/libmain_uvc.so 1089cf35639ec8c696d1b06591576a62 && omc_same $OMC/libmain_uvc.so 9863cf91c10420f36d9827704eaf1078; then
    mount -o bind $OMC/libmain_uvc.so /customer/lib/plugins/libmain_uvc.so && echo "omc: RNDIS for the vendor app" >> /tmp/omc.log
  else
    echo "omc: RNDIS skipped: a checksum differs" >> /tmp/omc.log; OMC_RNDIS=0
  fi
fi
if [ "$OMC_RNDIS" = 1 ]; then
  # the stock boot mounts /dev/pts only at a console login; telnet needs it. The vendor's USB driver drops frames larger
  # than one USB packet, hence MTU 400 (set it on the computer's side as well)
  grep -q ' /dev/pts ' /proc/mounts || { mkdir -p /dev/pts; mount -t devpts devpts /dev/pts; }
  ( while true; do
      if ifconfig usb0 >/dev/null 2>&1 && ! ifconfig usb0 | grep -q 'inet addr:192.168.1.100'; then
        ifconfig usb0 192.168.1.100 netmask 255.255.255.0 mtu 400 up
      fi
      pidof telnetd >/dev/null || telnetd
      sleep 2
    done ) &
fi
