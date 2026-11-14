#!/usr/bin/env python3
"""A stream start microcamd cannot serve must still be answered, and the next normal start must work.

Starts in turn: refused (H.264 424x240: microcamd brings no frames at that width) and normal (MJPEG 4K30, 60 frames), the host's SET_INTERFACE
requests timed through usbmon (status 0 = answered; -2 = the host gave up after 5 s). One line a start.
usage: uvcanswer.py [--json] [--order rnrrn]        r = refused, n = normal; details: $MICROCAM_LOGS/uvcanswer-<date>.log
Thresholds: a refused start answered within 1 s; a normal start answered within 3 s with ≥ 55 of 60 frames.
"""
import argparse, os, re, subprocess, sys, time

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), '..'))
import camlib  # noqa: E402

ap = argparse.ArgumentParser()
ap.add_argument('--json', action='store_true')
ap.add_argument('--order', default='rnrrn')
a = ap.parse_args()
rep = camlib.Report('uvcanswer', a.json)


def usb_ids():
    node = os.path.basename(os.path.realpath(camlib.VIDEO))
    dev = os.path.realpath(f'/sys/class/video4linux/{node}/device/..')
    return int(open(f'{dev}/busnum').read()), int(open(f'{dev}/devnum').read())


def answers(text, bus, dev):
    """[(alt, status, seconds)] of the host's SET_INTERFACE requests in a usbmon text capture."""
    out, sent = [], {}
    for l in text.splitlines():
        f = l.split()
        if len(f) < 5 or f[3] != f'Co:{bus}:{dev:03d}:0':
            continue
        if f[2] == 'S' and re.search(r' s 01 0b ', l):
            sent[f[0]] = (int(f[1]), int(f[7], 16) if len(f) > 7 else -1)  # S: tag time S addr s 01 0b <alt> <iface> …
        elif f[2] == 'C' and f[0] in sent:
            t, alt = sent.pop(f[0])
            out.append((alt, int(f[4]), (int(f[1]) - t) / 1e6))
    return out


was = camlib.stream_release()
mon = None
try:
    bus, dev = usb_ids()
    subprocess.run(['sudo', '-n', 'modprobe', 'usbmon'], capture_output=True)
    cap = os.path.join(os.environ.get('TMPDIR', '/tmp'), f'uvcanswer-{os.getpid()}.mon')
    mon = subprocess.Popen(['sudo', '-n', 'cat', f'/sys/kernel/debug/usb/usbmon/{bus}u'], stdout=open(cap, 'w'))
    time.sleep(0.5)
    mark = camlib.log_mark()
    runs = []
    for i, kind in enumerate(a.order):
        t0 = time.time()
        if kind == 'r':
            n, err = camlib.host_stream('h264', '424x240', 30, 5, 9)
        else:
            n, err = camlib.host_stream('mjpeg', '3840x2160', 30, 60, 14)
        runs.append((kind, n, err, time.time() - t0))
        time.sleep(2)
    subprocess.run(['sudo', '-n', 'kill', str(mon.pid)], capture_output=True)
    mon.wait(3)
    sel = [x for x in answers(open(cap).read(), bus, dev) if x[0] != 0]  # the stream starts (alt ≠ 0)
    os.remove(cap)
    rep.detail('\n'.join(camlib.log_since(mark)))
    for i, (kind, n, err, secs) in enumerate(runs):
        alt, status, took = sel[i] if i < len(sel) else (None, None, None)
        answered = status == 0
        name = f'{i + 1} {"refused H.264 424x240" if kind == "r" else "normal MJPEG 4K30"}'
        if kind == 'r':
            ok = answered and took is not None and took <= 1.0
            rep.case(name, ok, err if not ok else '', answer_s=None if took is None else round(took, 2), status=status)
        else:
            ok = answered and took is not None and took <= 3.0 and n >= 55
            rep.case(name, ok, err if not ok else '', answer_s=None if took is None else round(took, 2), status=status,
                     frames=f'{n}/60')
finally:
    if mon and mon.poll() is None:
        subprocess.run(['sudo', '-n', 'kill', str(mon.pid)], capture_output=True)
    camlib.stream_restore(was)
sys.exit(rep.fails)
