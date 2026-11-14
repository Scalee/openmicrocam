#!/usr/bin/env python3
"""The panel rig's pictures, a window of its own: fullscreen on the laptop panel (the camera's view) by itself, no browser.
The clock: tiles of 4×8 cells; cells 0..18 the Gray code of the display frame number n = round(wall-clock ms / P)
(its 19 low bits: a reader takes the rest from its own clock), cells 19..23 the tile's height on the panel,
round(31 · centre / panel height) in binary (the panel lights its rows top to bottom over a refresh, so a tile's number
shows that much later), cells 24..31 references W B W B W B W B. n is taken for the time the compositor predicts the
frame reaches the panel (the frame clock's own time when it gives none), so the compositor's queue is not inside the
measurement. The tiles are laid out in the camera's view (`view.py cal`, view.json): upright in the camera's frame and
filling it, and a thin green line just inside the camera's frame edge shows where the camera looks (--outline: the line
alone); with no view made, or --whole, they tile the whole panel at --cell px.
--motion: the view's first and last row of clock tiles and, between them, a bar sweeping across at the panel's rate:
a recording's lost frames are read from the counter (`framegaps.py`). --cal: the view calibration's pattern (view.py).
Prints `shown W H` (the panel in px) once it is drawn at its full size.

usage: clock.py [--motion | --outline | --cal | --whole] [--size WxH] [--tiles N] [--cell PX] [--p MS] [--monitor CONNECTOR] [--secs S]
       (Esc or q quits; default monitor: the laptop panel, eDP*; --size: the camera size whose view to draw in)
"""
import argparse, os, sys, time

import gi
gi.require_version('Gtk', '4.0')
gi.require_version('Gdk', '4.0')
gi.require_version('Gsk', '4.0')
gi.require_version('Graphene', '1.0')
from gi.repository import Gdk, GLib, Graphene, Gsk, Gtk  # noqa: E402
import numpy as np  # noqa: E402

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import view  # noqa: E402

ap = argparse.ArgumentParser()
ap.add_argument('--cal', action='store_true', help='the view calibration\'s pattern')
ap.add_argument('--motion', action='store_true', help='a moving bar between two rows of clock tiles')
ap.add_argument('--outline', action='store_true', help='only the outline of the camera\'s view')
ap.add_argument('--whole', action='store_true', help='the clock over the whole panel, not in the camera\'s view')
ap.add_argument('--size', help='the camera size WxH whose view to draw in (default: the latest made)')
ap.add_argument('--tiles', type=int, default=4, help='clock tiles across the view; a reader must take the same')
ap.add_argument('--cell', type=int, default=8, help='cell side in panel px (--cal, --whole)')
ap.add_argument('--p', type=float, default=1000 / 120, help='ms per clock step; a reader\'s --p must match')
ap.add_argument('--monitor', help='connector name (default: the laptop panel, eDP*)')
ap.add_argument('--secs', type=float, help='quit after this long')
a = ap.parse_args()

C, R = 8, 4
CODE = 19   # bits of the frame number; 5 more cells carry the tile's height
SWEEP = 2.0  # s for the bar to cross the view
OFFSET = time.time() - time.monotonic()  # GTK's frame times are CLOCK_MONOTONIC, the decoder's are wall clock
BLACK, WHITE, GREEN = Gdk.RGBA(), Gdk.RGBA(), Gdk.RGBA()
BLACK.parse('#000')
WHITE.parse('#fff')
GREEN.parse('#0f0')
VIEW = None if (a.cal or a.whole) else view.load(a.size)


def quad_path(pts):
    b = Gsk.PathBuilder.new()
    b.move_to(*pts[0])
    for x, y in pts[1:]:
        b.line_to(x, y)
    b.close()
    return b.to_path()


def box_quad(h_inv, x0, y0, x1, y1):
    return view.apply_h(h_inv, [[x0, y0], [x1, y0], [x1, y1], [x0, y1]]).tolist()


class Clock(Gtk.Widget):
    def __init__(self, mon):
        super().__init__()
        self.mon, self.shown, self.plan = mon.get_geometry(), False, None
        self.add_tick_callback(lambda w, clock: (w.queue_draw(), True)[1])

    def make_plan(self, w, h):
        """What is drawn where, for this panel size: tiles as (cell paths or rects, height code), the bar's frame."""
        if a.cal:
            k = view.cal_geometry(a.cell)
            return [(Graphene.Rect().init(x, y, k['tw'], k['th']),
                     [Graphene.Rect().init(*b[:2], k['c'], k['c']) for i, on in enumerate(view.cal_bits(col, row)) if on
                      for b in [view.cell_box(i, x, y, k['c'], k['g'], k['m'])]])
                    for col, row, x, y in view.cal_tiles(w, h, a.cell)]
        if VIEW:
            hi = np.linalg.inv(np.array(VIEW['h']))
            lay = view.layout(VIEW, a.tiles, a.motion)
            tiles = []
            for t in lay['tiles']:
                cy = view.apply_h(hi, [t['centre']])[0][1]
                tiles.append(([quad_path(box_quad(hi, *b)) for b in t['cells']], round(31 * min(max(cy, 0), h) / h)))
            W, H = map(int, VIEW['size'].split('x'))
            e = 0.004 * W   # inside the frame's edge, outside the rectangle the readers take
            return dict(tiles=[] if a.outline else tiles, hi=hi, bar=None if a.outline else lay['bar'], cell=lay['cell'],
                        outline=quad_path(box_quad(hi, e, e, W - e, H - e)))
        cell, gap, margin = a.cell, round(a.cell / 3), 2 * a.cell
        tw, th = C * (cell + gap) - gap + 2 * margin, R * (cell + gap) - gap + 2 * margin
        tiles = []
        for ty in range(0, h, th):
            at = round(31 * min(ty + th / 2, h) / h)
            for tx in range(0, w, tw):
                tiles.append(([Graphene.Rect().init(tx + margin + (i % C) * (cell + gap), ty + margin + (i // C) * (cell + gap), cell, cell)
                               for i in range(C * R)], at))
        return dict(tiles=tiles, hi=None, bar=None, outline=None)

    def do_snapshot(self, s):
        w, h = self.get_width(), self.get_height()
        if not self.plan or self.plan[0] != (w, h):
            self.plan = ((w, h), self.make_plan(w, h))
        plan = self.plan[1]
        s.append_color(BLACK, Graphene.Rect().init(0, 0, w, h))
        if a.cal:
            for frame, holes in plan:
                s.append_color(WHITE, frame)
                for r in holes:
                    s.append_color(BLACK, r)
        else:
            clock = self.get_frame_clock()
            t = clock.get_current_timings() if clock else None
            us = (t.get_predicted_presentation_time() if t else 0) or (clock.get_frame_time() if clock else 0)
            ms = (us / 1e6 + OFFSET) * 1000
            m = round(ms / a.p) & (1 << CODE) - 1   # the Gray code is cyclic: m's wrap changes one cell too
            gray = m ^ (m >> 1)
            code = [bool((gray >> i) & 1) for i in range(CODE)]
            for cells, at in plan['tiles']:
                on = code + [bool((at >> i) & 1) for i in range(5)] + [i % 2 == 0 for i in range(8)]
                for c, o in zip(cells, on):
                    if not o:
                        continue
                    if isinstance(c, Gsk.Path):
                        s.append_fill(c, Gsk.FillRule.WINDING, WHITE)
                    else:
                        s.append_color(WHITE, c)
            if plan['outline']:
                s.append_stroke(plan['outline'], Gsk.Stroke.new(1.0), GREEN)
            if plan['bar']:
                x0, y0, x1, y1 = plan['bar']
                bw = (x1 - x0) / 16
                x = x0 + (ms / 1000 % SWEEP) / SWEEP * (x1 - x0 - bw)
                s.append_fill(quad_path(box_quad(plan['hi'], x, y0 + plan['cell'], x + bw, y1 - plan['cell'])), Gsk.FillRule.WINDING, WHITE)
        if not self.shown and (w, h) == (self.mon.width, self.mon.height):
            self.shown = True
            print(f'shown {w} {h}', flush=True)


def monitor():
    mons = Gdk.Display.get_default().get_monitors()
    mons = [mons.get_item(i) for i in range(mons.get_n_items())]
    want = [m for m in mons if (m.get_connector() == a.monitor if a.monitor else (m.get_connector() or '').startswith('eDP'))]
    if not want:
        sys.exit(f'no monitor {a.monitor or "eDP*"}; have: {", ".join(m.get_connector() or "?" for m in mons)}')
    return want[0]


def activate(app):
    win = Gtk.ApplicationWindow(application=app, title='microcam panel rig')
    mon = monitor()
    win.set_child(Clock(mon))
    win.set_cursor_from_name('none')
    keys = Gtk.EventControllerKey()
    keys.connect('key-pressed', lambda c, kv, code, st: (app.quit(), True)[1] if kv in (Gdk.KEY_Escape, Gdk.KEY_q) else False)
    win.add_controller(keys)
    win.fullscreen_on_monitor(mon)
    win.present()
    if a.secs:
        GLib.timeout_add(int(a.secs * 1000), lambda: app.quit())


if not (a.cal or a.whole) and not VIEW:
    print('no view of the camera made (view.py cal): the clock tiles the whole panel', file=sys.stderr)
app = Gtk.Application(application_id=None)
app.connect('activate', activate)
app.run([])
