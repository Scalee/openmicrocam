//! MI_SNR: the sensor pad — mode list, mode select, enable/disable, pad/plane info for the VIF.

use microcam_core::modes::DriverMode;
use microcam_core::{Rect, Size};
use mi_sys as ffi;

use crate::{check, Result};

pub fn set_plane_mode(pad: u32, enable: bool) -> Result<()> {
    check("MI_SNR_SetPlaneMode", unsafe { ffi::MI_SNR_SetPlaneMode(pad, enable as u8) })
}

pub fn set_res(pad: u32, index: u8) -> Result<()> {
    check("MI_SNR_SetRes", unsafe { ffi::MI_SNR_SetRes(pad, index) })
}

pub fn enable(pad: u32) -> Result<()> {
    check("MI_SNR_Enable", unsafe { ffi::MI_SNR_Enable(pad) })
}

pub fn disable(pad: u32) -> Result<()> {
    check("MI_SNR_Disable", unsafe { ffi::MI_SNR_Disable(pad) })
}

/// The driver's resolution list, as `MI_SNR_QueryResCount` / `MI_SNR_GetRes` report it.
pub fn modes(pad: u32) -> Result<Vec<DriverMode>> {
    let mut n: u32 = 0;
    check("MI_SNR_QueryResCount", unsafe { ffi::MI_SNR_QueryResCount(pad, &mut n) })?;
    let mut out = Vec::with_capacity(n as usize);
    for i in 0..n.min(255) {
        let mut r: ffi::MI_SNR_Res_t = Default::default();
        // SAFETY: owned packed struct filled by the call.
        check("MI_SNR_GetRes", unsafe { ffi::MI_SNR_GetRes(pad, i as u8, &mut r) })?;
        let desc = r.strResDesc;
        let bytes: Vec<u8> = desc.iter().map(|&c| c as u8).take_while(|&c| c != 0).collect();
        let out_size = r.stOutputSize;
        out.push(DriverMode {
            index: i as u8,
            name: String::from_utf8_lossy(&bytes).into_owned(),
            size: Size::new(out_size.u16Width as u32, out_size.u16Height as u32),
            max_fps: r.u32MaxFps,
        });
    }
    Ok(out)
}

/// What the VIF needs from the sensor: the capture window and the Bayer pixel format of plane 0.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlaneInfo {
    pub cap: Rect,
    /// `RGB_BAYER_PIXEL(precision, bayer id)` or the plane's own pixel format when it is not Bayer
    pub pixel: u32,
}

pub fn plane_info(pad: u32) -> Result<PlaneInfo> {
    let mut pad_info: ffi::MI_SNR_PADInfo_t = Default::default();
    check("MI_SNR_GetPadInfo", unsafe { ffi::MI_SNR_GetPadInfo(pad, &mut pad_info) })?;
    let mut p: ffi::MI_SNR_PlaneInfo_t = Default::default();
    check("MI_SNR_GetPlaneInfo", unsafe { ffi::MI_SNR_GetPlaneInfo(pad, 0, &mut p) })?;
    let cap = Rect { x: p.stCapRect.u16X as u32, y: p.stCapRect.u16Y as u32, w: p.stCapRect.u16Width as u32, h: p.stCapRect.u16Height as u32 };
    let pixel = if p.eBayerId >= ffi::MI_SYS_BayerId_e::E_MI_SYS_PIXEL_BAYERID_MAX {
        p.ePixel
    } else {
        // the SDK's RGB_BAYER_PIXEL(BitMode, PixelID) macro: 0x14 + bits*12 + id
        ffi::MI_SYS_PixelFormat_e::E_MI_SYS_PIXEL_FRAME_RGB_BAYER_BASE
            + p.ePixPrecision * ffi::MI_SYS_BayerId_e::E_MI_SYS_PIXEL_BAYERID_MAX
            + p.eBayerId
    };
    Ok(PlaneInfo { cap, pixel })
}
