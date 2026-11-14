#!/usr/bin/env python3
"""Full isoc IN capture from the binary usbmon interface; checks the UVC header of every non-empty packet.

usage: sudo python3 isocap.py <bus> <dev> <seconds> <outdir>
Prints packet/length/header statistics; for each anomalous packet saves the whole URB and the URB before it.
"""
import fcntl, os, struct, sys, time, ctypes

bus, dev, secs, out = int(sys.argv[1]), int(sys.argv[2]), float(sys.argv[3]), sys.argv[4]
os.makedirs(out, exist_ok=True)
fd = os.open(f"/dev/usbmon{bus}", os.O_RDONLY)
fcntl.ioctl(fd, 0x9204, 1200 * 1024)                     # MON_IOCT_RING_SIZE: the maximum ring
HDR = struct.Struct("<QBBBBHbbqiiIIiiiiII")              # struct mon_bin_hdr (64 bytes)
assert HDR.size == 64
hbuf = ctypes.create_string_buffer(64)
dbuf = ctypes.create_string_buffer(256 * 1024)
arg = struct.pack("<QQQ", ctypes.addressof(hbuf), ctypes.addressof(dbuf), len(dbuf))
MON_IOCX_GETX = 0x4018920A
stats = {"urbs": 0, "pkts": 0, "empty": 0, "lost": 0, "anom": 0, "trunc": 0}
lens, hdrs = {}, {}
prev = None
t_end = time.time() + secs
while time.time() < t_end:
    fcntl.ioctl(fd, MON_IOCX_GETX, arg)
    h = HDR.unpack(hbuf.raw)
    (_id, typ, xfer, ep, devnum, busnum, fs, fdat, ts, tus, status, len_urb, len_cap,
     ec, nd, interval, start_frame, xflags, ndesc) = h
    if typ != ord('C') or xfer != 0 or devnum != dev or not (ep & 0x80):
        continue
    stats["urbs"] += 1
    raw = dbuf.raw[:len_cap]
    descs = [struct.unpack_from("<iIII", raw, 16 * i)[:3] for i in range(ndesc)]
    data = raw[16 * ndesc:]
    if len(data) < len_urb:
        stats["trunc"] += 1
    bad = []
    for i, (st, off, ln) in enumerate(descs):
        stats["pkts"] += 1
        if st < 0:
            stats["lost"] += 1
            bad.append((i, st, off, ln, b""))
            continue
        lens[ln] = lens.get(ln, 0) + 1
        if ln == 0:
            stats["empty"] += 1
            continue
        p = data[off:off + min(ln, 16)]
        key = p[:2].hex()
        hdrs[key] = hdrs.get(key, 0) + 1
        if len(p) < 2 or p[0] != 12 or p[1] & ~0x83:
            bad.append((i, st, off, ln, p))
    if bad:
        stats["anom"] += 1
        n = stats["anom"]
        if n <= 20:
            with open(f"{out}/anom{n:02d}.txt", "w") as f:
                f.write(f"start_frame={start_frame} ndesc={ndesc} bad={[(i, s, o, l, p.hex()) for i, s, o, l, p in bad]}\n")
                f.write("descs=" + " ".join(f"{s}:{o}:{l}" for s, o, l in descs) + "\n")
            open(f"{out}/anom{n:02d}.urb", "wb").write(data)
            if prev:
                open(f"{out}/anom{n:02d}.prev", "wb").write(prev)
    prev = data
print(stats)
print("lengths:", sorted(lens.items(), key=lambda x: -x[1])[:8])
print("headers:", sorted(hdrs.items(), key=lambda x: -x[1])[:8])
