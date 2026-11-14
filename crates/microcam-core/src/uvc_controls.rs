//! The UVC side of the camera's controls (`crate::controls`): which camera-terminal and processing-unit controls map
//! onto which control, and how their values travel (UVC 1.0 §4.2.2). Controls not listed here are answered with a STALL
//! and the matching request error code.

use crate::controls::{Control as Entry, Value};

/// Entity ids in the gadget's control descriptors (`g_webcam.ko`).
pub const CAMERA_TERMINAL: u8 = 1;
pub const PROCESSING_UNIT: u8 = 3;

/// `VC_REQUEST_ERROR_CODE_CONTROL`, addressed to the VideoControl interface itself (entity 0).
pub const CS_REQUEST_ERROR_CODE: u8 = 0x02;

/// `bRequestErrorCode` values.
pub mod error {
    pub const NONE: u8 = 0x00;
    /// the device has not completed a previous operation
    pub const NOT_READY: u8 = 0x01;
    pub const INVALID_UNIT: u8 = 0x05;
    pub const INVALID_CONTROL: u8 = 0x06;
    pub const INVALID_REQUEST: u8 = 0x07;
}

/// GET_INFO bits.
pub const INFO_GET: u8 = 0x01;
pub const INFO_SET: u8 = 0x02;
pub const INFO_DISABLED_BY_AUTO: u8 = 0x04;
/// the device changes the value itself, so the host must not cache it
pub const INFO_AUTOUPDATE: u8 = 0x08;

/// `CT_AE_MODE_CONTROL` bitmap values.
const AE_MANUAL: i32 = 0x01;
const AE_AUTO: i32 = 0x02;
const AE_APERTURE_PRIORITY: i32 = 0x08;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Encoding {
    /// a little-endian integer of `len` bytes, signed for brightness; a switch is 0 or 1, a choice its place in
    /// `values` (power-line: 0 off, 1 50 Hz, 2 60 Hz)
    Int,
    /// the auto-exposure mode bitmap: manual or aperture priority (the auto mode of a fixed-iris camera)
    AeMode,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Control {
    /// the control's name in the list
    pub name: &'static str,
    pub len: u8,
    pub encoding: Encoding,
}

pub fn lookup(entity: u8, cs: u8) -> Option<Control> {
    let c = |name, len| Some(Control { name, len, encoding: Encoding::Int });
    match (entity, cs) {
        (CAMERA_TERMINAL, 0x02) => Some(Control { name: "auto-exposure", len: 1, encoding: Encoding::AeMode }),
        (CAMERA_TERMINAL, 0x04) => c("exposure", 4),
        (PROCESSING_UNIT, 0x01) => c("backlight", 2),
        (PROCESSING_UNIT, 0x02) => c("brightness", 2),
        (PROCESSING_UNIT, 0x03) => c("contrast", 2),
        (PROCESSING_UNIT, 0x04) => c("gain", 2),
        (PROCESSING_UNIT, 0x05) => c("power-line", 1),
        (PROCESSING_UNIT, 0x07) => c("saturation", 2),
        (PROCESSING_UNIT, 0x08) => c("sharpness", 2),
        (PROCESSING_UNIT, 0x09) => c("gamma", 2),
        (PROCESSING_UNIT, 0x0a) => c("white-balance", 2),
        (PROCESSING_UNIT, 0x0b) => c("auto-white-balance", 1),
        _ => None,
    }
}

/// Is this entity one the camera has (so an unknown selector on it is an invalid control, not an invalid unit)?
pub fn known_entity(entity: u8) -> bool {
    entity == CAMERA_TERMINAL || entity == PROCESSING_UNIT
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Query {
    Cur,
    Min,
    Max,
    Res,
    Def,
}

impl Control {
    /// The value a GET answers from the control's entry, in its own encoding; None = the request does not apply (STALL).
    pub fn answer(&self, q: Query, e: &Entry) -> Option<i32> {
        let (cur, default) = (wire(e, &e.value), wire(e, &e.default));
        match self.encoding {
            Encoding::Int => Some(match q {
                Query::Cur => cur,
                Query::Def => default,
                Query::Min => e.min.unwrap_or(0) as i32,
                Query::Max => e.max.map_or(e.values.len().max(2) as i32 - 1, |m| m as i32),
                Query::Res => e.step.unwrap_or(1) as i32,
            }),
            // the mode bitmap has no MIN/MAX; RES lists the supported modes
            Encoding::AeMode => match q {
                Query::Cur => Some(ae_mode(cur)),
                Query::Def => Some(ae_mode(default)),
                Query::Res => Some(AE_MANUAL | AE_APERTURE_PRIORITY),
                Query::Min | Query::Max => None,
            },
        }
    }

    /// A SET_CUR payload as the control's value (`e` for a choice's `values`); None = a value the camera cannot take.
    pub fn decode(&self, data: &[u8], e: &Entry) -> Option<Value> {
        let n = self.len as usize;
        if data.len() < n {
            return None;
        }
        let raw = match n {
            1 => data[0] as i32,
            2 if self.name == "brightness" => i16::from_le_bytes([data[0], data[1]]) as i32,
            2 => u16::from_le_bytes([data[0], data[1]]) as i32,
            _ => i32::from_le_bytes([data[0], data[1], data[2], data[3]]),
        };
        let raw = match self.encoding {
            Encoding::Int => raw,
            Encoding::AeMode => match raw {
                AE_MANUAL => 0,
                AE_AUTO | AE_APERTURE_PRIORITY => 1,
                _ => return None,
            },
        };
        match e.kind {
            crate::controls::Kind::Int => Some(Value::Int(raw as i64)),
            crate::controls::Kind::Bool => (raw == 0 || raw == 1).then_some(Value::Bool(raw == 1)),
            crate::controls::Kind::Choice => e.values.get(usize::try_from(raw).ok()?).map(|v| Value::Text(v.clone())),
            crate::controls::Kind::Order => None,
        }
    }

    pub fn encode(&self, v: i32, out: &mut [u8]) -> usize {
        let n = self.len as usize;
        out[..n].copy_from_slice(&v.to_le_bytes()[..n]);
        n
    }

    /// GET_INFO: readable and writable; flagged while it cannot be changed (its auto mode runs).
    pub fn info(&self, e: &Entry) -> u8 {
        // every control also moves without the host: the menu, the CLI, the locks
        INFO_GET | INFO_SET | INFO_AUTOUPDATE | if e.available { 0 } else { INFO_DISABLED_BY_AUTO }
    }
}

/// A value as UVC carries it: a switch 0 or 1, a choice its place in `values`.
fn wire(e: &Entry, v: &Value) -> i32 {
    match v {
        Value::Bool(b) => *b as i32,
        Value::Int(i) => *i as i32,
        Value::Text(t) => e.values.iter().position(|x| x == t).unwrap_or(0) as i32,
    }
}

fn ae_mode(auto: i32) -> i32 {
    if auto != 0 {
        AE_APERTURE_PRIORITY
    } else {
        AE_MANUAL
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::controls::List;

    fn entry(name: &str) -> Entry {
        List::new(&crate::policy::Prefs::default(), &crate::policy::tests::policy()).get(name).unwrap().clone()
    }

    #[test]
    fn brightness_is_signed_and_round_trips() {
        let c = lookup(PROCESSING_UNIT, 0x02).unwrap();
        let e = entry("brightness");
        let mut b = [0u8; 4];
        assert_eq!(c.encode(60, &mut b), 2);
        assert_eq!(c.decode(&b[..2], &e), Some(Value::Int(60)));
        assert_eq!(c.decode(&(-5i16).to_le_bytes(), &e), Some(Value::Int(-5)));
        assert_eq!(c.answer(Query::Def, &e), Some(50));
    }

    #[test]
    fn exposure_is_four_bytes_in_100us() {
        let c = lookup(CAMERA_TERMINAL, 0x04).unwrap();
        assert_eq!(c.len, 4);
        assert_eq!(c.decode(&333u32.to_le_bytes(), &entry("exposure")), Some(Value::Int(333)));
    }

    #[test]
    fn ae_mode_speaks_the_bitmap() {
        let c = lookup(CAMERA_TERMINAL, 0x02).unwrap();
        let e = entry("auto-exposure");
        assert_eq!(c.decode(&[1], &e), Some(Value::Bool(false)));
        assert_eq!(c.decode(&[8], &e), Some(Value::Bool(true)));
        assert_eq!(c.decode(&[2], &e), Some(Value::Bool(true)));
        assert_eq!(c.decode(&[4], &e), None, "shutter priority is not offered");
        assert_eq!(c.answer(Query::Cur, &e), Some(8));
        assert_eq!(c.answer(Query::Res, &e), Some(9));
        assert_eq!(c.answer(Query::Min, &e), None);
    }

    #[test]
    fn manual_controls_are_flagged_while_auto_runs() {
        let gain = lookup(PROCESSING_UNIT, 0x04).unwrap();
        assert_eq!(gain.info(&entry("gain")), 0x0f);
        assert_eq!(gain.info(&entry("brightness")), 0x0b);
    }

    #[test]
    fn power_line_travels_as_its_place_and_saturation_as_0_to_100() {
        let c = lookup(PROCESSING_UNIT, 0x05).unwrap();
        let e = entry("power-line");
        assert_eq!(c.decode(&[2], &e), Some(Value::Text("60".into())));
        assert_eq!(c.decode(&[3], &e), None);
        assert_eq!((c.answer(Query::Min, &e), c.answer(Query::Max, &e)), (Some(0), Some(2)));
        let s = lookup(PROCESSING_UNIT, 0x07).unwrap();
        assert_eq!(s.answer(Query::Max, &entry("saturation")), Some(100));
    }

    #[test]
    fn hue_and_zoom_are_not_served() {
        assert!(lookup(PROCESSING_UNIT, 0x06).is_none());
        assert!(lookup(CAMERA_TERMINAL, 0x0b).is_none());
        assert!(known_entity(CAMERA_TERMINAL) && !known_entity(6));
    }
}
