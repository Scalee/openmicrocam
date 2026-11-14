#!/usr/bin/env python3
"""The SD card as the USB host's disk beside the webcam, from the laptop, a host's USB stream on (docs/BENCH.md).

  share  `card share`: the menu told `host`, the laptop sees the gadget's disk, reads the card's files (MB/s), writes,
         reads back and deletes a test file; USB video and microcamd's control go on meanwhile; a recording is refused;
         `card take` while the laptop holds it mounted is refused or not (told); the laptop unmounts and ejects it — the
         camera takes it back by itself (`camera`, mounted, the same files)
  take   `card share`, then `card take` with the disk not mounted on the laptop: back on the camera, mounted
  plug   by hand (not in `cam check`): a 1080p recording runs, the user pulls the card — `none` told, the recording
         stopped, /mnt let go; the user puts it back — `camera` told, mounted, the files there (waits 3 min for each)

usage: card.py [--json] case...        details: $MICROCAM_LOGS/card-<date>.log
Thresholds: the menu's `card` events within 3 s; the disk on the laptop within 15 s; the test file read back equal;
USB video at least 90 % of the stream's rate while the disk is read; microcamd answering; the recording refused.
"""
import argparse, glob, hashlib, json, os, subprocess, sys, tempfile, time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(HERE, '..'))
import camlib  # noqa: E402

ap = argparse.ArgumentParser()
ap.add_argument('case', nargs='+', choices=['share', 'take', 'plug'])
ap.add_argument('--json', action='store_true')
a = ap.parse_args()
rep = camlib.Report('card', a.json)


def microcam(req):
    return camlib.microcam(req, 20)


def run(*cmd, check=False):
    p = subprocess.run(cmd, capture_output=True, text=True)
    rep.detail(f'$ {" ".join(cmd)} → {p.returncode} {p.stdout.strip()[-300:]} {p.stderr.strip()[-300:]}')
    if check and p.returncode:
        raise RuntimeError(f'{cmd[0]}: {p.stderr.strip()}')
    return p


def event(c, since, want, secs=3.0):
    """Seconds from `since` (time.monotonic(), as camlib.Menu stamps events) to the menu's `card` event `want`, or None."""
    end = since + secs
    while time.monotonic() < end:
        for t, m in list(c.events):
            if t >= since and m.get('event') == 'card' and (m.get('card') or {}).get('at') == want:
                return round(t - since, 2)
        time.sleep(0.05)
    return None


def gadget_disk(secs=15.0):
    """The camera's mass-storage disk on the laptop (/dev/sdX), waited for."""
    end = time.time() + secs
    while time.time() < end:
        for p in glob.glob('/dev/disk/by-id/usb-Linux_File-Stor_Gadget*'):
            if '-part' not in p:
                return os.path.realpath(p)
        time.sleep(0.3)
    return None


def partition(disk, secs=10.0):
    """The disk's first partition once the laptop's kernel has read the table (the whole disk when it has none)."""
    end = time.time() + secs
    while time.time() < end:
        parts = sorted(glob.glob(disk + '[0-9]*'))
        if parts:
            return parts[0]
        time.sleep(0.2)
    return disk


def let_go(disk, secs=15.0):
    """The laptop's mounts of the disk undone: the desktop mounts it by itself and keeps it busy for a while after
    (`udisksctl unmount` «target is busy», 2026-11-14), so an unmount is tried again until `secs` are over; who still
    holds it then goes into the details."""
    end = time.time() + secs
    for part in sorted(glob.glob(disk + '*')):
        while mountpoint(part):
            if run('udisksctl', 'unmount', '-b', part).returncode == 0:
                break
            if time.time() > end:
                run('fuser', '-vm', part)
                return
            time.sleep(0.5)


def give_back(disk):
    """The card back on the camera however the case ended: the laptop lets it go, the camera takes it; ejected when
    the camera is still refused (the eject gives it back by itself)."""
    if (camlib.status(5).get('card') or {}).get('at') != 'host':
        return
    if disk:
        let_go(disk)
    if not camlib.take_card().get('ok') and disk:
        run('sudo', 'eject', disk)
        end = time.time() + 5
        while time.time() < end and (camlib.status(5).get('card') or {}).get('at') == 'host':
            time.sleep(0.3)
        camlib.take_card()


def mountpoint(dev):
    for l in open('/proc/mounts'):
        f = l.split()
        if f[0] == dev:
            return f[1]
    return None


def usb_fps(secs):
    """microcamd's pump frames per second over `secs`."""
    f0, t0 = camlib.status(5)['pump'][0], time.time()
    time.sleep(secs)
    return (camlib.status(5)['pump'][0] - f0) / (time.time() - t0)


def stream_fps():
    host = camlib.status(5).get('host_stream') or ''
    return float(host.split('@')[-1].split()[0]) if '@' in host else None


def camera_files():
    rc, out = camlib.sh('mount | grep -c " /mnt "; { ls /mnt/VIDEO; ls /mnt/PHOTO; } 2>/dev/null | wc -l', 10, log=False)
    m, n = (out.split() + ['0', '0'])[:2]
    return int(m) > 0, int(n)


def share(c):
    t = time.monotonic()
    r = camlib.share_card()
    if not r.get('ok'):
        raise RuntimeError(f"card-share: {r.get('error')}")
    told = event(c, t, 'host')
    disk = gadget_disk()
    return told, disk


def case_share():
    disk = None
    was_mounted, files = camera_files()
    c = camlib.Menu()
    c.ask({'cmd': 'hello', 'take': True})
    try:
        told, disk = share(c)
        rep.case('share: host told', told is not None, s=told)
        rep.case('share: disk on the laptop', disk is not None, dev=disk)
        if not disk:
            return
        part = partition(disk)
        end = time.time() + 4  # the desktop mounts it by itself, give it the time
        while not mountpoint(part) and time.time() < end:
            time.sleep(0.2)
        mnt = mountpoint(part)
        own = None
        if not mnt:
            own = tempfile.mkdtemp(prefix='microcamcard-', dir=os.environ.get('TMPDIR'))
            run('sudo', 'mount', '-o', f'uid={os.getuid()}', part, own, check=True)
            mnt = own
        names = [n for d in ('VIDEO', 'PHOTO') for n in os.listdir(os.path.join(mnt, d)) if os.path.isdir(os.path.join(mnt, d))] if mnt else []
        rep.case('share: the card\'s files', len(names) == files, laptop=len(names), camera=files)
        # read the largest recording while the stream runs
        videos = sorted(glob.glob(os.path.join(mnt, 'VIDEO', '*.mp4')), key=os.path.getsize)
        rate = stream_fps()
        if videos:
            v = videos[-1]
            run('sudo', 'sh', '-c', 'echo 1 > /proc/sys/vm/drop_caches')
            t0 = time.time()
            reader = subprocess.Popen(['dd', f'if={v}', 'of=/dev/null', 'bs=1M', 'count=64'], stderr=subprocess.PIPE, text=True)
            fps = usb_fps(3)
            reader.wait()
            mb = min(os.path.getsize(v), 64 << 20) / (1 << 20)
            secs = time.time() - t0
            ok = rate is None or fps >= 0.9 * rate
            rep.case('share: read beside video', ok, mb=round(mb, 1), mb_s=round(mb / secs, 2), usb_fps=round(fps, 1), stream=rate)
        # a test file written, read back, deleted
        data = os.urandom(8 << 20)
        test = os.path.join(mnt, 'MICROCAMTEST.BIN')
        t0 = time.time()
        with open(test, 'wb') as f:
            f.write(data)
            f.flush()
            os.fsync(f.fileno())
        w = time.time() - t0
        run('sudo', 'sh', '-c', 'echo 1 > /proc/sys/vm/drop_caches')
        back = open(test, 'rb').read()
        os.remove(test)
        os.sync()
        rep.case('share: write, read back', hashlib.md5(back).digest() == hashlib.md5(data).digest(), mb_s_write=round(8 / w, 2))
        r = microcam({'cmd': 'record'})
        rep.case('share: recording refused', not r.get('ok') and 'host' in (r.get('error') or ''), error=r.get('error'))
        r = camlib.take_card()
        rep.case('share: take while mounted', None, taken=r.get('ok'), error=r.get('error'))
        if r.get('ok'):
            return
        # the laptop lets it go: unmounted and ejected
        t = time.monotonic()
        if own:
            run('sudo', 'umount', own)
            os.rmdir(own)
        else:
            run('udisksctl', 'unmount', '-b', part)
        run('sudo', 'eject', disk)
        told = event(c, t, 'camera', 5)
        mounted, n = camera_files()
        rep.case('share: back after the eject', told is not None and mounted and n == files, s=told, mounted=mounted, files=n)
    finally:
        give_back(disk)
        c.close()


def case_take():
    disk = None
    c = camlib.Menu()
    c.ask({'cmd': 'hello', 'take': True})
    try:
        told, disk = share(c)
        rep.case('take: host told', told is not None and disk is not None, s=told, dev=disk)
        if disk:
            part = partition(disk)
            end = time.time() + 4
            while not mountpoint(part) and time.time() < end:
                time.sleep(0.2)
            let_go(disk)
        t = time.monotonic()
        r = camlib.take_card()
        told = event(c, t, 'camera')
        mounted, n = camera_files()
        rep.case('take: back on the camera', bool(r.get('ok')) and told is not None and mounted, s=told, files=n, error=r.get('error'))
    finally:
        give_back(disk)
        c.close()


def case_plug():
    c = camlib.Menu()
    c.ask({'cmd': 'hello', 'take': True})
    try:
        camlib.change('record-size', '1080')
        err = camlib.record_start()
        rep.case('plug: recording started', not err, error=err)
        time.sleep(3)
        print('>>> pull the SD card out', flush=True)
        t = time.monotonic()
        told = event(c, t, 'none', 180)
        time.sleep(1)
        st = camlib.status(5)
        recording = bool((st.get('desired') or {}).get('media'))
        mounted, _ = camera_files()
        rep.case('plug: pulled', told is not None and not recording and not mounted, s=told, recording=recording, mounted=mounted)
        print('>>> put the SD card back', flush=True)
        t = time.monotonic()
        told = event(c, t, 'camera', 180)
        mounted, n = camera_files()
        rep.case('plug: back', told is not None and mounted and n > 0, s=told, files=n)
    finally:
        c.close()


for case in a.case:
    try:
        {'share': case_share, 'take': case_take, 'plug': case_plug}[case]()
    except (RuntimeError, camlib.Gone) as e:
        rep.case(case, False, note=str(e))
sys.exit(1 if rep.fails else 0)
