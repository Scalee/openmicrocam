#!/usr/bin/env python3
"""The cursor (an own-window group) over a full-colour picture with a menu panel on it, moved by `patch` commits;
commit → `presented` per output.

usage: cursor.py [--json] [--secs S] [--rate R] [--layout full|edge] [--paced hdmi|usb] [--churn K] [--big]
  full: the picture covers the screen (the covered case); edge: live in the right half; paced: the next move only once
  the last is presented there; churn K: every K-th commit also changes the panel; big: the panel is the whole screen.
Threshold: with --paced, that output's median commit → presented ≤ --max-ms (40). Details: $MICROCAM_LOGS/cursor-<date>.log
"""
import argparse, json, math, os, signal, sys, time

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), '..'))
import camlib  # noqa: E402

ap = argparse.ArgumentParser()
ap.add_argument('--json', action='store_true')
ap.add_argument('--secs', type=float, default=10)
ap.add_argument('--rate', type=float, default=30)
ap.add_argument('--layout', choices=['full', 'edge'], default='full')
ap.add_argument('--paced', choices=['hdmi', 'usb'])
ap.add_argument('--churn', type=int, default=0)
ap.add_argument('--big', action='store_true')
ap.add_argument('--max-ms', type=float, default=40)
ap.add_argument('--photo', default='/mnt/PHOTO/P00001.jpg')
a = ap.parse_args()
signal.alarm(int(a.secs + 40))  # bounded
rep = camlib.Report('cursor', a.json)
menu = json.load(open(os.path.join(camlib.REPO, 'tools/ui/demo-menu.json')))

img = lambda x, y, w, h: {'kind': 'image', 'id': 'pic', 'res': 'ph', 'x': x, 'y': y, 'w': w, 'h': h, 'fit': 'cover',
                          'full_colour': True}
LAYS = {'full': ([], img(0, 0, 1920, 1080)),
        'edge': ([{'kind': 'live', 'anchor': 'top-right', 'x': -960, 'y': 0, 'w': 960, 'h': 1080}], img(0, 0, 960, 1080))}


def panel(k):
    if a.big:
        return [{'kind': 'group', 'id': 'panel', 'level': 3, 'children': [
            {'kind': 'rect', 'x': 0, 'y': 0, 'w': 1920, 'h': 1080, 'fill': '#0c0e1260'},
            {'kind': 'text', 'x': 960, 'y': 500, 'size': 60, 'text': f'весь экран · {k}', 'fill': '#ffb300', 'align': 'centre'}]}]
    nodes = [dict(n) for n in menu['nodes'] if n.get('level') == 3 and 'anchor' not in n]
    nodes[0]['id'] = 'panel'
    nodes[0]['children'] = nodes[0]['children'] + [{'kind': 'text', 'x': 300, 'y': 640, 'size': 36, 'text': f'шаг {k}', 'fill': '#ffb300'}]
    return nodes


CUR = {'kind': 'group', 'id': 'cur', 'window': 'own', 'level': 2, 'x': 300, 'y': 400, 'children': [
    {'kind': 'rect', 'x': -24, 'y': -24, 'w': 48, 'h': 48, 'r': 24, 'fill': '#00e5ff'},
    {'kind': 'rect', 'x': -6, 'y': -6, 'w': 12, 'h': 12, 'r': 6, 'fill': '#ff3060'}]}

live, pic = LAYS[a.layout]
c = camlib.Menu()
c.ask({'cmd': 'hello', 'take': True})
c.ask({'cmd': 'resource', 'id': 'ph', 'file': a.photo})
time.sleep(2)
r = c.ask({'cmd': 'commit', 'base': 0, 'ops': [{'op': 'replace', 'nodes': live + [pic] + panel(0) + [CUR]}]})
rev = r.get('revision', 1)
time.sleep(2)
sent, kind = {}, {}
t0, n = time.monotonic(), 0
while time.monotonic() - t0 < a.secs:
    t = time.monotonic() - t0
    ops = [{'op': 'patch', 'id': 'cur', 'fields': {'x': round(300 + 250 * math.sin(t * 1.3)), 'y': round(400 + 300 * math.sin(t * 0.9))}}]
    k = 'move'
    if a.churn and n % a.churn == a.churn - 1:
        ops, k = [{'op': 'set', 'node': panel(n)[0]}] + ops, 'panel'
    c.send({'cmd': 'commit', 'base': rev, 'ops': ops})
    rev += 1
    n += 1
    sent[rev], kind[rev] = time.monotonic(), k
    if a.paced:
        want = rev
        with c.cv:
            c.cv.wait_for(lambda: any(e['event'] == 'presented' and e['output'] == a.paced and e['revision'] >= want
                                      for _, e in c.events[-20:]), 0.5)
    d = t0 + n / a.rate - time.monotonic()
    if d > 0:
        time.sleep(d)
time.sleep(1)
with c.cv:
    ev = list(c.events)
    refused = [m for m in c.replies.values() if not m.get('ok')]
c.ask({'cmd': 'commit', 'base': rev, 'ops': [{'op': 'replace', 'nodes': []}]})
c.close()
name = f'{a.layout}{" big" if a.big else ""}{f" churn {a.churn}" if a.churn else ""}{f" paced {a.paced}" if a.paced else ""}'
rep.detail(f'{name}: {n} commits, refused {refused[:2]}, events {len(ev)}')
for out in sorted({e['output'] for _, e in ev if e['event'] == 'presented'}):
    pres = [(tm, e['revision']) for tm, e in ev if e['event'] == 'presented' and e['output'] == out and e['revision'] in sent]
    for kk in ('move', 'panel'):
        lat = sorted((tm - sent[rv]) * 1000 for tm, rv in pres if kind[rv] == kk)
        tot = sum(1 for rv in kind if kind[rv] == kk)
        if not tot:
            continue
        q = lambda p: round(lat[min(len(lat) - 1, int(p * len(lat)))]) if lat else None
        ok = (bool(lat) and q(0.5) <= a.max_ms and not refused) if out == a.paced and kk == 'move' else None
        rep.case(f'{name} {out} {kk}', ok, f'refused {len(refused)}' if refused else '', shown=f'{len(lat)}/{tot}',
                 median_ms=q(0.5), p90_ms=q(0.9), max_ms=q(1.0))
if a.paced and not any(e['event'] == 'presented' and e['output'] == a.paced for _, e in ev):
    rep.case(f'{name} {a.paced} move', False, f'nothing presented on {a.paced}')
sys.exit(rep.fails)
