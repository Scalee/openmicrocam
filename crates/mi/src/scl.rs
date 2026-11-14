//! MI_SCL: the scaler device, channel and output ports, and the memory-to-memory scaler (SCL dev 3), one engine for
//! every output: every copy goes through [`stretch`], which counts the engine's time per user (the library serialises
//! the calls itself).

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use microcam_core::{Rect, Size};
use mi_sys as ffi;

use crate::{check, rect, size, Result, PIXEL_NV12};

pub fn create_device(dev: u32, hw_mask: u32) -> Result<()> {
    let mut a = ffi::MI_SCL_DevAttr_t { u32NeedUseHWOutPortMask: hw_mask };
    check("MI_SCL_CreateDevice", unsafe { ffi::MI_SCL_CreateDevice(dev, &mut a) })
}

pub fn destroy_device(dev: u32) -> Result<()> {
    check("MI_SCL_DestroyDevice", unsafe { ffi::MI_SCL_DestroyDevice(dev) })
}

pub fn create_channel(dev: u32, chn: u32) -> Result<()> {
    let mut a = ffi::MI_SCL_ChannelAttr_t { u32Reserved: 0 };
    check("MI_SCL_CreateChannel", unsafe { ffi::MI_SCL_CreateChannel(dev, chn, &mut a) })?;
    let mut p = ffi::MI_SCL_ChnParam_t { eRot: ffi::MI_SYS_Rotate_e::E_MI_SYS_ROTATE_NONE };
    check("MI_SCL_SetChnParam", unsafe { ffi::MI_SCL_SetChnParam(dev, chn, &mut p) })
}

pub fn destroy_channel(dev: u32, chn: u32) -> Result<()> {
    check("MI_SCL_DestroyChannel", unsafe { ffi::MI_SCL_DestroyChannel(dev, chn) })
}

pub fn start_channel(dev: u32, chn: u32) -> Result<()> {
    check("MI_SCL_StartChannel", unsafe { ffi::MI_SCL_StartChannel(dev, chn) })
}

pub fn stop_channel(dev: u32, chn: u32) -> Result<()> {
    check("MI_SCL_StopChannel", unsafe { ffi::MI_SCL_StopChannel(dev, chn) })
}

/// Output port: input window `crop` (sensor-frame coordinates) scaled to `out`, NV12, no mirror/flip/compression.
pub fn set_port_param(dev: u32, chn: u32, port: u32, crop: Rect, out: Size) -> Result<()> {
    let mut p: ffi::MI_SCL_OutPortParam_t = Default::default();
    p.stSCLOutCropRect = rect(crop);
    p.stSCLOutputSize = size(out);
    p.bMirror = 0;
    p.bFlip = 0;
    p.ePixelFormat = PIXEL_NV12;
    p.eCompressMode = ffi::MI_SYS_CompressMode_e::E_MI_SYS_COMPRESS_MODE_NONE;
    check("MI_SCL_SetOutputPortParam", unsafe { ffi::MI_SCL_SetOutputPortParam(dev, chn, port, &mut p) })
}

pub fn enable_port(dev: u32, chn: u32, port: u32) -> Result<()> {
    check("MI_SCL_EnableOutputPort", unsafe { ffi::MI_SCL_EnableOutputPort(dev, chn, port) })
}

pub fn disable_port(dev: u32, chn: u32, port: u32) -> Result<()> {
    check("MI_SCL_DisableOutputPort", unsafe { ffi::MI_SCL_DisableOutputPort(dev, chn, port) })
}

/// Who asks SCL dev 3 for a copy.
#[derive(Clone, Copy)]
pub enum User {
    /// the feed thread: frames on their way to an encoder or DISP
    Feed,
    /// the pipeline: pictures into DISP ports, camera pictures
    Pipeline,
    /// the `ui`: pictures scaled to their windows, the menu baked over them
    Ui,
    /// the `res` thread: a hardware-decoded picture brought to its form
    Res,
}

impl User {
    pub const ALL: [User; 4] = [User::Feed, User::Pipeline, User::Ui, User::Res];
    pub fn name(self) -> &'static str {
        match self {
            User::Feed => "feed",
            User::Pipeline => "pipeline",
            User::Ui => "ui",
            User::Res => "res",
        }
    }
}

/// µs of SCL dev 3 per user since microcamd started.
static BUSY_US: [AtomicU64; 4] = [AtomicU64::new(0), AtomicU64::new(0), AtomicU64::new(0), AtomicU64::new(0)];

/// µs of SCL dev 3 each user has taken since microcamd started, in [`User::ALL`]'s order.
pub fn busy_us() -> [u64; 4] {
    BUSY_US.each_ref().map(|b| b.load(Ordering::Relaxed))
}

/// `crop` of `src` scaled into `dst` by SCL dev 3 (synchronous), counted for `user`.
pub fn stretch(user: User, src: &ffi::MI_SCL_DirectBuf_t, crop: Rect, dst: &ffi::MI_SCL_DirectBuf_t) -> Result<()> {
    let (mut s, mut d, mut c) = (*src, *dst, rect(crop));
    let t0 = Instant::now();
    let r = check("MI_SCL_StretchBuf", unsafe { ffi::MI_SCL_StretchBuf(&mut s, &mut c, &mut d, ffi::MI_SCL_FilterType_e::E_MI_SCL_FILTER_TYPE_AUTO) });
    BUSY_US[user as usize].fetch_add(t0.elapsed().as_micros() as u64, Ordering::Relaxed);
    r
}
