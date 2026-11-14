#!/usr/bin/env python3
"""Patch the ISP API bin (the camera's /customer/iq_file_gc8613.bin) so the ISP's dynamic defect-pixel correction replaces
hot pixels instead of blending the correction away: bBlendEn = 0 in the 16 auto sets and the manual set of the DynamicDP
record (API id 0x1017), then the header's byte-sum checksum. microcamd is tested with the patched file; the stock one shows
the hot pixels. Format: docs/firmware/components/isp-3-iq-api.md; the record is 17 sets of 44 bytes, bBlendEn at +16.
usage: iq-dpc-noblend.py <in.bin> <out.bin>"""
import struct, sys
b = bytearray(open(sys.argv[1], 'rb').read())
chip, ver, ln, csum, key = struct.unpack_from('<5I', b, 0)
assert ln == len(b) - 32 and csum == sum(b[32:]) & 0xffffffff, 'header'
o = 32; found = 0
while o < len(b):
    k, t, n = struct.unpack_from('<3I', b, o)
    assert k == key
    api, cnt = struct.unpack_from('<HH', b, o + 12)
    if api == 0x1017:
        assert t == 5 and cnt == 1; found += 1
        p = o + 12 + 4 + 4 * cnt + struct.unpack_from('<I', b, o + 16)[0]
        assert n - 4 - 4 * cnt == 8 + 17 * 44
        for s in range(17):
            q = p + 8 + 44 * s + 16
            assert struct.unpack_from('<I', b, q)[0] in (0, 1)
            struct.pack_into('<I', b, q, 0)
        print('DynamicDP at 0x%x: enable %d, bBlendEn cleared in 17 sets' % (p, struct.unpack_from('<I', b, p)[0]))
    o += 12 + n
assert o == len(b) and found == 1, 'DynamicDP record not found exactly once'
struct.pack_into('<I', b, 12, sum(b[32:]) & 0xffffffff)
open(sys.argv[2], 'wb').write(b)
