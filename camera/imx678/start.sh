#!/bin/sh
# OpenMicroCam's microcamd in place of the vendor app on the IMX678 camera (black YIZHAN 4K). hook.sh bind-mounts this
# file over /customer/WS_H when OMC_MICROCAMD is on, so the stock boot script's `./customer/WS_H &` starts it: the USB
# gadget (the patched g_webcam.ko, RNDIS as OMC_RNDIS says), OpenMicroCam's modules, the IMX678 driver in place of the
# vendor's, then microcamd, restarted when it exits.
# Fallback: three exits within a minute of their start set OMC_MICROCAMD=0 and reboot to the vendor app (hook.sh does the
# same after three boots that never reach a minute). Logs in RAM only: /tmp/omc.log, /tmp/omc/.
M=/customer/omc/mcd
OMC=/customer/omc
L=/tmp/omc
# the SD hotplug script signals `pidof WS_H` (this script) on card insert / removal: ignored here, reset for microcamd
trap '' USR1 USR2 HUP
mkdir -p $L
exec >> /tmp/omc.log 2>&1
export LD_LIBRARY_PATH=/lib:/config/lib
OMC_RNDIS=1
[ -f /tmp/omc.env ] && . /tmp/omc.env
up() { cut -d. -f1 /proc/uptime; }
fallback() {
  echo "omc: $(up) s: microcamd: $1; OMC_MICROCAMD=0, rebooting to the vendor app"
  sed -i 's/^OMC_MICROCAMD=.*/OMC_MICROCAMD=0/' $OMC/omc.conf
  rm -f $M/starts
  sync; sleep 1; reboot; exit 1
}
echo "omc: $(up) s: microcamd start, RNDIS=$OMC_RNDIS"

# the boot animation (started by /etc/init.sh) holds the display; the vendor app stops it as well
killall bootAnim 2>/dev/null
for i in 1 2 3 4 5; do pidof bootAnim >/dev/null || break; sleep 1; done

# UVC interfaces: RNDIS takes 0 and 1 when it is there
if [ "$OMC_RNDIS" = 1 ]; then VC=2; VS=3; else VC=0; VS=1; fi
sed -e "s/^vc_interface = .*/vc_interface = $VC/" -e "s/^vs_interface = .*/vs_interface = $VS/" $M/config.toml > $L/config.toml

insmod $OMC/g_webcam.ko streaming_maxpacket=3072 uvc_function_enable=1 uac_function_enable=0 rndis_function_enable=$OMC_RNDIS ||
  fallback "the USB gadget did not load"
insmod $M/microcam_sys.ko || fallback "microcam_sys did not load"
insmod $M/microcam_canvas.ko || fallback "microcam_canvas did not load"
echo 480000000 > /sys/devices/virtual/mstar/mscl/clk
# kernel errors off the serial console (written synchronously at 115200 baud); they stay in dmesg
echo 3 > /proc/sys/kernel/printk
rmmod drv_ms_cus_imx678_MIPI || fallback "the vendor's sensor driver did not unload"
insmod $M/drv_ms_cus_imx678_MIPI.ko chmap=1 lane_num=4 || fallback "our sensor driver did not load"
echo "omc: $(up) s: gadget, modules and sensor driver in place; isp clk $(cat /sys/devices/virtual/mstar/isp0/isp_clk)"

quick=0
while true; do
  t0=$(up)
  ( trap - USR1 USR2 HUP; exec $M/microcamd $L/config.toml ) > $L/microcamd.log 2>&1 &
  pid=$!
  echo "omc: $t0 s: microcamd pid $pid"
  # a minute up: this boot counts as good
  ( sleep 60; kill -0 $pid 2>/dev/null && rm -f $M/starts && sync ) &
  wait $pid
  rc=$?
  t=$(( $(up) - t0 ))
  echo "omc: $(up) s: microcamd exited rc=$rc after $t s"
  cp $L/microcamd.log $L/microcamd.prev.log
  if [ $t -lt 60 ]; then
    quick=$((quick + 1))
    [ $quick -ge 3 ] && fallback "three exits within a minute"
  else
    quick=0
  fi
  sleep 2
done
