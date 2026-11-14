#!/usr/bin/env python3
"""The player on the camera: an MP4 on the card in the outputs' live windows, beside a host's USB stream.

  play   `play` a file: answered at once, the player `starting` then `playing` (watched), its time going on with the
         clock, DISP's canvases and USB's frames at the file's rate, a USB frame in $MICROCAM_LOGS/play-<time>-playing.jpg;
         `play pause`: `paused`, the time standing, the frames still at the rate (the decoder refreshed), two USB frames
         a second apart the same; `play seek` to the middle while paused; `play resume`: `playing` again; `play stop`: the sensor back, no decoder left
  ended  sought near the end: `ended` on the last frame, the frames still at the rate
  menu   the demo menu over the playing picture; paused, a note's text changed 10 times: each revision `presented` on
         HDMI and USB within 250 ms (the decoder's refresh carries the menu's layer), the last frame in
         $MICROCAM_LOGS/play-<time>-menu-0.png

usage: play.py [--json] [--secs S] [--file F] case...        details: $MICROCAM_LOGS/play-<date>.log
F: a video's path on the camera (default: the largest video in microcamd's file list). Thresholds: DISP and USB at least 80 % of the
file's rate (USB's the stream's when lower) in every state; the time within 15 % of the clock while playing, still while
paused; two paused USB frames within 2 levels of each other on average; after the stop no decoder and no player.
"""
import argparse, os, re, subprocess, sys, time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(HERE, '..'))
import camlib  # noqa: E402

ap = argparse.ArgumentParser()
ap.add_argument('case', nargs='+', choices=['play', 'ended', 'menu'])
ap.add_argument('--json', action='store_true')
ap.add_argument('--secs', type=float, default=5)
ap.add_argument('--file')
a = ap.parse_args()
rep = camlib.Report('play', a.json)


def microcam(req):
    return camlib.microcam(req, 20)


def longest():
    """The largest video, so the longest: the play case outlasts a short one (it ends before the pause)."""
    files = (microcam({'cmd': 'files', 'count': 1000}).get('files') or {}).get('list', [])
    return max((f for f in files if f['kind'] == 'video'), key=lambda f: f['bytes'], default={}).get('path', '')


def player():
    return camlib.status(5).get('player')


def rates(secs):
    """DISP's canvases and USB's frames a second, and the player's ms, over `secs`."""
    h0, p0, i0, t0 = camlib.disp_handed(2), camlib.status().get('pump'), player(), time.monotonic()
    time.sleep(secs)
    h1, p1, i1, t1 = camlib.disp_handed(2), camlib.status().get('pump'), player(), time.monotonic()
    dt = t1 - t0
    disp = round((h1[1] - h0[1]) / dt, 1) if h0 and h1 else None
    usb = round((p1[0] - p0[0]) / dt, 1) if p0 and p1 else None
    played = round((i1['ms'] - i0['ms']) / 1000 / dt, 2) if i0 and i1 else None
    return disp, usb, played, i1


def grab(name, n=1, gap=1.0):
    """`n` USB frames `gap` s apart into $MICROCAM_LOGS/play-<time>-<name>-<k>.png (the stream's own format); their paths."""
    st = camlib.status()
    stream = st.get('host_stream') or ''
    m = re.search(r'(\w+) (\d+)x(\d+) @(\d+)', stream)
    if not m:
        return []
    fmt = {'Mjpeg': 'mjpeg', 'H264': 'h264', 'H265': 'hevc'}.get(m[1], 'mjpeg')
    size, fps = f'{m[2]}x{m[3]}', int(m[4])
    stamp = time.strftime('%H%M%S')
    out = []
    was = camlib.stream_release()
    try:
        for k in range(n):
            p = os.path.join(camlib.LOGS, f'play-{stamp}-{name}-{k}.png')
            subprocess.run(['timeout', '-k', '2', '20', 'ffmpeg', '-loglevel', 'error', '-y', '-f', 'v4l2', '-input_format', fmt,
                            '-video_size', size, '-framerate', str(fps), '-i', camlib.VIDEO, '-vf', 'select=gte(n\\,15),scale=640:-2',
                            '-frames:v', '1', p], capture_output=True)
            if os.path.exists(p):
                out.append(p)
            time.sleep(gap)
    finally:
        camlib.stream_restore(was)
    return out


def differ(p, q):
    """Mean absolute difference of two grabbed frames' luma, 0–255."""
    r = subprocess.run(['ffprobe', '-v', 'error', '-f', 'lavfi', f'movie={p}[a];movie={q}[b];[a][b]blend=all_mode=difference,format=gray,signalstats',
                        '-show_entries', 'frame_tags=lavfi.signalstats.YAVG', '-of', 'csv=p=0'], capture_output=True, text=True)
    try:
        return float(r.stdout.strip().splitlines()[0])
    except (IndexError, ValueError):
        return None


def outcome(req, done):
    """`req` (answered at once, the player starting or stopping) and the player's state that ends it, followed on a
    watching connection: None, or what went wrong."""
    w = camlib.Menu()
    try:
        w.ask({'cmd': 'watch'})
        t0 = time.monotonic()
        r = w.ask(req)
        if not r.get('ok'):
            return r.get('error') or 'refused'
        ev = w.event(lambda m: m.get('event') in ('player', 'failed') and (m.get('event') == 'failed' or done(m.get('player'))), t0, 15)
        if ev is None:
            return f'{req["cmd"]}: no outcome in 15 s'
        return ev.get('error') if ev['event'] == 'failed' else None
    finally:
        w.close()


def started(file):
    err = outcome({'cmd': 'play', 'file': file}, lambda p: (p or {}).get('state') == 'playing')
    if err is None:
        time.sleep(1)  # the decoder's first frames through the outputs
    return err


def stop():
    outcome({'cmd': 'play-stop'}, lambda p: p is None)
    st = camlib.status()
    return st.get('player'), (st.get('current') or {}).get('vdec')


def ok_rate(r, want):
    return r is not None and r >= 0.8 * want


def play():
    file = a.file or longest()
    mark = camlib.log_mark()
    err = started(file)
    if err:
        return rep.case('play', False, f'play {file}: {err}'[:100])
    fps = None
    try:
        st = camlib.status()
        fps = ((st.get('desired') or {}).get('player') or {}).get('fps') or 25
        usb_fps = min(fps, ((st.get('desired') or {}).get('usb') or {}).get('fps') or fps)
        acc = camlib.account()
        playing = rates(a.secs)
        shot = grab('playing')
        microcam({'cmd': 'play-pause'})
        time.sleep(1)
        paused = rates(a.secs)
        still = grab('paused', 2)
        diff = differ(*still) if len(still) == 2 else None
        dur = (paused[3] or {}).get('duration_ms') or 0
        microcam({'cmd': 'play-seek', 'ms': dur // 2})
        time.sleep(1)
        sought = player()
        microcam({'cmd': 'play-resume'})
        time.sleep(1)
        resumed = rates(2)
    finally:
        after = stop()
    bad = camlib.bad_lines(camlib.log_since(mark))
    rep.detail(f'{file}: playing {playing} paused {paused} sought {sought} resumed {resumed} after {after}; account {acc}; '
               f'frames {shot} {still} diff {diff}; warnings {bad[:5]}')
    ok = ok_rate(playing[0], fps) and ok_rate(playing[1], usb_fps) and playing[2] is not None and abs(playing[2] - 1) <= 0.15 \
        and (playing[3] or {}).get('state') == 'playing' \
        and ok_rate(paused[0], fps) and ok_rate(paused[1], usb_fps) and paused[2] == 0 and (paused[3] or {}).get('state') == 'paused' \
        and diff is not None and diff <= 2 \
        and (sought or {}).get('state') == 'paused' and abs((sought or {}).get('ms', -10 ** 9) - dur // 2) <= 3000 \
        and (resumed[3] or {}).get('state') == 'playing' and (resumed[2] or 0) > 0.5 \
        and after == (None, None)
    rep.case('play', ok, f'{file} {fps} fps', disp=(playing[0], paused[0], resumed[0]), usb=(playing[1], paused[1], resumed[1]),
             clock=(playing[2], paused[2], resumed[2]), paused_diff=diff, sought_ms=(sought or {}).get('ms'), heap_free=acc.get('free'),
             frame=shot[0] if shot else None, warnings=len(bad))


def ended():
    file = a.file or longest()
    err = started(file)
    if err:
        return rep.case('ended', False, f'play {file}: {err}'[:100])
    try:
        st = camlib.status()
        fps = ((st.get('desired') or {}).get('player') or {}).get('fps') or 25
        dur = (player() or {}).get('duration_ms') or 0
        microcam({'cmd': 'play-seek', 'ms': max(0, dur - 1500)})
        time.sleep(3.5)
        end = rates(a.secs)
    finally:
        after = stop()
    rep.detail(f'{file}: ended {end} after {after}')
    ok = (end[3] or {}).get('state') == 'ended' and end[2] == 0 and ok_rate(end[0], fps) and after == (None, None)
    rep.case('ended', ok, f'{file}: {end[3]}', disp=end[0], usb=end[1])


def note(k):
    return {'kind': 'group', 'id': 'play-note', 'children': [
        {'kind': 'rect', 'x': 200, 'y': 900, 'w': 600, 'h': 160, 'fill': '#203040'},
        {'kind': 'text', 'x': 500, 'y': 1000, 'size': 64, 'text': f'Пауза {k}', 'fill': '#ffd060', 'align': 'centre'}]}


def menu():
    import json
    nodes = json.load(open(os.path.join(camlib.REPO, 'tools/ui/demo-menu.json')))['nodes']
    file = a.file or longest()
    err = started(file)
    if err:
        return rep.case('menu', False, f'play {file}: {err}'[:100])
    c = camlib.Menu()
    lat = []
    try:
        c.ask({'cmd': 'hello', 'take': True})
        c.ask({'cmd': 'commit', 'base': 0, 'ops': [{'op': 'replace', 'nodes': list(nodes) + [note(0)]}]})
        time.sleep(2)
        microcam({'cmd': 'play-pause'})
        time.sleep(1)
        rev = 1
        for k in range(10):
            t0 = time.monotonic()
            r = c.ask({'cmd': 'commit', 'base': rev, 'ops': [{'op': 'replace', 'nodes': list(nodes) + [note(k + 1)]}]})
            rev = r.get('revision', rev + 1)
            got = {}
            while time.monotonic() - t0 < 1.0 and len(got) < 2:
                for t, m in c.events:
                    if m.get('event') == 'presented' and m.get('revision', 0) >= rev and m['output'] not in got and t >= t0:
                        got[m['output']] = round((t - t0) * 1000)
                time.sleep(0.01)
            lat.append(got)
            time.sleep(0.3)
        shot = grab('menu')
        c.ask({'cmd': 'commit', 'base': rev, 'ops': [{'op': 'replace', 'nodes': []}]})
    finally:
        c.close()
        after = stop()
    hd = [g.get('hdmi') for g in lat]
    us = [g.get('usb') for g in lat]
    rep.detail(f'{file}: presented ms hdmi {hd} usb {us}; frame {shot}; after {after}')
    ok = all(x is not None and x <= 250 for x in hd) and all(x is not None and x <= 250 for x in us) and after == (None, None)
    rep.case('menu', ok, f'{file}: paused, 10 changes', hdmi_ms=max((x for x in hd if x is not None), default=None),
             usb_ms=max((x for x in us if x is not None), default=None), missed=sum(x is None for x in hd + us), frame=shot[0] if shot else None)


for case in a.case:
    {'play': play, 'ended': ended, 'menu': menu}[case]()
    time.sleep(2)
sys.exit(rep.fails)
