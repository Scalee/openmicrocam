//! MI_ISP: device, channel, output port, IQ bin (prototypes: `mi_isp.h`, written by us).

use std::ffi::CString;

use microcam_core::Rect;
use mi_sys as ffi;

use crate::{check, rect, Result, PIXEL_NV12};

/// The vendor app's IQ-bin user key: the last argument it passes to `MI_ISP_API_CmdLoadBinFile` when it loads the IQ file.
const IQ_USER_KEY: u32 = 0x4d2;

/// The AE's exposure limit, raw words (MI_ISP_AE_EXPO_LIMIT_TYPE_t: [0] min shutter us, [1] max shutter us, ...).
pub fn ae_limit(dev: u32, chn: u32) -> Result<[u32; 16]> {
    let mut w = [0u32; 16];
    check("MI_ISP_AE_GetExposureLimit", unsafe { ffi::MI_ISP_AE_GetExposureLimit(dev, chn, w.as_mut_ptr()) })?;
    Ok(w)
}

/// The AE's longest exposure, the rest of its limit kept.
pub fn set_ae_max_shutter(dev: u32, chn: u32, us: u32) -> Result<[u32; 16]> {
    let mut w = ae_limit(dev, chn)?;
    let was = w;
    w[1] = us.max(w[0]);
    check("MI_ISP_AE_SetExposureLimit", unsafe { ffi::MI_ISP_AE_SetExposureLimit(dev, chn, w.as_mut_ptr()) })?;
    Ok(was)
}

pub fn create_device(dev: u32) -> Result<()> {
    let mut a = ffi::MI_ISP_DevAttr_t { u32DevStitchMask: 1 };
    check("MI_ISP_CreateDevice", unsafe { ffi::MI_ISP_CreateDevice(dev, &mut a) })
}

pub fn destroy_device(dev: u32) -> Result<()> {
    check("MI_ISP_DestoryDevice", unsafe { ffi::MI_ISP_DestoryDevice(dev) })
}

pub fn create_channel(dev: u32, chn: u32, pad: u32) -> Result<()> {
    let mut a: ffi::MI_ISP_ChannelAttr_t = Default::default();
    a.u32SensorBindId = 1 << pad;
    check("MI_ISP_CreateChannel", unsafe { ffi::MI_ISP_CreateChannel(dev, chn, &mut a) })
}

pub fn destroy_channel(dev: u32, chn: u32) -> Result<()> {
    check("MI_ISP_DestroyChannel", unsafe { ffi::MI_ISP_DestroyChannel(dev, chn) })
}

pub fn set_chn_param(dev: u32, chn: u32, nr3d: u8) -> Result<()> {
    let mut p: ffi::MI_ISP_ChnParam_t = Default::default();
    p.eHDRType = ffi::MI_ISP_HDRType_e::E_MI_ISP_HDR_TYPE_OFF;
    p.e3DNRLevel = nr3d as u32;
    p.bMirror = 0;
    p.bFlip = 0;
    p.eRot = ffi::MI_ISP_Rotate_e::E_MI_ISP_ROTATE_NONE;
    check("MI_ISP_SetChnParam", unsafe { ffi::MI_ISP_SetChnParam(dev, chn, &mut p) })
}

pub fn start_channel(dev: u32, chn: u32) -> Result<()> {
    check("MI_ISP_StartChannel", unsafe { ffi::MI_ISP_StartChannel(dev, chn) })
}

pub fn stop_channel(dev: u32, chn: u32) -> Result<()> {
    check("MI_ISP_StopChannel", unsafe { ffi::MI_ISP_StopChannel(dev, chn) })
}

/// Output port: `crop` None = the whole input (all-zero rect, as the vendor app passes), NV12.
pub fn set_port_param(dev: u32, chn: u32, port: u32, crop: Option<Rect>) -> Result<()> {
    let mut p: ffi::MI_ISP_OutPortParam_t = Default::default();
    if let Some(c) = crop {
        p.stCropRect = rect(c);
    }
    p.ePixelFormat = PIXEL_NV12;
    check("MI_ISP_SetOutputPortParam", unsafe { ffi::MI_ISP_SetOutputPortParam(dev, chn, port, &mut p) })
}

pub fn enable_port(dev: u32, chn: u32, port: u32) -> Result<()> {
    check("MI_ISP_EnableOutputPort", unsafe { ffi::MI_ISP_EnableOutputPort(dev, chn, port) })
}

pub fn disable_port(dev: u32, chn: u32, port: u32) -> Result<()> {
    check("MI_ISP_DisableOutputPort", unsafe { ffi::MI_ISP_DisableOutputPort(dev, chn, port) })
}

pub fn input_crop(dev: u32, chn: u32) -> Result<Rect> {
    let mut r: ffi::MI_SYS_WindowRect_t = Default::default();
    check("MI_ISP_GetInputPortCrop", unsafe { ffi::MI_ISP_GetInputPortCrop(dev, chn, &mut r) })?;
    Ok(Rect { x: r.u16X as u32, y: r.u16Y as u32, w: r.u16Width as u32, h: r.u16Height as u32 })
}

pub fn load_iq_bin(dev: u32, chn: u32, path: &str) -> Result<()> {
    let c = CString::new(path).map_err(|_| crate::MiError { call: "MI_ISP_API_CmdLoadBinFile", code: -1 })?;
    // SAFETY: the library reads the path (it takes `char*` but does not write it).
    check("MI_ISP_API_CmdLoadBinFile", unsafe { ffi::MI_ISP_API_CmdLoadBinFile(dev, chn, c.as_ptr() as *mut _, IQ_USER_KEY) })
}

/// Poll the 3A until AE and AWB report ready (the vendor polls up to 101 × 10 ms before loading the IQ bin). Returns
/// how many polls it took, or None if the 3A never reported ready within the budget.
pub fn wait_3a_ready(dev: u32, chn: u32, polls: u32) -> Option<u32> {
    let mut st = [0u32; 3];
    for i in 0..polls {
        // SAFETY: the library fills three words.
        unsafe { ffi::CUS3A_GetAlgoStatus(dev, chn, st.as_mut_ptr()) };
        if st[0] == 1 && st[1] == 1 {
            return Some(i);
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    None
}
