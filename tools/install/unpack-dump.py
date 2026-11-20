#!/usr/bin/env python3
"""Unpack a NOR dump of the camera into the tree the build and checkparts.py read.

usage: unpack-dump.py NOR.BIN [OUTDIR]
NOR.BIN: the whole 16 MB flash, or the flash up to the end of its last partition (a dump read through the camera's
kernel, `install.py dump`). OUTDIR gets: nor.bin (a copy), kernel-Image (the kernel uncompressed from its uImage), rootfs/ and miservice/ (squashfs),
customer/ (jffs2), env.txt (U-Boot's environment, one NAME=value a line). OUTDIR defaults to MICROCAM_SYSROOT
(docs/TOOLS.md), where the build and the install look for the dump.
Needs unsquashfs (squashfs-tools) and jefferson (pip install jefferson). The regions are those of the tested camera's
partition table; checkparts.py stops when the table (MXPT) of a dump differs.
"""
import binascii, lzma, os, shutil, struct, subprocess, sys, tempfile

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from checkparts import MTD, hostconf  # noqa: E402

FLASH = 16 << 20
PARTITIONED = MTD[-1][1] + MTD[-1][2]  # the end of the last partition
REGIONS = {'UBOOT_ENV': (0x6F000, 0x1000), 'KERNEL': (0x70000, 0x210000), 'rootfs': (0x280000, 0x200000),
           'miservice': (0x480000, 0x400000), 'customer': (0x880000, 0x610000)}


def env(blob):
    """U-Boot's environment block: CRC32 (little endian) of the rest, then NAME=value strings up to an empty one."""
    crc = struct.unpack('<I', blob[:4])[0]
    if binascii.crc32(blob[4:]) != crc:
        raise SystemExit('unpack: the U-Boot environment fails its CRC')
    return [s.decode('latin-1') for s in blob[4:].split(b'\0\0')[0].split(b'\0') if s]


def main():
    if len(sys.argv) not in (2, 3):
        sys.exit(__doc__.strip().split('\n')[2])
    src = sys.argv[1]
    out = sys.argv[2] if len(sys.argv) == 3 else hostconf.get('MICROCAM_SYSROOT')
    nor = open(src, 'rb').read()
    if len(nor) not in (FLASH, PARTITIONED):
        sys.exit(f'unpack: {src} has {len(nor)} bytes, a dump of this flash has {FLASH} (or {PARTITIONED}, read through '
                 f'the kernel)')
    # pipx puts jefferson into ~/.local/bin, which a shell opened before that install does not search
    os.environ['PATH'] += os.pathsep + os.path.expanduser('~/.local/bin')
    for tool, pkg in (('unsquashfs', 'sudo apt install squashfs-tools'), ('jefferson', 'pipx install jefferson')):
        if not shutil.which(tool):
            sys.exit(f'unpack: {tool} is not installed ({pkg})')
    os.makedirs(out, exist_ok=True)
    if os.path.abspath(src) != os.path.abspath(os.path.join(out, 'nor.bin')):
        shutil.copyfile(src, os.path.join(out, 'nor.bin'))
    off, _ = REGIONS['KERNEL']
    size = struct.unpack('>I', nor[off + 12:off + 16])[0]
    open(os.path.join(out, 'kernel-Image'), 'wb').write(lzma.decompress(nor[off + 64:off + 64 + size]))
    off, size = REGIONS['UBOOT_ENV']
    open(os.path.join(out, 'env.txt'), 'w').write(''.join(l + '\n' for l in env(nor[off:off + size])))
    with tempfile.TemporaryDirectory() as tmp:
        for name, tool in (('rootfs', 'squashfs'), ('miservice', 'squashfs'), ('customer', 'jffs2')):
            off, size = REGIONS[name]
            img = os.path.join(tmp, name)
            open(img, 'wb').write(nor[off:off + size])
            dest = os.path.join(out, name)
            if os.path.exists(dest):
                shutil.rmtree(dest)
            cmd = (['unsquashfs', '-no-xattrs', '-quiet', '-d', dest, img] if tool == 'squashfs'
                   else ['jefferson', '-d', dest, img])
            p = subprocess.run(cmd, capture_output=True, text=True)
            if p.returncode != 0 or not os.path.isdir(dest):
                sys.exit(f'unpack: {name} failed: {(p.stderr or p.stdout).strip()[-300:]}')
            n = sum(len(f) for _, _, f in os.walk(dest))
            print(f'{name}: {n} files')
    print(f'the dump is {os.path.join(out, "nor.bin")}: the camera\'s flash, the copy of the original firmware. The rest of '
          f'{out} is the same flash unpacked, which the build reads')


if __name__ == '__main__':
    main()
