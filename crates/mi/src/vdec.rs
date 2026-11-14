//! MI_VDEC: the player's decoder — one H.264 / H.265 channel on the Wave5 VPU, fed a sample at a time, its port 0
//! bound into an SCL dev-1 channel that writes the picture into an output's canvases.
//!
//! The device is made at the first channel and kept to the process's exit, as mi_venc keeps its devices: mhal's
//! `MHAL_VDEC_DestroyDevice` (and VENC's) shuts the shared Wave5 core down at its own last user without looking at the
//! other side's, after which no H.26x encoder channel can be made until a reboot.

use std::sync::atomic::{AtomicBool, Ordering};

use microcam_core::{Codec, Size};
use mi_sys as ffi;

use crate::{check, MiError, Result};

pub const DEV: i32 = 0;
pub const CHN: i32 = 0;

/// The send found no room in the ring within its time (`MI_ERR_VDEC_BUF_FULL`).
pub const BUF_FULL: i32 = 0xA008200Fu32 as i32;
/// A refresh or step is still pending (`MI_ERR_VDEC_BUSY`).
pub const BUSY: i32 = 0xA0082012u32 as i32;

static DEVICE: AtomicBool = AtomicBool::new(false);

/// The channel made and started: `codec`, a picture of `size` (range-checked only), an ES ring of `ring` bytes, port 0
/// at `port` (the VPU scales down to it).
pub fn up(codec: Codec, size: Size, ring: u32, port: Size) -> Result<()> {
    if !DEVICE.load(Ordering::Acquire) {
        let mut p = ffi::MI_VDEC_InitParam_t { u32Reserved: 0 };
        // SAFETY: a plain struct we own, not read by the kernel.
        let r = unsafe { ffi::MI_VDEC_CreateDev(DEV, &mut p) };
        // INITED: made by an earlier run of ours in this process
        if r != 0 && r != 0xA0082016u32 as i32 {
            check("MI_VDEC_CreateDev", r)?;
        }
        DEVICE.store(true, Ordering::Release);
    }
    let mut a: ffi::MI_VDEC_ChnAttr_t = Default::default();
    a.eCodecType = match codec {
        Codec::H265 => ffi::MI_VDEC_CodecType_e::E_MI_VDEC_CODEC_TYPE_H265,
        _ => ffi::MI_VDEC_CodecType_e::E_MI_VDEC_CODEC_TYPE_H264,
    };
    a.u32BufSize = ring;
    a.u32PicWidth = size.w.clamp(176, 4096) & !1;
    a.u32PicHeight = size.h.clamp(128, 4096) & !1;
    a.eVideoMode = ffi::MI_VDEC_VideoMode_e::E_MI_VDEC_VIDEO_MODE_FRAME;
    a.eDpbBufMode = ffi::MI_VDEC_DpbBufMode_e::E_MI_VDEC_DPB_MODE_NORMAL;
    a.stVdecVideoAttr.u32RefFrameNum = microcam_core::desired::VDEC_REF_FRAMES;
    // SAFETY: the attribute is a plain struct we own.
    check("MI_VDEC_CreateChn", unsafe { ffi::MI_VDEC_CreateChn(DEV, CHN, &mut a) })?;
    let r = set_port(port).and_then(|_| check("MI_VDEC_StartChn", unsafe { ffi::MI_VDEC_StartChn(DEV, CHN) }));
    if let Err(e) = r {
        // SAFETY: plain call.
        unsafe { ffi::MI_VDEC_DestroyChn(DEV, CHN) };
        return Err(e);
    }
    Ok(())
}

/// Port 0's size: the VPU scales the picture down to it from the next decode on.
pub fn set_port(port: Size) -> Result<()> {
    let mut p = ffi::MI_VDEC_OutputPortAttr_t { u16Width: port.w as u16, u16Height: port.h as u16 };
    // SAFETY: a plain struct we own.
    check("MI_VDEC_SetOutputPortAttr", unsafe { ffi::MI_VDEC_SetOutputPortAttr(DEV, CHN, &mut p) })
}

/// The channel stopped and destroyed (the device stays).
pub fn down() -> Result<()> {
    // SAFETY: plain calls.
    check("MI_VDEC_StopChn", unsafe { ffi::MI_VDEC_StopChn(DEV, CHN) })?;
    check("MI_VDEC_DestroyChn", unsafe { ffi::MI_VDEC_DestroyChn(DEV, CHN) })
}

/// One access unit (Annex B) into the ring, waiting at most `timeout_ms` for room; `pts` (µs) comes back on its frame.
pub fn send(au: &[u8], pts: u64, timeout_ms: i32) -> std::result::Result<(), MiError> {
    let mut s = ffi::MI_VDEC_VideoStream_t { pu8Addr: au.as_ptr() as *mut u8, u32Len: au.len() as u32, u64PTS: pts, bEndOfFrame: 1, bEndOfStream: 0 };
    // SAFETY: the kernel copies `au` (alive for the call) into its ring; nothing is kept.
    let r = unsafe { ffi::MI_VDEC_SendStream(DEV, CHN, &mut s, timeout_ms) };
    if r == 0 {
        Ok(())
    } else {
        Err(MiError { call: "MI_VDEC_SendStream", code: r })
    }
}

/// No new picture decoded until [`resume`]; [`refresh`] decodes the head sample again meanwhile. `FlushChn` is not
/// offered: on a paused channel with a refresh pending it never returns (the thread stuck in the kernel until a reboot).
pub fn pause() -> Result<()> {
    check("MI_VDEC_PauseChn", unsafe { ffi::MI_VDEC_PauseChn(DEV, CHN) })
}

/// Decoding again; `BUSY` while a refresh is still pending (try again).
pub fn resume() -> std::result::Result<(), MiError> {
    // SAFETY: plain call.
    let r = unsafe { ffi::MI_VDEC_ResumeChn(DEV, CHN) };
    if r == 0 {
        Ok(())
    } else {
        Err(MiError { call: "MI_VDEC_ResumeChn", code: r })
    }
}

/// While paused: the head sample (sent, not yet released) decoded again, its picture through the port again. `BUSY`
/// while the last one is still pending — not an error for the caller.
pub fn refresh() -> std::result::Result<(), MiError> {
    // SAFETY: plain call.
    let r = unsafe { ffi::MI_VDEC_RefreshChn(DEV, CHN) };
    if r == 0 {
        Ok(())
    } else {
        Err(MiError { call: "MI_VDEC_RefreshChn", code: r })
    }
}

/// (received, decoded, left in the ring) samples.
pub fn stat() -> Result<(u32, u32, u32)> {
    let mut s: ffi::MI_VDEC_ChnStat_t = Default::default();
    // SAFETY: the library fills a plain struct we own.
    check("MI_VDEC_GetChnStat", unsafe { ffi::MI_VDEC_GetChnStat(DEV, CHN, &mut s) })?;
    Ok((s.u32RecvStreamFrames, s.u32DecodeStreamFrames, s.u32LeftStreamFrames))
}
