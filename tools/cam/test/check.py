#!/usr/bin/env python3
"""`cam check [smoke|full]`: the regression set as one table, a line a case, PASS / FAIL by its thresholds; details
into $MICROCAM_LOGS/check-<date>.log and the testers' own logs. Exit code = the failures.

  smoke  the menu's memory back to 0 after the menu leaves · no microcam_canvas pool stuck closing · a stream start and
         stop (MJPEG 4K30: ≥ 55 of 60 frames, microcamd sees the stop within 5 s) · a refused start, then a normal one
  full   smoke + the heavy layouts edge / full / quarter / swap (no not-shown, the heap's minimum) + the covered cursor paced
         by HDMI (median ≤ 40 ms) + a 4K recording on HDMI's frame with USB stopped and started, and a photo during it
         (onepool.py shared photo) + USB's JPEG rings at 4K30, 4K45, 1080p120 on the latency clock (rings.py) + a paused
         output beside a recording (pause.py hdmi usb) + the player: play, pause, seek, resume, the end, the menu while
         paused (play.py) + the SD card as the laptop's disk and back (card.py share take photo) + a 4K recording in
         H.265 and in H.264 (record.py h265 h264)
The menu cases run beside a host's USB stream when one runs (as in use); the stream cases free it through the host's
stream hooks and give it back. What the bench needs: docs/BENCH.md.
"""
import json, os, subprocess, sys, time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(HERE, '..'))
import camlib  # noqa: E402

full = (sys.argv[1:] or ['smoke'])[0] == 'full'
rep = camlib.Report('check')
t0 = time.time()


def tester(args, limit):
    """A tester's --json lines, each a line of this table under the tester's name."""
    p = subprocess.Popen([sys.executable, os.path.join(HERE, args[0]), '--json', *args[1:]], stdout=subprocess.PIPE,
                         stderr=subprocess.PIPE, text=True)
    try:
        out, err = p.communicate(timeout=limit)
    except subprocess.TimeoutExpired:
        # ended as a tester ends on SIGTERM: what it started stopped, what it set put back; killed only if that hangs
        p.terminate()
        try:
            out, err = p.communicate(timeout=60)
        except subprocess.TimeoutExpired:
            p.kill()
            out, err = p.communicate()
        err = f'{err}\ntimed out after {limit} s'
    rep.detail(out + err)
    rows = [json.loads(l) for l in out.splitlines() if l.startswith('{')]
    if not rows:
        rep.case(args[0][:-3], False, (err.strip().splitlines() or ['no result'])[-1][:90])
    for r in rows:
        case, ok, note = r.pop('case'), r.pop('ok'), r.pop('note')
        rep.case(f'{args[0][:-3]} {case}', ok, note, **r)


# the menu's memory after the menu leaves
try:
    menu = json.load(open(os.path.join(camlib.REPO, 'tools/ui/demo-menu.json')))
    c = camlib.Menu()
    c.ask({'cmd': 'hello', 'take': True})
    r = c.ask({'cmd': 'commit', 'base': 0, 'ops': [{'op': 'replace', 'nodes': menu['nodes']}]})
    time.sleep(2)
    during = camlib.menu_held()
    c.close()
    after = None
    for _ in range(15):
        time.sleep(0.2)
        after = camlib.menu_held()
        if after == 0:
            break
    rep.case('menu memory after it leaves', after == 0 and r.get('ok') is True,
             '' if r.get('ok') else f'commit refused: {r.get("error")}', during=during, after=after)
except camlib.Gone as e:
    rep.case('menu memory after it leaves', False, str(e))

# no pool stuck closing (read twice, a second apart: a pool closes for a moment in every transition)
rc, out = camlib.sh('grep "^pool" /proc/microcam_canvas; sleep 1; echo --; grep "^pool" /proc/microcam_canvas', 10, log=False)
first, _, then = out.partition('--\n')
stuck = {l for l in first.splitlines() if 'closing' in l} & {l for l in then.splitlines() if 'closing' in l}
rep.case('no pool stuck closing', not stuck and rc == 0, '; '.join(stuck)[:90], pools=len(then.splitlines()))

if full:
    tester(['heavy.py', 'edge', 'full', 'quarter', 'swap'], 260)
    tester(['cursor.py', '--layout', 'full', '--paced', 'hdmi', '--secs', '10'], 70)
    tester(['onepool.py', 'shared', 'photo'], 160)
    tester(['rings.py'], 150)
    tester(['pause.py', 'hdmi', 'usb'], 200)
    tester(['play.py', 'play', 'ended', 'menu'], 200)
    tester(['card.py', 'share', 'take', 'photo'], 200)
    tester(['record.py', 'h265', 'h264'], 120)

was = camlib.stream_release()
try:
    # a stream start and stop
    n, err = camlib.host_stream('mjpeg', '3840x2160', 30, 60, 14)
    stopped, waited = False, 0.0
    ts = time.time()
    while time.time() - ts < 5:
        try:
            stopped = not camlib.status(2).get('host_stream')
        except camlib.Gone:
            stopped = False
        if stopped:
            break
        time.sleep(0.25)
    waited = round(time.time() - ts, 1)
    rep.case('stream start and stop', n >= 55 and stopped, err, frames=f'{n}/60', stop_seen_s=waited if stopped else None)
    time.sleep(2)
    tester(['uvcanswer.py', '--order', 'rn'], 60)
finally:
    camlib.stream_restore(was)
print(f'{rep.fails} failed · {time.time() - t0:.0f} s · details {rep.path}')
sys.exit(rep.fails)
