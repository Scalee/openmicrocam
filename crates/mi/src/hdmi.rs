//! MI_HDMI: the transmitter — init with the hot-plug callback, open/attr/start, EDID, connection state.
//!
//! One rule from the camera: closing any fd of `/dev/mi_hdmi` stops HDMI, so `MI_HDMI_Init` happens
//! once per process and is never undone while the daemon runs; up/down use Open/Start/Stop/Close only.

use std::ffi::c_void;

use microcam_core::hdmi::Timing;
use mi_sys as ffi;

use crate::{check, Result};

const DEV: u32 = ffi::MI_HDMI_DeviceId_e::E_MI_HDMI_ID_0;

/// `MI_HDMI_TimingType_e` of this build (4K30 = 13 and 1080p60 = 9 are what the vendor app sets).
pub fn hdmi_timing(t: Timing) -> u32 {
    use ffi::MI_HDMI_TimingType_e as e;
    use Timing::*;
    match t {
        P480_60 => e::E_MI_HDMI_TIMING_480_60P,
        P576_50 => e::E_MI_HDMI_TIMING_576_50P,
        P720_50 => e::E_MI_HDMI_TIMING_720_50P,
        P720_60 => e::E_MI_HDMI_TIMING_720_60P,
        P1080_24 => e::E_MI_HDMI_TIMING_1080_24P,
        P1080_25 => e::E_MI_HDMI_TIMING_1080_25P,
        P1080_30 => e::E_MI_HDMI_TIMING_1080_30P,
        P1080_50 => e::E_MI_HDMI_TIMING_1080_50P,
        P1080_60 => e::E_MI_HDMI_TIMING_1080_60P,
        P1440_30 => e::E_MI_HDMI_TIMING_1440_30P,
        P1440_50 => e::E_MI_HDMI_TIMING_1440_50P,
        P1440_60 => e::E_MI_HDMI_TIMING_1440_60P,
        P2160_25 => e::E_MI_HDMI_TIMING_4K2K_25P,
        P2160_30 => e::E_MI_HDMI_TIMING_4K2K_30P,
    }
}

/// Hot-plug events from the transmitter's own thread.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    Plugged,
    Unplugged,
}

/// Registered once; the callback is called on the library's thread with `Event`s.
pub type EventSink = Box<dyn Fn(Event) + Send + Sync>;

unsafe extern "C" fn trampoline(_dev: u32, event: u32, _param: *mut c_void, user: *mut c_void) -> i32 {
    // SAFETY: `user` is the Box<EventSink> leaked by `init` for the life of the process.
    let sink = unsafe { &*(user as *const EventSink) };
    match event {
        ffi::MI_HDMI_EventType_e::E_MI_HDMI_EVENT_HOTPLUG => sink(Event::Plugged),
        ffi::MI_HDMI_EventType_e::E_MI_HDMI_EVENT_NO_PLUG => sink(Event::Unplugged),
        _ => {}
    }
    0
}

/// `MI_HDMI_Init` with the hot-plug callback. Call once per process.
pub fn init(sink: EventSink) -> Result<()> {
    let user = Box::into_raw(Box::new(sink)) as *mut c_void;
    let mut p = ffi::MI_HDMI_InitParam_t { pfnHdmiEventCallback: Some(trampoline), pCallBackArgs: user };
    check("MI_HDMI_Init", unsafe { ffi::MI_HDMI_Init(&mut p) })
}

pub fn open() -> Result<()> {
    check("MI_HDMI_Open", unsafe { ffi::MI_HDMI_Open(DEV) })
}

pub fn close() -> Result<()> {
    check("MI_HDMI_Close", unsafe { ffi::MI_HDMI_Close(DEV) })
}

/// Video on at `timing`, YCbCr 4:4:4 out, 24-bit colour, no audio (the vendor app's settings).
/// Starts from the module's stored attributes like the vendor: the input colour type stays the module's own
/// (RGB 4:4:4); declaring YCbCr input skips the transmitter's conversion and the
/// monitor shows white as pink, grey as green.
pub fn set_timing(timing: Timing) -> Result<()> {
    let mut a: ffi::MI_HDMI_Attr_t = Default::default();
    check("MI_HDMI_GetAttr", unsafe { ffi::MI_HDMI_GetAttr(DEV, &mut a) })?;
    a.stVideoAttr.bEnableVideo = 1;
    a.stVideoAttr.eTimingType = hdmi_timing(timing);
    a.stVideoAttr.eOutputMode = ffi::MI_HDMI_OutputMode_e::E_MI_HDMI_OUTPUT_MODE_HDMI;
    a.stVideoAttr.eColorType = ffi::MI_HDMI_ColorType_e::E_MI_HDMI_COLOR_TYPE_YCBCR444;
    a.stVideoAttr.eDeepColorMode = ffi::MI_HDMI_DeepColor_e::E_MI_HDMI_DEEP_COLOR_24BIT;
    a.stAudioAttr.bEnableAudio = 0;
    check("MI_HDMI_SetAttr", unsafe { ffi::MI_HDMI_SetAttr(DEV, &mut a) })
}

pub fn start() -> Result<()> {
    check("MI_HDMI_Start", unsafe { ffi::MI_HDMI_Start(DEV) })
}

pub fn stop() -> Result<()> {
    check("MI_HDMI_Stop", unsafe { ffi::MI_HDMI_Stop(DEV) })
}

/// `MI_HDMI_GetAttr().bConnect` — the vendor polled this every 500 ms.
pub fn connected() -> Result<bool> {
    let mut a: ffi::MI_HDMI_Attr_t = Default::default();
    check("MI_HDMI_GetAttr", unsafe { ffi::MI_HDMI_GetAttr(DEV, &mut a) })?;
    Ok(a.bConnect != 0)
}

/// The monitor's EDID bytes, if the read succeeded and the library marks them valid.
pub fn edid() -> Result<Option<Vec<u8>>> {
    let mut e: ffi::MI_HDMI_Edid_t = Default::default();
    check("MI_HDMI_ForceGetEdid", unsafe { ffi::MI_HDMI_ForceGetEdid(DEV, &mut e) })?;
    if e.bEdidValid == 0 || e.u32Edidlength == 0 {
        return Ok(None);
    }
    let n = (e.u32Edidlength as usize).min(e.au8Edid.len());
    Ok(Some(e.au8Edid[..n].to_vec()))
}
