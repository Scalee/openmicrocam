#!/bin/sh
# Link-time copies of the camera's MI libraries. The originals (in the camera's dump) carry no DT_SONAME, so a linker
# would record their full laptop path as NEEDED; the copies get SONAME = file name. Only the linker reads these; the
# camera runs its own. usage: mk-linklibs.sh   (settings, docs/TOOLS.md «Settings»: MICROCAM_SYSROOT in, MICROCAM_LINK_LIBS out)
set -e
eval "$(python3 "$(dirname "$0")/hostconf.py" --sh)"
S=${MICROCAM_SYSROOT:?MICROCAM_SYSROOT=<dump dir with miservice/lib>}
D=${MICROCAM_LINK_LIBS:?MICROCAM_LINK_LIBS=<a directory for the link-time copies>}
# a dump whose libraries or kernel differ from the tested camera's builds a microcamd that fails only on the camera
python3 "$(dirname "$0")/install/checkparts.py" "$S" --for build --quiet >&2 || exit 1
mkdir -p "$D"
for l in mi_common cam_os_wrapper mi_sys mi_sensor mi_vif mi_isp cus3a ispalgo mi_scl mi_venc mi_disp mi_hdmi mi_rgn mi_gfx mi_vdec mi_jpd; do
  f=lib$l.so
  cp "$S/miservice/lib/$f" "$D/$f"
  patchelf --set-soname "$f" "$D/$f"
done
ls "$D" | wc -l; echo "link libs in $D"
