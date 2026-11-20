#!/bin/sh
# Bring up the laptop side of the camera's USB-RNDIS link (after every camera reboot the interface gets a new name; the
# old one may linger without a carrier, so each enx* is tried until the camera answers). Needs `sudo ip`
# and leaves the interface unmanaged by NetworkManager when nmcli is there.
# usage: cam-net.sh   -> prints the interface and whether the camera answers (addresses: docs/TOOLS.md)
eval "$(python3 "$(dirname "$0")/../hostconf.py" --sh)" || exit 1
C=$MICROCAM_CAMERA H=$MICROCAM_HOST
IFS_=$(ip -br link | awk '/^enx/{print $1}')
[ -z "$IFS_" ] && { echo "no enx* network interface"; exit 1; }
for IF in $IFS_; do
  nmcli dev set "$IF" managed no >/dev/null 2>&1
  ip -4 addr show "$IF" | grep -q "inet $H/" || sudo ip addr add "$H/24" dev "$IF"
  sudo ip link set "$IF" up
  sleep 1
  if ping -c1 -W2 -I "$IF" "$C" >/dev/null 2>&1; then
    for O in $IFS_; do [ "$O" = "$IF" ] || sudo ip addr del "$H/24" dev "$O" 2>/dev/null; done
    # another interface holding the same network takes the camera's route, and every tool loses it
    R=$(ip route get "$C" | sed -n 's/.* dev \([^ ]*\).*/\1/p;q')
    [ "$R" = "$IF" ] || { echo "$IF: camera answers, but the route to $C goes through $R: take $H off $R"; exit 1; }
    echo "$IF: camera answers"; exit 0
  fi
done
echo "$(echo $IFS_ | tr ' ' ','): no answer from $C"; exit 1
