#!/usr/bin/env python3
"""The camera's serial console (docs/SHELL-ACCESS.md): one bridge owns the port; every other command talks
through it. Port and speed: MICROCAM_SERIAL, MICROCAM_SERIAL_BAUD (docs/TOOLS.md).
usage: uart.py bridge          own the port: the console into $MICROCAM_LOGS/uart-<date>.log, lines from the FIFO to it
       uart.py send TEXT [-w SECS] [--until RE] [--every SECS]
                               TEXT and a CR to the console ("" = just Enter); prints what came back within SECS
                               (default 2), or up to the first RE (exit 1 when it does not come); --every sends TEXT
                               again every SECS until RE comes (catching the U-Boot prompt from power-up)
       uart.py login           log in at a login prompt (MICROCAM_USER / MICROCAM_PASSWORD)
       uart.py uboot           the camera to the U-Boot prompt: when it stands there already, says so and leaves it;
                               else asks for a power-cycle and catches the prompt (Enter every 50 ms, up to 120 s:
                               this U-Boot waits no time for a key); exit 1 when no prompt comes
"""
import argparse, errno, fcntl, os, re, select, stat, sys, termios, time, tty

sys.path.insert(0, os.path.join(os.path.dirname(os.path.realpath(__file__)), '..'))
import hostconf  # noqa: E402

LOGS = hostconf.get('MICROCAM_LOGS')
FIFO = os.path.join(os.environ.get('XDG_RUNTIME_DIR') or '/tmp', 'microcam-uart')
ANSI = re.compile(rb'\x1b\[[0-9;]*m')
PROMPT = 'SigmaStar #'


def log_path():
    return os.path.join(LOGS, f'uart-{time.strftime("%F")}.log')


def bridge():
    os.makedirs(LOGS, exist_ok=True)
    lock = open(FIFO + '.lock', 'w')
    try:  # two bridges on one port make it flap
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
    except OSError:
        sys.exit('uart: a bridge already runs')
    dev, baud = hostconf.get('MICROCAM_SERIAL'), int(hostconf.get('MICROCAM_SERIAL_BAUD'))
    speed = getattr(termios, f'B{baud}')
    # a stale regular file in its place turns every send into a write to that file: make it a FIFO again
    if os.path.exists(FIFO) and not stat.S_ISFIFO(os.stat(FIFO).st_mode):
        os.remove(FIFO)
    if not os.path.exists(FIFO):
        os.mkfifo(FIFO)
    fifo = os.open(FIFO, os.O_RDWR | os.O_NONBLOCK)
    log, name = None, None

    def write(data):
        nonlocal log, name
        if name != log_path():  # a new file at midnight: senders read today's
            name = log_path()
            log = open(name, 'ab', buffering=0)
        log.write(data)

    def note(text):
        write(f'\n\033[1;33m[{time.strftime("%H:%M:%S")} {text}]\033[0m\n'.encode())

    print(f'uart: bridge on {dev} at {baud}, log {LOGS}/uart-<date>.log, send through {FIFO}', flush=True)
    while True:
        try:
            port = os.open(dev, os.O_RDWR | os.O_NOCTTY | os.O_NONBLOCK)
        except OSError:
            time.sleep(0.3)
            continue
        tty.setraw(port)
        attrs = termios.tcgetattr(port)
        attrs[4] = attrs[5] = speed
        termios.tcsetattr(port, termios.TCSANOW, attrs)
        note(f'port open, {baud}')
        try:
            while True:
                ready, _, _ = select.select([port, fifo], [], [])
                if port in ready:
                    data = os.read(port, 4096)
                    if not data:
                        raise OSError('eof')
                    write(data)
                if fifo in ready:
                    out = os.read(fifo, 4096)
                    if out:
                        shown = out.decode('latin-1').replace('\r', '\\r').replace('\n', '\\n')
                        write(f'\033[1;36m>>> {shown}\033[0m'.encode())
                        for b in out:  # paced: the camera's console drops characters on a burst
                            os.write(port, bytes([b]))
                            time.sleep(0.004)
        except OSError:
            note('port lost')
            os.close(port)


def put(text):
    try:
        fd = os.open(FIFO, os.O_WRONLY | os.O_NONBLOCK)
    except OSError as e:
        if e.errno in (errno.ENXIO, errno.ENOENT):
            sys.exit('uart: no bridge runs (uart.py bridge)')
        raise
    os.write(fd, text.encode() + b'\r')
    os.close(fd)


def since(pos):
    try:
        with open(log_path(), 'rb') as f:
            f.seek(pos)
            return ANSI.sub(b'', f.read().replace(b'\0', b'')).replace(b'\r', b'').decode(errors='replace')
    except OSError:
        return ''


def send(text, wait=2.0, until=None, every=None):
    """(whether RE came or no RE was asked, what came back)."""
    try:
        pos = os.path.getsize(log_path())
    except OSError:
        pos = 0
    rx = re.compile(until, re.M) if until else None
    end = time.time() + wait
    put(text)
    nxt = time.time() + every if every else None
    while True:
        if nxt and time.time() >= nxt:
            put(text)
            nxt = time.time() + every
        got = since(pos)
        if rx and rx.search(got):
            return True, got
        if time.time() >= end:
            return rx is None, got
        time.sleep(min(0.05, every or 0.05))


def uboot():
    """Whether the camera is at the U-Boot prompt: there already, or caught after the power-cycle this asks for. A
    command that writes the flash starts at once at a prompt that is already there, so the power-cycle is asked for
    only when it is not: one landing in the middle of a write leaves the region half written."""
    def say(msg):
        print(f'[{time.strftime("%H:%M:%S")}] {msg}', flush=True)
    if send('', 1.5, PROMPT)[0]:
        say('the camera is already at the U-Boot prompt: do not touch its power')
        return True
    say('power-cycle the camera now; sending Enter every 50 ms for up to 120 s')
    return send('', 120, PROMPT, 0.05)[0]


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest='cmd', required=True)
    sub.add_parser('bridge')
    p = sub.add_parser('send')
    p.add_argument('text')
    p.add_argument('-w', type=float, default=2.0)
    p.add_argument('--until')
    p.add_argument('--every', type=float)
    sub.add_parser('login')
    sub.add_parser('uboot')
    a = ap.parse_args()
    if a.cmd == 'bridge':
        bridge()
    elif a.cmd == 'uboot':
        sys.exit(0 if uboot() else 1)
    elif a.cmd == 'send':
        ok, got = send(a.text, a.w, a.until, a.every)
        sys.stdout.write(got)
        sys.exit(0 if ok else 1)
    else:
        user, password = hostconf.get('MICROCAM_USER'), hostconf.get('MICROCAM_PASSWORD')
        if not password:
            sys.exit('uart: MICROCAM_PASSWORD is set empty: unset it for the stock login (docs/TOOLS.md)')
        _, got = send('', 2)
        if 'login:' not in '\n'.join(got.splitlines()[-2:]):
            print('already logged in (or no prompt)')
            return
        send(user, 1.5)
        _, got = send(password, 2.5)
        print((got.splitlines() or [''])[-1])


if __name__ == '__main__':
    main()
