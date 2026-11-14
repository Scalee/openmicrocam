//! Safe layer over the camera's MI user libraries (`mi-sys`). The only crate with `unsafe` outside generated code.
//!
//! Every function here is one MI call with Rust types in and a `Result` out. Nothing is created or destroyed
//! implicitly: there are no `Drop` impls that touch hardware. The daemon drives these through
//! [`hal::MiHal`], which executes the reconciler's operations in the order the reconciler decided.

pub mod canvas;
pub mod compose;
pub mod disp;
pub mod feed;
pub mod gfx;
pub mod hal;
pub mod hdmi;
pub mod isp;
pub mod jpd;
pub mod layer;
pub mod picture;
pub mod rgn;
pub mod scl;
pub mod sensor;
pub mod shot;
pub mod sys;
pub mod sysfs;
pub mod tune;
pub mod vdec;
pub mod venc;
pub mod vif;

/// An MI call that returned an error code. Codes are the SDK's `MI_S32` results (0 = success, else 0xA0xxxxxx).
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{call} failed: {code:#010x}")]
pub struct MiError {
    pub call: &'static str,
    pub code: i32,
}

pub type Result<T> = std::result::Result<T, MiError>;

/// Turn an `MI_S32` into a `Result`.
pub(crate) fn check(call: &'static str, code: i32) -> Result<()> {
    if code == 0 {
        Ok(())
    } else {
        log::warn!("{call}: {code:#010x}");
        Err(MiError { call, code })
    }
}

/// `MI_SYS_WindowRect_t` from a core rect (the SDK keeps u16 coordinates).
pub(crate) fn rect(r: microcam_core::Rect) -> mi_sys::MI_SYS_WindowRect_t {
    mi_sys::MI_SYS_WindowRect_t { u16X: r.x as u16, u16Y: r.y as u16, u16Width: r.w as u16, u16Height: r.h as u16 }
}

pub(crate) fn size(s: microcam_core::Size) -> mi_sys::MI_SYS_WindowSize_t {
    mi_sys::MI_SYS_WindowSize_t { u16Width: s.w as u16, u16Height: s.h as u16 }
}

/// NV12 everywhere between ISP, SCL, DISP and VENC.
pub(crate) const PIXEL_NV12: u32 = mi_sys::MI_SYS_PixelFormat_e::E_MI_SYS_PIXEL_FRAME_YUV_SEMIPLANAR_420;
