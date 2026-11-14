#!/usr/bin/env python3
"""The GC8613 driver's sensor modes: each mode's register table, built from the three bases in tables/ and our changes
to them, and what the driver and microcamd need to know of the mode.

usage: modes.py > modes.h   the C data the driver is built with (Kbuild runs it), the mode list it shows microcamd
                            among them (/sys/module/drv_ms_cus_gc8613_MIPI/parameters/modes)
       modes.py list        that mode list (the host tests read it)

Facts are computed from the tables: the frame length (VTS, 0x0340/0x0341), the exposure the table leaves (0x0202/0x0203),
the line length (HTS, 0x0342/0x0343) and from it with the PLL's clock the line time, the MIPI lanes (0x0114). The mode list states what is ours to choose: the name
(microcamd's identity of the mode), the output size, the rate it runs at, whether its rate is pinned.
"""
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
PAUSE = 0xffff  # a table entry that waits its value in ms


def base(file, section):
    """[(reg, value)] of a section of tables/<file>.txt; `pause MS` becomes (PAUSE, MS)."""
    out, cur = [], None
    for line in open(os.path.join(HERE, 'tables', file + '.txt')):
        line = line.split('#')[0].strip()
        if not line:
            continue
        if line.startswith('['):
            cur = line[1:-1]
        elif cur == section:
            a, b = line.split()
            out.append((PAUSE, int(b)) if a == 'pause' else (int(a, 16), int(b, 16)))
    assert out, f'{file} [{section}] empty'
    return out


def gain_steps():
    """[(total gain, 1024 = 1x; values of 0614 0615 0225 1467 1468 00b8 00b9 1447)] of the stock analog gain steps."""
    out, on = [], False
    for line in open(os.path.join(HERE, 'tables', 'stock.txt')):
        line = line.split('#')[0].strip()
        if line.startswith('['):
            on = line == '[gain]'
        elif line and on:
            f = line.split()
            out.append((int(f[0]), [int(x, 16) for x in f[1:]]))
    return out


def value(t, reg):
    v = [b for a, b in t if a == reg]
    assert len(v) == 1, f'{reg:04x} written {len(v)} times'
    return v[0]


def word(t, hi):
    return value(t, hi) << 8 | value(t, hi + 1)


def put(t, regs):
    """Every write of these registers takes the new value."""
    return [(a, regs.get(a, b)) for a, b in t]


def insert_after(t, after, new):
    i = max(k for k, (a, _) in enumerate(t) if a == after)
    return t[:i + 1] + [new] + t[i + 1:]


def otp_pause(t):
    """20 ms after the write that starts the sensor's OTP load (0x0a5a = 0x80) before the next one, as the stock table
    and the sensor's orientation procedure wait there."""
    out = []
    for k, w in enumerate(t):
        out.append(w)
        if w == (0x0a5a, 0x80) and k + 1 < len(t) and t[k + 1][0] != PAUSE:
            out.append((PAUSE, 20))
    return out


def hts(t, v):
    return put(t, {0x0342: v >> 8, 0x0343: v & 0xff})


def vts(t, v):
    return put(t, {0x0340: v >> 8, 0x0341: v & 0xff})


def window(t, w, h, margin):
    """A centred w × h part of the 3840 × 2160 readout: the array reads h rows (+ 24 for the sensor's own margin rows),
    the output takes w of its columns; the frame is the read rows + `margin` lines. Starts stay even (the Bayer phase)."""
    rows = h + 24
    row0 = 2 + (2160 - h) // 2
    col0 = 12 + (3840 - w) // 2
    assert row0 % 2 == 0 and col0 % 2 == 0 and w % 8 == 0
    regs = {0x0346: row0 >> 8, 0x0347: row0 & 0xff,   # first array row
            0x034a: rows >> 8, 0x034b: rows & 0xff,   # array rows read
            0x0096: h >> 8, 0x0097: h & 0xff,         # output rows
            0x0259: (rows + 14) >> 8, 0x025a: (rows + 14) & 0xff,  # follows the rows read (14 more)
            0x0094: w >> 8, 0x0095: w & 0xff,         # output columns
            0x009a: col0 >> 8, 0x009b: col0 & 0xff,   # first output column
            0x010d: (w * 10 // 8) & 0xff, 0x010e: (w * 10 // 8) >> 8}  # MIPI line: w RAW10 pixels in bytes
    out = put(t, regs)
    for after, reg in ((0x0345, 0x0346), (0x0099, 0x009a)):
        if reg not in dict(t):
            out = insert_after(out, after, (reg, regs[reg]))
    return vts(out, rows + margin)


SHENSHU = base('shenshu', 'init-1080p-binned')
# the registers in which the GC8613's 2x2-binned 1920 × 1080 readout differs from the full one, from the binned table
BINNING = (0x0075, 0x0099, 0x009b, 0x010d, 0x010e, 0x0259, 0x025a, 0x0349, 0x060c, 0x070c, 0x0902)
BINNING_ON = (0x0077, 0x0218)  # row and column binning on: absent from the full readout, written after 0x0075


def binned(t):
    """The full readout `t` 2x2-binned to 1920 × 1080 on 2 lanes."""
    out = put(t, {r: value(SHENSHU, r) for r in BINNING + (0x0114,)})
    for r in reversed(BINNING_ON):
        out = insert_after(out, 0x0075, (r, value(SHENSHU, r)))
    return out


STOCK = base('stock', 'init')
RK40 = otp_pause(base('rockchip', 'init-40fps'))
# the stock table's PLL settings that differ from the Shenshu table's: MCLK 24 MHz to wpclk 216 MHz
STOCK_PLL = {r: value(STOCK, r) for r in (0x061e, 0x0a31)}
WPCLK_STOCK, WPCLK_RK = 216e6, 351e6  # the sensor's pixel clock of the stock and the Rockchip PLL settings


class Mode:
    def __init__(self, name, table, wpclk, size, fps, pinned=True):
        self.name, self.table, self.width, self.height, self.max_fps, self.pinned = name, table, *size, fps, pinned
        self.frame_lines = word(table, 0x0340)
        self.expo = word(table, 0x0202) & 0x3fff   # the exposure the table leaves
        self.line_ns = round(word(table, 0x0342) * 4e9 / wpclk)   # a line is HTS × 4 pixel clocks
        self.lanes = {0x03: 4, 0x01: 2}[value(table, 0x0114)]
        rate = 1e9 / (self.line_ns * self.frame_lines)
        assert len(name) <= 31 and abs(rate - fps) < 1, (name, rate)


# Index 0 is the stock 4K30: the mode a daemon runs when the driver gives no mode list. The rest are the modes microcamd's policy uses,
# 4K first, then the 3840-wide windows, the narrower windows, the binned 1080p.
MODES = [
    Mode('3840x2160@30 stock', STOCK, WPCLK_STOCK, (3840, 2160), 30, pinned=False),
    Mode('3840x2160@45 hts340', hts(RK40, 0x340), WPCLK_RK, (3840, 2160), 45),
    Mode('3840x2160@43 hts360', hts(RK40, 0x360), WPCLK_RK, (3840, 2160), 43),
    Mode('3840x2160@40 rk fastpll', RK40, WPCLK_RK, (3840, 2160), 40),
    Mode('3840x1600@60 hts340', window(hts(RK40, 0x340), 3840, 1600, 130), WPCLK_RK, (3840, 1600), 60),
    Mode('3840x1440@66 hts340', window(hts(RK40, 0x340), 3840, 1440, 130), WPCLK_RK, (3840, 1440), 66),
    Mode('3840x1600@52 hts3b4', window(RK40, 3840, 1600, 130), WPCLK_RK, (3840, 1600), 52),
    Mode('3840x1440@58 hts3b4', window(RK40, 3840, 1440, 130), WPCLK_RK, (3840, 1440), 58),
    Mode('2848x1600@60 crop', window(hts(RK40, 0x340), 2848, 1600, 130), WPCLK_RK, (2848, 1600), 60),
    Mode('2560x1440@66 crop', window(hts(RK40, 0x340), 2560, 1440, 130), WPCLK_RK, (2560, 1440), 66),
    Mode('2560x1440@92 crop h278', window(hts(RK40, 0x278), 2560, 1440, 46), WPCLK_RK, (2560, 1440), 92),
    Mode('1920x1080@121 rk40 bin hts278', hts(vts(binned(RK40), 1142), 0x278), WPCLK_RK, (1920, 1080), 121),
    Mode('1920x1080@81 rk40 bin 2lane', vts(binned(RK40), 1142), WPCLK_RK, (1920, 1080), 81),
    Mode('1920x1080@59 2lane vts1142', vts(hts(otp_pause(put(SHENSHU, STOCK_PLL)), 0x320), 1142), WPCLK_STOCK,
         (1920, 1080), 59),
]


def header():
    out = ['/* Generated by modes.py from the tables in tables/; do not edit. */']
    for i, m in enumerate(MODES):
        out.append(f'static const struct reg mode{i}_regs[] = {{')
        regs = [f'{{0x{a:04x}, 0x{b:02x}}},' for a, b in m.table]
        out += ['\t' + ' '.join(regs[k:k + 6]) for k in range(0, len(regs), 6)]
        out.append('};')
    out.append('static const struct mode modes[] = {')
    for i, m in enumerate(MODES):
        out.append(f'\t{{ "{m.name}", mode{i}_regs, ARRAY_SIZE(mode{i}_regs), {m.width}, {m.height}, {m.max_fps}, '
                   f'{m.lanes}, {m.line_ns}, {m.frame_lines}, {m.expo}, {int(m.pinned)} }},')
    out.append('};')
    out.append('static const struct gain_step gain_steps[] = {')
    for g, v in gain_steps():
        out.append(f'\t{{ {g}, {{ ' + ', '.join(f'0x{x:02x}' for x in v) + ' } },')
    out.append('};')
    out.append('static const char mode_list[] =')
    out += ['\t"' + l.replace('"', '\\"') + '\\n"' for l in mode_list().splitlines()]
    out.append(';')
    return '\n'.join(out) + '\n'


def mode_list():
    """The mode list the loaded driver shows microcamd (parameter `modes`): what the MI API does not give, a mode a line,
    identity = name."""
    out = ['schema = 1', 'mode = [']
    for m in MODES:
        cls = 'full' if (m.width, m.height) == (3840, 2160) else 'binned' if m.lanes == 2 else 'crop'
        out.append(f'{{name = "{m.name}", width = {m.width}, height = {m.height}, max_fps = {m.max_fps}, min_fps = 5, '
                   f'line_ns = {m.line_ns}, frame_lines = {m.frame_lines}, lanes = {m.lanes}, class = "{cls}", '
                   f'fps_pinned = {"true" if m.pinned else "false"}}},')
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
