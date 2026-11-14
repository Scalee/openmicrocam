#!/bin/sh
# What the camera will see of a built binary: interpreter, NEEDED, GLIBC symbol versions, arch, size.
# usage: tools/elfcheck.sh target/armv7-unknown-linux-gnueabihf/release/microcamd
set -e
export LC_ALL=C
B=${1:?binary}
RE=arm-linux-gnueabihf-readelf; OD=arm-linux-gnueabihf-objdump
echo "== $B  $(stat -c %s "$B") bytes"
$RE -l "$B" | grep -i "interpreter"
$RE -d "$B" | grep -E "NEEDED|RPATH|RUNPATH" | sed 's/^ *//'
echo "-- GLIBC versions required (must be <= 2.30):"
$OD -T "$B" | grep -o "GLIBC_[0-9.]*" | sort -t. -k2,2n -k3,3n -u | tr '\n' ' '; echo
echo "-- highest: $($OD -T "$B" | grep -o 'GLIBC_[0-9.]*' | sort -t. -k2,2n -k3,3n -u | tail -1)"
echo "-- arch:"; $RE -A "$B" | grep -E "Tag_CPU_name|Tag_CPU_arch:|Tag_FP_arch|Tag_ABI_VFP_args|Tag_THUMB" | sed 's/^ *//'
echo "-- undefined MI symbols resolved by the camera libs:"; $OD -T "$B" | grep -c "\*UND\*.*MI_" || true
