#!/usr/bin/env python3
"""Install OpenMicroCam on a YIZHAN 4K camera, or take it back to the stock firmware (docs/INSTALL.md).

usage: install.py dump [OUTDIR] [--card | --serial [--reader DEV]]
       install.py install [--dump DIR] [--microcamd FILE] [--menu FILE] [--no-reboot | --card]
       install.py uninstall [--dump DIR] [-n | --card]
       install.py access
  dump       read this camera's flash into OUTDIR/nor.bin (default: MICROCAM_SYSROOT, work/dump), unpack it
             (unpack-dump.py) and compare it with the tested camera's (checkparts.py). It stops when OUTDIR already
             holds a nor.bin: firmware saved earlier is never overwritten. By default over the USB network
             through the camera's kernel: its six partitions, which end 832 KB before the flash does (that tail is unused
             and the kernel does not show it). --card: the whole flash by U-Boot into a file on a microSD card: the
             first run writes the card's files into work/card/dump, the camera fills FLASH.BIN there at power-on, the
             second run takes that FLASH.BIN back. --serial: the whole flash through U-Boot onto the SD card's raw
             sectors (dump.sh): first without --reader, then with the card in the laptop's reader (--reader /dev/sdX)
  install    check the dump, put the built files together with the camera's own stock files they depend on, upload them
             over the camera's USB network, run camera/install.sh there, reboot, wait for microcamd; the login stays.
             --card: the same files and installer into work/card/install for a microSD card instead (below)
  uninstall  the stock firmware's files and bootargs back from the dump (-n: only say what would change); after the
             next boot the camera is stock and the login is the vendor's again. --card: into work/card/stock
  access     the fallback for a camera whose shell takes neither the stock login nor the one of the settings: over the
             serial console (tools/cam/uart.py bridge running) boot once into a shell, set root's password to
             MICROCAM_PASSWORD (the stock one unless set) and put the boot hook in; then `install`
Every step but access, --card and `dump --serial` goes over the camera's USB network with the camera's stock login, or
the one MICROCAM_USER / MICROCAM_PASSWORD set (docs/TOOLS.md); when the camera does not answer there, the step says what
this computer sees of it on USB; when it refuses the login, what the card or the serial console can do instead.
--card writes files for the root of a microSD card (FAT32) into a folder of work/card, emptied first; the user copies
them onto the card. At every power-on the camera's U-Boot runs the card's H161X.bin as a script; ours first overwrites it
in place with DONE.BIN (U-Boot writes only over files of the same size: a file it makes anew is broken), so a card left
in does nothing at the next boot. The dump's script reads the flash into FLASH.BIN; the install's and uninstall's boot
Linux once into OPENMICROCAM/run.sh (camera/card-run.sh), which runs camera/install.sh on the bundle beside it, logs to
OPENMICROCAM/install.log and reboots.
--dump: the unpacked dump of this camera (unpack-dump.py), default $MICROCAM_SYSROOT. The built files come from
--microcamd and --menu (default target/armv7-unknown-linux-gnueabihf/release/microcamd and microcam-menu; an .xz is taken as
it is) and $MICROCAM_STAGE
(modules, fbhide); settings: docs/TOOLS.md.
"""
import argparse, base64, glob, hashlib, os, re, shutil, socket, subprocess, sys, tempfile, threading, time

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(HERE))
sys.path.insert(0, os.path.join(REPO, 'tools', 'cam'))
sys.path.insert(0, os.path.join(REPO, 'tools'))
import hostconf  # noqa: E402
import camlib  # noqa: E402
sys.path.insert(0, HERE)
from checkparts import MTD  # noqa: E402

PORT = 8004
TARGET = 'armv7-unknown-linux-gnueabihf'
RELEASE = os.path.join(REPO, 'target', TARGET, 'release')
CAMERA_ROOT = {'rootfs': '', 'miservice': '/config', 'customer': '/customer'}
REPO_FILES = ['config.toml', 'policy.toml', 'app-start.sh', 'udc-insmod.sh', 'boot-hook.sh', 'install.sh']
STAGE_FILES = ['fbhide', 'microcam_sys.ko', 'microcam_canvas.ko', 'microcam_rgn.ko', 'udc-msb250x.ko', 'g_webcam.ko',
               'drv_ms_cus_gc8613_MIPI.ko']
NEEDED = ['microcamd.xz', 'microcam-menu.xz', 'fbhide', 'microcam_sys.ko', 'microcam_canvas.ko']
# the owner's menu settings stay on uninstall: the vendor's application rewrites them at every change
KEEP_ON_UNINSTALL = {'ws.bin'}


def say(msg):
    print(f'[{time.strftime("%H:%M:%S")}] {msg}', flush=True)


def sha(path):
    return hashlib.sha256(open(path, 'rb').read()).hexdigest()


def stock_parts():
    rows = []
    for line in open(os.path.join(HERE, 'stock-parts.tsv')):
        if line.startswith('#') or not line.strip():
            continue
        kind, part, size, digest, need, on = line.rstrip('\n').split('\t')[:6]
        rows.append((kind, part, digest, need.split(','), on))
    return rows


def camera_path(part):
    top, rest = part.split('/', 1)
    return f'{CAMERA_ROOT[top]}/{rest}'


def checkparts(dump, purposes):
    for p in purposes:
        r = subprocess.run([sys.executable, os.path.join(HERE, 'checkparts.py'), dump, '--for', p, '--quiet'],
                           capture_output=True, text=True)
        stops = [l for l in r.stdout.splitlines() if '[STOP' in l]
        if r.returncode != 0:
            print(r.stdout.strip())
            sys.exit(f'install: the dump differs from the tested camera where {p} depends on it (above)')
        for l in r.stdout.splitlines():
            if l.startswith(('DIFFERS', 'missing')) and l not in stops:
                print(l)



def bundle(dump, microcamd, menu):
    """The files for the camera, in a new directory."""
    for path in (microcamd, menu):
        if not os.path.exists(path):
            sys.exit(f'install: no {path}: build it first (docs/INSTALL.md «Build OpenMicroCam»)')
    b = tempfile.mkdtemp(prefix='microcam-bundle-')
    camlib.pack_xz(microcamd, f'{b}/microcamd.xz')
    camlib.pack_xz(menu, f'{b}/microcam-menu.xz')
    for name in REPO_FILES:
        shutil.copy(os.path.join(REPO, 'camera', name), b)
    stage = hostconf.get('MICROCAM_STAGE')
    for name in STAGE_FILES:
        if os.path.exists(os.path.join(stage, name)):
            shutil.copy(os.path.join(stage, name), b)
    missing = [n for n in NEEDED if not os.path.exists(os.path.join(b, n))]
    if missing:
        sys.exit(f'install: not built: {", ".join(missing)} (docs/INSTALL.md «Build OpenMicroCam»; modules and fbhide land in MICROCAM_STAGE)')
    # the ISP tuning microcamd is tested with: the camera's own file with the defect-pixel blending off
    subprocess.run([sys.executable, os.path.join(HERE, 'iq-dpc-noblend.py'), os.path.join(dump, 'customer/iq_file_gc8613.bin'),
                    os.path.join(b, 'iq_file_gc8613.bin')], check=True, stdout=subprocess.DEVNULL)
    with open(os.path.join(b, 'stock.sha256'), 'w') as f:
        for kind, part, digest, need, on in stock_parts():
            if kind == 'file' and on == 'stop' and ({'run', 'install'} & set(need)):
                f.write(f'{digest}  {camera_path(part)}\n')
    return b


def dump_env(dump):
    """U-Boot's environment as the dump has it: env.txt from unpack-dump.py, else straight from nor.bin."""
    p = os.path.join(dump, 'env.txt')
    if os.path.exists(p):
        lines = open(p).read().splitlines()
    else:
        blob = open(os.path.join(dump, 'nor.bin'), 'rb').read()[0x6F000 + 4:0x70000]
        lines = [s.decode('latin-1') for s in blob.split(b'\0\0')[0].split(b'\0') if s]
    return dict(l.partition('=')[::2] for l in lines)


CARD = os.path.join(REPO, 'work', 'card')
CARD_TRIGGER = 'H161X.bin'  # the file U-Boot's sdstar runs: env SdUpgradeImage, the stock value (boot-hook.sh keeps it)
CARD_DONE = 'DONE.BIN'
CARD_DIR = 'OPENMICROCAM'
CARD_END = '% <- this is end of script symbol'
CARD_SCRIPT = 0x4000  # sdstar reads this much of the file as the script
CARD_SIZE = CARD_SCRIPT + 8 + 16  # and the SDK packer's tail: `12345678`, the file's first 16 bytes
CARD_LINE = 511  # U-Boot cuts a command line after this many bytes, silently
CARD_ADDR = '0x21000000'
CARD_INERT = [f'fatload mmc 0 {CARD_ADDR} {CARD_DONE}', f'fatwrite mmc 0 {CARD_ADDR} {CARD_TRIGGER} {CARD_SIZE:#x}']
FLASH_SIZE = 16 << 20
FLASH_PLACEHOLDER = b'OpenMicroCam FLASH.BIN: not written by the camera yet\n'


def rel(path):
    return os.path.relpath(path)


def card_script(lines):
    """The bytes of a file sdstar runs: the U-Boot commands and the end line, padded with 0xFF, then the packer's tail."""
    for l in lines:
        if len(l.encode()) > CARD_LINE or '$' in l or ';' in l:
            sys.exit(f'install: a card script line would not reach U-Boot whole ({len(l.encode())} bytes of at most '
                     f'{CARD_LINE}, no $ or ;): {l}')
    text = ('\n'.join(lines + [CARD_END]) + '\n').encode()
    if len(text) > CARD_SCRIPT:
        sys.exit(f'install: the card script has {len(text)} bytes, U-Boot reads {CARD_SCRIPT}')
    body = text + b'\xff' * (CARD_SCRIPT - len(text))
    return body + b'12345678' + body[:16]


def card_boot(dump, job):
    """The U-Boot line that has the next autoboot start Linux once with OPENMICROCAM/run.sh JOB of the card as PID 1. The
    memory layout comes from the dump's own bootargs: without mmap_reserved U-Boot's bootlogo crashes, and the crash's
    reset saves these temporary bootargs to the flash."""
    have = dict(a.partition('=')[::2] for a in dump_env(dump)['bootargs'].split())
    keep = ['LX_MEM', 'cma', 'mmap_reserved']
    if not all(have.get(k) for k in keep):
        sys.exit(f'install: the dump\'s bootargs lack {", ".join(k for k in keep if not have.get(k))}')
    init = ' && '.join(['mount -t squashfs /dev/mtdblock3 /config', 'cd /config/modules/4.9.227']
                       + [f'insmod {m}.ko' for m in ('mmc_core', 'mmc_block', 'kdrv_sdmmc', 'fat', 'vfat')]
                       + ['sleep 2', 'mount -t vfat /dev/mmcblk0p1 /mnt', f'exec sh /mnt/{CARD_DIR}/run.sh {job}'])
    return (f'setenv bootargs console=ttyS0,115200 root=/dev/mtdblock2 rootfstype=squashfs ro '
            f'{" ".join(f"{k}={have[k]}" for k in keep)} init=/bin/sh -- -c "{init} || reboot -f"')


def card_folder(name, lines):
    """work/card/NAME emptied, with the trigger script of `lines` and DONE.BIN, the inert one of the same size."""
    trigger, done = card_script(lines), card_script([])
    assert len(trigger) == len(done) == CARD_SIZE
    d = os.path.join(CARD, name)
    shutil.rmtree(d, ignore_errors=True)
    os.makedirs(d)
    open(os.path.join(d, CARD_TRIGGER), 'wb').write(trigger)
    open(os.path.join(d, CARD_DONE), 'wb').write(done)
    return d


def card_job(name, job, b, dump):
    """The bundle `b` (moved) and the card's boot into run.sh JOB, into work/card/NAME."""
    d = card_folder(name, CARD_INERT + [card_boot(dump, job)])
    shutil.move(b, os.path.join(d, CARD_DIR))
    shutil.copy(os.path.join(REPO, 'camera', 'card-run.sh'), os.path.join(d, CARD_DIR, 'run.sh'))
    say(f'in {rel(d)}: {CARD_TRIGGER}, {CARD_DONE}, {CARD_DIR}/. With them in the card\'s root the camera runs the {job} '
        f'once at power-on, writes {CARD_DIR}/install.log on the card and reboots')
    print(f'copy {rel(d)} onto the card')
    return 0


def dump_card():
    """The first run: the card's files into work/card/dump, None. The second: the FLASH.BIN the camera filled there."""
    d = os.path.join(CARD, 'dump')
    flash = os.path.join(d, 'FLASH.BIN')
    if os.path.exists(flash):
        if open(flash, 'rb').read(len(FLASH_PLACEHOLDER)) == FLASH_PLACEHOLDER:
            sys.exit(f'dump: {rel(flash)} is the one this tool wrote: the camera did not write it. Copy the files of '
                     f'{rel(d)} into the card\'s root, power the camera on with the card in, let it boot, then '
                     f'copy the card\'s FLASH.BIN back into {rel(d)}')
        return flash
    d = card_folder('dump', ['sf probe 0', f'sf read {CARD_ADDR} 0x0 {FLASH_SIZE:#x}',
                             f'fatwrite mmc 0 {CARD_ADDR} FLASH.BIN {FLASH_SIZE:#x}'] + CARD_INERT)
    with open(flash, 'wb') as f:  # U-Boot writes the flash over it: a file of that size must be there
        f.write(FLASH_PLACEHOLDER + b'\xff' * (FLASH_SIZE - len(FLASH_PLACEHOLDER)))
    say(f'in {rel(d)}: {CARD_TRIGGER}, {CARD_DONE}, FLASH.BIN. With them in the card\'s root the camera writes its whole '
        f'flash into FLASH.BIN at power-on and boots as usual; copy that FLASH.BIN back into {rel(d)} and run this again')
    print(f'copy {rel(d)} onto the card')
    return None


def login_ok():
    try:
        return camlib.sh('true', 15, log=False)[0] == 0
    except Exception:
        return False


SERIAL_DOC = 'docs/INSTALL.md «Over the serial console (UART)»'


USB_CAMERA = ('0ac8', '3420')  # the camera on USB, under the stock firmware and under OpenMicroCam
USB_CARD = ('0525', 'a4a5')  # the stock firmware with a microSD card in: a storage gadget, no camera and no network


def usb_ids():
    """The (vendor, product) ids of the USB devices this computer sees."""
    ids = set()
    for vendor in glob.glob('/sys/bus/usb/devices/*/idVendor'):
        try:
            with open(vendor) as v, open(os.path.join(os.path.dirname(vendor), 'idProduct')) as p:
                ids.add((v.read().strip(), p.read().strip()))
        except OSError:
            pass
    return ids


def need_shell(refused):
    """The camera's shell over its USB network with the login in force; else what this computer sees of the camera
    (`refused`: what to do when the camera answers but takes no login)."""
    net = ''
    if not camlib.ping(2):  # this computer's address on the camera's USB network: the interface is new at every camera boot
        net = subprocess.run(['sh', os.path.join(REPO, 'tools', 'cam', 'cam-net.sh')], stdout=subprocess.PIPE, text=True).stdout.strip()
    if login_ok():
        return
    if camlib.ping(2):
        login = (hostconf.get('MICROCAM_USER'), hostconf.get('MICROCAM_PASSWORD'))
        if login == (hostconf.STOCK_USER, hostconf.STOCK_PASSWORD):
            sys.exit(f'install: the camera answers at {camlib.CAM}, but the stock login and password did not work. If you '
                     f'know your camera\'s login and password, set them in MICROCAM_USER and MICROCAM_PASSWORD '
                     f'(docs/TOOLS.md). {refused}')
        sys.exit(f'install: the camera answers at {camlib.CAM}, but the login and password set in MICROCAM_USER / '
                 f'MICROCAM_PASSWORD did not work (docs/TOOLS.md). {refused}')
    usb = usb_ids()
    if USB_CAMERA in usb:
        sys.exit(f'install: the camera is on USB, but {camlib.CAM} does not answer on its network' + (f' ({net}).' if net else '.'))
    if USB_CARD in usb:
        sys.exit(f'install: the camera is on USB as a drive ({":".join(USB_CARD)}): a microSD card is in it. Take the card '
                 f'out and power-cycle the camera.')
    sys.exit(f'install: no camera on USB (no device {":".join(USB_CAMERA)}): check the cable and that the camera is powered on.')


def vendor_udc():
    """Whether the camera runs the vendor's USB controller driver (full-speed receive hangs it: MTU 400 then)."""
    return camlib.sh('[ -d /sys/module/udc_msb250x/parameters ]', 10, log=False)[0] == 0


def set_mtu(mtu):
    camlib.sh(f'ifconfig usb0 mtu {mtu}', 10, log=False)
    iface = camlib.link()
    if iface:
        subprocess.run(['sudo', 'ip', 'link', 'set', iface, 'mtu', str(mtu)], check=False)


def upload(b, where):
    """Every file of the bundle into `where` on the camera (RAM), checked by md5."""
    names = sorted(os.path.relpath(os.path.join(d, f), b) for d, _, fs in os.walk(b) for f in fs)
    want = {n: hashlib.md5(open(os.path.join(b, n), 'rb').read()).hexdigest() for n in names}
    server = subprocess.Popen([sys.executable, '-m', 'http.server', str(PORT), '--bind', camlib.HOST, '-d', b],
                              stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    try:
        time.sleep(0.5)
        script = (f'rm -rf {where}; mkdir -p {where} && cd {where} || exit 3\n'
                  f'for f in {" ".join(names)}; do mkdir -p $(dirname $f); '
                  f'wget -q -O $f http://{camlib.HOST}:{PORT}/$f || exit 3; done\nmd5sum {" ".join(names)}')
        rc, out = camlib.sh(None, 300, script=script)
    finally:
        server.terminate()
    got = {l.split()[1]: l.split()[0] for l in out.splitlines() if re.match(r'^[0-9a-f]{32}  ', l)}
    if rc != 0 or got != want:
        sys.exit(f'install: the upload failed (exit {rc}): {out.strip()[-300:]}')
    say(f'{len(names)} files on the camera in {where}, md5 checked')


def run_on_camera(cmd, timeout=180):
    rc, out = camlib.sh(cmd, timeout)
    for l in out.splitlines():
        if l.startswith(('ok ', 'FAIL ', 'would: ')) or l.startswith(' ') or rc != 0:
            print('  ' + l)
    return rc


def reboot_and_wait(released, want_microcamd=True):
    say('rebooting')
    camlib.sh('sync; (sleep 1; reboot) >/dev/null 2>&1 & true', 10, log=False)
    t = time.time()
    while time.time() - t < 30 and camlib.ping(1):
        time.sleep(0.5)
    cam = os.path.join(REPO, 'tools', 'cam', 'cam')
    ok = True
    for what, secs in (('up', 120), ('microcamd', 60)) if want_microcamd else (('up', 120),):
        r = subprocess.run([cam, 'wait', what, str(secs)])
        if r.returncode != 0:
            ok = False
            break
    camlib.stream_restore(released)
    return ok


NO_LOGIN = f'Over the serial console `install.py access` gives it the login of the settings, then run this again ({SERIAL_DOC}).'


def dump_network(path):
    """The flash through the camera's kernel into `path`: its partitions /dev/mtd0… in order, which lie end to end from
    the flash's start, streamed by nc to the laptop and checked against the camera's own sha256 of a second read."""
    rc, out = camlib.sh('cat /proc/mtd', 10, log=False)
    sizes = [int(m[1], 16) for m in re.finditer(r'^mtd\d+: ([0-9a-f]+) ', out, re.M)]
    if sizes != [size for _, _, size in MTD]:
        sys.exit(f'dump: the camera\'s partitions are not the tested camera\'s (/proc/mtd above); the card\'s dump reads '
                 f'the flash whole: `install.py dump --card`\n{out.strip()}')
    devs = ' '.join(f'/dev/mtd{n}' for n, _, _ in MTD)
    if vendor_udc():
        say('the vendor USB driver runs: MTU 400 on both ends for the transfer')
        set_mtu(400)
    srv = socket.create_server((camlib.HOST, PORT))
    srv.settimeout(30)
    got, err = bytearray(), []

    def receive():
        try:
            conn, _ = srv.accept()
            with conn:
                conn.settimeout(60)
                while chunk := conn.recv(1 << 16):
                    got.extend(chunk)
        except OSError as e:
            err.append(e)

    released = camlib.stream_release()
    t = threading.Thread(target=receive)
    t.start()
    say(f'reading {devs} ({sum(sizes) >> 10} KB) over the network')
    try:
        rc, out = camlib.sh(f'cat {devs} | nc {camlib.HOST} {PORT}', 600, log=False)
        t.join()
    finally:
        srv.close()
        camlib.stream_restore(released)
    if err or rc != 0 or len(got) != sum(sizes):
        sys.exit(f'dump: the transfer broke off ({len(got)} of {sum(sizes)} bytes, exit {rc}{", " + str(err[0]) if err else ""}): '
                 f'run it again')
    rc, out = camlib.sh(f'cat {devs} | sha256sum', 600, log=False)
    mine = hashlib.sha256(got).hexdigest()
    if rc != 0 or out.split()[:1] != [mine]:
        sys.exit(f'dump: the laptop got sha256 {mine}, the camera reads {out.strip()[:64]}: run it again')
    open(path, 'wb').write(got)
    say(f'{path}: {len(got)} bytes, sha256 {mine}, the same as the camera\'s own read')


def cmd_dump(a):
    nor = os.path.join(a.outdir, 'nor.bin')
    if os.path.exists(nor):
        sys.exit(f'dump: {rel(a.outdir)} already holds a dump ({rel(nor)}). A dump is taken only into a folder with no '
                 f'earlier dump in it: a precaution, so that firmware saved earlier is not overwritten.')
    if a.card:
        nor = dump_card()
        if not nor:
            return 0  # the card's files are written; the camera fills FLASH.BIN
    os.makedirs(a.outdir, exist_ok=True)
    if a.serial:
        r = subprocess.run([os.path.join(HERE, 'dump.sh'), a.outdir] + ([a.reader] if a.reader else []))
        if r.returncode != 0:
            sys.exit(r.returncode)
        if not a.reader:
            return 0  # the flash is on the card; dump.sh said what comes next
    elif not a.card:
        need_shell('Without a login, take the dump through a microSD card: `install.py dump --card`.')
        dump_network(nor)
    if subprocess.run([sys.executable, os.path.join(HERE, 'unpack-dump.py'), nor, a.outdir]).returncode != 0:
        saved = os.path.join(a.outdir, 'nor.bin')
        if os.path.exists(saved):
            sys.exit(f'install: {rel(saved)} is saved, but unpacking it failed (the reason is above); when that is fixed, '
                     f'unpack it: tools/install/unpack-dump.py {rel(saved)} {rel(a.outdir)}')
        sys.exit('install: unpacking the dump failed (the reason is above); run this again')
    return subprocess.run([sys.executable, os.path.join(HERE, 'checkparts.py'), a.outdir, '--quiet']).returncode


def cmd_install(a):
    dump = a.dump
    say(f'checking the dump {dump}')
    checkparts(dump, ['build', 'run', 'install'])
    b = bundle(dump, a.microcamd, a.menu)
    say(f'bundle {b}: {", ".join(sorted(os.listdir(b)))}')
    if a.card:
        return card_job('install', 'install', b, dump)
    need_shell(NO_LOGIN)
    released = camlib.stream_release()
    try:
        if vendor_udc():
            say('the vendor USB driver runs: MTU 400 on both ends for the upload')
            set_mtu(400)
        upload(b, '/tmp/microcam-install')
        say('installing on the camera')
        rc = run_on_camera('sh /tmp/microcam-install/install.sh install /tmp/microcam-install')
    except BaseException:
        camlib.stream_restore(released)
        raise
    shutil.rmtree(b)
    if rc != 0:
        camlib.stream_restore(released)
        sys.exit('install: stopped at the step above; nothing after it ran (each step can be run again)')
    if a.no_reboot:
        camlib.stream_restore(released)
        say('installed; microcamd starts at the next boot')
        return 0
    if not reboot_and_wait(released):
        sys.exit('install: microcamd did not come up: `tools/cam/cam state`, `cam log`')
    say('microcamd runs')
    return 0


def cmd_uninstall(a):
    dump = a.dump
    if not a.card:
        need_shell(NO_LOGIN)
    b = tempfile.mkdtemp(prefix='microcam-stock-')
    shutil.copy(os.path.join(REPO, 'camera', 'install.sh'), b)
    with open(os.path.join(b, 'stock.sha256'), 'w') as f:
        for kind, part, digest, need, on in stock_parts():
            if kind == 'file' and part == 'customer/demo.sh':
                f.write(f'{digest}  /customer/demo.sh\n')
    open(os.path.join(b, 'bootargs.stock'), 'w').write(dump_env(dump)['bootargs'])
    croot = os.path.join(dump, 'customer')
    files = sorted(os.path.relpath(os.path.join(d, f), croot) for d, _, fs in os.walk(croot) for f in fs)
    open(os.path.join(b, 'stock.list'), 'w').write(''.join(f + '\n' for f in files))
    if a.card:  # the camera is not asked: every file goes, and install.sh skips those already the same
        restore = [f for f in files if f not in KEEP_ON_UNINSTALL]
    else:  # which of the dump's /customer files the camera lacks or holds changed
        rc, out = camlib.sh(None, 60, script=''.join(f'echo "@{f} $(sha256sum \'/customer/{f}\' 2>/dev/null | cut -c1-64)"\n' for f in files))
        there = dict(re.match(r'^@(\S+) ?([0-9a-f]{64})?$', l.strip()).groups() for l in out.splitlines() if l.startswith('@'))
        restore = [f for f in files if f not in KEEP_ON_UNINSTALL and there.get(f) != sha(os.path.join(croot, f))]
    for f in restore + ['demo.sh']:
        os.makedirs(os.path.dirname(os.path.join(b, 'stock', f)), exist_ok=True)
        shutil.copy(os.path.join(croot, f), os.path.join(b, 'stock', f))
    if a.card:
        return card_job('stock', 'uninstall', b, dump)
    say(f'stock files to put back: {", ".join(restore) or "none"}')
    # the flash regions our install never writes: a difference there needs restore-region.sh over the serial console
    r = subprocess.run([sys.executable, os.path.join(HERE, 'checkparts.py'), '--camera', '--quiet'], capture_output=True, text=True)
    regions = [l for l in r.stdout.splitlines() if l.startswith(('DIFFERS', 'missing')) and '@0x' in l]
    say('flash regions: ' + ('; '.join(regions) + ' — restore them with restore-region.sh' if regions else 'all as in the dump'))
    released = camlib.stream_release()
    try:
        upload(b, '/tmp/microcam-stock')
        rc = run_on_camera('sh /tmp/microcam-stock/install.sh uninstall /tmp/microcam-stock' + (' -n' if a.n else ''), 300)
    finally:
        shutil.rmtree(b)
    if rc != 0:
        camlib.stream_restore(released)
        sys.exit('uninstall: stopped at the step above')
    if a.n:
        camlib.stream_restore(released)
        return 0
    say('rebooting into the stock firmware; the login is the vendor\'s from now on')
    camlib.sh('sync; (sleep 1; reboot) >/dev/null 2>&1 & true', 10, log=False)
    camlib.stream_restore(released)
    return 0


def uart(text, wait=2.0, until=None, every=None):
    cmd = [sys.executable, os.path.join(REPO, 'tools', 'cam', 'uart.py'), 'send', text, '-w', str(wait)]
    if until:
        cmd += ['--until', until]
    if every:
        cmd += ['--every', str(every)]
    r = subprocess.run(cmd, capture_output=True, text=True)
    return r.returncode, r.stdout


def uart_file(local, remote):
    """A file to the camera's shell over the serial console: base64 lines appended, then decoded."""
    data = base64.b64encode(open(local, 'rb').read()).decode()
    uart(f'rm -f {remote}.b64')
    for i in range(0, len(data), 64):
        rc, _ = uart(f'echo {data[i:i + 64]} >> {remote}.b64', 3, r'^/ # |# $')
        if rc != 0:
            sys.exit(f'access: the serial console stopped answering at {remote}')
    want = hashlib.md5(open(local, 'rb').read()).hexdigest()
    rc, out = uart(f'base64 -d {remote}.b64 > {remote} && rm {remote}.b64 && md5sum {remote}', 5, want)
    if rc != 0:
        sys.exit(f'access: {remote} did not arrive whole')


def cmd_access(a):
    pw = hostconf.get('MICROCAM_PASSWORD')
    if not pw:
        sys.exit('access: MICROCAM_PASSWORD is set empty: unset it for the stock password, or set your own')
    line = subprocess.run(['openssl', 'passwd', '-6', '-stdin'], input=pw, capture_output=True, text=True, check=True).stdout.strip()
    b = tempfile.mkdtemp(prefix='microcam-access-')
    open(os.path.join(b, 'shadow.line'), 'w').write(f'root:{line}:19000:0:99999:7:::\n')
    with open(os.path.join(b, 'stock.sha256'), 'w') as f:
        for kind, part, digest, need, on in stock_parts():
            if part == 'customer/demo.sh':
                f.write(f'{digest}  /customer/demo.sh\n')
    say('power-cycle the camera now; catching the U-Boot prompt for up to 120 s')
    if uart('', 120, 'SigmaStar #', 0.05)[0] != 0:
        sys.exit('access: no U-Boot prompt (is `uart.py bridge` running, the adapter on the console pads?)')
    # one boot into a shell, from RAM: U-Boot's environment is not saved
    uart('setenv bootargs ${bootargs} init=/bin/sh', 2)
    if uart('run bootcmd', 60, r'(^/ # |# $)')[0] != 0:
        sys.exit('access: no shell after the boot')
    time.sleep(3)
    say('in the shell; mounting the flash partitions')
    uart('mount -t proc proc /proc; mount -t sysfs sys /sys; mount -t squashfs /dev/mtdblock3 /config; '
         'mount -t jffs2 mtd:customer /customer; mkdir -p /tmp/a /customer/microcam', 5)
    for name in ('install.sh', 'boot-hook.sh'):
        uart_file(os.path.join(REPO, 'camera', name), f'/tmp/a/{name}' if name == 'install.sh' else f'/customer/microcam/{name}')
    for name in ('shadow.line', 'stock.sha256'):
        uart_file(os.path.join(b, name), f'/tmp/a/{name}')
    shutil.rmtree(b)
    rc, out = uart('sh /tmp/a/install.sh access /tmp/a && sh /tmp/a/install.sh demo /tmp/a && sync && echo ACCESS_DONE', 30,
                   '^ACCESS_DONE|^FAIL')
    print('\n'.join(l for l in out.splitlines() if l.startswith(('ok ', 'FAIL '))))
    if rc != 0 or 'ACCESS_DONE' not in out:
        sys.exit('access: the camera-side step failed (above); the camera boots as before')
    uart('reboot -f', 2)
    say('done; the camera boots its own firmware with the login of the settings. Next: install.py install')
    return 0


def main():
    ap = argparse.ArgumentParser(description=__doc__.split('\n')[0])
    sub = ap.add_subparsers(dest='cmd', required=True)
    p = sub.add_parser('dump')
    p.add_argument('outdir', nargs='?', default=hostconf.get('MICROCAM_SYSROOT'))
    p.add_argument('--card', action='store_true', help='through U-Boot into FLASH.BIN on a microSD card (work/card/dump)')
    p.add_argument('--serial', action='store_true', help='through U-Boot onto the SD card, over the serial console')
    p.add_argument('--reader', help='with --serial: the card\'s block device in the laptop\'s reader, for the second part')
    sub.add_parser('access')
    for name in ('install', 'uninstall'):
        p = sub.add_parser(name)
        p.add_argument('--dump', default=hostconf.get('MICROCAM_SYSROOT'))
        p.add_argument('--card', action='store_true', help=f'the files for a microSD card into work/card/{"install" if name == "install" else "stock"}')
        if name == 'install':
            p.add_argument('--microcamd', default=os.path.join(RELEASE, 'microcamd'))
            p.add_argument('--menu', default=os.path.join(RELEASE, 'microcam-menu'))
            p.add_argument('--no-reboot', action='store_true')
        if name == 'uninstall':
            p.add_argument('-n', action='store_true', help='only say what would change')
    a = ap.parse_args()
    if a.cmd == 'dump' and a.reader and not a.serial:
        sys.exit('install: --reader belongs to the serial dump (--serial)')
    if a.cmd == 'dump' and a.card and a.serial:
        sys.exit('install: --card and --serial are two ways of the dump: one of them')
    if getattr(a, 'card', False) and (getattr(a, 'no_reboot', False) or getattr(a, 'n', False)):
        sys.exit('install: the card always runs the whole step and reboots: no -n or --no-reboot with --card')
    if a.cmd in ('install', 'uninstall') and (not a.dump or not os.path.isdir(a.dump)):
        sys.exit(f'install: no unpacked dump in {a.dump}: `install.py dump` first, or name one (--dump or MICROCAM_SYSROOT)')
    return {'dump': cmd_dump, 'install': cmd_install, 'uninstall': cmd_uninstall, 'access': cmd_access}[a.cmd](a)


if __name__ == '__main__':
    sys.exit(main())
