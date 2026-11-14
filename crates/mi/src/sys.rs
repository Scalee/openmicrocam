//! MI_SYS: process init, version, binds, port depths.

use microcam_core::reconcile::{BindKind, ChnPort, Module};
use mi_sys as ffi;

use crate::{check, Result};

const SOC: u16 = 0;

pub fn init() -> Result<()> {
    // SAFETY: plain call, no pointers.
    check("MI_SYS_Init", unsafe { ffi::MI_SYS_Init(SOC) })
}

pub fn exit() -> Result<()> {
    check("MI_SYS_Exit", unsafe { ffi::MI_SYS_Exit(SOC) })
}

/// The media clock now, in µs — the clock frames are stamped with at capture.
pub fn now_pts() -> u64 {
    let mut pts = 0;
    // SAFETY: the library writes one u64 we own.
    unsafe { ffi::MI_SYS_GetCurPts(SOC, &mut pts) };
    pts
}

pub fn version() -> Result<String> {
    let mut v: ffi::MI_SYS_Version_t = Default::default();
    // SAFETY: the library fills a plain 128-byte struct we own.
    check("MI_SYS_GetVersion", unsafe { ffi::MI_SYS_GetVersion(SOC, &mut v) })?;
    let end = v.u8Version.iter().position(|&b| b == 0).unwrap_or(v.u8Version.len());
    Ok(String::from_utf8_lossy(&v.u8Version[..end]).into_owned())
}

pub(crate) fn module_id(m: Module) -> u32 {
    use ffi::MI_ModuleId_e as id;
    match m {
        Module::Vif => id::E_MI_MODULE_ID_VIF,
        Module::Isp => id::E_MI_MODULE_ID_ISP,
        Module::Scl => id::E_MI_MODULE_ID_SCL,
        Module::Venc => id::E_MI_MODULE_ID_VENC,
        Module::Disp => id::E_MI_MODULE_ID_DISP,
        Module::Vdec => id::E_MI_MODULE_ID_VDEC,
    }
}

pub(crate) fn chn_port(p: ChnPort) -> ffi::MI_SYS_ChnPort_t {
    ffi::MI_SYS_ChnPort_t { eModId: module_id(p.module), u32DevId: p.dev, u32ChnId: p.chn, u32PortId: p.port }
}

fn bind_type(k: BindKind) -> u32 {
    match k {
        BindKind::FrameBase => ffi::MI_SYS_BindType_e::E_MI_SYS_BIND_TYPE_FRAME_BASE,
        BindKind::Realtime => ffi::MI_SYS_BindType_e::E_MI_SYS_BIND_TYPE_REALTIME,
    }
}

pub fn bind(src: ChnPort, dst: ChnPort, kind: BindKind, src_fps: u32, dst_fps: u32) -> Result<()> {
    let mut s = chn_port(src);
    let mut d = chn_port(dst);
    // SAFETY: both structs are owned locals, read by the call only.
    check("MI_SYS_BindChnPort2", unsafe { ffi::MI_SYS_BindChnPort2(SOC, &mut s, &mut d, src_fps, dst_fps, bind_type(kind), 0) })
}

/// `src` (an SCL port) bound into `dst` (an H.26x encoder's input) in a ring of `lines` lines: the encoder reads
/// behind the port's write (E_MI_SYS_BIND_TYPE_HW_RING, the bind's parameter the ring's height), `dst_fps` of the
/// port's `src_fps` frames a second.
pub fn bind_ring(src: ChnPort, dst: ChnPort, src_fps: u32, dst_fps: u32, lines: u32) -> Result<()> {
    let mut s = chn_port(src);
    let mut d = chn_port(dst);
    // SAFETY: both structs are owned locals, read by the call only.
    check("MI_SYS_BindChnPort2", unsafe {
        ffi::MI_SYS_BindChnPort2(SOC, &mut s, &mut d, src_fps, dst_fps, ffi::MI_SYS_BindType_e::E_MI_SYS_BIND_TYPE_HW_RING, lines)
    })
}

pub fn unbind(src: ChnPort, dst: ChnPort) -> Result<()> {
    let mut s = chn_port(src);
    let mut d = chn_port(dst);
    check("MI_SYS_UnBindChnPort", unsafe { ffi::MI_SYS_UnBindChnPort(SOC, &mut s, &mut d) })
}

pub fn set_depth(port: ChnPort, user: u32, buffers: u32) -> Result<()> {
    let mut p = chn_port(port);
    check("MI_SYS_SetChnOutputPortDepth", unsafe { ffi::MI_SYS_SetChnOutputPortDepth(SOC, &mut p, user, buffers) })
}
