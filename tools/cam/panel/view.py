#!/usr/bin/env python3
"""The camera's view of the laptop panel: which piece of the panel the camera sees, and where each test picture goes in it.
`cal` shows a frame of tiles on the panel, each a white rectangle with 4×8 cell holes: 24 code cells carry the tile's
column, row and a CRC-8 of both, the bottom row is the reference (no hole, hole, …). One camera frame is grabbed; every
whole tile read gives its four corners on the panel and in the frame, and a least-squares homography panel → camera is
fitted to them: the camera may be turned and look at the panel at a slant. Saved per camera size in `view.json` with the
mode it was made at, the panel's size, and `rect`: the largest rectangle of the camera's frame that shows only panel.
The test pictures (`clock.py`) lay their tiles out in that rectangle in the camera's coordinates and draw them on the
panel through the homography, so the camera sees them upright and filling its view; the readers take the same layout.
After `cal` the view's outline stays on the panel for --show s.

usage: view.py cal [--size WxH] [--fps N] [--cell PX,…] [--settle S] [--show S] [--monitor CONNECTOR]
                                                    show the pattern, grab, fit, save
       view.py show                                                              the saved views
"""
import json, os, sys, time

import numpy as np

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(HERE, '..'))
import camlib  # noqa: E402

DATA = os.path.join(camlib.LOGS, 'panel')   # this rig's files: view.json, panel.json, results.jsonl
VIEW = os.path.join(DATA, 'view.json')
C, R = 8, 4                                  # a tile's cells across and down


# ---- geometry --------------------------------------------------------------------------------------------------------

def fit_h(src, dst):
    """The homography src → dst (N×2 each, N ≥ 4) by least squares (normalised DLT)."""
    src, dst = np.asarray(src, float), np.asarray(dst, float)

    def norm(p):
        c, s = p.mean(0), np.sqrt(2) / max(np.sqrt(((p - p.mean(0)) ** 2).sum(1)).mean(), 1e-9)
        return np.array([[s, 0, -s * c[0]], [0, s, -s * c[1]], [0, 0, 1]])
    Ts, Td = norm(src), norm(dst)
    s, d = apply_h(Ts, src), apply_h(Td, dst)
    rows = []
    for (x, y), (u, v) in zip(s, d):
        rows += [[-x, -y, -1, 0, 0, 0, u * x, u * y, u], [0, 0, 0, -x, -y, -1, v * x, v * y, v]]
    h = np.linalg.svd(np.array(rows))[2][-1].reshape(3, 3)
    h = np.linalg.inv(Td) @ h @ Ts
    return h / h[2, 2]


def apply_h(h, pts):
    p = np.hstack([np.asarray(pts, float), np.ones((len(pts), 1))]) @ np.asarray(h).T
    return p[:, :2] / p[:, 2:]


def largest_rect(h_inv, W, H, pw, ph):
    """The largest axis-aligned rectangle of the W×H camera frame whose points all fall on the pw×ph panel."""
    gw = 192
    gh = max(1, round(gw * H / W))
    xs, ys = (np.arange(gw) + 0.5) * W / gw, (np.arange(gh) + 0.5) * H / gh
    gx, gy = np.meshgrid(xs, ys)
    p = apply_h(h_inv, np.column_stack([gx.ravel(), gy.ravel()]))
    ok = ((p[:, 0] >= 0) & (p[:, 0] <= pw) & (p[:, 1] >= 0) & (p[:, 1] <= ph)).reshape(gh, gw)
    ok[[0, -1], :] = False   # a grid cell is inside by its centre: one cell of margin all round
    ok[:, [0, -1]] = False
    best, run = (0, None), np.zeros(gw, int)
    for y in range(gh):        # the largest rectangle under each row's histogram of runs
        run = np.where(ok[y], run + 1, 0)
        stack = []
        for x in range(gw + 1):
            hgt = run[x] if x < gw else 0
            start = x
            while stack and stack[-1][1] >= hgt:
                start, sh = stack.pop()
                if sh * (x - start) > best[0]:
                    best = (sh * (x - start), (start, y - sh + 1, x, y + 1))
            stack.append((start, hgt))
    if not best[1]:
        return None
    x0, y0, x1, y1 = best[1]
    x0, x1 = x0 + 1, x1 - 1   # the margin cell: the edge cells' centres were inside, their outer halves may not be
    y0, y1 = y0 + 1, y1 - 1
    return [x0 * W / gw, y0 * H / gh, x1 * W / gw, y1 * H / gh]


# ---- the calibration pattern -----------------------------------------------------------------------------------------

def crc8(data):
    c = 0
    for b in data:
        c ^= b
        for _ in range(8):
            c = ((c << 1) ^ 0x07) & 0xff if c & 0x80 else (c << 1) & 0xff
    return c


def cal_geometry(cell):
    """A calibration tile's parts in panel px: cell side, gap between cells, white margin, tile w×h, black between tiles."""
    g = max(1, round(cell / 3))
    return dict(c=cell, g=g, m=cell, tw=C * cell + (C - 1) * g + 2 * cell, th=R * cell + (R - 1) * g + 2 * cell, sep=2 * cell)


def cal_bits(col, row):
    """32 cells, True = a hole: col, row, CRC-8 (low bit first), then the reference row."""
    word = col | row << 8 | crc8([col, row]) << 16
    return [bool(word >> i & 1) for i in range(24)] + [i % 2 == 1 for i in range(8)]


def cal_tiles(pw, ph, cell):
    """The pattern's tiles on a pw×ph panel: (col, row, x0, y0), the grid centred."""
    k = cal_geometry(cell)
    sx, sy = k['tw'] + k['sep'], k['th'] + k['sep']
    nx, ny = min(256, (pw + k['sep']) // sx), min(256, (ph + k['sep']) // sy)
    ox, oy = (pw - (nx * sx - k['sep'])) / 2, (ph - (ny * sy - k['sep'])) / 2
    return [(i, j, ox + i * sx, oy + j * sy) for j in range(ny) for i in range(nx)]


def cell_box(i, x0, y0, c, g, m):
    """Cell i of a tile whose top-left corner is (x0, y0): (x0, y0, x1, y1)."""
    x, y = x0 + m + (i % C) * (c + g), y0 + m + (i // C) * (c + g)
    return x, y, x + c, y + c


# ---- finding the pattern in a frame ----------------------------------------------------------------------------------

def quad_corners(pts):
    """The four corners of a convex quadrilateral's outline points, in clockwise order on screen."""
    c = pts.mean(0)
    a = pts[np.argmax(((pts - c) ** 2).sum(1))]
    b = pts[np.argmax(((pts - a) ** 2).sum(1))]
    n = np.array([a[1] - b[1], b[0] - a[0]])
    side = (pts - a) @ n
    q = np.array([a, pts[np.argmax(side)], b, pts[np.argmin(side)]])
    ang = np.arctan2(q[:, 1] - c[1], q[:, 0] - c[0])
    return q[np.argsort(ang)]


def find_tiles(img, cell):
    """The pattern's tiles read in a grey frame: [(col, row, corners in the frame clockwise from the tile's top-left)],
    and how many tile-like shapes were seen."""
    from scipy import ndimage
    k = cal_geometry(cell)
    lo, hi = np.percentile(img, [2, 98])
    lab, n = ndimage.label(img > (lo + hi) / 2)
    tile_uv = np.array([[0, 0], [k['tw'], 0], [k['tw'], k['th']], [0, k['th']]], float)
    centres = np.array([[(x0 + x1) / 2, (y0 + y1) / 2] for x0, y0, x1, y1 in (cell_box(i, 0, 0, k['c'], k['g'], k['m']) for i in range(32))])
    found, shapes = [], 0
    for i, sl in enumerate(ndimage.find_objects(lab), 1):
        if sl is None or sl[0].stop - sl[0].start < 8 or sl[1].stop - sl[1].start < 12:
            continue
        if sl[0].start == 0 or sl[1].start == 0 or sl[0].stop == img.shape[0] or sl[1].stop == img.shape[1]:
            continue   # cut by the frame's edge
        solid = ndimage.binary_fill_holes(lab[sl] == i)
        holes = solid.sum() - (lab[sl] == i).sum()
        if holes < 0.05 * solid.sum():
            continue   # no holes: not a tile
        shapes += 1
        edge = solid & ~ndimage.binary_erosion(solid)
        ys, xs = np.nonzero(edge)
        q = quad_corners(np.column_stack([xs + sl[1].start, ys + sl[0].start]).astype(float))
        r = max(1.0, 0.2 * cell * np.hypot(*(q[1] - q[0])) / k['tw'])
        for flip in (False, True):
            for rot in range(4):
                order = [(rot + j) % 4 for j in range(4)]
                if flip:
                    order = [order[0], order[3], order[2], order[1]]
                img_q = q[order]
                h = fit_h(tile_uv, img_q)
                pc = apply_h(h, centres)
                lv = np.array([img[max(0, int(y - r)):int(y + r) + 1, max(0, int(x - r)):int(x + r) + 1].mean() for x, y in pc])
                white, black = lv[24::2], lv[25::2]
                if white.min() - black.max() < 0.25 * (hi - lo):
                    continue
                bits = lv[:24] < (white.mean() + black.mean()) / 2
                col = sum(1 << b for b in range(8) if bits[b])
                row = sum(1 << b for b in range(8) if bits[8 + b])
                if crc8([col, row]) == sum(1 << b for b in range(8) if bits[16 + b]):
                    found.append((col, row, img_q))
                    break
            else:
                continue
            break
    return found, shapes


# ---- the saved views -------------------------------------------------------------------------------------------------

def views():
    try:
        return json.load(open(VIEW))
    except (OSError, ValueError):
        return {}


def load(size=None):
    """The view for a camera size 'WxH' (the latest made when None), its homography and rectangle scaled when it was
    made at another size of the same field of view. None when there is none."""
    vs = views()
    if not vs:
        return None
    e = dict(vs.get(size) or max(vs.values(), key=lambda v: v['when']))
    if size and e['size'] != size:
        (W, H), (w0, h0) = map(int, size.split('x')), map(int, e['size'].split('x'))
        s = np.diag([W / w0, H / h0, 1])
        e['h'] = (s @ np.array(e['h'])).tolist()
        e['rect'] = [e['rect'][0] * W / w0, e['rect'][1] * H / h0, e['rect'][2] * W / w0, e['rect'][3] * H / h0]
        e['scaled_from'], e['size'] = e['size'], size
    return e


def layout(e, tiles=4, motion=False):
    """The test picture's tiles in the view's rectangle, in camera px: tiles across (more when fewer than three rows
    fit), each [cell boxes (x0, y0, x1, y1) × 32], its centre; `motion` keeps the first and last row only and gives the
    rows between them to the moving bar (`bar`: x0, y0, x1, y1)."""
    x0, y0, x1, y1 = e['rect']
    while True:
        c = (x1 - x0) / tiles / (C + (C - 1) / 3 + 2)
        tw, th = (x1 - x0) / tiles, R * c + (R - 1) * c / 3 + 2 * c
        rows = int((y1 - y0) // th)
        if rows >= 3 or tiles > 40:
            break
        tiles += 1
    oy = y0 + ((y1 - y0) - rows * th) / 2
    keep = [0, rows - 1] if motion else range(rows)
    out = []
    for j in keep:
        for i in range(tiles):
            tx, ty = x0 + i * tw, oy + j * th
            cells = [cell_box(k, tx, ty, c, c / 3, c) for k in range(32)]
            out.append(dict(cells=cells, centre=(tx + tw / 2, ty + th / 2), row=j))
    return dict(tiles=out, bar=(x0, oy + th, x1, oy + (rows - 1) * th) if motion else None, cell=c)


# ---- cal -------------------------------------------------------------------------------------------------------------

def grab_frame(size, fps, frames=3):
    """The last of a few MJPEG frames of the camera's USB stream, grey, full size."""
    import io, re, subprocess
    from PIL import Image
    W, H = map(int, size.split('x'))
    path = os.path.join(os.environ.get('TMPDIR', '/tmp'), 'view.mjpg')
    r = subprocess.run(['v4l2-ctl', '-d', camlib.VIDEO, f'--set-fmt-video=width={W},height={H},pixelformat=MJPG',
                        f'--set-parm={fps}', '--stream-mmap', f'--stream-count={frames}', f'--stream-to={path}', '--verbose'],
                       capture_output=True, text=True, timeout=30)
    sizes = [int(m[1]) for m in re.finditer(r'bytesused: (\d+)', r.stdout + r.stderr)]
    data = open(path, 'rb').read()
    os.remove(path)
    if not sizes:
        sys.exit(f'no frame from {camlib.VIDEO}: {(r.stderr or r.stdout).strip()[-200:]}')
    last = sum(sizes[:-1])
    return np.asarray(Image.open(io.BytesIO(data[last:last + sizes[-1]])).convert('L'), float)


def show_pattern(cell, monitor):
    """clock.py --cal on the panel; returns (process, panel w, panel h) once it is drawn at its full size."""
    import subprocess
    args = [sys.executable, os.path.join(HERE, 'clock.py'), '--cal', '--cell', str(cell)] + (['--monitor', monitor] if monitor else [])
    p = subprocess.Popen(args, stdout=subprocess.PIPE, text=True)
    line = p.stdout.readline().split()      # 'shown W H', or nothing when it failed
    if len(line) != 3:
        p.terminate()
        sys.exit('the pattern window did not come up')
    return p, int(line[1]), int(line[2])


def cal(a):
    W, H = map(int, a.size.split('x'))
    t0 = time.monotonic()
    was = camlib.stream_release()
    best = None
    try:
        for cell in a.cell:
            p, pw, ph = show_pattern(cell, a.monitor)
            t1, tries = time.monotonic(), 0
            try:
                while True:   # the camera's exposure settles on the pattern within a second or two: grab until read
                    img = grab_frame(a.size, a.fps)
                    found, shapes = find_tiles(img, cell)
                    tries += 1
                    if len(found) >= 3 or time.monotonic() - t1 > a.settle:
                        break
            finally:
                p.terminate()
                p.wait(5)
            print(f'cell {cell} px: {len(found)} tiles read of {shapes} seen ({tries} grabs, {time.monotonic() - t1:.1f} s)', flush=True)
            if not best or len(found) > len(best[1]):
                best = (cell, found, pw, ph)
            if len(found) >= 3:
                break
    finally:
        camlib.stream_restore(was)
    cell, found, pw, ph = best
    if not found:
        sys.exit('no tile read: is the camera looking at the panel, in focus? (the cell sizes tried: '
                 f'{", ".join(map(str, a.cell))} px; another with --cell)')
    k = cal_geometry(cell)
    where = {(c_, r_): (x, y) for c_, r_, x, y in cal_tiles(pw, ph, cell)}
    src, dst = [], []
    for col, row, q in found:
        if (col, row) not in where:
            continue
        x, y = where[(col, row)]
        src += [[x, y], [x + k['tw'], y], [x + k['tw'], y + k['th']], [x, y + k['th']]]
        dst += q.tolist()
    h = fit_h(src, dst)
    rms = float(np.sqrt(((apply_h(h, src) - np.array(dst)) ** 2).sum(1).mean()))
    hi = np.linalg.inv(h)
    rect = largest_rect(hi, W, H, pw, ph)
    if not rect:
        sys.exit('the panel fills no part of the camera\'s frame')
    quad = apply_h(hi, [[0, 0], [W, 0], [W, H], [0, H]])
    turn = float(np.degrees(np.arctan2(*(apply_h(h, [[pw / 2 + 100, ph / 2]]) - apply_h(h, [[pw / 2, ph / 2]]))[0][::-1])))
    e = dict(size=a.size, fps=a.fps, codec='mjpeg', monitor=a.monitor or 'eDP*', panel=[pw, ph], cell=cell, tiles_read=len(found), rms_px=round(rms, 2), h=h.tolist(), rect=[round(v, 1) for v in rect],
             seen=quad.round(1).tolist(), turn_deg=round(turn, 2), when=time.strftime('%F %T'))
    vs = views()
    vs[a.size] = e
    json.dump(vs, open(VIEW, 'w'), indent=1)
    sx, sy = quad[:, 0], quad[:, 1]
    print(f'{a.size}: {len(found)} tiles, fit rms {rms:.1f} camera px; sees panel x {sx.min():.0f}–{sx.max():.0f}, '
          f'y {sy.min():.0f}–{sy.max():.0f} of {pw}×{ph}, turned {turn:+.1f}°; picture rectangle '
          f'{rect[2] - rect[0]:.0f}×{rect[3] - rect[1]:.0f} of {W}×{H}; {time.monotonic() - t0:.1f} s; saved {VIEW}')
    if a.show:   # where the camera looks, on the panel for a moment
        import subprocess
        subprocess.Popen([sys.executable, os.path.join(HERE, 'clock.py'), '--outline', '--size', a.size, '--secs', str(a.show)]
                         + (['--monitor', a.monitor] if a.monitor else []), stdout=subprocess.DEVNULL, start_new_session=True)


if __name__ == '__main__':
    import argparse
    ap = argparse.ArgumentParser()
    ap.add_argument('mode', choices=['cal', 'show'])
    ap.add_argument('--size', default='3840x2160')
    ap.add_argument('--fps', type=int, default=30)
    ap.add_argument('--cell', type=lambda s: [int(x) for x in s.split(',')], default=[8, 12, 5],
                    help='cell sides in panel px, tried in order until three tiles are read')
    ap.add_argument('--settle', type=float, default=3, help='s to keep grabbing while the exposure settles on the pattern')
    ap.add_argument('--show', type=float, default=3, help='s to show the view\'s outline on the panel after (0: not)')
    ap.add_argument('--monitor')
    a = ap.parse_args()
    os.makedirs(DATA, exist_ok=True)
    if a.mode == 'cal':
        cal(a)
    else:
        print(json.dumps(views(), indent=1))
