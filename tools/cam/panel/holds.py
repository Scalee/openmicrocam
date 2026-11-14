#!/usr/bin/env python3
"""How long each reader of a canvas pool holds a canvas, over time — from /proc/microcam_canvas_holds (the module keeps each
reader's last 256 releases). Per reader: hold ms (median, p10, p90, max), the step between hand-overs, and the holds in
time order as a row of ms. usage: holds.py [--json] [--raw FILE]"""
import argparse, json, re, statistics, sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
import camlib

ap = argparse.ArgumentParser()
ap.add_argument('--json', action='store_true')
ap.add_argument('--raw', help='also save the module listing to this file')
a = ap.parse_args()

_, text = camlib.sh('cat /proc/microcam_canvas_holds', timeout=15, log=False)
if a.raw:
    Path(a.raw).write_text(text)
names, rows = {}, {}
for line in text.splitlines():
    if m := re.match(r'# pool (\d+) reader (\d+) (\S+)', line):
        names[(int(m[1]), len([k for k in names if k[0] == int(m[1])]))] = f'mod {m[2]} {m[3]}'
    elif m := re.match(r'(\d+) (\d+) (\d+) (\d+) (\d+)$', line):
        rows.setdefault((int(m[1]), int(m[2])), []).append((int(m[4]), int(m[5]), int(m[3])))


def q(xs, f):
    s = sorted(xs)
    return s[min(len(s) - 1, int(f * len(s)))]


out = []
for key, hs in sorted(rows.items()):
    hs.sort()
    hold = [(r - h) / 1e6 for h, r, _ in hs]
    step = [(b[0] - a_[0]) / 1e6 for a_, b in zip(hs, hs[1:])]
    out.append(dict(pool=key[0], reader=key[1], name=names.get(key, '?'), n=len(hs),
                    hold_ms=dict(median=statistics.median(hold), p10=q(hold, .1), p90=q(hold, .9), max=max(hold)),
                    step_ms=dict(median=statistics.median(step), max=max(step)) if step else None,
                    series=[round(x, 1) for x in hold]))
if a.json:
    print(json.dumps(out))
else:
    for r in out:
        h, s = r['hold_ms'], r['step_ms'] or {}
        print(f"pool {r['pool']} reader {r['reader']} ({r['name']}): {r['n']} holds, ms median {h['median']:.1f} "
              f"p10 {h['p10']:.1f} p90 {h['p90']:.1f} max {h['max']:.1f}; hand-over step median {s.get('median', 0):.1f} "
              f"max {s.get('max', 0):.1f}")
        print('  ' + ' '.join(f'{x:.0f}' for x in r['series']))
