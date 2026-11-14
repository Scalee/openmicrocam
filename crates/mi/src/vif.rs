//! MI_VIF: the MIPI receiver group, device and output port (the SDK demo's ST_VideoModuleInit).

use mi_sys as ffi;

use crate::sensor::PlaneInfo;
use crate::{check, rect, size, Result};

pub fn create_group(group: u32) -> Result<()> {
    let mut a: ffi::MI_VIF_GroupAttr_t = Default::default();
    a.eIntfMode = ffi::MI_VIF_IntfMode_e::E_MI_VIF_MODE_MIPI;
    a.eWorkMode = ffi::MI_VIF_WorkMode_e::E_MI_VIF_WORK_MODE_1MULTIPLEX;
    a.eHDRType = ffi::MI_VIF_HDRType_e::E_MI_VIF_HDR_TYPE_OFF;
    a.eClkEdge = ffi::MI_VIF_ClkEdge_e::E_MI_VIF_CLK_EDGE_DOUBLE;
    check("MI_VIF_CreateDevGroup", unsafe { ffi::MI_VIF_CreateDevGroup(group, &mut a) })
}

pub fn destroy_group(group: u32) -> Result<()> {
    check("MI_VIF_DestroyDevGroup", unsafe { ffi::MI_VIF_DestroyDevGroup(group) })
}

pub fn set_dev_attr(dev: u32, plane: &PlaneInfo) -> Result<()> {
    let mut a: ffi::MI_VIF_DevAttr_t = Default::default();
    a.eInputPixel = plane.pixel;
    a.stInputRect = rect(plane.cap);
    a.eField = ffi::MI_SYS_FieldType_e::E_MI_SYS_FIELDTYPE_NONE;
    a.bEnH2T1PMode = 0;
    check("MI_VIF_SetDevAttr", unsafe { ffi::MI_VIF_SetDevAttr(dev, &mut a) })
}

pub fn enable_dev(dev: u32) -> Result<()> {
    check("MI_VIF_EnableDev", unsafe { ffi::MI_VIF_EnableDev(dev) })
}

pub fn disable_dev(dev: u32) -> Result<()> {
    check("MI_VIF_DisableDev", unsafe { ffi::MI_VIF_DisableDev(dev) })
}

pub fn set_port_attr(dev: u32, port: u32, plane: &PlaneInfo) -> Result<()> {
    let mut a: ffi::MI_VIF_OutputPortAttr_t = Default::default();
    a.stCapRect = rect(plane.cap);
    a.stDestSize = size(plane.cap.size());
    a.ePixFormat = plane.pixel;
    a.eFrameRate = ffi::MI_VIF_FrameRate_e::E_MI_VIF_FRAMERATE_FULL;
    check("MI_VIF_SetOutputPortAttr", unsafe { ffi::MI_VIF_SetOutputPortAttr(dev, port, &mut a) })
}

pub fn enable_port(dev: u32, port: u32) -> Result<()> {
    check("MI_VIF_EnableOutputPort", unsafe { ffi::MI_VIF_EnableOutputPort(dev, port) })
}

pub fn disable_port(dev: u32, port: u32) -> Result<()> {
    check("MI_VIF_DisableOutputPort", unsafe { ffi::MI_VIF_DisableOutputPort(dev, port) })
}

/// Frame interrupts the VIF port has taken so far (`MI_VIF_Query().u32IntCnt`, as the vendor app reads it before it
/// starts USB).
pub fn interrupts(dev: u32, port: u32) -> Result<u32> {
    let mut st = ffi::MI_VIF_DevPortStat_t::default();
    check("MI_VIF_Query", unsafe { ffi::MI_VIF_Query(dev, port, &mut st) })?;
    Ok(st.u32IntCnt)
}
