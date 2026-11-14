//! Domain of the camera daemon, without I/O: everything here is testable on the laptop.
//!
//! - [`modes`] — the sensor modes the loaded driver shows and their validation against the MI API's list.
//! - [`formats`] — the USB formats the loaded gadget shows, which the UVC probe/commit refers to.
//! - [`hdmi`] — HDMI timings and the monitor rule (EDID capabilities → timing); [`edid`] parses the EDID bytes.
//! - [`policy`] — `Inputs` (monitor, host stream, preferences) → [`desired::Desired`].
//! - [`reconcile`] — `Current` + `Desired` → ordered hardware operations ([`reconcile::Op`]) and the state they leave.
//! - [`pace`] — which frames of a faster source a reader takes, by their capture times.

pub mod bmff;
pub mod controls;
pub mod demux;
pub mod desired;
pub mod edid;
pub mod edit;
pub mod files;
pub mod formats;
pub mod frame;
pub mod hdmi;
pub mod jpeg;
pub mod keys;
pub mod lock;
pub mod heap;
pub mod modes;
pub mod mp4;
pub mod overlay;
pub mod pace;
pub mod policy;
pub mod precedence;
pub mod protocol;
pub mod reconcile;
pub mod scene;
pub mod tuning;
pub mod ui;
pub mod uvc_controls;
pub mod wish;

/// Width × height in pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct Size {
    pub w: u32,
    pub h: u32,
}

impl Size {
    pub const fn new(w: u32, h: u32) -> Self {
        Self { w, h }
    }
    /// Relative difference of two aspect ratios, 0.0 = equal.
    pub fn aspect_diff(self, other: Size) -> f64 {
        let a = self.w as f64 / self.h as f64;
        let b = other.w as f64 / other.h as f64;
        ((a - b) / b).abs()
    }
}

impl std::fmt::Display for Size {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}x{}", self.w, self.h)
    }
}

/// A window inside a frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

impl Rect {
    pub const fn full(size: Size) -> Self {
        Self { x: 0, y: 0, w: size.w, h: size.h }
    }
    pub const fn size(self) -> Size {
        Size { w: self.w, h: self.h }
    }
    pub const fn area(self) -> u64 {
        self.w as u64 * self.h as u64
    }
    pub const fn is_empty(self) -> bool {
        self.w == 0 || self.h == 0
    }
    /// The common part (empty when they do not overlap).
    pub fn intersect(self, o: Rect) -> Rect {
        let (x0, y0) = (self.x.max(o.x), self.y.max(o.y));
        let (x1, y1) = ((self.x + self.w).min(o.x + o.w), (self.y + self.h).min(o.y + o.h));
        Rect { x: x0, y: y0, w: x1.saturating_sub(x0), h: y1.saturating_sub(y0) }
    }
    /// The smallest rectangle holding both.
    pub fn union(self, o: Rect) -> Rect {
        let (x0, y0) = (self.x.min(o.x), self.y.min(o.y));
        Rect { x: x0, y: y0, w: (self.x + self.w).max(o.x + o.w) - x0, h: (self.y + self.h).max(o.y + o.h) - y0 }
    }
    pub fn contains(self, o: Rect) -> bool {
        o.x >= self.x && o.y >= self.y && o.x + o.w <= self.x + self.w && o.y + o.h <= self.y + self.h
    }
    /// The largest centred window of `frame` with the aspect of `aspect`, even-aligned (NV12 needs even sizes/offsets).
    /// Returns the full frame when the aspects differ by 1 % or less.
    pub fn centre_crop(frame: Size, aspect: Size) -> Self {
        if frame.aspect_diff(aspect) <= 0.01 {
            return Self::full(frame);
        }
        let fa = frame.w as f64 / frame.h as f64;
        let ta = aspect.w as f64 / aspect.h as f64;
        let (w, h) = if ta > fa {
            (frame.w, ((frame.w as f64 / ta).round() as u32) & !1)
        } else {
            (((frame.h as f64 * ta).round() as u32) & !1, frame.h)
        };
        Self { x: ((frame.w - w) / 2) & !1, y: ((frame.h - h) / 2) & !1, w, h }
    }
}

/// Video codec of a USB stream. Written as the protocol's words; the format tables' upper-case names are read too.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Codec {
    #[serde(alias = "MJPEG")]
    Mjpeg,
    #[serde(alias = "H264")]
    H264,
    #[serde(alias = "H265")]
    H265,
}

impl Codec {
    /// A recording's codec by its settings name: `h264` or `h265`.
    pub fn recordable(name: &str) -> Option<Codec> {
        match name {
            "h264" => Some(Codec::H264),
            "h265" => Some(Codec::H265),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn centre_crop_matches_the_shim_rules() {
        // 3840×2160 → 3840×1600+0+280 and → 3840×1440+0+360
        let f = Size::new(3840, 2160);
        assert_eq!(Rect::centre_crop(f, Size::new(3840, 1600)), Rect { x: 0, y: 280, w: 3840, h: 1600 });
        assert_eq!(Rect::centre_crop(f, Size::new(3840, 1440)), Rect { x: 0, y: 360, w: 3840, h: 1440 });
        assert_eq!(Rect::centre_crop(f, Size::new(1920, 1080)), Rect::full(f));
        assert_eq!(Rect::centre_crop(f, Size::new(2848, 1600)), Rect { x: 0, y: 0, w: 3844 & !1, h: 2160 }.clamp_to(f));
    }

    impl Rect {
        /// test helper: a crop never exceeds its frame
        fn clamp_to(self, f: Size) -> Self {
            Rect { x: self.x, y: self.y, w: self.w.min(f.w), h: self.h.min(f.h) }
        }
    }
}
