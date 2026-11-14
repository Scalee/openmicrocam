//! The pipeline thread: the one owner of the MI graph. It reduces inputs to `Desired` (policy), plans and
//! applies transitions (reconciler), and answers status. Everything reaches it as a [`Msg`].

use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

use microcam_core::desired::{Desired, Epoch, PictureId, Revision};
use microcam_core::formats::HostStream;
use microcam_core::hdmi::Monitor;
use microcam_core::heap;
use microcam_core::overlay::{self, LayerRgn, RgnOutput};
use microcam_core::ui::OutputUi;
use microcam_core::modes::ModeTable;
use microcam_core::controls::{self, By, Change, Code, Drive, Held, Holder, List, Now, Refusal, Value};
use microcam_core::policy::{Frames, Giving, Inputs, Policy, Prefs};
use microcam_core::precedence::{Step, Who};
use microcam_core::protocol::{Action, DaemonState, Event, Gives, Output, OutputInfo, Price, ResCode, Status};
use microcam_core::wish::Wish;
use microcam_core::reconcile::{self, Current, Hal, Rung};
use microcam_core::tuning::{Knob, Tuning};
use microcam_core::{Rect, Size};

use crate::controls::{Controls, Reply};
use crate::running::Running;
use crate::ui::{GiveBack, RgnId};
use crate::watchdog::Health;

pub enum Msg {
    Monitor(Monitor),
    /// a host committed a stream; reply = the session epoch once the pipeline runs it, or the error
    StreamOn(HostStream, Sender<Result<Epoch, String>>),
    StreamOff(Sender<()>),
    /// the rate the policy would run this host stream at now (the UVC probe answer); None if it has no plan for it
    UsbRate(HostStream, Sender<Option<u32>>),
    /// a wish of the control's or the card's (a recording started or stopped, the player stopped); the reply (if any)
    /// says whether it was built
    Wish(Wish, Option<Sender<Result<(), String>>>),
    /// `play` admitted (the player `starting`): the file opened and built, the outcome told (`Player::play_done`)
    Play { file: String },
    /// a control the pipeline builds, at a value the list accepted and admitted as being built: built, stored
    /// (`Controls::follow`, its `control` event), then freed (`Running::built`, `failed` if it was not)
    Control { name: String, value: Value },
    /// `reset`'s built controls at their defaults, built as one change; `names`: every built control, admitted as being
    /// built by the reset and freed once it is
    Reset { build: Vec<(String, Value)>, names: Vec<String> },
    /// the video's owners as priced in the state built (`heap::video`)
    Account(Sender<Vec<(String, u64)>>),
    /// what the wishes would cost applied in order to a copy, with the host stream as it is (None) or replaced
    /// (Some: a stream, or none); nothing built
    Price { wishes: Vec<Wish>, host: Option<Option<HostStream>>, reply: Sender<Result<Price, String>> },
    /// a `set` or `adjust` of a picture knob or a lock, resolved against the list as the changes before it left it
    /// (refused when the list refuses it): what the auto runs read first where the change fixes it, then built with every other knob as the list holds it; stored and answered `Ok` once the ISP took it,
    /// refused if it did not (the list keeps the old value). No ISP running: stored at once, it lands with the next IQ
    /// load; a lock then is refused — there is nothing running to hold
    Knob { name: String, change: Change, by: By, reply: Reply },
    /// the USB host gone: the user's values of what its changes held, stored once the ISP took them
    /// (`List::host_gone`); nothing done when its changes hold nothing
    HostGone,
    /// `reset`'s knobs and locks back to their defaults: stored once the ISP took them (`List::reset_picture`), `failed`
    /// as `reset` if it did not — the list keeps what the ISP has
    PictureReset,
    /// what the autos run now, for a USB host's read of a live control; nothing read while no ISP runs
    Live(Sender<Now>),
    /// take one photo at this size (fitted to what the HDMI shows) into this file
    Photo { size: Size, path: PathBuf },
    /// take the camera's picture now into the menu's resource `picture`, at most `size` (`res::Resources::camera`)
    Grab { picture: PictureId, size: Size },
    /// what the scene puts into each output's frame, laid out by the `ui` thread
    Frames(Frames),
    /// what microcamd holds firm fell to the mark armed (`mi::layer::wake_at`): a layout refused for the heap is built
    /// now, and the `ui` looks again at the forms that waited for room
    Freed,
    Rebuild(Sender<Result<(), String>>),
    /// a scene came or went (`Ui::scene`): the RGN outputs carry a layer only while there is one
    UiScene,
    /// the `ui` thread handed the photo's menu over to the media layer drawn for the output as built at `generation`:
    /// every frame of the port from microcam_rgn's frame `from` on carries it; None: the photo goes without it (`Ui::photo`);
    /// the menu over the frame's pictures is in the media output's patch set `seq` (`ui::Out`)
    PhotoUi { id: u64, generation: u64, from: Option<u64>, seq: u64 },
    /// the encoder on this channel is lost (`mi::feed`)
    EncoderLost(u32),
    Status(Sender<Status>),
    /// answered once every message sent before it is handled: what must come after the pipeline's queued steps (the
    /// card handed to the host after the player stopped on its owner's loss)
    Settle(Sender<()>),
    Shutdown(Sender<()>),
}

pub struct Pipeline {
    hal: Box<dyn Hal + Send>,
    modes: ModeTable,
    policy: Policy,
    prefs: Prefs,
    monitor: Monitor,
    host: Option<(HostStream, Epoch)>,
    epoch: u64,
    revision: u64,
    current: Current,
    desired: Option<Desired>,
    state: DaemonState,
    last_error: Option<String>,
    /// what the last transition refused by the guard was short of
    short: Option<heap::Short>,
    /// the outputs the precedence has off (HDMI off, USB refused), as told
    left_out: Vec<Output>,
    /// the latest frames the `ui` sent that were refused, and microcamd's share then: built when the share moved or
    /// what microcamd holds firm fell to what they need ([`Msg::Freed`]); a newer layout replaces them
    pending_frames: Option<(Frames, u64)>,
    last_transition: Option<String>,
    transitions: u64,
    started: Instant,
    health: Arc<Health>,
    /// the capture's frame counter at the last health tick, and when that tick ran
    frames_seen: Option<u64>,
    /// the display lock's last reading of DISP's counters: pool, phase sum µs, count, last release ns
    /// ([`microcam_core::lock`])
    lock_seen: Option<(u32, u64, u64, u64)>,
    last_tick: Instant,
    /// the `ui`: told where each output is
    ui: Arc<crate::ui::Ui>,
    /// the controls: the built ones follow the wishes as built
    controls: Arc<Controls>,
    /// the knobs the ISP refused, as last told to the controls
    refused_told: std::collections::BTreeMap<Knob, String>,
    /// what runs: the photo's, the recording's and the builds' outcomes
    running: Arc<Running>,
    player: crate::player::Player,
    /// a recording's cover, once taken, for its writer
    covers: Arc<crate::recorder::Covers>,
    /// the `ui`'s layer on each RGN output's port, by [`RgnId`]
    rgn: [RgnSide; 3],
    /// the photo being taken while its menu is drawn into the media layer
    photo: Option<PendingPhoto>,
    next_photo: u64,
    /// the running recording's cover, asked once its first GOP is encoded ([`Self::cover_due`])
    cover: Option<(Instant, Size, String)>,
}

/// A photo whose port is built while the `ui` thread draws the scene of its press into the media layer.
struct PendingPhoto {
    id: u64,
    to: Shot,
    asked: Instant,
    /// the recording into this file had its cover being taken when this photo came: the cover is made from its shot
    cover: Option<String>,
}

/// Where a photo goes.
enum Shot {
    /// a file on the card, with its thumbnail (`photo`)
    File(PathBuf),
    /// the cover of the recording into this file (its first frame): its thumbnail alone, into the MP4 by the recorder
    Cover(String),
}

/// One RGN output's layer on its port and the output as last published.
struct RgnSide {
    /// None: RGN did not initialise
    layer: Option<LayerRgn>,
    /// when to step the layer next (right after a transition took it off)
    wake: Option<Instant>,
    out: Option<RgnOutput>,
    /// names the output's drawing; a new one when the drawing changes (it starts at 1)
    generation: u64,
    /// what the menu was told of the output, and why its layer cannot be put on its port
    info: Option<OutputInfo>,
    why_not: Option<String>,
}


/// How long a photo waits for its frame (the first one after the encoder starts, or after its menu is in the frames).
const PHOTO_WAIT_MS: u32 = 2000;

/// How long a photo waits for its menu to be drawn into the media layer before it fails (the backstop for a `ui`
/// thread that never tells).
const PHOTO_DRAW_WAIT: Duration = Duration::from_secs(5);

/// How often the pipeline thread checks itself and the capture for the watchdog (well inside `watchdog::STALE`).
const TICK: Duration = Duration::from_secs(5);
/// How long an outside fact waits for the `ui` to give microcamd's share back before its transition is tried again.
const GIVE_BACK_WAIT: Duration = Duration::from_secs(1);

impl Pipeline {
    #[allow(clippy::too_many_arguments)]
    pub fn new(hal: Box<dyn Hal + Send>, modes: ModeTable, policy: Policy, prefs: Prefs, monitor: Monitor, health: Arc<Health>, ui: Arc<crate::ui::Ui>, controls: Arc<Controls>, running: Arc<Running>, player: crate::player::Player, covers: Arc<crate::recorder::Covers>, rgn: bool) -> Self {
        Self {
            hal,
            modes,
            policy,
            prefs,
            monitor,
            host: None,
            epoch: 0,
            revision: 0,
            current: Current::default(),
            desired: None,
            state: DaemonState::Starting,
            last_error: None,
            short: None,
            pending_frames: None,
            left_out: Vec::new(),
            last_transition: None,
            transitions: 0,
            started: Instant::now(),
            health,
            frames_seen: None,
            lock_seen: None,
            last_tick: Instant::now(),
            ui,
            controls,
            refused_told: Default::default(),
            running,
            player,
            covers,
            photo: None,
            next_photo: 0,
            cover: None,
            rgn: [(); 3].map(|_| RgnSide { layer: rgn.then(LayerRgn::new), wake: None, out: None, generation: 1, info: None, why_not: None }),
        }
    }

    /// The thread body. Returns when `Shutdown` arrives (pipeline torn down) or every sender is gone.
    pub fn run(mut self, rx: Receiver<Msg>) {
        // first build from the inputs known at start
        let _ = self.reconcile("start");
        self.publish_ui();
        self.controls.follow(&self.prefs, &self.monitor);
        loop {
            let photo = self.photo.as_ref().map(|p| p.asked + PHOTO_DRAW_WAIT);
            let cover = self.cover.as_ref().map(|c| c.0);
            let wait = self.rgn.iter().filter_map(|r| r.wake).chain(photo).chain(cover).map(|w| w.saturating_duration_since(Instant::now())).fold(TICK, Duration::min);
            let msg = match rx.recv_timeout(wait) {
                Ok(m) => Some(m),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => break,
            };
            if self.last_tick.elapsed() >= TICK {
                self.health_tick();
                // layers that failed to come or go are retried here too
                self.step_layers();
            }
            if self.rgn.iter().any(|r| r.wake.is_some_and(|w| Instant::now() >= w)) {
                self.step_layers();
            }
            if self.photo.as_ref().is_some_and(|p| p.asked.elapsed() >= PHOTO_DRAW_WAIT) {
                let e = format!("its menu was not drawn in {} ms", PHOTO_DRAW_WAIT.as_millis());
                log::error!("photo failed: {e}");
                if let Some(p) = self.photo_end() {
                    self.shot_failed(&p, Refusal::new(Code::Refused, e));
                }
                self.publish_ui();
            }
            if self.cover.as_ref().is_some_and(|c| Instant::now() >= c.0) {
                self.cover_due();
                self.publish_ui();
            }
            let Some(msg) = msg else { continue };
            self.handle(msg);
            if self.state == DaemonState::Stopping {
                return;
            }
            // a refused layout is built once the share it was refused beside moved
            if let Some((frames, _)) = self.pending_frames.take_if(|(_, at)| *at != mi::layer::share()) {
                self.frames(frames);
            }
            self.publish_ui();
            self.controls.follow(&self.prefs, &self.monitor);
        }
        self.teardown("control channel closed");
    }

    /// Where the RGN outputs and their live pictures are, as built, for the `ui`.
    fn publish_ui(&mut self) {
        self.ui.video_holders(self.video_holders());
        for id in RgnId::ALL {
            let now = match id {
                RgnId::Usb => self.usb_rgn_now(),
                RgnId::Media => self.media_rgn_now(),
                RgnId::Hdmi => self.hdmi_rgn_now(),
            };
            self.publish_rgn(id, now);
        }
        self.step_layers();
    }

    fn publish_rgn(&mut self, id: RgnId, now: Result<Option<RgnOutput>, String>) {
        // the output as the menu is told of it: from the plan built, whether its layer can be put on its port or not
        let info = match &now {
            Ok(Some(o)) => Some(OutputInfo { name: id.output(), w: o.ui.raster.w, h: o.ui.raster.h, level: o.ui.level, paused: o.ui.paused, timing: o.timing, codec: o.codec, fps: o.codec.map(|_| o.fps) }),
            Ok(None) => None,
            Err(_) => self.output_info(id),
        };
        let (o, why_not) = match now {
            Ok(o) => (o, None),
            Err(e) => (None, Some(e)),
        };
        let side = &mut self.rgn[id as usize];
        if o == side.out && info == side.info && why_not == side.why_not {
            return;
        }
        side.info = info.clone();
        side.why_not = why_not.clone();
        if !matches!((&side.out, &o), (Some(a), Some(b)) if a.same_drawing(b)) {
            side.generation += 1;
        }
        side.out = o.clone();
        if let Some(l) = side.layer.as_mut() {
            if l.must_go(o.clone()) {
                let_go(&self.ui, id, l);
            }
            l.set_output(side.generation, o.clone());
        }
        match (&o, why_not.clone()) {
            (Some(RgnOutput { port: Some(p), area, ui, .. }), _) => log::info!("{} ui: on SCL {}/{}/{}, {} of a {} frame", id.name(), p.dev, p.chn, p.port, area.size(), ui.raster),
            (Some(o), _) => log::info!("{} ui: covered, the menu as patches in the {} frame", id.name(), o.ui.raster),
            (None, Some(e)) => log::warn!("{} ui: none: {e}", id.name()),
            (None, None) => log::info!("{} ui: no output", id.name()),
        }
        let generation = side.generation;
        self.ui.rgn(id, o.map(|o| (generation, o)), info, why_not);
    }

    /// What the menu is told of `id` when its layer cannot be put on its port: the plan built, as `status` has it.
    fn output_info(&self, id: RgnId) -> Option<OutputInfo> {
        let d = self.current.desired.as_ref()?;
        let levels = &self.prefs.levels;
        match id {
            RgnId::Hdmi => d.hdmi.as_ref().map(|h| OutputInfo { name: Output::Hdmi, w: h.timing.size().w, h: h.timing.size().h, level: levels.hdmi, paused: h.paused, timing: Some(h.timing), codec: None, fps: None }),
            RgnId::Usb => d.usb.as_ref().map(|u| OutputInfo { name: Output::Usb, w: u.size.w, h: u.size.h, level: levels.usb, paused: u.paused, timing: None, codec: Some(u.codec), fps: Some(u.fps) }),
            RgnId::Media => d.media.as_ref().map(|m| OutputInfo { name: Output::Media, w: m.size.w, h: m.size.h, level: levels.media, paused: None, timing: None, codec: None, fps: None }),
        }
    }

    /// The USB output as built, for the `ui`: the port whose frames the feed hands the encoder in its canvases, at the
    /// part of the frame it writes; covered, no port; reading HDMI's canvases (joined), HDMI's layer and patches.
    /// Ok(None): no USB output; Err: one the regions cannot be put on.
    fn usb_rgn_now(&self) -> Result<Option<RgnOutput>, String> {
        let Some(u) = self.current.desired.as_ref().and_then(|d| d.usb.as_ref()) else { return Ok(None) };
        let ui = OutputUi { raster: u.size, live: u.frame.window, level: self.prefs.levels.usb, paused: u.paused };
        if self.current.usb_joined() {
            let visible = self.current.feed(reconcile::topo::DISP_IN).map(|f| visible(&f.way)).unwrap_or_default();
            return Ok(Some(RgnOutput { ui, port: None, area: Rect::full(u.size), fps: u.fps, timing: None, codec: Some(u.codec), joined: Some(reconcile::topo::POOL_HDMI), visible }));
        }
        let Some(f) = self.current.feed_into(reconcile::topo::VENC_CHN_USB) else { return Ok(None) };
        let reconcile::FeedWay::Canvas { view, .. } = &f.way;
        let visible = visible(&f.way);
        let Some(area) = *view else { return Ok(Some(RgnOutput { ui, port: None, area: Rect::full(u.size), fps: u.fps, timing: None, codec: Some(u.codec), joined: None, visible })) };
        self.fits(f.src, area)?;
        Ok(Some(RgnOutput { ui, port: Some(f.src), area, fps: u.fps, timing: None, codec: Some(u.codec), joined: None, visible }))
    }

    /// The HDMI output as built, for the `ui`: as USB's — the port (SCL port 0, or HDMI's upscaler when zoomed) whose
    /// frames the feed hands DISP in its canvases, at the part of the screen it writes; covered, no port.
    fn hdmi_rgn_now(&self) -> Result<Option<RgnOutput>, String> {
        let (Some(h), Some(f)) = (self.current.desired.as_ref().and_then(|d| d.hdmi.as_ref()), self.current.feed(reconcile::topo::DISP_IN)) else { return Ok(None) };
        let raster = h.timing.size();
        let ui = OutputUi { raster, live: h.frame.window, level: self.prefs.levels.hdmi, paused: h.paused };
        let reconcile::FeedWay::Canvas { view, .. } = &f.way;
        let visible = visible(&f.way);
        let Some(area) = *view else { return Ok(Some(RgnOutput { ui, port: None, area: Rect::full(raster), fps: f.fps, timing: Some(h.timing), codec: None, joined: None, visible })) };
        self.fits(f.src, area)?;
        Ok(Some(RgnOutput { ui, port: Some(f.src), area, fps: f.fps, timing: Some(h.timing), codec: None, joined: None, visible }))
    }

    /// The media encoder's RGN target as built: the port its feed takes frames from, or the port a photo with a menu
    /// is taken from (its encoder has no feed); the whole frame live. A recording reading another output's canvases
    /// (its frame is that output's): that output's layer and patches.
    fn media_rgn_now(&self) -> Result<Option<RgnOutput>, String> {
        let Some(m) = self.current.desired.as_ref().and_then(|d| d.media.as_ref()) else { return Ok(None) };
        if let Some(pool) = self.current.media_joined() {
            let ui = OutputUi { raster: m.size, live: m.frame.window, level: self.prefs.levels.media, paused: None };
            let feed = self.current.feeds.iter().find(|f| matches!(&f.way, reconcile::FeedWay::Canvas { pool: p, .. } if *p == pool));
            let visible = feed.map(|f| visible(&f.way)).unwrap_or_default();
            return Ok(Some(RgnOutput { ui, port: None, area: Rect::full(m.size), fps: m.fps, timing: None, codec: None, joined: Some(pool), visible }));
        }
        // a photo's port has no feed: its frame as planned
        let (port, visible) = match self.current.feed_into(reconcile::topo::VENC_CHN_MEDIA) {
            Some(f) => (f.src, visible(&f.way)),
            None if m.shot() && self.photo.is_some() && self.current.media.is_some() => (self.media_src(), m.frame.visible.clone()),
            None => return Ok(None),
        };
        let ui = OutputUi { raster: m.size, live: m.frame.window, level: self.prefs.levels.media, paused: None };
        // covered: the port is off, the whole menu goes into the patches
        if m.frame.way == microcam_core::frame::Way::Covered {
            return Ok(Some(RgnOutput { ui, port: None, area: Rect::full(m.size), fps: m.fps, timing: None, codec: None, joined: None, visible }));
        }
        let area = m.frame.view;
        self.fits(port, area)?;
        Ok(Some(RgnOutput { ui, port: Some(port), area, fps: m.fps, timing: None, codec: None, joined: None, visible }))
    }

    /// The layer sits at the port's own pixels: its frames must be `area` of the output frame one to one.
    fn fits(&self, port: reconcile::ChnPort, area: Rect) -> Result<(), String> {
        let size = self.current.port_size(port);
        if size != Some(area.size()) {
            return Err(format!("SCL {}/{}/{} hands on {size:?}, not the {} it lands in", port.dev, port.chn, port.port, area.size()));
        }
        Ok(())
    }

    /// Every RGN output carries a layer while it is built, shows something at its level and there is a scene; the `ui`
    /// thread is told of a new one and lets go of one before it goes.
    fn step_layers(&mut self) {
        // the 2D engine lost: no layer is made, cleaned or destroyed again (it may still be writing one)
        if mi::gfx::lost() {
            return;
        }
        let scene = self.ui.has_scene();
        // a photo's menu is the scene of its press, held by the `ui` thread
        let photo = self.photo.is_some();
        for id in RgnId::ALL {
            let level = match id {
                RgnId::Usb => self.prefs.levels.usb,
                RgnId::Media => self.prefs.levels.media,
                RgnId::Hdmi => self.prefs.levels.hdmi,
            };
            let side = &mut self.rgn[id as usize];
            let Some(l) = side.layer.as_mut() else { continue };
            let want = side.out.is_some() && level > 0 && (scene || (id == RgnId::Media && photo));
            if !want && l.shown().is_some() {
                let_go(&self.ui, id, l);
            }
            if let Some(a) = l.step(want) {
                if let Some(p) = a.out.port {
                    log::info!("{} ui: layer on SCL {}/{}/{}, {}", id.name(), p.dev, p.chn, p.port, a.out.area.size());
                }
                self.ui.attached(id, a);
            }
            side.wake = None;
        }
    }

    fn handle(&mut self, msg: Msg) {
        {
            // what a refusal is short of is this message's: one an earlier transition met says nothing of it
            self.short = None;
            match msg {
                Msg::Monitor(m) => {
                    if m != self.monitor {
                        log::info!("monitor: {}", describe_monitor(&m));
                        self.monitor = m;
                        if let Err(e) = self.ask("monitor", self.prefs.clone(), Some(Who::Hdmi)) {
                            self.ui.followers.tell(&Event::failed(Action::Output(Output::Hdmi), self.refusal(e)));
                        }
                    }
                }
                Msg::StreamOn(h, reply) => {
                    self.epoch += 1;
                    let epoch = Epoch(self.epoch);
                    log::info!("host stream on: {:?} {} @{} (epoch {})", h.codec, h.size, h.fps, epoch.0);
                    self.host = Some((h, epoch));
                    let r = self.ask("stream on", self.prefs.clone(), Some(Who::Usb));
                    // a stream that cannot be served is answered without frames (uvc), so it is no fact to plan with
                    if let Err(e) = &r {
                        self.host = None;
                        self.ui.followers.tell(&Event::failed(Action::Output(Output::Usb), self.refusal(e.clone())));
                    }
                    let _ = reply.send(r.map(|_| epoch));
                }
                Msg::UsbRate(h, reply) => {
                    let tuning = self.controls.tuning();
                    let inputs = Inputs { monitor: &self.monitor, host: Some((&h, Epoch(0))), prefs: &self.prefs, tuning: &tuning };
                    let fps = self.policy.plan(&inputs, &self.modes, Revision(self.revision)).ok().and_then(|d| d.usb).map(|u| u.fps);
                    let _ = reply.send(fps);
                }
                Msg::StreamOff(reply) => {
                    if self.host.take().is_some() {
                        log::info!("host stream off");
                        let _ = self.reconcile("stream off");
                    }
                    let _ = reply.send(());
                }
                Msg::Wish(w, reply) => {
                    let r = self.wish(w.clone()).map_err(|e| self.refusal(e));
                    match (&w, &r) {
                        (Wish::Record(rec), _) => {
                            self.running.record_built(r.clone());
                            // its cover: a photo once its first GOP is encoded — the encoder's slowest frames (its rate
                            // control's start, `venc::START_QP`) with the photo holding a canvas of the same pool beside
                            // them lose a frame — of a picture settled since the sensor's restart
                            if r.is_ok() {
                                let gop = self.current.desired.as_ref().and_then(|d| d.media.as_ref()).map_or(Duration::from_secs(1), |m| Duration::from_secs_f64(m.gop as f64 / m.fps.max(1) as f64));
                                self.cover = Some((Instant::now() + gop, rec.size, rec.path.clone()));
                            }
                        }
                        (Wish::RecordStop, Ok(())) => self.running.record_stopped(),
                        (Wish::RecordStop, Err(e)) => self.running.record_stop_failed(e.error.clone()),
                        (Wish::PlayStop, _) => self.player.stop_done(r.clone()),
                        _ => {}
                    }
                    if let Some(reply) = reply {
                        let _ = reply.send(r.map_err(|r| r.error));
                    }
                }
                Msg::Play { file } => {
                    let r = self.player.open(&file).and_then(|pb| self.wish(Wish::Play(pb))).map_err(|e| self.refusal(e));
                    self.player.play_done(r);
                }
                Msg::Account(reply) => {
                    let _ = reply.send(heap::video(&self.current).owners);
                }
                Msg::Price { wishes, host, reply } => {
                    let _ = reply.send(self.price(&wishes, host));
                }
                Msg::Control { name, value } => {
                    let r = self.wish(Wish::Control(name.clone(), value)).map_err(|e| self.refusal(e));
                    // stored and told before the build is freed
                    self.controls.follow(&self.prefs, &self.monitor);
                    if r.is_ok() {
                        self.controls.chosen(&name);
                    }
                    self.running.built(Action::Control(name.clone()), std::slice::from_ref(&name), r);
                }
                Msg::Reset { build, names } => {
                    let mut p = self.prefs.clone();
                    let r = build.iter().try_for_each(|(n, v)| controls::apply(n, v, &mut p)).and_then(|()| self.ask("reset", p, None)).map_err(|e| self.refusal(e));
                    self.controls.follow(&self.prefs, &self.monitor);
                    self.controls.save();
                    self.running.built(Action::Reset, &names, r);
                }
                Msg::Knob { name, change, by, reply } => {
                    let r = self.knob(&name, change, by);
                    reply(r);
                }
                Msg::HostGone if self.controls.host_holds() => match self.picture("the USB host's going", List::host_gone) {
                    Ok(()) => log::info!("controls: the USB host gone, the user's values back"),
                    Err(r) => log::error!("controls: {}", r.error),
                },
                Msg::HostGone => {}
                Msg::PictureReset => match self.picture("reset", List::reset_picture) {
                    Ok(()) => self.controls.save(),
                    Err(r) => self.ui.followers.tell(&Event::failed(Action::Reset, r)),
                },
                Msg::Live(reply) => {
                    let isp = self.current.isp.started;
                    let now = Now { exposure: isp.then(|| self.hal.exposure_now()).flatten(), white_balance: isp.then(|| self.hal.white_balance_now()).flatten() };
                    let _ = reply.send(now);
                }
                Msg::Photo { size, path } => self.photo(size, Shot::File(path)),
                Msg::PhotoUi { id, generation, from, seq } => self.photo_drawn(id, generation, from, seq),
                Msg::Grab { picture, size } => self.grab(picture, size),
                Msg::Frames(frames) => {
                    self.pending_frames = None;
                    self.frames(frames);
                }
                Msg::Freed => {
                    if let Some((frames, _)) = self.pending_frames.take() {
                        self.frames(frames);
                    }
                    self.ui.notice();
                }
                Msg::Rebuild(reply) => {
                    self.teardown("rebuild requested");
                    let _ = reply.send(self.reconcile("rebuild"));
                }
                Msg::Status(reply) => {
                    let _ = reply.send(self.status());
                }
                Msg::Settle(reply) => {
                    let _ = reply.send(());
                }
                Msg::UiScene => self.step_layers(),
                // the lost encoder's feed may be gone already (a transition took it down)
                Msg::EncoderLost(chn) if self.hal.encoder_lost(chn) => self.recover(chn),
                Msg::EncoderLost(_) => {}
                Msg::Shutdown(reply) => {
                    self.state = DaemonState::Stopping;
                    self.teardown("shutdown");
                    let _ = reply.send(());
                }
            }
        }
        self.tell_refused();
    }

    /// A picture knob or a lock changed `by` the user or a USB host ([`Msg::Knob`]): the value it asks of the list now,
    /// stored once the ISP took it, at once while no ISP runs.
    fn knob(&mut self, name: &str, change: Change, by: By) -> Result<Value, Refusal> {
        let v = self.controls.target(name, change)?;
        let refused = |why: String| Refusal::new(Code::Refused, format!("{name}: {why}"));
        let (exposure, white_balance) = self.controls.reads(name, &v);
        let now = if self.current.isp.started {
            Now {
                exposure: if exposure { Some(self.hal.exposure_now().ok_or_else(|| refused("the exposure the auto runs could not be read".into()))?) } else { None },
                white_balance: if white_balance { Some(self.hal.white_balance_now().ok_or_else(|| refused("the white balance the auto runs could not be read".into()))?) } else { None },
            }
        } else if matches!(self.controls.drive(name), Some(Drive::ExposureLock | Drive::WhiteBalanceLock)) && (exposure || white_balance) {
            return Err(refused("no picture is running: nothing to hold".into()));
        } else {
            Now::default()
        };
        self.picture(name, |l| l.store(name, v.clone(), by, &now))?;
        Ok(v)
    }

    /// The picture's controls changed by `edit` (`what` asked it): built with every knob as the list holds them after it
    /// and stored once the ISP took it; stored at once while no ISP runs, it lands with the next IQ load.
    fn picture(&mut self, what: &str, edit: impl Fn(&mut List) -> Vec<String>) -> Result<(), Refusal> {
        if self.current.isp.started {
            let tuning = self.controls.tuning_with(&edit);
            let before = self.current.isp.refused.clone();
            let prefs = self.prefs.clone();
            self.prefs.giving = self.transition("picture", &prefs, &tuning, false).map_err(|e| self.refusal(e))?;
            // what the ISP refused of this change is no record of it: the list keeps the old values, a retry asks again
            let new: Vec<Knob> = self.current.isp.refused.iter().filter(|(k, r)| before.get(k) != Some(r)).map(|(k, _)| *k).collect();
            if let Some(why) = self.current.isp.refusal(&new).map(String::from) {
                for k in &new {
                    self.current.isp.refused.remove(k);
                }
                return Err(Refusal::new(Code::Refused, format!("{what}: the ISP refused it: {why}")));
            }
        }
        self.controls.edit(edit);
        Ok(())
    }

    /// The knobs the ISP refused since its last IQ load told to the controls when they changed.
    fn tell_refused(&mut self) {
        let now: std::collections::BTreeMap<Knob, String> = self.current.isp.refused.iter().map(|(k, (_, why))| (*k, why.clone())).collect();
        if now != self.refused_told {
            self.controls.refused(now.clone());
            self.refused_told = now;
        }
    }

    /// The `ui`'s frames built; refused, they are kept and built again when the share moves or what microcamd holds
    /// firm falls to what they need (a newer layout replaces them); the menu hears the refusal from the `ui`.
    fn frames(&mut self, frames: Frames) {
        if self.prefs.frames == frames {
            return;
        }
        let p = Prefs { frames: frames.clone(), ..self.prefs.clone() };
        if let Err(e) = self.ask("frame layout", p, None) {
            // short of the heap: what microcamd may hold firm for them to fit at their peak
            if let Some(s) = self.short {
                mi::layer::wake_at(heap::share_beside(s.needs));
            }
            self.pending_frames = Some((frames, mi::layer::share()));
            self.ui.frames_refused(self.refusal(e));
        }
    }

    /// The encoder on channel `chn` is lost: it goes down with its feed and drain — steps that fail on what is already
    /// gone (the encoder itself, when another process took its device down) count it as gone — and is built again; the
    /// other outputs and the capture stay.
    fn recover(&mut self, chn: u32) {
        if self.current.desired.is_none() {
            return;
        }
        let ops = reconcile::teardown_encoder(&self.current, chn);
        log::warn!("encoder {chn} lost; taking it down ({} ops)", ops.len());
        self.take_down(&ops);
        self.current.desired = None;
        let _ = self.reconcile("encoder recovery");
    }

    /// The port the media encoder's frames come from: SCL port 1, or its upscaler when zoomed.
    fn media_src(&self) -> reconcile::ChnPort {
        if self.current.scl_media_up.channel {
            reconcile::topo::SCL_MEDIA_UP
        } else {
            reconcile::topo::SCL_OUT_MEDIA
        }
    }

    /// One frame of the media port through the media encoder built for a photo (`from`: see [`Msg::PhotoUi`]); during a
    /// recording, a finished frame of the recording through the photo's own JPEG encoder beside it (at the recording's
    /// size), handed it by the compositor of the pool the recording reads. Returns the JPEG and its size.
    fn shoot(&mut self, from: Option<u64>, seq: u64) -> Result<(Vec<u8>, Size), String> {
        use reconcile::topo::{venc_in, VENC_CHN_SNAP, VENC_DEV_JPEG};
        match (self.current.media.clone(), self.current.snap.is_some()) {
            (Some(v), true) => {
                // the feed whose compositor hands the canvases of the pool the photo's encoder reads
                let pool = self.current.snap_pool().ok_or("the photo's encoder reads no pool")?;
                let feed = self.current.feeds.iter().find(|f| matches!(&f.way, reconcile::FeedWay::Canvas { pool: p, .. } if *p == pool)).map(|f| f.dst).ok_or("no feed hands the recording's canvases")?;
                self.hal.snapshot(self.media_src(), feed, venc_in(VENC_DEV_JPEG, VENC_CHN_SNAP), from, seq, PHOTO_WAIT_MS).map(|b| (b, v.size)).map_err(|e| e.to_string())
            }
            (Some(v), false) => {
                let way = self.current.desired.as_ref().and_then(|d| d.media.as_ref()).map(reconcile::media_way).ok_or("no media plan")?;
                self.hal.shoot(self.media_src(), v.dev, v.chn, v.codec, v.fps, &way, from, seq, PHOTO_WAIT_MS).map(|b| (b, v.size)).map_err(|e| e.to_string())
            }
            (None, _) => Err("no media encoder after the build".into()),
        }
    }

    /// A photo of `size` with no menu: the media chain built for it, its newest frame taken; the next reconcile takes
    /// the chain down again.
    fn grab_clean(&mut self, size: Size) -> Result<(Vec<u8>, Size), Refusal> {
        let p = Prefs { photo: Some(size), ..self.prefs.clone() };
        self.ask("photo", p, Some(Who::Photo)).map_err(|e| self.refusal(e)).and_then(|_| self.shoot(None, 0).map_err(|e| Refusal::new(Code::Refused, e)))
    }

    /// A photo. With a scene, the media chain is built for it with the press's scene in its layer and its frame's
    /// pictures, and the photo is taken once the `ui` thread has them in the frames ([`Self::photo_drawn`]); otherwise
    /// at once.
    /// The running recording's cover asked: from the photo being taken when one is (as a photo pressed during the cover);
    /// none when the recording it was for has ended.
    fn cover_due(&mut self) {
        let Some((_, size, path)) = self.cover.take() else { return };
        if self.prefs.recording.as_ref().is_none_or(|r| r.path != path) {
            return;
        }
        if let Some(pending) = self.photo.as_mut().filter(|p| matches!(p.to, Shot::File(_))) {
            log::info!("photo {}: the cover of {path} made from it", pending.id);
            pending.cover = Some(path);
            return;
        }
        self.photo(size, Shot::Cover(path));
    }

    fn photo(&mut self, size: Size, to: Shot) {
        if self.prefs.recording.is_some() {
            log::info!("photo: during a recording, a frame of the recording at its size (asked {size})");
        }
        self.next_photo += 1;
        let p = PendingPhoto { id: self.next_photo, to, asked: Instant::now(), cover: None };
        // the media output's frame with the player in it is not designed
        if self.prefs.player.is_some() {
            log::warn!("photo: a file is playing; this press is dropped");
            return self.shot_failed(&p, Refusal::new(Code::Playing, "a file is playing"));
        }
        // a photo (admitted by `Running`, one at a time) during a recording's cover takes the cover's place, its shot
        // the recording's frame either way: the cover is made from it
        if let (Some(pending), Shot::File(_)) = (self.photo.as_mut(), &p.to) {
            if let Shot::Cover(rec) = &pending.to {
                log::info!("photo {}: in place of the cover of {rec} being taken, the cover made from it", pending.id);
                pending.cover = Some(rec.clone());
                pending.to = p.to;
                return;
            }
        }
        // one at a time; not met: `Running` admits one photo at a time, and a recording gives up the photo being taken
        // before its cover is asked
        if self.photo.is_some() {
            log::warn!("photo: the previous one (or a recording's cover) is still being taken; this one is dropped");
            return self.shot_failed(&p, Refusal::new(Code::Busy, "a photo or a recording's cover is being taken"));
        }
        // with a scene even at media level 0: the frame's pictures shown there go into the photo
        let Some((revision, scene)) = self.ui.scene_now() else {
            let jpeg = self.grab_clean(size);
            self.no_photo_wish("photo done");
            return self.photo_taken(p, jpeg);
        };
        log::info!("photo {}: the scene at revision {revision}", p.id);
        self.ui.photo(crate::ui::PhotoJob { id: p.id, scene });
        let id = p.id;
        self.photo = Some(p);
        // the media output is published once this message is handled; its layer drawn, the `ui` thread tells
        let wish = Prefs { photo: Some(size), ..self.prefs.clone() };
        if let Err(e) = self.ask("photo", wish, Some(Who::Photo)) {
            log::error!("photo failed: {e}");
            let r = self.refusal(e);
            if let Some(p) = self.photo_end() {
                self.shot_failed(&p, r);
            }
            return;
        }
        // a recording reading another output's frame has no layer of its own to draw the press's scene in: the photo
        // is the recording's next frame, as that output shows it
        if self.current.media_joined().is_some() {
            log::info!("photo {id}: the recording reads another output's frame; its next one taken");
            self.photo_drawn(id, self.rgn[RgnId::Media as usize].generation, None, 0);
        }
    }

    /// The photo's media chain is wished away (taken down), if it is wished.
    fn no_photo_wish(&mut self, why: &str) {
        if self.prefs.photo.is_some() {
            let p = Prefs { photo: None, ..self.prefs.clone() };
            let _ = self.ask(why, p, Some(self.outputs_first()));
        }
    }

    /// The photo's menu is in the media layer: the photo is taken with it, its encoder torn down right after. A media
    /// output built anew meanwhile (a zoom) gets the menu drawn again, and the `ui` thread tells again.
    fn photo_drawn(&mut self, id: u64, generation: u64, from: Option<u64>, seq: u64) {
        if self.photo.as_ref().is_none_or(|p| p.id != id) {
            return;
        }
        if from.is_some() && generation != self.rgn[RgnId::Media as usize].generation {
            return;
        }
        let drawn = self.photo.as_ref().unwrap().asked.elapsed();
        if from.is_none() {
            log::info!("photo: no menu in its frames (none at its level, or it could not be drawn); the newest taken");
        }
        let jpeg = self.shoot(from, seq).map_err(|e| Refusal::new(Code::Refused, e));
        let p = self.photo_end().unwrap();
        log::debug!("photo: menu in the frames after {} ms", drawn.as_millis());
        self.photo_taken(p, jpeg);
    }

    /// The photo taken or given up: its chain goes, the media layer follows the scene again.
    fn photo_end(&mut self) -> Option<PendingPhoto> {
        let p = self.photo.take();
        self.no_photo_wish("photo done");
        self.ui.photo_done();
        p
    }

    /// A photo that did not get there: a file's told as its outcome, a cover's logged (the video has none).
    fn shot_failed(&self, p: &PendingPhoto, r: Refusal) {
        if let Some(rec) = &p.cover {
            log::warn!("cover of {rec}: {}", r.error);
        }
        match &p.to {
            Shot::File(_) => self.running.photo_done(Err(r)),
            Shot::Cover(rec) => log::warn!("cover of {rec}: {}", r.error),
        }
    }

    /// The file is written in the background so the video goes on at once; its outcome told when written. A cover's
    /// thumbnail is made in the background too and handed to the recording's writer.
    fn photo_taken(&self, p: PendingPhoto, jpeg: Result<(Vec<u8>, Size), Refusal>) {
        let running = self.running.clone();
        match (p.cover, &jpeg) {
            (Some(rec), Ok((bytes, _))) => self.cover(rec, bytes.clone(), p.asked),
            (Some(rec), Err(e)) => log::warn!("cover of {rec}: {}", e.error),
            (None, _) => {}
        }
        let (path, jpeg) = match (p.to, jpeg) {
            (Shot::File(path), jpeg) => (path, jpeg),
            (Shot::Cover(rec), Ok((bytes, _))) => return self.cover(rec, bytes, p.asked),
            (Shot::Cover(rec), Err(e)) => return log::warn!("cover of {rec}: {}", e.error),
        };
        match jpeg {
            Ok((bytes, px)) => {
                log::info!("photo {} {px} {} KB in {} ms → {}", p.id, bytes.len() / 1024, p.asked.elapsed().as_millis(), path.display());
                let writer = running.clone();
                let spawned = std::thread::Builder::new().name("photo-write".into()).spawn(move || {
                    // with its thumbnail, or as the encoder made it
                    let t0 = Instant::now();
                    let bytes = crate::res::with_thumbnail(&bytes)
                        .inspect(|b| log::debug!("photo: thumbnail of {} KB in {} ms", (b.len() - bytes.len()) >> 10, t0.elapsed().as_millis()))
                        .unwrap_or_else(|e| {
                            log::warn!("photo {}: no thumbnail: {e}", path.display());
                            bytes
                        });
                    match write_file(&path, &bytes) {
                        Ok(()) => {
                            log::info!("photo written: {}", path.display());
                            writer.photo_done(Ok(()));
                        }
                        Err(e) => {
                            log::error!("photo {}: {e}", path.display());
                            writer.photo_done(Err(Refusal::new(Code::Refused, e)));
                        }
                    }
                });
                if let Err(e) = spawned {
                    running.photo_done(Err(Refusal::new(Code::Refused, format!("no thread to write it: {e}"))));
                }
            }
            Err(r) => {
                log::error!("photo failed: {}", r.error);
                running.photo_done(Err(r));
            }
        }
    }

    /// The cover of the recording into `rec` made from a shot's JPEG in the background, for its writer.
    fn cover(&self, rec: String, bytes: Vec<u8>, t0: Instant) {
        let covers = self.covers.clone();
        let made = std::thread::Builder::new().name("cover".into()).spawn(move || match crate::res::thumbnail(&bytes, crate::res::COVER_W) {
            Ok(thumb) => {
                log::info!("cover of {rec}: {} KB in {} ms", thumb.len() >> 10, t0.elapsed().as_millis());
                covers.put(&rec, thumb);
            }
            Err(e) => log::warn!("cover of {rec}: no thumbnail: {e}"),
        });
        if let Err(e) = made {
            log::warn!("cover: no thread to make it: {e}");
        }
    }

    /// The camera's picture now into the menu's resource `picture`: the media chain built as for a photo of `size`
    /// (fitted to the zoom window, its width on 16 for the scaler that copies it), its newest frame kept; the next
    /// reconcile takes the chain down again. Not during a recording or a photo (the media port is theirs).
    fn grab(&mut self, picture: PictureId, size: Size) {
        let res = self.ui.res.clone();
        // the recording outranks the freeze: its media port is no room for the picture, said as lack of resources
        if let Some(rec) = &self.prefs.recording {
            let holder = Holder { what: Held::Recording { w: rec.size.w, h: rec.size.h }, bytes: None };
            res.grab_failed(picture, ResCode::Resources, "the recording holds the media port".into(), vec![holder]);
            return;
        }
        if self.photo.is_some() {
            res.grab_failed(picture, ResCode::Busy, "the media port is busy with a photo".into(), Vec::new());
            return;
        }
        // the picture's room in microcamd's share, asked of the book before its chain is built: refused for lack of
        // resources when it cannot have it
        let need = (size.w & !15) as u64 * (size.h & !1) as u64 * 3 / 2;
        if !mi::layer::room(need) {
            let error = format!("the camera picture: {} KiB of microcamd's share wanted, {} KiB of {} KiB held", need >> 10, mi::layer::held() >> 10, mi::layer::share() >> 10);
            log::warn!("camera picture {}: {error}", picture.0);
            res.grab_failed(picture, ResCode::Resources, error, self.ui.holders());
            return;
        }
        let t0 = Instant::now();
        let wish = Prefs { photo: Some(size), ..self.prefs.clone() };
        let r = self.ask("camera picture", wish, Some(Who::Picture)).and_then(|_| {
            let m = self.current.media.clone().ok_or("no media output after the build")?;
            let size = Size::new(m.size.w & !15, m.size.h & !1);
            self.hal.grab(self.media_src(), size, picture, self.current.scl_stretch.device, PHOTO_WAIT_MS).map_err(|e| e.to_string())
        });
        // refused for the heap: what holds it, read before the chain goes
        let r = r.map_err(|e| match self.refusal(e) {
            r if r.code == Code::Resources => (ResCode::Resources, r),
            r => (ResCode::Camera, r),
        });
        self.no_photo_wish("camera picture done");
        match r {
            Ok(()) => log::info!("camera picture {} taken in {} ms", picture.0, t0.elapsed().as_millis()),
            Err((code, r)) => {
                log::error!("camera picture {}: {}", picture.0, r.error);
                res.grab_failed(picture, code, r.error, r.holders);
            }
        }
    }

    /// A wish applied to a copy of the wishes and built; kept once built ([`Self::want`]). The outputs follow when the
    /// `ui` is published after the message.
    fn wish(&mut self, w: Wish) -> Result<(), String> {
        w.apply(&mut self.prefs.clone()).inspect_err(|e| log::warn!("{}: {e}", w.name()))?;
        // a photo being taken holds the media port a recording takes: given up when the precedence puts the recording
        // above it, else the recording waits for it; the frame's pictures wait (policy)
        if matches!(w, Wish::Record(_)) && self.photo.is_some() {
            if !self.prefs.precedence.above(Who::Recording, Who::Photo) {
                return Err("a photo is being taken (it comes before a recording in the precedence)".into());
            }
            log::warn!("photo: a recording starts; no photo");
            if let Some(p) = self.photo_end() {
                self.shot_failed(&p, Refusal::new(Code::Recording, "a recording started"));
            }
        }
        let mut p = self.prefs.clone();
        w.apply(&mut p)?;
        match w {
            // ending what the video serves is never refused for the heap
            Wish::RecordStop | Wish::PlayStop => self.stop(w.name(), p),
            Wish::Record(_) => self.ask(w.name(), p, Some(Who::Recording)).inspect_err(|e| log::error!("record: {e}")),
            Wish::Play(_) => self.ask(w.name(), p, Some(Who::Player)),
            _ => self.ask(w.name(), p, None),
        }
    }

    /// What `wishes` would cost from the state built, `host` replacing the host stream when given: the video after
    /// them, the transition's worst step and microcamd's share after; refused when it would be. Nothing is built.
    fn price(&self, wishes: &[Wish], host: Option<Option<HostStream>>) -> Result<Price, String> {
        let mut p = self.prefs.clone();
        for w in wishes {
            w.apply(&mut p).map_err(|e| format!("{}: {e}", w.name()))?;
        }
        // who asks: a recording, the player, a host's stream; nobody for anything else
        let asker = match (wishes.last(), &host) {
            (_, Some(Some(_))) => Some(Who::Usb),
            (Some(Wish::Record(_)), _) => Some(Who::Recording),
            (Some(Wish::Play(_)), _) => Some(Who::Player),
            _ => None,
        };
        let host = match host {
            None => self.host.clone(),
            Some(h) => h.map(|h| (h, Epoch(self.epoch + 1))),
        };
        let host = host.as_ref().map(|(h, e)| (h, *e));
        let tuning = self.controls.tuning();
        let firm = mi::layer::firm();
        let (_, mut plan, _) = self.planned(&p, host, &tuning, Revision(self.revision + 1), firm)?;
        let mut short = heap::admit(&self.current, &plan.ops, firm).err();
        let mut gives = Vec::new();
        // short: what the precedence would have give way, priced as built with it
        if let Some(way) = short.and_then(|_| self.way_for(&p, host, asker)) {
            let (given, freed) = given(&p, &way.steps, self.menu_bytes());
            plan = self.planned(&given, host, &tuning, Revision(self.revision + 1), firm.saturating_sub(freed))?.1;
            short = None;
            gives = way.steps.into_iter().map(|(holder, step)| Gives { holder, step }).collect();
        }
        let mut after = self.current.clone();
        for op in &plan.ops {
            after.apply(op);
        }
        Ok(Price { priced: heap::video(&after).owners, peak: heap::peak(&self.current, &plan.ops), share: heap::share(&after), short, holders: if short.is_some() { self.ui.holders() } else { Vec::new() }, gives })
    }

    /// `prefs` with the outside facts (`host` for the host stream) → `Desired` → the plan from the state built, admitted
    /// beside `others` held in the heap. What HDMI and USB gave by the precedence comes back as soon as the video fits
    /// without it; the giving built is the third part.
    fn planned(&self, prefs: &Prefs, host: Option<(&HostStream, Epoch)>, tuning: &Tuning, revision: Revision, others: u64) -> Result<(Desired, reconcile::Plan, Giving), String> {
        let plan_of = |prefs: &Prefs| -> Result<(Desired, reconcile::Plan), String> {
            let inputs = Inputs { monitor: &self.monitor, host, prefs, tuning };
            let desired = self.policy.plan(&inputs, &self.modes, revision).map_err(|e| format!("policy: {e}"))?;
            let plan = reconcile::plan(&self.current, &desired, &self.modes).map_err(|e| format!("plan: {e}"))?;
            Ok((desired, plan))
        };
        let fits = |plan: &reconcile::Plan| heap::admit(&self.current, &plan.ops, others).is_ok();
        if prefs.giving != Giving::default() {
            let back = Prefs { giving: Giving::default(), ..prefs.clone() };
            if let Ok((desired, plan)) = plan_of(&back).inspect_err(|e| log::debug!("what gave way not back: {e}")) {
                if fits(&plan) {
                    log::info!("what gave way by the precedence is back: {:?}", prefs.giving);
                    return Ok((desired, plan, Giving::default()));
                }
            }
        }
        let (desired, plan) = plan_of(prefs)?;
        Ok((desired, plan, prefs.giving))
    }

    /// Refused for the heap (the last transition was short of it): what holds it named; else refused.
    fn refusal(&self, e: String) -> Refusal {
        match self.short {
            Some(_) => Refusal::resources(e, self.ui.holders()),
            None => Refusal::new(Code::Refused, e),
        }
    }

    /// What the video holds, by what turns it off or lowers it, for a refusal to name ([`Ui::holders`]).
    fn video_holders(&self) -> Vec<Holder> {
        let [hdmi, usb, media, player] = heap::served(&self.current);
        let d = self.current.desired.as_ref();
        let mut v = Vec::new();
        if let Some(h) = d.and_then(|d| d.hdmi.as_ref()) {
            v.push(Holder { what: Held::Hdmi { timing: h.timing }, bytes: Some(hdmi) });
        }
        if let Some(u) = d.and_then(|d| d.usb.as_ref()) {
            v.push(Holder { what: Held::Usb { codec: u.codec, w: u.size.w, h: u.size.h, fps: u.fps }, bytes: Some(usb) });
        }
        if let Some(r) = &self.prefs.recording {
            v.push(Holder { what: Held::Recording { w: r.size.w, h: r.size.h }, bytes: Some(media) });
        } else if let (Some(_), Some(m)) = (self.photo.as_ref(), d.and_then(|d| d.media.as_ref())) {
            v.push(Holder { what: Held::Photo { w: m.size.w, h: m.size.h }, bytes: Some(media) });
        }
        if let Some(p) = &self.prefs.player {
            v.push(Holder { what: Held::Player { file: p.path.clone() }, bytes: Some(player) });
        }
        v
    }

    /// Healthy = this thread is here (not stuck in a call) and, while a capture is built, its frame counter moved since
    /// the last tick. A capture whose counter stands still gets no mark; the watchdog acts when that lasts.
    fn health_tick(&mut self) {
        self.last_tick = Instant::now();
        self.lock_tick();
        // the 2D engine lost: what it may still write is held in place; only a reset clears that, so the watchdog gets
        // no more health marks
        if mi::gfx::lost() {
            log::error!("the 2D engine is lost: no health mark, the watchdog resets the camera");
            return;
        }
        let capture = self.current.sensor.as_ref().is_some_and(|s| s.enabled) && self.current.vif.port_enabled;
        if !capture {
            self.frames_seen = None;
            self.health.ok();
            return;
        }
        match self.hal.frames() {
            Some(n) if self.frames_seen != Some(n) => {
                self.frames_seen = Some(n);
                self.health.ok();
            }
            Some(n) => log::warn!("capture: no new frames since the last check (counter {n})"),
            None => log::warn!("capture: frame counter unreadable"),
        }
    }

    /// The sensor kept in step with the display while HDMI shows its live picture at the display's rate: from the
    /// hand-overs' mean phase and DISP's releases since the last tick, the frame length for the next ([`microcam_core::lock`]);
    /// and with the phase near the aim, DISP handed each canvas when the port is lent it.
    fn lock_tick(&mut self) {
        let lock = self.current.sensor.as_ref().and_then(|s| self.modes.get(&s.mode).ok()).and_then(|m| microcam_core::lock::lock(&self.current, m));
        let Some(l) = lock else {
            self.lock_seen = None;
            self.hal.early(reconcile::topo::DISP_IN, false);
            return;
        };
        let Some((sum, n, released)) = self.hal.after_release(l.pool, l.disp) else { return };
        let seen = self.lock_seen.replace((l.pool, sum, n, released));
        let Some((_, sum0, n0, released0)) = seen.filter(|s| s.0 == l.pool && n > s.2 && released > s.3) else { return };
        let Some(period) = microcam_core::lock::period(&l, released - released0) else { return };
        let mean = (sum - sum0) as f64 / (n - n0) as f64;
        self.hal.early(l.disp, microcam_core::lock::early(&l, mean));
        let lines = microcam_core::lock::frame_lines(&l, period, mean, (released - released0) as f64 / period);
        if self.hal.frame_lines() != Some(lines) {
            if let Err(e) = self.hal.set_frame_lines(lines) {
                log::warn!("display lock: {e}");
            }
        }
    }

    fn status(&self) -> Status {
        Status {
            state: self.state,
            uptime_s: self.started.elapsed().as_secs(),
            monitor: describe_monitor(&self.monitor),
            host_stream: self.host.as_ref().map(|(h, e)| format!("{:?} {} @{} epoch {}", h.codec, h.size, h.fps, e.0)),
            desired: self.desired.clone(),
            current: self.current.clone(),
            last_error: self.last_error.clone(),
            last_transition: self.last_transition.clone(),
            transitions: self.transitions,
            pump: None,
            usb_delay: None,
            usb_ui: self.rgn[RgnId::Usb as usize].layer.as_ref().and_then(|r| r.describe()),
            media_ui: self.rgn[RgnId::Media as usize].layer.as_ref().and_then(|r| r.describe()),
            ui_levels: [self.prefs.levels.hdmi, self.prefs.levels.usb, self.prefs.levels.media],
            build: String::new(),
            scene: false,
            player: None,
            card: Default::default(),
            lists: Vec::new(),
        }
    }

    /// Inputs → Desired → plan → apply. On failure: roll back to nothing built, remember the error, stay Degraded.
    /// The wishes as they are, built again (an outside fact changed, or a step was asked for), asked by nobody.
    fn reconcile(&mut self, why: &str) -> Result<(), String> {
        self.ask(why, self.prefs.clone(), None)
    }

    /// `prefs` built for `asker` : when the media heap is short for it, the holders below it in the precedence
    /// give way ([`microcam_core::precedence::Precedence::give_way`]); refused when nothing below it can. Asked by
    /// nobody (a setting, a layout, an outside fact), it is refused when short, but for a paused output whose frame does
    /// not fit: that output asks for its frame ([`Self::way_for`]).
    fn ask(&mut self, why: &str, prefs: Prefs, asker: Option<Who>) -> Result<(), String> {
        let r = self.want(why, prefs.clone());
        if r.is_ok() || self.short.is_none() {
            return r;
        }
        let host = self.host.clone();
        let Some(way) = self.way_for(&prefs, host.as_ref().map(|(h, e)| (h, *e)), asker) else {
            log::info!("{why}: nothing below {} in the precedence frees what is short", asker.map_or("the paused output", Who::name));
            return r;
        };
        self.give(why, prefs, way)
    }

    /// The asker of an ending (a recording's stop, a photo's chain going): the outputs it gives back to, the higher of
    /// the two in the precedence.
    fn outputs_first(&self) -> Who {
        if self.prefs.precedence.above(Who::Usb, Who::Hdmi) {
            Who::Usb
        } else {
            Who::Hdmi
        }
    }

    /// Who gives way to `asker` for `prefs` to fit, each step priced by the plan that would build it beside what the
    /// menu would hold after; None when nothing below the asker can. Asked by nobody, the asker is the plan's paused
    /// output (the higher, when both are), and its going dark — HDMI off, USB refused — is the walk's top rung;
    /// None when the plan pauses nothing.
    fn way_for(&self, prefs: &Prefs, host: Option<(&HostStream, Epoch)>, asker: Option<Who>) -> Option<Way> {
        let menu = self.menu_bytes();
        let firm = mi::layer::firm();
        let tuning = self.controls.tuning();
        let revision = Revision(self.revision + 1);
        let own = match asker {
            Some(a) => (a, None),
            None => {
                let (d, _, _) = self.planned(prefs, host, &tuning, revision, firm).ok()?;
                let hdmi = d.hdmi.as_ref().is_some_and(|h| h.paused.is_some()).then_some((Who::Hdmi, Step::Off));
                let usb = d.usb.as_ref().is_some_and(|u| u.paused.is_some()).then_some((Who::Usb, Step::Refused));
                let (who, step) = match (hdmi, usb) {
                    (Some(h), Some(u)) => if prefs.precedence.above(Who::Hdmi, Who::Usb) { h } else { u },
                    (h, u) => h.or(u)?,
                };
                (who, Some(step))
            }
        };
        let mut found = None;
        let fits = |set: &[(Who, Step)]| {
            let (p, freed) = given(prefs, set, menu);
            let others = firm.saturating_sub(freed);
            let Ok((desired, plan, _)) = self.planned(&p, host, &tuning, revision, others) else { return false };
            match heap::admit(&self.current, &plan.ops, others) {
                Ok(needs) => {
                    found = Some((needs, desired));
                    true
                }
                Err(_) => false,
            }
        };
        let ladders = self.ladders(prefs);
        let steps = match own {
            (a, None) => prefs.precedence.give_way(a, &ladders, fits),
            (a, Some(step)) => prefs.precedence.give_way_or_go(a, step, &ladders, fits),
        }?;
        let (needs, desired) = found?;
        Some(Way { asker, steps, needs, desired })
    }

    /// What each holder now could give, best first, for [`Self::way_for`]: HDMI its lower timings, paused, off; USB
    /// paused, refused; a photo being taken given up; the player stopped; the menu's pictures and its drawing.
    fn ladders(&self, prefs: &Prefs) -> Vec<(Who, Vec<Step>)> {
        let d = self.current.desired.as_ref();
        let mut v = Vec::new();
        if let Some(h) = d.and_then(|d| d.hdmi.as_ref()) {
            let mut s: Vec<Step> = self.policy.hdmi_below(&self.monitor, h.timing).into_iter().map(Step::Timing).collect();
            if h.paused.is_none() {
                s.push(Step::Paused);
            }
            s.push(Step::Off);
            v.push((Who::Hdmi, s));
        }
        if let Some(u) = d.and_then(|d| d.usb.as_ref()) {
            v.push((Who::Usb, if u.paused.is_none() { vec![Step::Paused, Step::Refused] } else { vec![Step::Refused] }));
        }
        if self.photo.is_some() && prefs.photo.is_some() {
            v.push((Who::Photo, vec![Step::GivenUp]));
        }
        if prefs.player.is_some() {
            v.push((Who::Player, vec![Step::Stopped]));
        }
        let (pictures, drawing) = self.menu_bytes();
        if pictures > 0 {
            v.push((Who::Picture, vec![Step::NotShown]));
        }
        if drawing > 0 {
            v.push((Who::Drawing, vec![Step::Off]));
        }
        v
    }

    /// What the menu's steps free of microcamd's share now: its pictures, and the rest of its drawing — the bands go
    /// with the scene.
    fn menu_bytes(&self) -> (u64, u64) {
        let hs = self.ui.holders();
        let of = |w: Who| hs.iter().filter(|h| h.what.who() == w).filter_map(|h| h.bytes).sum::<u64>();
        (of(Who::Picture), of(Who::Drawing))
    }

    /// `prefs` built for the asker with `way` given: the menu's part first — microcamd's share shrunk to what the video
    /// will leave, the `ui` giving back until it holds no more — a photo being taken given up, then the build with the
    /// outputs' steps and the player stopped; an output gone dark told, with what took its room.
    fn give(&mut self, why: &str, prefs: Prefs, way: Way) -> Result<(), String> {
        let gives = |w: Who| way.steps.iter().any(|(x, _)| *x == w);
        log::info!("{why}: short for {}; giving way: {:?}", way.asker.map_or("the paused output's frame", Who::name), way.steps);
        // what took the room: the asker; asked by nobody, the video beside what goes dark (the paused frame among it)
        let holders: Vec<Holder> = match way.asker {
            Some(a) => held_by(a, &way.desired, &prefs).into_iter().collect(),
            None => self.video_holders().into_iter().filter(|h| !way.steps.iter().any(|&(w, s)| h.what.who() == w && matches!(s, Step::Off | Step::Refused))).collect(),
        };
        if gives(Who::Picture) || gives(Who::Drawing) {
            self.set_share(heap::share_beside(way.needs));
            self.ui.give_back(GiveBack { pictures: gives(Who::Picture), scene: gives(Who::Drawing), holders: holders.clone() });
            // given back once the book holds no more than the share and the RGN driver has freed the canvases of the
            // regions destroyed (the book lets a region go as it is destroyed, the heap gets it a frame or two later)
            let t0 = Instant::now();
            while (mi::layer::held() > mi::layer::share() || mi::rgn::burying()) && t0.elapsed() < GIVE_BACK_WAIT {
                std::thread::sleep(Duration::from_millis(10));
            }
            log::info!("{why}: microcamd's share down to {} KiB for the video, {} KiB held after {} ms", mi::layer::share() >> 10, mi::layer::held() >> 10, t0.elapsed().as_millis());
        }
        if gives(Who::Photo) {
            if let Some(p) = self.photo.take() {
                self.ui.photo_done();
                self.shot_failed(&p, Refusal::resources(format!("{why} took its room"), holders.clone()));
            }
        }
        let (p, _) = given(&prefs, &way.steps, (0, 0));
        let r = self.want(why, p);
        if r.is_err() {
            self.set_share(heap::share(&self.current));
        }
        for o in off(self.prefs.giving) {
            if !self.left_out.contains(&o) {
                self.left_out.push(o);
                self.ui.followers.tell(&Event::failed(Action::Output(o), Refusal::resources(format!("{why}: off by the precedence, the media heap short"), holders.clone())));
            }
        }
        r
    }

    /// A wish: `prefs` planned with the outside facts, admitted against the media heap and built; only then are they
    /// the wishes. Refused or failed, nothing of it stays and the error says why.
    fn want(&mut self, why: &str, prefs: Prefs) -> Result<(), String> {
        let giving = self.transition(why, &prefs, &self.controls.tuning(), false)?;
        self.prefs = Prefs { giving, ..prefs };
        Ok(())
    }

    /// A stop (the player's, a recording's) wanted: its plan gives back before it takes (`reconcile`), so it is let
    /// through whatever the menu holds when it holds no more than the video does now ([`heap::gives_back`]); one that
    /// takes more (an output coming back larger) is asked for by the outputs.
    fn stop(&mut self, why: &str, prefs: Prefs) -> Result<(), String> {
        match self.transition(why, &prefs, &self.controls.tuning(), true) {
            Ok(giving) => {
                self.prefs = Prefs { giving, ..prefs };
                Ok(())
            }
            Err(e) if self.short.is_some() => {
                let host = self.host.clone();
                match self.way_for(&prefs, host.as_ref().map(|(h, e)| (h, *e)), Some(self.outputs_first())) {
                    Some(way) => self.give(why, prefs, way),
                    None => Err(e),
                }
            }
            Err(e) => Err(e),
        }
    }

    /// Inputs → Desired → plan → admission → apply. A plan whose peak does not fit the heap beside what microcamd holds
    /// is not started, but a `stop` that only gives back. A step failing on the way: back to the last built state;
    /// everything down only when that fails too (Degraded).
    /// Returns what HDMI and USB gave by the precedence in the state built.
    fn transition(&mut self, why: &str, prefs: &Prefs, tuning: &Tuning, stop: bool) -> Result<Giving, String> {
        self.revision += 1;
        self.short = None;
        let (desired, plan, giving) = match self.planned(prefs, self.host.as_ref().map(|(h, e)| (h, *e)), tuning, Revision(self.revision), mi::layer::firm()) {
            Ok(x) => x,
            Err(msg) => {
                log::error!("{msg}");
                self.last_error = Some(msg.clone());
                return Err(msg);
            }
        };
        if plan.rung == Rung::Nothing {
            self.desired = Some(desired);
            return Ok(giving);
        }
        // microcamd's share while the steps run: what the video leaves at their peak, never above the share now (a stop
        // that only gives back keeps it). Set before what microcamd holds is read, so whatever the `ui` and `res` make
        // meanwhile is checked against it and takes nothing a step then misses; the build sets the share it leaves.
        let before = mi::layer::share();
        self.set_share(heap::share_beside(heap::peak(&self.current, &plan.ops)).min(before));
        // a stop that only gives back is let through whatever microcamd holds; any other fits beside it
        let admitted = if stop && heap::gives_back(&self.current, &plan.ops) { Ok(0) } else { admit(&self.current, &plan.ops) };
        if let Err(short) = admitted {
            self.set_share(before);
            self.short = Some(short);
            let msg = format!("{why}: refused, {short}");
            log::warn!("transition {msg}");
            self.last_error = Some(msg.clone());
            return Err(msg);
        }
        log::info!("transition ({why}): {:?}, {} ops → mode '{}' @{} hdmi {:?} usb {:?}", plan.rung, plan.ops.len(), desired.sensor.mode, desired.sensor.fps,
            desired.hdmi.as_ref().map(|h| h.timing.name()), desired.usb.as_ref().map(|u| format!("{:?} {} @{}", u.codec, u.size, u.fps)));
        // DISP handed canvases before their write only once the lock's next tick finds the new state locked
        self.hal.early(reconcile::topo::DISP_IN, false);
        let t0 = Instant::now();
        match self.build(&desired, &plan.ops) {
            Ok(()) => {
                let ms = t0.elapsed().as_millis();
                log::info!("transition done in {ms} ms");
                self.last_transition = Some(format!("{why}: {:?} {} ops {ms} ms", plan.rung, plan.ops.len()));
                self.transitions += 1;
                self.desired = Some(desired);
                self.state = DaemonState::Running;
                self.left_out.retain(|o| off(giving).contains(o));
                self.last_error = None;
                Ok(giving)
            }
            Err(e) => {
                let msg = format!("{why}: {e}");
                log::error!("transition failed: {msg}");
                self.last_error = Some(msg.clone());
                // the reconciler plans from any state: back to the last built one
                if let Some(back) = self.desired.clone() {
                    let undone = reconcile::plan(&self.current, &back, &self.modes).map_err(|e| e.to_string()).and_then(|p| {
                        log::info!("transition back to the state before it: {} ops", p.ops.len());
                        self.build(&back, &p.ops).map_err(|e| e.to_string())
                    });
                    match undone {
                        Ok(()) => return Err(msg),
                        Err(e) => log::error!("the way back failed too: {e}"),
                    }
                }
                self.state = DaemonState::Degraded;
                self.desired = None;
                self.teardown("rollback");
                Err(msg)
            }
        }
    }

    /// `ops` applied towards `desired`, the RGN layers retired before the steps that take their ports.
    /// microcamd's share is what the video built leaves, set after every one.
    fn build(&mut self, desired: &Desired, ops: &[reconcile::Op]) -> Result<(), reconcile::ApplyError> {
        let (rgn, ui) = (&mut self.rgn, &self.ui);
        let r = reconcile::apply_with(self.hal.as_mut(), &mut self.current, desired, ops, &mut |_, op| retire_for(rgn, &|id| ui.release_layer(id), op));
        self.set_share(heap::share(&self.current));
        r
    }

    /// microcamd's share of the heap set; the menu hears when it moved.
    fn set_share(&self, bytes: u64) {
        if mi::layer::share() != bytes {
            mi::layer::set_share(bytes);
            self.ui.share_moved();
        }
    }

    /// Tear down whatever `Current` says exists, consumer first. Errors are logged; the state keeps the truth.
    fn teardown(&mut self, why: &str) {
        let ops = reconcile::teardown_all(&self.current);
        if ops.is_empty() {
            return;
        }
        log::info!("teardown ({why}): {} ops", ops.len());
        self.take_down(&ops);
        self.current.desired = None;
    }

    /// Run teardown steps to the end whatever fails: a step that fails is logged and its resource counted as gone.
    fn take_down(&mut self, ops: &[reconcile::Op]) {
        for op in ops {
            let ui = &self.ui;
            if let Err(e) = retire_for(&mut self.rgn, &|id| ui.release_layer(id), op) {
                log::error!("teardown stopped: {e}");
                break;
            }
            match self.hal.exec(op) {
                Ok(()) => self.current.apply(op),
                Err(e) => {
                    log::error!("teardown step failed, continuing: {e}");
                    // assume the resource is gone as far as our bookkeeping goes: a stuck resource is reported by the
                    // next build failing, not hidden by a stale flag
                    self.current.apply(op);
                }
            }
        }
        self.set_share(heap::share(&self.current));
    }
}

/// Before a step that takes an RGN output's port away, that output's layer goes — right before it, not before the whole
/// transition: a recording's last frames still carry its `ui` while the steps before the port's stop the recorder.
/// `release` has the `ui` thread take the layer's regions off; when it says the 2D engine is lost, they stay and the
/// step must not run: taking the port away while the engine may still write a region is not risked.
fn retire_for(rgn: &mut [RgnSide; 3], release: &dyn Fn(RgnId) -> bool, op: &reconcile::Op) -> Result<(), reconcile::HalError> {
    for id in RgnId::ALL {
        let side = &mut rgn[id as usize];
        let Some(l) = side.layer.as_mut() else { continue };
        // a layer left in place keeps its own port, whatever the output became since
        if l.holds_port(op) {
            return Err(engine_lost(id));
        }
        let Some(port) = side.out.as_ref().and_then(|o| o.port) else { continue };
        if !overlay::touches(std::slice::from_ref(op), port) {
            continue;
        }
        if l.shows() {
            if !release(id) {
                l.abandon();
                return Err(engine_lost(id));
            }
            l.retire();
            // the output may come back the same: the layer is put back by the step right after the transition
            side.wake = Some(Instant::now());
        }
    }
    Ok(())
}

fn engine_lost(id: RgnId) -> reconcile::HalError {
    reconcile::HalError { op: format!("{} ui layer", id.name()), detail: "the 2D engine is lost; its layer's port stays until the watchdog resets the camera".into() }
}

/// Have the `ui` thread take a layer's regions off (or, the 2D engine lost, leave them).
fn let_go(ui: &crate::ui::Ui, id: RgnId, l: &mut LayerRgn) {
    if ui.release_layer(id) {
        l.retire();
    } else {
        log::error!("{} ui: the 2D engine is lost; layer left in place", id.name());
        l.abandon();
    }
}

/// Write and flush to the card as a new file, never over one there: the user may pull it right after the shot.
fn write_file(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut f = std::fs::File::create_new(path)?;
    f.write_all(bytes)?;
    f.sync_all()
}

pub fn describe_monitor(m: &Monitor) -> String {
    match m {
        Monitor::Absent => "absent".into(),
        Monitor::PresentNoEdid => "present, no EDID".into(),
        Monitor::Present(c) => {
            let best = c.modes.iter().max_by_key(|(s, r)| (s.w as u64 * s.h as u64, *r));
            match best {
                Some((s, r)) => format!("present, up to {s}@{r} ({} modes)", c.modes.len()),
                None => "present, empty EDID".into(),
            }
        }
    }
}

/// Send a message and wait for its reply with a bound.
pub fn ask<T>(tx: &Sender<Msg>, make: impl FnOnce(Sender<T>) -> Msg, timeout: Duration) -> Result<T, String> {
    let (rtx, rrx) = std::sync::mpsc::channel();
    tx.send(make(rtx)).map_err(|_| "pipeline thread gone".to_string())?;
    rrx.recv_timeout(timeout).map_err(|_| "pipeline did not answer in time".to_string())
}

/// Who gives way and how ([`Pipeline::way_for`]): the steps, the video's peak with them and the plan's `Desired`.
struct Way {
    /// None: asked by nobody, the paused output asking for its frame
    asker: Option<Who>,
    steps: Vec<(Who, Step)>,
    needs: u64,
    desired: Desired,
}

/// `prefs` with `set` given — HDMI's and USB's steps into the giving, a photo given up, the player stopped — and the
/// bytes of microcamd's share the menu's steps free (`menu`: its pictures', its drawing's).
fn given(prefs: &Prefs, set: &[(Who, Step)], (pictures, drawing): (u64, u64)) -> (Prefs, u64) {
    let mut p = prefs.clone();
    let mut freed = 0;
    for &(w, step) in set {
        match w {
            Who::Hdmi => p.giving.hdmi = Some(step),
            Who::Usb => p.giving.usb = Some(step),
            Who::Photo => p.photo = None,
            Who::Player => p.player = None,
            Who::Picture => freed += pictures,
            Who::Drawing => freed += drawing,
            Who::Recording => unreachable!("a recording gives nothing"),
        }
    }
    (p, freed)
}

/// The outputs `g` has off: HDMI off, USB refused.
fn off(g: Giving) -> Vec<Output> {
    [(Output::Hdmi, g.hdmi == Some(Step::Off)), (Output::Usb, g.usb == Some(Step::Refused))].into_iter().filter(|(_, o)| *o).map(|(o, _)| o).collect()
}

/// What `asker` holds in `d` (built from `prefs`), as the menu is told what took the room.
fn held_by(asker: Who, d: &Desired, prefs: &Prefs) -> Option<Holder> {
    let what = match asker {
        Who::Hdmi => Held::Hdmi { timing: d.hdmi.as_ref()?.timing },
        Who::Usb => d.usb.as_ref().map(|u| Held::Usb { codec: u.codec, w: u.size.w, h: u.size.h, fps: u.fps })?,
        Who::Recording => prefs.recording.as_ref().map(|r| Held::Recording { w: r.size.w, h: r.size.h })?,
        Who::Photo => d.media.as_ref().map(|m| Held::Photo { w: m.size.w, h: m.size.h })?,
        Who::Player => Held::Player { file: prefs.player.as_ref()?.path.clone() },
        Who::Picture | Who::Drawing => return None,
    };
    Some(Holder { what, bytes: None })
}

/// Whether `ops` may start from `cur` beside what microcamd holds firm in the heap (its share's use: the menu's tiles,
/// pictures, RGN canvases, bands, staging, patches; the spare frames are the book's to drop): admitted, the spare
/// beyond microcamd's share — set to what the peak leaves — goes.
fn admit(cur: &reconcile::Current, ops: &[reconcile::Op]) -> Result<u64, heap::Short> {
    let needs = heap::admit(cur, ops, mi::layer::firm())?;
    mi::layer::room(0);
    Ok(needs)
}

/// The visible live part of a feed's frame, as its compositor has it.
fn visible(way: &reconcile::FeedWay) -> Vec<Rect> {
    let reconcile::FeedWay::Canvas { visible, .. } = way;
    visible.clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use microcam_core::reconcile::{topo, ChnPort, HalError, Op};

    /// Records the steps and the layers taken off in one list.
    #[derive(Default, Clone)]
    struct Log(std::rc::Rc<std::cell::RefCell<Vec<String>>>);

    impl Hal for Log {
        fn exec(&mut self, op: &Op) -> Result<(), reconcile::HalError> {
            self.0.borrow_mut().push(format!("{op:?}").split([' ', '{']).next().unwrap().to_string());
            Ok(())
        }
    }

    impl Log {
        fn release(&self, ok: bool) -> impl Fn(RgnId) -> bool + '_ {
            move |id| {
                self.0.borrow_mut().push(format!("release {}", id.name()));
                ok
            }
        }
        fn take(&self) -> Vec<String> {
            std::mem::take(&mut self.0.borrow_mut())
        }
    }

    /// Every output with a layer shown: USB on SCL port 2, the recording on port 1, HDMI on port 0.
    fn shown() -> [RgnSide; 3] {
        let mut sides = [(); 3].map(|_| RgnSide { layer: Some(LayerRgn::new()), wake: None, out: None, generation: 1, info: None, why_not: None });
        for (side, port) in sides.iter_mut().zip([topo::SCL_OUT_USB, topo::SCL_OUT_MEDIA, topo::SCL_OUT_HDMI]) {
            let size = Size::new(1920, 1080);
            let out = RgnOutput { ui: OutputUi { raster: size, live: Rect::full(size), level: 3, paused: None }, port: Some(port), area: Rect::full(size), fps: 30, timing: None, codec: None, joined: None, visible: vec![Rect::full(size)] };
            let l = side.layer.as_mut().unwrap();
            l.set_output(1, Some(out.clone()));
            l.step(true);
            side.out = Some(out);
        }
        sides
    }

    #[test]
    fn a_recording_keeps_its_ui_until_the_step_that_takes_its_port() {
        let mut hal = Log::default();
        let mut rgn = shown();
        let (venc, port) = (topo::venc_in(topo::VENC_DEV_H26X, topo::VENC_CHN_MEDIA), topo::SCL_PORT_MEDIA);
        let ops = [
            Op::RecordStop,
            Op::Unbind { src: topo::SCL_OUT_MEDIA, dst: venc },
            Op::VencDown { dev: topo::VENC_DEV_H26X, chn: topo::VENC_CHN_MEDIA },
            Op::SclDisablePort { dev: topo::SCL_DEV, chn: topo::SCL_CHN, port },
        ];
        for op in &ops {
            retire_for(&mut rgn, &hal.release(true), op).unwrap();
            hal.exec(op).unwrap();
        }
        assert_eq!(hal.take(), ["RecordStop", "Unbind", "VencDown", "release media", "SclDisablePort"], "USB's and HDMI's layers stay");
    }

    #[test]
    fn with_the_engine_lost_the_step_that_takes_a_layer_s_port_does_not_run_and_never_will() {
        let hal = Log::default();
        let mut rgn = shown();
        let op = Op::SclDisablePort { dev: topo::SCL_DEV, chn: topo::SCL_CHN, port: topo::SCL_PORT_USB };
        assert!(retire_for(&mut rgn, &hal.release(false), &op).is_err());
        assert_eq!(hal.take(), ["release usb"]);
        // later, the engine still lost and the layer abandoned: still refused, not asked again
        assert!(retire_for(&mut rgn, &hal.release(true), &op).is_err());
        assert!(hal.take().is_empty());
        // the output gone since (a refused transition leaves nothing built): its old port is still held
        rgn[RgnId::Usb as usize].out = None;
        rgn[RgnId::Usb as usize].layer.as_mut().unwrap().set_output(2, None);
        for o in [op.clone(), Op::SclStopChannel { dev: topo::SCL_DEV, chn: topo::SCL_CHN }, Op::SclDestroyDevice { dev: topo::SCL_DEV }] {
            assert!(retire_for(&mut rgn, &hal.release(true), &o).is_err(), "{o:?}");
        }
        assert!(hal.take().iter().all(|c| c != "release usb"));
        // a step on another port goes on
        let other = Op::SclDisablePort { dev: topo::SCL_UP_DEV, chn: topo::SCL_UP_CHN, port: topo::SCL_UP_PORT };
        assert!(retire_for(&mut rgn, &hal.release(false), &other).is_ok());
    }

    #[test]
    fn a_step_on_the_whole_scaler_takes_every_output_s_layer_once() {
        let mut hal = Log::default();
        let mut rgn = shown();
        for op in [Op::SclStopChannel { dev: topo::SCL_DEV, chn: topo::SCL_CHN }, Op::SclDestroyChannel { dev: topo::SCL_DEV, chn: topo::SCL_CHN }] {
            retire_for(&mut rgn, &hal.release(true), &op).unwrap();
            hal.exec(&op).unwrap();
        }
        assert_eq!(hal.take(), ["release usb", "release media", "release hdmi", "SclStopChannel", "SclDestroyChannel"]);
    }
}
