#!/usr/bin/env python3
"""The IMX678 driver's sensor modes for the black YIZHAN 4K camera: each mode's register table, made from the vendor
driver's tables (tables/vendor.txt) and our changes to them, and what the driver and microcamd need to know of the mode.

usage: modes.py > modes.h   the C data the driver is built with (Kbuild runs it), and the mode list it shows microcamd
       modes.py list        that mode list

The IMX678 (Sony STARVIS 2, 1/1.8"): INCK 24 MHz (INCK_SEL 4), MIPI 4 lanes at DATARATE_SEL 3 (720 MHz link), 10-bit AD
and output in every vendor table. A line is HMAX counts of 74.25 MHz (the 594 MHz pixel rate over 8 pixels a clock), a
frame VMAX lines. The SoC's ISP takes about one pixel a clock: a mode's output pixels per line over its line time must stay
under the ISP clock, 432 MHz unless raised (the vendor's 4K45, 3856 px per 660 counts = 433.8 Mpx/s, overflows it).
"""
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
PAUSE = 0xffff
COUNT_HZ = 74.25e6            # HMAX counts a second
ISP_HZ = 432e6                # the ISP clock the modes are made for (the stock one)
FULL_W, FULL_H = 3856, 2180   # the full readout (PIX_HWIDTH / PIX_VWIDTH of the vendor's 4K tables)


def base(section):
    """[(reg, value)] of a section of tables/vendor.txt; `pause MS` becomes (PAUSE, MS)."""
    out, cur = [], None
    for line in open(os.path.join(HERE, 'tables', 'vendor.txt'), encoding='utf-8'):
        line = line.split('#')[0].strip()
        if not line:
            continue
        if line.startswith('['):
            cur = line[1:-1]
        elif cur == section:
            a, b = line.split()
            out.append((PAUSE, int(b)) if a == 'pause' else (int(a, 16), int(b, 16)))
    assert out, f'[{section}] empty'
    return out


def value(t, reg):
    v = [b for a, b in t if a == reg]
    assert len(v) == 1, f'{reg:04x} written {len(v)} times'
    return v[0]


def le(t, reg, n):
    return sum(value(t, reg + k) << (8 * k) for k in range(n))


def put(t, regs):
    """Every write of these registers takes the new value; each must already be in the table."""
    for r in regs:
        value(t, r)
    return [(a, regs.get(a, b)) for a, b in t]


def split(reg, v, n):
    return {reg + k: (v >> (8 * k)) & 0xff for k in range(n)}


def timing(t, hmax, vmax):
    return put(t, {**split(0x302c, hmax, 2), **split(0x3028, vmax, 3)})


def window(t, w, h):
    """A centred w × h window of the full readout, read 1:1 (WINMODE 4: window cropping). Widths step by 16, the rest by 4."""
    hst, vst = (FULL_W - w) // 2, (FULL_H - h) // 2
    assert w % 16 == 0 and h % 4 == 0 and hst % 4 == 0 and vst % 4 == 0
    return put(t, {0x3018: 0x04, **split(0x303c, hst, 2), **split(0x303e, w, 2), **split(0x3044, vst, 2),
                   **split(0x3046, h, 2)})


class Mode:
    def __init__(self, name, table, size, out, cls, isp_hz=ISP_HZ):
        self.name, self.table, self.cls, self.isp_hz = name, table, cls, isp_hz
        self.width, self.height = size
        self.out_width, self.out_height = out
        self.hmax, self.frame_lines = le(table, 0x302c, 2), le(table, 0x3028, 3)
        self.line_ns = round(self.hmax * 1e9 / COUNT_HZ)
        self.expo = self.frame_lines - le(table, 0x3050, 3)          # SHR counts back from the frame's end
        self.lanes = 4
        self.max_fps = int(COUNT_HZ / (self.hmax * self.frame_lines) + 1e-6)
        rate = COUNT_HZ / (self.hmax * self.frame_lines)
        px_rate = self.out_width * COUNT_HZ / self.hmax
        assert len(name) <= 31 and name.endswith(f'@{self.max_fps}fps'), (name, rate)
        assert px_rate < isp_hz * 0.99, f'{name}: {px_rate / 1e6:.1f} Mpx/s into a {isp_hz / 1e6:.0f} MHz ISP'


V4K45 = base('4k45')
MODES = [
    # the vendor's own 2x2-binned 1080p60, unchanged: full view (ADDMODE 1), 1928 x 1090 read, the ISP takes 1920 x 1080
    Mode('1920x1080@60fps', base('1080p60-binned'), (1920, 1080), (1920, 1080), 'binned'),
    # full 4K: the vendor's 4K45 table with a line long enough for the 432 MHz ISP (680 counts, 9.16 us: 421 Mpx/s) and the
    # shortest frame the vendor's 4K30 runs (2250 lines: 70 lines between frames, 0.64 ms)
    Mode('3840x2160@48fps', timing(V4K45, 680, 2250), (3840, 2160), (FULL_W, FULL_H), 'full'),
    # 2688 x 1512 read 1:1 from the centre (1.43x narrower view than the full 3840): a 2704 x 1532 window, the sensor's
    # 10-bit minimum line (550 counts, 7.41 us: 365 Mpx/s)
    Mode('2688x1512@60fps', timing(window(V4K45, 2704, 1532), 550, 2250), (2688, 1512), (2704, 1532), 'crop'),
    # full 4K for a 480 MHz ISP (/sys/devices/virtual/mstar/isp0/isp_clk; the highest rate of its clock table): 630 counts,
    # 8.48 us, 454 Mpx/s (5.3 % under 480; 610 counts, 469 Mpx/s, overflowed on one frame in four)
    Mode('3840x2160@52fps', timing(V4K45, 630, 2250), (3840, 2160), (FULL_W, FULL_H), 'full', isp_hz=480e6),
]


def gains():
    """The gain register's codes (0.3 dB each, 0 .. 240 = 72 dB; above 100 = 30 dB the sensor adds digital gain) as
    total gain, 1024 = 1x."""
    return [round(1024 * 10 ** (0.3 * c / 20)) for c in range(241)]


def header():
    out = ['/* Generated by modes.py from tables/vendor.txt; do not edit. */']
    for i, m in enumerate(MODES):
        out.append(f'static const struct reg mode{i}_regs[] = {{')
        regs = [f'{{0x{a:04x}, 0x{b:02x}}},' for a, b in m.table]
        out += ['\t' + ' '.join(regs[k:k + 6]) for k in range(0, len(regs), 6)]
        out.append('};')
    out.append('static const struct mode modes[] = {')
    for i, m in enumerate(MODES):
        out.append(f'\t{{ "{m.name}", mode{i}_regs, ARRAY_SIZE(mode{i}_regs), {m.width}, {m.height}, {m.out_width}, '
                   f'{m.out_height}, {m.max_fps}, {m.lanes}, {m.line_ns}, {m.frame_lines}, {m.expo} }},')
    out.append('};')
    out.append('static const u32 gain_codes[] = {')
    g = gains()
    out += ['\t' + ', '.join(str(x) for x in g[k:k + 12]) + ',' for k in range(0, len(g), 12)]
    out.append('};')
    out.append('static const char mode_list[] =')
    out += ['\t"' + l.replace('"', '\\"') + '\\n"' for l in mode_list().splitlines()]
    out.append(';')
    return '\n'.join(out) + '\n'


def mode_list():
    """The mode list the loaded driver shows microcamd (parameter `modes`), identity = name."""
    out = ['schema = 1', 'mode = [']
    for m in MODES:
        out.append(f'{{name = "{m.name}", width = {m.width}, height = {m.height}, max_fps = {m.max_fps}, min_fps = 3, '
                   f'line_ns = {m.line_ns}, frame_lines = {m.frame_lines}, lanes = {m.lanes}, class = "{m.cls}", '
                   f'fps_pinned = false}},')
    text = '\n'.join(out + [']', ''])
    assert len(text) < 4000, 'a module parameter shows at most a page'
    return text


if __name__ == '__main__':
    if sys.argv[1:] == ['list']:
        sys.stdout.write(mode_list())
    elif len(sys.argv) == 1:
        sys.stdout.write(header())
    else:
        sys.exit('usage: modes.py > modes.h | modes.py list')
