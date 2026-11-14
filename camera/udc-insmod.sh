# Own USB device-controller driver (OpenMicroCam, 2026-10-24). udc.try = this boot only, udc.ok = permanent; else vendor.
# Sourced by the boot hook (boot-hook.sh) in place of the stock `insmod /config/modules/4.9.227/udc-msb250x.ko` line
# (FASTDIR is set there).
# usb.try: this boot only, the trial driver udc-msb250x.try.ko and, in app-start.sh, the trial gadget g_webcam.try.ko;
# TRIAL makes the hook's late check reboot (onto the usual modules) when the USB network did not come up.
UDC=/config/modules/4.9.227/udc-msb250x.ko
if [ -f /customer/microcam/udc-msb250x.ko ] && { [ -f $FASTDIR/udc.ok ] || [ -f $FASTDIR/udc.try ]; }; then rm -f $FASTDIR/udc.try; sync; UDC=/customer/microcam/udc-msb250x.ko; fi
USBTRY=; if [ -f $FASTDIR/usb.try ]; then rm -f $FASTDIR/usb.try; sync; USBTRY=1; TRIAL=1; fi
[ -n "$USBTRY" ] && [ -f /customer/microcam/udc-msb250x.try.ko ] && UDC=/customer/microcam/udc-msb250x.try.ko
insmod $UDC || insmod /config/modules/4.9.227/udc-msb250x.ko
