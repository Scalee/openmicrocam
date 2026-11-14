#!/usr/bin/env python3
"""Throttled HTTP file server for the camera (serves $MICROCAM_STAGE like `python3 -m http.server`, but sends the body
in small chunks with pauses), on the laptop's address of the camera link. The stock USB controller driver hangs or
panics the camera on full-speed RNDIS receive; this keeps the receive rate low (docs/BENCH.md «Uploads to a stock
camera»). usage: serve-slow.py [port=8001] [kB/s=32]"""
import http.server, os, sys, time

sys.path.insert(0, os.path.join(os.path.dirname(os.path.realpath(__file__)), '..'))
import hostconf  # noqa: E402

PORT = int(sys.argv[1]) if len(sys.argv) > 1 else 8001
RATE = (int(sys.argv[2]) if len(sys.argv) > 2 else 32) * 1024      # bytes per second
CHUNK = 2048
ROOT = hostconf.get('MICROCAM_STAGE')
HOST = hostconf.get('MICROCAM_HOST')


class Slow(http.server.SimpleHTTPRequestHandler):
    def __init__(self, *a, **kw):
        super().__init__(*a, directory=ROOT, **kw)

    def copyfile(self, src, dst):
        while True:
            buf = src.read(CHUNK)
            if not buf:
                break
            dst.write(buf)
            dst.flush()
            time.sleep(len(buf) / RATE)

    def log_message(self, fmt, *args):
        sys.stderr.write('%s %s\n' % (time.strftime('%H:%M:%S'), fmt % args))


http.server.ThreadingHTTPServer.allow_reuse_address = True
with http.server.ThreadingHTTPServer((HOST, PORT), Slow) as httpd:
    print('serving %s on %s:%d at %d kB/s' % (ROOT, HOST, PORT, RATE // 1024), flush=True)
    httpd.serve_forever()
