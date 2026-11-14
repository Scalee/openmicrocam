#!/usr/bin/env python3
"""One pool per distinct frame on the camera, beside a host's USB 4K30 stream and HDMI 2160p:

  shared   a 4K recording on HDMI's frame (no scene): one pool, its readers and each one's hold and hand-over step; the
           USB stream stopped and started mid-recording; the MP4's frames and gaps
  clean    a scene (the demo menu), the recording at level 0: a pool of its own; then at the outputs' level (onto HDMI's
           pool) and back — the MP4's gaps at each move
  photo    a photo during a shared recording: one canvas to the photo's encoder; the photo written, the MP4 unbroken

usage: onepool.py [--json] [--secs S] case...        details: $MICROCAM_LOGS/onepool-<date>.log
Thresholds: shared — one pool read by DISP, USB's encoder and the recording's, the MP4 with no gap over 1.5 frame
periods; clean — the recording on a pool of its own beside the menu (the moves' gaps measured only); photo — the photo
written, no gap.
"""
import argparse, json, os, re, subprocess, sys, time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(HERE, '..'))
import camlib  # noqa: E402

ap = argparse.ArgumentParser()
ap.add_argument('case', nargs='+', choices=['shared', 'clean', 'photo'])
ap.add_argument('--json', action='store_true')
ap.add_argument('--secs', type=float, default=8)
a = ap.parse_args()
rep = camlib.Report('onepool', a.json)
MENU = json.load(open(os.path.join(camlib.REPO, 'tools/ui/demo-menu.json')))


def microcam(req):
    return camlib.microcam(req, 20)


def pools():
    """/proc/microcam_canvas: pool id → (canvases, [reader names])."""
    _, text = camlib.sh('cat /proc/microcam_canvas', 10, log=False)
    out, cur = {}, None
    for l in text.splitlines():
        if m := re.match(r'pool (\d+) \S+ rows \d+ size \d+ canvases (\d+) open', l):
            cur = int(m[1])
            out[cur] = (int(m[2]), [])
        elif l.startswith('pool '):
            cur = None
        elif cur is not None and (m := re.match(r'\s+reader (\d+) (\S+) handed (\d+)', l)):
            out[cur][1].append({'15': 'DISP', '2': 'venc'}.get(m[1], m[1]) + ' ' + m[2])
    return out


def holds():
    """Each reader's hold and hand-over step, ms (tools/cam/panel/holds.py)."""
    p = subprocess.run([sys.executable, os.path.join(camlib.REPO, 'tools/cam/panel/holds.py'), '--json'], capture_output=True, text=True, timeout=40)
    try:
        rows = json.loads(p.stdout)
    except ValueError:
        return {}
    return {f'{r["pool"]}:{r["name"]}': (round(r['hold_ms']['median'], 1), round((r['step_ms'] or {}).get('median', 0), 1)) for r in rows}


def record(mark_note):
    camlib.change('record-size', '4k')
    err = camlib.record_start()
    if err:
        mark_note.append(f'record: {err}'[:80])
        return False
    return True


def stop_and_check():
    path, _ = camlib.record_stop()
    if not path:
        return None, None, None, None
    frames, period, gaps = camlib.mp4_gaps(path)
    return path, frames, period, gaps


def shared():
    notes = []
    if not record(notes):
        return rep.case('shared', False, '; '.join(notes))
    time.sleep(a.secs / 2)
    p = pools()
    h = holds()
    rep.detail(f'pools {p}\nholds {h}')
    # USB's stream stopped and started mid-recording (the host's stream hooks); without them it runs on
    released = camlib.stream_release()
    time.sleep(3)
    p_off = pools() if released else 'not stopped (no stream-release hook)'
    camlib.stream_restore(released)
    time.sleep(a.secs / 2)
    path, frames, period, gaps = stop_and_check()
    one = len(p) == 1 and len(next(iter(p.values()))[1]) == 3
    ok = one and frames is not None and not gaps
    rep.case('shared', ok, f'pools {p}; USB off: {p_off}', frames=frames, period_ms=period, gaps=gaps[:5] if gaps else gaps,
             holds_ms={k: v[0] for k, v in h.items()}, step_ms={k: v[1] for k, v in h.items()})


def clean():
    notes = []
    c = camlib.Menu()
    c.ask({'cmd': 'hello', 'take': True})
    c.ask({'cmd': 'commit', 'base': 0, 'ops': [{'op': 'replace', 'nodes': list(MENU['nodes'])}]})
    time.sleep(1.5)
    try:
        if not record(notes):
            return rep.case('clean', False, '; '.join(notes))
        time.sleep(a.secs / 3)
        own = pools()
        moves = []
        for level in (3, 0):
            r = camlib.change('level-media', level)
            moves.append(f'level {level}: {r.get("error") or "ok"}'[:90])
            time.sleep(a.secs / 3)
            moves.append(str(pools()))
        path, frames, period, gaps = stop_and_check()
    finally:
        c.ask({'cmd': 'commit', 'base': 1, 'ops': [{'op': 'replace', 'nodes': []}]})
        c.close()
    apart = 1 in own and len(own) == 2
    rep.case('clean', apart, f'beside the menu {own}; ' + '; '.join(moves), frames=frames, period_ms=period, gaps=gaps)


def photo():
    notes, mark = [], camlib.log_mark()
    if not record(notes):
        return rep.case('photo', False, '; '.join(notes))
    time.sleep(a.secs / 2)
    camlib.press_photo()
    time.sleep(a.secs / 2)
    path, frames, period, gaps = stop_and_check()
    shot = [l for l in camlib.log_since(mark) if re.search(r'photo \d+ \S+ \d+ KB|photo failed|photo: ', l)]
    rep.detail('\n'.join(shot))
    written = any(re.search(r'photo \d+ \S+ \d+ KB', l) for l in shot)
    rep.case('photo', written and frames is not None and not gaps, (shot[-1][-100:] if shot else 'no photo line'), frames=frames, period_ms=period, gaps=gaps)


for case in a.case:
    {'shared': shared, 'clean': clean, 'photo': photo}[case]()
    time.sleep(2)
sys.exit(rep.fails)
