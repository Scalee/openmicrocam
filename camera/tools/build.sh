#!/bin/sh
# Build a camera-side C tool: host cross-gcc, but linked against the camera's own glibc 2.30 and MI libs from its dump.
# usage: build.sh <tool.c> [-lLIB…]   (-> the binary beside it, also copied to $MICROCAM_STAGE; -lLIB: a library of the dump's
#        miservice/lib the tool needs beyond the common set below); settings: docs/TOOLS.md «Settings»
set -e
eval "$(python3 "$(dirname "$0")/../../tools/hostconf.py" --sh)"
SRC=$(cd "$(dirname "$1")" && pwd)/$(basename "$1" .c); shift
ST=${MICROCAM_STAGE:-$(cd "$(dirname "$0")/../.." && pwd)/stage}
D=${MICROCAM_SYSROOT:?MICROCAM_SYSROOT=<unpacked camera dump with rootfs/lib and miservice/lib>}; S=/usr/arm-linux-gnueabihf/lib
arm-linux-gnueabihf-gcc -std=gnu11 -O2 -U_TIME_BITS -D_TIME_BITS=32 -U_FILE_OFFSET_BITS -c $SRC.c -o $SRC.o
arm-linux-gnueabihf-gcc -nostdlib -Wl,--no-as-needed -o $SRC $S/crt1.o $S/crti.o $SRC.o -L$D/miservice/lib \
  -lmi_isp -lmi_sys -lmi_common -lmi_disp -lmi_hdmi -lmi_scl -lcam_os_wrapper -lcus3a -lispalgo "$@" $D/rootfs/lib/libc.so.6 $D/rootfs/lib/ld-linux-armhf.so.3 \
  -lgcc $S/crtn.o -Wl,--allow-shlib-undefined -Wl,-rpath,/config/lib -Wl,--dynamic-linker=/lib/ld-linux-armhf.so.3
mkdir -p "$ST"; cp $SRC "$ST/"
