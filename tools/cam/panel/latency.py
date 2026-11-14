#!/usr/bin/env python3
# Glass-to-glass latency: the laptop panel shows clock.py (Gray-coded display frame number n, time = n·P, and each tile's
# height on the panel) in the camera's view (view.py cal), the camera looks at it, we grab its USB stream and read the clock in one tile per band of rows of
# every frame: a reading = the frame's arrival on the laptop (its first packet, or with --end its last) − the time that
# tile's panel rows lit (n·P + the tile's height × P: the panel lights its rows top to bottom over a refresh).
# The sensor reads its rows one after another, so the readings fall with the tile's row in the frame: their slope is
# the sensor's readout over the frame's height, and the latency is given for the frame's first row (the readings moved
# along the slope to row 0). The exposure is set to one clock step for the run (the camera's own restored after): then
# exactly one cell changes during it, and its level is the share of the exposure after the change — the time inside
# the step, where a whole-step reading would round it (and a camera rate near a divisor of the panel's, as 30 of 120,
# keeps the rounding from averaging out). A reading is the middle of the row's exposure.
# In no number: the panel's own response (a constant).
#   latency.py panel [--fps 45 --secs 6]                    clock.py on the panel: the panel's table → panel.json
#   latency.py run  [--size … --secs S --p MS --tag TEXT --frames F --gain G --tiles N]
#                    clock.py on the panel: prints the latency statistics (row 0) and the readout
#   latency.py matrix --modes 3840x2160@30:mjpeg,3840x2160@30:h264,… [--runs N] [--end]
#                    each size's view calibrated, the clock window (clock.py) on the laptop panel by itself, one table
# The tiles are read where the clock draws them: its layout in the view of the run's size (view.json).
import argparse, json, os, subprocess, sys, time
import numpy as np

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(HERE, '..'))
import camlib  # noqa: E402
import view  # noqa: E402
DATA = view.DATA   # this panel's files: view.json, panel.json, the results
os.makedirs(DATA, exist_ok=True)
ap = argparse.ArgumentParser()
ap.add_argument('mode', choices=['panel', 'run', 'matrix'])
ap.add_argument('--dev', default=camlib.VIDEO)
ap.add_argument('--size', default='3840x2160')
ap.add_argument('--fps', type=int, default=30)
ap.add_argument('--codec', default='mjpeg')
ap.add_argument('--secs', type=float, default=2)
ap.add_argument('--p', type=float, default=1000 / 120)
ap.add_argument('--down', type=int, default=4, help='decode at 1/down of the size')
ap.add_argument('--tiles', type=int, default=4, help='clock tiles across the view, as clock.py --tiles')
ap.add_argument('--tag', default='')
ap.add_argument('--keep', action='store_true')
ap.add_argument('--frames', help='also write each frame\'s arrival and latency (ms, null when unread) to this JSON file')
ap.add_argument('--modes', default='3840x2160@30:mjpeg,3840x2160@30:h264,3840x2160@45:mjpeg,3840x2160@45:h264')
ap.add_argument('--runs', type=int, default=1)
ap.add_argument('--gain', type=int, default=400, help='the UVC gain during the run\'s exposure (100 = ×1)')
ap.add_argument('--end', action='store_true', help="time each frame when the laptop has all of it (ffmpeg's wall clock at the read), not at its first packet")
a = ap.parse_args()
W, H = map(int, a.size.split('x'))
w, h = W // a.down, H // a.down
tmp = os.environ.get('BIGTMP', '/tmp')
vid = f'{tmp}/lat.mjpg'


def grab_end():
    # the frame's end: ffmpeg stamps a packet with the wall clock when it has read it (the whole frame is in);
    # the stream is copied into a file with those stamps, then decoded
    mkv = f'{tmp}/lat.mkv'
    subprocess.run(['timeout', '-k', '1', str(int(a.secs + 12)), 'ffmpeg', '-loglevel', 'error', '-y', '-f', 'v4l2',
                    '-input_format', a.codec, '-video_size', f'{W}x{H}', '-framerate', str(a.fps),
                    '-use_wallclock_as_timestamps', '1', '-i', a.dev, '-frames:v', str(int(a.fps * a.secs)), '-c', 'copy', '-copyts', '-f', 'matroska', mkv])  # by count: with -copyts a -t would be met by the first wall-clock stamp
    pts = subprocess.run(['ffprobe', '-v', 'error', '-select_streams', 'v', '-show_entries', 'packet=pts_time', '-of', 'csv=p=0', mkv],
                         capture_output=True, text=True).stdout.split()
    # every packet decoded once, in order: frame k ↔ stamp k (the default, a constant rate, duplicates frames when the
    # camera delivers fewer than asked, and the pairing drifts by a frame per duplicate)
    raw = subprocess.run(['ffmpeg', '-loglevel', 'error', '-i', mkv, '-fps_mode', 'passthrough', '-vf', f'scale={w}:{h}', '-pix_fmt', 'gray', '-f', 'rawvideo', '-'],
                         capture_output=True).stdout
    n = min(len(raw) // (w * h), len(pts))
    os.remove(mkv)
    return np.array([float(x) * 1000 for x in pts[:n]]), np.frombuffer(raw[:n * w * h], np.uint8).reshape(n, h, w)


def grab():
    if a.end:
        return grab_end()
    # v4l2-ctl: raw MJPEG frames to a file + a monotonic timestamp (the first packet of the frame, ts-src-soe) per frame
    # (ffmpeg with -ts abs duplicates frames from the huge first timestamp and hangs). The page's clock is epoch time,
    # the V4L2 timestamps are CLOCK_MONOTONIC: add their offset.
    off = (time.time() - time.monotonic()) * 1000
    n = int(a.fps * a.secs)
    fcc = {'mjpeg': 'MJPG', 'h264': 'H264', 'h265': 'HEVC'}[a.codec]
    r = subprocess.run(['v4l2-ctl', '-d', a.dev, f'--set-fmt-video=width={W},height={H},pixelformat={fcc}',
                        f'--set-parm={a.fps}', '--stream-mmap', f'--stream-count={n}', f'--stream-to={vid}', '--verbose'],
                       capture_output=True, text=True, timeout=a.secs + 40)
    import re, io
    from PIL import Image
    info = [(int(m[1]), float(m[2]) * 1000 + off) for m in re.finditer(r'bytesused: (\d+) ts: ([\d.]+)', r.stdout + r.stderr)]
    if a.codec != 'mjpeg':
        # H.264 / H.265: the stream decoded whole by ffmpeg (the stream starts on an IDR), frame k ↔ timestamp k
        raw = subprocess.run(['ffmpeg', '-loglevel', 'error', '-f', 'hevc' if a.codec == 'h265' else 'h264', '-i', vid,
                              '-fps_mode', 'passthrough', '-vf', f'scale={w}:{h}', '-pix_fmt', 'gray', '-f', 'rawvideo', '-'], capture_output=True).stdout
        n = min(len(raw) // (w * h), len(info))
        fr = np.frombuffer(raw[:n * w * h], np.uint8).reshape(n, h, w)
        return np.array([t for _, t in info[:n]]), fr
    data = open(vid, 'rb').read()
    ts, fr, pos = [], [], 0
    for size, t in info:
        im = Image.open(io.BytesIO(data[pos:pos + size])); pos += size
        im.draft('L', (w, h))
        g = np.asarray(im.convert('L').resize((w, h)))
        ts.append(t); fr.append(g)
    return np.array(ts), np.array(fr)


def read_tile(v, ts):
    """One tile's 32 cell means per frame → per frame (the clock's time at the tile's panel rows at the step's start
    + half a step, ms; the changed cell's share; whether it lit) or None."""
    rows = v.reshape(len(v), 4, 8)
    # orientation: the reference row is one of the outer rows, constant over time, alternating
    var = rows.std(0).mean(1)
    if var[0] < var[3]: rows = rows[:, ::-1, :]
    if rows[:, 3, 1].mean() > rows[:, 3, 0].mean(): rows = rows[:, :, ::-1]   # W B W B … read from the left unless mirrored
    white, black = rows[:, 3, 0::2].mean(1), rows[:, 3, 1::2].mean(1)
    data = rows[:, :3, :].reshape(len(v), 24)
    thr = ((white + black) / 2)[:, None]
    span = (white - black)[:, None]
    bits = data > thr
    amb = np.abs(data - thr) < 0.2 * span                   # only for the height cells: they never change
    def number(g, est):
        b, prev = 0, 0
        for k in range(18, -1, -1):     # Gray → binary
            prev ^= (g >> k) & 1; b |= prev << k
        n = (int(est) >> 19 << 19) | b  # the high bits from the grab's own clock: the nearest such n
        return n + (round((est - n) / (1 << 19)) << 19)
    out = []
    for i in range(len(v)):
        if span[i, 0] < 20 or amb[i, 19:24].any():
            out.append(None); continue
        at = sum(1 << j for j in range(5) if bits[i, 19 + j]) / 31
        est = ts[i] / a.p
        lvl = (data[i, :19] - black[i]) / span[i, 0]
        grey = np.minimum(lvl, 1 - lvl)                     # 0 for a cell that stayed, up to 0.5 for a change mid-exposure
        k = int(np.argmax(grey))
        if np.sort(grey)[-2] > 0.2:                          # two cells between: not one clock change, unreadable
            out.append(None); continue
        g = sum(1 << j for j in range(19) if bits[i, j])
        n1, n0 = number(g | 1 << k, est), number(g & ~(1 << k), est)
        if abs(n1 - n0) != 1:
            out.append(None); continue
        # the changed cell's level against the share of the exposure after the change: the panel's own table (its cells
        # light and go dark at their own pace), straight without one
        on = n1 > n0
        share = float(np.clip(lvl[k] if on else 1 - lvl[k], 0, 1))
        out.append(((min(n0, n1) + 0.5 + at) * a.p, share, on))
    return out, float(np.median(span))


PANEL = os.path.join(DATA, 'panel.json')
QS = np.linspace(0, 1, 21)


def shares_to_time(readings, table):
    """(start, share, lit) → ms: the share made the exposure's true share by the panel's table (when there is one)."""
    out = []
    for r in readings:
        if r is None:
            out.append(None); continue
        t, share, on = r
        if table:
            share = float(np.interp(share, table['lit' if on else 'dark'], QS))
        out.append(t + share * a.p)
    return out


def camera_exposure(set_to):
    """The UVC exposure: set a manual one (ms, gain) and return the camera's own to put back, or put that back."""
    ctl = lambda *c: subprocess.run(['v4l2-ctl', '-d', a.dev, *c], capture_output=True, text=True)
    if isinstance(set_to, dict):   # one at a time: the camera refuses the batch whole (the exposure time beside auto)
        for k, v in set_to.items():
            ctl('-c', f'{k}={v}')
        return None
    names = ('auto_exposure', 'exposure_time_absolute', 'gain')
    was = {k: v.split()[0] for k, v in (l.split(': ', 1) for l in ctl('-C', ','.join(names)).stdout.split('\n') if ': ' in l)}
    ctl('-c', f'auto_exposure=1,exposure_time_absolute={max(1, round(set_to[0] * 10))},gain={set_to[1]}')
    return {k: was[k] for k in ('gain', 'exposure_time_absolute', 'auto_exposure') if k in was}


def tiles():
    """The clock's tiles in this size's view, at the decode's scale: each its 32 cells' inner halves [y0, y1, x0, x1]
    and its row in the frame (0..1)."""
    e = view.load(a.size)
    if not e:
        sys.exit('no view of the camera made: view.py cal')
    out = []
    for t in view.layout(e, a.tiles)['tiles']:
        cells = []
        for x0, y0, x1, y1 in t['cells']:
            qx, qy = (x1 - x0) / 4, (y1 - y0) / 4
            cells.append([int((y0 + qy) / a.down), max(int((y1 - qy) / a.down), int((y0 + qy) / a.down) + 1),
                          int((x0 + qx) / a.down), max(int((x1 - qx) / a.down), int((x0 + qx) / a.down) + 1)])
        out.append(dict(cells=cells, row_y=t['centre'][1] / H))
    return out


def grab_short():
    """The grab with the exposure one clock step long, the camera's own put back after."""
    was = camera_exposure((a.p, a.gain))
    try:
        return grab()
    finally:
        camera_exposure(was)


def panel():
    """The panel's table: at a camera rate whose exposures fall at every phase of the panel's refresh (45 against 120),
    the true shares are even over 0..1, so the measured shares' quantiles map back to them — one table for a cell that
    lit, one for a cell that went dark."""
    ts, fr = grab_short()
    lit, dark = [], []
    for t in tiles():
        v = np.array([[f[y0:y1, x0:x1].mean() for y0, y1, x0, x1 in t['cells']] for f in fr])
        for r in read_tile(v, ts)[0]:
            if r is not None:
                (lit if r[2] else dark).append(r[1])
    if min(len(lit), len(dark)) < 100:
        sys.exit(f'too few changes read ({len(lit)} lit, {len(dark)} dark): a longer --secs')
    tab = dict(lit=np.quantile(lit, QS).round(3).tolist(), dark=np.quantile(dark, QS).round(3).tolist(),
               fps=a.fps, readings=[len(lit), len(dark)], when=time.strftime('%F %T'))
    json.dump(tab, open(PANEL, 'w'))
    print(f'{len(lit)} lit, {len(dark)} dark; at a half step a lit cell reads {tab["lit"][10]}, a dark one {tab["dark"][10]}; saved {PANEL}')


def run():
    c = dict(tiles=tiles())
    table = json.load(open(PANEL)) if os.path.exists(PANEL) else None
    ts, fr = grab_short()
    ys = np.array([t['row_y'] for t in c['tiles']])
    lat = np.full((len(fr), len(ys)), np.nan)          # frames × tiles: arrival − the clock seen at the tile
    spans = []
    for j, t in enumerate(c['tiles']):
        v = np.array([[f[y0:y1, x0:x1].mean() for y0, y1, x0, x1 in t['cells']] for f in fr])
        rd, span = read_tile(v, ts); spans.append(span)
        clk = shares_to_time(rd, table)
        for i, k in enumerate(clk):
            if k is not None and -50 < ts[i] - k < 2000:
                lat[i, j] = ts[i] - k
    # the slope over the frame's height, each frame its own offset: deviations from the frame's mean against the rows'
    ok = ~np.isnan(lat)
    dy, dl = [], []
    for i in range(len(lat)):
        if ok[i].sum() >= 2:
            dy += list(ys[ok[i]] - ys[ok[i]].mean()); dl += list(lat[i, ok[i]] - lat[i, ok[i]].mean())
    slope = float(np.dot(dy, dl) / np.dot(dy, dy)) if len(dy) and np.dot(dy, dy) > 0 else 0.0
    row0 = np.array([np.mean(lat[i, ok[i]] - slope * ys[ok[i]]) for i in range(len(lat)) if ok[i].any()])
    if a.frames:
        json.dump(dict(rows=ys.round(3).tolist(), slope=round(slope, 2),
                       frames=[[round(float(t), 1)] + [None if np.isnan(x) else round(float(x), 1) for x in l] for t, l in zip(ts, lat)]),
                  open(a.frames, 'w'))
    st = dict(tag=a.tag, size=a.size, fps=a.fps, codec=a.codec, to='end' if a.end else 'first packet', at='row 0', panel_table=bool(table),
              frames=len(lat), decoded=len(row0), tiles=len(ys),
              readout_ms=round(-slope, 1) if len(ys) > 1 else None, contrast=float(np.median(spans)), when=time.strftime('%F %T'))
    if len(row0):
        st.update({k: round(float(x), 1) for k, x in zip(('min', 'p10', 'median', 'p90', 'max'), np.percentile(row0, [0, 10, 50, 90, 100]))})
    print(json.dumps(st, ensure_ascii=False))
    with open(os.path.join(DATA, 'results.jsonl'), 'a') as f:
        f.write(json.dumps(st, ensure_ascii=False) + '\n')
    if not a.keep and os.path.exists(vid):
        os.remove(vid)


def matrix():
    """Each size's view calibrated, then that size's modes `runs` times each with the clock running in it; one table."""
    shown = []

    def page(*args):
        """The clock window (clock.py) fullscreen on the laptop panel, in place of the one before."""
        for p in shown:
            p.terminate()
            p.wait(5)
        shown[:] = [subprocess.Popen([sys.executable, os.path.join(HERE, 'clock.py'), '--p', str(a.p), '--tiles', str(a.tiles), *args], stdout=subprocess.DEVNULL)]
    modes = []
    for m in a.modes.split(','):
        size, _, rest = m.partition('@')
        fps, _, codec = rest.partition(':')
        modes.append((size, int(fps), codec or 'mjpeg'))
    me = [sys.executable, os.path.abspath(__file__), '--dev', a.dev, '--down', str(a.down), '--tiles', str(a.tiles)]
    rows, was = [], camlib.stream_release()
    try:
        for size in dict.fromkeys(s_ for s_, _, _ in modes):
            for p in shown:
                p.terminate()
                p.wait(5)
            r = subprocess.run(['timeout', '-k', '1', '60', sys.executable, os.path.join(HERE, 'view.py'), 'cal', '--size', size, '--show', '0'],
                               capture_output=True, text=True)
            print(f'view {size}: ' + (r.stdout.strip().splitlines() or [''])[-1] + r.stderr.strip()[-160:], flush=True)
            if r.returncode:
                rows += [(f'{c} {s_}@{f}', [], [], []) for s_, f, c in modes if s_ == size]
                continue
            page('--size', size)
            time.sleep(4)
            if not os.path.exists(PANEL):
                r = subprocess.run(['timeout', '-k', '1', '90', *me, 'panel', '--size', size, '--fps', '45', '--secs', '6',
                                    '--gain', str(a.gain)], capture_output=True, text=True)
                print('panel: ' + (r.stdout.strip().splitlines() or [''])[-1] + r.stderr.strip()[-120:], flush=True)
            for _, fps, codec in [m for m in modes if m[0] == size]:
                meds, dec, reads = [], [], []
                for _ in range(a.runs):
                    r = subprocess.run(['timeout', '-k', '1', '120', *me, 'run', '--size', size, '--fps', str(fps), '--codec', codec,
                                        '--secs', str(a.secs), '--tag', f'matrix {a.tag}'.strip(), '--gain', str(a.gain)]
                                       + (['--end'] if a.end else []), capture_output=True, text=True)
                    st = next((json.loads(l) for l in r.stdout.splitlines() if l.startswith('{')), {})
                    if 'median' in st:
                        meds.append(st['median'])
                        reads.append(st.get('readout_ms'))
                    dec.append(f"{st.get('decoded', 0)}/{st.get('frames', 0)}")
                    time.sleep(2)
                rows.append((f'{codec} {size}@{fps}', meds, dec, reads))
    finally:
        for p in shown:
            p.terminate()
        camlib.stream_restore(was)
    print(f"{'mode':<24} {'runs':>4} {'median ms':>9} {'min':>6} {'max':>6} {'readout':>7}  decoded  (row 0 to {'the frame end' if a.end else 'its first packet'})")
    for name, meds, dec, reads in rows:
        if meds:
            rd = [r for r in reads if r is not None]
            print(f'{name:<24} {len(meds):>4} {np.median(meds):9.1f} {min(meds):6.1f} {max(meds):6.1f} {np.median(rd) if rd else float("nan"):7.1f}  {" ".join(dec)}')
        else:
            print(f'{name:<24} {0:>4} {"—":>9} {"":>6} {"":>6}  {" ".join(dec) or "not calibrated"}')


{'panel': panel, 'run': run, 'matrix': matrix}[a.mode]()
