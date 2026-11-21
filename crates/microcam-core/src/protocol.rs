//! The control protocol between `microcam` (CLI, tests, the menu program) and `microcamd`: one JSON object per line in each
//! direction. A request may carry a `seq` that its reply repeats, so
//! requests can follow each other unanswered; microcamd's own messages to the menu are [`Event`]s, tagged `event`. Shared
//! types only; transport lives in the daemon and the CLI.

use serde::{Deserialize, Serialize};

use crate::controls::{Code, Control, Holder, Refusal, Value};
use crate::desired::{Desired, Pause};
use crate::edit::Op;
use crate::files::FileEntry;
use crate::keys::{Key, Phase, Push};
use crate::reconcile::Current;
use crate::hdmi::Timing;
use crate::scene;
use crate::Codec;

/// The protocol this daemon speaks (the scene's own format is `scene::VERSION`).
pub const PROTOCOL: u32 = 7;

/// A request as it comes: the request and the number its reply repeats.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Envelope {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seq: Option<u64>,
    #[serde(flatten)]
    pub req: Request,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "kebab-case")]
pub enum Request {
    /// What the daemon is doing.
    Status,
    /// A control (`crate::controls`) to this value; answered with the value accepted, or refused with a [`Code`].
    Set { name: String, value: Value },
    /// An int control by `steps` × its step, a choice by that many places along its values, clamped; computed from the
    /// value microcamd holds, answered as `set`.
    Adjust { name: String, steps: i64 },
    /// Follow the camera without taking the scene: answered with the [`Snapshot`], then every event but the scene's.
    Watch,
    /// The menu's own record (any JSON, at most [`MAX_RECORD`] bytes), kept in the settings file and handed back in the
    /// snapshot; the scene's owner's only.
    Keep { record: serde_json::Value },
    /// A photo at the full size into the card's next `PHOTO/P#####.jpg`: answered at once, its outcome
    /// [`Event::PhotoSaved`] or [`Event::Failed`].
    Photo,
    /// A remote's key (`crate::keys`) pressed or released (`press` · `release`) without the remote: written into its
    /// device, it comes back as the remote's own, the kernel's repeats included. A key this connection leaves pressed is
    /// released when it closes.
    Key { key: Key, phase: Push },
    /// Start a recording at the `record-size` control's size into the next `V#####.mp4` on the card: answered at once,
    /// its state in [`Running`].
    Record,
    /// The recording stopped: answered at once, its outcome [`Event::Recorded`].
    RecordStop,
    /// The SD card formatted (FAT32), `PHOTO/` and `VIDEO/` made: answered at once, its outcome [`Event::Formatted`].
    Format,
    /// Every control back to its default, the menu's record gone, the scene taken from its owner.
    Reset,
    /// `count` of the card's photos and videos from place `from`, newest first, with the total and the list's version.
    Files {
        #[serde(default)]
        from: usize,
        #[serde(default = "default_files")]
        count: usize,
    },
    /// Pretend a host committed this stream (format index, frame index, interval) or stopped (`off = true`).
    /// For tests without a USB host; a real host stream always wins.
    SimStream {
        #[serde(default)]
        off: bool,
        #[serde(default)]
        format: u8,
        #[serde(default)]
        frame: u8,
        #[serde(default)]
        interval: u32,
    },
    /// This connection is the menu: it owns the scene (empty, revision 0), is answered with the [`Snapshot`] and gets
    /// [`Event`]s from now on; when it closes, the scene is cleared on every output. Refused while another menu holds the
    /// scene, unless `take`: then that one is told ([`Event::Lost`]) and its commits are refused.
    Hello {
        #[serde(default)]
        take: bool,
    },
    /// The menu's one change to the scene: `ops` applied in order to revision `base`; accepted whole as `base + 1` or
    /// refused whole (also when the scene is no longer at `base`).
    Commit { base: u64, ops: Vec<Op> },
    /// A picture the menu's scene can show by `id` (its own name), kept until released; its state comes as
    /// [`Event::Resource`].
    Resource {
        id: String,
        #[serde(flatten)]
        source: Source,
    },
    /// The menu needs the resource no more: it goes once nothing shown uses it ([`ResState::Gone`]).
    Release { id: String },
    /// How microcamd lays out and draws this text: its box, lines and glyph positions, in its own units.
    Measure { text: scene::TextNode },
    /// The overlay outputs' hardware windows as they are now: a query for us and the menu's
    /// author, one line per window.
    UiWindows,
    /// The memory pools as they are now, one line each; `reset` starts the peaks again.
    Mem {
        #[serde(default)]
        reset: bool,
        /// one line: heap free, canvases, the menu's share of its limit, Linux available
        #[serde(default)]
        brief: bool,
    },
    /// The media heap now: the video's owners priced, the heap's listing by owner, microcamd's share and its use, free.
    Account,
    /// What `with` — `set` of a control the pipeline builds, `record`, `record-stop`, `play-stop`, and `sim-stream` as
    /// the host — would cost, applied in order to a copy of the wishes, planned and priced; nothing is built.
    Price { with: Vec<Request> },
    /// Show the MP4 `file` (its path on the camera, as a [`FileEntry`] gives it) in the outputs' live windows instead of
    /// the sensor, from its start: answered at once, the player `starting` before the answer; its outcome the player
    /// `playing`, or `failed` with the player gone. Refused while a recording runs, while the card is formatted, while
    /// the player starts or stops; a recording and a photo are refused while it plays.
    Play { file: String },
    /// The picture held where it is (the menu over it goes on changing).
    PlayPause,
    PlayResume,
    /// From the sync sample at or before `ms`, paused or playing as it was.
    PlaySeek { ms: u64 },
    /// The sensor back in the live windows: answered at once, the player `stopping` before the answer; its outcome the
    /// player gone, or `failed` with the player as it was.
    PlayStop,
    /// Tear the pipeline down (then build it again from the current inputs).
    Rebuild,
    /// Last log lines.
    Log {
        #[serde(default = "default_lines")]
        lines: usize,
    },
    /// The log lines from number `from` on (`u64::MAX`: none, only where the ring is now); the message is the number of
    /// the next line — `microcam log follow` asks again from there.
    LogSince { from: u64 },
    /// Set the log level live (`debug`, `off`, `info,ui=debug,kernel=off`); without one, say what it is.
    LogLevel {
        #[serde(default)]
        spec: Option<String>,
    },
    /// Stop the daemon (pipeline torn down first).
    Quit,
}

fn default_lines() -> usize {
    40
}

fn default_files() -> usize {
    50
}

/// The longest menu record `keep` takes, bytes of its JSON.
pub const MAX_RECORD: usize = 4096;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Status {
    pub state: DaemonState,
    pub uptime_s: u64,
    pub monitor: String,
    pub host_stream: Option<String>,
    pub desired: Option<Desired>,
    pub current: Current,
    pub last_error: Option<String>,
    pub last_transition: Option<String>,
    pub transitions: u64,
    /// USB pump counters since the stream started (frames queued to the gadget, dropped, too big for a buffer)
    #[serde(default)]
    pub pump: Option<(u64, u64, u64)>,
    /// capture → USB queue over the last complete window of the stream, average and largest, in ms
    #[serde(default)]
    pub usb_delay: Option<(f32, f32)>,
    /// the `ui` on USB: its RGN regions
    #[serde(default)]
    pub usb_ui: Option<String>,
    /// the `ui` in a recording: its RGN regions
    #[serde(default)]
    pub media_ui: Option<String>,
    /// display levels: HDMI, USB, media (recording and photo), as the `level-*` controls
    #[serde(default)]
    pub ui_levels: [u8; 3],
    /// the build: «<commit>[-dirty] <UTC time>»
    #[serde(default)]
    pub build: String,
    /// a menu's scene is held (memory under the menu's share with none held is left over)
    #[serde(default)]
    pub scene: bool,
    /// the player, when a file is open
    #[serde(default)]
    pub player: Option<PlayerInfo>,
    #[serde(default)]
    pub card: CardInfo,
    /// the lists (sensor modes, USB formats) microcamd did not take from the loaded module at start
    #[serde(default)]
    pub lists: Vec<ListInUse>,
}

/// What microcamd is doing: building its first state, running, degraded by a build that failed (its error in
/// `last_error`), stopping.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DaemonState {
    Starting,
    Running,
    Degraded,
    Stopping,
}

/// Where the SD card is: none in the camera, mounted on the camera, or the USB host's disk.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Card {
    #[default]
    None,
    Camera,
    Host,
}

/// The SD card: where it is and, mounted on the camera, its space — read when it is mounted (at microcamd's start just
/// after, a `card` event then) and after each change of microcamd's own (a photo written, a recording closed, a format),
/// not while a recording writes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CardInfo {
    pub at: Card,
    /// bytes free and the file system's size; on the camera only
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub free: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total: Option<u64>,
}

/// A list microcamd works with that the loaded module did not give: the stock module's, which gives none, or the
/// conservative profile in place of a list microcamd cannot use.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListInUse {
    /// `sensor mode list` · `USB format list`
    pub table: String,
    /// what microcamd works with
    pub used: String,
    /// why this is not as it should be, with the file; none for the stock module of the tested firmware
    pub anomaly: Option<String>,
}

/// The player's file and where it is: told in `status` and, on every change of `state`, as [`Event::Player`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlayerInfo {
    /// its path on the camera
    pub file: String,
    pub state: PlayerState,
    /// the time of the frame last sent to the decoder
    pub ms: u64,
    pub duration_ms: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PlayerState {
    /// `play` accepted, the file not shown yet (`ms` and `duration_ms` 0)
    Starting,
    Playing,
    Paused,
    /// paused on the last frame
    Ended,
    /// `play-stop` accepted, the sensor not back yet
    Stopping,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Response {
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// why a request was refused, for the program to word
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<Code>,
    /// refused for lack of resources (`code` `resources`): what holds them, the most first
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub holders: Vec<Holder>,
    /// the value a `set` or `adjust` was accepted at
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot: Option<Snapshot>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub files: Option<FilesWindow>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<Status>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lines: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// the request's `seq`
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seq: Option<u64>,
    /// a commit's accepted revision
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hello: Option<Hello>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub measure: Option<Measure>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<Account>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub price: Option<Price>,
}

/// The media heap as microcamd sees it, bytes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Account {
    /// the video's owners as priced (`heap::video` of the state built)
    pub priced: Vec<(String, u64)>,
    /// the heap's own listing by owner
    pub held: Vec<(String, u64)>,
    pub heap: u64,
    pub free: u64,
    /// microcamd's share (what the video leaves; the menu's tiles and pictures and microcamd's own uses take from it)
    /// and what it holds of it
    pub share: u64,
    pub used: u64,
}

/// What `price` found, bytes: the video's owners after the requests, the transition's worst step, microcamd's share after
/// — with what would give way by the precedence for them (a recording, a file played, a host's stream) — and, when it
/// would be refused, what is short and what holds the camera's resources now.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Price {
    pub priced: Vec<(String, u64)>,
    pub peak: u64,
    pub share: u64,
    pub short: Option<crate::heap::Short>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub holders: Vec<Holder>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub gives: Vec<Gives>,
}

/// A holder giving way by the precedence, and its step.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Gives {
    pub holder: crate::precedence::Who,
    pub step: crate::precedence::Step,
}

/// A text as microcamd lays it out, in the node's units (screen units), from its (`x`, `y`): the box its lines take,
/// and each line.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Measure {
    pub w: f32,
    pub h: f32,
    pub lines: Vec<MeasuredLine>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MeasuredLine {
    /// what the line shows (wrapped, cut, with its "…")
    pub text: String,
    /// its left edge, top, baseline and width
    pub x: f32,
    pub y: f32,
    pub baseline: f32,
    pub w: f32,
    /// each character's left edge
    pub glyphs: Vec<f32>,
}

/// The card's file list as a whole: its version (raised at every change of the set) and how many files it holds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FilesInfo {
    pub version: u64,
    pub total: usize,
}

/// A window of the card's file list, newest first: from place `from`, of the list at `version`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FilesWindow {
    pub from: usize,
    pub total: usize,
    pub version: u64,
    pub list: Vec<FileEntry>,
}

/// What takes time and runs now, one of each kind at a time: it is in here before the request that began it is
/// answered, and its outcome event goes before the next of its kind is admitted.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Running {
    /// the photo being taken: its path on the camera
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub photo: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub record: Option<RecordRun>,
    /// the card being formatted
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub format: bool,
    /// the controls the pipeline is building
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub building: Vec<String>,
}

/// The recording: its path on the camera, where it is, and how long it has run when told (a client counts on from there).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordRun {
    pub file: String,
    pub state: RecordState,
    pub ms: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RecordState {
    Starting,
    Running,
    Stopping,
}

/// What the menu is told when it takes the scene (beside the [`Snapshot`]).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Hello {
    pub protocol: u32,
    /// the scene format (`scene::VERSION`)
    pub scene: u32,
    pub limits: Limits,
}

/// Everything a client needs to draw the camera's state, queued before any event that follows it: an event may repeat
/// what the snapshot holds, never the reverse.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    pub controls: Vec<Control>,
    pub running: Running,
    pub files: FilesInfo,
    pub outputs: Vec<OutputInfo>,
    pub player: Option<PlayerInfo>,
    pub card: CardInfo,
    /// the menu's own record (`keep`)
    pub record: Option<serde_json::Value>,
}

/// What a scene may hold (checked at every commit), and what microcamd holds for it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Limits {
    pub nodes: usize,
    pub depth: usize,
    pub segments: usize,
    /// characters in one text node, and in the whole scene
    pub text: usize,
    pub text_total: usize,
    pub levels: u8,
    /// the menu's pixels — the resources' pictures and the drawn tiles of every output, 4 bytes a pixel — together:
    /// what an accepted revision may need to be shown
    pub menu_bytes: u64,
    /// an uploaded picture's longest side
    pub picture_side: u32,
}

/// An output the scene is shown on, its frame in pixels and display level;
/// `paused`: its live picture is paused and why — the frame keeps the pictures and the menu, the scene's `paused`
/// nodes show on it. HDMI carries its timing; USB, there while a host reads its stream, the stream's codec and rate
/// (its size is the frame's) — as built, what `status` reports.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutputInfo {
    pub name: Output,
    pub w: u32,
    pub h: u32,
    pub level: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paused: Option<Pause>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timing: Option<Timing>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub codec: Option<Codec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fps: Option<u32>,
}

/// An output the scene is shown on: HDMI, USB, or a recording (`media`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Output {
    Hdmi,
    Usb,
    Media,
}

/// Where a resource's picture comes from.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Source {
    /// `w` × `h` pixels of 8-bit RGBA, straight alpha, rows without padding, in a memfd passed with the request
    /// (SCM_RIGHTS on the Unix socket); read once, the menu may close or reuse it after the reply
    Pixels { w: u32, h: u32 },
    /// a JPEG file or, by its cover, an MP4 on the camera (a [`FileEntry`]'s path), decoded by microcamd at the size it is
    /// shown
    File(String),
    /// the camera's picture now (what freezing was): the live view as a recording sees it (the zoom window, no menu),
    /// taken once and kept at most `w` × `h`, its aspect kept; shown in full colour only (`full_colour` images)
    Camera { w: u32, h: u32 },
}

/// A resource's state.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "kebab-case")]
pub enum ResState {
    Loading,
    /// the source picture's size
    Ready { w: u32, h: u32 },
    /// `code` for the menu to act on, `error` for people; `resources`: `holders` hold them
    Failed {
        code: ResCode,
        error: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        holders: Vec<Holder>,
    },
    /// released and no longer used by anything shown
    Gone,
}

/// Why a resource failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ResCode {
    /// a video without a cover (the menu draws its own sign for it)
    NoPicture,
    /// the file could not be read, or is no picture the decoders read
    Unreadable,
    /// the camera lacks the resources for it (microcamd's share of the media heap): `holders` hold them
    Resources,
    /// the camera's picture could not be taken now: a recording or a photo has the media port
    Busy,
    /// the camera's picture could not be taken: the build or the grab failed
    Camera,
}

/// An action whose outcome comes later, as it is named in [`Event::Failed`]: `"photo"`, `"record-stop"`, …, and a
/// control's build as `{"control": "<name>"}`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Action {
    Photo,
    Record,
    RecordStop,
    Format,
    Reset,
    Play,
    PlayStop,
    Control(String),
    /// an output microcamd starts or keeps by itself — a monitor plugged, the host's stream, a paused output's frame
    /// beside a recording — refused or left out: `{"output": "usb"}`
    Output(Output),
}

/// microcamd's own messages: to the menu all of them, to a `watch`ing connection all but the scene's (`presented`,
/// `not-shown`, `lost`, `resource`, `account`, `key`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "kebab-case")]
pub enum Event {
    /// `output` shows `revision` from its next frame on; revisions before it that it never presented it skipped.
    /// `complete`: all of it drawn (false while a resource it shows is not ready).
    Presented { output: Output, revision: u64, complete: bool },
    /// `output` cannot show `revision` and keeps showing what it showed; it tries again, and says `presented` if it gets
    /// there. `code` `resources`: its drawing does not fit, `holders` hold the room; `refused`: the output cannot carry
    /// the menu now (`reason` for people).
    NotShown {
        output: Output,
        revision: u64,
        code: Code,
        reason: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        holders: Vec<Holder>,
    },
    /// The outputs now (one came, went, or changed its size, level, pause, timing or stream).
    Outputs { outputs: Vec<OutputInfo> },
    /// Another menu took the scene: this connection's commits are refused from now on.
    Lost,
    Resource {
        id: String,
        #[serde(flatten)]
        state: ResState,
    },
    /// microcamd's share moved (the video took or gave back heap): the menu's pictures and tiles may take `menu_bytes` less
    /// what microcamd holds, `held`.
    Account { menu_bytes: u64, held: u64 },
    /// The player starting, playing, paused, resumed, sought, ended, stopping or stopped (`player` none).
    Player { player: Option<PlayerInfo> },
    /// The SD card came, went, moved between the camera and the USB host, or its space changed.
    Card { card: CardInfo },
    /// A control's value, availability or values changed, whoever changed it: the whole entry.
    Control { control: Control },
    /// A remote's key, to the scene's owner alone (the owner it was pressed under).
    Key { key: Key, phase: Phase },
    /// What runs changed (the whole of it).
    Running { running: Running },
    /// The card's file list changed: its new version and total.
    Files { version: u64, total: usize },
    /// The photo is whole on the card: its path.
    PhotoSaved { file: String },
    /// The recording stopped as asked and its file (its path) closed.
    Recorded { file: String },
    Formatted,
    /// An action accepted did not get there, or an output microcamd keeps by itself was refused or left out: which,
    /// why; `resources`: `holders` hold them.
    Failed {
        what: Action,
        code: Code,
        error: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        holders: Vec<Holder>,
    },
}

impl Event {
    /// `what` did not get there: the refusal's code, text and holders.
    pub fn failed(what: Action, r: Refusal) -> Self {
        Event::Failed { what, code: r.code, error: r.error, holders: r.holders }
    }
}

impl Response {
    pub fn ok() -> Self {
        Self { ok: true, error: None, code: None, holders: Vec::new(), value: None, snapshot: None, files: None, status: None, lines: None, message: None, seq: None, revision: None, hello: None, measure: None, account: None, price: None }
    }
    pub fn err(e: impl ToString) -> Self {
        Self { ok: false, error: Some(e.to_string()), ..Self::ok() }
    }
    pub fn refused(r: Refusal) -> Self {
        Self { ok: false, error: Some(r.error), code: Some(r.code), holders: r.holders, ..Self::ok() }
    }
    /// A `set` or `adjust` answered: the value accepted, or the refusal.
    pub fn of(r: Result<Value, Refusal>) -> Self {
        match r {
            Ok(v) => Self { value: Some(v), ..Self::ok() },
            Err(r) => Self::refused(r),
        }
    }
    pub fn msg(m: impl ToString) -> Self {
        Self { message: Some(m.to_string()), ..Self::ok() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn envelope_carries_seq_beside_the_request() {
        let e: Envelope = serde_json::from_str(r#"{"seq":7,"cmd":"commit","base":3,"ops":[{"op":"remove","id":"a"}]}"#).unwrap();
        assert_eq!(e.seq, Some(7));
        assert!(matches!(e.req, Request::Commit { base: 3, .. }));
        let e: Envelope = serde_json::from_str(r#"{"cmd":"status"}"#).unwrap();
        assert_eq!((e.seq, e.req), (None, Request::Status));
        let ev = serde_json::to_string(&Event::Presented { output: Output::Usb, revision: 4, complete: true }).unwrap();
        assert_eq!(ev, r#"{"event":"presented","output":"usb","revision":4,"complete":true}"#);
        let r: Envelope = serde_json::from_str(r#"{"cmd":"resource","id":"icon","pixels":{"w":2,"h":3}}"#).unwrap();
        assert_eq!(r.req, Request::Resource { id: "icon".into(), source: Source::Pixels { w: 2, h: 3 } });
        let r: Envelope = serde_json::from_str(r#"{"cmd":"resource","id":"ph","file":"/mnt/sd/P1.jpg"}"#).unwrap();
        assert_eq!(r.req, Request::Resource { id: "ph".into(), source: Source::File("/mnt/sd/P1.jpg".into()) });
        let r: Envelope = serde_json::from_str(r#"{"cmd":"resource","id":"fr","camera":{"w":1920,"h":1080}}"#).unwrap();
        assert_eq!(r.req, Request::Resource { id: "fr".into(), source: Source::Camera { w: 1920, h: 1080 } });
        let r: Envelope = serde_json::from_str(r#"{"cmd":"price","with":[{"cmd":"record"},{"cmd":"sim-stream","format":1,"frame":1,"interval":333333}]}"#).unwrap();
        assert!(matches!(&r.req, Request::Price { with } if with.len() == 2 && with[0] == Request::Record));
        let r: Envelope = serde_json::from_str(r#"{"seq":2,"cmd":"set","name":"hdmi","value":"1080p60"}"#).unwrap();
        assert_eq!(r.req, Request::Set { name: "hdmi".into(), value: Value::Text("1080p60".into()) });
        let r: Envelope = serde_json::from_str(r#"{"cmd":"set","name":"grey","value":true}"#).unwrap();
        assert_eq!(r.req, Request::Set { name: "grey".into(), value: Value::Bool(true) });
        let refused = serde_json::to_string(&Response::refused(Refusal::new(Code::Unavailable, "exposure: auto-exposure is on"))).unwrap();
        assert_eq!(refused, r#"{"ok":false,"error":"exposure: auto-exposure is on","code":"unavailable"}"#);
        assert_eq!(serde_json::to_string(&Response::of(Ok(Value::Int(53)))).unwrap(), r#"{"ok":true,"value":53}"#);
        let ev = serde_json::to_string(&Event::Resource { id: "ph".into(), state: ResState::Ready { w: 4, h: 3 } }).unwrap();
        assert_eq!(ev, r#"{"event":"resource","id":"ph","state":"ready","w":4,"h":3}"#);
    }

    #[test]
    fn closed_sets_go_as_words() {
        let ev = Event::Resource { id: "v".into(), state: ResState::Failed { code: ResCode::NoPicture, error: "no cover".into(), holders: vec![] } };
        assert_eq!(serde_json::to_string(&ev).unwrap(), r#"{"event":"resource","id":"v","state":"failed","code":"no-picture","error":"no cover"}"#);
        let ev = Event::Failed { what: Action::RecordStop, code: Code::Refused, error: "e".into(), holders: vec![] };
        assert_eq!(serde_json::to_string(&ev).unwrap(), r#"{"event":"failed","what":"record-stop","code":"refused","error":"e"}"#);
        let ev = Event::Failed { what: Action::Control("hdmi".into()), code: Code::Refused, error: "e".into(), holders: vec![] };
        assert_eq!(serde_json::to_string(&ev).unwrap(), r#"{"event":"failed","what":{"control":"hdmi"},"code":"refused","error":"e"}"#);
        let host = CardInfo { at: Card::Host, ..CardInfo::default() };
        assert_eq!(serde_json::to_string(&Event::Card { card: host }).unwrap(), r#"{"event":"card","card":{"at":"host"}}"#);
        let here = CardInfo { at: Card::Camera, free: Some(1), total: Some(2) };
        assert_eq!(serde_json::to_string(&Event::Card { card: here }).unwrap(), r#"{"event":"card","card":{"at":"camera","free":1,"total":2}}"#);
        let ev = Event::Failed { what: Action::PlayStop, code: Code::Busy, error: "e".into(), holders: vec![] };
        assert_eq!(serde_json::to_string(&ev).unwrap(), r#"{"event":"failed","what":"play-stop","code":"busy","error":"e"}"#);
        assert_eq!(serde_json::to_string(&PlayerState::Starting).unwrap(), r#""starting""#);
        let p = PlayerInfo { file: "/mnt/VIDEO/V00001.mp4".into(), state: PlayerState::Ended, ms: 1, duration_ms: 2 };
        assert_eq!(serde_json::to_string(&p).unwrap(), r#"{"file":"/mnt/VIDEO/V00001.mp4","state":"ended","ms":1,"duration_ms":2}"#);
        let k: Envelope = serde_json::from_str(r#"{"cmd":"key","key":"zoom-in","phase":"press"}"#).unwrap();
        assert_eq!(k.req, Request::Key { key: Key::ZoomIn, phase: Push::Press });
        assert!(serde_json::from_str::<Envelope>(r#"{"cmd":"key","key":"up","phase":"repeat"}"#).is_err(), "a request presses or releases");
        assert_eq!(serde_json::to_string(&Event::Key { key: Key::Up, phase: Phase::Repeat }).unwrap(), r#"{"event":"key","key":"up","phase":"repeat"}"#);
        let o = OutputInfo { name: Output::Media, w: 1, h: 1, level: 0, paused: None, timing: None, codec: None, fps: None };
        assert_eq!(serde_json::to_string(&o).unwrap(), r#"{"name":"media","w":1,"h":1,"level":0}"#);
        let o = OutputInfo { name: Output::Hdmi, w: 3840, h: 2160, level: 3, paused: None, timing: Some(Timing::P2160_30), codec: None, fps: None };
        assert_eq!(serde_json::to_string(&o).unwrap(), r#"{"name":"hdmi","w":3840,"h":2160,"level":3,"timing":"2160p30"}"#);
        let o = OutputInfo { name: Output::Usb, w: 1920, h: 1080, level: 3, paused: None, timing: None, codec: Some(Codec::Mjpeg), fps: Some(60) };
        assert_eq!(serde_json::to_string(&o).unwrap(), r#"{"name":"usb","w":1920,"h":1080,"level":3,"codec":"mjpeg","fps":60}"#);
        assert_eq!(serde_json::to_string(&DaemonState::Degraded).unwrap(), r#""degraded""#);
    }

    /// A refusal for lack of resources names what holds them, each a typed item, the most first; the same holders in an
    /// answer, a `failed`, a `not-shown` and a resource's failure.
    #[test]
    fn a_refusal_for_lack_of_resources_names_its_holders() {
        use crate::controls::{ordered, Held};
        let holders = ordered(vec![
            Holder { what: Held::Drawing, bytes: Some(2) },
            Holder { what: Held::Picture { id: "frozen-1".into() }, bytes: Some(12) },
            Holder { what: Held::Player { file: "/mnt/VIDEO/V00001.mp4".into() }, bytes: None },
            Holder { what: Held::Hdmi { timing: Timing::P2160_30 }, bytes: Some(60) },
            Holder { what: Held::Usb { codec: Codec::Mjpeg, w: 3840, h: 2160, fps: 45 }, bytes: Some(0) },
        ]);
        let r = serde_json::to_string(&Response::refused(Refusal::resources("short", holders.clone()))).unwrap();
        assert_eq!(
            r,
            r#"{"ok":false,"error":"short","code":"resources","holders":[{"holder":"hdmi","timing":"2160p30","bytes":60},{"holder":"picture","id":"frozen-1","bytes":12},{"holder":"drawing","bytes":2},{"holder":"player","file":"/mnt/VIDEO/V00001.mp4"}]}"#
        );
        let rec = vec![Holder { what: Held::Recording { w: 3840, h: 2160 }, bytes: Some(30) }];
        let ev = serde_json::to_string(&Event::failed(Action::Output(Output::Usb), Refusal::resources("e", rec.clone()))).unwrap();
        assert_eq!(ev, r#"{"event":"failed","what":{"output":"usb"},"code":"resources","error":"e","holders":[{"holder":"recording","w":3840,"h":2160,"bytes":30}]}"#);
        let ev = Event::NotShown { output: Output::Hdmi, revision: 3, code: Code::Resources, reason: "e".into(), holders: rec.clone() };
        let back: Event = serde_json::from_str(&serde_json::to_string(&ev).unwrap()).unwrap();
        assert_eq!(back, ev);
        let ev = Event::Resource { id: "p".into(), state: ResState::Failed { code: ResCode::Resources, error: "e".into(), holders: rec } };
        assert!(serde_json::to_string(&ev).unwrap().contains(r#""code":"resources","error":"e","holders":[{"holder":"recording""#));
    }
}
