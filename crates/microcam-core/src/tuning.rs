//! Picture knobs: what the controls (`crate::controls`) ask of the ISP. Every knob is defined relative to the IQ file's
//! own tuning, so a knob's default always means "as the IQ file tunes it" and the IQ's per-gain tables keep working:
//! levels shift the IQ's levels (50 = unchanged), strengths scale them (50 = unchanged, 100 = twice).
//!
//! Every knob has a value: the controls list hands the planner all of them, and after every IQ load (which resets them
//! all) every one goes to the ISP again. The planner turns them into [`Adjust`] operations; the hardware layer knows how
//! each one maps onto the ISP structures.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Knob {
    /// tone-curve brightness level, 50 = as tuned
    Brightness,
    /// tone-curve contrast level, 50 = as tuned
    Contrast,
    /// colour strength: 50 = as tuned, 0 none, 100 twice the tuned strength
    Saturation,
    /// edge enhancement: 50 = as tuned, 0 none, 100 twice the tuned strength
    Sharpness,
    /// gamma × 100 applied on top of the tuned gamma curve (100 = as tuned, more = brighter mid-tones)
    Gamma,
    /// wide dynamic range: 0 off, 1 on, [`AS_TUNED`] as the IQ file has it
    Backlight,
    /// 1 = black and white
    Grey,
    /// 0 off, 1 = 50 Hz, 2 = 60 Hz (UVC's encoding)
    PowerLine,
    /// 1 auto exposure, 0 manual
    AutoExposure,
    /// manual exposure time in 100 µs units (UVC's unit)
    Exposure,
    /// manual total gain in % (100 = 1×)
    Gain,
    /// auto-exposure compensation in tenths of an EV, by thirds of an EV
    ExposureBias,
    /// 1 auto white balance, 0 manual
    AutoWhiteBalance,
    /// manual white balance colour temperature in kelvin
    WhiteBalance,
    /// manual white balance set by channel gains instead of a temperature (the white-balance lock's)
    WhiteBalanceByGains,
    /// manual channel gains in 1/32 (32 = 1×)
    RedGain,
    GreenGain,
    BlueGain,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Range {
    pub min: i32,
    pub max: i32,
    pub step: i32,
    /// the default: for an IQ-relative knob the value meaning "as the IQ file tunes it"; for backlight, whose default is
    /// the IQ file's state (read after the load, `crate::controls::List::hardware`), only the answer before it is read
    pub default: i32,
}

/// Channel gains of the gain-based manual white balance are in these units of 1×.
pub const GAIN_UNIT: i32 = 32;

/// The backlight knob as the IQ file has it: the wide dynamic range's state lives in the IQ file and is read only after
/// the load the knobs follow, so a backlight the user has not set goes to the ISP as this, not as a guess.
pub const AS_TUNED: i32 = -1;

impl Knob {
    pub fn range(self) -> Range {
        let r = |min, max, step, default| Range { min, max, step, default };
        match self {
            Knob::Brightness | Knob::Contrast | Knob::Saturation | Knob::Sharpness => r(0, 100, 1, 50),
            Knob::Gamma => r(50, 300, 1, 100),
            Knob::Backlight | Knob::Grey | Knob::WhiteBalanceByGains => r(0, 1, 1, 0),
            Knob::PowerLine => r(0, 2, 1, 1),
            Knob::AutoExposure | Knob::AutoWhiteBalance => r(0, 1, 1, 1),
            // up to one second: the sensor clamps the exposure to its frame period
            Knob::Exposure => r(1, 10_000, 1, 333),
            Knob::Gain => r(100, 3200, 1, 100),
            Knob::ExposureBias => r(-30, 30, 3, 0),
            Knob::WhiteBalance => r(2800, 6500, 10, 5000),
            Knob::RedGain | Knob::GreenGain | Knob::BlueGain => r(0, 255, 1, GAIN_UNIT),
        }
    }

    pub fn clamp(self, v: i32) -> i32 {
        let r = self.range();
        v.clamp(r.min, r.max)
    }

    /// The knob's name as the status shows it (the same as its serde name).
    pub fn name(self) -> &'static str {
        match self {
            Knob::Brightness => "brightness",
            Knob::Contrast => "contrast",
            Knob::Saturation => "saturation",
            Knob::Sharpness => "sharpness",
            Knob::Gamma => "gamma",
            Knob::Backlight => "backlight",
            Knob::Grey => "grey",
            Knob::PowerLine => "power-line",
            Knob::AutoExposure => "auto-exposure",
            Knob::Exposure => "exposure",
            Knob::Gain => "gain",
            Knob::ExposureBias => "exposure-bias",
            Knob::AutoWhiteBalance => "auto-white-balance",
            Knob::WhiteBalance => "white-balance",
            Knob::WhiteBalanceByGains => "white-balance-by-gains",
            Knob::RedGain => "red-gain",
            Knob::GreenGain => "green-gain",
            Knob::BlueGain => "blue-gain",
        }
    }
}

/// Knob values: the controls list's, every knob (what is asked of the ISP), or those in effect on it.
pub type Tuning = BTreeMap<Knob, i32>;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum WbManual {
    Kelvin(i32),
    /// red, green, blue in 1/32
    Gains(i32, i32, i32),
}

/// One change to the ISP. The exposure and white-balance knobs act together, so each group is one operation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Adjust {
    /// an IQ-relative knob (brightness, contrast, saturation, sharpness, gamma, backlight, grey)
    Iq { knob: Knob, value: i32 },
    PowerLine { value: i32 },
    Exposure { auto: bool, time_100us: i32, gain_pct: i32 },
    ExposureBias { tenths: i32 },
    WhiteBalance { auto: bool, manual: WbManual },
}

const IQ_KNOBS: [Knob; 7] = [Knob::Brightness, Knob::Contrast, Knob::Saturation, Knob::Sharpness, Knob::Gamma, Knob::Backlight, Knob::Grey];
const EXPOSURE: [Knob; 3] = [Knob::AutoExposure, Knob::Exposure, Knob::Gain];
const WHITE_BALANCE: [Knob; 6] =
    [Knob::AutoWhiteBalance, Knob::WhiteBalance, Knob::WhiteBalanceByGains, Knob::RedGain, Knob::GreenGain, Knob::BlueGain];

impl Adjust {
    /// The knob values this operation puts into effect (what `Current` records once it ran).
    pub fn knobs(&self) -> Vec<(Knob, i32)> {
        match *self {
            Adjust::Iq { knob, value } => vec![(knob, value)],
            Adjust::PowerLine { value } => vec![(Knob::PowerLine, value)],
            Adjust::Exposure { auto, time_100us, gain_pct } => {
                vec![(Knob::AutoExposure, auto as i32), (Knob::Exposure, time_100us), (Knob::Gain, gain_pct)]
            }
            Adjust::ExposureBias { tenths } => vec![(Knob::ExposureBias, tenths)],
            Adjust::WhiteBalance { auto, manual } => {
                let mut v = vec![(Knob::AutoWhiteBalance, auto as i32)];
                match manual {
                    WbManual::Kelvin(k) => v.extend([(Knob::WhiteBalanceByGains, 0), (Knob::WhiteBalance, k)]),
                    WbManual::Gains(r, g, b) => {
                        v.extend([(Knob::WhiteBalanceByGains, 1), (Knob::RedGain, r), (Knob::GreenGain, g), (Knob::BlueGain, b)])
                    }
                }
                v
            }
        }
    }
}

/// The operations that bring the ISP from `applied` (what is in effect since the last IQ load) to `want`; a knob
/// absent from `want` is not asked for.
pub fn plan(want: &Tuning, applied: &Tuning) -> Vec<Adjust> {
    let mut out = Vec::new();
    let single = |k: Knob| want.get(&k).copied().filter(|v| applied.get(&k) != Some(v));
    for k in IQ_KNOBS {
        if let Some(v) = single(k) {
            out.push(Adjust::Iq { knob: k, value: v });
        }
    }
    if let Some(v) = single(Knob::PowerLine) {
        out.push(Adjust::PowerLine { value: v });
    }
    if let Some(v) = single(Knob::ExposureBias) {
        out.push(Adjust::ExposureBias { tenths: v });
    }
    // a group is applied as a whole, one operation for its knobs; one of them not asked takes its default
    let get = |k: Knob| want.get(&k).copied().unwrap_or(k.range().default);
    let asked = |ks: &[Knob]| ks.iter().any(|k| want.contains_key(k));
    if asked(&EXPOSURE) {
        let a = Adjust::Exposure { auto: get(Knob::AutoExposure) != 0, time_100us: get(Knob::Exposure), gain_pct: get(Knob::Gain) };
        if a.knobs().iter().any(|(k, v)| applied.get(k) != Some(v)) {
            out.push(a);
        }
    }
    if asked(&WHITE_BALANCE) {
        let manual = if get(Knob::WhiteBalanceByGains) != 0 {
            WbManual::Gains(get(Knob::RedGain), get(Knob::GreenGain), get(Knob::BlueGain))
        } else {
            WbManual::Kelvin(get(Knob::WhiteBalance))
        };
        let a = Adjust::WhiteBalance { auto: get(Knob::AutoWhiteBalance) != 0, manual };
        if a.knobs().iter().any(|(k, v)| applied.get(k) != Some(v)) {
            out.push(a);
        }
    }
    out
}

/// What the AE is doing right now (what the exposure lock and switching the auto off fix).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExposureNow {
    pub time_100us: i32,
    pub gain_pct: i32,
}

/// What the AWB is doing right now (what the white-balance lock and switching the auto off fix): its channel gains and
/// the colour temperature it estimates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WhiteBalanceNow {
    /// red, green, blue in 1/32
    pub gains: (i32, i32, i32),
    pub kelvin: i32,
}

// ---- arithmetic on the IQ file's values (the hardware layer applies it to every ISO row of a structure) ----------

/// The ISP's neutral level for brightness/contrast; a disabled level structure acts as this value.
pub const NEUTRAL_LEVEL: i32 = 50;

/// A level row after the knob: the knob's distance from its default is added to the IQ's level.
pub fn shift_level(iq_level: u32, iq_enabled: bool, knob: i32) -> u32 {
    let base = if iq_enabled { iq_level as i32 } else { NEUTRAL_LEVEL };
    (base + knob - NEUTRAL_LEVEL).clamp(0, 100) as u32
}

/// A strength after a 0–100 strength knob (50 = as tuned, 100 = twice), kept within the field's range.
pub fn scale_strength(iq_value: u8, knob: i32, max: u8) -> u8 {
    ((iq_value as i32 * knob + 25) / 50).clamp(0, max as i32) as u8
}

/// A gamma curve after the gamma knob: the curve's input axis is warped by x ↦ x^(100/knob), so 100 leaves it as is
/// and larger values lift the mid-tones. Black and white stay where the IQ curve puts them.
pub fn warp_gamma(lut: &[u16; 256], knob: i32) -> [u16; 256] {
    let e = 100.0 / knob as f64;
    let mut out = [0u16; 256];
    for (x, o) in out.iter_mut().enumerate() {
        let pos = 255.0 * (x as f64 / 255.0).powf(e);
        let i = (pos.floor() as usize).min(254);
        let f = pos - i as f64;
        *o = (lut[i] as f64 * (1.0 - f) + lut[i + 1] as f64 * f).round() as u16;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn knobs_not_asked_produce_nothing() {
        assert!(plan(&Tuning::new(), &Tuning::new()).is_empty());
    }

    #[test]
    fn a_changed_level_is_applied_once() {
        let want = Tuning::from([(Knob::Brightness, 60)]);
        let ops = plan(&want, &Tuning::new());
        assert_eq!(ops, vec![Adjust::Iq { knob: Knob::Brightness, value: 60 }]);
        let mut applied = Tuning::new();
        for op in &ops {
            applied.extend(op.knobs());
        }
        assert!(plan(&want, &applied).is_empty());
    }

    #[test]
    fn manual_exposure_carries_the_default_gain() {
        let want = Tuning::from([(Knob::AutoExposure, 0), (Knob::Exposure, 50)]);
        assert_eq!(plan(&want, &Tuning::new()), vec![Adjust::Exposure { auto: false, time_100us: 50, gain_pct: 100 }]);
    }

    #[test]
    fn white_balance_temperature_alone_keeps_auto_on() {
        let want = Tuning::from([(Knob::WhiteBalance, 4000)]);
        assert_eq!(plan(&want, &Tuning::new()), vec![Adjust::WhiteBalance { auto: true, manual: WbManual::Kelvin(4000) }]);
    }

    #[test]
    fn locked_gains_make_a_gain_based_manual_white_balance() {
        let want = Tuning::from([(Knob::AutoWhiteBalance, 0), (Knob::WhiteBalanceByGains, 1), (Knob::RedGain, 52)]);
        assert_eq!(plan(&want, &Tuning::new()), vec![Adjust::WhiteBalance { auto: false, manual: WbManual::Gains(52, 32, 32) }]);
    }

    #[test]
    fn knob_names_are_their_serde_names() {
        const ALL: [Knob; 18] = [
            Knob::Brightness,
            Knob::Contrast,
            Knob::Saturation,
            Knob::Sharpness,
            Knob::Gamma,
            Knob::Backlight,
            Knob::Grey,
            Knob::PowerLine,
            Knob::AutoExposure,
            Knob::Exposure,
            Knob::Gain,
            Knob::ExposureBias,
            Knob::AutoWhiteBalance,
            Knob::WhiteBalance,
            Knob::WhiteBalanceByGains,
            Knob::RedGain,
            Knob::GreenGain,
            Knob::BlueGain,
        ];
        for k in ALL {
            assert_eq!(toml::Value::try_from(k).unwrap().as_str(), Some(k.name()), "serde name of {k:?}");
        }
    }

    #[test]
    fn levels_shift_from_the_iq_value_and_a_disabled_row_counts_as_neutral() {
        assert_eq!(shift_level(40, true, 50), 40);
        assert_eq!(shift_level(40, true, 70), 60);
        assert_eq!(shift_level(0, false, 70), 70);
        assert_eq!(shift_level(90, true, 100), 100);
    }

    #[test]
    fn strengths_scale_and_clamp() {
        assert_eq!(scale_strength(32, 50, 127), 32);
        assert_eq!(scale_strength(32, 75, 127), 48);
        assert_eq!(scale_strength(100, 100, 127), 127);
        assert_eq!(scale_strength(10, 0, 127), 0);
    }

    #[test]
    fn gamma_100_is_the_identity_and_keeps_the_ends() {
        let mut lut = [0u16; 256];
        for (i, v) in lut.iter_mut().enumerate() {
            *v = (i * 4000 / 255) as u16;
        }
        assert_eq!(warp_gamma(&lut, 100), lut);
        let lifted = warp_gamma(&lut, 200);
        assert_eq!(lifted[0], lut[0]);
        assert_eq!(lifted[255], lut[255]);
        assert!(lifted[64] > lut[64], "more gamma lifts the mid-tones");
        assert!(lifted.windows(2).all(|w| w[0] <= w[1]), "still increasing");
    }
}
