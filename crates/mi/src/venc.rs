//! MI_VENC: the encoders feeding USB (MJPEG on the JPEG device, H.264/H.265 on the H.26x device) and the stream
//! fetch used by the pump.

use microcam_core::reconcile::{ChnPort, Module, Ring};
use microcam_core::{Codec, Size};
use mi_sys as ffi;

use crate::{check, sys, Result};

/// Encoder channel settings, from the vendor app's per-format table.
#[derive(Clone, Copy, Debug)]
pub struct EncoderSpec {
    pub codec: Codec,
    pub size: Size,
    pub fps: u32,
    pub mjpeg_quality: u8,
    /// H.26x bitrate in bit/s; 0 = the vendor's size rule
    pub bitrate: u32,
    /// H.26x frames from one IDR to the next; 0 = 30
    pub gop: u32,
    /// JPEG: the bytes of each of the channel's two bitstream rings (`microcam_core::heap::jpeg_ring`)
    pub ring: u32,
    /// H.26x: the rate control held at [`START_QP`] or above from the first frame on, until [`qp_floor`] lets it go
    pub start_floor: bool,
}

/// The QP an H.26x recording's rate control is held at or above over its first GOP: left alone it starts well below
/// where our picture settles and climbs a few steps a frame, so its first frames take several times their share of bits
/// and the encoder holds each of their canvases over a frame period — with the cover's photo holding one more, the
/// recording's pool runs dry and a frame is lost. About where a 4K recording settles.
pub const START_QP: u32 = 40;
/// The encoder's own lowest QP, the floor given back.
const MIN_QP: u32 = 12;

/// The vendor's bitrate by picture size: 8 Mbit above 2560×1440, 4 Mbit from 1080p, 512 kbit under
/// 640×480, else 2 Mbit.
pub fn vendor_bitrate(s: Size) -> u32 {
    let px = s.w as u64 * s.h as u64;
    if px > 2560 * 1440 {
        8_000_000
    } else if px >= 1920 * 1080 {
        4_000_000
    } else if px < 640 * 480 {
        512_000
    } else {
        2_000_000
    }
}

/// The highest source rate `mi_venc` passes on: above it the channel's rate is taken for 30/1 (its `VerifyFps`).
const MAX_RC_FPS: u32 = 60;

/// The H.26x rate control's frame rate (num, den) and bitrate for `fps` frames a second at `bitrate`: the controller
/// spends bitrate × den / num a frame and takes any fraction, `mi_venc` refuses a rate above 60. Above it both go
/// divided by the same k, so each frame gets the bits of `bitrate` at `fps`.
fn rc_rate(fps: u32, bitrate: u32) -> (u32, u32, u32) {
    let k = fps.div_ceil(MAX_RC_FPS).max(1);
    (fps, k, bitrate / k)
}

fn attr(spec: &EncoderSpec) -> ffi::MI_VENC_ChnAttr_t {
    let mut a: ffi::MI_VENC_ChnAttr_t = Default::default();
    let (w, h) = (spec.size.w, spec.size.h);
    let bitrate = if spec.bitrate == 0 { vendor_bitrate(spec.size) } else { spec.bitrate };
    let (fps_num, fps_den, rc_bitrate) = rc_rate(spec.fps, bitrate);
    let gop = if spec.gop == 0 { 30 } else { spec.gop };
    match spec.codec {
        Codec::Mjpeg => {
            a.stVeAttr.eType = ffi::MI_VENC_ModType_e::E_MI_VENC_MODTYPE_JPEGE;
            let j = unsafe { &mut a.stVeAttr.__bindgen_anon_1.stAttrJpeg };
            j.u32PicWidth = w;
            j.u32PicHeight = h;
            j.u32MaxPicWidth = w;
            j.u32MaxPicHeight = (h + 15) & !15;
            // each of the channel's bitstream rings; the default (half a frame of pixels) holds two 4K JPEGs or fewer,
            // and a frame that finds no room is dropped
            j.u32BufSize = spec.ring;
            j.bByFrame = 1;
            a.stRcAttr.eRcMode = ffi::MI_VENC_RcMode_e::E_MI_VENC_RC_MODE_MJPEGFIXQP;
            let rc = unsafe { &mut a.stRcAttr.__bindgen_anon_1.stAttrMjpegFixQp };
            rc.u32SrcFrmRateNum = spec.fps;
            rc.u32SrcFrmRateDen = 1;
            rc.u32Qfactor = spec.mjpeg_quality as u32;
        }
        Codec::H264 => {
            a.stVeAttr.eType = ffi::MI_VENC_ModType_e::E_MI_VENC_MODTYPE_H264E;
            let x = unsafe { &mut a.stVeAttr.__bindgen_anon_1.stAttrH264e };
            x.u32PicWidth = w;
            x.u32PicHeight = h;
            x.u32MaxPicWidth = w;
            x.u32MaxPicHeight = h;
            x.bByFrame = 1;
            x.u32Profile = 1;
            x.u32BFrameNum = 0;
            a.stRcAttr.eRcMode = ffi::MI_VENC_RcMode_e::E_MI_VENC_RC_MODE_H264CBR;
            let rc = unsafe { &mut a.stRcAttr.__bindgen_anon_1.stAttrH264Cbr };
            rc.u32Gop = gop;
            rc.u32StatTime = 0;
            rc.u32SrcFrmRateNum = fps_num;
            rc.u32SrcFrmRateDen = fps_den;
            rc.u32BitRate = rc_bitrate;
            rc.u32FluctuateLevel = 0;
        }
        Codec::H265 => {
            a.stVeAttr.eType = ffi::MI_VENC_ModType_e::E_MI_VENC_MODTYPE_H265E;
            let x = unsafe { &mut a.stVeAttr.__bindgen_anon_1.stAttrH265e };
            x.u32PicWidth = w;
            x.u32PicHeight = h;
            x.u32MaxPicWidth = w;
            x.u32MaxPicHeight = h;
            x.bByFrame = 1;
            x.u32Profile = 0;
            x.u32BFrameNum = 0;
            a.stRcAttr.eRcMode = ffi::MI_VENC_RcMode_e::E_MI_VENC_RC_MODE_H265CBR;
            let rc = unsafe { &mut a.stRcAttr.__bindgen_anon_1.stAttrH265Cbr };
            rc.u32Gop = gop;
            rc.u32StatTime = 0;
            rc.u32SrcFrmRateNum = fps_num;
            rc.u32SrcFrmRateDen = fps_den;
            rc.u32BitRate = rc_bitrate;
            rc.u32FluctuateLevel = 0;
        }
    }
    a
}

/// Create the channel, set its input, start receiving pictures. Frame-based input, frames handed to it; or, with `ring`
/// (an H.26x channel), the SCL port bound into it in a one-frame ring (HW_RING, its line count the picture's height)
/// before it starts, the order the SDK's UVC demo for this chip uses; the bind's source rate the ring's, its
/// destination the channel's.
pub fn up(dev: u32, chn: u32, spec: &EncoderSpec, ring: Option<Ring>) -> Result<()> {
    let mut a = attr(spec);
    check("MI_VENC_CreateChn", unsafe { ffi::MI_VENC_CreateChn(dev as i32, chn as i32, &mut a) })?;
    let mut src: ffi::MI_VENC_InputSourceConfig_t = Default::default();
    src.eInputSrcBufferMode = if ring.is_some() {
        ffi::MI_VENC_InputSrcBufferMode_e::E_MI_VENC_INPUT_MODE_RING_ONE_FRM
    } else {
        ffi::MI_VENC_InputSrcBufferMode_e::E_MI_VENC_INPUT_MODE_NORMAL_FRMBASE
    };
    if let Err(e) = check("MI_VENC_SetInputSourceConfig", unsafe { ffi::MI_VENC_SetInputSourceConfig(dev as i32, chn as i32, &mut src) }) {
        let _ = destroy(dev, chn);
        return Err(e);
    }
    if spec.codec == Codec::Mjpeg {
        if let Err(e) = set_jpeg_tables(dev, chn, spec.mjpeg_quality as u32) {
            let _ = destroy(dev, chn);
            return Err(e);
        }
    } else if spec.start_floor {
        if let Err(e) = qp_floor(dev, chn, Some(START_QP)) {
            let _ = destroy(dev, chn);
            return Err(e);
        }
    }
    let input = ChnPort { module: Module::Venc, dev, chn, port: 0 };
    if let Some(r) = ring {
        if let Err(e) = sys::bind_ring(r.port, input, r.src_fps, spec.fps, spec.size.h) {
            let _ = destroy(dev, chn);
            return Err(e);
        }
    }
    if let Err(e) = check("MI_VENC_StartRecvPic", unsafe { ffi::MI_VENC_StartRecvPic(dev as i32, chn as i32) }) {
        if let Some(r) = ring {
            let _ = sys::unbind(r.port, input);
        }
        let _ = destroy(dev, chn);
        return Err(e);
    }
    Ok(())
}

/// The base JPEG quantisation tables (Annex K), luma and chroma, in the order the encoder takes them.
const BASE_Y: [u8; 64] = [
    16, 11, 10, 16, 24, 40, 51, 61, 12, 12, 14, 19, 26, 58, 60, 55, 14, 13, 16, 24, 40, 57, 69, 56, 14, 17, 22, 29, 51, 87, 80, 62, 18, 22, 37, 56, 68, 109, 103, 77, 24, 35, 55, 64, 81, 104,
    113, 92, 49, 64, 78, 87, 103, 121, 120, 101, 72, 92, 95, 98, 112, 100, 103, 99,
];
const BASE_C: [u8; 64] = [
    17, 18, 24, 47, 99, 99, 99, 99, 18, 21, 26, 66, 99, 99, 99, 99, 24, 26, 56, 99, 99, 99, 99, 99, 47, 66, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99,
    99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99,
];

/// Largest DC step of a JPEG channel's tables. At a low quality the stock DC step (53 at 15) makes a block's mean change
/// in steps of ~6.6 levels and a smooth surface shows bands; 8 removes them for a few percent more bytes.
const JPEG_DC_STEP: u8 = 8;

/// The channel's quantisation tables: the stock ones for `quality` (the Annex K rule) with the DC entry capped. Set before
/// the channel starts: `MI_VENC_SetJpegParam` on a running channel hung the SoC. The channel's own factor goes to 50, the
/// identity scale, so the tables are used as given.
fn set_jpeg_tables(dev: u32, chn: u32, quality: u32) -> Result<()> {
    let q = quality.clamp(1, 100);
    let scale = if q < 50 { 5000 / q } else { 200 - 2 * q };
    let table = |base: &[u8; 64]| -> [u8; 64] {
        let mut t = [0u8; 64];
        for (o, b) in t.iter_mut().zip(base) {
            *o = ((*b as u32 * scale + 50) / 100).clamp(1, 255) as u8;
        }
        t[0] = t[0].min(JPEG_DC_STEP);
        t
    };
    let mut p: ffi::MI_VENC_ParamJpeg_t = Default::default();
    p.u32Qfactor = 50;
    p.au8YQt = table(&BASE_Y);
    p.au8CbCrQt = table(&BASE_C);
    check("MI_VENC_SetJpegParam", unsafe { ffi::MI_VENC_SetJpegParam(dev as i32, chn as i32, &mut p) })
}

/// Stop receiving, then for a ring (`ring`: the SCL port bound into it) the port off and the bind taken off, destroy —
/// a ring's port left writing unbound makes SCL fail its whole channel's frame. Every step is
/// attempted; the first error is returned.
pub fn down(dev: u32, chn: u32, ring: Option<ChnPort>) -> Result<()> {
    let r1 = check("MI_VENC_StopRecvPic", unsafe { ffi::MI_VENC_StopRecvPic(dev as i32, chn as i32) });
    let r2 = ring.map_or(Ok(()), |p| crate::scl::disable_port(p.dev, p.chn, p.port).and(sys::unbind(p, ChnPort { module: Module::Venc, dev, chn, port: 0 })));
    let r3 = destroy(dev, chn);
    r1.and(r2).and(r3)
}

fn destroy(dev: u32, chn: u32) -> Result<()> {
    check("MI_VENC_DestroyChn", unsafe { ffi::MI_VENC_DestroyChn(dev as i32, chn as i32) })
}

/// The lowest QP of an H.26x channel's rate control, its I and P frames alike: `Some(q)`, or None for the encoder's own.
/// Taken at the channel's next frame; a QP range change forces no IDR.
pub fn qp_floor(dev: u32, chn: u32, floor: Option<u32>) -> Result<()> {
    let mut p: ffi::MI_VENC_RcParam_t = Default::default();
    check("MI_VENC_GetRcParam", unsafe { ffi::MI_VENC_GetRcParam(dev as i32, chn as i32, &mut p) })?;
    let q = floor.unwrap_or(MIN_QP);
    // SAFETY: the CBR member is the one an H.264 / H.265 CBR channel's parameters fill.
    let c = unsafe { &mut p.__bindgen_anon_1.stParamCbr };
    (c.u32MinQp, c.u32MinIQp) = (q, q);
    check("MI_VENC_SetRcParam", unsafe { ffi::MI_VENC_SetRcParam(dev as i32, chn as i32, &mut p) })
}

pub fn request_idr(dev: u32, chn: u32) -> Result<()> {
    check("MI_VENC_RequestIdr", unsafe { ffi::MI_VENC_RequestIdr(dev as i32, chn as i32, 1) })
}

/// One encoded frame as the encoder hands it out. Must be given back with [`release`] before the next fetch.
pub struct Stream {
    raw: ffi::MI_VENC_Stream_t,
    packs: Vec<ffi::MI_VENC_Pack_t>,
    pub keyframe: bool,
}

impl Stream {
    /// Total payload length over all packs.
    pub fn len(&self) -> usize {
        self.packs[..self.raw.u32PackCount as usize].iter().map(|p| p.u32Len as usize).sum()
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    /// Copy the payload (all packs, in order) into `dst`; returns bytes written (0 if `dst` is too small).
    pub fn copy_to(&self, dst: &mut [u8]) -> usize {
        let n = self.len();
        if dst.len() < n {
            return 0;
        }
        let mut off = 0;
        for p in &self.packs[..self.raw.u32PackCount as usize] {
            let len = p.u32Len as usize;
            // SAFETY: pu8Addr/u32Len describe the encoder's mapped output buffer, valid until release().
            let src = unsafe { std::slice::from_raw_parts(p.pu8Addr as *const u8, len) };
            dst[off..off + len].copy_from_slice(src);
            off += len;
        }
        off
    }
    pub fn pts(&self) -> u64 {
        self.packs.first().map(|p| p.u64PTS).unwrap_or(0)
    }
}

/// Wait up to `timeout_ms` for an encoded frame. `Ok(None)` = nothing within the timeout.
///
/// Exactly the vendor demo's sequence: `MI_VENC_Query` until `u32CurPacks` > 0, then `MI_VENC_GetStream`
/// with **that** pack count. Asking for more packs than the encoder holds makes `ReleaseStream` walk empty packs
/// (`FlushInvCache(NULL)` → SYS BUF_EMPTY) and skip its final release, after which the channel stays BUSY and can never be
/// stopped (seen 2026-10-24: every frame logged `MI_VENC_ReleaseStream: 0xa009200e`, then `StopRecvPic: 0xa0022012`).
pub fn get_stream(dev: u32, chn: u32, codec: Codec, timeout_ms: i32) -> Result<Option<Stream>> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(timeout_ms.max(0) as u64);
    let n = loop {
        let mut stat: ffi::MI_VENC_ChnStat_t = Default::default();
        let r = unsafe { ffi::MI_VENC_Query(dev as i32, chn as i32, &mut stat) };
        if r != 0 {
            return Err(crate::MiError { call: "MI_VENC_Query", code: r });
        }
        if stat.u32CurPacks > 0 {
            break stat.u32CurPacks as usize;
        }
        if std::time::Instant::now() >= deadline {
            return Ok(None);
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    };
    let mut packs: Vec<ffi::MI_VENC_Pack_t> = vec![Default::default(); n];
    let mut raw: ffi::MI_VENC_Stream_t = Default::default();
    raw.pstPack = packs.as_mut_ptr();
    raw.u32PackCount = n as u32;
    // SAFETY: `raw.pstPack` points to `packs`, which lives in the returned Stream as long as `raw` does.
    let r = unsafe { ffi::MI_VENC_GetStream(dev as i32, chn as i32, &mut raw, 40) };
    if r != 0 {
        log::warn!("MI_VENC_GetStream: {r:#010x}");
        return Ok(None);
    }
    let keyframe = is_keyframe(codec, &raw);
    Ok(Some(Stream { raw, packs, keyframe }))
}

fn is_keyframe(codec: Codec, raw: &ffi::MI_VENC_Stream_t) -> bool {
    // SAFETY: reading the union member that matches the codec of this channel.
    // E_MI_VENC_BASE_IDR = 0 in both the H.264 and the H.265 reference-type enums of this header set.
    const BASE_IDR: u32 = 0;
    unsafe {
        match codec {
            Codec::Mjpeg => true,
            Codec::H264 => raw.__bindgen_anon_1.stH264Info.eRefType == BASE_IDR,
            Codec::H265 => raw.__bindgen_anon_1.stH265Info.eRefType == BASE_IDR,
        }
    }
}

/// Encoded frames ready and not yet taken out (the one taken and not released yet is not among them: `MI_VENC_Query`
/// reports the ring's unread records).
pub fn waiting(dev: u32, chn: u32) -> Result<u32> {
    let mut stat: ffi::MI_VENC_ChnStat_t = Default::default();
    check("MI_VENC_Query", unsafe { ffi::MI_VENC_Query(dev as i32, chn as i32, &mut stat) })?;
    Ok(stat.u32LeftStreamFrames)
}

pub fn release(dev: u32, chn: u32, mut s: Stream) -> Result<()> {
    check("MI_VENC_ReleaseStream", unsafe { ffi::MI_VENC_ReleaseStream(dev as i32, chn as i32, &mut s.raw) })
}
