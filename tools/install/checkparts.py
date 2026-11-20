#!/usr/bin/env python3
"""Compare a camera's stock firmware with the tested one (stock-parts.tsv beside this file), part by part.

usage: checkparts.py [DUMP] [--for build|run|install|restore|all] [--camera] [--quiet]
  DUMP      a dump unpacked by unpack-dump.py (nor.bin, rootfs/, miservice/, customer/); default MICROCAM_SYSROOT
            (docs/TOOLS.md)
  --for     which parts may stop: those whose need includes it (default all); the rest only warn
  --camera  read the parts on the running camera (its shell, docs/TOOLS.md) instead of a dump
Prints one line per part: «same», «DIFFERS» or «missing», with the stock firmware's version strings for a differing
kernel or U-Boot. A part nothing of ours depends on (on_differ «info») never stops: its difference is printed as
information about another firmware build, and under --quiet only counted. A part that holds the camera's own data
(on_differ «data»: it differs on every camera) and a flash range the dump does not hold (a dump read over the network
ends with the last partition) are not compared. Exit 0 when every part that may stop is the same, 1 when one differs or
is missing, 2 on a usage error.
Our kernel modules take the vendor's symbols by name and microcamd links the vendor's libraries without symbol versions:
another build of those parts loads without an error and fails at run time, so a difference there stops the build and
the install.
"""
import argparse, hashlib, lzma, os, re, struct, subprocess, sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.dirname(HERE))
import hostconf  # noqa: E402
LIST = os.path.join(HERE, 'stock-parts.tsv')
# camera paths of the dump's top directories; flash regions as /dev/mtdN (index, offset of the partition in the flash)
CAMERA_ROOT = {'rootfs': '', 'miservice': '/config', 'customer': '/customer'}
MTD = [(0, 0x0, 0x70000), (1, 0x70000, 0x210000), (2, 0x280000, 0x200000), (3, 0x480000, 0x400000),
       (4, 0x880000, 0x610000), (5, 0xE90000, 0xA0000)]


def parts():
    rows = []
    for line in open(LIST):
        if line.startswith('#') or not line.strip():
            continue
        kind, part, size, sha, need, on, note = (line.rstrip('\n').split('\t') + [''])[:7]
        rows.append(dict(kind=kind, part=part, size=int(size), sha=sha, need=need.split(','), on=on, note=note))
    return rows


def region_span(part):
    name, off = part.split('@')
    return name, int(off, 16)


def uimage_name(blob):
    return blob[32:64].rstrip(b'\0').decode('latin-1') if blob[:4] == b'\x27\x05\x19\x56' else '?'


def kernel_image(nor):
    size = struct.unpack('>I', nor[0x70000 + 12:0x70000 + 16])[0]
    return lzma.decompress(nor[0x70040:0x70040 + size])


class Dump:
    def __init__(self, root, rows):
        self.root = root
        self.regions = {region_span(r['part'])[0]: region_span(r['part'])[1] for r in rows if r['kind'] == 'region'}
        p = os.path.join(root, 'nor.bin')
        self.nor = open(p, 'rb').read() if os.path.exists(p) else None

    def offset(self, r):
        return self.regions[r['part'].split(':')[0]] if r['kind'] == 'image' else region_span(r['part'])[1]

    def beyond(self, r):
        """The part is a flash range past this dump's end: a dump read through the kernel ends with the last partition."""
        return r['kind'] != 'file' and self.nor is not None and self.offset(r) + r['size'] > len(self.nor)

    def blob(self, r):
        if r['kind'] == 'file':
            p = os.path.join(self.root, r['part'])
            return open(p, 'rb').read() if os.path.isfile(p) else None
        if self.nor is None or self.beyond(r):
            return None
        if r['part'] == 'KERNEL:Image':
            return kernel_image(self.nor)
        off = self.offset(r)
        return self.nor[off:off + r['size']]

    def sha(self, r):
        b = self.blob(r)
        return None if b is None else hashlib.sha256(b).hexdigest()

    def versions(self, r):
        b = self.blob(r)
        if b is None:
            return ''
        if r['part'].startswith(('UBOOT', 'KERNEL@', 'KERNEL:uImage')):
            return uimage_name(b)
        if r['part'] == 'KERNEL:Image':
            m = re.search(rb'Linux version [^\n\0]+', b)
            return m[0].decode('latin-1').strip() if m else ''
        m = re.search(rb'project_commit\.\w+ sdk_commit\.\w+ build_time\.\d+', b)
        return m[0].decode() if m else ''


class Camera:
    """The running camera through its shell: sha256sum of files, of /dev/mtdN ranges for flash regions."""

    def __init__(self):
        sys.path.insert(0, os.path.join(HERE, '..', 'cam'))
        import camlib
        self.camlib = camlib
        self.cache = {}

    def cmd(self, r):
        if r['kind'] == 'file':
            if '/' not in r['part']:
                return None  # nor.bin: the regions cover it
            top, rest = r['part'].split('/', 1)
            return f'sha256sum {CAMERA_ROOT[top]}/{rest}'
        if r['kind'] == 'image':
            return None  # its region covers it
        _, off = region_span(r['part'])
        for n, start, size in MTD:
            if start <= off < start + size:
                if off == start and r['size'] == size:
                    return f'sha256sum /dev/mtd{n}'
                return f'dd if=/dev/mtd{n} bs=4096 skip={(off - start) // 4096} count={r["size"] // 4096} 2>/dev/null | sha256sum'
        return None

    def prefetch(self, rows):
        script = ''.join(f'echo "@{i} $({c} 2>/dev/null | cut -c1-64)"\n' for i, r in enumerate(rows) if (c := self.cmd(r)))
        rc, out = self.camlib.sh(None, 240, script=script)
        if rc != 0 and not out:
            sys.exit(f'checkparts: the camera did not answer (exit {rc})')
        for line in out.splitlines():
            m = re.match(r'^@(\d+) ([0-9a-f]{64})?$', line.strip())
            if m:
                self.cache[int(m[1])] = m[2]

    def sha(self, i):
        return self.cache.get(i)


def main():
    ap = argparse.ArgumentParser(description=__doc__.split('\n')[0])
    ap.add_argument('dump', nargs='?', default=hostconf.get('MICROCAM_SYSROOT'))
    ap.add_argument('--for', dest='purpose', default='all', choices=['build', 'run', 'install', 'restore', 'all'])
    ap.add_argument('--camera', action='store_true')
    ap.add_argument('--quiet', action='store_true', help='print only the parts that are not the same')
    a = ap.parse_args()
    rows = parts()
    if a.camera:
        src = Camera()
        rows = [r for r in rows if src.cmd(r) and r['need'] != ['build']]
        src.prefetch(rows)
        where = 'the camera'
    else:
        if not a.dump or not os.path.isdir(a.dump):
            print('checkparts: name the unpacked dump (or set MICROCAM_SYSROOT)', file=sys.stderr)
            return 2
        src = Dump(a.dump, rows)
        where = a.dump
    bad = warn = info = same = 0
    for i, r in enumerate(rows):
        if r['on'] == 'data' or (not a.camera and src.beyond(r)):
            if not a.quiet:
                print(f'{"-":8} {r["part"]}  not compared: {r["note"] if r["on"] == "data" else "this dump does not hold it"}')
            continue
        sha = src.sha(i) if a.camera else src.sha(r)
        stops = r['on'] == 'stop' and (a.purpose == 'all' or a.purpose in r['need'])
        if sha == r['sha']:
            same += 1
            if not a.quiet:
                print(f'same     {r["part"]}')
            continue
        state = 'missing' if sha is None else 'DIFFERS'
        if r['on'] == 'info':
            info += 1
            if not a.quiet:
                print(f'{state:8} {r["part"]}  [info] {r["note"] or "another firmware build"}')
            continue
        extra = '' if a.camera or sha is None else src.versions(r)
        mark = 'STOP' if stops else 'warn'
        print(f'{state:8} {r["part"]}  [{mark}: {",".join(r["need"])}] {r["note"]}' + (f' — yours: {extra}' if extra else ''))
        if stops:
            bad += 1
        else:
            warn += 1
    if bad:
        verdict = (f'NOT the tested firmware: {bad} part{"s" if bad > 1 else ""} OpenMicroCam needs '
                   f'differ{"" if bad > 1 else "s"} (STOP above); the build and the install refuse it')
    elif info:
        verdict = (f'OK — your firmware differs from the tested one only in {info} part{"s" if info > 1 else ""} '
                   'OpenMicroCam does not use; it works, this is for information')
    else:
        verdict = f'OK, the tested firmware — all {same + warn} parts identical'
    if not bad and warn:
        verdict += f'; {warn} warning{"s" if warn > 1 else ""} above'
    print(f'checkparts: {where}: {verdict}')
    return 1 if bad else 0


if __name__ == '__main__':
    sys.exit(main())
