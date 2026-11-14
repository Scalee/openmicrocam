#!/usr/bin/env python3
"""USB's JPEG rings: a host MJPEG stream a mode, the camera looking at the latency clock on the laptop panel
(fine cells changing every frame: large JPEGs); the rate at the host, the largest frame, the pump's dropped and
oversize, the encoder's «Outputbuf0 full» lines the kernel logged during the mode (read from microcamd's log, which
numbers the kernel's messages from boot on: dmesg's ring of ~500 lines rolls over during a run), and the rings as the
heap holds and microcamd prices them.

usage: rings.py [--json] [--secs S] [--cell PX] [--no-clock] [MODE…]     MODE = WxH@FPS (default 3840x2160@30
       3840x2160@45 1920x1080@120)                                       details: $MICROCAM_LOGS/rings-<date>.log
Threshold: ≥ 90 % of the asked rate (4K45 on the clock: 42.7 fps with the old w × h rings too), dropped, oversize and
ring-full 0.
"""
import argparse, os, subprocess, sys, threading, time

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), '..'))
import camlib  # noqa: E402

ap = argparse.ArgumentParser()
ap.add_argument('modes', nargs='*', default=['3840x2160@30', '3840x2160@45', '1920x1080@120'])
ap.add_argument('--json', action='store_true')
ap.add_argument('--secs', type=int, default=10)
ap.add_argument('--cell', type=int, default=4)
ap.add_argument('--no-clock', action='store_true')
a = ap.parse_args()
S = a.secs
rep = camlib.Report('rings', a.json)
MIB = 1 << 20


def log_mark():
    """The number of microcamd's next log line."""
    return int(camlib.microcam({'cmd': 'log-since', 'from': 2 ** 64 - 1}, 5)['message'])


def ring_full(since):
    """The encoder's «Outputbuf0 full» lines in microcamd's log from line `since` on."""
    lines = camlib.microcam({'cmd': 'log-since', 'from': since}, 10).get('lines') or []
    return sum('Outputbuf0 full' in l for l in lines)


def account():
    """(the heap's venc-ring lines, microcamd's price of the USB encoder), MiB."""
    acc = camlib.microcam({'cmd': 'account'}, 6).get('account') or {}
    held = sum(b for n, b in acc.get('held', []) if n.startswith('venc-ring'))
    priced = sum(b for n, b in acc.get('priced', []) if n == 'encoder usb')
    return round(held / MIB, 2), round(priced / MIB, 2)


def run(mode):
    size, fps = mode.split('@')
    fps = int(fps)
    mark = log_mark()
    out = {}

    def probe():
        out['r'] = subprocess.run(
            ['timeout', '-k', '1', str(S + 8), 'ffprobe', '-v', 'error', '-f', 'v4l2', '-input_format', 'mjpeg',
             '-video_size', size, '-framerate', str(fps), '-read_intervals', f'%+{S}',
             '-show_entries', 'packet=pts_time,size', '-of', 'csv=p=0', camlib.VIDEO], capture_output=True, text=True)

    th = threading.Thread(target=probe)
    th.start()
    pump, rings = None, (None, None)
    t0 = time.time()
    while th.is_alive():
        time.sleep(1)
        try:
            p = camlib.status().get('pump')
            pump = p or pump
            if rings[0] is None and time.time() - t0 > S / 2:
                rings = account()
        except camlib.Gone:
            pass
    th.join()
    rows = [l.split(',') for l in out['r'].stdout.split() if l.count(',') == 1]
    pk = [(float(t), int(n)) for t, n in rows][15:]  # past the stream's start
    if len(pk) < 2:
        rep.case(mode, False, 'no frames ' + out['r'].stderr.strip()[-100:])
        return
    rate = (len(pk) - 1) / (pk[-1][0] - pk[0][0])
    sizes = sorted(n for _, n in pk)
    full = ring_full(mark)
    dropped, oversize = (pump[1], pump[2]) if pump else (None, None)
    ok = rate >= 0.9 * fps and dropped == 0 and oversize == 0 and full == 0
    rep.case(mode, ok, fps=round(rate, 1), max_kb=sizes[-1] // 1000, p50_kb=sizes[len(sizes) // 2] // 1000,
             dropped=dropped, oversize=oversize, ring_full=full, rings_mib=rings[0], priced_mib=rings[1])


clock = None
if not a.no_clock:
    clock = subprocess.Popen([sys.executable, os.path.join(camlib.REPO, 'tools/cam/panel/clock.py'), '--whole', '--cell', str(a.cell),
                              '--secs', str(len(a.modes) * (S + 15) + 10)], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    time.sleep(2)
was = camlib.stream_release()
try:
    for m in a.modes:
        run(m)
finally:
    camlib.stream_restore(was)
    if clock:
        clock.terminate()
sys.exit(1 if rep.fails else 0)
