# OpenMicroCam boot hook for the YIZHAN 4K camera's own /customer/demo.sh, which sources it as its second line and calls
# microcam_start just before the line that would start the vendor's application (camera/install.sh puts both lines in and
# takes them out again). The rest of the camera's boot script runs as the vendor wrote it; this hook changes it only
#   - as the stock module list is loaded: an insmod wrapper puts our modules in place of the vendor's where ours are
#     installed and switched on, and loads ours that have no vendor counterpart;
#   - the login, when `install.py access` (the serial fallback) set one: /customer/shadow.custom over the read-only
#     /etc/shadow; without it the camera keeps its stock login;
#   - at microcam_start: microcamd starts instead of the vendor's application, and demo.sh ends there; the SD card
#     script's name (U-Boot's SdUpgradeImage) is set back to H161X.bin when it is not, as the rest of demo.sh would.
# A file MICROCAM-STOCK in the SD card's root leaves the whole boot the vendor's own (no module of ours, no login, the
# vendor's application); so does a camera without microcamd on its flash, but for the login.
# Switches in /customer/fast: NAME.ok = always, NAME.try = the next boot only (consumed by it):
#   microcam  start microcamd (app-start.sh)       rgn  our RGN driver in place of mi_rgn.ko
#   udc       our USB controller driver; usb.try tries udc-msb250x.try.ko and g_webcam.try.ko (udc-insmod.sh)
#   enabled   our sensor driver /customer/fast/drv_ms_cus_gc8613_MIPI.ko; drv.try names another driver file there
# A trial boot (drv.try, usb.try) reboots by itself when it has no USB network 100 s after start, back onto the usual set.
MICROCAM_DIR=/customer/microcam
FASTDIR=/customer/fast
MICROCAM_STOCK=

# decided once, at the first module of ours: by then the card is detected and mdev mounts it on /mnt
microcam_decide() {
  [ -n "$MICROCAM_STOCK" ] && return 0
  MICROCAM_STOCK=0
  if [ -e /dev/mmcblk0 ]; then
    for i in 1 2 3 4 5 6 7 8 9 10; do grep -q ' /mnt ' /proc/mounts && break; usleep 200000; done
    [ -f /mnt/MICROCAM-STOCK ] && MICROCAM_STOCK=1
  fi
  [ $MICROCAM_STOCK = 1 ] && return 0
  [ -f /customer/shadow.custom ] && mount -o bind /customer/shadow.custom /etc/shadow
  # KERN_ERR off the serial console (it stays in dmesg and microcamd's log): the console is written synchronously at
  # 115200 baud, and an error reported per frame there holds the USB stream below its rate
  [ -f $MICROCAM_DIR/microcamd.xz ] && echo 3 > /proc/sys/kernel/printk
  return 0
}

# the stock list's insmod lines come through here
insmod() {
  case "$1" in
  */mi_rgn.ko)
    microcam_decide
    if [ $MICROCAM_STOCK = 0 ] && [ -f $MICROCAM_DIR/microcam_rgn.ko ] && { [ -f $FASTDIR/rgn.ok ] || [ -f $FASTDIR/rgn.try ]; }; then
      rm -f $FASTDIR/rgn.try; sync
      shift; set -- $MICROCAM_DIR/microcam_rgn.ko "$@"
    fi ;;
  */mi_vdisp.ko)
    microcam_decide
    command insmod "$@"
    # ours without a vendor counterpart, once the MI modules they call into are in
    if [ $MICROCAM_STOCK = 0 ]; then
      for m in microcam_sys microcam_canvas; do [ -f $MICROCAM_DIR/$m.ko ] && command insmod $MICROCAM_DIR/$m.ko; done
    fi
    return 0 ;;
  */gc8613_MIPI.ko)
    microcam_decide
    DRV=$1
    if [ $MICROCAM_STOCK = 0 ] && [ -f $FASTDIR/enabled ] && [ -f $FASTDIR/drv_ms_cus_gc8613_MIPI.ko ]; then
      OURS=$FASTDIR/drv_ms_cus_gc8613_MIPI.ko
      if [ -f $FASTDIR/drv.try ]; then OURS=$FASTDIR/$(cat $FASTDIR/drv.try); rm -f $FASTDIR/drv.try; sync; TRIAL=1; fi
      shift
      # the vendor's driver when ours does not load: no sensor driver at all leaves the camera without video
      if command insmod $OURS "$@"; then DRV=$OURS; return 0; fi
      set -- "$DRV" "$@"
    fi ;;
  */udc-msb250x.ko)
    microcam_decide
    # udc-insmod.sh picks the driver and loads it with insmod, which is this function again: its own calls go through
    if [ -z "$MICROCAM_UDC" ] && [ $MICROCAM_STOCK = 0 ] && [ -f $MICROCAM_DIR/udc-insmod.sh ]; then
      MICROCAM_UDC=1; . $MICROCAM_DIR/udc-insmod.sh; MICROCAM_UDC=; return 0
    fi ;;
  esac
  command insmod "$@"
}

# where demo.sh would start the vendor's application: microcamd instead, and the end of demo.sh
microcam_start() {
  microcam_decide
  [ $MICROCAM_STOCK = 0 ] && [ -f $MICROCAM_DIR/app-start.sh ] || return 0
  . $MICROCAM_DIR/app-start.sh    # MICROCAM=1 when it started microcamd
  [ -n "$MICROCAM" ] || return 0
  # scaler clock 432 → 480 MHz: at 432 the scaler cannot write a 3840-wide output at the line time of the fast 4K modes
  echo 480000000 > /sys/devices/virtual/mstar/mscl/clk
  # the card script's name: a U-Boot reset saves another one, and demo.sh's line that puts it back does not run now
  [ "$(/etc/fw_printenv SdUpgradeImage 2>/dev/null)" = SdUpgradeImage=H161X.bin ] || /etc/fw_setenv SdUpgradeImage H161X.bin
  if [ -n "$TRIAL" ]; then
    (sleep 100
     if lsmod | grep -q '^g_webcam' && ifconfig usb0 2>/dev/null | grep -q 'inet addr'; then echo ok > $FASTDIR/trial.log
     else echo "no usb net, reboot" > $FASTDIR/trial.log; sync; reboot; fi) &
  fi
  exit 0
}
