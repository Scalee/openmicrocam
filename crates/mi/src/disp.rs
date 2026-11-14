//! MI_DISP: the display device feeding the HDMI transmitter (the order of the vendor app's St_DispModuleInit).

use microcam_core::hdmi::Timing;
use microcam_core::{Rect, Size};
use mi_sys as ffi;

use crate::{check, Result, PIXEL_NV12};

/// `MI_DISP_OutputTiming_e` of this build (values checked against the 2022-06-06 library DWARF: 1080P60 = 16,
/// 3840x2160_30 = 33).
pub fn disp_timing(t: Timing) -> u32 {
    use ffi::MI_DISP_OutputTiming_e as e;
    use Timing::*;
    match t {
        P480_60 => e::E_MI_DISP_OUTPUT_480P60,
        P576_50 => e::E_MI_DISP_OUTPUT_576P50,
        P720_50 => e::E_MI_DISP_OUTPUT_720P50,
        P720_60 => e::E_MI_DISP_OUTPUT_720P60,
        P1080_24 => e::E_MI_DISP_OUTPUT_1080P24,
        P1080_25 => e::E_MI_DISP_OUTPUT_1080P25,
        P1080_30 => e::E_MI_DISP_OUTPUT_1080P30,
        P1080_50 => e::E_MI_DISP_OUTPUT_1080P50,
        P1080_60 => e::E_MI_DISP_OUTPUT_1080P60,
        P1440_30 => e::E_MI_DISP_OUTPUT_2560x1440_30,
        P1440_50 => e::E_MI_DISP_OUTPUT_2560x1440_50,
        P1440_60 => e::E_MI_DISP_OUTPUT_2560x1440_60,
        P2160_25 => e::E_MI_DISP_OUTPUT_3840x2160_25,
        P2160_30 => e::E_MI_DISP_OUTPUT_3840x2160_30,
    }
}

fn win(r: Rect) -> ffi::MI_DISP_VidWinRect_t {
    ffi::MI_DISP_VidWinRect_t { u16X: r.x as u16, u16Y: r.y as u16, u16Width: r.w as u16, u16Height: r.h as u16 }
}

/// The vendor's background: U/V 0x80, Y 0 — black on a YUV display.
const BG: u32 = 0x800080;

/// Bring the display up for `timing`: device, video layer the size of the screen, one input port showing `src`-sized
/// frames in `window`, free-running sync (microcamd's: the whole screen, the HDMI pool's canvases 1:1).
pub fn up(dev: u32, layer: u32, port: u32, timing: Timing, window: Rect, src: Size) -> Result<()> {
    let (dev, layer) = (dev as i32, layer as i32); // MI_DISP_DEV / MI_DISP_LAYER are MI_S32
    let screen = timing.size();
    let mut pub_attr: ffi::MI_DISP_PubAttr_t = Default::default();
    pub_attr.u32BgColor = BG;
    pub_attr.eIntfType = ffi::MI_DISP_Interface_e::E_MI_DISP_INTF_HDMI;
    pub_attr.eIntfSync = disp_timing(timing);
    check("MI_DISP_SetPubAttr", unsafe { ffi::MI_DISP_SetPubAttr(dev, &pub_attr) })?;
    check("MI_DISP_Enable", unsafe { ffi::MI_DISP_Enable(dev) })?;
    check("MI_DISP_BindVideoLayer", unsafe { ffi::MI_DISP_BindVideoLayer(layer, dev) })?;
    let mut la: ffi::MI_DISP_VideoLayerAttr_t = Default::default();
    la.stVidLayerDispWin = win(Rect::full(screen));
    la.stVidLayerSize = ffi::MI_DISP_VideoLayerSize_t { u16Width: screen.w, u16Height: screen.h };
    la.ePixFormat = PIXEL_NV12;
    check("MI_DISP_SetVideoLayerAttr", unsafe { ffi::MI_DISP_SetVideoLayerAttr(layer, &la) })?;
    check("MI_DISP_EnableVideoLayer", unsafe { ffi::MI_DISP_EnableVideoLayer(layer) })?;
    let pa = ffi::MI_DISP_InputPortAttr_t { stDispWin: win(window), u16SrcWidth: src.w as u16, u16SrcHeight: src.h as u16 };
    check("MI_DISP_SetInputPortAttr", unsafe { ffi::MI_DISP_SetInputPortAttr(layer, port, &pa) })?;
    check("MI_DISP_EnableInputPort", unsafe { ffi::MI_DISP_EnableInputPort(layer, port) })?;
    check("MI_DISP_SetInputPortSyncMode", unsafe {
        ffi::MI_DISP_SetInputPortSyncMode(layer, port, ffi::MI_DISP_SyncMode_e::E_MI_DISP_SYNC_MODE_FREE_RUN)
    })
}

/// Reverse of [`up`]. Every step is attempted; the first error is returned.
pub fn down(dev: u32, layer: u32, port: u32) -> Result<()> {
    let (dev, layer) = (dev as i32, layer as i32);
    let r = [
        check("MI_DISP_DisableInputPort", unsafe { ffi::MI_DISP_DisableInputPort(layer, port) }),
        check("MI_DISP_DisableVideoLayer", unsafe { ffi::MI_DISP_DisableVideoLayer(layer) }),
        check("MI_DISP_UnBindVideoLayer", unsafe { ffi::MI_DISP_UnBindVideoLayer(layer, dev) }),
        check("MI_DISP_Disable", unsafe { ffi::MI_DISP_Disable(dev) }),
    ];
    r.into_iter().collect::<Result<Vec<_>>>().map(|_| ())
}
