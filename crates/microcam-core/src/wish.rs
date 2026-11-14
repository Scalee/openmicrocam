//! A change to the wishes ([`Prefs`]) as one value: a control the pipeline builds, a recording, the player — and one
//! function applies it to a copy of the wishes — the pipeline builds that copy, `price` plans and prices it, building
//! nothing.

use crate::controls::{self, Value};
use crate::policy::{Playback, Prefs, Recording};

#[derive(Clone, Debug, PartialEq)]
pub enum Wish {
    /// a control the pipeline builds (`controls::Drive::Built`) at a value the list accepted
    Control(String, Value),
    /// a recording into its file (set by whoever starts it: the card's next one)
    Record(Recording),
    RecordStop,
    /// a file shown in the outputs' live windows (opened by whoever asks: `Request::Play` names only its path)
    Play(Playback),
    PlayStop,
}

impl Wish {
    /// The wishes `p` with this one applied, or why it cannot be.
    pub fn apply(&self, p: &mut Prefs) -> Result<(), String> {
        match self {
            Wish::Control(name, v) => controls::apply(name, v, p)?,
            Wish::Record(_) if p.recording.is_some() => return Err("already recording".into()),
            Wish::Record(_) if p.player.is_some() => return Err("a file is playing".into()),
            Wish::Record(r) => p.recording = Some(r.clone()),
            Wish::RecordStop => p.recording = None,
            Wish::Play(_) if p.recording.is_some() => return Err("a recording is running".into()),
            Wish::Play(b) => p.player = Some(b.clone()),
            Wish::PlayStop => p.player = None,
        }
        Ok(())
    }

    /// What a transition for it is called in the log and the status.
    pub fn name(&self) -> &str {
        match self {
            Wish::Control(name, _) => name,
            Wish::Record(_) => "record",
            Wish::RecordStop => "record stop",
            Wish::Play(_) => "play",
            Wish::PlayStop => "play stop",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Codec, Size};

    #[test]
    fn wishes_apply_to_a_copy() {
        let mut p = Prefs::default();
        Wish::Control("hdmi".into(), Value::Text("off".into())).apply(&mut p).unwrap();
        assert!(!p.hdmi_enabled);
        Wish::Control("hdmi".into(), Value::Text("auto".into())).apply(&mut p).unwrap();
        assert!(p.hdmi_enabled && p.hdmi_override.is_none());
        Wish::Record(Recording { size: Size::new(1920, 1080), path: String::new() }).apply(&mut p).unwrap();
        assert_eq!(p.recording.as_ref().map(|r| r.size), Some(Size::new(1920, 1080)));
        assert_eq!(Wish::Record(Recording { size: Size::new(3840, 2160), path: String::new() }).apply(&mut p), Err("already recording".into()));
        assert!(Wish::Control("record-fps".into(), Value::Int(30)).apply(&mut p).is_err());
        assert!(Wish::Control("record-codec".into(), Value::Text("h265".into())).apply(&mut p).is_err());
        Wish::RecordStop.apply(&mut p).unwrap();
        Wish::Control("record-codec".into(), Value::Text("h265".into())).apply(&mut p).unwrap();
        assert_eq!(p.record_codec, Codec::H265);
        assert!(Wish::Control("record-codec".into(), Value::Text("mjpeg".into())).apply(&mut p).is_err());
        Wish::Control("level-usb".into(), Value::Int(0)).apply(&mut p).unwrap();
        assert_eq!(p.levels.usb, 0);
        assert!(Wish::Control("hdmi".into(), Value::Text("nonsense".into())).apply(&mut p).is_err());
        assert!(Wish::Control("precedence".into(), Value::Text("usb > hdmi".into())).apply(&mut p).is_err());
    }
}
