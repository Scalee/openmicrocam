#!/usr/bin/env python3
"""A recording's lost frames, read from the picture itself: the camera recorded `clock.py --motion` (its clock tiles count
the panel's display frames), and each recorded frame is matched to the stretch of display frames its exposure saw.
The exposure may be several display frames long: then the counter's low cells are a blend, so the frame is not read
cell by cell but fitted — the levels a window of exposure E would give, slid over the display frames, against the
cells' levels (the reference cells give black and white); E is fitted once per file. A frame's reading is the exposure's
middle in display frames (the median of the first row of tiles); a step between frames is rounded to the file's frame
period: one period is the norm, k more are k frames lost, none is a frame repeated (a recording taken at 30 from a 45 fps
sensor steps 2.7 or 5.3 display frames, both one period). The tiles are where clock.py drew them: the view of the
file's size (view.json, `view.py cal` at that size, or another size of the same field of view).

usage: framegaps.py FILE|URL [--size WxH] [--tiles N] [--p MS] [--json]
"""
import argparse, json, os, subprocess, sys

import numpy as np

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import view  # noqa: E402

CODE = 19
WRAP = 1 << CODE


def gray_bits(m):
    m = np.asarray(m, np.int64) % WRAP
    g = m ^ (m >> 1)
    return (g[..., None] >> np.arange(CODE)) & 1


def model(s, E):
    """The 19 counter cells' levels for exposures of E display frames centred at s (an array): S × 19."""
    lo, hi = s - E / 2, s + E / 2
    ms = np.floor(lo)[:, None] + np.arange(int(np.ceil(E)) + 2)
    w = np.clip(np.minimum(ms + 1, hi[:, None]) - np.maximum(ms, lo[:, None]), 0, None)
    return (w[..., None] * gray_bits(ms.astype(np.int64))).sum(1) / E


def fit(lvl, E):
    """The exposure's middle (display frames, mod 2^19) and the fit's rms for one tile's 19 levels."""
    bits = lvl > 0.5
    amb = np.argsort(np.abs(lvl - 0.5))[:min(3, int((np.abs(lvl - 0.5) < 0.3).sum()))]
    best = (None, 9.0)
    for k in range(1 << len(amb)):
        b = bits.copy()
        for j, i in enumerate(amb):
            if k >> j & 1:
                b[i] = not b[i]
        g = sum(1 << i for i in range(CODE) if b[i])
        m, x = 0, 0
        for i in range(CODE - 1, -1, -1):   # Gray → binary
            x ^= g >> i & 1
            m |= x << i
        s = m + 0.5 + np.arange(-2 - E / 2, 2 + E / 2, 0.1)
        r = np.sqrt(((model(s, E) - lvl) ** 2).mean(1))
        i = int(np.argmin(r))
        if r[i] < best[1]:
            best = (float(s[i]) % WRAP, float(r[i]))
    return best


def levels(path, size, tiles):
    """Per frame and tile: the 19 counter levels (0 black … 1 white) or NaN when the tile is too dim to read."""
    W, H = map(int, size.split('x'))
    down = max(1, W // 960)
    w, h = W // down, H // down
    e = view.load(size)
    if not e:
        sys.exit(f'no view of the camera made (view.py cal --size {size})')
    boxes = []
    for t in view.layout(e, tiles, motion=True)['tiles']:
        bb = []
        for x0, y0, x1, y1 in t['cells']:
            qx, qy = (x1 - x0) / 4, (y1 - y0) / 4
            bb.append((int((y0 + qy) / down), max(int((y1 - qy) / down), int((y0 + qy) / down) + 1),
                       int((x0 + qx) / down), max(int((x1 - qx) / down), int((x0 + qx) / down) + 1)))
        boxes.append(bb)
    p = subprocess.Popen(['ffmpeg', '-loglevel', 'error', '-i', path, '-fps_mode', 'passthrough', '-vf', f'scale={w}:{h}',
                          '-pix_fmt', 'gray', '-f', 'rawvideo', '-'], stdout=subprocess.PIPE)
    out = []
    while True:
        buf = p.stdout.read(w * h)
        if len(buf) < w * h:
            break
        f = np.frombuffer(buf, np.uint8).reshape(h, w).astype(float)
        row = []
        for bb in boxes:
            v = np.array([f[y0:y1, x0:x1].mean() for y0, y1, x0, x1 in bb])
            white, black = v[24::2].mean(), v[25::2].mean()
            row.append((v[:CODE] - black) / (white - black) if white - black > 20 else np.full(CODE, np.nan))
        out.append(row)
    p.wait()
    return np.array(out)


def read(path, size=None, tiles=4, p=1000 / 120):
    """The file's frames matched to display frames: a dict of the counts, the lost and the repeated frames."""
    probe = json.loads(subprocess.run(['ffprobe', '-v', 'error', '-select_streams', 'v:0', '-show_entries',
                                       'stream=width,height,r_frame_rate:packet=pts_time', '-of', 'json', path],
                                      capture_output=True, text=True).stdout)
    st = probe['streams'][0]
    size = size or f"{st['width']}x{st['height']}"
    period = 1000 / eval(st['r_frame_rate']) / p          # the file's frame period in display frames
    pts = sorted(float(k['pts_time']) for k in probe.get('packets', []) if 'pts_time' in k)
    lv = levels(path, size, tiles)[:, :tiles]             # the first row of tiles
    n = len(lv)
    sample = [lv[i, 0] for i in range(n) if not np.isnan(lv[i, 0, 0])][:10]
    Es = np.arange(0.5, 6.01, 0.25)
    E = float(Es[np.argmin([sum(fit(x, E_)[1] for x in sample) for E_ in Es])]) if sample else 1.0
    s = np.full(n, np.nan)
    for i in range(n):
        xs = [x for x, r in (fit(lv[i, j], E) for j in range(tiles) if not np.isnan(lv[i, j, 0])) if x is not None and r < 0.12]
        if xs:
            s[i] = np.median(xs)
    read_at = [i for i in range(n) if not np.isnan(s[i])]
    lost, repeated = [], []
    for a_, b in zip(read_at, read_at[1:]):
        d = (s[b] - s[a_] + WRAP / 2) % WRAP - WRAP / 2
        k = round(d / period) - (b - a_)
        at = round(pts[b] - pts[0], 3) if b < len(pts) else None
        if k > 0:
            lost.append(dict(frame=b, at_s=at, lost=k, step_ms=round(d * p, 1)))
        elif round(d / period) == 0:
            repeated.append(dict(frame=b, at_s=at, step_ms=round(d * p, 1)))
    return dict(file=path, size=size, frames=n, read=len(read_at), exposure_ms=round(E * p, 1),
                lost=sum(x['lost'] for x in lost), gaps=lost, repeated=repeated)


if __name__ == '__main__':
    ap = argparse.ArgumentParser()
    ap.add_argument('file')
    ap.add_argument('--size', help='the view to take (default: the file\'s own size)')
    ap.add_argument('--tiles', type=int, default=4, help='as clock.py --tiles')
    ap.add_argument('--p', type=float, default=1000 / 120, help='ms per display frame, as clock.py --p')
    ap.add_argument('--json', action='store_true')
    a = ap.parse_args()
    r = read(a.file, a.size, a.tiles, a.p)
    if a.json:
        print(json.dumps(r))
    else:
        print(f"{r['frames']} frames, {r['read']} read (exposure ~{r['exposure_ms']} ms); lost {r['lost']}"
              + (': ' + ', '.join(f"{g['lost']} at {g['at_s']} s" for g in r['gaps']) if r['gaps'] else '')
              + (f"; repeated {len(r['repeated'])}: " + ', '.join(f"{g['at_s']} s" for g in r['repeated']) if r['repeated'] else ''))
