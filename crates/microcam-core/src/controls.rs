//! The camera's settings as one typed list, the one place they live: whoever changes one — the menu, the USB host, the
//! CLI — changes the same entry, and every entry always has a value. Each entry is driven one of three ways: a picture
//! knob of the ISP (stored once the ISP took it, or at once while none runs: it lands with the next IQ load), a wish the
//! pipeline builds (its value is the wishes' as built, read back by [`List::follow`]), or a value only kept here (the
//! next recording's size). Switching an auto off, or locking it, fixes what the auto runs at that moment: the pipeline
//! reads it ([`List::reads`]) and hands it in with the change ([`Now`]).
//!
//! The rules live here, without I/O: what a request turns into, clamping and steps, availability, every knob's value
//! for the ISP ([`List::tuning`]), and the settings file's contents: what the user set himself.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::hdmi::{Monitor, Timing};
use crate::policy::{BootPreset, Prefs, ScreenAspect, MAX_ZOOM};
use crate::precedence::{Precedence, Who};
use crate::policy::Policy;
use crate::tuning::{ExposureNow, Knob, Tuning, WhiteBalanceNow, AS_TUNED, GAIN_UNIT};
use crate::{Codec, Size};

/// A control's value: a number, a switch or one of its `values`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Value {
    Bool(bool),
    Int(i64),
    Text(String),
}

impl std::fmt::Display for Value {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Value::Bool(b) => write!(f, "{b}"),
            Value::Int(i) => write!(f, "{i}"),
            Value::Text(t) => write!(f, "{t}"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Kind {
    Int,
    Bool,
    Choice,
    /// every one of `values` once, the most important first, as text: `a > b:floor > c` (`precedence`)
    Order,
}

/// One entry as clients see it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Control {
    pub name: String,
    pub kind: Kind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step: Option<i64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub values: Vec<String>,
    pub default: Value,
    pub value: Value,
    /// kept in the settings file across power-offs
    pub kept: bool,
    /// false: it cannot be changed now, `why` says why
    pub available: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub why: Option<String>,
}

/// Why a `set` or `adjust` was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Code {
    /// no control of that name
    Unknown,
    /// a value of the wrong kind, a choice not among `values`, an adjust of a switch
    Invalid,
    /// it cannot be changed now (`why`)
    Unavailable,
    /// the pipeline could not build it, or an action failed on the way: the old state stays
    Refused,
    /// no SD card in the camera
    NoCard,
    /// the SD card is with the USB host
    CardShared,
    /// one of its kind is running already (a photo, a recording starting or stopping, a format, this control's build)
    Busy,
    /// a file is playing
    Playing,
    /// a recording is running
    Recording,
    /// the card is being formatted
    Formatting,
    /// the camera lacks the resources for it (the media heap, a pool, a region): the refusal's `holders` name what holds
    /// them now
    Resources,
}

/// A refusal: why, for the program to word, the text for people, and — refused for lack of resources — what holds them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refusal {
    pub code: Code,
    pub error: String,
    pub holders: Vec<Holder>,
}

impl Refusal {
    pub fn new(code: Code, error: impl ToString) -> Self {
        Self { code, error: error.to_string(), holders: Vec::new() }
    }

    /// Refused for lack of resources, `holders` holding them.
    pub fn resources(error: impl ToString, holders: Vec<Holder>) -> Self {
        Self { code: Code::Resources, error: error.to_string(), holders }
    }
}

/// Something running now that holds the camera's resources, as a refusal for lack of them names it — what the user
/// could turn off or lower — with the bytes of the media heap it holds, when known.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Holder {
    #[serde(flatten)]
    pub what: Held,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bytes: Option<u64>,
}

/// What holds them.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "holder", rename_all = "kebab-case")]
pub enum Held {
    /// HDMI at its timing
    Hdmi { timing: Timing },
    /// the USB stream: its codec, size and rate
    Usb { codec: Codec, w: u32, h: u32, fps: u32 },
    /// the recording at its size
    Recording { w: u32, h: u32 },
    /// a photo being taken at its size
    Photo { w: u32, h: u32 },
    /// a video playing: its path on the camera
    Player { file: String },
    /// one of the menu's pictures: a `resource` by its id (a frozen picture, a gallery's photo …)
    Picture { id: String },
    /// the menu's drawing beside its pictures: its tiles, windows and the frames' patches
    Drawing,
}

impl Held {
    /// Its place in the precedence.
    pub fn who(&self) -> Who {
        match self {
            Held::Hdmi { .. } => Who::Hdmi,
            Held::Usb { .. } => Who::Usb,
            Held::Recording { .. } => Who::Recording,
            Held::Photo { .. } => Who::Photo,
            Held::Player { .. } => Who::Player,
            Held::Picture { .. } => Who::Picture,
            Held::Drawing => Who::Drawing,
        }
    }
}

/// `holders` ordered by the bytes they hold, the most first, those of unknown bytes last; nothing that holds none.
pub fn ordered(mut holders: Vec<Holder>) -> Vec<Holder> {
    holders.retain(|h| h.bytes != Some(0));
    holders.sort_by_key(|h| std::cmp::Reverse(h.bytes.map_or(0, |b| b + 1)));
    holders
}

/// Who asks a change: the user (the menu, the CLI), or a USB host, whose changes last while it is connected and are
/// never kept ([`List::host_gone`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum By {
    User,
    Host,
}

/// What a request asks of a control.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Change {
    Set(Value),
    /// an int: value + steps × step; a choice: that many places along `values`; clamped at the ends
    Adjust(i64),
}

/// How a control acts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Drive {
    /// a picture knob of the ISP, applied at once
    Knob(Knob),
    ExposureLock,
    WhiteBalanceLock,
    /// a wish the pipeline builds; the value is the one built
    Built,
    /// only kept here
    Stored,
}

const POWER_LINE: [&str; 3] = ["off", "50", "60"];
const ASPECTS: [&str; 3] = ["16:9", "16:10", "4:3"];
/// The highest standard recording rate `policy.toml [[record]]` may list.
const MAX_RECORD_FPS: i64 = 120;
/// ISP 3D noise reduction levels on this SoC family (SigmaStar: at most 2 on Tiramisu chips).
const MAX_NR3D: i64 = 2;

const RECORDING: &str = "a recording is running";

#[derive(Clone)]
struct Entry {
    c: Control,
    drive: Drive,
}

/// What the running auto ran when a change that fixes it was asked ([`List::reads`]); None: not read (no ISP runs).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Now {
    pub exposure: Option<ExposureNow>,
    pub white_balance: Option<WhiteBalanceNow>,
}

/// The manual white balance, in the one form it is set by: a temperature (`white-balance` set by hand, the default), or
/// the channel gains the auto ran when it was switched off or locked (red, green, blue in 1/32) with the temperature the
/// auto estimated for them then, which `white-balance` reads. The settings file keeps it in its form: a number, or
/// `{"gains": [r, g, b], "kelvin": k}`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ManualWhiteBalance {
    Temperature(i32),
    Gains { gains: [i32; 3], kelvin: i32 },
}

impl ManualWhiteBalance {
    /// The temperature `white-balance` reads.
    pub fn kelvin(self) -> i32 {
        match self {
            Self::Temperature(k) | Self::Gains { kelvin: k, .. } => k,
        }
    }
}

/// The list and what it asks of the ISP.
#[derive(Clone)]
pub struct List {
    entries: Vec<Entry>,
    /// the kept controls the user set himself, whatever their value: the settings file holds these (a reset clears the
    /// mark); a knob whose default is a hardware state follows that state until set
    chosen: BTreeSet<String>,
    /// a recording accepted, from its `record` until it has ended (`running`): the record controls are its
    record_runs: bool,
    /// the manual white balance: the value of `white-balance`, whose control shows its temperature
    white_balance: ManualWhiteBalance,
    /// the user's values of the controls a USB host's change holds now — the one it set and what that fixed beside it —,
    /// set aside at the host's first change of each: the settings file keeps these meanwhile, and they come back when the
    /// host goes ([`Self::host_gone`]); a change of the user's makes a control his again
    users: BTreeMap<String, Value>,
    /// the user's manual white balance while a host's change holds it (`white-balance` is never in `users`)
    users_white_balance: Option<ManualWhiteBalance>,
    /// the knobs the ISP refused since its last IQ load, with why: their controls are unavailable
    refused: BTreeMap<Knob, String>,
    /// the record sizes the tables serve, by `record-size`'s values
    record_sizes: Vec<(String, Size)>,
    /// the HDMI timings the camera drives, best first (the order `values` lists the monitor's in)
    timings: Vec<Timing>,
}

fn int(name: &str, min: i64, max: i64, step: i64, default: i64, kept: bool) -> Control {
    Control { name: name.into(), kind: Kind::Int, min: Some(min), max: Some(max), step: Some(step), values: Vec::new(), default: Value::Int(default), value: Value::Int(default), kept, available: true, why: None }
}

fn switch(name: &str, default: bool, kept: bool) -> Control {
    Control { name: name.into(), kind: Kind::Bool, min: None, max: None, step: None, values: Vec::new(), default: Value::Bool(default), value: Value::Bool(default), kept, available: true, why: None }
}

fn choice(name: &str, values: &[&str], default: &str) -> Control {
    let values = values.iter().map(|v| v.to_string()).collect();
    Control { name: name.into(), kind: Kind::Choice, min: None, max: None, step: None, values, default: Value::Text(default.into()), value: Value::Text(default.into()), kept: true, available: true, why: None }
}

fn order(name: &str, values: &[&str], default: &str) -> Control {
    Control { kind: Kind::Order, ..choice(name, values, default) }
}

/// A knob's control from its range: a switch for 0/1 knobs, the mains choice for power-line, an int otherwise.
fn knob(name: &str, k: Knob) -> Entry {
    let r = k.range();
    let c = match k {
        Knob::PowerLine => choice(name, &POWER_LINE, POWER_LINE[r.default as usize]),
        _ if r.min == 0 && r.max == 1 => switch(name, r.default != 0, true),
        _ => int(name, r.min as i64, r.max as i64, r.step as i64, r.default as i64, true),
    };
    Entry { c, drive: Drive::Knob(k) }
}

/// A knob's value as its control holds it.
fn knob_value(c: &Control, v: i32) -> Value {
    match c.kind {
        Kind::Bool => Value::Bool(v != 0),
        Kind::Int => Value::Int(v as i64),
        Kind::Choice => Value::Text(c.values.get(v as usize).cloned().unwrap_or_default()),
        Kind::Order => unreachable!("no knob is an order"),
    }
}

/// A control's value as its knob takes it.
fn knob_of(c: &Control) -> i32 {
    match &c.value {
        Value::Bool(b) => *b as i32,
        Value::Int(i) => *i as i32,
        Value::Text(t) => c.values.iter().position(|v| v == t).unwrap_or(0) as i32,
    }
}

/// A record size's `record-size` value: `4k` for 2160 rows, else its rows.
fn record_size_name(s: Size) -> String {
    if s.h == 2160 {
        "4k".into()
    } else {
        s.h.to_string()
    }
}

/// The value a built control has in `p`.
fn built(name: &str, p: &Prefs) -> Value {
    let text = |s: &str| Value::Text(s.into());
    match name {
        "hdmi" if !p.hdmi_enabled => text("off"),
        "hdmi" => text(p.hdmi_override.map_or("auto", Timing::name)),
        "precedence" => Value::Text(p.precedence.to_string()),
        "preset" => text(match p.boot_preset {
            BootPreset::Uhd => "4k",
            BootPreset::Fhd => "1080",
        }),
        "level-hdmi" => Value::Int(p.levels.hdmi as i64),
        "level-usb" => Value::Int(p.levels.usb as i64),
        "level-media" => Value::Int(p.levels.media as i64),
        "record-fps" => Value::Int(p.record_fps as i64),
        "record-codec" => text(if p.record_codec == Codec::H265 { "h265" } else { "h264" }),
        "nr3d" => Value::Int(p.nr3d as i64),
        "mjpeg-quality" => Value::Int(p.mjpeg_quality as i64),
        "zoom" => Value::Int(p.view.zoom as i64),
        "mirror" => Value::Bool(p.user_mirror),
        "flip" => Value::Bool(p.user_flip),
        "aspect" => text(match p.view.aspect {
            ScreenAspect::Wide => ASPECTS[0],
            ScreenAspect::Sixteen10 => ASPECTS[1],
            ScreenAspect::Four3 => ASPECTS[2],
        }),
        "standby" => Value::Bool(p.standby),
        other => unreachable!("{other} is not a built control"),
    }
}

/// A built control's value put into the wishes `p`, or why it cannot be.
pub fn apply(name: &str, v: &Value, p: &mut Prefs) -> Result<(), String> {
    let bad = || format!("{name}: {v} does not fit");
    let int = || match v {
        Value::Int(i) => Ok(*i),
        _ => Err(bad()),
    };
    let byte = || int().and_then(|i| u8::try_from(i).map_err(|_| bad()));
    let on = || match v {
        Value::Bool(b) => Ok(*b),
        _ => Err(bad()),
    };
    let text = || match v {
        Value::Text(t) => Ok(t.as_str()),
        _ => Err(bad()),
    };
    match name {
        "hdmi" => match text()? {
            "off" => p.hdmi_enabled = false,
            "auto" => (p.hdmi_enabled, p.hdmi_override) = (true, None),
            t => (p.hdmi_enabled, p.hdmi_override) = (true, Some(Timing::parse(t).ok_or_else(bad)?)),
        },
        "precedence" => p.precedence = Precedence::parse(text()?).map_err(|e| format!("{name}: {e}"))?,
        "preset" => {
            p.boot_preset = match text()? {
                "4k" => BootPreset::Uhd,
                "1080" => BootPreset::Fhd,
                _ => return Err(bad()),
            }
        }
        "level-hdmi" | "level-usb" | "level-media" => {
            let level = byte()?;
            if level > crate::scene::MAX_LEVEL {
                return Err(bad());
            }
            *p.levels.of(&name["level-".len()..]).expect("an output's name") = level;
        }
        "record-fps" | "record-codec" if p.recording.is_some() => return Err(RECORDING.into()),
        "record-fps" => p.record_fps = u32::try_from(int()?).map_err(|_| bad())?,
        "record-codec" => p.record_codec = Codec::recordable(text()?).ok_or_else(bad)?,
        "nr3d" => p.nr3d = byte()?,
        "mjpeg-quality" => p.mjpeg_quality = byte()?,
        "zoom" => p.view.zoom = byte()?.min(MAX_ZOOM),
        "mirror" => p.user_mirror = on()?,
        "flip" => p.user_flip = on()?,
        "aspect" => {
            p.view.aspect = match text()? {
                "16:9" => ScreenAspect::Wide,
                "16:10" => ScreenAspect::Sixteen10,
                "4:3" => ScreenAspect::Four3,
                _ => return Err(bad()),
            }
        }
        "standby" => p.standby = on()?,
        _ => return Err(format!("{name} is not built by the pipeline")),
    }
    Ok(())
}

/// The value of a control as `change` makes it from `c`, without the availability check.
fn resolve(c: &Control, change: Change) -> Result<Value, Refusal> {
    let invalid = |what: String| Err(Refusal::new(Code::Invalid, format!("{}: {what}", c.name)));
    match (c.kind, change) {
        (Kind::Int, Change::Set(Value::Int(v))) => Ok(Value::Int(snap(c, v))),
        (Kind::Int, Change::Adjust(n)) => {
            let Value::Int(v) = c.value else { unreachable!("an int holds an int") };
            Ok(Value::Int(snap(c, v.saturating_add(n.saturating_mul(c.step.unwrap_or(1))))))
        }
        (Kind::Bool, Change::Set(Value::Bool(b))) => Ok(Value::Bool(b)),
        (Kind::Choice, Change::Set(v @ (Value::Text(_) | Value::Int(_)))) => {
            let t = v.to_string();
            if c.values.contains(&t) {
                Ok(Value::Text(t))
            } else {
                invalid(format!("'{t}' is not one of {}", c.values.join(", ")))
            }
        }
        (Kind::Choice, Change::Adjust(n)) => {
            let at = c.values.iter().position(|v| Value::Text(v.clone()) == c.value).unwrap_or(0) as i64;
            let to = at.saturating_add(n).clamp(0, c.values.len() as i64 - 1);
            Ok(Value::Text(c.values[to as usize].clone()))
        }
        (Kind::Order, Change::Set(Value::Text(t))) => Precedence::parse(&t).map(|p| Value::Text(p.to_string())).or_else(invalid),
        (Kind::Order, Change::Adjust(_)) => invalid("an order is set whole, not adjusted".into()),
        (Kind::Bool, Change::Adjust(_)) => invalid("a switch is set, not adjusted".into()),
        (kind, Change::Set(v)) => invalid(format!("{v} is not a{} {}", if kind == Kind::Int { "n" } else { "" }, kind_name(kind))),
    }
}

fn kind_name(k: Kind) -> &'static str {
    match k {
        Kind::Int => "int",
        Kind::Bool => "bool",
        Kind::Choice => "choice",
        Kind::Order => "order",
    }
}

/// Clamped into the range and onto its steps.
fn snap(c: &Control, v: i64) -> i64 {
    let (min, max, step) = (c.min.unwrap_or(i64::MIN), c.max.unwrap_or(i64::MAX), c.step.unwrap_or(1).max(1));
    let v = v.clamp(min, max);
    let k = (v - min + step / 2) / step;
    (min + k * step).min(max)
}

/// The knobs a control moves on the ISP: a group acts as a whole (`tuning::plan`).
fn group(drive: Drive) -> &'static [Knob] {
    const EXPOSURE: [Knob; 3] = [Knob::AutoExposure, Knob::Exposure, Knob::Gain];
    const WHITE_BALANCE: [Knob; 6] = [Knob::AutoWhiteBalance, Knob::WhiteBalance, Knob::WhiteBalanceByGains, Knob::RedGain, Knob::GreenGain, Knob::BlueGain];
    match drive {
        Drive::Knob(Knob::AutoExposure | Knob::Exposure | Knob::Gain) | Drive::ExposureLock => &EXPOSURE,
        Drive::Knob(Knob::AutoWhiteBalance | Knob::WhiteBalance) | Drive::WhiteBalanceLock => &WHITE_BALANCE,
        Drive::Knob(Knob::Brightness) => &[Knob::Brightness],
        Drive::Knob(Knob::Contrast) => &[Knob::Contrast],
        Drive::Knob(Knob::Saturation) => &[Knob::Saturation],
        Drive::Knob(Knob::Sharpness) => &[Knob::Sharpness],
        Drive::Knob(Knob::Gamma) => &[Knob::Gamma],
        Drive::Knob(Knob::Backlight) => &[Knob::Backlight],
        Drive::Knob(Knob::Grey) => &[Knob::Grey],
        Drive::Knob(Knob::PowerLine) => &[Knob::PowerLine],
        Drive::Knob(Knob::ExposureBias) => &[Knob::ExposureBias],
        _ => &[],
    }
}

/// The settings file: the kept controls the user set, and the menu's own record.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Saved {
    #[serde(default)]
    pub controls: BTreeMap<String, serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub record: Option<serde_json::Value>,
}

impl List {
    /// The list with its defaults: a built control's as the wishes `p` have it (`Prefs::default`'s, the mounting beside
    /// them from `config.toml`), a knob's its range's (backlight's the IQ file's once read, [`Self::hardware`]); the
    /// record sizes and HDMI timings `policy`'s tables serve.
    pub fn new(p: &Prefs, policy: &Policy) -> Self {
        let b = |c: Control| Entry { c, drive: Drive::Built };
        let num = |name: &str, min: i64, max: i64| match built(name, p) {
            Value::Int(v) => b(int(name, min, max, 1, v, true)),
            v => unreachable!("{name} is a number, not {v}"),
        };
        let on = |name: &str, kept: bool| b(switch(name, built(name, p) == Value::Bool(true), kept));
        let pick = |name: &str, values: &[&str]| b(choice(name, values, &built(name, p).to_string()));
        let record_sizes: Vec<(String, Size)> = policy.record_sizes().into_iter().map(|s| (record_size_name(s), s)).collect();
        let size_names: Vec<&str> = record_sizes.iter().map(|(n, _)| n.as_str()).collect();
        let max_level = crate::scene::MAX_LEVEL as i64;
        let mut entries = vec![
            knob("brightness", Knob::Brightness),
            knob("contrast", Knob::Contrast),
            knob("saturation", Knob::Saturation),
            knob("sharpness", Knob::Sharpness),
            knob("gamma", Knob::Gamma),
            knob("backlight", Knob::Backlight),
            knob("grey", Knob::Grey),
            knob("power-line", Knob::PowerLine),
            knob("auto-exposure", Knob::AutoExposure),
            knob("exposure", Knob::Exposure),
            knob("gain", Knob::Gain),
            knob("exposure-bias", Knob::ExposureBias),
            Entry { c: switch("exposure-lock", false, false), drive: Drive::ExposureLock },
            knob("auto-white-balance", Knob::AutoWhiteBalance),
            knob("white-balance", Knob::WhiteBalance),
            Entry { c: switch("white-balance-lock", false, false), drive: Drive::WhiteBalanceLock },
            num("zoom", 0, MAX_ZOOM as i64),
            on("mirror", true),
            on("flip", true),
            pick("aspect", &ASPECTS),
            pick("hdmi", &["auto", "off"]),
            b(order("precedence", &Who::ALL.map(Who::name), &built("precedence", p).to_string())),
            pick("preset", &["4k", "1080"]),
            num("level-hdmi", 0, max_level),
            num("level-usb", 0, max_level),
            num("level-media", 0, max_level),
            Entry { c: choice("record-size", &size_names, size_names.first().expect("policy.toml lists a record size")), drive: Drive::Stored },
            num("record-fps", 0, MAX_RECORD_FPS),
            pick("record-codec", &["h264", "h265"]),
            num("nr3d", 0, MAX_NR3D),
            num("mjpeg-quality", 1, 99),
            on("standby", false),
        ];
        let timings = policy.hdmi_timings().to_vec();
        for e in entries.iter_mut().filter(|e| e.c.name == "hdmi") {
            e.c.values = hdmi_values(&timings, &Monitor::Absent, &e.c.value);
        }
        let mut l = Self { entries, chosen: BTreeSet::new(), record_runs: false, white_balance: ManualWhiteBalance::Temperature(Knob::WhiteBalance.range().default), users: BTreeMap::new(), users_white_balance: None, refused: BTreeMap::new(), record_sizes, timings };
        l.refresh();
        l
    }

    fn entry(&self, name: &str) -> Option<&Entry> {
        self.entries.iter().find(|e| e.c.name == name)
    }

    fn entry_mut(&mut self, name: &str) -> &mut Entry {
        self.entries.iter_mut().find(|e| e.c.name == name).expect("a control of the list")
    }

    fn on(&self, name: &str) -> bool {
        self.entry(name).is_some_and(|e| e.c.value == Value::Bool(true))
    }

    pub fn get(&self, name: &str) -> Option<&Control> {
        self.entry(name).map(|e| &e.c)
    }

    pub fn all(&self) -> Vec<Control> {
        self.entries.iter().map(|e| e.c.clone()).collect()
    }

    pub fn drive(&self, name: &str) -> Option<Drive> {
        self.entry(name).map(|e| e.drive)
    }

    /// The value `change` asks of `name` now: refused when unknown, invalid or unavailable.
    pub fn target(&self, name: &str, change: Change) -> Result<Value, Refusal> {
        let e = self.entry(name).ok_or_else(|| Refusal::new(Code::Unknown, format!("no control '{name}'")))?;
        if !e.c.available {
            return Err(Refusal::new(Code::Unavailable, format!("{name}: {}", e.c.why.as_deref().unwrap_or("not now"))));
        }
        resolve(&e.c, change)
    }

    /// The next recording's size.
    pub fn record_size(&self) -> Size {
        self.get("record-size").and_then(|c| self.record_size_of(&c.value)).expect("record-size is one of the tables' sizes")
    }

    /// A `record-size` value's size.
    pub fn record_size_of(&self, v: &Value) -> Option<Size> {
        self.record_sizes.iter().find(|(n, _)| Value::Text(n.clone()) == *v).map(|(_, s)| *s)
    }

    /// The names of the entries `f` changed.
    fn edit(&mut self, f: impl FnOnce(&mut Self)) -> Vec<String> {
        let before = self.all();
        f(self);
        self.refresh();
        self.entries.iter().zip(before).filter(|(e, b)| e.c != *b).map(|(e, _)| e.c.name.clone()).collect()
    }

    /// What setting `name` to `v` needs read from the running auto first: the exposure, the white balance. Switching
    /// an auto off or locking it fixes what it runs then.
    pub fn reads(&self, name: &str, v: &Value) -> (bool, bool) {
        let on = *v == Value::Bool(true);
        match name {
            "auto-exposure" => (!on && self.on(name), false),
            "exposure-lock" => (on, false),
            "auto-white-balance" => (false, !on && self.on(name)),
            "white-balance-lock" => (false, on),
            _ => (false, false),
        }
    }

    /// A value from [`Self::target`] stored for a control that is not built (a knob, a lock, a kept value), asked `by`
    /// the user or a USB host, with what the auto ran when [`Self::reads`] asked for it; the names of the entries it
    /// changed. The auto switched off fixes `exposure` and `gain`, or the manual white balance by the gains it ran, at
    /// what it ran; a lock fixes them while it is on. The user's: a kept control, and what switching the auto off fixed,
    /// are his from now on, a host's hold on them gone. A host's: the user's values of what it changed are set aside
    /// until it goes, nothing is marked his.
    pub fn store(&mut self, name: &str, v: Value, by: By, now: &Now) -> Vec<String> {
        let before: Vec<Value> = self.entries.iter().map(|e| e.c.value.clone()).collect();
        let white_balance = self.white_balance;
        let told = self.store_value(name, v, by == By::User, now);
        let changed: Vec<String> = self.entries.iter().zip(&before).filter(|(e, b)| e.c.value != **b).map(|(e, _)| e.c.name.clone()).collect();
        let white_balance_changed = self.white_balance != white_balance || name == "white-balance";
        match by {
            By::Host => {
                for (e, b) in self.entries.iter().zip(before) {
                    if e.c.value != b && e.c.name != "white-balance" && !self.users.contains_key(&e.c.name) {
                        self.users.insert(e.c.name.clone(), b);
                    }
                }
                if self.white_balance != white_balance && self.users_white_balance.is_none() {
                    self.users_white_balance = Some(white_balance);
                }
            }
            By::User => {
                self.users.remove(name);
                for n in &changed {
                    self.users.remove(n);
                }
                if white_balance_changed {
                    self.users_white_balance = None;
                }
            }
        }
        told
    }

    /// [`Self::store`]'s change of the list; `user`: kept controls and what the auto's going off fixed become his.
    fn store_value(&mut self, name: &str, v: Value, user: bool, now: &Now) -> Vec<String> {
        self.edit(|l| {
            let e = l.entry_mut(name);
            assert!(e.drive != Drive::Built, "{name} is built by the pipeline");
            let kept = e.c.kept;
            let was = std::mem::replace(&mut e.c.value, v.clone());
            if kept && user {
                l.chosen.insert(name.to_string());
            }
            // a temperature set by hand: the manual white balance is by it, though it reads the temperature a hold by
            // gains shows
            if name == "white-balance" {
                l.white_balance = ManualWhiteBalance::Temperature(knob_of(&l.entry_mut(name).c));
            }
            if was == v {
                return;
            }
            let on = v == Value::Bool(true);
            match name {
                "auto-exposure" => {
                    l.entry_mut("exposure-lock").c.value = Value::Bool(false);
                    if let Some(e) = now.exposure.filter(|_| !on) {
                        l.fix_exposure(e, user);
                    }
                }
                "exposure-lock" => {
                    if let Some(e) = now.exposure.filter(|_| on) {
                        l.fix_exposure(e, false);
                    }
                }
                "auto-white-balance" => {
                    l.entry_mut("white-balance-lock").c.value = Value::Bool(false);
                    if let Some(w) = now.white_balance.filter(|_| !on) {
                        l.fix_white_balance(w, user);
                    }
                }
                "white-balance-lock" => {
                    if let Some(w) = now.white_balance.filter(|_| on) {
                        l.fix_white_balance(w, false);
                    }
                }
                _ => {}
            }
        })
    }

    /// `exposure` and `gain` at what the auto ran; the user's when `chosen`.
    fn fix_exposure(&mut self, e: ExposureNow, chosen: bool) {
        self.entry_mut("exposure").c.value = Value::Int(Knob::Exposure.clamp(e.time_100us) as i64);
        self.entry_mut("gain").c.value = Value::Int(Knob::Gain.clamp(e.gain_pct) as i64);
        if chosen {
            self.chosen.extend(["exposure".to_string(), "gain".to_string()]);
        }
    }

    /// The manual white balance by the gains the auto ran, with its temperature then; the user's when `chosen`.
    fn fix_white_balance(&mut self, w: WhiteBalanceNow, chosen: bool) {
        let kelvin = snap(&self.entry_mut("white-balance").c, w.kelvin as i64) as i32;
        self.set_white_balance(ManualWhiteBalance::Gains { gains: [w.gains.0, w.gains.1, w.gains.2], kelvin });
        if chosen {
            self.chosen.insert("white-balance".to_string());
        }
    }

    /// The manual white balance set, `white-balance` showing its temperature.
    fn set_white_balance(&mut self, m: ManualWhiteBalance) {
        self.white_balance = m;
        self.entry_mut("white-balance").c.value = Value::Int(m.kelvin() as i64);
    }

    /// A built control the pipeline built at the user's `set`: his from now on, when it is kept.
    pub fn chosen(&mut self, name: &str) {
        if self.entry(name).is_some_and(|e| e.c.kept) {
            self.chosen.insert(name.to_string());
        }
    }

    /// The controls that are not the picture's — the kept values and the built ones — back to their defaults and no
    /// longer the user's, but the recording's while one runs (they are its): the names of the entries changed here, and
    /// the built ones to build, each with its default. The picture's go back once the ISP took them
    /// ([`Self::reset_picture`]).
    pub fn reset(&mut self) -> (Vec<String>, Vec<(String, Value)>) {
        let recording = self.record_runs;
        let mut build = Vec::new();
        let told = self.edit(|l| {
            let Self { entries, chosen, .. } = l;
            for e in entries.iter_mut().filter(|e| matches!(e.drive, Drive::Built | Drive::Stored) && !(recording && e.c.name.starts_with("record-"))) {
                chosen.remove(&e.c.name);
                if e.c.value == e.c.default {
                    continue;
                }
                match e.drive {
                    Drive::Built => build.push((e.c.name.clone(), e.c.default.clone())),
                    _ => e.c.value = e.c.default.clone(),
                }
            }
        });
        (told, build)
    }

    /// The picture's controls — the knobs and the locks — back to their defaults, the locks off, the white balance by
    /// its default temperature, no longer the user's nor a USB host's: the names of the entries changed.
    pub fn reset_picture(&mut self) -> Vec<String> {
        self.edit(|l| {
            l.white_balance = ManualWhiteBalance::Temperature(Knob::WhiteBalance.range().default);
            l.users.clear();
            l.users_white_balance = None;
            let Self { entries, chosen, .. } = l;
            for e in entries.iter_mut().filter(|e| matches!(e.drive, Drive::Knob(_) | Drive::ExposureLock | Drive::WhiteBalanceLock)) {
                chosen.remove(&e.c.name);
                e.c.value = e.c.default.clone();
            }
        })
    }

    /// Whether a USB host's change holds any control now.
    pub fn host_holds(&self) -> bool {
        !self.users.is_empty() || self.users_white_balance.is_some()
    }

    /// The USB host gone: the user's values of what its changes held back; the names of the entries changed.
    pub fn host_gone(&mut self) -> Vec<String> {
        self.edit(|l| {
            for (name, v) in std::mem::take(&mut l.users) {
                l.entry_mut(&name).c.value = v;
            }
            if let Some(m) = l.users_white_balance.take() {
                l.set_white_balance(m);
            }
        })
    }

    /// Every knob's value for the ISP, the locks and the manual white balance's form folded in; a backlight the user
    /// has not set as the IQ file has it ([`AS_TUNED`]).
    pub fn tuning(&self) -> Tuning {
        let ae = self.on("auto-exposure") && !self.on("exposure-lock");
        let awb = self.on("auto-white-balance") && !self.on("white-balance-lock");
        let mut t = Tuning::new();
        for e in &self.entries {
            if let Drive::Knob(k) = e.drive {
                t.insert(k, knob_of(&e.c));
            }
        }
        if !self.chosen.contains("backlight") && !self.users.contains_key("backlight") {
            t.insert(Knob::Backlight, AS_TUNED);
        }
        t.insert(Knob::AutoExposure, ae as i32);
        t.insert(Knob::AutoWhiteBalance, awb as i32);
        let [r, g, b] = match self.white_balance {
            ManualWhiteBalance::Temperature(_) => [GAIN_UNIT; 3],
            ManualWhiteBalance::Gains { gains, .. } => gains,
        };
        t.insert(Knob::WhiteBalanceByGains, matches!(self.white_balance, ManualWhiteBalance::Gains { .. }) as i32);
        t.insert(Knob::RedGain, r);
        t.insert(Knob::GreenGain, g);
        t.insert(Knob::BlueGain, b);
        t
    }

    /// Whether `name`'s value now is the running auto's rather than the list's: `exposure` and `gain` while the auto
    /// exposure runs, `white-balance` while the auto white balance does (a USB host reads them live).
    pub fn live(&self, name: &str) -> bool {
        match name {
            "exposure" | "gain" => self.on("auto-exposure") && !self.on("exposure-lock"),
            "white-balance" => self.on("auto-white-balance") && !self.on("white-balance-lock"),
            _ => false,
        }
    }

    /// The defaults of the knobs whose default is a hardware state (backlight), read after an IQ load; one the user has
    /// not set follows its default.
    pub fn hardware(&mut self, values: &[(Knob, i32)]) -> Vec<String> {
        self.edit(|l| {
            let Self { entries, chosen, users, .. } = l;
            for &(k, v) in values {
                if let Some(e) = entries.iter_mut().find(|e| e.drive == Drive::Knob(k)) {
                    e.c.default = knob_value(&e.c, v);
                    if chosen.contains(&e.c.name) {
                        continue;
                    }
                    // held by a USB host: the user's value aside follows
                    match users.get_mut(&e.c.name) {
                        Some(u) => *u = e.c.default.clone(),
                        None => e.c.value = e.c.default.clone(),
                    }
                }
            }
        })
    }

    /// The knobs the ISP refused since its last IQ load, with why: their controls unavailable until a load takes them.
    pub fn refused(&mut self, knobs: BTreeMap<Knob, String>) -> Vec<String> {
        self.edit(|l| l.refused = knobs)
    }

    /// The built controls as the wishes `p` are built, the HDMI timings the monitor offers.
    pub fn follow(&mut self, p: &Prefs, monitor: &Monitor) -> Vec<String> {
        self.edit(|l| {
            for e in l.entries.iter_mut().filter(|e| e.drive == Drive::Built) {
                e.c.value = built(&e.c.name, p);
                if e.c.name == "hdmi" {
                    e.c.values = hdmi_values(&l.timings, monitor, &e.c.value);
                }
            }
        })
    }

    /// A recording accepted (`on`) or ended: the record controls unavailable from its acceptance, before it is built,
    /// until it has ended — after its stop is built, while its file is closed.
    pub fn record_runs(&mut self, on: bool) -> Vec<String> {
        self.edit(|l| l.record_runs = on)
    }

    /// Availability from the values, the recording and the knobs the ISP refused.
    fn refresh(&mut self) {
        let ae = self.on("auto-exposure");
        let awb = self.on("auto-white-balance");
        let recording = self.record_runs;
        for e in &mut self.entries {
            let refused = group(e.drive).iter().find_map(|k| self.refused.get(k));
            let why = match e.c.name.as_str() {
                _ if refused.is_some() => refused.map(|r| format!("the ISP refused it: {r}")),
                "exposure" | "gain" if ae => Some("auto-exposure is on".into()),
                "exposure-bias" | "exposure-lock" if !ae => Some("auto-exposure is off".into()),
                "white-balance" if awb => Some("auto-white-balance is on".into()),
                "white-balance-lock" if !awb => Some("auto-white-balance is off".into()),
                "record-size" | "record-fps" | "record-codec" if recording => Some(RECORDING.into()),
                _ => None,
            };
            e.c.available = why.is_none();
            e.c.why = why;
        }
    }

    /// The settings file's controls: the kept ones the user set, at his values where a USB host holds them; the manual
    /// white balance in its form.
    pub fn saved(&self) -> BTreeMap<String, serde_json::Value> {
        let kept = self.entries.iter().filter(|e| e.c.kept && self.chosen.contains(&e.c.name));
        let json = |e: &Entry| match e.c.name.as_str() {
            "white-balance" => serde_json::to_value(self.users_white_balance.unwrap_or(self.white_balance)),
            name => serde_json::to_value(self.users.get(name).unwrap_or(&e.c.value)),
        };
        kept.map(|e| (e.c.name.clone(), json(e).expect("a value is JSON"))).collect()
    }

    /// The settings file's controls taken as the values at start, the user's; what does not fit stays at its default
    /// and is said in the lines returned. No control is treated as built here: [`Self::into_prefs`] puts the built
    /// ones into the wishes.
    pub fn load(&mut self, saved: &BTreeMap<String, serde_json::Value>) -> Vec<String> {
        let mut said = Vec::new();
        for (name, v) in saved {
            let Some(e) = self.entries.iter_mut().find(|e| &e.c.name == name) else {
                said.push(format!("settings: no control '{name}', left out"));
                continue;
            };
            if !e.c.kept {
                said.push(format!("settings: {name} is not kept, left out"));
                continue;
            }
            if name == "white-balance" {
                match white_balance_of(&e.c, v) {
                    Ok(m) => {
                        self.set_white_balance(m);
                        self.chosen.insert(name.clone());
                    }
                    Err(why) => said.push(format!("settings: {name} = {v}: {why}; the default kept")),
                }
                continue;
            }
            let keepable = keepable(&e.c, &self.timings);
            let value = serde_json::from_value::<Value>(v.clone()).map_err(|e| Refusal::new(Code::Invalid, e)).and_then(|v| resolve(&keepable, Change::Set(v.clone())).and_then(|r| if r == v { Ok(r) } else { Err(Refusal::new(Code::Invalid, format!("{v} is outside {name}'s range or steps"))) }));
            match value {
                Ok(v) => {
                    // kept but not offered now (a timing the monitor at hand does not list): offered beside the rest
                    if let Value::Text(t) = &v {
                        if !e.c.values.contains(t) {
                            e.c.values.push(t.clone());
                        }
                    }
                    e.c.value = v;
                    self.chosen.insert(name.clone());
                }
                Err(r) => said.push(format!("settings: {name} = {v}: {}; the default kept", r.error)),
            }
        }
        self.refresh();
        said
    }

    /// The built controls' values put into the wishes `p`; a value the wishes cannot take goes back to its default, said
    /// in the lines returned.
    pub fn into_prefs(&mut self, p: &mut Prefs) -> Vec<String> {
        let mut said = Vec::new();
        for e in self.entries.iter_mut().filter(|e| e.drive == Drive::Built) {
            if let Err(why) = apply(&e.c.name, &e.c.value, p) {
                said.push(format!("settings: {why}; the default kept"));
                e.c.value = e.c.default.clone();
                self.chosen.remove(&e.c.name);
            }
        }
        said
    }
}

/// The manual white balance the settings file keeps for `white-balance` (`c`): a temperature on its range and steps, or
/// gains on theirs with such a temperature.
fn white_balance_of(c: &Control, v: &serde_json::Value) -> Result<ManualWhiteBalance, String> {
    let m: ManualWhiteBalance = serde_json::from_value(v.clone()).map_err(|e| e.to_string())?;
    let gains_fit = match m {
        ManualWhiteBalance::Gains { gains, .. } => gains.iter().all(|g| Knob::RedGain.clamp(*g) == *g),
        ManualWhiteBalance::Temperature(_) => true,
    };
    let fits = gains_fit && snap(c, m.kelvin() as i64) == m.kelvin() as i64;
    if fits {
        Ok(m)
    } else {
        Err("outside its range or steps".into())
    }
}

/// A live control's value from what the auto runs `now` ([`List::live`]), on its range; None: not read.
pub fn live_value(name: &str, now: &Now) -> Option<Value> {
    let v = match name {
        "exposure" => Knob::Exposure.clamp(now.exposure?.time_100us),
        "gain" => Knob::Gain.clamp(now.exposure?.gain_pct),
        "white-balance" => Knob::WhiteBalance.clamp(now.white_balance?.kelvin),
        _ => return None,
    };
    Some(Value::Int(v as i64))
}

/// `c` with every value it may be kept at, which is not what it offers now for `hdmi`: any timing the camera drives,
/// whether the monitor at hand lists it or not.
fn keepable(c: &Control, timings: &[Timing]) -> Control {
    let mut c = c.clone();
    if c.name == "hdmi" {
        c.values.extend(timings.iter().map(|t| t.name().to_string()));
    }
    c
}

/// `auto`, `off`, the monitor's timings among `timings`, and the value when it is a timing the monitor does not list.
fn hdmi_values(timings: &[Timing], monitor: &Monitor, value: &Value) -> Vec<String> {
    let mut v = vec!["auto".to_string(), "off".to_string()];
    if let Monitor::Present(caps) = monitor {
        v.extend(timings.iter().filter(|t| caps.supports(**t)).map(|t| t.name().to_string()));
    }
    let now = value.to_string();
    if !v.contains(&now) {
        v.push(now);
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hdmi::MonitorCaps;
    use crate::policy::tests::policy;
    use crate::policy::Recording;

    fn list() -> List {
        List::new(&Prefs::default(), &policy())
    }

    fn set(l: &mut List, name: &str, v: Value) -> Result<Vec<String>, Refusal> {
        read(l, name, v, Now::default())
    }

    fn read(l: &mut List, name: &str, v: Value, now: Now) -> Result<Vec<String>, Refusal> {
        let v = l.target(name, Change::Set(v))?;
        Ok(l.store(name, v, By::User, &now))
    }

    fn value(l: &List, name: &str) -> Value {
        l.get(name).unwrap().value.clone()
    }

    #[test]
    fn adjust_steps_clamps_and_walks_choices() {
        let mut l = list();
        assert_eq!(l.target("brightness", Change::Adjust(3)), Ok(Value::Int(53)));
        assert_eq!(l.target("brightness", Change::Adjust(-80)), Ok(Value::Int(0)));
        assert_eq!(l.target("exposure-bias", Change::Adjust(2)), Ok(Value::Int(6)), "exposure-bias steps by 3");
        assert_eq!(l.target("exposure-bias", Change::Adjust(20)), Ok(Value::Int(30)));
        assert_eq!(l.target("exposure-bias", Change::Set(Value::Int(4))), Ok(Value::Int(3)), "onto its steps");
        assert_eq!(l.target("saturation", Change::Set(Value::Int(150))), Ok(Value::Int(100)), "saturation is 0–100");
        assert_eq!(l.get("sharpness").unwrap().default, Value::Int(50));
        assert_eq!(l.target("power-line", Change::Adjust(1)), Ok(Value::Text("60".into())));
        assert_eq!(l.target("power-line", Change::Adjust(-5)), Ok(Value::Text("off".into())));
        assert_eq!(l.target("power-line", Change::Set(Value::Int(60))), Ok(Value::Text("60".into())), "a number names a choice");
        assert_eq!(l.target("record-size", Change::Adjust(1)), Ok(Value::Text("1440".into())));
        assert_eq!(l.target("aspect", Change::Set(Value::Text("21:9".into()))).unwrap_err().code, Code::Invalid);
        assert_eq!(l.target("grey", Change::Adjust(1)).unwrap_err().code, Code::Invalid);
        assert_eq!(l.target("grey", Change::Set(Value::Int(1))).unwrap_err().code, Code::Invalid);
        assert_eq!(l.target("hue", Change::Adjust(1)).unwrap_err().code, Code::Unknown);
        set(&mut l, "brightness", Value::Int(70)).unwrap();
        assert_eq!(l.target("brightness", Change::Adjust(-1)), Ok(Value::Int(69)), "from the value stored");
    }

    #[test]
    fn availability_follows_the_autos_and_the_recording() {
        let mut l = list();
        assert_eq!(l.target("exposure", Change::Set(Value::Int(50))).unwrap_err().code, Code::Unavailable);
        assert!(!l.get("white-balance").unwrap().available);
        assert!(l.get("exposure-lock").unwrap().available);
        let changed = set(&mut l, "auto-exposure", Value::Bool(false)).unwrap();
        for n in ["auto-exposure", "exposure", "gain", "exposure-bias", "exposure-lock"] {
            assert!(changed.contains(&n.to_string()), "{n} told");
        }
        assert!(l.get("exposure").unwrap().available && !l.get("exposure-lock").unwrap().available);
        assert_eq!(l.get("exposure-bias").unwrap().why.as_deref(), Some("auto-exposure is off"));
        // accepted, not built yet: the record controls are its already, until it has ended
        assert_eq!(l.record_runs(true), ["record-size", "record-fps", "record-codec"]);
        assert_eq!(l.target("record-codec", Change::Set(Value::Text("h265".into()))).unwrap_err().code, Code::Unavailable);
        assert!(l.follow(&Prefs::default(), &Monitor::Absent).is_empty(), "the wishes without it yet change nothing");
        let p = Prefs { recording: Some(Recording { size: Size::new(3840, 2160), path: String::new() }), ..Prefs::default() };
        assert!(l.follow(&p, &Monitor::Absent).is_empty(), "built: nothing told twice");
        assert_eq!(l.target("record-size", Change::Set(Value::Text("1080".into()))).unwrap_err().code, Code::Unavailable);
        // ended while the wishes still have it (the pipeline follows after): free at once
        assert_eq!(l.record_runs(false).len(), 3);
        assert!(l.target("record-codec", Change::Set(Value::Text("h265".into()))).is_ok());
    }

    #[test]
    fn the_exposure_lock_fixes_what_the_auto_runs() {
        let mut l = list();
        let ran = Now { exposure: Some(ExposureNow { time_100us: 120, gain_pct: 450 }), white_balance: None };
        assert_eq!(l.reads("exposure-lock", &Value::Bool(true)), (true, false));
        let changed = read(&mut l, "exposure-lock", Value::Bool(true), ran).unwrap();
        assert_eq!(changed, ["exposure", "gain", "exposure-lock"]);
        assert_eq!((value(&l, "exposure"), value(&l, "gain")), (Value::Int(120), Value::Int(450)));
        let t = l.tuning();
        assert_eq!((t[&Knob::AutoExposure], t[&Knob::Exposure], t[&Knob::Gain]), (0, 120, 450));
        assert!(l.saved().is_empty(), "a lock's values are not kept");
        set(&mut l, "exposure-lock", Value::Bool(false)).unwrap();
        assert_eq!(l.tuning()[&Knob::AutoExposure], 1, "the auto back once unlocked");
        // switched off it fixes the exposure too — the user's then — and a lock left on goes
        set(&mut l, "exposure-lock", Value::Bool(true)).unwrap();
        assert_eq!(l.reads("auto-exposure", &Value::Bool(false)), (true, false));
        let ran = Now { exposure: Some(ExposureNow { time_100us: 80, gain_pct: 200 }), white_balance: None };
        read(&mut l, "auto-exposure", Value::Bool(false), ran).unwrap();
        assert_eq!((value(&l, "exposure"), value(&l, "exposure-lock")), (Value::Int(80), Value::Bool(false)));
        assert_eq!(l.saved().keys().collect::<Vec<_>>(), ["auto-exposure", "exposure", "gain"]);
        assert_eq!(l.reads("auto-exposure", &Value::Bool(false)), (false, false), "off already: nothing to read");
        set(&mut l, "exposure", Value::Int(40)).unwrap();
        assert_eq!(l.tuning()[&Knob::Exposure], 40);
        // switched off with no ISP to read: the stored values
        let mut l = list();
        set(&mut l, "auto-exposure", Value::Bool(false)).unwrap();
        assert_eq!(l.tuning()[&Knob::Exposure], Knob::Exposure.range().default);
    }

    #[test]
    fn the_white_balance_lock_and_the_auto_off_hold_the_colour() {
        let mut l = list();
        let ran = Now { exposure: None, white_balance: Some(WhiteBalanceNow { gains: (52, 32, 40), kelvin: 4312 }) };
        assert_eq!(l.reads("white-balance-lock", &Value::Bool(true)), (false, true));
        read(&mut l, "white-balance-lock", Value::Bool(true), ran).unwrap();
        let t = l.tuning();
        assert_eq!((t[&Knob::AutoWhiteBalance], t[&Knob::WhiteBalanceByGains], t[&Knob::RedGain], t[&Knob::BlueGain]), (0, 1, 52, 40));
        assert_eq!(value(&l, "white-balance"), Value::Int(4310), "its temperature, onto the steps");
        set(&mut l, "white-balance-lock", Value::Bool(false)).unwrap();
        assert_eq!(l.tuning()[&Knob::AutoWhiteBalance], 1, "the auto back once unlocked");
        assert!(l.saved().is_empty(), "a lock's white balance is not kept");
        // switched off it keeps the colour the auto ran, by its gains, until a temperature is set
        assert_eq!(l.reads("auto-white-balance", &Value::Bool(false)), (false, true));
        read(&mut l, "auto-white-balance", Value::Bool(false), ran).unwrap();
        let t = l.tuning();
        assert_eq!((t[&Knob::AutoWhiteBalance], t[&Knob::WhiteBalanceByGains], t[&Knob::RedGain]), (0, 1, 52));
        assert!(l.get("white-balance").unwrap().available);
        assert_eq!(l.saved().keys().collect::<Vec<_>>(), ["auto-white-balance", "white-balance"]);
        assert_eq!(l.saved()["white-balance"], serde_json::json!({"gains": [52, 32, 40], "kelvin": 4310}), "kept in its form");
        // the temperature it shows, set: by that temperature from now on
        set(&mut l, "white-balance", Value::Int(4310)).unwrap();
        assert_eq!((l.tuning()[&Knob::WhiteBalanceByGains], l.saved()["white-balance"].clone()), (0, serde_json::json!(4310)));
        set(&mut l, "white-balance", Value::Int(5600)).unwrap();
        assert_eq!(l.saved()["white-balance"], serde_json::json!(5600));
        let t = l.tuning();
        assert_eq!((t[&Knob::WhiteBalanceByGains], t[&Knob::WhiteBalance]), (0, 5600), "manual by temperature");
        // switched off with no ISP to read: by the temperature it holds
        let mut l = list();
        set(&mut l, "auto-white-balance", Value::Bool(false)).unwrap();
        assert_eq!((l.tuning()[&Knob::WhiteBalanceByGains], value(&l, "white-balance-lock")), (0, Value::Bool(false)));
    }

    /// A white balance held by the auto's gains comes back from the settings file as it was: the same file, byte for
    /// byte, and the same gains to the ISP; a temperature, and an old file's number, come back as a temperature.
    #[test]
    fn a_held_white_balance_survives_the_file() {
        let mut l = list();
        let ran = Now { exposure: None, white_balance: Some(WhiteBalanceNow { gains: (65, 32, 55), kelvin: 5000 }) };
        read(&mut l, "auto-white-balance", Value::Bool(false), ran).unwrap();
        let text = serde_json::to_string_pretty(&Saved { controls: l.saved(), record: None }).unwrap();
        let mut back = list();
        assert!(back.load(&serde_json::from_str::<Saved>(&text).unwrap().controls).is_empty());
        assert_eq!(serde_json::to_string_pretty(&Saved { controls: back.saved(), record: None }).unwrap(), text, "byte for byte");
        assert_eq!(back.tuning(), l.tuning());
        let t = back.tuning();
        assert_eq!((t[&Knob::AutoWhiteBalance], t[&Knob::WhiteBalanceByGains], t[&Knob::RedGain], t[&Knob::BlueGain]), (0, 1, 65, 55));
        assert_eq!(value(&back, "white-balance"), Value::Int(5000), "the control reads its temperature");
        let mut old = list();
        assert!(old.load(&[("white-balance".to_string(), serde_json::json!(4200))].into()).is_empty());
        assert_eq!((old.tuning()[&Knob::WhiteBalanceByGains], old.tuning()[&Knob::WhiteBalance]), (0, 4200));
        let bad = [("white-balance".to_string(), serde_json::json!({"gains": [999, 32, 32], "kelvin": 5000}))].into();
        assert_eq!(list().load(&bad).len(), 1, "gains off their range: the default kept");
    }

    /// The live values are the auto's while it runs and not locked.
    #[test]
    fn the_autos_values_are_live_while_they_run() {
        let mut l = list();
        assert!(l.live("exposure") && l.live("gain") && l.live("white-balance") && !l.live("brightness"));
        set(&mut l, "exposure-lock", Value::Bool(true)).unwrap();
        set(&mut l, "auto-white-balance", Value::Bool(false)).unwrap();
        assert!(!l.live("exposure") && !l.live("white-balance"));
    }

    #[test]
    fn built_controls_are_the_wishes_and_hdmi_lists_the_monitor() {
        let mut l = list();
        assert_eq!(l.drive("zoom"), Some(Drive::Built));
        let mut p = Prefs::default();
        apply("zoom", &Value::Int(12), &mut p).unwrap();
        apply("hdmi", &Value::Text("1080p60".into()), &mut p).unwrap();
        apply("level-usb", &Value::Int(1), &mut p).unwrap();
        apply("aspect", &Value::Text("4:3".into()), &mut p).unwrap();
        assert!(apply("level-usb", &Value::Int(9), &mut p).is_err());
        assert!(apply("brightness", &Value::Int(9), &mut p).is_err());
        let caps = MonitorCaps { modes: vec![(Size::new(3840, 2160), 30), (Size::new(1920, 1080), 60)] };
        let changed = l.follow(&p, &Monitor::Present(caps));
        assert_eq!(changed, ["zoom", "aspect", "hdmi", "level-usb"]);
        assert_eq!(l.get("hdmi").unwrap().values, ["auto", "off", "2160p30", "1080p60"]);
        assert_eq!(p.levels.usb, 1);
        for name in ["zoom", "hdmi", "level-usb", "aspect", "precedence", "preset", "record-fps", "record-codec", "nr3d", "mjpeg-quality", "mirror", "flip", "standby"] {
            let mut q = Prefs::default();
            apply(name, &value(&l, name), &mut q).unwrap();
            assert_eq!(built(name, &q), built(name, &p), "{name} round trips");
        }
        // a fixed timing the monitor gone no longer lists stays a value
        l.follow(&p, &Monitor::Absent);
        assert_eq!(l.get("hdmi").unwrap().values, ["auto", "off", "1080p60"]);
    }

    #[test]
    fn settings_round_trip_and_fall_back() {
        let mut l = list();
        set(&mut l, "brightness", Value::Int(70)).unwrap();
        set(&mut l, "power-line", Value::Text("60".into())).unwrap();
        set(&mut l, "record-size", Value::Text("1080".into())).unwrap();
        set(&mut l, "exposure-lock", Value::Bool(true)).unwrap();
        let mut p = Prefs::default();
        apply("zoom", &Value::Int(5), &mut p).unwrap();
        l.follow(&p, &Monitor::Absent);
        assert!(!l.saved().contains_key("zoom"), "built, but not at the user's set");
        l.chosen("zoom");
        let saved = l.saved();
        assert_eq!(saved.keys().collect::<Vec<_>>(), ["brightness", "power-line", "record-size", "zoom"], "the lock is not kept");
        let text = serde_json::to_string(&Saved { controls: saved, record: Some(serde_json::json!({"page": 2})) }).unwrap();
        let back: Saved = serde_json::from_str(&text).unwrap();
        assert_eq!(back.record, Some(serde_json::json!({"page": 2})));
        let mut fresh = list();
        let mut controls = back.controls;
        controls.insert("hue".into(), serde_json::json!(3));
        controls.insert("contrast".into(), serde_json::json!("high"));
        controls.insert("gamma".into(), serde_json::json!(9999));
        controls.insert("exposure-lock".into(), serde_json::json!(true));
        let said = fresh.load(&controls);
        assert_eq!(said.len(), 4, "{said:?}");
        let mut p = Prefs::default();
        assert!(fresh.into_prefs(&mut p).is_empty());
        assert_eq!((value(&fresh, "brightness"), value(&fresh, "contrast"), value(&fresh, "gamma")), (Value::Int(70), Value::Int(50), Value::Int(100)));
        assert_eq!((p.view.zoom, fresh.record_size()), (5, Size::new(1920, 1080)));
        assert_eq!(fresh.tuning().get(&Knob::Brightness), Some(&70));
        assert_eq!(fresh.tuning().get(&Knob::PowerLine), Some(&2));
        assert_eq!(fresh.saved(), l.saved());
    }

    /// `precedence` is set whole: every holder once, a floor its holder has; else refused as invalid and the order
    /// stays. A stored order this build cannot take is the default, whole, said.
    #[test]
    fn the_precedence_is_checked_whole_on_set_and_at_start() {
        let l = list();
        let c = l.get("precedence").unwrap();
        assert_eq!((c.kind, c.kept), (Kind::Order, true));
        assert_eq!(c.values, ["recording", "player", "usb", "hdmi", "photo", "drawing", "picture"]);
        assert_eq!(c.value, Value::Text("recording > player > usb > hdmi > photo > drawing > picture".into()));
        let target = |v: &str| l.target("precedence", Change::Set(Value::Text(v.into())));
        assert_eq!(target("recording>player>hdmi:paused>usb>photo>drawing>picture"), Ok(Value::Text("recording > player > hdmi:paused > usb > photo > drawing > picture".into())));
        for bad in ["recording > player > usb > hdmi > photo > drawing", "recording > player > usb > usb > hdmi > photo > drawing > picture", "recording > player > usb > hdmi:gone > photo > drawing > picture", "recording > player > usb > hdmi > photo > drawing > picture > tv"] {
            assert_eq!(target(bad).unwrap_err().code, Code::Invalid, "{bad}");
        }
        assert_eq!(l.target("precedence", Change::Adjust(1)).unwrap_err().code, Code::Invalid);
        assert_eq!(l.target("precedence", Change::Set(Value::Int(1))).unwrap_err().code, Code::Invalid);
        // kept: back at start; one this build cannot take (a holder it does not know): the default, said once
        let mut l = list();
        assert!(l.load(&[("precedence".to_string(), serde_json::json!("recording > player > hdmi > usb > photo > drawing > picture"))].into()).is_empty());
        let mut p = Prefs::default();
        assert!(l.into_prefs(&mut p).is_empty());
        assert!(p.precedence.above(Who::Hdmi, Who::Usb));
        let mut l = list();
        let said = l.load(&[("precedence".to_string(), serde_json::json!("recording > player > usb > hdmi > photo > drawing > picture > overlay"))].into());
        assert_eq!(said.len(), 1, "{said:?}");
        let mut p = Prefs::default();
        assert!(l.into_prefs(&mut p).is_empty());
        assert_eq!(p.precedence, crate::precedence::Precedence::default());
        assert!(!l.saved().contains_key("precedence"));
    }

    /// A fixed HDMI timing kept comes back at start, before any monitor is known; one that is no timing does not.
    #[test]
    fn a_kept_hdmi_timing_survives_the_start() {
        let mut l = list();
        assert!(l.load(&[("hdmi".to_string(), serde_json::json!("1080p60"))].into()).is_empty());
        let mut p = Prefs::default();
        assert!(l.into_prefs(&mut p).is_empty());
        assert_eq!(built("hdmi", &p), Value::Text("1080p60".into()));
        assert_eq!(l.get("hdmi").unwrap().values, ["auto", "off", "1080p60"]);
        assert_eq!(list().load(&[("hdmi".to_string(), serde_json::json!("1080p61"))].into()).len(), 1);
    }

    #[test]
    fn reset_brings_every_control_back_but_a_running_recording_s() {
        let mut l = list();
        set(&mut l, "brightness", Value::Int(70)).unwrap();
        set(&mut l, "exposure-lock", Value::Bool(true)).unwrap();
        set(&mut l, "record-size", Value::Text("1080".into())).unwrap();
        let mut p = Prefs::default();
        apply("zoom", &Value::Int(5), &mut p).unwrap();
        apply("record-codec", &Value::Text("h265".into()), &mut p).unwrap();
        p.recording = Some(Recording { size: Size::new(1920, 1080), path: String::new() });
        l.record_runs(true);
        l.follow(&p, &Monitor::Absent);
        let (told, build) = l.reset();
        assert!(told.is_empty(), "{told:?}");
        assert_eq!(build, [("zoom".to_string(), Value::Int(0))], "the recording's codec stays while it runs");
        assert_eq!(value(&l, "brightness"), Value::Int(70), "the picture's wait for the ISP");
        assert_eq!(l.reset_picture(), ["brightness", "exposure-lock"]);
        assert_eq!(value(&l, "record-size"), Value::Text("1080".into()));
        assert_eq!(l.tuning()[&Knob::Brightness], 50, "the ISP told the default: as tuned");
        assert_eq!(l.saved().keys().collect::<Vec<_>>(), ["record-size"], "the recording's own stays the user's");
        p.recording = None;
        l.record_runs(false);
        l.follow(&p, &Monitor::Absent);
        let (told, build) = l.reset();
        assert_eq!((told, build.len()), (vec!["record-size".to_string()], 2));
    }

    #[test]
    fn hardware_defaults_move_values_the_user_has_not_set() {
        let mut l = list();
        assert_eq!(l.tuning()[&Knob::Backlight], AS_TUNED, "before the IQ file is read: as it has it");
        assert_eq!(l.hardware(&[(Knob::Backlight, 1)]), ["backlight"]);
        assert_eq!(value(&l, "backlight"), Value::Bool(true), "not set: follows");
        assert_eq!(l.tuning()[&Knob::Backlight], AS_TUNED);
        set(&mut l, "backlight", Value::Bool(false)).unwrap();
        assert_eq!(l.hardware(&[(Knob::Backlight, 1)]), Vec::<String>::new());
        assert_eq!((value(&l, "backlight"), l.tuning()[&Knob::Backlight]), (Value::Bool(false), 0), "set: stays");
        l.reset_picture();
        assert_eq!((value(&l, "backlight"), l.tuning()[&Knob::Backlight]), (Value::Bool(true), AS_TUNED), "reset: the IQ file's again");
    }

    /// The settings file keeps what the user set, even at its default; a reset clears that; an old file (only the values
    /// that differed from their defaults) is the user's as it is.
    #[test]
    fn the_file_keeps_what_the_user_set() {
        let mut l = list();
        set(&mut l, "brightness", Value::Int(50)).unwrap();
        set(&mut l, "power-line", Value::Text("50".into())).unwrap();
        set(&mut l, "record-size", Value::Text("4k".into())).unwrap();
        assert_eq!(l.saved().len(), 3);
        l.reset();
        assert_eq!(l.saved().keys().collect::<Vec<_>>(), ["brightness", "power-line"], "the picture's are reset once the ISP took it");
        l.reset_picture();
        assert!(l.saved().is_empty());
        let mut old = list();
        assert!(old.load(&[("contrast".to_string(), serde_json::json!(70))].into()).is_empty());
        assert_eq!(old.saved(), [("contrast".to_string(), serde_json::json!(70))].into());
    }

    fn host(l: &mut List, name: &str, v: Value, now: Now) -> Vec<String> {
        let v = l.target(name, Change::Set(v)).unwrap();
        l.store(name, v, By::Host, &now)
    }

    /// A USB host's change holds while it is connected: the picture and the controls show it, the file keeps the user's
    /// values, nothing is marked his; the host gone, the user's come back. A change of the user's meanwhile is his.
    #[test]
    fn a_host_s_change_lasts_while_it_is_connected() {
        let mut l = list();
        set(&mut l, "brightness", Value::Int(40)).unwrap();
        host(&mut l, "brightness", Value::Int(70), Now::default());
        host(&mut l, "power-line", Value::Text("60".into()), Now::default());
        assert_eq!((value(&l, "brightness"), l.tuning()[&Knob::Brightness], l.tuning()[&Knob::PowerLine]), (Value::Int(70), 70, 2));
        assert_eq!(l.saved(), [("brightness".to_string(), serde_json::json!(40))].into(), "the user's, and no power-line");
        assert!(l.host_holds());
        assert_eq!(l.host_gone(), ["brightness", "power-line"]);
        assert_eq!((value(&l, "brightness"), value(&l, "power-line")), (Value::Int(40), Value::Text("50".into())));
        assert!(!l.host_holds() && l.host_gone().is_empty());
        // the user's change meanwhile is his: it stays when the host goes
        host(&mut l, "contrast", Value::Int(80), Now::default());
        let v = l.target("contrast", Change::Adjust(1)).unwrap();
        l.store("contrast", v, By::User, &Now::default());
        assert!(!l.host_holds() && l.host_gone().is_empty());
        assert_eq!((value(&l, "contrast"), l.saved()["contrast"].clone()), (Value::Int(81), serde_json::json!(81)));
    }

    /// A host switching an auto off: what it fixes beside it is its too, and all of it comes back when it goes.
    #[test]
    fn a_host_s_auto_off_comes_back_whole() {
        let mut l = list();
        let ran = Now { exposure: Some(ExposureNow { time_100us: 120, gain_pct: 450 }), white_balance: Some(WhiteBalanceNow { gains: (52, 32, 40), kelvin: 4312 }) };
        set(&mut l, "exposure-lock", Value::Bool(true)).unwrap();
        host(&mut l, "auto-exposure", Value::Bool(false), ran);
        host(&mut l, "exposure", Value::Int(40), Now::default());
        host(&mut l, "auto-white-balance", Value::Bool(false), ran);
        let t = l.tuning();
        assert_eq!((t[&Knob::AutoExposure], t[&Knob::Exposure], t[&Knob::WhiteBalanceByGains]), (0, 40, 1));
        assert!(l.saved().is_empty(), "{:?}", l.saved());
        l.host_gone();
        let t = l.tuning();
        assert_eq!((t[&Knob::AutoExposure], t[&Knob::AutoWhiteBalance], t[&Knob::WhiteBalanceByGains]), (0, 1, 0), "the user's lock holds the exposure again");
        assert_eq!((value(&l, "auto-exposure"), value(&l, "exposure-lock"), value(&l, "exposure")), (Value::Bool(true), Value::Bool(true), Value::Int(Knob::Exposure.range().default as i64)));
        // a reset is the user's: no host's hold outlives it
        host(&mut l, "brightness", Value::Int(70), Now::default());
        l.reset_picture();
        assert!(!l.host_holds());
    }

    /// A backlight a host holds stays the host's through an IQ load; the user's value aside follows the IQ file's.
    #[test]
    fn a_host_s_backlight_holds_through_the_hardware_default() {
        let mut l = list();
        host(&mut l, "backlight", Value::Bool(true), Now::default());
        assert_eq!(l.tuning()[&Knob::Backlight], 1, "the host's, not as tuned");
        l.hardware(&[(Knob::Backlight, 0)]);
        assert_eq!(value(&l, "backlight"), Value::Bool(true));
        l.host_gone();
        assert_eq!((value(&l, "backlight"), l.tuning()[&Knob::Backlight]), (Value::Bool(false), AS_TUNED));
    }

    /// A knob the ISP refused after its IQ load: its controls unavailable with why, until a load takes it.
    #[test]
    fn a_refused_knob_is_unavailable() {
        let mut l = list();
        let told = l.refused([(Knob::Exposure, "no IQ baseline".to_string())].into());
        assert_eq!(told, ["auto-exposure", "exposure", "gain", "exposure-lock"], "the exposure is one operation");
        let r = l.target("auto-exposure", Change::Set(Value::Bool(false))).unwrap_err();
        assert_eq!((r.code, r.error.as_str()), (Code::Unavailable, "auto-exposure: the ISP refused it: no IQ baseline"));
        assert!(l.get("brightness").unwrap().available);
        l.refused(BTreeMap::new());
        assert!(l.get("auto-exposure").unwrap().available && !l.get("exposure").unwrap().available, "back to the autos' rule");
    }

    /// A knob changed while no ISP runs is kept and lands with the next IQ load, with every other knob.
    #[test]
    fn a_change_while_no_isp_runs_lands_with_the_next_load() {
        use crate::desired::Revision;
        use crate::policy::Inputs;
        use crate::reconcile::{plan, Current, Op};
        use crate::tuning::Adjust;
        let modes = crate::modes::tests::table();
        let mut l = list();
        set(&mut l, "brightness", Value::Int(70)).unwrap();
        set(&mut l, "auto-exposure", Value::Bool(false)).unwrap();
        let d = policy().plan(&Inputs { monitor: &Monitor::Absent, host: None, prefs: &Prefs::default(), tuning: &l.tuning() }, &modes, Revision(1)).unwrap();
        let ops = plan(&Current::default(), &d, &modes).unwrap().ops;
        let load = ops.iter().position(|o| matches!(o, Op::IspLoadIq { .. })).unwrap();
        let after = |a: Adjust| ops.iter().position(|o| *o == Op::IspAdjust { dev: 0, chn: 0, adjust: a.clone() }).is_some_and(|i| i > load);
        assert!(after(Adjust::Iq { knob: Knob::Brightness, value: 70 }));
        assert!(after(Adjust::Exposure { auto: false, time_100us: 333, gain_pct: 100 }), "manual at the values it holds");
    }

    /// For every control: the ISP gets the same steps after a cold start with no settings as after its default is set
    /// explicitly — every knob goes to the ISP after the IQ load, whether the user set it or not. A backlight not set is
    /// sent as the IQ file has it ([`AS_TUNED`]): the hardware writes the IQ's own state then, the control's default.
    #[test]
    fn a_cold_start_asks_the_isp_what_the_defaults_set_explicitly_ask() {
        use crate::desired::Revision;
        use crate::policy::Inputs;
        use crate::reconcile::{plan, Current, Op};
        use crate::tuning::Adjust;
        let modes = crate::modes::tests::table();
        let steps = |l: &List, p: &Prefs| -> Vec<Op> {
            let backlight = knob_of(l.get("backlight").unwrap());
            let d = policy().plan(&Inputs { monitor: &Monitor::Absent, host: None, prefs: p, tuning: &l.tuning() }, &modes, Revision(1)).unwrap();
            let mut ops = plan(&Current::default(), &d, &modes).unwrap().ops;
            for op in &mut ops {
                if let Op::IspAdjust { adjust: Adjust::Iq { knob: Knob::Backlight, value }, .. } = op {
                    if *value == AS_TUNED {
                        *value = backlight;
                    }
                }
            }
            ops
        };
        let cold = || {
            let mut l = list();
            l.hardware(&[(Knob::Backlight, 1)]);
            l
        };
        let base = steps(&cold(), &Prefs::default());
        assert!(base.iter().any(|o| matches!(o, Op::IspAdjust { adjust: Adjust::Exposure { auto: true, .. }, .. })), "the auto exposure asked for");
        assert!(base.iter().any(|o| matches!(o, Op::IspAdjust { adjust: Adjust::WhiteBalance { auto: true, .. }, .. })), "the auto white balance asked for");
        for c in cold().all() {
            let (mut l, mut p) = (cold(), Prefs::default());
            match l.drive(&c.name).unwrap() {
                Drive::Built => {
                    apply(&c.name, &c.default, &mut p).unwrap();
                    l.follow(&p, &Monitor::Absent);
                    l.chosen(&c.name);
                }
                _ => {
                    l.store(&c.name, c.default.clone(), By::User, &Now::default());
                }
            }
            assert_eq!(steps(&l, &p), base, "{} set to its default", c.name);
        }
    }

    #[test]
    fn a_control_is_json_as_the_protocol_says() {
        let l = list();
        let j = serde_json::to_value(l.get("exposure-bias").unwrap()).unwrap();
        assert_eq!(j, serde_json::json!({"name": "exposure-bias", "kind": "int", "min": -30, "max": 30, "step": 3, "default": 0, "value": 0, "kept": true, "available": true}));
        let j = serde_json::to_value(l.get("white-balance").unwrap()).unwrap();
        assert_eq!(j["why"], "auto-white-balance is on");
    }
}
