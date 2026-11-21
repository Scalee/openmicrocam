//! Picture knobs on the ISP (`microcam_core::tuning`). The IQ-relative knobs are computed from a copy of each ISP
//! structure as the IQ file left it (the [`Baseline`], read after every IQ load) and change every ISO row of it, so the
//! IQ's gain-dependent tuning keeps working and a knob turned back to its default restores the IQ's own values.
//! Exposure, white balance and flicker are read-modify-write on the 3A attributes.

use microcam_core::tuning::{self, Adjust, ExposureNow, Knob, WbManual, WhiteBalanceNow, AS_TUNED, GAIN_UNIT};
use mi_sys as ffi;

use crate::{check, Result};

/// The ISP's saturation scale: 32 = 1×.
const SAT_UNITY: u8 = 32;
const SAT_MAX: u8 = 127;
const SHARPNESS_MAX: u8 = 127;
/// AE and AWB gains are in 1/1024.
const GAIN_UNITY: u32 = 1024;
const ISP_GAIN_UNIT: i32 = 1024;

macro_rules! get {
    ($f:ident, $dev:expr, $chn:expr, $t:ty) => {{
        let mut v: Box<$t> = Box::default();
        check(stringify!($f), unsafe { ffi::$f($dev, $chn, &mut *v) }).map(|_| v)
    }};
}

/// The IQ file's values of every structure a knob changes.
/// The IQ file's mains half-cycle in us (the AE's anti-flicker step): 10000 at 50 Hz, 8333 at 60 Hz, None when off.
pub fn flicker_half_us(b: &Baseline) -> Option<u32> {
    use ffi::MI_ISP_AE_FLICKER_TYPE_e::*;
    match b.flicker {
        SS_AE_FLICKER_TYPE_50HZ => Some(10000),
        SS_AE_FLICKER_TYPE_60HZ => Some(8333),
        _ => None,
    }
}

pub struct Baseline {
    brightness: Box<ffi::MI_ISP_IQ_BRIGHTNESS_TYPE_t>,
    contrast: Box<ffi::MI_ISP_IQ_CONTRAST_TYPE_t>,
    saturation: Box<ffi::MI_ISP_IQ_SATURATION_TYPE_t>,
    sharpness: Box<ffi::MI_ISP_IQ_SHARPNESS_TYPE_t>,
    gamma: Box<ffi::MI_ISP_IQ_RGBGAMMA_TYPE_t>,
    wdr: Box<ffi::MI_ISP_IQ_WDR_TYPE_t>,
    flicker: ffi::MI_ISP_AE_FLICKER_TYPE_e::Type,
}

impl Baseline {
    pub fn read(dev: u32, chn: u32) -> Result<Self> {
        let mut flicker = 0;
        check("MI_ISP_AE_GetFlicker", unsafe { ffi::MI_ISP_AE_GetFlicker(dev, chn, &mut flicker) })?;
        Ok(Self {
            brightness: get!(MI_ISP_IQ_GetBrightness, dev, chn, ffi::MI_ISP_IQ_BRIGHTNESS_TYPE_t)?,
            contrast: get!(MI_ISP_IQ_GetContrast, dev, chn, ffi::MI_ISP_IQ_CONTRAST_TYPE_t)?,
            saturation: get!(MI_ISP_IQ_GetSaturation, dev, chn, ffi::MI_ISP_IQ_SATURATION_TYPE_t)?,
            sharpness: get!(MI_ISP_IQ_GetSharpness, dev, chn, ffi::MI_ISP_IQ_SHARPNESS_TYPE_t)?,
            gamma: get!(MI_ISP_IQ_GetRGBGamma, dev, chn, ffi::MI_ISP_IQ_RGBGAMMA_TYPE_t)?,
            wdr: get!(MI_ISP_IQ_GetWDR, dev, chn, ffi::MI_ISP_IQ_WDR_TYPE_t)?,
            flicker,
        })
    }

    /// The untouched values of the knobs whose default is a hardware state.
    pub fn hardware_defaults(&self) -> Vec<(Knob, i32)> {
        vec![(Knob::Backlight, (self.wdr.bEnable != 0) as i32)]
    }

    /// One line for the log: what the IQ file enables.
    pub fn describe(&self) -> String {
        format!(
            "IQ baseline: brightness {}/{} contrast {}/{} saturation {} sharpness {} gamma {} wdr {} flicker {}",
            self.brightness.bEnable,
            self.brightness.stManual.stParaAPI.u32Lev,
            self.contrast.bEnable,
            self.contrast.stManual.stParaAPI.u32Lev,
            self.saturation.bEnable,
            self.sharpness.bEnable,
            self.gamma.bEnable,
            self.wdr.bEnable,
            self.flicker
        )
    }
}

/// Apply one picture operation.
pub fn apply(dev: u32, chn: u32, base: &Baseline, a: &Adjust) -> Result<()> {
    match *a {
        Adjust::Iq { knob, value } => apply_iq(dev, chn, base, knob, value),
        Adjust::PowerLine { value } => {
            use ffi::MI_ISP_AE_FLICKER_TYPE_e::*;
            let mut f = match value {
                0 => SS_AE_FLICKER_TYPE_DISABLE,
                2 => SS_AE_FLICKER_TYPE_60HZ,
                _ => SS_AE_FLICKER_TYPE_50HZ,
            };
            check("MI_ISP_AE_SetFlicker", unsafe { ffi::MI_ISP_AE_SetFlicker(dev, chn, &mut f) })
        }
        Adjust::Exposure { auto, time_100us, gain_pct } => {
            use ffi::MI_ISP_AE_MODE_TYPE_e::*;
            if auto {
                let mut m = SS_AE_MODE_A;
                return check("MI_ISP_AE_SetExpoMode", unsafe { ffi::MI_ISP_AE_SetExpoMode(dev, chn, &mut m) });
            }
            // the aperture field stays as the AE has it (a fixed lens); the whole gain goes to the sensor
            let mut e = ffi::MI_ISP_AE_EXPO_VALUE_TYPE_t::default();
            check("MI_ISP_AE_GetManualExpo", unsafe { ffi::MI_ISP_AE_GetManualExpo(dev, chn, &mut e) })?;
            e.u32US = time_100us as u32 * 100;
            e.u32SensorGain = gain_pct as u32 * GAIN_UNITY / 100;
            e.u32ISPGain = GAIN_UNITY;
            let mut m = SS_AE_MODE_M;
            check("MI_ISP_AE_SetExpoMode", unsafe { ffi::MI_ISP_AE_SetExpoMode(dev, chn, &mut m) })?;
            check("MI_ISP_AE_SetManualExpo", unsafe { ffi::MI_ISP_AE_SetManualExpo(dev, chn, &mut e) })
        }
        Adjust::ExposureBias { tenths } => {
            // EV = s32EV / u32Grad (SigmaStar AE tuning: "EVComp −2, Grad 3")
            let mut ev = ffi::MI_ISP_AE_EV_COMP_TYPE_t { s32EV: tenths, u32Grad: 10 };
            check("MI_ISP_AE_SetEVComp", unsafe { ffi::MI_ISP_AE_SetEVComp(dev, chn, &mut ev) })
        }
        Adjust::WhiteBalance { auto, manual } => {
            use ffi::MI_ISP_AWB_MODE_TYPE_e::*;
            if let (false, WbManual::Kelvin(k)) = (auto, manual) {
                let mut ct = ffi::MI_ISP_AWB_CTMWB_PARAM_t { u32CT: k as u32 };
                check("MI_ISP_AWB_SetCTMwbAttr", unsafe { ffi::MI_ISP_AWB_SetCTMwbAttr(dev, chn, &mut ct) })?;
            }
            let mut attr: Box<ffi::MI_ISP_AWB_ATTR_TYPE_t> = get!(MI_ISP_AWB_GetAttr, dev, chn, ffi::MI_ISP_AWB_ATTR_TYPE_t)?;
            attr.eOpType = match (auto, manual) {
                (true, _) => SS_AWB_MODE_AUTO,
                (false, WbManual::Kelvin(_)) => SS_AWB_MODE_CTMANUAL,
                (false, WbManual::Gains(r, g, b)) => {
                    let isp = |v: i32| (v * ISP_GAIN_UNIT / GAIN_UNIT) as u16;
                    let m = &mut attr.stManualParaAPI;
                    m.u16Rgain = isp(r);
                    m.u16Grgain = isp(g);
                    m.u16Gbgain = isp(g);
                    m.u16Bgain = isp(b);
                    SS_AWB_MODE_MANUAL
                }
            };
            check("MI_ISP_AWB_SetAttr", unsafe { ffi::MI_ISP_AWB_SetAttr(dev, chn, &mut *attr) })
        }
    }
}

fn apply_iq(dev: u32, chn: u32, base: &Baseline, knob: Knob, value: i32) -> Result<()> {
    let untouched = value == knob.range().default;
    macro_rules! level {
        ($field:ident, $set:ident) => {{
            let mut s = base.$field.clone();
            if !untouched {
                let on = s.bEnable != 0;
                for row in s.stAuto.stParaAPI.iter_mut().chain(std::iter::once(&mut s.stManual.stParaAPI)) {
                    row.u32Lev = tuning::shift_level(row.u32Lev, on, value);
                }
                s.bEnable = 1;
            }
            check(stringify!($set), unsafe { ffi::$set(dev, chn, &mut *s) })
        }};
    }
    match knob {
        Knob::Brightness => level!(brightness, MI_ISP_IQ_SetBrightness),
        Knob::Contrast => level!(contrast, MI_ISP_IQ_SetContrast),
        Knob::Saturation => {
            let mut s = base.saturation.clone();
            if !untouched {
                let on = s.bEnable != 0;
                for row in s.stAuto.stParaAPI.iter_mut().chain(std::iter::once(&mut s.stManual.stParaAPI)) {
                    let iq = if on { row.u8SatAllStr } else { SAT_UNITY };
                    row.u8SatAllStr = tuning::scale_strength(iq, value, SAT_MAX);
                }
                s.bEnable = 1;
            }
            check("MI_ISP_IQ_SetSaturation", unsafe { ffi::MI_ISP_IQ_SetSaturation(dev, chn, &mut *s) })
        }
        Knob::Sharpness => {
            let mut s = base.sharpness.clone();
            if !untouched {
                if s.bEnable == 0 {
                    log::warn!("sharpness: the IQ file leaves edge enhancement off; scaling its zero strengths changes nothing");
                }
                for row in s.stAuto.stParaAPI.iter_mut().chain(std::iter::once(&mut s.stManual.stParaAPI)) {
                    for v in row.u8SharpnessUD.iter_mut().chain(row.u8SharpnessD.iter_mut()) {
                        *v = tuning::scale_strength(*v, value, SHARPNESS_MAX);
                    }
                }
            }
            check("MI_ISP_IQ_SetSharpness", unsafe { ffi::MI_ISP_IQ_SetSharpness(dev, chn, &mut *s) })
        }
        Knob::Gamma => {
            let mut s = base.gamma.clone();
            if !untouched {
                // a disabled gamma structure still holds the IQ file's curve, so enabling it with the warped copy is right
                for row in s.stAuto.stParaAPI.iter_mut().chain(std::iter::once(&mut s.stManual.stParaAPI)) {
                    row.u16LutR = tuning::warp_gamma(&row.u16LutR, value);
                    row.u16LutG = tuning::warp_gamma(&row.u16LutG, value);
                    row.u16LutB = tuning::warp_gamma(&row.u16LutB, value);
                }
                s.bEnable = 1;
            }
            check("MI_ISP_IQ_SetRGBGamma", unsafe { ffi::MI_ISP_IQ_SetRGBGamma(dev, chn, &mut *s) })
        }
        Knob::Grey => {
            let mut g = ffi::MI_ISP_IQ_COLORTOGRAY_TYPE_t { bEnable: (value != 0) as u32 };
            check("MI_ISP_IQ_SetColorToGray", unsafe { ffi::MI_ISP_IQ_SetColorToGray(dev, chn, &mut g) })
        }
        Knob::Backlight => {
            let mut s = base.wdr.clone();
            if value != AS_TUNED {
                s.bEnable = (value != 0) as u32;
            }
            check("MI_ISP_IQ_SetWDR", unsafe { ffi::MI_ISP_IQ_SetWDR(dev, chn, &mut *s) })
        }
        other => unreachable!("{other:?} is not an IQ knob"),
    }
}

/// The exposure the AE runs right now, in knob units.
pub fn exposure_now(dev: u32, chn: u32) -> Result<ExposureNow> {
    let i: Box<ffi::MI_ISP_AE_EXPO_INFO_TYPE_t> = get!(MI_ISP_AE_QueryExposureInfo, dev, chn, ffi::MI_ISP_AE_EXPO_INFO_TYPE_t)?;
    let e = i.stExpoValueLong;
    let total = e.u32SensorGain as u64 * e.u32ISPGain as u64 / GAIN_UNITY as u64;
    Ok(ExposureNow { time_100us: (e.u32US / 100).max(1) as i32, gain_pct: (total * 100 / GAIN_UNITY as u64) as i32 })
}

/// What the AWB runs right now: the gains it applies (red, green, blue in 1/32) and the colour temperature it estimates.
pub fn white_balance_now(dev: u32, chn: u32) -> Result<WhiteBalanceNow> {
    let q: Box<ffi::MI_ISP_AWB_QUERY_INFO_TYPE_t> = get!(MI_ISP_AWB_QueryInfo, dev, chn, ffi::MI_ISP_AWB_QUERY_INFO_TYPE_t)?;
    let menu = |v: u16| v as i32 * GAIN_UNIT / ISP_GAIN_UNIT;
    Ok(WhiteBalanceNow { gains: (menu(q.u16Rgain), menu(q.u16Grgain), menu(q.u16Bgain)), kelvin: q.u16ColorTemp as i32 })
}
