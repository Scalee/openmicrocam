#!/usr/bin/env python3
"""A recording in each codec on the camera, from the laptop, a host's USB stream on.

  h264, h265  the `record-codec` control at C, then a 4K recording for --secs: refused to change the codec meanwhile; at its middle
              the heap's listing of the video (the fixed owners left out) against microcamd's price; stopped, the MP4 closed, pulled to the laptop: ffprobe names the
              codec, ffmpeg decodes every frame with no error, the frames' timing has no gap; its Mbit/s
  The controls it sets are put back as it found them (`camlib.change`).

usage: record.py [--json] [--secs S] case...        details: $MICROCAM_LOGS/record-<date>.log
Thresholds: the codec as asked; frames at least 90 % of the rate × the time; no gap over 1.5 periods; no decode
error; the video's listing at most 0.5 MiB over the price.
"""
import argparse, json, os, subprocess, sys, tempfile, time, urllib.request

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(HERE, '..'))
import camlib  # noqa: E402

ap = argparse.ArgumentParser()
ap.add_argument('case', nargs='+', choices=['h264', 'h265'])
ap.add_argument('--json', action='store_true')
ap.add_argument('--secs', type=float, default=10)
a = ap.parse_args()
rep = camlib.Report('record', a.json)


# the heap's owners that are no video's: the command queue, microcamd's share (its scratch and tiles, the menu's RGN
# canvases), logs, ISP, GE, the JPEG decoder's buffer
FIXED = ('CMDMEM', 'app-mmaAlloc', 'microcam-rgn', 'sys-', 'ISP_', 'gfx_', 'interbuf')


def video_account():
    """(priced, held) MiB: microcamd's price of the video and the heap's listing without the fixed owners."""
    acc = camlib.microcam({'cmd': 'account'}, 6).get('account') or {}
    m = lambda b: round(b / (1 << 20), 1)
    return m(sum(b for _, b in acc.get('priced', []))), m(sum(b for n, b in acc.get('held', []) if not n.startswith(FIXED)))


def microcam(req):
    return camlib.microcam(req, 20)


def pull(path, to):
    """The camera's file `path` into the laptop's `to`, over the camera's httpd, the host's stream freed meanwhile: beside
    a 4K stream the network gets a tenth of the link."""
    d, f = os.path.split(path)
    was = camlib.stream_release()
    camlib.sh(f'killall httpd 2>/dev/null; httpd -p 8090 -h {d}', 10)
    try:
        with urllib.request.urlopen(f'http://{camlib.CAM}:8090/{f}', timeout=60) as r, open(to, 'wb') as out:
            while chunk := r.read(1 << 20):
                out.write(chunk)
    finally:
        camlib.sh('killall httpd', 10)
        camlib.stream_restore(was)


def case(codec):
    r = camlib.change('record-codec', codec)
    rep.case(f'{codec}: set', bool(r.get('ok')), error=r.get('error'))
    camlib.change('record-size', '4k')
    err = camlib.record_start()
    if err:
        rep.case(f'{codec}: recording', False, error=err)
        return
    stopped = False
    try:
        t0 = time.monotonic()
        other = 'h264' if codec == 'h265' else 'h265'
        r = camlib.change('record-codec', other)
        rep.case(f'{codec}: change refused', not r.get('ok'), error=r.get('error'))
        time.sleep(a.secs / 2)
        priced, held = video_account()
        time.sleep(max(0, a.secs - (time.monotonic() - t0)))
        path, err = camlib.record_stop()
        stopped = True
    finally:
        # ended early (an error, SIGTERM at the caller's limit): the recording does not outlive the tester
        if not stopped:
            camlib.record_stop()
    rep.case(f'{codec}: heap', held <= priced + 0.5, priced=priced, held=held)
    if not path:
        rep.case(f'{codec}: closed', False, note=err)
        return
    n, period, gaps = camlib.mp4_gaps(path)
    with tempfile.TemporaryDirectory(dir=os.environ.get('BIGTMP')) as tmp:
        local = os.path.join(tmp, 'r.mp4')
        pull(path, local)
        size = os.path.getsize(local)
        probe = json.loads(subprocess.run(['ffprobe', '-v', 'error', '-show_streams', '-of', 'json', local], capture_output=True, text=True).stdout or '{}')
        s = (probe.get('streams') or [{}])[0]
        dec = subprocess.run(['ffmpeg', '-v', 'error', '-i', local, '-f', 'null', '-'], capture_output=True, text=True)
        errors = [l for l in dec.stderr.splitlines() if l.strip()]
        rep.detail(f'{codec}: {path} ffprobe {s}\ndecode errors: {errors[:20]}')
    fps = round(1000 / period, 1) if period else None
    secs = n * period / 1000 if period else 0
    mbit = round(size * 8 / secs / 1e6, 1) if secs else None
    want = {'h264': 'h264', 'h265': 'hevc'}[codec]
    ok = s.get('codec_name') == want and fps and n >= 0.9 * fps * a.secs and not gaps and dec.returncode == 0 and not errors
    rep.case(f'{codec}: file', bool(ok), codec=s.get('codec_name'), tag=s.get('codec_tag_string'), frames=n, fps=fps, gaps=len(gaps),
             decode_errors=len(errors), mbit=mbit, mb=round(size / 1e6, 1), file=os.path.basename(path))


for c in a.case:
    try:
        case(c)
    except (RuntimeError, camlib.Gone) as e:
        rep.case(c, False, note=str(e))
sys.exit(1 if rep.fails else 0)
