//! `Desired` — the complete description of what the pipeline should be doing. Produced by the policy, consumed by the
//! reconciler. Pure data; comparing two `Desired`s is how the reconciler finds the cheapest transition.

use serde::{Deserialize, Serialize};

use crate::formats::HostStream;
use crate::frame::Way;
use crate::hdmi::Timing;
use crate::tuning::Tuning;
use crate::{Codec, Rect, Size};

/// Monotonic revision of the policy's output; a transition serves one revision.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Revision(pub u64);

/// A host stream session: STREAMON → STREAMOFF. A new STREAMON is a new epoch even with identical dimensions, so the
/// pump is restarted and a stale transition can never publish "ready" for it (ARCHITECTURE §2).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Epoch(pub u64);

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SensorPlan {
    /// mode name (identity; the driver index is resolved by the mode table)
    pub mode: String,
    pub size: Size,
    /// frame rate the sensor runs at (≤ the mode's max)
    pub fps: u32,
    pub mode_max_fps: u32,
    pub line_ns: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HdmiPlan {
    pub timing: Timing,
    /// window of the sensor frame the live picture shows (full frame unless a menu crop or the live window asks
    /// otherwise), widened to the port's grid as the frame's `view` is
    pub crop: Rect,
    /// what SCL port 0 outputs: the crop scaled down to the frame's `view` where larger, never up (an SCL upscale on the
    /// realtime ISP→SCL link outruns it and stalls the ISP for every port, 2026-10-24); smaller than `view` → the HDMI
    /// upscaler (a frame-mode SCL pass, as USB's zoom) scales it up to it. DISP shows the canvas 1:1: the menu is in it
    pub scl_out: Size,
    /// the frame DISP shows, the timing's size: the live window letterboxed or pillarboxed where the aspects differ,
    /// black and the pictures around it, assembled in the HDMI pool's canvases as an encoder's frame is
    pub frame: OutputFrame,
    /// paused: the frame has no live picture, and why
    #[serde(default)]
    pub paused: Option<Pause>,
    /// what its live window shows: the sensor, or the player's file
    #[serde(default)]
    pub from: Input,
}

impl HdmiPlan {
    /// The rate its feed hands DISP canvases at: the timing's, or the sensor's (`src_fps`) when slower; covered, the
    /// compositor's own clock at the same rate.
    pub fn rate(&self, src_fps: u32) -> u32 {
        self.timing.rate_hz().min(src_fps).max(1)
    }
}

/// A picture the daemon holds in memory (a resource's decode, a camera picture taken into it): its identity only.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct PictureId(pub u64);

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsbPlan {
    pub epoch: Epoch,
    pub codec: Codec,
    pub size: Size,
    pub fps: u32,
    /// window of the sensor frame encoded for USB (centre crop to the format's aspect, then the zoom window)
    pub crop: Rect,
    /// what SCL port 2 outputs: the crop scaled down to `view` where larger, never up (see `HdmiPlan::scl_out`); smaller
    /// than `view` → a second, frame-mode SCL pass scales it up to it
    pub scl_out: Size,
    pub host: HostStreamRef,
    /// what the encoder's frames hold besides the live picture
    #[serde(default)]
    pub frame: OutputFrame,
    /// whose frames the encoder reads: its own pool's, or HDMI's canvases when its frame is HDMI's
    /// (`policy::Policy::plan`)
    #[serde(default)]
    pub frame_of: FrameOf,
    /// paused: the frame has no live picture, and why
    #[serde(default)]
    pub paused: Option<Pause>,
    /// what its live window shows: the sensor, or the player's file
    #[serde(default)]
    pub from: Input,
}

/// What an output's live window shows. The sensor: SCL 0's port of the output writes it (through the output's
/// channel of SCL dev 1 when it must be scaled up), `crop` in the sensor's pixels. The player: the decoder's port 0 bound
/// into the output's channel of SCL dev 1, which writes it, `crop` in the decoder port's pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Input {
    #[default]
    Sensor,
    Player,
}

/// Reference frames the player's decoder keeps: the camera's recordings use one (IPPP), two leave room.
pub const VDEC_REF_FRAMES: u32 = 2;

/// The file the player shows in the outputs' live windows.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlayerPlan {
    pub path: String,
    pub codec: Codec,
    /// the picture the file's sample entry names
    pub size: Size,
    /// the file's frame rate: the samples are sent at their times, the outputs' feeds take what comes
    pub fps: u32,
    /// the decoder's ring: its largest sample twice, at least 2 MiB
    pub ring: u32,
    /// the decoder's port 0: the picture scaled down to what the largest view showing it needs (never up: the VPU only
    /// scales down, to 1/8)
    pub port: Size,
}

/// Why an output's live picture is paused; its frame keeps the pictures and the menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Pause {
    /// a recording runs, and HDMI's, USB's and the recording's frames would be three written by the scaler's ports;
    /// the higher of the two outputs in the precedence keeps its live picture
    Recording,
    /// the media heap is short for something above it in the precedence
    Short,
}

/// Whose canvases an output's encoder reads (one pool per distinct frame). An output reads the
/// pool of the first earlier output — HDMI, then USB — whose frame is its own; that output's port, compositor, menu
/// layer and bake make the frame for both.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum FrameOf {
    /// its own pool, its own port, compositor and `ui` layer
    #[default]
    Own,
    /// HDMI's pool: one more reader of the canvases HDMI's compositor hands DISP
    Hdmi,
    /// USB's own pool (the recording's encoder beside USB's)
    Usb,
    /// no pool: USB's SCL port bound straight into its H.26x encoder in a ring (HW_RING), the encoder reading each
    /// line band as the port writes it — for USB's H.264 / H.265 while its frame is the live picture alone from the
    /// sensor at the port's own size
    Ring,
}

/// An output's frame as the feed assembles it in its pool's canvases (`crate::frame::assemble`): an encoder's, or the
/// one DISP shows on HDMI.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutputFrame {
    /// where the live picture lands in the frame: all of it, or its part beside the pictures
    pub window: Rect,
    /// where the port writes: `window` widened to the port's 16-column grid (`policy::port_view`; the crop is widened
    /// alike); all of it when the live picture is the frame
    pub view: Rect,
    /// the pictures' windows, bottom to top, as on the screen: what lies in them is the `ui`'s (its still set)
    pub windows: Vec<Rect>,
    pub way: Way,
    /// the visible live part, disjoint rectangles of the frame (none when covered)
    pub visible: Vec<Rect>,
}

impl OutputFrame {
    /// The frame with no live picture: its pictures alone, covered (no port writes it).
    pub fn paused(self) -> Self {
        Self { window: Rect::default(), view: Rect::default(), way: Way::Covered, visible: Vec::new(), ..self }
    }
}


/// The media encoder on SCL port 1: a photo (one JPEG frame) or a recording (H.264 into `file`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MediaPlan {
    pub codec: Codec,
    /// window of the sensor frame: the zoom window (the display aspect is the panel's business, as on USB)
    pub crop: Rect,
    /// what SCL port 1 outputs: the crop scaled down to the frame's `view` where larger, never up
    pub scl_out: Size,
    /// what the encoder takes; a `view` larger than `scl_out` → the media upscaler scales it up (a recording keeps its
    /// size while the zoom changes)
    pub size: Size,
    /// the rate frames arrive at (the sensor's) and the rate the encoder takes: SCL drops evenly between them
    pub src_fps: u32,
    pub fps: u32,
    /// JPEG quality (photo)
    pub quality: u8,
    /// H.264 bit/s and GOP (recording)
    pub bitrate: u32,
    pub gop: u32,
    /// the recording's file; None for a photo
    pub file: Option<String>,
    /// what the encoder's frames hold besides the live picture
    #[serde(default)]
    pub frame: OutputFrame,
    /// a photo is taken from the recording's frames: a JPEG encoder of the recording's size beside it, a reader of
    /// the pool the recording reads that the compositor hands one canvas
    #[serde(default)]
    pub snapshot: bool,
    /// whose canvases the encoder reads: a recording whose frame is HDMI's or USB's reads that pool; a photo, its own
    #[serde(default)]
    pub frame_of: FrameOf,
}

impl MediaPlan {
    /// A photo: one frame put into the encoder by hand, not a bound feed.
    pub fn shot(&self) -> bool {
        self.file.is_none()
    }
}

/// The committed UVC indices, kept so the gadget side can be answered without the table.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostStreamRef {
    pub format_index: u8,
    pub frame_index: u8,
    pub interval: u32,
}

impl From<&HostStream> for HostStreamRef {
    fn from(h: &HostStream) -> Self {
        Self { format_index: h.format_index, frame_index: h.frame_index, interval: h.interval }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImagePlan {
    /// ISP 3D noise reduction level, 0 = off (off by default: it smears motion)
    pub nr3d: u8,
    /// sensor orientation applied after every sensor (re)start: `setmirrorflip <pad> <mirror> <flip>`
    pub mirror: bool,
    pub flip: bool,
    pub iq_bin: String,
    pub mjpeg_quality: u8,
    /// picture knobs the user turned; the rest stay as the IQ file tunes them
    #[serde(default)]
    pub tuning: Tuning,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Desired {
    pub revision: Revision,
    pub sensor: SensorPlan,
    pub hdmi: Option<HdmiPlan>,
    pub usb: Option<UsbPlan>,
    #[serde(default)]
    pub media: Option<MediaPlan>,
    pub image: ImagePlan,
    /// a file shown in the outputs' live windows instead of the sensor
    #[serde(default)]
    pub player: Option<PlayerPlan>,
}

impl Desired {
    /// Whether an output is paused.
    pub fn pauses(&self) -> bool {
        self.hdmi.as_ref().is_some_and(|h| h.paused.is_some()) || self.usb.as_ref().is_some_and(|u| u.paused.is_some())
    }

    pub fn with_revision(mut self, r: Revision) -> Self {
        self.revision = r;
        self
    }
}
