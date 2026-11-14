//! `Inputs` → `Desired`. Pure, data-driven by `camera/policy.toml`. Rules: every configuration gets the hardware maximum; HDMI defaults to the monitor's largest resolution at its
//! highest rate; with a 4K monitor HDMI keeps 4K and USB adapts; the requested USB frame rate really applies.

use serde::Deserialize;

use crate::desired::{Desired, Epoch, FrameOf, HdmiPlan, HostStreamRef, ImagePlan, Input, MediaPlan, OutputFrame, Pause, PlayerPlan, Revision, SensorPlan, UsbPlan};
use crate::formats::HostStream;
use crate::frame::{self, Way};
use crate::hdmi::{best_timing, Monitor, Timing};
use crate::modes::{Mode, ModeTable};
use crate::precedence::{Precedence, Step, Who};
use crate::ui::Levels;
use crate::{Codec, Rect, Size};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BootPreset {
    #[serde(rename = "4k")]
    Uhd,
    #[serde(rename = "1080")]
    Fhd,
}

/// What HDMI and USB give by the precedence while the media heap is short for something above them: HDMI a
/// lower timing, paused or off, USB paused or refused; kept with the wishes, and given back as soon as the video fits
/// without it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Giving {
    pub hdmi: Option<Step>,
    pub usb: Option<Step>,
}

/// What the user set (config file, CLI, later the menu).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Prefs {
    pub boot_preset: BootPreset,
    /// a fixed HDMI timing instead of the EDID rule
    pub hdmi_override: Option<Timing>,
    pub hdmi_enabled: bool,
    /// who gives way when the camera is short
    pub precedence: Precedence,
    /// what HDMI and USB gave by it
    pub giving: Giving,
    pub nr3d: u8,
    pub mirror: bool,
    pub flip: bool,
    pub iq_bin: String,
    pub mjpeg_quality: u8,
    /// what of the frame the HDMI shows (the menu's display aspect and zoom)
    pub view: View,
    /// the user's mirror/flip on top of the mounting orientation (`mirror`/`flip`)
    pub user_mirror: bool,
    pub user_flip: bool,
    /// the menu's standby: the screen is off until it wakes
    pub standby: bool,
    /// a photo being taken: the size the menu asked for
    pub photo: Option<Size>,
    /// a recording running
    pub recording: Option<Recording>,
    /// a file the outputs' live windows show instead of the sensor
    pub player: Option<Playback>,
    /// the recording rate from the settings; 0 = the highest the encoder sustains at the size
    pub record_fps: u32,
    /// the next recording's codec: H.264 or H.265
    pub record_codec: Codec,
    /// what the scene puts into each output's frame (`crate::frame`)
    pub frames: Frames,
    /// each output's display level (the menu's)
    pub levels: Levels,
}

/// Where the live picture lies in each output's frame and the full-colour pictures beside it, as the `ui` laid them
/// out from the scene (`crate::frame::plane`). None: the live picture takes the whole frame.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Frames {
    pub hdmi: Option<Layout>,
    pub usb: Option<Layout>,
    /// the recording's or the photo's
    pub media: Option<Layout>,
    /// a menu has a scene: the outputs' levels say what each shows (without one every output shows the live picture
    /// alone, whatever its level)
    pub scene: bool,
}

/// One output's frame, for the raster it was laid out on: an output built at another size shows the live picture
/// whole until the `ui` lays the frame out again for it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layout {
    pub raster: Size,
    pub live: Rect,
    /// the pictures' windows, in the order the scene has them (bottom to top): the geometry only — what lies in them
    /// is the `ui`'s still set, no step of the pipeline's
    pub windows: Vec<Rect>,
}

impl Layout {
    /// The layout if it was made for `raster` and puts anything besides a whole-frame live picture there.
    fn on(l: &Option<Layout>, raster: Size) -> Option<&Layout> {
        l.as_ref().filter(|l| l.raster == raster && (l.live != Rect::full(raster) || !l.windows.is_empty()))
    }
}

/// An output's frame of `size` for the layout `l` the `ui` laid out (None: the live picture alone in `full`, black
/// around it), and the crop of the sensor frame (`sensor` pixels) its port takes: `crop` (what `full` shows: the zoom
/// window) narrowed to the live window's aspect, measured in the frame's pixels as `full` shows them (a 16:10 or 4:3
/// panel stretches HDMI's signal), both widened to the port's grid (`port_view`).
fn output_frame(l: Option<&Layout>, size: Size, full: Rect, crop: Rect, sensor: Size) -> (OutputFrame, Rect) {
    let aspect = |w: Rect| Size::new(w.w * crop.w / full.w.max(1), w.h * crop.h / full.h.max(1));
    let (live, crop, windows) = match l {
        None => (full, crop, Vec::new()),
        Some(l) => (l.live, centre_part(crop, aspect(l.live)), l.windows.clone()),
    };
    let a = frame::assemble(size, live, &windows);
    let (view, crop) = port_view(size, live, crop, Rect::full(sensor));
    (OutputFrame { window: live, view, windows, way: a.way, visible: a.visible }, crop)
}

/// The scaler port's write area for a live window `live` of a `frame`: `live` widened to the port's grid
/// ([`frame::PORT_GRID`]), and the crop that
/// keeps the live picture's scale and place inside it — `crop` (what `live` shows) widened by the same columns in the
/// sensor's pixels, kept inside `bounds` (at the sensor's edge the widened columns squeeze the picture by at most
/// 15 columns of the window). The columns the widening adds show still content again (the feed puts it back).
pub fn port_view(frame: Size, live: Rect, crop: Rect, bounds: Rect) -> (Rect, Rect) {
    let g = frame::PORT_GRID;
    let x0 = live.x / g * g;
    let x1 = ((live.x + live.w).div_ceil(g) * g).min(frame.w);
    if (x0, x1) == (live.x, live.x + live.w) {
        return (live, crop);
    }
    let view = Rect { x: x0, w: x1 - x0, ..live };
    let sx = crop.w as f64 / live.w.max(1) as f64;
    let mut cw = (((view.w as f64 * sx).round() as u32) & !1).min(bounds.w);
    let mut cx = (crop.x as f64 - (live.x - x0) as f64 * sx).round().max(0.0) as u32 & !1;
    cx = cx.max(bounds.x);
    if cx + cw > bounds.x + bounds.w {
        cx = (bounds.x + bounds.w - cw) & !1;
    }
    cw = cw.min(bounds.x + bounds.w - cx) & !1;
    (view, Rect { x: cx, w: cw, ..crop })
}

/// The largest centred part of `area` with the aspect of `aspect`, even-aligned.
fn centre_part(area: Rect, aspect: Size) -> Rect {
    let c = Rect::centre_crop(area.size(), aspect);
    Rect { x: area.x + c.x, y: area.y + c.y, ..c }
}

/// A recording, resolved against the tables.
struct RecordPlan<'a> {
    mode: &'a Mode,
    size: Size,
    fps: u32,
    codec: Codec,
    path: &'a String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Recording {
    /// the size the menu asked for
    pub size: Size,
    pub path: String,
}

/// A file opened for the player: what its video track says (`crate::demux`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Playback {
    pub path: String,
    pub codec: Codec,
    pub size: Size,
    pub fps: u32,
    /// the largest sample's bytes as the decoder takes it
    pub largest: u32,
}

impl Playback {
    /// The decoder's ring: two of the largest sample, at least 2 MiB.
    pub fn ring(&self) -> u32 {
        (2 * self.largest).max(2 << 20).next_multiple_of(4096)
    }
}

/// The decoder port's size for the player: the picture scaled down by the most any output needs of it — a view `v`
/// showing `crop` of the picture needs `v / crop` of its pixels — never up and not below 1/8 (the VPU's range); width
/// to 16, height even.
pub fn player_port(pic: Size, outs: &[(Rect, Size)]) -> Size {
    let need = outs.iter().map(|(c, v)| (v.w as f64 / c.w.max(1) as f64).max(v.h as f64 / c.h.max(1) as f64)).fold(0.0, f64::max);
    let s = need.clamp(0.125, 1.0);
    let w = ((pic.w as f64 * s).ceil() as u32).next_multiple_of(16).min(pic.w & !15);
    let h = ((pic.h as f64 * s).ceil() as u32).next_multiple_of(2).min(pic.h & !1);
    Size::new(w, h)
}

/// `r` in a picture of `from` pixels, in the same picture scaled to `to`: x and width even, inside `to`.
pub fn scale_rect(r: Rect, from: Size, to: Size) -> Rect {
    let sx = |v: u32| (v as u64 * to.w as u64 / from.w.max(1) as u64) as u32 & !1;
    let sy = |v: u32| (v as u64 * to.h as u64 / from.h.max(1) as u64) as u32 & !1;
    let (x, y) = (sx(r.x), sy(r.y));
    Rect { x, y, w: sx(r.w).min(to.w - x), h: sy(r.h).min(to.h - y) }
}

/// The aspect of the monitor's panel. A 16:10 or 4:3 panel shows the 16:9 signal stretched, so the HDMI gets a crop of
/// the frame with the panel's aspect over the whole screen (the vendor menu's "Display" setting).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, Deserialize)]
pub enum ScreenAspect {
    #[default]
    Wide,
    Sixteen10,
    Four3,
}

/// The largest digital zoom step: factor 1 + step/10, so ×5.
pub const MAX_ZOOM: u8 = 40;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, Deserialize)]
pub struct View {
    pub aspect: ScreenAspect,
    /// 0 = none; factor 1 + zoom/10
    pub zoom: u8,
}

impl View {
    /// The part of the sensor frame the HDMI shows: the panel-aspect area, then the zoom window centred in it.
    pub fn crop(&self, frame: Size) -> Rect {
        let area = match self.aspect {
            ScreenAspect::Wide => Rect::full(frame),
            ScreenAspect::Sixteen10 => Rect::centre_crop(frame, Size::new(16, 10)),
            ScreenAspect::Four3 => Rect::centre_crop(frame, Size::new(4, 3)),
        };
        self.zoom_window(area)
    }

    /// The zoom window centred in `area` (the whole area at zoom 0).
    pub fn zoom_window(&self, area: Rect) -> Rect {
        let z = self.zoom.min(MAX_ZOOM) as u32;
        if z == 0 {
            return area;
        }
        let w = (area.w * 10 / (10 + z)) & !1;
        let h = (area.h * 10 / (10 + z)) & !1;
        Rect { x: area.x + (((area.w - w) / 2) & !1), y: area.y + (((area.h - h) / 2) & !1), w, h }
    }

    /// Where the crop lands on the screen: fitted for a 16:9 panel, the whole screen for the others (the panel stretches).
    pub fn window(&self, crop: Size, screen: Size) -> Rect {
        match self.aspect {
            ScreenAspect::Wide => fit_window(crop, screen),
            _ => Rect::full(screen),
        }
    }
}

impl Prefs {
    /// What an output at `level` shows of the menu, for comparing two outputs' frames: its level while a menu has a
    /// scene, nothing without one.
    fn shows(&self, level: u8) -> u8 {
        if self.frames.scene {
            level
        } else {
            0
        }
    }
}

impl Default for Prefs {
    fn default() -> Self {
        Self {
            boot_preset: BootPreset::Uhd,
            hdmi_override: None,
            hdmi_enabled: true,
            precedence: Precedence::default(),
            giving: Giving::default(),
            // 3D noise reduction off: it smears motion
            nr3d: 0,
            mirror: true,
            flip: true,
            iq_bin: "/customer/microcam/iq_file_gc8613.bin".into(),
            // the highest Qfactor at which 4K45 MJPEG keeps its rate through USB
            mjpeg_quality: 20,
            view: View::default(),
            user_mirror: false,
            user_flip: false,
            standby: false,
            photo: None,
            recording: None,
            player: None,
            record_fps: 0,
            record_codec: Codec::H264,
            frames: Frames::default(),
            levels: Levels::default(),
        }
    }
}

/// Everything the policy decides from.
#[derive(Clone, Debug)]
pub struct Inputs<'a> {
    pub monitor: &'a Monitor,
    pub host: Option<(&'a HostStream, Epoch)>,
    pub prefs: &'a Prefs,
    /// every picture knob as the controls list holds it (`crate::controls::List::tuning`)
    pub tuning: &'a crate::tuning::Tuning,
}

/// Monitor class the USB mapping is keyed by.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MonitorClass {
    None,
    /// up to 1440 rows
    Hd,
    /// 2160 rows
    Uhd,
}

#[derive(Debug, Deserialize)]
struct PolicyFile {
    schema: u32,
    presets: Presets,
    hdmi: HdmiRules,
    #[serde(rename = "usb")]
    usb: Vec<UsbRow>,
    #[serde(default)]
    record: Vec<RecordRow>,
    h26x: H26xRules,
}

/// The H.26x encoder's time: what a frame of each size takes of it, alone and beside USB's MJPEG stream, and how much of
/// each second the channels may take.
#[derive(Debug, Deserialize)]
struct H26xRules {
    budget_ms: f64,
    frame: Vec<H26xRow>,
    mjpeg: MjpegRule,
}

/// Beside USB's MJPEG stream of more than `px_per_s` pixels a second: what a frame of each size takes of the H.26x
/// encoder.
#[derive(Debug, Deserialize)]
struct MjpegRule {
    px_per_s: u64,
    frame: Vec<H26xRow>,
}

#[derive(Debug, Deserialize)]
struct H26xRow {
    size: [u32; 2],
    ms: f64,
}

#[derive(Debug, Deserialize)]
struct RecordRow {
    size: [u32; 2],
    modes: Vec<String>,
    rates: Vec<u32>,
}

#[derive(Debug, Deserialize)]
struct Presets {
    boot_4k: String,
    boot_1080: String,
}

#[derive(Debug, Deserialize)]
struct HdmiRules {
    candidates: Vec<Timing>,
    no_edid: Timing,
}

#[derive(Debug, Deserialize)]
struct UsbRow {
    size: [u32; 2],
    hd: String,
    none: String,
    uhd: String,
}

#[derive(Debug, thiserror::Error)]
pub enum PolicyError {
    #[error("policy.toml: {0}")]
    Parse(#[from] toml::de::Error),
    #[error("policy.toml schema {0} not supported (want 1)")]
    Schema(u32),
    #[error("policy names a mode the table lacks: {0}")]
    Mode(#[from] crate::modes::ModesError),
    #[error("no USB rule serves {0}")]
    NoUsbRule(Size),
    #[error("no recording rule serves {0}")]
    NoRecordRule(Size),
    #[error("beside the recording {0} would pause, and its floor in the precedence forbids it")]
    NoPause(&'static str),
    #[error("the H.26x encoder cannot carry the recording beside USB's {0} stream ({1} in the precedence)")]
    Encoder(Size, &'static str),
}

#[derive(Clone, Debug)]
pub struct Policy {
    boot_4k: String,
    boot_1080: String,
    hdmi_candidates: Vec<Timing>,
    hdmi_no_edid: Timing,
    usb: Vec<UsbRow2>,
    record: Vec<RecordRule>,
    h26x_budget_ms: f64,
    /// the frame sizes, the smallest first, with the ms a frame takes
    h26x_frames: Vec<(Size, f64)>,
    /// USB's MJPEG stream up to this many pixels a second leaves the H.26x encoder as alone
    h26x_mjpeg_px: u64,
    /// the frames beside a faster one, as `h26x_frames`
    h26x_mjpeg_frames: Vec<(Size, f64)>,
}

/// Record size → sensor modes (best picture first) and the measured standard rates.
#[derive(Clone, Debug)]
struct RecordRule {
    size: Size,
    modes: Vec<String>,
    rates: Vec<u32>,
}

#[derive(Clone, Debug)]
struct UsbRow2 {
    size: Size,
    hd: String,
    none: String,
    uhd: String,
}

impl UsbRow2 {
    fn mode(&self, class: MonitorClass) -> &str {
        match class {
            MonitorClass::Hd => &self.hd,
            MonitorClass::None => &self.none,
            MonitorClass::Uhd => &self.uhd,
        }
    }
}

/// A table's rows, the smallest size first.
fn rows(rows: &[H26xRow]) -> Vec<(Size, f64)> {
    let mut v: Vec<(Size, f64)> = rows.iter().map(|r| (Size::new(r.size[0], r.size[1]), r.ms)).collect();
    v.sort_by_key(|(s, _)| s.w as u64 * s.h as u64);
    v
}

impl Policy {
    pub fn parse(text: &str) -> Result<Self, PolicyError> {
        let f: PolicyFile = toml::from_str(text)?;
        if f.schema != 1 {
            return Err(PolicyError::Schema(f.schema));
        }
        Ok(Self {
            boot_4k: f.presets.boot_4k,
            boot_1080: f.presets.boot_1080,
            hdmi_candidates: f.hdmi.candidates,
            hdmi_no_edid: f.hdmi.no_edid,
            usb: f.usb.into_iter().map(|r| UsbRow2 { size: Size::new(r.size[0], r.size[1]), hd: r.hd, none: r.none, uhd: r.uhd }).collect(),
            record: f
                .record
                .into_iter()
                .map(|r| {
                    let mut rates = r.rates;
                    rates.sort_unstable();
                    RecordRule { size: Size::new(r.size[0], r.size[1]), modes: r.modes, rates }
                })
                .collect(),
            h26x_budget_ms: f.h26x.budget_ms,
            h26x_frames: rows(&f.h26x.frame),
            h26x_mjpeg_px: f.h26x.mjpeg.px_per_s,
            h26x_mjpeg_frames: rows(&f.h26x.mjpeg.frame),
        })
    }

    /// The same rules with every sensor mode replaced by `mode` (the conservative profile: the driver's mode list not used,
    /// only the driver's mode 0 is known; HDMI and USB get what that mode gives).
    pub fn single_mode(&self, mode: &str) -> Self {
        let m = mode.to_string();
        Self {
            boot_4k: m.clone(),
            boot_1080: m.clone(),
            hdmi_candidates: self.hdmi_candidates.clone(),
            hdmi_no_edid: self.hdmi_no_edid,
            usb: self.usb.iter().map(|r| UsbRow2 { size: r.size, hd: m.clone(), none: m.clone(), uhd: m.clone() }).collect(),
            record: self.record.iter().map(|r| RecordRule { modes: vec![m.clone()], ..r.clone() }).collect(),
            h26x_budget_ms: self.h26x_budget_ms,
            h26x_frames: self.h26x_frames.clone(),
            h26x_mjpeg_px: self.h26x_mjpeg_px,
            h26x_mjpeg_frames: self.h26x_mjpeg_frames.clone(),
        }
    }

    /// Every mode the policy may ever pick must exist in the table — checked once at start.
    pub fn validate(&self, modes: &ModeTable) -> Result<(), PolicyError> {
        modes.get(&self.boot_4k)?;
        modes.get(&self.boot_1080)?;
        for r in &self.usb {
            modes.get(&r.hd)?;
            modes.get(&r.none)?;
            modes.get(&r.uhd)?;
        }
        for r in &self.record {
            for m in &r.modes {
                modes.get(m)?;
            }
            if r.modes.is_empty() || r.rates.is_empty() {
                return Err(PolicyError::NoRecordRule(r.size));
            }
        }
        Ok(())
    }

    pub fn monitor_class(monitor: &Monitor, hdmi: Option<Timing>) -> MonitorClass {
        match (monitor, hdmi) {
            (Monitor::Absent, _) | (_, None) => MonitorClass::None,
            (_, Some(t)) if t.size().h >= 2160 => MonitorClass::Uhd,
            _ => MonitorClass::Hd,
        }
    }

    /// The recording's sensor mode, size and rate: the wanted rate if the size's row lists it, else the highest listed
    /// below it (the lowest listed if none is; 0 = the highest listed); then the first mode of the row reaching it.
    fn record_plan<'a>(&self, r: &'a Recording, wanted: u32, codec: Codec, modes: &'a ModeTable) -> Result<RecordPlan<'a>, PolicyError> {
        let row = self.record.iter().find(|x| x.size == r.size).ok_or(PolicyError::NoRecordRule(r.size))?;
        let fps = match wanted {
            0 => row.rates[row.rates.len() - 1],
            w => row.rates.iter().copied().filter(|&x| x <= w).max().unwrap_or(row.rates[0]),
        };
        let list: Vec<&Mode> = row.modes.iter().map(|n| modes.get(n)).collect::<Result<_, _>>()?;
        let mode = list.iter().copied().find(|m| m.max_fps >= fps).ok_or(PolicyError::NoRecordRule(r.size))?;
        Ok(RecordPlan { mode, size: row.size, fps, codec, path: &r.path })
    }

    /// The record sizes the tables serve, as they list them (the largest first).
    pub fn record_sizes(&self) -> Vec<Size> {
        self.record.iter().map(|r| r.size).collect()
    }

    /// The HDMI timings the camera can drive, best first: those the EDID rule may pick, and a fixed `hdmi` may be.
    pub fn hdmi_timings(&self) -> &[Timing] {
        &self.hdmi_candidates
    }

    /// USB above HDMI in the precedence: the first HDMI step, from the preferred timing down, at which the sensor mode for
    /// that monitor class delivers the host's rate — the preferred timing, the monitor's best below 4K (only from 4K: the
    /// measured USB rates do not differ between 1080p and 1440p), HDMI off. Off when no step delivers it: that is the
    /// fastest; when HDMI's floor keeps it on (`may_go_off` false), its lowest timing instead.
    fn hdmi_giving_way(&self, monitor: &Monitor, preferred: Option<Timing>, host: &HostStream, modes: &ModeTable, may_go_off: bool) -> Result<Option<Timing>, PolicyError> {
        let Some(top) = preferred else { return Ok(None) };
        let mut steps = vec![top];
        if top.size().h >= 2160 {
            let below: Vec<Timing> = self.hdmi_candidates.iter().copied().filter(|t| t.size().h < 2160).collect();
            steps.extend(best_timing(monitor, &below, self.hdmi_no_edid));
        }
        let rule = self.usb_rule(host.size)?;
        for &t in &steps {
            if modes.get(rule.mode(Self::monitor_class(monitor, Some(t))))?.max_fps >= host.fps {
                return Ok(Some(t));
            }
        }
        Ok(if may_go_off { None } else { steps.last().copied() })
    }

    /// The timings HDMI can step down to from `now` on `monitor`, the best first: for each smaller size the monitor
    /// offers among the candidates, its best rate (none without an EDID: there is only the one timing).
    pub fn hdmi_below(&self, monitor: &Monitor, now: Timing) -> Vec<Timing> {
        let area = |t: Timing| t.size().w as u64 * t.size().h as u64;
        let Monitor::Present(_) = monitor else { return Vec::new() };
        let mut below: Vec<Timing> = self.hdmi_candidates.iter().copied().filter(|t| area(*t) < area(now)).collect();
        let mut out = Vec::new();
        // the monitor's best among them (none it offers: the rule's timing without an EDID, no step of these)
        while let Some(t) = best_timing(monitor, &below, self.hdmi_no_edid).filter(|t| below.contains(t)) {
            out.push(t);
            below.retain(|b| area(*b) < area(t));
        }
        out
    }

    /// The USB rule for a size: exact row, else the row with the same aspect (±1 %) and the smallest larger size.
    fn usb_rule(&self, size: Size) -> Result<&UsbRow2, PolicyError> {
        if let Some(r) = self.usb.iter().find(|r| r.size == size) {
            return Ok(r);
        }
        self.usb
            .iter()
            .filter(|r| r.size.aspect_diff(size) <= 0.01 && r.size.w >= size.w && r.size.h >= size.h)
            .min_by_key(|r| r.size.w as u64 * r.size.h as u64)
            .ok_or(PolicyError::NoUsbRule(size))
    }

    /// The decision. Beside a recording at most two distinct frames are written (three SCL ports starved each
    /// other, 13–19 of 25 fps, 2026-10-25; two run clean): when HDMI, USB and the recording would each write their own,
    /// the lower of the two outputs in the precedence pauses — it keeps its frame without the live picture.
    /// The H.26x encoder the recording runs on is the third such limit: USB's H.264 / H.265 stream shares it, an MJPEG
    /// one slows it; when the two would take more of its time than the budget, the lower of them in the precedence
    /// gives way ([`Self::encoder`]).
    pub fn plan(&self, inputs: &Inputs, modes: &ModeTable, revision: Revision) -> Result<Desired, PolicyError> {
        let d = self.plan_outputs(inputs, modes, revision, false, None)?;
        let one = d.media.as_ref().is_some_and(|m| !m.shot()) && written(&d) > 2;
        let d = if one { self.plan_outputs(inputs, modes, revision, true, None)? } else { d };
        match self.encoder(&d, &inputs.prefs.precedence)? {
            None => Ok(d),
            cap => self.plan_outputs(inputs, modes, revision, one, cap),
        }
    }

    /// The ms of the H.26x encoder's time a frame of `size` takes in `rows`: the row of the smallest size holding it,
    /// else the largest row's.
    fn frame_ms(rows: &[(Size, f64)], size: Size) -> f64 {
        rows.iter().find(|(s, _)| s.w >= size.w && s.h >= size.h).or(rows.last()).map_or(0.0, |(_, ms)| *ms)
    }

    /// USB's stream beside a recording on the H.26x encoder: an H.264 / H.265 stream encoded by it, an MJPEG one slowing
    /// its frames above the table's pixels a second. None when the recording and the stream fit its budget;
    /// else USB's rate with the recording above it in the precedence — the highest that fits (its `lower`; paused frees
    /// nothing here, a paused stream is still encoded), 0 when none does (`refused`) — and with USB above, the
    /// recording gives way: it has no step but not starting, so the asker is refused. A floor that forbids USB's step
    /// refuses too.
    fn encoder(&self, d: &Desired, precedence: &Precedence) -> Result<Option<u32>, PolicyError> {
        let (Some(m), Some(u)) = (d.media.as_ref().filter(|m| !m.shot()), d.usb.as_ref()) else { return Ok(None) };
        let budget = self.h26x_budget_ms;
        let rec = |rows: &[(Size, f64)]| m.fps as f64 * Self::frame_ms(rows, m.size);
        let fits = match u.codec {
            Codec::H264 | Codec::H265 => {
                let (rec, frame) = (rec(&self.h26x_frames), Self::frame_ms(&self.h26x_frames, u.size));
                if rec + u.fps as f64 * frame <= budget {
                    return Ok(None);
                }
                ((budget - rec) / frame).floor().max(0.0) as u32
            }
            Codec::Mjpeg => {
                // the highest rate of the stream's size that leaves the encoder as alone
                let alone = (self.h26x_mjpeg_px / (u.size.w as u64 * u.size.h as u64).max(1)) as u32;
                if u.fps <= alone || rec(&self.h26x_mjpeg_frames) <= budget {
                    return Ok(None);
                }
                if rec(&self.h26x_frames) <= budget { alone } else { 0 }
            }
        };
        if !precedence.above(Who::Recording, Who::Usb) {
            return Err(PolicyError::Encoder(u.size, "USB above the recording"));
        }
        match fits {
            0 if precedence.allows(Who::Usb, Step::Refused) => Ok(Some(0)),
            0 => Err(PolicyError::Encoder(u.size, "USB's floor forbids refusing it")),
            f => Ok(Some(f.min(u.fps))),
        }
    }

    /// The decision, with one output's live picture only beside a recording when `one_beside_recording`, and USB's
    /// rate at most `usb_fps` (0: no frames) where the H.26x encoder cannot carry more.
    fn plan_outputs(&self, inputs: &Inputs, modes: &ModeTable, revision: Revision, one_beside_recording: bool, usb_fps: Option<u32>) -> Result<Desired, PolicyError> {
        let prefs = inputs.prefs;
        // HDMI first: it decides the monitor class the USB mapping depends on
        let preferred = if !prefs.hdmi_enabled || prefs.standby {
            None
        } else {
            match prefs.hdmi_override {
                Some(t) if *inputs.monitor != Monitor::Absent => Some(t),
                Some(_) => None,
                None => best_timing(inputs.monitor, &self.hdmi_candidates, self.hdmi_no_edid),
            }
        };
        // what HDMI gave by the precedence (a timing it could not step down to from here is none)
        let preferred = match (preferred, prefs.giving.hdmi) {
            (Some(_), Some(Step::Off)) => None,
            (Some(p), Some(Step::Timing(t))) if self.hdmi_below(inputs.monitor, p).contains(&t) => Some(t),
            (p, _) => p,
        };
        let usb_first = prefs.precedence.above(Who::Usb, Who::Hdmi);
        // a recording comes first: it fixes the sensor mode and rate, the outputs are served from them
        let record = match &prefs.recording {
            Some(r) => Some(self.record_plan(r, prefs.record_fps, prefs.record_codec, modes)?),
            None => None,
        };
        let held = record.as_ref().map(|r| r.mode);
        // with the mode fixed by a recording, stepping HDMI down cannot buy USB a faster mode
        // USB refused by the precedence: planned as without a host's stream (its host gets no frames)
        let host = inputs.host.filter(|_| prefs.giving.usb != Some(Step::Refused) && usb_fps != Some(0));
        let hdmi_timing = match host {
            Some((host, _)) if usb_first && held.is_none() => self.hdmi_giving_way(inputs.monitor, preferred, host, modes, prefs.precedence.allows(Who::Hdmi, Step::Off))?,
            _ => preferred,
        };
        // beside a recording three frames written starve each other ([`Self::plan`]): the higher of the two outputs stays,
        // the lower pauses — its frame keeps the pictures and the menu, no live picture (no port writes it); one whose
        // floor forbids the pause is no plan. An output paused by the precedence for memory is paused anyway
        let both = one_beside_recording && record.is_some() && hdmi_timing.is_some() && host.is_some();
        let lower = if usb_first { Who::Hdmi } else { Who::Usb };
        if both && !prefs.precedence.allows(lower, Step::Paused) {
            return Err(PolicyError::NoPause(lower.name()));
        }
        let given = |o: Who| if o == Who::Hdmi { prefs.giving.hdmi } else { prefs.giving.usb };
        let pause = |o: Who| match () {
            _ if both && o == lower => Some(Pause::Recording),
            _ if given(o) == Some(Step::Paused) => Some(Pause::Short),
            _ => None,
        };
        let class = Self::monitor_class(inputs.monitor, hdmi_timing);
        // a file played: the outputs' live windows show its picture, crops in its pixels (the zoom and the panel's
        // aspect act on it as on the sensor's)
        let play = prefs.player.as_ref();

        // idle beside a UHD monitor runs the mode a 4K stream takes for that class, at HDMI's rate: a 4K stream start is
        // then a reader added, not a sensor rebuild, and HDMI gets every frame instead of a faster rate thinned
        let idle_as_4k_stream = host.is_none() && held.is_none() && prefs.boot_preset == BootPreset::Uhd && class == MonitorClass::Uhd;
        // sensor mode: from the host stream's size and the monitor class, else the boot preset
        let (mode, usb): (&Mode, Option<UsbPlan>) = match host {
            None => {
                let name = match prefs.boot_preset {
                    _ if idle_as_4k_stream => self.usb_rule(Size::new(3840, 2160))?.mode(class),
                    BootPreset::Uhd => &self.boot_4k,
                    BootPreset::Fhd => &self.boot_1080,
                };
                (held.map_or_else(|| modes.get(name), Ok)?, None)
            }
            Some((host, epoch)) => {
                let mode = match held {
                    Some(m) => m,
                    None => modes.get(self.usb_rule(host.size)?.mode(class))?,
                };
                let fps = match &record {
                    Some(r) => host.fps.min(r.fps),
                    None => host.fps.min(mode.max_fps),
                }
                .max(mode.min_fps)
                .min(usb_fps.unwrap_or(u32::MAX));
                // the zoom acts on USB as on HDMI; the display aspect is the panel's business and stays off USB
                let src = play.map_or(mode.size(), |p| p.size);
                let crop = prefs.view.zoom_window(Rect::centre_crop(src, host.size));
                // the scene's frame: the live picture in its window, the pictures beside it. At HDMI's raster, showing
                // what HDMI shows, it is HDMI's layout: the `ui` lays both out from the scene alike (`frame::plane`), and
                // HDMI's is there before USB is first built — a stream start that joins HDMI's frames does not build a
                // pool of its own first
                let frames = if hdmi_timing.is_some_and(|t| t.size() == host.size) && prefs.shows(prefs.levels.usb) == prefs.shows(prefs.levels.hdmi) { &prefs.frames.hdmi } else { &prefs.frames.usb };
                let layout = Layout::on(frames, host.size);
                let (frame, crop) = output_frame(layout, host.size, Rect::full(host.size), crop, src);
                let paused = pause(Who::Usb);
                let frame = if paused.is_some() { frame.paused() } else { frame };
                let usb = UsbPlan {
                    epoch,
                    codec: host.codec,
                    size: host.size,
                    fps,
                    crop,
                    scl_out: scl_output(crop.size(), frame.view.size()),
                    host: HostStreamRef::from(host),
                    frame,
                    frame_of: FrameOf::Own,
                    paused,
                    from: Input::Sensor,
                };
                (mode, Some(usb))
            }
        };
        // the sensor runs at the recording's rate (the encoder gets every frame at an even pace), else at the fastest
        // output's — HDMI's timing as far as the mode goes, or the host's: a slower output takes its rate of the frames
        // (`pace`), and HDMI at its timing's rate keeps the display lock
        let hdmi_fps = hdmi_timing.map(|t| t.rate_hz().clamp(mode.min_fps, mode.max_fps));
        let fps = match (&record, &usb) {
            (Some(r), _) => r.fps,
            (None, Some(u)) => u.fps.max(hdmi_fps.unwrap_or(0)),
            (None, None) => match hdmi_fps {
                Some(f) if idle_as_4k_stream => f,
                _ => mode.max_fps,
            },
        };

        let hdmi = hdmi_timing.map(|t| {
            let screen = t.size();
            let src = play.map_or(mode.size(), |p| p.size);
            let crop = prefs.view.crop(src);
            // the whole view letterboxed or pillarboxed on a 16:9 panel, the whole screen on the others (they stretch)
            let full = prefs.view.window(crop.size(), screen);
            let layout = Layout::on(&prefs.frames.hdmi, screen);
            let (frame, crop) = output_frame(layout, screen, full, crop, src);
            let paused = pause(Who::Hdmi);
            let frame = if paused.is_some() { frame.paused() } else { frame };
            HdmiPlan { timing: t, crop, scl_out: scl_output(crop.size(), frame.view.size()), frame, paused, from: Input::Sensor }
        });
        // the player: the decoder's port scaled down to what the views need, the crops moved into its pixels, the
        // live windows written by the outputs' channels of SCL dev 1 from it (no SCL 0 port: `scl_out` is the view)
        let (mut hdmi, mut usb) = (hdmi, usb);
        let player = play.map(|p| {
            let live = |f: &OutputFrame| f.way != Way::Covered;
            let outs: Vec<(Rect, Size)> = hdmi.iter().filter(|h| live(&h.frame)).map(|h| (h.crop, h.frame.view.size())).chain(usb.iter().filter(|u| live(&u.frame)).map(|u| (u.crop, u.frame.view.size()))).collect();
            let port = player_port(p.size, &outs);
            if let Some(h) = hdmi.as_mut() {
                (h.crop, h.scl_out, h.from) = (scale_rect(h.crop, p.size, port), h.frame.view.size(), Input::Player);
            }
            if let Some(u) = usb.as_mut() {
                (u.crop, u.scl_out, u.from) = (scale_rect(u.crop, p.size, port), u.frame.view.size(), Input::Player);
            }
            PlayerPlan { path: p.path.clone(), codec: p.codec, size: p.size, fps: p.fps, ring: p.ring(), port }
        });
        // USB's frame equal to HDMI's: its encoder reads HDMI's canvases at its own rate
        let hdmi_key = hdmi.as_ref().map(|h| FrameKey::hdmi(h, prefs.shows(prefs.levels.hdmi)));
        let usb = usb.map(|u| {
            let frame_of = if ring(&u, record.is_some()) {
                FrameOf::Ring
            } else if hdmi_key.as_ref() == Some(&FrameKey::usb(&u, prefs.shows(prefs.levels.usb))) {
                FrameOf::Hdmi
            } else {
                FrameOf::Own
            };
            UsbPlan { frame_of, ..u }
        });
        let usb_key = usb.as_ref().map(|u| FrameKey::usb(u, prefs.shows(prefs.levels.usb)));

        // photo and recording show the zoom window; a photo takes the window's pixels as they are (no mode change for
        // it), a recording keeps its size and has the window scaled to it
        let crop = prefs.view.zoom_window(Rect::full(mode.size()));
        // the media output's frame as the `ui` laid it out for the encoder's size
        let media_frame = |l: &Option<Layout>, size: Size| output_frame(Layout::on(l, size), size, Rect::full(size), crop, mode.size());
        let media = match (record, prefs.photo) {
            // the media output's frame with the player in it is not designed: a recording and a photo are refused while
            // a file plays (`Wish::apply`, the photo's press)
            _ if player.is_some() => None,
            (Some(r), _) => {
                // at HDMI's or USB's raster, showing what that output shows, it is that output's layout (as USB's is
                // HDMI's), so a recording that starts beside them reads their frame at once
                let shows = prefs.shows(prefs.levels.media);
                let like = |l: Option<Size>, level: u8| l == Some(r.size) && prefs.shows(level) == shows;
                let layout = if like(hdmi.as_ref().map(|h| h.timing.size()), prefs.levels.hdmi) {
                    &prefs.frames.hdmi
                } else if like(usb.as_ref().map(|u| u.size), prefs.levels.usb) {
                    if usb.as_ref().is_some_and(|u| u.frame_of == FrameOf::Hdmi) { &prefs.frames.hdmi } else { &prefs.frames.usb }
                } else {
                    &prefs.frames.media
                };
                let (frame, crop) = media_frame(layout, r.size);
                let m = MediaPlan {
                    codec: r.codec,
                    crop,
                    scl_out: scl_output(crop.size(), frame.view.size()),
                    size: r.size,
                    src_fps: fps,
                    fps: r.fps,
                    quality: 0,
                    bitrate: record_bitrate(r.size),
                    gop: r.fps,
                    file: Some(r.path.clone()),
                    frame,
                    // a photo asked during the recording: its finished frame, at its size
                    snapshot: prefs.photo.is_some(),
                    frame_of: FrameOf::Own,
                };
                // its frame HDMI's, else USB's own: its encoder reads that pool
                let key = FrameKey::media(&m, shows);
                let frame_of = if hdmi_key.as_ref() == Some(&key) {
                    FrameOf::Hdmi
                } else if usb.as_ref().is_some_and(|u| u.frame_of == FrameOf::Own) && usb_key.as_ref() == Some(&key) {
                    FrameOf::Usb
                } else {
                    FrameOf::Own
                };
                Some(MediaPlan { frame_of, ..m })
            }
            (None, Some(asked)) => {
                let size = fit_size(crop.size(), asked);
                let (frame, crop) = media_frame(&prefs.frames.media, size);
                let scl_out = scl_output(crop.size(), frame.view.size());
                Some(MediaPlan { codec: Codec::Mjpeg, crop, scl_out, size, src_fps: fps, fps, quality: PHOTO_QUALITY, bitrate: 0, gop: 0, file: None, frame, snapshot: false, frame_of: FrameOf::Own })
            }
            (None, None) => None,
        };

        Ok(Desired {
            revision,
            sensor: SensorPlan { mode: mode.name.clone(), size: mode.size(), fps, mode_max_fps: mode.max_fps, line_ns: mode.line_ns },
            hdmi,
            usb,
            media,
            image: ImagePlan { nr3d: prefs.nr3d, mirror: prefs.mirror ^ prefs.user_mirror, flip: prefs.flip ^ prefs.user_flip, iq_bin: prefs.iq_bin.clone(), mjpeg_quality: prefs.mjpeg_quality, tuning: inputs.tuning.clone() },
            player,
        })
    }
}

/// An output's frame as another output's encoder would read it:
/// the size, what of the menu it shows ([`Prefs::shows`]) and the live frame (crop, the port's output, the layout).
/// Two outputs with equal keys show the same frame: the later one's encoder reads the earlier one's pool at its own
/// rate — one port, one compositor, one `ui` layer and bake for both. Neither the rate nor the codec matters: the port
/// writes every frame of the source and each reader takes its rate of them; every pool has its rows to 32, as H.264
/// reads them.
#[derive(Debug, PartialEq, Eq)]
struct FrameKey<'a> {
    size: Size,
    shows: u8,
    crop: Rect,
    scl_out: Size,
    frame: &'a OutputFrame,
}

impl<'a> FrameKey<'a> {
    fn hdmi(h: &'a HdmiPlan, shows: u8) -> Self {
        Self { size: h.timing.size(), shows, crop: h.crop, scl_out: h.scl_out, frame: &h.frame }
    }

    fn usb(u: &'a UsbPlan, shows: u8) -> Self {
        Self { size: u.size, shows, crop: u.crop, scl_out: u.scl_out, frame: &u.frame }
    }

    fn media(m: &'a MediaPlan, shows: u8) -> Self {
        Self { size: m.size, shows, crop: m.crop, scl_out: m.scl_out, frame: &m.frame }
    }
}

/// USB's port bound into its encoder in a ring (`FrameOf::Ring`): an H.264 / H.265 stream whose frame is the
/// sensor's live picture alone, written by SCL 0's port at the stream's size (no upscaler, no pictures, not paused),
/// while nothing records — a recording takes the port-written frames counted, and reads USB's pool when its frame
/// is USB's. The ring is chosen over joining HDMI's frame: the encoder reads each band as the port writes it, so the stream leaves ~28 ms sooner
///, for one 4K port write and one frame's ring more.
fn ring(u: &UsbPlan, recording: bool) -> bool {
    matches!(u.codec, Codec::H264 | Codec::H265)
        && u.from == Input::Sensor
        && u.paused.is_none()
        && u.frame.way == Way::Live
        && u.frame.view == Rect::full(u.size)
        && u.scl_out == u.size
        && !recording
}

/// The distinct frames `d`'s scaler ports write: HDMI's, and USB's and the media encoder's own, unless covered (no
/// port writes a frame without a live part).
fn written(d: &Desired) -> usize {
    let port = |f: &OutputFrame| f.way != Way::Covered;
    d.hdmi.as_ref().is_some_and(|h| port(&h.frame)) as usize
        + d.usb.as_ref().is_some_and(|u| matches!(u.frame_of, FrameOf::Own | FrameOf::Ring) && port(&u.frame)) as usize
        + d.media.as_ref().is_some_and(|m| m.frame_of == FrameOf::Own && port(&m.frame)) as usize
}

/// JPEG quality of a photo: it is stored, not streamed, so bandwidth does not limit it.
pub const PHOTO_QUALITY: u8 = 90;

/// H.264 and H.265 bit/s of a recording (the card's and the file's size, so the same for both; H.265 spends it on a
/// better picture): twice the vendor's 10 Mbit/s at 4K (soft on fine detail), 10 Mbit/s at 1080p, the 1440p
/// size in between by its pixels.
pub fn record_bitrate(size: Size) -> u32 {
    let px = size.w as u64 * size.h as u64;
    if px >= 3840 * 2160 {
        20_000_000
    } else if px > 1920 * 1080 {
        15_000_000
    } else {
        10_000_000
    }
}

/// The crop scaled down to fit inside `asked`, keeping its aspect, never up; width a multiple of 16, height of 8
/// (JPEG blocks).
pub fn fit_size(crop: Size, asked: Size) -> Size {
    let s = (asked.w as f64 / crop.w as f64).min(asked.h as f64 / crop.h as f64).min(1.0);
    Size::new(((crop.w as f64 * s) as u32) & !15, ((crop.h as f64 * s) as u32) & !7)
}

/// The SCL output for a crop shown in `window`: each axis scaled down to the window where larger, never up, even-aligned.
pub fn scl_output(crop: Size, window: Size) -> Size {
    Size::new(crop.w.min(window.w) & !1, crop.h.min(window.h) & !1)
}

/// Largest centred window of `screen` with the aspect of `src` (letterbox/pillarbox), even-aligned.
pub fn fit_window(src: Size, screen: Size) -> Rect {
    if src.aspect_diff(screen) <= 0.01 {
        return Rect::full(screen);
    }
    let sa = src.w as f64 / src.h as f64;
    let (w, h) = if sa > screen.w as f64 / screen.h as f64 {
        (screen.w, ((screen.w as f64 / sa).round() as u32) & !1)
    } else {
        (((screen.h as f64 * sa).round() as u32) & !1, screen.h)
    };
    Rect { x: ((screen.w - w) / 2) & !1, y: ((screen.h - h) / 2) & !1, w, h }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::formats::tests::{table as formats, H264, H265, MJPEG};
    use crate::frame::Way;
    use crate::hdmi::MonitorCaps;
    use crate::modes::tests::table as modes;
    use crate::Codec;

    pub(crate) const POLICY_TOML: &str = include_str!("../../../camera/policy.toml");

    pub(crate) fn policy() -> Policy {
        let p = Policy::parse(POLICY_TOML).unwrap();
        p.validate(&modes()).unwrap();
        p
    }

    pub(crate) fn policy_hdmi_candidates() -> Vec<Timing> {
        policy().hdmi_candidates.clone()
    }

    pub(crate) fn philips() -> Monitor {
        let mut m = vec![];
        for r in [24, 25, 30, 50, 60] {
            m.push((Size::new(3840, 2160), r));
        }
        m.push((Size::new(2560, 1440), 60));
        m.push((Size::new(1920, 1080), 60));
        Monitor::Present(MonitorCaps { modes: m })
    }

    fn fhd_monitor() -> Monitor {
        Monitor::Present(MonitorCaps { modes: vec![(Size::new(1920, 1080), 60), (Size::new(1280, 720), 60)] })
    }

    /// HDMI above USB, the rest as the default.
    pub(crate) fn hdmi_over_usb() -> Precedence {
        Precedence::default().with_above(Who::Hdmi, Who::Usb)
    }

    pub(crate) fn hdmi_first() -> Prefs {
        Prefs { precedence: hdmi_over_usb(), ..Prefs::default() }
    }

    fn host(fmt: u8, frame: u8, interval: u32) -> HostStream {
        formats().resolve(fmt, frame, interval).unwrap()
    }

    #[test]
    fn every_listed_frame_is_served_beside_every_monitor_class() {
        let stock_driver = crate::modes::ParsedModes::conservative(&crate::modes::tests::live_from_file(&crate::modes::ModeTable::parse(crate::modes::tests::modes_toml()).unwrap())).unwrap();
        let single = policy().single_mode("3840x2160@30 stock");
        for (list, policy, modes) in [(formats(), policy(), modes()), (crate::formats::FormatTable::stock(), policy(), modes()), (crate::formats::FormatTable::stock(), single, stock_driver)] {
            for f in list.iter() {
                for fr in &f.frames {
                    for monitor in [Monitor::Absent, fhd_monitor(), philips()] {
                        let h = list.resolve(f.index, fr.index, 0).unwrap();
                        let d = policy.plan(&Inputs { monitor: &monitor, host: Some((&h, Epoch(1))), prefs: &Prefs::default(), tuning: &Default::default() }, &modes, Revision(2));
                        let u = d.unwrap_or_else(|e| panic!("{:?} {}: {e}", f.codec, fr.size())).usb.unwrap();
                        assert_eq!((u.codec, u.size), (f.codec, fr.size()));
                    }
                }
            }
        }
    }

    #[test]
    fn a_recording_runs_at_a_listed_standard_rate_on_the_first_mode_reaching_it() {
        let at = |w, h, fps| {
            let prefs = Prefs { recording: Some(Recording { size: Size::new(w, h), path: "v".into() }), record_fps: fps, ..Prefs::default() };
            let d = policy().plan(&Inputs { monitor: &Monitor::Absent, host: None, prefs: &prefs, tuning: &Default::default() }, &modes(), Revision(1)).unwrap();
            let m = d.media.unwrap();
            assert_eq!((d.sensor.fps, m.size), (m.fps, Size::new(w, h)));
            (d.sensor.mode, m.fps)
        };
        let s = |m: &str, f| (m.to_string(), f);
        assert_eq!(at(3840, 2160, 0), s("3840x2160@30 stock", 25), "0 = the highest listed");
        assert_eq!(at(3840, 2160, 60), s("3840x2160@30 stock", 25), "never above the list");
        assert_eq!(at(3840, 2160, 24), s("3840x2160@30 stock", 24));
        assert_eq!(at(2560, 1440, 45), s("3840x2160@30 stock", 30), "an unlisted rate → the listed one below it");
        assert_eq!(at(2560, 1440, 30), s("3840x2160@30 stock", 30), "full view while a 4K mode reaches the rate");
        assert_eq!(at(2560, 1440, 50), s("2560x1440@66 crop", 50));
        assert_eq!(at(2560, 1440, 60), s("2560x1440@66 crop", 50), "never above the list");
        assert_eq!(at(1920, 1080, 30), s("3840x2160@30 stock", 30), "scaled down from 4K");
        assert_eq!(at(1920, 1080, 50), s("1920x1080@59 2lane vts1142", 50));
        assert_eq!(at(1920, 1080, 60), s("1920x1080@81 rk40 bin 2lane", 60));
        assert_eq!(at(1920, 1080, 100), s("1920x1080@121 rk40 bin hts278", 100));
        assert_eq!(at(1920, 1080, 0), s("1920x1080@121 rk40 bin hts278", 100));
    }

    #[test]
    fn a_recording_is_encoded_in_the_codec_the_settings_name() {
        for codec in [Codec::H264, Codec::H265] {
            let prefs = Prefs { recording: Some(Recording { size: Size::new(3840, 2160), path: "v".into() }), record_codec: codec, ..Prefs::default() };
            let m = policy().plan(&Inputs { monitor: &Monitor::Absent, host: None, prefs: &prefs, tuning: &Default::default() }, &modes(), Revision(1)).unwrap().media.unwrap();
            assert_eq!((m.codec, m.bitrate), (codec, 20_000_000));
        }
    }

    #[test]
    fn idle_with_4k_monitor_runs_the_4k_stream_mode_at_hdmi_rate() {
        let d = policy().plan(&Inputs { monitor: &philips(), host: None, prefs: &Prefs::default(), tuning: &Default::default() }, &modes(), Revision(1)).unwrap();
        assert_eq!(d.sensor.mode, "3840x2160@45 hts340");
        assert_eq!(d.sensor.fps, 30);
        let h = d.hdmi.unwrap();
        assert_eq!(h.timing, Timing::P2160_30);
        assert_eq!((h.frame.window, h.frame.way), (Rect::full(Size::new(3840, 2160)), Way::Live));
        assert!(d.usb.is_none());
        // a 4K30 stream start keeps the sensor as it is
        let s = host(MJPEG, 1, 333_333);
        let d2 = policy().plan(&Inputs { monitor: &philips(), host: Some((&s, Epoch(1))), prefs: &Prefs::default(), tuning: &Default::default() }, &modes(), Revision(2)).unwrap();
        assert_eq!(d2.sensor, d.sensor);
    }

    #[test]
    fn idle_1080_preset_on_a_fhd_monitor() {
        let prefs = Prefs { boot_preset: BootPreset::Fhd, ..Prefs::default() };
        let d = policy().plan(&Inputs { monitor: &fhd_monitor(), host: None, prefs: &prefs, tuning: &Default::default() }, &modes(), Revision(1)).unwrap();
        assert_eq!(d.sensor.mode, "1920x1080@121 rk40 bin hts278");
        assert_eq!(d.hdmi.unwrap().timing, Timing::P1080_60);
    }

    #[test]
    fn usb_1080p_at_120_with_fhd_monitor() {
        let h = host(MJPEG, 4, 83_333);
        let d = policy().plan(&Inputs { monitor: &fhd_monitor(), host: Some((&h, Epoch(3))), prefs: &Prefs::default(), tuning: &Default::default() }, &modes(), Revision(2)).unwrap();
        assert_eq!(d.sensor.mode, "1920x1080@121 rk40 bin hts278");
        assert_eq!(d.sensor.fps, 120);
        let u = d.usb.unwrap();
        assert_eq!((u.codec, u.size, u.fps, u.epoch), (Codec::Mjpeg, Size::new(1920, 1080), 120, Epoch(3)));
        assert_eq!(u.crop, Rect::full(Size::new(1920, 1080)));
    }

    #[test]
    fn usb_4k_with_4k_monitor_takes_the_4k45_mode_and_hdmi_keeps_4k() {
        let h = host(MJPEG, 1, 222_222); // 45 fps asked
        let d = policy().plan(&Inputs { monitor: &philips(), host: Some((&h, Epoch(1))), prefs: &hdmi_first(), tuning: &Default::default() }, &modes(), Revision(2)).unwrap();
        assert_eq!(d.sensor.mode, "3840x2160@45 hts340");
        assert_eq!(d.sensor.fps, 45);
        assert_eq!(d.hdmi.unwrap().timing, Timing::P2160_30);
    }

    #[test]
    fn usb_3840x1600_with_4k_monitor_is_a_centre_crop_of_the_4k45_frame() {
        let h = host(MJPEG, 2, 166_666);
        let d = policy().plan(&Inputs { monitor: &philips(), host: Some((&h, Epoch(1))), prefs: &hdmi_first(), tuning: &Default::default() }, &modes(), Revision(2)).unwrap();
        assert_eq!(d.sensor.mode, "3840x2160@45 hts340");
        assert_eq!(d.usb.unwrap().crop, Rect { x: 0, y: 280, w: 3840, h: 1600 });
    }

    #[test]
    fn a_live_window_off_the_port_grid_is_written_widened_with_its_crop_widened_alike() {
        let r = |x, y, w, h| Rect { x, y, w, h };
        let frame = Size::new(3840, 2160);
        let sensor = r(0, 0, 3840, 2160);
        // on the grid: as it is
        assert_eq!(port_view(frame, r(1920, 0, 1920, 2160), r(960, 0, 1920, 2160), sensor), (r(1920, 0, 1920, 2160), r(960, 0, 1920, 2160)));
        // 10 columns right of the grid, 10 short of the frame's edge, shown 1:1: widened by 10 each side, the crop alike
        let (view, crop) = port_view(frame, r(1930, 0, 1900, 2160), r(970, 0, 1900, 2160), sensor);
        assert_eq!(view, r(1920, 0, 1920, 2160));
        assert_eq!(crop, r(960, 0, 1920, 2160));
        // shown at 1.5 (the crop 1.5 times the window): the widening 1.5 times as wide in the sensor's pixels
        let (view, crop) = port_view(frame, r(1930, 0, 1900, 2160), r(115, 0, 2850, 2160), sensor);
        assert_eq!((view, crop), (r(1920, 0, 1920, 2160), r(100, 0, 2880, 2160)));
        // at the sensor's edge the crop stays inside it
        let (_, crop) = port_view(frame, r(1930, 0, 1900, 2160), r(0, 0, 3800, 2160), sensor);
        assert!(crop.x == 0 && crop.w <= 3840, "{crop:?}");
    }

    #[test]
    fn usb_without_monitor_takes_the_fast_column() {
        let h = host(MJPEG, 6, 151_515); // 3840x1440 @66
        let d = policy().plan(&Inputs { monitor: &Monitor::Absent, host: Some((&h, Epoch(1))), prefs: &Prefs::default(), tuning: &Default::default() }, &modes(), Revision(2)).unwrap();
        assert_eq!(d.sensor.mode, "3840x1440@66 hts340");
        assert_eq!(d.sensor.fps, 66);
        assert!(d.hdmi.is_none());
    }

    fn usb_first(fmt: u8, frame: u8, interval: u32, monitor: &Monitor, prefs: Prefs) -> (Option<Timing>, u32) {
        let h = host(fmt, frame, interval);
        let prefs = Prefs { precedence: Precedence::default(), ..prefs };
        let d = policy().plan(&Inputs { monitor, host: Some((&h, Epoch(1))), prefs: &prefs, tuning: &Default::default() }, &modes(), Revision(2)).unwrap();
        (d.hdmi.map(|x| x.timing), d.usb.unwrap().fps)
    }

    #[test]
    fn usb_priority_steps_hdmi_down_only_as_far_as_the_rate_needs() {
        let m = philips();
        // what the 4K45 mode beside HDMI 4K delivers: HDMI keeps 4K
        assert_eq!(usb_first(MJPEG, 1, 333_333, &m, Prefs::default()), (Some(Timing::P2160_30), 30));
        assert_eq!(usb_first(MJPEG, 4, 250_000, &m, Prefs::default()), (Some(Timing::P2160_30), 40));
        // more than 45: the monitor's best below 4K
        assert_eq!(usb_first(MJPEG, 4, 83_333, &m, Prefs::default()), (Some(Timing::P1440_60), 120));
        assert_eq!(usb_first(MJPEG, 5, 151_515, &m, Prefs::default()), (Some(Timing::P1440_60), 66));
        assert_eq!(usb_first(MJPEG, 7, 166_666, &m, Prefs::default()), (Some(Timing::P1440_60), 60));
        // 4K45: HDMI keeps 4K (SCL writes both at 45); more than any HDMI step allows: HDMI off
        assert_eq!(usb_first(MJPEG, 1, 222_222, &m, Prefs::default()), (Some(Timing::P2160_30), 45));
        assert_eq!(usb_first(MJPEG, 6, 151_515, &m, Prefs::default()), (None, 66));
    }

    #[test]
    fn usb_priority_starts_from_the_chosen_timing_and_hdmi_priority_keeps_it() {
        let fixed = Prefs { hdmi_override: Some(Timing::P1080_60), ..Prefs::default() };
        assert_eq!(usb_first(MJPEG, 4, 83_333, &philips(), fixed.clone()), (Some(Timing::P1080_60), 120));
        assert_eq!(usb_first(MJPEG, 2, 166_666, &philips(), fixed.clone()), (None, 60));
        assert_eq!(usb_first(MJPEG, 2, 166_666, &fhd_monitor(), Prefs::default()), (None, 60));
        // HDMI above USB: the timing stays, USB takes what the class gives
        let h = host(MJPEG, 2, 166_666);
        let fixed = Prefs { precedence: hdmi_over_usb(), ..fixed };
        let d = policy().plan(&Inputs { monitor: &philips(), host: Some((&h, Epoch(1))), prefs: &fixed, tuning: &Default::default() }, &modes(), Revision(2)).unwrap();
        assert_eq!((d.hdmi.map(|x| x.timing), d.usb.unwrap().fps), (Some(Timing::P1080_60), 52));
    }

    #[test]
    fn usb_priority_without_a_stream_shows_the_preferred_timing() {
        let prefs = Prefs { precedence: Precedence::default(), ..Prefs::default() };
        let d = policy().plan(&Inputs { monitor: &philips(), host: None, prefs: &prefs, tuning: &Default::default() }, &modes(), Revision(1)).unwrap();
        assert_eq!(d.hdmi.unwrap().timing, Timing::P2160_30);
    }

    /// HDMI's floor above `off` keeps it on where the scaler would turn it off for USB's rate: its lowest timing, USB
    /// at what that leaves.
    #[test]
    fn hdmi_s_floor_keeps_it_on_beside_the_fastest_usb() {
        let h = host(MJPEG, 6, 151_515);
        for floor in ["lower", "paused"] {
            let precedence = Precedence::parse(&format!("recording > player > usb > hdmi:{floor} > photo > drawing > picture")).unwrap();
            let prefs = Prefs { precedence, ..Prefs::default() };
            let d = policy().plan(&Inputs { monitor: &philips(), host: Some((&h, Epoch(1))), prefs: &prefs, tuning: &Default::default() }, &modes(), Revision(2)).unwrap();
            assert_eq!(d.hdmi.map(|x| x.timing), Some(Timing::P1440_60), "{floor}");
            assert!(d.usb.unwrap().fps < 66);
        }
    }

    /// What HDMI and USB gave by the precedence shapes the plan: HDMI at a lower timing, paused (its frame without the
    /// live picture), off; USB paused, refused (planned as with no stream). A timing HDMI cannot step down to from here
    /// is none.
    #[test]
    fn what_hdmi_and_usb_gave_shapes_the_plan() {
        let p = policy();
        assert_eq!(p.hdmi_below(&philips(), Timing::P2160_30), [Timing::P1440_60, Timing::P1080_60]);
        assert_eq!(p.hdmi_below(&philips(), Timing::P1080_60), []);
        assert_eq!(p.hdmi_below(&Monitor::PresentNoEdid, Timing::P1080_60), []);
        let plan = |giving: Giving, host: Option<&HostStream>| {
            let prefs = Prefs { giving, ..Prefs::default() };
            p.plan(&Inputs { monitor: &philips(), host: host.map(|h| (h, Epoch(1))), prefs: &prefs, tuning: &Default::default() }, &modes(), Revision(1)).unwrap()
        };
        let hdmi = |step| Giving { hdmi: Some(step), usb: None };
        assert_eq!(plan(hdmi(Step::Timing(Timing::P1080_60)), None).hdmi.unwrap().timing, Timing::P1080_60);
        assert_eq!(plan(hdmi(Step::Timing(Timing::P2160_25)), None).hdmi.unwrap().timing, Timing::P2160_30, "not below: none");
        let paused = plan(hdmi(Step::Paused), None).hdmi.unwrap();
        assert_eq!((paused.timing, paused.paused, paused.frame.way), (Timing::P2160_30, Some(Pause::Short), Way::Covered));
        assert!(plan(hdmi(Step::Off), None).hdmi.is_none());
        let h = host(MJPEG, 1, 222_222);
        let usb = |step| Giving { hdmi: None, usb: Some(step) };
        let paused = plan(usb(Step::Paused), Some(&h)).usb.unwrap();
        assert_eq!((paused.paused, paused.frame.way), (Some(Pause::Short), Way::Covered));
        let refused = plan(usb(Step::Refused), Some(&h));
        assert_eq!((refused.usb, refused.sensor), (None, plan(Giving::default(), None).sensor));
    }

    /// Without a recording the sensor runs at the fastest output's rate: a host slower than HDMI's timing takes its rate
    /// of HDMI's frames, HDMI keeps its timing's rate (and the display lock); a faster host sets the sensor's.
    #[test]
    fn the_sensor_runs_at_the_fastest_output_s_rate() {
        let plan = |interval: u32| {
            let h = host(MJPEG, 1, interval);
            policy().plan(&Inputs { monitor: &philips(), host: Some((&h, Epoch(1))), prefs: &Prefs::default(), tuning: &Default::default() }, &modes(), Revision(1)).unwrap()
        };
        // 4K at its slowest listed interval beside 2160p30: the sensor and HDMI at 30, USB at the mode's floor reading
        // HDMI's pool
        let slow = plan(1_000_000);
        let (u, h) = (slow.usb.as_ref().unwrap(), slow.hdmi.as_ref().unwrap());
        assert_eq!((slow.sensor.fps, h.rate(slow.sensor.fps), u.frame_of), (30, 30, FrameOf::Hdmi));
        assert!(u.fps < 30, "{}", u.fps);
        // 4K45: the sensor at the host's 45, HDMI takes 30 of them
        let fast = plan(222_222);
        assert_eq!((fast.sensor.fps, fast.hdmi.as_ref().unwrap().rate(fast.sensor.fps), fast.usb.as_ref().unwrap().fps), (45, 30, 45));
    }

    /// A 4K recording and USB's stream on the H.26x encoder (the measured table): the stream gives what the encoder
    /// cannot carry beside the recording — its rate first, no frames when none is left — or, put above the recording,
    /// the recording is not started. MJPEG is the JPEG encoder's and never gives way here.
    #[test]
    fn usb_s_h26x_stream_gives_way_to_a_recording_on_the_shared_encoder() {
        let rec = |precedence: Precedence| Prefs { recording: Some(Recording { size: Size::new(3840, 2160), path: "v".into() }), precedence, ..Prefs::default() };
        let plan = |p: &Policy, prefs: &Prefs, fmt: u8, frame: u8| {
            let h = host(fmt, frame, 333_333);
            p.plan(&Inputs { monitor: &philips(), host: Some((&h, Epoch(1))), prefs, tuning: &Default::default() }, &modes(), Revision(1))
        };
        let usb = |d: &Desired| d.usb.as_ref().map(|u| (u.size, u.fps));
        let p = policy();
        let d = plan(&p, &rec(Precedence::default()), H264, 6).unwrap();
        let m = d.media.as_ref().unwrap();
        assert_eq!((m.size, m.fps), (Size::new(3840, 2160), 25), "the recording whole");
        // 4K beside 4K: what the budget leaves after the recording's 25 frames
        assert_eq!(usb(&d), Some((Size::new(3840, 2160), 7)));
        // its picture still HDMI's: USB and the recording read HDMI's pool, each at its rate — one frame written, HDMI
        // live at its timing
        let (u, h) = (d.usb.as_ref().unwrap(), d.hdmi.as_ref().unwrap());
        assert_eq!((u.frame_of, m.frame_of, h.paused, h.timing), (FrameOf::Hdmi, FrameOf::Hdmi, None, Timing::P2160_30));
        assert_eq!(written(&d), 1);
        // 1440: the rate the budget leaves; 1080 fits at the recording's rate
        assert_eq!(usb(&plan(&p, &rec(Precedence::default()), H264, 5).unwrap()), Some((Size::new(2560, 1440), 16)));
        assert_eq!(usb(&plan(&p, &rec(Precedence::default()), H264, 4).unwrap()), Some((Size::new(1920, 1080), 25)));
        assert_eq!(usb(&plan(&p, &rec(Precedence::default()), H265, 6).unwrap()), Some((Size::new(3840, 2160), 7)), "H.265 alike");
        assert_eq!(usb(&plan(&p, &rec(Precedence::default()), MJPEG, 1).unwrap()), Some((Size::new(3840, 2160), 25)), "MJPEG 4K slows a 4K25 recording within the budget");
        // USB above the recording: the recording gives way, and its only step is not starting
        let above = Precedence::parse("usb > recording > player > hdmi > photo > drawing > picture").unwrap();
        assert!(matches!(plan(&p, &rec(above.clone()), H264, 6), Err(PolicyError::Encoder(..))));
        assert_eq!(usb(&plan(&p, &rec(above), H264, 4).unwrap()), Some((Size::new(1920, 1080), 25)), "what fits is not refused");
        // nothing left beside the recording: USB refused (no frames), unless its floor forbids it
        let tight = Policy::parse(&POLICY_TOML.replace("budget_ms = 900", "budget_ms = 700")).unwrap();
        let d = plan(&tight, &rec(Precedence::default()), H264, 6).unwrap();
        assert!(d.usb.is_none() && d.media.is_some());
        let floor = Precedence::parse("recording > player > usb:lower > hdmi > photo > drawing > picture").unwrap();
        assert!(matches!(plan(&tight, &rec(floor), H264, 6), Err(PolicyError::Encoder(..))));
        // without a recording the stream is the encoder's alone
        let alone = plan(&p, &Prefs::default(), H264, 6).unwrap();
        assert_eq!(usb(&alone), Some((Size::new(3840, 2160), 30)));
    }

    /// USB's MJPEG stream slows the H.26x encoder beside it above the table's pixels a second (the measured table): a
    /// recording whose frames no longer fit takes the stream down to that rate; put above the recording, USB refuses
    /// the recording.
    #[test]
    fn usb_s_mjpeg_4k_stream_gives_way_to_a_recording_it_slows_past_the_encoder_s_budget() {
        let rec = |w, h, fps, precedence: Precedence| Prefs { recording: Some(Recording { size: Size::new(w, h), path: "v".into() }), record_fps: fps, precedence, ..Prefs::default() };
        let plan = |prefs: &Prefs, frame: u8, interval: u32| {
            let h = host(MJPEG, frame, interval);
            policy().plan(&Inputs { monitor: &philips(), host: Some((&h, Epoch(1))), prefs, tuning: &Default::default() }, &modes(), Revision(1))
        };
        let usb = |d: &Desired| d.usb.as_ref().map(|u| (u.size, u.fps));
        let media = |d: &Desired| d.media.as_ref().map(|m| (m.size, m.fps));
        let q = Precedence::default;
        let uhd = Size::new(3840, 2160);
        // 1440@50 and 1080@100 beside 4K above 25 fps: USB at 25, the recording whole; 10 asked stays
        for (interval, fps) in [(222_222, 25), (333_333, 25), (1_000_000, 10)] {
            let d = plan(&rec(2560, 1440, 50, q()), 1, interval).unwrap();
            assert_eq!((usb(&d), media(&d)), (Some((uhd, fps)), Some((Size::new(2560, 1440), 50))), "4K at {interval}");
            let d = plan(&rec(1920, 1080, 100, q()), 1, interval).unwrap();
            assert_eq!((usb(&d), media(&d)), (Some((uhd, fps)), Some((Size::new(1920, 1080), 100))));
        }
        // 3840x1600 beside 1080@100: what the same pixels a second allow
        assert_eq!(usb(&plan(&rec(1920, 1080, 100, q()), 2, 166_666).unwrap()), Some((Size::new(3840, 1600), 33)));
        // the rates below fit beside 4K
        assert_eq!(usb(&plan(&rec(2560, 1440, 30, q()), 1, 222_222).unwrap()), Some((uhd, 30)));
        assert_eq!(usb(&plan(&rec(1920, 1080, 60, q()), 1, 222_222).unwrap()), Some((uhd, 45)));
        // streams at the recording's size and rate leave the encoder as alone
        assert_eq!(usb(&plan(&rec(2560, 1440, 50, q()), 5, 151_515).unwrap()), Some((Size::new(2560, 1440), 50)));
        assert_eq!(usb(&plan(&rec(1920, 1080, 100, q()), 4, 83_333).unwrap()), Some((Size::new(1920, 1080), 100)));
        // USB above the recording: the recording is refused, unless the stream already fits
        let above = || Precedence::parse("usb > recording > player > hdmi > photo > drawing > picture").unwrap();
        assert!(matches!(plan(&rec(2560, 1440, 50, above()), 1, 333_333), Err(PolicyError::Encoder(..))));
        assert_eq!(usb(&plan(&rec(2560, 1440, 50, above()), 1, 1_000_000).unwrap()), Some((uhd, 10)));
    }

    #[test]
    fn conservative_profile_runs_everything_in_mode_0() {
        let parsed = ModeTable::parse(crate::modes::tests::modes_toml()).unwrap();
        let live = crate::modes::tests::live_from_file(&parsed);
        let table = crate::modes::ParsedModes::conservative(&live).unwrap();
        let name = table.iter().next().unwrap().name.clone();
        let p = policy().single_mode(&name);
        p.validate(&table).unwrap();
        for (i, prio) in [(1, Precedence::default()), (4, hdmi_over_usb())] {
            let h = host(MJPEG, i, 83_333);
            let prefs = Prefs { precedence: prio, ..Prefs::default() };
            let d = p.plan(&Inputs { monitor: &philips(), host: Some((&h, Epoch(1))), prefs: &prefs, tuning: &Default::default() }, &table, Revision(1)).unwrap();
            assert_eq!(d.sensor.mode, name);
        }
    }

    #[test]
    fn hdmi_override_wins_when_a_monitor_is_present() {
        let prefs = Prefs { hdmi_override: Some(Timing::P1440_60), ..Prefs::default() };
        let d = policy().plan(&Inputs { monitor: &philips(), host: None, prefs: &prefs, tuning: &Default::default() }, &modes(), Revision(1)).unwrap();
        assert_eq!(d.hdmi.unwrap().timing, Timing::P1440_60);
        let d = policy().plan(&Inputs { monitor: &Monitor::Absent, host: None, prefs: &prefs, tuning: &Default::default() }, &modes(), Revision(1)).unwrap();
        assert!(d.hdmi.is_none(), "no monitor: no HDMI even with an override");
    }

    #[test]
    fn hdmi_window_letterboxes_a_wide_sensor_frame() {
        // 3840x1600 sensor mode on a 16:9 screen → bars top and bottom
        let w = fit_window(Size::new(3840, 1600), Size::new(1920, 1080));
        assert_eq!(w, Rect { x: 0, y: 140, w: 1920, h: 800 });
        assert_eq!(fit_window(Size::new(3840, 2160), Size::new(1920, 1080)), Rect::full(Size::new(1920, 1080)));
    }

    #[test]
    fn a_letterboxed_hdmi_frame_is_a_canvas_with_black_around_the_live_window() {
        let p = policy().single_mode("3840x1600@60 hts340");
        let d = p.plan(&Inputs { monitor: &philips(), host: None, prefs: &Prefs::default(), tuning: &Default::default() }, &modes(), Revision(1)).unwrap();
        let h = d.hdmi.unwrap();
        let screen = h.timing.size();
        // the bars are still content: black in the canvases, the port writes the window only
        assert_eq!((h.frame.window, h.frame.way, h.frame.visible.clone()), (fit_window(Size::new(3840, 1600), screen), Way::Canvas, vec![h.frame.window]));
        assert_eq!((h.frame.view, h.crop, h.scl_out), (h.frame.window, Rect::full(d.sensor.size), h.frame.window.size()));
    }

    #[test]
    fn small_sizes_use_the_same_aspect_row() {
        let h = host(H264, 3, 333_333); // 1280x720 H.264
        let d = policy().plan(&Inputs { monitor: &fhd_monitor(), host: Some((&h, Epoch(1))), prefs: &Prefs::default(), tuning: &Default::default() }, &modes(), Revision(2)).unwrap();
        assert_eq!(d.sensor.mode, "1920x1080@121 rk40 bin hts278");
        assert_eq!(d.usb.as_ref().unwrap().fps, 30);
        // 4:3 sizes: the 4:3 row, a centre crop of the binned frame
        for frame in [1, 2] {
            let h = host(H264, frame, 333_333); // 320x240, 640x480
            let d = policy().plan(&Inputs { monitor: &fhd_monitor(), host: Some((&h, Epoch(1))), prefs: &Prefs::default(), tuning: &Default::default() }, &modes(), Revision(2)).unwrap();
            assert_eq!(d.sensor.mode, "1920x1080@121 rk40 bin hts278");
            assert_eq!(d.usb.as_ref().unwrap().crop, Rect { x: 240, y: 0, w: 1440, h: 1080 });
        }
        // a size of no aspect a row has: an error, not a silent guess
        let h = HostStream { size: Size::new(1000, 1000), ..host(H264, 2, 333_333) };
        assert!(matches!(policy().plan(&Inputs { monitor: &fhd_monitor(), host: Some((&h, Epoch(1))), prefs: &Prefs::default(), tuning: &Default::default() }, &modes(), Revision(2)), Err(PolicyError::NoUsbRule(_))));
    }

    #[test]
    fn display_aspect_crops_like_the_vendor_menu() {
        let f = Size::new(3840, 2160);
        let v = |aspect, zoom| View { aspect, zoom };
        assert_eq!(v(ScreenAspect::Wide, 0).crop(f), Rect::full(f));
        assert_eq!(v(ScreenAspect::Sixteen10, 0).crop(f), Rect { x: 192, y: 0, w: 3456, h: 2160 });
        assert_eq!(v(ScreenAspect::Four3, 0).crop(f), Rect { x: 480, y: 0, w: 2880, h: 2160 });
        let z = v(ScreenAspect::Wide, 10).crop(f); // ×2
        assert_eq!(z, Rect { x: 960, y: 540, w: 1920, h: 1080 });
        assert_eq!(v(ScreenAspect::Four3, 0).window(Size::new(2880, 2160), Size::new(3840, 2160)), Rect::full(Size::new(3840, 2160)));
    }

    #[test]
    fn zoom_reaches_the_hdmi_plan() {
        let mut prefs = Prefs::default();
        prefs.view.zoom = 10;
        let d = policy().plan(&Inputs { monitor: &philips(), host: None, prefs: &prefs, tuning: &Default::default() }, &modes(), Revision(1)).unwrap();
        let h = d.hdmi.unwrap();
        assert_eq!((h.crop.w, h.crop.h), (d.sensor.size.w / 2, d.sensor.size.h / 2));
        assert_eq!(h.frame.window, Rect::full(h.timing.size()));
        // the port never scales up: the crop comes at its own size, the HDMI upscaler takes it to the screen
        assert_eq!((h.scl_out, h.frame.view.size()), (h.crop.size(), h.timing.size()));
    }

    /// The scene's frame for `raster`: the live picture in the right half, `n` pictures in the left half's quarters.
    pub(crate) fn beside(n: u64, raster: Size) -> Layout {
        let (w, h) = (raster.w / 2, raster.h / 2);
        let r = |x, y, w, h| Rect { x, y, w, h };
        let windows = (0..n as u32).map(|k| r(0, k * h, w, h)).collect();
        Layout { raster, live: r(w, 0, w, raster.h), windows }
    }

    #[test]
    fn the_scenes_frame_places_the_live_picture_and_the_pictures_for_its_raster_only() {
        let r = |x, y, w, h| Rect { x, y, w, h };
        let at = |frames: Frames| policy().plan(&Inputs { monitor: &philips(), host: None, prefs: &Prefs { frames, ..Prefs::default() }, tuning: &Default::default() }, &modes(), Revision(1)).unwrap().hdmi.unwrap();
        let none = at(Frames::default());
        let uhd = Size::new(3840, 2160);
        let two = at(Frames { hdmi: Some(beside(2, uhd)), usb: None, media: None, scene: true });
        // the live picture in its half shows the centre half of the view, the port writing that half of the canvas
        assert_eq!((two.frame.window, two.frame.view, two.crop, two.scl_out), (r(1920, 0, 1920, 2160), r(1920, 0, 1920, 2160), r(960, 0, 1920, 2160), Size::new(1920, 2160)));
        // the pictures beside it lie in the canvases as the scene has them
        assert_eq!((two.frame.way, &two.frame.windows), (Way::Canvas, &beside(2, uhd).windows));
        assert_eq!(two.frame.visible, vec![r(1920, 0, 1920, 2160)]);
        assert_eq!(two.timing, none.timing, "the layout never changes the timing");
        // laid out for another raster: the live picture whole until the ui lays it out again
        let other = at(Frames { hdmi: Some(beside(2, Size::new(1920, 1080))), scene: true, ..Frames::default() });
        assert_eq!((other.frame.window, other.frame.way, other.frame.windows.len()), (r(0, 0, 3840, 2160), Way::Live, 0));
    }

    #[test]
    fn the_live_window_shows_the_part_of_the_view_its_aspect_takes_on_every_panel() {
        for aspect in [ScreenAspect::Wide, ScreenAspect::Sixteen10, ScreenAspect::Four3] {
            let prefs = Prefs { frames: Frames { hdmi: Some(beside(1, Size::new(3840, 2160))), scene: true, ..Frames::default() }, view: View { aspect, zoom: 0 }, ..Prefs::default() };
            let h = policy().plan(&Inputs { monitor: &philips(), host: None, prefs: &prefs, tuning: &Default::default() }, &modes(), Revision(1)).unwrap().hdmi.unwrap();
            let full = policy().plan(&Inputs { monitor: &philips(), host: None, prefs: &Prefs { view: View { aspect, zoom: 0 }, ..Prefs::default() }, tuning: &Default::default() }, &modes(), Revision(1)).unwrap().hdmi.unwrap();
            // half the width of what the whole screen shows, its full height
            assert_eq!((h.crop.w, h.crop.h), ((full.crop.w / 2) & !1, full.crop.h), "{aspect:?}");
        }
    }

    #[test]
    fn the_scenes_frame_reaches_usb_with_or_without_a_monitor() {
        let h = host(MJPEG, 1, 222_222);
        let r = |x, y, w, h| Rect { x, y, w, h };
        for monitor in [philips(), fhd_monitor(), Monitor::Absent] {
            let plan = |prefs: &Prefs| policy().plan(&Inputs { monitor: &monitor, host: Some((&h, Epoch(1))), prefs, tuning: &Default::default() }, &modes(), Revision(1)).unwrap();
            let plain = plan(&hdmi_first());
            // the `ui` lays a 4K raster out alike for both outputs at one level
            let layout = Some(beside(2, Size::new(3840, 2160)));
            let with = plan(&Prefs { frames: Frames { hdmi: layout.clone(), usb: layout, media: None, scene: true }, ..hdmi_first() });
            assert_eq!((plain.sensor, plain.media), (with.sensor, with.media));
            assert_eq!(plain.hdmi.map(|x| x.timing), with.hdmi.as_ref().map(|x| x.timing));
            // USB: the scene's frame, the encoder unchanged
            let (p, u) = (plain.usb.unwrap(), with.usb.unwrap());
            assert!(p.frame.way == Way::Live && u.frame.way == Way::Canvas);
            assert_eq!((p.codec, p.size, p.fps), (u.codec, u.size, u.fps));
            assert_eq!((p.frame.window, u.frame.window), (r(0, 0, 3840, 2160), r(1920, 0, 1920, 2160)));
            assert_eq!(u.frame.windows, vec![r(0, 0, 1920, 1080), r(0, 1080, 1920, 1080)]);
        }
    }
}
