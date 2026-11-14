#!/usr/bin/env python3
"""Generate yz_desc.h for our g_webcam rebuild: the UVC control and streaming descriptors byte-exact from the stock
g_webcam.ko's .data (offsets below; the descriptors: docs/firmware/components/usb-gadget.md §4.3-4.4), in the stock list
order, with the frame-interval lists of the frame sizes the FPS table names replaced in every format. A host that asks
for nothing gets MJPEG 4K45, and H.264's default frame is its 4K35 (the stock one, 320x240, does not stream). Beside the
descriptors, the format list the gadget shows microcamd (/sys/module/g_webcam/parameters/formats), made from them.
usage: gen_desc.py <stock g_webcam.ko> <out.h>"""
import struct, subprocess, sys
src, out = sys.argv[1:3]
hdr = subprocess.check_output(['arm-linux-gnueabihf-readelf', '-S', '-W', src], text=True)
line = [l for l in hdr.splitlines() if ' .data ' in l][0].split()
i = line.index('.data'); off = int(line[i + 3], 16)
b = open(src, 'rb').read()
def d(o):
    L = b[off + o]; return bytearray(b[off + o: off + o + L])

# interval lists (100 ns units), fastest first; dwDefaultFrameInterval stays 333333 (30 fps), except the 4K frames:
# MJPEG 4K45 and H.264 4K35, each its format's fastest
FPS = {(3840, 2160): (222222, 333333, 1000000), (1920, 1080): (83333, 166666, 333333),    # 4K 45/30/10 (sensor mode 29 = 45.6 fps)
       (2560, 1440): (151515, 333333, 1000000), (3840, 1600): (166666, 333333, 1000000),      # window-crop sensor modes:
       (3840, 1440): (151515, 333333, 1000000), (2848, 1600): (166666, 333333, 1000000)}      # 66 / 60 / 66 / 60 fps
# per format: the H.26x encoder (Wave5, one for H.264 and H.265) encodes 4K at ~36 fps at most, so 4K is
# offered at 35 / 30 / 10 there
FPS_OF = {(0x07bc, 3840, 2160): (285714, 333333, 1000000), (0x068b, 3840, 2160): (285714, 333333, 1000000)}
# MJPEG sizes nobody uses are replaced by the native sizes of the window-crop modes (same index in prog_uvc's uvc_formats[])
RESIZE = {(640, 480): (3840, 1600), (320, 240): (3840, 1440), (424, 240): (2848, 1600)}

CONTROL = [0x0154, 0x0161, 0x0173, 0x017e, 0x0199, 0x0130]          # VC header, IT, PU, XU6, XU2, OT

# Advertised controls = the ones microcamd serves (the stock module sets every bit; the rest only ever STALLed). The served set is
# read from microcamd's control table; selector → bmControls bit per UVC 1.5 tables 4-3 (camera terminal) and 4-6 (processing unit).
import os, re
MICROCAM = os.path.join(os.path.dirname(os.path.abspath(__file__)), '..', '..', 'crates/microcam-core/src/uvc_controls.rs')
served = re.findall(r'\((CAMERA_TERMINAL|PROCESSING_UNIT), (0x[0-9a-fA-F]+)\) =>', open(MICROCAM).read())
CT_BIT = {0x01: 0, 0x02: 1, 0x03: 2, 0x04: 3, 0x05: 4, 0x06: 5, 0x07: 6, 0x09: 7, 0x0a: 8, 0x0b: 9, 0x0c: 10, 0x0d: 11, 0x0e: 12,
          0x0f: 13, 0x10: 14, 0x11: 18, 0x08: 17, 0x12: 19, 0x13: 20, 0x14: 21}
PU_BIT = {0x02: 0, 0x03: 1, 0x06: 2, 0x07: 3, 0x08: 4, 0x09: 5, 0x0a: 6, 0x0c: 7, 0x01: 8, 0x04: 9, 0x05: 10, 0x10: 11, 0x0b: 12,
          0x0d: 13, 0x0e: 14, 0x0f: 15, 0x11: 16, 0x12: 17}
mask = {u: sum(1 << (CT_BIT if u == 'CAMERA_TERMINAL' else PU_BIT)[int(sel, 16)] for uu, sel in served if uu == u)
        for u in ('CAMERA_TERMINAL', 'PROCESSING_UNIT')}
assert mask['CAMERA_TERMINAL'] and mask['PROCESSING_UNIT'], served

def set_controls(x):
    """bmControls of an input terminal / processing unit = microcamd's set; an extension unit advertises none."""
    if x[2] == 0x02:                                                  # camera terminal: bControlSize at 14
        n = x[14]; x[15:15 + n] = mask['CAMERA_TERMINAL'].to_bytes(n, 'little')
    elif x[2] == 0x05:                                                # processing unit: bControlSize at 7
        n = x[7]; x[8:8 + n] = mask['PROCESSING_UNIT'].to_bytes(n, 'little')
    elif x[2] == 0x06:                                                # extension unit: after bNrInPins and baSourceID
        x[20] = 0; p = x[21]; n = x[22 + p]; x[23 + p:23 + p + n] = bytes(n)
STREAM = ([0x00b8, 0x0903, 0x081f, 0x08b7, 0x0891, 0x086b, 0x0845, 0x08dd, 0x07f9, 0x07ee, 0x07e3, 0x07d8]   # in-hdr, MJPEG fmt+7 frames+3 stills
          + [0x07bc, 0x0796, 0x0770, 0x074a, 0x0724, 0x06fe, 0x06d8, 0x06b2, 0x06a7]                         # H.264 fmt + 7 frames + still
          + [0x068b, 0x0665, 0x063f, 0x0619, 0x05f3, 0x05cd, 0x05a7, 0x0581, 0x0576]                         # H.265 fmt + 7 frames + still
          + [0x0570])                                                                                           # colour matching
assert len(STREAM) == 31
FORMAT_INDEX = {0x0903: 1, 0x07bc: 2, 0x068b: 3}
descs = []
fmt_of = None
for o in CONTROL + STREAM:
    x = d(o); assert x[1] == 0x24, hex(o)
    if o in FORMAT_INDEX:
        fmt_of = o
    if o in CONTROL:
        set_controls(x)
    if o in FORMAT_INDEX:                                             # format descriptor: bFormatIndex at 3
        x[3] = FORMAT_INDEX[o]
    if x[2] in (0x07, 0x11) and len(x) == 38:                       # MJPEG frame / frame-based frame, 3 discrete intervals
        w, h = struct.unpack_from('<HH', x, 5); n_off = 21 if x[2] == 0x11 else 25
        assert x[n_off] == 3 and struct.unpack_from('<3I', x, 26) == (333333, 666666, 1000000)
        if x[2] == 0x07 and (w, h) in RESIZE:                        # bitrate = w*h*160 and buffer = w*h*2, as the stock 4K entry
            w, h = RESIZE[(w, h)]; struct.pack_into('<HH4I', x, 5, w, h, w * h * 160, w * h * 160, w * h * 2, 333333)
        if (w, h) in FPS:
            struct.pack_into('<3I', x, 26, *FPS_OF.get((fmt_of, w, h), FPS[(w, h)]))
        if (w, h) == (3840, 2160) and fmt_of in (0x0903, 0x07bc):   # dwDefaultFrameInterval: MJPEG at 21, frame-based at 17
            struct.pack_into('<I', x, 21 if x[2] == 0x07 else 17, FPS_OF.get((fmt_of, w, h), FPS[(w, h)])[0])
    descs.append((o, x))
# H.264's default frame (stock: 320x240, which does not stream) = its 3840x2160 frame; bDefaultFrameIndex at 22
fmt = next(x for o, x in descs if o == 0x07bc)
h264 = STREAM[STREAM.index(0x07bc) + 1:STREAM.index(0x06a7)]   # its frame descriptors
fmt[22] = next(x[3] for o, x in descs if o in h264 and struct.unpack_from('<HH', x, 5) == (3840, 2160))


def format_list():
    """The format list the loaded gadget shows microcamd (parameter `formats`), from these very descriptors: probe/commit
    name formats and frames by index. Intervals in UVC 100 ns units, fastest first."""
    GUIDS = {bytes.fromhex('4832363400001000800000aa00389b71'): 'H264', bytes.fromhex('4832363500001000800000aa00389b71'): 'H265'}
    out = ['schema = 1']
    for o, x in descs:
        if o not in STREAM:
            continue
        if x[2] in (0x06, 0x10):                                    # MJPEG / frame-based format
            if out[-1].endswith(','):
                out.append(']')
            codec = 'MJPEG' if x[2] == 0x06 else GUIDS[bytes(x[5:21])]
            out += ['[[format]]', f'index = {x[3]}', f'codec = "{codec}"', f'default_frame = {x[6] if x[2] == 0x06 else x[22]}',
                    'frame = [']
        elif x[2] in (0x07, 0x11):                                  # its frames
            w, h = struct.unpack_from('<HH', x, 5)
            n, dflt = (x[25], struct.unpack_from('<I', x, 21)[0]) if x[2] == 0x07 else (x[21], struct.unpack_from('<I', x, 17)[0])
            ints = ', '.join(str(i) for i in struct.unpack_from(f'<{n}I', x, 26))
            out.append(f'{{index = {x[3]}, width = {w}, height = {h}, intervals = [{ints}], default_interval = {dflt}}},')
    text = '\n'.join(out + [']', ''])
    assert len(text) < 4000, 'a module parameter shows at most a page'
    return text


with open(out, 'w') as f:
    f.write('/* Generated by gen_desc.py from the stock g_webcam.ko (.data); do not edit. */\n')
    for o, x in descs:
        f.write('static u8 yz_d%04x[%d] __aligned(4) = { %s };\n' % (o, len(x), ', '.join('0x%02x' % c for c in x)))
    f.write('static struct uvc_descriptor_header *yz_control[] = {\n%s\tNULL,\n};\n' % ''.join(
        '\t(struct uvc_descriptor_header *)yz_d%04x,\n' % o for o in CONTROL))
    f.write('static struct uvc_descriptor_header *yz_streaming[] = {\n%s\tNULL,\n};\n' % ''.join(
        '\t(struct uvc_descriptor_header *)yz_d%04x,\n' % o for o in STREAM))
    f.write('static const char yz_format_list[] =\n%s;\n' % ''.join(
        '\t"%s\\n"\n' % l.replace('"', '\\"') for l in format_list().splitlines()))
print('control', len(CONTROL), 'streaming', len(STREAM))
