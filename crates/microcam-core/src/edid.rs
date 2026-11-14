//! EDID → the (size, rate) pairs a monitor accepts. Enough of E-EDID 1.3/1.4 and CEA-861 for the HDMI rule: detailed
//! timing descriptors, standard timings, CEA short video descriptors (VIC) and the HDMI vendor block's 4K VICs.
//! Rates are rounded to whole hertz (59.94 counts as 60: the transmitter outputs exact 60 anyway).

use crate::hdmi::MonitorCaps;
use crate::Size;

/// Parse an EDID (base block + extensions). Returns None when the header is wrong or the block is too short.
pub fn parse(edid: &[u8]) -> Option<MonitorCaps> {
    if edid.len() < 128 || edid[..8] != [0x00, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x00] {
        return None;
    }
    let mut modes: Vec<(Size, u32)> = Vec::new();
    let mut push = |s: Size, r: u32| {
        if s.w >= 640 && s.h >= 480 && r > 0 && !modes.contains(&(s, r)) {
            modes.push((s, r));
        }
    };

    // standard timings, bytes 38..53
    for i in 0..8 {
        let b0 = edid[38 + 2 * i];
        let b1 = edid[39 + 2 * i];
        if b0 == 0x01 && b1 == 0x01 || b0 == 0 {
            continue;
        }
        let w = (b0 as u32 + 31) * 8;
        let h = match b1 >> 6 {
            0 => w * 10 / 16,
            1 => w * 3 / 4,
            2 => w * 4 / 5,
            _ => w * 9 / 16,
        };
        push(Size::new(w, h), (b1 & 0x3f) as u32 + 60);
    }
    // detailed timing descriptors, 4 × 18 bytes from 54
    for i in 0..4 {
        dtd(&edid[54 + 18 * i..72 + 18 * i], &mut push);
    }
    // extensions
    let n_ext = edid[126] as usize;
    for e in 1..=n_ext {
        let Some(blk) = edid.get(128 * e..128 * e + 128) else { break };
        if blk[0] != 0x02 {
            continue; // not CEA-861
        }
        let dtd_start = blk[2] as usize;
        // data block collection 4..dtd_start
        let mut p = 4;
        while p + 1 <= dtd_start.min(127) && p < 127 {
            let tag = blk[p] >> 5;
            let len = (blk[p] & 0x1f) as usize;
            let body = &blk[p + 1..(p + 1 + len).min(128)];
            match tag {
                2 => {
                    for &svd in body {
                        if let Some((s, r)) = vic(svd & 0x7f) {
                            push(s, r);
                        }
                    }
                }
                3 if body.len() >= 3 && body[..3] == [0x03, 0x0c, 0x00] => {
                    // HDMI VSDB: 4K VICs behind the latency/3D flags
                    if body.len() > 8 && body[7] & 0x20 != 0 {
                        let mut q = 8;
                        if body[7] & 0x80 != 0 {
                            q += 2;
                        }
                        if body[7] & 0x40 != 0 {
                            q += 2;
                        }
                        // body[q] = 3D flags byte, body[q+1] = HDMI_VIC_LEN (3 bits) | HDMI_3D_LEN (5 bits)
                        if q + 1 < body.len() {
                            let hdmi_vic_len = (body[q + 1] >> 5) as usize;
                            for k in 0..hdmi_vic_len {
                                match body.get(q + 2 + k) {
                                    Some(1) => push(Size::new(3840, 2160), 30),
                                    Some(2) => push(Size::new(3840, 2160), 25),
                                    Some(3) => push(Size::new(3840, 2160), 24),
                                    _ => {}
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
            p += 1 + len;
        }
        // DTDs in the extension
        let mut q = dtd_start;
        while q + 18 <= 127 && q >= 4 {
            dtd(&blk[q..q + 18], &mut push);
            q += 18;
        }
    }
    Some(MonitorCaps { modes })
}

fn dtd(d: &[u8], push: &mut impl FnMut(Size, u32)) {
    let clock_khz = (d[0] as u32 | (d[1] as u32) << 8) * 10;
    if clock_khz == 0 {
        return; // a display descriptor, not a timing
    }
    let hact = d[2] as u32 | ((d[4] as u32 & 0xf0) << 4);
    let hblank = d[3] as u32 | ((d[4] as u32 & 0x0f) << 8);
    let vact = d[5] as u32 | ((d[7] as u32 & 0xf0) << 4);
    let vblank = d[6] as u32 | ((d[7] as u32 & 0x0f) << 8);
    let interlaced = d[17] & 0x80 != 0;
    if hact == 0 || vact == 0 || hblank == 0 || vblank == 0 || interlaced {
        return;
    }
    let total = (hact + hblank) as f64 * (vact + vblank) as f64;
    let rate = (clock_khz as f64 * 1000.0 / total).round() as u32;
    push(Size::new(hact, vact), rate);
}

/// CEA-861 video identification codes the camera can match.
fn vic(code: u8) -> Option<(Size, u32)> {
    let (w, h, r) = match code {
        1 => (640, 480, 60),
        2 | 3 => (720, 480, 60),
        4 => (1280, 720, 60),
        16 => (1920, 1080, 60),
        17 | 18 => (720, 576, 50),
        19 => (1280, 720, 50),
        31 => (1920, 1080, 50),
        32 => (1920, 1080, 24),
        33 => (1920, 1080, 25),
        34 => (1920, 1080, 30),
        60 => (1280, 720, 24),
        61 => (1280, 720, 25),
        62 => (1280, 720, 30),
        63 => (1920, 1080, 120),
        64 => (1920, 1080, 100),
        93 => (3840, 2160, 24),
        94 => (3840, 2160, 25),
        95 => (3840, 2160, 30),
        96 => (3840, 2160, 50),
        97 => (3840, 2160, 60),
        _ => return None,
    };
    Some((Size::new(w, h), r))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A synthetic EDID: one DTD 2560×1440@60 (241.5 MHz, 2720×1481 total), standard timing 1920×1080@60,
    /// CEA block with VICs 16, 95, 94 and an HDMI VSDB listing HDMI_VIC 1 (2160p30).
    fn edid() -> Vec<u8> {
        let mut e = vec![0u8; 256];
        e[..8].copy_from_slice(&[0x00, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x00]);
        e[18] = 1;
        e[19] = 4;
        for i in 0..8 {
            e[38 + 2 * i] = 0x01;
            e[39 + 2 * i] = 0x01;
        }
        e[38] = (1920 / 8 - 31) as u8;
        e[39] = 0xc0; // 16:9, 60 Hz
        // DTD 2560x1440@60: clock 241500 kHz → 24150 in 10 kHz units
        let d = &mut e[54..72];
        d[0] = (24150 & 0xff) as u8;
        d[1] = (24150 >> 8) as u8;
        d[2] = (2560 & 0xff) as u8;
        d[3] = (160 & 0xff) as u8;
        d[4] = (((2560 >> 8) as u8) << 4) | ((160 >> 8) as u8);
        d[5] = (1440 & 0xff) as u8;
        d[6] = (41 & 0xff) as u8;
        d[7] = (((1440 >> 8) as u8) << 4) | ((41 >> 8) as u8);
        e[126] = 1;
        // CEA block
        let b = &mut e[128..256];
        b[0] = 0x02;
        b[1] = 0x03;
        let mut p = 4;
        b[p] = (2 << 5) | 3;
        b[p + 1] = 16;
        b[p + 2] = 95;
        b[p + 3] = 94;
        p += 4;
        // HDMI VSDB: tag 3, len 12: 03 0c 00, phys addr 10 00, caps 00, max tmds 00, flags (byte 7) 0x20 (HDMI video present),
        // byte 8: 3D flags 0, byte 9: hdmi_vic_len=1 <<5, byte 10: HDMI_VIC 1
        let vsdb = [0x03, 0x0c, 0x00, 0x10, 0x00, 0x00, 0x00, 0x20, 0x00, 0x20, 0x01, 0x00];
        b[p] = (3 << 5) | vsdb.len() as u8;
        b[p + 1..p + 1 + vsdb.len()].copy_from_slice(&vsdb);
        p += 1 + vsdb.len();
        b[2] = p as u8;
        e
    }

    #[test]
    fn parses_dtd_standard_vic_and_hdmi_vic() {
        let caps = parse(&edid()).unwrap();
        assert!(caps.modes.contains(&(Size::new(2560, 1440), 60)), "{:?}", caps.modes);
        assert!(caps.modes.contains(&(Size::new(1920, 1080), 60)));
        assert!(caps.modes.contains(&(Size::new(3840, 2160), 30)));
        assert!(caps.modes.contains(&(Size::new(3840, 2160), 25)));
        assert_eq!(caps.modes.iter().filter(|m| **m == (Size::new(3840, 2160), 30)).count(), 1, "no duplicates");
    }

    /// A Philips 276E8V's EDID, read through the camera 2026-10-19.
    const PHILIPS: &str = "00ffffffffffff00410c8fc11b0d0000241d0103803c22782a67a1a5554da2270e5054bfef00d1c0b300950081808140\
81c0010101014dd000a0f0703e803020350055502100001aa36600a0f0701f803020350055502100001a000000fc0050\
484c203237364538560a2020000000fd0017501ea03c000a20202020202001f2020333f14c9004031f1301125d5e5f60\
6123090707830100006d030c001000387820006001020367d85dc401788003e30f000c565e00a0a0a029503020350055\
502100001e023a801871382d40582c450055502100001e011d007251d01e206e28550055502100001e4d6c80a070703e\
8030203a0055502100001a000000004e";

    fn philips_bytes() -> Vec<u8> {
        (0..PHILIPS.len() / 2).map(|i| u8::from_str_radix(&PHILIPS[2 * i..2 * i + 2], 16).unwrap()).collect()
    }

    #[test]
    fn philips_276e8v() {
        let caps = parse(&philips_bytes()).unwrap();
        let has = |w, h, r| caps.modes.contains(&(Size::new(w, h), r));
        for r in [24, 25, 30, 50, 60] {
            assert!(has(3840, 2160, r), "4K@{r} missing: {:?}", caps.modes);
        }
        assert!(has(2560, 1440, 60) && has(1920, 1080, 60) && has(1920, 1080, 50) && has(1920, 2160, 60));
        assert!(has(1280, 720, 60) && has(1280, 720, 50) && has(720, 576, 50) && has(720, 480, 60));
        // the default rule gives 2160p30 on this monitor
        use crate::hdmi::{best_timing, Monitor, Timing};
        let cands = crate::policy::tests::policy_hdmi_candidates();
        assert_eq!(best_timing(&Monitor::Present(caps), &cands, Timing::P1080_60), Some(Timing::P2160_30));
    }

    #[test]
    fn rejects_garbage() {
        assert!(parse(&[0u8; 128]).is_none());
        assert!(parse(&[0u8; 10]).is_none());
    }
}
