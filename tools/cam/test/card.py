#!/usr/bin/env python3
"""The SD card as the USB host's disk beside the webcam in the card reader mode (the control `card-reader`), from the
laptop, a host's USB stream on (docs/BENCH.md).

  share  the mode on: the menu told `host`, the laptop sees the gadget's disk, reads the card's files (MB/s), writes,
         reads back and deletes a test file; USB video and microcamd's control go on meanwhile; with the card mounted
         on the laptop a recording and the mode's off are refused (`card-open`); the laptop unmounts and ejects it —
         the camera takes it back by itself (`camera`, mounted, the same files) and the mode is off
  take   the mode on, then off with the disk not mounted on the laptop: back on the camera, mounted
  photo  the mode on, the laptop unmounts it, the photo key pressed (the remote's, through the running menu): the menu
         takes the card back, the photo saved, the mode off
  boot   by hand (not in `cam check`): the mode on, the camera rebooted — the mode kept, the card the laptop's again
  plug   by hand (not in `cam check`): a 1080p recording runs, the user pulls the card — `none` told, the recording
         stopped, /mnt let go; the user puts it back — `camera` told, mounted, the files there (waits 3 min for each)

usage: card.py [--json] case...        details: $MICROCAM_LOGS/card-<date>.log
Thresholds: the menu's `card` events within 3 s; the disk on the laptop within 15 s; the test file read back equal;
USB video at least 90 % of the stream's rate while the disk is read; microcamd answering; the refusals' code.
"""
import argparse, glob, hashlib, json, os, subprocess, sys, tempfile, time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(HERE, '..'))
import camlib  # noqa: E402

ap = argparse.ArgumentParser()
ap.add_argument('case', nargs='+', choices=['share', 'take', 'photo', 'boot', 'plug'])
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


def switch_mode(on):
    """The card reader mode switched (`set card-reader`, put back when the tester ends): microcamd's reply."""
    return camlib.change('card-reader', on)


def give_back(disk):
    """The card back on the camera however the case ended: the laptop lets it go, the mode goes off; ejected when the
    camera is still refused (the eject gives it back by itself and the mode goes off with it)."""
    if (camlib.status(5).get('card') or {}).get('at') != 'host':
        return
    if disk:
        let_go(disk)
    if not switch_mode(False).get('ok') and disk:
        run('sudo', 'eject', disk)
        end = time.time() + 5
        while time.time() < end and (camlib.status(5).get('card') or {}).get('at') == 'host':
            time.sleep(0.3)


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


def mode_on():
    snap = camlib.microcam({'cmd': 'watch'}, 5).get('snapshot') or {}
    return next((x['value'] for x in snap.get('controls', []) if x['name'] == 'card-reader'), None)


def share(c):
    t = time.monotonic()
    r = switch_mode(True)
    if not r.get('ok'):
        raise RuntimeError(f"card-reader on: {r.get('error')}")
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
        rep.case('share: recording refused', not r.get('ok') and r.get('code') == 'card-open', error=r.get('error'))
        r = switch_mode(False)
        rep.case('share: mode off while mounted refused', not r.get('ok') and r.get('code') == 'card-open', error=r.get('error'))
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
        rep.case('share: back after the eject, the mode off', told is not None and mounted and n == files and mode_on() is False, s=told, mounted=mounted, files=n)
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
        r = switch_mode(False)
        told = event(c, t, 'camera')
        mounted, n = camera_files()
        rep.case('take: back on the camera', bool(r.get('ok')) and told is not None and mounted, s=told, files=n, error=r.get('error'))
    finally:
        give_back(disk)
        c.close()


def mounted_on_laptop(disk):
    """The disk's partition once the desktop has mounted it (it does by itself), mounted here when it has not."""
    part = partition(disk)
    end = time.time() + 6
    while not mountpoint(part) and time.time() < end:
        time.sleep(0.2)
    if not mountpoint(part):
        run('udisksctl', 'mount', '-b', part)
    return part


def case_photo():
    disk = None
    c = camlib.Menu()
    c.ask({'cmd': 'watch'})   # the menu itself takes the card for the photo: it stays the menu's
    try:
        told, disk = share(c)
        rep.case('photo: host told', told is not None and disk is not None, s=told, dev=disk)
        if not disk:
            return
        mounted_on_laptop(disk)
        let_go(disk)
        t = time.monotonic()
        r = microcam({'cmd': 'key', 'key': 'photo', 'phase': 'press'})
        microcam({'cmd': 'key', 'key': 'photo', 'phase': 'release'})
        back = event(c, t, 'camera')
        saved = c.event(lambda m: m.get('event') in ('photo-saved', 'failed'), t, 10)
        rep.case('photo: card taken back, photo saved, the mode off', bool(r.get('ok')) and back is not None and (saved or {}).get('event') == 'photo-saved' and mode_on() is False,
                 s=back, error=r.get('error'), saved=(saved or {}).get('file'))
    finally:
        give_back(disk)
        c.close()


def case_boot():
    disk = None
    c = camlib.Menu()
    c.ask({'cmd': 'hello', 'take': True})
    try:
        told, disk = share(c)
        rep.case('boot: host told', told is not None and disk is not None, s=told, dev=disk)
        if not disk:
            return
        let_go(disk)
    finally:
        c.close()
    # the settings file is written once they have stayed for a moment
    time.sleep(3)
    camlib.sh('setsid sh -c "sleep 1; reboot" </dev/null >/dev/null 2>&1 & true', 10, log=False)
    time.sleep(5)
    end = time.time() + 120
    while time.time() < end and not camlib.ping():
        time.sleep(1)
    subprocess.run([os.path.join(HERE, '..', 'cam'), 'wait', 'microcamd', '60'], capture_output=True)
    disk = gadget_disk(30)
    at = (camlib.status(5).get('card') or {}).get('at')
    rep.case('boot: the mode kept, the card the laptop\'s', mode_on() is True and at == 'host' and disk is not None, at=at, dev=disk)
    give_back(disk)


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
        {'share': case_share, 'take': case_take, 'photo': case_photo, 'boot': case_boot, 'plug': case_plug}[case]()
    except (RuntimeError, camlib.Gone) as e:
        rep.case(case, False, note=str(e))
sys.exit(1 if rep.fails else 0)
