//! The player's MP4 reader: the first video track's samples — where each lies in the file, its size, whether a
//! decoder can start at it, its time — and the parameter sets, from the boxes alone: `moov`/`stbl` of a classic file,
//! every `moof`/`trun` of a fragmented one (the recorder's, `crate::mp4`), up to where a cut file ends. Then one sample
//! at a time made into what the decoder takes (`annexb`): start codes for the length prefixes, the parameter sets before
//! a sync sample.

use std::io::{self, Read, Seek, SeekFrom};

use crate::bmff::{self, boxes, child, path};
use crate::{Codec, Size};

/// One sample of the track.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sample {
    pub offset: u64,
    pub size: u32,
    pub sync: bool,
    /// decode time in the track's timescale
    pub time: u64,
}

/// The video track of a file, as the player needs it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Track {
    pub codec: Codec,
    /// the picture size the sample entry names
    pub size: Size,
    pub timescale: u32,
    /// end of the last sample, in the timescale
    pub duration: u64,
    /// the bytes before each NAL unit's length (1, 2 or 4)
    pub length_size: u8,
    /// the parameter sets (SPS, PPS; VPS first for H.265), each without a start code
    pub params: Vec<Vec<u8>>,
    pub samples: Vec<Sample>,
}

impl Track {
    /// Frames a second, from the mean sample duration (at least 1).
    pub fn fps(&self) -> u32 {
        if self.samples.is_empty() || self.duration == 0 {
            return 1;
        }
        ((self.samples.len() as u64 * self.timescale as u64 + self.duration / 2) / self.duration).max(1) as u32
    }

    /// A sample's time in milliseconds.
    pub fn ms(&self, time: u64) -> u64 {
        time * 1000 / self.timescale.max(1) as u64
    }

    /// The sync sample at or before `ms` (the first sample when none is).
    pub fn sync_at(&self, ms: u64) -> usize {
        let t = ms * self.timescale as u64 / 1000;
        self.samples.iter().enumerate().filter(|(_, s)| s.sync && s.time <= t).map(|(i, _)| i).last().unwrap_or(0)
    }

    /// The largest sample's bytes once made into what the decoder takes.
    pub fn largest(&self) -> usize {
        self.samples.iter().map(|s| s.size as usize).max().unwrap_or(0) + self.params_len()
    }

    fn params_len(&self) -> usize {
        self.params.iter().map(|p| p.len() + 4).sum()
    }

    /// Sample `s` (`raw`, as read from the file) made into an Annex B access unit in `out`: every length prefix a start
    /// code, the parameter sets before a sync sample.
    pub fn annexb(&self, s: &Sample, raw: &[u8], out: &mut Vec<u8>) -> Result<(), String> {
        out.clear();
        if s.sync {
            for p in &self.params {
                out.extend_from_slice(&[0, 0, 0, 1]);
                out.extend_from_slice(p);
            }
        }
        let n = self.length_size as usize;
        let mut i = 0;
        while i < raw.len() {
            if i + n > raw.len() {
                return Err(format!("a NAL unit's length cut at byte {i} of {}", raw.len()));
            }
            let len = raw[i..i + n].iter().fold(0usize, |a, &b| a << 8 | b as usize);
            i += n;
            if i + len > raw.len() {
                return Err(format!("a NAL unit of {len} bytes at byte {i} of {}", raw.len()));
            }
            out.extend_from_slice(&[0, 0, 0, 1]);
            out.extend_from_slice(&raw[i..i + len]);
            i += len;
        }
        Ok(())
    }
}

/// The first video track of the file `f`.
pub fn read<F: Read + Seek>(f: &mut F) -> Result<Track, String> {
    let end = f.seek(SeekFrom::End(0)).map_err(io_err)?;
    let mut moov = None;
    let mut moofs = Vec::new();
    for h in bmff::heads(f, end).map_err(io_err)? {
        match &h.kind {
            b"moov" => moov = Some(load(f, &h)?),
            b"moof" => moofs.push((h.at, load(f, &h)?)),
            _ => {}
        }
    }
    let moov = moov.ok_or("no moov box")?;
    let mut track = None;
    for (kind, trak) in boxes(&moov) {
        if &kind == b"trak" {
            if let Some(t) = video_trak(trak)? {
                track = Some(t);
                break;
            }
        }
    }
    let (mut t, id, trex) = track.ok_or("no video track")?;
    if t.samples.is_empty() {
        let trex = trex_of(&moov, id).or(trex);
        let mut time = 0u64;
        for (at, moof) in &moofs {
            fragment(moof, *at, id, trex, &mut time, &mut t.samples)?;
        }
        t.duration = time;
    }
    // a file cut inside a fragment: the samples before the cut
    if let Some(k) = t.samples.iter().position(|s| s.offset + s.size as u64 > end) {
        t.duration = t.samples[k].time;
        t.samples.truncate(k);
    }
    if t.samples.is_empty() {
        return Err("the video track has no samples".into());
    }
    if t.params.is_empty() {
        return Err("no parameter sets in the sample entry".into());
    }
    Ok(t)
}

fn io_err(e: io::Error) -> String {
    e.to_string()
}

fn load<F: Read + Seek>(f: &mut F, h: &bmff::Head) -> Result<Vec<u8>, String> {
    bmff::body(f, h, 64 << 20).map_err(io_err)?.ok_or_else(|| format!("a box of {} bytes", h.body_len()))
}

struct Rd<'a> {
    b: &'a [u8],
    i: usize,
}

impl<'a> Rd<'a> {
    fn new(b: &'a [u8]) -> Self {
        Self { b, i: 0 }
    }
    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        let s = self.b.get(self.i..self.i + n).ok_or("a box shorter than its fields")?;
        self.i += n;
        Ok(s)
    }
    fn u8(&mut self) -> Result<u8, String> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16, String> {
        Ok(u16::from_be_bytes(self.take(2)?.try_into().unwrap()))
    }
    fn u32(&mut self) -> Result<u32, String> {
        Ok(u32::from_be_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn u64(&mut self) -> Result<u64, String> {
        Ok(u64::from_be_bytes(self.take(8)?.try_into().unwrap()))
    }
    /// version and flags of a full box
    fn full(&mut self) -> Result<(u8, u32), String> {
        let v = self.u32()?;
        Ok(((v >> 24) as u8, v & 0x00ff_ffff))
    }
}

/// Defaults of a fragmented track (`trex`): sample duration, size, flags.
#[derive(Clone, Copy, Default)]
struct Trex {
    duration: u32,
    size: u32,
    flags: u32,
}

/// A video `trak`: the track (samples filled when the file is classic), its id, and none for `trex`.
fn video_trak(trak: &[u8]) -> Result<Option<(Track, u32, Option<Trex>)>, String> {
    let Some(hdlr) = path(trak, &[b"mdia", b"hdlr"]) else { return Ok(None) };
    if hdlr.get(8..12) != Some(b"vide") {
        return Ok(None);
    }
    let tkhd = child(trak, b"tkhd").ok_or("no tkhd")?;
    let mut r = Rd::new(tkhd);
    let (v, _) = r.full()?;
    r.take(if v == 1 { 16 } else { 8 })?;
    let id = r.u32()?;
    let mdhd = path(trak, &[b"mdia", b"mdhd"]).ok_or("no mdhd")?;
    let mut r = Rd::new(mdhd);
    let (v, _) = r.full()?;
    r.take(if v == 1 { 16 } else { 8 })?;
    let timescale = r.u32()?;
    let stbl = path(trak, &[b"mdia", b"minf", b"stbl"]).ok_or("no stbl")?;
    let stsd = child(stbl, b"stsd").ok_or("no stsd")?;
    let mut r = Rd::new(stsd);
    r.full()?;
    if r.u32()? == 0 {
        return Err("no sample entry".into());
    }
    let entry = &stsd[8..];
    let size = u32::from_be_bytes(entry.get(0..4).ok_or("short stsd")?.try_into().unwrap()) as usize;
    let kind: [u8; 4] = entry.get(4..8).ok_or("short stsd")?.try_into().unwrap();
    let entry = entry.get(8..size).ok_or("short sample entry")?;
    let codec = match &kind {
        b"avc1" | b"avc3" => Codec::H264,
        b"hvc1" | b"hev1" => Codec::H265,
        k => return Err(format!("the video is {}, not H.264 or H.265", String::from_utf8_lossy(k))),
    };
    // VisualSampleEntry: 6 reserved + 2 index, 16 predefined, width, height, … 78 bytes before its boxes
    let w = u16::from_be_bytes(entry.get(24..26).ok_or("short sample entry")?.try_into().unwrap()) as u32;
    let h = u16::from_be_bytes(entry.get(26..28).ok_or("short sample entry")?.try_into().unwrap()) as u32;
    let inner = entry.get(78..).ok_or("short sample entry")?;
    let (length_size, params) = match codec {
        Codec::H265 => hvcc(child(inner, b"hvcC").ok_or("no hvcC")?)?,
        _ => avcc(child(inner, b"avcC").ok_or("no avcC")?)?,
    };
    let mut t = Track { codec, size: Size::new(w, h), timescale, duration: 0, length_size, params, samples: Vec::new() };
    classic(stbl, &mut t)?;
    Ok(Some((t, id, None)))
}

fn avcc(b: &[u8]) -> Result<(u8, Vec<Vec<u8>>), String> {
    let mut r = Rd::new(b);
    r.take(4)?;
    let length_size = (r.u8()? & 3) + 1;
    let mut params = Vec::new();
    for _ in 0..2 {
        let n = r.u8()? & if params.is_empty() { 31 } else { 255 };
        for _ in 0..n {
            let len = r.u16()? as usize;
            params.push(r.take(len)?.to_vec());
        }
    }
    Ok((length_size, params))
}

fn hvcc(b: &[u8]) -> Result<(u8, Vec<Vec<u8>>), String> {
    let mut r = Rd::new(b);
    r.take(21)?;
    let length_size = (r.u8()? & 3) + 1;
    let arrays = r.u8()?;
    let mut params = Vec::new();
    for _ in 0..arrays {
        r.u8()?;
        let n = r.u16()?;
        for _ in 0..n {
            let len = r.u16()? as usize;
            params.push(r.take(len)?.to_vec());
        }
    }
    Ok((length_size, params))
}

/// The samples of a classic `stbl` (none when its tables are empty: a fragmented file).
fn classic(stbl: &[u8], t: &mut Track) -> Result<(), String> {
    let table = |kind: &[u8; 4]| child(stbl, kind);
    let sizes: Vec<u32> = match table(b"stsz") {
        Some(b) => {
            let mut r = Rd::new(b);
            r.full()?;
            let fixed = r.u32()?;
            let n = r.u32()?;
            (0..n).map(|_| if fixed != 0 { Ok(fixed) } else { r.u32() }).collect::<Result<_, _>>()?
        }
        None => return Ok(()),
    };
    if sizes.is_empty() {
        return Ok(());
    }
    let chunks: Vec<u64> = if let Some(b) = table(b"stco") {
        let mut r = Rd::new(b);
        r.full()?;
        let n = r.u32()?;
        (0..n).map(|_| r.u32().map(|v| v as u64)).collect::<Result<_, _>>()?
    } else if let Some(b) = table(b"co64") {
        let mut r = Rd::new(b);
        r.full()?;
        let n = r.u32()?;
        (0..n).map(|_| r.u64()).collect::<Result<_, _>>()?
    } else {
        return Err("no stco".into());
    };
    let stsc: Vec<(u32, u32)> = {
        let mut r = Rd::new(table(b"stsc").ok_or("no stsc")?);
        r.full()?;
        let n = r.u32()?;
        (0..n).map(|_| -> Result<(u32, u32), String> { let first = r.u32()?; let per = r.u32()?; r.u32()?; Ok((first, per)) }).collect::<Result<_, _>>()?
    };
    let mut durations = Vec::with_capacity(sizes.len());
    {
        let mut r = Rd::new(table(b"stts").ok_or("no stts")?);
        r.full()?;
        let n = r.u32()?;
        for _ in 0..n {
            let count = r.u32()?;
            let d = r.u32()?;
            durations.extend(std::iter::repeat_n(d, count as usize));
        }
    }
    let syncs: Option<Vec<u32>> = match table(b"stss") {
        Some(b) => {
            let mut r = Rd::new(b);
            r.full()?;
            let n = r.u32()?;
            Some((0..n).map(|_| r.u32()).collect::<Result<_, _>>()?)
        }
        None => None,
    };
    let mut k = 0usize;
    let mut time = 0u64;
    for (c, &chunk) in chunks.iter().enumerate() {
        let c1 = c as u32 + 1;
        let per = stsc.iter().rev().find(|(first, _)| *first <= c1).map(|(_, p)| *p).unwrap_or(0);
        let mut off = chunk;
        for _ in 0..per {
            let Some(&size) = sizes.get(k) else { break };
            let sync = syncs.as_ref().is_none_or(|s| s.binary_search(&(k as u32 + 1)).is_ok());
            t.samples.push(Sample { offset: off, size, sync, time });
            time += *durations.get(k).unwrap_or(&0) as u64;
            off += size as u64;
            k += 1;
        }
    }
    t.duration = time;
    Ok(())
}

fn trex_of(moov: &[u8], id: u32) -> Option<Trex> {
    let mvex = child(moov, b"mvex")?;
    for (kind, b) in boxes(mvex) {
        if &kind == b"trex" {
            let mut r = Rd::new(b);
            r.full().ok()?;
            if r.u32().ok()? == id {
                r.u32().ok()?;
                return Some(Trex { duration: r.u32().ok()?, size: r.u32().ok()?, flags: r.u32().ok()? });
            }
        }
    }
    None
}

/// The samples of one `moof` (at `at` in the file) for track `id`.
fn fragment(moof: &[u8], at: u64, id: u32, trex: Option<Trex>, time: &mut u64, out: &mut Vec<Sample>) -> Result<(), String> {
    let trex = trex.unwrap_or_default();
    for (kind, traf) in boxes(moof) {
        if &kind != b"traf" {
            continue;
        }
        let mut r = Rd::new(child(traf, b"tfhd").ok_or("no tfhd")?);
        let (_, f) = r.full()?;
        if r.u32()? != id {
            continue;
        }
        let base = if f & 0x1 != 0 { r.u64()? } else { at };
        if f & 0x2 != 0 {
            r.u32()?;
        }
        let d_dur = if f & 0x8 != 0 { r.u32()? } else { trex.duration };
        let d_size = if f & 0x10 != 0 { r.u32()? } else { trex.size };
        let d_flags = if f & 0x20 != 0 { r.u32()? } else { trex.flags };
        if let Some(b) = child(traf, b"tfdt") {
            let mut r = Rd::new(b);
            let (v, _) = r.full()?;
            *time = if v == 1 { r.u64()? } else { r.u32()? as u64 };
        }
        for (kind, b) in boxes(traf) {
            if &kind != b"trun" {
                continue;
            }
            let mut r = Rd::new(b);
            let (_, tf) = r.full()?;
            let n = r.u32()?;
            let mut off = base;
            if tf & 0x1 != 0 {
                off = (base as i64 + r.u32()? as i32 as i64) as u64;
            }
            let first_flags = if tf & 0x4 != 0 { Some(r.u32()?) } else { None };
            for k in 0..n {
                let d = if tf & 0x100 != 0 { r.u32()? } else { d_dur };
                let size = if tf & 0x200 != 0 { r.u32()? } else { d_size };
                let flags = if tf & 0x400 != 0 { r.u32()? } else if k == 0 { first_flags.unwrap_or(d_flags) } else { d_flags };
                if tf & 0x800 != 0 {
                    r.u32()?;
                }
                out.push(Sample { offset: off, size, sync: flags & 0x0001_0000 == 0, time: *time });
                off += size as u64;
                *time += d as u64;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mp4;

    fn annexb(units: &[&[u8]]) -> Vec<u8> {
        units.iter().flat_map(|u| [&[0u8, 0, 0, 1][..], u].concat()).collect()
    }

    /// The recorder's own file: init segment and two GOPs of three frames at 25 fps.
    fn recorded() -> Vec<u8> {
        let sps: &[u8] = &[0x67, 0x4d, 0x00, 0x33, 0xaa];
        let pps: &[u8] = &[0x68, 0xee, 0x3c];
        let params = mp4::Params { vps: None, sps: Some(sps.to_vec()), pps: Some(pps.to_vec()) };
        let mut w = mp4::Muxer::new(Size::new(3840, 2160), 25, Codec::H264, &params).unwrap();
        let mut file = w.init();
        for i in 0..6u64 {
            let unit: Vec<u8> = if i % 3 == 0 { [&[0x65u8][..], &[i as u8; 10]].concat() } else { [&[0x41u8][..], &[i as u8; 7]].concat() };
            let au = if i % 3 == 0 { annexb(&[sps, pps, &unit]) } else { annexb(&[&unit]) };
            let (f, _) = mp4::frame(Codec::H264, &au, i * 40_000);
            if let Some(b) = w.push(f) {
                file.extend(b);
            }
        }
        file.extend(w.finish().unwrap());
        file
    }

    #[test]
    fn the_recorders_fragments_give_every_sample_and_the_parameter_sets() {
        let file = recorded();
        let t = read(&mut io::Cursor::new(&file)).unwrap();
        assert_eq!((t.codec, t.size, t.length_size), (Codec::H264, Size::new(3840, 2160), 4));
        assert_eq!(t.params, vec![vec![0x67, 0x4d, 0x00, 0x33, 0xaa], vec![0x68, 0xee, 0x3c]]);
        assert_eq!(t.samples.len(), 6);
        assert_eq!(t.samples.iter().map(|s| s.sync).collect::<Vec<_>>(), [true, false, false, true, false, false]);
        assert_eq!(t.fps(), 25);
        assert_eq!(t.sync_at(130), 3);
        assert_eq!(t.sync_at(100), 0);
        // the third sample, made into what the decoder takes
        let s = t.samples[2];
        let raw = &file[s.offset as usize..(s.offset + s.size as u64) as usize];
        let mut out = Vec::new();
        t.annexb(&s, raw, &mut out).unwrap();
        assert_eq!(out, annexb(&[&[0x41, 2, 2, 2, 2, 2, 2, 2]]));
        // a sync sample: the parameter sets first
        let s = t.samples[3];
        let raw = &file[s.offset as usize..(s.offset + s.size as u64) as usize];
        t.annexb(&s, raw, &mut out).unwrap();
        assert!(out.starts_with(&annexb(&[&[0x67, 0x4d, 0x00, 0x33, 0xaa], &[0x68, 0xee, 0x3c], &[0x65]])));
    }

    #[test]
    fn a_file_cut_inside_its_last_fragment_gives_the_samples_before_the_cut() {
        let file = recorded();
        let whole = read(&mut io::Cursor::new(&file)).unwrap();
        let cut = read(&mut io::Cursor::new(&file[..file.len() - 3])).unwrap();
        assert_eq!(cut.samples, whole.samples[..5]);
        assert_eq!(cut.duration, whole.samples[5].time);
        // cut inside the last moof: its fragment is gone, the first one whole
        let last_moof = file.windows(4).rposition(|w| w == b"moof").unwrap() - 4;
        let cut = read(&mut io::Cursor::new(&file[..last_moof + 20])).unwrap();
        assert_eq!(cut.samples, whole.samples[..3]);
    }

    #[test]
    fn a_file_without_a_video_track_or_moov_is_refused() {
        assert!(read(&mut io::Cursor::new(vec![0u8, 0, 0, 8, b'f', b'r', b'e', b'e'])).is_err());
        assert!(read(&mut io::Cursor::new(Vec::<u8>::new())).is_err());
    }
}

#[cfg(test)]
mod file_check {
    /// `MICROCAM_MP4=<file> cargo test -p microcam-core demux::file_check -- --ignored --nocapture`: a real file's index printed.
    #[test]
    #[ignore]
    fn a_real_file() {
        let p = std::env::var("MICROCAM_MP4").unwrap();
        let t = super::read(&mut std::fs::File::open(p).unwrap()).unwrap();
        println!("{:?} {:?} ts {} fps {} samples {} syncs {} largest {}", t.codec, t.size, t.timescale, t.fps(), t.samples.len(), t.samples.iter().filter(|s| s.sync).count(), t.largest());
        for s in t.samples.iter().take(3) {
            println!("{} {} {}", s.offset, s.size, s.sync as u8);
        }
    }
}
