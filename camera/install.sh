#!/bin/sh
# OpenMicroCam installer on the YIZHAN 4K camera itself (busybox sh). tools/install/install.py uploads a bundle into a
# directory in RAM and runs it from there; each step can also be run alone, and running one again changes nothing.
# usage: install.sh install BUNDLE          check, files, flags, demo, bootargs, verify — stops at the first failure
#        install.sh check|files|flags|access|demo|bootargs|verify BUNDLE   (access: install.py access, the serial fallback)
#        install.sh status
#        install.sh patch-demo SRC DST      the camera's demo.sh with the two hook lines, into DST
#        install.sh uninstall BUNDLE [-n]  back to the stock firmware's files and bootargs (-n: only say what it would do)
# BUNDLE holds microcamd.xz, microcam-menu.xz, the tables, the scripts and modules for /customer/microcam (whatever of them was built),
# stock.sha256 (the stock files that must be the tested ones: tools/install/stock-parts.tsv), and, as a step needs them,
# shadow.line (the root line with the owner's password hash), bootargs.stock and stock/ (the dump's files, for uninstall).
# Each step prints «ok <step>: …» or «FAIL <step>: …» (exit 1).
MC=/customer/microcam
FAST=/customer/fast
DEMO=/customer/demo.sh
SETENV=/etc/fw_setenv
PRINTENV=/etc/fw_printenv
HEAP_OURS=0xB000000          # the media heap microcamd needs: 176 MB of the 256 MB DRAM (the stock boot gives 144 MB)
MARK='# OpenMicroCam'
LINE_A="[ -f $MC/boot-hook.sh ] && . $MC/boot-hook.sh  $MARK"
LINE_B="type microcam_start >/dev/null 2>&1 && microcam_start  $MARK"
# what goes where: the sensor driver lives beside its switch in /customer/fast, the rest in /customer/microcam
FILES="microcamd.xz microcam-menu.xz config.toml policy.toml app-start.sh udc-insmod.sh boot-hook.sh fbhide iq_file_gc8613.bin
microcam_sys.ko microcam_canvas.ko microcam_rgn.ko udc-msb250x.ko g_webcam.ko drv_ms_cus_gc8613_MIPI.ko"

ok() { echo "ok $STEP: $*"; }
fail() { echo "FAIL $STEP: $*"; exit 1; }
dest() { [ "$1" = drv_ms_cus_gc8613_MIPI.ko ] && echo $FAST/$1 || echo $MC/$1; }
sha() { sha256sum "$1" 2>/dev/null | cut -c1-64; }
stock_sha() { grep "  $1\$" "$B/stock.sha256" 2>/dev/null | cut -c1-64; }
heap() { $PRINTENV bootargs 2>/dev/null | sed -n 's/.*mma_heap=[^ ]*sz=\(0x[0-9A-Fa-f]*\).*/\1/p'; }

# the camera's demo.sh without our two lines
unpatch() { grep -v "$MARK\$" "$1"; }

patch_demo() {   # SRC DST
  n=$(grep -n 'fw_setenv\|prog_uvc' "$1" | head -1 | cut -d: -f1)
  [ -n "$n" ] && [ "$n" -gt 1 ] || return 1
  awk -v a="$LINE_A" -v b="$LINE_B" -v n="$n" 'NR == n { print b } { print } NR == 1 { print a }' "$1" > "$2.new" &&
    sh -n "$2.new" && chmod 755 "$2.new" && mv "$2.new" "$2"
}

# stock | ours | other: what the camera's demo.sh is
demo_state() {
  s=$(stock_sha /customer/demo.sh)
  [ -n "$s" ] || { echo unknown; return; }
  if [ "$(sha $DEMO)" = "$s" ]; then echo stock
  elif [ "$(grep -c "$MARK\$" $DEMO)" = 2 ] && [ "$(unpatch $DEMO | sha256sum | cut -c1-64)" = "$s" ]; then echo ours
  else echo other; fi
}

step_check() {
  [ -f "$B/stock.sha256" ] || fail "no stock.sha256 in $B"
  bad=$(sha256sum -c "$B/stock.sha256" 2>/dev/null | grep -v ': OK$' | grep -v '/customer/demo.sh:' | grep -v '/customer/sd_hotplug.sh:')
  [ -z "$bad" ] || fail "these stock files differ from the tested camera's — microcamd and our modules are built for those:
$bad"
  need=0
  for f in $FILES; do
    [ -f "$B/$f" ] || continue
    s=$(wc -c < "$B/$f"); o=0; [ -f "$(dest $f)" ] && o=$(wc -c < "$(dest $f)"); need=$((need + s - o))
  done
  free=$(df /customer | tail -1 | awk '{print $4 * 1024}')
  [ $need -lt $free ] || fail "/customer has $free bytes free, the files need $need more"
  ok "stock files as tested, $((free / 1024)) KB free, $((need / 1024)) KB more needed"
}

step_files() {
  mkdir -p $MC $FAST || fail "mkdir"
  for f in microcamd.xz microcam-menu.xz config.toml policy.toml app-start.sh boot-hook.sh fbhide microcam_sys.ko microcam_canvas.ko; do
    [ -f "$B/$f" ] || fail "the bundle has no $f"
  done
  n=0; same=0
  for f in $FILES; do
    [ -f "$B/$f" ] || continue
    d=$(dest $f)
    if cmp -s "$B/$f" "$d"; then same=$((same + 1)); continue; fi
    # straight over the old one: the partition has no room for both copies of microcamd or of the menu at once
    cp "$B/$f" "$d" && sync && cmp -s "$B/$f" "$d" || fail "$f did not land whole"
    n=$((n + 1))
  done
  chmod 755 $MC/fbhide
  sync
  ok "$n written, $same already in place"
}

step_flags() {
  on="microcam.ok"
  [ -f $MC/microcam_rgn.ko ] && on="$on rgn.ok"
  [ -f $MC/udc-msb250x.ko ] && on="$on udc.ok"
  [ -f $FAST/drv_ms_cus_gc8613_MIPI.ko ] && on="$on enabled"
  for f in $on; do [ -f $FAST/$f ] || touch $FAST/$f || fail "touch $f"; done
  sync
  ok "$on"
}

step_access() {
  [ -f "$B/shadow.line" ] || fail "no shadow.line in $B"
  grep -q '^root:\$[156]\$[^:]*\$[^:]*:' "$B/shadow.line" || fail "shadow.line is not a root line with a crypt hash"
  if cmp -s "$B/shadow.line" /customer/shadow.custom; then ok "unchanged"; return; fi
  cp "$B/shadow.line" /customer/shadow.custom.new && chmod 600 /customer/shadow.custom.new &&
    mv /customer/shadow.custom.new /customer/shadow.custom && sync || fail "write"
  ok "/customer/shadow.custom written; it takes effect at the next boot"
}

step_demo() {
  case $(demo_state) in
  ours) ok "already patched" ;;
  stock)
    patch_demo $DEMO $DEMO || fail "no place for the hook in $DEMO (no fw_setenv or prog_uvc line)"
    [ "$(demo_state)" = ours ] || fail "the patch did not come out as intended"
    sync; ok "the two hook lines are in $DEMO" ;;
  *) fail "$DEMO is neither the stock one nor the stock one with our hook: restore the stock one first (uninstall)" ;;
  esac
}

step_bootargs() {
  h=$(heap)
  case $h in
  $HEAP_OURS) ok "media heap already $h" ;;
  0x9000000)
    a=$($PRINTENV bootargs | sed "s/^bootargs=//; s/\(mma_heap=[^ ]*sz=\)0x9000000/\1$HEAP_OURS/")
    $SETENV bootargs "$a" >/dev/null || fail "fw_setenv"
    [ "$(heap)" = $HEAP_OURS ] || fail "bootargs did not take the new heap size"
    ok "media heap 0x9000000 → $HEAP_OURS, from the next boot" ;;
  *) fail "unexpected media heap size «$h» in bootargs" ;;
  esac
}

step_verify() {
  for f in $FILES; do
    [ -f "$B/$f" ] || continue
    cmp -s "$B/$f" "$(dest $f)" || fail "$(dest $f) differs from the bundle"
  done
  sh -n $MC/boot-hook.sh && sh -n $MC/app-start.sh && sh -n $DEMO || fail "a boot script does not parse"
  [ -x $DEMO ] || fail "$DEMO is not executable: the boot would skip it"
  [ "$(demo_state)" = ours ] || fail "$DEMO is not patched"
  [ "$(heap)" = $HEAP_OURS ] || fail "bootargs"
  ok "installed; reboot to start microcamd"
}

status() {
  echo "demo.sh: $(B=${B:-/nonexistent}; demo_state 2>/dev/null)"
  echo "media heap: $(heap)"
  echo "switches: $(cd $FAST 2>/dev/null && ls *.ok enabled 2>/dev/null | tr '\n' ' ')"
  echo "login: $([ -f /customer/shadow.custom ] && echo set || echo stock)"
  echo "microcamd: $(pidof microcamd >/dev/null && echo running || echo 'not running')"
  df /customer | tail -1 | awk '{print "free: " $4 " KB of " $2}'
}

step_uninstall() {
  [ -f "$B/bootargs.stock" ] || fail "no bootargs.stock in $B"
  [ -f "$B/stock.list" ] || fail "no stock.list in $B (the dump's /customer files)"
  run() { if [ -n "$DRY" ]; then echo "would: $*"; else eval "$@" || fail "$*"; fi; }
  # 1. the boot script: ours out, the stock one back
  case $(demo_state) in
  stock) ok "demo.sh is the stock one" ;;
  ours) run "unpatch $DEMO > $DEMO.new && chmod 755 $DEMO.new && mv $DEMO.new $DEMO" ;;
  *) [ -f "$B/stock/demo.sh" ] || fail "demo.sh is not ours and the bundle has no stock copy"
     run "cp $B/stock/demo.sh $DEMO.new && chmod 755 $DEMO.new && mv $DEMO.new $DEMO" ;;
  esac
  # 2. everything of ours on the flash; the login and microcamd's card hook are mounted over stock files, a file under a
  #    mount cannot be removed or replaced
  for m in /etc/shadow /customer/sd_hotplug.sh; do grep -q " $m " /proc/mounts && run "umount $m"; done
  for p in $MC $FAST /customer/shadow.custom /customer/microcam-crash.log /customer/microcamd-boot.log; do
    [ -e $p ] && run "rm -rf $p"
  done
  # 3. the stock files the dump has and the camera lacks or holds changed (the owner's menu settings ws.bin stay); a
  #    card's bundle holds them all, unasked
  if [ -d "$B/stock" ]; then
    (cd "$B/stock" && find . -type f) | while read f; do
      f=${f#./}; [ "$f" = demo.sh ] && continue
      cmp -s "$B/stock/$f" "/customer/$f" && continue
      run "mkdir -p \$(dirname /customer/$f) && cp $B/stock/$f /customer/$f.new && mv /customer/$f.new /customer/$f"
    done || exit 1   # a failure in the loop exits only its subshell
  fi
  # 4. files on /customer the stock firmware does not have: ours from development, or left by the vendor's application
  (cd /customer && find . -type f) | sed 's|^\./||' | while read f; do
    grep -qxF "$f" "$B/stock.list" || run "rm -f '/customer/$f'"
  done || exit 1
  # 5. bootargs as the dump has them
  want=$(cat "$B/bootargs.stock")
  if [ "$($PRINTENV bootargs | sed 's/^bootargs=//')" != "$want" ]; then run "$SETENV bootargs \"$want\" >/dev/null"; fi
  run sync
  [ -n "$DRY" ] && { ok "dry run, nothing changed"; return; }
  [ "$(sha $DEMO)" = "$(stock_sha /customer/demo.sh)" ] || fail "demo.sh is not the stock one after all"
  [ "$($PRINTENV bootargs | sed 's/^bootargs=//')" = "$want" ] || fail "bootargs"
  ok "stock files and bootargs back; after the next boot the camera runs the vendor's firmware and the login is the vendor's"
}

CMD=$1; B=$2; STEP=$CMD
case $CMD in
install)
  [ -d "$B" ] || { echo "usage: install.sh install BUNDLE"; exit 2; }
  for STEP in check files flags demo bootargs verify; do (step_$STEP) || exit 1; done ;;
check|files|flags|access|demo|bootargs|verify)
  [ -d "$B" ] || { echo "usage: install.sh $CMD BUNDLE"; exit 2; }
  step_$CMD ;;
uninstall)
  [ -d "$B" ] || { echo "usage: install.sh uninstall BUNDLE [-n]"; exit 2; }
  [ "$3" = -n ] && DRY=1
  step_uninstall ;;
patch-demo)
  patch_demo "$2" "$3" || { echo "FAIL patch-demo"; exit 1; }
  echo "ok patch-demo: $3" ;;
status) B=$2; status ;;
*) sed -n '2,15p' "$0"; exit 2 ;;
esac
