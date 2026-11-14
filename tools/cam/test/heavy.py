#!/usr/bin/env python3
"""The heavy menu layouts on the camera. Be the menu: the demo menu, a moving own-window cursor (15 commits a second)
and a scenario; the heap's free memory sampled every 200 ms; after the menu leaves, whether its memory went back to 0.

  edge     live in the right half, two pictures left        full     a 4K card photo full screen, full colour
  quarter  a quarter-screen picture over the live picture   big      full + a translucent panel over the whole screen
  rec      edge + a 4K recording at the outputs' level (media 3: on HDMI's frame) with a photo at 5 s
  recclean edge + a 4K recording clean (media 0: a pool of its own) with a photo at 5 s
  swap     the menu replacing its own shown scene every 3 s: full, edge, big, quarter in turn, neighbours on two
           different photos (a new scene shares no tile with the one it replaces)
For the recordings also: microcamd's account at its peak (the video priced, the heap held), USB's rate, the MP4's frames and
gaps, the photo; a refused recording is followed by what `price` says of the alternatives (the outputs' level, 1440).

usage: heavy.py [--json] [--secs S] [--photo FILE] [--photo2 FILE] scenario...     details: $MICROCAM_LOGS/heavy-<date>.log
Thresholds (edge, full, quarter, swap): no refused commit, no not-shown, the menu's memory 0 within 3 s of the menu leaving;
big and rec are measured only.
"""
import argparse, json, math, os, re, signal, subprocess, sys, threading, time

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), '..'))
import camlib  # noqa: E402

ap = argparse.ArgumentParser()
ap.add_argument('scen', nargs='+', choices=['edge', 'full', 'quarter', 'big', 'rec', 'recclean', 'swap'])
ap.add_argument('--json', action='store_true')
ap.add_argument('--secs', type=float, default=12)
ap.add_argument('--photo', default='/mnt/PHOTO/P00001.jpg')
ap.add_argument('--photo2', default='/mnt/PHOTO/P00009.jpg')
a = ap.parse_args()
signal.alarm(int(len(a.scen) * (a.secs + 100)))  # bounded
rep = camlib.Report('heavy', a.json)
menu = json.load(open(os.path.join(camlib.REPO, 'tools/ui/demo-menu.json')))

img = lambda i, x, y, w, h: {'kind': 'image', 'id': f'p{i}', 'res': 'ph', 'level': 1, 'x': x, 'y': y, 'w': w, 'h': h,
                             'fit': 'cover', 'full_colour': True}
HALF = [{'kind': 'live', 'anchor': 'top-right', 'x': -960, 'y': 0, 'w': 960, 'h': 1080}]
LAYS = {'full': ([], [img(0, 0, 0, 1920, 1080)]),
        'quarter': ([], [img(0, 960, 540, 960, 540)]),
        'edge': (HALF, [img(0, 700, 300, 640, 360), img(1, 100, 100, 400, 300)])}
LAYS['big'] = LAYS['full']
LAYS['rec'] = LAYS['recclean'] = LAYS['swap'] = LAYS['edge']
PANEL = [{'kind': 'group', 'id': 'panel', 'level': 3, 'children': [
    {'kind': 'rect', 'x': 0, 'y': 0, 'w': 1920, 'h': 1080, 'fill': '#0c0e1260'},
    {'kind': 'text', 'x': 960, 'y': 500, 'size': 60, 'text': 'весь экран', 'fill': '#ffb300', 'align': 'centre'}]}]
CURSOR = {'kind': 'group', 'id': 'cur', 'window': 'own', 'level': 2, 'x': 300, 'y': 400, 'children': [
    {'kind': 'rect', 'x': -24, 'y': -24, 'w': 48, 'h': 48, 'r': 24, 'fill': '#00e5ff'}]}


def on(res, nodes):
    return [dict(n, res=res) if n.get('kind') == 'image' else n for n in nodes]


SWAPS = [on('ph', LAYS['full'][1]), HALF + on('ph2', LAYS['edge'][1]), on('ph', LAYS['full'][1]) + PANEL,
         on('ph2', LAYS['quarter'][1])]


def microcam(req):
    return camlib.microcam(req, 20)


def alternatives():
    """What `price` says of a 4K recording at the outputs' level and of a 1440 one: 'fits, peak N' or the shortfall."""
    out = []
    size = lambda s: {'cmd': 'set', 'name': 'record-size', 'value': s}
    for name, reqs in (('at level 3', [{'cmd': 'set', 'name': 'level-media', 'value': 3}, size('4k'), {'cmd': 'record'}]),
                       ('1440', [size('1440'), {'cmd': 'record'}])):
        p = (microcam({'cmd': 'price', 'with': reqs}).get('price') or {})
        sh = p.get('short')
        out.append(f'{name}: ' + (f"short, needs {sh['needs'] / 2**20:.1f} of {sh['room'] / 2**20:.1f}" if sh
                                  else f"fits, peak {p.get('peak', 0) / 2**20:.1f}"))
    return '; '.join(out)


def run(scen):
    live, pics = LAYS[scen]
    recording = scen in ('rec', 'recclean')
    samples, stop, peak = [], threading.Event(), {}

    def sampler():
        k = 0
        while not stop.is_set():
            try:
                f = camlib.heap_free()
                if recording and k % 5 == 0:
                    acc = camlib.account()
                    if acc['held'] > peak.get('held', 0):
                        peak.update(acc)
            except camlib.Gone:
                f = None
            if f is not None:
                samples.append(f)
            k += 1
            time.sleep(0.2)

    mark = camlib.log_mark()
    c = camlib.Menu()
    threading.Thread(target=sampler, daemon=True).start()
    c.ask({'cmd': 'hello', 'take': True})
    c.ask({'cmd': 'mem', 'reset': True})
    c.ask({'cmd': 'resource', 'id': 'ph', 'file': a.photo})
    if scen == 'swap':
        c.ask({'cmd': 'resource', 'id': 'ph2', 'file': a.photo2})
    time.sleep(1.5)
    r = c.ask({'cmd': 'commit', 'base': 0, 'ops': [{'op': 'replace', 'nodes': list(menu['nodes']) + live + pics
                                                     + (PANEL if scen == 'big' else []) + [CURSOR]}]})
    rev = r.get('revision', 1)
    notes = []
    recorded, pump0 = False, None
    if recording:
        camlib.change('level-media', 3 if scen == 'rec' else 0)
        camlib.change('record-size', '4k')
        time.sleep(1)
        err = camlib.record_start()
        recorded = not err
        notes.append(f'record: {err or "ok"}'[:120])
        if not recorded:
            notes.append(alternatives())
        pump0 = (camlib.status().get('pump'), time.monotonic())
    end, photo_at, t0 = time.monotonic() + a.secs, time.monotonic() + 5 if recording else None, time.monotonic()
    # microcamd's log taken as it comes: its ring holds minutes, not a long run
    log, at = [], {'mark': mark, 'next': time.monotonic() + 15}

    def drain():
        r = microcam({'cmd': 'log-since', 'from': at['mark']})
        log.extend(l for l in r.get('lines') or [] if ' photo ' in l or camlib.bad_lines([l]))
        at['mark'], at['next'] = int(r.get('message') or at['mark']), time.monotonic() + 15
    swap_at, k = t0 + 3, 0
    while time.monotonic() < end:
        t = time.monotonic() - t0
        if scen == 'swap' and time.monotonic() >= swap_at:
            k, swap_at = k + 1, swap_at + 3
            rev += 1
            c.send({'cmd': 'commit', 'base': rev - 1, 'ops': [{'op': 'replace', 'nodes': list(menu['nodes'])
                                                               + SWAPS[k % len(SWAPS)] + [CURSOR]}]})
        rev += 1
        c.send({'cmd': 'commit', 'base': rev - 1, 'ops': [{'op': 'patch', 'id': 'cur', 'fields': {
            'x': round(300 + 250 * math.sin(t * 1.3)), 'y': round(400 + 300 * math.sin(t * 0.9))}}]})
        if photo_at and time.monotonic() >= photo_at:
            photo_at = None
            camlib.press_photo()
        if time.monotonic() >= at['next']:
            drain()
        time.sleep(1 / 15)
    rec = {}
    if recording:
        p1, t1 = camlib.status().get('pump'), time.monotonic()
        if pump0 and pump0[0] and p1:
            rec['usb_fps'] = round((p1[0] - pump0[0][0]) / (t1 - pump0[1]), 1)
        path, _ = camlib.record_stop()
        camlib.change('level-media', 0)
        drain()
        shot = next((l for l in log if ' photo ' in l and re.search(r'\d+ KB in \d+ ms', l)), None)
        rec['photo'] = re.search(r'(\d+ KB in \d+ ms)', shot)[1] if shot and re.search(r'\d+ KB in \d+ ms', shot) else None
        if recorded and path:
            n, period, gaps = camlib.mp4_gaps(path)
            rec.update(mp4_frames=n, period_ms=period, gaps=gaps[:4])
            camlib.sh(f'rm -f {path}', 10)
        rec.update({f'{k}_peak': v for k, v in peak.items() if k in ('priced', 'held', 'free')})
    time.sleep(1)
    stop.set()
    with c.cv:
        refused = [m for m in c.replies.values() if not m.get('ok')]
        notshown = [e for _, e in c.events if e['event'] == 'not-shown']
    c.ask({'cmd': 'commit', 'base': rev, 'ops': [{'op': 'replace', 'nodes': []}]})
    c.ask({'cmd': 'release', 'id': 'ph'})
    if scen == 'swap':
        c.ask({'cmd': 'release', 'id': 'ph2'})
    c.close()
    held = None
    for _ in range(15):  # the menu left: its share back to 0 within 3 s
        time.sleep(0.2)
        held = camlib.menu_held()
        if held == 0:
            break
    drain()
    bad = camlib.bad_lines(log)
    rep.detail(f'{scen}: refused {refused[:3]}\nnot-shown {notshown[:3]}\nheap samples {samples}\n' + '\n'.join(bad[:30]))
    graded = scen in ('edge', 'full', 'quarter', 'swap')
    ok = (not refused and not notshown and held == 0) if graded else None
    if refused:
        notes.append(f'refused: {(refused[0].get("error") or "")[:60]}')
    if notshown:
        notes.append(f'not-shown: {notshown[0].get("reason", "")[:60]}')
    if held:
        notes.append(f'menu still holds {held} MiB')
    rep.case(scen, ok, '; '.join(notes), refused=len(refused), not_shown=len(notshown),
             heap_min=min(samples) if samples else None, menu_after=held, microcamd_warn=len(bad), **rec)


for s in a.scen:
    run(s)
    time.sleep(2)
sys.exit(rep.fails)
