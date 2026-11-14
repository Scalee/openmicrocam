//! Where an output shows the `ui`: its raster, its live picture's window and its display
//! level. The scene itself is [`crate::scene`]; how it is drawn is `microcam-draw`'s.

use crate::desired::Pause;
use crate::{Rect, Size};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OutputUi {
    pub raster: Size,
    pub live: Rect,
    /// 0 = the picture alone … `scene::MAX_LEVEL` = the full menu
    pub level: u8,
    /// its live picture paused, and why: the scene's `paused` nodes show here
    pub paused: Option<Pause>,
}

/// Each output's display level: which of the scene's nodes it shows. The menu app sets them; microcamd starts with the
/// full menu on HDMI and USB and a clean recording and photo. An output showing what an earlier one
/// shows at its raster reads that output's frames (`policy::FrameKey`), so the policy decides
/// from them too.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Levels {
    pub hdmi: u8,
    pub usb: u8,
    /// a recording and a photo
    pub media: u8,
}

impl Default for Levels {
    fn default() -> Self {
        Self { hdmi: crate::scene::MAX_LEVEL, usb: crate::scene::MAX_LEVEL, media: 0 }
    }
}

impl Levels {
    /// The level of the output named `hdmi`, `usb` or `media`; None for another name.
    pub fn of(&mut self, output: &str) -> Option<&mut u8> {
        match output {
            "hdmi" => Some(&mut self.hdmi),
            "usb" => Some(&mut self.usb),
            "media" => Some(&mut self.media),
            _ => None,
        }
    }
}
