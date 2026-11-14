#!/bin/bash
# Build a C tester against the camera's own libraries (the dump's glibc and MI libs) and run it on the camera from
# /tmp: the tester's output and exit code. usage: ctester.sh <file.c> [args...]
#   RENV="NAME=value …" sets its environment on the camera; TMO=<secs> bounds the run (default 90).
#   The tester includes "mi.h" (crates/mi-sys: the MI declarations microcamd uses). Settings (docs/TOOLS.md):
#   MICROCAM_SYSROOT (the dump), the camera's and the laptop's addresses.
# A tester beside microcamd puts every VENC buffer back and catches SIGTERM: its exit takes the devices it opened with it.
set -e
R=$(cd "$(dirname "$0")/../.." && pwd)
eval "$(python3 "$R/tools/hostconf.py" --sh)"
SRC=$(realpath "${1:?file.c}"); shift
N=$(basename "$SRC" .c)
D=${MICROCAM_SYSROOT:?MICROCAM_SYSROOT=<unpacked camera dump with rootfs/lib and miservice/lib>}; S=/usr/arm-linux-gnueabihf/lib
W=$(mktemp -d -p "${TMPDIR:-/tmp}" ctester.XXXXXX)
trap 'rm -rf "$W"; [ -n "$SRV" ] && kill $SRV 2>/dev/null' EXIT
arm-linux-gnueabihf-gcc -std=gnu11 -O2 -Wall -Wno-unused-variable -U_TIME_BITS -D_TIME_BITS=32 -U_FILE_OFFSET_BITS \
  -I"$R/crates/mi-sys" -c "$SRC" -o "$W/$N.o"
arm-linux-gnueabihf-gcc -nostdlib -Wl,--no-as-needed -o "$W/$N" $S/crt1.o $S/crti.o "$W/$N.o" -L$D/miservice/lib \
  -lmi_gfx -lmi_disp -lmi_vdisp -lmi_venc -lmi_rgn -lmi_jpd -lmi_vdec -lmi_ldc -lmi_isp -lcus3a -lmi_scl -lmi_sensor -lmi_vif -lmi_sys \
  -lmi_common -lcam_os_wrapper $D/rootfs/lib/libc.so.6 $D/rootfs/lib/libm.so.6 $D/rootfs/lib/libpthread.so.0 \
  $D/rootfs/lib/ld-linux-armhf.so.3 -lgcc $S/crtn.o -Wl,--allow-shlib-undefined -Wl,-rpath,/config/lib \
  -Wl,--dynamic-linker=/lib/ld-linux-armhf.so.3
python3 -m http.server 8011 --bind "$MICROCAM_HOST" -d "$W" >/dev/null 2>&1 & SRV=$!
sleep 0.5
set +e
"$R/tools/cam/cam" sh -t "${TMO:-90}" "cd /tmp && rm -f $N && wget -q http://$MICROCAM_HOST:8011/$N && chmod +x $N && ${RENV:-} ./$N $*"
