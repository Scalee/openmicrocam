//! The media heap a built pipeline holds, per owner: microcamd's own prices (→ «Prices»).
//! [`video`] prices a [`Current`] from what it holds — the canvas pools, the SCL ports writing buffers of their own, the
//! encoders and the H.26x device. The menu's, microcamd's scratch and the fixed owners are not video's; the constants below
//! carry their origin.

use serde::{Deserialize, Serialize};

use crate::reconcile::{topo, ChnPort, Current, Op, PoolState, VdecState, VencState};
use crate::{Codec, Size};

const MIB: u64 = 1 << 20;

/// The media heap: U-Boot's `mma_heap`, behind an MMU, so the owners' sum is what an allocation meets
/// [R: `mem.rs`].
pub const HEAP: u64 = 176 * MIB;
/// Owners always there, neither video nor menu: CMDQ 3.0 MiB, ISP statistics and command queues, the logs, GE 0.6
/// [R 2026-11-10, the heap's listing]; the JPEG decoder's `interbuf` 2.0, made with the decoder's device at the first
/// picture decoded and kept after it (absent after a boot until then) [R 2026-11-14, the heap's listing; mhal-jpd.md].
pub const FIXED: u64 = 56 * MIB / 10;
/// Kept free beside a transition's peak: what the prices miss — the MMU's 256-KiB pages under the listing's bytes
/// (1.5–2.1 MiB between the heap's free bytes and its free pages, idle and beside the `edge` scene [R 2026-11-11]) and
/// what is not found yet [I, to set from a transition's peak measured in free pages].
pub const MARGIN: u64 = 6 * MIB;
/// A JPEG channel's output: two bitstream rings of [`jpeg_ring`] bytes each [R `venc-ring-mem0` at w × h: 4K 15.8,
/// 3840×1600 11.7, 1440p 7.0, 1080p 4.0, 720p 1.8 MiB].
const JPEG_RINGS: u64 = 2;
/// USB 2.0 high speed: at most three isochronous packets of 1024 bytes in a microframe, 8000 microframes a second
/// [S USB 2.0 §5.6.3]; the gadget is loaded with that most (`camera/config.toml` `max_payload`).
pub const USB_PAYLOAD: u32 = 3072;
const MICROFRAMES: u64 = 8000;
/// An H.26x channel's reference frames, fitted to the listed `ven_rec_*` (4K 13.3, 1440p 6.2, 1080p 3.7, 720p 1.8 MiB):
/// 0.5 MiB + 1.618 B a pixel, within 0.12 MiB of each [computed].
const H26X_REFS_BASE: u64 = MIB / 2;
const H26X_REFS_MILLI_PER_PX: u64 = 1618;
/// An H.26x channel's working buffers beside its references (`ven_rdo`, `ven_mv`, `ven_fcty`, `ven_fctc`, `ven_inst`,
/// `ven_work`), fitted to the recording's at 1080p, 1440p and 4K (0.79, 1.21, 2.46 MiB): 230 KiB + 0.283 B a pixel,
/// within 6 KB of each [R 2026-11-11, computed]; USB's channel taken to hold the same [I].
const H26X_WORK_BASE: u64 = 230 * 1024;
const H26X_WORK_MILLI_PER_PX: u64 = 283;
/// An H.26x channel's output ring, priced at the most seen [R `venc-ring-mem*`: USB's 4.0 MiB at 4K, 1440p and 1080p;
/// the recording's 4.0 at 4K, 1.8 at 1440p, 1.0 at 1080p].
const H26X_RING: u64 = 4 * MIB;
/// The H.26x device from its first channel on, kept after the channel goes: `mma_heap_codec` 4.0 + `vcodec_dev` 0.9 MiB
/// [R, listed with no encoder up].
const H26X_DEVICE: u64 = 49 * MIB / 10;

/// The player's decoder, fitted to the one case listed — 4K H.264, 2 reference frames, microcamd's own channel
/// [R 2026-11-11, `microcam account` while playing]: each reference a frame (`vde_rec_*` 12.0 MiB, 1.517 B a pixel) with its
/// motion vectors and compressed-frame tables (`vde_mv_*` 2.5, `vde_fcty_*` 0.3, `vde_fctc_*` 0.1 MiB: 0.367 B a pixel),
/// one scratch (`vde_tmp_0` 6.1 MiB, 0.771 B a pixel), `vde_work_0` 0.2 MiB, the ring, port 0's two frames
/// (`vdec0-out0-0` 23.7 MiB at 4K); other sizes scaled by the pixels [I].
const VDEC_REF_MILLI_PER_PX: u64 = 1517;
const VDEC_MV_MILLI_PER_PX: u64 = 367;
const VDEC_TMP_MILLI_PER_PX: u64 = 771;
const VDEC_WORK: u64 = MIB / 5;
const VDEC_PORT_FRAMES: u64 = 2;

fn decoder(v: &VdecState) -> u64 {
    let px = v.size.w as u64 * v.size.h as u64;
    let refs = crate::desired::VDEC_REF_FRAMES as u64;
    refs * px * (VDEC_REF_MILLI_PER_PX + VDEC_MV_MILLI_PER_PX) / 1000 + px * VDEC_TMP_MILLI_PER_PX / 1000 + VDEC_WORK + v.ring as u64 + VDEC_PORT_FRAMES * nv12(v.port)
}

/// What a built state holds, bytes per owner.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Account {
    pub owners: Vec<(String, u64)>,
}

impl Account {
    pub fn total(&self) -> u64 {
        self.owners.iter().map(|(_, b)| b).sum()
    }

    /// The bytes of the owners whose name starts with `prefix`.
    pub fn of(&self, prefix: &str) -> u64 {
        self.owners.iter().filter(|(n, _)| n.starts_with(prefix)).map(|(_, b)| b).sum()
    }
}

/// A transition that does not fit: its worst step, and what the heap leaves the video beside the rest.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Short {
    /// the video's bytes at the transition's worst step
    pub needs: u64,
    /// the heap less the fixed owners, the margin and `others`
    pub room: u64,
    /// what the heap holds beside the video (the menu's and microcamd's own)
    pub others: u64,
}

impl std::fmt::Display for Short {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        let m = |b: u64| b as f64 / MIB as f64;
        write!(f, "the video needs {:.1} MiB at its peak, {:.1} free for it beside the menu's {:.1}", m(self.needs), m(self.room), m(self.others))
    }
}

/// The video's bytes at the worst point of `ops` run from `cur`, the start and every step's end counted. A pool DISP
/// reads leaves the canvas DISP shows held after its `PoolDown`, until DISP shows a frame of a newer pool — after the
/// steps that build it, so to the transition's end here (the old pool «gone» 1.2 s after its `PoolDown`, after the
/// recording's pool had failed beside it [R 2026-11-11]); one canvas at a time.
pub fn peak(cur: &Current, ops: &[Op]) -> u64 {
    let mut c = cur.clone();
    let mut top = video(&c).total();
    let mut shown = 0;
    for op in ops {
        if let Op::PoolDown { id } = op {
            if let Some(p) = c.pool(*id).filter(|p| p.readers.contains(&topo::DISP_IN)) {
                shown = shown.max(canvas(p));
            }
        }
        c.apply(op);
        top = top.max(video(&c).total() + shown);
    }
    top
}

fn canvas(p: &PoolState) -> u64 {
    p.size.w as u64 * p.rows as u64 * 3 / 2
}

/// microcamd's share of the heap in state `c`: what the video, the fixed owners and the margin leave — the menu's
/// tiles and pictures, the GE's bands, the compositor's patches and the menu's RGN canvases all come out of it.
pub fn share(c: &Current) -> u64 {
    share_beside(video(c).total())
}

/// microcamd's share beside `video` bytes of the video: during a transition, beside its peak.
pub fn share_beside(video: u64) -> u64 {
    HEAP.saturating_sub(FIXED + MARGIN + video)
}

/// Whether `ops` may start from `cur` beside `others` held in the heap: its peak, or what is short.
pub fn admit(cur: &Current, ops: &[Op], others: u64) -> Result<u64, Short> {
    let needs = peak(cur, ops);
    let room = HEAP.saturating_sub(FIXED + MARGIN + others);
    if needs <= room {
        Ok(needs)
    } else {
        Err(Short { needs, room, others })
    }
}

/// Whether `ops` at no step hold more than the video holds in `cur`: they only give back, so they need nothing from
/// the heap whatever the rest holds (a stop: the player's, a recording's).
pub fn gives_back(cur: &Current, ops: &[Op]) -> bool {
    peak(cur, ops) <= video(cur).total()
}

/// The bytes of each of channel `chn`'s two JPEG rings at `size` and `fps`. A frame that finds no room in the first
/// free ring is dropped by the encoder [R]. Every channel takes w × h, h up to
/// 16 rows — the photo's frame at Q90 is one and short-lived. USB's takes three of the largest frame its stream carries
/// at `fps` (one the pump copies out, one written, one spare [I]), at least 1 MiB [I], a whole page, and never more than
/// w × h, at which it lost none [R ibid.].
pub fn jpeg_ring(chn: u32, size: Size, fps: u32) -> u64 {
    let whole = size.w as u64 * (size.h as u64).div_ceil(16) * 16;
    if chn != topo::VENC_CHN_USB || fps == 0 {
        return whole;
    }
    let frame = USB_PAYLOAD as u64 * MICROFRAMES / fps as u64;
    (3 * frame).max(MIB).next_multiple_of(4096).min(whole)
}

fn nv12(s: Size) -> u64 {
    s.w as u64 * s.h as u64 * 3 / 2
}

fn encoder(v: &VencState) -> u64 {
    let (w, h) = (v.size.w as u64, v.size.h as u64);
    match v.codec {
        Codec::Mjpeg => JPEG_RINGS * jpeg_ring(v.chn, v.size, v.fps),
        Codec::H264 | Codec::H265 => {
            H26X_REFS_BASE + w * h * H26X_REFS_MILLI_PER_PX / 1000 + H26X_WORK_BASE + w * h * H26X_WORK_MILLI_PER_PX / 1000 + H26X_RING
        }
    }
}

/// Whom a video owner serves, as a refusal for lack of resources names it ([`crate::controls::Held`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Serves {
    Hdmi,
    Usb,
    /// the media output: a recording, a photo or the camera's picture being taken
    Media,
    Player,
    /// more than one of them: the H.26x device while USB's H.26x and the recording both use it
    Shared,
}

/// The video's owners in state `c`.
pub fn video(c: &Current) -> Account {
    Account { owners: owners(c).into_iter().map(|(n, b, _)| (n, b)).collect() }
}

/// The video's bytes each output holds in state `c`, its owners counted for the one they serve alone: HDMI, USB, the
/// media output, the player.
pub fn served(c: &Current) -> [u64; 4] {
    let mut by = [0; 4];
    for (_, b, s) in owners(c) {
        match s {
            Serves::Hdmi => by[0] += b,
            Serves::Usb => by[1] += b,
            Serves::Media => by[2] += b,
            Serves::Player => by[3] += b,
            Serves::Shared => {}
        }
    }
    by
}

fn owners(c: &Current) -> Vec<(String, u64, Serves)> {
    use topo::*;
    let mut owners = Vec::new();
    for p in &c.pools {
        let (name, serves) = match p.id {
            POOL_USB => ("usb", Serves::Usb),
            POOL_MEDIA => ("media", Serves::Media),
            POOL_HDMI => ("hdmi", Serves::Hdmi),
            _ => ("?", Serves::Shared),
        };
        owners.push((format!("pool {name}"), p.n as u64 * canvas(p), serves));
    }
    // an enabled port that lends no pool its views writes buffers of its own, up to its quota: the port in front of an
    // upscaler, or one fed to the feed thread
    // a port bound into an encoder in a ring writes one frame's ring (the bind's line count is the picture's height)
    let rings: Vec<(ChnPort, Serves)> = [(&c.venc, Serves::Usb), (&c.media, Serves::Media), (&c.snap, Serves::Media)].iter().filter_map(|(v, s)| v.as_ref().and_then(|v| v.bound).map(|r| (r.port, *s))).collect();
    for (p, serves) in &rings {
        if let Some(s) = c.port_size(*p) {
            owners.push((format!("ring {}/{}/{}", p.dev, p.chn, p.port), nv12(s), *serves));
        }
    }
    for (f, serves) in [(&HDMI_FEED, Serves::Hdmi), (&USB_FEED, Serves::Usb), (&MEDIA_FEED, Serves::Media)] {
        for p in [f.out, f.up] {
            if let Some(s) = c.port_size(p).filter(|_| !c.pools.iter().any(|q| q.src == p) && !rings.iter().any(|(r, _)| *r == p)) {
                owners.push((format!("port {}/{}/{}", p.dev, p.chn, p.port), f.buffers as u64 * nv12(s), serves));
            }
        }
    }
    for (name, v, serves) in [("usb", &c.venc, Serves::Usb), ("media", &c.media, Serves::Media), ("photo", &c.snap, Serves::Media)] {
        if let Some(v) = v {
            owners.push((format!("encoder {name}"), encoder(v), serves));
        }
    }
    if let Some(v) = &c.vdec {
        owners.push(("decoder".into(), decoder(v), Serves::Player));
    }
    if c.codec_device {
        // kept after its last channel goes: it serves whichever H.26x channel is left, or none
        let h26x = |v: &Option<VencState>| v.as_ref().is_some_and(|v| v.codec != Codec::Mjpeg);
        let serves = match (h26x(&c.venc), h26x(&c.media) || h26x(&c.snap)) {
            (true, false) => Serves::Usb,
            (false, true) => Serves::Media,
            _ => Serves::Shared,
        };
        owners.push(("codec device".into(), H26X_DEVICE, serves));
    }
    owners
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::desired::{Epoch, Revision};
    use crate::formats::tests::{table as formats, H264, MJPEG};
    use crate::hdmi::Monitor;
    use crate::modes::tests::table as modes;
    use crate::policy::tests::{philips, policy};
    use crate::policy::{Inputs, Prefs, Recording};
    use crate::reconcile::{apply, plan, TraceHal};

    /// `built` with a recording of `rec` rows running as measured: HDMI paused then, the recording's frame apart
    /// from USB's (a scene, the recording at level 0, USB at 3).
    fn recording(fmt: u8, w: u32, h: u32, fps: u32, rec: u32) -> Current {
        built_with(fmt, w, h, fps, false, Some(Recording { size: Size::new(rec * 16 / 9, rec), path: "v".into() }))
    }

    fn built(fmt: u8, w: u32, h: u32, fps: u32, hdmi: bool) -> Current {
        built_with(fmt, w, h, fps, hdmi, None)
    }

    /// The state a host stream of `codec` `w`×`h` at `fps` builds from nothing, HDMI on («auto», a 4K monitor that also takes 1440p60 and 1080p60, as the measured one) or off.
    fn built_with(fmt: u8, w: u32, h: u32, fps: u32, hdmi: bool, recording: Option<Recording>) -> Current {
        let f = formats();
        let frame = f.format(fmt).unwrap().frames.iter().find(|x| (x.width, x.height) == (w, h)).unwrap().index;
        let host = f.resolve(fmt, frame, 10_000_000 / fps).unwrap();
        let frames = crate::policy::Frames { scene: recording.is_some(), ..Default::default() };
        let prefs = Prefs { hdmi_enabled: hdmi, recording, frames, ..Prefs::default() };
        let monitor = if hdmi { philips() } else { Monitor::Absent };
        let want = policy().plan(&Inputs { monitor: &monitor, host: Some((&host, Epoch(1))), prefs: &prefs, tuning: &Default::default() }, &modes(), Revision(1)).unwrap();
        let mut cur = Current::default();
        let p = plan(&cur, &want, &modes()).unwrap();
        apply(&mut TraceHal::default(), &mut cur, &want, &p.ops).unwrap();
        cur
    }

    fn mib(b: u64) -> f64 {
        b as f64 / MIB as f64
    }

    /// The measured cases of 2026-11-10 with no recording, MiB as the heap listed them: canvases
    /// (`microcam_canvas`), a port's own buffers (`scl0-out*`), the USB encoder (`venc-ring-mem0` for JPEG; `ven_rec_0_0` +
    /// its 4.0 ring + the working buffers the recording's channel listed 2026-11-11 for H.26x). The listing's lines are
    /// cut, so an owner it does not show is not compared. USB's JPEG rings are 9e.2's (`jpeg_ring`), not the listing's
    /// w × h.
    #[test]
    fn video_matches_the_heap_listing_of_the_priced_cases() {
        // (format, w, h, fps, hdmi, canvases, port, encoder)
        let rows: &[(u8, u32, u32, u32, bool, f64, Option<f64>, f64)] = &[
            (MJPEG, 3840, 2160, 30, true, 59.8, None, 4.7),
            (MJPEG, 3840, 2160, 30, false, 35.9, None, 4.7),
            (MJPEG, 1920, 1080, 30, true, 68.7, None, 4.0),
            (MJPEG, 1920, 1080, 120, true, 35.3, Some(5.9), 2.0),
            (MJPEG, 1920, 1080, 30, false, 9.0, None, 4.0),
            (MJPEG, 2560, 1440, 30, true, 75.6, None, 4.7),
            // 4 canvases since the count is computed (1440 takes the 4K hold), the listing saw 3 × 5.3
            (MJPEG, 2560, 1440, 66, false, 21.1, None, 2.1),
            // 4 canvases since the count is computed (60 a second at the 4K hold), the listing saw 3 × 8.8
            (MJPEG, 3840, 1600, 60, true, 35.2, None, 2.3),
            (MJPEG, 1280, 720, 30, false, 4.0, None, 1.8),
        ];
        for &(fmt, w, h, fps, hdmi, canvases, port, enc) in rows {
            let a = video(&built(fmt, w, h, fps, hdmi));
            let case = format!("{fmt} {w}x{h}@{fps} hdmi {hdmi}: {a:?}");
            assert!((mib(a.of("pool")) - canvases).abs() < 1.0, "canvases {case}");
            assert!((mib(a.of("encoder usb")) - enc).abs() < 1.0, "encoder {case}");
            // a port is priced at its quota; the listing saw what it held then (2 of 3 buffers)
            let ports = mib(a.of("port"));
            match port {
                Some(held) => assert!(ports >= held - 0.1 && ports <= held * 1.5 + 0.1, "port {case}"),
                None => assert_eq!(ports, 0.0, "port {case}"),
            }
            assert_eq!(a.of("codec device") > 0, fmt != MJPEG, "device {case}");
        }
    }

    /// USB's H.264 from its port in a ring, as the heap listed it 2026-11-11 at 4K30 beside HDMI 2160p30
    /// (`RingHeapAlloc` 11.9, `ven_rec_0_0` 13.3 + `venc-ring-mem0` 4.0 + the working buffers, HDMI's pool 59.8); the
    /// earlier rows of H.264 (canvases) describe a way USB's H.26x no longer takes beside nothing else.
    /// Each owner is counted once, for the output it serves: HDMI's pool to HDMI, USB's pool and encoder to USB, the
    /// recording's to the media output.
    #[test]
    fn the_video_is_counted_by_the_output_it_serves() {
        for c in [built(MJPEG, 3840, 2160, 30, true), recording(MJPEG, 3840, 2160, 30, 2160)] {
            let [hdmi, usb, media, player] = served(&c);
            let a = video(&c);
            assert_eq!(hdmi + usb + media + player, a.total(), "{a:?}");
            assert!(hdmi >= a.of("pool hdmi") && usb >= a.of("pool usb") + a.of("encoder usb") && media >= a.of("pool media") + a.of("encoder media"), "{a:?}");
            assert_eq!(player, 0);
        }
        assert!(served(&recording(MJPEG, 3840, 2160, 30, 2160))[2] > 0);
    }

    #[test]
    fn video_matches_the_heap_listing_of_a_ring() {
        let a = video(&built(H264, 3840, 2160, 30, true));
        assert!((mib(a.of("pool")) - 59.8).abs() < 0.5, "{a:?}");
        assert!((mib(a.of("ring")) - 11.9).abs() < 0.1, "{a:?}");
        assert!((mib(a.of("encoder usb")) - (13.3 + 4.0 + 2.46)).abs() < 1.0, "{a:?}");
        assert_eq!(a.of("port"), 0, "{a:?}");
        let a = video(&built(H264, 1920, 1080, 30, false));
        assert_eq!((a.of("pool"), mib(a.of("ring")).round()), (0, 3.0), "{a:?}");
    }

    /// The recording rows of the same listing (HDMI paused beside a recording): canvases (USB's pool and the
    /// recording's), the USB encoder (JPEG rings 9e.2's) and the recording's H.264 (`ven_rec_*`, its 4.0 ring, its
    /// working buffers as listed 2026-11-11).
    #[test]
    fn video_matches_the_heap_listing_with_a_recording() {
        // (format, w, h, fps, recording rows, canvases, usb encoder, media encoder)
        let rows: &[(u8, u32, u32, u32, u32, f64, f64, f64)] = &[
            // the recording's pool 3 canvases since the count is computed, the listing saw 4 × 3.0
            (MJPEG, 1920, 1080, 120, 1080, 17.9, 2.0, 3.7 + 4.0 + 0.79),
            (MJPEG, 3840, 2160, 45, 2160, 83.7, 5.6, 13.3 + 4.0 + 2.46),
            // USB's pool 3 canvases since the count is computed (the H.26x budget leaves it 7 a second), the listing saw 4
            (H264, 3840, 2160, 45, 2160, 83.7, 13.3 + 4.0 + 2.46, 13.3 + 4.0 + 2.46),
            // 3 + 3 canvases since the count is computed (the 1080 holds), the listing saw 4 + 4
            (H264, 1280, 720, 30, 1080, 13.0, 1.8 + 4.0 + 0.47, 3.7 + 4.0 + 0.79),
        ];
        for &(fmt, w, h, fps, rec, canvases, usb, media) in rows {
            let a = video(&recording(fmt, w, h, fps, rec));
            let case = format!("{fmt} {w}x{h}@{fps} rec {rec}: {a:?}");
            assert!((mib(a.of("pool")) - canvases).abs() < 1.0, "canvases {case}");
            assert!((mib(a.of("encoder usb")) - usb).abs() < 1.0, "usb {case}");
            assert!((mib(a.of("encoder media")) - media).abs() < 1.0, "media {case}");
        }
    }

    /// USB 4K25 MJPEG beside a 4K25 recording and a UHD monitor, no scene: one frame for all three; a menu holding
    /// more than what the video leaves is refused before any step, one holding less is let through.
    #[test]
    fn a_transition_over_the_heap_is_refused_before_its_first_step() {
        let f = formats();
        let host = f.resolve(MJPEG, 1, 400_000).unwrap();
        let prefs = Prefs { recording: Some(Recording { size: Size::new(3840, 2160), path: "v".into() }), ..Prefs::default() };
        let want = policy().plan(&Inputs { monitor: &philips(), host: Some((&host, Epoch(1))), prefs: &prefs, tuning: &Default::default() }, &modes(), Revision(1)).unwrap();
        let cur = Current::default();
        let ops = plan(&cur, &want, &modes()).unwrap().ops;
        let needs = peak(&cur, &ops);
        // HDMI's pool of 5 at 4K read by DISP and both encoders, USB's JPEG rings at 25 fps, the H.264 references,
        // working buffers and ring, the device
        assert!((mib(needs) - (5.0 * 11.95 + 5.6 + 19.8 + 4.9)).abs() < 1.0, "{}", mib(needs));
        let room = HEAP - FIXED - MARGIN - needs;
        assert_eq!(admit(&cur, &ops, room), Ok(needs));
        let short = admit(&cur, &ops, room + MIB).unwrap_err();
        assert_eq!((short.needs, short.room), (needs, needs - MIB));
        // microcamd's share during the steps is what admission leaves it: all it held then fits, nothing more; the
        // built state leaves it at least that
        assert_eq!(share_beside(needs), room);
        let mut after = cur.clone();
        ops.iter().for_each(|o| after.apply(o));
        assert!(share(&after) >= share_beside(needs));
    }

    /// HDMI's pool taken down and built again as it was (a rebuild under a UHD monitor): DISP keeps showing a canvas of
    /// the old one through the transition, so the peak is the start and that canvas; the built state does not keep it.
    #[test]
    fn a_rebuild_counts_the_canvas_disp_still_shows() {
        let cur = built(MJPEG, 3840, 2160, 30, true);
        let p = cur.pool(topo::POOL_HDMI).unwrap().clone();
        assert!(p.readers.contains(&topo::DISP_IN));
        let ops = [
            Op::PoolDown { id: p.id },
            Op::PoolUp { id: p.id, src: p.src, readers: p.readers.clone(), size: p.size, rows: p.rows, n: p.n },
        ];
        assert_eq!(peak(&cur, &ops), video(&cur).total() + 3840 * 2176 * 3 / 2);
        assert_eq!(peak(&cur, &ops[1..1]), video(&cur).total());
    }

    /// Taking everything down never holds more than the start: each pool's port goes before its pool, so no port takes
    /// buffers of its own between the two (USB 4K45 and a 4K recording, each on a pool of its own).
    #[test]
    fn a_teardown_peaks_at_its_start() {
        let cur = recording(MJPEG, 3840, 2160, 45, 2160);
        assert!(cur.pool(topo::POOL_USB).is_some() && cur.pool(topo::POOL_MEDIA).is_some());
        let mut ops = Vec::new();
        crate::reconcile::teardown_capture(&cur, &mut ops);
        assert_eq!(peak(&cur, &ops), video(&cur).total());
    }

    /// Stopping the player or a recording never holds more than the state it leaves, so it gives back whatever the
    /// menu holds: the decoder, the recording's encoder, goes before the live path comes back. HDMI 2160p30 beside USB MJPEG
    /// 4K45, as on the camera when `play-stop` was refused (2026-11-13).
    #[test]
    fn a_stop_peaks_no_higher_than_the_state_it_leaves() {
        let f = formats();
        let frame = f.format(MJPEG).unwrap().frames.iter().find(|x| (x.width, x.height) == (3840, 2160)).unwrap().index;
        let host = f.resolve(MJPEG, frame, 10_000_000 / 45).unwrap();
        let want = |prefs: &Prefs, rev| policy().plan(&Inputs { monitor: &philips(), host: Some((&host, Epoch(1))), prefs, tuning: &Default::default() }, &modes(), Revision(rev)).unwrap();
        let run = |cur: &mut Current, want: &crate::desired::Desired| {
            let ops = plan(cur, want, &modes()).unwrap().ops;
            let before = cur.clone();
            apply(&mut TraceHal::default(), cur, want, &ops).unwrap();
            (before, ops)
        };
        let base = Prefs::default();
        let player = crate::policy::Playback { path: "/mnt/VIDEO/V00153.mp4".into(), codec: Codec::H264, size: Size::new(3840, 2160), fps: 30, largest: 720_000 };
        let mut cur = Current::default();
        run(&mut cur, &want(&base, 1));
        assert_eq!(cur.hdmi.as_ref().map(|h| h.timing), Some(crate::hdmi::Timing::P2160_30));
        run(&mut cur, &want(&Prefs { player: Some(player), ..base.clone() }, 2));
        assert!(cur.vdec.is_some());
        let (before, ops) = run(&mut cur, &want(&base, 3));
        let first_down = ops.iter().position(|o| matches!(o, Op::VdecDown)).unwrap();
        assert!(!ops[..first_down].iter().any(|o| matches!(o, Op::PoolUp { .. } | Op::SclEnablePort { .. })), "the decoder goes first: {ops:?}");
        assert_eq!(peak(&before, &ops), video(&before).total(), "play-stop peaks at {:.1} MiB from {:.1}, leaves {:.1}", mib(peak(&before, &ops)), mib(video(&before).total()), mib(video(&cur).total()));
        assert!(gives_back(&before, &ops));
        // a recording beside them (a scene: its frame apart), stopped
        let frames = crate::policy::Frames { scene: true, ..Default::default() };
        let rec = Prefs { recording: Some(Recording { size: Size::new(3840, 2160), path: "v".into() }), frames: frames.clone(), ..base.clone() };
        run(&mut cur, &want(&rec, 4));
        assert!(cur.media.is_some());
        let (before, ops) = run(&mut cur, &want(&Prefs { frames, ..base }, 5));
        assert!(cur.media.is_none());
        assert_eq!(peak(&before, &ops), video(&before).total(), "record-stop peaks at {:.1} MiB from {:.1}, leaves {:.1}: {ops:?}", mib(peak(&before, &ops)), mib(video(&before).total()), mib(video(&cur).total()));
        assert!(gives_back(&before, &ops));
    }

    /// Entering the gallery — the photo over the whole frame and five thumbnails in a strip over it, on HDMI 2160p30 and
    /// USB MJPEG 4K45 — from the live view: one transition that covers both outputs and only gives video memory back, so
    /// it is admitted beside whatever microcamd holds within its share.
    #[test]
    fn entering_a_covered_gallery_from_the_live_view_only_gives_back() {
        use crate::policy::{Frames, Layout};
        use crate::Rect;
        let f = formats();
        let frame = f.format(MJPEG).unwrap().frames.iter().find(|x| (x.width, x.height) == (3840, 2160)).unwrap().index;
        let host = f.resolve(MJPEG, frame, 10_000_000 / 45).unwrap();
        let want = |prefs: &Prefs, rev| policy().plan(&Inputs { monitor: &philips(), host: Some((&host, Epoch(1))), prefs, tuning: &Default::default() }, &modes(), Revision(rev)).unwrap();
        let uhd = Size::new(3840, 2160);
        let live = Prefs { frames: Frames { scene: true, ..Default::default() }, ..Prefs::default() };
        let mut cur = Current::default();
        let d = want(&live, 1);
        let ops = plan(&cur, &d, &modes()).unwrap().ops;
        apply(&mut TraceHal::default(), &mut cur, &d, &ops).unwrap();
        assert_eq!(cur.hdmi.as_ref().map(|h| h.timing), Some(crate::hdmi::Timing::P2160_30));
        let strip = (0..5).map(|k| Rect { x: 128 + k * 736, y: 1780, w: 640, h: 360 });
        let gallery = Layout { raster: uhd, live: Rect::full(uhd), windows: std::iter::once(Rect::full(uhd)).chain(strip).collect() };
        let frames = Frames { hdmi: Some(gallery.clone()), usb: Some(gallery), media: None, scene: true };
        let d = want(&Prefs { frames, ..live }, 2);
        let ops = plan(&cur, &d, &modes()).unwrap().ops;
        let before = video(&cur).total();
        assert!(peak(&cur, &ops) <= before, "entering peaks at {:.1} MiB from {:.1}: {ops:?}", mib(peak(&cur, &ops)), mib(before));
        apply(&mut TraceHal::default(), &mut cur, &d, &ops).unwrap();
        assert!(video(&cur).total() < before, "covered: canvases given back");
    }

    /// USB's rings by the largest frame its stream carries, the floor and the cap; every other channel at w × h.
    #[test]
    fn usb_jpeg_rings_follow_the_frame_the_stream_carries() {
        use topo::{VENC_CHN_SNAP, VENC_CHN_USB};
        let uhd = Size::new(3840, 2160);
        assert_eq!(jpeg_ring(VENC_CHN_USB, uhd, 30), 2_457_600);
        assert_eq!(jpeg_ring(VENC_CHN_USB, uhd, 45), 1_638_400);
        assert_eq!(jpeg_ring(VENC_CHN_USB, Size::new(1920, 1080), 120), MIB);
        assert_eq!(jpeg_ring(VENC_CHN_USB, Size::new(1920, 1080), 30), 1920 * 1088);
        assert_eq!(jpeg_ring(VENC_CHN_USB, Size::new(2560, 1440), 66) % 4096, 0);
        assert_eq!(jpeg_ring(VENC_CHN_SNAP, uhd, 30), 3840 * 2160);
    }
}
