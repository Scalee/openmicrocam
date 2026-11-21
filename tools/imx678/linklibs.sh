#!/bin/sh
# Link-time copies of the IMX678 camera's MI libraries, as tools/mk-linklibs.sh makes them for the GC8613 camera, from a
# copy of the camera's /config/lib (its firmware is another SDK build, so mk-linklibs.sh's check of the tested camera's
# files refuses it). The originals carry no DT_SONAME; the copies get SONAME = file name. Only the linker reads them.
# usage: linklibs.sh CAMERA_LIB   (out: MICROCAM_LINK_LIBS, docs/TOOLS.md «Settings»)
set -e
eval "$(python3 "$(dirname "$0")/../hostconf.py" --sh)"
S=${1:?usage: linklibs.sh <a copy of the camera's /config/lib>}
D=${MICROCAM_LINK_LIBS:?MICROCAM_LINK_LIBS=<a directory for the link-time copies>}
mkdir -p "$D"
for l in mi_common cam_os_wrapper mi_sys mi_sensor mi_vif mi_isp cus3a ispalgo mi_scl mi_venc mi_disp mi_hdmi mi_rgn mi_gfx mi_vdec mi_jpd; do
  f=lib$l.so
  cp "$S/$f" "$D/$f"
  patchelf --set-soname "$f" "$D/$f"
done
ls "$D" | wc -l; echo "link libs in $D"
