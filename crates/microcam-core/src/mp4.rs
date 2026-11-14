//! Fragmented MP4 (ISO BMFF) for the recorder: an init segment (`ftyp` + `moov` with the H.264 or H.265 parameter
//! sets), then one `moof` + `mdat` per GOP. Every fragment is complete on its own, so a file cut by a pulled card or a power loss
//! plays up to the last fragment written. The init segment ends in `moov/udta` holding a `free` box of [`COVER_RESERVE`]
//! bytes, which the video's cover ([`cover_box`], `udta/meta/ilst/covr`) is written over later; until then the file
//! is whole with the `free` box. Pure byte building; the recorder does the I/O.

use std::io::{Read, Seek, SeekFrom};

use crate::bmff::{self, child};
use crate::{Codec, Size};

/// The bytes the init segment reserves for the cover, the `free` box's header included: our thumbnails are about
/// 25 KB, a busy picture's more, the boxes around it under 100 B.
pub const COVER_RESERVE: usize = 48 * 1024;

/// The cover reserve's offset in an init segment of `init_len` bytes: it is the segment's last box.
pub fn cover_at(init_len: usize) -> usize {
    init_len - COVER_RESERVE
}

/// The cover written over the reserve: `meta` (an `mdir` handler, `ilst/covr/data` holding the JPEG), then a `free`
/// box over the rest — exactly [`COVER_RESERVE`] bytes; None when the JPEG does not fit.
pub fn cover_box(jpeg: &[u8]) -> Option<Vec<u8>> {
    let hdlr = full(b"hdlr", 0, 0, &cat(&[&[0u8; 4], b"mdir", b"appl", &[0u8; 8], &[0]]));
    // the data box's type 13: a JPEG; locale 0
    let data = bx(b"data", &cat(&[&13u32.to_be_bytes(), &0u32.to_be_bytes(), jpeg]));
    let meta = full(b"meta", 0, 0, &cat(&[&hdlr, &bx(b"ilst", &bx(b"covr", &data))]));
    match COVER_RESERVE.checked_sub(meta.len())? {
        0 => Some(meta),
        n if n >= 8 => Some(cat(&[&meta, &free(n)])),
        _ => None,
    }
}

/// A `free` box of `n` bytes, header included.
fn free(n: usize) -> Vec<u8> {
    bx(b"free", &vec![0u8; n - 8])
}

/// The cover JPEG of a `moov` box's body, if it carries one (`udta/meta/ilst/covr/data`).
pub fn cover(moov: &[u8]) -> Option<&[u8]> {
    let meta = child(child(moov, b"udta")?, b"meta")?;
    let data = child(child(child(meta.get(4..)?, b"ilst")?, b"covr")?, b"data")?;
    data.get(8..).filter(|j| !j.is_empty())
}

/// The longest `moov` read when looking for a cover.
const MOOV_MAX: u64 = 1 << 20;

/// The body of the file's `moov` box, reading the top-level boxes' headers until it: None when the file has none
/// before its end or it is larger than a recording's could be.
pub fn moov<R: Read + Seek>(r: &mut R) -> std::io::Result<Option<Vec<u8>>> {
    let end = r.seek(SeekFrom::End(0))?;
    let mut at = 0;
    while let Some(h) = bmff::head_at(r, at, end)? {
        if &h.kind == b"moov" {
            return bmff::body(r, &h, MOOV_MAX);
        }
        at = h.end();
    }
    Ok(None)
}

/// Media timescale: the 90 kHz video clock.
pub const TIMESCALE: u32 = 90_000;

/// What the muxer tells apart among a stream's NAL units.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Nal {
    Vps,
    Sps,
    Pps,
    /// an access unit delimiter: left out of the samples
    Aud,
    /// a picture a decoder can start at (H.264 IDR, H.265 IRAP)
    Sync,
    Other,
}

fn kind(codec: Codec, unit: &[u8]) -> Nal {
    match codec {
        Codec::H265 => match (unit[0] >> 1) & 0x3f {
            32 => Nal::Vps,
            33 => Nal::Sps,
            34 => Nal::Pps,
            35 => Nal::Aud,
            16..=21 => Nal::Sync,
            _ => Nal::Other,
        },
        _ => match unit[0] & 0x1f {
            7 => Nal::Sps,
            8 => Nal::Pps,
            9 => Nal::Aud,
            5 => Nal::Sync,
            _ => Nal::Other,
        },
    }
}

/// The NAL units of an Annex B byte stream (start codes 00 00 01 / 00 00 00 01 removed).
pub fn nals(data: &[u8]) -> Vec<&[u8]> {
    let mut starts = Vec::new();
    let mut i = 0;
    while i + 3 <= data.len() {
        if data[i] == 0 && data[i + 1] == 0 && data[i + 2] == 1 {
            starts.push(i + 3);
            i += 3;
        } else {
            i += 1;
        }
    }
    let mut out = Vec::with_capacity(starts.len());
    for (k, &s) in starts.iter().enumerate() {
        let mut e = starts.get(k + 1).map(|&n| n - 3).unwrap_or(data.len());
        // a 4-byte start code leaves its leading zero at the end of the previous unit
        while e > s && data[e - 1] == 0 && k + 1 < starts.len() {
            e -= 1;
        }
        if e > s {
            out.push(&data[s..e]);
        }
    }
    out
}

/// One encoded picture as an MP4 sample.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    /// NAL units with 4-byte big-endian lengths; parameter sets and delimiters left out (they live in the sample entry)
    pub sample: Vec<u8>,
    pub sync: bool,
    /// encoder timestamp, µs
    pub pts_us: u64,
}

/// The parameter sets a stream carries (H.264 has no VPS).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Params {
    pub vps: Option<Vec<u8>>,
    pub sps: Option<Vec<u8>>,
    pub pps: Option<Vec<u8>>,
}

impl Params {
    /// All the sample entry of `codec` needs.
    pub fn complete(&self, codec: Codec) -> bool {
        self.sps.is_some() && self.pps.is_some() && (codec != Codec::H265 || self.vps.is_some())
    }
}

/// An encoder output (Annex B) → the sample, plus the parameter sets it carries.
pub fn frame(codec: Codec, data: &[u8], pts_us: u64) -> (Frame, Params) {
    let (mut params, mut sync) = (Params::default(), false);
    let mut sample = Vec::with_capacity(data.len());
    for n in nals(data) {
        match kind(codec, n) {
            Nal::Vps => params.vps = Some(n.to_vec()),
            Nal::Sps => params.sps = Some(n.to_vec()),
            Nal::Pps => params.pps = Some(n.to_vec()),
            Nal::Aud => {}
            k => {
                sync |= k == Nal::Sync;
                sample.extend_from_slice(&(n.len() as u32).to_be_bytes());
                sample.extend_from_slice(n);
            }
        }
    }
    (Frame { sample, sync, pts_us }, params)
}

/// Builds the file: [`Muxer::init`] once, then whatever [`Muxer::push`] and [`Muxer::finish`] return, in order.
pub struct Muxer {
    size: Size,
    codec: Codec,
    /// `avcC` or `hvcC`
    config: Vec<u8>,
    /// a frame's duration when no next frame tells it (the last one of the file)
    nominal: u32,
    seq: u32,
    start_us: Option<u64>,
    pending: Vec<Frame>,
}

impl Muxer {
    /// Err when `params` lack a set `codec` needs or its SPS does not parse.
    pub fn new(size: Size, fps: u32, codec: Codec, params: &Params) -> Result<Self, String> {
        let (Some(sps), Some(pps)) = (&params.sps, &params.pps) else { return Err("no SPS or PPS".into()) };
        let config = match codec {
            Codec::H264 => bx(b"avcC", &cat(&[&[1, sps[1], sps[2], sps[3], 0xff, 0xe1], &(sps.len() as u16).to_be_bytes(), sps, &[1], &(pps.len() as u16).to_be_bytes(), pps])),
            Codec::H265 => bx(b"hvcC", &hvcc(params.vps.as_deref().ok_or("no VPS")?, sps, pps)?),
            Codec::Mjpeg => return Err("MJPEG is not muxed".into()),
        };
        Ok(Self { size, codec, config, nominal: TIMESCALE / fps.max(1), seq: 0, start_us: None, pending: Vec::new() })
    }

    /// `ftyp` + `moov`.
    pub fn init(&self) -> Vec<u8> {
        let entry: &[u8; 4] = if self.codec == Codec::H265 { b"hvc1" } else { b"avc1" };
        let ftyp = bx(b"ftyp", &cat(&[b"isom", &0x200u32.to_be_bytes(), b"isom", b"iso6", entry, b"mp41"]));
        let mvhd = full(b"mvhd", 0, 0, &cat(&[&[0u8; 8], &1000u32.to_be_bytes(), &0u32.to_be_bytes(), &0x0001_0000u32.to_be_bytes(), &0x0100u16.to_be_bytes(), &[0u8; 10], &MATRIX, &[0u8; 24], &2u32.to_be_bytes()]));
        let tkhd = full(b"tkhd", 0, 3, &cat(&[&[0u8; 8], &1u32.to_be_bytes(), &[0u8; 4], &0u32.to_be_bytes(), &[0u8; 8], &[0u8; 4], &[0u8; 4], &MATRIX, &(self.size.w << 16).to_be_bytes(), &(self.size.h << 16).to_be_bytes()]));
        let mdhd = full(b"mdhd", 0, 0, &cat(&[&[0u8; 8], &TIMESCALE.to_be_bytes(), &0u32.to_be_bytes(), &0x55c4u16.to_be_bytes(), &[0u8; 2]]));
        let hdlr = full(b"hdlr", 0, 0, &cat(&[&[0u8; 4], b"vide", &[0u8; 12], b"VideoHandler\0"]));
        let vmhd = full(b"vmhd", 0, 1, &[0u8; 8]);
        let dinf = bx(b"dinf", &full(b"dref", 0, 0, &cat(&[&1u32.to_be_bytes(), &full(b"url ", 0, 1, &[])])));
        let sample_entry = bx(entry, &cat(&[&[0u8; 6], &1u16.to_be_bytes(), &[0u8; 16], &(self.size.w as u16).to_be_bytes(), &(self.size.h as u16).to_be_bytes(), &0x0048_0000u32.to_be_bytes(), &0x0048_0000u32.to_be_bytes(), &[0u8; 4], &1u16.to_be_bytes(), &[0u8; 32], &0x0018u16.to_be_bytes(), &0xffffu16.to_be_bytes(), &self.config]));
        let stsd = full(b"stsd", 0, 0, &cat(&[&1u32.to_be_bytes(), &sample_entry]));
        let empty = 0u32.to_be_bytes();
        let stbl = bx(b"stbl", &cat(&[&stsd, &full(b"stts", 0, 0, &empty), &full(b"stsc", 0, 0, &empty), &full(b"stsz", 0, 0, &[0u8; 8]), &full(b"stco", 0, 0, &empty)]));
        let minf = bx(b"minf", &cat(&[&vmhd, &dinf, &stbl]));
        let mdia = bx(b"mdia", &cat(&[&mdhd, &hdlr, &minf]));
        let trak = bx(b"trak", &cat(&[&tkhd, &mdia]));
        let trex = full(b"trex", 0, 0, &cat(&[&1u32.to_be_bytes(), &1u32.to_be_bytes(), &[0u8; 12]]));
        let mvex = bx(b"mvex", &trex);
        // the cover's reserve last: written over in place, at `cover_at`
        let udta = bx(b"udta", &free(COVER_RESERVE));
        let moov = bx(b"moov", &cat(&[&mvhd, &trak, &mvex, &udta]));
        cat(&[&ftyp, &moov])
    }

    /// Queue a frame; a sync frame closes the fragment before it, which is returned.
    pub fn push(&mut self, f: Frame) -> Option<Vec<u8>> {
        let out = if f.sync && !self.pending.is_empty() { Some(self.fragment(Some(f.pts_us))) } else { None };
        self.start_us.get_or_insert(f.pts_us);
        self.pending.push(f);
        out
    }

    /// The last fragment, if frames are queued.
    pub fn finish(&mut self) -> Option<Vec<u8>> {
        if self.pending.is_empty() {
            None
        } else {
            Some(self.fragment(None))
        }
    }

    /// `moof` + `mdat` of the queued frames; `next_us` = the timestamp of the frame after them, if known.
    fn fragment(&mut self, next_us: Option<u64>) -> Vec<u8> {
        let frames = std::mem::take(&mut self.pending);
        self.seq += 1;
        let start = self.start_us.unwrap_or(frames[0].pts_us);
        let base = to_ticks(frames[0].pts_us.saturating_sub(start));
        let mut durations: Vec<u32> = frames.windows(2).map(|w| to_ticks(w[1].pts_us.saturating_sub(w[0].pts_us)) as u32).collect();
        let last = next_us.map(|n| to_ticks(n.saturating_sub(frames[frames.len() - 1].pts_us)) as u32).filter(|&d| d > 0);
        durations.push(last.or(durations.last().copied()).unwrap_or(self.nominal));

        let mut trun_body = cat(&[&(frames.len() as u32).to_be_bytes(), &0u32.to_be_bytes()]);
        for (f, d) in frames.iter().zip(&durations) {
            let flags: u32 = if f.sync { 0x0200_0000 } else { 0x0101_0000 };
            trun_body.extend_from_slice(&cat(&[&d.to_be_bytes(), &(f.sample.len() as u32).to_be_bytes(), &flags.to_be_bytes()]));
        }
        let build = |trun_body: &[u8]| {
            let mfhd = full(b"mfhd", 0, 0, &self.seq.to_be_bytes());
            let tfhd = full(b"tfhd", 0, 0x02_0000, &1u32.to_be_bytes());
            let tfdt = full(b"tfdt", 1, 0, &base.to_be_bytes());
            let trun = full(b"trun", 0, 0x0701, trun_body);
            bx(b"moof", &cat(&[&mfhd, &bx(b"traf", &cat(&[&tfhd, &tfdt, &trun]))]))
        };
        // data offset: from the start of moof to the first sample byte (moof size + mdat header)
        let moof_len = build(&trun_body).len() as u32;
        trun_body[4..8].copy_from_slice(&(moof_len + 8).to_be_bytes());
        let moof = build(&trun_body);
        let payload: usize = frames.iter().map(|f| f.sample.len()).sum();
        let mut out = Vec::with_capacity(moof.len() + 8 + payload);
        out.extend_from_slice(&moof);
        out.extend_from_slice(&((payload + 8) as u32).to_be_bytes());
        out.extend_from_slice(b"mdat");
        for f in &frames {
            out.extend_from_slice(&f.sample);
        }
        out
    }
}

/// The `hvcC` record (ISO/IEC 14496-15 §8.3.3): the profile, tier and level and the picture format read from the SPS,
/// then the three parameter sets, one array each.
fn hvcc(vps: &[u8], sps: &[u8], pps: &[u8]) -> Result<Vec<u8>, String> {
    let rbsp = unescape(sps.get(2..).ok_or("short SPS")?);
    let mut b = Bits { b: &rbsp, at: 0 };
    b.skip(4)?; // sps_video_parameter_set_id
    let sub_layers = b.read(3)?;
    let nesting = b.read(1)?;
    // the general profile_tier_level: the same 12 bytes the record carries
    let ptl = (0..12).map(|_| b.read(8).map(|x| x as u8)).collect::<Result<Vec<u8>, _>>()?;
    let present = (0..sub_layers).map(|_| Ok((b.read(1)?, b.read(1)?))).collect::<Result<Vec<_>, String>>()?;
    if sub_layers > 0 {
        b.skip(2 * (8 - sub_layers))?;
    }
    for (profile, level) in present {
        b.skip(88 * profile + 8 * level)?;
    }
    b.ue()?; // sps_seq_parameter_set_id
    let chroma = b.ue()?;
    if chroma == 3 {
        b.skip(1)?;
    }
    b.ue()?; // width
    b.ue()?; // height
    if b.read(1)? == 1 {
        for _ in 0..4 {
            b.ue()?; // the conformance window
        }
    }
    let (depth_luma, depth_chroma) = (b.ue()?, b.ue()?);
    let array = |t: u8, unit: &[u8]| cat(&[&[0x80 | t], &1u16.to_be_bytes(), &(unit.len() as u16).to_be_bytes(), unit]);
    Ok(cat(&[
        &[1],
        &ptl,
        &0xf000u16.to_be_bytes(),
        &[0xfc, 0xfc | chroma as u8, 0xf8 | depth_luma as u8, 0xf8 | depth_chroma as u8],
        &0u16.to_be_bytes(),
        // no constant rate, the layers the SPS counts, its nesting flag, 4-byte lengths
        &[((sub_layers as u8 + 1) << 3) | ((nesting as u8) << 2) | 3, 3],
        &array(32, vps),
        &array(33, sps),
        &array(34, pps),
    ]))
}

/// A NAL unit's payload without its emulation-prevention bytes (00 00 03 → 00 00).
fn unescape(b: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(b.len());
    let mut zeros = 0;
    for &x in b {
        if zeros >= 2 && x == 3 {
            zeros = 0;
            continue;
        }
        zeros = if x == 0 { zeros + 1 } else { 0 };
        out.push(x);
    }
    out
}

struct Bits<'a> {
    b: &'a [u8],
    at: usize,
}

impl Bits<'_> {
    fn read(&mut self, n: u32) -> Result<u32, String> {
        let mut v = 0;
        for _ in 0..n {
            let byte = self.b.get(self.at / 8).ok_or("SPS ends early")?;
            v = v << 1 | ((byte >> (7 - self.at % 8)) & 1) as u32;
            self.at += 1;
        }
        Ok(v)
    }

    fn skip(&mut self, n: u32) -> Result<(), String> {
        for _ in 0..n {
            self.read(1)?;
        }
        Ok(())
    }

    /// Exp-Golomb ue(v).
    fn ue(&mut self) -> Result<u32, String> {
        let mut zeros = 0;
        while self.read(1)? == 0 {
            zeros += 1;
            if zeros > 31 {
                return Err("bad ue(v) in the SPS".into());
            }
        }
        Ok((1u32 << zeros) - 1 + self.read(zeros)?)
    }
}

fn to_ticks(us: u64) -> u64 {
    us * TIMESCALE as u64 / 1_000_000
}

const MATRIX: [u8; 36] = [0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x40, 0, 0, 0];

fn cat(parts: &[&[u8]]) -> Vec<u8> {
    parts.concat()
}

fn bx(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
    cat(&[&((body.len() + 8) as u32).to_be_bytes(), kind, body])
}

fn full(kind: &[u8; 4], version: u8, flags: u32, body: &[u8]) -> Vec<u8> {
    let vf = (version as u32) << 24 | (flags & 0x00ff_ffff);
    bx(kind, &cat(&[&vf.to_be_bytes(), body]))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn annexb(units: &[&[u8]]) -> Vec<u8> {
        let mut v = Vec::new();
        for u in units {
            v.extend_from_slice(&[0, 0, 0, 1]);
            v.extend_from_slice(u);
        }
        v
    }

    const SPS: &[u8] = &[0x67, 0x64, 0x00, 0x33, 0xac];
    const PPS: &[u8] = &[0x68, 0xee, 0x3c, 0x80];

    #[test]
    fn an_idr_access_unit_gives_the_parameter_sets_and_a_length_prefixed_sync_sample() {
        let (f, params) = frame(Codec::H264, &annexb(&[&[0x09, 0x10], SPS, PPS, &[0x65, 1, 2, 0]]), 7);
        assert_eq!(params, Params { vps: None, sps: Some(SPS.to_vec()), pps: Some(PPS.to_vec()) });
        assert!(params.complete(Codec::H264) && !params.complete(Codec::H265));
        assert!(f.sync);
        assert_eq!(f.sample, vec![0, 0, 0, 4, 0x65, 1, 2, 0], "a trailing zero inside a unit stays");
        let (p, _) = frame(Codec::H264, &[0, 0, 1, 0x41, 9, 9], 8);
        assert!(!p.sync);
        assert_eq!(p.sample, vec![0, 0, 0, 3, 0x41, 9, 9]);
    }

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
    }

    /// A 1080p libx265 stream's parameter sets (ffmpeg 7, `testsrc`), with emulation-prevention bytes in the SPS.
    fn hevc_params() -> Params {
        Params {
            vps: Some(hex("40010c01ffff016000000300900000030000030078928090")),
            sps: Some(hex("420101016000000300900000030000030078a003c08010e5964a924caf016808000003000800000300c840")),
            pps: Some(hex("4401c172b46240")),
        }
    }

    #[test]
    fn an_h265_sample_entry_carries_the_hvcc_ffmpeg_writes_for_the_same_stream() {
        let p = hevc_params();
        let au = annexb(&[&[0x46, 0x01, 0x10], p.vps.as_deref().unwrap(), p.sps.as_deref().unwrap(), p.pps.as_deref().unwrap(), &[0x26, 0x01, 0xaf]]);
        let (f, got) = frame(Codec::H265, &au, 0);
        assert!(f.sync && got.complete(Codec::H265));
        assert_eq!(f.sample, vec![0, 0, 0, 3, 0x26, 0x01, 0xaf], "the delimiter and the parameter sets left out");
        let init = Muxer::new(Size::new(1920, 1080), 25, Codec::H265, &got).unwrap().init();
        let at = init.windows(4).position(|w| w == b"hvcC").unwrap();
        let len = u32::from_be_bytes(init[at - 4..at].try_into().unwrap()) as usize;
        // `ffmpeg -i t.hevc -c copy -tag:v hvc1 t.mp4`
        let ffmpeg = hex("01016000000090000000000078f000fcfdf8f800000f03a00001001840010c01ffff016000000300900000030000030078928090a10001002b420101016000000300900000030000030078a003c08010e5964a924caf016808000003000800000300c840a2000100074401c172b46240");
        assert_eq!(&init[at + 4..at - 4 + len], &ffmpeg[..]);
        assert!(init.windows(4).any(|w| w == b"hvc1"));
    }

    /// Walk top-level boxes: (type, size).
    fn boxes(b: &[u8]) -> Vec<(String, usize)> {
        let mut out = Vec::new();
        let mut i = 0;
        while i + 8 <= b.len() {
            let n = u32::from_be_bytes(b[i..i + 4].try_into().unwrap()) as usize;
            out.push((String::from_utf8_lossy(&b[i + 4..i + 8]).into_owned(), n));
            i += n;
        }
        assert_eq!(i, b.len(), "boxes tile the buffer");
        out
    }

    #[test]
    fn fragments_split_at_sync_frames_with_durations_from_the_timestamps() {
        let params = Params { vps: None, sps: Some(SPS.to_vec()), pps: Some(PPS.to_vec()) };
        let mut m = Muxer::new(Size::new(3840, 2160), 45, Codec::H264, &params).unwrap();
        let init = m.init();
        assert_eq!(boxes(&init).iter().map(|b| b.0.as_str()).collect::<Vec<_>>(), ["ftyp", "moov"]);
        let f = |sync: bool, us: u64| Frame { sample: vec![0, 0, 0, 1, if sync { 0x65 } else { 0x41 }], sync, pts_us: us };
        assert!(m.push(f(true, 1_000_000)).is_none());
        assert!(m.push(f(false, 1_022_222)).is_none());
        let frag = m.push(f(true, 1_044_444)).expect("a sync frame closes the fragment");
        assert_eq!(boxes(&frag).iter().map(|b| b.0.as_str()).collect::<Vec<_>>(), ["moof", "mdat"]);
        // trun: 2 samples, data offset = moof size + 8, durations 1999 and 2000 ticks (22.222 ms at 90 kHz)
        let moof_len = boxes(&frag)[0].1;
        let t = frag.windows(4).position(|w| w == b"trun").unwrap() + 8;
        let be = |o: usize| u32::from_be_bytes(frag[t + o..t + o + 4].try_into().unwrap());
        assert_eq!(be(0), 2);
        assert_eq!(be(4) as usize, moof_len + 8);
        assert_eq!((be(8), be(20)), (1999, 1999));
        assert_eq!(&frag[moof_len + 8..], &[0, 0, 0, 1, 0x65, 0, 0, 0, 1, 0x41]);
        // tfdt of the second fragment = its first frame since the start
        let last = m.finish().unwrap();
        let d = last.windows(4).position(|w| w == b"tfdt").unwrap() + 8;
        assert_eq!(u64::from_be_bytes(last[d..d + 8].try_into().unwrap()), 3999);
        assert!(m.finish().is_none());
    }

    #[test]
    fn the_init_segment_reserves_the_cover_and_takes_it_in_place() {
        let params = Params { vps: None, sps: Some(SPS.to_vec()), pps: Some(PPS.to_vec()) };
        let mut m = Muxer::new(Size::new(3840, 2160), 30, Codec::H264, &params).unwrap();
        let init = m.init();
        let at = cover_at(init.len());
        assert_eq!(&init[at + 4..at + 8], b"free");
        assert_eq!(u32::from_be_bytes(init[at..at + 4].try_into().unwrap()) as usize, COVER_RESERVE);
        // a file cut before its cover: the reserve unwritten, the boxes whole, no cover
        let mut file = init.clone();
        m.push(Frame { sample: vec![0, 0, 0, 1, 0x65], sync: true, pts_us: 0 });
        file.extend(m.finish().unwrap());
        assert_eq!(boxes(&file).iter().map(|b| b.0.as_str()).collect::<Vec<_>>(), ["ftyp", "moov", "moof", "mdat"]);
        let body = moov(&mut std::io::Cursor::new(&file)).unwrap().expect("a moov");
        assert_eq!(cover(&body), None);
        // the cover written over the reserve: the same size, the boxes whole, the JPEG read back
        let jpeg: Vec<u8> = (0..25_000u32).map(|i| (i % 251) as u8).collect();
        let c = cover_box(&jpeg).expect("it fits");
        assert_eq!(c.len(), COVER_RESERVE);
        file[at..at + COVER_RESERVE].copy_from_slice(&c);
        assert_eq!(boxes(&file).iter().map(|b| b.0.as_str()).collect::<Vec<_>>(), ["ftyp", "moov", "moof", "mdat"]);
        let body = moov(&mut std::io::Cursor::new(&file)).unwrap().unwrap();
        assert_eq!(cover(&body), Some(&jpeg[..]));
        // one that fills the reserve to the byte needs no free box after it; one longer does not fit
        let overhead = full(b"meta", 0, 0, &cat(&[&full(b"hdlr", 0, 0, &[0u8; 21]), &bx(b"ilst", &bx(b"covr", &bx(b"data", &[0u8; 8])))])).len();
        assert_eq!(cover_box(&vec![1u8; COVER_RESERVE - overhead]).map(|c| c.len()), Some(COVER_RESERVE));
        assert_eq!(cover_box(&vec![1u8; COVER_RESERVE - overhead - 3]), None, "a gap too small for a free box");
        assert_eq!(cover_box(&vec![1u8; COVER_RESERVE]), None);
    }
}
