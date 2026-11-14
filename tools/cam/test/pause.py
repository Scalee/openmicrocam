#!/usr/bin/env python3
"""A paused output beside a recording on the camera, beside a host's USB 4K30 stream, HDMI at a timing the monitor lists
(the `hdmi` control's values) no faster than the host's 30: the fastest such 1080p one, else 2160p25 (a frame apart from
USB's still); said in the output:
a scene (the demo menu and a note shown only on a paused output), HDMI at level 3, USB 2, the recording 0 — three
distinct frames, so the lower of the two outputs in the precedence pauses and keeps a frame without the live picture.

  hdmi   USB above HDMI: HDMI paused — `outputs` names it, its pool covered and still handed to DISP, USB's rate, the
         MP4's gaps; after the stop HDMI unpaused
  usb    HDMI above USB: USB paused — `outputs` names it, the host's stream still delivered, a frame of it in
         $MICROCAM_LOGS/pause-usb-<time>.jpg (the note over black), the MP4's gaps; after the stop USB unpaused

usage: pause.py [--json] [--secs S] case...        details: $MICROCAM_LOGS/pause-<date>.log
Thresholds: the paused output in `outputs` with `recording`; its frames still go (DISP handed / the pump) at least at
80 % of its rate; the MP4 with no gap over 1.5 frame periods; the output unpaused after the stop.
"""
import argparse, json, os, re, subprocess, sys, time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(HERE, '..'))
import camlib  # noqa: E402

ap = argparse.ArgumentParser()
ap.add_argument('case', nargs='+', choices=['hdmi', 'usb'])
ap.add_argument('--json', action='store_true')
ap.add_argument('--secs', type=float, default=8)
a = ap.parse_args()
rep = camlib.Report('pause', a.json)
MENU = json.load(open(os.path.join(camlib.REPO, 'tools/ui/demo-menu.json')))
NOTE = {'kind': 'group', 'id': 'pause-note', 'level': 0, 'paused': True, 'anchor': 'centre', 'children': [
    {'kind': 'rect', 'x': -420, 'y': -70, 'w': 840, 'h': 140, 'fill': '#202830'},
    {'kind': 'text', 'x': 0, 'y': 15, 'size': 48, 'text': 'Пауза: идёт запись', 'fill': '#ffd060', 'align': 'centre'}]}


def microcam(req):
    return camlib.microcam(req, 20)


def outputs(c, since):
    """The last `outputs` event after `since`: name → paused reason (None when not paused)."""
    ev = [m for t, m in c.events if t >= since and m.get('event') == 'outputs']
    return {o['name']: o.get('paused') for o in ev[-1]['outputs']} if ev else None


HOST_FPS = 30


def hdmi_timing():
    """Among the `hdmi` control's values (the monitor's) at most HOST_FPS: the fastest 1080p one, else 2160p25; None
    when neither is listed."""
    snap = microcam({'cmd': 'watch'}).get('snapshot') or {}
    values = next((c.get('values', []) for c in snap.get('controls', []) if c['name'] == 'hdmi'), [])
    p1080 = [v for v in values if v.startswith('1080p') and int(v[len('1080p'):]) <= HOST_FPS]
    if p1080:
        return max(p1080, key=lambda v: int(v[len('1080p'):]))
    return '2160p25' if '2160p25' in values else None


# the precedence with `first` above the other output (the default's order otherwise)
QUEUE = 'recording > player > {} > {} > photo > drawing > picture'


def setup(first):
    timing = hdmi_timing()
    print(f'HDMI at {timing or f"auto: the monitor lists no 1080p timing at most {HOST_FPS} Hz, nor 2160p25"}', flush=True)
    camlib.change('hdmi', timing or 'auto')
    camlib.change('precedence', QUEUE.format(first, 'usb' if first == 'hdmi' else 'hdmi'))
    camlib.change('record-size', '4k')
    for out, level in (('hdmi', 3), ('usb', 2), ('media', 0)):
        camlib.change(f'level-{out}', level)
    c = camlib.Menu()
    c.ask({'cmd': 'hello', 'take': True})
    c.ask({'cmd': 'commit', 'base': 0, 'ops': [{'op': 'replace', 'nodes': list(MENU['nodes']) + [NOTE]}]})
    time.sleep(2)
    return c


def teardown(c):
    c.ask({'cmd': 'commit', 'base': 1, 'ops': [{'op': 'replace', 'nodes': []}]})
    c.close()


def recorded():
    path, _ = camlib.record_stop()
    return camlib.mp4_gaps(path) if path else (None, None, None)


def hdmi():
    c = setup('usb')
    stopped = False
    try:
        t0 = time.monotonic()
        err = camlib.record_start()
        if err:
            return rep.case('hdmi', False, f'record: {err}'[:100])
        time.sleep(3)
        paused = outputs(c, t0)
        h0, p0, t1 = camlib.disp_handed(2), camlib.status().get('pump'), time.monotonic()
        time.sleep(a.secs)
        h1, p1, t2 = camlib.disp_handed(2), camlib.status().get('pump'), time.monotonic()
        acc = camlib.account()
        t3 = time.monotonic()
        frames, period, gaps = recorded()
        stopped = True
        time.sleep(2)
        after = outputs(c, t3)
    finally:
        # ended early (an error, SIGTERM at the caller's limit): the recording does not outlive the tester
        if not stopped:
            camlib.record_stop()
        teardown(c)
    disp_fps = round((h1[1] - h0[1]) / (t2 - t1), 1) if h0 and h1 else None
    usb_fps = round((p1[0] - p0[0]) / (t2 - t1), 1) if p0 and p1 else None
    rep.detail(f'outputs {paused} after {after}; pool 2 {h0} → {h1}; account {acc}')
    ok = (paused or {}).get('hdmi') == 'recording' and bool(h1) and h1[0] == 3 and (disp_fps or 0) >= 24 \
        and frames is not None and not gaps and (after or {}).get('hdmi', 'x') is None
    rep.case('hdmi', ok, f'outputs {paused}, after {after}', hdmi_canvases=h1 and h1[0], disp_fps=disp_fps, usb_fps=usb_fps,
             mp4_frames=frames, gaps=gaps[:5] if gaps else gaps, heap_free=acc.get('free'))


def usb():
    c = setup('hdmi')
    shot = os.path.join(camlib.LOGS, f'pause-usb-{time.strftime("%H%M%S")}.jpg')
    stopped = False
    try:
        t0 = time.monotonic()
        err = camlib.record_start()
        if err:
            return rep.case('usb', False, f'record: {err}'[:100])
        time.sleep(3)
        paused = outputs(c, t0)
        p0, t1 = camlib.status().get('pump'), time.monotonic()
        time.sleep(a.secs)
        p1, t2 = camlib.status().get('pump'), time.monotonic()
        was = camlib.stream_release()
        try:
            g = subprocess.run(['timeout', '-k', '2', '20', 'ffmpeg', '-loglevel', 'error', '-y', '-f', 'v4l2', '-input_format', 'mjpeg',
                                '-video_size', '3840x2160', '-framerate', '30', '-i', camlib.VIDEO, '-vf', 'select=gte(n\\,20)',
                                '-frames:v', '1', shot], capture_output=True, text=True)
        finally:
            camlib.stream_restore(was)
        t3 = time.monotonic()
        frames, period, gaps = recorded()
        stopped = True
        time.sleep(2)
        after = outputs(c, t3)
    finally:
        # ended early (an error, SIGTERM at the caller's limit): the recording does not outlive the tester
        if not stopped:
            camlib.record_stop()
        teardown(c)
    usb_fps = round((p1[0] - p0[0]) / (t2 - t1), 1) if p0 and p1 else None
    rep.detail(f'outputs {paused} after {after}; grab rc {g.returncode} {g.stderr[-200:]}')
    ok = (paused or {}).get('usb') == 'recording' and (usb_fps or 0) >= 24 and os.path.exists(shot) and frames is not None \
        and not gaps and (after or {}).get('usb', 'x') is None
    rep.case('usb', ok, f'outputs {paused}, after {after}, frame {shot if os.path.exists(shot) else "none"}', usb_fps=usb_fps,
             mp4_frames=frames, gaps=gaps[:5] if gaps else gaps)


for case in a.case:
    {'hdmi': hdmi, 'usb': usb}[case]()
    time.sleep(2)
sys.exit(rep.fails)
