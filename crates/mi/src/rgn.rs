//! MI_RGN: OSD regions blended into an SCL output port's frames by its GOP, through microcam_rgn (our driver behind the
//! stock API, `kmods/microcam-rgn`). Every region here is part of an output's `ui`: ARGB4444 with per-pixel alpha, one GWIN
//! each, scanned from the region's front canvas while the GE draws into the back one ([`canvas`] … [`update`]).

use microcam_core::reconcile::ChnPort;
use microcam_core::{Rect, Size};
use mi_sys as ffi;

use crate::sys::module_id;
use crate::{check, Result};

const ARGB4444: u32 = ffi::MI_RGN_PixelFormat_e::E_MI_RGN_PIXEL_FORMAT_ARGB4444;

/// The palette is global; no region here uses it.
pub fn init() -> Result<()> {
    let mut t: ffi::MI_RGN_PaletteTable_t = Default::default();
    check("MI_RGN_Init", unsafe { ffi::MI_RGN_Init(0, &mut t) })
}

pub fn deinit() -> Result<()> {
    check("MI_RGN_DeInit", unsafe { ffi::MI_RGN_DeInit(0) })
}

/// Comes transparent.
pub fn create(handle: u32, size: Size) -> Result<()> {
    let mut a = ffi::MI_RGN_Attr_t { eType: ffi::MI_RGN_Type_e::E_MI_RGN_TYPE_OSD, ePixelFmt: ARGB4444, u32Width: size.w, u32Height: size.h };
    check("MI_RGN_Create", unsafe { ffi::MI_RGN_Create(0, handle, &mut a) })
}

/// A cover: one colour laid by the port's cover engine into its frames, under every window; no memory (4 per port).
pub fn create_cover(handle: u32) -> Result<()> {
    let mut a = ffi::MI_RGN_Attr_t { eType: ffi::MI_RGN_Type_e::E_MI_RGN_TYPE_COVER, ePixelFmt: 0, u32Width: 0, u32Height: 0 };
    check("MI_RGN_Create", unsafe { ffi::MI_RGN_Create(0, handle, &mut a) })
}

/// `MI_RGN_Destroy` of a handle no region has (unknown handle → 01).
const UNKNOWN_HANDLE: u32 = 0xA003_2001;

/// Also takes it off every port it is on; the driver frees its memory once no frame reads it. A handle no region has
/// is gone already.
pub fn destroy(handle: u32) -> Result<()> {
    match check("MI_RGN_Destroy", unsafe { ffi::MI_RGN_Destroy(0, handle) }) {
        Err(e) if e.code as u32 == UNKNOWN_HANDLE => Ok(()),
        r => r,
    }
}

fn port(p: ChnPort) -> ffi::MI_RGN_ChnPort_t {
    ffi::MI_RGN_ChnPort_t { eModId: module_id(p.module), s32DevId: p.dev as i32, s32ChnId: p.chn as i32, s32PortId: p.port as i32 }
}

/// Shown (or not) at (`x`, `y`) of the port's frames: per-pixel alpha, or with `alpha` one constant alpha for the whole
/// window that replaces the pixels' own (right only for a window opaque all over).
/// Where windows overlap the one of the higher `layer` shows.
fn param(show: bool, x: u32, y: u32, alpha: Option<u8>, layer: u32) -> ffi::MI_RGN_ChnPortParam_t {
    let mut p: ffi::MI_RGN_ChnPortParam_t = Default::default();
    p.bShow = show as u8;
    p.stPoint = ffi::MI_RGN_Point_t { u32X: x, u32Y: y };
    p.unPara.stOsdChnPort = match alpha {
        None => ffi::MI_RGN_OsdChnPortParam_t { u32Layer: layer, eAlphaMode: ffi::MI_RGN_AlphaMode_e::E_MI_RGN_PIXEL_ALPHA, u8BgAlpha: 0, u8FgAlpha: 0xff, ..Default::default() },
        Some(a) => ffi::MI_RGN_OsdChnPortParam_t { u32Layer: layer, eAlphaMode: ffi::MI_RGN_AlphaMode_e::E_MI_RGN_CONSTANT_ALPHA, u8BgAlpha: a, u8FgAlpha: a, ..Default::default() },
    };
    p
}

/// A colour as the cover engine takes it: the port's own YUV, bytes V, Y, U from the top (the register takes them
/// unconverted; full-range BT.601 as the frames decode).
fn cover_colour([r, g, b]: [u8; 3]) -> u32 {
    let (r, g, b) = (r as f32, g as f32, b as f32);
    let byte = |v: f32| v.round().clamp(0.0, 255.0) as u32;
    let y = byte(0.299 * r + 0.587 * g + 0.114 * b);
    let u = byte(128.0 - 0.168_736 * r - 0.331_264 * g + 0.5 * b);
    let v = byte(128.0 + 0.5 * r - 0.418_688 * g - 0.081_312 * b);
    v << 16 | y << 8 | u
}

/// A cover shown (or not) over `r` of the port's `frame` in `rgb`. The driver takes the rectangle in 1/8192 of the
/// frame and rounds back to pixels: the nearest unit lands on the same pixel.
fn cover_param(show: bool, frame: Size, r: Rect, rgb: [u8; 3]) -> ffi::MI_RGN_ChnPortParam_t {
    let unit = |v: u32, len: u32| ((v as u64 * 8192 + len as u64 / 2) / len as u64) as u32;
    let mut p: ffi::MI_RGN_ChnPortParam_t = Default::default();
    p.bShow = show as u8;
    p.stPoint = ffi::MI_RGN_Point_t { u32X: unit(r.x, frame.w), u32Y: unit(r.y, frame.h) };
    p.unPara.stCoverChnPort = ffi::MI_RGN_CoverChnPortParam_t {
        u32Layer: 0,
        stSize: ffi::MI_RGN_Size_t { u32Width: unit(r.w, frame.w), u32Height: unit(r.h, frame.h) },
        u32Color: cover_colour(rgb),
    };
    p
}

/// A cover put on the port, shown over `r` of its `frame` in `rgb`.
pub fn attach_cover(handle: u32, to: ChnPort, frame: Size, r: Rect, rgb: [u8; 3]) -> Result<()> {
    let mut c = port(to);
    let mut p = cover_param(true, frame, r, rgb);
    check("MI_RGN_AttachToChn", unsafe { ffi::MI_RGN_AttachToChn(0, handle, &mut c, &mut p) })
}

/// A cover shown or hidden, moved or recoloured, from the port's next frame on.
pub fn display_cover(handle: u32, on: ChnPort, show: bool, frame: Size, r: Rect, rgb: [u8; 3]) -> Result<()> {
    let mut c = port(on);
    let mut p = cover_param(show, frame, r, rgb);
    check("MI_RGN_SetDisplayAttr", unsafe { ffi::MI_RGN_SetDisplayAttr(0, handle, &mut c, &mut p) })
}

/// Put on the port, shown at (`x`, `y`) of its frames, per-pixel alpha or `alpha` all over.
pub fn attach(handle: u32, to: ChnPort, x: u32, y: u32, alpha: Option<u8>, layer: u32) -> Result<()> {
    let mut c = port(to);
    let mut p = param(true, x, y, alpha, layer);
    check("MI_RGN_AttachToChn", unsafe { ffi::MI_RGN_AttachToChn(0, handle, &mut c, &mut p) })
}

/// Shown or hidden, at (`x`, `y`), per-pixel alpha or `alpha` all over, at `layer`, from the port's next frame on: the
/// window moves, fades or goes without a redraw (a hidden one takes no GOP window, only its canvas). Set while the
/// region's canvas is lent ([`canvas`]), it waits for [`update`] and lands in the same frame (microcam_rgn).
pub fn display(handle: u32, on: ChnPort, show: bool, x: u32, y: u32, alpha: Option<u8>, layer: u32) -> Result<()> {
    let mut c = port(on);
    let mut p = param(show, x, y, alpha, layer);
    check("MI_RGN_SetDisplayAttr", unsafe { ffi::MI_RGN_SetDisplayAttr(0, handle, &mut c, &mut p) })
}

/// The canvas to draw into: its physical address and row stride. It holds what the region shows now; the call waits
/// for the frames still scanning it (a frame or two) and is lent until [`update`].
pub fn canvas(handle: u32) -> Result<(u64, u32)> {
    let mut c: ffi::MI_RGN_CanvasInfo_t = Default::default();
    check("MI_RGN_GetCanvasInfo", unsafe { ffi::MI_RGN_GetCanvasInfo(0, handle, &mut c) })?;
    Ok((c.phyAddr, c.u32Stride))
}

/// The region gets `n` canvases (microcam_rgn: 3 lets a region updated every frame never wait for one).
pub fn canvases(handle: u32, n: u32) -> std::io::Result<()> {
    std::fs::write("/proc/microcam_rgn", format!("canvases {handle} {n}\n"))
}

/// Whether microcam_rgn still holds canvases of destroyed regions: they go back to the media heap once no frame reads
/// them, a frame or two after [`destroy`]. False without the driver's file.
pub fn burying() -> bool {
    std::fs::read_to_string("/proc/microcam_rgn").is_ok_and(|t| t.lines().any(|l| l == "graveyard 1"))
}

/// The lent canvas shown from the port's next frame on.
pub fn update(handle: u32) -> Result<()> {
    check("MI_RGN_UpdateCanvas", unsafe { ffi::MI_RGN_UpdateCanvas(0, handle) })
}

/// The port's frames as microcam_rgn counts them: programmed (each takes the front canvases of that moment) and done.
/// A canvas handed over by [`update`] is in every frame programmed after the count read once it returned.
pub fn frames(on: ChnPort) -> std::io::Result<(u64, u64)> {
    let text = std::fs::read_to_string("/proc/microcam_rgn")?;
    let name = format!("port {:x}/{}/{}/{} ", module_id(on.module), on.dev, on.chn, on.port);
    let line = text.lines().find_map(|l| l.strip_prefix(&name)).ok_or_else(|| std::io::Error::other(format!("microcam_rgn has no {}", name.trim_end())))?;
    let mut w = line.split_whitespace();
    let mut field = |key: &str| -> std::io::Result<u64> {
        w.by_ref().skip_while(|t| *t != key).nth(1).and_then(|v| v.parse().ok()).ok_or_else(|| std::io::Error::other(format!("microcam_rgn: no '{key}' in '{line}'")))
    };
    Ok((field("frames")?, field("done")?))
}
