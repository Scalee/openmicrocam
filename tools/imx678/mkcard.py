#!/usr/bin/env python3
"""Build the install SD card for the IMX678 camera (black YIZHAN 4K, SPI NAND): OpenMicroCam's microcamd with the IMX678
driver in place of the vendor app, and the USB network (RNDIS) with telnet. The camera does not need to be opened.

usage: mkcard.py OUTDIR [STAGE]
  OUTDIR  emptied, then filled with the card's files; copy them into the root of a FAT32 card
  STAGE   the built files: microcamd, microcam, microcam_sys.ko, microcam_canvas.ko, drv_ms_cus_imx678_MIPI.ko.
          Without it the card installs the USB network and telnet only (the vendor app stays), which is what the
          build needs first: the camera's libraries (docs/IMX678.md «Building»)

How the card runs (docs/IMX678.md):
 1. Normal boot with the card in: the vendor's sd_version_recognition finds H161X.bin and an sd_version.txt whose version
    differs from the camera's, sets ota_upgrade_status 2 and reboots.
 2. The IPL starts U-Boot (it skips it while ota_upgrade_status is 0); U-Boot runs the card's H161X.bin as a script: it
    makes the card inert (sd_version.txt -> the camera's own version, H161X.bin -> an empty script), sets
    ota_upgrade_status back to 0 and saves, then boots Linux once with BOOTARGS.TXT: the stock command line plus an
    rdinit shell that mounts /config and the card and runs OMC/install.sh.
 3. OMC/install.sh checks the camera's files, makes the patched copies, installs /customer/omc, adds one line to
    /customer/set.sh, logs to OMC/install.log on the card and reboots.
Nothing of the boot chain (IPL, U-Boot, kernel, rootfs) is written; U-Boot writes only its environment. No vendor file is
on the card: the patched copies are made on the camera from its own files.
"""
import hashlib, os, shutil, sys

HERE = os.path.dirname(os.path.abspath(__file__))
CAM = os.path.join(os.path.dirname(os.path.dirname(HERE)), 'camera', 'imx678')

CAMERA_VERSION = 'W1167822507261'          # /customer/version.txt of the firmware this is made for
CARD_VERSION = 'OMC-IMX678'                # differs: makes sd_version_recognition set ota_upgrade_status 2
SCRIPT_LEN = 0x4000                        # sdstar reads this much of H161X.bin as the script
SCRIPT_SIZE = SCRIPT_LEN + 8 + 16          # plus the SDK packer's tail
LINE_MAX = 511                             # U-Boot cuts a command line after this many bytes
VER_SIZE = 0x200                           # sd_version.txt and VEROK.TXT: one size, U-Boot writes over it in place
BUF = '0x21000000'
BUILT = ['microcamd', 'microcam', 'microcam_sys.ko', 'microcam_canvas.ko', 'drv_ms_cus_imx678_MIPI.ko']
MCD = ['start.sh', 'config.toml', 'policy.toml', 'formats.toml']

STOCK_BOOTARGS = ('ubi.mtd=ubi0,2048 rootfstype=ramfs initrd=0x21000000,0x4C0000 LX_MEM=0xffe0000 '
                  'mma_heap=mma_heap_name0,miu=0,sz=0x9000000 mma_memblock_remove=1 cma=2M@0x23800000 '
                  'mmap_reserved=fb,miu=0,sz=0x800000,max_start_off=0xF700000,max_end_off=0xFF00000 loglevel=0 '
                  'mtdparts=nand0:768k@0x140000(IPL0),384k(IPL_CUST0),384k(IPL_CUST1),640k(UBOOT0),640k(UBOOT1),'
                  '256k(ENV0),5m(KERNEL),5m(RECOVERY),5504k(rootfs),1280k(MISC),-(ubi0)')

# PID 1 of the one-time boot: what rcS and /etc/init.sh would do up to the card (the GPIOs init.sh switches on), then
# OMC/install.sh; any failure falls through to a reboot, and the next boot is the normal one (ota_upgrade_status is 0)
RDINIT = ('mount -t proc p /proc;mount -t sysfs s /sys;mount -t devtmpfs d /dev||mdev -s;'
          'cd /sys/class/gpio;for g in 130 136 138;do echo $g>export;echo out>gpio$g/direction;echo 1>gpio$g/value;done;'
          'insmod /lib/modules/4.9.227/ubi.ko;insmod /lib/modules/4.9.227/ubifs.ko;mdev -s;'
          'ubiattach /dev/ubi_ctrl -m 10;mdev -s;mount -t ubifs ubi0:miservice /config;cd /config/modules/4.9.227;'
          'for m in mmc_core pwrseq_simple mmc_block kdrv_sdmmc fat vfat;do insmod $m.ko;done;sleep 3;mdev -s;'
          'mount -t vfat /dev/mmcblk0p1 /mnt&&exec sh /mnt/OMC/install.sh;sleep 5;reboot -f')


def card_script(lines):
    for l in lines:
        if len(l.encode()) > LINE_MAX or '$' in l or ';' in l:
            sys.exit(f'card line not safe for U-Boot: {l}')
    text = ('\n'.join(lines + ['% <- this is end of script symbol']) + '\n').encode()
    assert len(text) <= SCRIPT_LEN
    body = text + b'\xff' * (SCRIPT_LEN - len(text))
    return body + b'12345678' + body[:16]


def version_file(v):
    text = f'version:{v}\n'.encode()
    return text + b'#' * (VER_SIZE - len(text) - 1) + b'\n'


def main():
    if len(sys.argv) not in (2, 3):
        sys.exit(__doc__)
    out, stage = sys.argv[1], (sys.argv[2] if len(sys.argv) == 3 else None)
    if stage:
        missing = [f for f in BUILT if not os.path.isfile(os.path.join(stage, f))]
        if missing:
            sys.exit(f'{stage}: missing {", ".join(missing)} (docs/IMX678.md «Building»)')
    shutil.rmtree(out, ignore_errors=True)
    for d in ('OMC/mcd', 'OMC/patches') if stage else ('OMC/patches',):
        os.makedirs(os.path.join(out, d))

    def put(rel, data):
        with open(os.path.join(out, rel), 'wb') as f:
            f.write(data)

    def text(path):
        return open(path, 'rb').read().replace(b'\r\n', b'\n')

    bootargs = f'bootargs={STOCK_BOOTARGS} rdinit=/bin/sh -- -c "{RDINIT}"\n'.encode()
    cmdline = len(bootargs) - len('bootargs=') - 1
    if cmdline >= 1024:
        sys.exit(f'kernel command line {cmdline} bytes, ARM takes 1023')
    put('BOOTARGS.TXT', bootargs)

    script = [
        # the card inert first, so that a failure further on cannot leave a card that reboots the camera at every boot
        f'fatload mmc 0 {BUF} VEROK.TXT',
        f'fatwrite mmc 0 {BUF} sd_version.txt {VER_SIZE:#x}',
        f'fatload mmc 0 {BUF} DONE.BIN',
        f'fatwrite mmc 0 {BUF} H161X.bin {SCRIPT_SIZE:#x}',
        'setenv ota_upgrade_status 0',
        'saveenv',
        # Linux once, with the one-time command line (in RAM only: nothing saves the environment after this)
        'fatload mmc 0 0x22000000 BOOTARGS.TXT',
        f'env import -t 0x22000000 {len(bootargs):#x}',
        'nand read.e 0x22000000 KERNEL 0x460000',
        'nand read.e 0x21000000 rootfs 0x4C0000',
        'bootm 0x22000000',
    ]
    put('H161X.bin', card_script(script))
    put('DONE.BIN', card_script([]))
    put('sd_version.txt', version_file(CARD_VERSION))
    put('VEROK.TXT', version_file(CAMERA_VERSION))

    for name in ('install.sh', 'hook.sh'):
        put(f'OMC/{name}', text(os.path.join(CAM, name)))
    # the card's intent: microcamd on when it carries it, the vendor app (with the USB network) when not
    conf = text(os.path.join(CAM, 'omc.conf'))
    on, off = b'\nOMC_MICROCAMD=1\n', b'\nOMC_MICROCAMD=0\n'
    assert conf.count(on) == 1
    put('OMC/omc.conf', conf if stage else conf.replace(on, off))
    for name in os.listdir(os.path.join(CAM, 'patches')):
        put(f'OMC/patches/{name}', text(os.path.join(CAM, 'patches', name)))
    if not stage:
        print(f'{out}: network only; H161X.bin script {len(script)} lines; kernel command line {cmdline} bytes')
        return
    sums = []
    for name in MCD + BUILT:
        data = text(os.path.join(CAM, name)) if name in MCD else open(os.path.join(stage, name), 'rb').read()
        put(f'OMC/mcd/{name}', data)
        sums.append(f'{hashlib.md5(data).hexdigest()}  {name}\n')
    put('OMC/mcd/md5.txt', ''.join(sums).encode())
    print(f'{out}: H161X.bin script {len(script)} lines; kernel command line {cmdline} bytes; {len(sums)} files in OMC/mcd')


if __name__ == '__main__':
    main()
